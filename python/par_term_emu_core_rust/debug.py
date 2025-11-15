"""
Comprehensive debugging infrastructure for par-term-emu Python code

Controlled by DEBUG_LEVEL environment variable:
- 0 or unset: No debugging
- 1: Errors only
- 2: Info level (render calls, widget lifecycle)
- 3: Debug level (detailed render info, generation tracking)
- 4: Trace level (every operation, full state dumps)

All output goes to par_term_emu_debug_python.log in the system temp directory
to avoid breaking TUI apps (Unix/macOS: /tmp, Windows: %TEMP%)
"""

import os
import sys
import tempfile
import time
from datetime import datetime
from enum import IntEnum
from pathlib import Path
from threading import Lock
from typing import Any, Dict, Optional

DEBUG_FILE = Path(tempfile.gettempdir()) / "par_term_emu_debug_python.log"


class DebugLevel(IntEnum):
    """Debug verbosity levels"""

    OFF = 0
    ERROR = 1
    INFO = 2
    DEBUG = 3
    TRACE = 4


class DebugLogger:
    """File-based debug logger (singleton)"""

    _instance: Optional["DebugLogger"] = None
    _lock = Lock()

    def __new__(cls):
        if cls._instance is None:
            with cls._lock:
                if cls._instance is None:
                    cls._instance = super().__new__(cls)
                    cls._instance._initialize()
        return cls._instance

    def _initialize(self):
        """Initialize the logger"""
        self.level = self._get_debug_level()
        self.file_handle = None

        if self.level != DebugLevel.OFF:
            try:
                # Open in write mode to truncate (separate log from Rust)
                self.file_handle = open(DEBUG_FILE, "w", buffering=1)
                self._write_header()
            except Exception as e:
                print(f"Failed to open debug log file: {e}", file=sys.stderr)
                self.level = DebugLevel.OFF

    def _get_debug_level(self) -> DebugLevel:
        """Get debug level from environment"""
        try:
            level_str = os.environ.get("DEBUG_LEVEL", "0")
            level = int(level_str)
            return DebugLevel(level) if 0 <= level <= 4 else DebugLevel.OFF
        except (ValueError, KeyError):
            return DebugLevel.OFF

    def _write_header(self):
        """Write session header"""
        if self.file_handle:
            separator = "=" * 80
            timestamp = datetime.now().isoformat()
            self.file_handle.write(
                f"\n{separator}\n"
                f"par-term-emu Python debug session started at {timestamp} (level={self.level.name})\n"
                f"{separator}\n"
            )
            self.file_handle.flush()

    def _get_timestamp(self) -> str:
        """Get current timestamp with microsecond precision"""
        return f"{time.time():.6f}"

    def log(self, level: DebugLevel, category: str, message: str):
        """Write a log message"""
        if level <= self.level and self.file_handle:
            timestamp = self._get_timestamp()
            level_str = level.name.ljust(5)
            line = f"[{timestamp}] [{level_str}] [{category}] {message}\n"
            self.file_handle.write(line)
            self.file_handle.flush()

    def is_enabled(self, level: DebugLevel) -> bool:
        """Check if logging is enabled for this level"""
        return level <= self.level

    def __del__(self):
        """Close the log file"""
        if self.file_handle:
            try:
                self.file_handle.close()
            except Exception:  # noqa: S110
                pass


# Global logger instance
_logger = DebugLogger()


def is_enabled(level: DebugLevel) -> bool:
    """Check if debugging is enabled at the given level"""
    return _logger.is_enabled(level)


def log(level: DebugLevel, category: str, message: str):
    """Log a message at the specified level"""
    _logger.log(level, category, message)


# Convenience functions
def debug_error(category: str, message: str):
    """Log an error message"""
    log(DebugLevel.ERROR, category, message)


def debug_info(category: str, message: str):
    """Log an info message"""
    log(DebugLevel.INFO, category, message)


def debug_log(category: str, message: str):
    """Log a debug message"""
    log(DebugLevel.DEBUG, category, message)


def debug_trace(category: str, message: str):
    """Log a trace message"""
    log(DebugLevel.TRACE, category, message)


# Specific logging functions for common operations
def log_render_call(widget_id: str, line_number: int, generation: int):
    """Log a render_line() call"""
    if is_enabled(DebugLevel.DEBUG):
        log(
            DebugLevel.DEBUG,
            "RENDER",
            f"widget={widget_id} line={line_number} gen={generation}",
        )


def log_render_content(widget_id: str, line_number: int, content: str):
    """Log rendered content"""
    if is_enabled(DebugLevel.TRACE):
        # Escape control characters for logging
        escaped = content.encode("unicode_escape").decode("ascii")
        log(
            DebugLevel.TRACE,
            "RENDER_CONTENT",
            f"widget={widget_id} line={line_number} content=[{escaped}]",
        )


def log_generation_check(widget_id: str, old_gen: int, new_gen: int):
    """Log generation counter check"""
    if is_enabled(DebugLevel.DEBUG):
        status = "CHANGED" if old_gen != new_gen else "unchanged"
        log(
            DebugLevel.DEBUG,
            "GENERATION",
            f"widget={widget_id} {old_gen} -> {new_gen} ({status})",
        )


def log_widget_lifecycle(widget_id: str, event: str):
    """Log widget lifecycle events (mount, unmount, resize, etc.)"""
    if is_enabled(DebugLevel.INFO):
        log(DebugLevel.INFO, "LIFECYCLE", f"widget={widget_id} event={event}")


def log_snapshot(label: str, content: str):
    """Log a full content snapshot"""
    if is_enabled(DebugLevel.TRACE) and _logger.file_handle:
        separator = "-" * 80
        _logger.file_handle.write(
            f"\n{separator}\nSNAPSHOT: {label}\n{separator}\n{content}\n{separator}\n"
        )
        _logger.file_handle.flush()


def log_terminal_state(widget_id: str, state: Dict[str, Any]):
    """Log terminal state information"""
    if is_enabled(DebugLevel.DEBUG):
        state_str = ", ".join(f"{k}={v}" for k, v in state.items())
        log(DebugLevel.DEBUG, "TERM_STATE", f"widget={widget_id} {state_str}")


def log_textual_event(widget_id: str, event_type: str, details: str = ""):
    """Log Textual event processing"""
    if is_enabled(DebugLevel.DEBUG):
        msg = f"widget={widget_id} event={event_type}"
        if details:
            msg += f" {details}"
        log(DebugLevel.DEBUG, "TEXTUAL_EVENT", msg)


def log_get_line_cells_call(widget_id: str, line: int, generation: int):
    """Log get_line_cells() API call"""
    if is_enabled(DebugLevel.TRACE):
        log(
            DebugLevel.TRACE,
            "GET_LINE_CELLS",
            f"widget={widget_id} line={line} gen={generation}",
        )


def log_screen_corruption(widget_id: str, line: int, content: str):
    """Log potential screen corruption"""
    if is_enabled(DebugLevel.ERROR):
        escaped = content.encode("unicode_escape").decode("ascii")
        log(
            DebugLevel.ERROR,
            "CORRUPTION",
            f"widget={widget_id} line={line} suspicious_content=[{escaped}]",
        )


# Context manager for operation timing
class LogTimer:
    """Context manager for timing operations"""

    def __init__(
        self, category: str, operation: str, level: DebugLevel = DebugLevel.DEBUG
    ):
        self.category = category
        self.operation = operation
        self.level = level
        self.start_time = None

    def __enter__(self):
        if is_enabled(self.level):
            self.start_time = time.time()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if is_enabled(self.level) and self.start_time:
            duration = (time.time() - self.start_time) * 1000  # milliseconds
            log(self.level, self.category, f"{self.operation} took {duration:.3f}ms")
