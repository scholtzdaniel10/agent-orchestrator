//! The same data folder and file names as the Electron app.

use std::fs;
use std::path::{Path, PathBuf};

use legate_core::{EnvVars, Platform, data_dir, legacy_data_dir};

use crate::error::AppError;

/// `orchestrator.sqlite` beside `settings.json`, under the Legate data folder.
pub fn database_path(dir: &Path) -> PathBuf {
    dir.join("orchestrator.sqlite")
}

/// `settings.json` in the data folder.
pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// Saved lead chats live in `{data}/lead`.
pub fn lead_dir(dir: &Path) -> PathBuf {
    dir.join("lead")
}

/// Where the installed app keeps its database, unless `options` or `LEGATE_DATA` says otherwise.
pub fn app_data_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    let legate = std::env::var("LEGATE_DATA").ok();
    let appdata = std::env::var("APPDATA").ok();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let platform = if cfg!(windows) {
        Platform::Win32
    } else if cfg!(target_os = "macos") {
        Platform::Darwin
    } else {
        Platform::Linux
    };
    PathBuf::from(data_dir(
        &EnvVars {
            legate_data: legate.as_deref(),
            appdata: appdata.as_deref(),
            xdg_config_home: xdg.as_deref(),
        },
        platform,
        &home,
    ))
}

/// `{home}/.orchestrator/wt`, or `ORCH_WORKTREES` when that is set.
pub fn default_worktrees_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    if let Ok(dir) = std::env::var("ORCH_WORKTREES")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".orchestrator").join("wt")
}

/// Copy `orchestrator.sqlite` and `settings.json` from the pre-1.0 folder when they are missing.
///
/// Skipped when the caller picked a data directory (the CLI's `LEGATE_DATA` rule). A temp
/// test directory must not inherit the person's real database.
pub fn bring_old_data(dir: &Path, allow: bool) -> Result<(), AppError> {
    if !allow {
        return Ok(());
    }
    let legate = std::env::var("LEGATE_DATA").ok();
    let appdata = std::env::var("APPDATA").ok();
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let platform = if cfg!(windows) {
        Platform::Win32
    } else if cfg!(target_os = "macos") {
        Platform::Darwin
    } else {
        Platform::Linux
    };
    let old = PathBuf::from(legacy_data_dir(
        &EnvVars {
            legate_data: legate.as_deref(),
            appdata: appdata.as_deref(),
            xdg_config_home: xdg.as_deref(),
        },
        platform,
        &home,
    ));
    if old == dir || !old.join("orchestrator.sqlite").is_file() {
        return Ok(());
    }
    if dir.join("orchestrator.sqlite").exists() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|err| AppError::message(err.to_string()))?;
    for name in ["orchestrator.sqlite", "settings.json"] {
        let from = old.join(name);
        let to = dir.join(name);
        if from.is_file() && !to.exists() {
            fs::copy(&from, &to).map_err(|err| AppError::message(err.to_string()))?;
        }
    }
    Ok(())
}

pub fn is_folder(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
}
