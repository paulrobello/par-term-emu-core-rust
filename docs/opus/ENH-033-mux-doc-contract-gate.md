# ENH-033: `check_mux_docs.py` — gate MUX.md and the `notification_type` list against the mux code

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-033]`.
> Sequencing: lands together with, or after, the DOC-100 and DOC-102 doc fixes. The gate fails on HEAD by design, so it cannot enter `checkall` first. DOC-100 is blocked by QA-190, which changes where the exit code is documented, not the type list this gate checks.

**Priority**: high · **Estimate**: S

## Goal

The 0.57.0 par-mux contract drifted from its docs in three High findings (DOC-100, DOC-101, DOC-102), and the audit's gate-coverage note records that nothing in `checkall` compares MUX.md with the code. The API reference and the FFI surface are both machine-checked. The par-mux wire contract is not, although it is the repository's highest-churn area (`dispatch::dispatch_command` hotspot 1470, `server::handle_client` 1271).

Add `scripts/check_mux_docs.py`, run by `make mux-docs-check` inside `checkall`. It diffs four code-owned lists against their documentation, so the next command or notification added without its doc row fails the gate.

## Current state

The four lists and their docs, simulated against HEAD f6535f2 with the regexes proposed below:

| # | Code source | Doc target | Result on HEAD |
|---|---|---|---|
| 1 | `COMMANDS` table, `src/mux/command.rs:763-797` (33 `(name, parser)` pairs) | MUX.md "Command Reference" table rows (`docs/MUX.md:163-195`) | Clean: 33 names, 33 rows |
| 2 | `MuxCommand::mutates()` true arms, `src/mux/command.rs:315-351` | MUX.md "When it saves" list (`docs/MUX.md:350`) | Misses `rename-session`, `kill-session` (DOC-102 item 1) |
| 3 | `%…` format strings in `emit::emit`, `src/mux/emit.rs:37-158` | MUX.md Notifications table (`docs/MUX.md:225-241`) | Misses `%session-renamed` (DOC-102 item 2), plus five false positives (see below) |
| 4 | `TmuxNotification::notification_type` arms, `src/tmux_control.rs:209-246` (34 strings) | `docs/API_REFERENCE.md:1838` bullet | Misses `pane-exited`, `pane-respawned` (DOC-100) |

Details the script must handle:
- **`mutates()` names variants, not commands.** The variant names do not convert mechanically to command names (`SwapPanes` is `swap-pane`, `SwapWindows` is `swap-window`). The mapping comes from `COMMANDS`: each parser (`parse_swap_windows`, …) constructs exactly one `MuxCommand::X`. `RefreshClient { size, .. } => size.is_some()` is conditional, and the save list documents it as the literal `refresh-client -C`.
- **Five `emit()` strings are not client notifications.**
  - `%begin`, `%end` and `%error` are reply framing, documented under "Protocol Overview" (`docs/MUX.md:126-141`), not in the Notifications table.
  - `%unlinked-window-close` and `%pane-mode-changed` have emit arms (`emit.rs:48,54`), but only emit.rs's own tests construct them (`emit.rs:296,306`). The daemon never sends them.
- **The `emit()` wildcard.** `_ => String::new()` at `emit.rs:156` means a variant with no arm emits nothing. ARC-094 replaces that wildcard. This gate is complementary: ARC-094 makes an unwired variant a compile error, and this gate makes an undocumented wired one a check failure.
- **What the gate cannot see.** DOC-101 (the inverted `kill-pane` last-pane rule) and DOC-102 item 3 (the stale "reaper cascade" wording in the `%sessions-changed` row) are prose meaning, not list membership. A list diff cannot catch them. This plan does not claim them.
- **Existing gate style to copy.** `scripts/check_ffi_surface.py:35-41` refuses a regex that matched nothing ("a broken gate, not a clean pass"). The FFI gates run with `python3` and no build (`Makefile:363-364`), so this gate needs no maturin build either.

## Implementation

1. **Create `scripts/check_mux_docs.py`** (stdlib only, `python3`), modeled on `check_ffi_surface.py`.
   - Take a `--root PATH` argument (default: the repo root, from `Path(__file__)`). Every file is read relative to it.
   - **Every extraction fails closed.** Each regex that returns zero items raises `SystemExit("error: … parsed nothing from <file>")`.
2. **Check 1: command rows.**
   - Parse `COMMANDS` as `\("([a-z-]+)",\s*(\w+)\)` inside the `const COMMANDS` block.
   - Parse the MUX.md rows as `^\| \`([a-z-]+)\` \|` within the `## Command Reference` section (up to the next `## `).
   - Report names missing in either direction.
3. **Check 2: save list.**
   - Build a variant-to-command map. For each `(name, parser)` pair, take the body of `fn <parser>` and collect `MuxCommand::(\w+)`. Exactly one distinct variant is required; zero or several is a hard error naming the parser.
   - Take the variants before `=> true` in the `mutates()` body.
   - Parse the backticked names in the `**When it saves:**` sentence.
   - Report every mutating command missing from it. Additionally require the literal `` `refresh-client -C` `` for the conditional `RefreshClient` arm.
4. **Check 3: notification rows.**
   - Collect `"%([a-z-]+)` inside `pub fn emit(`, stopping at the function's closing brace.
   - Split the non-table strings into two module-level sets, each with a reason comment:
     - `FRAMING = {"begin", "end", "error"}`: reply framing. The check requires each to appear as `` `%begin` ``/`` `%end` ``/`` `%error` `` in MUX.md's `## Protocol Overview` section instead of the table, so the framing docs are gated too.
     - `NEVER_SENT = {"unlinked-window-close", "pane-mode-changed"}`: emit arms kept for parser round-trip tests, never constructed by the daemon.
   - Compare the rest with `^\| \`%([a-z-]+)` rows in `## Notifications`, in both directions.
   - **Guard against stale allowlist entries.**
     - Fail if any `FRAMING` or `NEVER_SENT` name has no emit arm.
     - For `NEVER_SENT` only, also fail if `TmuxNotification::<Variant>` for that name appears in production code under `src/mux/` (any `.rs` other than `emit.rs`, with `#[cfg(test)] mod tests` blocks stripped). That means the daemon started sending it, so it now needs a table row.
5. **Check 4: `notification_type` strings.**
   - Parse `=> "([a-z-]+)"` inside `pub fn notification_type(&self)`.
   - Parse the backticked tokens on the `- \`notification_type: str\`` line of API_REFERENCE.md.
   - Report types missing from the doc, and doc tokens that are not types.
6. **Add `--self-test`.**
   - Copy the four inputs (`src/mux/command.rs`, `src/mux/emit.rs`, `src/tmux_control.rs`, `docs/MUX.md`, `docs/API_REFERENCE.md`) into a `tempfile.TemporaryDirectory()`.
   - Run all checks against the copy, which must pass (the self-test runs after the doc fixes).
   - Then apply one injected drift per check and assert that each run exits 1 and names the injected item:
     1. Delete the `| \`version\` |` row.
     2. Remove `` `set-buffer`, `` from the save list.
     3. Delete the `%window-add` row.
     4. Remove `` `exit`, `` from the `notification_type` line.
     5. Add a fake `("fake-cmd", parse_version),` entry to `COMMANDS`, to check the reverse direction.
7. **Makefile.**
   - Add `mux-docs-check: ; python3 scripts/check_mux_docs.py && python3 scripts/check_mux_docs.py --self-test`.
   - Add it to the `checkall` prerequisites next to `ffi-surface-check`.
   - Add a `make help` line.
8. **Land it with the doc fixes.** If DOC-100 and DOC-102 are not merged yet, fix exactly the lists the gate reports in the same branch:
   - `rename-session` and `kill-session` in the save list.
   - A `%session-renamed $N <name>` row.
   - The two type strings.
   - Leave DOC-101 and DOC-102 item 3 to their own cards.
9. Add one bullet to CONTRIBUTING's "Verification" section (it has no mux-specific section): "`make mux-docs-check` fails until a new mux command, notification or notification type is listed in MUX.md and API_REFERENCE."

## Files to touch

- `scripts/check_mux_docs.py` (new)
- `Makefile` (`mux-docs-check` target, the `checkall` prerequisite, `help`)
- `docs/MUX.md`, `docs/API_REFERENCE.md` (only if DOC-100/DOC-102 have not landed; the list fixes above)
- `CONTRIBUTING.md` (one line)

## Verify

- Against HEAD f6535f2 (before DOC-100/102): `git worktree add <tmp> f6535f2`, then `python3 scripts/check_mux_docs.py --root <tmp>` from the implementation checkout. It exits 1, and its output names exactly `rename-session`, `kill-session`, `%session-renamed`, `pane-exited` and `pane-respawned`, and none of `begin`, `end`, `error`, `unlinked-window-close` or `pane-mode-changed`. Remove the worktree afterwards.
- After the list fixes, `python3 scripts/check_mux_docs.py` exits 0 and prints a one-line summary with the four counts (33 commands, the mutating count, the notification rows, 34 types).
- `python3 scripts/check_mux_docs.py --self-test` exits 0, and each of its five injected drifts is individually reported as a failure naming the injected item.
- Replacing the `const COMMANDS` identifier in a temp copy (run with `--root <copy>`) makes the script exit non-zero with a "parsed nothing" error, not a pass.
- `make checkall` is green and lists `mux-docs-check` among its prerequisites.

## Rollback

Remove the `mux-docs-check` prerequisite from `checkall` and delete the script. The doc fixes are independent corrections, so keep them.
