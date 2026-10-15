# ENH-057: "Pinned by `test_name`" citation gate (`pinned-tests-check`)

> Filed from the 2026-10-12 /opus-audit enhancement pass. Board card: `[ENH-057]`.
> Sequencing: independent. All 17 citations on HEAD aaf35ef resolve today (measured), so the gate lands green.
> Line-number citations below are pinned to commit `f4161f6` (the commit that added this plan); re-locate them by symbol on later trees.

**Priority**: low · **Estimate**: S

## Goal

Docs and CHANGELOG bullets cite regression tests by name as evidence: "pinned by `render_mode_border_style_survives_split`". A rename or deletion leaves a dangling citation that reads as proof but points at nothing, and no gate notices. Add `scripts/check_pinned_tests.py`, wired into `checkall`, that requires every backticked identifier in a **pinned clause** in `docs/*.md` and in the CHANGELOG `[Unreleased]` body to name a test that exists in the tree.

## Current state

- **How the docs phrase it** (every `pinned` occurrence near a backtick, measured on aaf35ef across `docs/*.md` and `[Unreleased]`):
  - `docs/MUX.md:276` `(pinned by `render_mode_ctrl_d_closing_a_tab_keeps_both_clients_in_the_session`)`.
  - `docs/MUX.md:279` "pinned by `a` and `b`".
  - `docs/MUX.md:341` "pinned at the renderer level by `sidebar_offset_paints_content_and_dividers_together` and end to end by `render_mode_sidebar_toggle_refits_pane_geometry`, which parses the CUP column…". Qualifiers sit between `pinned` and `by`.
  - `docs/MUX.md:343`, `:346` "pinned red-to-green by `…`", `:387` (two clauses on one line), `:548` "Pinned by `a_slow_draining_client_is_evicted_once_its_backlog_passes_the_cap` (`crates/par-mux/src/mux/server/tests.rs`)". The path is in a parenthetical.
  - `docs/BUILDING.md:47` "pinned to the Rust values by the `layout_header_defines_match_rust` test".
  - `CHANGELOG.md:11` "Pinned by `sidebar_agents_section_groups_by_workspace`, `sidebar_agent_row_click_focuses_the_pane`, the updated wire round-trip and `mux_agents` integration assertions, and the PTY-level `render_mode_side_panel_carries_the_agent_roster` + …". `mux_agents` is an integration-test **target** (`crates/par-mux/tests/mux_agents.rs`), not a fn.
  - `CHANGELOG.md:14` "Pinned by `heartbeat_re_emits_the_placement_when_quiet`."
- **Non-test backticks inside pinned clauses that must not be flagged**: `docs/API_REFERENCE.md:2524` "pinned by `_Static_assert`s" (leading underscore, uppercase), and `MUX.md:548`'s path token (contains `/`).
- **Other phrasings** ("covered by", "tested by", "regression test `x`"): zero backticked hits in scope. "pins" occurrences (`ARCHITECTURE.md:75,341`, `BUILDING.md:47`, `FFI_GUIDE.md:3`) cite versions, files or header constants, not tests. So the grammar is the word `pinned`/`Pinned`, the rest of that sentence, and lowercase snake_case backticked tokens.
- **Prototype result** (clause regex below, run on aaf35ef): 17 citations, 0 dangling. 16 resolve to `#[test]` fns, and `mux_agents` resolves to a test target stem. A `#[test]`-attribute scan over all tracked `.rs` files finds 3,759 test fn names.
- **Scope rule**: `docs/*.md` matches the `DOC_LINK_FILES` glob comment at `Makefile:490-495` (it keeps out `docs/opus/`, `docs/fable/`, `docs/research/`). For CHANGELOG, only `[Unreleased]` is in scope, because released sections are history and may cite tests since renamed.
- **Makefile**: template `mux-docs-check` `:432-434`, `checkall` `:525`, `.PHONY` `:4`, help `:62`.

## Implementation

1. **Create `scripts/check_pinned_tests.py`** (stdlib, typed, `argparse` with `--root` and `--self-test`, shaped like `check_mux_docs.py:762-793`).
2. **Grammar**:
   ```python
   # A pinned clause: the word "pinned" to the end of its sentence (". ", ";" or EOL).
   CLAUSE_RE = re.compile(
       r"\b[Pp]inned\b(?P<clause>[^\n]*?)(?=\.(?:\s|$)|;|$)", re.MULTILINE
   )
   # Candidate test names: lowercase snake_case only (skips `_Static_assert`, paths, flags).
   CITED_RE = re.compile(r"`([a-z][a-z0-9_]*)`")
   UNRELEASED_RE = re.compile(
       r"^## \[Unreleased\]\s*\n(?P<body>.*?)(?=^## \[)", re.MULTILINE | re.DOTALL
   )
   ```
   Report line numbers absolute in the file. For CHANGELOG, offset by the line where the body starts.
   A token with no `_` (for example `` `mux_agents` `` has one, but a hypothetical `` `foo` `` has none) is still checked. Single words are rare in clauses and still must resolve.
3. **Test-name universe** (walk `root.rglob("*.rs")`, skipping `target/`, `fuzz/target/` and `node_modules/`):
   ```python
   TEST_FN_RE = re.compile(
       r"#\[(?:[\w:]+::)?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*|///[^\n]*\n\s*)*(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)"
   )
   ```
   - The regex covers `#[test]`, `#[tokio::test]`, `#[rstest]`-style prefixes, and stacked `#[ignore]`, `#[cfg_attr(…)]` and `#[should_panic]`.
   - Add **test target stems**: the file stem of every `crates/*/tests/*.rs` and `tests/*.rs`, which accepts `mux_agents`.
   - Add Python tests: `def (test_[a-z0-9_]+)` in `tests/**/*.py`, since docs may cite pytest names.
   - Fail closed: fewer than 100 names, or no `.rs` file scanned, raises `fail("parsed nothing: test-name scan found N names")`. Zero clauses across all docs is also a fail, because the grammar must match at least the 17 known citations.
4. **Report**: `docs/MUX.md:343: pinned clause cites `x`, which is not a test fn or test target in the tree` for each miss. Exit 1. On success print `pinned tests ok: N citations in M files resolve`.
5. **Self-test** (temp copy of `docs/`, `CHANGELOG.md` and `crates/`, `src/`, `tests/` `.rs`/`.py` files only, via `shutil.copytree(..., ignore=shutil.ignore_patterns("target", "node_modules", "*.png", …))`; or build a minimal synthetic tree, which is faster: one `crates/x/src/lib.rs` with `#[test] fn real_one() {}` padded to 100 dummy `#[test]` fns, one `crates/x/tests/suite_a.rs`, and docs with clauses):
   - **Synthetic baseline passes**: "pinned by `real_one` and `suite_a`.", "pinned at the unit level by `real_one`, which parses `not_checked_after_period`." (the token after the sentence end is out of the clause, so it is not flagged), "pinned by `_Static_assert`s" and "(`crates/x/src/lib.rs`)" are ignored.
   - **Drift reported**: "pinned by `renamed_away`" in a `docs/*.md` file gives exit 1 naming `renamed_away`. The same in CHANGELOG `[Unreleased]` also fails. The same text in a released `## [0.1.0]` section passes (out of scope), and so does the same text in `docs/opus/x.md` (out of scope).
   - **Fail closed**: an empty `.rs` tree raises `parsed nothing`.
   - **Real-tree check**: the self-test also runs the gate on the real root and requires at least 17 citations, which guards against a grammar regression silently matching fewer.
6. **Makefile**:
   ```make
   # ENH-057: every "pinned by `test`" citation in docs/*.md and CHANGELOG
   # [Unreleased] names a test fn or test target that exists. Pure python3.
   pinned-tests-check:
   	python3 scripts/check_pinned_tests.py
   	python3 scripts/check_pinned_tests.py --self-test
   ```
   Add it to `.PHONY` (`:4`), add a help line, and put it in `checkall` (`:525`) after `doc-links-check`.
7. **CONTRIBUTING.md**: after the `make mux-docs-check` bullet at `:53`, add one bullet: "Cite regression tests as `pinned by `<test_fn>``. `pinned-tests-check` fails when the named test no longer exists."

## Files to touch

- `scripts/check_pinned_tests.py` (new)
- `Makefile` (`.PHONY`, help, target, `checkall`)
- `CONTRIBUTING.md` (one sentence)
- `CHANGELOG.md` `[Unreleased]` (one `### Added` bullet)

## Verify

Acceptance criteria:
- `PATH="$PWD/.venv/bin:$PATH" make pinned-tests-check` exits 0 on the clean tree and reports at least 17 citations resolving (aaf35ef baseline: 17 citations, 16 test fns plus the `mux_agents` target).
- Seeded violation fails: renaming `render_mode_border_style_survives_split` to `render_mode_border_style_survives_split_x` in `docs/MUX.md:343` makes `python3 scripts/check_pinned_tests.py` exit 1 with one line naming `docs/MUX.md:343` and the bad name. The same edit inside a released CHANGELOG section does not fail.
- `python3 scripts/check_pinned_tests.py --self-test` exits 0, covering the out-of-scope (released section, `docs/opus/`), non-identifier (`_Static_assert`, path) and fail-closed cases.
- `PATH="$PWD/.venv/bin:$PATH" make checkall-ci` exits 0 with `check_pinned_tests` in the `make -n checkall` output.

Commands:
```bash
python3 scripts/check_pinned_tests.py > /tmp/enh057.log 2>&1; echo EXIT=$?; cat /tmp/enh057.log
python3 scripts/check_pinned_tests.py --self-test > /tmp/enh057-st.log 2>&1; echo EXIT=$?
sed -i '' 's/render_mode_border_style_survives_split`/render_mode_border_style_survives_split_x`/' docs/MUX.md
python3 scripts/check_pinned_tests.py > /tmp/enh057-bad.log 2>&1; echo EXIT=$?; grep -c 'MUX.md:343' /tmp/enh057-bad.log   # EXIT=1, 1
git checkout -- docs/MUX.md
make -n checkall > /tmp/enh057-n.log 2>&1; grep -c check_pinned_tests /tmp/enh057-n.log    # 2
PATH="$PWD/.venv/bin:$PATH" make checkall-ci > /tmp/enh057-ci.log 2>&1; echo EXIT=$?
```

## Rollback

Revert the script, the Makefile lines and the CONTRIBUTING sentence. The gate is read-only. If a legitimate new phrasing (for example "covered by `x`") appears later, widen `CLAUSE_RE` there and add a self-test case for it, rather than adding exceptions.

## Dependencies / sequencing

- Independent of ENH-055 and ENH-056. If they run in parallel, sequence the single-line `checkall` edit in the `Makefile`.
- DOC-144 (the MUX.md side-panel rewrite) will add new "pinning tests" prose. Land this gate first so DOC-144's citations are checked when it lands.
