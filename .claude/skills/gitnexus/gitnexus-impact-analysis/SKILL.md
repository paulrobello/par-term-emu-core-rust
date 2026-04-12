---
name: gitnexus-impact-analysis
description: "Use when the user wants to know what will break if they change something, or needs safety analysis before editing code. Examples: \"Is it safe to change X?\", \"What depends on this?\", \"What will break?\""
---

# Impact Analysis with GitNexus

> **IMPORTANT — How to use GitNexus**: GitNexus is a standalone CLI tool. Run it directly
> via `gitnexus <command>` in the Bash tool. Do **NOT** use `mcpl call gitnexus ...`.

> **Multi-repo note**: Always pass `--repo <name>` to every command to avoid
> "multiple repositories" errors.

## When to Use

- "Is it safe to change this function?"
- "What will break if I modify X?"
- "Show me the blast radius"
- "Who uses this code?"
- Before making non-trivial code changes
- Before committing — to understand what your changes affect

## Workflow

```
1. gitnexus impact "X" --direction upstream --repo <name>  → What depends on this
2. gitnexus context "X" --repo <name>                      → See affected execution flows
3. gitnexus detect-changes --repo <name>                   → Map current git changes to affected flows
4. Assess risk and report to user
```

> If "Index is stale" → run `gitnexus analyze` in terminal.

## Checklist

```
- [ ] gitnexus impact "X" --direction upstream to find dependents
- [ ] Review d=1 items first (these WILL BREAK)
- [ ] Check high-confidence (>0.8) dependencies
- [ ] gitnexus context to see affected execution flows
- [ ] gitnexus detect-changes for pre-commit check
- [ ] Assess risk level and report to user
```

## Understanding Output

| Depth | Risk Level       | Meaning                  |
| ----- | ---------------- | ------------------------ |
| d=1   | **WILL BREAK**   | Direct callers/importers |
| d=2   | LIKELY AFFECTED  | Indirect dependencies    |
| d=3   | MAY NEED TESTING | Transitive effects       |

## Risk Assessment

| Affected                       | Risk     |
| ------------------------------ | -------- |
| <5 symbols, few processes      | LOW      |
| 5-15 symbols, 2-5 processes    | MEDIUM   |
| >15 symbols or many processes  | HIGH     |
| Critical path (auth, payments) | CRITICAL |

## Commands

**`gitnexus impact`** — the primary tool for symbol blast radius:

```bash
gitnexus impact "validateUser" --direction upstream --repo my-app
# Optional flags: --min-confidence 0.8 --max-depth 3
#
# → d=1 (WILL BREAK):
#   - loginHandler (src/auth/login.ts:42) [CALLS, 100%]
#   - apiMiddleware (src/api/middleware.ts:15) [CALLS, 100%]
#
# → d=2 (LIKELY AFFECTED):
#   - authRouter (src/routes/auth.ts:22) [CALLS, 95%]
```

**`gitnexus detect-changes`** — git-diff based impact analysis:

```bash
gitnexus detect-changes --repo my-app
# → Changed: 5 symbols in 3 files
# → Affected: LoginFlow, TokenRefresh, APIMiddlewarePipeline
# → Risk: MEDIUM
```

## Example: "What breaks if I change validateUser?"

```bash
# 1. Upstream blast radius
gitnexus impact "validateUser" --direction upstream --repo my-app
# → d=1: loginHandler, apiMiddleware (WILL BREAK)
# → d=2: authRouter, sessionManager (LIKELY AFFECTED)

# 2. Confirm affected execution flows
gitnexus context "validateUser" --repo my-app
# → Processes: LoginFlow, TokenRefresh

# 3. Risk: 2 direct callers, 2 processes = MEDIUM
```
