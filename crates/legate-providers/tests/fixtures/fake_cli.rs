//! Stand-in for the Claude and Cursor CLIs. The first argument selects a behavior;
//! anything else is treated as a script name and the remaining arguments are
//! written to stderr as JSON, matching `process.argv.slice(2)`.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{ExitCode, Stdio};
use std::time::Duration;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(mode) = args.next() else {
        return dump(&[]);
    };
    let rest: Vec<String> = args.collect();
    match mode.as_str() {
        "replay" => replay(rest.first().map(String::as_str).unwrap_or("")),
        "hang" => hang(),
        "version" => version(
            rest.first().map(String::as_str).unwrap_or(""),
            rest.get(1).map(String::as_str).unwrap_or(""),
        ),
        "models-ok" => models_ok(rest.first().map(String::as_str).unwrap_or("")),
        "models-fail" => models_fail(rest.first().map(String::as_str).unwrap_or("")),
        "models-empty" => models_empty(rest.first().map(String::as_str).unwrap_or("")),
        "models-grok" => {
            println!("grok-4.7-high - Grok 4.7");
            println!("composer-2.5 - Composer");
            ExitCode::SUCCESS
        }
        "env" => report_env(),
        "edit-report" => permission_report(&rest, false),
        "full-report" => permission_report(&rest, true),
        "tree" => tree(rest.first().map(String::as_str).unwrap_or("")),
        "sleep" => sleep(),
        "sleep-pid" => sleep_pid(rest.first().map(String::as_str).unwrap_or("")),
        _ => dump(&rest),
    }
}

fn dump(args: &[String]) -> ExitCode {
    match serde_json::to_string(args) {
        Ok(json) => {
            let _ = std::io::stderr().write_all(json.as_bytes());
            ExitCode::SUCCESS
        }
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "{err}");
            ExitCode::from(1)
        }
    }
}

fn replay(path: &str) -> ExitCode {
    let mut stdin = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut stdin);
    let _ = std::io::stderr().write_all(&stdin);
    let _ = std::io::stderr().flush();
    match std::fs::read(path) {
        Ok(bytes) => {
            let _ = std::io::stdout().write_all(&bytes);
            let _ = std::io::stdout().flush();
            ExitCode::SUCCESS
        }
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "{err}");
            ExitCode::from(1)
        }
    }
}

fn hang() -> ExitCode {
    let mut out = std::io::stdout();
    let _ = writeln!(
        out,
        r#"{{"type":"system","subtype":"init","session_id":"kill-test","model":"m"}}"#
    );
    let _ = out.flush();
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn version(counter: &str, text: &str) -> ExitCode {
    let _ = bump(Path::new(counter));
    println!("{text}");
    ExitCode::SUCCESS
}

fn models_ok(counter: &str) -> ExitCode {
    let n = bump(Path::new(counter));
    if n > 0 {
        std::process::exit(1);
    }
    println!("Available models\n");
    println!("auto - Auto");
    println!("composer-2.5 - Composer 2.5");
    ExitCode::SUCCESS
}

fn models_fail(counter: &str) -> ExitCode {
    let n = bump(Path::new(counter));
    if n == 0 {
        std::process::exit(2);
    }
    println!("auto - Auto");
    ExitCode::SUCCESS
}

fn models_empty(counter: &str) -> ExitCode {
    let n = bump(Path::new(counter));
    if n == 0 {
        println!("Available models");
        println!();
        return ExitCode::SUCCESS;
    }
    println!("haiku - Haiku");
    ExitCode::SUCCESS
}

fn report_env() -> ExitCode {
    let shell = std::env::var("SHELL").ok();
    let msystem = std::env::var("MSYSTEM").ok();
    let path = if std::env::var("PATH").is_ok() {
        "string"
    } else {
        "undefined"
    };
    let report = serde_json::json!({ "shell": shell, "msystem": msystem, "path": path });
    match serde_json::to_string(&report) {
        Ok(json) => {
            let _ = std::io::stderr().write_all(json.as_bytes());
            ExitCode::SUCCESS
        }
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "{err}");
            ExitCode::from(1)
        }
    }
}

fn permission_report(argv: &[String], has_cli: bool) -> ExitCode {
    // Stay alive long enough for the caller to see the permission file before cleanup.
    std::thread::sleep(Duration::from_millis(600));
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let cli = cwd.join(".cursor").join("cli.json");
    let body = std::fs::read_to_string(&cli).unwrap_or_default();
    let report = if has_cli {
        serde_json::json!({ "argv": argv, "body": body, "hasCli": cli.exists() })
    } else {
        serde_json::json!({ "argv": argv, "body": body })
    };
    match serde_json::to_string(&report) {
        Ok(json) => {
            let _ = std::io::stderr().write_all(json.as_bytes());
            ExitCode::SUCCESS
        }
        Err(err) => {
            let _ = writeln!(std::io::stderr(), "{err}");
            ExitCode::from(1)
        }
    }
}

fn tree(dir: &str) -> ExitCode {
    std::thread::sleep(Duration::from_millis(300));
    let dir = Path::new(dir);
    let grand_path = dir.join("grand.pid");
    if let Ok(exe) = std::env::current_exe()
        && let Ok(grand) = std::process::Command::new(exe)
            .arg("sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    {
        // Record the pid here. The grandchild must not rewrite this file, or a
        // reader can observe the truncated file mid-write.
        let _ = std::fs::write(&grand_path, grand.id().to_string());
        // Detach. Dropping `Child` does not kill the process.
        drop(grand);
    }
    let _ = std::fs::write(dir.join("child.pid"), std::process::id().to_string());
    let mut out = std::io::stdout();
    let _ = writeln!(
        out,
        r#"{{"type":"system","subtype":"init","session_id":"tree","model":"m"}}"#
    );
    let _ = out.flush();
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn sleep() -> ExitCode {
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn sleep_pid(path: &str) -> ExitCode {
    let _ = std::fs::write(path, std::process::id().to_string());
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn bump(path: &Path) -> u64 {
    let n = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let _ = std::fs::write(path, (n + 1).to_string());
    n
}
