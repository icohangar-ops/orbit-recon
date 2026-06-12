# Orbit Recon — 3-Minute Demo Video Script

## Target Runtime: ~3 minutes (~450 words spoken + pauses for terminal action)

---

### [0:00 — 0:20] INTRO: The Problem

**[Terminal animation: large codebase, scrolling through hundreds of files]**

**NARRATOR:**
Every codebase accumulates problems over time. Functions that nothing calls.
Modules that depend on fifteen other modules. Domain code that secretly imports
the database driver. Circular dependencies that make changes unpredictable.

Most teams know these problems exist. But finding them requires manual code
review, grep-heavy shell scripts, or expensive commercial tools.

**[Title card: ORBIT RECON]**

**NARRATOR:**
Orbit Recon automates all of this — by reading the GitLab Orbit Knowledge Graph.

---

### [0:20 — 0:50] HOW IT WORKS

**[Screen recording: terminal showing orbit index running]**

**NARRATOR:**
Here's how it works. GitLab Orbit indexes your repository into a property
graph stored in DuckDB. Every function, every class, every cross-file reference
becomes a node or an edge in this graph.

Orbit Recon reads that graph directly. No server. No API keys. No network
calls. It's a single Rust binary that opens the DuckDB file and runs targeted
graph queries.

**[Terminal: orbit-recon --repo . running, output scrolling]**

**NARRATOR:**
One command — `orbit-recon` — and you get a full health report. Dead code.
Circular dependencies. Module coupling. Architectural drift. All classified by
severity with specific remediation suggestions.

---

### [0:50 — 1:30] THE FOUR CHECKS

**[Split screen: four panels, each showing a different finding type]**

**NARRATOR:**
Let me walk through what it finds.

**Dead code.** Orbit Recon queries the graph for definition nodes with zero
incoming references. That's functions, classes, methods — things that exist
but nothing in the codebase uses. It classifies public definitions as critical
since they're dead API surface, and private ones as warnings since they might
be used via reflection.

**Circular dependencies.** It builds a module-level graph from the raw
references and detects cycles — not just simple A-depends-on-B pairs, but
longer chains like A to B to C back to A. Each cycle is scored by the number
of cross-references involved.

**Module coupling.** This is the fan-out metric — how many other modules each
module depends on. Modules with high fan-out are the most change-prone parts of
your codebase. Orbit Recon lists every dependency so you know exactly where to
refactor.

**Architectural drift.** You define your layer boundaries in a simple YAML
config — domain can only import domain and types, application can import domain
but not infrastructure. Orbit Recon checks every cross-file reference against
these rules and catches violations automatically.

---

### [1:30 — 2:10] DUO AGENT PLATFORM INTEGRATION

**[Screen: GitLab Duo chat interface, typing "Run an Orbit Recon health scan"]**

**NARRATOR:**
But here's where it gets really interesting for the hackathon. Orbit Recon
isn't just a CLI — it's also a GitLab Duo Agent Platform skill.

**[Screen: AGENTS.md and SKILL.md files shown]**

**NARRATOR:**
We've defined an AGENTS.md and a SKILL.md following the Agent Skills
specification. This means any AI agent on the Duo platform can invoke Orbit
Recon. A developer just types "run a health scan" and the agent knows the
prerequisites, runs the binary, and presents the findings.

**[Screen: glab skills install orbit-recon]**

**NARRATOR:**
Installation is one command — `glab skills install orbit-recon` — and the
skill is available to every agent working on that project.

---

### [2:10 — 2:40] CI/CD INTEGRATION

**[Screen: .gitlab-ci.yml shown, then a GitLab MR pipeline passing/failing]**

**NARRATOR:**
Orbit Recon also works in CI/CD. The `--ci` flag makes the binary exit with
code 1 if any critical findings exist. Combined with the JSON output format,
it feeds directly into GitLab Code Quality reports.

You add it to your `.gitlab-ci.yml`, and every merge request automatically gets
a code health check. No new dead code, no new cycles, no architectural drift
can be merged without being flagged.

---

### [2:40 — 3:00] CLOSING

**[Screen: GitHub repo, README, all three remotes shown]**

**NARRATOR:**
Orbit Recon is open source, written in Rust, and uses zero external services.
It's a tool that turns the Orbit Knowledge Graph from a code navigation aid
into an automated code quality auditor.

The project is at github.com/Cubiczan/orbit-recon. Try it on your codebase.
Run `orbit index`, then `orbit-recon`, and see what your graph reveals.

**[End card: Orbit Recon logo, GitLab Transcend Hackathon logo, links]**

**NARRATOR:**
Built for the GitLab Transcend Hackathon, Showcase Track.

---

## Production Notes

- **Screen recordings**: Use a medium-sized open-source TypeScript or Rust repo (50-200 files) for demos — small enough to index quickly, large enough to show real findings
- **Terminal font**: JetBrains Mono or Fira Code, 14pt, dark theme
- **Pacing**: Leave 2-3 second pauses after each terminal command completes so viewers can read the output
- **Music**: Lo-fi ambient, barely audible under narration
- **Resolution**: 1920x1080
- **Format**: MP4, H.264, uploaded to YouTube as unlisted for the Devpost link