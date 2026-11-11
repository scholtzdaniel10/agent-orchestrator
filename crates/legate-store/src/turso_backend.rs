//! Turso open path, compiled only with `--features turso`.
//!
//! Not used by [`crate::Store`]. A second process on Windows does not share a Turso
//! file reliably; the Log in `docs/rust-rewrite.md` has the failure. The desktop app
//! and the CLI use the rusqlite backend instead.

use std::time::Duration;

use turso::Builder;

/// Open a local file and set the same busy timeout the TypeScript store uses.
pub async fn open_local(path: &str) -> Result<(), turso::Error> {
    let db = Builder::new_local(path).build().await?;
    let conn = db.connect()?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.execute("PRAGMA busy_timeout = 5000", ()).await?;
    let _ = conn;
    Ok(())
}

/// Same as [`open_local`], with Turso's experimental multi-process WAL flag.
/// On this Windows machine that open fails: the flag is not supported by the IO backend.
pub async fn open_multiprocess(path: &str) -> Result<(), turso::Error> {
    let db = Builder::new_local(path)
        .experimental_multiprocess_wal(true)
        .build()
        .await?;
    let conn = db.connect()?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    let _ = conn;
    Ok(())
}
