//! Commit-graph lanes, newest first. Port of `commit-graph.ts`.

/// A line segment in one half of a graph row, from lane `from` to lane `to`, coloured by `lane`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLine {
    pub from: i64,
    pub to: i64,
    pub lane: i64,
}

/// What a row of the commit graph draws: lines from above, its dot, and lines going down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    /// Lane of this commit's dot.
    pub col: i64,
    /// Top half: lines that end at the dot, and lines passing by.
    pub top: Vec<GraphLine>,
    /// Bottom half: lines from the dot to its parents, and lines passing by.
    pub bottom: Vec<GraphLine>,
    /// Lanes in use in this row.
    pub width: i64,
}

/// One commit in newest-first order. Parents are hashes of the parent commits.
#[derive(Debug, Clone, Copy)]
pub struct CommitNode<'a> {
    pub hash: &'a str,
    pub parents: &'a [&'a str],
}

/// Lays commits out in lanes, newest first, as VS Code and `git log --graph` do: a commit takes the
/// lane that expected it, its first parent stays in that lane, and other parents take free lanes.
/// Lanes never shift sideways, so a passing line is always straight.
pub fn graph_rows(commits: &[CommitNode<'_>]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let col = index_of(&lanes, commit.hash)
            .unwrap_or_else(|| index_of_none(&lanes).unwrap_or(lanes.len()));
        let mut before = lanes.clone();
        while before.len() <= col {
            before.push(None);
        }

        let mut top = Vec::new();
        for (lane, hash) in before.iter().enumerate() {
            let Some(hash) = hash else {
                continue;
            };
            if hash == commit.hash {
                top.push(line(lane, col, lane));
            } else {
                top.push(line(lane, lane, lane));
            }
        }

        let mut after: Vec<Option<String>> = before
            .iter()
            .map(|hash| {
                if hash.as_deref() == Some(commit.hash) {
                    None
                } else {
                    hash.clone()
                }
            })
            .collect();
        let mut joins = Vec::new();
        if let Some((first, others)) = commit.parents.split_first() {
            match index_of(&after, first) {
                None => after[col] = Some((*first).to_string()),
                Some(waiting) if waiting > col => {
                    after[waiting] = None;
                    after[col] = Some((*first).to_string());
                    joins.push(line(waiting, col, waiting));
                }
                Some(_) => {}
            }
            for parent in others {
                if index_of(&after, parent).is_some() {
                    continue;
                }
                match index_of_none(&after) {
                    Some(free) => after[free] = Some((*parent).to_string()),
                    None => after.push(Some((*parent).to_string())),
                }
            }
        }

        let mut bottom = Vec::new();
        for (lane, hash) in after.iter().enumerate() {
            let Some(hash) = hash else {
                continue;
            };
            if before.get(lane).and_then(|item| item.as_ref()) == Some(hash) {
                bottom.push(line(lane, lane, lane));
            }
        }
        for parent in commit.parents {
            if let Some(lane) = index_of(&after, parent) {
                let colour = if lane == col { col } else { lane };
                bottom.push(line(col, lane, colour));
            }
        }
        bottom.extend(joins);

        let width = i64::try_from(before.len().max(after.len()).max(col + 1)).unwrap_or(i64::MAX);
        while after.last().is_some_and(|slot| slot.is_none()) {
            after.pop();
        }
        lanes = after;
        let col = i64::try_from(col).unwrap_or(i64::MAX);
        rows.push(GraphRow {
            col,
            top,
            bottom,
            width,
        });
    }
    rows
}

fn line(from: usize, to: usize, lane: usize) -> GraphLine {
    GraphLine {
        from: i64::try_from(from).unwrap_or(i64::MAX),
        to: i64::try_from(to).unwrap_or(i64::MAX),
        lane: i64::try_from(lane).unwrap_or(i64::MAX),
    }
}

fn index_of(lanes: &[Option<String>], hash: &str) -> Option<usize> {
    lanes.iter().position(|slot| slot.as_deref() == Some(hash))
}

fn index_of_none(lanes: &[Option<String>]) -> Option<usize> {
    lanes.iter().position(|slot| slot.is_none())
}

#[cfg(test)]
mod tests {
    use super::{CommitNode, GraphLine, graph_rows};

    fn straight(from: i64, to: i64, lane: i64) -> GraphLine {
        GraphLine { from, to, lane }
    }

    #[test]
    fn a_straight_history_stays_in_one_lane() {
        let rows = graph_rows(&[
            CommitNode {
                hash: "c",
                parents: &["b"],
            },
            CommitNode {
                hash: "b",
                parents: &["a"],
            },
            CommitNode {
                hash: "a",
                parents: &[],
            },
        ]);
        assert_eq!(
            rows.iter().map(|row| row.col).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(rows[0].top.is_empty());
        assert_eq!(rows[0].bottom, vec![straight(0, 0, 0)]);
        assert_eq!(rows[1].top, vec![straight(0, 0, 0)]);
        assert!(rows[2].bottom.is_empty());
    }

    #[test]
    fn a_merge_opens_a_second_lane_that_closes_at_the_fork_point() {
        let rows = graph_rows(&[
            CommitNode {
                hash: "m",
                parents: &["b", "f"],
            },
            CommitNode {
                hash: "f",
                parents: &["a"],
            },
            CommitNode {
                hash: "b",
                parents: &["a"],
            },
            CommitNode {
                hash: "a",
                parents: &[],
            },
        ]);
        assert_eq!(
            rows.iter().map(|row| row.col).collect::<Vec<_>>(),
            vec![0, 1, 0, 0]
        );
        assert_eq!(rows[0].bottom, vec![straight(0, 0, 0), straight(0, 1, 1)]);
        assert_eq!(rows[1].top, vec![straight(0, 0, 0), straight(1, 1, 1)]);
        assert_eq!(rows[1].bottom, vec![straight(0, 0, 0), straight(1, 1, 1)]);
        assert_eq!(rows[2].bottom, vec![straight(0, 0, 0), straight(1, 0, 1)]);
        assert_eq!(rows[3].top, vec![straight(0, 0, 0)]);
        assert!(rows.iter().all(|row| row.width <= 2));
    }

    #[test]
    fn a_commit_nobody_expected_starts_a_lane_of_its_own() {
        let rows = graph_rows(&[
            CommitNode {
                hash: "x",
                parents: &["a"],
            },
            CommitNode {
                hash: "y",
                parents: &["a"],
            },
            CommitNode {
                hash: "a",
                parents: &[],
            },
        ]);
        assert_eq!(
            rows.iter().map(|row| row.col).collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        assert_eq!(rows[1].top, vec![straight(0, 0, 0)]);
        assert_eq!(rows[1].bottom, vec![straight(0, 0, 0), straight(1, 0, 0)]);
    }
}
