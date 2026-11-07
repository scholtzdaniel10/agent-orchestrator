//! Placeholder for `commit-graph.ts`. The history slice fills this in.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLine {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub lane: i64,
    pub lines: Vec<GraphLine>,
}

pub fn graph_rows(hashes: &[String]) -> Vec<GraphRow> {
    let _ = hashes;
    Vec::new()
}
