//! Bundled skills and the person's skill folders.
//!
//! [`crate::AgentPlugins`] is `legate_providers::AgentPlugins`. This module finds the folders.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use legate_core::{BuiltInSkill, FoundSkill, GivenTo, SkillFolderReport};

use crate::AgentPlugins;
use crate::error::LeadError;

const BUNDLED: [&str; 3] = ["legate-core", "legate-worker", "legate-lead"];
const USER_PLUGIN: &str = "legate-your-skills";

/// What [`build_user_plugin`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPluginBuild {
    pub plugin: Option<String>,
    pub reports: Vec<SkillFolderReport>,
}

/// Legate's own plugins, found by walking up from `from_dir` to `resources/plugins`.
/// Packaged, the code sits in `app.asar` and the skills are unpacked beside it.
pub fn bundled_plugins(from_dir: &Path) -> AgentPlugins {
    let mut dir = normalize(from_dir);
    for _ in 0..6 {
        let root = unpacked(&dir).join("resources").join("plugins");
        let ready = BUNDLED.iter().all(|name| {
            root.join(name)
                .join(".claude-plugin")
                .join("plugin.json")
                .exists()
        });
        if ready {
            return AgentPlugins {
                lead: vec![
                    path_string(&root.join("legate-core")),
                    path_string(&root.join("legate-lead")),
                ],
                worker: vec![
                    path_string(&root.join("legate-core")),
                    path_string(&root.join("legate-worker")),
                ],
            };
        }
        if !dir.pop() {
            break;
        }
    }
    AgentPlugins::default()
}

fn unpacked(path: &Path) -> PathBuf {
    let sep = std::path::MAIN_SEPARATOR;
    let text = path_string(path);
    let parts: Vec<String> = text
        .split(sep)
        .map(|part| {
            if part == "app.asar" {
                "app.asar.unpacked".to_string()
            } else {
                part.to_string()
            }
        })
        .collect();
    PathBuf::from(parts.join(&sep.to_string()))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(crate) fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn given_to(name: &str) -> Option<GivenTo> {
    match name {
        "legate-core" => Some(GivenTo::EveryAgent),
        "legate-worker" => Some(GivenTo::Workers),
        "legate-lead" => Some(GivenTo::Lead),
        _ => None,
    }
}

/// Legate's own skills, each with the agents it is given to.
pub fn bundled_skills(plugins: &AgentPlugins) -> Vec<BuiltInSkill> {
    let mut dirs = Vec::new();
    for dir in plugins.worker.iter().chain(plugins.lead.iter()) {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        let base = Path::new(&dir)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let Some(given) = given_to(base) else {
            continue;
        };
        for skill in skills_in(&Path::new(&dir).join("skills")).skills {
            out.push(BuiltInSkill {
                name: skill.name,
                given_to: given,
            });
        }
    }
    out
}

/// The plugin built from the person's skills, when there is one. Never rebuilds it.
pub fn existing_user_plugin(data_dir: &Path) -> Option<String> {
    let target = data_dir.join(USER_PLUGIN);
    if target.join(".claude-plugin").join("plugin.json").exists() {
        Some(path_string(&target))
    } else {
        None
    }
}

pub fn with_plugins(base: &AgentPlugins, extra: &[String]) -> AgentPlugins {
    let mut lead = base.lead.clone();
    let mut worker = base.worker.clone();
    lead.extend(extra.iter().cloned());
    worker.extend(extra.iter().cloned());
    AgentPlugins { lead, worker }
}

fn is_dir(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
}

/// The skills in a folder: the folder itself when it holds a `SKILL.md`, otherwise each
/// subfolder that does.
pub fn skills_in(folder: &Path) -> SkillFolderReport {
    let folder_string = path_string(folder);
    if !is_dir(folder) {
        return SkillFolderReport {
            folder: folder_string,
            skills: Vec::new(),
            problem: Some("Folder not found.".to_string()),
        };
    }
    if folder.join("SKILL.md").exists() {
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        return SkillFolderReport {
            folder: folder_string.clone(),
            skills: vec![FoundSkill {
                name,
                folder: folder_string,
            }],
            problem: None,
        };
    }
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(_) => {
            return SkillFolderReport {
                folder: folder_string,
                skills: Vec::new(),
                problem: Some("Folder cannot be read.".to_string()),
            };
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return SkillFolderReport {
                folder: folder_string,
                skills: Vec::new(),
                problem: Some("Folder cannot be read.".to_string()),
            };
        };
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    let skills = names
        .into_iter()
        .filter_map(|name| {
            let path = folder.join(&name);
            if path.join("SKILL.md").exists() {
                Some(FoundSkill {
                    name,
                    folder: path_string(&path),
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let problem = if skills.is_empty() {
        Some("No SKILL.md here or in its subfolders.".to_string())
    } else {
        None
    };
    SkillFolderReport {
        folder: folder_string,
        skills,
        problem,
    }
}

/// Copies the person's skills into one plugin folder under `data_dir`.
/// Copies, never links, so clearing the old copy cannot touch the originals.
pub fn build_user_plugin(
    folders: &[PathBuf],
    data_dir: &Path,
) -> Result<UserPluginBuild, LeadError> {
    let target = data_dir.join(USER_PLUGIN);
    if target.exists() {
        fs::remove_dir_all(&target).map_err(|err| LeadError::message(err.to_string()))?;
    }
    let reports: Vec<SkillFolderReport> = folders.iter().map(|folder| skills_in(folder)).collect();
    let mut used = HashSet::new();
    let mut chosen: Vec<&FoundSkill> = Vec::new();
    for report in &reports {
        for skill in &report.skills {
            if !used.insert(skill.name.to_lowercase()) {
                continue;
            }
            chosen.push(skill);
        }
    }
    if chosen.is_empty() {
        return Ok(UserPluginBuild {
            plugin: None,
            reports,
        });
    }
    let meta = target.join(".claude-plugin");
    fs::create_dir_all(&meta).map_err(|err| LeadError::message(err.to_string()))?;
    let body = "{\n  \"name\": \"legate-your-skills\",\n  \"version\": \"1.0.0\",\n  \"description\": \"Skills you added in Legate.\"\n}\n";
    fs::write(meta.join("plugin.json"), body).map_err(|err| LeadError::message(err.to_string()))?;
    for skill in chosen {
        copy_dir(
            Path::new(&skill.folder),
            &target.join("skills").join(&skill.name),
        )?;
    }
    Ok(UserPluginBuild {
        plugin: Some(path_string(&target)),
        reports,
    })
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), LeadError> {
    fs::create_dir_all(to).map_err(|err| LeadError::message(err.to_string()))?;
    let entries = fs::read_dir(from).map_err(|err| LeadError::message(err.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|err| LeadError::message(err.to_string()))?;
        let dest = to.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| LeadError::message(err.to_string()))?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest).map_err(|err| LeadError::message(err.to_string()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use legate_core::GivenTo;
    use legate_providers::plugin_args;

    use super::{
        AgentPlugins, build_user_plugin, bundled_plugins, bundled_skills, existing_user_plugin,
        skills_in, with_plugins,
    };

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("legate-plugins-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn skill(folder: &Path, text: &str) {
        fs::create_dir_all(folder).unwrap();
        let name = folder.file_name().unwrap().to_string_lossy();
        fs::write(
            folder.join("SKILL.md"),
            format!("---\nname: {name}\n---\n{text}\n"),
        )
        .unwrap();
    }

    fn base_name(path: &str) -> String {
        Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn finds_legates_plugins_and_splits_them_by_role() {
        let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let found = bundled_plugins(&here);
        assert_eq!(
            found
                .lead
                .iter()
                .map(|dir| base_name(dir))
                .collect::<Vec<_>>(),
            vec!["legate-core", "legate-lead"]
        );
        assert_eq!(
            found
                .worker
                .iter()
                .map(|dir| base_name(dir))
                .collect::<Vec<_>>(),
            vec!["legate-core", "legate-worker"]
        );
        let skills = bundled_skills(&found);
        assert_eq!(skills[0].name, "legate-review");
        assert_eq!(skills[0].given_to, GivenTo::EveryAgent);
        assert_eq!(skills[1].name, "legate-cli");
        assert_eq!(skills[1].given_to, GivenTo::Workers);
        assert_eq!(skills[2].name, "legate-worktree");
        assert_eq!(skills[2].given_to, GivenTo::Workers);
        assert_eq!(skills[3].name, "legate-delegate");
        assert_eq!(skills[3].given_to, GivenTo::Lead);
        assert_eq!(skills.len(), 4);
    }

    #[test]
    fn every_bundled_skill_has_a_name_and_a_description() {
        let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let found = bundled_plugins(&here);
        let mut dirs = found.lead.clone();
        for dir in found.worker {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        for dir in dirs {
            for entry in skills_in(&Path::new(&dir).join("skills")).skills {
                let text = fs::read_to_string(Path::new(&entry.folder).join("SKILL.md")).unwrap();
                assert!(
                    skill_header(&text, &entry.name),
                    "{} is missing a name and description",
                    entry.folder
                );
            }
        }
    }

    fn skill_header(text: &str, name: &str) -> bool {
        let Some(rest) = text
            .strip_prefix("---\n")
            .or_else(|| text.strip_prefix("---\r\n"))
        else {
            return false;
        };
        let line = format!("name: {name}");
        let Some(rest) = rest
            .strip_prefix(&format!("{line}\n"))
            .or_else(|| rest.strip_prefix(&format!("{line}\r\n")))
        else {
            return false;
        };
        rest.starts_with("description: ")
    }

    #[test]
    fn packaged_code_is_in_app_asar_and_plugins_are_unpacked() {
        let root = TempDir::new();
        let plugins = root
            .path
            .join("resources")
            .join("app.asar.unpacked")
            .join("resources")
            .join("plugins");
        for name in ["legate-core", "legate-worker", "legate-lead"] {
            let dir = plugins.join(name).join(".claude-plugin");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("plugin.json"), "{}").unwrap();
        }
        let from = root
            .path
            .join("resources")
            .join("app.asar")
            .join("out")
            .join("main");
        let found = bundled_plugins(&from);
        assert_eq!(
            found.worker,
            vec![
                path(&plugins.join("legate-core")),
                path(&plugins.join("legate-worker"))
            ]
        );
    }

    fn path(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn without_the_plugins_folder_there_are_none() {
        let root = TempDir::new();
        assert_eq!(bundled_plugins(&root.path), AgentPlugins::default());
    }

    #[test]
    fn a_picked_folder_is_one_skill_or_a_folder_of_skills() {
        let root = TempDir::new();
        skill(&root.path.join("one"), "body");
        skill(&root.path.join("many").join("b"), "body");
        skill(&root.path.join("many").join("a"), "body");
        fs::create_dir_all(root.path.join("many").join("not-a-skill")).unwrap();
        fs::create_dir_all(root.path.join("empty")).unwrap();

        assert_eq!(
            skills_in(&root.path.join("one"))
                .skills
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["one"]
        );
        assert_eq!(
            skills_in(&root.path.join("many"))
                .skills
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            skills_in(&root.path.join("empty")).problem.as_deref(),
            Some("No SKILL.md here or in its subfolders.")
        );
        assert_eq!(
            skills_in(&root.path.join("gone")).problem.as_deref(),
            Some("Folder not found.")
        );
    }

    #[test]
    fn the_persons_skills_are_copied_first_name_wins_originals_untouched() {
        let root = TempDir::new();
        let data = root.path.join("data");
        skill(&root.path.join("mine").join("tone"), "first");
        skill(&root.path.join("other").join("tone"), "second");
        skill(&root.path.join("other").join("tests"), "body");

        let built =
            build_user_plugin(&[root.path.join("mine"), root.path.join("other")], &data).unwrap();
        let plugin = built.plugin.clone().unwrap();
        assert_eq!(plugin, path(&data.join("legate-your-skills")));
        assert_eq!(
            existing_user_plugin(&data).as_deref(),
            Some(plugin.as_str())
        );
        let tone = fs::read_to_string(
            Path::new(&plugin)
                .join("skills")
                .join("tone")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(tone.contains("first"));
        assert!(
            Path::new(&plugin)
                .join("skills")
                .join("tests")
                .join("SKILL.md")
                .exists()
        );

        let again = build_user_plugin(&[root.path.join("other")], &data).unwrap();
        let tone = fs::read_to_string(
            Path::new(&plugin)
                .join("skills")
                .join("tone")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(tone.contains("second"));
        assert_eq!(again.reports[0].skills.len(), 2);
        assert!(
            root.path
                .join("mine")
                .join("tone")
                .join("SKILL.md")
                .exists()
        );

        assert!(build_user_plugin(&[], &data).unwrap().plugin.is_none());
        assert!(existing_user_plugin(&data).is_none());
        assert!(
            root.path
                .join("other")
                .join("tests")
                .join("SKILL.md")
                .exists()
        );
    }

    #[test]
    fn extra_plugins_go_to_every_role() {
        let both = with_plugins(
            &AgentPlugins {
                lead: vec!["L".to_string()],
                worker: vec!["W".to_string()],
            },
            &["mine".to_string()],
        );
        assert_eq!(
            both,
            AgentPlugins {
                lead: vec!["L".to_string(), "mine".to_string()],
                worker: vec!["W".to_string(), "mine".to_string()],
            }
        );
        assert_eq!(
            plugin_args(&both.worker),
            vec![
                "--plugin-dir".to_string(),
                "W".to_string(),
                "--plugin-dir".to_string(),
                "mine".to_string()
            ]
        );
    }
}
