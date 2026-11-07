//! The lead's planning read, and the check command a goal runs in its worktree.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use legate_core::{AgentEvent, JobAccess, ProviderId, RunOptions};
use legate_providers::ProcessExit;
use tokio::io::AsyncReadExt;

use crate::error::LeadError;
use crate::text::{fresh_id, split_lines};

const CHECK_LIMIT: Duration = Duration::from_secs(20 * 60);
const OUTPUT_MAX: usize = 200_000;
const OUTPUT_KEEP: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanWorker {
    pub id: ProviderId,
    pub available: bool,
}

#[async_trait]
pub trait PlanSession: Send {
    async fn recv(&mut self) -> Option<AgentEvent>;
    async fn exit_status(&mut self) -> ProcessExit;
}

pub trait PlanAdapter: Send + Sync {
    fn run(
        &self,
        job_id: &str,
        prompt: &str,
        cwd: &str,
        opts: &RunOptions,
    ) -> Result<Box<dyn PlanSession>, String>;
}

pub struct PlanGoalDeps {
    pub folder: String,
    pub prompt: String,
    pub workers: Vec<PlanWorker>,
    pub preferred: Option<ProviderId>,
    pub adapter: Arc<dyn Fn(ProviderId) -> Arc<dyn PlanAdapter>>,
    pub model_for: Arc<dyn Fn(ProviderId) -> Option<String> + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutput {
    pub code: Option<i32>,
    pub output: String,
}

/// Reads the goal folder and returns the plan text.
pub async fn plan_goal(deps: PlanGoalDeps) -> Result<String, LeadError> {
    let provider = deps
        .workers
        .iter()
        .filter(|worker| worker.available)
        .find(|worker| Some(worker.id) == deps.preferred)
        .or_else(|| deps.workers.iter().find(|worker| worker.available))
        .map(|worker| worker.id)
        .ok_or_else(|| LeadError::message("No plan is signed in and ready."))?;
    let adapter = (deps.adapter)(provider);
    let mut session = adapter
        .run(
            &fresh_id(),
            &deps.prompt,
            &deps.folder,
            &RunOptions {
                access: Some(JobAccess::Read),
                model: (deps.model_for)(provider),
                ..RunOptions::default()
            },
        )
        .map_err(LeadError::message)?;
    let mut result = String::new();
    let mut said = String::new();
    while let Some(event) = session.recv().await {
        match event {
            AgentEvent::Result { text, .. } => result = text,
            AgentEvent::Text { text } => said.push_str(&text),
            _ => {}
        }
    }
    let exit = session.exit_status().await;
    let text = if result.trim().is_empty() {
        said
    } else {
        result
    };
    if text.trim().is_empty() {
        let trimmed = exit.stderr.trim();
        let line = split_lines(trimmed)
            .into_iter()
            .next_back()
            .unwrap_or("no answer");
        return Err(LeadError::message(line));
    }
    Ok(text)
}

/// Runs a check the person typed. `shell` is on because it is a command line, not an argv list.
pub async fn run_goal_check(command: &str, folder: &str) -> CheckOutput {
    let mut cmd = shell(command);
    cmd.current_dir(folder)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return CheckOutput {
                code: None,
                output: format!("\n{err}"),
            };
        }
    };
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let out = std::sync::Arc::clone(&buf);
    let err = std::sync::Arc::clone(&buf);
    let pumped = async {
        let read_out = async {
            if let Some(reader) = stdout.as_mut() {
                drain(reader, &out).await;
            }
        };
        let read_err = async {
            if let Some(reader) = stderr.as_mut() {
                drain(reader, &err).await;
            }
        };
        let (_, _, status) = tokio::join!(read_out, read_err, child.wait());
        status
    };
    match tokio::time::timeout(CHECK_LIMIT, pumped).await {
        Ok(Ok(status)) => CheckOutput {
            code: status.code(),
            output: take_buf(&buf),
        },
        Ok(Err(error)) => CheckOutput {
            code: None,
            output: format!("{}\n{error}", take_buf(&buf)),
        },
        Err(_elapsed) => {
            let mut output = take_buf(&buf);
            output.push_str("\n… stopped after 20 minutes");
            CheckOutput { code: None, output }
        }
    }
}

fn shell(command: &str) -> tokio::process::Command {
    // Node wraps the command in quotes and passes it verbatim. `cmd /s` then strips the
    // outer pair, which is what keeps `node -e "..."` intact.
    #[cfg(windows)]
    {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut cmd = tokio::process::Command::new(comspec);
        cmd.raw_arg("/d");
        cmd.raw_arg("/s");
        cmd.raw_arg("/c");
        cmd.raw_arg(format!("\"{command}\""));
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

async fn drain(
    reader: &mut (impl AsyncReadExt + Unpin),
    buf: &std::sync::Arc<std::sync::Mutex<String>>,
) {
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => append(buf, &chunk[..n]),
        }
    }
}

fn append(buf: &std::sync::Arc<std::sync::Mutex<String>>, bytes: &[u8]) {
    let mut out = buf.lock().unwrap_or_else(|err| err.into_inner());
    out.push_str(&String::from_utf8_lossy(bytes));
    if out.len() > OUTPUT_MAX {
        let start = out.len() - OUTPUT_KEEP;
        let start = (start..=out.len())
            .find(|index| out.is_char_boundary(*index))
            .unwrap_or(out.len());
        *out = out[start..].to_string();
    }
}

fn take_buf(buf: &std::sync::Arc<std::sync::Mutex<String>>) -> String {
    std::mem::take(&mut *buf.lock().unwrap_or_else(|err| err.into_inner()))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use legate_core::{AgentEvent, JobAccess, ProviderId, RunOptions};
    use legate_providers::ProcessExit;

    use super::{PlanAdapter, PlanGoalDeps, PlanSession, PlanWorker, plan_goal, run_goal_check};

    struct ScriptSession {
        events: Vec<AgentEvent>,
        index: usize,
        exit: ProcessExit,
    }

    #[async_trait]
    impl PlanSession for ScriptSession {
        async fn recv(&mut self) -> Option<AgentEvent> {
            if self.index >= self.events.len() {
                return None;
            }
            let event = self.events[self.index].clone();
            self.index += 1;
            Some(event)
        }

        async fn exit_status(&mut self) -> ProcessExit {
            self.exit.clone()
        }
    }

    struct Seen {
        rows: Mutex<Vec<SeenCall>>,
    }

    struct SeenCall {
        id: ProviderId,
        cwd: String,
        prompt: String,
        model: Option<String>,
        access: Option<JobAccess>,
    }

    fn adapter(
        id: ProviderId,
        events: Vec<AgentEvent>,
        exit: ProcessExit,
        seen: Arc<Seen>,
    ) -> Arc<dyn PlanAdapter> {
        struct Ad {
            id: ProviderId,
            events: Vec<AgentEvent>,
            exit: ProcessExit,
            seen: Arc<Seen>,
        }
        impl PlanAdapter for Ad {
            fn run(
                &self,
                _job_id: &str,
                prompt: &str,
                cwd: &str,
                opts: &RunOptions,
            ) -> Result<Box<dyn PlanSession>, String> {
                self.seen.rows.lock().unwrap().push(SeenCall {
                    id: self.id,
                    cwd: cwd.to_string(),
                    prompt: prompt.to_string(),
                    model: opts.model.clone(),
                    access: opts.access,
                });
                Ok(Box::new(ScriptSession {
                    events: self.events.clone(),
                    index: 0,
                    exit: self.exit.clone(),
                }))
            }
        }
        Arc::new(Ad {
            id,
            events,
            exit,
            seen,
        })
    }

    fn exit_ok() -> ProcessExit {
        ProcessExit {
            code: Some(0),
            stderr: String::new(),
            unreadable: false,
        }
    }

    #[tokio::test]
    async fn plan_goal_uses_the_preferred_plan_then_the_first_ready_one() {
        let seen = Arc::new(Seen {
            rows: Mutex::new(Vec::new()),
        });
        let seen_for = Arc::clone(&seen);
        let text = plan_goal(PlanGoalDeps {
            folder: "/wt/goal".into(),
            prompt: "plan the round".into(),
            workers: vec![
                PlanWorker {
                    id: ProviderId::Claude,
                    available: true,
                },
                PlanWorker {
                    id: ProviderId::Cursor,
                    available: true,
                },
            ],
            preferred: Some(ProviderId::Cursor),
            adapter: Arc::new(move |id| {
                adapter(
                    id,
                    vec![
                        AgentEvent::Text {
                            text: "draft ".into(),
                        },
                        AgentEvent::Result {
                            ok: true,
                            text: format!("plan from {}", id.as_str()),
                            session_id: None,
                            duration_ms: None,
                            cost_usd: None,
                            tokens: None,
                        },
                    ],
                    exit_ok(),
                    Arc::clone(&seen_for),
                )
            }),
            model_for: Arc::new(|id| {
                if id == ProviderId::Cursor {
                    Some("composer".into())
                } else {
                    None
                }
            }),
        })
        .await
        .unwrap();
        assert_eq!(text, "plan from cursor");
        {
            let rows = seen.rows.lock().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, ProviderId::Cursor);
            assert_eq!(rows[0].cwd, "/wt/goal");
            assert_eq!(rows[0].prompt, "plan the round");
            assert_eq!(rows[0].model.as_deref(), Some("composer"));
            assert_eq!(rows[0].access, Some(JobAccess::Read));
        }
        seen.rows.lock().unwrap().clear();
        let seen_for = Arc::clone(&seen);
        let fallback = plan_goal(PlanGoalDeps {
            folder: "/wt/goal".into(),
            prompt: "again".into(),
            workers: vec![
                PlanWorker {
                    id: ProviderId::Claude,
                    available: false,
                },
                PlanWorker {
                    id: ProviderId::Cursor,
                    available: true,
                },
            ],
            preferred: Some(ProviderId::Claude),
            adapter: Arc::new(move |id| {
                adapter(
                    id,
                    vec![AgentEvent::Result {
                        ok: true,
                        text: format!("plan from {}", id.as_str()),
                        session_id: None,
                        duration_ms: None,
                        cost_usd: None,
                        tokens: None,
                    }],
                    exit_ok(),
                    Arc::clone(&seen_for),
                )
            }),
            model_for: Arc::new(|_| None),
        })
        .await
        .unwrap();
        assert_eq!(fallback, "plan from cursor");
        let rows = seen.rows.lock().unwrap();
        assert_eq!(rows[0].id, ProviderId::Cursor);
        assert!(rows[0].model.is_none());
    }

    #[tokio::test]
    async fn plan_goal_keeps_spoken_text_and_names_a_failed_run() {
        let seen = Arc::new(Seen {
            rows: Mutex::new(Vec::new()),
        });
        let spoken = Arc::clone(&seen);
        let text = plan_goal(PlanGoalDeps {
            folder: "/wt".into(),
            prompt: "plan".into(),
            workers: vec![PlanWorker {
                id: ProviderId::Claude,
                available: true,
            }],
            preferred: None,
            adapter: Arc::new(move |id| {
                adapter(
                    id,
                    vec![
                        AgentEvent::Text {
                            text: "hello".into(),
                        },
                        AgentEvent::Result {
                            ok: true,
                            text: "   ".into(),
                            session_id: None,
                            duration_ms: None,
                            cost_usd: None,
                            tokens: None,
                        },
                    ],
                    exit_ok(),
                    Arc::clone(&spoken),
                )
            }),
            model_for: Arc::new(|_| None),
        })
        .await
        .unwrap();
        assert_eq!(text, "hello");

        let silent = Arc::clone(&seen);
        let err = plan_goal(PlanGoalDeps {
            folder: "/wt".into(),
            prompt: "plan".into(),
            workers: vec![PlanWorker {
                id: ProviderId::Claude,
                available: true,
            }],
            preferred: Some(ProviderId::Claude),
            adapter: Arc::new(move |id| {
                adapter(
                    id,
                    Vec::new(),
                    ProcessExit {
                        code: Some(1),
                        stderr: "noise\nout of credits".into(),
                        unreadable: false,
                    },
                    Arc::clone(&silent),
                )
            }),
            model_for: Arc::new(|_| None),
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("out of credits"));

        let none = plan_goal(PlanGoalDeps {
            folder: "/wt".into(),
            prompt: "plan".into(),
            workers: vec![PlanWorker {
                id: ProviderId::Claude,
                available: false,
            }],
            preferred: Some(ProviderId::Claude),
            adapter: Arc::new(|_| {
                adapter(
                    ProviderId::Claude,
                    Vec::new(),
                    exit_ok(),
                    Arc::new(Seen {
                        rows: Mutex::new(Vec::new()),
                    }),
                )
            }),
            model_for: Arc::new(|_| None),
        })
        .await
        .unwrap_err();
        assert!(none.to_string().contains("No plan is signed in and ready."));
    }

    #[tokio::test]
    async fn run_goal_check_returns_the_exit_code_of_a_real_command() {
        let result = run_goal_check(
            "node -e \"process.exit(3)\"",
            &std::env::current_dir().unwrap().to_string_lossy(),
        )
        .await;
        assert_eq!(result.code, Some(3));
    }

    #[tokio::test]
    async fn run_goal_check_keeps_the_end_of_a_long_command_and_both_streams() {
        let marked = run_goal_check(
            "node -e \"process.stdout.write('A'.repeat(250000)+'END')\"",
            &std::env::current_dir().unwrap().to_string_lossy(),
        )
        .await;
        assert_eq!(marked.code, Some(0));
        assert!(marked.output.len() <= 200_000);
        assert!(marked.output.len() < 250_000);
        assert!(marked.output.ends_with("END"));

        let err = run_goal_check(
            "node -e \"process.stderr.write('bye'); process.exit(3)\"",
            &std::env::current_dir().unwrap().to_string_lossy(),
        )
        .await;
        assert_eq!(err.code, Some(3));
        assert!(err.output.contains("bye"));
    }
}
