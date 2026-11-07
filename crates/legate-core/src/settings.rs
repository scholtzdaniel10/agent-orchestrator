//! Settings file from `src/core/settings.ts`, including the Live usage switch.

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::types::{
    CursorModels, DEFAULT_LOCAL_MODEL, DEFAULT_LOCAL_NUM_CTX, DEFAULT_LOCAL_URL, JobAccess,
    LOCAL_NUM_CTX_MAX, LOCAL_NUM_CTX_MIN, Prefs, ProviderId, UsageShows,
};
use crate::util::json_i64;

const MODEL_MAX: usize = 200;
const PREF_KEYS: &[&str] = &[
    "maxParallel",
    "terminalFontSize",
    "notify",
    "cloneDir",
    "uiScale",
    "usageShows",
    "cursorModels",
    "usageCheck",
    "localUrl",
    "localModel",
    "localSmallModel",
    "localNumCtx",
];

/// Window sizes the person can pick, in percent.
pub const UI_SCALES: [i64; 5] = [80, 90, 100, 110, 120];
pub const DEFAULT_UI_SCALE: i64 = 90;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid model name")]
    InvalidModel,
    #[error("cannot remove the active project")]
    CannotRemoveActive,
    #[error("access must be read, edit, or full")]
    InvalidAccess,
    #[error("invalid settings")]
    InvalidSettings,
    #[error("failed to save settings: {0}")]
    Io(#[from] io::Error),
    #[error("failed to write settings json: {0}")]
    Json(#[from] serde_json::Error),
}

/// A model id safe to pass as one command-line argument.
pub fn is_valid_model(value: &str) -> bool {
    let trimmed = value.trim();
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && trimmed.chars().count() <= MODEL_MAX
        && chars.all(is_model_char)
}

fn is_model_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '/' | '=' | ',' | '[' | ']' | '-')
}

pub struct Settings {
    path: PathBuf,
    data: Map<String, Value>,
}

impl Settings {
    /// Missing or corrupt files read as empty. The file is created on the first write.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let data = read_settings(&path);
        Self { path, data }
    }

    pub fn model(&self, provider: ProviderId) -> Option<String> {
        self.stored_model("models", provider)
    }

    /// Model the lead uses on this plan. Unset means the plan's own model.
    pub fn lead_model(&self, provider: ProviderId) -> Option<String> {
        self.stored_model("leadModels", provider)
    }

    pub fn set_model(
        &mut self,
        provider: ProviderId,
        model: Option<&str>,
    ) -> Result<(), SettingsError> {
        self.write_model("models", provider, model)
    }

    pub fn set_lead_model(
        &mut self,
        provider: ProviderId,
        model: Option<&str>,
    ) -> Result<(), SettingsError> {
        self.write_model("leadModels", provider, model)
    }

    pub fn project(&self) -> Option<String> {
        match self.data.get("project").and_then(Value::as_str) {
            Some(value) if !value.is_empty() => Some(value.to_string()),
            _ => None,
        }
    }

    pub fn projects(&self) -> Vec<String> {
        let mut list = read_projects(self.data.get("projects"));
        if let Some(current) = self.project()
            && !list.iter().any(|entry| entry == &current)
        {
            list.insert(0, current);
        }
        list
    }

    pub fn set_project(&mut self, path: &str) -> Result<(), SettingsError> {
        let mut list = self.projects();
        self.data
            .insert("project".to_string(), Value::String(path.to_string()));
        if !list.iter().any(|entry| entry == path) {
            list.push(path.to_string());
        }
        self.data
            .insert("projects".to_string(), serde_json::to_value(&list)?);
        self.persist()
    }

    pub fn remove_project(&mut self, path: &str) -> Result<(), SettingsError> {
        if self.project().as_deref() == Some(path) {
            return Err(SettingsError::CannotRemoveActive);
        }
        let list: Vec<String> = self
            .projects()
            .into_iter()
            .filter(|entry| entry != path)
            .collect();
        self.data
            .insert("projects".to_string(), serde_json::to_value(&list)?);
        self.persist()
    }

    pub fn lead_plan(&self) -> Option<ProviderId> {
        self.data
            .get("lead")
            .and_then(Value::as_str)
            .and_then(ProviderId::parse)
    }

    pub fn set_lead_plan(&mut self, plan: Option<ProviderId>) -> Result<(), SettingsError> {
        match plan {
            None => {
                self.data.remove("lead");
            }
            Some(plan) => {
                self.data
                    .insert("lead".to_string(), Value::String(plan.as_str().to_string()));
            }
        }
        self.persist()
    }

    pub fn lead_access(&self) -> JobAccess {
        self.data
            .get("leadAccess")
            .and_then(Value::as_str)
            .and_then(JobAccess::parse)
            .unwrap_or(JobAccess::Read)
    }

    pub fn set_lead_access(&mut self, value: JobAccess) -> Result<(), SettingsError> {
        self.data.insert(
            "leadAccess".to_string(),
            Value::String(value.as_str().to_string()),
        );
        self.persist()
    }

    /// Rejects a value that is not `read`, `edit`, or `full`.
    pub fn try_set_lead_access(&mut self, value: &str) -> Result<(), SettingsError> {
        let access = JobAccess::parse(value).ok_or(SettingsError::InvalidAccess)?;
        self.set_lead_access(access)
    }

    pub fn skill_folders(&self) -> Vec<String> {
        read_projects(self.data.get("skillFolders"))
    }

    pub fn set_skill_folders(&mut self, folders: &[&str]) -> Result<(), SettingsError> {
        let owned: Vec<Value> = folders
            .iter()
            .copied()
            .map(|folder| Value::String(folder.to_string()))
            .collect();
        let list = read_projects(Some(&Value::Array(owned)));
        self.data
            .insert("skillFolders".to_string(), serde_json::to_value(&list)?);
        self.persist()
    }

    pub fn prefs(&self) -> Prefs {
        let raw = self.data.get("prefs").and_then(Value::as_object);
        let get = |key: &str| raw.and_then(|map| map.get(key));
        Prefs {
            max_parallel: whole_in(get("maxParallel"), 1, 8),
            terminal_font_size: whole_in(get("terminalFontSize"), 10, 22).unwrap_or(13),
            notify: flag_on(get("notify")),
            clone_dir: match get("cloneDir").and_then(Value::as_str) {
                Some(dir) if !dir.is_empty() => Some(dir.to_string()),
                _ => None,
            },
            ui_scale: whole_in(get("uiScale"), i64::MIN, i64::MAX)
                .filter(|scale| UI_SCALES.contains(scale))
                .unwrap_or(DEFAULT_UI_SCALE),
            usage_shows: if get("usageShows").and_then(Value::as_str) == Some("used") {
                UsageShows::Used
            } else {
                UsageShows::Left
            },
            cursor_models: if get("cursorModels").and_then(Value::as_str) == Some("all") {
                CursorModels::All
            } else {
                CursorModels::Own
            },
            usage_check: flag_on(get("usageCheck")),
            local_url: local_url(get("localUrl")),
            local_model: required_model(get("localModel"), DEFAULT_LOCAL_MODEL),
            local_small_model: optional_model(get("localSmallModel")),
            local_num_ctx: local_num_ctx(get("localNumCtx")),
        }
    }

    /// Merges the known keys of `patch`. An invalid value falls back to its default.
    pub fn set_prefs(&mut self, patch: &Value) -> Result<Prefs, SettingsError> {
        let Some(patch) = patch.as_object() else {
            return Err(SettingsError::InvalidSettings);
        };
        let mut next = match self.data.get("prefs").and_then(Value::as_object) {
            Some(existing) => existing.clone(),
            None => Map::new(),
        };
        for key in PREF_KEYS {
            if let Some(value) = patch.get(*key) {
                next.insert((*key).to_string(), value.clone());
            }
        }
        self.data.insert("prefs".to_string(), Value::Object(next));
        let validated = self.prefs();
        self.data
            .insert("prefs".to_string(), serde_json::to_value(&validated)?);
        self.persist()?;
        Ok(validated)
    }

    fn stored_model(&self, key: &str, provider: ProviderId) -> Option<String> {
        let models = self.data.get(key)?.as_object()?;
        let value = models.get(provider.as_str())?.as_str()?;
        let trimmed = value.trim();
        is_valid_model(trimmed).then(|| trimmed.to_string())
    }

    fn write_model(
        &mut self,
        key: &str,
        provider: ProviderId,
        model: Option<&str>,
    ) -> Result<(), SettingsError> {
        let stored = match model {
            None => None,
            Some(model) => {
                let trimmed = model.trim();
                if !is_valid_model(trimmed) {
                    return Err(SettingsError::InvalidModel);
                }
                Some(trimmed.to_string())
            }
        };
        let mut models = match self.data.get(key).and_then(Value::as_object) {
            Some(existing) => existing.clone(),
            None => Map::new(),
        };
        match stored {
            None => {
                models.remove(provider.as_str());
            }
            Some(name) => {
                models.insert(provider.as_str().to_string(), Value::String(name));
            }
        }
        self.data.insert(key.to_string(), Value::Object(models));
        self.persist()
    }

    fn persist(&self) -> Result<(), SettingsError> {
        #[cfg(target_arch = "wasm32")]
        {
            return Err(SettingsError::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "settings files are not available on wasm ({})",
                    self.path.display()
                ),
            )));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(parent) = self.path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            let tmp = tmp_path(&self.path);
            let text = serde_json::to_string(&self.data)?;
            fs::write(&tmp, text)?;
            replace_file(&tmp, &self.path)?;
            Ok(())
        }
    }
}

fn read_settings(path: &Path) -> Map<String, Value> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        return Map::new();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Ok(text) = fs::read_to_string(path) else {
            return Map::new();
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        }
    }
}

fn read_projects(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in items {
        let Some(text) = entry.as_str() else {
            continue;
        };
        if text.trim().is_empty() || out.iter().any(|kept| kept == text) {
            continue;
        }
        out.push(text.to_string());
    }
    out
}

fn whole_in(value: Option<&Value>, min: i64, max: i64) -> Option<i64> {
    let number = json_i64(value?)?;
    (min..=max).contains(&number).then_some(number)
}

/// `notify` and `usageCheck` are on unless the stored value is boolean false.
fn flag_on(value: Option<&Value>) -> bool {
    !matches!(value, Some(Value::Bool(false)))
}

fn local_url(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .and_then(normalize_local_url)
        .unwrap_or_else(|| DEFAULT_LOCAL_URL.to_string())
}

/// `http://` or `https://` with a host. A trailing slash is removed.
fn normalize_local_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    let (scheme, rest) = if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "https://") {
        ("https", rest)
    } else {
        let rest = strip_prefix_ignore_ascii_case(trimmed, "http://")?;
        ("http", rest)
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if !value.is_char_boundary(prefix.len()) {
        return None;
    }
    let (head, tail) = value.split_at(prefix.len());
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

fn required_model(value: Option<&Value>, default: &str) -> String {
    match value.and_then(Value::as_str) {
        Some(text) if is_valid_model(text.trim()) => text.trim().to_string(),
        _ => default.to_string(),
    }
}

fn optional_model(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() || !is_valid_model(trimmed) {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => None,
    }
}

fn local_num_ctx(value: Option<&Value>) -> i64 {
    let Some(value) = value else {
        return DEFAULT_LOCAL_NUM_CTX;
    };
    let Some(number) = json_i64(value) else {
        return DEFAULT_LOCAL_NUM_CTX;
    };
    number.clamp(LOCAL_NUM_CTX_MIN, LOCAL_NUM_CTX_MAX)
}

#[cfg(not(target_arch = "wasm32"))]
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(not(target_arch = "wasm32"))]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if to.exists() {
        fs::remove_file(to)?;
        return fs::rename(from, to);
    }
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Temp {
        root: PathBuf,
    }

    impl Temp {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let root =
                std::env::temp_dir().join(format!("ao-settings-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn file(&self) -> PathBuf {
            self.root.join("settings.json")
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn saved(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn missing_file_reads_as_empty_and_set_model_round_trips() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        assert!(!path.exists());
        assert_eq!(settings.model(ProviderId::Claude), None);
        assert_eq!(settings.model(ProviderId::Cursor), None);

        settings
            .set_model(ProviderId::Claude, Some("  opus  "))
            .unwrap();
        settings
            .set_model(ProviderId::Cursor, Some("auto"))
            .unwrap();
        let again = Settings::open(&path);
        assert_eq!(again.model(ProviderId::Claude).as_deref(), Some("opus"));
        assert_eq!(again.model(ProviderId::Cursor).as_deref(), Some("auto"));

        let mut cleared_writer = again;
        cleared_writer.set_model(ProviderId::Claude, None).unwrap();
        let cleared = Settings::open(&path);
        assert_eq!(cleared.model(ProviderId::Claude), None);
        assert_eq!(cleared.model(ProviderId::Cursor).as_deref(), Some("auto"));
    }

    #[test]
    fn corrupt_json_is_empty_and_a_later_set_model_still_persists() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(&path, "{").unwrap();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.model(ProviderId::Claude), None);
        settings
            .set_model(ProviderId::Cursor, Some("auto"))
            .unwrap();
        assert_eq!(
            Settings::open(&path).model(ProviderId::Cursor).as_deref(),
            Some("auto")
        );
        assert_eq!(
            saved(&path),
            serde_json::json!({"models": {"cursor": "auto"}})
        );
    }

    #[test]
    fn an_invalid_stored_model_is_ignored_and_unknown_keys_survive_a_write() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::to_string(&serde_json::json!({
                "theme": "dark",
                "models": { "claude": "--force", "cursor": "auto", "extra": 1 }
            }))
            .unwrap(),
        )
        .unwrap();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.model(ProviderId::Claude), None);
        assert_eq!(settings.model(ProviderId::Cursor).as_deref(), Some("auto"));
        settings
            .set_model(ProviderId::Claude, Some("opus"))
            .unwrap();
        assert_eq!(
            saved(&path),
            serde_json::json!({
                "theme": "dark",
                "models": { "claude": "opus", "cursor": "auto", "extra": 1 }
            })
        );
    }

    #[test]
    fn set_model_rejects_names_that_must_not_become_command_line_arguments() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        settings
            .set_model(ProviderId::Claude, Some("opus"))
            .unwrap();
        let bad = [
            "",
            "   ",
            "--force",
            "-m",
            "a b",
            &"a".repeat(201),
            "opus\n4",
        ];
        for name in bad {
            let err = settings
                .set_model(ProviderId::Claude, Some(name))
                .unwrap_err();
            assert_eq!(err.to_string(), "invalid model name");
        }
        assert_eq!(
            Settings::open(&path).model(ProviderId::Claude).as_deref(),
            Some("opus")
        );
    }

    #[test]
    fn is_valid_model_accepts_cli_model_ids_including_bracketed_options() {
        assert!(is_valid_model("opus"));
        assert!(is_valid_model("grok-4.7-xhigh"));
        assert!(is_valid_model(
            "claude-opus-4-8[context=1m,effort=high,fast=false]"
        ));
        assert!(is_valid_model("gpt-5.2"));
        assert!(is_valid_model(&"a".repeat(200)));
        assert!(!is_valid_model(&"a".repeat(201)));
        assert!(!is_valid_model("--force"));
        assert!(!is_valid_model(""));
    }

    #[test]
    fn lead_plan_round_trips_null_clears_and_an_invalid_stored_value_is_ignored() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"lead": "nope", "models": {"claude": "opus"}}).to_string(),
        )
        .unwrap();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.lead_plan(), None);
        assert_eq!(settings.model(ProviderId::Claude).as_deref(), Some("opus"));

        settings.set_lead_plan(Some(ProviderId::Claude)).unwrap();
        let saved_settings = Settings::open(&path);
        assert_eq!(saved_settings.lead_plan(), Some(ProviderId::Claude));
        assert_eq!(
            saved_settings.model(ProviderId::Claude).as_deref(),
            Some("opus")
        );

        let mut saved_settings = saved_settings;
        saved_settings
            .set_lead_plan(Some(ProviderId::Cursor))
            .unwrap();
        assert_eq!(Settings::open(&path).lead_plan(), Some(ProviderId::Cursor));
        saved_settings
            .set_lead_plan(Some(ProviderId::Grok))
            .unwrap();
        assert_eq!(Settings::open(&path).lead_plan(), Some(ProviderId::Grok));
        saved_settings
            .set_model(ProviderId::Grok, Some("grok-4.7-high"))
            .unwrap();
        assert_eq!(
            Settings::open(&path).model(ProviderId::Grok).as_deref(),
            Some("grok-4.7-high")
        );
        saved_settings.set_model(ProviderId::Grok, None).unwrap();
        saved_settings.set_lead_plan(None).unwrap();
        let cleared = Settings::open(&path);
        assert_eq!(cleared.lead_plan(), None);
        assert_eq!(cleared.model(ProviderId::Claude).as_deref(), Some("opus"));
        assert_eq!(
            saved(&path),
            serde_json::json!({"models": {"claude": "opus"}})
        );
    }

    #[test]
    fn lead_access_defaults_to_read_round_trips_and_rejects_a_bad_value() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"leadAccess": "nope", "models": {"claude": "opus"}}).to_string(),
        )
        .unwrap();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.lead_access(), JobAccess::Read);
        assert_eq!(settings.model(ProviderId::Claude).as_deref(), Some("opus"));

        settings.set_lead_access(JobAccess::Edit).unwrap();
        assert_eq!(Settings::open(&path).lead_access(), JobAccess::Edit);
        settings.set_lead_access(JobAccess::Full).unwrap();
        assert_eq!(Settings::open(&path).lead_access(), JobAccess::Full);
        settings.set_lead_access(JobAccess::Read).unwrap();
        let again = Settings::open(&path);
        assert_eq!(again.lead_access(), JobAccess::Read);
        assert_eq!(again.model(ProviderId::Claude).as_deref(), Some("opus"));
        assert_eq!(
            saved(&path),
            serde_json::json!({"models": {"claude": "opus"}, "leadAccess": "read"})
        );
        let err = settings.try_set_lead_access("write").unwrap_err();
        assert_eq!(err.to_string(), "access must be read, edit, or full");
    }

    #[test]
    fn project_round_trips_and_ignores_a_stored_value_that_is_not_a_non_empty_string() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"project": "", "models": {"claude": "opus"}}).to_string(),
        )
        .unwrap();
        assert_eq!(Settings::open(&path).project(), None);
        fs::write(
            &path,
            serde_json::json!({"project": 4, "models": {"claude": "opus"}}).to_string(),
        )
        .unwrap();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.project(), None);
        assert_eq!(settings.model(ProviderId::Claude).as_deref(), Some("opus"));
        settings.set_project("repo").unwrap();
        let again = Settings::open(&path);
        assert_eq!(again.project().as_deref(), Some("repo"));
        assert_eq!(again.model(ProviderId::Claude).as_deref(), Some("opus"));
        assert_eq!(
            saved(&path),
            serde_json::json!({"project": "repo", "models": {"claude": "opus"}, "projects": ["repo"]})
        );
    }

    #[test]
    fn projects_round_trip_through_set_project_and_a_new_instance() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.projects(), Vec::<String>::new());
        settings.set_project("/a").unwrap();
        settings.set_project("/b").unwrap();
        assert_eq!(
            settings.projects(),
            vec!["/a".to_string(), "/b".to_string()]
        );
        assert_eq!(
            Settings::open(&path).projects(),
            vec!["/a".to_string(), "/b".to_string()]
        );
        assert_eq!(Settings::open(&path).project().as_deref(), Some("/b"));
    }

    #[test]
    fn projects_de_duplicate_and_keep_order() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"projects": ["/a", "/b", "/a", "/c", "/b"]}).to_string(),
        )
        .unwrap();
        assert_eq!(
            Settings::open(&path).projects(),
            vec!["/a".to_string(), "/b".to_string(), "/c".to_string()]
        );
    }

    #[test]
    fn projects_ignore_junk_values() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"projects": ["/ok", "", 4, null, "  ", "/also", {"x": 1}]})
                .to_string(),
        )
        .unwrap();
        assert_eq!(
            Settings::open(&path).projects(),
            vec!["/ok".to_string(), "/also".to_string()]
        );
    }

    #[test]
    fn projects_include_the_current_project_first_when_it_is_missing_from_the_list() {
        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"project": "/current", "projects": ["/a", "/b"]}).to_string(),
        )
        .unwrap();
        assert_eq!(
            Settings::open(&path).projects(),
            vec!["/current".to_string(), "/a".to_string(), "/b".to_string()]
        );
        fs::write(
            &path,
            serde_json::json!({"project": "/a", "projects": ["/a", "/b"]}).to_string(),
        )
        .unwrap();
        assert_eq!(
            Settings::open(&path).projects(),
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn remove_project_drops_a_folder_from_the_list() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        settings.set_project("/a").unwrap();
        settings.set_project("/b").unwrap();
        settings.remove_project("/a").unwrap();
        assert_eq!(settings.projects(), vec!["/b".to_string()]);
        assert_eq!(Settings::open(&path).projects(), vec!["/b".to_string()]);
        assert_eq!(settings.project().as_deref(), Some("/b"));
    }

    #[test]
    fn remove_project_throws_when_asked_to_remove_the_active_project() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        settings.set_project("/a").unwrap();
        settings.set_project("/b").unwrap();
        let err = settings.remove_project("/b").unwrap_err();
        assert_eq!(err.to_string(), "cannot remove the active project");
        assert_eq!(
            settings.projects(),
            vec!["/a".to_string(), "/b".to_string()]
        );
    }

    #[test]
    fn set_model_creates_a_missing_parent_folder() {
        let temp = Temp::new();
        let path = temp.root.join("nested").join("dir").join("settings.json");
        assert!(!temp.root.join("nested").exists());
        let mut settings = Settings::open(&path);
        settings
            .set_model(ProviderId::Cursor, Some("auto"))
            .unwrap();
        assert!(path.exists());
        assert!(!tmp_path(&path).exists());
        assert_eq!(
            Settings::open(&path).model(ProviderId::Cursor).as_deref(),
            Some("auto")
        );
    }

    #[test]
    fn prefs_default_validate_and_persist() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        let defaults = settings.prefs();
        assert_eq!(
            defaults,
            Prefs {
                max_parallel: None,
                terminal_font_size: 13,
                notify: true,
                clone_dir: None,
                ui_scale: 90,
                usage_shows: UsageShows::Left,
                cursor_models: CursorModels::Own,
                usage_check: true,
                local_url: DEFAULT_LOCAL_URL.to_string(),
                local_model: DEFAULT_LOCAL_MODEL.to_string(),
                local_small_model: None,
                local_num_ctx: DEFAULT_LOCAL_NUM_CTX,
            }
        );
        assert_eq!(
            serde_json::to_string(&defaults).unwrap(),
            concat!(
                r#"{"maxParallel":null,"terminalFontSize":13,"notify":true,"cloneDir":null,"#,
                r#""uiScale":90,"usageShows":"left","cursorModels":"own","usageCheck":true,"#,
                r#""localUrl":"http://127.0.0.1:11434","localModel":"coder-agent","#,
                r#""localSmallModel":null,"localNumCtx":65536}"#
            )
        );
        settings
            .set_prefs(&serde_json::json!({"maxParallel": 5, "notify": false, "unknown": "x"}))
            .unwrap();
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"uiScale": 110}))
                .unwrap()
                .ui_scale,
            110
        );
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"uiScale": 95}))
                .unwrap()
                .ui_scale,
            90
        );
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"usageShows": "used"}))
                .unwrap()
                .usage_shows,
            UsageShows::Used
        );
        assert_eq!(Settings::open(&path).prefs().usage_shows, UsageShows::Used);
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"usageShows": "both"}))
                .unwrap()
                .usage_shows,
            UsageShows::Left
        );
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"cursorModels": "all"}))
                .unwrap()
                .cursor_models,
            CursorModels::All
        );
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"cursorModels": "some"}))
                .unwrap()
                .cursor_models,
            CursorModels::Own
        );
        assert!(
            !settings
                .set_prefs(&serde_json::json!({"usageCheck": false}))
                .unwrap()
                .usage_check
        );
        let again = Settings::open(&path).prefs();
        assert_eq!(again.max_parallel, Some(5));
        assert!(!again.notify);
        assert!(!again.usage_check);
        let clamped = settings
            .set_prefs(&serde_json::json!({"maxParallel": 99, "terminalFontSize": 2}))
            .unwrap();
        assert_eq!(clamped.max_parallel, None);
        assert_eq!(clamped.terminal_font_size, 13);
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!("nope"))
                .unwrap_err()
                .to_string(),
            "invalid settings"
        );
    }

    #[test]
    fn local_model_prefs_default_clamp_and_fall_back() {
        let old = concat!(
            r#"{"maxParallel":null,"terminalFontSize":13,"notify":true,"cloneDir":null,"#,
            r#""uiScale":90,"usageShows":"left","cursorModels":"own","usageCheck":true}"#
        );
        let parsed: Prefs = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.local_url, DEFAULT_LOCAL_URL);
        assert_eq!(parsed.local_model, DEFAULT_LOCAL_MODEL);
        assert_eq!(parsed.local_small_model, None);
        assert_eq!(parsed.local_num_ctx, DEFAULT_LOCAL_NUM_CTX);

        let temp = Temp::new();
        let path = temp.file();
        fs::write(
            &path,
            serde_json::json!({"prefs": serde_json::from_str::<Value>(old).unwrap()}).to_string(),
        )
        .unwrap();
        let mut settings = Settings::open(&path);
        let loaded = settings.prefs();
        assert_eq!(loaded.local_url, DEFAULT_LOCAL_URL);
        assert_eq!(loaded.local_model, DEFAULT_LOCAL_MODEL);
        assert_eq!(loaded.local_small_model, None);
        assert_eq!(loaded.local_num_ctx, DEFAULT_LOCAL_NUM_CTX);
        assert_eq!(loaded.terminal_font_size, 13);

        let custom = settings
            .set_prefs(&serde_json::json!({
                "localUrl": " HTTP://127.0.0.1:8080/ ",
                "localModel": " coder-agent:latest ",
                "localSmallModel": "tiny-coder",
                "localNumCtx": 8192
            }))
            .unwrap();
        assert_eq!(custom.local_url, "http://127.0.0.1:8080");
        assert_eq!(custom.local_model, "coder-agent:latest");
        assert_eq!(custom.local_small_model.as_deref(), Some("tiny-coder"));
        assert_eq!(custom.local_num_ctx, 8192);
        assert_eq!(custom.terminal_font_size, 13);

        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"localNumCtx": 10}))
                .unwrap()
                .local_num_ctx,
            LOCAL_NUM_CTX_MIN
        );
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"localNumCtx": 9_999_999}))
                .unwrap()
                .local_num_ctx,
            LOCAL_NUM_CTX_MAX
        );
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"localNumCtx": "nope"}))
                .unwrap()
                .local_num_ctx,
            DEFAULT_LOCAL_NUM_CTX
        );
        let fallback = settings
            .set_prefs(&serde_json::json!({
                "localUrl": "ftp://files",
                "localModel": "--force",
                "localSmallModel": "bad model"
            }))
            .unwrap();
        assert_eq!(fallback.local_url, DEFAULT_LOCAL_URL);
        assert_eq!(fallback.local_model, DEFAULT_LOCAL_MODEL);
        assert_eq!(fallback.local_small_model, None);
        assert_eq!(
            settings
                .set_prefs(&serde_json::json!({"localSmallModel": null}))
                .unwrap()
                .local_small_model,
            None
        );
        let again = Settings::open(&path).prefs();
        assert_eq!(again.local_url, DEFAULT_LOCAL_URL);
        assert_eq!(again.local_model, DEFAULT_LOCAL_MODEL);
        assert_eq!(again.local_small_model, None);
        assert_eq!(again.local_num_ctx, DEFAULT_LOCAL_NUM_CTX);
        assert!(again.notify);
    }

    #[test]
    fn the_lead_model_is_its_own_apart_from_the_plan_model() {
        let temp = Temp::new();
        let path = temp.file();
        let mut settings = Settings::open(&path);
        assert_eq!(settings.lead_model(ProviderId::Claude), None);
        settings
            .set_model(ProviderId::Claude, Some("sonnet"))
            .unwrap();
        settings
            .set_lead_model(ProviderId::Claude, Some(" opus "))
            .unwrap();
        settings
            .set_lead_model(ProviderId::Grok, Some("grok-4.7"))
            .unwrap();
        let again = Settings::open(&path);
        assert_eq!(again.model(ProviderId::Claude).as_deref(), Some("sonnet"));
        assert_eq!(
            again.lead_model(ProviderId::Claude).as_deref(),
            Some("opus")
        );
        assert_eq!(
            again.lead_model(ProviderId::Grok).as_deref(),
            Some("grok-4.7")
        );
        assert_eq!(again.lead_model(ProviderId::Cursor), None);
        let mut again = again;
        again.set_lead_model(ProviderId::Claude, None).unwrap();
        assert_eq!(Settings::open(&path).lead_model(ProviderId::Claude), None);
        assert_eq!(
            Settings::open(&path).model(ProviderId::Claude).as_deref(),
            Some("sonnet")
        );
        let err = again
            .set_lead_model(ProviderId::Claude, Some("bad model; rm"))
            .unwrap_err();
        assert_eq!(err.to_string(), "invalid model name");
    }
}
