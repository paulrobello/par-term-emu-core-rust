# ENH-036: Release-notes completeness check (`make release-check`) with compare-link writing

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-036]`.
> Sequencing: independent. On HEAD it reports DOC-099's missing commit, so fix DOC-099 before relying on a green run. Backfilling the 0.38.0–0.56.0 compare links stays with DOC-126. This card only writes the top section's link going forward.

**Priority**: medium · **Estimate**: S

## Goal

Two recurring documentation findings are release-record gaps a script could catch mechanically:
- **DOC-099**: 0.57.0 shipped `1c4c479` (`split-window -b` and the `pane-info cmd=` token), but its CHANGELOG section does not mention it.
- **DOC-126**: the compare-link reference block stops at 0.37.0.

Add `scripts/check_release_notes.py`, run by a new `make release-check`. It lists every `feat`/`fix` commit since the previous release tag that the top CHANGELOG section does not reference, and, when asked with `--write`, adds the top section's compare link.

## Current state

- **CHANGELOG format** (Keep a Changelog):
  - `## [Unreleased]` at `CHANGELOG.md:8`, then `## [0.57.0] - 2026-09-29` at `:10`.
  - The link-reference block starts at `:1801` with `[0.37.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.36.0...v0.37.0` and ends with `[0.1.0]` at `:1851`.
- **How 0.57.0 cites its commits.** Its bullets cite commits in two ways:
  - Backticked short SHAs, for example "(`b42e1e0`)" and "(`4437667`, `0b58dc3`, `4dde9e5`)".
  - Card IDs without a SHA, for example "ENH-030" and "ENH-028". `22d2731`, `56e7a0a`, `2626707` and `4e89c23` are cited only by ID.
  - A SHA-only matcher would therefore report four false misses, so the matcher must accept either form.
- **Tags.**
  - Local release tags are `v0.1.0`…`v0.56.0`. `v0.57.0` is not tagged locally: the release tag is created by CI and is absent until `git fetch --tags`.
  - A non-release tag `bench-baseline` also exists.
  - So "the latest tag" is unreliable. It is wrong whenever `[Unreleased]` has content (it would still pick v0.56.0 and report all of 0.57.0's commits), and whenever the previous release's tag has not been fetched.
  - The CHANGELOG itself is the reliable source: the **base** is `v<the versioned heading below the section being checked>`.
- **Simulation.** On HEAD f6535f2 over `v0.56.0..HEAD` (25 commits, 11 `feat`/`fix`), a matcher that accepts a cited SHA prefix or a card ID from the commit subject reports exactly one miss: `1c4c479 feat(mux): split-window -b and pane-info cmd= foreground token`. That is DOC-099.
- **The convention is new.** Run against 0.56.0's section (`v0.55.0..v0.56.0`), the same rule reports 15 of 27 commits. 0.56.0 predates the SHA-citing convention that 0.57.0 introduced. The check therefore enforces the convention going forward and only ever inspects the top section. It must not be run over history.
- **Where the release step lives.** There is no release target in the Makefile, and `release.yml` only dispatches `deployment.yml`. The release procedure is the user's `cut-release` skill, which is outside this repository.

## Implementation

1. **Create `scripts/check_release_notes.py`** (stdlib plus `git` via `subprocess`, `python3`). Arguments: `--root` (default: the repo root), `--base TAG` and `--head REF` (overrides), `--write`, and `--self-test`.
2. **Section under check.**
   - Parse all `^## \[([^\]]+)\]` headings in order.
   - If the first is `Unreleased` **and** its body has any non-blank line, check `Unreleased`: the base is `v<first versioned heading>` and the head defaults to `HEAD`.
   - Otherwise check the first versioned heading `X`: the base is `v<second versioned heading>`, and the head defaults to `vX` when that tag exists locally, else `HEAD`.
   - The section text runs up to the next `^## \[`.
3. **Resolve refs.**
   - `git rev-parse --verify <base>^{commit}`. If the base tag is missing, fail closed: "tag vY not found; run `git fetch --tags`" (exit 2, distinct from "missing entries").
   - Never fall back to "latest tag". `--base`/`--head` override for audits of historical states.
   - Print the resolved `base..head` on the first output line so a reader sees what was compared.
4. **Candidates.**
   - Run `git log --format=%H%x09%s%x09%(trailers:key=Changelog,valueonly,separator=%x2C) <base>..<head>`.
   - Keep subjects matching `^(feat|fix)(\([^)]*\))?!?:`.
   - Skip commits whose `Changelog:` trailer is `skip`. That is the escape hatch for internal-only fixes, and it is visible in `git log`.
5. **Referenced when either holds:**
   - Any backticked 7–40-hex token in the section is a prefix of the full SHA.
   - Any card ID in the subject (`\b(ENH|QA|SEC|ARC|DOC)-\d+\b`) appears in the section as a whole word.
6. **Report.**
   - Print `MISSING <short-sha> <subject>` per miss, then a one-line hint: cite the SHA in a bullet, or add a `Changelog: skip` trailer with `git commit --amend` before the tag.
   - Exit 1 on any miss, 0 otherwise.
7. **`--write`** (idempotent; touches at most two lines).
   - Build `[<ver>]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v<prev>...v<ver>` for the top versioned section, where `<prev>` is the next versioned heading.
   - If a `[<ver>]:` definition already exists, replace it only when it differs. Otherwise insert it immediately above the first existing `^\[\d` reference line.
   - If `## [Unreleased]` exists, maintain `[Unreleased]: …/compare/v<ver>...HEAD` the same way.
   - Never touch other lines. Backfilling 0.38.0–0.56.0 is DOC-126, and the script may print a notice that older links are absent.
8. **`--self-test`.** Create a throwaway repo in a `tempfile.TemporaryDirectory()` (`git init`, then commits using `-c user.name=t -c user.email=t@t`) containing:
   - tags `v0.1.0` and `bench-baseline`;
   - four `feat`/`fix` commits after `v0.1.0`: one cited by SHA, one by card ID, one with a `Changelog: skip` trailer, and one uncited;
   - one `docs:` commit;
   - a CHANGELOG with `## [0.2.0]`.

   Assert that:
   - only the uncited commit is reported;
   - the base resolves to `v0.1.0`, not `bench-baseline`;
   - `--write` twice produces one `[0.2.0]:` line with byte-identical output on the second run;
   - **latest tag missing:** with a `## [0.3.0]` section above `## [0.2.0]` and no `v0.2.0` tag, the script exits 2 with the `git fetch --tags` hint rather than silently comparing against `v0.1.0`;
   - **Unreleased has content:** with a non-empty `## [Unreleased]` above `## [0.2.0]` and `v0.2.0` tagged, only commits after `v0.2.0` are candidates, and they are matched against the Unreleased section.
9. **Makefile.**
   - Add `release-check: ; python3 scripts/check_release_notes.py && python3 scripts/check_release_notes.py --self-test`.
   - It stays out of `checkall`: mid-cycle, `[Unreleased]` is legitimately incomplete, and the check belongs at the moment of release.
   - Add a `make help` line and one CONTRIBUTING "Version Sync" bullet: "Before tagging, `make release-check` must pass; then run `python3 scripts/check_release_notes.py --write` for the compare link."
10. **Follow-up outside this repo.** Add `make release-check` (then `--write`) to the `cut-release` skill's pre-tag step. Record this in the PR description. The skill is not a repo file, so it is not edited here.

## Files to touch

- `scripts/check_release_notes.py` (new)
- `Makefile` (`release-check`, `help`)
- `CONTRIBUTING.md` (one bullet in "Version Sync")
- `CHANGELOG.md`, only through `--write` when it is run at a release, and only the top link line(s)

## Verify

- Against HEAD f6535f2's CHANGELOG (`git worktree add <tmp> f6535f2`, copy the script in, then run `python3 scripts/check_release_notes.py --root <tmp> --head f6535f2`), the script exits 1, prints `v0.56.0..f6535f2`, and reports exactly one commit, `1c4c479`. The four ID-cited ENH-028/ENH-030 commits are not reported.
- After citing `1c4c479` in the `## [0.57.0]` section (DOC-099), `python3 scripts/check_release_notes.py --head f6535f2` exits 0.
- On a committed (clean) CHANGELOG, `python3 scripts/check_release_notes.py --write` adds exactly two lines: `[0.57.0]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.56.0...v0.57.0` and `[Unreleased]: https://github.com/paulrobello/par-term-emu-core-rust/compare/v0.57.0...HEAD`. `git diff --numstat CHANGELOG.md` reports `2 0`, and a second `--write` leaves that diff byte-identical.
- `python3 scripts/check_release_notes.py --self-test` exits 0. Only the uncited commit is reported, `bench-baseline` is never chosen as the base, the `Changelog: skip` commit is excluded, a missing base tag exits 2 with the fetch hint, and a non-empty `[Unreleased]` is checked against commits since the top tag.
- `make release-check` exists and is listed by `make help`. `make -n checkall` does not mention it, and `make checkall` stays green.

## Rollback

Delete the script and the target. Any compare link written by `--write` is a correct CHANGELOG line and can stay.
