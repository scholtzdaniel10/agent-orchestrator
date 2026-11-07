//! Usage phrases and the reset calendar. Port of `usage-format.ts`.
//!
//! Month numbers are 0–11, matching `Date` in the TypeScript.

use legate_core::UsageShows;
use time::{Date, Duration, Month, OffsetDateTime, UtcOffset};

pub const RESET_DAYS_AHEAD: i64 = 32;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetChoice {
    pub value: String,
    pub label: String,
}

pub fn ago_phrase(at: i64, now: i64) -> String {
    let minutes = ((now - at) / 60_000).max(0);
    if minutes < 1 {
        return "just now".into();
    }
    if minutes < 60 {
        return format!("{minutes} min ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} h ago");
    }
    let days = hours / 24;
    if days == 1 {
        "1 day ago".into()
    } else {
        format!("{days} days ago")
    }
}

pub fn usage_figure(used: f64, shows: UsageShows) -> i64 {
    let raw = ((1.0 - used) * 100.0).round();
    let left = if raw.is_finite() {
        raw.clamp(0.0, 100.0) as i64
    } else {
        0
    };
    if shows == UsageShows::Left {
        left
    } else {
        100 - left
    }
}

pub fn updated_phrase(at: i64, now: i64) -> String {
    let minutes = ((now - at) / 60_000).max(0);
    if minutes < 1 {
        return "updated just now".into();
    }
    if minutes < 60 {
        return format!("updated {minutes} min ago");
    }
    format!("updated {} h ago", minutes / 60)
}

pub fn date_value(ms: i64) -> String {
    let local = to_local(ms);
    format!(
        "{:04}-{:02}-{:02}",
        local.year(),
        u8::from(local.month()),
        local.day()
    )
}

pub fn date_from_value(value: &str) -> Option<i64> {
    let mut parts = value.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || value.len() != 10 {
        return None;
    }
    let date = Date::from_calendar_date(year, Month::try_from(month).ok()?, day).ok()?;
    Some(local_midnight(date)?.unix_timestamp() * 1000)
}

pub fn days_between(from_ms: i64, to_ms: i64) -> i64 {
    let from = to_local(from_ms).date();
    let to = to_local(to_ms).date();
    let start = from.midnight().assume_utc().unix_timestamp();
    let end = to.midnight().assume_utc().unix_timestamp();
    (end - start).div_euclid(86_400)
}

pub fn reset_label(ms: i64, now: i64) -> String {
    let date = to_local(ms);
    let left = days_between(now, ms);
    let month = MONTHS[usize::from(u8::from(date.month()) - 1)];
    format!("{month} {} ({left}d)", date.day())
}

pub fn month_title(year: i32, month: u8) -> String {
    format!("{} {year}", MONTH_NAMES[usize::from(month)])
}

pub fn month_weeks(year: i32, month: u8) -> Vec<Vec<Option<u8>>> {
    let first = Date::from_calendar_date(
        year,
        Month::try_from(month + 1).unwrap_or(Month::January),
        1,
    )
    .unwrap_or(Date::MIN);
    let lead = usize::from(first.weekday().number_days_from_sunday());
    let length = first.month().length(year);
    let mut cells = Vec::new();
    cells.extend(std::iter::repeat_n(None, lead));
    for day in 1..=length {
        cells.push(Some(day));
    }
    while cells.len() % 7 != 0 {
        cells.push(None);
    }
    cells.chunks(7).map(<[Option<u8>]>::to_vec).collect()
}

pub fn reset_choices(now: i64, days: i64) -> Vec<ResetChoice> {
    let today = to_local(now).date();
    let mut choices = Vec::new();
    for ahead in 1..=days {
        let Some(date) = today.checked_add(Duration::days(ahead)) else {
            continue;
        };
        let Some(midnight) = local_midnight(date) else {
            continue;
        };
        let ms = midnight.unix_timestamp() * 1000;
        choices.push(ResetChoice {
            value: date_value(ms),
            label: reset_label(ms, now),
        });
    }
    choices
}

fn to_local(ms: i64) -> OffsetDateTime {
    let Ok(utc) = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    utc.to_offset(offset_at(utc))
}

fn offset_at(moment: OffsetDateTime) -> UtcOffset {
    UtcOffset::local_offset_at(moment).unwrap_or(UtcOffset::UTC)
}

fn local_midnight(date: Date) -> Option<OffsetDateTime> {
    let utc_midnight = date.with_hms(0, 0, 0).ok()?.assume_utc();
    let offset = offset_at(utc_midnight);
    let candidate = date.with_hms(0, 0, 0).ok()?.assume_offset(offset);
    let local = candidate.to_offset(offset);
    if local.date() == date && local.hour() == 0 {
        return Some(candidate);
    }
    let offset = offset_at(candidate);
    Some(date.with_hms(0, 0, 0).ok()?.assume_offset(offset))
}

#[cfg(test)]
mod tests {
    use super::{
        ago_phrase, date_from_value, date_value, days_between, month_title, month_weeks,
        reset_choices, updated_phrase, usage_figure,
    };
    use legate_core::UsageShows;

    fn local_ms(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> i64 {
        let date = time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day)
            .unwrap();
        let midnight = super::local_midnight(date).unwrap();
        midnight.unix_timestamp() * 1000 + i64::from(hour) * 3_600_000 + i64::from(minute) * 60_000
    }

    #[test]
    fn how_long_ago_the_usage_was_read() {
        let now = 10 * 3_600_000;
        assert_eq!(updated_phrase(now - 20_000, now), "updated just now");
        assert_eq!(updated_phrase(now - 4 * 60_000, now), "updated 4 min ago");
        assert_eq!(
            updated_phrase(now - 2 * 3_600_000 - 60_000, now),
            "updated 2 h ago"
        );
        assert_eq!(updated_phrase(now + 60_000, now), "updated just now");
    }

    #[test]
    fn how_old_a_dashboard_reading_is() {
        let now = 10 * 24 * 3_600_000;
        assert_eq!(ago_phrase(now - 20_000, now), "just now");
        assert_eq!(ago_phrase(now - 4 * 60_000, now), "4 min ago");
        assert_eq!(ago_phrase(now - 23 * 3_600_000, now), "23 h ago");
        assert_eq!(ago_phrase(now - 25 * 3_600_000, now), "1 day ago");
        assert_eq!(ago_phrase(now - 3 * 24 * 3_600_000, now), "3 days ago");
    }

    #[test]
    fn date_input_values_round_trip_through_local_midnight() {
        let nov5 = local_ms(2026, 11, 5, 0, 0);
        assert_eq!(date_value(nov5), "2026-11-05");
        assert_eq!(date_value(local_ms(2026, 11, 5, 23, 59)), "2026-11-05");
        assert_eq!(date_from_value("2026-11-05"), Some(nov5));
        assert_eq!(date_from_value(""), None);
        assert_eq!(date_from_value("5 Nov"), None);
        assert_eq!(date_from_value("2026-02-30"), None);
    }

    #[test]
    fn reset_choices_run_from_tomorrow() {
        let now = local_ms(2026, 10, 21, 15, 30);
        let choices = reset_choices(now, 32);
        assert_eq!(choices.len(), 32);
        assert_eq!(choices[0].value, "2026-10-22");
        assert_eq!(choices[0].label, "Oct 22 (1d)");
        assert_eq!(choices[13].value, "2026-11-04");
        assert_eq!(choices[13].label, "Nov 4 (14d)");
        assert_eq!(choices[31].value, "2026-11-22");
        assert_eq!(choices[31].label, "Nov 22 (32d)");
        let year_end = reset_choices(local_ms(2026, 12, 31, 0, 0), 1);
        assert_eq!(year_end[0].value, "2027-01-01");
        assert_eq!(year_end[0].label, "Jan 1 (1d)");
    }

    #[test]
    fn days_between_counts_calendar_days() {
        let late = local_ms(2026, 10, 21, 23, 59);
        assert_eq!(days_between(late, local_ms(2026, 11, 4, 0, 1)), 14);
        assert_eq!(days_between(late, late), 0);
        assert_eq!(
            days_between(local_ms(2026, 3, 1, 0, 0), local_ms(2026, 4, 1, 0, 0)),
            31
        );
    }

    #[test]
    fn a_month_as_calendar_weeks_sunday_first() {
        let nov = month_weeks(2026, 10);
        assert_eq!(
            nov[0],
            vec![
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7)
            ]
        );
        assert_eq!(
            nov.last().cloned().unwrap_or_default(),
            vec![Some(29), Some(30), None, None, None, None, None]
        );
        assert_eq!(
            month_weeks(2026, 9)[0],
            vec![None, None, None, None, Some(1), Some(2), Some(3)]
        );
        let days = month_weeks(2026, 9)
            .into_iter()
            .flatten()
            .filter(|day| day.is_some())
            .count();
        assert_eq!(days, 31);
        assert_eq!(month_title(2026, 10), "November 2026");
    }

    #[test]
    fn a_plan_reads_as_left_or_used() {
        assert_eq!(usage_figure(0.18, UsageShows::Left), 82);
        assert_eq!(usage_figure(0.18, UsageShows::Used), 18);
        assert_eq!(
            usage_figure(0.775, UsageShows::Left) + usage_figure(0.775, UsageShows::Used),
            100
        );
        assert_eq!(usage_figure(1.2, UsageShows::Left), 0);
        assert_eq!(usage_figure(1.2, UsageShows::Used), 100);
        assert_eq!(usage_figure(-0.1, UsageShows::Used), 0);
        assert_eq!(usage_figure(f64::NAN, UsageShows::Left), 0);
    }
}
