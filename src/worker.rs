//! A long-running Python process that imports yt-dlp once and answers extraction requests.
//!
//! Starting yt-dlp costs about two seconds of Python startup on a small VM, on every song. The
//! worker (`ytdlp_worker.py`, embedded in the binary) pays that once. It is started on first use,
//! stopped after `IDLE_STOP` without requests, and restarted after a crash. When Python cannot
//! import yt-dlp (for example a standalone yt-dlp binary), the resolver keeps spawning yt-dlp.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::oneshot;

const SCRIPT: &str = include_str!("ytdlp_worker.py");
/// Python startup plus the yt-dlp import; the first start also unpacks the zipapp.
const READY_TIMEOUT: Duration = Duration::from_secs(30);
/// Stop the worker after this long without requests, to give its memory back.
const IDLE_STOP: Duration = Duration::from_secs(15 * 60);
/// After this many failed starts or crashes in a row, use the spawn path for a while.
const MAX_FAILURES: u32 = 3;
const BACKOFF: Duration = Duration::from_secs(10 * 60);

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
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>;

struct Proc {
    stdin: ChildStdin,
    pending: Pending,
    alive: Arc<AtomicBool>,
    last_used: Arc<Mutex<Instant>>,
    child: Child,
}

struct State {
    proc: Option<Proc>,
    failures: u32,
    /// The worker is not used before this time (`None`: no backoff).
    disabled_until: Option<Instant>,
}

pub struct Worker {
    python: String,
    ytdlp: String,
    /// yt-dlp options, without the target.
    args: Vec<String>,
    /// False when the worker cannot work here (`YTDLP_WORKER=0`, yt-dlp is not Python, no Python).
    usable: AtomicBool,
    state: tokio::sync::Mutex<State>,
    next_id: AtomicU64,
}

impl Worker {
    pub fn new(ytdlp: &str, args: Vec<String>) -> Arc<Self> {
        let off = std::env::var("YTDLP_WORKER").is_ok_and(|v| matches!(v.trim(), "0" | "false" | "off" | "no"));
        let python = if cfg!(windows) { "python" } else { "python3" }.to_string();
        Arc::new(Self {
            python,
            ytdlp: ytdlp.to_string(),
            args,
            usable: AtomicBool::new(!off && is_python_ytdlp(ytdlp)),
            state: tokio::sync::Mutex::new(State { proc: None, failures: 0, disabled_until: None }),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn is_usable(&self) -> bool {
        self.usable.load(Ordering::Relaxed)
    }

    /// Starts the worker in the background so the first song does not wait for it.
    pub fn prewarm(self: &Arc<Self>) {
        if !self.is_usable() {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let mut state = me.state.lock().await;
            if let Err(Unavailable) = me.ensure(&mut state).await {
                tracing::debug!("yt-dlp worker did not start");
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
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let pending = {
            let mut state = self.state.lock().await;
            self.ensure(&mut state).await?;
            let p = state.proc.as_mut().ok_or(Unavailable)?;
            *p.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            p.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
            let mut line = serde_json::to_vec(&serde_json::json!({ "id": id, "target": target })).map_err(|_| Unavailable)?;
            line.push(b'\n');
            if p.stdin.write_all(&line).await.is_err() || p.stdin.flush().await.is_err() {
                tracing::warn!("yt-dlp worker stopped accepting requests; restarting it");
                state.proc = None;
                return Err(Unavailable);
            }
            p.pending.clone()
        };
        let reply = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => return Err(Unavailable), // the process died; the reader dropped the sender
            Err(_) => {
                pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                // A stuck extraction holds one of the two threads; start over with a fresh process.
                let mut state = self.state.lock().await;
                if state.proc.as_ref().is_some_and(|p| Arc::ptr_eq(&p.pending, &pending)) {
                    state.proc = None;
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

    /// Makes sure a live process is in `state`, starting one when needed.
    async fn ensure(self: &Arc<Self>, state: &mut State) -> Result<(), Unavailable> {
        if state.proc.as_ref().is_some_and(|p| p.alive.load(Ordering::Relaxed)) {
            return Ok(());
        }
        if state.proc.take().is_some() {
            // It exited by itself, which only happens when it crashed.
            state.failures += 1;
        }
        if state.disabled_until.is_some_and(|t| Instant::now() < t) {
            return Err(Unavailable);
        }
        if state.failures >= MAX_FAILURES {
            tracing::warn!("yt-dlp worker failed {} times; spawning yt-dlp per request for a while", state.failures);
            state.failures = 0;
            state.disabled_until = Some(Instant::now() + BACKOFF);
            return Err(Unavailable);
        }
        let started = Instant::now();
        match self.start().await {
            Ok((proc, version)) => {
                tracing::info!("yt-dlp worker {version} ready in {} ms", started.elapsed().as_millis());
                self.watch_idle(proc.alive.clone(), proc.last_used.clone(), proc.pending.clone());
                state.proc = Some(proc);
                Ok(())
            }
            Err(StartError::Never(e)) => {
                tracing::info!("not using the yt-dlp worker: {e}");
                self.usable.store(false, Ordering::Relaxed);
                Err(Unavailable)
            }
            Err(StartError::Failed(e)) => {
                tracing::warn!("yt-dlp worker did not start: {e}");
                state.failures += 1;
                Err(Unavailable)
            }
        }
    }

    async fn start(&self) -> Result<(Proc, String), StartError> {
        let dir = cache_dir();
        std::fs::create_dir_all(&dir).map_err(|e| StartError::Never(format!("cannot create {}: {e}", dir.display())))?;
        let script = dir.join("ytdlp_worker.py");
        if std::fs::read_to_string(&script).ok().as_deref() != Some(SCRIPT) {
            std::fs::write(&script, SCRIPT).map_err(|e| StartError::Never(format!("cannot write {}: {e}", script.display())))?;
        }
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
            Ok(_) => return Err(StartError::Never(format!("{} exited during startup", self.python))),
        };
        let hello: Reply = serde_json::from_str(&first).map_err(|_| StartError::Never("unexpected startup output".into()))?;
        if hello.ready != Some(true) {
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
        let proc = Proc { stdin, pending, alive, last_used: Arc::new(Mutex::new(Instant::now())), child };
        Ok((proc, hello.version.unwrap_or_default()))
    }

    /// Stops the worker once it has been idle for `IDLE_STOP`.
    fn watch_idle(self: &Arc<Self>, alive: Arc<AtomicBool>, last_used: Arc<Mutex<Instant>>, pending: Pending) {
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(me) = me.upgrade() else { return };
                if !alive.load(Ordering::Relaxed) {
                    return;
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

/// Whether `program` is Python yt-dlp: the zipapp release, or a script with a Python shebang.
/// A bare name is looked up on PATH. Standalone yt-dlp binaries cannot be imported.
fn is_python_ytdlp(program: &str) -> bool {
    let path = if Path::new(program).components().count() > 1 {
        Some(PathBuf::from(program))
    } else {
        std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(program)).find(|p| p.is_file()))
    };
    let Some(mut file) = path.and_then(|p| std::fs::File::open(p).ok()) else { return false };
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
            assert_eq!(is_python_ytdlp(p.to_str().unwrap()), expected, "{name}");
        }
        assert!(!is_python_ytdlp(dir.join("missing").to_str().unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
