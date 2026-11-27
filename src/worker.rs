//! A long-running Python process that imports yt-dlp once and answers extraction requests.
//!
//! Starting yt-dlp costs about two seconds of Python startup on a small VM, on every song. The
//! worker (`ytdlp_worker.py`, embedded in the binary) pays that once. It is started in the
//! background on first use, stopped after `IDLE_STOP` without requests or playback, and restarted
//! after a crash or when the yt-dlp file changes. When Python cannot import yt-dlp (for example a
//! standalone yt-dlp binary), the resolver keeps spawning yt-dlp.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Notify, Semaphore, oneshot};

const SCRIPT: &str = include_str!("ytdlp_worker.py");
/// Python startup plus the yt-dlp import; the first start also unpacks the zipapp.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a request waits for a starting worker before it spawns yt-dlp itself.
const START_WAIT: Duration = Duration::from_secs(3);
/// Stop the worker after this long without requests or playback, to give its memory back.
const IDLE_STOP: Duration = Duration::from_secs(15 * 60);
/// After this many failed starts or crashes in a row, use the spawn path for a while.
const MAX_FAILURES: u32 = 3;
const BACKOFF: Duration = Duration::from_secs(10 * 60);
/// A failed start is not tried again before this.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// Python threads in the worker (`THREADS` in the script); more requests wait in line here.
const THREADS: usize = 2;
/// A replaced worker may finish the requests it is running for this long, then it is stopped.
const RETIRE_WAIT: Duration = Duration::from_secs(60);

/// Why the worker could not answer; the caller then runs yt-dlp the old way.
#[derive(Debug)]
pub struct Unavailable;

#[derive(Deserialize)]
struct Reply {
    id: Option<u64>,
    #[serde(default)]
    ok: bool,
    data: Option<serde_json::Value>,
    error: Option<String>,
    ready: Option<bool>,
    version: Option<String>,
    routed: Option<bool>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>;
/// Size and modification time of the yt-dlp file; a change means yt-dlp was updated.
type Stamp = Option<(u64, SystemTime)>;

struct Proc {
    stdin: ChildStdin,
    pending: Pending,
    alive: Arc<AtomicBool>,
    last_used: Arc<Mutex<Instant>>,
    /// The yt-dlp file when this process started.
    stamp: Stamp,
    child: Child,
}

struct State {
    proc: Option<Proc>,
    /// A start is running in the background.
    starting: bool,
    failures: u32,
    /// The worker is not started before this time (`None`: no backoff).
    disabled_until: Option<Instant>,
}

pub struct Worker {
    python: String,
    ytdlp: String,
    /// The yt-dlp file, found on PATH for a bare name.
    ytdlp_path: Option<PathBuf>,
    /// yt-dlp options, without the target.
    args: Vec<String>,
    /// False when the worker cannot work here (`YTDLP_WORKER=0`, yt-dlp is not Python, no Python).
    usable: AtomicBool,
    /// yt-dlp's options, config files included, send requests through a proxy, another address
    /// or with cookies.
    routed: AtomicBool,
    state: tokio::sync::Mutex<State>,
    /// Woken when a start finishes, either way.
    started: Notify,
    /// One permit per Python thread, so a request's timeout only runs once it is being worked on.
    slots: Semaphore,
    next_id: AtomicU64,
    /// Whether music is playing; the worker is then kept even without new requests.
    playing: OnceLock<Box<dyn Fn() -> bool + Send + Sync>>,
}

impl Worker {
    pub fn new(ytdlp: &str, args: Vec<String>) -> Arc<Self> {
        let off = std::env::var("YTDLP_WORKER").is_ok_and(|v| matches!(v.trim(), "0" | "false" | "off" | "no"));
        let python = if cfg!(windows) { "python" } else { "python3" }.to_string();
        let ytdlp_path = find_program(ytdlp);
        Arc::new(Self {
            python,
            ytdlp: ytdlp.to_string(),
            usable: AtomicBool::new(!off && ytdlp_path.as_deref().is_some_and(is_python_ytdlp)),
            ytdlp_path,
            args,
            routed: AtomicBool::new(false),
            state: tokio::sync::Mutex::new(State { proc: None, starting: false, failures: 0, disabled_until: None }),
            started: Notify::new(),
            slots: Semaphore::new(THREADS),
            next_id: AtomicU64::new(1),
            playing: OnceLock::new(),
        })
    }

    pub fn is_usable(&self) -> bool {
        self.usable.load(Ordering::Relaxed)
    }

    /// Whether yt-dlp's options route requests through a proxy, another address or cookies, so
    /// other requests to YouTube should go through yt-dlp too. Known once the worker has started.
    pub fn is_routed(&self) -> bool {
        self.routed.load(Ordering::Relaxed)
    }

    /// Tells the worker how to see that music is playing, so it is not stopped meanwhile.
    pub fn keep_while(&self, playing: impl Fn() -> bool + Send + Sync + 'static) {
        let _ = self.playing.set(Box::new(playing));
    }

    /// Starts the worker in the background so the first song does not wait for it.
    pub fn prewarm(self: &Arc<Self>) {
        if !self.is_usable() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let mut state = me.state.lock().await;
            if !me.live(&mut state) {
                me.begin_start(&mut state);
            }
        });
    }

    /// Counts as activity, so a worker is kept while music plays even without new requests.
    pub fn touch(&self) {
        if let Ok(state) = self.state.try_lock()
            && let Some(p) = &state.proc
        {
            *p.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        }
    }

    /// Process id of the running worker, for the self-test's memory report.
    pub async fn pid(&self) -> Option<u32> {
        self.state.lock().await.proc.as_ref().and_then(|p| p.child.id())
    }

    /// Runs yt-dlp on `target`. The inner result is yt-dlp's answer (the JSON info, or the error
    /// line); `Err(Unavailable)` means the worker could not be used and nothing was decided.
    pub async fn extract(self: &Arc<Self>, target: &str, timeout: Duration) -> Result<Result<serde_json::Value, String>, Unavailable> {
        if !self.is_usable() {
            return Err(Unavailable);
        }
        self.wait_ready().await?;
        // Wait in line for a free Python thread without a time limit, like the spawn path's gate.
        let _slot = self.slots.acquire().await.map_err(|_| Unavailable)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let pending = {
            let mut state = self.state.lock().await;
            if !self.live(&mut state) {
                return Err(Unavailable);
            }
            let p = state.proc.as_mut().ok_or(Unavailable)?;
            *p.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            p.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
            let mut line = serde_json::to_vec(&serde_json::json!({ "id": id, "target": target })).map_err(|_| Unavailable)?;
            line.push(b'\n');
            if p.stdin.write_all(&line).await.is_err() || p.stdin.flush().await.is_err() {
                tracing::warn!("yt-dlp worker stopped accepting requests; restarting it");
                state.proc = None;
                self.begin_start(&mut state);
                return Err(Unavailable);
            }
            p.pending.clone()
        };
        let reply = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => return Err(Unavailable), // the process died; the reader dropped the sender
            Err(_) => {
                pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                // A stuck extraction holds one of the Python threads. Send new requests to a
                // fresh process, and let the old one finish what its other thread is doing.
                let mut state = self.state.lock().await;
                if state.proc.as_ref().is_some_and(|p| Arc::ptr_eq(&p.pending, &pending)) {
                    retire(state.proc.take());
                    self.begin_start(&mut state);
                }
                return Ok(Err("yt-dlp took too long to answer.".into()));
            }
        };
        self.state.lock().await.failures = 0;
        Ok(if reply.ok {
            Ok(reply.data.unwrap_or(serde_json::Value::Null))
        } else {
            Err(reply.error.unwrap_or_else(|| "yt-dlp failed".into()))
        })
    }

    /// Waits up to `START_WAIT` for a live process, starting one in the background when needed.
    async fn wait_ready(self: &Arc<Self>) -> Result<(), Unavailable> {
        let started = {
            let mut state = self.state.lock().await;
            if self.live(&mut state) {
                return Ok(());
            }
            if !self.begin_start(&mut state) {
                return Err(Unavailable);
            }
            // Created before the lock is released, so the wakeup cannot be missed.
            self.started.notified()
        };
        let _ = tokio::time::timeout(START_WAIT, started).await;
        let mut state = self.state.lock().await;
        if self.live(&mut state) { Ok(()) } else { Err(Unavailable) }
    }

    /// Whether `state` holds a usable process. Drops one that crashed, and replaces one whose
    /// yt-dlp file has changed since it started (after `yt-dlp -U`).
    fn live(&self, state: &mut State) -> bool {
        let Some(p) = &state.proc else { return false };
        if !p.alive.load(Ordering::Relaxed) {
            // It exited by itself, which only happens when it crashed.
            state.proc = None;
            state.failures += 1;
            return false;
        }
        if p.stamp != self.stamp() {
            tracing::info!("yt-dlp changed on disk; restarting the yt-dlp worker");
            retire(state.proc.take());
            return false;
        }
        true
    }

    /// Starts a process in the background unless one is starting already. Returns whether a start
    /// is running, so it is worth waiting for.
    fn begin_start(self: &Arc<Self>, state: &mut State) -> bool {
        if !self.is_usable() {
            return false;
        }
        if state.starting {
            return true;
        }
        if state.disabled_until.is_some_and(|t| Instant::now() < t) {
            return false;
        }
        state.disabled_until = None;
        if state.failures >= MAX_FAILURES {
            tracing::warn!("yt-dlp worker failed {} times; spawning yt-dlp per request for a while", state.failures);
            state.failures = 0;
            state.disabled_until = Some(Instant::now() + BACKOFF);
            return false;
        }
        state.starting = true;
        let me = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let result = me.start().await;
            let mut state = me.state.lock().await;
            state.starting = false;
            match result {
                Ok((proc, version, routed)) => {
                    tracing::info!("yt-dlp worker {version} ready in {} ms", started.elapsed().as_millis());
                    me.routed.store(routed, Ordering::Relaxed);
                    me.watch_idle(proc.alive.clone(), proc.last_used.clone(), proc.pending.clone());
                    state.proc = Some(proc);
                }
                Err(StartError::Never(e)) => {
                    tracing::info!("not using the yt-dlp worker: {e}");
                    me.usable.store(false, Ordering::Relaxed);
                }
                Err(StartError::Failed(e)) => {
                    tracing::warn!("yt-dlp worker did not start: {e}");
                    state.failures += 1;
                    state.disabled_until = Some(Instant::now() + RETRY_AFTER);
                }
            }
            drop(state);
            me.started.notify_waiters();
        });
        true
    }

    fn stamp(&self) -> Stamp {
        let meta = std::fs::metadata(self.ytdlp_path.as_ref()?).ok()?;
        Some((meta.len(), meta.modified().ok()?))
    }

    async fn start(&self) -> Result<(Proc, String, bool), StartError> {
        let dir = cache_dir();
        std::fs::create_dir_all(&dir).map_err(|e| StartError::Never(format!("cannot create {}: {e}", dir.display())))?;
        let script = dir.join("ytdlp_worker.py");
        if std::fs::read_to_string(&script).ok().as_deref() != Some(SCRIPT) {
            // Written aside and renamed, so a process starting at the same time never reads half a file.
            let tmp = dir.join(format!("ytdlp_worker.py.{}.tmp", std::process::id()));
            std::fs::write(&tmp, SCRIPT)
                .and_then(|()| std::fs::rename(&tmp, &script))
                .map_err(|e| StartError::Never(format!("cannot write {}: {e}", script.display())))?;
        }
        let stamp = self.stamp();
        let args_json = serde_json::to_string(&self.args).unwrap_or_else(|_| "[]".into());
        // Lowest CPU priority on Unix, like the per-request yt-dlp, so audio never stutters.
        let mut cmd = if cfg!(unix) {
            let mut c = tokio::process::Command::new("nice");
            c.args(["-n", "19", &self.python]);
            c
        } else {
            tokio::process::Command::new(&self.python)
        };
        cmd.arg(&script)
            .arg(&self.ytdlp)
            .arg(&dir)
            .arg(args_json)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StartError::Never(format!("{} not found", self.python)),
            _ => StartError::Failed(format!("could not start {}: {e}", self.python)),
        })?;
        let stdin = child.stdin.take().ok_or_else(|| StartError::Failed("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| StartError::Failed("no stdout".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let first = match tokio::time::timeout(READY_TIMEOUT, lines.next_line()).await {
            Err(_) => return Err(StartError::Failed("no answer from Python".into())),
            Ok(Ok(Some(line))) => line,
            Ok(_) => {
                // Python exited before it was ready. Only a missing interpreter is permanent; a
                // killed process (out of memory) or a script read mid-write may work next time.
                let status = tokio::time::timeout(Duration::from_secs(2), child.wait()).await.ok().and_then(Result::ok);
                if status.and_then(|s| s.code()) == Some(127) {
                    return Err(StartError::Never(format!("{} not found", self.python)));
                }
                let status = status.map_or_else(|| "unknown status".to_string(), |s| s.to_string());
                return Err(StartError::Failed(format!("{} exited during startup ({status})", self.python)));
            }
        };
        let hello: Reply = serde_json::from_str(&first).map_err(|_| StartError::Failed("unexpected startup output".into()))?;
        if hello.ready != Some(true) {
            // The script reports that yt-dlp cannot be imported or rejects its options.
            return Err(StartError::Never(hello.error.unwrap_or_else(|| "yt-dlp could not be imported".into())));
        }

        let pending: Pending = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let (pending2, alive2) = (pending.clone(), alive.clone());
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(reply) = serde_json::from_str::<Reply>(&line) else { continue };
                let tx = reply.id.and_then(|id| pending2.lock().unwrap_or_else(|e| e.into_inner()).remove(&id));
                if let Some(tx) = tx {
                    let _ = tx.send(reply);
                }
            }
            alive2.store(false, Ordering::Relaxed);
            // Dropping the senders fails every request still waiting.
            pending2.lock().unwrap_or_else(|e| e.into_inner()).clear();
        });
        let proc = Proc { stdin, pending, alive, last_used: Arc::new(Mutex::new(Instant::now())), stamp, child };
        Ok((proc, hello.version.unwrap_or_default(), hello.routed.unwrap_or(false)))
    }

    /// Stops the worker once it has been idle for `IDLE_STOP` with no music playing.
    fn watch_idle(self: &Arc<Self>, alive: Arc<AtomicBool>, last_used: Arc<Mutex<Instant>>, pending: Pending) {
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(me) = me.upgrade() else { return };
                if !alive.load(Ordering::Relaxed) {
                    return;
                }
                if me.playing.get().is_some_and(|playing| playing()) {
                    *last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                    continue;
                }
                let idle = last_used.lock().unwrap_or_else(|e| e.into_inner()).elapsed() > IDLE_STOP;
                if !idle || !pending.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                    continue;
                }
                let mut state = me.state.lock().await;
                if state.proc.as_ref().is_some_and(|p| Arc::ptr_eq(&p.pending, &pending)) {
                    tracing::info!("stopping the idle yt-dlp worker");
                    state.proc = None;
                }
                return;
            }
        });
    }
}

/// Lets a replaced process finish the requests it is running, then stops it (`kill_on_drop`).
fn retire(proc: Option<Proc>) {
    let Some(proc) = proc else { return };
    tokio::spawn(async move {
        let deadline = Instant::now() + RETIRE_WAIT;
        while Instant::now() < deadline
            && proc.alive.load(Ordering::Relaxed)
            && !proc.pending.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
        {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        drop(proc);
    });
}

enum StartError {
    /// This machine cannot run the worker; do not try again.
    Never(String),
    /// Might work next time.
    Failed(String),
}

/// `$XDG_CACHE_HOME/lumi-discord-bot`, `~/.cache/lumi-discord-bot`, or a temp directory.
fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("lumi-discord-bot")
}

/// The file `program` runs: the path itself, or for a bare name the first match on PATH.
fn find_program(program: &str) -> Option<PathBuf> {
    if Path::new(program).components().count() > 1 {
        Some(PathBuf::from(program))
    } else {
        std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(program)).find(|p| p.is_file()))
    }
}

/// Whether `path` is Python yt-dlp: the zipapp release, or a script with a Python shebang.
/// Standalone yt-dlp binaries cannot be imported.
fn is_python_ytdlp(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 256];
    let n = file.read(&mut head).unwrap_or(0);
    let head = &head[..n];
    let first_line = head.split(|&b| b == b'\n').next().unwrap_or_default();
    head.starts_with(b"PK\x03\x04") || (first_line.starts_with(b"#!") && first_line.windows(6).any(|w| w == b"python"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_python_ytdlp() {
        let dir = std::env::temp_dir().join(format!("lumi-worker-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases: [(&str, &[u8], bool); 4] = [
            ("zipapp", b"#!/usr/bin/env python3\nPK\x03\x04", true),
            ("script", b"#!/home/u/.venv/bin/python\nimport yt_dlp", true),
            ("zip", b"PK\x03\x04rest", true),
            ("binary", b"\x7fELF\x02\x01\x01", false),
        ];
        for (name, bytes, expected) in cases {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            assert_eq!(is_python_ytdlp(&p), expected, "{name}");
        }
        assert!(!is_python_ytdlp(&dir.join("missing")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
