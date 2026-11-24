//! Copy bundled skills into the data folder and install embedded skill text.

use std::fs;
use std::path::{Path, PathBuf};

use legate_lead::{LEAD_DELEGATE_SKILL, LEAD_LEG_IT_SKILL};

const PLUGINS: [&str; 3] = ["legate-core", "legate-worker", "legate-lead"];
const WORKTREE_SKILL: &str =
    include_str!("../../../resources/plugins/legate-worker/skills/legate-worktree/SKILL.md");

/// Install plugins under `{data}/resources/plugins`.
pub fn install_bundled(data_dir: &Path) {
    let dest = data_dir.join("resources").join("plugins");
    if let Some(source) = find_resources() {
        for name in PLUGINS {
            let from = source.join(name);
            let to = dest.join(name);
            if from.is_dir() {
                let _ = copy_dir(&from, &to);
            }
        }
    }
    for name in PLUGINS {
        let meta = dest.join(name).join(".claude-plugin");
        let file = meta.join("plugin.json");
        if !file.is_file() {
            let _ = fs::create_dir_all(&meta);
            let body = format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"Legate skill plugin.\"\n}}\n"
            );
            let _ = fs::write(&file, body);
        }
    }
    for (plugin, name, text) in [
        ("legate-lead", "legate-delegate", LEAD_DELEGATE_SKILL),
        ("legate-lead", "legate-leg-it", LEAD_LEG_IT_SKILL),
        ("legate-worker", "legate-worktree", WORKTREE_SKILL),
    ] {
        let skill = dest.join(plugin).join("skills").join(name);
        if fs::create_dir_all(&skill).is_ok() {
            let _ = fs::write(skill.join("SKILL.md"), text);
        }
    }
}

fn find_resources() -> Option<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        starts.push(dir.to_path_buf());
    }
    for start in starts {
        let mut dir = start;
        for _ in 0..8 {
            let plugins = dir.join("resources").join("plugins");
            let ready = PLUGINS.iter().all(|name| {
                plugins
                    .join(name)
                    .join(".claude-plugin")
                    .join("plugin.json")
                    .is_file()
            });
            if ready {
                return Some(plugins);
            }
            if !dir.pop() {
                break;
            }
        }
    }
    None
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
