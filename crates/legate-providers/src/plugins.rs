/// Plugin folders (`--plugin-dir`) for each kind of agent Legate starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentPlugins {
    /// The lead chat, which hands out jobs.
    pub lead: Vec<String>,
    /// Jobs and terminal agents, which work in a worktree.
    pub worker: Vec<String>,
}

/// Arguments that load each plugin folder, for CLIs that take `--plugin-dir`.
pub fn plugin_args(dirs: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(dirs.len() * 2);
    for dir in dirs {
        args.push("--plugin-dir".to_string());
        args.push(dir.clone());
    }
    args
}
