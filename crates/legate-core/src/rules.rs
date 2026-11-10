//! Router rules loading from `src/core/router/router.ts`.
//!
//! The embedded file is `rules.json` with `local` added at the end of every job-type list.
//! Files passed to [`load_rules`] are used as written.

use std::path::Path;

use serde_json::Value;

use crate::types::{JobType, PaceRules, RouterRules};
use crate::util::json_i64;

/// Bundled copy of `rules.json`, including the local provider.
const BUNDLED_RULES: &str = include_str!("../rules.json");

pub const DEFAULT_PACE: PaceRules = PaceRules {
    weight: 1.5,
    min: 0.5,
    max: 2.0,
    at_risk_slack: 0.25,
    at_risk_left: 0.15,
};

/// Jobs one plan may run at once when rules omit or mis-set `maxParallel`.
pub const DEFAULT_MAX_PARALLEL: i64 = 3;
const MAX_PARALLEL_MIN: i64 = 1;
const MAX_PARALLEL_MAX: i64 = 8;

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("router rules must be a JSON object")]
    NotObject,
    #[error("router rules missing fallbackFit")]
    MissingFallbackFit,
    #[error("router rules missing allowance")]
    MissingAllowance,
    #[error("router rules missing defaultRestHours")]
    MissingDefaultRestHours,
    #[error("router rules missing rules")]
    MissingRules,
    #[error("router rules missing a non-empty provider list for job type \"{0}\"")]
    MissingJobType(String),
    #[error("router rules: invalid pace settings")]
    InvalidPace,
    #[error("failed to read router rules: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse router rules: {0}")]
    Json(#[from] serde_json::Error),
}

/// Load router settings. With no path, returns the embedded rules.
pub fn load_rules(path: Option<&Path>) -> Result<RouterRules, RulesError> {
    let text = match path {
        Some(path) => read_rules_file(path)?,
        None => BUNDLED_RULES.to_string(),
    };
    validate(&serde_json::from_str(&text)?)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_rules_file(path: &Path) -> Result<String, RulesError> {
    Ok(std::fs::read_to_string(path)?)
}

#[cfg(target_arch = "wasm32")]
fn read_rules_file(_path: &Path) -> Result<String, RulesError> {
    Err(RulesError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "router rules files are not available on wasm",
    )))
}

fn validate(raw: &Value) -> Result<RouterRules, RulesError> {
    let Some(body) = raw.as_object() else {
        return Err(RulesError::NotObject);
    };
    if is_nullish(body.get("fallbackFit")) {
        return Err(RulesError::MissingFallbackFit);
    }
    if is_nullish(body.get("allowance")) {
        return Err(RulesError::MissingAllowance);
    }
    if is_nullish(body.get("defaultRestHours")) {
        return Err(RulesError::MissingDefaultRestHours);
    }
    let Some(table) = body.get("rules").and_then(Value::as_object) else {
        return Err(RulesError::MissingRules);
    };
    for job in JobType::ALL {
        require_provider_list(table, job.as_str())?;
    }
    if let Some(pace) = body.get("pace") {
        validate_pace(pace)?;
    }
    let max_parallel = read_max_parallel(body.get("maxParallel"));

    let mut normalized = body.clone();
    normalized.insert("maxParallel".to_string(), Value::from(max_parallel));
    Ok(serde_json::from_value(Value::Object(normalized))?)
}

fn is_nullish(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(_) => false,
    }
}

fn require_provider_list(
    table: &serde_json::Map<String, Value>,
    job: &str,
) -> Result<(), RulesError> {
    match table.get(job).and_then(Value::as_array) {
        Some(list) if !list.is_empty() => Ok(()),
        _ => Err(RulesError::MissingJobType(job.to_string())),
    }
}

fn read_max_parallel(value: Option<&Value>) -> i64 {
    let Some(value) = value else {
        return DEFAULT_MAX_PARALLEL;
    };
    let Some(number) = json_i64(value) else {
        return DEFAULT_MAX_PARALLEL;
    };
    number.clamp(MAX_PARALLEL_MIN, MAX_PARALLEL_MAX)
}

fn validate_pace(pace: &Value) -> Result<(), RulesError> {
    let Some(body) = pace.as_object() else {
        return Err(RulesError::InvalidPace);
    };
    let Some(weight) = body.get("weight").and_then(finite) else {
        return Err(RulesError::InvalidPace);
    };
    let Some(min) = body.get("min").and_then(finite) else {
        return Err(RulesError::InvalidPace);
    };
    let Some(max) = body.get("max").and_then(finite) else {
        return Err(RulesError::InvalidPace);
    };
    let Some(at_risk_slack) = body.get("atRiskSlack").and_then(finite) else {
        return Err(RulesError::InvalidPace);
    };
    let Some(at_risk_left) = body.get("atRiskLeft").and_then(finite) else {
        return Err(RulesError::InvalidPace);
    };
    if !(min > 0.0 && min <= 1.0 && max >= 1.0)
        || weight < 0.0
        || !(0.0..=1.0).contains(&at_risk_slack)
        || !(0.0..=1.0).contains(&at_risk_left)
    {
        return Err(RulesError::InvalidPace);
    }
    Ok(())
}

fn finite(value: &Value) -> Option<f64> {
    let number = value.as_f64()?;
    number.is_finite().then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ProviderId;
    use std::fs;

    fn temp_dir() -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("legate-rules-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn providers(names: &[&str]) -> Vec<ProviderId> {
        names
            .iter()
            .map(|name| ProviderId::parse(name).unwrap())
            .collect()
    }

    #[test]
    fn load_rules_parses_the_bundled_file_and_covers_every_job_type() {
        let rules = load_rules(None).unwrap();
        assert_eq!(rules.fallback_fit, 0.5);
        assert_eq!(rules.default_rest_hours, 5.0);
        assert_eq!(rules.max_parallel, 3);
        assert_eq!(rules.allowance[&ProviderId::Claude].window_hours, 5.0);
        assert_eq!(rules.allowance[&ProviderId::Claude].units, 15.0);
        assert_eq!(rules.allowance[&ProviderId::Cursor].window_hours, 720.0);
        assert_eq!(rules.allowance[&ProviderId::Cursor].units, 20_000_000.0);
        let keys: Vec<_> = rules.allowance.keys().map(|id| id.as_str()).collect();
        assert_eq!(keys, vec!["claude", "cursor"]);
        assert_eq!(
            rules.rules.planning,
            providers(&["claude", "codex", "cursor", "grok", "local"])
        );
        assert_eq!(
            rules.rules.review,
            providers(&["claude", "codex", "cursor", "grok", "local"])
        );
        assert_eq!(
            rules.rules.debugging,
            providers(&["grok", "cursor", "codex", "claude", "local"])
        );
        assert_eq!(
            rules.rules.refactor,
            providers(&["grok", "cursor", "codex", "claude", "local"])
        );
        assert_eq!(
            rules.rules.boilerplate,
            providers(&["grok", "cursor", "codex", "claude", "local"])
        );
        assert_eq!(rules.pace, Some(DEFAULT_PACE));
        for job in JobType::ALL {
            assert!(!rules.rules.get(job).is_empty());
        }
    }

    #[test]
    fn load_rules_defaults_and_clamps_max_parallel() {
        let dir = Temp(temp_dir());
        let path = dir.0.join("rules.json");
        let bundled = serde_json::to_value(load_rules(None).unwrap()).unwrap();

        let mut without = bundled.clone();
        without.as_object_mut().unwrap().remove("maxParallel");
        fs::write(&path, serde_json::to_string(&without).unwrap()).unwrap();
        assert_eq!(load_rules(Some(&path)).unwrap().max_parallel, 3);

        for (value, expected) in [(0, 1), (99, 8), (2, 2)] {
            let mut body = bundled.clone();
            body["maxParallel"] = Value::from(value);
            fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
            assert_eq!(load_rules(Some(&path)).unwrap().max_parallel, expected);
        }

        let bad = [
            Value::from(1.5),
            Value::String("3".to_string()),
            Value::Null,
            Value::Bool(true),
            // JSON.stringify(NaN) is null, so this is the same case the TypeScript test writes.
            Value::Null,
        ];
        for value in bad {
            let mut body = bundled.clone();
            body["maxParallel"] = value;
            fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
            assert_eq!(load_rules(Some(&path)).unwrap().max_parallel, 3);
        }
    }

    #[test]
    fn load_rules_path_loads_an_edited_copy_of_the_rules() {
        let dir = Temp(temp_dir());
        let path = dir.0.join("rules.json");
        let mut edited = load_rules(None).unwrap();
        edited.fallback_fit = 0.9;
        edited.rules.boilerplate = providers(&["claude", "cursor"]);
        fs::write(&path, serde_json::to_string(&edited).unwrap()).unwrap();
        let rules = load_rules(Some(&path)).unwrap();
        assert_eq!(rules.fallback_fit, 0.9);
        assert_eq!(rules.rules.boilerplate, providers(&["claude", "cursor"]));
        assert_eq!(
            load_rules(None).unwrap().rules.boilerplate,
            providers(&["grok", "cursor", "codex", "claude", "local"])
        );
    }

    #[test]
    fn load_rules_rejects_a_file_missing_a_job_type() {
        let dir = Temp(temp_dir());
        let path = dir.0.join("rules.json");
        let body = serde_json::json!({
            "rules": { "planning": ["claude"] },
            "fallbackFit": 0.5,
            "allowance": { "claude": { "windowHours": 5, "units": 15 } },
            "defaultRestHours": 5
        });
        fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
        let err = load_rules(Some(&path)).unwrap_err();
        assert!(err.to_string().contains("debugging"), "{err}");
    }

    #[test]
    fn load_rules_accepts_a_file_with_no_pace_block_and_rejects_a_bad_one() {
        let dir = Temp(temp_dir());
        let path = dir.0.join("rules.json");
        let bundled = serde_json::to_value(load_rules(None).unwrap()).unwrap();
        let mut without_pace = bundled.clone();
        without_pace.as_object_mut().unwrap().remove("pace");
        fs::write(&path, serde_json::to_string(&without_pace).unwrap()).unwrap();
        let loaded = load_rules(Some(&path)).unwrap();
        assert_eq!(loaded.pace, None);
        assert_eq!(loaded.fallback_fit, 0.5);

        let mut boundary = bundled.clone();
        boundary["pace"] = serde_json::json!({
            "weight": 0, "min": 1, "max": 1, "atRiskSlack": 0, "atRiskLeft": 1
        });
        fs::write(&path, serde_json::to_string(&boundary).unwrap()).unwrap();
        assert_eq!(
            load_rules(Some(&path)).unwrap().pace,
            Some(PaceRules {
                weight: 0.0,
                min: 1.0,
                max: 1.0,
                at_risk_slack: 0.0,
                at_risk_left: 1.0
            })
        );

        let bad = [
            serde_json::json!({"weight": -1, "min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
            serde_json::json!({"weight": 1.5, "min": 0, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
            serde_json::json!({"weight": 1.5, "min": 1.1, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
            serde_json::json!({"weight": 1.5, "min": 0.5, "max": 0.9, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
            serde_json::json!({"weight": null, "min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
            serde_json::json!({"weight": 1.5, "min": 0.5, "max": 2, "atRiskSlack": 1.1, "atRiskLeft": 0.15}),
            serde_json::json!({"weight": 1.5, "min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": -0.01}),
            serde_json::json!({"min": 0.5, "max": 2, "atRiskSlack": 0.25, "atRiskLeft": 0.15}),
            Value::Null,
            Value::String("fast".to_string()),
        ];
        for pace in bad {
            let mut body = bundled.clone();
            body["pace"] = pace;
            fs::write(&path, serde_json::to_string(&body).unwrap()).unwrap();
            let err = load_rules(Some(&path)).unwrap_err();
            assert_eq!(err.to_string(), "router rules: invalid pace settings");
        }
    }
}
