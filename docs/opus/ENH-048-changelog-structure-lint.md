# ENH-048: CHANGELOG structure lint in `check_release_notes.py`

> Filed from the 2026-10-08 /opus-audit enhancement pass (cycle `audit-2026-10-08`). Board card: `[ENH-048]`.
> Sequencing: land this **after DOC-138, and preferably after DOC-139 (the 0.58.1 release fold)**. The rules **hard-fail**, with no warn mode. This is safe because `release-check` is not part of `checkall` (Makefile `:493-497`, `:516`), so a red result on a mid-cycle HEAD blocks nothing except tagging, and blocking a tag is the point. On HEAD the release check already exits 2 (`tag v0.58.1 not found`) before any rule can run, so a warn mode would never print anything until DOC-139 lands. It would only defer the same failure. Two more items must be fixed before a green run:
> - **DOC-138** must collapse two blank-line breaks inside subsections (`:29-30` and `:91-92`) as well as the duplicate headings. Its remediation steps do not say so, and the rule in step 3 reports both breaks.
> - **DOC-139's fold** must rewrite the 0.58.1 body's paths. 23 of its 25 repo paths are pre-split `src/…` paths that do not exist at HEAD.

**Priority**: low · **Estimate**: S

## Goal

`make release-check` (ENH-036) proves that the top CHANGELOG section cites every `feat`/`fix` commit, but it ignores the section's shape. DOC-138 and DOC-139 are shape defects a script can catch.

Extend `scripts/check_release_notes.py` with five structure rules, applied to the **section under check** (the same section ENH-036's derivation already picks):
1. Each Keep-a-Changelog subsection (`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`) appears at most once.
2. Every bullet sits under a `###` subsection, attached to it with no blank-line break.
3. No `` Commit `<this>` `` placeholders.
4. Every repo path in backticks exists at the head ref.
5. The `[Unreleased]:` compare link's base equals `v<newest versioned heading>`.

Any violation exits 1, the same code as a missing commit.

**Deviation from the brief, on purpose.** The brief scopes the path rule to `[Unreleased]`. But ENH-036 checks `[Unreleased]` only while it has content. At tagging time `[Unreleased]` is empty and the check moves to the top versioned section. A rule tied to `[Unreleased]` would therefore check nothing at the only moment `release-check` runs. Scoping every section rule to the checked section covers `[Unreleased]` mid-cycle and the release section at tagging time.

## Current state

- **Script.** `scripts/check_release_notes.py` (459 lines):
  - Constants `HEADING_RE`, `SUBJECT_RE`, `CARD_ID_RE`, `SHA_TOKEN_RE` and `RELEASE_TAG_RE` at `:26-31`.
  - `parse_sections()` `:43`, `section_text()` `:48`, `derive_checked_section()` `:86-120` (returns `(checked, base, head, top_versioned)`), `link_def_url()` `:165`, `write_links()` `:190-229`.
  - `check()` at `:232-261` prints `release-check: {base}..{head} — checking [{checked}] …`, then `MISSING …` lines, and returns 1 on a miss.
  - `self_test()` `:264-413`. Its CHANGELOG fixtures are minimal, for example `"# Changelog\n\n## [0.2.0]\n\n### Added\n- sha cited …"` at `:319-323`.
  - `main()` `:416-455` runs `check()` and then, with `--write`, `write_links()`. Exit 2 means an infrastructure `RuntimeError`.
- **Makefile.**
  - `release-check: derive-version-check core-version-check` at `:496-497` runs the script, then `--self-test`.
  - The comment at `:493-495` says it is deliberately not in `checkall`, and `checkall` at `:516` confirms that.
  - The help line is at `:68`.
- **CONTRIBUTING.md:108** reads: "Before tagging, `make release-check` must pass; then run `python3 scripts/check_release_notes.py --write` for the compare link." Rule 5 makes that order fail, because the check would run before `--write` fixes the link.
- **CHANGELOG on HEAD 2cf0957.** Measured with the step 3 code: 52 problems in `[Unreleased]` (`:8-102`).
  - Duplicate subsections: `### Added` at `:55`, `:58`, `:80` and `:97` (first at `:24`), and `### Changed` at `:85` (first at `:10`).
  - Detached bullet runs: `:30` (a blank line at `:29` inside `### Added`) and `:92-95` (the unheaded Fixed bullets after the blank at `:91`, which follow `### Changed` at `:85`). The audit cites the second run as `:90-95`, but `:89-90` are real `### Changed` bullets.
  - Eight `` Commit `<this>` `` placeholders at `:75-78` and `:98-101`.
  - 37 backticked repo paths missing at HEAD, 20 of them `src/mux/…`. The audit counted 18 with a narrower pattern, and this extraction also catches `src/grid/…`, `src/bin/par_mux/main.rs`, `src/tmux_control.rs` and `tests/mux_*.rs`.
- **Version and link state.**
  - `## [0.58.1] - 2026-10-02` at `:103` is dated but untagged (`git tag -l 'v0.58*'` shows only `v0.58.0`).
  - The link block has `[Unreleased]: …/compare/v0.58.0...HEAD` at `:2031` and `[0.58.0]` at `:2032`. There is no `[0.58.1]:` line.
  - `python3 scripts/check_release_notes.py` exits 2 with `ERROR: tag v0.58.1 not found; run \`git fetch --tags\``, so every Verify run on HEAD below passes `--base v0.58.0`.
- **0.58.1 body** (`:103-128`): 23 of its 25 repo paths are stale. It has no duplicate headings, no detached runs and no placeholders.

## Implementation

1. **Constants.** Add these after `RELEASE_TAG_RE` (`:31`):
   ```python
   KAC_SUBSECTIONS = ("Added", "Changed", "Deprecated", "Removed", "Fixed", "Security")
   SUBSECTION_RE = re.compile(r"^### (.+?)\s*$")
   PLACEHOLDER = "`<this>`"
   # A backticked token containing `/` with no placeholder/glob/shell chars;
   # only tokens whose first segment is a top-level path at the head ref count.
   PATH_TOKEN_RE = re.compile(r"`([^`\s<>*{}$~]+/[^`\s<>*{}$~]*)`")
   LINE_SUFFIX_RE = re.compile(r":\d+(?:-\d+)?$")
   UNRELEASED_LINK_RE = re.compile(
       r"^\[Unreleased\]: \S+/compare/(v[^.\s]+\.[^.\s]+\.[^.\s]+)\.\.\.HEAD\s*$",
       re.MULTILINE,
   )
   ```
2. **`head_tree(root, head) -> tuple[set[str], set[str]]`.** It runs `run_git(root, "ls-tree", "-r", "--name-only", head)` and returns the file set and the set of every directory prefix with a trailing `/`. Using the head ref's tree rather than the working tree makes the result reproducible for `--head`.
3. **`structure_problems(text, checked, head, root) -> list[str]`.** Each problem string starts with `CHANGELOG.md:<line>:` (1-based, absolute in the file).
   ```python
   def structure_problems(text: str, checked: str, head: str, root: Path) -> list[str]:
       sections = parse_sections(text)
       names = [n for n, _ in sections]
       start = sections[names.index(checked)][1]
       body = section_text(text, start)
       first = text[:start].count("\n") + 1
       lines = body.split("\n")
       problems: list[str] = []
       seen: dict[str, int] = {}
       current: str | None = None
       prev: str | None = None  # "H" after a ### line, "" after a blank line
       for off, line in enumerate(lines[1:], 1):
           n = first + off
           m = SUBSECTION_RE.match(line)
           if m:
               name = m.group(1)
               if name in KAC_SUBSECTIONS and name in seen:
                   problems.append(
                       f"CHANGELOG.md:{n}: duplicate `### {name}` in [{checked}] (first at :{seen[name]})"
                   )
               seen.setdefault(name, n)
               current, prev = name, "H"
               continue
           if line.startswith("- "):
               if current is None:
                   problems.append(
                       f"CHANGELOG.md:{n}: bullet outside any ### subsection in [{checked}]"
                   )
               elif prev == "":
                   problems.append(
                       f"CHANGELOG.md:{n}: bullet run detached from `### {current}` by a blank line in [{checked}] (merge it or give it its own heading)"
                   )
           prev = line if line.strip() else ""
           if PLACEHOLDER in line:
               problems.append(
                   f"CHANGELOG.md:{n}: `Commit {PLACEHOLDER}` placeholder in [{checked}] (cite the short SHA or drop the clause)"
               )
       files, dirs = head_tree(root, head)
       top = {f.split("/")[0] for f in files}
       reported: set[str] = set()
       for off, line in enumerate(lines):
           for token in PATH_TOKEN_RE.findall(line):
               path = LINE_SUFFIX_RE.sub("", token)
               if path.split("/")[0] not in top or path in reported:
                   continue
               if path in files or (path if path.endswith("/") else path + "/") in dirs:
                   continue
               reported.add(path)
               problems.append(
                   f"CHANGELOG.md:{first + off}: path `{path}` does not exist at {head} in [{checked}]"
               )
       return problems
   ```
   - Non-KAC headings (for example `### Removed (breaking for Rust embedders)` at `:260`) are tolerated as subsections. Only the six KAC names are checked for duplicates, because the brief enumerates those six.
   - The placeholder check is part of the line walk, so it is scoped to the checked section.
4. **`link_problem(text) -> str | None`** (rule 5). It returns `None` when there is no `## [Unreleased]` heading.
   - Otherwise it takes `newest = next(n for n, _ in parse_sections(text) if n != "Unreleased")` and matches `UNRELEASED_LINK_RE`.
   - With no match it returns `"CHANGELOG.md: no `[Unreleased]: …/compare/vX.Y.Z...HEAD` link (run --write)"`.
   - If the captured base is not `f"v{newest}"`, it returns `f"CHANGELOG.md: [Unreleased] link compares from {base}, but the newest version heading is [{newest}] (run --write)"`.
5. **Wire into `check()`** (`:232-261`).
   - After the `MISSING` printing, compute `problems = structure_problems(text, checked, head, root)`, then append `link_problem(text)` when it is not `None`.
   - Print each problem as `STRUCTURE {problem}`, followed by a one-line hint: "fix the CHANGELOG shape: one ### per Keep-a-Changelog subsection, no detached bullets, no `<this>`, paths that exist at head, [Unreleased] based on the newest version".
   - Return `1 if (missing or problems) else 0`.
   - Append `f", {len(problems)} structure problem(s)"` to the existing `release-check:` summary print (compute `problems` before that print).
   - For historical audits, add one flag, `--no-structure`, that skips steps 3–4. The default is on. It is not used by the Makefile.
6. **Write before checking** (resolves the CONTRIBUTING order conflict).
   - In `main()` (`:443-450`), when `args.write` is set, call `write_links(...)` **before** `check(...)`, so the check sees the fixed link.
   - Change `CONTRIBUTING.md:108` to: "Before tagging, run `python3 scripts/check_release_notes.py --write` (compare links), then `make release-check`, which must pass. It also lints the checked CHANGELOG section's structure: one subsection per Keep-a-Changelog type, no detached bullets, no `` `<this>` `` placeholders, existing paths, and an `[Unreleased]` link based on the newest version."
7. **Self-test.** Add these scenarios to `self_test()` after the existing ones (`:386-406`), reusing `commit()`, `changelog`, `run_in_root()` and `expect()`. The base repo has tags `v0.1.0` and `v0.2.0`, with `base.txt` and `f.txt` tracked.
   - **Clean shape passes.** `changelog.write_text` the following:
     ```
     # Changelog

     ## [Unreleased]

     ### Added
     - post release feature (`<sha of uncited2>`) `f.txt`

     ## [0.2.0]

     ### Added
     - released

     [Unreleased]: https://github.com/<slug>/compare/v0.2.0...HEAD
     ```
     Expect exit 0 and no `STRUCTURE` line.
   - **Each rule bites.** Starting from the clean text, apply one mutation at a time, and for each expect exit 1 plus a `STRUCTURE` line containing the needle:
     - a second `### Added\n- dup` block inside `[Unreleased]` → `duplicate`;
     - a `\n- detached` after a blank line under `### Added` → `detached`;
     - `- x Commit `<this>`.` → `placeholder`;
     - `` `scripts/missing.py` `` in a bullet (`scripts/` is a top-level dir of the self-test repo because the script copy lives there) → ``path `scripts/missing.py` does not exist``;
     - the link changed to `compare/v0.1.0...HEAD` → `newest version heading is [0.2.0]`.
   - **Write fixes the link.** With the bad link from the last mutation, `run_in_root("--write")` exits 0 and leaves `[Unreleased]: …/compare/v0.2.0...HEAD`. That proves the write-then-check order.
   - **Bullet outside a subsection.** `## [Unreleased]\n\n- orphan\n` → `outside any ### subsection`.
8. **Makefile `:68`.** Append to the help text: "; also lints that section's structure (duplicate subsections, detached bullets, `<this>`, stale paths, [Unreleased] link base)". Leave `release-check` out of `checkall`.

## Files to touch

- `scripts/check_release_notes.py` (constants, `head_tree`, `structure_problems`, `link_problem`, `check()`, `main()` order, `--no-structure`, self-test scenarios)
- `Makefile` (`:68` help text only)
- `CONTRIBUTING.md` (`:108`)

## Verify

- `python3 scripts/check_release_notes.py --self-test > /tmp/enh048-st.log 2>&1; echo EXIT=$?` prints `EXIT=0`. The log has no `FAIL` lines (`grep -c '  FAIL' /tmp/enh048-st.log` prints `0`), and it includes PASS lines for the duplicate, detached, placeholder, path, link, write-fixes-link and outside-subsection scenarios.
- **On HEAD 2cf0957 the lint reproduces DOC-138/DOC-139.** In a worktree at 2cf0957 with only this card's script copied in, run `python3 scripts/check_release_notes.py --base v0.58.0 > /tmp/enh048-head.log 2>&1; echo EXIT=$?`. It prints `EXIT=1`, and:
  - `grep -c '^STRUCTURE .*duplicate' /tmp/enh048-head.log` prints `5`;
  - `grep -c '^STRUCTURE .*detached' /tmp/enh048-head.log` prints `2`, naming `CHANGELOG.md:30` and `CHANGELOG.md:92`;
  - `grep -c '^STRUCTURE .*placeholder' /tmp/enh048-head.log` prints `8`;
  - `grep -c '^STRUCTURE .*does not exist' /tmp/enh048-head.log` prints `37`;
  - `grep -c '^STRUCTURE .*newest version heading is \[0.58.1\]' /tmp/enh048-head.log` prints `1`.
- `python3 scripts/check_release_notes.py --base v0.58.0 --no-structure > /tmp/enh048-ns.log 2>&1; grep -c '^STRUCTURE' /tmp/enh048-ns.log` prints `0`.
- After DOC-138 and the DOC-139 fold (0.58.1 tagged, links written), `make release-check > /tmp/enh048-rc.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and `grep -c '^STRUCTURE' /tmp/enh048-rc.log` prints `0`.
- `release-check` stays out of checkall: `make -n checkall > /tmp/enh048-n.log 2>&1; grep -c check_release_notes /tmp/enh048-n.log` prints `0`.
- `grep -c 'then `make release-check`' CONTRIBUTING.md` prints `1`.
- `make checkall > /tmp/enh048-c.log 2>&1; echo EXIT=$?` prints `EXIT=0`.

## Rollback

Revert the three files. The structure rules are additive inside `check()`, and the write-before-check order in `main()` is behavior-neutral apart from making the link rule passable. `--no-structure` is the in-place escape hatch if a rule misfires at release time.
