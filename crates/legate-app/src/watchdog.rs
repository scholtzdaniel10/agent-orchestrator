//! Notices when the backend's thread stops running and writes down what it was doing.
//!
//! Every call into the backend, and every background loop, runs as a task on one thread (a
//! `LocalSet`). If any of them blocks that thread synchronously, every button in the app stops
//! answering while the process uses no CPU. To find out which, a heartbeat task on that thread
//! ticks every second; a plain thread watches the heartbeat and, when it has been still for
//! `stale_after`, writes a report to `<data dir>/hangs/hang-<epoch ms>.log` and logs it: the
//! calls still running with their age, and the last things the backend started or finished
//! (the last one started is the likely culprit). When the heartbeat comes back, the report gets
//! a "recovered" line.
//!
//! `mark` and `begin` are cheap and do nothing until `start` has run, so tests and the CLI are
//! unaffected.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// How many recent marks a report shows.
const TRAIL: usize = 64;

static WATCH: OnceLock<Arc<Watch>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    /// How long the heartbeat may stand still before it counts as a hang.
    pub stale_after: Duration,
    /// How often the watcher looks.
    pub check_every: Duration,
    /// How often the heartbeat ticks.
    pub beat_every: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            stale_after: Duration::from_secs(15),
            check_every: Duration::from_secs(1),
            beat_every: Duration::from_secs(1),
        }
    }
}

pub(crate) struct Watch {
    dir: PathBuf,
    timing: Timing,
    beat: AtomicI64,
    next: AtomicU64,
    running: Mutex<HashMap<u64, (String, i64)>>,
    trail: Mutex<VecDeque<(i64, String)>>,
    /// The report written for the hang in progress, if any.
    open_report: Mutex<Option<(PathBuf, i64)>>,
    /// When the watcher last looked, and when it last woke from a gap in its own looking (the
    /// computer slept): silence is only counted from then.
    last_check: AtomicI64,
    woke_at: AtomicI64,
}

/// Drops out of the running list when the call it names finishes.
pub(crate) struct Running {
    id: u64,
    started: i64,
}

impl Drop for Running {
    fn drop(&mut self) {
        let Some(watch) = WATCH.get() else {
            return;
        };
        let name = lock(&watch.running).remove(&self.id).map(|(name, _)| name);
        if let Some(name) = name {
            let took = now_ms() - self.started;
            watch.push(format!("done {name} ({took} ms)"));
        }
    }
}

/// Starts watching. Call once, from the backend's thread inside its `LocalSet`, which also
/// starts the heartbeat there. Later calls do nothing.
pub(crate) fn start(data_dir: &Path, timing: Timing) {
    let watch = Arc::new(Watch {
        dir: data_dir.join("hangs"),
        timing,
        beat: AtomicI64::new(now_ms()),
        next: AtomicU64::new(1),
        running: Mutex::new(HashMap::new()),
        trail: Mutex::new(VecDeque::new()),
        open_report: Mutex::new(None),
        last_check: AtomicI64::new(0),
        woke_at: AtomicI64::new(0),
    });
    if WATCH.set(Arc::clone(&watch)).is_err() {
        return;
    }
    let beat = Arc::clone(&watch);
    tokio::task::spawn_local(async move {
        loop {
            beat.beat.store(now_ms(), Ordering::Relaxed);
            tokio::time::sleep(beat.timing.beat_every).await;
        }
    });
    let watcher = Arc::clone(&watch);
    let _ = std::thread::Builder::new()
        .name("legate-watchdog".into())
        .spawn(move || {
            loop {
                std::thread::sleep(watcher.timing.check_every);
                watcher.check(now_ms());
            }
        });
}

/// Notes something the backend is about to do. Shows in a hang report's trail.
pub(crate) fn mark(what: impl Into<String>) {
    if let Some(watch) = WATCH.get() {
        watch.push(what.into());
    }
}

/// Notes a call that started; it stays in the running list until the guard drops.
pub(crate) fn begin(name: &str) -> Option<Running> {
    let watch = WATCH.get()?;
    let id = watch.next.fetch_add(1, Ordering::Relaxed);
    let started = now_ms();
    lock(&watch.running).insert(id, (name.to_string(), started));
    watch.push(format!("start {name}"));
    Some(Running { id, started })
}

/// A readable name for a backend call from the type of the closure that made it:
/// `legate_app::handle::Handle::open_chat_in_terminal::{{closure}}` → `open_chat_in_terminal`.
pub(crate) fn call_name(type_name: &'static str) -> &'static str {
    let trimmed = type_name.trim_end_matches("::{{closure}}");
    trimmed.rsplit("::").next().unwrap_or(trimmed)
}

impl Watch {
    fn push(&self, what: String) {
        let mut trail = lock(&self.trail);
        if trail.len() == TRAIL {
            trail.pop_front();
        }
        trail.push_back((now_ms(), what));
    }

    /// One look at the heartbeat. Writes a report when it has gone still, and a recovery line
    /// when it moves again. Returns the report's path when it wrote one.
    fn check(&self, now: i64) -> Option<PathBuf> {
        let stale_after = self.timing.stale_after.as_millis() as i64;
        // When the watcher itself has not run for a while, the computer was asleep, not the
        // backend stuck: count silence again from now.
        let previous = self.last_check.swap(now, Ordering::Relaxed);
        if previous != 0 && now - previous >= stale_after {
            self.woke_at.store(now, Ordering::Relaxed);
            self.push(format!(
                "the computer slept or the watcher paused for {} s",
                (now - previous) / 1000
            ));
        }
        let beat = self.beat.load(Ordering::Relaxed);
        let since = beat.max(self.woke_at.load(Ordering::Relaxed));
        let still_ms = now - since;
        let stale = still_ms >= stale_after;
        let mut open = lock(&self.open_report);
        match (stale, open.as_ref()) {
            (true, None) => {
                let text = self.report(now, still_ms);
                tracing::error!(
                    "backend thread has not run for {} s\n{text}",
                    still_ms / 1000
                );
                let path = self.dir.join(format!("hang-{now}.log"));
                let _ = std::fs::create_dir_all(&self.dir);
                let _ = std::fs::write(&path, &text);
                *open = Some((path.clone(), beat));
                Some(path)
            }
            (false, Some((path, since))) => {
                let line = format!(
                    "\nrecovered: the backend thread ran again after {} s\n",
                    (beat - since) / 1000
                );
                tracing::warn!("{}", line.trim());
                let mut text = std::fs::read_to_string(path).unwrap_or_default();
                text.push_str(&line);
                let _ = std::fs::write(path, text);
                *open = None;
                None
            }
            _ => None,
        }
    }

    fn report(&self, now: i64, still_ms: i64) -> String {
        let mut out = format!(
            "Legate backend hang report\n\
             written at epoch ms {now}\n\
             the backend thread has not run for {} s (heartbeat stopped at epoch ms {})\n\n",
            still_ms / 1000,
            now - still_ms
        );
        let mut running = lock(&self.running).values().cloned().collect::<Vec<_>>();
        running.sort_by_key(|(_, started)| *started);
        out.push_str("calls still running (oldest first):\n");
        if running.is_empty() {
            out.push_str("  none\n");
        }
        for (name, started) in running {
            out.push_str(&format!("  {name}: {} s\n", (now - started) / 1000));
        }
        out.push_str("\nlast things the backend started or finished (newest last):\n");
        for (at, what) in lock(&self.trail).iter() {
            out.push_str(&format!("  -{:.1} s  {what}\n", (now - at) as f64 / 1000.0));
        }
        out.push_str(
            "\nThe last entry that started before the heartbeat stopped, and is not marked done, \
             is the likely culprit.\n",
        );
        out
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch_in(dir: &Path) -> Watch {
        Watch {
            dir: dir.to_path_buf(),
            timing: Timing {
                stale_after: Duration::from_secs(15),
                ..Timing::default()
            },
            beat: AtomicI64::new(0),
            next: AtomicU64::new(1),
            running: Mutex::new(HashMap::new()),
            trail: Mutex::new(VecDeque::new()),
            open_report: Mutex::new(None),
            last_check: AtomicI64::new(0),
            woke_at: AtomicI64::new(0),
        }
    }

    #[test]
    fn call_names_come_from_the_handle_method() {
        assert_eq!(
            call_name("legate_app::handle::Handle::open_chat_in_terminal::{{closure}}"),
            "open_chat_in_terminal"
        );
        assert_eq!(call_name("terminal data"), "terminal data");
    }

    #[test]
    fn a_still_heartbeat_writes_one_report_and_a_recovery_line() {
        let dir = std::env::temp_dir().join(format!("legate-watchdog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let watch = watch_in(&dir);
        watch.beat.store(1_000, Ordering::Relaxed);
        lock(&watch.running).insert(1, ("send_lead".into(), 500));
        lock(&watch.running).insert(2, ("open_chat_in_terminal".into(), 900));
        watch.push("chat sync: apply claude-s-1".into());

        assert_eq!(watch.check(10_000), None, "9 s is not a hang yet");
        let path = watch.check(16_000).expect("16 s still is a hang");
        assert_eq!(watch.check(17_000), None, "one report per hang");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("has not run for 15 s"));
        let send = text.find("send_lead: 15 s").unwrap();
        let open = text.find("open_chat_in_terminal: 15 s").unwrap();
        assert!(send < open, "oldest first");
        assert!(text.contains("chat sync: apply claude-s-1"));

        watch.beat.store(20_000, Ordering::Relaxed);
        assert_eq!(watch.check(20_500), None);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("recovered: the backend thread ran again after 19 s"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn waking_from_sleep_is_not_a_hang_but_silence_after_it_is() {
        let dir =
            std::env::temp_dir().join(format!("legate-watchdog-sleep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let watch = watch_in(&dir);
        watch.beat.store(1_000, Ordering::Relaxed);
        assert_eq!(watch.check(2_000), None);
        // The computer slept for an hour: the watcher's own gap gives it away.
        assert_eq!(watch.check(3_602_000), None, "a sleep is not a hang");
        assert_eq!(watch.check(3_603_000), None);
        // The backend answers again after waking: still no report.
        watch.beat.store(3_604_000, Ordering::Relaxed);
        assert_eq!(watch.check(3_610_000), None);
        // But 15 s of real silence after waking is a hang.
        assert!(watch.check(3_619_000).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_trail_keeps_the_last_entries() {
        let watch = watch_in(Path::new("."));
        for n in 0..(TRAIL + 10) {
            watch.push(format!("mark {n}"));
        }
        let trail = lock(&watch.trail);
        assert_eq!(trail.len(), TRAIL);
        assert_eq!(trail.front().unwrap().1, "mark 10");
    }
}
