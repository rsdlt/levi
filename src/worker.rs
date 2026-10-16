//! Background threads. Everything reports back to the UI through one `Msg` channel.

use std::mem;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::crossterm::event;

use crate::app::{DiffRequest, Msg};
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

/// Streams the log in batches so the UI is usable before git finishes.
pub fn spawn_log(root: PathBuf, tx: Sender<Msg>) {
    thread::spawn(move || {
        let stream = match git::log(&root) {
            Ok(stream) => stream,
            Err(e) => {
                let _ = tx.send(Msg::LogDone(Err(e.to_string())));
                return;
            }
        };
        let mut batch = Vec::new();
        let mut last_send = Instant::now();
        for item in stream {
            match item {
                Ok(commit) => batch.push(commit),
                Err(e) => {
                    let _ = tx.send(Msg::Commits(batch));
                    let _ = tx.send(Msg::LogDone(Err(e.to_string())));
                    return;
                }
            }
            if batch.len() >= BATCH_SIZE || last_send.elapsed() >= BATCH_INTERVAL {
                // A send error means the UI is gone; dropping the stream kills git.
                if tx.send(Msg::Commits(mem::take(&mut batch))).is_err() {
                    return;
                }
                last_send = Instant::now();
            }
        }
        let _ = tx.send(Msg::Commits(batch));
        let _ = tx.send(Msg::LogDone(Ok(())));
    });
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
