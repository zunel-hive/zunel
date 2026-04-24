use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::tool::{DynTool, ToolContext, ToolResult};

/// Suffix appended to error strings so the LLM can self-correct.
/// Byte-identical to Python's `zunel/agent/tools/registry.py` suffix.
const HINT_SUFFIX: &str = "\n\n[Analyze the error above and try a different approach.]";

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, DynTool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: DynTool) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&DynTool> {
        self.tools.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    /// Tool definitions in OpenAI function-call format, with `mcp_*`
    /// tools sorted to the end (matches Python).
    pub fn get_definitions(&self) -> Vec<Value> {
        let (mcp, native): (Vec<&DynTool>, Vec<&DynTool>) = self
            .tools
            .values()
            .partition(|t| t.name().starts_with("mcp_"));

        let mut out: Vec<Value> = Vec::with_capacity(native.len() + mcp.len());
        let mut push = |t: &DynTool| {
            out.push(json!({
                "type": "function",
                "function": {
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters(),
                }
            }));
        };
        for t in &native {
            push(t);
        }
        for t in &mcp {
            push(t);
        }
        out
    }

    /// Dispatch a tool call. Always returns `Ok(ToolResult)` — schema
    /// or unknown-tool failures become `ToolResult::err` with the
    /// byte-compatible hint suffix.
    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, std::convert::Infallible> {
        let Some(tool) = self.tools.get(name) else {
            return Ok(ToolResult::err(format!(
                "unknown tool: {name}{HINT_SUFFIX}"
            )));
        };
        if let Err(msg) = validate_args(&tool.parameters(), &args) {
            return Ok(ToolResult::err(format!("{msg}{HINT_SUFFIX}")));
        }
        Ok(tool.execute(args, ctx).await)
    }
}

/// Minimal JSON-schema validation: checks `required` keys exist and
/// values match the declared primitive type. Matches Python's
/// `Schema.validate_json_schema_value` for the subset our tools use.
fn validate_args(schema: &Value, args: &Value) -> Result<(), String> {
    let Some(obj) = schema.as_object() else {
        return Ok(());
    };
    if let Some(req) = obj.get("required").and_then(Value::as_array) {
        for key in req {
            let Some(k) = key.as_str() else { continue };
            if args.get(k).is_none() {
                return Err(format!("missing required field: {k}"));
            }
        }
    }
    if let Some(props) = obj.get("properties").and_then(Value::as_object) {
        for (k, prop) in props {
            let Some(v) = args.get(k) else { continue };
            let ty = prop.get("type").and_then(Value::as_str).unwrap_or("any");
            let ok = match ty {
                "string" => v.is_string(),
                "integer" => v.is_i64() || v.is_u64(),
                "number" => v.is_number(),
                "boolean" => v.is_boolean(),
                "array" => v.is_array(),
                "object" => v.is_object(),
                _ => true,
            };
            if !ok {
                return Err(format!("field {k}: expected {ty}, got {v}"));
            }
        }
    }
    Ok(())
}
