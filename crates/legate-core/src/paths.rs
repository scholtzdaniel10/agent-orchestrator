//! Data directories and project resolution from `src/cli/paths.ts`.
//!
//! `data_dir` joins with the requested platform's separator, not the host's.
//! `resolve_project` follows Node's `path.resolve` + `path.normalize` on the host.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Win32,
    Darwin,
    Linux,
}

/// Environment values the directory helpers read. Empty strings count as unset.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvVars<'a> {
    pub legate_data: Option<&'a str>,
    pub appdata: Option<&'a str>,
    pub xdg_config_home: Option<&'a str>,
}

/// Data folder the installed app uses for its product name, Legate.
pub fn data_dir(env: &EnvVars<'_>, platform: Platform, home: &str) -> String {
    if let Some(override_dir) = nonempty(env.legate_data) {
        return override_dir.to_string();
    }
    match platform {
        Platform::Win32 => match nonempty(env.appdata) {
            Some(roaming) => join(platform, &[roaming, "Legate"]),
            None => join(platform, &[home, "AppData", "Roaming", "Legate"]),
        },
        Platform::Darwin => join(
            platform,
            &[home, "Library", "Application Support", "Legate"],
        ),
        Platform::Linux => match nonempty(env.xdg_config_home) {
            Some(config) => join(platform, &[config, "Legate"]),
            None => join(platform, &[home, ".config", "Legate"]),
        },
    }
}

/// Folder the app used before it was renamed to Legate.
pub fn legacy_data_dir(env: &EnvVars<'_>, platform: Platform, home: &str) -> String {
    match platform {
        Platform::Win32 => match nonempty(env.appdata) {
            Some(roaming) => join(platform, &[roaming, "agent-orchestrator"]),
            None => join(
                platform,
                &[home, "AppData", "Roaming", "agent-orchestrator"],
            ),
        },
        Platform::Darwin => join(
            platform,
            &[home, "Library", "Application Support", "agent-orchestrator"],
        ),
        Platform::Linux => match nonempty(env.xdg_config_home) {
            Some(config) => join(platform, &[config, "agent-orchestrator"]),
            None => join(platform, &[home, ".config", "agent-orchestrator"]),
        },
    }
}

/// `--project` wins. Otherwise the git top-level of `cwd`, or `cwd` when it is not a repository.
///
/// `git_top` is synchronous: this crate has no async runtime, including on wasm.
pub fn resolve_project(
    cwd: &str,
    override_path: Option<&str>,
    git_top: impl FnOnce(&str) -> Option<String>,
) -> String {
    if let Some(over) = override_path.filter(|value| !value.is_empty()) {
        return normalize_abs(&Path::new(cwd).join(over));
    }
    match git_top(cwd).filter(|top| !top.is_empty()) {
        Some(top) => normalize_abs(Path::new(&top)),
        None => normalize_abs(Path::new(cwd)),
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}

fn join(platform: Platform, parts: &[&str]) -> String {
    let sep = if platform == Platform::Win32 {
        '\\'
    } else {
        '/'
    };
    let mut out = String::new();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(part);
            continue;
        }
        if !out.ends_with(sep) {
            out.push(sep);
        }
        let piece = part.trim_start_matches(['/', '\\']);
        out.push_str(piece);
    }
    out
}

/// Node's `normalize(resolve(path))`: absolute, `.` and `..` removed, no `\\?\` prefix.
fn normalize_abs(path: &Path) -> String {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = abs.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    trim_trailing_sep(text)
}

fn trim_trailing_sep(path: &str) -> String {
    if is_root(path) {
        return path.to_string();
    }
    path.trim_end_matches(['\\', '/']).to_string()
}

fn is_root(path: &str) -> bool {
    if path == "/" || path == "\\" {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() == 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_folder_follows_legate_data_then_the_platform_app_folder() {
        assert_eq!(
            data_dir(
                &EnvVars {
                    legate_data: Some(r"D:\custom"),
                    ..EnvVars::default()
                },
                Platform::Win32,
                r"C:\Users\a"
            ),
            r"D:\custom"
        );
        assert_eq!(
            data_dir(
                &EnvVars {
                    legate_data: Some(""),
                    ..EnvVars::default()
                },
                Platform::Linux,
                "/home/a"
            ),
            "/home/a/.config/Legate"
        );
        assert_eq!(
            data_dir(
                &EnvVars {
                    appdata: Some(r"C:\Users\a\AppData\Roaming"),
                    ..EnvVars::default()
                },
                Platform::Win32,
                r"C:\Users\a"
            ),
            r"C:\Users\a\AppData\Roaming\Legate"
        );
        assert_eq!(
            data_dir(&EnvVars::default(), Platform::Win32, r"C:\Users\a"),
            r"C:\Users\a\AppData\Roaming\Legate"
        );
        assert_eq!(
            data_dir(&EnvVars::default(), Platform::Darwin, "/Users/a"),
            "/Users/a/Library/Application Support/Legate"
        );
        assert_eq!(
            data_dir(
                &EnvVars {
                    xdg_config_home: Some("/xdg"),
                    ..EnvVars::default()
                },
                Platform::Linux,
                "/home/a"
            ),
            "/xdg/Legate"
        );
        assert_eq!(
            data_dir(&EnvVars::default(), Platform::Linux, "/home/a"),
            "/home/a/.config/Legate"
        );
    }

    #[test]
    fn legacy_data_folder_is_the_pre_rename_app_folder() {
        assert_eq!(
            legacy_data_dir(
                &EnvVars {
                    appdata: Some(r"C:\Roaming"),
                    ..EnvVars::default()
                },
                Platform::Win32,
                r"C:\Users\a"
            ),
            r"C:\Roaming\agent-orchestrator"
        );
        assert_eq!(
            legacy_data_dir(&EnvVars::default(), Platform::Darwin, "/Users/a"),
            "/Users/a/Library/Application Support/agent-orchestrator"
        );
        assert_eq!(
            legacy_data_dir(&EnvVars::default(), Platform::Linux, "/home/a"),
            "/home/a/.config/agent-orchestrator"
        );
    }

    #[test]
    fn resolve_project_prefers_project_then_git_top_then_the_directory() {
        let cwd = normalize_abs(Path::new("work").join("sub").as_path());
        let work = normalize_abs(Path::new("work"));
        assert_eq!(
            resolve_project(&cwd, None, |_| None),
            normalize_abs(Path::new(&cwd))
        );
        assert_eq!(
            resolve_project(&cwd, None, |_| Some(work.clone())),
            normalize_abs(Path::new(&work))
        );
        assert_eq!(
            resolve_project(&cwd, Some("other"), |_| Some(work)),
            normalize_abs(&Path::new(&cwd).join("other"))
        );
        // An empty override is the same as omitting it, so the git top still wins.
        assert_eq!(
            resolve_project(&cwd, Some(""), |_| Some("ignored".to_string())),
            normalize_abs(Path::new("ignored"))
        );
        assert_eq!(
            resolve_project(&cwd, None, |_| Some(String::new())),
            normalize_abs(Path::new(&cwd))
        );
    }
}
