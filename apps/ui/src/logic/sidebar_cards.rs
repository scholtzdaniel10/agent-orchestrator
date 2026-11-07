//! Sidebar worktree cards. Port of `sidebar-cards.ts`.

use std::collections::HashMap;

use serde_json::Value;

pub const WORKING_MS: i64 = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    Working,
    Unread,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    None,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Created,
    Recent,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardLayout {
    Detailed,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarOptions {
    pub group_by: GroupBy,
    pub sort_by: SortBy,
    pub layout: CardLayout,
    pub hide_clean: bool,
}

pub const DEFAULT_OPTIONS: SidebarOptions = SidebarOptions {
    group_by: GroupBy::None,
    sort_by: SortBy::Created,
    layout: CardLayout::Detailed,
    hide_clean: false,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardInput {
    pub key: Option<String>,
    pub title: String,
    pub branch: Option<String>,
    pub terminal_ids: Vec<String>,
    pub job_running: bool,
    pub has_changes: bool,
    pub created_index: i64,
    pub pinned: bool,
    pub seen_at: i64,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub input: CardInput,
    pub status: CardStatus,
    pub last_active: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardGroup {
    pub label: Option<&'static str>,
    pub cards: Vec<Card>,
}

pub fn card_status(input: CardInput, activity: &HashMap<String, i64>, now: i64) -> Card {
    let mut last_active = 0;
    for id in &input.terminal_ids {
        last_active = last_active.max(activity.get(id).copied().unwrap_or(0));
    }
    let status = if input.job_running || (last_active > 0 && now - last_active < WORKING_MS) {
        CardStatus::Working
    } else if !input.selected && last_active > input.seen_at {
        CardStatus::Unread
    } else {
        CardStatus::Idle
    };
    Card {
        input,
        status,
        last_active,
    }
}

pub fn arrange_cards(
    mut cards: Vec<Card>,
    options: SidebarOptions,
    filter: &str,
) -> Vec<CardGroup> {
    let needle = filter.trim().to_lowercase();
    cards.retain(|card| {
        if !needle.is_empty() {
            let branch = card.input.branch.as_deref().unwrap_or("");
            let hay = format!("{} {}", card.input.title, branch).to_lowercase();
            if !hay.contains(&needle) {
                return false;
            }
        }
        if !options.hide_clean
            || card.input.key.is_none()
            || card.input.selected
            || card.input.pinned
        {
            return true;
        }
        card.input.has_changes
            || !card.input.terminal_ids.is_empty()
            || card.status != CardStatus::Idle
    });
    cards.sort_by(|a, b| {
        if options.sort_by == SortBy::Name {
            return a.input.title.cmp(&b.input.title);
        }
        if options.sort_by == SortBy::Recent && a.last_active != b.last_active {
            return b.last_active.cmp(&a.last_active);
        }
        a.input.created_index.cmp(&b.input.created_index)
    });
    let pinned: Vec<Card> = cards
        .iter()
        .filter(|card| card.input.pinned || card.input.key.is_none())
        .cloned()
        .collect();
    let rest: Vec<Card> = cards
        .into_iter()
        .filter(|card| !card.input.pinned && card.input.key.is_some())
        .collect();
    let mut groups = Vec::new();
    if !pinned.is_empty() {
        groups.push(CardGroup {
            label: Some("Pinned"),
            cards: pinned,
        });
    }
    if options.group_by == GroupBy::Status {
        type CardTest = fn(&Card) -> bool;
        let buckets: [(&str, CardTest); 4] = [
            ("In progress", |card| card.status == CardStatus::Working),
            ("Needs you", |card| card.status == CardStatus::Unread),
            ("Ready to review", |card| {
                card.status == CardStatus::Idle && card.input.has_changes
            }),
            ("Quiet", |card| {
                card.status == CardStatus::Idle && !card.input.has_changes
            }),
        ];
        for (label, test) in buckets {
            let in_group: Vec<Card> = rest.iter().filter(|card| test(card)).cloned().collect();
            if !in_group.is_empty() {
                groups.push(CardGroup {
                    label: Some(label),
                    cards: in_group,
                });
            }
        }
    } else if !rest.is_empty() {
        groups.push(CardGroup {
            label: if groups.is_empty() {
                None
            } else {
                Some("Worktrees")
            },
            cards: rest,
        });
    }
    groups
}

pub fn read_options(raw: &Value) -> SidebarOptions {
    let Some(value) = raw.as_object() else {
        return DEFAULT_OPTIONS;
    };
    SidebarOptions {
        group_by: if value.get("groupBy").and_then(Value::as_str) == Some("status") {
            GroupBy::Status
        } else {
            GroupBy::None
        },
        sort_by: match value.get("sortBy").and_then(Value::as_str) {
            Some("recent") => SortBy::Recent,
            Some("name") => SortBy::Name,
            _ => SortBy::Created,
        },
        layout: if value.get("layout").and_then(Value::as_str) == Some("compact") {
            CardLayout::Compact
        } else {
            CardLayout::Detailed
        },
        hide_clean: value.get("hideClean").and_then(Value::as_bool) == Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CardInput, CardLayout, CardStatus, DEFAULT_OPTIONS, GroupBy, SidebarOptions, SortBy,
        WORKING_MS, arrange_cards, card_status, read_options,
    };
    use serde_json::{Value, json};
    use std::collections::HashMap;

    fn input(over: CardInput) -> CardInput {
        over
    }

    fn base() -> CardInput {
        CardInput {
            key: Some("aaaa0001".into()),
            title: "card".into(),
            branch: None,
            terminal_ids: Vec::new(),
            job_running: false,
            has_changes: false,
            created_index: 0,
            pinned: false,
            seen_at: 0,
            selected: false,
        }
    }

    #[test]
    fn status_recent_output_or_a_running_job_is_working() {
        let now = 100_000;
        let activity = HashMap::from([
            ("t1".into(), now - 1000),
            ("t2".into(), now - WORKING_MS - 1),
        ]);
        let mut with_t1 = base();
        with_t1.terminal_ids = vec!["t1".into()];
        assert_eq!(
            card_status(with_t1, &activity, now).status,
            CardStatus::Working
        );
        let mut running = base();
        running.job_running = true;
        assert_eq!(
            card_status(running, &activity, now).status,
            CardStatus::Working
        );
        let mut with_t2 = base();
        with_t2.terminal_ids = vec!["t2".into()];
        assert_eq!(
            card_status(with_t2.clone(), &activity, now).status,
            CardStatus::Unread
        );
        let mut seen = with_t2.clone();
        seen.seen_at = now;
        assert_eq!(card_status(seen, &activity, now).status, CardStatus::Idle);
        let mut selected = with_t2.clone();
        selected.selected = true;
        assert_eq!(
            card_status(selected, &activity, now).status,
            CardStatus::Idle
        );
        assert_eq!(
            card_status(with_t2, &activity, now).last_active,
            activity["t2"]
        );
    }

    fn titles(groups: &[super::CardGroup]) -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|group| {
                group
                    .cards
                    .iter()
                    .map(|card| card.input.title.clone())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn arrange_filters_hides_sorts_and_groups() {
        let activity = HashMap::from([("t".into(), 50)]);
        let now = 10_000;
        let mut main = base();
        main.key = None;
        main.title = "main".into();
        let mut beta = base();
        beta.key = Some("b".into());
        beta.title = "beta".into();
        beta.created_index = 1;
        beta.has_changes = true;
        let mut alpha = base();
        alpha.key = Some("a".into());
        alpha.title = "alpha".into();
        alpha.created_index = 2;
        alpha.terminal_ids = vec!["t".into()];
        let mut clean = base();
        clean.key = Some("c".into());
        clean.title = "clean".into();
        clean.created_index = 3;
        let mut pinned = base();
        pinned.key = Some("p".into());
        pinned.title = "pinned".into();
        pinned.created_index = 4;
        pinned.pinned = true;
        let cards = vec![
            card_status(main, &activity, now),
            card_status(beta, &activity, now),
            card_status(alpha, &activity, now),
            card_status(clean, &activity, now),
            card_status(pinned, &activity, now),
        ];
        assert_eq!(DEFAULT_OPTIONS.group_by, GroupBy::None);
        let status = SidebarOptions {
            group_by: GroupBy::Status,
            ..DEFAULT_OPTIONS
        };
        assert_eq!(
            titles(&arrange_cards(cards.clone(), status, "ALP")),
            vec![vec!["alpha"]]
        );
        let hidden = SidebarOptions {
            hide_clean: true,
            ..status
        };
        assert_eq!(
            titles(&arrange_cards(cards.clone(), hidden, "")),
            vec![vec!["main", "pinned"], vec!["alpha"], vec!["beta"]]
        );
        let by_name = SidebarOptions {
            sort_by: SortBy::Name,
            ..DEFAULT_OPTIONS
        };
        assert_eq!(
            titles(&arrange_cards(cards.clone(), by_name, "")),
            vec![vec!["main", "pinned"], vec!["alpha", "beta", "clean"]]
        );
        let recent = SidebarOptions {
            sort_by: SortBy::Recent,
            ..status
        };
        assert_eq!(
            titles(&arrange_cards(cards.clone(), recent, ""))[1][0],
            "alpha"
        );
        let by_status = arrange_cards(cards, status, "");
        assert_eq!(
            by_status
                .iter()
                .map(|group| group.label)
                .collect::<Vec<_>>(),
            vec![
                Some("Pinned"),
                Some("Needs you"),
                Some("Ready to review"),
                Some("Quiet")
            ]
        );
        let only = vec![card_status(
            {
                let mut card = base();
                card.key = Some("b".into());
                card.title = "beta".into();
                card.created_index = 1;
                card.has_changes = true;
                card
            },
            &activity,
            now,
        )];
        assert_eq!(arrange_cards(only, DEFAULT_OPTIONS, "")[0].label, None);

        let mut main_job = base();
        main_job.key = None;
        main_job.title = "main".into();
        main_job.job_running = true;
        let mut busy = base();
        busy.key = Some("w".into());
        busy.title = "busy".into();
        busy.created_index = 1;
        busy.job_running = true;
        let working = arrange_cards(
            vec![
                card_status(main_job, &HashMap::new(), now),
                card_status(busy, &HashMap::new(), now),
            ],
            status,
            "",
        );
        let labelled: Vec<(Option<&str>, Vec<String>)> = working
            .iter()
            .map(|group| {
                (
                    group.label,
                    group
                        .cards
                        .iter()
                        .map(|card| card.input.title.clone())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            labelled,
            vec![
                (Some("Pinned"), vec!["main".into()]),
                (Some("In progress"), vec!["busy".into()])
            ]
        );
    }

    #[test]
    fn read_options_falls_back_for_junk() {
        assert_eq!(read_options(&Value::Null), DEFAULT_OPTIONS);
        assert_eq!(
            read_options(
                &json!({ "groupBy": "status", "sortBy": "x", "layout": "compact", "hideClean": 1 })
            ),
            SidebarOptions {
                group_by: GroupBy::Status,
                sort_by: SortBy::Created,
                layout: CardLayout::Compact,
                hide_clean: false,
            }
        );
    }

    #[test]
    fn input_helper_keeps_the_shape() {
        let card = input(base());
        assert_eq!(card.title, "card");
    }
}
