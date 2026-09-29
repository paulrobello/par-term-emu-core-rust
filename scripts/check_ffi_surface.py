#!/usr/bin/env python3
"""ENH-027 — FFI surface documentation gate.

Every `#[no_mangle] extern "C" fn` exported from src/ffi.rs must:
  1. appear as a prototype in include/terminal_core.h (cbindgen-generated —
     `make ffi-header-check` covers regeneration drift, this covers the fn
     being absent entirely, e.g. emitted under a different name), and
  2. be documented in docs/FFI_GUIDE.md (backticked function name).

Fails with exit 1 listing every miss. DELIBERATELY_UNDOCUMENTED below is the
escape hatch for internals that must stay exported but are not embedding
surface; it should stay empty.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
FFI_RS = REPO_ROOT / "src" / "ffi.rs"
HEADER = REPO_ROOT / "include" / "terminal_core.h"
GUIDE = REPO_ROOT / "docs" / "FFI_GUIDE.md"

# Exported internals that are intentionally absent from the guide.
DELIBERATELY_UNDOCUMENTED: set[str] = set()

# #[no_mangle] \n pub (unsafe) extern "C" fn NAME
FN_RE = re.compile(
    r'#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(\w+)', re.DOTALL
)


def exported_functions() -> list[str]:
    names = FN_RE.findall(FFI_RS.read_text())
    if not names:
        # A regex that stopped matching anything is a broken gate, not a
        # clean pass — refuse to report zero.
        raise SystemExit(f'error: no extern "C" fns parsed from {FFI_RS}')
    return names


def main() -> int:
    fns = exported_functions()
    header = HEADER.read_text()
    guide = GUIDE.read_text()

    missing_header = [name for name in fns if not re.search(rf"\b{name}\s*\(", header)]
    missing_guide = [
        name
        for name in fns
        if name not in DELIBERATELY_UNDOCUMENTED and f"`{name}`" not in guide
    ]

    ok = True
    if missing_header:
        ok = False
        print("FFI functions missing from include/terminal_core.h:")
        for name in missing_header:
            print(f"  {name}  (run: make ffi-header)")
    if missing_guide:
        ok = False
        print("FFI functions missing from docs/FFI_GUIDE.md:")
        for name in missing_guide:
            print(
                f"  {name}  (document it, or add to DELIBERATELY_UNDOCUMENTED with a reason)"
            )

    if ok:
        print(f"ffi surface ok: {len(fns)} exported fns present in header and guide")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
