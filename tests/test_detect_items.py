#!/usr/bin/env python3
"""DetectedItem row/col ordering (QA-220).

The Rust core constructs ``DetectedItem`` variants as ``(text, col, row)``,
but the Python bindings destructured them as ``(text, row, col)``, returning
swapped coordinates. Every assertion below pins a position where
``row != col``, so a regression to the swapped order fails.
"""

from par_term_emu_core_rust import Terminal


def test_detect_urls_position():
    term = Terminal(80, 24)
    line = "see https://example.com now"
    term.process_str(line + "\r\n")
    items = term.detect_urls()
    assert len(items) == 1
    item = items[0]
    assert item.item_type == "url"
    assert item.text == "https://example.com"
    assert item.row == 0
    assert item.col == line.index("https://")


def test_detect_urls_row_matches_screen_row():
    term = Terminal(80, 24)
    term.process_str("row zero\r\nrow one\r\nsee https://example.com now\r\n")
    items = term.detect_urls()
    assert [(i.row, i.col) for i in items] == [(2, 4)]


def test_detect_file_paths_position_and_line_number():
    term = Terminal(80, 24)
    line = "error in /usr/src/app/main.py:42"
    term.process_str(line + "\r\n")
    items = term.detect_file_paths()
    assert len(items) == 1
    item = items[0]
    assert item.item_type == "filepath"
    assert item.text == "/usr/src/app/main.py"
    assert item.line_number == 42
    assert item.row == 0
    assert item.col == line.index("/usr/src/app/main.py")


def test_detect_semantic_items_positions():
    term = Terminal(80, 24)
    # Keep the line under 80 columns: the detector scans row by row, and a
    # wrapped 40-char hash never sits whole inside one row.
    git_hash = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0"
    line = f"admin@example.com 192.168.1.10 {git_hash}"
    term.process_str(line + "\r\n")
    items = term.detect_semantic_items()
    by_type: dict[str, list] = {}
    for item in items:
        by_type.setdefault(item.item_type, []).append(item)
    assert by_type["email"][0].col == line.index("admin@example.com")
    assert by_type["ip"][0].col == line.index("192.168.1.10")
    assert by_type["git_hash"][0].col == line.index(git_hash)
    assert all(i.row == 0 for i in items)
