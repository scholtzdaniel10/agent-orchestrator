#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Legate's desktop shell. The backend is `legate_app::AppHandle`; this process
//! owns windows, dialogs, and the events the webview listens for.

mod commands;
mod events;
mod state;
mod windows;

use tauri::{Manager, WindowEvent};

const UPDATER_ENABLED: bool = false;

fn main() -> anyhow::Result<()> {
    let shell = state::Shell::boot();
    let events_shell = shell.clone();
    let window_shell = shell.clone();
    let managed = shell.clone();
    let mut builder =
        tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    if UPDATER_ENABLED {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }
    builder
        .manage(managed)
        .on_window_event(move |window, event| {
            if matches!(event, WindowEvent::Focused(_) | WindowEvent::Destroyed) {
                windows::on_event(&window_shell, window, event);
            }
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            let prefs = events_shell.prefs();
            windows::apply_zoom(&handle, prefs.ui_scale);
            events::spawn(handle.clone(), events_shell.clone());
            let shell = events_shell.clone();
            tauri::async_runtime::spawn(async move {
                windows::publish_sight(&handle, &shell).await;
            });
            let _ = commands::updater_enabled();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::submit_job,
            commands::list_jobs,
            commands::cancel_job,
            commands::refresh_usage,
            commands::set_cursor_reading,
            commands::clear_cursor_reading,
            commands::take_over_job,
            commands::list_plans,
            commands::recheck_plans,
            commands::list_models,
            commands::set_model,
            commands::get_project,
            commands::list_projects,
            commands::choose_project,
            commands::switch_project,
            commands::remove_project,
            commands::github_available,
            commands::github_repos,
            commands::github_clone,
            commands::github_open_pr,
            commands::setup_status,
            commands::setup_sign_in,
            commands::skills_get,
            commands::skills_add,
            commands::skills_remove,
            commands::skills_reload,
            commands::list_changes,
            commands::change_diff,
            commands::merge_change,
            commands::discard_change,
            commands::create_worktree,
            commands::rename_worktree,
            commands::list_files,
            commands::read_file,
            commands::file_diff,
            commands::resolve_file,
            commands::resolve_folder,
            commands::list_goals,
            commands::create_goal,
            commands::approve_goal,
            commands::answer_goal,
            commands::stop_goal,
            commands::remove_goal,
            commands::list_triggers,
            commands::add_trigger,
            commands::toggle_trigger,
            commands::remove_trigger,
            commands::run_trigger,
            commands::commit_log,
            commands::commit_detail,
            commands::commit_file_diff,
            commands::get_prefs,
            commands::set_prefs,
            commands::choose_clone_dir,
            commands::get_app_info,
            commands::get_project_setup,
            commands::set_project_setup,
            commands::list_routines,
            commands::save_routine,
            commands::remove_routine,
            commands::run_routine,
            commands::get_lead_plan,
            commands::set_lead_plan,
            commands::get_lead_model,
            commands::set_lead_model,
            commands::get_lead_access,
            commands::set_lead_access,
            commands::send_lead,
            commands::run_lead_proposal,
            commands::dismiss_lead_proposal,
            commands::list_lead_messages,
            commands::list_lead_chats,
            commands::open_lead_chat,
            commands::reset_lead,
            commands::get_lead_mode,
            commands::rename_lead_chat,
            commands::remove_lead_chat,
            commands::open_terminal,
            commands::write_terminal,
            commands::claim_terminal,
            commands::resize_terminal,
            commands::release_window,
            commands::close_terminal,
            commands::sleep_terminal,
            commands::wake_terminal,
            commands::list_terminals,
            commands::list_project_terminals,
            commands::restore_terminals,
            commands::terminal_snapshot,
            commands::note_window_focus,
            commands::set_window_sight,
            commands::new_window,
            commands::pop_out_terminal,
            commands::popped_out,
            commands::pop_out_panel,
            commands::close_panel,
            commands::popped_panels,
            commands::open_file,
            commands::reveal_project,
            commands::reveal_folder,
            commands::open_data_dir,
        ])
        .build(tauri::generate_context!())
        .map_err(|err| anyhow::anyhow!(err))?
        .run(|_app, _event| {});
    Ok(())
}
