//! Prefix store.
//!
//! The system prompt and tool list are identical on every turn. llama-server
//! processes that prefix once, saves the KV cache (`/slots/{id}?action=save`),
//! and restores it after a restart. Ollama has no slot files, so the store is
//! disabled there and the append-only message list is what hits Ollama's cache.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::ServerKind;
use crate::client::Backend;
use crate::error::Result;

/// Counts from [`PrefixStore::ensure`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrefixStats {
    /// The slot already held this prefix.
    pub warm_hits: u64,
    /// The prefix was loaded from a slot file.
    pub restores: u64,
    /// The prefix was processed and then saved.
    pub builds: u64,
}

/// Remembers which prefix is loaded in one server slot.
#[derive(Debug, Clone)]
pub struct PrefixStore {
    /// Counts across runs on this agent.
    pub stats: PrefixStats,
    save_dir: Option<PathBuf>,
    loaded_key: Option<String>,
    enabled: bool,
}

impl PrefixStore {
    pub(crate) fn new(kind: ServerKind, save_dir: Option<PathBuf>) -> Self {
        Self {
            stats: PrefixStats::default(),
            save_dir,
            loaded_key: None,
            enabled: matches!(kind, ServerKind::LlamaServer),
        }
    }

    /// `sha256(model_tag + NUL + prefix)[:24]`, matching the Python store.
    pub fn key_for(prefix_text: &str, model_tag: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model_tag.as_bytes());
        hasher.update([0u8]);
        hasher.update(prefix_text.as_bytes());
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(24);
        for byte in digest {
            if hex.len() >= 24 {
                break;
            }
            hex.push_str(&format!("{byte:02x}"));
        }
        hex.truncate(24);
        hex
    }

    /// Make sure the server slot holds the KV cache for `prefix_text`.
    ///
    /// Restore and save failures are ignored: the in-memory cache still works
    /// for this process when the server was started without `--slot-save-path`.
    pub(crate) async fn ensure(&mut self, server: &mut Backend, prefix_text: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let key = Self::key_for(prefix_text, server.name());
        if self.loaded_key.as_deref() == Some(key.as_str()) {
            self.stats.warm_hits += 1;
            return Ok(());
        }
        let filename = format!("{key}.bin");
        let on_disk = match &self.save_dir {
            None => true,
            Some(dir) => dir.join(&filename).exists(),
        };
        if on_disk && let Ok(()) = server.slot_restore(&filename).await {
            self.loaded_key = Some(key);
            self.stats.restores += 1;
            return Ok(());
        }
        // Process the prefix once and do not generate. n_predict = 0.
        server.complete_prompt(prefix_text, None, 0).await?;
        // Save can fail when the server was started without --slot-save-path.
        // The in-memory slot still holds the prefix for this process.
        let _saved = server.slot_save(&filename).await;
        self.loaded_key = Some(key);
        self.stats.builds += 1;
        Ok(())
    }
}
