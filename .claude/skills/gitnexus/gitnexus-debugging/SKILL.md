---
name: gitnexus-debugging
description: "Use when the user is debugging a bug, tracing an error, or asking why something fails. Examples: \"Why is X failing?\", \"Where does this error come from?\", \"Trace this bug\""
---

# Debugging with GitNexus

> **IMPORTANT — How to use GitNexus**: GitNexus is a standalone CLI tool. Run it directly
> via `gitnexus <command>` in the Bash tool. Do **NOT** use `mcpl call gitnexus ...`.

> **Multi-repo note**: Always pass `--repo <name>` to every command to avoid
> "multiple repositories" errors.

## When to Use

- "Why is this function failing?"
- "Trace where this error comes from"
- "Who calls this method?"
- "This endpoint returns 500"
- Investigating bugs, errors, or unexpected behavior

## Workflow

```
1. gitnexus query "<error or symptom>" --repo <name>   → Find related execution flows
2. gitnexus context "<suspect>" --repo <name>          → See callers/callees/processes
3. gitnexus cypher "MATCH path..." --repo <name>       → Custom call-chain traces if needed
4. Read the actual source files to confirm root cause
```

> If "Index is stale" → run `gitnexus analyze` in terminal.

## Checklist

```
- [ ] Understand the symptom (error message, unexpected behavior)
- [ ] gitnexus query for error text or related code
- [ ] Identify the suspect function from returned processes
- [ ] gitnexus context to see callers and callees
- [ ] gitnexus cypher for custom call chain traces if needed
- [ ] Read source files to confirm root cause
```

## Debugging Patterns

| Symptom              | GitNexus Approach                                                   |
| -------------------- | ------------------------------------------------------------------- |
| Error message        | `gitnexus query` for error text → `gitnexus context` on throw sites |
| Wrong return value   | `gitnexus context` on the function → trace callees for data flow    |
| Intermittent failure | `gitnexus context` → look for external calls, async deps            |
| Performance issue    | `gitnexus context` → find symbols with many callers (hot paths)     |
| Recent regression    | `gitnexus detect-changes` to see what your changes affect           |

## Commands

**`gitnexus query`** — find code related to error:

```bash
gitnexus query "payment validation error" --repo my-app
# → Processes: CheckoutFlow, ErrorHandling
# → Symbols: validatePayment, handlePaymentError, PaymentException
```

**`gitnexus context`** — full context for a suspect:

```bash
gitnexus context "validatePayment" --repo my-app
# → Incoming calls: processCheckout, webhookHandler
# → Outgoing calls: verifyCard, fetchRates (external API!)
# → Processes: CheckoutFlow (step 3/7)
```

**`gitnexus cypher`** — custom call chain traces:

```bash
gitnexus cypher 'MATCH path = (a)-[:CodeRelation {type: "CALLS"}*1..2]->(b:Function {name: "validatePayment"}) RETURN [n IN nodes(path) | n.name] AS chain' --repo my-app
```

## Example: "Payment endpoint returns 500 intermittently"

```bash
# 1. Find related execution flows
gitnexus query "payment error handling" --repo my-app
# → Processes: CheckoutFlow, ErrorHandling
# → Symbols: validatePayment, handlePaymentError

# 2. Inspect the suspect
gitnexus context "validatePayment" --repo my-app
# → Outgoing calls: verifyCard, fetchRates (external API!)

# 3. Trace the full call chain
gitnexus cypher 'MATCH path = (a)-[:CodeRelation*1..3]->(b:Function {name: "fetchRates"}) RETURN path' --repo my-app

# 4. Root cause: fetchRates calls external API without proper timeout
```
