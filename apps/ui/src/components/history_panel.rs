//! Commit log, graph, and per-file diff. Port of `HistoryPanel.tsx`.

use std::cell::Cell;
use std::rc::Rc;

use dioxus::prelude::*;
use js_sys::Date;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::JsValue;
use wasm_bindgen_futures::spawn_local;

use super::files_panel::{DiffBlock, bridge_error, document_element};
use crate::api;
use crate::logic::commit_graph::{CommitNode, GraphLine, GraphRow, graph_rows};
use crate::logic::file_tree::{FILE_WINDOW_THRESHOLD, window_range};
use crate::logic::history_format::{ago, ref_badges};
use crate::wire::{Commit, CommitDetail};

const ROW: f64 = 30.0;
const LANE: i64 = 12;
const PAD: i64 = 8;
const LANE_COLORS: [&str; 6] = [
    "var(--accent)",
    "var(--bot-claude)",
    "var(--bot-cursor)",
    "var(--signal)",
    "#6ba8ff",
    "#c45ad6",
];

#[derive(Clone, PartialEq, Eq)]
struct LogState {
    key: String,
    commits: Vec<Commit>,
    more: bool,
    at: i64,
}

#[derive(Clone, PartialEq, Eq)]
struct Failure {
    key: String,
    text: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Pick {
    key: String,
    hash: String,
}

#[derive(Clone, PartialEq, Eq)]
struct FilePick {
    hash: String,
    path: String,
}

#[derive(Clone, PartialEq, Eq)]
struct DiffPick {
    hash: String,
    path: String,
    text: String,
}

fn lane_color(lane: i64) -> &'static str {
    let index = usize::try_from(lane.rem_euclid(LANE_COLORS.len() as i64)).unwrap_or(0);
    LANE_COLORS[index]
}

fn lane_x(lane: i64) -> f64 {
    f64::from(i32::try_from(PAD + lane * LANE).unwrap_or(PAD as i32))
}

fn js_num(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn curve(line: &GraphLine, y1: f64, y2: f64) -> String {
    let x1 = lane_x(line.from);
    let x2 = lane_x(line.to);
    if x1 == x2 {
        format!("M{} {}V{}", js_num(x1), js_num(y1), js_num(y2))
    } else {
        let mid = (y1 + y2) / 2.0;
        format!(
            "M{} {}C{} {} {} {} {} {}",
            js_num(x1),
            js_num(y1),
            js_num(x1),
            js_num(mid),
            js_num(x2),
            js_num(mid),
            js_num(x2),
            js_num(y2)
        )
    }
}

fn layout_rows(commits: &[Commit]) -> Vec<GraphRow> {
    let parents: Vec<Vec<String>> = commits
        .iter()
        .map(|commit| commit.parents.clone())
        .collect();
    let parent_refs: Vec<Vec<&str>> = parents
        .iter()
        .map(|parent| parent.iter().map(String::as_str).collect())
        .collect();
    let nodes: Vec<CommitNode<'_>> = commits
        .iter()
        .zip(parent_refs.iter())
        .map(|(commit, parent)| CommitNode {
            hash: commit.hash.as_str(),
            parents: parent.as_slice(),
        })
        .collect();
    graph_rows(&nodes)
}

fn locale_time(ms: i64) -> String {
    let date = Date::new(&JsValue::from_f64(ms as f64));
    let Ok(method) = js_sys::Reflect::get(&date, &JsValue::from_str("toLocaleString")) else {
        return String::new();
    };
    let Some(fun) = method.dyn_ref::<js_sys::Function>() else {
        return String::new();
    };
    fun.call0(&date)
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn copy_text(text: &str) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let nav = win.navigator();
    let Ok(clipboard) = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")) else {
        return;
    };
    let Ok(write) = js_sys::Reflect::get(&clipboard, &JsValue::from_str("writeText")) else {
        return;
    };
    let Some(fun) = write.dyn_ref::<js_sys::Function>() else {
        return;
    };
    let _ = fun.call1(&clipboard, &JsValue::from_str(text));
}

fn is_head(refs: &[String]) -> bool {
    refs.iter()
        .any(|name| name == "HEAD" || name.starts_with("HEAD -> "))
}

#[component]
pub fn HistoryPanel(r#where: Option<String>, label: String, refresh_key: String) -> Element {
    let mut nonce = use_signal(|| 0_u32);
    let mut log = use_signal(|| None::<LogState>);
    let mut failure = use_signal(|| None::<Failure>);
    let paging = use_signal(|| false);
    let pick = use_signal(|| None::<Pick>);
    let mut loaded = use_signal(|| None::<CommitDetail>);
    let file_sel = use_signal(|| None::<FilePick>);
    let mut diff_sel = use_signal(|| None::<DiffPick>);
    let mut scroll_top = use_signal(|| 0.0_f64);
    let mut view_h = use_signal(|| 600.0_f64);
    let list_num = use_hook(next_list_id);
    let list_id = format!("history-list-{list_num}");
    let log_gen = use_hook(|| Rc::new(Cell::new(0_u64)));
    let detail_gen = use_hook(|| Rc::new(Cell::new(0_u64)));
    let diff_gen = use_hook(|| Rc::new(Cell::new(0_u64)));

    let key = format!(
        "{}|{}|{refresh_key}",
        r#where.clone().unwrap_or_default(),
        nonce()
    );
    let mut key_sig = use_signal(String::new);
    if key_sig.peek().as_str() != key {
        key_sig.set(key.clone());
    }
    let mut where_sig = use_signal(|| r#where.clone());
    if where_sig.peek().as_ref() != r#where.as_ref() {
        where_sig.set(r#where.clone());
    }

    let log_ticket = log_gen.clone();
    use_effect(move || {
        let asked = key_sig();
        let place = where_sig();
        let generation = log_ticket.get().saturating_add(1);
        log_ticket.set(generation);
        let log_ticket = log_ticket.clone();
        spawn_local(async move {
            let result = api::commit_log(place.as_deref(), 0.0).await;
            if log_ticket.get() != generation {
                return;
            }
            match result {
                Ok(page) => log.set(Some(LogState {
                    key: asked,
                    commits: page.commits,
                    more: page.more,
                    at: Date::now() as i64,
                })),
                Err(err) => failure.set(Some(Failure {
                    key: asked,
                    text: bridge_error(&err),
                })),
            }
        });
    });

    let current = log.read().as_ref().filter(|item| item.key == key).cloned();
    let commits = current.as_ref().map(|item| item.commits.clone());
    let more = current.as_ref().is_some_and(|item| item.more);
    let now = current.as_ref().map(|item| item.at).unwrap_or(0);
    let error = failure
        .read()
        .as_ref()
        .filter(|item| item.key == key)
        .map(|item| item.text.clone());
    let selected = pick
        .read()
        .as_ref()
        .filter(|item| item.key == key)
        .map(|item| item.hash.clone());
    let detail = loaded
        .read()
        .as_ref()
        .filter(|item| {
            selected
                .as_ref()
                .is_some_and(|hash| item.hash.starts_with(hash))
        })
        .cloned();
    let file = file_sel
        .read()
        .as_ref()
        .filter(|item| Some(&item.hash) == selected.as_ref())
        .map(|item| item.path.clone());
    let diff = diff_sel
        .read()
        .as_ref()
        .filter(|item| Some(&item.hash) == selected.as_ref() && Some(&item.path) == file.as_ref())
        .map(|item| item.text.clone());

    let mut selected_sig = use_signal(|| None::<String>);
    if selected_sig.peek().as_ref() != selected.as_ref() {
        selected_sig.set(selected.clone());
    }
    let mut file_sig = use_signal(|| None::<String>);
    if file_sig.peek().as_ref() != file.as_ref() {
        file_sig.set(file.clone());
    }

    let detail_ticket = detail_gen.clone();
    use_effect(move || {
        let Some(hash) = selected_sig() else {
            return;
        };
        let asked = key_sig();
        let place = where_sig();
        let generation = detail_ticket.get().saturating_add(1);
        detail_ticket.set(generation);
        let detail_ticket = detail_ticket.clone();
        spawn_local(async move {
            let result = api::commit_detail(place.as_deref(), &hash).await;
            if detail_ticket.get() != generation {
                return;
            }
            match result {
                Ok(value) => loaded.set(Some(value)),
                Err(err) => failure.set(Some(Failure {
                    key: asked,
                    text: bridge_error(&err),
                })),
            }
        });
    });

    let diff_ticket = diff_gen.clone();
    use_effect(move || {
        let Some(hash) = selected_sig() else {
            return;
        };
        let Some(path) = file_sig() else {
            return;
        };
        let asked = key_sig();
        let place = where_sig();
        let generation = diff_ticket.get().saturating_add(1);
        diff_ticket.set(generation);
        let diff_ticket = diff_ticket.clone();
        spawn_local(async move {
            let result = api::commit_file_diff(place.as_deref(), &hash, &path).await;
            if diff_ticket.get() != generation {
                return;
            }
            match result {
                Ok(text) => diff_sel.set(Some(DiffPick { hash, path, text })),
                Err(err) => failure.set(Some(Failure {
                    key: asked,
                    text: bridge_error(&err),
                })),
            }
        });
    });

    let rows = use_memo(move || {
        let asked = key_sig();
        log.read()
            .as_ref()
            .filter(|item| item.key == asked)
            .map(|item| layout_rows(&item.commits))
            .unwrap_or_default()
    });
    let row_guard = rows.read();
    let width = row_guard
        .iter()
        .map(|row| row.width)
        .max()
        .unwrap_or(1)
        .max(1);
    let row_len = row_guard.len();
    let count = commits.as_ref().map(|items| items.len()).unwrap_or(0) as i64;
    let range = window_range(count, scroll_top(), view_h(), ROW);
    let start = usize::try_from(range.start.max(0)).unwrap_or(0);
    let end = usize::try_from(range.end.max(0)).unwrap_or(0);
    let start = start.min(count.max(0) as usize);
    let end = end.min(count.max(0) as usize).max(start);
    let loading = commits.is_none() || paging();
    let split = selected.is_some();
    let list_class = if split {
        "history-list is-split"
    } else {
        "history-list"
    };
    let count_label = commits.as_ref().map(|items| {
        let extra = if more { "+" } else { "" };
        format!("{}{extra} commits", items.len())
    });
    let top_gap = (start as f64) * ROW;
    let bottom_gap = ((count as usize).saturating_sub(end) as f64) * ROW;
    let windowed = count > FILE_WINDOW_THRESHOLD;
    let slice_commits = commits
        .as_ref()
        .map(|items| items[start..end].to_vec())
        .unwrap_or_default();
    let row_start = start.min(row_len);
    let row_end = end.min(row_len).max(row_start);
    let slice_rows = row_guard[row_start..row_end].to_vec();
    drop(row_guard);
    let count_for_scroll = count;
    let more_key = key.clone();
    let more_place = r#where.clone();

    let mut watch_count = use_signal(|| 0_i64);
    if *watch_count.peek() != count {
        watch_count.set(count);
    }
    let measure_id = list_id.clone();
    use_effect(move || {
        let count = watch_count();
        let Some(el) = document_element(&measure_id) else {
            return;
        };
        let top = f64::from(el.scroll_top());
        let view = f64::from(el.client_height());
        if view < 1.0 {
            return;
        }
        let next = window_range(count, top, view, ROW);
        let prev = window_range(count, *scroll_top.peek(), *view_h.peek(), ROW);
        if next != prev {
            scroll_top.set(top);
            view_h.set(view);
        }
    });

    rsx! {
        div { class: "history-panel",
            div { class: "files-head",
                span { class: "files-label", "{label}" }
                span { class: "files-count",
                    if let Some(text) = count_label { "{text}" }
                }
                button {
                    r#type: "button",
                    class: "btn btn-quiet btn-compact",
                    disabled: loading,
                    onclick: move |_| nonce.set(nonce() + 1),
                    "Refresh"
                }
            }
            if let Some(message) = error.clone() {
                p { class: "files-error", "{message}" }
            }
            if commits.is_none() && error.is_none() {
                p { class: "files-status", "Loading history\u{2026}" }
            }
            if commits.as_ref().is_some_and(|items| items.is_empty()) {
                p { class: "files-status",
                    "This repository has no commits yet. Commit your files (git add, then git commit) and they show here."
                }
            }
            ol {
                id: "{list_id}",
                class: "{list_class}",
                onscroll: move |event| {
                    if !windowed {
                        return;
                    }
                    let data = event.data();
                    let top = data.scroll_top();
                    let view = f64::from(data.client_height());
                    let next = window_range(count_for_scroll, top, view, ROW);
                    let prev = window_range(count_for_scroll, *scroll_top.peek(), *view_h.peek(), ROW);
                    if next != prev {
                        scroll_top.set(top);
                        view_h.set(view);
                    }
                },
                if top_gap > 0.0 {
                    li { style: "height: {top_gap}px;", aria_hidden: "true" }
                }
                for (offset, commit) in slice_commits.into_iter().enumerate() {
                    HistoryCommit {
                        key: "{commit.hash}",
                        commit: commit,
                        row: slice_rows.get(offset).cloned(),
                        width: width,
                        now: now,
                        selected: selected.clone(),
                        list_key: key.clone(),
                        pick: pick,
                    }
                }
                if bottom_gap > 0.0 {
                    li { style: "height: {bottom_gap}px;", aria_hidden: "true" }
                }
                if more {
                    li { class: "history-more",
                        button {
                            r#type: "button",
                            class: "btn btn-quiet btn-compact",
                            disabled: loading,
                            onclick: move |_| load_more(more_key.clone(), more_place.clone(), log, paging, failure),
                            if paging() { "Loading\u{2026}" } else { "Load more" }
                        }
                    }
                }
            }
            if let Some(hash) = selected.clone() {
                div { class: "history-detail",
                    if detail.is_none() {
                        p { class: "files-status", "Loading commit\u{2026}" }
                    } else if let Some(detail) = detail.clone() {
                        CommitBody {
                            detail: detail,
                            file: file.clone(),
                            diff: diff.clone(),
                            selected_hash: hash,
                            file_sel: file_sel,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn HistoryCommit(
    commit: Commit,
    row: Option<GraphRow>,
    width: i64,
    now: i64,
    selected: Option<String>,
    list_key: String,
    mut pick: Signal<Option<Pick>>,
) -> Element {
    let badges = ref_badges(&commit.refs);
    let head = is_head(&commit.refs);
    let is_selected = Some(&commit.hash) == selected.as_ref();
    let class = if is_selected {
        "history-row is-selected"
    } else {
        "history-row"
    };
    let title = format!(
        "{}\n{} \u{00b7} {}\n{}",
        commit.subject,
        commit.author,
        locale_time(commit.time),
        commit.hash
    );
    let hash = commit.hash.clone();
    let hash_for_click = hash.clone();
    let age = ago(commit.time, now);
    rsx! {
        li {
            button {
                r#type: "button",
                class: "{class}",
                aria_pressed: "{is_selected}",
                title: "{title}",
                onclick: move |_| {
                    let next = if is_selected { None } else { Some(hash_for_click.clone()) };
                    pick.set(next.map(|hash| Pick { key: list_key.clone(), hash }));
                },
                if let Some(row) = row {
                    HistoryGraph { row: row, width: width, head: head }
                }
                span { class: "history-text",
                    span { class: "history-subject",
                        for badge in badges {
                            span { key: "{badge.kind.as_str()}-{badge.name}", class: "history-ref is-{badge.kind.as_str()}", "{badge.name}" }
                        }
                        "{commit.subject}"
                    }
                }
                span { class: "history-meta",
                    span { class: "history-author", "{commit.author}" }
                    span { class: "history-age", "{age}" }
                }
            }
        }
    }
}

#[component]
fn HistoryGraph(row: GraphRow, width: i64, head: bool) -> Element {
    let mid = ROW / 2.0;
    let svg_width = PAD * 2 + (width - 1) * LANE;
    let radius = if head { 4.5 } else { 3.5 };
    let stroke = if head { 2 } else { 0 };
    let fill = if head {
        "var(--bg)"
    } else {
        lane_color(row.col)
    };
    let color = lane_color(row.col);
    let cx = js_num(lane_x(row.col));
    rsx! {
        svg {
            class: "history-graph",
            width: "{svg_width}",
            height: "{ROW}",
            "aria-hidden": "true",
            for (index, line) in row.top.iter().cloned().enumerate() {
                path { key: "t{index}", d: "{curve(&line, 0.0, mid)}", stroke: "{lane_color(line.lane)}" }
            }
            for (index, line) in row.bottom.iter().cloned().enumerate() {
                path { key: "b{index}", d: "{curve(&line, mid, ROW)}", stroke: "{lane_color(line.lane)}" }
            }
            circle {
                cx: "{cx}",
                cy: "{js_num(mid)}",
                r: "{radius}",
                fill: "{fill}",
                stroke: "{color}",
                stroke_width: "{stroke}",
            }
        }
    }
}

#[component]
fn CommitBody(
    detail: CommitDetail,
    file: Option<String>,
    diff: Option<String>,
    selected_hash: String,
    mut file_sel: Signal<Option<FilePick>>,
) -> Element {
    let short = detail.hash.chars().take(10).collect::<String>();
    let when = locale_time(detail.time);
    let copy_id = detail.hash.clone();
    let file_for_block = file.clone();
    rsx! {
        div { class: "history-detail-head",
            span { class: "history-hash", "{short}" }
            span { class: "history-who", "{detail.author} \u{00b7} {when}" }
            button {
                r#type: "button",
                class: "btn btn-quiet btn-compact",
                onclick: move |_| copy_text(&copy_id),
                "Copy id"
            }
        }
        pre { class: "history-message", "{detail.message}" }
        ul { class: "history-files",
            if detail.files.is_empty() {
                li { class: "files-status", "No file changes." }
            }
            for entry in detail.files.iter().cloned() {
                HistoryFile {
                    key: "{entry.path}",
                    path: entry.path.clone(),
                    added: entry.added,
                    removed: entry.removed,
                    current: file.clone(),
                    selected_hash: selected_hash.clone(),
                    file_sel: file_sel,
                }
            }
        }
        if file_for_block.is_some() {
            DiffBlock { text: diff }
        }
    }
}

#[component]
fn HistoryFile(
    path: String,
    added: Option<i64>,
    removed: Option<i64>,
    current: Option<String>,
    selected_hash: String,
    mut file_sel: Signal<Option<FilePick>>,
) -> Element {
    let chosen = Some(&path) == current.as_ref();
    let class = if chosen {
        "history-file is-selected"
    } else {
        "history-file"
    };
    let click_path = path.clone();
    rsx! {
        li {
            button {
                r#type: "button",
                class: "{class}",
                onclick: move |_| {
                    let next = if chosen {
                        None
                    } else {
                        Some(FilePick { hash: selected_hash.clone(), path: click_path.clone() })
                    };
                    file_sel.set(next);
                },
                span { class: "history-file-path", "{path}" }
                if let (Some(added), Some(removed)) = (added, removed) {
                    span { class: "history-file-stat",
                        span { class: "stat-add", "+{added}" }
                        " "
                        span { class: "stat-del", "\u{2212}{removed}" }
                    }
                } else {
                    span { class: "history-file-stat", "binary" }
                }
            }
        }
    }
}

fn load_more(
    asked: String,
    place: Option<String>,
    mut log: Signal<Option<LogState>>,
    mut paging: Signal<bool>,
    mut failure: Signal<Option<Failure>>,
) {
    if *paging.peek() {
        return;
    }
    let Some(prev) = log.peek().clone() else {
        return;
    };
    if prev.key != asked {
        return;
    }
    paging.set(true);
    let skip = prev.commits.len() as f64;
    spawn_local(async move {
        match api::commit_log(place.as_deref(), skip).await {
            Ok(page) => {
                let current = log.peek().clone().filter(|item| item.key == asked);
                if let Some(current) = current {
                    let mut commits = current.commits;
                    commits.extend(page.commits);
                    log.set(Some(LogState {
                        key: asked,
                        commits,
                        more: page.more,
                        at: current.at,
                    }));
                }
            }
            Err(err) => failure.set(Some(Failure {
                key: asked,
                text: bridge_error(&err),
            })),
        }
        paging.set(false);
    });
}

fn next_list_id() -> u64 {
    use std::cell::Cell;
    thread_local! {
        static NEXT: Cell<u64> = const { Cell::new(1) };
    }
    NEXT.with(|slot| {
        let id = slot.get();
        slot.set(id.saturating_add(1));
        id
    })
}
