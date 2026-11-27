//! Per-guild playback state on top of songbird.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serenity::all::{ChannelId, GuildId, ShardManager};
use serenity::async_trait;
use songbird::events::{CoreEvent, Event, EventContext, EventHandler, TrackEvent};
use songbird::input::{HlsRequest, HttpRequest, Input};
use songbird::tracks::TrackHandle;
use songbird::Songbird;
use tokio::task::JoinHandle;

use crate::queue::{Queue, RepeatMode};
use crate::resolver::{Resolver, Track};

/// Leave after this long with nothing playing and nothing queued.
const IDLE_LEAVE: Duration = Duration::from_secs(180);
/// Stream URLs older than this are resolved again before playing. Googlevideo URLs last about
/// 6 h, SoundCloud HLS URLs much less.
const STREAM_MAX_AGE: Duration = Duration::from_secs(30 * 60);

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
    players: Mutex<HashMap<GuildId, GuildPlayer>>,
    /// One lock per guild so two play commands cannot join the same channel at once
    /// (songbird cancels an in-progress join when a second one starts).
    join_locks: Mutex<HashMap<GuildId, Arc<tokio::sync::Mutex<()>>>>,
}

pub struct PlayOutcome {
    pub queued: bool,
    pub track: Track,
    pub repeat: RepeatMode,
}

impl Bot {
    pub fn new(songbird: Arc<Songbird>, http: reqwest::Client) -> Self {
        Self {
            songbird,
            resolver: Resolver::new(http.clone()),
            http,
            started: Instant::now(),
            shard_manager: OnceLock::new(),
            players: Mutex::new(HashMap::new()),
            join_locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn players(&self) -> MutexGuard<'_, HashMap<GuildId, GuildPlayer>> {
        // A panic while holding this lock aborts the process (panic = "abort"), so poisoning
        // cannot be observed in release builds; recover the data in debug builds too.
        self.players.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn has_player(&self, guild: GuildId) -> bool {
        self.players().contains_key(&guild)
    }

    /// Runs `f` on the guild's player, if there is one.
    pub fn with_player<R>(&self, guild: GuildId, f: impl FnOnce(&mut GuildPlayer) -> R) -> Option<R> {
        self.players().get_mut(&guild).map(f)
    }

    /// Resolves the query, joins the voice channel if needed, and starts or queues the track.
    pub async fn play(self: &Arc<Self>, guild: GuildId, channel: ChannelId, query: &str) -> Result<PlayOutcome, String> {
        let track = self.resolver.resolve(query).await?;
        if track.protocol.as_deref().is_some_and(|p| p.starts_with("m3u8")) {
            self.probe(&track).await?;
        }

        if !self.has_player(guild) {
            let lock = self.join_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(guild).or_default().clone();
            let _joining = lock.lock().await;
            if !self.has_player(guild) {
                self.join(guild, channel).await?;
            }
        }

        let (start_now, repeat) = {
            let mut players = self.players();
            let player = players.entry(guild).or_default();
            (player.queue.enqueue(track.clone()), player.queue.repeat)
        };
        if start_now {
            self.play_current(guild, false).await;
        }
        Ok(PlayOutcome { queued: !start_now, track, repeat })
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
    /// `refresh` forces a new stream URL even when the stored one is still young.
    async fn play_current(self: &Arc<Self>, guild: GuildId, refresh: bool) {
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
        self.with_player(guild, |p| {
            if p.generation == generation {
                p.handle = Some(handle);
            }
        });
    }

    /// Opens an HLS stream and lets symphonia probe it, so formats it cannot demux (MPEG-TS
    /// segments) fail with a message here instead of being skipped silently after the reply.
    async fn probe(&self, track: &Track) -> Result<(), String> {
        use songbird::input::codecs::{get_codec_registry, get_probe};
        let probe = self.input_for(track).make_playable_async(get_codec_registry(), get_probe());
        match tokio::time::timeout(Duration::from_secs(15), probe).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                tracing::warn!("probe failed for {}: {e}", track.url);
                Err("This stream format is not supported (live streams and MPEG-TS HLS cannot be played).".into())
            }
            Err(_) => Err("The audio stream took too long to open.".into()),
        }
    }

    pub fn input_for(&self, track: &Track) -> Input {
        let headers: HeaderMap = track
            .http_headers
            .iter()
            .filter_map(|(k, v)| Some((HeaderName::from_bytes(k.as_bytes()).ok()?, HeaderValue::from_str(v).ok()?)))
            .collect();
        if track.protocol.as_deref().is_some_and(|p| p.starts_with("m3u8")) {
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
            self.play_current(guild, retry).await;
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
        self.play_current(guild, false).await;
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
