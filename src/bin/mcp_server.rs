//! Orbit Recon MCP server.
//!
//! A minimal, dependency-light [Model Context Protocol](https://modelcontextprotocol.io)
//! server that exposes Orbit Recon's four deterministic health checks as MCP
//! tools over stdio (newline-delimited JSON-RPC 2.0). It is a thin wrapper: every
//! tool calls straight into the `orbit_recon` library — there is no analysis
//! logic here.
//!
//! ## Why a hand-rolled server instead of `rmcp`?
//!
//! The tools are deterministic, synchronous DuckDB reads. A single-threaded
//! stdin/stdout loop over `serde_json` (already a dependency) is a perfect fit
//! and adds **zero** new crates, keeping the build offline-friendly and
//! preserving DuckDB's single-threaded usage. Pulling in the async `rmcp` SDK
//! would force a full multi-feature tokio runtime — the opposite of this repo's
//! deliberately minimal `tokio = ["rt", "time"]` pin.
//!
//! ## Protocol surface
//!
//! - `initialize` — handshake, returns server info + capabilities.
//! - `tools/list` — the five tools below with JSON input schemas.
//! - `tools/call` — dispatches to the matching library function.
//! - `notifications/initialized` (notification) — acknowledged, no reply.
//!
//! ## Tools
//!
//! - `analyze_dead_code`
//! - `detect_circular_dependencies`
//! - `analyze_coupling`
//! - `detect_architectural_drift`
//! - `health_scan` (all four + graph stats + summary)

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use orbit_recon::config::Config;
use orbit_recon::findings::Severity;

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "orbit-recon";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// JSON-RPC standard error codes.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle_message(&request),
            Err(e) => Some(error_response(
                Value::Null,
                INVALID_PARAMS,
                &format!("Parse error: {e}"),
            )),
        };

        // Notifications (no `id`) produce no response.
        if let Some(response) = response {
            let serialized = serde_json::to_string(&response)
                .unwrap_or_else(|_| "{}".to_string());
            writeln!(out, "{serialized}")?;
            out.flush()?;
        }
    }

    Ok(())
}

/// Dispatch a single JSON-RPC message. Returns `None` for notifications (which
/// must not receive a response).
pub fn handle_message(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    // Notifications have no `id`; acknowledge silently.
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    match method {
        "initialize" => Some(success_response(id, initialize_result())),
        "tools/list" => Some(success_response(id, json!({ "tools": tool_definitions() }))),
        "tools/call" => Some(handle_tools_call(id, &params)),
        "ping" => Some(success_response(id, json!({}))),
        other => Some(error_response(
            id,
            METHOD_NOT_FOUND,
            &format!("Method not found: {other}"),
        )),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
    })
}

/// The MCP tool catalogue. Descriptions are sourced from AGENTS.md so the agent
/// sees the same semantics documented for the GitLab Duo skill.
pub fn tool_definitions() -> Value {
    // Shared input properties reused across tools.
    let repo_prop = json!({
        "type": "string",
        "description": "Path to the repository containing a .orbit/ directory (defaults to the current directory)."
    });
    let db_prop = json!({
        "type": "string",
        "description": "Explicit path to the Orbit DuckDB graph file, overriding .orbit/ auto-detection."
    });
    let severity_prop = json!({
        "type": "string",
        "enum": ["info", "warning", "critical"],
        "description": "Minimum severity to include in findings (default: info)."
    });

    json!([
        {
            "name": "analyze_dead_code",
            "description": "Identify dead code: functions, classes, methods, structs, enums, traits, and interfaces that have no incoming reference edges in the Orbit Knowledge Graph — meaning nothing in the codebase calls or uses them. Returns findings with file, line, severity, and remediation.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "detect_circular_dependencies",
            "description": "Detect circular dependency chains between modules — bidirectional (A<->B) and longer cycles (A->B->C->A) — a key indicator of coupling problems that slow builds and increase complexity. Returns each cycle with its modules and cross-reference count.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "analyze_coupling",
            "description": "Measure module coupling via the fan-out metric — how many other modules each module depends on. Flags modules whose fan-out exceeds the configured warning/critical thresholds. Returns modules with their fan-out and dependency lists.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "detect_architectural_drift",
            "description": "Track architectural drift — when the actual dependency graph diverges from intended module boundaries (e.g. domain-layer code reaching into infrastructure). Checks configured (or default layered) boundary rules and reports imports outside the allowed scope.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "health_scan",
            "description": "Run a full codebase health scan: dead code, circular dependencies, module coupling, and architectural drift, plus graph statistics. Returns the complete structured report (graph size, per-category counts, and all findings) — the same report the orbit-recon CLI produces.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": repo_prop,
                    "db": db_prop,
                    "severity": severity_prop,
                    "format": {
                        "type": "string",
                        "enum": ["json", "markdown"],
                        "description": "Report shape returned in the text content (default: json)."
                    }
                },
                "additionalProperties": false
            }
        }
    ])
}

fn handle_tools_call(id: Value, params: &Value) -> Value {
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return error_response(id, INVALID_PARAMS, "Missing tool `name`."),
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match call_tool(name, &args) {
        Ok(text) => success_response(
            id,
            json!({
                "content": [ { "type": "text", "text": text } ],
                "isError": false
            }),
        ),
        // Tool-execution failures are returned as a successful JSON-RPC response
        // with `isError: true`, per the MCP tools/call convention, so the agent
        // can read the message rather than getting a protocol-level error.
        Err(ToolError::Execution(msg)) => success_response(
            id,
            json!({
                "content": [ { "type": "text", "text": msg } ],
                "isError": true
            }),
        ),
        Err(ToolError::UnknownTool(msg)) => error_response(id, METHOD_NOT_FOUND, &msg),
        Err(ToolError::BadParams(msg)) => error_response(id, INVALID_PARAMS, &msg),
    }
}

/// Errors distinguishing protocol-level failures from in-tool execution failures.
#[derive(Debug)]
pub enum ToolError {
    UnknownTool(String),
    BadParams(String),
    Execution(String),
}

/// Execute a named tool with its JSON arguments, returning the text payload.
///
/// This is the testable core: it performs no I/O of its own beyond the DuckDB
/// reads the library functions do, so unit tests can drive it directly.
pub fn call_tool(name: &str, args: &Value) -> Result<String, ToolError> {
    let repo = args
        .get("repo")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let db_override = args.get("db").and_then(|v| v.as_str()).map(PathBuf::from);
    let min_severity = args
        .get("severity")
        .and_then(|v| v.as_str())
        .map(Severity::from_str)
        .unwrap_or(Severity::Info);

    // Config is loaded from the repo's .orbit-recon.yml when present, otherwise
    // defaults — matching CLI behaviour.
    let cfg = load_config(&repo);

    let check = match name {
        "analyze_dead_code" => Some("dead_code"),
        "detect_circular_dependencies" => Some("circular_dependencies"),
        "analyze_coupling" => Some("coupling"),
        "detect_architectural_drift" => Some("architectural_drift"),
        "health_scan" => None,
        other => {
            return Err(ToolError::UnknownTool(format!("Unknown tool: {other}")));
        }
    };

    match check {
        // Single-check tools: return the findings array as JSON.
        Some(check) => {
            let db_path = resolve_db(&repo, db_override.as_deref())
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            let conn = orbit_recon::open_graph(&db_path)
                .map_err(|e| ToolError::Execution(e.to_string()))?;
            let findings = orbit_recon::run_checks(
                &conn,
                &cfg,
                &[check.to_string()],
                min_severity,
            )
            .map_err(|e| ToolError::Execution(e.to_string()))?;

            let payload = json!({
                "check": check,
                "count": findings.len(),
                "findings": findings,
            });
            serde_json::to_string_pretty(&payload)
                .map_err(|e| ToolError::Execution(e.to_string()))
        }
        // Full scan: return the whole report.
        None => {
            let format = args
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("json");
            let checks: Vec<String> = orbit_recon::ALL_CHECKS
                .iter()
                .map(|s| s.to_string())
                .collect();
            let report = orbit_recon::scan_repo(
                &repo,
                db_override.as_deref(),
                &cfg,
                &checks,
                min_severity,
            )
            .map_err(|e| ToolError::Execution(e.to_string()))?;

            match format {
                "markdown" => Ok(report.to_markdown()),
                _ => serde_json::to_string_pretty(&report)
                    .map_err(|e| ToolError::Execution(e.to_string())),
            }
        }
    }
}

fn resolve_db(repo: &Path, db_override: Option<&Path>) -> anyhow::Result<PathBuf> {
    match db_override {
        Some(p) => Ok(p.to_path_buf()),
        None => orbit_recon::find_duckdb_path(repo),
    }
}

fn load_config(repo: &Path) -> Config {
    let default_config = repo.join(".orbit-recon.yml");
    if default_config.exists() {
        Config::from_file(&default_config).unwrap_or_default()
    } else {
        Config::default()
    }
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    let _ = INTERNAL_ERROR; // reserved for future use
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_returns_server_info_and_protocol() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        let resp = handle_message(&req).expect("initialize must respond");
        assert_eq!(resp["id"], json!(1));
        assert_eq!(resp["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
        assert_eq!(resp["result"]["serverInfo"]["name"], json!(SERVER_NAME));
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notification_produces_no_response() {
        // A message without `id` is a notification and must be silently accepted.
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        assert!(handle_message(&notif).is_none());
    }

    #[test]
    fn tools_list_returns_the_five_expected_tools() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        let resp = handle_message(&req).expect("tools/list must respond");
        let tools = resp["result"]["tools"]
            .as_array()
            .expect("tools must be an array");

        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();

        assert_eq!(names.len(), 5, "expected exactly five tools");
        for expected in [
            "analyze_dead_code",
            "detect_circular_dependencies",
            "analyze_coupling",
            "detect_architectural_drift",
            "health_scan",
        ] {
            assert!(names.contains(&expected), "missing tool `{expected}`");
        }

        // Every tool must carry a description and a JSON object input schema —
        // the metadata an MCP client relies on.
        for tool in tools {
            assert!(
                tool["description"].as_str().is_some_and(|d| !d.is_empty()),
                "tool `{}` is missing a description",
                tool["name"]
            );
            assert_eq!(tool["inputSchema"]["type"], json!("object"));
        }
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "does/not/exist",
            "params": {}
        });
        let resp = handle_message(&req).expect("must respond");
        assert_eq!(resp["error"]["code"], json!(METHOD_NOT_FOUND));
    }

    #[test]
    fn unknown_tool_is_reported() {
        // Drive the tool core directly with a nonexistent tool name.
        let err = call_tool("not_a_tool", &json!({}));
        assert!(matches!(err, Err(ToolError::UnknownTool(_))));
    }

    #[test]
    fn tools_call_on_real_graph_returns_findings() {
        // Build a tiny in-memory-style Orbit graph on disk (no network) so a
        // tools/call exercises the full path: dispatch -> library -> DuckDB.
        let dir = std::env::temp_dir().join(format!("orbit-recon-mcp-test-{}", std::process::id()));
        let orbit_dir = dir.join(".orbit");
        std::fs::create_dir_all(&orbit_dir).unwrap();
        let db_path = orbit_dir.join("orbit.duckdb");

        // Create the schema the queries expect and insert one clearly-dead
        // definition (a function referenced by nothing).
        {
            let conn = duckdb::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE definitions (name VARCHAR, file VARCHAR, kind VARCHAR, line BIGINT);
                CREATE TABLE "references" (
                    source_name VARCHAR, source_file VARCHAR,
                    target_name VARCHAR, target_file VARCHAR
                );
                INSERT INTO definitions VALUES ('orphan_fn', 'src/util/orphan.rs', 'function', 10);
                INSERT INTO definitions VALUES ('used_fn', 'src/util/used.rs', 'function', 5);
                INSERT INTO "references" VALUES ('caller', 'src/main.rs', 'used_fn', 'src/util/used.rs');
                "#,
            )
            .unwrap();
        }

        // Call the dead-code tool via the JSON-RPC dispatch layer.
        let req = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/call",
            "params": {
                "name": "analyze_dead_code",
                "arguments": { "repo": dir.to_str().unwrap() }
            }
        });
        let resp = handle_message(&req).expect("tools/call must respond");
        assert_eq!(resp["id"], json!(42));
        assert_eq!(
            resp["result"]["isError"],
            json!(false),
            "tool errored: {}",
            resp["result"]["content"][0]["text"]
        );

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["check"], json!("dead_code"));

        // `orphan_fn` is dead; `used_fn` is referenced. Expect the orphan to
        // appear and the used function not to.
        let findings = parsed["findings"].as_array().unwrap();
        let names: Vec<&str> = findings
            .iter()
            .filter_map(|f| f["location"]["name"].as_str())
            .collect();
        assert!(names.contains(&"orphan_fn"), "orphan_fn should be dead code");
        assert!(!names.contains(&"used_fn"), "used_fn is referenced, not dead");

        // Also verify the full health_scan tool runs end to end and yields a
        // report with graph_stats.
        let scan = call_tool("health_scan", &json!({ "repo": dir.to_str().unwrap() }))
            .expect("health_scan must succeed");
        let scan_json: Value = serde_json::from_str(&scan).unwrap();
        assert!(scan_json["graph_stats"]["nodes"].as_i64().unwrap() >= 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
