//! Placeholder for `history-format.ts`. The history slice fills this in.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefBadge {
    pub kind: String,
    pub name: String,
}

pub fn ago(time: i64, now: i64) -> String {
    let _ = (time, now);
    String::new()
}

pub fn ref_badges(refs: &[String]) -> Vec<RefBadge> {
    refs.iter()
        .map(|name| RefBadge {
            kind: "ref".into(),
            name: name.clone(),
        })
        .collect()
}
