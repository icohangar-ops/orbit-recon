//! Orbit Recon MCP server.
//!
//! This module exposes Orbit Recon's graph-health checks as MCP tools over
//! stdio using framed JSON-RPC 2.0 messages (`Content-Length` headers). The
//! analysis itself stays in the shared library layer; the MCP server only
//! handles transport, tool dispatch, and result formatting.

use crate::findings::Severity;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "orbit-recon";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

#[derive(Debug)]
pub enum ToolError {
    UnknownTool(String),
    BadParams(String),
    Execution(String),
}

pub fn serve_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    while let Some(message) = read_message(&mut reader)? {
        if let Some(response) = handle_message(&message) {
            write_message(&mut writer, &response)?;
        }
    }

    Ok(())
}

pub fn handle_message(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned()?;
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    Some(match method {
        "initialize" => success_response(id, initialize_result()),
        "tools/list" => success_response(id, json!({ "tools": tool_definitions() })),
        "tools/call" => handle_tools_call(id, &params),
        "ping" => success_response(id, json!({})),
        other => error_response(id, METHOD_NOT_FOUND, &format!("Method not found: {other}")),
    })
}

pub fn tool_definitions() -> Value {
    let repo_prop = json!({
        "type": "string",
        "description": "Path to the repository containing a .orbit/ directory (defaults to the current directory).",
    });
    let db_prop = json!({
        "type": "string",
        "description": "Explicit path to the Orbit DuckDB graph file, overriding .orbit/ auto-detection.",
    });
    let severity_prop = json!({
        "type": "string",
        "enum": ["info", "warning", "critical"],
        "description": "Minimum severity to include in findings (default: info).",
    });

    json!([
        {
            "name": "analyze_dead_code",
            "description": "Find unreferenced definitions in the Orbit Knowledge Graph and return file-level findings.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "detect_circular_dependencies",
            "description": "Detect bidirectional and longer module dependency cycles with reference counts.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "analyze_coupling",
            "description": "Measure module fan-out and flag highly coupled modules.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "detect_architectural_drift",
            "description": "Check boundary rules and report imports that cross allowed architectural layers.",
            "inputSchema": {
                "type": "object",
                "properties": { "repo": repo_prop, "db": db_prop, "severity": severity_prop },
                "additionalProperties": false
            }
        },
        {
            "name": "health_scan",
            "description": "Run the full Orbit Recon report and return either JSON or Markdown output.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": repo_prop,
                    "db": db_prop,
                    "severity": severity_prop,
                    "format": {
                        "type": "string",
                        "enum": ["json", "markdown"],
                        "description": "Report shape returned in the text content.",
                    },
                },
                "additionalProperties": false,
            },
        }
    ])
}

pub fn call_tool(name: &str, args: &Value) -> std::result::Result<Value, ToolError> {
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
    let cfg = load_config(&repo);

    match name {
        "analyze_dead_code" => run_named_check(
            &repo,
            db_override.as_deref(),
            &cfg,
            "dead_code",
            min_severity,
        ),
        "detect_circular_dependencies" => run_named_check(
            &repo,
            db_override.as_deref(),
            &cfg,
            "circular_dependencies",
            min_severity,
        ),
        "analyze_coupling" => run_named_check(
            &repo,
            db_override.as_deref(),
            &cfg,
            "coupling",
            min_severity,
        ),
        "detect_architectural_drift" => run_named_check(
            &repo,
            db_override.as_deref(),
            &cfg,
            "architectural_drift",
            min_severity,
        ),
        "health_scan" => run_health_scan(&repo, db_override.as_deref(), &cfg, min_severity, args),
        other => Err(ToolError::UnknownTool(format!("Unknown tool: {other}"))),
    }
}

fn run_named_check(
    repo: &Path,
    db_override: Option<&Path>,
    cfg: &crate::config::Config,
    check: &str,
    min_severity: Severity,
) -> std::result::Result<Value, ToolError> {
    let db_path = resolve_db(repo, db_override).map_err(|e| ToolError::Execution(e.to_string()))?;
    let conn = crate::open_graph(&db_path).map_err(|e| ToolError::Execution(e.to_string()))?;
    let findings = crate::run_checks(&conn, cfg, &[check.to_string()], min_severity)
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    Ok(json!({
        "check": check,
        "count": findings.len(),
        "findings": findings,
    }))
}

fn run_health_scan(
    repo: &Path,
    db_override: Option<&Path>,
    cfg: &crate::config::Config,
    min_severity: Severity,
    args: &Value,
) -> std::result::Result<Value, ToolError> {
    let format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("json");
    let checks: Vec<String> = crate::ALL_CHECKS.iter().map(|s| s.to_string()).collect();
    let report = crate::scan_repo(repo, db_override, cfg, &checks, min_severity)
        .map_err(|e| ToolError::Execution(e.to_string()))?;

    let payload = match format {
        "markdown" => report.to_markdown(),
        _ => serde_json::to_string_pretty(&report)
            .map_err(|e| ToolError::Execution(e.to_string()))?,
    };

    Ok(json!({"report": payload, "format": format}))
}

fn handle_tools_call(id: Value, params: &Value) -> Value {
    let name = match params.get("name").and_then(|n| n.as_str()) {
        Some(n) => n,
        None => return error_response(id, INVALID_PARAMS, "Missing tool `name`."),
    };
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match call_tool(name, &args) {
        Ok(payload) => success_response(
            id,
            json!({
                "content": [ { "type": "text", "text": payload.to_string() } ],
                "isError": false,
            }),
        ),
        Err(ToolError::Execution(msg)) => success_response(
            id,
            json!({
                "content": [ { "type": "text", "text": msg } ],
                "isError": true,
            }),
        ),
        Err(ToolError::UnknownTool(msg)) => error_response(id, METHOD_NOT_FOUND, &msg),
        Err(ToolError::BadParams(msg)) => error_response(id, INVALID_PARAMS, &msg),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
    })
}

fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut content_length = None;
    let mut line = String::new();

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                let parsed = value.trim().parse::<usize>().map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid Content-Length: {e}"),
                    )
                })?;
                content_length = Some(parsed);
            }
        }
    }

    let len = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    let mut buf = vec![0_u8; len];
    reader.read_exact(&mut buf)?;
    let value = serde_json::from_slice(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    Ok(Some(value))
}

fn write_message<W: Write>(writer: &mut W, response: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(response)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

fn resolve_db(repo: &Path, db_override: Option<&Path>) -> Result<PathBuf> {
    match db_override {
        Some(p) => Ok(p.to_path_buf()),
        None => crate::find_duckdb_path(repo),
    }
}

fn load_config(repo: &Path) -> crate::config::Config {
    let default_config = repo.join(".orbit-recon.yml");
    if default_config.exists() {
        crate::config::Config::from_file(&default_config).unwrap_or_default()
    } else {
        crate::config::Config::default()
    }
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    let _ = INTERNAL_ERROR;
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
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
            "params": {},
        });
        let resp = handle_message(&req).expect("initialize must respond");
        assert_eq!(resp["id"], json!(1));
        assert_eq!(resp["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
        assert_eq!(resp["result"]["serverInfo"]["name"], json!(SERVER_NAME));
    }

    #[test]
    fn notification_produces_no_response() {
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        assert!(handle_message(&notif).is_none());
    }

    #[test]
    fn tools_list_returns_the_expected_tools() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {},
        });
        let resp = handle_message(&req).expect("tools/list must respond");
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

        assert_eq!(names.len(), 5);
        for expected in [
            "analyze_dead_code",
            "detect_circular_dependencies",
            "analyze_coupling",
            "detect_architectural_drift",
            "health_scan",
        ] {
            assert!(names.contains(&expected), "missing tool `{expected}`");
        }
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "does/not/exist",
            "params": {},
        });
        let resp = handle_message(&req).expect("must respond");
        assert_eq!(resp["error"]["code"], json!(METHOD_NOT_FOUND));
    }

    #[test]
    fn unknown_tool_is_reported() {
        let err = call_tool("not_a_tool", &json!({}));
        assert!(matches!(err, Err(ToolError::UnknownTool(_))));
    }

    #[test]
    fn tools_call_on_real_graph_returns_findings() {
        let dir = std::env::temp_dir().join(format!("orbit-recon-mcp-test-{}", std::process::id()));
        let orbit_dir = dir.join(".orbit");
        std::fs::create_dir_all(&orbit_dir).unwrap();
        let db_path = orbit_dir.join("orbit.duckdb");

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

        let req = json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/call",
            "params": {
                "name": "analyze_dead_code",
                "arguments": { "repo": dir.to_str().unwrap() },
            },
        });
        let resp = handle_message(&req).expect("tools/call must respond");
        assert_eq!(resp["id"], json!(42));
        assert_eq!(resp["result"]["isError"], json!(false));

        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .expect("text content");
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["check"], json!("dead_code"));

        let findings = parsed["findings"].as_array().unwrap();
        let names: Vec<&str> = findings
            .iter()
            .filter_map(|f| f["location"]["name"].as_str())
            .collect();
        assert!(names.contains(&"orphan_fn"));
        assert!(!names.contains(&"used_fn"));

        let scan = call_tool("health_scan", &json!({ "repo": dir.to_str().unwrap() }))
            .expect("health_scan must succeed");
        assert!(scan["report"].is_string());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
