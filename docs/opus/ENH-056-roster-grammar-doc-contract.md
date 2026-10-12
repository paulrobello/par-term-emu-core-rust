# ENH-056: `list-agents` roster-grammar contract in `check_mux_docs.py`

> Filed from the 2026-10-12 /opus-audit enhancement pass. Board card: `[ENH-056]`.
> Sequencing: land **after DOC-146** (which rewrites SKILL.md's `list-agents` lines into the `key=value` form) and **after DOC-147** (which adds SKILL.md to the mux-docs inputs). Before DOC-146, the new check fails on the current SKILL.md, and that failure is the defect this card exists to catch.

**Priority**: medium · **Estimate**: S

## Goal

The `list-agents` row grammar is code-owned: `roster_row` (`crates/par-mux/src/mux/dispatch/client.rs:186-232`) appends the trailing `key=value` tokens. Two docs describe that grammar for consumers, and nothing ties them to the code. DOC-146 is the result: the embedded agent skill still documents `%N <agent> <state> <source> [message]`.

Add a sixth check to `scripts/check_mux_docs.py`. It extracts the extra keys from `roster_row` and requires each one to be named as `` `<key>=`` in:
1. `docs/MUX.md`, inside the `- **\`list-agents\`**` bullet of `## Command Reference`;
2. `crates/par-mux/skills/par-mux/SKILL.md`, anywhere in the file (it has no stable section for the grammar).

In SKILL.md the check also rejects the stale free-text form `[message]` or `<reason>` on a `list-agents` line.

## Current state

- **Code**: `roster_row` at `client.rs:194-232`. Its keys come from four `extras.push(("<key>".to_string(), …))` calls at `:210` `reason`, `:213` `telemetry`, `:216` `host_telemetry` and `:219` `ws`. The doc comment `:186-193` restates them. The only production caller is `client.rs:297`. The consumer side is `AgentRow::parse` (`crates/par-mux/src/mux/ipc/rows.rs:178-200`), which splits each trailing token on its first `=` and returns `None` for a token with no `=`.
- **MUX.md**:
  - `:623` Command Reference table row: `` `list-agents` `` … `%N <agent> <state> <source>`. It is positional only, which is acceptable because the bullet is the grammar's home.
  - `:675` is the bullet `- **\`list-agents\`** returns one line per pane…`. It names `` `reason=<base64>` ``, `` `telemetry=<base64>` ``, `` `host_telemetry=<base64>` `` and `` `ws=<id>` ``, and it states "unknown keys are ignored". The check passes on it today.
- **SKILL.md** (`crates/par-mux/skills/par-mux/SKILL.md`, embedded by `include_str!` at `crates/par-mux/src/mux/skill_installer.rs:16`):
  - `:68` reads `par-mux --cmd list-agents       # agent roster: "%N <agent> <state> <source> [message]"`.
  - `:131` reads `par-mux --cmd list-agents       # %N claude blocked hook <reason>`.
  - None of `reason=`, `telemetry=`, `host_telemetry=` or `ws=` appears, so the new check fails on HEAD aaf35ef. DOC-146 fixes that.
- **Gate script** (`scripts/check_mux_docs.py`, 793 lines):
  - `SELF_TEST_FILES` at `:51-60` lacks both `dispatch/client.rs` and SKILL.md. DOC-147 adds SKILL.md. If it has not landed, this card adds it.
  - `read()` `:105`, `fn_block()` `:124-140` (brace-matched body after a marker), `md_section()` `:143-150`, `collect_problems()` `:438-583` (checks 1-5, `counts` dict `:575-581`).
  - `run_self_test()` `:596-759`. Its drift list is at `:614-674`, and the success string `:755` hard-codes "all 8 injected drifts".
  - The `--self-test` help at `:773` says "inject eight drifts", and the success print is at `:784-789`.
- **Makefile**: `mux-docs-check` `:432-434` is already in `checkall` `:525`. No Makefile change is needed.

## Implementation

1. **Inputs.** Add the two constants and extend `SELF_TEST_FILES` (`:51-60`). Skip any entry DOC-147 already added.
   ```python
   ROSTER_RS = "crates/par-mux/src/mux/dispatch/client.rs"
   SKILL_MD = "crates/par-mux/skills/par-mux/SKILL.md"
   ```
   ```python
   (ROSTER_RS,)
   (SKILL_MD,)
   ```
2. **Extraction helper** (place it after `doc_notification_types`, `:400-410`):
   ```python
   ROSTER_KEY_RE = re.compile(r'extras\.push\(\(\s*"([a-z_]+)"\.to_string\(\)')


   def roster_extra_keys(client_rs: str) -> list[str]:
       """The trailing key=value keys roster_row appends, in emit order."""
       keys = ROSTER_KEY_RE.findall(fn_block(client_rs, "fn roster_row(", ROSTER_RS))
       if not keys:
           fail(
               f'parsed nothing from {ROSTER_RS}: no extras.push(("key"… in fn roster_row('
           )
       return keys
   ```
   `fn_block` already fails closed when the marker or its body is missing.
3. **MUX.md bullet helper:**
   ```python
   def list_agents_bullet(mux_md: str) -> str:
       section = md_section(mux_md, "Command Reference", "docs/MUX.md")
       m = re.search(
           r"^- \*\*`list-agents`\*\*[^\n]*(?:\n(?!- |\n)[^\n]*)*", section, re.MULTILINE
       )
       if not m:
           fail(
               "parsed nothing from docs/MUX.md: no `- **`list-agents`**` bullet in Command Reference"
           )
       return m.group(0)
   ```
   The bullet runs to the next `- ` bullet or blank line. Today it is the single line `:675`.
4. **Check 6, in `collect_problems()`** after check 5 (before `counts`):
   ```python
   # Check 6: list-agents trailing keys (roster_row) ↔ MUX.md bullet + SKILL.md (ENH-056).
   client_rs = read(root, ROSTER_RS)
   skill_md = read(root, SKILL_MD)
   roster_keys = roster_extra_keys(client_rs)
   bullet = list_agents_bullet(mux_md)
   for where, text in (("docs/MUX.md `list-agents` bullet", bullet), (SKILL_MD, skill_md)):
       for key in roster_keys:
           if f"`{key}=" not in text and f" {key}=" not in text:
               problems.append(
                   f"{where} does not name the list-agents `{key}=` token that roster_row emits ({ROSTER_RS})"
               )
   if "unknown keys are ignored" not in bullet:
       problems.append(
           "docs/MUX.md `list-agents` bullet must keep the consumer rule 'unknown keys are ignored'"
       )
   for n, line in enumerate(skill_md.splitlines(), 1):
       if "list-agents" in line and re.search(r"\[message\]|<reason>", line):
           problems.append(
               f"{SKILL_MD}:{n}: list-agents line uses the free-text message/reason form; the row is "
               "`%N <agent> <state> <source>` then key=value tokens (reason=<base64>)"
           )
   ```
   The `" {key}="` alternative accepts the key inside a fenced code example such as `# %N claude blocked hook reason=aGk= ws=+1` in SKILL.md, where it is not individually backticked.
   Add `"roster_keys": len(roster_keys)` to `counts`, and append `, {counts['roster_keys']} list-agents keys` to the success print (`:784-789`).
5. **Module docstring** (`:1-26`): add item 6, `roster_row extras.push keys (crates/par-mux/src/mux/dispatch/client.rs) ↔ MUX.md list-agents bullet and SKILL.md`. Change "five code-owned lists" to "six".
6. **Self-test drifts.** Append to the list at `:614-674`:
   - `("drop `ws=` from the MUX.md list-agents bullet", "docs/MUX.md", lambda t: t.replace("and a `ws=<id>` token", "and a workspace token", 1), "ws=")`. Confirm the exact substring on the tree before committing. On aaf35ef the bullet contains `` and a `ws=<id>` token ``.
   - `("add a fake extras key to roster_row", ROSTER_RS, lambda t: t.replace('extras.push(("ws".to_string(), ws));', 'extras.push(("ws".to_string(), ws));\n        extras.push(("frob".to_string(), String::new()));', 1), "frob=")`.
   - `("restore the stale [message] form in SKILL.md", SKILL_MD, lambda t: t + '\npar-mux --cmd list-agents  # "%N <agent> <state> <source> [message]"\n', "free-text")`.
   - Fail-closed case, next to the clap one at `:690-705`: overwrite `ROSTER_RS` with `"fn roster_row() {}\n"` and expect `SystemExit` containing `parsed nothing from crates/par-mux/src/mux/dispatch/client.rs`.
   - Replace the hard-coded counts in `:755` and `:773` with `len(drifts)` and "inject one drift per contract" so the next drift added does not leave them stale.

## Files to touch

- `scripts/check_mux_docs.py` (constants, `SELF_TEST_FILES`, two helpers, check 6, counts and print, docstring, self-test drifts and fail-closed case)
- `CHANGELOG.md` `[Unreleased]` (one `### Added` bullet)

## Verify

Acceptance criteria:
- `PATH="$PWD/.venv/bin:$PATH" make mux-docs-check` exits 0 on the tree after DOC-146, prints `4 list-agents keys`, and its self-test reports the three new drifts plus the empty-`roster_row` fail-closed case.
- Seeded violation fails: with this card's script on aaf35ef (before DOC-146), `python3 scripts/check_mux_docs.py` exits 1 with four `SKILL.md does not name the list-agents` lines (`reason=`, `telemetry=`, `host_telemetry=`, `ws=`) and two free-text lines (`SKILL.md:68` and `SKILL.md:131`).
- Removing `` `ws=<id>` `` from MUX.md:675 on the finished tree exits 1 naming `ws=`.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci` exits 0.

Commands:
```bash
python3 scripts/check_mux_docs.py > /tmp/enh056.log 2>&1; echo EXIT=$?; grep -c 'list-agents keys' /tmp/enh056.log   # EXIT=0, 1
python3 scripts/check_mux_docs.py --self-test > /tmp/enh056-st.log 2>&1; echo EXIT=$?; grep -c 'drift reported' /tmp/enh056-st.log
PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh056-ci.log 2>&1; echo EXIT=$?
```
Run the seeded-violation demonstration in a scratch worktree at aaf35ef with the new script copied in. Do not stash in the working checkout.

## Rollback

Revert `scripts/check_mux_docs.py`. Check 6 is additive and isolated in `collect_problems()`. If a later SKILL.md rewrite needs a different grammar phrasing, adjust the `" {key}="` acceptance rather than removing the check.

## Dependencies / sequencing

- **Blocks on DOC-146.** SKILL.md must name the four keys first.
- **Coordinates with DOC-147**, which edits the same `SELF_TEST_FILES` tuple and may add drifts. Land DOC-147 first and rebase onto it, keeping the `len(drifts)` count.
- **ENH-061** adds `ws=` to `%agent-state-changed`. When it lands, it may extend check 6 to require the Notifications row to name `source=` and `ws=`. That extension is out of scope here.
