# ENH-035: Offline intra-repo link and anchor gate (`make doc-links-check`) in `checkall`

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-035]`.
> Sequencing: lands together with, or after, DOC-124, which fixes the three links this gate reports on HEAD.

**Priority**: medium · **Estimate**: S

## Goal

Broken intra-doc anchors have recurred across audit cycles: DOC-124 now, DOC-093 before it. The audit's gate-coverage note lists "intra-repo anchors" as ungated. Add an offline link and fragment check over the shipped docs to `checkall`, so a renamed heading or a moved file fails locally instead of waiting for the next audit.

## Decision: lychee, not a custom checker

Use `lychee --offline --include-fragments`. The brief also allows a small Python checker. lychee wins on evidence from a probe run at HEAD f6535f2 with lychee 0.24.2:

- Over `docs/*.md README.md CONTRIBUTING.md QUICKSTART.md CHANGELOG.md` it checked 1018 links (854 local; the 164 `http(s)` links are skipped by `--offline`). It reported **exactly the three DOC-124 errors** and no false positives:
  - `docs/API_REFERENCE.md:872`: `CHANGELOG.md#0500---2026-09-21`. lychee reports "File not found" because the link is relative to `docs/` and resolves to `docs/CHANGELOG.md`. This is a path error as well as a stale date (the heading is dated 2026-09-23). DOC-124's remedy only mentions the date; the fix needs `../CHANGELOG.md#0500---2026-09-23`.
  - `docs/SECURITY.md:40`: `#do-use-command--args-array-format`
  - `docs/SECURITY.md:41`: `#dont-concatenate-user-input-into-commands`
- **Emoji-prefixed headings resolve correctly.** A scratch file with `# ✅ DO: Use Command + Args` accepted `#-do-use-command--args` and rejected `#do-use-command--args`. That is GitHub's slug rule: the emoji is dropped and its trailing space becomes a leading hyphen. So SECURITY.md:40-41's TOC links need the leading `-`. A Python checker would have to reimplement this slugger, including its duplicate-heading `-1` suffixes and punctuation stripping, and keep it in sync with GitHub. lychee already does.
- `CLAUDE.md` and `AGENTS.md` also pass today (0 errors).
- lychee is installed locally (`/opt/homebrew/bin/lychee`), is a single static binary, and has a pinnable GitHub Action for CI. It is guarded in the Makefile the same way cbindgen is (`Makefile:335-338`).

Cost of the choice: one more developer tool. `checkall` fails with an install hint when lychee is missing, which is the same contract cbindgen already imposes.

## Current state

- `checkall` (`Makefile:366`) has no link check. There is no `lychee.toml`, `.lycheeignore` or `.lycheecache` in the repo.
- **Scope decisions:**
  - A non-recursive `docs/*.md` already skips `docs/opus/`, `docs/fable/` (executed plans) and `docs/research/`. Those are planning and archive material, and DOC-129 may delete some of them.
  - `AUDIT*.md` stay out. They quote broken links as evidence, and the audit pipeline owns them.
  - `CHANGELOG.md` goes **in**: it passes today, and its historical sections cannot rot because they link only to GitHub compare URLs, which `--offline` skips. The brief's "exclude CHANGELOG historical sections" therefore needs no special handling.
- **`<!-- doc-path: example -->` waivers are out of scope.** Those waivers (DOC-132; `docs/DOCUMENTATION_STYLE_GUIDE.md:197,210,212`, `docs/MUX.md:340`) silence parsight's scanner for *backticked placeholder paths* such as `` `src/config.ts` ``. lychee checks Markdown *links* (`[text](target)`), not backticked paths, so those lines never reach it and no waiver mechanism is needed. Checking backticked paths is a separate job, and parsight's `find_broken_doc_links` already covers it.
- **Do not pass `--cache`.** It writes `.lycheecache` into the working tree, and an offline run takes well under a second without it.

## Implementation

1. **Makefile target**, placed next to `ffi-surface-check`:
   ```make
   # ENH-035: offline intra-repo link + GitHub-slug anchor check. External
   # URLs are skipped (--offline); planning dirs are outside the glob.
   DOC_LINK_FILES := docs/*.md README.md CONTRIBUTING.md QUICKSTART.md CHANGELOG.md CLAUDE.md
   doc-links-check:
   	@command -v lychee >/dev/null 2>&1 || { \
   		echo "ERROR: lychee not found — brew install lychee (or cargo install lychee --locked)"; \
   		exit 1; \
   	}
   	lychee --offline --include-fragments --no-progress $(DOC_LINK_FILES)
   ```
   Add `doc-links-check` to the `checkall` prerequisites and a `make help` line (this also advances DOC-128).
2. **Fix the three links** in the same branch if DOC-124 has not landed. **DOC-124 has landed** (fix/audit-remediation): API_REFERENCE now links `../CHANGELOG.md#0500---2026-09-23`, and the two SECURITY.md headings lost their emoji, so the existing non-hyphen TOC slugs resolve. Skip this step; applying the leading-hyphen slugs below would break the TOC again. Just confirm with lychee that all three resolve.
   - `docs/API_REFERENCE.md:872`: point at `../CHANGELOG.md#0500---2026-09-23`.
   - `docs/SECURITY.md:40-41`: use the leading-hyphen slugs `#-do-use-command--args-array-format` and `#-dont-concatenate-user-input-into-commands`.
   - Confirm each target with `lychee` rather than by eye.
3. **Record the scope** in a comment above the target: which globs are in, why the planning dirs and `AUDIT*.md` are out, and that backticked paths belong to parsight, not this gate.
4. **CI** (non-gated half of ARC-104's spirit): add a `doc-links` step to `.github/workflows/ci.yml`'s lint job. Install lychee with the pinned `lycheeverse/lychee-action@<40-char-sha> # vX.Y.Z`, per ENH-032's policy for third-party actions (`args: --offline --include-fragments --no-progress <same globs>`, `fail: true`, `lycheeVersion: v0.24.2`, so CI's fragment and slug behavior matches the local probe). Resolve the SHA with `gh api repos/lycheeverse/lychee-action/git/ref/tags/<tag> --jq .object.sha`, dereferencing an annotated tag. Dependabot keeps it current. CI stays dispatch-only (D1 is not affected).
5. **Add one line to `docs/DOCUMENTATION_STYLE_GUIDE.md`**: "Intra-repo links and heading anchors are checked by `make doc-links-check` (lychee, GitHub slug rules; an emoji-prefixed heading's slug starts with `-`)."

## Files to touch

- `Makefile` (`doc-links-check`, the `checkall` prerequisite, `help`)
- `docs/API_REFERENCE.md`, `docs/SECURITY.md` (only if DOC-124 has not landed)
- `docs/DOCUMENTATION_STYLE_GUIDE.md` (one line)
- `.github/workflows/ci.yml` (one pinned step)

## Verify

- Against HEAD f6535f2: `git worktree add <tmp> f6535f2`, then inside `<tmp>` run `lychee --offline --include-fragments --no-progress docs/*.md README.md CONTRIBUTING.md QUICKSTART.md CHANGELOG.md CLAUDE.md`. It exits non-zero and reports exactly three errors: `docs/API_REFERENCE.md` at 872 and `docs/SECURITY.md` at 40 and 41. Remove the worktree afterwards.
- After the fixes, `make doc-links-check` exits 0, and `git status --porcelain` shows no `.lycheecache` or other new file.
- Injecting a broken anchor (for example changing `(#notifications)` in `docs/MUX.md`'s TOC to `(#notification)`) makes `make doc-links-check` exit non-zero, naming `docs/MUX.md`. Revert the change afterwards.
- With lychee absent from `PATH` (`env PATH=/usr/bin:/bin make doc-links-check`), the target exits 1 with the install hint, not a silent pass.
- `make checkall` is green and lists `doc-links-check` among its prerequisites.

## Rollback

Remove the target and its `checkall` prerequisite, and remove the CI step. The link fixes are independent, so keep them.
