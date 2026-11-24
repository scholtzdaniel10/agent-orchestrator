//! Match words across the saved text of a single chat or job.

use std::cmp::Reverse;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{JobRecord, LeadMessage};

pub const DEFAULT_SEARCH_LIMIT: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchKind {
    Chat,
    Job,
}

impl SearchKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Job => "job",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetPart {
    pub text: String,
    pub marked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub kind: SearchKind,
    pub id: String,
    pub title: String,
    pub snippet: Vec<SnippetPart>,
    pub at: i64,
    pub message_id: Option<String>,
}

#[derive(Debug)]
pub struct SearchQuery {
    words: Vec<String>,
    like_word: Option<String>,
}

impl SearchQuery {
    pub fn new(query: &str) -> Self {
        let mut words = query.split_whitespace().map(fold).collect::<Vec<_>>();
        words.sort_unstable();
        words.dedup();
        let like_word = query
            .split_whitespace()
            .filter(|word| {
                word.bytes()
                    .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'\'' | b'"' | b'\\'))
            })
            .max_by_key(|word| word.len())
            .map(str::to_ascii_lowercase);
        Self { words, like_word }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// The longest word SQLite can use as a conservative prefilter.
    pub fn like_word(&self) -> Option<&str> {
        self.like_word.as_deref()
    }

    pub fn chat(
        &self,
        id: &str,
        title: &str,
        at: i64,
        messages: &[LeadMessage],
    ) -> Option<SearchHit> {
        let mut matched = Matches::new(self);
        matched.field(title, None, true);
        for message in messages {
            let id = Some(message.id.as_str());
            matched.field(&message.text, id, true);
            if let Some(plan) = &message.proposal {
                matched.field(&plan.summary, id, true);
                for job in &plan.jobs {
                    matched.field(&job.title, id, true);
                    matched.field(&job.prompt, id, true);
                }
            }
        }
        Some(SearchHit {
            kind: SearchKind::Chat,
            id: id.into(),
            title: title.into(),
            message_id: matched.best_message.map(|(id, _)| id.into()),
            snippet: matched.snippet()?,
            at,
        })
    }

    pub fn job(&self, job: &JobRecord, at: i64) -> Option<SearchHit> {
        let title = job
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty());
        let mut matched = Matches::new(self);
        if let Some(title) = title {
            matched.field(title, None, true);
            matched.field(&job.prompt, None, true);
            matched.field(&job.output, None, true);
        } else {
            let output_matches = matched.field(&job.output, None, true);
            matched.field(&job.prompt, None, !output_matches);
        }
        if let Some(error) = &job.error {
            matched.field(error, None, true);
        }
        for step in job.steps.iter().flatten() {
            matched.field(&step.title, None, true);
            if let Some(detail) = &step.detail {
                matched.field(detail, None, true);
            }
        }
        Some(SearchHit {
            kind: SearchKind::Job,
            id: job.id.clone(),
            title: title.unwrap_or("Untitled job").into(),
            snippet: matched.snippet()?,
            at,
            message_id: None,
        })
    }
}

pub fn sort_hits(hits: &mut [SearchHit]) {
    hits.sort_by_key(|hit| Reverse(hit.at));
}

struct Matches<'query, 'field> {
    query: &'query SearchQuery,
    seen: Vec<bool>,
    best: Option<(&'field str, usize)>,
    best_message: Option<(&'field str, usize)>,
}

impl<'query, 'field> Matches<'query, 'field> {
    fn new(query: &'query SearchQuery) -> Self {
        Self {
            query,
            seen: vec![false; query.words.len()],
            best: None,
            best_message: None,
        }
    }

    fn field(&mut self, text: &'field str, message: Option<&'field str>, snippet: bool) -> bool {
        let folded = fold(text);
        let mut score = 0;
        for (word, seen) in self.query.words.iter().zip(&mut self.seen) {
            if folded.contains(word) {
                *seen = true;
                score += 1;
            }
        }
        if score == 0 {
            return false;
        }
        if snippet && self.best.is_none_or(|(_, best)| score > best) {
            self.best = Some((text, score));
        }
        if let Some(id) = message
            && self.best_message.is_none_or(|(_, best)| score > best)
        {
            self.best_message = Some((id, score));
        }
        true
    }

    fn snippet(&self) -> Option<Vec<SnippetPart>> {
        if self.seen.iter().all(|seen| *seen) {
            let (text, _) = self.best?;
            return Some(snippet(text, &self.query.words));
        }
        None
    }
}

fn lower(ch: char) -> std::char::ToLowercase {
    // Fold the final and medial forms of Greek sigma together.
    if ch == 'ς' { 'σ' } else { ch }.to_lowercase()
}

fn fold(text: &str) -> String {
    text.chars().flat_map(lower).collect()
}

fn snippet(text: &str, words: &[String]) -> Vec<SnippetPart> {
    const BEFORE: usize = 40;
    const WIDTH: usize = 160;

    let mut folded = String::new();
    let mut offsets = Vec::new();
    for (offset, ch) in text.char_indices() {
        offsets.push((folded.len(), offset));
        folded.extend(lower(ch));
    }
    offsets.push((folded.len(), text.len()));

    // Lowercasing can expand a character. Keep snippet slices on the original UTF-8 boundaries.
    let mut ranges = words
        .iter()
        .flat_map(|word| {
            folded.match_indices(word).map(|(start, part)| {
                let first = offsets.partition_point(|&(byte, _)| byte <= start) - 1;
                let end = offsets.partition_point(|&(byte, _)| byte < start + part.len());
                first..end
            })
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable_by_key(|range| range.start);

    let first = &ranges[0];
    let start = first.start.saturating_sub(BEFORE);
    let end = (start + WIDTH).max(first.end).min(offsets.len() - 1);
    let mut parts = Vec::new();
    let mut cursor = start;
    let mut marked: Option<Range<usize>> = None;
    let slice = |from: usize, to: usize| &text[offsets[from].1..offsets[to].1];
    let mut push = |value: &str, marked| {
        if !value.is_empty() {
            parts.push(SnippetPart {
                text: value.into(),
                marked,
            });
        }
    };

    if start > 0 {
        push("…", false);
    }
    for mut range in ranges.into_iter().take_while(|range| range.start < end) {
        range.end = range.end.min(end);
        if let Some(previous) = &mut marked
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
            continue;
        }
        if let Some(previous) = marked.replace(range) {
            push(slice(cursor, previous.start), false);
            push(slice(previous.start, previous.end), true);
            cursor = previous.end;
        }
    }
    if let Some(previous) = marked {
        push(slice(cursor, previous.start), false);
        push(slice(previous.start, previous.end), true);
        cursor = previous.end;
    }
    push(slice(cursor, end), false);
    if end < offsets.len() - 1 {
        push("…", false);
    }
    parts
}

#[cfg(test)]
mod tests;
