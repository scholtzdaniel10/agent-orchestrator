//! Extra windows: cascade, terminal pop-out, and panel pop-out.

use legate_app::{Bounds, Size, cascade_bounds, refuse_window};
use tauri::webview::WebviewWindowBuilder;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindow, WindowEvent};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;

use crate::state::Shell;

const PANEL_TABS: [&str; 9] = [
    "files", "lead", "jobs", "changes", "routines", "usage", "activity", "history", "goals",
];

pub fn on_event(shell: &Shell, window: &tauri::Window, event: &WindowEvent) {
    match event {
        WindowEvent::Focused(true) => {
            let app = window.app_handle().clone();
            let shell = shell.clone();
            tauri::async_runtime::spawn(async move {
                publish_sight(&app, &shell).await;
                if let Ok(backend) = shell.backend() {
                    let _ = backend.note_window_focus().await;
                }
            });
        }
        WindowEvent::Focused(false) | WindowEvent::Destroyed => {
            let label = window.label().to_string();
            if matches!(event, WindowEvent::Destroyed) {
                if let Some(id) = label.strip_prefix("term-") {
                    shell.terminals().remove(id);
                    let _ = window
                        .app_handle()
                        .emit("terminals:popped", shell.terminals().list());
                }
                if let Some(id) = label.strip_prefix("panel-") {
                    shell.panels().remove(id);
                    let _ = window
                        .app_handle()
                        .emit("panels:popped", shell.panels().list());
                }
                if let Some(window_id) = shell.forget_window(&label)
                    && let Ok(backend) = shell.backend()
                {
                    let backend = backend.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = backend.release_window(window_id).await;
                    });
                }
            }
            let app = window.app_handle().clone();
            let shell = shell.clone();
            tauri::async_runtime::spawn(async move {
                publish_sight(&app, &shell).await;
            });
        }
        _ => {}
    }
}

pub async fn publish_sight(app: &tauri::AppHandle, shell: &Shell) {
    let Ok(backend) = shell.backend() else {
        return;
    };
    let _ = backend.set_window_sight(current_sight(app)).await;
}

pub fn current_sight(app: &tauri::AppHandle) -> legate_app::WindowSight {
    let mut focused = false;
    let mut visible = false;
    for window in app.webview_windows().values() {
        if window.is_visible().unwrap_or(false) {
            visible = true;
        }
        if window.is_focused().unwrap_or(false) {
            focused = true;
        }
    }
    legate_app::WindowSight { focused, visible }
}

pub fn any_focused(app: &tauri::AppHandle) -> bool {
    app.webview_windows()
        .values()
        .any(|window| window.is_focused().unwrap_or(false))
}

pub fn apply_zoom(app: &tauri::AppHandle, percent: i64) {
    let factor = (percent as f64) / 100.0;
    for window in app.webview_windows().values() {
        let _ = window.set_zoom(factor);
    }
}

pub fn pick_folder(app: &tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(file_path_string)
}

pub fn open_path(app: &tauri::AppHandle, path: &str) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|err| err.to_string())
}

pub fn new_window(
    app: &tauri::AppHandle,
    shell: &Shell,
    from: &WebviewWindow,
) -> Result<(), String> {
    let count = app.webview_windows().len();
    if let Some(message) = refuse_window(count) {
        return Err(message.to_string());
    }
    let label = shell.fresh_label("win");
    open_page(app, shell, &label, "Legate", None, from, None)
}

pub fn pop_out_terminal(
    app: &tauri::AppHandle,
    shell: &Shell,
    from: &WebviewWindow,
    id: &str,
) -> Result<(), String> {
    if shell.terminal_popped(id) {
        if let Some(window) = app.get_webview_window(&format!("term-{id}")) {
            let _ = window.close();
        }
        shell.terminals().remove(id);
        let _ = app.emit("terminals:popped", shell.terminals().list());
        return Ok(());
    }
    let count = app.webview_windows().len();
    if let Some(message) = refuse_window(count) {
        return Err(message.to_string());
    }
    if !shell.terminals().add(id) {
        return Err("Already open in its own window.".into());
    }
    let _ = app.emit("terminals:popped", shell.terminals().list());
    let query = format!("terminal={id}");
    let opened = open_page(
        app,
        shell,
        &format!("term-{id}"),
        "Legate",
        Some(&query),
        from,
        Some((900, 600)),
    );
    if opened.is_err() {
        shell.terminals().remove(id);
        let _ = app.emit("terminals:popped", shell.terminals().list());
    }
    opened
}

pub fn pop_out_panel(
    app: &tauri::AppHandle,
    shell: &Shell,
    from: &WebviewWindow,
    tab: &str,
) -> Result<(), String> {
    let tab = require_panel(tab)?;
    let label = format!("panel-{tab}");
    if shell.panels().has(tab) {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        return Ok(());
    }
    let count = app.webview_windows().len();
    if let Some(message) = refuse_window(count) {
        return Err(message.to_string());
    }
    if !shell.panels().add(tab) {
        return Err("Already open in its own window.".into());
    }
    let _ = app.emit("panels:popped", shell.panels().list());
    let title = format!("{} · Legate", panel_title(tab));
    let query = format!("panel={tab}");
    let opened = open_page(
        app,
        shell,
        &label,
        &title,
        Some(&query),
        from,
        Some((760, 720)),
    );
    if opened.is_err() {
        shell.panels().remove(tab);
        let _ = app.emit("panels:popped", shell.panels().list());
    }
    opened
}

pub fn close_panel(app: &tauri::AppHandle, shell: &Shell, tab: &str) -> Result<(), String> {
    let tab = require_panel(tab)?;
    if let Some(window) = app.get_webview_window(&format!("panel-{tab}")) {
        let _ = window.close();
    }
    shell.panels().remove(tab);
    let _ = app.emit("panels:popped", shell.panels().list());
    Ok(())
}

fn require_panel(tab: &str) -> Result<&str, String> {
    if PANEL_TABS.contains(&tab) {
        Ok(tab)
    } else {
        Err("unknown panel".into())
    }
}

fn panel_title(tab: &str) -> &'static str {
    match tab {
        "files" => "Files",
        "lead" => "Lead",
        "jobs" => "Jobs",
        "changes" => "Changes",
        "routines" => "Routines",
        "usage" => "Usage",
        "activity" => "Activity",
        "history" => "History",
        "goals" => "Goals",
        _ => "Legate",
    }
}

fn open_page(
    app: &tauri::AppHandle,
    shell: &Shell,
    label: &str,
    title: &str,
    query: Option<&str>,
    from: &WebviewWindow,
    size: Option<(i64, i64)>,
) -> Result<(), String> {
    let asked = size.unwrap_or((1400, 820));
    let placed = place_from(from, asked);
    let url = page_url(query)?;
    let mut builder = WebviewWindowBuilder::new(app, label, url)
        .title(title)
        .inner_size(asked.0 as f64, asked.1 as f64);
    if let Some(bounds) = placed {
        builder = builder
            .inner_size(bounds.width as f64, bounds.height as f64)
            .position(bounds.x as f64, bounds.y as f64);
    }
    let window = builder.build().map_err(|err| err.to_string())?;
    let _ = shell.window_id(label);
    let _ = window.set_zoom((shell.prefs().ui_scale as f64) / 100.0);
    Ok(())
}

fn place_from(from: &WebviewWindow, asked: (i64, i64)) -> Option<Bounds> {
    let scale = from.scale_factor().ok()?;
    let origin = from.outer_position().ok()?.to_logical::<f64>(scale);
    let current = from.outer_size().ok()?.to_logical::<f64>(scale);
    let monitor = from.current_monitor().ok()??;
    let work = monitor.work_area();
    let factor = monitor.scale_factor();
    let area_at = work.position.to_logical::<f64>(factor);
    let area_size = work.size.to_logical::<f64>(factor);
    let from_bounds = Bounds {
        x: origin.x.round() as i64,
        y: origin.y.round() as i64,
        width: current.width.round() as i64,
        height: current.height.round() as i64,
    };
    let work_area = Bounds {
        x: area_at.x.round() as i64,
        y: area_at.y.round() as i64,
        width: area_size.width.round() as i64,
        height: area_size.height.round() as i64,
    };
    let width = if asked == (1400, 820) {
        from_bounds.width
    } else {
        asked.0
    };
    let height = if asked == (1400, 820) {
        from_bounds.height
    } else {
        asked.1
    };
    Some(cascade_bounds(
        from_bounds,
        Size { width, height },
        work_area,
    ))
}

fn page_url(query: Option<&str>) -> Result<WebviewUrl, String> {
    if cfg!(debug_assertions) {
        let mut text = "http://127.0.0.1:8080/".to_string();
        if let Some(query) = query {
            text.push('?');
            text.push_str(query);
        }
        let url = text
            .parse()
            .map_err(|err: url::ParseError| err.to_string())?;
        Ok(WebviewUrl::External(url))
    } else {
        let path = match query {
            Some(query) => format!("index.html?{query}"),
            None => "index.html".to_string(),
        };
        Ok(WebviewUrl::App(path.into()))
    }
}

fn file_path_string(path: FilePath) -> Option<String> {
    path.into_path()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn close_terminal_window(app: &tauri::AppHandle, shell: &Shell, id: &str) {
    if !shell.terminal_popped(id) {
        return;
    }
    if let Some(window) = app.get_webview_window(&format!("term-{id}")) {
        let _ = window.close();
    }
    shell.terminals().remove(id);
    let _ = app.emit("terminals:popped", shell.terminals().list());
}
