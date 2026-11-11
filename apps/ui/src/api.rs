//! Typed calls into the Tauri shell. Every command and every event the shell emits.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use legate_core::{
    AppInfo, ChangeSet, FileContent, FileList, JobAccess, JobRecord, JobType, LeadChat,
    LeadMessage, ModelOption, PlanStatus, Prefs, ProjectEntry, ProjectInfo, ProposedJob,
    ProviderId, SkillsState, TerminalInfo,
};

use crate::wire::{
    CommitDetail, CommitPage, Goal, GoalInput, MergeOutcome, OpenedPr, Requirement, Routine,
    Trigger, TriggerInput, UsageOutcome,
};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadUpdate {
    pub message: LeadMessage,
    pub chat_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TerminalData {
    pub id: String,
    pub data: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TerminalUpdate {
    pub info: TerminalInfo,
    pub removed: bool,
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn tauri_listen(name: &str, cb: &js_sys::Function) -> Result<JsValue, JsValue>;
}

fn js_error(value: JsValue) -> String {
    if let Some(text) = value.as_string() {
        return text;
    }
    js_sys::Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| "The desktop bridge rejected the call.".into())
}

async fn call<T: DeserializeOwned>(cmd: &str, args: &impl Serialize) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(args).map_err(|err| err.to_string())?;
    let value = tauri_invoke(cmd, args).await.map_err(js_error)?;
    serde_wasm_bindgen::from_value(value).map_err(|err| err.to_string())
}

pub async fn listen<T, F>(name: &str, mut on_event: F) -> Result<(), String>
where
    T: DeserializeOwned + 'static,
    F: FnMut(T) + 'static,
{
    let closure = Closure::wrap(Box::new(move |event: JsValue| {
        let Ok(payload) = js_sys::Reflect::get(&event, &JsValue::from_str("payload")) else {
            return;
        };
        if let Ok(value) = serde_wasm_bindgen::from_value(payload) {
            on_event(value);
        }
    }) as Box<dyn FnMut(JsValue)>);
    let function: &js_sys::Function = closure.as_ref().unchecked_ref();
    tauri_listen(name, function).await.map_err(js_error)?;
    closure.forget();
    Ok(())
}

pub async fn submit_job(
    job_type: JobType,
    prompt: &str,
    provider: Option<ProviderId>,
    access: Option<JobAccess>,
    group: Option<&str>,
) -> Result<JobRecord, String> {
    call("submit_job", &json!({ "type": job_type, "prompt": prompt, "provider": provider, "access": access, "group": group })).await
}

pub async fn list_jobs() -> Result<Vec<JobRecord>, String> {
    call("list_jobs", &json!({})).await
}
pub async fn cancel_job(id: &str) -> Result<JobRecord, String> {
    call("cancel_job", &json!({ "id": id })).await
}
pub async fn refresh_usage() -> Result<UsageOutcome, String> {
    call("refresh_usage", &json!({})).await
}
pub async fn set_cursor_reading(used_percent: f64, resets_at: f64) -> Result<UsageOutcome, String> {
    call(
        "set_cursor_reading",
        &json!({ "usedPercent": used_percent, "resetsAt": resets_at }),
    )
    .await
}
pub async fn clear_cursor_reading() -> Result<(), String> {
    call("clear_cursor_reading", &json!({})).await
}
pub async fn take_over_job(id: &str, cols: f64, rows: f64) -> Result<TerminalInfo, String> {
    call(
        "take_over_job",
        &json!({ "id": id, "cols": cols, "rows": rows }),
    )
    .await
}
pub async fn list_plans() -> Result<Vec<PlanStatus>, String> {
    call("list_plans", &json!({})).await
}
pub async fn recheck_plans() -> Result<Vec<PlanStatus>, String> {
    call("recheck_plans", &json!({})).await
}
pub async fn list_models(provider: ProviderId) -> Result<Vec<ModelOption>, String> {
    call("list_models", &json!({ "provider": provider })).await
}
pub async fn set_model(provider: ProviderId, model: Option<&str>) -> Result<(), String> {
    call(
        "set_model",
        &json!({ "provider": provider, "model": model }),
    )
    .await
}
pub async fn get_project() -> Result<ProjectInfo, String> {
    call("get_project", &json!({})).await
}
pub async fn list_projects() -> Result<Vec<ProjectEntry>, String> {
    call("list_projects", &json!({})).await
}
pub async fn choose_project() -> Result<Option<ProjectInfo>, String> {
    call("choose_project", &json!({})).await
}
pub async fn switch_project(path: &str) -> Result<ProjectInfo, String> {
    call("switch_project", &json!({ "path": path })).await
}
pub async fn remove_project(path: &str) -> Result<(), String> {
    call("remove_project", &json!({ "path": path })).await
}
pub async fn github_available() -> Result<bool, String> {
    call("github_available", &json!({})).await
}
pub async fn github_repos() -> Result<Vec<legate_core::GithubRepo>, String> {
    call("github_repos", &json!({})).await
}
pub async fn github_clone(name_with_owner: &str) -> Result<Option<ProjectInfo>, String> {
    call("github_clone", &json!({ "nameWithOwner": name_with_owner })).await
}
pub async fn github_open_pr(title: &str, body: &str) -> Result<OpenedPr, String> {
    call("github_open_pr", &json!({ "title": title, "body": body })).await
}
pub async fn setup_status() -> Result<Vec<Requirement>, String> {
    call("setup_status", &json!({})).await
}
pub async fn setup_sign_in(id: &str) -> Result<TerminalInfo, String> {
    call("setup_sign_in", &json!({ "id": id })).await
}
pub async fn skills_get() -> Result<SkillsState, String> {
    call("skills_get", &json!({})).await
}
pub async fn skills_add() -> Result<SkillsState, String> {
    call("skills_add", &json!({})).await
}
pub async fn skills_remove(folder: &str) -> Result<SkillsState, String> {
    call("skills_remove", &json!({ "folder": folder })).await
}
pub async fn skills_reload() -> Result<SkillsState, String> {
    call("skills_reload", &json!({})).await
}
pub async fn list_changes() -> Result<Vec<ChangeSet>, String> {
    call("list_changes", &json!({})).await
}
pub async fn change_diff(id: &str) -> Result<String, String> {
    call("change_diff", &json!({ "id": id })).await
}
pub async fn merge_change(id: &str) -> Result<MergeOutcome, String> {
    call("merge_change", &json!({ "id": id })).await
}
pub async fn discard_change(id: &str) -> Result<(), String> {
    call("discard_change", &json!({ "id": id })).await
}
pub async fn create_worktree(name: Option<&str>) -> Result<ChangeSet, String> {
    call("create_worktree", &json!({ "name": name })).await
}
pub async fn rename_worktree(id: &str, name: &str) -> Result<(), String> {
    call("rename_worktree", &json!({ "id": id, "name": name })).await
}
pub async fn list_files(where_id: Option<&str>) -> Result<FileList, String> {
    call("list_files", &json!({ "where": where_id })).await
}
pub async fn read_file(where_id: Option<&str>, path: &str) -> Result<FileContent, String> {
    call("read_file", &json!({ "where": where_id, "path": path })).await
}
pub async fn file_diff(where_id: Option<&str>, path: &str) -> Result<String, String> {
    call("file_diff", &json!({ "where": where_id, "path": path })).await
}
pub async fn resolve_file(where_id: Option<&str>, path: &str) -> Result<String, String> {
    call("resolve_file", &json!({ "where": where_id, "path": path })).await
}
pub async fn resolve_folder(where_id: Option<&str>) -> Result<String, String> {
    call("resolve_folder", &json!({ "where": where_id })).await
}
pub async fn list_goals() -> Result<Vec<Goal>, String> {
    call("list_goals", &json!({})).await
}
pub async fn create_goal(input: &GoalInput) -> Result<Goal, String> {
    call("create_goal", &json!({ "input": input })).await
}
pub async fn approve_goal(
    id: &str,
    jobs: Option<&[crate::wire::PlannedJob]>,
) -> Result<Goal, String> {
    call("approve_goal", &json!({ "id": id, "jobs": jobs })).await
}
pub async fn answer_goal(id: &str, text: &str) -> Result<Goal, String> {
    call("answer_goal", &json!({ "id": id, "text": text })).await
}
pub async fn stop_goal(id: &str) -> Result<Goal, String> {
    call("stop_goal", &json!({ "id": id })).await
}
pub async fn remove_goal(id: &str) -> Result<(), String> {
    call("remove_goal", &json!({ "id": id })).await
}
pub async fn list_triggers() -> Result<Vec<Trigger>, String> {
    call("list_triggers", &json!({})).await
}
pub async fn add_trigger(input: &TriggerInput) -> Result<Trigger, String> {
    call("add_trigger", &json!({ "input": input })).await
}
pub async fn toggle_trigger(id: &str, enabled: bool) -> Result<Trigger, String> {
    call("toggle_trigger", &json!({ "id": id, "enabled": enabled })).await
}
pub async fn remove_trigger(id: &str) -> Result<(), String> {
    call("remove_trigger", &json!({ "id": id })).await
}
pub async fn run_trigger(id: &str) -> Result<(), String> {
    call("run_trigger", &json!({ "id": id })).await
}
pub async fn commit_log(where_id: Option<&str>, skip: f64) -> Result<CommitPage, String> {
    call("commit_log", &json!({ "where": where_id, "skip": skip })).await
}
pub async fn commit_detail(where_id: Option<&str>, hash: &str) -> Result<CommitDetail, String> {
    call("commit_detail", &json!({ "where": where_id, "hash": hash })).await
}
pub async fn commit_file_diff(
    where_id: Option<&str>,
    hash: &str,
    path: &str,
) -> Result<String, String> {
    call(
        "commit_file_diff",
        &json!({ "where": where_id, "hash": hash, "path": path }),
    )
    .await
}
pub async fn get_prefs() -> Result<Prefs, String> {
    call("get_prefs", &json!({})).await
}
pub async fn set_prefs(patch: &Value) -> Result<Prefs, String> {
    call("set_prefs", &json!({ "patch": patch })).await
}
pub async fn choose_clone_dir() -> Result<Option<Prefs>, String> {
    call("choose_clone_dir", &json!({})).await
}
pub async fn get_app_info() -> Result<AppInfo, String> {
    call("get_app_info", &json!({})).await
}
pub async fn get_project_setup() -> Result<Vec<String>, String> {
    call("get_project_setup", &json!({})).await
}
pub async fn set_project_setup(commands: &[String]) -> Result<(), String> {
    call("set_project_setup", &json!({ "commands": commands })).await
}
pub async fn list_routines() -> Result<Vec<Routine>, String> {
    call("list_routines", &json!({})).await
}
pub async fn save_routine(routine: &Value) -> Result<(), String> {
    call("save_routine", &json!({ "routine": routine })).await
}
pub async fn remove_routine(id: &str) -> Result<(), String> {
    call("remove_routine", &json!({ "id": id })).await
}
pub async fn run_routine(id: &str) -> Result<JobRecord, String> {
    call("run_routine", &json!({ "id": id })).await
}
pub async fn get_lead_plan() -> Result<Option<ProviderId>, String> {
    call("get_lead_plan", &json!({})).await
}
pub async fn set_lead_plan(plan: Option<ProviderId>) -> Result<(), String> {
    call("set_lead_plan", &json!({ "plan": plan })).await
}
pub async fn get_lead_model(provider: ProviderId) -> Result<Option<String>, String> {
    call("get_lead_model", &json!({ "provider": provider })).await
}
pub async fn set_lead_model(provider: ProviderId, model: Option<&str>) -> Result<(), String> {
    call(
        "set_lead_model",
        &json!({ "provider": provider, "model": model }),
    )
    .await
}
pub async fn get_lead_access() -> Result<JobAccess, String> {
    call("get_lead_access", &json!({})).await
}
pub async fn set_lead_access(access: JobAccess) -> Result<(), String> {
    call("set_lead_access", &json!({ "access": access })).await
}
pub async fn send_lead(text: &str) -> Result<LeadMessage, String> {
    call("send_lead", &json!({ "text": text })).await
}
pub async fn run_lead_proposal(
    message_id: &str,
    jobs: Option<&[ProposedJob]>,
) -> Result<Vec<JobRecord>, String> {
    call(
        "run_lead_proposal",
        &json!({ "messageId": message_id, "jobs": jobs }),
    )
    .await
}
pub async fn dismiss_lead_proposal(message_id: &str) -> Result<(), String> {
    call("dismiss_lead_proposal", &json!({ "messageId": message_id })).await
}
pub async fn list_lead_messages() -> Result<Vec<LeadMessage>, String> {
    call("list_lead_messages", &json!({})).await
}
pub async fn list_lead_chats() -> Result<Vec<LeadChat>, String> {
    call("list_lead_chats", &json!({})).await
}
pub async fn open_lead_chat(id: &str) -> Result<Vec<LeadMessage>, String> {
    call("open_lead_chat", &json!({ "id": id })).await
}
pub async fn reset_lead(direct: Option<ProviderId>) -> Result<(), String> {
    call("reset_lead", &json!({ "direct": direct })).await
}
pub async fn get_lead_mode() -> Result<Option<ProviderId>, String> {
    call("get_lead_mode", &json!({})).await
}
pub async fn rename_lead_chat(id: &str, title: &str) -> Result<(), String> {
    call("rename_lead_chat", &json!({ "id": id, "title": title })).await
}
pub async fn remove_lead_chat(id: &str) -> Result<(), String> {
    call("remove_lead_chat", &json!({ "id": id })).await
}
pub async fn open_terminal(
    provider: &str,
    cols: f64,
    rows: f64,
    worktree: Option<&str>,
    prompt: Option<&str>,
) -> Result<TerminalInfo, String> {
    call("open_terminal", &json!({ "provider": provider, "cols": cols, "rows": rows, "worktree": worktree, "prompt": prompt })).await
}
pub async fn write_terminal(id: &str, data: &str) -> Result<(), String> {
    call("write_terminal", &json!({ "id": id, "data": data })).await
}
pub async fn claim_terminal(id: &str) -> Result<(), String> {
    call("claim_terminal", &json!({ "id": id })).await
}
pub async fn resize_terminal(id: &str, cols: f64, rows: f64) -> Result<(), String> {
    call(
        "resize_terminal",
        &json!({ "id": id, "cols": cols, "rows": rows }),
    )
    .await
}
pub async fn release_window() -> Result<(), String> {
    call("release_window", &json!({})).await
}
pub async fn close_terminal(id: &str) -> Result<(), String> {
    call("close_terminal", &json!({ "id": id })).await
}
pub async fn sleep_terminal(id: &str) -> Result<bool, String> {
    call("sleep_terminal", &json!({ "id": id })).await
}
pub async fn wake_terminal(id: &str, cols: f64, rows: f64) -> Result<Option<TerminalInfo>, String> {
    call(
        "wake_terminal",
        &json!({ "id": id, "cols": cols, "rows": rows }),
    )
    .await
}
pub async fn list_terminals() -> Result<Vec<TerminalInfo>, String> {
    call("list_terminals", &json!({})).await
}
pub async fn list_project_terminals() -> Result<Vec<TerminalInfo>, String> {
    call("list_project_terminals", &json!({})).await
}
pub async fn restore_terminals(cols: f64, rows: f64) -> Result<Vec<TerminalInfo>, String> {
    call("restore_terminals", &json!({ "cols": cols, "rows": rows })).await
}
pub async fn terminal_snapshot(id: &str) -> Result<String, String> {
    call("terminal_snapshot", &json!({ "id": id })).await
}
pub async fn note_window_focus() -> Result<(), String> {
    call("note_window_focus", &json!({})).await
}
pub async fn set_window_sight(focused: bool, visible: bool) -> Result<(), String> {
    call(
        "set_window_sight",
        &json!({ "focused": focused, "visible": visible }),
    )
    .await
}
pub async fn new_window() -> Result<(), String> {
    call("new_window", &json!({})).await
}
pub async fn pop_out_terminal(id: &str) -> Result<(), String> {
    call("pop_out_terminal", &json!({ "id": id })).await
}
pub async fn popped_out() -> Result<Vec<String>, String> {
    call("popped_out", &json!({})).await
}
pub async fn pop_out_panel(tab: &str) -> Result<(), String> {
    call("pop_out_panel", &json!({ "tab": tab })).await
}
pub async fn close_panel(tab: &str) -> Result<(), String> {
    call("close_panel", &json!({ "tab": tab })).await
}
pub async fn popped_panels() -> Result<Vec<String>, String> {
    call("popped_panels", &json!({})).await
}
pub async fn open_file(where_id: Option<&str>, path: &str) -> Result<(), String> {
    call("open_file", &json!({ "where": where_id, "path": path })).await
}
pub async fn reveal_project(path: &str) -> Result<(), String> {
    call("reveal_project", &json!({ "path": path })).await
}
pub async fn reveal_folder(where_id: Option<&str>) -> Result<(), String> {
    call("reveal_folder", &json!({ "where": where_id })).await
}
pub async fn open_data_dir() -> Result<(), String> {
    call("open_data_dir", &json!({})).await
}

pub async fn on_jobs(on_event: impl FnMut(JobRecord) + 'static) -> Result<(), String> {
    listen("jobs:update", on_event).await
}
pub async fn on_plans(on_event: impl FnMut(Vec<PlanStatus>) + 'static) -> Result<(), String> {
    listen("plans:update", on_event).await
}
pub async fn on_changes(on_event: impl FnMut(Vec<ChangeSet>) + 'static) -> Result<(), String> {
    listen("changes:update", on_event).await
}
pub async fn on_goals(on_event: impl FnMut(Goal) + 'static) -> Result<(), String> {
    listen("goals:update", on_event).await
}
pub async fn on_triggers(on_event: impl FnMut(Vec<Trigger>) + 'static) -> Result<(), String> {
    listen("triggers:update", on_event).await
}
pub async fn on_prefs(on_event: impl FnMut(Prefs) + 'static) -> Result<(), String> {
    listen("prefs:update", on_event).await
}
pub async fn on_lead(on_event: impl FnMut(LeadUpdate) + 'static) -> Result<(), String> {
    listen("lead:update", on_event).await
}
pub async fn on_terminal_data(on_event: impl FnMut(TerminalData) + 'static) -> Result<(), String> {
    listen("terminals:data", on_event).await
}
pub async fn on_terminal_update(
    on_event: impl FnMut(TerminalUpdate) + 'static,
) -> Result<(), String> {
    listen("terminals:update", on_event).await
}
pub async fn on_popped_out(on_event: impl FnMut(Vec<String>) + 'static) -> Result<(), String> {
    listen("terminals:popped", on_event).await
}
pub async fn on_popped_panels(on_event: impl FnMut(Vec<String>) + 'static) -> Result<(), String> {
    listen("panels:popped", on_event).await
}
