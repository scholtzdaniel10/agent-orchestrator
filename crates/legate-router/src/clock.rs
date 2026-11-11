//! Clocks for the orchestrator. Job dispatch does not use these; only heartbeat, store
//! watch, and step flush do. [`ManualClock::tick`] runs due callbacks. [`SystemClock`]
//! sleeps until a deadline. Neither polls the queue.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ports::Clock;

pub struct FrozenClock {
    now: Rc<Cell<i64>>,
}

impl FrozenClock {
    pub fn new(now: i64) -> Self {
        Self {
            now: Rc::new(Cell::new(now)),
        }
    }

    pub fn from_cell(now: Rc<Cell<i64>>) -> Self {
        Self { now }
    }

    pub fn set(&self, now: i64) {
        self.now.set(now);
    }
}

impl Clock for FrozenClock {
    fn now_ms(&self) -> i64 {
        self.now.get()
    }

    fn set_interval(&self, _period_ms: i64, _cb: Rc<dyn Fn()>) -> u64 {
        0
    }

    fn clear_interval(&self, _id: u64) {}

    fn set_timeout(&self, _wait_ms: i64, _cb: Rc<dyn Fn()>) -> u64 {
        0
    }

    fn clear_timeout(&self, _id: u64) {}
}

struct Interval {
    period: i64,
    next: i64,
    cb: Rc<dyn Fn()>,
}

struct Timeout {
    at: i64,
    cb: Rc<dyn Fn()>,
}

enum Due {
    Interval(u64),
    Timeout(u64),
}

/// Test clock. `tick` moves time forward and runs callbacks whose deadline has arrived.
/// Two clocks can share one [`Rc<Cell<i64>>`] so each has its own callbacks.
pub struct ManualClock {
    time: Rc<Cell<i64>>,
    next_id: Cell<u64>,
    intervals: RefCell<HashMap<u64, Interval>>,
    timeouts: RefCell<HashMap<u64, Timeout>>,
}

impl ManualClock {
    pub fn new(now: i64) -> Self {
        Self::from_cell(Rc::new(Cell::new(now)))
    }

    pub fn from_cell(time: Rc<Cell<i64>>) -> Self {
        Self {
            time,
            next_id: Cell::new(1),
            intervals: RefCell::new(HashMap::new()),
            timeouts: RefCell::new(HashMap::new()),
        }
    }

    pub fn time_cell(&self) -> Rc<Cell<i64>> {
        Rc::clone(&self.time)
    }

    pub fn clear_all(&self) {
        self.intervals.borrow_mut().clear();
        self.timeouts.borrow_mut().clear();
    }

    pub fn tick(&self, amount: i64) {
        let end = self.time.get().saturating_add(amount);
        while let Some((at, due)) = self.next_due(end) {
            self.time.set(at);
            let cb = match due {
                Due::Interval(id) => {
                    let mut intervals = self.intervals.borrow_mut();
                    let Some(item) = intervals.get_mut(&id) else {
                        continue;
                    };
                    item.next = at.saturating_add(item.period);
                    Rc::clone(&item.cb)
                }
                Due::Timeout(id) => {
                    let Some(timeout) = self.timeouts.borrow_mut().remove(&id) else {
                        continue;
                    };
                    timeout.cb
                }
            };
            drop_and_call(cb);
        }
        self.time.set(end);
    }

    fn next_due(&self, end: i64) -> Option<(i64, Due)> {
        let intervals = self.intervals.borrow();
        let timeouts = self.timeouts.borrow();
        let mut best: Option<(i64, Due)> = None;
        for (id, item) in intervals.iter() {
            if item.next <= end && best.as_ref().is_none_or(|(at, _)| item.next < *at) {
                best = Some((item.next, Due::Interval(*id)));
            }
        }
        for (id, item) in timeouts.iter() {
            if item.at <= end && best.as_ref().is_none_or(|(at, _)| item.at < *at) {
                best = Some((item.at, Due::Timeout(*id)));
            }
        }
        best
    }

    fn alloc_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id.saturating_add(1));
        id
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.time.get()
    }

    fn set_interval(&self, period_ms: i64, cb: Rc<dyn Fn()>) -> u64 {
        let id = self.alloc_id();
        let next = self.time.get().saturating_add(period_ms);
        self.intervals.borrow_mut().insert(
            id,
            Interval {
                period: period_ms,
                next,
                cb,
            },
        );
        id
    }

    fn clear_interval(&self, id: u64) {
        self.intervals.borrow_mut().remove(&id);
    }

    fn set_timeout(&self, wait_ms: i64, cb: Rc<dyn Fn()>) -> u64 {
        let id = self.alloc_id();
        self.timeouts.borrow_mut().insert(
            id,
            Timeout {
                at: self.time.get().saturating_add(wait_ms),
                cb,
            },
        );
        id
    }

    fn clear_timeout(&self, id: u64) {
        self.timeouts.borrow_mut().remove(&id);
    }
}

fn drop_and_call(cb: Rc<dyn Fn()>) {
    cb();
}

/// Production clock. Each armed timer is a task that sleeps, then runs the callback.
///
/// `set_interval` and `set_timeout` must run inside a `tokio::task::LocalSet`, because the
/// tasks are `spawn_local` (the callback is not `Send`).
pub struct SystemClock {
    next_id: Cell<u64>,
    cancel: RefCell<HashMap<u64, tokio::sync::oneshot::Sender<()>>>,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            next_id: Cell::new(1),
            cancel: RefCell::new(HashMap::new()),
        }
    }

    fn alloc_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id.saturating_add(1));
        id
    }

    fn arm(&self, id: u64, wait_ms: i64, repeat: bool, cb: Rc<dyn Fn()>) {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        self.cancel.borrow_mut().insert(id, tx);
        tokio::task::spawn_local(async move {
            loop {
                let sleep =
                    tokio::time::sleep(std::time::Duration::from_millis(wait_ms.max(0) as u64));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => cb(),
                    _ = &mut rx => break,
                }
                if !repeat {
                    break;
                }
            }
        });
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        system_now_ms()
    }

    fn set_interval(&self, period_ms: i64, cb: Rc<dyn Fn()>) -> u64 {
        let id = self.alloc_id();
        self.arm(id, period_ms, true, cb);
        id
    }

    fn clear_interval(&self, id: u64) {
        self.cancel.borrow_mut().remove(&id);
    }

    fn set_timeout(&self, wait_ms: i64, cb: Rc<dyn Fn()>) -> u64 {
        let id = self.alloc_id();
        self.arm(id, wait_ms, false, cb);
        id
    }

    fn clear_timeout(&self, id: u64) {
        self.cancel.borrow_mut().remove(&id);
    }
}

pub fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
