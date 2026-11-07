//! Trailing debounce for worktree refreshes. Port of `src/main/debounce.ts`.
//!
//! Several bursts of terminal output become one change-list push, and two
//! worktrees never push closer than [`CHANGE_MIN_GAP_MS`]. A fallback push
//! runs every [`CHANGE_FALLBACK_MS`] while a worktree terminal is still running.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Quiet period after the last output from one worktree, in milliseconds.
pub const CHANGE_QUIET_MS: i64 = 1500;
/// Minimum gap between any two pushes, in milliseconds.
pub const CHANGE_MIN_GAP_MS: i64 = 1500;
/// Fallback push while a worktree terminal is still running, in milliseconds.
pub const CHANGE_FALLBACK_MS: i64 = 30_000;

/// Clock and timers the refresh schedules on. Production uses [`LocalTimers`].
pub trait Timers {
    /// Epoch milliseconds, or a test clock's current time.
    fn now(&self) -> i64;
    /// Run `callback` once after `ms`. The id can be cleared.
    fn set_timeout(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64;
    /// Cancel a timeout. Unknown ids are ignored.
    fn clear_timeout(&mut self, id: u64);
    /// Run `callback` every `ms`, first fire after one period.
    fn set_interval(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64;
    /// Cancel an interval. Unknown ids are ignored.
    fn clear_interval(&mut self, id: u64);
}

fn gap_wait(last_push: i64, now: i64) -> i64 {
    if last_push == i64::MIN {
        return -1;
    }
    last_push
        .saturating_add(CHANGE_MIN_GAP_MS)
        .saturating_sub(now)
}

struct State {
    quiet: HashMap<String, u64>,
    ready: HashSet<String>,
    fallback_pending: bool,
    gap_id: Option<u64>,
    coalesce_id: Option<u64>,
    fallback_id: Option<u64>,
    last_push: i64,
}

/// Trailing debounce per worktree, plus a global minimum gap between pushes.
pub struct ChangeRefresh {
    push_fn: Rc<dyn Fn()>,
    busy: Rc<dyn Fn() -> bool>,
    timers: Rc<RefCell<dyn Timers>>,
    state: RefCell<State>,
}

impl ChangeRefresh {
    /// `push` publishes the change list. `busy` is true while a worktree terminal runs.
    ///
    /// Must be called inside a `LocalSet` when `timers` is [`LocalTimers`].
    pub fn start(
        timers: Rc<RefCell<dyn Timers>>,
        push: impl Fn() + 'static,
        busy: impl Fn() -> bool + 'static,
    ) -> Rc<Self> {
        let refresh = Rc::new(Self {
            push_fn: Rc::new(push),
            busy: Rc::new(busy),
            timers,
            state: RefCell::new(State {
                quiet: HashMap::new(),
                ready: HashSet::new(),
                fallback_pending: false,
                gap_id: None,
                coalesce_id: None,
                fallback_id: None,
                last_push: i64::MIN,
            }),
        });
        let tick = Rc::downgrade(&refresh);
        let id = refresh.timers.borrow_mut().set_interval(
            CHANGE_FALLBACK_MS,
            Rc::new(move || {
                if let Some(refresh) = tick.upgrade() {
                    refresh.on_fallback();
                }
            }),
        );
        refresh.state.borrow_mut().fallback_id = Some(id);
        refresh
    }

    /// Output arrived for `change_id`. The push waits until that id has been quiet.
    pub fn note(self: &Rc<Self>, change_id: &str) {
        {
            let mut state = self.state.borrow_mut();
            state.ready.remove(change_id);
            if let Some(prev) = state.quiet.remove(change_id) {
                self.timers.borrow_mut().clear_timeout(prev);
            }
        }
        let owned = change_id.to_string();
        let this = Rc::downgrade(self);
        let id = self.timers.borrow_mut().set_timeout(
            CHANGE_QUIET_MS,
            Rc::new(move || {
                let Some(refresh) = this.upgrade() else {
                    return;
                };
                {
                    let key = owned.clone();
                    let mut state = refresh.state.borrow_mut();
                    state.quiet.remove(&key);
                    state.ready.insert(key);
                }
                refresh.enqueue_flush();
            }),
        );
        let ready_empty = {
            let mut state = self.state.borrow_mut();
            state.quiet.insert(change_id.to_string(), id);
            state.ready.is_empty() && !state.fallback_pending
        };
        if ready_empty {
            self.clear_gap();
        }
    }

    /// Drop pending quiet, gap, and fallback timers.
    pub fn clear(&self) {
        let mut state = self.state.borrow_mut();
        let quiet: Vec<u64> = state.quiet.drain().map(|(_, id)| id).collect();
        state.ready.clear();
        state.fallback_pending = false;
        let gap = state.gap_id.take();
        let coalesce = state.coalesce_id.take();
        let fallback = state.fallback_id.take();
        drop(state);
        let mut timers = self.timers.borrow_mut();
        for id in quiet {
            timers.clear_timeout(id);
        }
        if let Some(id) = gap {
            timers.clear_timeout(id);
        }
        if let Some(id) = coalesce {
            timers.clear_timeout(id);
        }
        if let Some(id) = fallback {
            timers.clear_interval(id);
        }
    }

    fn on_fallback(self: &Rc<Self>) {
        if !(self.busy)() {
            return;
        }
        self.state.borrow_mut().fallback_pending = true;
        self.enqueue_flush();
    }

    fn enqueue_flush(self: &Rc<Self>) {
        if self.state.borrow().coalesce_id.is_some() {
            return;
        }
        let this = Rc::downgrade(self);
        let id = self.timers.borrow_mut().set_timeout(
            0,
            Rc::new(move || {
                if let Some(refresh) = this.upgrade() {
                    refresh.state.borrow_mut().coalesce_id = None;
                    refresh.flush_soon();
                }
            }),
        );
        self.state.borrow_mut().coalesce_id = Some(id);
    }

    fn flush_soon(self: &Rc<Self>) {
        let wait = {
            let state = self.state.borrow();
            if state.ready.is_empty() && !state.fallback_pending {
                drop(state);
                self.clear_gap();
                return;
            }
            let now = self.timers.borrow().now();
            gap_wait(state.last_push, now)
        };
        if wait <= 0 {
            self.flush();
            return;
        }
        if self.state.borrow().gap_id.is_some() {
            return;
        }
        let this = Rc::downgrade(self);
        let id = self.timers.borrow_mut().set_timeout(
            wait,
            Rc::new(move || {
                if let Some(refresh) = this.upgrade() {
                    refresh.state.borrow_mut().gap_id = None;
                    refresh.flush_soon();
                }
            }),
        );
        self.state.borrow_mut().gap_id = Some(id);
    }

    fn flush(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.ready.clear();
            state.fallback_pending = false;
        }
        self.clear_gap();
        let now = self.timers.borrow().now();
        self.state.borrow_mut().last_push = now;
        (self.push_fn)();
    }

    fn clear_gap(&self) {
        let id = self.state.borrow_mut().gap_id.take();
        if let Some(id) = id {
            self.timers.borrow_mut().clear_timeout(id);
        }
    }
}

struct Armed {
    cancel: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
}

/// Timers on the orchestrator's `LocalSet`. Each wait is a `sleep`, not a poll.
pub struct LocalTimers {
    next: u64,
    tasks: HashMap<u64, Armed>,
}

impl LocalTimers {
    /// Empty timer table. Call from inside the `LocalSet`.
    pub fn new() -> Self {
        Self {
            next: 1,
            tasks: HashMap::new(),
        }
    }

    fn alloc(&mut self) -> u64 {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        id
    }

    fn arm(&mut self, id: u64, cancel: Arc<AtomicBool>, handle: tokio::task::JoinHandle<()>) {
        self.tasks.insert(id, Armed { cancel, handle });
    }

    fn stop(&mut self, id: u64) {
        if let Some(armed) = self.tasks.remove(&id) {
            armed.cancel.store(true, Ordering::Release);
            armed.handle.abort();
        }
    }
}

impl Default for LocalTimers {
    fn default() -> Self {
        Self::new()
    }
}

impl Timers for LocalTimers {
    fn now(&self) -> i64 {
        legate_router::system_now_ms()
    }

    fn set_timeout(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64 {
        let id = self.alloc();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let handle = tokio::task::spawn_local(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms.max(0) as u64)).await;
            if !flag.load(Ordering::Acquire) {
                callback();
            }
        });
        self.arm(id, cancel, handle);
        id
    }

    fn clear_timeout(&mut self, id: u64) {
        self.stop(id);
    }

    fn set_interval(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64 {
        let id = self.alloc();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let period = std::time::Duration::from_millis(ms.max(0) as u64);
        let handle = tokio::task::spawn_local(async move {
            loop {
                tokio::time::sleep(period).await;
                if flag.load(Ordering::Acquire) {
                    break;
                }
                callback();
            }
        });
        self.arm(id, cancel, handle);
        id
    }

    fn clear_interval(&mut self, id: u64) {
        self.stop(id);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use super::{CHANGE_FALLBACK_MS, CHANGE_MIN_GAP_MS, CHANGE_QUIET_MS, ChangeRefresh, Timers};

    struct TimeoutItem {
        at: i64,
        callback: Rc<dyn Fn()>,
    }

    struct IntervalItem {
        every: i64,
        next: i64,
        callback: Rc<dyn Fn()>,
    }

    struct FakeClock {
        now_ms: i64,
        next_id: u64,
        timeouts: HashMap<u64, TimeoutItem>,
        intervals: HashMap<u64, IntervalItem>,
    }

    impl FakeClock {
        fn new() -> Rc<RefCell<Self>> {
            Rc::new(RefCell::new(Self {
                now_ms: 0,
                next_id: 1,
                timeouts: HashMap::new(),
                intervals: HashMap::new(),
            }))
        }

        fn advance(this: &Rc<RefCell<Self>>, ms: i64) {
            let end = this.borrow().now_ms + ms;
            loop {
                let due = {
                    let mut clock = this.borrow_mut();
                    let mut next = i64::MAX;
                    for item in clock.timeouts.values() {
                        next = next.min(item.at);
                    }
                    for item in clock.intervals.values() {
                        next = next.min(item.next);
                    }
                    if next > end {
                        clock.now_ms = end;
                        return;
                    }
                    clock.now_ms = next;
                    let mut callbacks = Vec::new();
                    let timeout_ids: Vec<u64> = clock
                        .timeouts
                        .iter()
                        .filter(|(_, item)| item.at == next)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in timeout_ids {
                        if let Some(item) = clock.timeouts.remove(&id) {
                            callbacks.push(item.callback);
                        }
                    }
                    let interval_ids: Vec<u64> = clock
                        .intervals
                        .iter()
                        .filter(|(_, item)| item.next == next)
                        .map(|(id, _)| *id)
                        .collect();
                    for id in interval_ids {
                        if let Some(item) = clock.intervals.get_mut(&id) {
                            item.next = item.next.saturating_add(item.every);
                            callbacks.push(Rc::clone(&item.callback));
                        }
                    }
                    callbacks
                };
                for callback in due {
                    callback();
                }
            }
        }
    }

    impl Timers for FakeClock {
        fn now(&self) -> i64 {
            self.now_ms
        }

        fn set_timeout(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64 {
            let id = self.next_id;
            self.next_id += 1;
            self.timeouts.insert(
                id,
                TimeoutItem {
                    at: self.now_ms.saturating_add(ms),
                    callback,
                },
            );
            id
        }

        fn clear_timeout(&mut self, id: u64) {
            self.timeouts.remove(&id);
        }

        fn set_interval(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64 {
            let id = self.next_id;
            self.next_id += 1;
            self.intervals.insert(
                id,
                IntervalItem {
                    every: ms,
                    next: self.now_ms.saturating_add(ms),
                    callback,
                },
            );
            id
        }

        fn clear_interval(&mut self, id: u64) {
            self.intervals.remove(&id);
        }
    }

    struct ClockTimers(Rc<RefCell<FakeClock>>);

    impl Timers for ClockTimers {
        fn now(&self) -> i64 {
            self.0.borrow().now()
        }

        fn set_timeout(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64 {
            self.0.borrow_mut().set_timeout(ms, callback)
        }

        fn clear_timeout(&mut self, id: u64) {
            self.0.borrow_mut().clear_timeout(id);
        }

        fn set_interval(&mut self, ms: i64, callback: Rc<dyn Fn()>) -> u64 {
            self.0.borrow_mut().set_interval(ms, callback)
        }

        fn clear_interval(&mut self, id: u64) {
            self.0.borrow_mut().clear_interval(id);
        }
    }

    fn watch(
        clock: &Rc<RefCell<FakeClock>>,
        push: impl Fn() + 'static,
        busy: impl Fn() -> bool + 'static,
    ) -> Rc<ChangeRefresh> {
        let timers: Rc<RefCell<dyn Timers>> = Rc::new(RefCell::new(ClockTimers(Rc::clone(clock))));
        ChangeRefresh::start(timers, push, busy)
    }

    #[test]
    fn several_outputs_become_one_push_after_the_quiet_period() {
        let clock = FakeClock::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&calls);
        let clock_for_push = Rc::clone(&clock);
        let refresh = watch(
            &clock,
            move || seen.borrow_mut().push(clock_for_push.borrow().now_ms),
            || false,
        );
        refresh.note("a");
        FakeClock::advance(&clock, 400);
        refresh.note("a");
        FakeClock::advance(&clock, 400);
        refresh.note("a");
        FakeClock::advance(&clock, CHANGE_QUIET_MS - 1);
        assert!(calls.borrow().is_empty());
        FakeClock::advance(&clock, 1);
        assert_eq!(*calls.borrow(), vec![800 + CHANGE_QUIET_MS]);
        FakeClock::advance(&clock, CHANGE_FALLBACK_MS);
        assert_eq!(*calls.borrow(), vec![800 + CHANGE_QUIET_MS]);
        refresh.clear();
    }

    #[test]
    fn two_worktrees_share_one_push_when_they_go_quiet_together() {
        let clock = FakeClock::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&calls);
        let clock_for_push = Rc::clone(&clock);
        let refresh = watch(
            &clock,
            move || seen.borrow_mut().push(clock_for_push.borrow().now_ms),
            || false,
        );
        refresh.note("a");
        refresh.note("b");
        FakeClock::advance(&clock, CHANGE_QUIET_MS - 1);
        assert!(calls.borrow().is_empty());
        FakeClock::advance(&clock, 1);
        assert_eq!(*calls.borrow(), vec![CHANGE_QUIET_MS]);
        FakeClock::advance(&clock, CHANGE_MIN_GAP_MS);
        assert_eq!(*calls.borrow(), vec![CHANGE_QUIET_MS]);
        refresh.clear();
    }

    #[test]
    fn two_worktrees_never_push_closer_than_the_minimum_gap() {
        let clock = FakeClock::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&calls);
        let clock_for_push = Rc::clone(&clock);
        let refresh = watch(
            &clock,
            move || seen.borrow_mut().push(clock_for_push.borrow().now_ms),
            || false,
        );
        refresh.note("a");
        FakeClock::advance(&clock, 100);
        refresh.note("b");
        FakeClock::advance(&clock, CHANGE_QUIET_MS - 100);
        assert_eq!(*calls.borrow(), vec![CHANGE_QUIET_MS]);
        FakeClock::advance(&clock, 100);
        assert_eq!(*calls.borrow(), vec![CHANGE_QUIET_MS]);
        FakeClock::advance(&clock, CHANGE_MIN_GAP_MS - 100);
        assert_eq!(
            *calls.borrow(),
            vec![CHANGE_QUIET_MS, CHANGE_QUIET_MS + CHANGE_MIN_GAP_MS]
        );
        refresh.clear();
    }

    #[test]
    fn clear_drops_pending_quiet_gap_and_fallback_timers() {
        let clock = FakeClock::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&calls);
        let clock_for_push = Rc::clone(&clock);
        let refresh = watch(
            &clock,
            move || seen.borrow_mut().push(clock_for_push.borrow().now_ms),
            || true,
        );
        refresh.note("a");
        refresh.clear();
        FakeClock::advance(&clock, CHANGE_FALLBACK_MS + CHANGE_QUIET_MS);
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn fallback_pushes_every_30s_while_a_worktree_terminal_is_running() {
        let clock = FakeClock::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&calls);
        let clock_for_push = Rc::clone(&clock);
        let refresh = watch(
            &clock,
            move || seen.borrow_mut().push(clock_for_push.borrow().now_ms),
            || true,
        );
        FakeClock::advance(&clock, CHANGE_FALLBACK_MS - 1);
        assert!(calls.borrow().is_empty());
        FakeClock::advance(&clock, 1);
        assert_eq!(*calls.borrow(), vec![CHANGE_FALLBACK_MS]);
        FakeClock::advance(&clock, CHANGE_FALLBACK_MS);
        assert_eq!(
            *calls.borrow(),
            vec![CHANGE_FALLBACK_MS, CHANGE_FALLBACK_MS * 2]
        );
        refresh.clear();
    }
}
