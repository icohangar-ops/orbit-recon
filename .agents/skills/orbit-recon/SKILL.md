---
name: orbit-recon
version: 1.0.0
description: >
  Automated codebase health analysis using GitLab Orbit Knowledge Graph.
  Detects dead code, circular dependencies, architectural drift, and module
  coupling issues. Run `orbit-recon` in your repo to get a structured report.
agent: orbit-recon
tools:
  - shell
  - file_read
  - file_write
---

# Orbit Recon Skill

## Purpose

Analyze codebase health using the GitLab Orbit Knowledge Graph. This skill
uses the `orbit-recon` Rust binary to read the DuckDB graph produced by
`orbit index` and generate structured health reports.

## Trigger Conditions

Invoke this skill when the user asks to:

- "Analyze codebase health" / "Run a health scan"
- "Find dead code" / "Find unused functions"
- "Detect circular dependencies"
- "Check module coupling" / "Measure fan-out"
- "Detect architectural drift"
- "Run Orbit Recon"
- "Generate a code health report"

## Prerequisites

1. **Orbit Local** must be installed: `glab orbit setup`
2. **Repository must be indexed**: `orbit index .`
3. **orbit-recon binary** must be on PATH (install via `cargo install orbit-recon`)

## Instructions

### Step 1: Verify Setup

```bash
# Check Orbit CLI
orbit --version

# Check orbit-recon is available
orbit-recon --version

# Verify graph exists
ls .orbit/
```

If the graph doesn't exist, index first:

```bash
orbit index .
```

### Step 2: Run Analysis

Run the full analysis with default settings:

```bash
orbit-recon --repo .
```

For JSON output (useful for CI/CD):

```bash
orbit-recon --repo . --format json --output orbit-report.json
```

For CI mode (exits with code 1 on critical findings):

```bash
orbit-recon --repo . --format json --output orbit-report.json --ci
```

Run only specific checks:

```bash
# Only dead code + circular deps
orbit-recon --repo . --only dead_code,circular_dependencies

# Only coupling analysis
orbit-recon --repo . --only coupling
```

Filter by minimum severity:

```bash
# Only show warnings and critical
orbit-recon --repo . --severity warning

# Only show critical
orbit-recon --repo . --severity critical
```

### Step 3: Review & Present Report

After running, read the generated report and present the key findings to the user:

1. Summarize the total findings by category and severity
2. Highlight any critical issues first
3. Provide actionable next steps based on the recommendations section

### Step 4: Configuration (Optional)

If the user wants custom architecture boundaries, help them create an `.orbit-recon.yml`:

```yaml
boundaries:
  - name: "Domain Layer"
    pattern: "src/domain/**"
    allowed_imports:
      - "src/domain/**"
      - "src/types/**"

thresholds:
  dead_code_warning: 20
  dead_code_critical: 50
  coupling_warning_fan_out: 8
  coupling_critical_fan_out: 15

ignore:
  dead_code:
    - "test/**"
    - "**/*.test.*"
  drift:
    - "src/generated/**"
```

### Step 5: CI/CD Integration

To add Orbit Recon to GitLab CI, add a job to `.gitlab-ci.yml`:

```yaml
orbit-recon-check:
  stage: test
  before_script:
    - glab orbit setup
    - orbit index .
    - cargo install orbit-recon
  script:
    - orbit-recon --repo . --format json --output orbit-report.json --ci
  artifacts:
    reports:
      codequality: orbit-report.json
    paths:
      - orbit-report.json
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
```

## Output Formats

### Markdown (default)

Human-readable report with summary table, findings grouped by category, and
recommendations. Ideal for developer review.

### JSON

Machine-readable report matching the GitLab Code Quality report format.
Includes `version`, `timestamp`, `repository`, `graph_stats`, and `findings`
array with severity, category, location, description, and remediation.

### YAML

Same structure as JSON but in YAML format. Useful for configuration-driven
workflows.

## Error Handling

- If `orbit-recon` is not found: instruct user to `cargo install orbit-recon`
- If `.orbit/` is missing: run `orbit index .`
- If no findings: report that the codebase is healthy
- If critical findings in CI mode: the binary exits with code 1

## Tips

- Run before major refactors to establish a baseline
- Use JSON output for CI/CD integration
- Use `--only dead_code` for quick unused-code sweeps
- Use `--severity critical` to focus on the most impactful issues first
- Re-index after significant changes for accurate results