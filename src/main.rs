//! Lumi: a self-hosted Discord music bot (serenity + songbird + yt-dlp).

mod commands;
mod embed;
mod lyrics;
mod player;
mod queue;
mod resolver;
mod roles;
mod stats;
mod util;
mod worker;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serenity::all::{
    ActivityData, ApplicationId, Command, CommandDataOptionValue, CommandInteraction, ComponentInteraction,
    Context, CreateAllowedMentions, CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
    CreateMessage, EditInteractionResponse, EditMessage, EventHandler, GatewayIntents, GuildId, Http, Interaction, Message,
    OnlineStatus, Ready, UserId, VoiceState,
};
use serenity::{Client, async_trait};
use songbird::SerenityInit;

use crate::commands::Invocation;
use crate::player::Bot;
use crate::util::{MESSAGE_MAX, format_duration, truncate_chars};

/// Settings read from the environment (or a `.env` file).
struct Config {
    token: String,
    client_id: Option<u64>,
    guild_id: Option<u64>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let id = |k: &str| -> Result<Option<u64>, String> {
            var(k).map(|v| v.parse::<u64>().map_err(|_| format!("{k} must be a numeric Discord ID"))).transpose()
        };
        Ok(Self {
            token: var("DISCORD_TOKEN").ok_or("DISCORD_TOKEN is not set (see .env.example)")?,
            client_id: id("DISCORD_CLIENT_ID")?,
            guild_id: id("DISCORD_GUILD_ID")?,
        })
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("TLS backend available")
}

struct Handler {
    bot: Arc<Bot>,
    presence_started: AtomicBool,
}

impl Handler {
    fn ws_latency_ms(&self, ctx: &Context) -> impl Future<Output = Option<u128>> + Send + 'static {
        let manager = self.bot.shard_manager.get().cloned();
        let shard = ctx.shard_id;
        async move {
            let runners = manager?.runners.clone();
            let runners = runners.lock().await;
            runners.get(&shard)?.latency.map(|d| d.as_millis())
        }
    }

    fn voice_channel(ctx: &Context, guild: Option<GuildId>, user: UserId) -> Option<serenity::all::ChannelId> {
        ctx.cache.guild(guild?)?.voice_states.get(&user)?.channel_id
    }

    fn guild_name(ctx: &Context, guild: Option<GuildId>) -> Option<String> {
        Some(ctx.cache.guild(guild?)?.name.clone())
    }

    fn bot_channel(ctx: &Context, guild: Option<GuildId>) -> Option<serenity::all::ChannelId> {
        let me = ctx.cache.current_user().id;
        Self::voice_channel(ctx, guild, me)
    }

    /// Only members who can manage roles in the server that owns the role channel may post the panel.
    async fn may_send_panel(ctx: &Context, msg: &Message) -> bool {
        let Some(guild) = msg.guild_id else { return false };
        let owns_channel = ctx.cache.guild(guild).is_some_and(|g| g.channels.contains_key(&roles::TARGET_CHANNEL_ID));
        if !owns_channel {
            return false;
        }
        let Ok(member) = msg.member(ctx).await else { return false };
        let Some(g) = ctx.cache.guild(guild) else { return false };
        let Some(channel) = g.channels.get(&roles::TARGET_CHANNEL_ID) else { return false };
        let perms = g.user_permissions_in(channel, &member);
        perms.manage_roles() || perms.administrator()
    }

    async fn on_prefix(&self, ctx: &Context, msg: &Message) {
        let Some((typed, args)) = commands::parse_prefix(&msg.content) else { return };
        // Posts the role panel; prefix-only and not listed in help, as in the old bot.
        if matches!(typed.to_lowercase().as_str(), "sendroles" | "send-roles") {
            if !Self::may_send_panel(ctx, msg).await {
                return;
            }
            match roles::send_panel(ctx).await {
                Ok(()) => {
                    let _ = msg.reply(&ctx.http, "✅ ส่งแผงรับยศเข้าห้องเรียบร้อยแล้ว!").await;
                }
                Err(e) => tracing::error!("Role panel channel not found or is not text based: {e}"),
            }
            return;
        }
        let Some(name) = commands::canonical(typed) else { return };

        // Start the work at once; the typing indicator is only a hint.
        let typing = matches!(name, "play" | "lyrics").then(|| {
            let (http, channel) = (ctx.http.clone(), msg.channel_id);
            tokio::spawn(async move {
                let _ = channel.broadcast_typing(&http).await;
            })
        });
        // Like the old bot, ping first sends a placeholder and measures that round trip.
        let started = Instant::now();
        let mut placeholder = None;
        if name == "ping" {
            let out = CreateMessage::new().content("🏓 Pinging...").reference_message(msg);
            match msg.channel_id.send_message(&ctx.http, out).await {
                Ok(m) => placeholder = Some(m),
                Err(e) => return tracing::warn!("could not reply to \\ping: {e}"),
            }
        }
        let inv = Invocation {
            guild: msg.guild_id,
            user: msg.author.id,
            voice_channel: Self::voice_channel(ctx, msg.guild_id, msg.author.id),
            text_channel: msg.channel_id,
            guild_name: Self::guild_name(ctx, msg.guild_id),
            username: msg.author.name.clone(),
            target_user: None,
            slash: false,
            rest_ms: started.elapsed().as_millis(),
            ws_ms: self.ws_latency_ms(ctx).await,
        };
        let reply = commands::run(&self.bot, &inv, name, args).await;
        if reply.silent {
            return;
        }
        if let (Some(mut m), Some(e)) = (placeholder, reply.embed.clone()) {
            let edit = EditMessage::new().content("").embed(e);
            if let Err(e) = m.edit(&ctx.http, edit).await {
                tracing::warn!("could not answer \\ping: {e}");
            }
            return;
        }

        // A typing request that reached Discord after the reply would show "typing" for 10 s.
        if let Some(typing) = typing {
            let _ = tokio::time::timeout(Duration::from_millis(300), typing).await;
        }
        // Replies echo user text and track titles: never let them ping @everyone, roles or users.
        let mut out = CreateMessage::new()
            .reference_message(msg)
            .allowed_mentions(CreateAllowedMentions::new().replied_user(true))
            .components(reply.components);
        if let Some(c) = reply.content {
            out = out.content(truncate_chars(&c, MESSAGE_MAX));
        }
        if let Some(e) = reply.embed {
            out = out.embed(e);
        }
        if let Err(e) = msg.channel_id.send_message(&ctx.http, out).await {
            tracing::warn!("could not reply to \\{name}: {e}");
        }
    }

    async fn on_slash(&self, ctx: &Context, cmd: &CommandInteraction) {
        let Some(name) = commands::canonical(&cmd.data.name).filter(|n| *n == cmd.data.name.to_lowercase()) else {
            let msg = CreateInteractionResponseMessage::new().content("❌ Command not found.").ephemeral(true);
            let _ = cmd.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await;
            return;
        };
        let args = cmd
            .data
            .options
            .first()
            .map(|o| match &o.value {
                CommandDataOptionValue::String(s) => s.clone(),
                CommandDataOptionValue::Integer(i) => i.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default();
        let target_user = cmd.data.options.iter().find_map(|o| match o.value {
            CommandDataOptionValue::User(id) => {
                let name = cmd.data.resolved.users.get(&id).map_or_else(|| id.to_string(), |u| u.name.clone());
                Some((id, name))
            }
            _ => None,
        });

        let started = Instant::now();
        if let Err(e) = cmd.defer(&ctx.http).await {
            tracing::warn!("could not defer /{name}: {e}");
            return;
        }
        let inv = Invocation {
            guild: cmd.guild_id,
            user: cmd.user.id,
            voice_channel: Self::voice_channel(ctx, cmd.guild_id, cmd.user.id),
            text_channel: cmd.channel_id,
            guild_name: Self::guild_name(ctx, cmd.guild_id),
            username: cmd.user.name.clone(),
            target_user,
            slash: true,
            rest_ms: started.elapsed().as_millis(),
            ws_ms: self.ws_latency_ms(ctx).await,
        };
        let reply = commands::run(&self.bot, &inv, name, args.trim()).await;
        if reply.silent {
            let _ = cmd.delete_response(&ctx.http).await;
            return;
        }
        if reply.ephemeral {
            // The deferred reply is public; replace it with a reply only the caller sees.
            let _ = cmd.delete_response(&ctx.http).await;
            let followup = CreateInteractionResponseFollowup::new()
                .content(truncate_chars(reply.content.as_deref().unwrap_or("❌ An error occurred."), MESSAGE_MAX))
                .allowed_mentions(CreateAllowedMentions::new())
                .ephemeral(true);
            if let Err(e) = cmd.create_followup(&ctx.http, followup).await {
                tracing::warn!("could not answer /{name}: {e}");
            }
            return;
        }

        let mut edit =
            EditInteractionResponse::new().allowed_mentions(CreateAllowedMentions::new()).components(reply.components);
        if let Some(c) = reply.content {
            edit = edit.content(truncate_chars(&c, MESSAGE_MAX));
        }
        if let Some(e) = reply.embed {
            edit = edit.embed(e);
        }
        if let Err(e) = cmd.edit_response(&ctx.http, edit).await {
            tracing::warn!("could not answer /{name}: {e}");
        }
    }

    async fn on_button(&self, ctx: &Context, i: &ComponentInteraction) {
        let ephemeral = |text: String| {
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(truncate_chars(&text, MESSAGE_MAX))
                    .allowed_mentions(CreateAllowedMentions::new())
                    .ephemeral(true),
            )
        };
        if i.data.custom_id.starts_with(roles::BUTTON_PREFIX) {
            return roles::on_button(ctx, i).await;
        }
        let Some(guild) = i.guild_id else { return };
        if !self.bot.has_player(guild) {
            let _ = i.create_response(&ctx.http, ephemeral("❌ No active player in this server.".into())).await;
            return;
        }
        let bot = &self.bot;
        // Rebuilds the now-playing message after a state change; `None` when nothing is playing.
        let refreshed = || {
            bot.with_player(guild, |p| {
                p.queue.current.as_ref().map(|t| {
                    let (e, rows) = embed::now_playing(t, p.paused, p.queue.repeat, None);
                    CreateInteractionResponse::UpdateMessage(
                        CreateInteractionResponseMessage::new().embed(e).components(rows),
                    )
                })
            })
            .flatten()
        };

        let id = i.data.custom_id.as_str();
        let caller = Self::voice_channel(ctx, Some(guild), i.user.id);
        if caller.is_none() || Self::bot_channel(ctx, Some(guild)) != caller {
            let _ = i.create_response(&ctx.http, ephemeral("❌ You must be in the same voice channel.".into())).await;
            return;
        }
        let response = match id {
            "btn_pause" => {
                let paused = bot.with_player(guild, |p| p.paused).unwrap_or(false);
                let _ = bot.set_paused(guild, !paused);
                refreshed().unwrap_or(CreateInteractionResponse::Acknowledge)
            }
            "btn_skip" => match bot.skip(guild) {
                Some(t) => ephemeral(format!("⏭️ Skipped: **{}**", t.title)),
                None => ephemeral("❌ No track to skip.".into()),
            },
            "btn_loop" => {
                let mode = bot.with_player(guild, |p| p.queue.cycle_repeat()).unwrap_or_default();
                refreshed().unwrap_or_else(|| ephemeral(format!("🔁 Loop mode: `{}`", mode.as_str().to_uppercase())))
            }
            "btn_shuffle" => {
                let n = bot.with_player(guild, |p| p.queue.shuffle(&mut fastrand::Rng::new())).unwrap_or(0);
                ephemeral(if n > 0 { format!("🔀 Shuffled **{n}** tracks.") } else { "❌ Queue is empty.".into() })
            }
            "btn_stop" => {
                bot.stop(guild).await;
                ephemeral("⏹️ Playback stopped and disconnected.".into())
            }
            "btn_queue" => ephemeral(bot.with_player(guild, |p| commands::queue_button_text(p)).unwrap_or_default()),
            "btn_lyrics" => return self.button_lyrics(ctx, i, guild).await,
            _ => return,
        };
        if let Err(e) = i.create_response(&ctx.http, response).await {
            tracing::warn!("button {} failed: {e}", i.data.custom_id);
        }
    }

    async fn button_lyrics(&self, ctx: &Context, i: &ComponentInteraction, guild: GuildId) {
        let Some(title) = self.bot.with_player(guild, |p| p.queue.current.as_ref().map(|t| t.title.clone())).flatten()
        else {
            let msg = CreateInteractionResponseMessage::new().content("❌ No track currently playing.").ephemeral(true);
            let _ = i.create_response(&ctx.http, CreateInteractionResponse::Message(msg)).await;
            return;
        };
        let defer = CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new().ephemeral(true));
        if i.create_response(&ctx.http, defer).await.is_err() {
            return;
        }
        let query = commands::current_query(&self.bot, guild).unwrap_or_else(|| title.clone());
        let text = match lyrics::search(&self.bot.http, &query, Duration::from_secs(4)).await {
            Ok(Some(l)) => {
                format!("📝 **Lyrics for {title}:**\n\n{}", truncate_chars(l.plain_lyrics.as_deref().unwrap_or(""), 1900))
            }
            _ => format!("❌ No lyrics found for **{title}**."),
        };
        let followup = CreateInteractionResponseFollowup::new()
            .content(truncate_chars(&text, MESSAGE_MAX))
            .allowed_mentions(CreateAllowedMentions::new())
            .ephemeral(true);
        if let Err(e) = i.create_followup(&ctx.http, followup).await {
            tracing::warn!("lyrics button failed: {e}");
        }
    }

    /// Number of non-bot users in the bot's voice channel, or `None` if the bot is not in one.
    fn humans_with_bot(ctx: &Context, guild: GuildId) -> Option<usize> {
        let me = ctx.cache.current_user().id;
        let guild = ctx.cache.guild(guild)?;
        let channel = guild.voice_states.get(&me)?.channel_id?;
        Some(
            guild
                .voice_states
                .values()
                .filter(|vs| vs.channel_id == Some(channel))
                .filter(|vs| {
                    let is_bot = vs.member.as_ref().map(|m| m.user.bot).or_else(|| guild.members.get(&vs.user_id).map(|m| m.user.bot));
                    !is_bot.unwrap_or(false)
                })
                .count(),
        )
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("logged in as {}", ready.user.tag());
        if self.presence_started.swap(true, Ordering::SeqCst) {
            return;
        }
        // Like the old bot: drop guild-scoped commands, then sync the global set on every start.
        for guild in &ready.guilds {
            let _ = guild.id.set_commands(&ctx.http, Vec::new()).await;
        }
        match Command::set_global_commands(&ctx.http, commands::slash_commands()).await {
            Ok(set) => tracing::info!("Successfully synced {} global slash commands.", set.len()),
            Err(e) => tracing::error!("Failed to register slash commands: {e}"),
        }
        tokio::spawn(async move {
            let activities = [
                ActivityData::listening("☕ ayo chill"),
                ActivityData::watching("📖 /help to know command"),
                ActivityData::listening("🎵 /play <song title>"),
                ActivityData::playing("✨ waku waku"),
                ActivityData::playing("📻 24/7 For everyone"),
            ];
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            for activity in activities.iter().cycle() {
                tick.tick().await;
                ctx.set_presence(Some(activity.clone()), OnlineStatus::Online);
            }
        });
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if !msg.author.bot && msg.content.starts_with(commands::PREFIX) {
            self.on_prefix(&ctx, &msg).await;
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => self.on_slash(&ctx, &cmd).await,
            Interaction::Component(i) => self.on_button(&ctx, &i).await,
            Interaction::Modal(m) => {
                roles::on_modal(&ctx, &m).await;
            }
            _ => {}
        }
    }

    async fn voice_state_update(&self, ctx: Context, _old: Option<VoiceState>, new: VoiceState) {
        let Some(guild) = new.guild_id.filter(|g| self.bot.has_player(*g)) else { return };
        if Self::humans_with_bot(&ctx, guild) != Some(0) {
            return;
        }
        let bot = self.bot.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if Self::humans_with_bot(&ctx, guild) == Some(0) && bot.stop(guild).await {
                tracing::info!("left {guild}: voice channel empty");
            }
        });
    }
}

async fn deploy(config: &Config) -> Result<(), String> {
    let client_id = config.client_id.ok_or("DISCORD_CLIENT_ID is required for --deploy")?;
    let http = Http::new(&config.token);
    http.set_application_id(ApplicationId::new(client_id));
    let cmds = commands::slash_commands();
    let n = cmds.len();
    match config.guild_id {
        Some(g) => GuildId::new(g).set_commands(&http, cmds).await.map(|_| ()),
        None => Command::set_global_commands(&http, cmds).await.map(|_| ()),
    }
    .map_err(|e| format!("Failed to update commands: {e}"))?;
    let scope = config.guild_id.map_or("globally".to_string(), |g| format!("in guild {g}"));
    println!("Registered {n} slash commands {scope}.");
    Ok(())
}

/// Checks the parts that do not need Discord: yt-dlp search, the stream URL, and audio probing.
/// Also times each lookup path: spawned yt-dlp, the warm worker (cold and warm), the InnerTube
/// search, and a prefetch of a stale next track.
async fn selftest() -> Result<(), String> {
    let http = http_client();
    let bot = Arc::new(Bot::new(songbird::Songbird::serenity(), http, None));
    let resolver = &bot.resolver;
    let probe = tokio::process::Command::new(resolver.ytdlp_program()).arg("--version").output().await;
    let version = match probe {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            println!("SKIP: yt-dlp not found ({}); nothing to test.", resolver.ytdlp_program());
            return Ok(());
        }
    };
    println!("yt-dlp {version}");
    let secs = |t: Instant| t.elapsed().as_secs_f32();

    let started = Instant::now();
    resolver.extract_spawned("ytsearch1:yoasobi idol").await?;
    println!("spawned yt-dlp search (old path) in {:.2}s", secs(started));

    let started = Instant::now();
    let track = resolver.resolve("rick astley never gonna give you up").await?;
    println!(
        "cold resolve (worker start + search) in {:.2}s: {} | {} | {} | protocol {}",
        secs(started),
        track.title,
        track.author,
        format_duration(track.duration),
        track.protocol.as_deref().unwrap_or("?")
    );
    if !resolver.worker.is_usable() {
        println!("yt-dlp worker not available here; lookups spawn yt-dlp");
    }
    let started = Instant::now();
    let id = resolver.innertube_search("yoasobi idol").await?;
    println!("InnerTube search in {:.2}s: {}", secs(started), id.as_deref().unwrap_or("no result"));
    for query in ["yoasobi idol", "aimer kataomoi"] {
        let started = Instant::now();
        let t = resolver.resolve(query).await?;
        println!("warm resolve {query:?} in {:.2}s: {}", secs(started), t.title);
    }
    let started = Instant::now();
    resolver.resolve(&track.url).await?;
    println!("warm resolve of a watch URL in {:.2}s", secs(started));
    let cached = Instant::now();
    resolver.resolve("Rick Astley never gonna give you up ").await?;
    println!("cache hit in {} us", cached.elapsed().as_micros());

    // A queue whose next track's stream URL is 40 minutes old, as after a long session.
    let guild = GuildId::new(1);
    let mut stale = resolver.resolve("aimer kataomoi").await?;
    stale.resolved_at = Instant::now().checked_sub(Duration::from_secs(40 * 60)).unwrap_or(stale.resolved_at);
    bot.players().entry(guild).or_default().queue.current = Some(track.clone());
    bot.players().entry(guild).or_default().queue.upcoming.push_back(stale);
    let started = Instant::now();
    if let Some(task) = bot.prefetch_next(guild, track.duration) {
        let _ = task.await;
    }
    let fresh = bot.with_player(guild, |p| p.queue.upcoming.front().map(|t| t.resolved_at.elapsed())).flatten();
    match fresh {
        Some(age) if age < Duration::from_secs(60) => println!(
            "prefetch refreshed the stale next track in {:.2}s while the current one played; the skip waits 0s",
            secs(started)
        ),
        _ => return Err("prefetch did not refresh the next track".into()),
    }
    bot.players().remove(&guild);

    // Open the stream the same way playback does and let songbird/symphonia probe the format.
    let mut input = bot.input_for(&track);
    let started = Instant::now();
    input = input
        .make_playable_async(songbird::input::codecs::get_codec_registry(), songbird::input::codecs::get_probe())
        .await
        .map_err(|e| format!("audio probe failed: {e}"))?;
    let codec = match &input {
        songbird::input::Input::Live(songbird::input::LiveInput::Parsed(p), _) => {
            match p.format.default_track().map(|t| t.codec_params.codec) {
                Some(songbird::input::core::codecs::CODEC_TYPE_OPUS) => "opus".to_string(),
                other => format!("{other:?}"),
            }
        }
        _ => "unknown".into(),
    };
    println!("audio stream opened and probed in {:.1}s (codec {codec})", secs(started));
    if let Some(pid) = resolver.worker.pid().await {
        let rss = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.trim_start_matches("VmRSS:").trim().to_string()));
        println!("yt-dlp worker pid {pid}, RSS {}", rss.as_deref().unwrap_or("n/a"));
    }
    println!("SELFTEST OK");
    Ok(())
}

fn print_usage() {
    println!(
        "lumi-discord-bot {}\n\nUsage: lumi-discord-bot [--deploy | --selftest | --help]\n\n  (no flag)   run the bot\n  --deploy    register the slash commands, then exit\n  --selftest  resolve a YouTube search through yt-dlp and open the audio stream, without Discord",
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lumi_discord_bot=info,serenity=warn,songbird=warn".into()),
        )
        .init();

    let arg = std::env::args().nth(1);
    let result = match arg.as_deref() {
        Some("--help" | "-h") => {
            print_usage();
            Ok(())
        }
        Some("--selftest") => selftest().await,
        Some("--deploy") => match Config::from_env() {
            Ok(c) => deploy(&c).await,
            Err(e) => Err(e),
        },
        Some(other) => Err(format!("unknown argument {other:?}; try --help")),
        None => match Config::from_env() {
            Ok(c) => run(c).await,
            Err(e) => Err(e),
        },
    };
    if let Err(e) = result {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

/// Waits for Ctrl+C, or on Unix also for SIGTERM, which `systemctl stop` and `restart` send.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
                return;
            }
            Err(e) => tracing::warn!("could not listen for SIGTERM: {e}"),
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn run(config: Config) -> Result<(), String> {
    let songbird = songbird::Songbird::serenity();
    let http = http_client();
    let stats = stats::Stats::open(std::path::Path::new(stats::DB_PATH), http.clone())?;
    let bot = Arc::new(Bot::new(songbird.clone(), http, Some(stats)));
    let playing = Arc::downgrade(&bot);
    bot.resolver.worker.keep_while(move || playing.upgrade().is_some_and(|b| b.is_playing()));
    bot.resolver.worker.prewarm();
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_VOICE_STATES;

    let mut client = Client::builder(&config.token, intents)
        .event_handler(Handler { bot: bot.clone(), presence_started: AtomicBool::new(false) })
        .register_songbird_with(songbird)
        .await
        .map_err(|e| format!("could not create the Discord client: {e}"))?;
    let _ = bot.shard_manager.set(client.shard_manager.clone());
    let _ = bot.discord.set(client.http.clone());

    let shards = client.shard_manager.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutting down");
        shards.shutdown_all().await;
    });

    client.start().await.map_err(|e| format!("Discord login failed: {e}"))
}
