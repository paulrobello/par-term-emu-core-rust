---
name: gitnexus-exploring
description: "Use when the user asks how code works, wants to understand architecture, trace execution flows, or explore unfamiliar parts of the codebase. Examples: \"How does X work?\", \"What calls this function?\", \"Show me the auth flow\""
---

# Exploring Codebases with GitNexus

> **IMPORTANT — How to use GitNexus**: GitNexus is a standalone CLI tool. Run it directly
> via `gitnexus <command>` in the Bash tool. Do **NOT** use `mcpl call gitnexus ...`.

> **Multi-repo note**: Always pass `--repo <name>` to every command to avoid
> "multiple repositories" errors.

## When to Use

- "How does authentication work?"
- "What's the project structure?"
- "Show me the main components"
- "Where is the database logic?"
- Understanding code you haven't seen before

## Workflow

```
1. gitnexus list                                              → Discover indexed repos
2. gitnexus status                                            → Check index freshness
3. gitnexus query "<what you want to understand>" --repo <name>  → Find related execution flows
4. gitnexus context "<symbol>" --repo <name>                  → Deep dive on specific symbol
5. Read source files from the returned file paths for implementation details
```

> If step 2 says the index is stale → run `gitnexus analyze` in terminal.

## Checklist

```
- [ ] gitnexus status to verify index freshness
- [ ] gitnexus query for the concept you want to understand
- [ ] Review returned processes (execution flows) and file paths
- [ ] gitnexus context on key symbols for callers/callees
- [ ] Read source files for implementation details
```

## Commands

**`gitnexus query`** — find execution flows related to a concept:

```bash
gitnexus query "payment processing" --repo my-app
# → Processes: CheckoutFlow, RefundFlow, WebhookHandler
# → Symbols grouped by flow with file locations
```

**`gitnexus context`** — 360-degree view of a symbol:

```bash
gitnexus context "validateUser" --repo my-app
# → Incoming calls: loginHandler, apiMiddleware
# → Outgoing calls: checkToken, getUserById
# → Processes: LoginFlow (step 2/5), TokenRefresh (step 1/3)
```

**`gitnexus cypher`** — custom graph queries for structural exploration:

```bash
gitnexus cypher 'MATCH (f:Function)-[:MEMBER_OF]->(c:Class {name: "PaymentService"}) RETURN f.name, f.filePath' --repo my-app
```

## Example: "How does payment processing work?"

```bash
# 1. Check index is fresh
gitnexus status

# 2. Find execution flows for the concept
gitnexus query "payment processing" --repo my-app
# → CheckoutFlow: processPayment → validateCard → chargeStripe
# → RefundFlow: initiateRefund → calculateRefund → processRefund

# 3. Deep dive on the main entry point
gitnexus context "processPayment" --repo my-app
# → Incoming: checkoutHandler, webhookHandler
# → Outgoing: validateCard, chargeStripe, saveTransaction

# 4. Read src/payments/processor.ts for implementation details
```
