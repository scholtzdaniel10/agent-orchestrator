//! Named colour themes. Port of `theme.ts`.

pub const THEME_KEY: &str = "orch.theme";

pub const THEMES: &[(&str, &str)] = &[
    ("system", "System"),
    ("ember", "Ember"),
    ("paper", "Paper"),
    ("ultramarine", "Ultramarine"),
    ("phosphor", "Phosphor"),
    ("graphite", "Graphite"),
    ("dusk", "Dusk"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeId {
    System,
    Ember,
    Paper,
    Ultramarine,
    Phosphor,
    Graphite,
    Dusk,
}

impl ThemeId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Ember => "ember",
            Self::Paper => "paper",
            Self::Ultramarine => "ultramarine",
            Self::Phosphor => "phosphor",
            Self::Graphite => "graphite",
            Self::Dusk => "dusk",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "system" => Self::System,
            "ember" => Self::Ember,
            "paper" => Self::Paper,
            "ultramarine" => Self::Ultramarine,
            "phosphor" => Self::Phosphor,
            "graphite" => Self::Graphite,
            "dusk" => Self::Dusk,
            _ => return None,
        })
    }
}

#[allow(clippy::result_unit_err)]
pub trait ThemeStorage {
    fn get_item(&self, key: &str) -> Result<Option<String>, ()>;
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()>;
}

pub trait ThemeRoot {
    fn set_attribute(&mut self, name: &str, value: &str);
    fn remove_attribute(&mut self, name: &str);
}

pub trait ThemeTarget {
    fn dispatch(&mut self, name: &str);
}

pub fn is_theme_id(value: &str) -> bool {
    ThemeId::parse(value).is_some()
}

pub fn load_theme(storage: &dyn ThemeStorage) -> ThemeId {
    match storage.get_item(THEME_KEY) {
        Ok(Some(raw)) => ThemeId::parse(&raw).unwrap_or(ThemeId::System),
        _ => ThemeId::System,
    }
}

pub fn apply_theme(
    id: ThemeId,
    storage: &mut dyn ThemeStorage,
    root: &mut dyn ThemeRoot,
    target: &mut dyn ThemeTarget,
) {
    if id == ThemeId::System {
        root.remove_attribute("data-theme");
    } else {
        root.set_attribute("data-theme", id.as_str());
    }
    let _ = storage.set_item(THEME_KEY, id.as_str());
    target.dispatch("themechange");
}

#[cfg(test)]
mod tests {
    use super::{ThemeId, ThemeRoot, ThemeStorage, ThemeTarget, apply_theme, load_theme};
    use std::collections::BTreeMap;

    struct Memory {
        data: BTreeMap<String, String>,
        fail: bool,
    }

    impl ThemeStorage for Memory {
        fn get_item(&self, key: &str) -> Result<Option<String>, ()> {
            if self.fail {
                return Err(());
            }
            Ok(self.data.get(key).cloned())
        }
        fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()> {
            if self.fail {
                return Err(());
            }
            self.data.insert(key.to_string(), value.to_string());
            Ok(())
        }
    }

    struct Attrs {
        data: BTreeMap<String, String>,
    }

    impl ThemeRoot for Attrs {
        fn set_attribute(&mut self, name: &str, value: &str) {
            self.data.insert(name.to_string(), value.to_string());
        }
        fn remove_attribute(&mut self, name: &str) {
            self.data.remove(name);
        }
    }

    struct Counter(u32);

    impl ThemeTarget for Counter {
        fn dispatch(&mut self, name: &str) {
            if name == "themechange" {
                self.0 += 1;
            }
        }
    }

    #[test]
    fn load_theme_defaults_and_rejects_unknown() {
        let empty = Memory {
            data: BTreeMap::new(),
            fail: false,
        };
        assert_eq!(load_theme(&empty), ThemeId::System);
        let saved = Memory {
            data: BTreeMap::from([("orch.theme".into(), "phosphor".into())]),
            fail: false,
        };
        assert_eq!(load_theme(&saved), ThemeId::Phosphor);
        let unknown = Memory {
            data: BTreeMap::from([("orch.theme".into(), "sunset".into())]),
            fail: false,
        };
        assert_eq!(load_theme(&unknown), ThemeId::System);
        let blocked = Memory {
            data: BTreeMap::new(),
            fail: true,
        };
        assert_eq!(load_theme(&blocked), ThemeId::System);
    }

    #[test]
    fn apply_theme_sets_the_attribute_and_saves() {
        let mut storage = Memory {
            data: BTreeMap::new(),
            fail: false,
        };
        let mut root = Attrs {
            data: BTreeMap::new(),
        };
        let mut target = Counter(0);
        apply_theme(ThemeId::Ember, &mut storage, &mut root, &mut target);
        assert_eq!(
            root.data.get("data-theme").map(String::as_str),
            Some("ember")
        );
        assert_eq!(
            storage.data.get("orch.theme").map(String::as_str),
            Some("ember")
        );
        assert_eq!(target.0, 1);
    }

    #[test]
    fn apply_theme_removes_the_attribute_for_system() {
        let mut storage = Memory {
            data: BTreeMap::from([("orch.theme".into(), "dusk".into())]),
            fail: false,
        };
        let mut root = Attrs {
            data: BTreeMap::from([("data-theme".into(), "dusk".into())]),
        };
        let mut target = Counter(0);
        apply_theme(ThemeId::System, &mut storage, &mut root, &mut target);
        assert!(!root.data.contains_key("data-theme"));
        assert_eq!(
            storage.data.get("orch.theme").map(String::as_str),
            Some("system")
        );
        assert_eq!(target.0, 1);
    }
}
