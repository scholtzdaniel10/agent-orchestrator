//! Tool-call fast path.
//!
//! One JSON schema covers every tool. llama-server turns it into a grammar, and
//! Ollama receives it as `format`, so each step is JSON that names a real tool.
//!
//! The model is forced into:
//! `{"thought": "...short...", "tool": "<name>", "args": {...}}`

use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::pyjson::{self, PyVal};

/// Tool name that ends the task.
pub const FINAL: &str = "final_answer";
/// Tool name the small model uses to hand a step to the big model.
pub const ESCALATE: &str = "ask_bigger_model";

/// A tool the host or the built-in set can run.
pub struct Tool {
    /// Name the model must put in `tool`.
    pub name: String,
    /// One line shown to the model.
    pub description: String,
    /// JSON schema of the arguments object.
    pub params: Value,
    func: Arc<dyn Fn(Value) -> std::result::Result<String, String> + Send + Sync>,
}

impl Clone for Tool {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            description: self.description.clone(),
            params: self.params.clone(),
            func: Arc::clone(&self.func),
        }
    }
}

impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("params", &self.params)
            .finish()
    }
}

impl Tool {
    /// Register a tool. `func` receives the `args` object and returns the text
    /// shown back to the model, or an error string that is prefixed with `error: `.
    pub fn new<F>(
        name: impl Into<String>,
        description: impl Into<String>,
        params: Value,
        func: F,
    ) -> Self
    where
        F: Fn(Value) -> std::result::Result<String, String> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            description: description.into(),
            params,
            func: Arc::new(func),
        }
    }

    /// Run the tool. Unknown arguments and thrown errors become `error: ...`
    /// strings, matching the Python tools, so the loop never panics on a bad call.
    pub fn call(&self, args: &Value) -> String {
        let Some(obj) = args.as_object() else {
            return "error: bad arguments: args must be an object".to_string();
        };
        if let Some(props) = self.params.get("properties").and_then(Value::as_object) {
            let closed = self
                .params
                .get("additionalProperties")
                .and_then(Value::as_bool)
                == Some(false);
            if closed {
                for key in obj.keys() {
                    if !props.contains_key(key) {
                        return format!("error: bad arguments: unexpected argument '{key}'");
                    }
                }
            }
        }
        if let Some(required) = self.params.get("required").and_then(Value::as_array) {
            for item in required {
                if let Some(name) = item.as_str()
                    && !obj.contains_key(name)
                {
                    return format!("error: bad arguments: missing required argument '{name}'");
                }
            }
        }
        match (self.func)(args.clone()) {
            Ok(text) => text,
            Err(err) => format!("error: {err}"),
        }
    }
}

/// One tool as advertised to an OpenAI-compatible server.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Tool name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// JSON schema of the arguments object.
    pub params: Value,
}

/// One parsed model step.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Short note from the model. Empty when the field was absent.
    pub thought: String,
    /// Tool name.
    pub tool: String,
    /// Arguments object.
    pub args: Value,
    pub(crate) raw: PyVal,
}

impl Step {
    /// Python `json.dumps` of the whole step, used as the assistant message.
    pub fn to_json(&self) -> String {
        pyjson::dump(&self.raw, false)
    }

    pub(crate) fn signature(&self) -> String {
        let args = match &self.raw {
            PyVal::Obj(pairs) => pairs
                .iter()
                .find(|(k, _)| k == "args")
                .map(|(_, v)| v)
                .cloned()
                .unwrap_or(PyVal::Obj(Vec::new())),
            _ => PyVal::Obj(Vec::new()),
        };
        format!("{}{}", self.tool, pyjson::dump(&args, true))
    }

    pub(crate) fn args_json(&self) -> String {
        match &self.raw {
            PyVal::Obj(pairs) => pairs
                .iter()
                .find(|(k, _)| k == "args")
                .map(|(_, v)| pyjson::dump(v, false))
                .unwrap_or_else(|| "{}".to_string()),
            _ => "{}".to_string(),
        }
    }
}

/// Build `{"type":"object","properties":...,"required":[...],"additionalProperties":false}`.
/// `properties` are required, in the order given.
pub fn object_schema(properties: &[(&str, Value)]) -> Value {
    let mut props = Map::new();
    let mut required = Vec::with_capacity(properties.len());
    for (name, schema) in properties {
        props.insert((*name).to_string(), schema.clone());
        required.push(Value::String((*name).to_string()));
    }
    json!({
        "type": "object",
        "properties": Value::Object(props),
        "required": required,
        "additionalProperties": false
    })
}

/// Schema that allows every tool, plus `final_answer`, plus `ask_bigger_model`
/// when `allow_escalate` is set. `thought` is capped at 200 characters.
pub fn step_schema(tools: &[Tool], allow_escalate: bool) -> Value {
    let mut branches = Vec::new();
    for tool in tools {
        branches.push(branch(&tool.name, tool.params.clone()));
    }
    branches.push(branch(FINAL, final_args()));
    if allow_escalate {
        branches.push(branch(ESCALATE, escalate_args()));
    }
    json!({ "oneOf": branches })
}

pub(crate) fn specs_for(tools: &[Tool], allow_escalate: bool) -> Vec<ToolSpec> {
    let mut specs: Vec<ToolSpec> = tools
        .iter()
        .map(|t| ToolSpec {
            name: t.name.clone(),
            description: t.description.clone(),
            params: t.params.clone(),
        })
        .collect();
    specs.push(ToolSpec {
        name: FINAL.to_string(),
        description: "give the final answer to the user.".to_string(),
        params: final_args(),
    });
    if allow_escalate {
        specs.push(ToolSpec {
            name: ESCALATE.to_string(),
            description: "use if the step is too hard for you.".to_string(),
            params: escalate_args(),
        });
    }
    specs
}

pub(crate) fn finish_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: FINAL.to_string(),
        description: "give the final answer to the user.".to_string(),
        params: final_args(),
    }]
}

fn branch(name: &str, args: Value) -> Value {
    json!({
        "type": "object",
        "properties": {
            "thought": {"type": "string", "maxLength": 200},
            "tool": {"const": name},
            "args": args,
        },
        "required": ["thought", "tool", "args"],
        "additionalProperties": false,
    })
}

fn final_args() -> Value {
    json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
        "required": ["answer"],
        "additionalProperties": false
    })
}

fn escalate_args() -> Value {
    json!({
        "type": "object",
        "properties": {"reason": {"type": "string", "maxLength": 120}},
        "required": ["reason"],
        "additionalProperties": false
    })
}

/// The tool list pasted into the system prompt.
pub fn tools_prompt(tools: &[Tool], allow_escalate: bool) -> String {
    let mut lines = vec![
        "You can use these tools. Reply with exactly one JSON object per step:".to_string(),
        "{\"thought\": \"<one short sentence>\", \"tool\": \"<name>\", \"args\": {...}}"
            .to_string(),
        String::new(),
    ];
    for tool in tools {
        let props = tool
            .params
            .get("properties")
            .cloned()
            .unwrap_or_else(|| json!({}));
        lines.push(format!(
            "- {}: {} args={}",
            tool.name,
            tool.description,
            pyjson::dump_value(&props)
        ));
    }
    lines.push(format!(
        "- {FINAL}: give the final answer to the user. args={{\"answer\": \"string\"}}"
    ));
    if allow_escalate {
        lines.push(format!(
            "- {ESCALATE}: use if the step is too hard for you. args={{\"reason\": \"string\"}}"
        ));
    }
    lines.join("\n")
}

/// Parse one model step. `None` when it is not a JSON object with `tool` and object `args`.
pub fn parse_step(text: &str) -> Option<Step> {
    let raw = pyjson::parse_json(text)?;
    let PyVal::Obj(pairs) = &raw else {
        return None;
    };
    if !pairs.iter().any(|(k, _)| k == "tool") {
        return None;
    }
    let tool = match pairs.iter().find(|(k, _)| k == "tool") {
        Some((_, PyVal::Str(name))) => name.clone(),
        _ => return None,
    };
    let args_py = match pairs.iter().find(|(k, _)| k == "args") {
        Some((_, v @ PyVal::Obj(_))) => v,
        _ => return None,
    };
    let thought = match pairs.iter().find(|(k, _)| k == "thought") {
        Some((_, PyVal::Str(text))) => text.clone(),
        _ => String::new(),
    };
    Some(Step {
        thought,
        tool,
        args: pyjson::to_value(args_py),
        raw,
    })
}

/// Build the assistant JSON we store after an OpenAI tool call.
pub(crate) fn synthesize_step(thought: &str, tool: &str, args_json: &str) -> String {
    let args = pyjson::parse_json(args_json).unwrap_or(PyVal::Obj(Vec::new()));
    let obj = PyVal::Obj(vec![
        ("thought".to_string(), PyVal::Str(thought.to_string())),
        ("tool".to_string(), PyVal::Str(tool.to_string())),
        ("args".to_string(), args),
    ]);
    pyjson::dump(&obj, false)
}
