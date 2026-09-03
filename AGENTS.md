# Orbit Recon Agent

## Overview

Orbit Recon is a specialized agent for the GitLab Duo Agent Platform that leverages GitLab Orbit (the Knowledge Graph) to perform automated codebase health analysis. It detects dead code, circular dependencies, architectural drift, and module coupling issues — then generates actionable health reports directly from the graph data.

## When to Use This Agent

Use Orbit Recon when you need to:

- Identify dead code (unreferenced functions, classes, modules) across a codebase
- Detect circular dependency chains that slow builds and increase complexity
- Track architectural drift — when the actual dependency graph diverges from intended module boundaries
- Measure module coupling and cohesion scores for better boundaries
- Generate a health report before a major refactor or release

## How It Works

1. **Index**: Orbit Local indexes the repository into a local DuckDB database via `orbit index`
2. **Query**: Orbit Recon runs targeted graph queries using the Orbit DSL against the indexed graph
3. **Analyze**: Results are classified into severity categories (critical, warning, info)
4. **Report**: A structured health report is generated with metrics, findings, and remediation suggestions

## Orbit Queries Used

Orbit Recon relies on these core graph query patterns executed against the Orbit Knowledge Graph:

### Dead Code Detection

```
MATCH (d:Definition)
WHERE NOT (d)<-[:REFERENCES]-()
AND d.kind IN ['function', 'class', 'method']
RETURN d.name, d.file, d.kind, d.line
```

Finds definitions that have no incoming reference edges — meaning nothing in the codebase calls or uses them.

### Circular Dependency Detection

```
MATCH path = (a:Module)-[:CONTAINS]->(:Definition)-[:REFERENCES]->(:Definition)<-[:CONTAINS]-(b:Module)
WHERE a.name <> b.name
AND EXISTS {
  MATCH (b:Module)-[:CONTAINS]->(:Definition)-[:REFERENCES]->(:Definition)<-[:CONTAINS]-(a:Module)
}
RETURN DISTINCT a.name AS from_module, b.name AS to_module, COUNT(path) AS cycle_count
ORDER BY cycle_count DESC
```

Identifies module pairs with bidirectional dependencies — a key indicator of coupling problems.

### Module Coupling Analysis

```
MATCH (m:Module)
OPTIONAL MATCH (m)-[:CONTAINS]->(:Definition)-[:REFERENCES]->(:Definition)<-[:CONTAINS]-(dep:Module)
WHERE m.name <> dep.name
WITH m, COLLECT(DISTINCT dep.name) AS dependencies
RETURN m.name, SIZE(dependencies) AS fan_out, dependencies
ORDER BY fan_out DESC
```

Measures the fan-out metric per module — how many other modules each module depends on.

### Architectural Drift Detection

```
MATCH (d:Definition)-[:REFERENCES]->(t:Definition)
WHERE d.file =~ 'src/domain/.*' AND NOT t.file =~ 'src/domain/.*'
RETURN d.name, d.file, t.name, t.file
```

Finds domain-layer definitions that reach outside their intended boundary (e.g., domain code importing infrastructure code).

## Skills Provided

- **orbit-recon**: The main skill for running full codebase health scans. See `.agents/skills/orbit-recon/SKILL.md` for full specification.

## MCP Server

Orbit Recon also ships an **MCP (Model Context Protocol) server** so any
MCP-compatible agent (Claude Desktop, Cursor, GitLab Duo, etc.) can invoke its
health checks directly as tools — no shell orchestration required.

The server is a second binary target, `orbit-recon-mcp`, that speaks
framed JSON-RPC 2.0 over stdio (`Content-Length` headers). It is a thin wrapper
over the same `orbit_recon` library the CLI uses; there is no duplicated
analysis logic. It adds no new dependencies (only `serde_json`, already
present).

Build and run:

```bash
cargo build --release --bin orbit-recon-mcp
./target/release/orbit-recon-mcp        # serves on stdio
# or during development:
cargo run --bin orbit-recon-mcp
```

Each tool reads the Orbit DuckDB graph read-only (auto-detected from the target
repo's `.orbit/` directory, or via an explicit `db` path) and returns findings
as JSON text content.

### Tools

| Tool | Purpose |
|---|---|
| `analyze_dead_code` | Definitions (functions, classes, methods, structs, enums, traits, interfaces) with no incoming reference edges — nothing calls or uses them. |
| `detect_circular_dependencies` | Bidirectional (A↔B) and longer (A→B→C→A) module dependency cycles, with cross-reference counts. |
| `analyze_coupling` | Per-module fan-out metric; flags modules exceeding the configured warning/critical thresholds. |
| `detect_architectural_drift` | Imports that cross configured (or default layered) module boundaries. |
| `health_scan` | Runs all four checks plus graph statistics and returns the full structured report (`json` or `markdown`). |

Common tool inputs (all optional): `repo` (path to the repo containing
`.orbit/`, default `.`), `db` (explicit DuckDB path override), `severity`
(`info` \| `warning` \| `critical` minimum). `health_scan` additionally accepts
`format` (`json` \| `markdown`).

### Example MCP client config (Claude Desktop / Cursor)

```json
{
  "mcpServers": {
    "orbit-recon": {
      "command": "orbit-recon-mcp",
      "args": []
    }
  }
}
```

## Setup

### Prerequisites

- GitLab CLI (`glab`) v2.60+ installed
- Orbit Local CLI installed via `glab orbit setup` or direct binary
- A local clone of the target repository

### Quick Start

```bash
# Install the Orbit Recon skill into your project
glab skills install orbit-recon

# Index your codebase with Orbit Local
orbit index /path/to/your/repo

# Run a full health scan (the skill will guide the agent through queries)
# In GitLab Duo Agent Platform, just ask:
# "Run an Orbit Recon health scan on this codebase"
```

## Output Format

The agent produces a structured report with:

```yaml
orbit_recon_report:
  timestamp: "2026-06-12T10:00:00Z"
  repository: "my-project"
  graph_nodes: 12450
  graph_edges: 38200
  summary:
    dead_code_count: 42
    circular_dependencies: 3
    high_coupling_modules: 5
    architectural_drift_violations: 12
  findings:
    - severity: critical
      category: circular_dependency
      modules: ["auth", "user"]
      description: "..."
      remediation: "..."
    # ...
```

## Integration with CI/CD

Orbit Recon can be integrated into GitLab CI to run on every merge request:

```yaml
orbit-recon-check:
  image: node:20
  before_script:
    - glab orbit setup
    - orbit index .
  script:
    - glab skills run orbit-recon --format=json > orbit-report.json
    - |
      if [ $(jq '.summary.critical_count' orbit-report.json) -gt 0 ]; then
        echo "Critical findings detected. Review the report."
        exit 1
      fi
  artifacts:
    reports:
      codequality: orbit-report.json
```

## Limitations

- Orbit Local indexes code at a point in time; results reflect the last index run
- Dynamic imports, reflection, and runtime registration patterns may produce false positives for dead code detection
- Architectural drift rules are based on file path conventions and may need customization per project
- Large monorepos (>100k nodes) may require increased DuckDB memory limits

## License

MIT
