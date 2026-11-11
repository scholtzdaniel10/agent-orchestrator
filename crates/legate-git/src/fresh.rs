//! Windows PATH refresh from the registry, so a tool installed after launch is visible.

use std::collections::HashSet;

use legate_core::Platform;

use crate::cmd::CmdRun;

const USER_KEY: &str = r"HKCU\Environment";
const MACHINE_KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvMap {
    pairs: Vec<(String, String)>,
}

impl EnvMap {
    pub fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some(existing) = self
            .pairs
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(&key))
        {
            existing.1 = value;
        } else {
            self.pairs.push((key, value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    pub fn path(&self) -> &str {
        self.get("PATH").unwrap_or("")
    }

    pub fn set_path(&mut self, value: impl Into<String>) {
        self.insert("PATH", value);
    }
}

pub async fn refresh_path(run: &impl CmdRun, env: &mut EnvMap, platform: Platform) {
    if platform != Platform::Win32 {
        return;
    }
    let machine = run.run(
        "reg".to_string(),
        vec![
            "query".into(),
            MACHINE_KEY.into(),
            "/v".into(),
            "Path".into(),
        ],
        None,
    );
    let user = run.run(
        "reg".to_string(),
        vec!["query".into(), USER_KEY.into(), "/v".into(), "Path".into()],
        None,
    );
    let (machine, user) = tokio::join!(machine, user);
    let mut found = Vec::new();
    for result in [machine, user] {
        if result.code == 0 {
            found.push(registry_path(&result.stdout));
        }
    }
    let current = env.path().to_string();
    let merged = merged_path(&current, &found, env);
    env.set_path(merged);
}

/// The value of `Path` in `reg query <key> /v Path`, or empty when there is none.
pub fn registry_path(output: &str) -> String {
    for line in output.split(['\n', '\r']) {
        let line = line.trim();
        if line.len() < 4 || !line[..4].eq_ignore_ascii_case("path") {
            continue;
        }
        let after = line[4..].trim_start();
        let value = if after.len() >= 13 && after[..13].eq_ignore_ascii_case("REG_EXPAND_SZ") {
            after[13..].trim_start()
        } else if after.len() >= 6 && after[..6].eq_ignore_ascii_case("REG_SZ") {
            after[6..].trim_start()
        } else {
            continue;
        };
        return value.trim_end().to_string();
    }
    String::new()
}

/// Current PATH, then each registry entry it lacks, with `%NAME%` expanded.
pub fn merged_path(current: &str, from_registry: &[String], env: &EnvMap) -> String {
    let mut entries: Vec<String> = current
        .split(';')
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect();
    let mut seen: HashSet<String> = entries.iter().map(|entry| path_key(entry)).collect();
    for raw in from_registry {
        for entry in raw.split(';') {
            let expanded = expand_vars(entry.trim(), env);
            if expanded.is_empty() {
                continue;
            }
            let key = path_key(&expanded);
            if !seen.insert(key) {
                continue;
            }
            entries.push(expanded);
        }
    }
    entries.join(";")
}

fn expand_vars(entry: &str, env: &EnvMap) -> String {
    let chars: Vec<char> = entry.chars().collect();
    let mut out = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '%'
            && let Some(end_rel) = chars[index + 1..].iter().position(|ch| *ch == '%')
        {
            let name: String = chars[index + 1..index + 1 + end_rel].iter().collect();
            if !name.is_empty() {
                match env.get(&name) {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push('%');
                        out.push_str(&name);
                        out.push('%');
                    }
                }
                index = index + 1 + end_rel + 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn path_key(entry: &str) -> String {
    entry
        .to_lowercase()
        .trim_end_matches(['\\', '/'])
        .to_string()
}
