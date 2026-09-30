# ENH-034: Extend `check_ffi_surface.py` to structs, `TERM_*` constants, the ABI table and the stale "hand-written header" phrase

> Filed from the 2026-09-29 /opus-audit enhancement pass (cycle `audit-2026-09-29`). Board card: `[ENH-034]`.
> Sequencing: lands together with, or after, DOC-103 (the ABI table) and DOC-104 (the "hand-written" wording). Both new checks fail on HEAD by design.

**Priority**: medium · **Estimate**: S

## Goal

`scripts/check_ffi_surface.py` checks one thing: every exported `extern "C" fn` is in the header and backticked in `docs/FFI_GUIDE.md`. The rest of the C contract has no doc gate:
- The `typedef struct` names.
- The 56 `TERM_*` constants in the hand-written companion header.
- The ABI version history, which stopped at v2 while the shipped ABI is 3 (DOC-103).
- The docs that still call the cbindgen-generated header "hand-written" (DOC-104).

Extend the script so each of those drifts fails `make ffi-surface-check`, which `checkall` already runs.

## Current state

- **`scripts/check_ffi_surface.py`** (71 lines):
  - `FN_RE` over `src/ffi.rs`.
  - A header prototype check and a `` `name` `` guide check.
  - A refuse-zero-matches guard at `:35-41`.
  - A `DELIBERATELY_UNDOCUMENTED` escape hatch.
  - Wired as `ffi-surface-check` (`Makefile:363-364`) and in `checkall` (`:366`).
- **Structs.** `include/terminal_core.h` (cbindgen-generated) closes nine typedefs:
  - `TermRowRange`, `SharedCell`, `TermCursorState`, `TermModeState`, `TermKeyEvent`, `TermKeyOptions`, `SharedState`, `TerminalObserverVtable` (the `} Name;` form at `:62-335`).
  - The opaque `typedef struct Terminal Terminal;` (`:48`).
  - `term_event_cb` (`:291`) is a function-pointer typedef, not a struct.
  - Simulated on HEAD, all nine struct names are backticked in FFI_GUIDE, so this check passes today and exists to catch the next struct.
- **Constants.** `include/terminal_core_layout.h` has 56 `#define TERM_*` lines. Two points for the regex:
  - The include guard `PAR_TERM_EMU_CORE_TERMINAL_CORE_LAYOUT_H` also contains `TERM_`, so anchor at `^#define TERM_`.
  - FFI_GUIDE documents most constants by family wildcard, not one by one: `` `TERM_CELL_*` `` (`:184`), `` `TERM_MOUSE_MODE_*` `` (`:231`), `` `TERM_KEY_*` `` (`:256,258`). Line 3 also has a bare `` `TERM_*` ``, which must not count as coverage.
  - Simulated rule: a constant is covered when it is backticked literally, or when a backticked `PREFIX_*` with at least two underscores in `PREFIX` (so `TERM_CELL_`, not `TERM_`) is a prefix of it. Under that rule all 56 pass today.
- **ABI.**
  - `TERM_CORE_ABI_VERSION` is 3 in `src/ffi.rs:431` and `include/terminal_core_layout.h:25`, pinned to each other by `ffi::tests::abi_version_matches_header_macro` (`src/ffi.rs:1277-1290`).
  - FFI_GUIDE's `## ABI Version` section (`:414-434`) is prose: "Version 2 added …". There is no table and no v3 row. DOC-103 replaces it with a `| Version | Release | Adds |` table.
- **"hand-written header".**
  - The phrase appears at `docs/API_REFERENCE.md:2480` ("the hand-written header `include/terminal_core.h`") and as "hand-written header [`include/terminal_core.h`]" at `docs/RUST_USAGE.md:579` (DOC-104).
  - FFI_GUIDE.md:3 correctly says "hand-written companion `terminal_core_layout.h`". The check must not flag that sentence.
  - The planning directories `docs/opus/` and `docs/fable/`, including this file, quote the phrase and must be excluded.

## Implementation

1. **Refactor.** Split `main()` into small check functions, each returning a list of error strings. Keep the existing function-name check unchanged.
2. **`check_structs()`.**
   - Parse the typedef names from `HEADER` with `^\} (\w+);` plus `typedef struct (\w+) \1;`.
   - Fail closed on zero matches.
   - Require each name as `` `Name` `` in the guide, subject to a `DELIBERATELY_UNDOCUMENTED_TYPES: set[str] = set()` escape hatch.
3. **`check_constants()`.**
   - Parse `^#define (TERM_[A-Z0-9_]+)` (multiline) from `include/terminal_core_layout.h`, failing closed on zero.
   - Collect wildcards from the guide with `` `(TERM_[A-Z0-9_]*_)\*` ``, keeping only prefixes with `count("_") >= 2`.
   - A constant passes if `` `NAME` `` is in the guide or it starts with a kept wildcard prefix.
   - Report the misses with the hint "document it, or add a `PREFIX_*` family line".
4. **`check_abi_table()`.**
   - Read the Rust value with `pub const TERM_CORE_ABI_VERSION: u32 = (\d+);` from `src/ffi.rs`, failing closed.
   - Read the header value with `^#define TERM_CORE_ABI_VERSION (\d+)`, and fail if the two differ. The Rust test already pins this, but the script should not depend on `cargo test` having run.
   - Locate the `## ABI Version` section of FFI_GUIDE, up to the next `## `.
   - Require a table row matching `^\|\s*v?{N}\s*\|` for every version from 1 to N. Missing rows are listed, so a gap (v1 and v3 present, v2 absent) also fails.
5. **`check_stale_phrases()`.**
   - Walk `docs/*.md` (non-recursive, which skips `docs/opus/`, `docs/fable/` and `docs/research/`) plus `README.md`, `CONTRIBUTING.md` and `QUICKSTART.md`.
   - Fail on any line matching `(?i)hand-written header` (the phrase exactly as DOC-104 found it), and also on `(?i)hand-written[^.\n]{0,40}terminal_core\.h\b`, where `terminal_core\.h\b` cannot match `terminal_core_layout.h`.
   - Output is `path:line: <line>` with the hint "the header is cbindgen-generated (`make ffi-header`); only terminal_core_layout.h is hand-written".
   - FFI_GUIDE.md:3 names `terminal_core_layout.h` after "hand-written companion", so it passes both patterns. Assert this in the self-test (step 6).
6. **Add `--self-test`**, mirroring ENH-033's layout.
   - Take a `--root` argument and copy `src/ffi.rs`, both headers, and the scanned docs into a tempdir.
   - Assert that the unmodified copy passes (after DOC-103/104).
   - Then assert that each injected drift fails, naming the injected item:
     1. Delete the `` `TermKeyOptions` `` mention from the guide.
     2. Add `#define TERM_FAKE_THING 1` to the layout header.
     3. Delete the v3 table row.
     4. Append "the hand-written header `include/terminal_core.h`" to RUST_USAGE.md.
   - Also assert that FFI_GUIDE.md:3's "hand-written companion `terminal_core_layout.h`" sentence is not flagged.
   - Run it from the Makefile target: `ffi-surface-check: ; python3 scripts/check_ffi_surface.py && python3 scripts/check_ffi_surface.py --self-test`.
7. **Update the docstring** at the top of the script so it lists all five checks.
8. **Land it with the doc fixes.** If DOC-103 and DOC-104 are not merged yet, apply exactly their audited remedies in the same branch:
   - The v1/v2/v3 table in FFI_GUIDE (AUDIT.md DOC-103).
   - The "cbindgen-generated `include/terminal_core.h` (`make ffi-header`; …)" wording at both sites (AUDIT.md DOC-104).

## Files to touch

- `scripts/check_ffi_surface.py`
- `Makefile` (`ffi-surface-check` recipe gains `--self-test`)
- `docs/FFI_GUIDE.md`, `docs/API_REFERENCE.md`, `docs/RUST_USAGE.md` (only if DOC-103/DOC-104 have not landed)

## Verify

- Against HEAD f6535f2: `git worktree add <tmp> f6535f2`, then `python3 scripts/check_ffi_surface.py --root <tmp>` from the implementation checkout. It exits 1. It reports the missing ABI rows (v1, v2 and v3 have no table) and the two "hand-written header" lines at `docs/API_REFERENCE.md:2480` and `docs/RUST_USAGE.md:579`, and nothing for functions, structs or constants. Remove the worktree afterwards.
- After the DOC-103/104 fixes, `python3 scripts/check_ffi_surface.py` exits 0. Its summary line reports 9 structs, 56 constants and ABI version 3.
- `python3 scripts/check_ffi_surface.py --self-test` exits 0. Its four injected drifts each fail, naming the injected item, and FFI_GUIDE.md:3 is not flagged.
- In a temp copy (via `--root`), removing the `` `TERM_KEY_*` `` backticks makes the script exit 1 listing the `TERM_KEY_*` constants, not a pass through FFI_GUIDE:3's bare `` `TERM_*` ``.
- `make ffi-header-check ffi-surface-check` and `make checkall` are green.

## Rollback

Revert the script to its single-check form. The doc corrections are independent, so keep them.
