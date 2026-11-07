//! Copy bundled skills into the data folder, then overwrite the delegate skill
//! with the text embedded in `legate-lead`.

use std::fs;
use std::path::{Path, PathBuf};

use legate_lead::LEAD_DELEGATE_SKILL;

const PLUGINS: [&str; 3] = ["legate-core", "legate-worker", "legate-lead"];

/// `{data}/resources/plugins`, with `legate-delegate/SKILL.md` replaced.
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
    let skill = dest
        .join("legate-lead")
        .join("skills")
        .join("legate-delegate");
    if fs::create_dir_all(&skill).is_ok() {
        let _ = fs::write(skill.join("SKILL.md"), LEAD_DELEGATE_SKILL);
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
