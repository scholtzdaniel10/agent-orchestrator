use dioxus::prelude::*;
use legate_core::FileContent;
use wasm_bindgen::{JsCast, closure::Closure};

use super::files_panel::{bridge_error, document_element, next_dom_id};
use crate::{api, editor, state::use_store};

const FILE_CHANGED: &str = "The file changed on disk since you opened it.";

struct Session {
    id: String,
    token: f64,
    _on_change: Closure<dyn FnMut(bool)>,
    _on_save: Closure<dyn FnMut()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        editor::dispose(&self.id, self.token);
    }
}

fn replace_text(
    id: &str,
    text: String,
    mut base: Signal<String>,
    mut compare_to_base: Signal<bool>,
) -> Result<(), String> {
    let compare = *compare_to_base.peek();
    compare_to_base.set(false);
    if let Err(message) = editor::set_text(id, &text) {
        compare_to_base.set(compare);
        return Err(message);
    }
    base.set(text);
    Ok(())
}

#[component]
pub fn CodeEditor(
    r#where: Option<String>,
    path: String,
    text: String,
    on_saved: Callback,
) -> Element {
    let id = use_hook(|| format!("files-editor-{}", next_dom_id()));
    let store = use_store();
    let mut editor_dirty = store.editor_dirty;
    let mut session = use_signal(|| None::<Session>);
    let mut base = use_signal(|| text.clone());
    let mut dirty = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut compare_to_base = use_signal(|| false);

    let dirty_path = path.clone();
    let record_dirty = use_callback(move |changed: bool| {
        if *dirty.peek() != changed {
            dirty.set(changed);
        }
        let owns_dirty = editor_dirty.peek().as_deref() == Some(dirty_path.as_str());
        if changed && !owns_dirty {
            editor_dirty.set(Some(dirty_path.clone()));
        } else if !changed && owns_dirty {
            editor_dirty.set(None);
        }
    });
    let drop_path = path.clone();
    use_drop(move || {
        if editor_dirty.peek().as_deref() == Some(drop_path.as_str()) {
            editor_dirty.set(None);
        }
    });

    let save_id = id.clone();
    let save_path = path.clone();
    let save_place = r#where.clone();
    let save = use_callback(move |overwrite: bool| {
        if *busy.peek() || !*dirty.peek() || *store.project_switching.peek() {
            return;
        }
        let saved = match editor::get_text(&save_id) {
            Ok(text) => text,
            Err(message) => {
                error.set(Some(message));
                return;
            }
        };
        let expected = (!overwrite).then(|| base.peek().clone());
        let id = save_id.clone();
        let path = save_path.clone();
        let place = save_place.clone();
        busy.set(true);
        error.set(None);
        spawn(async move {
            match api::write_file(place.as_deref(), &path, &saved, expected.as_deref()).await {
                Ok(()) => {
                    let unchanged = editor::get_text(&id).is_ok_and(|text| text == saved);
                    base.set(saved);
                    if unchanged {
                        compare_to_base.set(false);
                        if let Err(message) = editor::mark_clean(&id) {
                            compare_to_base.set(true);
                            record_dirty.call(false);
                            error.set(Some(message));
                        }
                    } else {
                        // CodeMirror still compares against the old base if editing continued.
                        compare_to_base.set(true);
                        record_dirty.call(true);
                    }
                    on_saved.call(());
                }
                Err(message) => error.set(Some(bridge_error(&message))),
            }
            busy.set(false);
        });
    });

    let mount_id = id.clone();
    let mount_path = path.clone();
    let sync_id = id.clone();
    use_effect(use_reactive((&text,), move |(text,)| {
        if session.read().is_some() && !*dirty.peek() && *base.peek() != text {
            error.set(replace_text(&sync_id, text, base, compare_to_base).err());
        }
    }));
    let discard_id = id.clone();
    use_effect(move || {
        if editor_dirty.read().is_none() && *dirty.peek() {
            let text = base.peek().clone();
            let result = replace_text(&discard_id, text, base, compare_to_base);
            if result.is_err() {
                record_dirty.call(true);
            }
            error.set(result.err());
        }
    });

    let reload_id = id.clone();
    let reload_path = path.clone();
    let reload = move |_| {
        if *busy.peek() || *store.project_switching.peek() {
            return;
        }
        let id = reload_id.clone();
        let path = reload_path.clone();
        let place = r#where.clone();
        busy.set(true);
        spawn(async move {
            let result = async {
                let original = editor::get_text(&id)?;
                let text = match api::read_file(place.as_deref(), &path).await? {
                    FileContent::Text { text, .. } => text,
                    FileContent::Binary { .. } => return Err("This file is binary.".into()),
                    FileContent::TooLarge { .. } => {
                        return Err("This file is too large to show.".into());
                    }
                    FileContent::Missing => return Err("This file is missing.".into()),
                };
                if editor::get_text(&id)? != original {
                    return Err(FILE_CHANGED.into());
                }
                replace_text(&id, text, base, compare_to_base)?;
                editor::focus(&id);
                Ok::<(), String>(())
            }
            .await;
            error.set(result.err().map(|message| bridge_error(&message)));
            busy.set(false);
        });
    };

    rsx! {
        div { class: "files-editor",
            div { class: "files-preview-head",
                span { class: "files-preview-path", title: "{path}",
                    if dirty() {
                        span { class: "files-editor-dirty", aria_label: "Unsaved changes", "\u{25cf}" }
                    }
                    "{path}"
                }
                button {
                    r#type: "button",
                    class: "btn btn-quiet btn-compact",
                    disabled: !dirty() || busy(),
                    onclick: move |_| save.call(false),
                    "Save"
                }
            }
            if let Some(message) = error.read().as_ref() {
                div { class: "files-error",
                    p { class: "error-text", role: "alert", "{message}" }
                    if message == FILE_CHANGED {
                        button {
                            r#type: "button",
                            class: "btn btn-quiet btn-compact",
                            disabled: busy(),
                            onclick: reload,
                            "Reload"
                        }
                        button {
                            r#type: "button",
                            class: "btn btn-quiet btn-compact",
                            disabled: busy(),
                            onclick: move |_| save.call(true),
                            "Overwrite"
                        }
                    }
                }
            }
            div {
                class: "files-editor-host",
                id: "{id}",
                "inert": (store.project_switching)().then_some(""),
                onmounted: move |_| {
                    let id = mount_id.clone();
                    let path = mount_path.clone();
                    spawn(async move {
                        let Some(element) = document_element(&id) else {
                            return;
                        };
                        let text = base.peek().clone();
                        let change_id = id.clone();
                        let on_change = Closure::new(move |changed: bool| {
                            let changed = if *compare_to_base.peek() {
                                editor::get_text(&change_id).map_or(changed, |text| text != *base.peek())
                            } else {
                                changed
                            };
                            record_dirty.call(changed);
                        });
                        let on_save = Closure::new(move || save.call(false));
                        match editor::open(&id, &element, &text, &path, on_change.as_ref().unchecked_ref(), on_save.as_ref().unchecked_ref()).await {
                            Ok(token) => session.set(Some(Session { id, token, _on_change: on_change, _on_save: on_save })),
                            Err(message) => error.set(Some(message)),
                        }
                    });
                },
            }
        }
    }
}
