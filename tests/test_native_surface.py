"""The `_native` module registers exactly the classes `_native.pyi` declares (ENH-054)."""

import ast
from pathlib import Path

from par_term_emu_core_rust import _native

STUB = Path(__file__).resolve().parents[1] / "python" / "par_term_emu_core_rust" / "_native.pyi"


def _stub_classes() -> set[str]:
    tree = ast.parse(STUB.read_text(encoding="utf-8"))
    return {node.name for node in tree.body if isinstance(node, ast.ClassDef)}


def _registered_classes() -> set[str]:
    # Same classification rule as scripts/generate_stubs.py main().
    return {
        name
        for name in dir(_native)
        if not name.startswith("_") and type(getattr(_native, name)).__name__ == "type"
    }


def test_registered_classes_match_the_stub() -> None:
    stub, live = _stub_classes(), _registered_classes()
    assert live - stub == set(), f"registered but missing from the stub: {sorted(live - stub)}"
    assert stub - live == set(), f"in the stub but not registered: {sorted(stub - live)}"


def test_registered_class_count_is_pinned() -> None:
    assert len(_registered_classes()) == 66
