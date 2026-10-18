//! Background threads. Everything reports back to the UI through one `Msg` channel.

use std::collections::{HashMap, VecDeque};
use std::mem;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, MouseButton, MouseEventKind};

use crate::app::{CodeRequest, CodeTarget, DiffRequest, LogRequest, Msg};
use crate::git::{self, Commit};
use crate::query::Source;

/// Commits per message during the initial burst...
const BATCH_SIZE: usize = 512;
/// ...but never hold commits back longer than a frame.
const BATCH_INTERVAL: Duration = Duration::from_millis(16);

/// The thread reading keys and the mouse, which can stand aside while another
/// program, like an editor, has the terminal.
pub struct Input {
    paused: Arc<AtomicBool>,
    /// Set by the thread once it has seen `paused` and stopped reading.
    parked: Arc<AtomicBool>,
}

impl Input {
    /// Stops reading the terminal, returning once nothing is being read, so
    /// whatever runs next gets every key.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        while !self.parked.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }
    }

    pub fn resume(&self) {
        // Cleared here, not by the thread, so a quick pause() after this
        // can't take a stale `parked` for the real thing.
        self.parked.store(false, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
    }
}

pub fn spawn_input(tx: Sender<Msg>) -> Input {
    let paused = Arc::new(AtomicBool::new(false));
    let parked = Arc::new(AtomicBool::new(false));
    let input = Input {
        paused: paused.clone(),
        parked: parked.clone(),
    };
    thread::spawn(move || {
        loop {
            if paused.load(Ordering::SeqCst) {
                parked.store(true, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            // Wait for input in short spells, to notice a pause in between.
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(_) => break,
            }
            let Ok(ev) = event::read() else {
                break;
            };
            // Mouse capture reports every movement; only clicks, the wheel and
            // left-button drags (to select) matter, and the rest would each
            // cost a redraw.
            if let Event::Mouse(m) = &ev
                && matches!(
                    m.kind,
                    MouseEventKind::Moved
                        | MouseEventKind::Drag(MouseButton::Right | MouseButton::Middle)
                )
            {
                continue;
            }
            if tx.send(Msg::Input(ev)).is_err() {
                break;
            }
        }
    });
    input
}

/// Loads the commit list in the background, one source at a time.
pub struct LogLoader {
    root: PathBuf,
    /// What the log shows: `HEAD`, or the revision levi was started with.
    rev: String,
    tx: Sender<Msg>,
    /// Generation of the newest load. Older loads see they're stale and stop.
    current: Arc<AtomicU64>,
}

impl LogLoader {
    pub fn new(root: PathBuf, rev: String, tx: Sender<Msg>) -> LogLoader {
        LogLoader {
            root,
            rev,
            tx,
            current: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Loads `req.source` in the background. Any load already running stops at
    /// its next commit or chunk, killing its git.
    pub fn start(&self, req: LogRequest) {
        let LogRequest { source, generation } = req;
        self.current.store(generation, Ordering::SeqCst);
        let (root, tx, current) = (self.root.clone(), self.tx.clone(), self.current.clone());
        let rev = self.rev.clone();
        let live = move || current.load(Ordering::SeqCst) == generation;
        thread::spawn(move || {
            let result = match source {
                Source::Log => {
                    load_parallel(&root, &rev, &tx, generation, live, FIRST_CHUNK, LOG_CHUNK)
                }
                // The reflog is short, and only a reflog walk knows each entry's action.
                // It's every place HEAD has been, so a revision doesn't narrow it.
                Source::Reflog => load_serial(&root, &tx, generation, live),
            };
            if let Some(result) = result {
                let _ = tx.send(Msg::LogDone(generation, result.map_err(|e| e.to_string())));
            }
        });
    }
}

/// Commits in the first chunk of a parallel load, kept small for a fast first frame.
const FIRST_CHUNK: usize = 256;
/// Commits per chunk after that.
const LOG_CHUNK: usize = 2048;

/// Streams one `git log` in batches. `None` if the load went stale.
fn load_serial(
    root: &Path,
    tx: &Sender<Msg>,
    generation: u64,
    live: impl Fn() -> bool,
) -> Option<Result<()>> {
    let stream = match git::log(root, Source::Reflog) {
        Ok(stream) => stream,
        Err(e) => return Some(Err(e)),
    };
    let mut batch = Vec::new();
    let mut last_send = Instant::now();
    for item in stream {
        if !live() {
            return None;
        }
        match item {
            Ok(commit) => batch.push(commit),
            Err(e) => {
                let _ = tx.send(Msg::Commits(generation, batch));
                return Some(Err(e));
            }
        }
        if batch.len() >= BATCH_SIZE || last_send.elapsed() >= BATCH_INTERVAL {
            // A send error means the UI is gone; dropping the stream kills git.
            tx.send(Msg::Commits(generation, mem::take(&mut batch)))
                .ok()?;
            last_send = Instant::now();
        }
    }
    tx.send(Msg::Commits(generation, batch)).ok()?;
    Some(Ok(()))
}

enum LogEvent {
    Chunk(usize, Result<Vec<Commit>>),
    /// The producer handed out this many chunks in total.
    Total(usize),
}

/// Loads `rev` on every core: a producer cuts `git rev-list` into chunks as it
/// streams, one thread per core runs `git log` on a chunk at a time, and this
/// thread forwards the results in log order. `None` if the load went stale.
fn load_parallel(
    root: &Path,
    rev: &str,
    tx: &Sender<Msg>,
    generation: u64,
    live: impl Fn() -> bool + Clone + Send + 'static,
    first_chunk: usize,
    chunk_size: usize,
) -> Option<Result<()>> {
    let (job_tx, job_rx) = mpsc::channel::<(usize, Vec<String>)>();
    let job_rx = Arc::new(Mutex::new(job_rx));
    let (event_tx, event_rx) = mpsc::channel::<LogEvent>();

    let threads = thread::available_parallelism().map_or(1, |n| n.get());
    for _ in 0..threads {
        let (root, job_rx, event_tx, live) = (
            root.to_owned(),
            job_rx.clone(),
            event_tx.clone(),
            live.clone(),
        );
        thread::spawn(move || {
            loop {
                let job = job_rx.lock().expect("job lock").recv();
                let Ok((index, shas)) = job else { return };
                if !live() {
                    return;
                }
                let commits = git::log_commits(&root, &shas).and_then(|s| s.collect());
                if event_tx.send(LogEvent::Chunk(index, commits)).is_err() {
                    return;
                }
            }
        });
    }

    let (producer_root, rev) = (root.to_owned(), rev.to_owned());
    thread::spawn(move || {
        let revs = match git::rev_list(&producer_root, &rev) {
            Ok(revs) => revs,
            Err(e) => {
                let _ = event_tx.send(LogEvent::Chunk(0, Err(e)));
                return;
            }
        };
        let (mut chunk, mut count) = (Vec::with_capacity(first_chunk), 0);
        for sha in revs {
            match sha {
                Ok(sha) => chunk.push(sha),
                Err(e) => {
                    let _ = event_tx.send(LogEvent::Chunk(count, Err(e)));
                    return;
                }
            }
            let size = if count == 0 { first_chunk } else { chunk_size };
            if chunk.len() == size {
                // Fails only once the load is abandoned; dropping `revs` kills git.
                if job_tx.send((count, mem::take(&mut chunk))).is_err() {
                    return;
                }
                count += 1;
            }
        }
        if !chunk.is_empty() && job_tx.send((count, chunk)).is_ok() {
            count += 1;
        }
        let _ = event_tx.send(LogEvent::Total(count));
    });

    // Chunks finish out of order; hold each until the ones before it are sent.
    let (mut next, mut total, mut waiting) = (0, None, HashMap::new());
    while total != Some(next) {
        let event = event_rx.recv().ok()?;
        if !live() {
            return None;
        }
        match event {
            LogEvent::Total(n) => total = Some(n),
            LogEvent::Chunk(_, Err(e)) => return Some(Err(e)),
            LogEvent::Chunk(index, Ok(commits)) => {
                waiting.insert(index, commits);
                while let Some(commits) = waiting.remove(&next) {
                    tx.send(Msg::Commits(generation, commits)).ok()?;
                    next += 1;
                }
            }
        }
    }
    Some(Ok(()))
}

/// At most this many commits per `git log -G`, so the first matches show up quickly.
const CODE_CHUNK: usize = 500;
/// Chunks per core when there are fewer commits, so every core gets work and the
/// expensive commits (big merges) don't all land on one.
const CODE_CHUNKS_PER_CORE: usize = 4;

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
            targets,
            generation,
        } = req;
        self.cancel();
        self.current.store(generation, Ordering::SeqCst);
        let cores = thread::available_parallelism().map_or(1, |n| n.get());
        let chunks: VecDeque<CodeChunk> = plan_chunks(targets, cores).into();
        let total = chunks.len();
        let queue = Arc::new(Mutex::new(chunks));
        let finished = Arc::new(AtomicUsize::new(0));
        let pattern = Arc::new(pattern);
        if total == 0 {
            let _ = self.tx.send(Msg::CodeDone(generation, Ok(())));
            return;
        }
        let threads = cores.min(total);
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
                    let paths = chunk.paths.as_deref();
                    let stream = match git::code_search(&root, &pattern, &chunk.shas, paths) {
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

/// Commits for one `git log -G`, and the files to diff (`None`: all of them).
#[derive(Debug, PartialEq, Eq)]
pub struct CodeChunk {
    pub shas: Vec<String>,
    pub paths: Option<Vec<String>>,
}

/// Paths in one chunk's pathspec, beyond which it diffs whole commits instead.
const MAX_PATHSPEC: usize = 512;

/// Cuts the targets into chunks. Commits searched whole and commits searched
/// only in some files go in separate chunks; the latter diff just the union of
/// their files, which skips most of a big merge.
pub fn plan_chunks(targets: Vec<CodeTarget>, cores: usize) -> Vec<CodeChunk> {
    let size = targets
        .len()
        .div_ceil(cores * CODE_CHUNKS_PER_CORE)
        .clamp(1, CODE_CHUNK);
    let (whole, scoped): (Vec<_>, Vec<_>) = targets.into_iter().partition(|t| t.paths.is_none());
    let mut chunks: Vec<CodeChunk> = whole
        .chunks(size)
        .map(|group| CodeChunk {
            shas: group.iter().map(|t| t.sha.clone()).collect(),
            paths: None,
        })
        .collect();
    for group in scoped.chunks(size) {
        let mut paths: Vec<String> = group
            .iter()
            .flat_map(|t| t.paths.iter().flatten())
            .cloned()
            .collect();
        paths.sort_unstable();
        paths.dedup();
        chunks.push(CodeChunk {
            shas: group.iter().map(|t| t.sha.clone()).collect(),
            paths: (paths.len() <= MAX_PATHSPEC).then_some(paths),
        });
    }
    chunks
}

/// Runs a code search to completion on this thread, chunk by chunk, exactly as
/// the searcher would.
#[cfg(test)]
pub fn search_now(root: &Path, req: &CodeRequest) -> Vec<(String, Vec<String>)> {
    plan_chunks(req.targets.clone(), 3)
        .iter()
        .flat_map(|c| git::code_search(root, &req.pattern, &c.shas, c.paths.as_deref()).unwrap())
        .collect()
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
            let result = git::diff(&root, &req.sha, req.file.as_ref())
                .map(|text| git::parse_diff(&text))
                .map_err(|e| e.to_string());
            if tx.send(Msg::Diff(req.key(), result)).is_err() {
                break;
            }
        }
    });
    req_tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Repo;

    fn history() -> Repo {
        let repo = Repo::new();
        repo.write("a.txt", "one\ntwo\nthree\nfour\n");
        repo.commit("root");
        repo.git(&["checkout", "-q", "-b", "side"]);
        repo.write("side.txt", "side\n");
        repo.commit("side work");
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["mv", "a.txt", "b.txt"]);
        repo.commit("rename");
        repo.git(&["merge", "-q", "--no-edit", "side"]);
        for n in 0..5 {
            repo.write("n.txt", &format!("{n}\n"));
            repo.commit(&format!("commit {n}"));
        }
        repo
    }

    /// Everything but the relative date, which can tick between two loads.
    fn key(c: &Commit) -> (String, String, Vec<git::FileChange>) {
        (c.sha.clone(), c.subject.clone(), c.files.clone())
    }

    #[test]
    fn parallel_load_takes_a_revision() {
        let repo = history();
        let expected: Vec<_> = repo.log().iter().take(3).map(key).collect();
        let (tx, rx) = mpsc::channel();
        let result = load_parallel(repo.path(), "HEAD~3..HEAD", &tx, 1, || true, 1, 2);
        assert!(matches!(result, Some(Ok(()))));
        drop(tx);
        let loaded: Vec<_> = rx
            .into_iter()
            .flat_map(|msg| match msg {
                Msg::Commits(_, batch) => batch,
                _ => panic!("unexpected message"),
            })
            .collect();
        assert_eq!(loaded.iter().map(key).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn parallel_load_matches_git_log_in_order() {
        let repo = history();
        let expected: Vec<_> = repo.log().iter().map(key).collect();
        assert_eq!(expected.len(), 9);

        let (tx, rx) = mpsc::channel();
        // Tiny chunks: many of them, finishing out of order.
        let result = load_parallel(repo.path(), "HEAD", &tx, 7, || true, 1, 2);
        assert!(matches!(result, Some(Ok(()))));
        drop(tx);
        let mut loaded = Vec::new();
        for msg in rx {
            let Msg::Commits(7, batch) = msg else {
                panic!("unexpected message")
            };
            loaded.extend(batch.iter().map(key));
        }
        assert_eq!(loaded, expected);
    }

    fn target(sha: &str, paths: Option<&[&str]>) -> CodeTarget {
        CodeTarget {
            sha: sha.into(),
            paths: paths.map(|p| p.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn chunks_separate_whole_and_file_scoped_commits() {
        let targets = vec![
            target("a", None),
            target("b", Some(&["x.rs", "y.rs"])),
            target("c", None),
            target("d", Some(&["y.rs", "z.rs"])),
        ];
        // Four targets on one core: four chunks of one, whole commits first.
        let chunks = plan_chunks(targets, 1);
        let kinds: Vec<_> = chunks
            .iter()
            .map(|c| (c.shas[0].as_str(), c.paths.is_some()))
            .collect();
        assert_eq!(
            kinds,
            [("a", false), ("c", false), ("b", true), ("d", true)]
        );

        // Nine targets: chunks of three, scoped ones sharing a deduplicated pathspec.
        let together = plan_chunks(
            (0..8)
                .map(|n| target(&n.to_string(), Some(&["x.rs", "y.rs"])))
                .chain([target("w", None)])
                .collect(),
            1,
        );
        assert_eq!(
            together,
            vec![
                CodeChunk {
                    shas: vec!["w".into()],
                    paths: None
                },
                CodeChunk {
                    shas: (0..3).map(|n| n.to_string()).collect(),
                    paths: Some(vec!["x.rs".into(), "y.rs".into()]),
                },
                CodeChunk {
                    shas: (3..6).map(|n| n.to_string()).collect(),
                    paths: Some(vec!["x.rs".into(), "y.rs".into()]),
                },
                CodeChunk {
                    shas: (6..8).map(|n| n.to_string()).collect(),
                    paths: Some(vec!["x.rs".into(), "y.rs".into()]),
                },
            ]
        );
    }

    #[test]
    fn huge_pathspecs_fall_back_to_whole_commits() {
        let paths: Vec<String> = (0..=MAX_PATHSPEC).map(|n| format!("f{n}")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let chunks = plan_chunks(vec![target("a", Some(&refs))], 1);
        assert_eq!(chunks[0].paths, None);
    }

    #[test]
    fn stale_load_stops_quietly() {
        let repo = history();
        let (tx, rx) = mpsc::channel();
        assert!(load_parallel(repo.path(), "HEAD", &tx, 1, || false, 1, 2).is_none());
        drop(tx);
        assert_eq!(rx.iter().count(), 0);
    }

    /// `LEVI_BENCH_REPO=... cargo test --release log_load_speed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn log_load_speed() {
        let Ok(path) = std::env::var("LEVI_BENCH_REPO") else {
            return;
        };
        let root = PathBuf::from(path);
        let (tx, rx) = mpsc::channel();
        let t = Instant::now();
        LogLoader::new(root, "HEAD".into(), tx).start(LogRequest {
            source: Source::Log,
            generation: 1,
        });
        let (mut first, mut count) = (None, 0);
        for msg in rx {
            match msg {
                Msg::Commits(_, batch) => {
                    first.get_or_insert((t.elapsed(), batch.len()));
                    count += batch.len();
                }
                Msg::LogDone(_, r) => {
                    r.unwrap();
                    break;
                }
                _ => {}
            }
        }
        println!(
            "first batch {first:.2?}, all {count} commits in {:.2?}",
            t.elapsed()
        );
    }
}
