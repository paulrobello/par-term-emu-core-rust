# Repository Guidelines

## Project Structure & Modules
- Core Rust crate in `src/` (`terminal/`, `sixel/`, `pty_session.rs`, `html_export.rs`) with Python bindings in `src/python_bindings` and packaged Python shim under `python/`.
- Integration and unit tests: Rust tests co-located in `src/tests`, Python tests in `tests/`. Example scripts in `examples/` (basic, PTY, streaming).
- Docs and references live in `docs/`; helper scripts in `scripts/`; optional shell/terminfo add‑ons in `shell_integration/` and `terminfo/`.

## Build, Test, and Development Commands
- `make setup-venv` → create `.venv` with all dev deps (uv + maturin) before building.
- `make build` / `make build-release` → develop/install the Rust crate (debug vs release) via maturin.
- `make build-streaming` or `make dev-streaming` → enable the `streaming` feature; pair with `make examples-streaming` to run the WebSocket demo.
- `make test` → full Rust + Python suite; `make test-rust` runs `cargo test --lib --no-default-features --features pyo3/auto-initialize`; `make test-python` runs `pytest tests/ -v` via uv.
- `make fmt`, `make lint`, `make check` → format, clippy+fmt with autofix, and `cargo check`; `make checkall` runs format, lint, typecheck, and both test suites.
- Web frontend (Next.js) in `web-terminal-frontend/`: `make web-install`, `make web-dev`, `make web-build`, `make web-start`.

## Coding Style & Naming Conventions
- Rust: keep `rustfmt` clean; prefer explicit enums/structs; use `?` over `unwrap`; feature flags kept minimal (`streaming`).
- Python: Ruff formatting (`ruff format`) and lint (`ruff check --fix`) plus `pyright` types; modules and tests in `snake_case`.
- Naming: commits and PR titles use imperative, present-tense; prefer `feat:`, `fix:`, `chore:` prefixes seen in git log.

## Testing Guidelines
- Default expectation: `make test` green before pushing. For quick checks, run `make test-rust` when touching core and `make test-python` for bindings or examples.
- Add regression cases beside the touched code: Rust tests under `src/tests`, Python tests under `tests/` named `test_*.py`.
- Streaming or surface changes should be validated with `make examples-streaming` to ensure server/client handoff still works.

## Commit & Pull Request Guidelines
- Commits: small, focused, conventional prefix (`feat: improve cursor wrap`, `fix: reset kitty flags`).
- PRs: include scope, behavior change, and risk notes; list test commands executed; attach screenshots for web UI tweaks; link related issues/CHANGELOG entry when user-facing.
- Keep PRs draft until `make checkall` (or at least format + relevant tests) have run locally.

## Security & Configuration Tips
- Avoid adding default-privileged PTY or shell hooks; keep environment overrides explicit in examples.
- If adjusting terminfo or shell integration, document required exports (`TERM=par-term`, `COLORTERM=truecolor`) and avoid enabling system-wide changes by default.

## Agent Notes
- Use `Makefile` targets instead of ad-hoc cargo/pytest invocations to stay consistent with tooling (uv, maturin, feature flags).
- Clean builds with `make clean`; avoid removing user-created artifacts outside `target/`, `.next/`, and build caches.

<!-- gitnexus:start -->
# GitNexus MCP

This project is indexed by GitNexus as **par-term-emu-core-rust** (6912 symbols, 24797 relationships, 300 execution flows).

GitNexus provides a knowledge graph over this codebase — call chains, blast radius, execution flows, and semantic search.

## Always Start Here

For any task involving code understanding, debugging, impact analysis, or refactoring, you must:

1. **Read `gitnexus://repo/{name}/context`** — codebase overview + check index freshness
2. **Match your task to a skill below** and **read that skill file**
3. **Follow the skill's workflow and checklist**

> If step 1 warns the index is stale, run `npx gitnexus analyze` in the terminal first.

## Skills

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/refactoring/SKILL.md` |

## Tools Reference

| Tool | What it gives you |
|------|-------------------|
| `query` | Process-grouped code intelligence — execution flows related to a concept |
| `context` | 360-degree symbol view — categorized refs, processes it participates in |
| `impact` | Symbol blast radius — what breaks at depth 1/2/3 with confidence |
| `detect_changes` | Git-diff impact — what do your current changes affect |
| `rename` | Multi-file coordinated rename with confidence-tagged edits |
| `cypher` | Raw graph queries (read `gitnexus://repo/{name}/schema` first) |
| `list_repos` | Discover indexed repos |

## Resources Reference

Lightweight reads (~100-500 tokens) for navigation:

| Resource | Content |
|----------|---------|
| `gitnexus://repo/{name}/context` | Stats, staleness check |
| `gitnexus://repo/{name}/clusters` | All functional areas with cohesion scores |
| `gitnexus://repo/{name}/cluster/{clusterName}` | Area members |
| `gitnexus://repo/{name}/processes` | All execution flows |
| `gitnexus://repo/{name}/process/{processName}` | Step-by-step trace |
| `gitnexus://repo/{name}/schema` | Graph schema for Cypher |

## Graph Schema

**Nodes:** File, Function, Class, Interface, Method, Community, Process
**Edges (via CodeRelation.type):** CALLS, IMPORTS, EXTENDS, IMPLEMENTS, DEFINES, MEMBER_OF, STEP_IN_PROCESS

```cypher
MATCH (caller)-[:CodeRelation {type: 'CALLS'}]->(f:Function {name: "myFunc"})
RETURN caller.name, caller.filePath
```

<!-- gitnexus:end -->
