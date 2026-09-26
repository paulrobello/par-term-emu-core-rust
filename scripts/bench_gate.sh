#!/usr/bin/env bash
# Interleaved A/B benchmark gate driver (ENH-016). See docs/BENCHMARKING.md.
#
# Builds the bench binary for HEAD and for the bench-baseline tag, runs the
# two binaries in alternating rounds (A/B/A/B — absolute numbers swing ~2x
# with machine state, so only interleaved pairs are comparable), and hands
# the round outputs to scripts/bench_compare.py for the verdict table.
#
# On a clean compare (and BENCH_ROTATE_TAG=1) the baseline tag is moved to
# HEAD via a tag-only force push. Verdicts are advisory: the exit code is 0
# even when regressions are flagged (set BENCH_FAIL_ON_REGRESSION=1 to fail).
#
# Environment knobs (all optional):
#   BENCH_ROUNDS        interleaved rounds per binary (default 3)
#   BENCH_THRESHOLD     head/base ratio flagged as regression (default 1.2)
#   BENCH_BASELINE_TAG  git tag holding the baseline commit (default bench-baseline)
#   BENCH_GATE_DIR      scratch dir (default target/bench-gate, gitignored)
#   BENCH_ROTATE_TAG    1 = move the baseline tag to HEAD on a clean compare
#   BENCH_FAIL_ON_REGRESSION  1 = non-zero exit when regressions are flagged
#   BENCH_FILTER        single criterion positional filter (e.g. "kitty") for
#                       quick local checks; CI runs the full suite
set -euo pipefail

ROUNDS="${BENCH_ROUNDS:-3}"
THRESHOLD="${BENCH_THRESHOLD:-1.2}"
TAG="${BENCH_BASELINE_TAG:-bench-baseline}"
WORK="${BENCH_GATE_DIR:-$PWD/target/bench-gate}"
FEATURES=(--no-default-features --features rust-only)

REPO_DIR="$PWD"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# The baseline checkout below replaces the working tree, which would swap
# this script and bench_compare.py out from under a running shell — run the
# rest from a copy in the scratch dir instead.
if [[ "$SCRIPT_DIR" != "$WORK/scripts" ]]; then
    mkdir -p "$WORK/scripts"
    cp "$SCRIPT_DIR/bench_gate.sh" "$SCRIPT_DIR/bench_compare.py" "$WORK/scripts/"
    exec bash "$WORK/scripts/bench_gate.sh" "$@"
fi

if ! command -v cargo >/dev/null 2>&1 || ! command -v python3 >/dev/null 2>&1; then
    echo "bench_gate: cargo and python3 are required" >&2
    exit 2
fi
# Untracked files survive the baseline checkout (they are not in the baseline
# tree); tracked modifications would be discarded by checkout --force.
if [[ -n "$(git -C "$REPO_DIR" status --porcelain -uno)" ]]; then
    echo "bench_gate: refusing to run with tracked modifications (baseline checkout would discard them)" >&2
    exit 2
fi

build_current_binary() { # $1 = destination
    local dest="$1"
    (cd "$REPO_DIR" && cargo bench --no-run "${FEATURES[@]}" --message-format=json) \
        > "$WORK/build.json" 2> "$WORK/build.log" || {
            cat "$WORK/build.log" >&2
            echo "bench_gate: cargo bench --no-run failed" >&2
            exit 2
        }
    local bin
    bin="$(python3 - "$WORK/build.json" <<'PY'
import json, sys
for line in open(sys.argv[1], encoding="utf-8"):
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except json.JSONDecodeError:
        continue
    if msg.get("reason") == "compiler-artifact" and msg.get("executable"):
        if "bench" in (msg.get("target") or {}).get("kind", []):
            print(msg["executable"])
            break
PY
    )"
    if [[ -z "$bin" || ! -x "$bin" ]]; then
        echo "bench_gate: could not locate the bench binary artifact" >&2
        exit 2
    fi
    cp "$bin" "$dest"
    echo "bench_gate: built $(git -C "$REPO_DIR" rev-parse --short HEAD) -> $dest"
}

run_round() { # $1 = who (base|head), $2 = binary, $3 = round number
    local who="$1" bin="$2" round="$3"
    local home_dir="$WORK/criterion/$who/r$round"
    mkdir -p "$home_dir"
    echo "bench_gate: round $round — $who"
    local args=(--bench --noplot)
    if [[ -n "${BENCH_FILTER:-}" ]]; then
        args+=("$BENCH_FILTER") # criterion takes exactly one positional filter
    fi
    # --bench is required when invoking a criterion binary directly, or it
    # runs a single test iteration and exits without measuring.
    CRITERION_HOME="$home_dir" "$bin" "${args[@]}"
}

HEAD_SHA="$(git -C "$REPO_DIR" rev-parse HEAD)"
ORIG_REF="$(git -C "$REPO_DIR" symbolic-ref -q --short HEAD || git -C "$REPO_DIR" rev-parse HEAD)"
build_current_binary "$WORK/bench-head"

# Fetch the baseline tag from origin when missing locally; absence anywhere
# means first-run bootstrap (establish the tag, no compare).
BASE_SHA=""
if ! git -C "$REPO_DIR" rev-parse -q --verify "refs/tags/$TAG^{commit}" >/dev/null 2>&1; then
    git -C "$REPO_DIR" fetch --force origin "refs/tags/$TAG:refs/tags/$TAG" 2>"$WORK/fetch.log" || true
fi
if git -C "$REPO_DIR" rev-parse -q --verify "refs/tags/$TAG^{commit}" >/dev/null 2>&1; then
    BASE_SHA="$(git -C "$REPO_DIR" rev-parse "$TAG^{commit}")"
fi

BOOTSTRAP=0
if [[ -n "$BASE_SHA" && "$BASE_SHA" != "$HEAD_SHA" ]]; then
    echo "bench_gate: comparing HEAD $HEAD_SHA against baseline $BASE_SHA ($TAG)"
    git -C "$REPO_DIR" checkout --quiet --detach "$BASE_SHA"
    build_current_binary "$WORK/bench-base"
    git -C "$REPO_DIR" checkout --quiet --force "$ORIG_REF"
elif [[ -z "$BASE_SHA" ]]; then
    BOOTSTRAP=1
    echo "bench_gate: no $TAG tag found — bootstrap run (establishes the baseline, no compare)"
else
    echo "bench_gate: baseline tag already at HEAD — self-compare verify run"
    cp "$WORK/bench-head" "$WORK/bench-base"
fi

rm -rf "$WORK/criterion"
for r in $(seq 1 "$ROUNDS"); do
    if [[ "$BOOTSTRAP" == "1" ]]; then
        run_round head "$WORK/bench-head" "$r"
    elif (( r % 2 == 1 )); then
        run_round base "$WORK/bench-base" "$r"
        run_round head "$WORK/bench-head" "$r"
    else
        run_round head "$WORK/bench-head" "$r"
        run_round base "$WORK/bench-base" "$r"
    fi
done

COMPARE_ARGS=(--base "$WORK/criterion/base" --head "$WORK/criterion/head" --threshold "$THRESHOLD")
if [[ "${BENCH_FAIL_ON_REGRESSION:-0}" == "1" ]]; then
    COMPARE_ARGS+=(--fail-on-regression)
fi
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    COMPARE_ARGS+=(--summary "$GITHUB_STEP_SUMMARY")
fi

REGRESSIONS=0
if [[ "$BOOTSTRAP" == "1" ]]; then
    echo "bench_gate: bootstrap complete — tag $TAG will point at $HEAD_SHA"
else
    python3 "$WORK/scripts/bench_compare.py" "${COMPARE_ARGS[@]}" | tee "$WORK/compare.log"
    REGRESSIONS="$(tail -n 1 "$WORK/compare.log" | cut -d= -f2)"
fi

if [[ "${BENCH_ROTATE_TAG:-0}" == "1" && "$REGRESSIONS" == "0" ]]; then
    git -C "$REPO_DIR" tag -f "$TAG" "$HEAD_SHA"
    # Tag-only force push; the branch history is never rewritten (docs/BENCHMARKING.md).
    git -C "$REPO_DIR" push --force origin "refs/tags/$TAG"
    echo "bench_gate: rotated $TAG -> $HEAD_SHA"
fi

echo "bench_gate: done (regressions=$REGRESSIONS, bootstrap=$BOOTSTRAP)"
