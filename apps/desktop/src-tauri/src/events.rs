//! Forwards backend broadcasts onto the Tauri event names the UI listens for.

use legate_app::AppEvent;
use legate_core::{JobStatus, ProviderId};
use serde::Serialize;
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::Shell;
use crate::windows::{any_focused, apply_zoom, close_terminal_window};

pub fn spawn(app: tauri::AppHandle, shell: Shell) {
    let Ok(backend) = shell.backend().cloned() else {
        return;
    };
    let mut receiver = backend.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => forward(&app, &shell, event),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn forward(app: &tauri::AppHandle, shell: &Shell, event: AppEvent) {
    match event {
        AppEvent::Job(job) => {
            maybe_notify(app, shell, &job);
            emit(app, "jobs:update", &job);
        }
        AppEvent::Plans(plans) => emit(app, "plans:update", &plans),
        AppEvent::Changes(changes) => emit(app, "changes:update", &changes),
        AppEvent::Goal(goal) => emit(app, "goals:update", &goal),
        AppEvent::Triggers(triggers) => emit(app, "triggers:update", &triggers),
        AppEvent::Prefs(prefs) => {
            apply_zoom(app, prefs.ui_scale);
            shell.set_prefs(prefs.clone());
            emit(app, "prefs:update", &prefs);
        }
        AppEvent::Lead { message, chat_id } => {
            emit(app, "lead:update", &LeadPayload { message, chat_id });
        }
        AppEvent::TerminalData { id, data } => {
            emit(app, "terminals:data", &DataPayload { id, data })
        }
        AppEvent::TerminalUpdate { info, removed } => {
            if removed {
                close_terminal_window(app, shell, &info.id);
            }
            emit(app, "terminals:update", &UpdatePayload { info, removed });
        }
    }
}

fn maybe_notify(app: &tauri::AppHandle, shell: &Shell, job: &legate_core::JobRecord) {
    if job.status != JobStatus::Done && job.status != JobStatus::Failed {
        return;
    }
    if !shell.note_job(&job.id) || !shell.prefs().notify || any_focused(app) {
        return;
    }
    let verb = if job.status == JobStatus::Done {
        "finished"
    } else {
        "failed"
    };
    let who = job.provider.map(ProviderId::as_str).unwrap_or("Job");
    let title = format!("{who} {verb}");
    let body: String = job.prompt.chars().take(140).collect();
    let _ = app.notification().builder().title(title).body(body).show();
}

fn emit<T: Serialize + Clone>(app: &tauri::AppHandle, name: &str, payload: &T) {
    for window in app.webview_windows().values() {
        let _ = window.emit(name, payload);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LeadPayload {
    message: legate_core::LeadMessage,
    chat_id: Option<String>,
}

#[derive(Clone, Serialize)]
struct DataPayload {
    id: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct UpdatePayload {
    info: legate_core::TerminalInfo,
    removed: bool,
}
