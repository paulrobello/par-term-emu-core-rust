"""Tests for file transfer support (OSC 1337 File= and RequestUpload=).

Tests cover:
- Default and configurable max transfer size
- File download via OSC 1337 File= with inline=0
- Completed transfer retrieval and removal
- Inline=1 images do NOT create file transfers (regression)
- Upload request via OSC 1337 RequestUpload=format
- Upload data and cancel upload responses
- Cancel nonexistent transfer
"""

import base64
import struct
import zlib

from par_term_emu_core_rust import Terminal


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def make_osc1337_file(
    data: bytes, filename: str | None = None, inline: int = 0
) -> bytes:
    """Build an OSC 1337 File= sequence."""
    b64_data = base64.b64encode(data).decode("ascii")
    params = [f"inline={inline}", f"size={len(data)}"]
    if filename:
        b64_name = base64.b64encode(filename.encode()).decode("ascii")
        params.append(f"name={b64_name}")
    params_str = ";".join(params)
    return f"\x1b]1337;File={params_str}:{b64_data}\x07".encode()


def make_1x1_png() -> bytes:
    """Create a minimal valid 1x1 red PNG."""
    signature = b"\x89PNG\r\n\x1a\n"
    # IHDR: 1x1, 8-bit RGB
    ihdr_data = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    ihdr_crc = zlib.crc32(b"IHDR" + ihdr_data) & 0xFFFFFFFF
    ihdr = struct.pack(">I", 13) + b"IHDR" + ihdr_data + struct.pack(">I", ihdr_crc)
    # IDAT: filter=0, R=255, G=0, B=0
    raw = zlib.compress(b"\x00\xff\x00\x00")
    idat_crc = zlib.crc32(b"IDAT" + raw) & 0xFFFFFFFF
    idat = struct.pack(">I", len(raw)) + b"IDAT" + raw + struct.pack(">I", idat_crc)
    # IEND
    iend_crc = zlib.crc32(b"IEND") & 0xFFFFFFFF
    iend = struct.pack(">I", 0) + b"IEND" + struct.pack(">I", iend_crc)
    return signature + ihdr + idat + iend


# ---------------------------------------------------------------------------
# TestFileTransferBasics
# ---------------------------------------------------------------------------


class TestFileTransferBasics:
    """Basic file transfer configuration and state tests."""

    def test_default_max_size(self) -> None:
        """Default max transfer size should be 50 MB."""
        term = Terminal(80, 24)
        assert term.get_max_transfer_size() == 50 * 1024 * 1024

    def test_set_get_max_size(self) -> None:
        """Setting max transfer size should be reflected in get."""
        term = Terminal(80, 24)
        term.set_max_transfer_size(100 * 1024 * 1024)
        assert term.get_max_transfer_size() == 100 * 1024 * 1024

    def test_no_transfers_initially(self) -> None:
        """No active or completed transfers on a fresh terminal."""
        term = Terminal(80, 24)
        assert len(term.get_active_transfers()) == 0
        assert len(term.get_completed_transfers()) == 0


# ---------------------------------------------------------------------------
# TestFileDownload
# ---------------------------------------------------------------------------


class TestFileDownload:
    """File download via OSC 1337 File= with inline=0."""

    def test_single_file_download_events(self) -> None:
        """A single File= with inline=0 should emit started + completed events."""
        term = Terminal(80, 24)
        file_data = b"Hello, file transfer!"
        seq = make_osc1337_file(file_data, filename="test.pdf")
        term.process(seq)

        events = term.poll_events()
        event_types = [e["type"] for e in events]

        assert "file_transfer_started" in event_types
        assert "file_transfer_completed" in event_types

        # Check started event details
        started = [e for e in events if e["type"] == "file_transfer_started"][0]
        assert started["direction"] == "download"
        assert started["filename"] == "test.pdf"
        assert started["total_bytes"] == str(len(file_data))

        # Check completed event details
        completed = [e for e in events if e["type"] == "file_transfer_completed"][0]
        assert completed["filename"] == "test.pdf"
        assert completed["size"] == str(len(file_data))

    def test_take_completed_transfer(self) -> None:
        """take_completed_transfer should return data and remove from buffer."""
        term = Terminal(80, 24)
        file_data = b"Important document content"
        seq = make_osc1337_file(file_data, filename="doc.txt")
        term.process(seq)

        # Get the transfer ID from the completed event
        events = term.poll_events()
        completed_events = [e for e in events if e["type"] == "file_transfer_completed"]
        assert len(completed_events) == 1
        transfer_id = int(completed_events[0]["id"])

        # Take the completed transfer
        transfer = term.take_completed_transfer(transfer_id)
        assert transfer is not None
        assert transfer["data"] == file_data
        assert transfer["filename"] == "doc.txt"

        # Should be removed after taking
        transfer2 = term.take_completed_transfer(transfer_id)
        assert transfer2 is None

    def test_inline_1_is_image_not_file_transfer(self) -> None:
        """inline=1 should be treated as an inline image, NOT a file transfer.

        This is a regression test to ensure that inline images (inline=1)
        do NOT trigger file transfer events.
        """
        term = Terminal(80, 24)
        png_data = make_1x1_png()
        seq = make_osc1337_file(png_data, filename="image.png", inline=1)
        term.process(seq)

        events = term.poll_events()
        file_events = [
            e
            for e in events
            if e["type"] in ("file_transfer_started", "file_transfer_completed")
        ]
        assert len(file_events) == 0, (
            f"inline=1 should NOT produce file transfer events, got: {file_events}"
        )

        # Should have created a graphic instead
        assert term.graphics_count() >= 1

    def test_download_without_filename(self) -> None:
        """File download with no name= param should still work."""
        term = Terminal(80, 24)
        file_data = b"anonymous data"
        seq = make_osc1337_file(file_data)
        term.process(seq)

        events = term.poll_events()
        started = [e for e in events if e["type"] == "file_transfer_started"]
        assert len(started) == 1
        # When no filename is provided, the key should be absent
        assert "filename" not in started[0] or started[0].get("filename") == ""


# ---------------------------------------------------------------------------
# TestUploadRequest
# ---------------------------------------------------------------------------


class TestUploadRequest:
    """Upload request via OSC 1337 RequestUpload=."""

    def test_upload_requested_event(self) -> None:
        """RequestUpload=format=tgz should emit upload_requested event."""
        term = Terminal(80, 24)
        seq = b"\x1b]1337;RequestUpload=format=tgz\x07"
        term.process(seq)

        events = term.poll_events()
        upload_events = [e for e in events if e["type"] == "upload_requested"]
        assert len(upload_events) == 1
        assert upload_events[0]["format"] == "tgz"

    def test_send_upload_data_response(self) -> None:
        """send_upload_data should write 'ok\\n' + base64(data) + '\\n\\n' to responses."""
        term = Terminal(80, 24)
        upload_data = b"file content here"
        term.send_upload_data(upload_data)

        response = term.drain_responses()
        response_str = bytes(response).decode("ascii")
        assert response_str.startswith("ok\n")

        # The rest should be base64 encoded data followed by \n\n
        parts = response_str.split("\n", 1)
        assert parts[0] == "ok"
        b64_part = parts[1].rstrip("\n")
        decoded = base64.b64decode(b64_part)
        assert decoded == upload_data

    def test_cancel_upload_response(self) -> None:
        """cancel_upload should write Ctrl-C (0x03) to responses."""
        term = Terminal(80, 24)
        term.cancel_upload()

        response = term.drain_responses()
        assert bytes(response) == b"\x03"


# ---------------------------------------------------------------------------
# TestCancelTransfer
# ---------------------------------------------------------------------------


class TestCancelTransfer:
    """Transfer cancellation tests."""

    def test_cancel_nonexistent_returns_false(self) -> None:
        """Cancelling a transfer that doesn't exist should return False."""
        term = Terminal(80, 24)
        result = term.cancel_file_transfer(99999)
        assert result is False

    def test_cancel_active_transfer(self) -> None:
        """Cancelling an active transfer should return True and remove it from active list.

        Note: With single-shot File=, transfers complete immediately so there
        is nothing to cancel. This test verifies the API returns False for
        a transfer that already completed.
        """
        term = Terminal(80, 24)
        file_data = b"some data"
        seq = make_osc1337_file(file_data, filename="cancel_me.txt")
        term.process(seq)

        # Get the transfer ID
        events = term.poll_events()
        completed = [e for e in events if e["type"] == "file_transfer_completed"]
        assert len(completed) == 1
        transfer_id = int(completed[0]["id"])

        # Already completed, so cancel should return False (only cancels active transfers)
        result = term.cancel_file_transfer(transfer_id)
        assert result is False

    def test_get_transfer_and_completed_list(self) -> None:
        """After a download completes, it should appear in the completed transfers list."""
        term = Terminal(80, 24)
        file_data = b"test data for list"
        seq = make_osc1337_file(file_data, filename="listed.txt")
        term.process(seq)

        # Drain events
        term.poll_events()

        # Check completed list
        completed = term.get_completed_transfers()
        assert len(completed) >= 1

        # Find our transfer
        found = [t for t in completed if t["filename"] == "listed.txt"]
        assert len(found) == 1
        assert found[0]["status"] == "completed"
