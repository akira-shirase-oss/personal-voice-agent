//! Content-free logging (D-011). A field is a number, a boolean or a `&'static str`, so a
//! transcript, prompt, reply or tool result cannot be handed to a log line: the types refuse
//! it. Lines go to stderr through a bounded queue drained by its own thread; when the app
//! stops reading stderr, lines are dropped and counted instead of blocking the session.

use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

const QUEUE: usize = 1_024;

#[derive(Clone, Copy, Debug)]
pub enum Field {
    U(u64),
    B(bool),
    S(&'static str),
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U(v) => write!(f, "{v}"),
            Self::B(v) => write!(f, "{v}"),
            Self::S(v) => f.write_str(v),
        }
    }
}

impl From<u64> for Field {
    fn from(v: u64) -> Self {
        Self::U(v)
    }
}
impl From<u32> for Field {
    fn from(v: u32) -> Self {
        Self::U(u64::from(v))
    }
}
impl From<bool> for Field {
    fn from(v: bool) -> Self {
        Self::B(v)
    }
}
impl From<&'static str> for Field {
    fn from(v: &'static str) -> Self {
        Self::S(v)
    }
}

#[derive(Clone)]
pub struct Log {
    inner: Arc<Inner>,
}

struct Inner {
    stderr: Option<SyncSender<String>>,
    memory: Option<Mutex<Vec<u8>>>,
    dropped: AtomicU64,
}

impl Log {
    /// Lines to stderr through a bounded queue.
    pub fn stderr() -> Self {
        let (tx, rx) = sync_channel::<String>(QUEUE);
        let spawned = std::thread::Builder::new().name("log".into()).spawn(move || {
            let mut err = std::io::stderr();
            while let Ok(line) = rx.recv() {
                if err.write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
        });
        Self {
            inner: Arc::new(Inner {
                stderr: spawned.is_ok().then_some(tx),
                memory: None,
                dropped: AtomicU64::new(0),
            }),
        }
    }

    /// Lines kept in memory, for tests that inspect what was logged.
    pub fn memory() -> Self {
        Self {
            inner: Arc::new(Inner {
                stderr: None,
                memory: Some(Mutex::new(Vec::new())),
                dropped: AtomicU64::new(0),
            }),
        }
    }

    pub fn off() -> Self {
        Self {
            inner: Arc::new(Inner {
                stderr: None,
                memory: None,
                dropped: AtomicU64::new(0),
            }),
        }
    }

    pub fn event(&self, name: &'static str, fields: &[(&'static str, Field)]) {
        let mut line = String::with_capacity(64);
        line.push_str("event=");
        line.push_str(name);
        for (key, value) in fields {
            line.push(' ');
            line.push_str(key);
            line.push('=');
            line.push_str(&value.to_string());
        }
        line.push('\n');
        if let Some(tx) = &self.inner.stderr
            && let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = tx.try_send(line.clone())
        {
            self.inner.dropped.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(memory) = &self.inner.memory
            && let Ok(mut buf) = memory.lock()
        {
            buf.extend_from_slice(line.as_bytes());
        }
    }

    /// Everything logged so far (memory sink only).
    pub fn captured(&self) -> String {
        self.inner
            .memory
            .as_ref()
            .and_then(|m| m.lock().ok().map(|b| String::from_utf8_lossy(&b).into_owned()))
            .unwrap_or_default()
    }

    pub fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }
}
