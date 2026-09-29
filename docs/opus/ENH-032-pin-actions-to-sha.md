# ENH-032: Pin third-party GitHub Actions to commit SHAs in secret-bearing workflows, with Dependabot updates

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-032]`.
> Outward-facing only when pushed: editing workflows is local, but verifying them needs a push/dispatch, which requires user confirmation.

## Goal

The repo publishes to crates.io and PyPI from CI. `publish-crates.yml` carries `CARGO_REGISTRY_TOKEN`, `deployment.yml` has 7 secret references, and there are PyPI trusted-publishing jobs. Today every action is referenced by a floating tag, so a compromised or retagged upstream tag would run inside a job holding registry credentials.

Pin third-party (non-GitHub-owned) actions to full commit SHAs, with the tag kept in a trailing comment, and add a Dependabot config so the pins stay current.

## Current state

Action references across `.github/workflows/*.yml` (count × ref):
- `actions/checkout@v7` ×26, `actions/setup-python@v7` ×12, `actions/upload-artifact@v7` ×11, `actions/download-artifact@v8` ×6, `actions/github-script@v9` ×1: GitHub-owned, lower risk.
- `dtolnay/rust-toolchain@stable` ×12, `@nightly` ×1: third-party, a branch ref (moves by design).
- `PyO3/maturin-action@v1` ×7
- `Ilshidur/action-discord@0.4.0` ×7
- `ilammy/msvc-dev-cmd@v1` ×3
- `pypa/gh-action-pypi-publish@release/v1` ×2 (branch ref)
- `docker/setup-qemu-action@v4` ×2
- `anthropics/claude-code-action@v1` ×2
- `sigstore/gh-action-sigstore-python@v3.5.0` ×1
- `oven-sh/setup-bun@v2` ×1

Secret-bearing workflows: `deployment.yml` (7), `publish-crates.yml` (5), `publish-testpypi.yml` (2), `release.yml`, `claude.yml`, `claude-code-review.yml` (1 each). There is no `.github/dependabot.yml`.

The user guide `~/.claude/guides/git-ci.md` (section on keeping actions up to date) prefers exact tags where no floating major exists. This card goes one step further, to SHAs, for the credential-bearing third-party set only.

## Implementation

1. For each third-party action above, resolve the SHA behind the currently used ref with `gh api repos/<owner>/<repo>/git/ref/tags/<tag> --jq .object.sha`. If that is an annotated tag, dereference it with `gh api repos/<owner>/<repo>/git/tags/<sha> --jq .object.sha`.
   - For branch refs (`dtolnay/rust-toolchain@stable`, `pypa/gh-action-pypi-publish@release/v1`), resolve the branch head (`git/ref/heads/<branch>`) and pin that. For `rust-toolchain`, keep choosing the toolchain via its `toolchain:` input so `stable` is still selected (`uses: dtolnay/rust-toolchain@<sha>` + `with: toolchain: stable`).
   - Record each `owner/repo@tag → sha` mapping in the PR description.
2. Rewrite the references as `uses: owner/repo@<40-char-sha> # <original ref>`. Apply this to every workflow, not just the secret-bearing ones, for consistency. Leave `actions/*` on tags, per the guide's lower-risk classification. Pin them too only if the user asks.
3. Add `.github/dependabot.yml` with `package-ecosystem: github-actions`, `directory: /`, and a weekly `schedule.interval`. Dependabot updates SHA pins and their trailing comments.
4. Validate the syntax locally: run `actionlint` if installed. Otherwise at least run `python -c "import yaml,sys;[yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/*.yml`.
5. Do not push. Report that verification needs a CI dispatch and ask the user before running `gh workflow run ci.yml`.

## Files to touch

- `.github/workflows/*.yml` (all files that reference third-party actions)
- `.github/dependabot.yml` (new)

## Verify

- `grep -hoE 'uses: [^ ]+' .github/workflows/*.yml | grep -vE '^uses: actions/' | grep -vE '@[0-9a-f]{40}$'` prints nothing. Every third-party reference is a 40-char SHA.
- Each pinned SHA resolves: for every pin, `gh api repos/<owner>/<repo>/commits/<sha> --jq .sha` returns it (script over the grep output).
- `.github/dependabot.yml` parses (`python -c "import yaml;yaml.safe_load(open('.github/dependabot.yml'))"`).
- `actionlint` passes, if available.
- After user-approved dispatch, `gh run view <id> --json conclusion` reports `success` for `ci.yml`.

## Rollback

Revert the workflow edits (tags are in the trailing comments) and delete `dependabot.yml`.
