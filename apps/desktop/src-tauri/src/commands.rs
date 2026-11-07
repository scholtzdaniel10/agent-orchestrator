//! One Tauri command per `AppHandle` method, plus the window-only commands.
//! Argument names are camelCase on the wire, matching the TypeScript renderer.

use legate_app::{GoalInput, TerminalKind, TriggerInput, WindowSight, WorktreePick};
use legate_core::{JobAccess, JobType, ProviderId};
use legate_lead::PlannedJob;
use serde::Deserialize;
use serde_json::Value;
use tauri::{Manager, State, Window};

use crate::state::{Shell, err_string};
use crate::windows;

const UPDATER_ENABLED: bool = false;

pub fn updater_enabled() -> bool {
    UPDATER_ENABLED
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalBody {
    text: String,
    check: Option<String>,
    max_rounds: Option<f64>,
    quick_type: Option<JobType>,
    quick_access: Option<JobAccess>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerBody {
    kind: String,
    at: Option<String>,
    hours: Option<f64>,
    text: Option<String>,
    check: Option<String>,
    max_rounds: Option<f64>,
}

fn cells(value: f64) -> i64 {
    value.round() as i64
}

fn kind_of(provider: &str) -> Result<TerminalKind, String> {
    if provider == "shell" {
        return Ok(TerminalKind::Shell);
    }
    ProviderId::parse(provider)
        .map(TerminalKind::Plan)
        .ok_or_else(|| format!("unknown provider {provider}"))
}

fn worktree_of(value: Option<String>) -> Option<WorktreePick> {
    match value.as_deref() {
        None | Some("") => None,
        Some("new") => Some(WorktreePick::New),
        Some(id) => Some(WorktreePick::Existing(id.to_string())),
    }
}

async fn parent_dir(app: &tauri::AppHandle, shell: &Shell) -> Result<Option<String>, String> {
    let prefs = shell.backend()?.get_prefs().await.map_err(err_string)?;
    if let Some(dir) = prefs.clone_dir
        && std::path::Path::new(&dir).is_dir()
    {
        return Ok(Some(dir));
    }
    Ok(windows::pick_folder(app))
}

#[tauri::command]
pub async fn submit_job(
    shell: State<'_, Shell>,
    r#type: JobType,
    prompt: String,
    provider: Option<ProviderId>,
    access: Option<JobAccess>,
    group: Option<String>,
) -> Result<legate_core::JobRecord, String> {
    shell
        .backend()?
        .submit_job(r#type, prompt, provider, access, group)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_jobs(shell: State<'_, Shell>) -> Result<Vec<legate_core::JobRecord>, String> {
    shell.backend()?.list_jobs().await.map_err(err_string)
}

#[tauri::command]
pub async fn cancel_job(
    shell: State<'_, Shell>,
    id: String,
) -> Result<legate_core::JobRecord, String> {
    shell.backend()?.cancel_job(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn refresh_usage(shell: State<'_, Shell>) -> Result<legate_app::UsageOutcome, String> {
    shell.backend()?.refresh_usage().await.map_err(err_string)
}

#[tauri::command]
pub async fn set_cursor_reading(
    shell: State<'_, Shell>,
    used_percent: f64,
    resets_at: f64,
) -> Result<legate_app::UsageOutcome, String> {
    shell
        .backend()?
        .set_cursor_reading(used_percent, cells(resets_at))
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn clear_cursor_reading(shell: State<'_, Shell>) -> Result<(), String> {
    shell
        .backend()?
        .clear_cursor_reading()
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn take_over_job(
    shell: State<'_, Shell>,
    id: String,
    cols: f64,
    rows: f64,
) -> Result<legate_core::TerminalInfo, String> {
    shell
        .backend()?
        .take_over_job(id, cells(cols), cells(rows))
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_plans(shell: State<'_, Shell>) -> Result<Vec<legate_core::PlanStatus>, String> {
    shell.backend()?.list_plans().await.map_err(err_string)
}

#[tauri::command]
pub async fn recheck_plans(
    shell: State<'_, Shell>,
) -> Result<Vec<legate_core::PlanStatus>, String> {
    shell.backend()?.recheck_plans().await.map_err(err_string)
}

#[tauri::command]
pub async fn list_models(
    shell: State<'_, Shell>,
    provider: ProviderId,
) -> Result<Vec<legate_core::ModelOption>, String> {
    shell
        .backend()?
        .list_models(provider)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn set_model(
    shell: State<'_, Shell>,
    provider: ProviderId,
    model: Option<String>,
) -> Result<(), String> {
    shell
        .backend()?
        .set_model(provider, model)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn get_project(shell: State<'_, Shell>) -> Result<legate_core::ProjectInfo, String> {
    shell.backend()?.get_project().await.map_err(err_string)
}

#[tauri::command]
pub async fn list_projects(
    shell: State<'_, Shell>,
) -> Result<Vec<legate_core::ProjectEntry>, String> {
    shell.backend()?.list_projects().await.map_err(err_string)
}

#[tauri::command]
pub async fn choose_project(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
) -> Result<Option<legate_core::ProjectInfo>, String> {
    let Some(path) = windows::pick_folder(&app) else {
        return Ok(None);
    };
    shell
        .backend()?
        .choose_project(path)
        .await
        .map(Some)
        .map_err(err_string)
}

#[tauri::command]
pub async fn switch_project(
    shell: State<'_, Shell>,
    path: String,
) -> Result<legate_core::ProjectInfo, String> {
    shell
        .backend()?
        .switch_project(path)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn remove_project(shell: State<'_, Shell>, path: String) -> Result<(), String> {
    shell
        .backend()?
        .remove_project(path)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn github_available(shell: State<'_, Shell>) -> Result<bool, String> {
    shell
        .backend()?
        .github_available()
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn github_repos(shell: State<'_, Shell>) -> Result<Vec<legate_core::GithubRepo>, String> {
    shell.backend()?.github_repos().await.map_err(err_string)
}

#[tauri::command]
pub async fn github_clone(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
    name_with_owner: String,
) -> Result<Option<legate_core::ProjectInfo>, String> {
    let Some(parent) = parent_dir(&app, &shell).await? else {
        return Ok(None);
    };
    shell
        .backend()?
        .github_clone(name_with_owner, parent)
        .await
        .map(Some)
        .map_err(err_string)
}

#[tauri::command]
pub async fn github_open_pr(
    shell: State<'_, Shell>,
    title: String,
    body: String,
) -> Result<legate_app::OpenedPr, String> {
    shell
        .backend()?
        .github_open_pr(title, body)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn setup_status(
    shell: State<'_, Shell>,
) -> Result<Vec<legate_lead::Requirement>, String> {
    shell.backend()?.setup_status().await.map_err(err_string)
}

#[tauri::command]
pub async fn setup_sign_in(
    shell: State<'_, Shell>,
    id: String,
) -> Result<legate_core::TerminalInfo, String> {
    shell.backend()?.setup_sign_in(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn skills_get(shell: State<'_, Shell>) -> Result<legate_core::SkillsState, String> {
    shell.backend()?.skills_get().await.map_err(err_string)
}

#[tauri::command]
pub async fn skills_add(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
) -> Result<legate_core::SkillsState, String> {
    let Some(folder) = windows::pick_folder(&app) else {
        return shell.backend()?.skills_get().await.map_err(err_string);
    };
    shell
        .backend()?
        .skills_add(folder)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn skills_remove(
    shell: State<'_, Shell>,
    folder: String,
) -> Result<legate_core::SkillsState, String> {
    shell
        .backend()?
        .skills_remove(folder)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn skills_reload(shell: State<'_, Shell>) -> Result<legate_core::SkillsState, String> {
    shell.backend()?.skills_reload().await.map_err(err_string)
}

#[tauri::command]
pub async fn list_changes(shell: State<'_, Shell>) -> Result<Vec<legate_core::ChangeSet>, String> {
    shell.backend()?.list_changes().await.map_err(err_string)
}

#[tauri::command]
pub async fn change_diff(shell: State<'_, Shell>, id: String) -> Result<String, String> {
    shell.backend()?.change_diff(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn merge_change(
    shell: State<'_, Shell>,
    id: String,
) -> Result<legate_app::MergeOutcome, String> {
    shell.backend()?.merge_change(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn discard_change(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell
        .backend()?
        .discard_change(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn create_worktree(
    shell: State<'_, Shell>,
    name: Option<String>,
) -> Result<legate_core::ChangeSet, String> {
    shell
        .backend()?
        .create_worktree(name)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn rename_worktree(
    shell: State<'_, Shell>,
    id: String,
    name: String,
) -> Result<(), String> {
    shell
        .backend()?
        .rename_worktree(id, name)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_files(
    shell: State<'_, Shell>,
    r#where: Option<String>,
) -> Result<legate_core::FileList, String> {
    shell
        .backend()?
        .list_files(r#where)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn read_file(
    shell: State<'_, Shell>,
    r#where: Option<String>,
    path: String,
) -> Result<legate_core::FileContent, String> {
    shell
        .backend()?
        .read_file(r#where, path)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn file_diff(
    shell: State<'_, Shell>,
    r#where: Option<String>,
    path: String,
) -> Result<String, String> {
    shell
        .backend()?
        .file_diff(r#where, path)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn resolve_file(
    shell: State<'_, Shell>,
    r#where: Option<String>,
    path: String,
) -> Result<String, String> {
    shell
        .backend()?
        .resolve_file(r#where, path)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn resolve_folder(
    shell: State<'_, Shell>,
    r#where: Option<String>,
) -> Result<String, String> {
    shell
        .backend()?
        .resolve_folder(r#where)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_goals(shell: State<'_, Shell>) -> Result<Vec<legate_lead::Goal>, String> {
    shell.backend()?.list_goals().await.map_err(err_string)
}

#[tauri::command]
pub async fn create_goal(
    shell: State<'_, Shell>,
    input: GoalBody,
) -> Result<legate_lead::Goal, String> {
    shell
        .backend()?
        .create_goal(GoalInput {
            text: input.text,
            check: input.check,
            max_rounds: input.max_rounds,
            quick_type: input.quick_type,
            quick_access: input.quick_access,
        })
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn approve_goal(
    shell: State<'_, Shell>,
    id: String,
    jobs: Option<Vec<PlannedJob>>,
) -> Result<legate_lead::Goal, String> {
    shell
        .backend()?
        .approve_goal(id, jobs)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn answer_goal(
    shell: State<'_, Shell>,
    id: String,
    text: String,
) -> Result<legate_lead::Goal, String> {
    shell
        .backend()?
        .answer_goal(id, text)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn stop_goal(shell: State<'_, Shell>, id: String) -> Result<legate_lead::Goal, String> {
    shell.backend()?.stop_goal(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn remove_goal(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell.backend()?.remove_goal(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn list_triggers(shell: State<'_, Shell>) -> Result<Vec<legate_lead::Trigger>, String> {
    shell.backend()?.list_triggers().await.map_err(err_string)
}

#[tauri::command]
pub async fn add_trigger(
    shell: State<'_, Shell>,
    input: TriggerBody,
) -> Result<legate_lead::Trigger, String> {
    shell
        .backend()?
        .add_trigger(TriggerInput {
            kind: input.kind,
            at: input.at,
            hours: input.hours,
            text: input.text,
            check: input.check,
            max_rounds: input.max_rounds,
        })
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn toggle_trigger(
    shell: State<'_, Shell>,
    id: String,
    enabled: bool,
) -> Result<legate_lead::Trigger, String> {
    shell
        .backend()?
        .toggle_trigger(id, enabled)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn remove_trigger(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell
        .backend()?
        .remove_trigger(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn run_trigger(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell.backend()?.run_trigger(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn commit_log(
    shell: State<'_, Shell>,
    r#where: Option<String>,
    skip: f64,
) -> Result<legate_app::CommitPage, String> {
    shell
        .backend()?
        .commit_log(r#where, cells(skip))
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn commit_detail(
    shell: State<'_, Shell>,
    r#where: Option<String>,
    hash: String,
) -> Result<legate_app::CommitDetail, String> {
    shell
        .backend()?
        .commit_detail(r#where, hash)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn commit_file_diff(
    shell: State<'_, Shell>,
    r#where: Option<String>,
    hash: String,
    path: String,
) -> Result<String, String> {
    shell
        .backend()?
        .commit_file_diff(r#where, hash, path)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn get_prefs(shell: State<'_, Shell>) -> Result<legate_core::Prefs, String> {
    let prefs = shell.backend()?.get_prefs().await.map_err(err_string)?;
    shell.set_prefs(prefs.clone());
    Ok(prefs)
}

#[tauri::command]
pub async fn set_prefs(
    shell: State<'_, Shell>,
    patch: Value,
) -> Result<legate_core::Prefs, String> {
    let prefs = shell
        .backend()?
        .set_prefs(patch)
        .await
        .map_err(err_string)?;
    shell.set_prefs(prefs.clone());
    Ok(prefs)
}

#[tauri::command]
pub async fn choose_clone_dir(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
) -> Result<Option<legate_core::Prefs>, String> {
    let Some(path) = windows::pick_folder(&app) else {
        return Ok(None);
    };
    let prefs = shell
        .backend()?
        .choose_clone_dir(path)
        .await
        .map_err(err_string)?;
    shell.set_prefs(prefs.clone());
    Ok(Some(prefs))
}

#[tauri::command]
pub async fn get_app_info(shell: State<'_, Shell>) -> Result<legate_core::AppInfo, String> {
    shell.backend()?.get_app_info().await.map_err(err_string)
}

#[tauri::command]
pub async fn get_project_setup(shell: State<'_, Shell>) -> Result<Vec<String>, String> {
    shell
        .backend()?
        .get_project_setup()
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn set_project_setup(
    shell: State<'_, Shell>,
    commands: Vec<String>,
) -> Result<(), String> {
    shell
        .backend()?
        .set_project_setup(commands)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_routines(shell: State<'_, Shell>) -> Result<Vec<legate_lead::Routine>, String> {
    shell.backend()?.list_routines().await.map_err(err_string)
}

#[tauri::command]
pub async fn save_routine(shell: State<'_, Shell>, routine: Value) -> Result<(), String> {
    shell
        .backend()?
        .save_routine(routine)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn remove_routine(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell
        .backend()?
        .remove_routine(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn run_routine(
    shell: State<'_, Shell>,
    id: String,
) -> Result<legate_core::JobRecord, String> {
    shell.backend()?.run_routine(id).await.map_err(err_string)
}

#[tauri::command]
pub async fn get_lead_plan(shell: State<'_, Shell>) -> Result<Option<ProviderId>, String> {
    shell.backend()?.get_lead_plan().await.map_err(err_string)
}

#[tauri::command]
pub async fn set_lead_plan(
    shell: State<'_, Shell>,
    plan: Option<ProviderId>,
) -> Result<(), String> {
    shell
        .backend()?
        .set_lead_plan(plan)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn get_lead_model(
    shell: State<'_, Shell>,
    provider: ProviderId,
) -> Result<Option<String>, String> {
    shell
        .backend()?
        .get_lead_model(provider)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn set_lead_model(
    shell: State<'_, Shell>,
    provider: ProviderId,
    model: Option<String>,
) -> Result<(), String> {
    shell
        .backend()?
        .set_lead_model(provider, model)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn get_lead_access(shell: State<'_, Shell>) -> Result<JobAccess, String> {
    shell.backend()?.get_lead_access().await.map_err(err_string)
}

#[tauri::command]
pub async fn set_lead_access(shell: State<'_, Shell>, access: JobAccess) -> Result<(), String> {
    shell
        .backend()?
        .set_lead_access(access)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn send_lead(
    shell: State<'_, Shell>,
    text: String,
) -> Result<legate_core::LeadMessage, String> {
    shell.backend()?.send_lead(text).await.map_err(err_string)
}

#[tauri::command]
pub async fn run_lead_proposal(
    shell: State<'_, Shell>,
    message_id: String,
    jobs: Option<Vec<legate_core::ProposedJob>>,
) -> Result<Vec<legate_core::JobRecord>, String> {
    shell
        .backend()?
        .run_lead_proposal(message_id, jobs)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn dismiss_lead_proposal(
    shell: State<'_, Shell>,
    message_id: String,
) -> Result<(), String> {
    shell
        .backend()?
        .dismiss_lead_proposal(message_id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_lead_messages(
    shell: State<'_, Shell>,
) -> Result<Vec<legate_core::LeadMessage>, String> {
    shell
        .backend()?
        .list_lead_messages()
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_lead_chats(
    shell: State<'_, Shell>,
) -> Result<Vec<legate_core::LeadChat>, String> {
    shell.backend()?.list_lead_chats().await.map_err(err_string)
}

#[tauri::command]
pub async fn open_lead_chat(
    shell: State<'_, Shell>,
    id: String,
) -> Result<Vec<legate_core::LeadMessage>, String> {
    shell
        .backend()?
        .open_lead_chat(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn reset_lead(shell: State<'_, Shell>, direct: Option<ProviderId>) -> Result<(), String> {
    shell
        .backend()?
        .reset_lead(direct)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn get_lead_mode(shell: State<'_, Shell>) -> Result<Option<ProviderId>, String> {
    shell.backend()?.get_lead_mode().await.map_err(err_string)
}

#[tauri::command]
pub async fn rename_lead_chat(
    shell: State<'_, Shell>,
    id: String,
    title: String,
) -> Result<(), String> {
    shell
        .backend()?
        .rename_lead_chat(id, title)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn remove_lead_chat(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell
        .backend()?
        .remove_lead_chat(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn open_terminal(
    shell: State<'_, Shell>,
    provider: String,
    cols: f64,
    rows: f64,
    worktree: Option<String>,
    prompt: Option<String>,
) -> Result<legate_core::TerminalInfo, String> {
    shell
        .backend()?
        .open_terminal(
            kind_of(&provider)?,
            cols,
            rows,
            worktree_of(worktree),
            prompt,
        )
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn write_terminal(
    window: Window,
    shell: State<'_, Shell>,
    id: String,
    data: String,
) -> Result<(), String> {
    let window_id = shell.window_id(window.label());
    shell
        .backend()?
        .write_terminal(id, data, window_id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn claim_terminal(
    window: Window,
    shell: State<'_, Shell>,
    id: String,
) -> Result<(), String> {
    let window_id = shell.window_id(window.label());
    shell
        .backend()?
        .claim_terminal(id, window_id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn resize_terminal(
    window: Window,
    shell: State<'_, Shell>,
    id: String,
    cols: f64,
    rows: f64,
) -> Result<(), String> {
    let window_id = shell.window_id(window.label());
    shell
        .backend()?
        .resize_terminal(id, cols, rows, window_id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn release_window(window: Window, shell: State<'_, Shell>) -> Result<(), String> {
    shell
        .backend()?
        .release_window(shell.window_id(window.label()))
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn close_terminal(shell: State<'_, Shell>, id: String) -> Result<(), String> {
    shell
        .backend()?
        .close_terminal(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn sleep_terminal(shell: State<'_, Shell>, id: String) -> Result<bool, String> {
    if shell.terminal_popped(&id) {
        return Ok(false);
    }
    shell
        .backend()?
        .sleep_terminal(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn wake_terminal(
    shell: State<'_, Shell>,
    id: String,
    cols: f64,
    rows: f64,
) -> Result<Option<legate_core::TerminalInfo>, String> {
    shell
        .backend()?
        .wake_terminal(id, cells(cols), cells(rows))
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn list_terminals(
    shell: State<'_, Shell>,
) -> Result<Vec<legate_core::TerminalInfo>, String> {
    shell.backend()?.list_terminals().await.map_err(err_string)
}

#[tauri::command]
pub async fn restore_terminals(
    shell: State<'_, Shell>,
    cols: f64,
    rows: f64,
) -> Result<Vec<legate_core::TerminalInfo>, String> {
    shell
        .backend()?
        .restore_terminals(cells(cols), cells(rows))
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn terminal_snapshot(shell: State<'_, Shell>, id: String) -> Result<String, String> {
    shell
        .backend()?
        .terminal_snapshot(id)
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn note_window_focus(shell: State<'_, Shell>) -> Result<(), String> {
    shell
        .backend()?
        .note_window_focus()
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn set_window_sight(
    shell: State<'_, Shell>,
    focused: bool,
    visible: bool,
) -> Result<(), String> {
    shell
        .backend()?
        .set_window_sight(WindowSight { focused, visible })
        .await
        .map_err(err_string)
}

#[tauri::command]
pub async fn new_window(
    app: tauri::AppHandle,
    window: Window,
    shell: State<'_, Shell>,
) -> Result<(), String> {
    let Some(from) = app.get_webview_window(window.label()) else {
        return Err("This window has closed.".into());
    };
    windows::new_window(&app, &shell, &from)
}

#[tauri::command]
pub async fn pop_out_terminal(
    app: tauri::AppHandle,
    window: Window,
    shell: State<'_, Shell>,
    id: String,
) -> Result<(), String> {
    let Some(from) = app.get_webview_window(window.label()) else {
        return Err("This window has closed.".into());
    };
    windows::pop_out_terminal(&app, &shell, &from, &id)
}

#[tauri::command]
pub async fn popped_out(shell: State<'_, Shell>) -> Result<Vec<String>, String> {
    Ok(shell.terminals().list())
}

#[tauri::command]
pub async fn pop_out_panel(
    app: tauri::AppHandle,
    window: Window,
    shell: State<'_, Shell>,
    tab: String,
) -> Result<(), String> {
    let Some(from) = app.get_webview_window(window.label()) else {
        return Err("This window has closed.".into());
    };
    windows::pop_out_panel(&app, &shell, &from, &tab)
}

#[tauri::command]
pub async fn close_panel(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
    tab: String,
) -> Result<(), String> {
    windows::close_panel(&app, &shell, &tab)
}

#[tauri::command]
pub async fn popped_panels(shell: State<'_, Shell>) -> Result<Vec<String>, String> {
    Ok(shell.panels().list())
}

#[tauri::command]
pub async fn open_file(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
    r#where: Option<String>,
    path: String,
) -> Result<(), String> {
    let absolute = shell
        .backend()?
        .resolve_file(r#where, path)
        .await
        .map_err(err_string)?;
    windows::open_path(&app, &absolute)
}

#[tauri::command]
pub async fn reveal_project(app: tauri::AppHandle, path: String) -> Result<(), String> {
    windows::open_path(&app, &path)
}

#[tauri::command]
pub async fn reveal_folder(
    app: tauri::AppHandle,
    shell: State<'_, Shell>,
    r#where: Option<String>,
) -> Result<(), String> {
    let path = shell
        .backend()?
        .resolve_folder(r#where)
        .await
        .map_err(err_string)?;
    windows::open_path(&app, &path)
}

#[tauri::command]
pub async fn open_data_dir(app: tauri::AppHandle, shell: State<'_, Shell>) -> Result<(), String> {
    let info = shell.backend()?.get_app_info().await.map_err(err_string)?;
    windows::open_path(&app, &info.data_dir)
}
