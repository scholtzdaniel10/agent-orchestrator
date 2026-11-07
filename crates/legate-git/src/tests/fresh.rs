use std::future::ready;

use legate_core::Platform;

use crate::cmd::CmdOut;
use crate::fresh::{EnvMap, merged_path, refresh_path, registry_path};
use crate::test_support::Spy;

fn user_output() -> String {
    [
        "",
        r"HKEY_CURRENT_USER\Environment",
        r"    Path    REG_EXPAND_SZ    %USERPROFILE%\AppData\Local\Microsoft\WinGet\Links;C:\Tools\",
        "",
    ]
    .join("\r\n")
}

#[test]
fn reads_path_from_reg_query_output() {
    assert_eq!(
        registry_path(&user_output()),
        r"%USERPROFILE%\AppData\Local\Microsoft\WinGet\Links;C:\Tools\"
    );
    assert_eq!(
        registry_path(r"    Path    REG_SZ    C:\A;C:\B"),
        r"C:\A;C:\B"
    );
    assert_eq!(
        registry_path("ERROR: The system was unable to find the specified registry key"),
        ""
    );
}

#[test]
fn adds_new_registry_entries_after_the_current_ones() {
    let mut env = EnvMap::new();
    env.insert("UserProfile", r"C:\Users\new");
    let merged = merged_path(
        r"C:\Windows;C:\Tools",
        &[
            registry_path(&user_output()),
            r"C:\WINDOWS\;%MISSING%\bin;;".to_string(),
        ],
        &env,
    );
    assert_eq!(
        merged,
        r"C:\Windows;C:\Tools;C:\Users\new\AppData\Local\Microsoft\WinGet\Links;%MISSING%\bin"
    );
}

#[tokio::test]
async fn refresh_path_only_runs_on_windows_and_keeps_path_when_reg_fails() {
    let spy = Spy::new(|_command, _args, _cwd| ready(CmdOut::fail(1, "denied")));
    let mut env = EnvMap::new();
    env.set_path(r"C:\Windows");
    refresh_path(&spy, &mut env, Platform::Win32).await;
    assert_eq!(spy.calls().len(), 2);
    assert_eq!(env.path(), r"C:\Windows");

    let blocked = Spy::new(|_command, _args, _cwd| ready(CmdOut::fail(1, "should not run")));
    refresh_path(&blocked, &mut env, Platform::Linux).await;
    assert!(blocked.calls().is_empty());
    assert_eq!(env.path(), r"C:\Windows");
}

#[tokio::test]
async fn refresh_path_picks_up_a_tool_installed_later() {
    let user = user_output();
    let spy = Spy::new(
        move |_command: String, args: Vec<String>, _cwd: Option<std::path::PathBuf>| {
            let user = user.clone();
            ready(if args.get(1).is_some_and(|key| key.starts_with("HKCU")) {
                CmdOut::new(0, user, "")
            } else {
                CmdOut::new(
                    0,
                    r"    Path    REG_EXPAND_SZ    C:\Windows;C:\Program Files\GitHub CLI\",
                    "",
                )
            })
        },
    );
    let mut env = EnvMap::new();
    env.set_path(r"C:\Windows");
    env.insert("USERPROFILE", r"C:\Users\new");
    refresh_path(&spy, &mut env, Platform::Win32).await;
    assert_eq!(
        env.path(),
        r"C:\Windows;C:\Program Files\GitHub CLI\;C:\Users\new\AppData\Local\Microsoft\WinGet\Links;C:\Tools\"
    );
}
