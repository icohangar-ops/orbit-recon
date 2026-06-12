# Devpost Submission — Orbit Recon

## Project Name
Orbit Recon

## Elevator Pitch
Rust CLI + Duo Agent skill that reads the GitLab Orbit Knowledge Graph to detect dead code, circular dependencies, coupling issues, and architectural drift — then generates structured health reports with CI integration.

---

## Description (What it does)

Orbit Recon is a codebase health analysis tool that turns GitLab Orbit's Knowledge Graph into an automated code quality auditor. The Orbit Knowledge Graph indexes your entire repository into a property graph stored in DuckDB — every definition, every cross-file reference, every module relationship. Orbit Recon reads that graph and runs four targeted analyses that most teams either do manually or don't do at all.

**Dead Code Detection** finds functions, classes, methods, structs, and enums that have zero incoming references anywhere in the codebase. These are definitions that nothing calls, nothing imports, nothing uses. They accumulate silently over years of development, bloating codebases and confusing new developers. Orbit Recon classifies them by severity — public definitions with no references are flagged as critical since they represent dead public API surface, while private helpers get a warning since they might be used via reflection or plugin systems.

**Circular Dependency Detection** builds a module-level dependency graph from the raw cross-file references in the Orbit graph, then runs bidirectional and multi-module cycle detection. It finds not just A-depends-on-B-and-B-depends-on-A pairs, but also longer cycles like A→B→C→A using iterative DFS. Each cycle is scored by the total number of cross-references involved, with high-volume cycles flagged as critical because they represent deeply entangled module boundaries that make changes risky and unpredictable.

**Module Coupling Analysis** measures the fan-out metric for every module in the codebase — how many other modules each module directly depends on. High fan-out is one of the most reliable predictors of change-prone code: when a module depends on 15 other modules, any change to any one of those 15 can break it. Orbit Recon lists every dependency for high-coupling modules so developers know exactly where the coupling is coming from, making targeted refactoring possible.

**Architectural Drift Detection** enforces layer boundaries that teams define in a simple YAML config. In a typical layered architecture, the domain layer should only import from itself and shared types — it should never reach into infrastructure or presentation. But over time, developers take shortcuts, and suddenly your domain code has database driver imports and HTTP client references. Orbit Recon checks every cross-file reference against the configured boundaries and flags violations, catching architectural erosion before it becomes structural.

All findings are output as structured reports in Markdown (for developer review), JSON (for CI/CD and GitLab Code Quality integration), or YAML (for pipeline configuration). In CI mode, the binary exits with code 1 if any critical findings exist, making it a merge request gate.

---

## How we built it (How it works)

Orbit Recon is written in Rust and built on three pillars: direct DuckDB graph access, graph-query-based analysis, and the Agent Skills specification.

**Architecture:** The tool reads the DuckDB file that Orbit Local produces when you run `orbit index .` on a repository. Orbit parses the source code, extracts every definition and cross-file reference, and writes a property graph. We chose to read this DuckDB file directly using the `duckdb` Rust crate with bundled features, which means Orbit Recon works completely offline with no network calls, no API keys, and no server to configure. The entire analysis runs locally against the local graph snapshot.

**Query Engine:** Each of the four analysis checks is implemented as a separate module under `src/queries/`. Each module prepares SQL queries against the DuckDB graph, executes them via the DuckDB prepared statement API, and maps the result rows into typed Finding structs. The queries use the same property-graph patterns that Orbit itself uses: definitions as nodes, references as edges, and modules as aggregated node groups. For dead code detection, we look for definition nodes with zero in-degree on the REFERENCES edge type. For circular dependencies, we build an adjacency list in Rust and run DFS to find cycles of any length. For coupling, we aggregate the adjacency list into fan-out counts per module. For architectural drift, we match file paths against user-defined boundary rules using glob patterns and validate each cross-file reference.

**Configuration System:** The `.orbit-recon.yml` config file lets teams customize everything: define their own architecture boundaries with glob patterns, adjust severity thresholds to match their tolerance, and set ignore patterns to suppress false positives from test code, generated files, or known entry points. The config module uses serde_yaml for parsing and the `glob` crate for pattern matching, with sensible defaults that work out of the box for standard layered architectures.

**Report Generation:** The report module produces three output formats. The Markdown report includes a summary table, findings grouped by category with severity badges, and a prioritized recommendations section. The JSON report follows a schema compatible with GitLab Code Quality reports, so it can be used directly as a CI artifact. The YAML format is provided for teams that prefer YAML-based pipeline configurations.

**Agent Platform Integration:** The Duo Agent Platform skill is defined in `.agents/skills/orbit-recon/SKILL.md` following the Agent Skills specification. This means any AI agent on the GitLab Duo platform can invoke Orbit Recon by simply asking to "run a health scan" — the agent reads the SKILL.md, knows the prerequisite commands, executes the binary, and presents the results. The `AGENTS.md` file provides broader agent context including the graph query patterns used, integration examples, and CI/CD configuration snippets.

**Rust was chosen intentionally.** Orbit itself is a Rust service (the Knowledge Graph server is written in Rust with gRPC/HTTP). Using the same language means Orbit Recon naturally fits the Orbit ecosystem, can share types and patterns, and compiles to a single static binary with no runtime dependencies. The `duckdb` crate with bundled feature means users don't need to install DuckDB separately — everything is compiled in.

---

## Challenges we ran into

**Schema Discovery:** The Orbit Knowledge Graph's DuckDB schema is not yet fully documented for third-party consumers. The table names, column names, and edge representations can vary between Orbit Local and Orbit Remote. We solved this by building a schema discovery module (`src/queries/mod.rs`) that queries `information_schema.tables` and `information_schema.columns` at runtime to adapt to whatever schema version the graph uses. The queries use the most likely table/column names based on the Orbit source code, with graceful fallback when tables or columns don't exist.

**False Positives in Dead Code Detection:** Not every unreferenced definition is actually dead code. Entry points like `main()`, test functions, reflection-based plugin registrations, and barrel files (index.ts, mod.rs) all appear as unreferenced in the graph but are actually in use. We addressed this with a multi-layer filtering strategy: first, skip known entry-point filenames; second, check naming conventions (test_, _test, Spec suffixes); third, allow users to add glob-based ignore patterns in the config. The severity classification also helps — uncertain cases are downgraded to Info rather than Warning or Critical.

**Module Extraction from File Paths:** The Orbit graph stores file paths, not module names. Converting a file path like `src/infrastructure/database/postgres_connection.rs` into a meaningful module name like `infrastructure` requires assumptions about project structure. Our `extract_module()` function handles the common case (first directory under `src/`), but monorepos, flat structures, and non-standard layouts may need customization. We made this configurable as a future enhancement path.

**Longer Cycle Detection Performance:** Detecting cycles beyond simple A↔B pairs requires graph traversal, which can be expensive on large monorepos with tens of thousands of nodes. Our DFS-based cycle finder includes cycle normalization (rotating to the lexicographically smallest element) and deduplication to avoid reporting the same cycle multiple times. For very large codebases, the `--only` flag lets users run individual checks to keep analysis time reasonable.

---

## Accomplishments that we're proud of

- **Zero-config analysis** — Orbit Recon works out of the box with sensible defaults for standard layered architectures. Just `orbit index .` then `orbit-recon` and you have a full health report.
- **Three output formats** — Markdown for humans, JSON for machines/CI, YAML for pipeline configuration. The JSON schema is compatible with GitLab Code Quality reports.
- **Agent Skills specification compliance** — The SKILL.md follows the emerging standard, making Orbit Recon installable via `glab skills install` and invocable by any compatible AI agent.
- **CI/CD first-class support** — The `--ci` flag and `.gitlab-ci.yml` template make it trivial to add code health as a merge request gate.
- **Rust + DuckDB = single binary, no dependencies** — Users don't install DuckDB, don't configure a server, don't set API keys. `cargo install orbit-recon` and go.

---

## What we learned

- The Orbit Knowledge Graph is a powerful but underexplored surface for developer tools. Most Orbit usage today focuses on code navigation and AI context, but the property graph structure is equally valuable for static analysis, architectural enforcement, and code health monitoring.
- The Agent Skills specification is still emerging, and there's a gap between what the spec defines and what agents can actually execute. Our SKILL.md bridges this by including explicit shell commands that the agent can run, not just descriptions.
- Rust's DuckDB ecosystem is mature enough for production tooling. The bundled feature means no system dependency on DuckDB, and the prepared statement API is fast enough for graphs with tens of thousands of nodes.
- Architecture boundaries are one of the most requested but least automated aspects of code review. Teams know their code is drifting but have no automated way to detect it. Orbit Recon fills this gap by making boundary enforcement a query against the graph.

---

## What's next for Orbit Recon

1. **Schema adapter for Orbit Remote** — Currently reads Orbit Local's DuckDB file. Next step is an adapter that queries Orbit Remote via gRPC/HTTP, enabling analysis of GitLab-hosted repositories without local clones.
2. **Historical tracking** — Run Orbit Recon on each commit and track finding counts over time. Plot trends in a dashboard to see if code health is improving or degrading. Store results in a SQLite database alongside the DuckDB graph.
3. **Diff mode** — Compare two Orbit graph snapshots (before and after a merge request) and report only new findings. This makes MR reviews focused and actionable.
4. **VS Code extension** — Inline annotations for drift violations and dead code, powered by the Orbit Recon binary running in the background.
5. **AI Catalog publication** — Submit to the GitLab AI Catalog so any Duo Agent Platform user can install it with one click.
6. **Multi-language optimization** — Tune the module extraction and ignore patterns for Python, Go, Java, and Ruby, in addition to the current TypeScript/Rust defaults.
7. **Web dashboard** — A simple HTML report with an interactive dependency graph visualization using D3.js or vis.js, so teams can visually explore cycles and coupling.

---

## Try it out

```bash
# Install
cargo install orbit-recon

# Index your repo with Orbit Local
glab orbit setup
orbit index /path/to/your/repo

# Run full analysis
orbit-recon --repo /path/to/your/repo

# CI mode
orbit-recon --repo . --format json --output report.json --ci
```

## Built with

- Rust 2021 edition
- DuckDB (via `duckdb` crate, bundled)
- clap v4 (CLI)
- serde + serde_json + serde_yaml (serialization)
- glob (pattern matching)
- GitLab Orbit Knowledge Graph (data source)
- Agent Skills specification (skill interface)

## Links

- GitHub: https://github.com/icohangar-ops/orbit-recon
- Codeberg: https://codeberg.org/cubiczan/orbit-recon