use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::AtomicI64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::syncutil::lock;

/// Schedules the host's output flush (12 ms) and scrollback flush (2 s).
///
/// Tests supply [`ManualScheduler`], whose clock moves only when [`ManualScheduler::advance`]
/// is called. The desktop app uses [`thread_scheduler`].
pub trait Scheduler: Send + Sync {
    fn schedule(&self, delay: Duration, task: Box<dyn FnOnce() + Send>) -> u64;
    fn cancel(&self, id: u64);
}

pub(crate) fn thread_scheduler() -> Arc<dyn Scheduler> {
    Arc::new(ThreadScheduler::spawn())
}

struct ThreadScheduler {
    tx: Mutex<Option<Sender<SchedMsg>>>,
    next: AtomicU64,
}

enum SchedMsg {
    Add {
        id: u64,
        due: Instant,
        task: Box<dyn FnOnce() + Send>,
    },
    Cancel {
        id: u64,
    },
}

impl ThreadScheduler {
    fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("legate-pty-timer".to_string())
            .spawn(move || timer_loop(rx))
            .is_ok();
        Self {
            tx: Mutex::new(spawned.then_some(tx)),
            next: AtomicU64::new(1),
        }
    }
}

impl Drop for ThreadScheduler {
    fn drop(&mut self) {
        // Dropping the sender wakes the timer thread. Do not join it: a flush
        // callback can drop the last host clone on that same thread.
        lock(&self.tx).take();
    }
}

impl Scheduler for ThreadScheduler {
    fn schedule(&self, delay: Duration, task: Box<dyn FnOnce() + Send>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if let Some(tx) = lock(&self.tx).as_ref() {
            let _ = tx.send(SchedMsg::Add {
                id,
                due: Instant::now() + delay,
                task,
            });
        }
        id
    }

    fn cancel(&self, id: u64) {
        if let Some(tx) = lock(&self.tx).as_ref() {
            let _ = tx.send(SchedMsg::Cancel { id });
        }
    }
}

type TimerTask = Box<dyn FnOnce() + Send>;

fn timer_loop(rx: Receiver<SchedMsg>) {
    let mut pending: HashMap<u64, (Instant, TimerTask)> = HashMap::new();
    loop {
        let now = Instant::now();
        let next_due = pending.values().map(|(due, _)| *due).min();
        let incoming = if let Some(due) = next_due {
            if due <= now {
                None
            } else {
                match rx.recv_timeout(due.saturating_duration_since(now)) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        } else {
            match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            }
        };
        if let Some(msg) = incoming {
            match msg {
                SchedMsg::Add { id, due, task } => {
                    pending.insert(id, (due, task));
                }
                SchedMsg::Cancel { id } => {
                    pending.remove(&id);
                }
            }
            continue;
        }
        let now = Instant::now();
        let due_ids: Vec<u64> = pending
            .iter()
            .filter(|(_, (due, _))| *due <= now)
            .map(|(id, _)| *id)
            .collect();
        let mut tasks = Vec::with_capacity(due_ids.len());
        for id in due_ids {
            if let Some((_, task)) = pending.remove(&id) {
                tasks.push(task);
            }
        }
        for task in tasks {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
        }
    }
}

/// Test clock. Time moves only in [`ManualScheduler::advance`].
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct ManualScheduler {
    inner: Arc<ManualInner>,
}

#[cfg(test)]
struct ManualInner {
    now: AtomicI64,
    next: AtomicU64,
    entries: Mutex<Vec<ManualEntry>>,
}

#[cfg(test)]
struct ManualEntry {
    id: u64,
    due: i64,
    task: Option<Box<dyn FnOnce() + Send>>,
}

#[cfg(test)]
impl ManualScheduler {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(ManualInner {
                now: AtomicI64::new(0),
                next: AtomicU64::new(1),
                entries: Mutex::new(Vec::new()),
            }),
        }
    }

    pub(crate) fn advance(&self, by: Duration) {
        let add = i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
        self.inner.now.fetch_add(add, Ordering::Relaxed);
        loop {
            let Some(task) = self.pop_due() else {
                break;
            };
            task();
        }
    }

    fn pop_due(&self) -> Option<Box<dyn FnOnce() + Send>> {
        let mut entries = lock(&self.inner.entries);
        let now = self.inner.now.load(Ordering::Relaxed);
        let pos = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.task.is_some() && entry.due <= now)
            .min_by_key(|(_, entry)| entry.due)
            .map(|(index, _)| index)?;
        entries.swap_remove(pos).task
    }
}

#[cfg(test)]
impl Scheduler for ManualScheduler {
    fn schedule(&self, delay: Duration, task: Box<dyn FnOnce() + Send>) -> u64 {
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let delay_ms = i64::try_from(delay.as_millis()).unwrap_or(i64::MAX);
        let due = self
            .inner
            .now
            .load(Ordering::Relaxed)
            .saturating_add(delay_ms);
        lock(&self.inner.entries).push(ManualEntry {
            id,
            due,
            task: Some(task),
        });
        id
    }

    fn cancel(&self, id: u64) {
        lock(&self.inner.entries).retain(|entry| entry.id != id);
    }
}
