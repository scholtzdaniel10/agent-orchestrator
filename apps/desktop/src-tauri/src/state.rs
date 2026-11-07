//! Process-wide shell state: the backend handle, window ids, and pop-out lists.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use legate_app::{AppError, AppHandle, AppOptions, PoppedOutRegistry, Prefs, app_data_dir};
use legate_core::{CursorModels, UsageShows};

#[derive(Clone)]
pub struct Shell {
    inner: Arc<Inner>,
}

struct Inner {
    backend: Result<AppHandle, String>,
    windows: Mutex<HashMap<String, u64>>,
    next_id: AtomicU64,
    terminals: Mutex<PoppedOutRegistry>,
    panels: Mutex<PoppedOutRegistry>,
    notified: Mutex<HashSet<String>>,
    prefs: Mutex<Prefs>,
}

impl Shell {
    pub fn boot() -> Self {
        let options = AppOptions {
            data_dir: Some(app_data_dir(None)),
            ..AppOptions::default()
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let (backend, prefs) = match runtime {
            Ok(runtime) => {
                let started = runtime.block_on(AppHandle::start(options));
                match started {
                    Ok(handle) => {
                        let prefs = runtime
                            .block_on(handle.get_prefs())
                            .unwrap_or_else(|_| default_prefs());
                        (Ok(handle), prefs)
                    }
                    Err(err) => (Err(err.to_string()), default_prefs()),
                }
            }
            Err(err) => (Err(err.to_string()), default_prefs()),
        };
        Self {
            inner: Arc::new(Inner {
                backend,
                windows: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                terminals: Mutex::new(PoppedOutRegistry::new()),
                panels: Mutex::new(PoppedOutRegistry::new()),
                notified: Mutex::new(HashSet::new()),
                prefs: Mutex::new(prefs),
            }),
        }
    }

    pub fn backend(&self) -> Result<&AppHandle, String> {
        self.inner.backend.as_ref().map_err(Clone::clone)
    }

    pub fn fresh_label(&self, prefix: &str) -> String {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{id}")
    }

    pub fn window_id(&self, label: &str) -> u64 {
        let mut windows = self
            .inner
            .windows
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(id) = windows.get(label).copied() {
            return id;
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        windows.insert(label.to_string(), id);
        id
    }

    pub fn forget_window(&self, label: &str) -> Option<u64> {
        self.inner
            .windows
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(label)
    }

    pub fn terminals(&self) -> std::sync::MutexGuard<'_, PoppedOutRegistry> {
        self.inner
            .terminals
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    pub fn panels(&self) -> std::sync::MutexGuard<'_, PoppedOutRegistry> {
        self.inner
            .panels
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    pub fn terminal_popped(&self, id: &str) -> bool {
        self.terminals().has(id)
    }

    pub fn note_job(&self, id: &str) -> bool {
        self.inner
            .notified
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(id.to_string())
    }

    pub fn prefs(&self) -> Prefs {
        self.inner
            .prefs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub fn set_prefs(&self, prefs: Prefs) {
        *self
            .inner
            .prefs
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = prefs;
    }
}

fn default_prefs() -> Prefs {
    Prefs {
        max_parallel: None,
        terminal_font_size: 13,
        notify: true,
        clone_dir: None,
        ui_scale: 100,
        usage_shows: UsageShows::Left,
        cursor_models: CursorModels::Own,
        usage_check: true,
        local_url: legate_core::DEFAULT_LOCAL_URL.into(),
        local_model: legate_core::DEFAULT_LOCAL_MODEL.into(),
        local_small_model: None,
        local_num_ctx: legate_core::DEFAULT_LOCAL_NUM_CTX,
    }
}

pub fn err_string(err: AppError) -> String {
    err.to_string()
}
