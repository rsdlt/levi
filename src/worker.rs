//! Background threads. Everything reports back to the UI through one `Msg` channel.

use std::collections::VecDeque;
use std::mem;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::crossterm::event;

use crate::app::{CodeRequest, DiffRequest, LogRequest, Msg};
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

/// Commits handed to one `git log -G`. Small enough that the first matches show up
/// quickly and a cancelled search stops within a fraction of a second.
const CODE_CHUNK: usize = 500;

/// Searches diffs on every core: the commit list is cut into chunks, newest first,
/// and one thread per core takes chunks off a shared queue.
pub struct CodeSearcher {
    root: PathBuf,
    tx: Sender<Msg>,
    /// Generation of the search that should be running; 0 stops them all.
    current: Arc<AtomicU64>,
    /// git processes searching right now, so cancelling can kill them mid-chunk.
    running: Arc<Mutex<Vec<Arc<Mutex<Child>>>>>,
}

impl CodeSearcher {
    pub fn new(root: PathBuf, tx: Sender<Msg>) -> CodeSearcher {
        CodeSearcher {
            root,
            tx,
            current: Arc::new(AtomicU64::new(0)),
            running: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Stops any running search now, killing its git processes.
    pub fn cancel(&self) {
        self.current.store(0, Ordering::SeqCst);
        for process in self.running.lock().expect("running lock").drain(..) {
            if let Ok(mut child) = process.lock() {
                let _ = child.kill();
            }
        }
    }

    pub fn start(&self, req: CodeRequest) {
        let CodeRequest {
            pattern,
            shas,
            generation,
        } = req;
        self.cancel();
        self.current.store(generation, Ordering::SeqCst);
        let chunks: VecDeque<Vec<String>> = shas.chunks(CODE_CHUNK).map(<[_]>::to_vec).collect();
        let total = chunks.len();
        let queue = Arc::new(Mutex::new(chunks));
        let finished = Arc::new(AtomicUsize::new(0));
        let pattern = Arc::new(pattern);
        if total == 0 {
            let _ = self.tx.send(Msg::CodeDone(generation, Ok(())));
            return;
        }
        let threads = thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(total);
        for _ in 0..threads {
            let (root, tx, current) = (self.root.clone(), self.tx.clone(), self.current.clone());
            let (queue, finished, pattern) = (queue.clone(), finished.clone(), pattern.clone());
            let running = self.running.clone();
            thread::spawn(move || {
                let live = || current.load(Ordering::SeqCst) == generation;
                loop {
                    let Some(chunk) = queue.lock().expect("queue lock").pop_front() else {
                        return;
                    };
                    if !live() {
                        return;
                    }
                    let stream = match git::code_search(&root, &pattern, &chunk) {
                        Ok(stream) => stream,
                        Err(e) => {
                            let _ = tx.send(Msg::CodeDone(generation, Err(e.to_string())));
                            return;
                        }
                    };
                    let process = stream.process();
                    running.lock().expect("running lock").push(process.clone());
                    // Registered before checking, so a cancel can't slip in between.
                    if !live() {
                        return;
                    }
                    for hit in stream {
                        // Dropping the stream kills git.
                        if !live() || tx.send(Msg::CodeHits(generation, vec![hit])).is_err() {
                            return;
                        }
                    }
                    running
                        .lock()
                        .expect("running lock")
                        .retain(|p| !Arc::ptr_eq(p, &process));
                    let done = finished.fetch_add(1, Ordering::SeqCst) + 1;
                    let _ = tx.send(Msg::CodeProgress(generation, done, total));
                    if done == total {
                        let _ = tx.send(Msg::CodeDone(generation, Ok(())));
                    }
                }
            });
        }
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
