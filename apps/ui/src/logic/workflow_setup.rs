//! A portable handoff prompt; generation never changes project files itself.
pub fn setup_prompt(project: &str, workflow: &str, agents: &[&str]) -> String {
    let project = serde_json::to_string(project).unwrap_or_default();
    let workflow = if workflow.trim().is_empty() {
        "Recommend a workflow from the repository's existing conventions."
    } else {
        workflow.trim()
    };
    let agents = if agents.is_empty() {
        "Not detected; ask which agents the user has.".to_string()
    } else {
        agents.join(", ")
    };
    format!(
        r#"Configure this repository for my workflow in Legate.

Project path (JSON string): {project}
My workflow: {workflow}
Connected agents: {agents}

1. Read existing AGENTS.md guidance, manifests, scripts and project configuration before making changes. Preserve existing instructions and adapt to the repository's conventions.
2. Identify the real install, development, lint, test and build commands. Verify commands from repository evidence; mark anything you cannot verify.
3. Propose practical agent roles for this workflow: planning, implementation and review as needed. Use my connected agents; ask before requiring another subscription or paid service.
4. Create or update project-scoped agent instructions with the commands, acceptance checks, coding conventions and handoff expectations. Use the repository's existing agent/skill format when present.
5. Recommend worktree setup commands and a modular Legate workspace (panels, splits and agent tabs). Keep the user's preferred layout. Apply Legate settings only through supported controls/tools; otherwise provide the exact values to enter in Settings > Projects.

Finish with files changed, verified commands, agent roles and any remaining manual setup. Keep this setup specific to this repository and workflow."#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_carries_the_users_project_workflow_and_available_agents() {
        let text = setup_prompt(
            "C:/my repo",
            "Rust CLI with a separate reviewer",
            &["codex", "claude"],
        );
        assert!(text.contains("\"C:/my repo\""));
        assert!(text.contains("Rust CLI with a separate reviewer"));
        assert!(text.contains("Connected agents: codex, claude"));
        assert!(text.contains("Keep the user's preferred layout"));
    }
    #[test]
    fn absent_choices_are_explicit_instead_of_inventing_a_provider() {
        let text = setup_prompt("/project", "", &[]);
        assert!(text.contains("Not detected"));
        assert!(text.contains("Recommend a workflow"));
    }
}
