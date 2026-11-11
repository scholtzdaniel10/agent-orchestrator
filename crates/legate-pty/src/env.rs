use std::collections::BTreeMap;

/// Drop a "no colour" choice inherited from whoever launched the app, and ask for true colour.
///
/// `NO_COLOR` is removed. `FORCE_COLOR=0` and `CLICOLOR=0` are removed. `COLORTERM` is
/// always `truecolor`. Every other variable is copied unchanged.
pub fn terminal_env(source: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for (key, value) in source {
        if key.eq_ignore_ascii_case("NO_COLOR") {
            continue;
        }
        if (key.eq_ignore_ascii_case("FORCE_COLOR") || key.eq_ignore_ascii_case("CLICOLOR"))
            && value == "0"
        {
            continue;
        }
        env.insert(key.clone(), value.clone());
    }
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env
}

pub(crate) fn process_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for (key, value) in std::env::vars_os() {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            continue;
        };
        env.insert(key.to_string(), value.to_string());
    }
    env
}
