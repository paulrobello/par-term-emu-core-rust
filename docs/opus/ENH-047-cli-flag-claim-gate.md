# ENH-047: CLI flag-claim gate in `check_mux_docs.py`

> Filed from the 2026-10-08 /opus-audit enhancement pass (cycle `audit-2026-10-08`). Board card: `[ENH-047]`.
> Sequencing: land this **after ARC-126**. On HEAD 2cf0957 the new check correctly fails on the `--border-active-color` and `--border-color` claims at `CHANGELOG.md:11` and `crates/par-mux/src/mux/config.rs:393-396`. ARC-126 removes both claims along with the dead `Overrides` fields. `mux-docs-check` is part of `make checkall`, so landing this first would turn checkall red on main. The other ten `[Unreleased]` bullets mentioning par-mux carry no other false claim, so DOC-138's restructure does not affect this card.

**Priority**: low · **Estimate**: S

## Goal

Documentation keeps claiming `par-mux` command-line flags that clap does not define. ARC-126 is the live case: two attach flags are described in the CHANGELOG and in rustdoc as the "flag tier", but `AttachArgs` has no such fields.

Extend `scripts/check_mux_docs.py` (ENH-033's gate, already in `make checkall` through `mux-docs-check`) with a fifth list: every `--long-flag` token claimed for par-mux must exist as a clap long flag in `crates/par-mux/src/bin/par_mux/main.rs`. Claims are gathered from three places:
- `docs/MUX.md`;
- the par-mux bullets of the CHANGELOG `## [Unreleased]` section;
- the rustdoc in `crates/par-mux/src/mux/config.rs`.

Flags that belong to other programs (cargo, git, par-term-streamer, agent CLIs) go in an allowlist with a reason. The check fails closed wherever an empty extraction means the gate is broken.

## Current state

- **The gate.**
  - `scripts/check_mux_docs.py` (642 lines). The docstring at `:2-23` says "Diffs four code-owned lists" and states the fail-closed rule at `:20-21`.
  - `fail()` is at `:65`, `read()` at `:69`, `md_section()` at `:107`, and `strip_test_modules()` at `:190`. That last function only matches `#[cfg(test)] mod`.
  - `SELF_TEST_FILES` is at `:47-53`. `collect_problems()` is at `:359-475`, with `counts` at `:469-474`.
  - `run_self_test()` is at `:488`. Its drifts list starts at `:506`, and the final message at `:604-607` says "all 5 injected drifts reported".
  - `main()` is at `:611`, and the OK line is at `:633-637`.
- **Makefile and docs.**
  - The `mux-docs-check` recipe at Makefile `:429-431` runs the script and then `--self-test`. It is listed in `checkall` at `:516`.
  - The help text at `:62` reads "Fail when MUX.md or the API_REFERENCE notification_type list drifts from the mux code".
  - `CONTRIBUTING.md:53` describes the gate.
- **The clap surface** (`crates/par-mux/src/bin/par_mux/main.rs`):
  - `struct Cli` at `:64-154`: `--socket` `:71`, `--state-dir` `:76`, `--stop` `:83`, `--restart` `:90`, `--cmd` (explicit `long = "cmd"`, `:97-103`), `--list-servers` `:112-116`, `--pane-endpoints` `:123`, `--expose-control-socket` `:130`, `--gen-config` `:148`, `--force` `:152`.
  - `struct AttachArgs` at `:170-197`: `-t` (short only), `--prefix` (`long = "prefix"`), `--mode` (`long = "mode"`), and `--socket`.
  - That makes twelve distinct long names. clap's derive also provides `--help` and `--version` (`#[command(version)]` at `:60`).
  - The test module at `:932` is `#[cfg(all(test, feature = "attach"))]`, which `strip_test_modules` does not match. That is harmless because it holds no `#[arg(`. Truncating at the first `#[cfg(` that contains `test` covers it anyway (step 2).
- **Claims on HEAD 2cf0957**, measured with the token regex in step 2 and the claim sources in step 3. Anything not in the clap set:
  - `docs/MUX.md`: `bin` (`:36`, `:42`, `:86`, `:94-95`), `features` and `locked` (`:86`, `:95`), `path` (`:86`), `no-default-features` (`:95`), `test-threads` (`:682`), `mux-socket` (`:648`, par-term-streamer), `resume` (`:622`, claude/grok CLI), `session` (`:547`, pi CLI) and `others` (`:583`, `git ls-files`). All are other programs' flags.
  - CHANGELOG `[Unreleased]` par-mux bullets: `bin`, `features`, `locked`, `path`, `no-default-features` and `release`, which are cargo flags. Also `border-active-color` and `border-color` at `:11`, which are **false** claims (ARC-126).
  - `config.rs` rustdoc (production code above `#[cfg(test)]` at `:1028`): `border-active-color` and `border-color` at `:393` and `:395`, which are **false** claims (ARC-126).
- **Other binaries' flags stay out of scope by construction.** Non-par-mux `[Unreleased]` bullets mention `--allow-api-key-in-query` (`:32`) and `--download-frontend` (`:33`). Those are par-term-streamer flags in bullets that do not mention `par-mux`, so the bullet filter excludes them.

## Implementation

1. **Docstring.**
   - Change "Diffs four code-owned lists" to "Diffs five code-owned lists".
   - Append item 5: `` `#[arg(long…)]` fields (crates/par-mux/src/bin/par_mux/main.rs) ↔ every `--flag` claimed in MUX.md, CHANGELOG [Unreleased] par-mux bullets, and config.rs rustdoc ``.
2. **Constants and helpers.** Add these after `DOC_NOTIF_ROW_RE` (`:61`):
   ```python
   MAIN_RS = "crates/par-mux/src/bin/par_mux/main.rs"
   CONFIG_RS = "crates/par-mux/src/mux/config.rs"
   # `--` + kebab word, not preceded by a word char or `-` (so `---` rules and
   # `a--b` never match).
   FLAG_TOKEN_RE = re.compile(r"(?<![\w-])--([a-z][a-z0-9]*(?:-[a-z0-9]+)*)")
   ARG_ATTR_RE = re.compile(r"#\[arg\((.*?)\)\]", re.DOTALL)
   FIELD_RE = re.compile(r"^\s*(?:pub\s+)?(\w+)\s*:", re.MULTILINE)
   LONG_EXPLICIT_RE = re.compile(r'\blong\s*=\s*"([a-z0-9-]+)"')
   LONG_BARE_RE = re.compile(r"\blong\b(?!\s*=)")

   # clap derive built-ins (#[command(version)] plus the implicit help).
   CLAP_BUILTIN_FLAGS: set[str] = {"help", "version"}

   # Flags of OTHER programs that par-mux docs legitimately quote. Each needs a
   # reason; an entry that collides with a real par-mux flag fails the gate.
   FOREIGN_FLAGS: dict[str, str] = {
       "bin": "cargo build/run/install --bin",
       "features": "cargo --features",
       "no-default-features": "cargo --no-default-features",
       "locked": "cargo --locked",
       "path": "cargo install --path",
       "release": "cargo build --release",
       "test-threads": "libtest -- --test-threads",
       "mux-socket": "par-term-streamer --mux-socket (MUX.md Streaming Panes)",
       "resume": "claude/grok --resume <id> (agent resume argv)",
       "session": "pi --session <file> (agent resume argv)",
       "others": "git ls-files --others (host telemetry probe)",
   }
   ```
   Then the extractors:
   ```python
   def clap_long_flags(main_rs: str) -> set[str]:
       """Long flag names clap derives from main.rs (production code only)."""
       cut = re.search(r"#\[cfg\([^\]]*\btest\b", main_rs)
       code = strip_line_comments(main_rs[: cut.start()] if cut else main_rs)
       flags: set[str] = set()
       for m in ARG_ATTR_RE.finditer(code):
           args = m.group(1)
           explicit = LONG_EXPLICIT_RE.search(args)
           if explicit:
               flags.add(explicit.group(1))
           elif LONG_BARE_RE.search(args):
               field = FIELD_RE.search(code, m.end())
               if not field:
                   fail(f"parsed nothing from {MAIN_RS}: no field after #[arg(long …)]")
               flags.add(field.group(1).replace("_", "-"))
       if not flags:
           fail(
               f"parsed nothing from {MAIN_RS}: no #[arg(long …)] fields (clap derive moved?)"
           )
       return flags


   def unreleased_par_mux_bullets(changelog: str) -> str:
       """Top-level bullets of `## [Unreleased]` that mention par-mux. Empty is
       legitimate (right after a release fold); a missing heading is not."""
       m = re.search(
           r"^## \[Unreleased\]\s*$(.*?)(?=^## \[|\Z)", changelog, re.MULTILINE | re.DOTALL
       )
       if not m:
           fail("parsed nothing from CHANGELOG.md: no `## [Unreleased]` heading")
       bullets = re.split(r"\n(?=- )", m.group(1))
       return "\n".join(b for b in bullets if "par-mux" in b)


   def rustdoc_text(rs: str, where: str) -> str:
       cut = rs.find("#[cfg(test)]")
       code = rs[:cut] if cut >= 0 else rs
       docs = "\n".join(re.findall(r"^\s*//[/!][^\n]*", code, re.MULTILINE))
       if not docs:
           fail(f"parsed nothing from {where}: no rustdoc comments")
       return docs
   ```
   Note on `FIELD_RE`: it skips the `///` doc lines and `#[…]` attributes between `#[arg(…)]` and the field, because those lines do not match `^\s*(pub\s+)?\w+\s*:`. Verified on `main.rs:97-103`, where a multi-line `#[arg(` is followed by `command: Option<String>`.
3. **Check 5 in `collect_problems()`.**
   - Read the two new inputs next to the existing ones at `:360-364`: `main_rs = read(root, MAIN_RS)`, `config_rs = read(root, CONFIG_RS)` and `changelog = read(root, "CHANGELOG.md")`.
   - Insert this before `counts = {` (`:469`):
     ```python
     # Check 5: every --flag claimed for par-mux exists in clap (ARC-126 class).
     clap_flags = clap_long_flags(main_rs)
     for name in sorted(set(FOREIGN_FLAGS) & clap_flags):
         problems.append(
             f"FOREIGN_FLAGS entry `--{name}` is a real par-mux flag — drop it from the allowlist"
         )
     known = clap_flags | CLAP_BUILTIN_FLAGS | set(FOREIGN_FLAGS)
     mux_claims = set(FLAG_TOKEN_RE.findall(mux_md))
     if not mux_claims:
         fail("parsed nothing from docs/MUX.md: no --flag tokens")
     config_claims = set(FLAG_TOKEN_RE.findall(rustdoc_text(config_rs, CONFIG_RS)))
     if not config_claims:
         fail(f"parsed nothing from {CONFIG_RS}: no --flag tokens in rustdoc")
     changelog_claims = set(FLAG_TOKEN_RE.findall(unreleased_par_mux_bullets(changelog)))
     for where, claims in (
         ("docs/MUX.md", mux_claims),
         ("CHANGELOG.md [Unreleased] (par-mux bullets)", changelog_claims),
         (f"{CONFIG_RS} rustdoc", config_claims),
     ):
         for name in sorted(claims - known):
             problems.append(
                 f"{where} claims `--{name}`, which is not a par-mux clap flag in {MAIN_RS} "
                 "(add the flag, fix the claim, or allowlist it in FOREIGN_FLAGS with a reason)"
             )
     ```
   - Add `"flags": len(clap_flags),` to `counts`.
   - Do **not** add an "unused allowlist entry" check. `release` appears only in `[Unreleased]`, so that check would break checkall at the next release fold. The collision guard above is the safe direction.
4. **Self-test.**
   - Add `"crates/par-mux/src/bin/par_mux/main.rs"`, `"crates/par-mux/src/mux/config.rs"` and `"CHANGELOG.md"` to `SELF_TEST_FILES`.
   - Append to `drifts` (`:506`):
     ```python
     (
         (
             "claim a nonexistent --frobnicate flag in MUX.md",
             "docs/MUX.md",
             lambda t: t.replace(
                 "## Command Line\n", "## Command Line\n\n`par-mux --frobnicate`\n", 1
             ),
             "frobnicate",
         ),
     )
     (
         (
             "claim a nonexistent --border-color flag in config.rs rustdoc",
             "crates/par-mux/src/mux/config.rs",
             lambda t: t.replace(
                 "    /// `--state-dir`.\n",
                 "    /// `--state-dir`.\n    /// `--border-color` (attach).\n",
                 1,
             ),
             "border-color",
         ),
     )
     (
         (
             'drop the long = "cmd" spelling from main.rs',
             "crates/par-mux/src/bin/par_mux/main.rs",
             lambda t: t.replace('long = "cmd",', "", 1),
             "cmd",
         ),
     )
     ```
     The third drift removes `--cmd` from the clap set. MUX.md claims `--cmd` 19 times, so the report must name it.
   - After the drifts loop, add a fail-closed case. In a temp copy, replace main.rs's text with `"fn main() {}\n"` and assert that `collect_problems` raises `SystemExit` whose message contains `parsed nothing from crates/par-mux/src/bin/par_mux/main.rs`. Print `self-test: empty clap extraction fails closed`.
   - Change the final message at `:604-607` from "all 5 injected drifts reported" to "all 8 injected drifts reported, empty clap extraction fails closed".
5. **OK line** (`:633-637`): append `, {counts['flags']} clap long flags` to the f-string.
6. **Makefile `:62`.** Change the help text to "Fail when MUX.md / the API_REFERENCE notification_type list drifts from the mux code, or a doc claims a par-mux --flag clap lacks".
7. **CONTRIBUTING.md:53.** Append: ", and when MUX.md, a par-mux `[Unreleased]` CHANGELOG bullet, or `config.rs` rustdoc names a `--flag` that `par-mux`'s clap surface does not define (another program's flag goes in `FOREIGN_FLAGS` with a reason)".

## Files to touch

- `scripts/check_mux_docs.py` (docstring, constants, three helpers, check 5, `SELF_TEST_FILES`, drifts, fail-closed case, messages)
- `Makefile` (`:62` help text)
- `CONTRIBUTING.md` (`:53`)

## Verify

- **Pre-ARC-126 the gate bites.** On a checkout of 2cf0957 with only this card's script copied in, `python3 scripts/check_mux_docs.py > /tmp/enh047-pre.log 2>&1; echo EXIT=$?` prints `EXIT=1`. The log names `--border-active-color` and `--border-color` for both `CHANGELOG.md [Unreleased]` and the `config.rs` rustdoc: `grep -c 'border' /tmp/enh047-pre.log` prints `4`. No other check-5 line appears: `grep -c 'is not a par-mux clap flag' /tmp/enh047-pre.log` prints `4`.
- **After ARC-126:** `python3 scripts/check_mux_docs.py > /tmp/enh047.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and the OK line ends with `12 clap long flags`.
- `python3 scripts/check_mux_docs.py --self-test > /tmp/enh047-st.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and the log contains `all 8 injected drifts reported` and `self-test: empty clap extraction fails closed`.
- Every allowlist entry has a reason: `python3 -c "import importlib.util,sys;s=importlib.util.spec_from_file_location('m','scripts/check_mux_docs.py');m=importlib.util.module_from_spec(s);s.loader.exec_module(m);print(all(v.strip() for v in m.FOREIGN_FLAGS.values()))"`. It prints `True`, meaning every allowlist entry has a reason.
- `make mux-docs-check > /tmp/enh047-m.log 2>&1; echo EXIT=$?` prints `EXIT=0`.
- `make checkall > /tmp/enh047-c.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

## Rollback

Revert the three files. Check 5 is self-contained in `collect_problems()`, and removing it restores the four-list gate unchanged.
