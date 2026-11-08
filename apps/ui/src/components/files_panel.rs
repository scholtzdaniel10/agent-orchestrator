//! Files drawer: tree, filter, preview, and diff. Port of `FilesPanel.tsx`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use dioxus::prelude::*;
use gloo_timers::callback::Timeout;
use js_sys::Date;
use legate_core::{FileContent, FileList, FileStatus};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element as DomNode, EventTarget, window};

use crate::api;
use crate::logic::diff_lines::{DiffKind, DiffLine, parse_diff};
use crate::logic::file_tree::{
    FILE_LINE_CAP, FILE_ROW_HEIGHT, FILE_WINDOW_THRESHOLD, RowKind, VisibleRow, build_tree,
    files_count_label, flatten_visible, format_bytes, parent_path, status_mark, status_name,
    window_range,
};

const DIFF_ROW_PX: f64 = 17.4;
const TEXT_ROW_PX: f64 = 17.4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Preview {
    File,
    Changes,
}

thread_local! {
    static EXPANDED: RefCell<HashMap<String, HashSet<String>>> = RefCell::new(HashMap::new());
    static SPLITS: RefCell<HashMap<String, f64>> = RefCell::new(HashMap::new());
    static SELECTED: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
    static RELOADERS: RefCell<Vec<(u64, LoaderHandle)>> = const { RefCell::new(Vec::new()) };
    static CHANGES_ON: Cell<bool> = const { Cell::new(false) };
    static NEXT_DOM: Cell<u64> = const { Cell::new(1) };
    static SPLIT_DRAG: RefCell<Option<SplitSession>> = const { RefCell::new(None) };
    static SPLIT_LISTENING: Cell<bool> = const { Cell::new(false) };
}

fn next_dom_id() -> u64 {
    NEXT_DOM.with(|next| {
        let id = next.get();
        next.set(id.saturating_add(1));
        id
    })
}

fn where_key(place: Option<&str>) -> String {
    place.unwrap_or("").to_string()
}

fn load_expanded(key: &str) -> HashSet<String> {
    EXPANDED.with(|map| map.borrow().get(key).cloned().unwrap_or_default())
}

fn load_split(key: &str) -> f64 {
    SPLITS.with(|map| map.borrow().get(key).copied().unwrap_or(0.5))
}

fn load_selected(key: &str) -> Option<String> {
    SELECTED.with(|map| map.borrow().get(key).cloned().unwrap_or(None))
}

fn remember_expanded(key: &str, folders: &HashSet<String>) {
    EXPANDED.with(|map| {
        map.borrow_mut().insert(key.to_string(), folders.clone());
    });
}

fn remember_split(key: &str, split: f64) {
    SPLITS.with(|map| {
        map.borrow_mut().insert(key.to_string(), split);
    });
}

fn remember_selected(key: &str, path: &Option<String>) {
    SELECTED.with(|map| {
        map.borrow_mut().insert(key.to_string(), path.clone());
    });
}

struct LoaderInner {
    place: RefCell<Option<String>>,
    load_id: Cell<u64>,
    last_at: Cell<f64>,
    pending: RefCell<Option<Timeout>>,
    list: Signal<Option<FileList>>,
    loading: Signal<bool>,
    error: Signal<Option<String>>,
}

#[derive(Clone)]
struct LoaderHandle {
    inner: Rc<LoaderInner>,
}

impl LoaderHandle {
    fn request(&self, immediate: bool) {
        if let Some(pending) = self.inner.pending.borrow_mut().take() {
            drop(pending);
        }
        if immediate {
            self.fetch();
            return;
        }
        let wait = (1000.0 - (Date::now() - self.inner.last_at.get())).max(0.0);
        if wait <= 0.0 {
            self.fetch();
            return;
        }
        let later = self.clone();
        let millis = wait.round().clamp(0.0, u32::MAX as f64) as u32;
        *self.inner.pending.borrow_mut() = Some(Timeout::new(millis, move || later.fetch()));
    }

    fn fetch(&self) {
        self.inner.last_at.set(Date::now());
        let id = self.inner.load_id.get().saturating_add(1);
        self.inner.load_id.set(id);
        let mut list = self.inner.list;
        let mut error = self.inner.error;
        let mut loading = self.inner.loading;
        let handle = self.clone();
        if list.peek().is_none() {
            loading.set(true);
        }
        let place = self.inner.place.borrow().clone();
        spawn_local(async move {
            let outcome = api::list_files(place.as_deref()).await;
            if handle.inner.load_id.get() != id {
                return;
            }
            match outcome {
                Ok(next) => {
                    if list.peek().as_ref() != Some(&next) {
                        list.set(Some(next));
                    }
                    if error.peek().is_some() {
                        error.set(None);
                    }
                }
                Err(err) => error.set(Some(bridge_error(&err))),
            }
            if *loading.peek() {
                loading.set(false);
            }
        });
    }
}

fn ensure_changes_listener() {
    let start = CHANGES_ON.with(|on| !on.replace(true));
    if !start {
        return;
    }
    spawn_local(async {
        let _ = api::on_changes(|_list| {
            RELOADERS.with(|list| {
                for (_, loader) in list.borrow().iter() {
                    loader.request(false);
                }
            });
        })
        .await;
    });
}

struct SplitSession {
    body_id: String,
    key: String,
    split: Signal<f64>,
}

fn ensure_split_listeners() {
    let start = SPLIT_LISTENING.with(|on| !on.replace(true));
    if !start {
        return;
    }
    let Some(target) = window() else {
        return;
    };
    let target: &EventTarget = target.unchecked_ref();
    let on_move = Closure::wrap(Box::new(move |event: web_sys::Event| {
        let Some(y) = event_f64(&event, "clientY") else {
            return;
        };
        let applied = SPLIT_DRAG.with(|slot| {
            let borrow = slot.borrow();
            let session = borrow.as_ref()?;
            let body = document_element(&session.body_id)?;
            let rect = body.get_bounding_client_rect();
            if rect.height() <= 0.0 {
                return None;
            }
            let next = ((y - rect.y()) / rect.height()).clamp(0.18, 0.82);
            Some((next, session.key.clone(), session.split))
        });
        if let Some((next, key, mut split)) = applied {
            split.set(next);
            remember_split(&key, next);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    let on_up = Closure::wrap(Box::new(move |_event: web_sys::Event| {
        SPLIT_DRAG.with(|slot| {
            slot.borrow_mut().take();
        });
    }) as Box<dyn FnMut(web_sys::Event)>);
    let _ =
        target.add_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref());
    let _ = target.add_event_listener_with_callback("pointerup", on_up.as_ref().unchecked_ref());
    let _ =
        target.add_event_listener_with_callback("pointercancel", on_up.as_ref().unchecked_ref());
    on_move.forget();
    on_up.forget();
}

#[component]
pub fn FilesPanel(r#where: Option<String>, label: String) -> Element {
    let panel_id = use_hook(next_dom_id);
    let tree_id = format!("files-tree-{panel_id}");
    let body_id = format!("files-body-{panel_id}");
    let preview_id = format!("files-preview-{panel_id}");
    let key = where_key(r#where.as_deref());

    let mut list = use_signal(|| None::<FileList>);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);
    let mut filter = use_signal(String::new);
    let mut changed_only = use_signal(|| false);
    let mut expanded = use_signal(|| load_expanded(&key));
    let mut split = use_signal(|| load_split(&key));
    let mut selected = use_signal(|| load_selected(&key));
    let mut focused = use_signal(|| None::<String>);
    let mut content = use_signal(|| None::<(String, FileContent)>);
    let mut diff = use_signal(|| None::<(String, String)>);
    let mut preview = use_signal(|| Preview::File);
    let opening = use_signal(|| false);
    let mut open_error = use_signal(|| None::<String>);
    let mut scroll_top = use_signal(|| 0.0_f64);
    let mut view_h = use_signal(|| 320.0_f64);
    let mut preview_scroll = use_signal(|| 0.0_f64);
    let mut preview_view = use_signal(|| 400.0_f64);
    let mut seen = use_signal(|| key.clone());
    let opening_flag = use_hook(|| Rc::new(Cell::new(false)));
    let content_ticket = use_hook(|| Rc::new(Cell::new(0_u64)));
    let text_rows = use_hook(|| Rc::new(Cell::new(0_i64)));

    let loader = use_hook(|| LoaderHandle {
        inner: Rc::new(LoaderInner {
            place: RefCell::new(r#where.clone()),
            load_id: Cell::new(0),
            last_at: Cell::new(0.0),
            pending: RefCell::new(None),
            list,
            loading,
            error,
        }),
    });
    *loader.inner.place.borrow_mut() = r#where.clone();

    if seen.peek().as_str() != key {
        seen.set(key.clone());
        expanded.set(load_expanded(&key));
        split.set(load_split(&key));
        selected.set(load_selected(&key));
        focused.set(None);
        filter.set(String::new());
        changed_only.set(false);
        preview.set(Preview::File);
        content.set(None);
        diff.set(None);
        list.set(None);
        error.set(None);
        loading.set(true);
        open_error.set(None);
        scroll_top.set(0.0);
    }

    let registered = use_hook(|| Rc::new(Cell::new(false)));
    if !registered.get() {
        registered.set(true);
        let handle = loader.clone();
        RELOADERS.with(|slots| slots.borrow_mut().push((panel_id, handle)));
        ensure_changes_listener();
    }
    let loader_focus = loader.clone();
    let focus_slot = use_hook(|| Rc::new(RefCell::new(None::<Closure<dyn FnMut(web_sys::Event)>>)));
    if focus_slot.borrow().is_none() {
        let reload = loader_focus.clone();
        let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            reload.request(false);
        }) as Box<dyn FnMut(web_sys::Event)>);
        if let Some(target) = window() {
            let target: &EventTarget = target.unchecked_ref();
            let _ =
                target.add_event_listener_with_callback("focus", closure.as_ref().unchecked_ref());
        }
        *focus_slot.borrow_mut() = Some(closure);
    }

    let load_now = loader.clone();
    let place_loader = loader.clone();
    let refresh_loader = loader.clone();
    let retry_loader = loader.clone();
    let drop_loader = loader;
    use_drop(move || {
        RELOADERS.with(|slots| slots.borrow_mut().retain(|(id, _)| *id != panel_id));
        if let Some(pending) = drop_loader.inner.pending.borrow_mut().take() {
            drop(pending);
        }
        if let Some(target) = window()
            && let Some(closure) = focus_slot.borrow().as_ref()
        {
            let target: &EventTarget = target.unchecked_ref();
            let _ = target
                .remove_event_listener_with_callback("focus", closure.as_ref().unchecked_ref());
        }
    });

    use_effect(move || {
        let _ = seen();
        load_now.request(true);
    });

    let ticket = content_ticket.clone();
    use_effect(move || {
        let _ = seen();
        let path = selected.read().clone();
        let status = {
            let listed = list.read();
            path.as_ref().and_then(|wanted| {
                listed.as_ref().and_then(|item| {
                    item.entries
                        .iter()
                        .find(|row| &row.path == wanted)
                        .map(|row| row.status)
                })
            })
        };
        let id = ticket.get().saturating_add(1);
        ticket.set(id);
        let Some(path) = path else {
            return;
        };
        let Some(status) = status else {
            return;
        };
        let place = place_loader.inner.place.borrow().clone();
        let read_ticket = ticket.clone();
        let read_place = place.clone();
        let read_path = path.clone();
        spawn_local(async move {
            let value = match api::read_file(read_place.as_deref(), &read_path).await {
                Ok(value) => value,
                Err(_) => FileContent::Missing,
            };
            if read_ticket.get() == id {
                content.set(Some((read_path, value)));
            }
        });
        if status != FileStatus::Clean {
            let diff_ticket = ticket.clone();
            spawn_local(async move {
                let text = api::file_diff(place.as_deref(), &path)
                    .await
                    .unwrap_or_default();
                if diff_ticket.get() == id {
                    diff.set(Some((path, text)));
                }
            });
        }
    });

    let tree = use_memo(move || {
        let listed = list.read();
        let entries = listed
            .as_ref()
            .map(|item| item.entries.as_slice())
            .unwrap_or(&[]);
        build_tree(entries)
    });
    let rows = use_memo(move || {
        let nodes = tree.read();
        let open = expanded.read();
        flatten_visible(&nodes, &open, filter.read().as_str(), changed_only())
    });

    let tree_measure = tree_id.clone();
    use_effect(move || {
        let _ = (split(), list.read().is_some());
        let Some(el) = document_element(&tree_measure) else {
            return;
        };
        let height = f64::from(el.client_height());
        if height > 1.0 && (*view_h.peek() - height).abs() > 1.0 {
            view_h.set(height);
        }
    });

    let focus_id = tree_id.clone();
    let mut focus_watch = use_signal(|| -1_i64);
    let focus_at = {
        let listed = rows.read();
        let wanted = focused.peek().clone().or(selected.peek().clone());
        listed
            .iter()
            .position(|row| Some(&row.path) == wanted.as_ref())
            .unwrap_or(0) as i64
    };
    if *focus_watch.peek() != focus_at {
        focus_watch.set(focus_at);
    }
    use_effect(move || {
        let index = focus_watch();
        let Some(el) = document_element(&focus_id) else {
            return;
        };
        let top = index.saturating_mul(FILE_ROW_HEIGHT);
        let bottom = top.saturating_add(FILE_ROW_HEIGHT);
        let scroll = i64::from(el.scroll_top());
        let height = i64::from(el.client_height());
        if height <= 0 {
            return;
        }
        if top < scroll {
            el.set_scroll_top(i32::try_from(top).unwrap_or(i32::MAX));
        } else if bottom > scroll.saturating_add(height) {
            let next = bottom.saturating_sub(height);
            el.set_scroll_top(i32::try_from(next).unwrap_or(i32::MAX));
        }
    });

    let preview_measure = preview_id.clone();
    use_effect(move || {
        let _ = preview();
        let Some(el) = document_element(&preview_measure) else {
            return;
        };
        let height = f64::from(el.client_height());
        if height > 1.0 && (*preview_view.peek() - height).abs() > 1.0 {
            preview_view.set(height);
        }
    });

    let row_count = rows.read().len() as i64;
    let range = window_range(row_count, scroll_top(), view_h(), FILE_ROW_HEIGHT as f64);
    let start = usize::try_from(range.start.max(0)).unwrap_or(0);
    let end = usize::try_from(range.end.max(0))
        .unwrap_or(0)
        .min(rows.read().len());
    let start = start.min(end);
    let slice: Vec<VisibleRow> = rows.read()[start..end].to_vec();
    let narrowed = !filter.read().trim().is_empty() || changed_only();
    let total_files = list
        .read()
        .as_ref()
        .map(|item| item.entries.len())
        .unwrap_or(0);
    let shown_files = rows
        .read()
        .iter()
        .filter(|row| row.kind == RowKind::File)
        .count();
    let count_label = files_count_label(shown_files, total_files, narrowed);
    let selected_entry = {
        let wanted = selected.read().clone();
        list.read().as_ref().and_then(|item| {
            wanted.and_then(|path| item.entries.iter().find(|row| row.path == path).cloned())
        })
    };
    let shown_content = content.read().as_ref().and_then(|(path, value)| {
        selected_entry
            .as_ref()
            .filter(|row| &row.path == path)
            .map(|_| value.clone())
    });
    let shown_diff = diff.read().as_ref().and_then(|(path, value)| {
        selected_entry
            .as_ref()
            .filter(|row| &row.path == path)
            .map(|_| value.clone())
    });
    text_rows.set(match &shown_content {
        Some(FileContent::Text { text, .. }) => split_lines(text).len() as i64,
        _ => 0,
    });
    let dirty = selected_entry
        .as_ref()
        .is_some_and(|row| row.status != FileStatus::Clean);
    let mode = if dirty && preview() == Preview::Changes {
        Preview::Changes
    } else {
        Preview::File
    };
    let bytes = shown_content.as_ref().and_then(content_bytes);
    let show_loading = loading() && list.read().is_none();
    let truncated = list.read().as_ref().is_some_and(|item| item.truncated);
    let filtering = !filter.read().trim().is_empty() || changed_only();
    let open_folders = expanded.read().clone();
    let focused_path = focused.read().clone().or(selected.read().clone());
    let chosen_path = selected.read().clone();
    let split_value = split();
    let inner_height = row_count.saturating_mul(FILE_ROW_HEIGHT);
    let translate = range.start.saturating_mul(FILE_ROW_HEIGHT);

    let key_toggle = key.clone();
    let key_select = key.clone();
    let key_for_keys = key_toggle.clone();
    let select_for_keys = key_select.clone();
    let key_split = key.clone();
    let body_for_split = body_id.clone();
    let rows_key = rows;
    let place_open = r#where.clone();
    let place_for_open = r#where.clone();
    let opening_for_open = opening_flag.clone();

    rsx! {
        div { class: "files-panel",
            div { class: "files-head",
                span { class: "files-label", "{label}" }
                span { class: "files-count",
                    if list.read().is_some() { "{count_label}" }
                }
                button {
                    r#type: "button",
                    class: "btn btn-quiet btn-compact",
                    onclick: move |_| refresh_loader.request(false),
                    "Refresh"
                }
            }
            div { class: "files-tools",
                input {
                    r#type: "search",
                    class: "files-filter",
                    placeholder: "Filter files",
                    value: "{filter}",
                    aria_label: "Filter files",
                    oninput: move |event| filter.set(event.value()),
                }
                button {
                    r#type: "button",
                    class: "btn btn-quiet btn-compact",
                    aria_pressed: "{changed_only()}",
                    onclick: move |_| changed_only.set(!changed_only()),
                    "Changed"
                }
            }
            if show_loading {
                p { class: "files-status", "Loading files\u{2026}" }
            }
            if let Some(message) = error.read().as_ref() {
                div { class: "files-error",
                    p { class: "error-text", role: "alert", "{message}" }
                    button {
                        r#type: "button",
                        class: "btn btn-quiet btn-compact",
                        onclick: move |_| retry_loader.request(true),
                        "Retry"
                    }
                }
            }
            if truncated {
                p { class: "files-status", "List cut at 20 000 files." }
            }
            div { id: "{body_id}", class: "files-body",
                div {
                    id: "{tree_id}",
                    class: "files-tree",
                    style: "flex: {split_value} 1 0;",
                    role: "tree",
                    aria_label: "Files",
                    aria_busy: "{loading()}",
                    tabindex: "0",
                    onscroll: move |event| {
                        let top = event.data().scroll_top();
                        let height = f64::from(event.data().client_height());
                        let count = rows_key.peek().len() as i64;
                        let next = window_range(count, top, height, FILE_ROW_HEIGHT as f64);
                        let prev = window_range(count, *scroll_top.peek(), *view_h.peek(), FILE_ROW_HEIGHT as f64);
                        if next != prev {
                            scroll_top.set(top);
                            view_h.set(height);
                        }
                    },
                    onkeydown: move |event| {
                        on_tree_key(
                            event,
                            rows_key,
                            &key_for_keys,
                            &select_for_keys,
                            expanded,
                            filter,
                            changed_only,
                            selected,
                            focused,
                            open_error,
                            opening,
                            opening_flag.clone(),
                            place_open.clone(),
                        );
                    },
                    div { class: "files-tree-inner", style: "height: {inner_height}px;",
                        div {
                            class: "files-tree-window",
                            style: "transform: translateY({translate}px);",
                            for row in slice {
                                FileRow {
                                    key: "{row.kind.as_str()}:{row.path}",
                                    row: row.clone(),
                                    chosen: row.kind == RowKind::File && Some(&row.path) == chosen_path.as_ref(),
                                    focused: Some(&row.path) == focused_path.as_ref(),
                                    open: row.kind == RowKind::Dir && (open_folders.contains(&row.path) || filtering),
                                    folder_key: key_toggle.clone(),
                                    select_key: key_select.clone(),
                                    expanded: expanded,
                                    selected: selected,
                                    focus_to: focused,
                                    preview: preview,
                                    open_error: open_error,
                                }
                            }
                        }
                    }
                }
                div {
                    class: "files-split",
                    role: "separator",
                    aria_orientation: "horizontal",
                    aria_label: "Resize preview",
                    onpointerdown: move |event| {
                        event.prevent_default();
                        let pointer = event.data().pointer_id();
                        SPLIT_DRAG.with(|slot| {
                            *slot.borrow_mut() = Some(SplitSession {
                                body_id: body_for_split.clone(),
                                key: key_split.clone(),
                                split,
                            });
                        });
                        ensure_split_listeners();
                        if let Some(el) = document_element(&format!("files-split-{panel_id}")) {
                            let _ = el.set_pointer_capture(pointer);
                        }
                    },
                    id: "files-split-{panel_id}",
                }
                div {
                    id: "{preview_id}",
                    class: "files-preview",
                    style: "flex: {1.0 - split_value} 1 0;",
                    if selected_entry.is_none() {
                        p { class: "files-status", "Select a file." }
                    } else if let Some(file) = selected_entry.as_ref() {
                        div { class: "files-preview-head",
                            span { class: "files-preview-path", title: "{file.path}", "{file.path}" }
                            if let Some(size) = bytes {
                                span { class: "files-preview-size", "{format_bytes(size.max(0) as u64)}" }
                            }
                            div { class: "files-preview-actions",
                                button {
                                    r#type: "button",
                                    class: "btn btn-quiet btn-compact",
                                    aria_pressed: "{mode == Preview::File}",
                                    onclick: move |_| preview.set(Preview::File),
                                    "File"
                                }
                                if dirty {
                                    button {
                                        r#type: "button",
                                        class: "btn btn-quiet btn-compact",
                                        aria_pressed: "{mode == Preview::Changes}",
                                        onclick: move |_| preview.set(Preview::Changes),
                                        "Changes"
                                    }
                                }
                                button {
                                    r#type: "button",
                                    class: "btn btn-quiet btn-compact",
                                    disabled: opening(),
                                    onclick: move |_| open_current(place_for_open.clone(), selected, opening, opening_for_open.clone(), open_error),
                                    "Open"
                                }
                            }
                        }
                        if let Some(message) = open_error.read().as_ref() {
                            p { class: "error-text", role: "alert", "{message}" }
                        }
                        div {
                            class: "files-preview-body",
                            onscroll: move |event| {
                                let top = event.data().scroll_top();
                                let height = f64::from(event.data().client_height());
                                let count = text_rows.get();
                                let next = window_range(count, top, height, TEXT_ROW_PX);
                                let prev = window_range(count, *preview_scroll.peek(), *preview_view.peek(), TEXT_ROW_PX);
                                if next != prev {
                                    preview_scroll.set(top);
                                    preview_view.set(height);
                                }
                            },
                            if mode == Preview::Changes {
                                DiffBlock { text: shown_diff.clone(), jump_nonce: 0, jump_path: String::new() }
                            } else {
                                FileBody { content: shown_content.clone(), scroll_top: preview_scroll(), view_h: preview_view() }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FileRow(
    row: VisibleRow,
    chosen: bool,
    focused: bool,
    open: bool,
    folder_key: String,
    select_key: String,
    expanded: Signal<HashSet<String>>,
    selected: Signal<Option<String>>,
    focus_to: Signal<Option<String>>,
    preview: Signal<Preview>,
    open_error: Signal<Option<String>>,
) -> Element {
    let deleted = row.status == FileStatus::Deleted;
    let class = row_class(chosen, focused, deleted);
    let pad = 8 + row.depth * 12;
    let path = row.path.clone();
    let kind = row.kind;
    let mark = status_mark(row.status);
    let mark_class = format!("files-mark is-{}", status_name(row.status));
    let chevron = if open {
        "files-chevron is-open"
    } else {
        "files-chevron"
    };
    let level = row.depth + 1;
    let expanded_state = if kind == RowKind::Dir {
        Some(if open { "true" } else { "false" })
    } else {
        None
    };
    rsx! {
        div {
            id: "files-{row.kind.as_str()}-{row.path}",
            role: "treeitem",
            aria_level: "{level}",
            aria_selected: "{chosen}",
            aria_expanded: expanded_state,
            tabindex: "-1",
            class: "{class}",
            style: "padding-left: {pad}px; height: {FILE_ROW_HEIGHT}px;",
            onclick: move |_| {
                focus_to.set(Some(path.clone()));
                if kind == RowKind::Dir {
                    toggle_folder(&folder_key, expanded, &path);
                } else {
                    select_file(&select_key, selected, focus_to, preview, open_error, path.clone());
                }
            },
            if kind == RowKind::Dir {
                span { class: "{chevron}" }
            } else {
                span { class: "files-chevron-gap" }
            }
            span { class: "files-name", "{row.name}" }
            if kind == RowKind::Dir && row.has_changed {
                span { class: "files-dot" }
            }
            if kind == RowKind::File && let Some(letter) = mark {
                span { class: "{mark_class}", "{letter}" }
            }
        }
    }
}

#[component]
fn FileBody(content: Option<FileContent>, scroll_top: f64, view_h: f64) -> Element {
    let Some(content) = content else {
        return rsx! { p { class: "files-status", "Loading\u{2026}" } };
    };
    match content {
        FileContent::Missing => rsx! { p { class: "files-status", "This file is missing." } },
        FileContent::Binary { .. } => rsx! { p { class: "files-status", "This file is binary." } },
        FileContent::TooLarge { .. } => {
            rsx! { p { class: "files-status", "This file is too large to show." } }
        }
        FileContent::Text { text, .. } => {
            let lines = split_lines(&text);
            let cap = FILE_LINE_CAP as usize;
            let hidden = lines.len().saturating_sub(cap);
            let shown: Vec<&str> = lines.into_iter().take(cap).collect();
            let count = shown.len();
            let windowed = count as i64 > FILE_WINDOW_THRESHOLD;
            let range = window_range(count as i64, scroll_top, view_h, TEXT_ROW_PX);
            let start = usize::try_from(range.start.max(0)).unwrap_or(0).min(count);
            let end = usize::try_from(range.end.max(0)).unwrap_or(0).min(count);
            let start = start.min(end);
            let slice: Vec<(usize, String)> = shown[start..end]
                .iter()
                .enumerate()
                .map(|(offset, line)| (start + offset, (*line).to_string()))
                .collect();
            let height = (count as f64) * TEXT_ROW_PX;
            let shift = (start as f64) * TEXT_ROW_PX;
            rsx! {
                div { class: "files-text",
                    if windowed {
                        div { style: "position: relative; height: {height}px;",
                            div { style: "position: absolute; top: 0; left: 0; right: 0; transform: translateY({shift}px);",
                                for (index, line) in slice {
                                    TextLine { key: "{index}", index: index, line: line }
                                }
                            }
                        }
                    } else {
                        for (index, line) in shown.into_iter().enumerate() {
                            TextLine { key: "{index}", index: index, line: line.to_string() }
                        }
                    }
                    if hidden > 0 {
                        p { class: "files-status", "First {FILE_LINE_CAP} lines shown. {hidden} more." }
                    }
                }
            }
        }
    }
}

#[component]
fn TextLine(index: usize, line: String) -> Element {
    let number = index + 1;
    let source = if line.is_empty() {
        " ".to_string()
    } else {
        line
    };
    rsx! {
        div { class: "files-text-line",
            span { class: "files-text-n", "{number}" }
            span { class: "files-text-src", "{source}" }
        }
    }
}

#[component]
pub fn DiffBlock(
    text: Option<String>,
    #[props(default = 0)] jump_nonce: u32,
    #[props(default)] jump_path: String,
) -> Element {
    match text {
        None => rsx! { p { class: "files-status", "Loading\u{2026}" } },
        Some(text) if text.is_empty() => rsx! { p { class: "files-status", "No changes." } },
        Some(text) => rsx! {
            div { class: "diff-block",
                DiffLineList { text: text, jump_nonce: jump_nonce, jump_path: jump_path }
            }
        },
    }
}

#[component]
pub(super) fn DiffLineList(
    text: String,
    #[props(default = 0)] jump_nonce: u32,
    #[props(default)] jump_path: String,
) -> Element {
    let host_num = use_hook(next_dom_id);
    let host_id = format!("diff-host-{host_num}");
    let mut source = use_signal(|| text.clone());
    if source.peek().as_str() != text {
        source.set(text.clone());
    }
    let parsed = use_memo(move || {
        let all = parse_diff(&source.read());
        let cap = FILE_LINE_CAP as usize;
        let hidden = all.len().saturating_sub(cap);
        let shown: Vec<DiffLine> = all.into_iter().take(cap).collect();
        (shown, hidden)
    });
    let mut scroll_top = use_signal(|| 0.0_f64);
    let view_h = use_signal(|| 800.0_f64);
    let mut pending = use_signal(|| None::<usize>);
    let mut applied_jump = use_signal(|| 0_u32);
    let lines_keep = use_hook(|| Rc::new(RefCell::new(Vec::<DiffLine>::new())));
    let (hidden, windowed, range) = {
        let guard = parsed.read();
        *lines_keep.borrow_mut() = guard.0.clone();
        let count = guard.0.len() as i64;
        let windowed = count > FILE_WINDOW_THRESHOLD;
        let range = window_range(count, scroll_top(), view_h(), DIFF_ROW_PX);
        (guard.1, windowed, range)
    };
    let mut watch_window = use_signal(|| false);
    if watch_window() != windowed {
        watch_window.set(windowed);
    }
    let mut watch_jump = use_signal(|| 0_u32);
    if *watch_jump.peek() != jump_nonce {
        watch_jump.set(jump_nonce);
    }
    let jump_keep = use_hook(|| Rc::new(RefCell::new(String::new())));
    *jump_keep.borrow_mut() = jump_path.clone();
    let lines_for_jump = lines_keep.clone();
    let host_for_jump = host_id.clone();
    let jump_for_effect = jump_keep.clone();
    use_effect(move || {
        let nonce = watch_jump();
        if nonce == 0 || *applied_jump.peek() == nonce {
            return;
        }
        applied_jump.set(nonce);
        let path = jump_for_effect.borrow().clone();
        if path.is_empty() {
            return;
        }
        let Some(index) = header_index(&lines_for_jump.borrow(), &path) else {
            return;
        };
        scroll_top.set(index as f64 * DIFF_ROW_PX);
        let line_id = format!("diff-file-{host_for_jump}-{index}");
        if let Some(el) = document_element(&line_id) {
            el.scroll_into_view_with_bool(true);
        } else {
            pending.set(Some(index));
        }
    });

    let host_for_scroll = host_id.clone();
    let listens = use_hook(|| Rc::new(RefCell::new(Vec::<DomListen>::new())));
    let count_cell = use_hook(|| Rc::new(Cell::new(0_i64)));
    count_cell.set(lines_keep.borrow().len() as i64);
    use_effect(move || {
        listens.borrow_mut().clear();
        if !watch_window() {
            return;
        }
        let Some(win) = window() else {
            return;
        };
        let target: EventTarget = win.unchecked_into();
        let host = host_for_scroll.clone();
        let counts = count_cell.clone();
        let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            sync_diff_scroll(&host, counts.get(), scroll_top, view_h);
        }) as Box<dyn FnMut(web_sys::Event)>);
        listens
            .borrow_mut()
            .push(DomListen::add(target, "scroll", true, closure));
        let Some(win) = window() else {
            return;
        };
        let target: EventTarget = win.unchecked_into();
        let host = host_for_scroll.clone();
        let counts = count_cell.clone();
        let closure = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            sync_diff_scroll(&host, counts.get(), scroll_top, view_h);
        }) as Box<dyn FnMut(web_sys::Event)>);
        listens
            .borrow_mut()
            .push(DomListen::add(target, "resize", false, closure));
        sync_diff_scroll(&host_for_scroll, count_cell.get(), scroll_top, view_h);
    });

    let count = lines_keep.borrow().len();
    let start = usize::try_from(range.start.max(0)).unwrap_or(0).min(count);
    let end = usize::try_from(range.end.max(0)).unwrap_or(0).min(count);
    let start = start.min(end);
    let slice: Vec<(usize, DiffLine)> = lines_keep.borrow()[start..end]
        .iter()
        .cloned()
        .enumerate()
        .map(|(offset, line)| (start + offset, line))
        .collect();
    let height = (count as f64) * DIFF_ROW_PX;
    let shift = (start as f64) * DIFF_ROW_PX;
    let host_for_rows = host_id.clone();

    rsx! {
        if windowed {
            div { id: "{host_id}", style: "position: relative; height: {height}px;",
                div { style: "position: absolute; top: 0; left: 0; right: 0; transform: translateY({shift}px);",
                    for (index, line) in slice {
                        DiffRow { key: "{index}", index: index, line: line, host: host_for_rows.clone(), pending: pending }
                    }
                }
            }
        } else {
            for (index, line) in lines_keep.borrow().iter().cloned().enumerate() {
                DiffRow { key: "{index}", index: index, line: line, host: host_for_rows.clone(), pending: pending }
            }
        }
        if hidden > 0 {
            div { class: "diff-line is-meta", "\u{2026} {hidden} more lines not shown" }
        }
    }
}

#[component]
fn DiffRow(index: usize, line: DiffLine, host: String, pending: Signal<Option<usize>>) -> Element {
    let class = format!("diff-line is-{}", line.kind.as_str());
    let text = if line.text.is_empty() {
        " ".to_string()
    } else {
        line.text.clone()
    };
    let line_id = format!("diff-file-{host}-{index}");
    if line.kind == DiffKind::File {
        rsx! {
            div {
                class: "{class}",
                id: "{line_id}",
                onmounted: move |_| {
                    if pending.peek().as_ref() == Some(&index) {
                        if let Some(el) = document_element(&line_id) {
                            el.scroll_into_view_with_bool(true);
                        }
                        pending.set(None);
                    }
                },
                "{text}"
            }
        }
    } else {
        rsx! { div { class: "{class}", "{text}" } }
    }
}

#[allow(clippy::too_many_arguments)]
fn on_tree_key(
    event: Event<KeyboardData>,
    rows: Memo<Vec<VisibleRow>>,
    folder_key: &str,
    select_key: &str,
    expanded: Signal<HashSet<String>>,
    filter: Signal<String>,
    changed_only: Signal<bool>,
    selected: Signal<Option<String>>,
    focused: Signal<Option<String>>,
    open_error: Signal<Option<String>>,
    opening: Signal<bool>,
    opening_flag: Rc<Cell<bool>>,
    place: Option<String>,
) {
    let key_name = event.key();
    let listed = rows.read();
    let wanted = focused.peek().clone().or(selected.peek().clone());
    let index = listed
        .iter()
        .position(|row| Some(&row.path) == wanted.as_ref())
        .unwrap_or(0);
    let Some(row) = listed.get(index).cloned() else {
        return;
    };
    let filtering = !filter.peek().trim().is_empty() || *changed_only.peek();
    if key_name == Key::ArrowDown || key_name == Key::ArrowUp {
        event.prevent_default();
        let next_index = if key_name == Key::ArrowDown {
            index.saturating_add(1)
        } else if let Some(prev) = index.checked_sub(1) {
            prev
        } else {
            return;
        };
        let Some(next) = listed.get(next_index).cloned() else {
            return;
        };
        drop(listed);
        focus_row(select_key, selected, focused, open_error, next);
        return;
    }
    if key_name == Key::ArrowRight {
        event.prevent_default();
        if row.kind == RowKind::Dir {
            let closed = !expanded.peek().contains(&row.path);
            if closed && !filtering {
                drop(listed);
                toggle_folder(folder_key, expanded, &row.path);
            } else if let Some(child) = listed.get(index + 1).cloned() {
                drop(listed);
                if child.depth > row.depth {
                    focus_row(select_key, selected, focused, open_error, child);
                }
            }
        }
        return;
    }
    if key_name == Key::ArrowLeft {
        event.prevent_default();
        if row.kind == RowKind::Dir && expanded.peek().contains(&row.path) && !filtering {
            drop(listed);
            toggle_folder(folder_key, expanded, &row.path);
            return;
        }
        let Some(parent) = parent_path(&row.path).map(str::to_string) else {
            return;
        };
        let target = listed
            .iter()
            .find(|item| item.kind == RowKind::Dir && item.path == parent)
            .cloned();
        drop(listed);
        if let Some(target) = target {
            focus_row(select_key, selected, focused, open_error, target);
        }
        return;
    }
    if key_name == Key::Enter {
        event.prevent_default();
        drop(listed);
        if row.kind == RowKind::File {
            open_current(place, selected, opening, opening_flag, open_error);
        } else {
            toggle_folder(folder_key, expanded, &row.path);
        }
    }
}

fn focus_row(
    select_key: &str,
    mut selected: Signal<Option<String>>,
    mut focused: Signal<Option<String>>,
    mut open_error: Signal<Option<String>>,
    row: VisibleRow,
) {
    focused.set(Some(row.path.clone()));
    if row.kind == RowKind::File {
        selected.set(Some(row.path.clone()));
        remember_selected(select_key, &Some(row.path));
        open_error.set(None);
    }
}

fn toggle_folder(key: &str, mut expanded: Signal<HashSet<String>>, path: &str) {
    let mut next = expanded.peek().clone();
    if !next.remove(path) {
        next.insert(path.to_string());
    }
    remember_expanded(key, &next);
    expanded.set(next);
}

fn select_file(
    key: &str,
    mut selected: Signal<Option<String>>,
    mut focused: Signal<Option<String>>,
    mut preview: Signal<Preview>,
    mut open_error: Signal<Option<String>>,
    path: String,
) {
    selected.set(Some(path.clone()));
    remember_selected(key, &Some(path.clone()));
    focused.set(Some(path));
    preview.set(Preview::File);
    open_error.set(None);
}

fn open_current(
    place: Option<String>,
    selected: Signal<Option<String>>,
    mut opening: Signal<bool>,
    opening_flag: Rc<Cell<bool>>,
    mut open_error: Signal<Option<String>>,
) {
    let Some(path) = selected.peek().clone() else {
        return;
    };
    if opening_flag.get() {
        return;
    }
    opening_flag.set(true);
    opening.set(true);
    open_error.set(None);
    spawn_local(async move {
        if let Err(err) = api::open_file(place.as_deref(), &path).await {
            open_error.set(Some(bridge_error(&err)));
        }
        opening_flag.set(false);
        opening.set(false);
    });
}

fn row_class(chosen: bool, focused: bool, deleted: bool) -> &'static str {
    match (chosen, focused, deleted) {
        (true, true, true) => "files-row is-selected is-focused is-deleted",
        (true, true, false) => "files-row is-selected is-focused",
        (true, false, true) => "files-row is-selected is-deleted",
        (true, false, false) => "files-row is-selected",
        (false, true, true) => "files-row is-focused is-deleted",
        (false, true, false) => "files-row is-focused",
        (false, false, true) => "files-row is-deleted",
        (false, false, false) => "files-row",
    }
}

fn content_bytes(content: &FileContent) -> Option<i64> {
    match content {
        FileContent::Text { bytes, .. }
        | FileContent::Binary { bytes }
        | FileContent::TooLarge { bytes } => Some(*bytes),
        FileContent::Missing => None,
    }
}

fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            lines.push(&text[start..end]);
            index += 1;
            start = index;
        } else {
            index += 1;
        }
    }
    lines.push(&text[start..]);
    lines
}

pub(super) fn bridge_error(err: &str) -> String {
    let Some(rest) = err.strip_prefix("Error invoking remote method '") else {
        return err.to_string();
    };
    let Some(marker) = rest.find("': ") else {
        return err.to_string();
    };
    let after = &rest[marker + 3..];
    after.strip_prefix("Error: ").unwrap_or(after).to_string()
}

pub(super) fn header_matches(text: &str, path: &str) -> bool {
    let target = diff_target(path);
    text.ends_with(&format!("b/{target}")) || text.ends_with(&format!("b/{target}\""))
}

fn diff_target(path: &str) -> &str {
    const MARKER: &str = " => ";
    match path.rfind(MARKER) {
        Some(at) => &path[at + MARKER.len()..],
        None => path,
    }
}

fn header_index(lines: &[DiffLine], path: &str) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.kind == DiffKind::File && header_matches(&line.text, path))
}

fn sync_diff_scroll(
    host_id: &str,
    count: i64,
    mut scroll_top: Signal<f64>,
    mut view_h: Signal<f64>,
) {
    let Some(host) = document_element(host_id) else {
        return;
    };
    let scroller = scroll_parent(&host);
    let (top, view) = viewport_of(&host, &scroller);
    let view = if view < 1.0 { *view_h.peek() } else { view };
    let next = window_range(count, top, view, DIFF_ROW_PX);
    let prev = window_range(count, *scroll_top.peek(), *view_h.peek(), DIFF_ROW_PX);
    if next != prev {
        scroll_top.set(top);
        view_h.set(view);
    }
}

fn viewport_of(host: &DomNode, scroller: &DomNode) -> (f64, f64) {
    let host_node: &web_sys::Node = host.unchecked_ref();
    let scroller_node: &web_sys::Node = scroller.unchecked_ref();
    if host_node.is_same_node(Some(scroller_node)) {
        return (
            f64::from(host.scroll_top()),
            f64::from(host.client_height()),
        );
    }
    let host_box = host.get_bounding_client_rect();
    let scroll_box = scroller.get_bounding_client_rect();
    let offset = host_box.y() - scroll_box.y() + f64::from(scroller.scroll_top());
    (
        f64::from(scroller.scroll_top()) - offset,
        f64::from(scroller.client_height()),
    )
}

fn scroll_parent(el: &DomNode) -> DomNode {
    let Some(win) = window() else {
        return el.clone();
    };
    if is_scroller(&win, el) {
        return el.clone();
    }
    let mut current = parent_element(el);
    while let Some(node) = current {
        if is_scroller(&win, &node) {
            return node;
        }
        current = parent_element(&node);
    }
    el.clone()
}

fn is_scroller(win: &web_sys::Window, el: &DomNode) -> bool {
    let Ok(Some(style)) = win.get_computed_style(el) else {
        return false;
    };
    let Ok(value) = style.get_property_value("overflow-y") else {
        return false;
    };
    matches!(value.as_str(), "auto" | "scroll" | "overlay")
}

fn parent_element(el: &DomNode) -> Option<DomNode> {
    let node: &web_sys::Node = el.unchecked_ref();
    node.parent_element()
}

pub(super) fn document_element(id: &str) -> Option<DomNode> {
    window()?.document()?.get_element_by_id(id)
}

fn event_f64(event: &web_sys::Event, name: &str) -> Option<f64> {
    js_sys::Reflect::get(event, &JsValue::from_str(name))
        .ok()?
        .as_f64()
}

struct DomListen {
    target: EventTarget,
    kind: &'static str,
    capture: bool,
    closure: Closure<dyn FnMut(web_sys::Event)>,
}

impl DomListen {
    fn add(
        target: EventTarget,
        kind: &'static str,
        capture: bool,
        closure: Closure<dyn FnMut(web_sys::Event)>,
    ) -> Self {
        let _ = target.add_event_listener_with_callback_and_bool(
            kind,
            closure.as_ref().unchecked_ref(),
            capture,
        );
        Self {
            target,
            kind,
            capture,
            closure,
        }
    }
}

impl Drop for DomListen {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            self.kind,
            self.closure.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}
