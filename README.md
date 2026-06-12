<p align="center">
  <img src="orbit-recon-icon-512.png" alt="Orbit Recon" width="128" height="128">
</p>

<h1 align="center">Orbit Recon</h1>

<p align="center">
  <strong>Codebase health analysis powered by the GitLab Orbit Knowledge Graph</strong>
</p>

<p align="center">
  <a href="https://github.com/Cubiczan/orbit-recon"><img src="https://img.shields.io/badge/GitHub-Cubiczan%2Forbit--recon-181717?logo=github" alt="GitHub"></a>
  <a href="https://codeberg.org/cubiczan/orbit-recon"><img src="https://img.shields.io/badge/Codeberg-cubiczan%2Forbit--recon-2185d0?logo=codeberg" alt="Codeberg"></a>
  <img src="https://img.shields.io/badge/Rust-1.80%2B-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License">
  <a href="https://gitlab-transcend.devpost.com"><img src="https://img.shields.io/badge/GitLab-Transcend%20Hackathon-FC6D26?logo=gitlab" alt="Hackathon"></a>
</p>

---

## What is Orbit Recon?

Orbit Recon reads the DuckDB property graph produced by [GitLab Orbit](https://about.gitlab.com/gitlab-orbit) and runs four automated health checks that most teams either do manually or don't do at all:

| Check | What it finds | Why it matters |
|---|---|---|
| **Dead Code** | Functions, classes, methods with zero references anywhere in the codebase | Bloated codebases, confused developers, dead public API surface |
| **Circular Dependencies** | A↔B and multi-module cycles (A→B→C→A) | Unpredictable changes, tangled modules, risky refactors |
| **Module Coupling** | High fan-out modules that depend on too many other modules | Change-prone code — any dependency change breaks them |
| **Architectural Drift** | Layer boundary violations (domain importing infrastructure) | Silent erosion of architecture over time |

It outputs structured reports in **Markdown** (developer review), **JSON** (CI/CD + GitLab Code Quality), or **YAML** (pipeline config). In CI mode, it exits with code 1 on critical findings — a merge request gate.

## Built for the GitLab Transcend Hackathon — Showcase Track

Orbit Recon is a showcase submission demonstrating how the Orbit Knowledge Graph can be consumed by custom tools and AI agents to deliver real developer value beyond code navigation. It includes both a **Rust CLI binary** and a **GitLab Duo Agent Platform skill** so AI agents can orchestrate health scans.

---

## How It Works

```
┌─────────────┐     orbit index      ┌──────────────────┐
│  Repository  │ ──────────────────>  │  .orbit/          │
│  (source)    │                      │  orbit.duckdb     │
└─────────────┘                      └────────┬─────────┘
                                              │
                                     orbit-recon reads
                                     DuckDB directly
                                     (no server, no API)
                                              │
                                     ┌────────▼─────────┐
                                     │  Orbit Recon      │
                                     │  (Rust binary)    │
                                     │                   │
                                     │  ▸ Dead code      │
                                     │  ▸ Cycles (2+ mod)│
                                     │  ▸ Coupling       │
                                     │  ▸ Drift          │
                                     └────────┬─────────┘
                                              │
                               ┌──────────────┼──────────────┐
                               ▼              ▼              ▼
                        ┌────────────┐ ┌───────────┐ ┌──────────┐
                        │  Markdown  │ │   JSON    │ │   YAML   │
                        │  (review)  │ │ (CI/CD)   │ │ (config) │
                        └────────────┘ └───────────┘ └──────────┘
```

**Key design decisions:**

- **Direct DuckDB access** — no server, no API keys, no network. The `duckdb` Rust crate with the `bundled` feature compiles DuckDB into the binary. Everything runs offline.
- **Rust** — same language as Orbit itself. Single static binary, zero runtime dependencies, memory-safe, fast. `cargo install orbit-recon` and go.
- **Agent Skills specification** — the `SKILL.md` follows the emerging standard so any compatible AI agent (Duo, Claude Code, Codex, Gemini CLI) can invoke it.
- **Schema discovery** — Orbit's DuckDB schema is still evolving. Orbit Recon queries `information_schema` at runtime to adapt to whatever table/column names the graph uses.

---

## Quick Start

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (1.80+)
- [GitLab CLI (`glab`)](https://gitlab.com/gitlab-org/cli) with Orbit extension, or the `orbit` binary directly

### Install

```bash
# From crates.io (when published)
cargo install orbit-recon

# Or build from source
git clone https://github.com/Cubiczan/orbit-recon.git
cd orbit-recon
cargo build --release
# Binary at target/release/orbit-recon
```

### Run

```bash
# 1. Index your repository with Orbit Local
cd /path/to/your/repo
orbit index .

# 2. Run the full analysis
orbit-recon

# 3. Get JSON output (for CI/CD)
orbit-recon --format json --output report.json

# 4. CI mode — exits with code 1 on critical findings
orbit-recon --format json --output report.json --ci
```

### Install as a Duo Agent Skill

```bash
# Install the skill into your project
glab skills install orbit-recon
```

Then in GitLab Duo, just ask: **"Run an Orbit Recon health scan on this codebase."**

---

## CLI Reference

```
orbit-recon [OPTIONS]

Options:
  -r, --repo <PATH>          Repository path (contains .orbit/)       [default: .]
  -d, --db <PATH>            DuckDB file path (overrides auto-detection)
  -f, --format <FORMAT>      Output format: json, markdown, yaml       [default: markdown]
  -o, --output <PATH>        Output file path (stdout if omitted)
  -c, --config <PATH>        Config file path                          [default: .orbit-recon.yml]
      --only <CHECKS>        Only run specific checks (comma-separated)
                             Options: dead_code, circular_dependencies, coupling,
                                      architectural_drift
  -s, --severity <LEVEL>     Minimum severity to report                [default: info]
                             Options: info, warning, critical
      --ci                   CI mode: exit code 1 if critical findings exist
  -h, --help                 Show help
  -V, --version              Show version
```

### Examples

```bash
# Full analysis, Markdown to stdout
orbit-recon --repo /path/to/project

# Only check for dead code and circular deps
orbit-recon --only dead_code,circular_dependencies

# Only critical findings, JSON output for CI
orbit-recon --severity critical --format json --output report.json --ci

# Custom config file
orbit-recon --config /path/to/custom-rules.yml

# Point to a specific DuckDB file
orbit-recon --db /custom/path/orbit.duckdb
```

---

## The Four Checks

### 1. Dead Code Detection

Finds definitions (functions, methods, classes, structs, enums, traits) that have **zero incoming references** in the entire codebase. These are nodes in the Orbit graph with no incoming `REFERENCES` edges.

**Severity classification:**

| Condition | Severity | Rationale |
|---|---|---|
| Public class/struct/enum with no references | **CRITICAL** | Dead public API surface — likely confuses consumers |
| Private function/method with no references | **WARNING** | Possibly dead, but may be used via reflection |
| Entry points, test functions, barrel files | **INFO** | Known false positives — suppressed by default |

**False positive handling:** Automatically skips `index.ts`, `mod.rs`, `main.rs`, `lib.rs`, and files matching test naming conventions. Additional ignore patterns can be configured in `.orbit-recon.yml`.

### 2. Circular Dependency Detection

Builds a module-level dependency graph from the raw cross-file references in the Orbit graph, then runs cycle detection:

- **Pairwise cycles** (A↔B): Detects bidirectional module dependencies by checking if both A→B and B→A edges exist
- **Multi-module cycles** (A→B→C→A): Uses iterative DFS to find cycles involving 3+ modules
- **Scoring**: Each cycle is scored by the total number of cross-references, with high-volume cycles flagged as critical

**Example output:**

```
CRITICAL `auth` <-> `user` — 34 cross-references
  Chain: auth/service.rs imports user/model.rs, user/repository.rs imports auth/token.rs
  Suggestion: Extract shared code into a third module (dependency inversion principle)
```

### 3. Module Coupling Analysis

Measures the **fan-out metric** for every module — the number of other modules it directly depends on.

**Thresholds (configurable):**

| Fan-out | Severity | Meaning |
|---|---|---|
| < 8 | ✅ Healthy | Normal dependency count |
| 8–14 | ⚠️ Warning | Moderate coupling, worth reviewing |
| ≥ 15 | 🔴 Critical | Highly coupled, change-prone |

Lists every dependency for high-coupling modules so developers know exactly where to focus refactoring efforts.

### 4. Architectural Drift Detection

Validates every cross-file reference against **layer boundary rules** defined in `.orbit-recon.yml`. Each rule specifies:

- A glob pattern matching files in a layer (e.g., `src/domain/**`)
- A list of glob patterns the layer is allowed to import from

When a definition in a governed layer imports something outside its allowed scope, it's flagged as an architectural drift violation.

**Default boundaries** (for standard layered architectures):

| Layer | Allowed to Import |
|---|---|
| `src/domain/**` | `src/domain/**`, `src/types/**`, `src/models/**` |
| `src/application/**` | `src/domain/**`, `src/application/**`, `src/types/**`, `src/models/**` |
| `src/presentation/**` | `src/application/**`, `src/domain/**`, `src/types/**`, `src/models/**` |

---

## Configuration

Create `.orbit-recon.yml` in your repository root. A full example is at [`config/.orbit-recon.example.yml`](config/.orbit-recon.example.yml).

```yaml
# Architecture boundary rules
boundaries:
  - name: "Domain Layer"
    pattern: "src/domain/**"
    allowed_imports:
      - "src/domain/**"
      - "src/types/**"

  - name: "Application Layer"
    pattern: "src/application/**"
    allowed_imports:
      - "src/domain/**"
      - "src/application/**"
      - "src/types/**"

  - name: "Presentation Layer"
    pattern: "src/presentation/**"
    allowed_imports:
      - "src/application/**"
      - "src/domain/**"
      - "src/types/**"

# Severity thresholds
thresholds:
  dead_code_warning: 20        # Total dead code count to escalate to warning
  dead_code_critical: 50       # Total dead code count to escalate to critical
  coupling_warning_fan_out: 8  # Module fan-out for warning
  coupling_critical_fan_out: 15  # Module fan-out for critical

# Ignore patterns (glob syntax)
ignore:
  dead_code:
    - "test/**"
    - "tests/**"
    - "**/*.test.*"
    - "**/*.spec.*"
  drift:
    - "src/generated/**"
    - "**/*.generated.*"
```

---

## CI/CD Integration

### GitLab CI

Add to your `.gitlab-ci.yml`:

```yaml
orbit-recon-check:
  stage: test
  before_script:
    - glab orbit setup
    - orbit index .
    - cargo install orbit-recon
  script:
    - orbit-recon --format json --output orbit-report.json --ci
  artifacts:
    reports:
      codequality: orbit-report.json
    paths:
      - orbit-report.json
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
```

This runs Orbit Recon on every merge request, feeds results into GitLab Code Quality, and blocks the pipeline if critical findings exist.

### GitHub Actions

```yaml
- name: Orbit Recon
  run: |
    orbit index .
    orbit-recon --format json --output report.json --ci
```

---

## GitLab Duo Agent Platform Integration

Orbit Recon includes a skill that follows the [Agent Skills specification](https://docs.gitlab.com/user/duo_agent_platform/customize/agent_skills). This means any compatible AI agent can invoke it.

### Files

| File | Purpose |
|---|---|
| `AGENTS.md` | Agent context: architecture, query patterns, setup, limitations |
| `.agents/skills/orbit-recon/SKILL.md` | Skill definition: triggers, prerequisites, step-by-step instructions, error handling |

### How it works

1. Install the skill: `glab skills install orbit-recon`
2. The agent reads `SKILL.md` and learns the workflow
3. User asks: "Run an Orbit Recon health scan"
4. Agent verifies prerequisites, runs `orbit-recon`, presents findings

The skill is compatible with GitLab Duo Agent Platform, Claude Code, Codex, and Gemini CLI.

---

## Project Structure

```
orbit-recon/
├── .agents/
│   └── skills/
│       └── orbit-recon/
│           └── SKILL.md            # Duo Agent Platform skill definition
├── config/
│   └── .orbit-recon.example.yml   # Example configuration
├── src/
│   ├── main.rs                     # CLI entry point, argument parsing, orchestration
│   ├── config.rs                   # YAML config loader, boundary matching, ignore patterns
│   ├── findings.rs                 # Finding, Severity, Category, Location types
│   ├── report.rs                   # Markdown/JSON/YAML report generation
│   └── queries/
│       ├── mod.rs                  # Module aggregator, graph stats, schema discovery
│       ├── dead_code.rs            # Zero in-degree definition detection
│       ├── circular_deps.rs        # Module cycle detection (pair + multi-module DFS)
│       ├── coupling.rs             # Fan-out metric per module
│       └── drift.rs                # Boundary rule violation detection
├── AGENTS.md                       # Agent platform documentation
├── Cargo.toml                      # Rust dependencies
├── devpost-submission.md           # Devpost hackathon submission content
├── video-script.md                 # 3-minute demo video script
├── .gitlab-ci.yml                  # CI/CD pipeline template
├── .gitignore
├── orbit-recon-icon-512.png        # Project icon
├── devpost-thumbnail.png           # Devpost submission thumbnail
└── README.md                       # This file
```

---

## Tech Stack

| Dependency | Purpose |
|---|---|
| [duckdb](https://crates.io/crates/duckdb) (bundled) | Reads Orbit Knowledge Graph, no system dependency |
| [clap](https://crates.io/crates/clap) v4 | CLI argument parsing with derive macros |
| [serde](https://crates.io/crates/serde) + serde_json + serde_yaml | JSON/YAML report serialization |
| [glob](https://crates.io/crates/glob) | Boundary rule pattern matching |
| [chrono](https://crates.io/crates/chrono) | Timestamp formatting in reports |
| [colored](https://crates.io/crates/colored) | Terminal severity colors (future) |
| [anyhow](https://crates.io/crates/anyhow) + [thiserror](https://crates.io/crates/thiserror) | Error handling |
| [uuid](https://crates.io/crates/uuid) | Unique finding IDs |
| [log](https://crates.io/crates/log) + env_logger | Structured logging |

---

## Roadmap

- [ ] **Orbit Remote adapter** — Query Orbit Remote via gRPC/HTTP for GitLab-hosted repos
- [ ] **Historical tracking** — Track finding counts over time, plot trends
- [ ] **Diff mode** — Compare two graph snapshots, report only new findings per MR
- [ ] **VS Code extension** — Inline annotations for drift and dead code
- [ ] **Web dashboard** — Interactive dependency graph visualization (D3.js / vis.js)
- [ ] **Multi-language tuning** — Optimized defaults for Python, Go, Java, Ruby
- [ ] **AI Catalog** — Submit to GitLab AI Catalog for one-click install

---

## License

MIT — see [LICENSE](LICENSE).

---

## Links

- **GitHub**: https://github.com/Cubiczan/orbit-recon
- **Codeberg**: https://codeberg.org/cubiczan/orbit-recon
- **Hackathon**: https://gitlab-transcend.devpost.com
- **Orbit Docs**: https://docs.gitlab.com/orbit
- **Agent Skills Spec**: https://docs.gitlab.com/user/duo_agent_platform/customize/agent_skills