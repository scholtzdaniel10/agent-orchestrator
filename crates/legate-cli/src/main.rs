//! `legate`. `--help` and `--version` return before any runtime or database.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use legate_cli::early_exit;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if let Some(early) = early_exit(&argv, env!("CARGO_PKG_VERSION"), tty) {
        let mut stream: Box<dyn Write> = if early.stderr {
            Box::new(std::io::stderr())
        } else {
            Box::new(std::io::stdout())
        };
        let _ = stream.write_all(early.text.as_bytes());
        let _ = stream.flush();
        return ExitCode::from(early.code as u8);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    ExitCode::from(runtime.block_on(legate_cli::launch(&argv)) as u8)
}
