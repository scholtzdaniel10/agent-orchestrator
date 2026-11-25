//! Runs one short command in a pseudo-terminal nobody sees, for CLIs that refuse to run with
//! redirected output (`claude --desktop`).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::error::PtyError;

/// What the command printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnceOutput {
    /// `None` when it did not finish in time and was stopped.
    pub exit_code: Option<u32>,
    pub output: String,
}

/// Runs `command` in `cwd` inside a PTY with the app's own environment, waits up to `timeout`
/// for it to exit, and returns its output. Blocking: call it from a blocking task.
pub fn run_once(
    command: &str,
    args: &[String],
    cwd: &str,
    timeout: Duration,
) -> Result<OnceOutput, PtyError> {
    let mut cmd = CommandBuilder::new(command);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.cwd(cwd);
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| PtyError::Message(err.to_string()))?;
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|err| PtyError::Message(err.to_string()))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|err| PtyError::Message(err.to_string()))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|err| PtyError::Message(err.to_string()))?;
    let (sent, received) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || sent.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut answered = 0;
    let exit_code = loop {
        while let Ok(chunk) = received.try_recv() {
            bytes.extend(chunk);
        }
        // ConPTY asks where the cursor is (ESC[6n) and holds the program until a terminal
        // answers. Answer each ask once.
        let asks = count(&bytes, b"\x1b[6n");
        while answered < asks {
            let _ = writer.write_all(b"\x1b[1;1R");
            let _ = writer.flush();
            answered += 1;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.exit_code()),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                break None;
            }
        }
    };
    let drain_until = Instant::now() + Duration::from_millis(200);
    while Instant::now() < drain_until {
        match received.recv_timeout(Duration::from_millis(20)) {
            Ok(chunk) => bytes.extend(chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(writer);
    drop(pair.master);
    Ok(OnceOutput {
        exit_code,
        output: plain_text(&String::from_utf8_lossy(&bytes)),
    })
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Drops terminal escape sequences and carriage returns, keeping the readable text.
fn plain_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            },
            '\r' => {}
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_escape_sequences() {
        assert_eq!(
            plain_text("\u{1b}[?25l\u{1b}]0;title\u{7}Opening session x\r\n\u{1b}[0m"),
            "Opening session x"
        );
    }

    #[test]
    fn runs_a_command_in_a_pty_and_returns_its_output() {
        let (command, args) = if cfg!(windows) {
            (
                "cmd.exe",
                vec!["/c".to_string(), "echo legate-once".to_string()],
            )
        } else {
            ("sh", vec!["-c".to_string(), "echo legate-once".to_string()])
        };
        let cwd = std::env::temp_dir().to_string_lossy().into_owned();
        let out = run_once(command, &args, &cwd, Duration::from_secs(20)).unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(out.output.contains("legate-once"), "{:?}", out.output);
    }
}
