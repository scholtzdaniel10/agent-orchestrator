//! When to ask Claude for its usage. Port of `src/main/usage-schedule.ts`.

/// How often a trigger may run a check, in milliseconds.
pub struct UsageGapMs {
    /// Refresh in the Usage tab: a double click must not run two checks.
    pub manual: i64,
    /// Coming back to a Legate window.
    pub focus: i64,
    /// While a Legate window has focus.
    pub focused: i64,
    /// While a window is on screen but another app has focus.
    pub visible: i64,
}

/// Same gaps as `USAGE_GAP_MS` in `usage-schedule.ts`.
pub const USAGE_GAP_MS: UsageGapMs = UsageGapMs {
    manual: 2 * 60_000,
    focus: 30_000,
    focused: 5 * 60_000,
    visible: 10 * 60_000,
};

/// How often the timer looks at whether a check is due.
pub const USAGE_TICK_MS: i64 = 60_000;

/// Why a usage check was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageTrigger {
    /// The person pressed Refresh. At most once every two minutes.
    Manual,
    /// A Legate window just gained focus.
    Focus,
    /// The minute timer.
    Tick,
}

/// Whether any Legate window is focused or merely on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSight {
    pub focused: bool,
    pub visible: bool,
}

impl WindowSight {
    /// No Legate window is open. Automatic checks stay quiet.
    pub const fn none() -> Self {
        Self {
            focused: false,
            visible: false,
        }
    }
}

/// True when a check should run now.
///
/// `last` is the newest reading, from a check or from a job's own report, or
/// `None` when there has been none. A tick checks only while a window is on screen.
pub fn usage_check_due(
    trigger: UsageTrigger,
    now: i64,
    last: Option<i64>,
    windows: WindowSight,
) -> bool {
    let Some(last) = last else {
        return !matches!(trigger, UsageTrigger::Tick) || windows.visible;
    };
    let age = now.saturating_sub(last);
    match trigger {
        UsageTrigger::Manual => age >= USAGE_GAP_MS.manual,
        UsageTrigger::Focus => age >= USAGE_GAP_MS.focus,
        UsageTrigger::Tick if windows.focused => age >= USAGE_GAP_MS.focused,
        UsageTrigger::Tick if windows.visible => age >= USAGE_GAP_MS.visible,
        UsageTrigger::Tick => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{USAGE_GAP_MS, UsageTrigger, WindowSight, usage_check_due};

    const NOW: i64 = 10_000_000;
    const FOCUSED: WindowSight = WindowSight {
        focused: true,
        visible: true,
    };
    const VISIBLE: WindowSight = WindowSight {
        focused: false,
        visible: true,
    };
    const HIDDEN: WindowSight = WindowSight {
        focused: false,
        visible: false,
    };

    #[test]
    fn coming_back_to_the_window_checks_at_most_every_30_seconds() {
        assert!(!usage_check_due(
            UsageTrigger::Focus,
            NOW,
            Some(NOW - 29_000),
            FOCUSED
        ));
        assert!(usage_check_due(
            UsageTrigger::Focus,
            NOW,
            Some(NOW - USAGE_GAP_MS.focus),
            FOCUSED
        ));
    }

    #[test]
    fn refresh_keeps_its_two_minute_gap() {
        assert!(!usage_check_due(
            UsageTrigger::Manual,
            NOW,
            Some(NOW - 90_000),
            FOCUSED
        ));
        assert!(usage_check_due(
            UsageTrigger::Manual,
            NOW,
            Some(NOW - 2 * 60_000),
            FOCUSED
        ));
    }

    #[test]
    fn while_a_window_has_focus_the_timer_checks_every_5_minutes() {
        assert!(!usage_check_due(
            UsageTrigger::Tick,
            NOW,
            Some(NOW - 4 * 60_000),
            FOCUSED
        ));
        assert!(usage_check_due(
            UsageTrigger::Tick,
            NOW,
            Some(NOW - 5 * 60_000),
            FOCUSED
        ));
    }

    #[test]
    fn on_screen_without_focus_it_checks_every_10_minutes_and_not_when_hidden() {
        assert!(!usage_check_due(
            UsageTrigger::Tick,
            NOW,
            Some(NOW - 6 * 60_000),
            VISIBLE
        ));
        assert!(usage_check_due(
            UsageTrigger::Tick,
            NOW,
            Some(NOW - 10 * 60_000),
            VISIBLE
        ));
        assert!(!usage_check_due(
            UsageTrigger::Tick,
            NOW,
            Some(NOW - 60 * 60_000),
            HIDDEN
        ));
    }

    #[test]
    fn with_no_reading_yet_any_trigger_checks_except_a_hidden_tick() {
        assert!(usage_check_due(UsageTrigger::Tick, NOW, None, VISIBLE));
        assert!(usage_check_due(UsageTrigger::Focus, NOW, None, HIDDEN));
        assert!(!usage_check_due(UsageTrigger::Tick, NOW, None, HIDDEN));
    }
}
