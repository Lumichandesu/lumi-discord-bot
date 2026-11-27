//! Command logic shared by the `\` prefix commands and the slash commands.
//!
//! Texts follow the TypeScript bot that ran in production, so users see the same replies.

use std::sync::Arc;
use std::time::Duration;

use serenity::all::{
    ChannelId, CommandOptionType, CreateActionRow, CreateCommand, CreateCommandOption, CreateEmbed, CreateEmbedFooter,
    GuildId, Timestamp, UserId,
};

use crate::embed::{self, COLOR_MAIN};
use crate::lyrics;
use crate::player::{Bot, GuildPlayer};
use crate::queue::RepeatMode;
use crate::stats;
use crate::util::{format_duration, format_uptime, parse_time, truncate_chars};

pub const PREFIX: char = '\\';

/// Name, description, and an optional argument `(name, description, kind, required)`.
type Spec = (&'static str, &'static str, Option<(&'static str, &'static str, CommandOptionType, bool)>);

pub const SPECS: &[Spec] = &[
    ("play", "Play audio from supported platforms or search query", Some(("query", "Song title or URL", CommandOptionType::String, true))),
    ("pause", "Pause audio playback", None),
    ("resume", "Resume paused playback", None),
    ("skip", "Skip current track", None),
    ("stop", "Stop music and disconnect bot", None),
    ("queue", "Display current track and queue", None),
    ("nowplaying", "Display current song information", None),
    ("clear", "Clear entire music queue", None),
    ("loop", "Cycle loop mode (OFF -> TRACK -> QUEUE)", None),
    ("remove", "Remove track from queue by position", Some(("position", "Track position index", CommandOptionType::Integer, true))),
    ("shuffle", "Shuffle upcoming tracks in queue", None),
    ("ping", "Check bot latency and system performance", None),
    ("lyrics", "Fetch lyrics for current or specified song", Some(("query", "Song name and artist", CommandOptionType::String, false))),
    ("seek", "Jump to timestamp (mm:ss)", Some(("position", "Timestamp format mm:ss (e.g. 1:45)", CommandOptionType::String, true))),
    ("help", "Display command list and usage guide", None),
    ("stats", "View server or user music statistics", Some(("user", "Target user", CommandOptionType::User, false))),
    ("mystats", "View your personal music statistics", None),
    ("top", "View top played songs in this server", None),
];

/// The slash commands, with Discord's default contexts like the old registrations.
pub fn slash_commands() -> Vec<CreateCommand> {
    SPECS
        .iter()
        .map(|(name, desc, opt)| {
            let mut cmd = CreateCommand::new(*name).description(*desc);
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
        "leaderboard" => "top",
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
    /// Slash commands show this reply only to the caller.
    pub ephemeral: bool,
    /// Send nothing, as the old bot did for server-only commands used elsewhere.
    pub silent: bool,
}

impl Reply {
    pub fn text(s: impl Into<String>) -> Self {
        Self { content: Some(s.into()), ..Default::default() }
    }
    fn embed(e: CreateEmbed) -> Self {
        Self { embed: Some(e), ..Default::default() }
    }
    fn private(s: impl Into<String>) -> Self {
        Self { ephemeral: true, ..Self::text(s) }
    }
    fn silent() -> Self {
        Self { silent: true, ..Default::default() }
    }
}

/// Who ran a command and where, gathered from the cache before any `.await`.
pub struct Invocation {
    pub guild: Option<GuildId>,
    pub user: UserId,
    pub voice_channel: Option<ChannelId>,
    /// The text channel the command was used in.
    pub text_channel: ChannelId,
    pub guild_name: Option<String>,
    pub username: String,
    /// The `user` option of `/stats`, with that user's name.
    pub target_user: Option<(UserId, String)>,
    pub slash: bool,
    pub rest_ms: u128,
    pub ws_ms: Option<u128>,
}

impl Invocation {
    fn voice(&self) -> Result<ChannelId, String> {
        self.voice_channel.ok_or_else(|| "You must be in a voice channel to use this command.".into())
    }
    fn guild_name(&self) -> &str {
        self.guild_name.as_deref().unwrap_or("Server")
    }
}

/// Server-only commands answer nothing outside a server, like the old bot.
const GUILD_ONLY: &[&str] = &[
    "pause", "resume", "skip", "stop", "queue", "nowplaying", "clear", "loop", "remove", "shuffle", "seek", "stats",
    "mystats", "top",
];

pub async fn run(bot: &Arc<Bot>, inv: &Invocation, name: &str, args: &str) -> Reply {
    let Some(guild) = inv.guild else {
        if GUILD_ONLY.contains(&name) {
            return Reply::silent();
        }
        return match dispatch(bot, inv, None, name, args).await {
            Ok(r) => r,
            Err(e) => Reply::private(format!("❌ {e}")),
        };
    };
    match dispatch(bot, inv, Some(guild), name, args).await {
        Ok(r) => r,
        Err(e) => Reply::private(format!("❌ {e}")),
    }
}

/// `guild` is always `Some` for the commands in `GUILD_ONLY`.
async fn dispatch(bot: &Arc<Bot>, inv: &Invocation, guild: Option<GuildId>, name: &str, args: &str) -> Result<Reply, String> {
    let g = || guild.ok_or_else(|| "This command can only be used in a Discord server.".to_string());
    Ok(match name {
        "play" => {
            if args.is_empty() {
                return Ok(Reply::text("🎵 Usage: `\\play <Song Title / URL>`"));
            }
            // Play errors are public, as in the old bot.
            match play(bot, inv, guild, args).await {
                Ok(r) => r,
                Err(e) => Reply::text(format!("❌ {e}")),
            }
        }
        "pause" => Reply::text(match bot.set_paused(g()?, true) {
            Ok(()) => "⏸️ Playback paused.",
            Err(_) => "❌ Nothing playing or already paused.",
        }),
        "resume" => Reply::text(match bot.set_paused(g()?, false) {
            Ok(()) => "▶️ Playback resumed.",
            Err(_) => "❌ Playback is not paused.",
        }),
        "skip" => Reply::text(match bot.skip(g()?) {
            Some(t) => format!("⏭️ Skipped: **{}**", t.title),
            None => "❌ Nothing is playing.".into(),
        }),
        "stop" => Reply::text(if bot.stop(g()?).await {
            "⏹️ Playback stopped and disconnected."
        } else {
            "❌ Bot is not in voice channel."
        }),
        "queue" => {
            let desc = bot.with_player(g()?, |p| queue_description(p)).flatten();
            Reply::embed(
                CreateEmbed::new()
                    .color(COLOR_MAIN)
                    .title(truncate_chars(&format!("📜 Music Queue — {}", inv.guild_name()), 256))
                    .description(desc.unwrap_or_else(|| "The queue is currently empty.".into())),
            )
        }
        "nowplaying" => {
            let ui = bot
                .with_player(g()?, |p| p.queue.current.as_ref().map(|t| embed::now_playing(t, p.paused, p.queue.repeat, None)))
                .flatten();
            match ui {
                Some((e, rows)) => Reply { embed: Some(e), components: rows, ..Default::default() },
                None => Reply { ephemeral: inv.slash, ..Reply::text("❌ No track currently playing.") },
            }
        }
        "clear" => {
            let n = bot.with_player(g()?, |p| p.queue.clear()).unwrap_or(0);
            Reply::text(if n > 0 { format!("🗑️ Cleared **{n}** tracks from queue.") } else { "❌ Queue is already empty.".into() })
        }
        "loop" => {
            let mode = bot.with_player(g()?, |p| p.queue.cycle_repeat()).unwrap_or_default();
            bot.queue_changed(g()?);
            Reply::text(format!("🔁 Loop mode set to: **{}**", mode.as_str().to_uppercase()))
        }
        "remove" => {
            let Some(position) = parse_int_prefix(args.split_whitespace().next().unwrap_or("")) else {
                return Ok(Reply::text("❌ Usage: `\\remove <position>`"));
            };
            let removed = bot
                .with_player(g()?, |p| p.queue.remove(position))
                .unwrap_or_else(|| Err("The queue is currently empty.".into()));
            bot.queue_changed(g()?);
            match removed {
                Ok(t) => Reply::text(format!("🗑️ Removed: **{}**", t.title)),
                Err(e) => Reply { ephemeral: inv.slash, ..Reply::text(format!("❌ {e}")) },
            }
        }
        "shuffle" => {
            let n = bot.with_player(g()?, |p| p.queue.shuffle(&mut fastrand::Rng::new())).unwrap_or(0);
            bot.queue_changed(g()?);
            Reply::text(if n > 0 { format!("🔀 Shuffled **{n}** tracks.") } else { "❌ Queue is empty.".into() })
        }
        "ping" => Reply::embed(ping_embed(bot, inv)),
        "lyrics" => {
            let mut query = args.to_string();
            if query.is_empty()
                && let Some(guild) = guild
            {
                query = current_query(bot, guild).unwrap_or_default();
            }
            if query.is_empty() {
                return Ok(Reply::text("❌ Please provide a song name or play a track first."));
            }
            Reply::text(match lyrics::search(&bot.http, &query, Duration::from_secs(4)).await {
                Ok(Some(l)) => format!(
                    "📝 **Lyrics for {}:**\n\n{}",
                    l.track_name.as_deref().filter(|n| !n.is_empty()).unwrap_or(&query),
                    truncate_chars(l.plain_lyrics.as_deref().unwrap_or(""), 1900)
                ),
                Ok(None) => format!("❌ No lyrics found for **{query}**."),
                Err(_) => "❌ Failed to fetch lyrics.".into(),
            })
        }
        "seek" => {
            let guild = g()?;
            let time = args.split_whitespace().next().unwrap_or("");
            let Some(seconds) = parse_seek(time) else {
                return Ok(if inv.slash {
                    Reply::private("❌ Invalid format. Use `mm:ss` (e.g. `1:45`).")
                } else {
                    Reply::text("❌ Usage: `\\seek <mm:ss>` (e.g. `\\seek 1:45`)")
                });
            };
            let Some(Some(duration)) = bot.with_player(guild, |p| p.queue.current.as_ref().map(|t| t.duration)) else {
                return Ok(Reply::text("❌ No track currently playing."));
            };
            // Seeking past the end would only error the stream; the old bot then skipped the track.
            if duration > 0 && seconds >= duration {
                return Ok(Reply::text(format!("❌ Cannot seek past song duration ({}).", format_duration(duration))));
            }
            if bot.seek(guild, seconds).await {
                Reply::text(format!("⏩ Seeking to `{time}`..."))
            } else {
                Reply::text("❌ Failed to seek track.")
            }
        }
        "help" => Reply::embed(help_embed()),
        "stats" | "mystats" | "top" => {
            let (guild, Some(db)) = (g()?, bot.stats.as_ref()) else { return Ok(Reply::silent()) };
            let own = (inv.user, inv.username.clone());
            let first = args.split_whitespace().next().unwrap_or("").to_lowercase();
            let target = match name {
                "mystats" => Some(own),
                "stats" if matches!(first.as_str(), "me" | "user") => Some(own),
                "stats" => inv.target_user.clone(),
                _ => None,
            };
            let embed = match (name, target) {
                ("top", _) => stats::leaderboard_embed(&db.top(guild.to_string()).await?, inv.guild_name()),
                (_, Some((user, username))) => {
                    stats::user_embed(&db.user(guild.to_string(), user.to_string()).await?, &username)
                }
                _ => stats::server_embed(&db.server(guild.to_string()).await?, inv.guild_name()),
            };
            Reply::embed(embed)
        }
        _ => return Err("Command not found.".into()),
    })
}

async fn play(bot: &Arc<Bot>, inv: &Invocation, guild: Option<GuildId>, query: &str) -> Result<Reply, String> {
    let guild = guild.ok_or("This command can only be used in a Discord server.")?;
    // Like the old bot, only play needs a voice channel, and any channel will do.
    let channel = inv.voice()?;
    let out = bot.play(guild, channel, inv.text_channel, inv.user, query).await?;
    if out.queued {
        return Ok(Reply::text(format!("🎵 Added to queue: **{}**", out.track.title)));
    }
    // The old bot always showed loop OFF on this reply.
    let (e, rows) = embed::now_playing(&out.track, false, RepeatMode::Off, None);
    Ok(Reply { embed: Some(e), components: rows, ..Default::default() })
}

/// JavaScript `parseInt`: optional sign and leading digits, ignoring what follows (`3abc` is 3).
fn parse_int_prefix(s: &str) -> Option<i64> {
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
    digits[..end].parse::<i64>().ok().map(|n| sign * n)
}

/// Seek target: `mm:ss`, `h:mm:ss` or seconds. A bare `0` is rejected, as in the old bot.
fn parse_seek(s: &str) -> Option<u64> {
    let seconds = parse_time(s)?;
    (seconds > 0 || s.contains(':')).then_some(seconds)
}

/// Search text for the current track, used by lyrics.
pub fn current_query(bot: &Bot, guild: GuildId) -> Option<String> {
    bot.with_player(guild, |p| p.queue.current.as_ref().map(|t| format!("{} {}", t.title, t.author))).flatten()
}

/// Description of the queue embed. `None` when nothing is playing or queued.
fn queue_description(p: &GuildPlayer) -> Option<String> {
    if p.queue.is_idle() {
        return None;
    }
    let mut desc = String::new();
    if let Some(t) = &p.queue.current {
        desc += &format!("**Now Playing:**\n▶️ [{}]({}) (`{}`)\n\n", t.title, t.url, format_duration(t.duration));
    }
    let total = p.queue.upcoming.len();
    if total > 0 {
        desc += &format!("**Up Next ({total} tracks):**\n");
        let lines: Vec<String> = p
            .queue
            .upcoming
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, t)| format!("`{}.` [{}]({}) (`{}`)", i + 1, t.title, t.url, format_duration(t.duration)))
            .collect();
        desc += &lines.join("\n");
        if total > 10 {
            desc += &format!("\n*...and {} more*", total - 10);
        }
    }
    Some(truncate_chars(&desc, 4096).to_string())
}

/// The text of the Queue button.
pub fn queue_button_text(p: &GuildPlayer) -> String {
    let total = p.queue.upcoming.len();
    let mut text = format!("📜 **Queue List ({total} tracks):**\n");
    if total == 0 {
        text += "No upcoming tracks.";
        return text;
    }
    for (i, t) in p.queue.upcoming.iter().take(8).enumerate() {
        text += &format!("`{}.` {} (`{}`)\n", i + 1, t.title, format_duration(t.duration));
    }
    if total > 8 {
        text += &format!("\n*...and {} more*", total - 8);
    }
    text
}

fn ping_embed(bot: &Bot, inv: &Invocation) -> CreateEmbed {
    let ws = inv.ws_ms.map_or("n/a".into(), |ms| format!("{ms}ms"));
    let rss = memory_stats::memory_stats().map_or("n/a".into(), |m| format!("{:.2}", m.physical_mem as f64 / 1_048_576.0));
    CreateEmbed::new()
        .color(0x0099ff)
        .title("🏓 Pong!")
        .field("📡 Network Latency", format!("• WebSocket: `{ws}`\n• Roundtrip: `{}ms`", inv.rest_ms), true)
        .field(
            "💻 System Metrics",
            format!("• Uptime: `{}`\n• RAM Usage: `{rss} MB`", format_uptime(bot.started.elapsed().as_secs())),
            true,
        )
        .footer(CreateEmbedFooter::new(concat!("Lumi Bot | Rust v", env!("CARGO_PKG_VERSION"), " • power by Lumichandesu")))
        .timestamp(Timestamp::now())
}

fn help_embed() -> CreateEmbed {
    CreateEmbed::new()
        .color(COLOR_MAIN)
        .title("📖 Lumi Bot — Commands Guide")
        .description("Supported: Slash Commands (`/`) & Prefix Commands (`\\`)")
        .field(
            "🎵 Playback",
            [
                "`/play <query>` or `\\play` — Play audio from supported platforms",
                "`/pause` or `\\pause` — Pause playback",
                "`/resume` or `\\resume` — Resume playback",
                "`/skip` or `\\skip` — Skip track",
                "`/stop` or `\\stop` — Stop playback & leave",
                "`/seek <mm:ss>` or `\\seek` — Jump timestamp",
            ]
            .join("\n"),
            false,
        )
        .field(
            "📜 Queue",
            [
                "`/queue` or `\\queue` — View queue",
                "`/nowplaying` or `\\nowplaying` — Current song info",
                "`/loop` or `\\loop` — Cycle loop mode",
                "`/shuffle` or `\\shuffle` — Shuffle queue",
                "`/remove <#>` or `\\remove` — Remove track",
                "`/clear` or `\\clear` — Clear queue",
            ]
            .join("\n"),
            false,
        )
        .field(
            "📊 Analytics",
            [
                "`/stats` or `\\stats` — Server statistics",
                "`/mystats` or `\\mystats` — Personal listening stats",
                "`/top` or `\\top` — Server Top 10 chart",
            ]
            .join("\n"),
            false,
        )
        .field(
            "🛠️ Utilities",
            [
                "`/lyrics [query]` or `\\lyrics` — Fetch lyrics",
                "`/ping` or `\\ping` — Latency test",
                "`/help` or `\\help` — Command guide",
            ]
            .join("\n"),
            false,
        )
        .footer(CreateEmbedFooter::new("Lumi Bot | Ultra-Low RAM & Anti-Lag"))
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
        assert_eq!(canonical("Leaderboard"), Some("top"));
        assert_eq!(canonical("sendroles"), None);
        assert_eq!(canonical("nope"), None);
        assert_eq!(SPECS.len(), 18);
    }

    #[test]
    fn prefix_parsing() {
        assert_eq!(parse_prefix("\\play  never gonna  "), Some(("play", "never gonna")));
        assert_eq!(parse_prefix("\\skip"), Some(("skip", "")));
        assert_eq!(parse_prefix("\\ "), None);
        assert_eq!(parse_prefix("play x"), None);
    }

    #[test]
    fn remove_position_parses_like_parse_int() {
        assert_eq!(parse_int_prefix("3"), Some(3));
        assert_eq!(parse_int_prefix("3abc"), Some(3));
        assert_eq!(parse_int_prefix("-2"), Some(-2));
        assert_eq!(parse_int_prefix("abc"), None);
        assert_eq!(parse_int_prefix(""), None);
    }

    #[test]
    fn seek_rejects_bare_zero() {
        assert_eq!(parse_seek("0"), None);
        assert_eq!(parse_seek("0:00"), Some(0));
        assert_eq!(parse_seek("1:45"), Some(105));
        assert_eq!(parse_seek("90"), Some(90));
        assert_eq!(parse_seek(""), None);
    }
}
