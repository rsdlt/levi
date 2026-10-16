//! Background threads. Everything reports back to the UI through one `Msg` channel.

use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::crossterm::event;

use crate::app::{DiffRequest, LogRequest, Msg};
use crate::git;

/// Commits per message during the initial burst...
const BATCH_SIZE: usize = 512;
/// ...but never hold commits back longer than a frame.
const BATCH_INTERVAL: Duration = Duration::from_millis(16);

pub fn spawn_input(tx: Sender<Msg>) {
    thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(Msg::Input(ev)).is_err() {
                break;
            }
        }
    });
}

/// Loads the commit list in the background, one source at a time.
pub struct LogLoader {
    root: PathBuf,
    tx: Sender<Msg>,
    /// Generation of the newest load. Older loads see they're stale and stop.
    current: Arc<AtomicU64>,
}

impl LogLoader {
    pub fn new(root: PathBuf, tx: Sender<Msg>) -> LogLoader {
        LogLoader {
            root,
            tx,
            current: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Streams `req.source` in batches so the UI is usable before git finishes.
    /// Any load already running stops at its next commit, killing its git.
    pub fn start(&self, req: LogRequest) {
        let LogRequest { source, generation } = req;
        self.current.store(generation, Ordering::SeqCst);
        let (root, tx, current) = (self.root.clone(), self.tx.clone(), self.current.clone());
        thread::spawn(move || {
            let stream = match git::log(&root, source) {
                Ok(stream) => stream,
                Err(e) => {
                    let _ = tx.send(Msg::LogDone(generation, Err(e.to_string())));
                    return;
                }
            };
            let mut batch = Vec::new();
            let mut last_send = Instant::now();
            for item in stream {
                if current.load(Ordering::SeqCst) != generation {
                    return;
                }
                match item {
                    Ok(commit) => batch.push(commit),
                    Err(e) => {
                        let _ = tx.send(Msg::Commits(generation, batch));
                        let _ = tx.send(Msg::LogDone(generation, Err(e.to_string())));
                        return;
                    }
                }
                if batch.len() >= BATCH_SIZE || last_send.elapsed() >= BATCH_INTERVAL {
                    // A send error means the UI is gone; dropping the stream kills git.
                    if tx
                        .send(Msg::Commits(generation, mem::take(&mut batch)))
                        .is_err()
                    {
                        return;
                    }
                    last_send = Instant::now();
                }
            }
            let _ = tx.send(Msg::Commits(generation, batch));
            let _ = tx.send(Msg::LogDone(generation, Ok(())));
        });
    }
}

/// Runs `git show` off the UI thread. Requests that pile up while one is running
/// are skipped in favour of the newest, so scrolling fast never builds a queue.
pub fn spawn_diff(root: PathBuf, tx: Sender<Msg>) -> Sender<DiffRequest> {
    let (req_tx, req_rx) = mpsc::channel::<DiffRequest>();
    thread::spawn(move || {
        while let Ok(mut req) = req_rx.recv() {
            while let Ok(newer) = req_rx.try_recv() {
                req = newer;
            }
            let result = git::diff(&root, &req.sha, &req.file)
                .map(|text| git::parse_diff(&text))
                .map_err(|e| e.to_string());
            if tx.send(Msg::Diff(req.key(), result)).is_err() {
                break;
            }
        }
    });
    req_tx
}
