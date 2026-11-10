# Project workspaces

Legate keeps an independent live lead session for each project opened during the app session. Switching projects does not stop the other project's reply or running workers. Returning to a project restores its live/persisted conversation. Live processes still require the application to remain running.

Each lead has an immutable project identity and a separate authenticated local tool bridge. Its job submissions, proposal attachments and job lookups stay in that project, even when another project is visible. Provider allowance and model preferences continue to use the existing shared configuration.

The main window remembers each project's panel arrangement, selected worktree, active terminal, split seats and unsent draft. Layout controls can move tools above/below terminals, under projects or into the right inspector. Existing resize handles and split controls remain available. Project requests use generations to reject stale UI results. Pop-out windows do not own or overwrite the main window's layout snapshot; terminal lookup still supports all projects for existing pop-outs.

## Agent setup prompt

Open Settings > Projects, enter the workflow you want, and choose Copy setup prompt. Paste it into an agent working in that repository. Preview setup prompt exposes the full text and is also a clipboard fallback.

The prompt includes the repository path, the requested workflow and connected agents. It asks the agent to inspect existing instructions, verify development/test/build commands, configure repository-scoped guidance and recommend a modular layout and worktree setup commands. It does not invent subscriptions, edit the app database or execute setup by itself.

## Verification

Automated app tests cover background lead replies, separate histories, switching during a running worker, and project terminal filtering while retaining all-project lookups for pop-outs. Pure UI tests cover workspace serialization/key isolation and prompt contents. Source review checked session preparation races, send completion after a canceled folder chooser, and primary-window layout ownership. Final visual interaction checks were not performed through desktop control.
