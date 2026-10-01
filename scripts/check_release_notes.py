#!/usr/bin/env python3
"""Release-notes completeness check (ENH-036).

Every ``feat``/``fix`` commit since the base of the top CHANGELOG section must
be referenced by that section: either a backticked 7-40-hex SHA prefix or a
card ID from the commit subject. Commits carrying a ``Changelog: skip``
trailer are exempt.

With ``--write`` it also maintains the top compare-link definitions
(``[<ver>]:``, plus ``[Unreleased]:`` when that heading exists). Backfilling
older links is DOC-126 and stays out of scope.

Exit codes: 0 clean, 1 missing entries, 2 infrastructure error (missing base
tag, no derivable base, git failure).
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_SLUG = "paulrobello/par-term-emu-core-rust"
HEADING_RE = re.compile(r"^## \[([^\]]+)\]", re.MULTILINE)
SUBJECT_RE = re.compile(r"^(feat|fix)(\([^)]*\))?!?:")
CARD_ID_RE = re.compile(r"\b((?:ENH|QA|SEC|ARC|DOC)-\d+)\b")
SHA_TOKEN_RE = re.compile(r"`([0-9a-f]{7,40})`")
RELEASE_TAG_RE = re.compile(r"^v\d+\.\d+\.\d+$")


def run_git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=root, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"git {args[0]} failed")
    return result.stdout


def parse_sections(text: str) -> list[tuple[str, int]]:
    """Section heading names with their character offsets, in order."""
    return [(m.group(1), m.start()) for m in HEADING_RE.finditer(text)]


def section_text(text: str, start: int) -> str:
    """Body of the section whose heading starts at offset ``start``."""
    nxt = HEADING_RE.search(text, start + 1)
    return text[start : nxt.start()] if nxt else text[start:]


def tag_exists(root: Path, ref: str) -> bool:
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode == 0


def latest_release_tag(root: Path) -> str:
    """Newest ``vX.Y.Z`` tag.

    Only consulted when a single-versioned-heading CHANGELOG leaves nothing
    to derive the base from. The release pattern cannot match a tag like
    ``bench-baseline``, so the "latest tag picks the wrong ref" failure mode
    from the card notes cannot occur here.
    """
    tags = [
        t.strip()
        for t in run_git(root, "tag", "--list").splitlines()
        if RELEASE_TAG_RE.match(t.strip())
    ]
    if not tags:
        raise RuntimeError(
            "cannot derive the base: the CHANGELOG has a single versioned "
            "heading and no v*.*.* tags exist"
        )
    return max(tags, key=lambda t: tuple(int(p) for p in t[1:].split(".")))


def derive_checked_section(text: str, root: Path) -> tuple[str, str, str, str | None]:
    """Return (checked, base, head, top_versioned) from the CHANGELOG layout.

    - Non-empty ``[Unreleased]``: check it against
      ``v<first versioned heading>..HEAD``.
    - Otherwise check the first versioned section ``X`` against
      ``v<second versioned heading>..vX`` (``..HEAD`` when ``vX`` is not
      tagged locally).
    """
    sections = parse_sections(text)
    if not sections:
        raise RuntimeError("CHANGELOG.md has no '## [x.y.z]' headings")

    names = [name for name, _ in sections]
    unreleased_is_top = names[0] == "Unreleased"
    if unreleased_is_top and any(
        line.strip()
        # [1:] skips the heading line itself, which is always non-blank.
        for line in section_text(text, sections[0][1]).splitlines()[1:]
    ):
        if len(sections) < 2:
            raise RuntimeError(
                "non-empty [Unreleased] but no versioned heading below it"
            )
        top_versioned = sections[1][0]
        return "Unreleased", f"v{top_versioned}", "HEAD", top_versioned

    start = 1 if unreleased_is_top else 0
    top_versioned = sections[start][0]
    if len(sections) >= start + 2:
        base = f"v{sections[start + 1][0]}"
    else:
        base = latest_release_tag(root)
    head = f"v{top_versioned}" if tag_exists(root, f"v{top_versioned}") else "HEAD"
    return top_versioned, base, head, top_versioned


def resolve_base(root: Path, base: str) -> None:
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"{base}^{{commit}}"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        print(
            f"ERROR: tag {base} not found; run `git fetch --tags`",
            file=sys.stderr,
        )
        sys.exit(2)


def candidates(root: Path, base: str, head: str) -> list[tuple[str, str]]:
    fmt = "%H%x09%s%x09%(trailers:key=Changelog,valueonly,separator=%x2C)"
    out = run_git(root, "log", f"--format={fmt}", f"{base}..{head}")
    commits: list[tuple[str, str]] = []
    for line in out.splitlines():
        if not line.strip():
            continue
        sha, subject, trailer = (line.split("\t", 2) + [""])[:3]
        if not SUBJECT_RE.match(subject):
            continue
        if trailer.strip().lower() == "skip":
            continue
        commits.append((sha, subject))
    return commits


def referenced(section: str, sha: str, subject: str) -> bool:
    if any(sha.startswith(token) for token in SHA_TOKEN_RE.findall(section)):
        return True
    for match in CARD_ID_RE.finditer(subject):
        card_id = match.group(1)
        if re.search(rf"\b{re.escape(card_id)}\b", section):
            return True
    return False


def link_def_url(lines: list[str], key: str) -> str | None:
    prefix = f"[{key}]: "
    for line in lines:
        if line.startswith(prefix):
            return line[len(prefix) :].strip()
    return None


def upsert_link(lines: list[str], key: str, url: str) -> bool:
    """Insert or update the ``[<key>]: <url>`` definition. Returns changed."""
    prefix = f"[{key}]: "
    for i, line in enumerate(lines):
        if line.startswith(prefix):
            if line[len(prefix) :].strip() == url:
                return False
            lines[i] = f"{prefix}{url}\n"
            return True
    for i, line in enumerate(lines):
        if re.match(r"^\[\d", line):
            lines.insert(i, f"{prefix}{url}\n")
            return True
    lines.append(f"{prefix}{url}\n")
    return True


def write_links(changelog: Path, text: str, root: Path) -> str:
    """Maintain the top compare links; return the (possibly new) file text."""
    lines = text.splitlines(keepends=True)
    sections = parse_sections(text)
    names = [name for name, _ in sections]
    top_versioned = next((n for n in names if n != "Unreleased"), None)
    if top_versioned is None:
        raise RuntimeError("no versioned heading to link")
    idx = names.index(top_versioned)
    if len(names) >= idx + 2:
        prev = sections[idx + 1][0]
    else:
        # Single versioned heading: same fallback the base derivation uses.
        prev = latest_release_tag(root)[1:]
    version_url = f"https://github.com/{REPO_SLUG}/compare/v{prev}...v{top_versioned}"
    changed = []
    if upsert_link(lines, top_versioned, version_url):
        changed.append(f"[{top_versioned}]: {version_url}")
    if "Unreleased" in names:
        unreleased_url = (
            f"https://github.com/{REPO_SLUG}/compare/v{top_versioned}...HEAD"
        )
        if upsert_link(lines, "Unreleased", unreleased_url):
            changed.append(f"[Unreleased]: {unreleased_url}")
    for entry in changed:
        print(f"WROTE {entry}")
    missing_older = [
        n
        for n in names
        if n != "Unreleased" and n != top_versioned and link_def_url(lines, n) is None
    ]
    if missing_older:
        print(
            f"notice: {len(missing_older)} older sections have no compare "
            "link (backfill is DOC-126, out of scope here)"
        )
    new_text = "".join(lines)
    if new_text != text:
        changelog.write_text(new_text)
    return new_text


def check(root: Path, base: str | None, head: str | None) -> int:
    changelog = root / "CHANGELOG.md"
    text = changelog.read_text()
    checked, derived_base, derived_head, _ = derive_checked_section(text, root)
    base = base or derived_base
    head = head or derived_head
    resolve_base(root, base)

    sections = dict(parse_sections(text))
    section = section_text(text, sections[checked])
    commits = candidates(root, base, head)
    missing = [
        (sha, subject)
        for sha, subject in commits
        if not referenced(section, sha, subject)
    ]

    print(
        f"release-check: {base}..{head} — checking [{checked}] against "
        f"{len(commits)} feat/fix commit(s)"
    )
    for sha, subject in missing:
        print(f"MISSING {sha[:7]} {subject}")
    if missing:
        print(
            "hint: cite the SHA in the section's bullet (backticked), or add "
            "a `Changelog: skip` trailer with `git commit --amend` before "
            "tagging"
        )
    return 1 if missing else 0


def self_test() -> int:
    """Build a throwaway repo and assert the documented behaviors."""
    failures: list[str] = []

    def expect(cond: bool, label: str, detail: str = "") -> None:
        print(f"  {'PASS' if cond else 'FAIL'}  {label}")
        if not cond and detail:
            print(f"        {detail}")
        if not cond:
            failures.append(label)

    script = Path(__file__).resolve()

    def git(root: Path, *args: str) -> str:
        argv = ["git"]
        if args and args[0] == "commit":
            argv += ["-c", "user.name=t", "-c", "user.email=t@t"]
        argv += list(args)
        result = subprocess.run(
            argv, cwd=str(root), capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            raise RuntimeError(f"git {args}: {result.stderr}")
        return result.stdout

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        git(root, "init", "-q")
        (root / "base.txt").write_text("base\n")
        git(root, "add", "-A")
        git(root, "commit", "-q", "-m", "initial")
        git(root, "tag", "v0.1.0")

        def commit(msg: str, body: str = "") -> str:
            (root / "f.txt").write_text(msg + "\n")
            git(root, "add", "-A")
            args = ["commit", "-q", "-m", msg]
            if body:
                args += ["-m", body]
            git(root, *args)
            return git(root, "rev-parse", "HEAD").strip()

        sha_cited = commit("feat(api): sha cited feature")
        commit("fix(core): ENH-050 card cited fix")
        commit("feat: internal-only feature", "Changelog: skip")
        uncited = commit("feat(grid): uncited feature")
        commit("docs: update readme")
        git(root, "tag", "bench-baseline")

        changelog = root / "CHANGELOG.md"
        # Same scripts/ layout as the real repo so the default --root
        # derivation (script's parent's parent) resolves to the repo root.
        script_copy = root / "scripts" / "check_release_notes.py"
        script_copy.parent.mkdir(exist_ok=True)
        script_copy.write_text(script.read_text())
        changelog.write_text(
            "# Changelog\n\n## [0.2.0]\n\n### Added\n"
            f"- sha cited (`{sha_cited[:7]}`)\n"
            "- card cited ENH-050\n- internal skipped\n"
        )

        def run_in_root(*args: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                [sys.executable, str(script_copy), *args],
                cwd=str(root),
                capture_output=True,
                text=True,
                check=False,
            )

        print("scenario: single versioned section, one uncited commit")
        r = run_in_root()
        expect(r.returncode == 1, "exit 1 with a missing commit", r.stderr)
        expect(
            "v0.1.0..HEAD" in r.stdout,
            "base is v0.1.0 (not bench-baseline)",
            r.stdout,
        )
        misses = [ln for ln in r.stdout.splitlines() if ln.startswith("MISSING")]
        expect(
            len(misses) == 1 and uncited[:7] in misses[0],
            "exactly the uncited commit is reported",
            r.stdout,
        )

        print("scenario: --write is idempotent")
        r1 = run_in_root("--write")
        first = changelog.read_bytes()
        links = [ln for ln in first.decode().splitlines() if ln.startswith("[0.2.0]:")]
        expected_link = (
            f"[0.2.0]: https://github.com/{REPO_SLUG}/compare/v0.1.0...v0.2.0"
        )
        expect(
            len(links) == 1 and links[0] == expected_link,
            "one correct [0.2.0]: link written",
            first.decode(),
        )
        r2 = run_in_root("--write")
        expect(
            changelog.read_bytes() == first,
            "second --write is byte-identical",
        )
        expect(
            r1.returncode == r2.returncode == 1,
            "write runs still report the miss",
        )

        print("scenario: missing base tag fails closed")
        changelog.write_text(
            "# Changelog\n\n## [0.3.0]\n\n### Added\n- thing\n\n"
            "## [0.2.0]\n\n### Added\n- older\n"
        )
        r = run_in_root()
        expect(
            r.returncode == 2,
            "exit 2 for a missing base tag",
            r.stdout + r.stderr,
        )
        expect(
            "git fetch --tags" in (r.stderr + r.stdout),
            "fetch hint printed",
            r.stdout + r.stderr,
        )

        print("scenario: non-empty [Unreleased] checked since the top tag")
        git(root, "tag", "v0.2.0")
        uncited2 = commit("feat(api): post release feature")
        changelog.write_text(
            "# Changelog\n\n## [Unreleased]\n\n### Added\n- wip\n\n"
            "## [0.2.0]\n\n### Added\n- released\n"
        )
        r = run_in_root()
        expect(
            r.returncode == 1,
            "exit 1 with the new commit missing",
            r.stdout + r.stderr,
        )
        expect("v0.2.0..HEAD" in r.stdout, "base is v0.2.0", r.stdout)
        misses = [ln for ln in r.stdout.splitlines() if ln.startswith("MISSING")]
        expect(
            len(misses) == 1 and uncited2[:7] in misses[0],
            "only post-tag commits are candidates",
            r.stdout,
        )

    if failures:
        print(f"self-test FAILED: {failures}")
        return 1
    print("self-test: all scenarios passed")
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: this script's repo)",
    )
    parser.add_argument("--base", help="override the derived base tag")
    parser.add_argument("--head", help="override the default head ref")
    parser.add_argument(
        "--write",
        action="store_true",
        help="also maintain the top compare-link definitions",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run the built-in throwaway-repo test suite",
    )
    args = parser.parse_args()

    if args.self_test:
        sys.exit(self_test())

    root: Path = args.root.resolve()
    try:
        status = check(root, args.base, args.head)
        if args.write:
            write_links(
                root / "CHANGELOG.md",
                (root / "CHANGELOG.md").read_text(),
                root,
            )
    except RuntimeError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)
    sys.exit(status)


if __name__ == "__main__":
    main()
