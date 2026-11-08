//! The `legate` command line.

mod app_drive;
mod args;
mod drive;
mod format;
mod help;
mod run;
mod tui;

pub use app_drive::launch;
pub use args::parse_args;
pub use help::early_exit;
pub use run::run;
pub use tui::open;

#[cfg(test)]
mod cli_tests;
#[cfg(test)]
mod harness;
