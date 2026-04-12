---
name: gitnexus-refactoring
description: "Use when the user wants to rename, extract, split, move, or restructure code safely. Examples: \"Rename this function\", \"Extract this into a module\", \"Refactor this class\", \"Move this to a separate file\""
---

# Refactoring with GitNexus

> **IMPORTANT — How to use GitNexus**: GitNexus is a standalone CLI tool. Run it directly
> via `gitnexus <command>` in the Bash tool. Do **NOT** use `mcpl call gitnexus ...`.

> **Multi-repo note**: Always pass `--repo <name>` to every command to avoid
> "multiple repositories" errors.

## When to Use

- "Rename this function safely"
- "Extract this into a module"
- "Split this service"
- "Move this to a new file"
- Any task involving renaming, extracting, splitting, or restructuring code

## Workflow

```
1. gitnexus impact "X" --direction upstream --repo <name>  → Map all dependents
2. gitnexus query "X" --repo <name>                        → Find execution flows involving X
3. gitnexus context "X" --repo <name>                      → See all incoming/outgoing refs
4. Plan update order: interfaces → implementations → callers → tests
```

> If "Index is stale" → run `gitnexus analyze` in terminal.

## Checklists

### Rename Symbol

```
- [ ] gitnexus rename "oldName" "newName" --repo <name> --dry-run — preview all edits
- [ ] Review graph edits (high confidence) and text_search edits (review carefully)
- [ ] If satisfied: gitnexus rename "oldName" "newName" --repo <name> — apply edits
- [ ] gitnexus detect-changes --repo <name> — verify only expected files changed
- [ ] Run tests for affected processes
```

### Extract Module

```
- [ ] gitnexus context "target" --repo <name> — see all incoming/outgoing refs
- [ ] gitnexus impact "target" --direction upstream --repo <name> — find all external callers
- [ ] Define new module interface
- [ ] Extract code, update imports
- [ ] gitnexus detect-changes --repo <name> — verify affected scope
- [ ] Run tests for affected processes
```

### Split Function/Service

```
- [ ] gitnexus context "target" --repo <name> — understand all callees
- [ ] Group callees by responsibility
- [ ] gitnexus impact "target" --direction upstream --repo <name> — map callers to update
- [ ] Create new functions/services
- [ ] Update callers
- [ ] gitnexus detect-changes --repo <name> — verify affected scope
- [ ] Run tests for affected processes
```

## Commands

**`gitnexus rename`** — automated multi-file rename:

```bash
gitnexus rename "validateUser" "authenticateUser" --repo my-app --dry-run
# → 12 edits across 8 files
# → 10 graph edits (high confidence), 2 text_search edits (review)
```

**`gitnexus impact`** — map all dependents first:

```bash
gitnexus impact "validateUser" --direction upstream --repo my-app
# → d=1: loginHandler, apiMiddleware, testUtils
# → Affected Processes: LoginFlow, TokenRefresh
```

**`gitnexus detect-changes`** — verify your changes after refactoring:

```bash
gitnexus detect-changes --repo my-app
# → Changed: 8 files, 12 symbols
# → Affected processes: LoginFlow, TokenRefresh
# → Risk: MEDIUM
```

**`gitnexus cypher`** — custom reference queries:

```bash
gitnexus cypher 'MATCH (caller)-[:CodeRelation {type: "CALLS"}]->(f:Function {name: "validateUser"}) RETURN caller.name, caller.filePath ORDER BY caller.filePath' --repo my-app
```

## Risk Rules

| Risk Factor         | Mitigation                               |
| ------------------- | ---------------------------------------- |
| Many callers (>5)   | Use `gitnexus rename` for automated updates |
| Cross-area refs     | Use `gitnexus detect-changes` after to verify scope |
| String/dynamic refs | `gitnexus query` to find them            |
| External/public API | Version and deprecate properly           |

## Example: Rename `validateUser` to `authenticateUser`

```bash
# 1. Preview edits
gitnexus rename "validateUser" "authenticateUser" --repo my-app --dry-run
# → 12 edits: 10 graph (safe), 2 text_search (review)
# → Files: validator.ts, login.ts, middleware.ts, config.json...

# 2. Review text_search edits carefully (config.json: dynamic reference!)

# 3. Apply edits
gitnexus rename "validateUser" "authenticateUser" --repo my-app
# → Applied 12 edits across 8 files

# 4. Verify scope
gitnexus detect-changes --repo my-app
# → Affected: LoginFlow, TokenRefresh
# → Risk: MEDIUM — run tests for these flows
```
