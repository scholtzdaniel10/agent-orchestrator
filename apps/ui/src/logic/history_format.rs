//! Commit ages and ref badges. Port of `history-format.ts`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Head,
    Tag,
    Branch,
}

impl RefKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Tag => "tag",
            Self::Branch => "branch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefBadge {
    pub name: String,
    pub kind: RefKind,
}

/// "now", "5m", "3h", "2d", "4mo": how long ago a commit was made.
/// `time` and `now` are epoch milliseconds.
pub fn ago(time: i64, now: i64) -> String {
    let minutes = now.saturating_sub(time).max(0) / 60_000;
    if minutes < 1 {
        return "now".to_string();
    }
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 60 {
        return format!("{days}d");
    }
    format!("{}mo", days / 30)
}

/// "HEAD -> main" → main (current), "tag: v1" → v1 (tag), "origin/HEAD" left out.
pub fn ref_badges(refs: &[String]) -> Vec<RefBadge> {
    let mut out = Vec::new();
    for r#ref in refs {
        if r#ref == "HEAD" {
            continue;
        }
        if let Some(name) = r#ref.strip_prefix("HEAD -> ") {
            out.push(RefBadge {
                name: name.to_string(),
                kind: RefKind::Head,
            });
        } else if let Some(name) = r#ref.strip_prefix("tag: ") {
            out.push(RefBadge {
                name: name.to_string(),
                kind: RefKind::Tag,
            });
        } else if !r#ref.ends_with("/HEAD") {
            out.push(RefBadge {
                name: r#ref.clone(),
                kind: RefKind::Branch,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{RefKind, ago, ref_badges};

    #[test]
    fn ago_counts_minutes_hours_days_and_months() {
        let now = 100 * 86_400_000;
        assert_eq!(ago(now - 10_000, now), "now");
        assert_eq!(ago(now - 5 * 60_000, now), "5m");
        assert_eq!(ago(now - 3 * 3_600_000, now), "3h");
        assert_eq!(ago(now - 2 * 86_400_000, now), "2d");
        assert_eq!(ago(now - 90 * 86_400_000, now), "3mo");
    }

    #[test]
    fn ref_badges_mark_the_current_branch_and_tags_and_drop_head_pointers() {
        let refs = [
            "HEAD -> main".to_string(),
            "origin/main".to_string(),
            "origin/HEAD".to_string(),
            "tag: v1.1.1".to_string(),
            "HEAD".to_string(),
        ];
        let badges = ref_badges(&refs);
        assert_eq!(badges.len(), 3);
        assert_eq!(badges[0].name, "main");
        assert_eq!(badges[0].kind, RefKind::Head);
        assert_eq!(badges[1].name, "origin/main");
        assert_eq!(badges[1].kind, RefKind::Branch);
        assert_eq!(badges[2].name, "v1.1.1");
        assert_eq!(badges[2].kind, RefKind::Tag);
    }
}
