//! First-run setup flag. Port of `setup-seen.ts`.

const KEY: &str = "orch.setupSeen";

#[allow(clippy::result_unit_err)]
pub trait SeenStorage {
    fn get_item(&self, key: &str) -> Result<Option<String>, ()>;
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()>;
}

/// False until Setup has been closed once. Missing storage counts as seen.
pub fn setup_seen(storage: Option<&dyn SeenStorage>) -> bool {
    let Some(storage) = storage else {
        return true;
    };
    match storage.get_item(KEY) {
        Ok(value) => value.as_deref() == Some("1"),
        Err(()) => true,
    }
}

pub fn mark_setup_seen(storage: Option<&mut dyn SeenStorage>) {
    let Some(storage) = storage else {
        return;
    };
    let _ = storage.set_item(KEY, "1");
}

#[cfg(test)]
mod tests {
    use super::{SeenStorage, mark_setup_seen, setup_seen};
    use std::collections::HashMap;

    struct Memory {
        data: HashMap<String, String>,
    }

    impl SeenStorage for Memory {
        fn get_item(&self, key: &str) -> Result<Option<String>, ()> {
            Ok(self.data.get(key).cloned())
        }
        fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()> {
            self.data.insert(key.to_string(), value.to_string());
            Ok(())
        }
    }

    struct Broken;

    impl SeenStorage for Broken {
        fn get_item(&self, _key: &str) -> Result<Option<String>, ()> {
            Err(())
        }
        fn set_item(&mut self, _key: &str, _value: &str) -> Result<(), ()> {
            Err(())
        }
    }

    #[test]
    fn a_new_install_has_not_seen_setup() {
        let mut storage = Memory {
            data: HashMap::new(),
        };
        assert!(!setup_seen(Some(&storage)));
        mark_setup_seen(Some(&mut storage));
        assert!(setup_seen(Some(&storage)));
    }

    #[test]
    fn missing_or_throwing_storage_counts_as_seen() {
        assert!(setup_seen(None));
        let mut broken = Broken;
        assert!(setup_seen(Some(&broken)));
        mark_setup_seen(Some(&mut broken));
        mark_setup_seen(None);
    }
}
