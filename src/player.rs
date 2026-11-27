//! Per-guild playback state on top of songbird.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serenity::all::{ChannelId, CreateAllowedMentions, CreateMessage, GuildId, Http, ShardManager, UserId};
use serenity::async_trait;
use songbird::events::{CoreEvent, Event, EventContext, EventHandler, TrackEvent};
use songbird::input::{ChildContainer, HlsRequest, HttpRequest, Input, RawAdapter};
use songbird::tracks::TrackHandle;
use songbird::Songbird;
use symphonia::core::io::ReadOnlySource;
use tokio::task::JoinHandle;

use crate::embed;
use crate::queue::{MAX_UPCOMING, Queue, RepeatMode};
use crate::resolver::{Resolver, Track};
use crate::stats::{Play, Stats};

/// Leave after this long with nothing playing and nothing queued.
const IDLE_LEAVE: Duration = Duration::from_secs(180);
/// Stream URLs older than this are resolved again before playing. Googlevideo URLs last about
/// 6 h, SoundCloud HLS URLs much less.
const STREAM_MAX_AGE: Duration = Duration::from_secs(30 * 60);
/// A prefetch also covers a track that will turn stale within this long after the current one.
const PREFETCH_SLACK: Duration = Duration::from_secs(60);
/// The voice join starts this long after the lookup, so a lookup that fails at once (a bad or
/// private link) never joins the channel; any other lookup takes longer anyway.
const JOIN_DELAY: Duration = Duration::from_millis(250);

#[derive(Default)]
pub struct GuildPlayer {
    pub queue: Queue<Track>,
    pub paused: bool,
    handle: Option<TrackHandle>,
    /// Increases every time a track starts or ends, so stale songbird events can be ignored.
    generation: u64,
    idle_task: Option<JoinHandle<()>>,
    /// Set while a seek is in flight. A failed seek errors the track in songbird; that error
    /// must restart the track instead of dropping it from the queue.
    seeking: bool,
    /// Set by skip() so the end event moves past the track even in track-repeat mode.
    skip_requested: bool,
    /// The current track already failed once and was retried with a fresh stream URL.
    retried: bool,
    /// Where the last play command came from; later tracks are announced here.
    text_channel: Option<ChannelId>,
}

/// Why a track is being started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// The first track of a play command; the command's reply shows it.
    First,
    /// Queue advance, repeat, or the next track after a failure: announced in the text channel.
    Next,
    /// The same track again with a fresh stream URL after it failed.
    Retry,
    /// The same track restarted so it can seek.
    SeekRestart,
}

impl GuildPlayer {
    fn cancel_idle(&mut self) {
        if let Some(t) = self.idle_task.take() {
            t.abort();
        }
    }
}

pub struct Bot {
    pub songbird: Arc<Songbird>,
    pub http: reqwest::Client,
    pub resolver: Resolver,
    pub started: Instant,
    pub shard_manager: OnceLock<Arc<ShardManager>>,
    /// Discord REST client, used to announce tracks outside a command reply.
    pub discord: OnceLock<Arc<Http>>,
    pub stats: Option<Arc<Stats>>,
    players: Mutex<HashMap<GuildId, GuildPlayer>>,
    /// One lock per guild so two play commands cannot join the same channel at once
    /// (songbird cancels an in-progress join when a second one starts).
    join_locks: Mutex<HashMap<GuildId, Arc<tokio::sync::Mutex<()>>>>,
    /// Play commands still running, per guild.
    in_flight: Mutex<HashMap<GuildId, Flight>>,
}

#[derive(Default)]
struct Flight {
    count: usize,
    /// One of them joined the voice channel and nothing has been queued since.
    joined: bool,
}

/// Counts a running play command until it is dropped.
struct InFlight<'a> {
    bot: &'a Bot,
    guild: GuildId,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut flights = self.bot.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(f) = flights.get_mut(&self.guild) {
            f.count -= 1;
            if f.count == 0 {
                flights.remove(&self.guild);
            }
        }
    }
}

pub struct PlayOutcome {
    pub queued: bool,
    pub track: Track,
}

impl Bot {
    pub fn new(songbird: Arc<Songbird>, http: reqwest::Client, stats: Option<Stats>) -> Self {
        Self {
            songbird,
            resolver: Resolver::new(http.clone()),
            http,
            started: Instant::now(),
            shard_manager: OnceLock::new(),
            discord: OnceLock::new(),
            stats: stats.map(Arc::new),
            players: Mutex::new(HashMap::new()),
            join_locks: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    pub fn players(&self) -> MutexGuard<'_, HashMap<GuildId, GuildPlayer>> {
        // A panic while holding this lock aborts the process (panic = "abort"), so poisoning
        // cannot be observed in release builds; recover the data in debug builds too.
        self.players.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a track is loaded in any guild, playing or paused.
    pub fn is_playing(&self) -> bool {
        self.players().values().any(|p| p.handle.is_some())
    }

    pub fn has_player(&self, guild: GuildId) -> bool {
        self.players().contains_key(&guild)
    }

    /// Runs `f` on the guild's player, if there is one.
    pub fn with_player<R>(&self, guild: GuildId, f: impl FnOnce(&mut GuildPlayer) -> R) -> Option<R> {
        self.players().get_mut(&guild).map(f)
    }

    /// Resolves the query, joins the voice channel if needed, and starts or queues the track.
    pub async fn play(
        self: &Arc<Self>,
        guild: GuildId,
        channel: ChannelId,
        text_channel: ChannelId,
        user: UserId,
        query: &str,
    ) -> Result<PlayOutcome, String> {
        let queue_full = || format!("The queue is full ({MAX_UPCOMING} tracks).");
        if self.with_player(guild, |p| p.queue.is_full()) == Some(true) {
            return Err(queue_full());
        }
        let _flight = self.enter_flight(guild);
        // Join the voice channel while the track resolves; both take a second or more.
        let resolve = self.resolver.resolve(query);
        tokio::pin!(resolve);
        let (resolved, joined) = match tokio::time::timeout(JOIN_DELAY, &mut resolve).await {
            Ok(Err(e)) => return Err(e),
            Ok(Ok(track)) => (Ok(track), self.join_if_needed(guild, channel).await),
            Err(_) => tokio::join!(resolve, self.join_if_needed(guild, channel)),
        };
        let mut track = match resolved {
            Ok(track) => track,
            Err(e) => {
                self.leave_if_unused(guild).await;
                return Err(e);
            }
        };
        joined?;
        track.requester = Some(user);

        let start_now = {
            let mut players = self.players();
            let player = players.entry(guild).or_default();
            if player.queue.is_full() {
                return Err(queue_full());
            }
            player.text_channel = Some(text_channel);
            player.queue.enqueue(track.clone())
        };
        self.flight(guild, |f| f.joined = false);
        if start_now {
            self.play_current(guild, Start::First).await;
        } else {
            self.queue_changed(guild);
        }
        Ok(PlayOutcome { queued: !start_now, track })
    }

    fn enter_flight(&self, guild: GuildId) -> InFlight<'_> {
        self.flight(guild, |f| f.count += 1);
        InFlight { bot: self, guild }
    }

    fn flight<R>(&self, guild: GuildId, f: impl FnOnce(&mut Flight) -> R) -> R {
        f(self.in_flight.lock().unwrap_or_else(|e| e.into_inner()).entry(guild).or_default())
    }

    fn join_lock(&self, guild: GuildId) -> Arc<tokio::sync::Mutex<()>> {
        self.join_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(guild).or_default().clone()
    }

    /// Joins the voice channel unless the bot already has a player in the guild. Returns whether
    /// it joined.
    async fn join_if_needed(self: &Arc<Self>, guild: GuildId, channel: ChannelId) -> Result<bool, String> {
        let lock = self.join_lock(guild);
        let _joining = lock.lock().await;
        if self.has_player(guild) {
            return Ok(false);
        }
        self.join(guild, channel).await?;
        self.flight(guild, |f| f.joined = true);
        Ok(true)
    }

    /// After a failed lookup, leaves the voice channel if a play command joined it and nothing
    /// came of it: no other play command is still running in the guild and nothing is queued.
    async fn leave_if_unused(&self, guild: GuildId) {
        // Held while leaving, so another play command cannot use the player being removed.
        let lock = self.join_lock(guild);
        let _joining = lock.lock().await;
        let unused = self.flight(guild, |f| f.count == 1 && f.joined);
        if unused && self.with_player(guild, |p| p.queue.is_idle()) == Some(true) {
            self.stop(guild).await;
        }
    }

    /// Refreshes, in the background, a next track that a queue change made stale, as a track
    /// start does.
    pub fn queue_changed(self: &Arc<Self>, guild: GuildId) {
        let playing_for =
            self.with_player(guild, |p| p.handle.as_ref().and(p.queue.current.as_ref()).map(|t| t.duration)).flatten();
        if let Some(playing_for) = playing_for {
            self.prefetch_next(guild, playing_for);
        }
    }

    async fn join(self: &Arc<Self>, guild: GuildId, channel: ChannelId) -> Result<(), String> {
        let call = match self.songbird.join(guild, channel).await {
            Ok(call) => call,
            Err(e) => {
                // songbird keeps the Call after a failed join; drop it so the bot does not
                // stay in the channel without a player if the connection completes later.
                let _ = self.songbird.remove(guild).await;
                return Err(format!("Could not join the voice channel: {e}"));
            }
        };
        {
            let mut call = call.lock().await;
            if let Err(e) = call.deafen(true).await {
                tracing::warn!("could not self-deafen in {guild}: {e}");
            }
            call.remove_all_global_events();
            call.add_global_event(Event::Core(CoreEvent::DriverDisconnect), Disconnected { bot: self.clone(), guild });
        }
        self.players().entry(guild).or_default();
        Ok(())
    }

    /// Starts the queue's current track, or arms the idle timer when there is none.
    /// A retry forces a new stream URL even when the stored one is still young.
    async fn play_current(self: &Arc<Self>, guild: GuildId, start: Start) {
        let refresh = start == Start::Retry;
        let next = self.with_player(guild, |p| {
            p.cancel_idle();
            p.paused = false;
            p.generation += 1;
            p.queue.current.clone().map(|t| (t, p.generation))
        });
        let (track, generation) = match next {
            None => return,
            Some(None) => return self.arm_idle(guild),
            Some(Some(next)) => next,
        };
        let track = if refresh || track.resolved_at.elapsed() > STREAM_MAX_AGE {
            match self.resolver.refresh(&track).await {
                Ok(fresh) => {
                    let current = self.with_player(guild, |p| {
                        let current = p.generation == generation;
                        if current {
                            p.queue.current = Some(fresh.clone());
                        }
                        current
                    });
                    if current != Some(true) {
                        return;
                    }
                    fresh
                }
                Err(e) => {
                    tracing::warn!("could not refresh the stream URL for {}: {e}", track.url);
                    track
                }
            }
        } else {
            track
        };
        let Some(call) = self.songbird.get(guild) else {
            self.stop(guild).await;
            return;
        };

        let handle = call.lock().await.play_only_input(self.input_for(&track));
        for (event, failed) in [(TrackEvent::End, false), (TrackEvent::Error, true)] {
            let _ = handle.add_event(Event::Track(event), TrackEnded { bot: self.clone(), guild, generation, failed });
        }
        let announce = self
            .with_player(guild, |p| {
                let current = p.generation == generation;
                if current {
                    p.handle = Some(handle);
                }
                current.then_some((p.text_channel, p.queue.repeat))
            })
            .flatten();
        let Some((text_channel, repeat)) = announce else { return };
        self.resolver.worker.touch();
        self.prefetch_next(guild, track.duration);
        if !matches!(start, Start::First | Start::Next) {
            return;
        }
        if let Some(stats) = &self.stats {
            stats.record(Play {
                guild_id: guild.to_string(),
                user_id: track.requester.map_or_else(|| "system".into(), |u| u.to_string()),
                title: track.title.clone(),
                artist: track.author.clone(),
                duration: track.duration,
            });
        }
        if start == Start::Next
            && let (Some(http), Some(channel)) = (self.discord.get().cloned(), text_channel)
        {
            let (e, rows) = embed::now_playing(&track, false, repeat, Some(track.platform));
            let msg = CreateMessage::new().embed(e).components(rows).allowed_mentions(CreateAllowedMentions::new());
            tokio::spawn(async move {
                if let Err(e) = channel.send_message(&http, msg).await {
                    tracing::debug!("could not announce the next track in {guild}: {e}");
                }
            });
        }
    }

    /// Refreshes, in the background, the stream URLs of the tracks that can play after the current
    /// one (the next in the queue, and the current one again in track repeat) when they would be
    /// too old by then, so a skip or the next song does not wait for yt-dlp.
    pub fn prefetch_next(self: &Arc<Self>, guild: GuildId, playing_for: u64) -> Option<JoinHandle<()>> {
        let ahead = Duration::from_secs(playing_for) + PREFETCH_SLACK;
        let stale: Vec<Track> = self.with_player(guild, |p| {
            let q = &p.queue;
            let next = q.upcoming.front().or(q.current.as_ref().filter(|_| q.repeat != RepeatMode::Off));
            let again = q.current.as_ref().filter(|_| q.repeat == RepeatMode::Track);
            let mut stale: Vec<Track> =
                next.into_iter().chain(again).filter(|t| t.resolved_at.elapsed() + ahead > STREAM_MAX_AGE).cloned().collect();
            stale.dedup_by(|a, b| a.url == b.url && a.resolved_at == b.resolved_at);
            stale
        })?;
        if stale.is_empty() {
            return None;
        }
        let bot = self.clone();
        Some(tokio::spawn(async move {
            for old in stale {
                match bot.resolver.refresh(&old).await {
                    Ok(fresh) => {
                        bot.with_player(guild, |p| {
                            for slot in p.queue.current.iter_mut().chain(p.queue.upcoming.iter_mut()) {
                                if slot.url == old.url && slot.resolved_at == old.resolved_at {
                                    slot.set_stream(&fresh);
                                }
                            }
                        });
                    }
                    Err(e) => tracing::debug!("could not prefetch the stream URL for {}: {e}", old.url),
                }
            }
        }))
    }

    pub fn input_for(&self, track: &Track) -> Input {
        let headers: HeaderMap = track
            .http_headers
            .iter()
            .filter_map(|(k, v)| Some((HeaderName::from_bytes(k.as_bytes()).ok()?, HeaderValue::from_str(v).ok()?)))
            .collect();
        if track.protocol.as_deref().is_some_and(|p| p.starts_with("m3u8")) {
            // Live streams and many HLS sources use MPEG-TS segments, which symphonia cannot
            // demux, so ffmpeg decodes HLS to raw PCM, as the old bot did for every stream.
            match spawn_ffmpeg(track) {
                Ok(child) => {
                    let source = ReadOnlySource::new(ChildContainer::from(child));
                    return RawAdapter::new(source, 48_000, 2).into();
                }
                Err(e) => tracing::warn!("could not start ffmpeg ({e}); opening the HLS stream directly"),
            }
            HlsRequest::new_with_headers(self.http.clone(), track.stream_url.clone(), headers).into()
        } else {
            HttpRequest { client: self.http.clone(), request: track.stream_url.clone(), headers, content_length: track.filesize }
                .into()
        }
    }

    async fn on_track_end(self: &Arc<Self>, guild: GuildId, generation: u64, failed: bool) {
        let proceed = self.with_player(guild, |p| {
            if p.generation != generation {
                return None;
            }
            if failed && p.seeking {
                // seek() sees the same failure and restarts the track at the target.
                return None;
            }
            p.generation += 1;
            p.handle = None;
            // The first failure is often an expired stream URL: retry once with a fresh one.
            let retry = failed && !p.retried;
            p.retried = retry;
            if !retry {
                if failed {
                    p.queue.skip_failed();
                } else if std::mem::take(&mut p.skip_requested) {
                    p.queue.skip();
                } else {
                    p.queue.advance();
                }
            }
            Some(retry)
        });
        if let Some(Some(retry)) = proceed {
            self.play_current(guild, if retry { Start::Retry } else { Start::Next }).await;
        }
    }

    fn arm_idle(self: &Arc<Self>, guild: GuildId) {
        let bot = self.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep(IDLE_LEAVE).await;
            // Take our own handle out first: stop() aborts `idle_task`, which would cancel this
            // task before it finishes leaving the channel.
            let idle = bot.with_player(guild, |p| {
                let idle = p.queue.is_idle();
                if idle {
                    p.idle_task.take();
                }
                idle
            });
            if idle == Some(true) {
                tracing::info!("leaving {guild} after idle timeout");
                bot.stop(guild).await;
            }
        });
        if let Some(old) = self.with_player(guild, |p| p.idle_task.replace(task)).flatten() {
            old.abort();
        }
    }

    /// Clears the queue, stops playback and leaves the voice channel. Returns whether a player
    /// existed; the voice channel is left either way.
    pub async fn stop(&self, guild: GuildId) -> bool {
        let removed = self.players().remove(&guild);
        let had_player = removed.is_some();
        if let Some(mut player) = removed {
            player.cancel_idle();
            if let Some(h) = player.handle.take() {
                let _ = h.stop();
            }
        }
        if self.songbird.get(guild).is_some()
            && let Err(e) = self.songbird.remove(guild).await
        {
            tracing::debug!("leave {guild}: {e}");
        }
        had_player
    }

    /// Stops the current track; the end event then starts the next one. Returns the skipped track,
    /// or `None` when nothing was playing.
    pub fn skip(&self, guild: GuildId) -> Option<Track> {
        self.with_player(guild, |p| {
            let stopped = p.handle.as_ref().is_some_and(|h| h.stop().is_ok());
            p.skip_requested = stopped;
            if stopped { p.queue.current.clone() } else { None }
        })
        .flatten()
    }

    /// Pauses or resumes the current track. The error says why nothing changed.
    pub fn set_paused(&self, guild: GuildId, paused: bool) -> Result<(), &'static str> {
        self.with_player(guild, |p| {
            if p.paused == paused {
                return Err(if paused { "Playback is already paused." } else { "Playback is not paused." });
            }
            let ok = match &p.handle {
                Some(h) if paused => h.pause().is_ok(),
                Some(h) => h.play().is_ok(),
                None => false,
            };
            if !ok {
                return Err("Nothing is playing.");
            }
            p.paused = paused;
            Ok(())
        })
        .unwrap_or(Err("Nothing is playing."))
    }

    pub async fn seek(self: &Arc<Self>, guild: GuildId, seconds: u64) -> bool {
        let Some(Some((handle, generation))) = self.with_player(guild, |p| {
            p.seeking = true;
            p.handle.clone().map(|h| (h, p.generation))
        }) else {
            self.with_player(guild, |p| p.seeking = false);
            return false;
        };
        let result = handle.seek_async(Duration::from_secs(seconds)).await;
        let restart = self
            .with_player(guild, |p| {
                p.seeking = false;
                let restart = result.is_err() && p.generation == generation;
                if restart {
                    // The stream cannot seek backwards inside the data already read, and songbird
                    // has dropped the input. Ignore its error event and start the track again.
                    p.generation += 1;
                    p.handle = None;
                }
                restart
            })
            .unwrap_or(false);
        let Err(e) = result else { return true };
        tracing::warn!("seek failed in {guild}: {e}");
        if !restart {
            return false;
        }
        // A fresh stream only seeks forward from the start, which songbird supports.
        self.play_current(guild, Start::SeekRestart).await;
        let Some(Some(handle)) = self.with_player(guild, |p| p.handle.clone()) else { return false };
        match handle.seek_async(Duration::from_secs(seconds)).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("seek after restart failed in {guild}: {e}");
                false
            }
        }
    }
}

struct TrackEnded {
    bot: Arc<Bot>,
    guild: GuildId,
    generation: u64,
    failed: bool,
}

/// Starts ffmpeg reading the stream and writing 48 kHz stereo f32 PCM to stdout.
/// `FFMPEG_PATH` overrides the program, which is otherwise `ffmpeg` on PATH.
fn spawn_ffmpeg(track: &Track) -> std::io::Result<std::process::Child> {
    let program = std::env::var("FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".into());
    let mut cmd = std::process::Command::new(program);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]);
    // Only network protocols, so a playlist cannot point ffmpeg at local files.
    cmd.args(["-protocol_whitelist", "http,https,tls,tcp,crypto"]);
    let mut extra = String::new();
    for (k, v) in &track.http_headers {
        if k.eq_ignore_ascii_case("user-agent") {
            cmd.args(["-user_agent", v]);
        } else if !k.contains(['\r', '\n']) && !v.contains(['\r', '\n']) {
            extra.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    if !extra.is_empty() {
        cmd.args(["-headers", &extra]);
    }
    cmd.args(["-i", &track.stream_url, "-vn", "-threads", "1", "-f", "f32le", "-ar", "48000", "-ac", "2", "pipe:1"]);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    cmd.spawn()
}

#[async_trait]
impl EventHandler for TrackEnded {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if self.failed
            && let EventContext::Track(list) = ctx
        {
            for (state, _) in *list {
                tracing::warn!("track failed in {}: {:?}", self.guild, state.playing);
            }
        }
        self.bot.on_track_end(self.guild, self.generation, self.failed).await;
        Some(Event::Cancel)
    }
}

struct Disconnected {
    bot: Arc<Bot>,
    guild: GuildId,
}

#[async_trait]
impl EventHandler for Disconnected {
    async fn act(&self, _ctx: &EventContext<'_>) -> Option<Event> {
        let (bot, guild) = (self.bot.clone(), self.guild);
        // Give songbird a few seconds to reconnect (for example after a channel move).
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let connected = match bot.songbird.get(guild) {
                Some(call) => call.lock().await.current_channel().is_some(),
                None => false,
            };
            if !connected && bot.stop(guild).await {
                tracing::info!("voice connection lost in {guild}, player stopped");
            }
        });
        None
    }
}
