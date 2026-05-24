# KalamDB Agent Skills Distribution Plan

**Goal:** Create a dedicated sibling repository at `../kalamdb-skills` that becomes the source of truth for KalamDB agent skills, then publish/install those skills in a way that is easy for Codex, Claude Code, OpenCode, GitHub Copilot, and other Agent Skills consumers to adopt.

**Architecture:** Use the open Agent Skills format (`SKILL.md` plus supporting files) as the canonical source. Keep all hand-authored skill content in `../kalamdb-skills`, generate target-specific layouts from that source, and only mirror generated outputs back into this repository when local repo-scoped discovery is important. This keeps the skill content versioned, reusable, and synchronized with KalamDB command and syntax changes instead of hand-maintaining separate copies.

**Why a separate repo:** the skill should evolve like a product surface, not like incidental documentation. A sibling repo gives us versioning, release artifacts, installers, compatibility metadata, and cross-tool packaging without coupling every skill change to backend or UI code churn.

## Target Support Matrix

| Tool | Native install path | Recommended distribution unit | Notes |
| --- | --- | --- | --- |
| Codex | `.agents/skills/<name>/` or user/admin skill directories | Standard skill folder first, Codex plugin later | Codex supports `.agents/skills` directly and can later consume a plugin for curated installs. |
| Claude Code | `.claude/skills/<name>/` or `~/.claude/skills/<name>/` | Standard skill folder | Claude Code follows the Agent Skills standard and adds extra frontmatter features. |
| OpenCode | `.opencode/skills/<name>/`, `.claude/skills/<name>/`, or `.agents/skills/<name>/` | Native OpenCode folder plus compatibility copies | OpenCode can discover native, Claude-compatible, and agent-compatible skill folders. |
| GitHub Copilot / VS Code agent surfaces | `.agents/skills/<name>/` | Standard skill folder | Keep a repo-local mirror only if repo discovery materially improves usage. |
| Other tools | Tool-specific wrapper or exported instruction bundle | Generated adapter from the same canonical content | Favor the standard skill folder where supported; otherwise export instructions from the same source. |

## Skill Feature Set

The initial KalamDB skill should cover the full set of recurring knowledge users currently need to rediscover manually.

### 1. Repository and architecture guidance

- Repo layout and owning areas (`backend/`, `cli/`, `link/`, `ui/`, `pg/`, `docs/`, `benchv2/`)
- Crate ownership boundaries (`kalamdb-core`, `kalamdb-store`, `kalamdb-filestore`, `kalamdb-session`, `kalamdb-tables`, `kalamdb-system`)
- Core architectural rules from `AGENTS.md`
- DataFusion, MVCC, Raft, live query, storage, auth, and job-system boundaries
- Performance-first rules and hot-path constraints

### 2. User-facing syntax and command surfaces

- CLI commands and flags
- SDK entry points, package layout, and versioning rules
- HTTP/API entry points and auth bootstrap expectations
- SQL/DDL/DML syntax that is specific to KalamDB behavior
- Live query, topic, and stream workflows
- Config files, environment variables, and bootstrap commands

### 3. Development workflows

- Backend build/run commands
- `cargo nextest` and smoke/e2e expectations
- UI test/build commands
- SDK build/test/release workflows
- Cluster and local multi-node workflows
- Benchmark workflows and reporting expectations

### 4. Guardrails and review checklists

- Security review checklist
- Documentation update rules
- Generated-code boundaries
- Architecture-doc update requirements
- Do-not-do rules that are easy to regress (for example hot-path SQL rewrites or storage-boundary leaks)

### 5. Troubleshooting and known sharp edges

- Auth/bootstrap failure patterns
- Server lifecycle and smoke-test prerequisites
- Local SDK linking gotchas
- Live query/WebSocket behavior boundaries
- Cluster and port-selection gotchas
- Platform-specific notes where they affect common workflows

### 6. Installation and lifecycle UX

- User-scope install
- Project-scope install
- Update command
- Uninstall command
- Version check command
- Release notes / compatibility notes per skill version

## Source Repository Layout

The sibling repo should be structured so one canonical authoring tree can generate multiple install targets.

```text
../kalamdb-skills/
  README.md
  LICENSE
  package.json
  install.sh
  install.ps1
  skills/
    kalamdb/
      SKILL.md
      references/
        architecture.md
        commands.md
        sql.md
        sdk.md
        testing.md
        operations.md
        guardrails.md
        troubleshooting.md
      examples/
        sql.md
        cli.md
        sdk.md
      scripts/
        doctor.sh
      agents/
        openai.yaml
      manifest.json
  generated/
    agents/skills/kalamdb/
    claude/skills/kalamdb/
    opencode/skills/kalamdb/
    codex-plugin/
  scripts/
    build-targets.mjs
    install.mjs
    verify-targets.mjs
    package-plugin.mjs
  docs/
    support-matrix.md
    release-process.md
```

## Distribution Strategy

### Canonical authoring model

- Write the skill once under `skills/kalamdb/`
- Split large reference material into supporting files so the entry `SKILL.md` stays concise
- Use the same skill name across targets: `kalamdb`
- Keep the trigger description short and keyword-rich so implicit matching works across tools

### Generated targets

- Generate `.agents/skills/kalamdb/` for Codex, OpenCode compatibility, and other Agent Skills consumers
- Generate `.claude/skills/kalamdb/` for Claude Code users who expect the native directory
- Generate `.opencode/skills/kalamdb/` for native OpenCode installs
- Generate Codex metadata under `agents/openai.yaml`
- Optionally generate a Codex plugin bundle once the base skill is stable

### Easy installation UX

Provide three installation paths, all backed by the same generated artifacts:

1. Shell installer for direct installs
   - Example target UX: `curl -fsSL <release-url>/install.sh | bash -s -- --tool claude --scope user`
   - Supports `--tool codex|claude|opencode|agents`, `--scope user|project`, and `--path <dir>`

2. NPM-based installer for cross-platform scripting
   - Example target UX: `npx @kalamdb/skills install --tool codex --scope project`
   - Good default for JS-heavy environments and CI

3. Manual copy/install docs for locked-down environments
   - Copy the generated target directory into the tool’s supported skill location
   - Include verification commands for each tool

### Recommended per-tool install behavior

- Codex:
  - Project install copies `generated/agents/skills/kalamdb` into `.agents/skills/kalamdb`
  - User install copies into `~/.agents/skills/kalamdb`
  - Later phase: publish a Codex plugin for discoverable curated installs

- Claude Code:
  - Project install copies `generated/claude/skills/kalamdb` into `.claude/skills/kalamdb`
  - User install copies into `~/.claude/skills/kalamdb`

- OpenCode:
  - Prefer native install into `.opencode/skills/kalamdb` or `~/.config/opencode/skills/kalamdb`
  - Offer compatibility mode that installs `.agents/skills/kalamdb` for shared multi-tool repos

- GitHub Copilot and other Agent Skills consumers:
  - Install the generated `.agents/skills/kalamdb` target
  - If a tool needs extra metadata later, add another generator instead of forking the content

## Sync Rules With The Main KalamDB Repo

Any KalamDB change that modifies behavior the skill teaches must update the canonical skill content in `../kalamdb-skills`. The update trigger list should include:

- New or changed CLI commands or flags
- SQL syntax changes and new supported statements
- New system tables or changes to their user-facing behavior
- SDK API additions, removals, or rename migrations
- New config keys or environment variables
- New test commands or changes to required validation flow
- Changed operational runbooks (server start, auth bootstrap, cluster scripts, benchmark entry points)
- Any architecture decision that changes where work should happen in the codebase

## Implementation Tasks

### Task 1: Scaffold the sibling repository

**Deliverables:**
- `../kalamdb-skills` repo initialized
- README with support matrix and quick install instructions
- canonical `skills/kalamdb/` tree created
- install/build script placeholders added

**Notes:**
- Keep the repo independent from KalamDB source changes
- Version the repo separately, but record KalamDB compatibility in metadata

### Task 2: Author the canonical KalamDB skill

**Deliverables:**
- `skills/kalamdb/SKILL.md` with concise overview and trigger phrases
- reference files for architecture, commands, SQL, SDKs, testing, operations, guardrails, troubleshooting
- example files that show correct CLI, SQL, and SDK usage

**Authoring rules:**
- `SKILL.md` stays short and points to supporting files
- supporting files are the place for detailed syntax tables and examples
- prefer actionable rules over narrative background

### Task 3: Build target generators

**Deliverables:**
- generator that emits `.agents`, `.claude`, and `.opencode` target trees
- Codex metadata generation (`agents/openai.yaml`)
- optional plugin packaging skeleton for Codex

**Key requirement:**
- target-specific metadata may differ, but the behavioral guidance must come from one canonical source

### Task 4: Build easy installers

**Deliverables:**
- `install.sh`
- `install.ps1`
- `npx` installer entry point
- uninstall/update/version-check commands

**Installer behavior:**
- detect current repo vs user-home scope
- create missing directories
- avoid overwriting user customizations without confirmation or `--force`
- print the exact install location and verification hint

### Task 5: Add verification and compatibility checks

**Deliverables:**
- script that validates required files, frontmatter, name/path consistency, and generated targets
- smoke checks that install into temp directories and confirm expected output paths
- a small matrix test for Codex, Claude Code, and OpenCode target layouts

**Success criteria:**
- generated skill directories are valid for all supported tools
- install commands are idempotent
- version metadata stays aligned with release artifacts

### Task 6: Integrate repo-to-skill maintenance workflow

**Deliverables:**
- documented checklist in KalamDB for when the skill must be updated
- optional helper script in KalamDB that opens the relevant files in `../kalamdb-skills`
- release note template that includes “skill impact” whenever commands or syntax changed

**Policy goal:**
- skill updates happen in the same change window as the product change, not weeks later

### Task 7: Publish and document installation paths

**Deliverables:**
- GitHub Releases or equivalent release artifacts
- README install section with copy-pasteable commands
- per-tool examples for user-scope and project-scope installs
- upgrade notes for breaking changes in skill names or install locations

## Verification Plan

Before calling the work complete, verify:

1. Codex can discover the generated `.agents/skills/kalamdb` target from a repo-local install.
2. Claude Code can discover the generated `.claude/skills/kalamdb` target from both project and user scope.
3. OpenCode can discover either the native `.opencode/skills/kalamdb` target or the compatibility `.agents/skills/kalamdb` target.
4. Installers are idempotent and print deterministic paths.
5. A real KalamDB command/syntax change can be updated in one canonical source and regenerated across all targets.

## Rollout Recommendation

Phase the work so the distribution mechanism stabilizes before the content explodes:

1. Phase 1: Canonical skill + manual install docs + generated `.agents` and `.claude` targets
2. Phase 2: Native `.opencode` target + `npx` installer + verification scripts
3. Phase 3: Codex plugin packaging + release automation + compatibility CI
4. Phase 4: Additional focused KalamDB skills (`sql-debug`, `cluster-ops`, `sdk-release`) if the single root skill becomes too large

## Decision Summary

- Put the source of truth in `../kalamdb-skills`
- Author one canonical KalamDB skill and split details into supporting files
- Generate target-specific layouts instead of hand-maintaining multiple copies
- Ship simple install commands for Codex, Claude Code, OpenCode, and standard `.agents` consumers
- Treat command and syntax changes in KalamDB as mandatory skill-update triggers