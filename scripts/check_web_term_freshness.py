#!/usr/bin/env python3
"""QA-004: fail when the committed `web_term/` build is stale.

`web_term/` is the committed static export the streamer serves from the
repo (a serve-from-repo deployment model, kept by decision). Its inputs
are the `web-terminal-frontend/` sources, so a frontend change that lands
without `make web-build-static` ships a stale UI while the Rust side has
moved on — the drift no other gate covers (QA-004).

The gate compares the newest mtime under the frontend sources (excluding
`node_modules`, `.next`, and `out/`, which are install/dev/build
artifacts, not inputs) against the newest mtime under `web_term/`. It
fails when any input is newer than the committed build by more than the
checkout tolerance, telling the developer to run `make web-build-static`.
`bun.lock` counts as an input: a dependency change is a rebuild.

`--self-test` renders synthetic trees and requires the drift (an input
newer than the build, and a missing build) to fail naming the remedy.
"""

from __future__ import annotations

import argparse
import os
import tempfile
import time
from pathlib import Path

from _gate_common import ROOT, expect_failure, fail

FRONTEND_REL = "web-terminal-frontend"
BUILD_REL = "web_term"
# Install/dev/build artifacts inside the frontend tree that are not build
# inputs: `bun install` churn and dev-server output must not trip the gate.
FRONTEND_EXCLUDES = {"node_modules", ".next", "out"}
# A fresh git clone writes every file with near-identical mtimes in checkout
# order, so `web_term` can read microseconds older than its own inputs. Only
# a real post-build source edit exceeds this.
CHECKOUT_TOLERANCE_SECONDS = 2.0
REMEDY = "run: make web-build-static"


def newest_mtime(root: Path, excludes: set[str]) -> float:
    """Newest mtime under `root`, skipping any path with a part in `excludes`."""
    newest = 0.0
    for path in sorted(root.rglob("*")):
        if any(part in excludes for part in path.relative_to(root).parts):
            continue
        try:
            newest = max(newest, path.stat().st_mtime)
        except OSError:
            continue
    return newest


def check(frontend: Path, build: Path, tolerance: float) -> None:
    """Fail when the committed build is stale relative to the frontend inputs."""
    if not build.is_dir():
        fail(f"{BUILD_REL}/ is missing — {REMEDY}")
    build_newest = newest_mtime(build, set())
    frontend_newest = newest_mtime(frontend, FRONTEND_EXCLUDES)
    if build_newest == 0.0:
        fail(f"{BUILD_REL}/ is empty — {REMEDY}")
    if frontend_newest > build_newest + tolerance:
        delta = frontend_newest - build_newest
        fail(
            f"{BUILD_REL}/ is stale: a web-terminal-frontend input is "
            f"{delta:.0f}s newer than the newest committed build file — {REMEDY}"
        )


def self_test() -> None:
    """The gate must fail on a newer input and on a missing build, else pass."""
    past = time.time() - 60
    now = time.time()
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)

        # Happy path: build newer than its input; a node_modules churn file
        # newer still must not trip the gate.
        frontend = tmp / FRONTEND_REL
        (frontend / "app").mkdir(parents=True)
        (frontend / "app" / "page.tsx").write_text("export {}", encoding="utf-8")
        os.utime(frontend / "app" / "page.tsx", (past, past))
        build = tmp / BUILD_REL
        build.mkdir()
        (build / "index.html").write_text("ok", encoding="utf-8")
        os.utime(build / "index.html", (now, now))
        (frontend / "node_modules").mkdir()
        (frontend / "node_modules" / "pkg.js").write_text("x", encoding="utf-8")
        os.utime(frontend / "node_modules" / "pkg.js", (now + 5, now + 5))
        check(frontend, build, CHECKOUT_TOLERANCE_SECONDS)

        # Drift: a frontend input edited after the last build must fail
        # naming the remedy, and so must a missing build directory.
        newer_input = frontend / "app" / "page.tsx"
        os.utime(newer_input, (now + 10, now + 10))
        expect_failure(
            lambda: check(frontend, build, CHECKOUT_TOLERANCE_SECONDS),
            REMEDY,
        )
        expect_failure(
            lambda: check(frontend, tmp / "missing", CHECKOUT_TOLERANCE_SECONDS),
            "is missing",
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="pin gate invariants on synthetic trees",
    )
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("web-term-freshness self-test ok")
        return 0

    check(ROOT / FRONTEND_REL, ROOT / BUILD_REL, CHECKOUT_TOLERANCE_SECONDS)
    print(f"web_term: committed build is fresh against {FRONTEND_REL}/")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
