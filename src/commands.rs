//! Command logic shared by the `\` prefix commands and the slash commands.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{
    ChannelId, CommandOptionType, CreateActionRow, CreateCommand, CreateCommandOption, CreateEmbed,
    CreateEmbedFooter, GuildId, InstallationContext, InteractionContext, Timestamp,
};

use crate::embed::{self, COLOR_MAIN};
use crate::lyrics;
use crate::player::{Bot, GuildPlayer};
use crate::util::{format_duration, format_uptime, parse_time, truncate_chars};

pub const PREFIX: char = '\\';

/// Name, description, and an optional argument `(name, description, kind, required)`.
type Spec = (&'static str, &'static str, Option<(&'static str, &'static str, CommandOptionType, bool)>);

pub const SPECS: &[Spec] = &[
    ("play", "Stream music from YouTube, Spotify, Apple Music, TIDAL, or SoundCloud", Some(("query", "Song title or track URL", CommandOptionType::String, true))),
    ("pause", "Pause current playback", None),
    ("resume", "Resume paused playback", None),
    ("skip", "Skip the currently playing track", None),
    ("stop", "Stop playback, clear queue, and leave voice channel", None),
    ("queue", "View upcoming tracks in the queue", None),
    ("nowplaying", "Display the currently playing track", None),
    ("clear", "Clear all tracks in the upcoming queue", None),
    ("loop", "Cycle loop mode: off, track, queue", None),
    ("remove", "Remove a track from the queue", Some(("position", "Queue position to remove", CommandOptionType::Integer, true))),
    ("shuffle", "Randomize queued tracks", None),
    ("ping", "Show latency, memory use and uptime", None),
    ("lyrics", "Show lyrics for the current song or a search", Some(("query", "Song title (defaults to the current track)", CommandOptionType::String, false))),
    ("seek", "Jump to a time in the current track", Some(("time", "mm:ss, h:mm:ss or seconds", CommandOptionType::String, true))),
    ("help", "List commands", None),
];

pub fn slash_commands() -> Vec<CreateCommand> {
    SPECS
        .iter()
        .map(|(name, desc, opt)| {
            let mut cmd = CreateCommand::new(*name)
                .description(*desc)
                .contexts(vec![InteractionContext::Guild, InteractionContext::BotDm, InteractionContext::PrivateChannel])
                .integration_types(vec![InstallationContext::Guild, InstallationContext::User]);
            if let Some((oname, odesc, kind, required)) = opt {
                cmd = cmd.add_option(CreateCommandOption::new(*kind, *oname, *odesc).required(*required));
            }
            cmd
        })
        .collect()
}

/// Maps a typed command name or alias to its canonical name.
pub fn canonical(name: &str) -> Option<&'static str> {
    let name = name.to_lowercase();
    let alias = match name.as_str() {
        "p" => "play",
        "s" => "skip",
        "np" => "nowplaying",
        "q" => "queue",
        "h" => "help",
        "l" => "lyrics",
        other => other,
    };
    SPECS.iter().map(|s| s.0).find(|n| *n == alias)
}

/// Splits `\name rest of args` into the name and the trimmed argument text.
pub fn parse_prefix(content: &str) -> Option<(&str, &str)> {
    let body = content.strip_prefix(PREFIX)?.trim_start();
    let (name, args) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    (!name.is_empty()).then(|| (name, args.trim()))
}

#[derive(Default)]
pub struct Reply {
    pub content: Option<String>,
    pub embed: Option<CreateEmbed>,
    pub components: Vec<CreateActionRow>,
}

impl Reply {
    pub fn text(s: impl Into<String>) -> Self {
        Self { content: Some(s.into()), ..Default::default() }
    }
    fn embed(e: CreateEmbed) -> Self {
        Self { embed: Some(e), ..Default::default() }
    }
    pub fn error(s: impl std::fmt::Display) -> Self {
        Self::text(format!("❌ {s}"))
    }
}

/// Who ran a command and where, gathered from the cache before any `.await`.
pub struct Invocation {
    pub guild: Option<GuildId>,
    pub voice_channel: Option<ChannelId>,
    pub rest_ms: u128,
    pub ws_ms: Option<u128>,
}

impl Invocation {
    fn guild(&self) -> Result<GuildId, String> {
        self.guild.ok_or_else(|| "This command can only be used in a Discord server.".into())
    }
    fn voice(&self) -> Result<ChannelId, String> {
        self.voice_channel.ok_or_else(|| "You must be in a voice channel to use this command.".into())
    }
    /// Guild plus a check that the caller is in a voice channel.
    fn guild_in_voice(&self) -> Result<GuildId, String> {
        let g = self.guild()?;
        self.voice()?;
        Ok(g)
    }
}

pub async fn run(bot: &Arc<Bot>, inv: &Invocation, name: &str, args: &str) -> Reply {
    match dispatch(bot, inv, name, args).await {
        Ok(r) => r,
        Err(e) => Reply::error(e),
    }
}

async fn dispatch(bot: &Arc<Bot>, inv: &Invocation, name: &str, args: &str) -> Result<Reply, String> {
    Ok(match name {
        "play" => {
            if args.is_empty() {
                return Ok(Reply::text("🎵 Usage: `\\play <Song Title / URL (YouTube, Spotify, Apple Music, TIDAL, SoundCloud)>`"));
            }
            let guild = inv.guild()?;
            let channel = inv.voice()?;
            let out = bot.play(guild, channel, args).await?;
            if out.queued {
                Reply::text(format!("🎵 Added to queue: **{}**", out.track.title))
            } else {
                let (e, rows) = embed::now_playing(&out.track, false, out.repeat);
                Reply { content: None, embed: Some(e), components: rows }
            }
        }
        "pause" => {
            bot.set_paused(inv.guild_in_voice()?, true);
            Reply::text("⏸️ Playback paused.")
        }
        "resume" => {
            bot.set_paused(inv.guild_in_voice()?, false);
            Reply::text("▶️ Playback resumed.")
        }
        "skip" => {
            let skipped = bot.skip(inv.guild_in_voice()?).flatten();
            Reply::text(format!("⏭️ Skipped: **{}**", skipped.map_or("Current track".into(), |t| t.title)))
        }
        "stop" => {
            bot.stop(inv.guild_in_voice()?).await;
            Reply::text("⏹️ Stopped and cleared queue.")
        }
        "queue" => {
            let guild = inv.guild()?;
            let text = bot.with_player(guild, |p| queue_text(p, 10, false)).flatten();
            Reply::text(text.unwrap_or_else(|| "📭 Queue is empty.".into()))
        }
        "nowplaying" => {
            let guild = inv.guild()?;
            let text = bot
                .with_player(guild, |p| {
                    p.queue.current.as_ref().map(|t| {
                        let author = if t.author.is_empty() { "Unknown" } else { &t.author };
                        format!(
                            "🎵 **Now Playing:** [{}]({})\n👤 **Artist:** {author}\n🔁 **Loop Mode:** `{}`",
                            t.title,
                            t.url,
                            p.queue.repeat.as_str()
                        )
                    })
                })
                .flatten();
            Reply::text(text.unwrap_or_else(|| "🔇 Nothing is playing.".into()))
        }
        "clear" => {
            let n = bot.with_player(inv.guild_in_voice()?, |p| p.queue.clear()).unwrap_or(0);
            Reply::text(format!("🗑️ Cleared **{n}** track(s)."))
        }
        "loop" => {
            let mode = bot.with_player(inv.guild_in_voice()?, |p| p.queue.cycle_repeat()).unwrap_or_default();
            Reply::text(format!("🔁 Loop mode set to: `{}`", mode.as_str()))
        }
        "remove" => {
            let guild = inv.guild_in_voice()?;
            let Ok(position) = args.split_whitespace().next().unwrap_or("").parse::<i64>() else {
                return Ok(Reply::error("Usage: `\\remove <number>`"));
            };
            let removed = bot
                .with_player(guild, |p| p.queue.remove(position))
                .unwrap_or_else(|| Err("The queue is currently empty.".into()))?;
            Reply::text(format!("🗑️ Removed `#{position}`: **{}**", removed.title))
        }
        "shuffle" => {
            let n = bot.with_player(inv.guild_in_voice()?, |p| p.queue.shuffle(&mut fastrand::Rng::new())).unwrap_or(0);
            Reply::text(format!("🔀 Shuffled **{n}** tracks."))
        }
        "ping" => Reply::embed(ping_embed(bot, inv)),
        "lyrics" => {
            let mut query = args.to_string();
            if query.is_empty()
                && let Some(guild) = inv.guild
            {
                query = current_query(bot, guild).unwrap_or_default();
            }
            if query.is_empty() {
                return Ok(Reply::text("📝 Usage: `\\lyrics <song title>` (or play a song first)."));
            }
            match lyrics::search(&bot.http, &query, Duration::from_secs(5)).await {
                Ok(Some(l)) => Reply::embed(
                    CreateEmbed::new()
                        .color(COLOR_MAIN)
                        .title(format!(
                            "📝 {} - {}",
                            l.track_name.as_deref().unwrap_or("Unknown"),
                            l.artist_name.as_deref().unwrap_or("Unknown")
                        ))
                        .description(truncate_chars(l.plain_lyrics.as_deref().unwrap_or(""), 3900))
                        .footer(CreateEmbedFooter::new("Lyrics provided by LRCLIB"))
                        .timestamp(Timestamp::now()),
                ),
                Ok(None) => Reply::error(format!("No lyrics found for **{query}**.")),
                Err(_) => Reply::error("Failed to retrieve lyrics."),
            }
        }
        "seek" => {
            let guild = inv.guild_in_voice()?;
            let Some(Some(duration)) = bot.with_player(guild, |p| p.queue.current.as_ref().map(|t| t.duration)) else {
                return Ok(Reply::error("No track currently playing."));
            };
            if args.is_empty() {
                return Ok(Reply::text("⏩ Usage: `\\seek <mm:ss | seconds>` (e.g. `\\seek 1:45`)"));
            }
            let Some(seconds) = parse_time(args) else {
                return Ok(Reply::error("Invalid time format. Use `mm:ss` or seconds."));
            };
            if duration > 0 && seconds >= duration {
                return Ok(Reply::error(format!("Cannot seek past song duration ({}).", format_duration(duration))));
            }
            if bot.seek(guild, seconds).await {
                Reply::text(format!("⏩ Jumped to `{}`", format_duration(seconds)))
            } else {
                Reply::error("Failed to seek track.")
            }
        }
        "help" => Reply::embed(help_embed()),
        _ => return Err("Unknown command.".into()),
    })
}

/// Search text for the current track, used by lyrics.
pub fn current_query(bot: &Bot, guild: GuildId) -> Option<String> {
    bot.with_player(guild, |p| p.queue.current.as_ref().map(|t| format!("{} {}", t.title, t.author))).flatten()
}

/// Queue listing. `None` when nothing is playing or queued.
pub fn queue_text(p: &GuildPlayer, limit: usize, durations: bool) -> Option<String> {
    if p.queue.is_idle() {
        return None;
    }
    let mut text = match &p.queue.current {
        Some(t) => format!("🎶 **Now Playing:** [{}]({})\n\n", t.title, t.url),
        None => "🎶 **Now Playing:** None\n\n".into(),
    };
    let total = p.queue.upcoming.len();
    if total > 0 {
        let label = if durations { "Up Next" } else { "Queue" };
        text += &format!("📜 **{label} ({total} tracks):**\n");
        for (i, t) in p.queue.upcoming.iter().take(limit).enumerate() {
            text += &format!("`{}.` [{}]({})", i + 1, t.title, t.url);
            if durations {
                text += &format!(" - `{}`", format_duration(t.duration));
            }
            text.push('\n');
        }
        if total > limit {
            text += &format!("\n*...and {} more*", total - limit);
        }
    }
    Some(text)
}

fn ping_embed(bot: &Bot, inv: &Invocation) -> CreateEmbed {
    let ws = inv.ws_ms.map_or("n/a".into(), |ms| format!("{ms}ms"));
    let rss = memory_stats::memory_stats().map_or("n/a".into(), |m| format!("{:.2} MB", m.physical_mem as f64 / 1_048_576.0));
    CreateEmbed::new()
        .title("🏓 Pong!")
        .color(0x0099ff)
        .field("📡 Network Latency", format!("• WebSocket: `{ws}`\n• Roundtrip: `{}ms`", inv.rest_ms), true)
        .field(
            "🖥️ System Metrics",
            format!("• Uptime: `{}`\n• Memory (RSS): `{rss}`", format_uptime(bot.started.elapsed().as_secs())),
            true,
        )
        .footer(CreateEmbedFooter::new("Lumi Bot | Rust edition"))
        .timestamp(Timestamp::now())
}

fn help_embed() -> CreateEmbed {
    CreateEmbed::new()
        .color(COLOR_MAIN)
        .title("📖 Lumi Bot — Commands Guide")
        .description("Prefix: `\\` | Every command is also a slash command. Buttons appear under the player message.")
        .field(
            "🎵 Music Playback",
            [
                "`\\play <title/url>` — Play audio from YouTube, Spotify, Apple Music, TIDAL, SoundCloud",
                "`\\pause` — Pause current playback",
                "`\\resume` — Resume playback",
                "`\\skip` — Skip to the next track",
                "`\\stop` — Stop music and disconnect bot",
                "`\\seek <mm:ss>` — Jump to timestamp (e.g. `\\seek 1:45`)",
            ]
            .join("\n"),
            false,
        )
        .field(
            "📜 Queue & Controls",
            [
                "`\\queue` — Show current track and upcoming list",
                "`\\nowplaying` — Show detailed info of the current song",
                "`\\loop` — Cycle loop mode (`OFF` ➔ `TRACK` ➔ `QUEUE`)",
                "`\\shuffle` — Shuffle upcoming tracks",
                "`\\remove <#>` — Remove a specific track from queue",
                "`\\clear` — Clear entire queue",
            ]
            .join("\n"),
            false,
        )
        .field(
            "🛠️ Utilities",
            [
                "`\\lyrics [query]` — Fetch plain lyrics",
                "`\\ping` — Check latency and system performance",
                "`\\help` — Show this command list",
            ]
            .join("\n"),
            false,
        )
        .footer(CreateEmbedFooter::new("Lumi Bot | Rust edition"))
        .timestamp(Timestamp::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_and_names() {
        assert_eq!(canonical("p"), Some("play"));
        assert_eq!(canonical("NP"), Some("nowplaying"));
        assert_eq!(canonical("l"), Some("lyrics"));
        assert_eq!(canonical("seek"), Some("seek"));
        assert_eq!(canonical("nope"), None);
        assert_eq!(SPECS.len(), 15);
    }

    #[test]
    fn prefix_parsing() {
        assert_eq!(parse_prefix("\\play  never gonna  "), Some(("play", "never gonna")));
        assert_eq!(parse_prefix("\\skip"), Some(("skip", "")));
        assert_eq!(parse_prefix("\\ "), None);
        assert_eq!(parse_prefix("play x"), None);
    }
}
