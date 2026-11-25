"""Minimal protobuf wire codec + WebSocket plumbing for the streaming test clients.

The streamer's wire protocol is the ServerMessage/ClientMessage protobuf from
`proto/terminal.proto` (wire rules in `src/streaming/proto.rs`); the server
sends one serialized ServerMessage per WebSocket binary frame. Python has no
generated bindings for that schema, so this module hand-encodes only the
fields the test clients need.
"""

from __future__ import annotations

import asyncio
import zlib
from collections.abc import AsyncIterator
from dataclasses import dataclass
from typing import Any

# ServerMessage oneof field numbers (proto/terminal.proto `message ServerMessage`).
SM_OUTPUT = 1
SM_RESIZE = 2
SM_CONNECTED = 4

STREAMER_WS_URL = "ws://127.0.0.1:8080"
RECV_TIMEOUT_S = 10.0

# Frame envelope (src/streaming/proto.rs encode_with_compression): one flag
# byte, then the raw protobuf payload — zlib-compressed when it both exceeds
# COMPRESSION_THRESHOLD (256) and actually saves space.
FLAG_UNCOMPRESSED = 0x00
FLAG_COMPRESSED = 0x01
COMPRESSION_THRESHOLD = 256


def decode_frame(raw: bytes) -> bytes:
    """Strip the one-byte compression flag, decompressing when flagged."""
    if not raw:
        raise ValueError("empty frame")
    flag, payload = raw[0], raw[1:]
    if flag == FLAG_COMPRESSED:
        return zlib.decompress(payload)
    if flag == FLAG_UNCOMPRESSED:
        return payload
    raise ValueError(f"unknown frame flag 0x{flag:02x}")


def encode_frame(payload: bytes) -> bytes:
    """Wrap a serialized ClientMessage in the uncompressed frame envelope."""
    return bytes([FLAG_UNCOMPRESSED]) + payload


def encode_varint(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def encode_len_field(field: int, payload: bytes) -> bytes:
    """Length-delimited field: (field << 3 | 2) + varint len + payload."""
    return bytes([(field << 3) | 2]) + encode_varint(len(payload)) + payload


def encode_client_message_input(data: bytes) -> bytes:
    """Complete frame for ClientMessage{ input = 1 } wrapping Input{ data = 1 }."""
    inner = encode_len_field(1, data)
    return encode_frame(encode_len_field(1, inner))


def encode_client_message_resize(cols: int, rows: int) -> bytes:
    """Complete frame for ClientMessage{ resize = 2 } wrapping ClientResize."""
    inner = bytes([0x08]) + encode_varint(cols) + bytes([0x10]) + encode_varint(rows)
    return encode_frame(encode_len_field(2, inner))


def _read_varint(buf: bytes, pos: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        byte = buf[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def _read_tag(buf: bytes, pos: int) -> tuple[int, int, int]:
    """Read a field tag: tags are varints, so fields >= 16 take two bytes."""
    tag, pos = _read_varint(buf, pos)
    return tag >> 3, tag & 0x07, pos


def _skip_field(buf: bytes, pos: int, wire_type: int) -> int:
    if wire_type == 0:  # varint
        _, pos = _read_varint(buf, pos)
    elif wire_type == 1:  # fixed64
        pos += 8
    elif wire_type == 2:  # length-delimited
        length, pos = _read_varint(buf, pos)
        if pos + length > len(buf):
            raise ValueError("truncated length-delimited field")
        pos += length
    elif wire_type == 5:  # fixed32
        if pos + 4 > len(buf):
            raise ValueError("truncated fixed32 field")
        pos += 4
    else:
        raise ValueError(f"unsupported wire type {wire_type}")
    return pos


def parse_top_level(frame: bytes) -> tuple[int, bytes]:
    """Parse a ServerMessage frame: the top-level oneof member as (field, payload)."""
    if not frame:
        raise ValueError("empty frame")
    field, wire_type, pos = _read_tag(frame, 0)
    if wire_type != 2:
        raise ValueError(f"unexpected top-level wire type {wire_type} on field {field}")
    length, pos = _read_varint(frame, pos)
    if pos + length > len(frame):
        raise ValueError("truncated ServerMessage frame")
    return field, frame[pos : pos + length]


@dataclass
class ConnectedInfo:
    cols: int
    rows: int
    session_id: str
    client_id: str
    initial_screen: bytes | None


def parse_connected(payload: bytes) -> ConnectedInfo:
    """Connected{ cols=1, rows=2, initial_screen=3, session_id=4, client_id=10 }."""
    cols = rows = 0
    session_id = ""
    client_id = ""
    initial_screen: bytes | None = None
    pos = 0
    while pos < len(payload):
        field, wire_type, pos = _read_tag(payload, pos)
        if field == 1 and wire_type == 0:
            cols, pos = _read_varint(payload, pos)
        elif field == 2 and wire_type == 0:
            rows, pos = _read_varint(payload, pos)
        elif field == 3 and wire_type == 2:
            length, pos = _read_varint(payload, pos)
            initial_screen = payload[pos : pos + length]
            pos += length
        elif field == 4 and wire_type == 2:
            length, pos = _read_varint(payload, pos)
            session_id = payload[pos : pos + length].decode("utf-8")
            pos += length
        elif field == 10 and wire_type == 2:
            length, pos = _read_varint(payload, pos)
            client_id = payload[pos : pos + length].decode("utf-8")
            pos += length
        else:
            pos = _skip_field(payload, pos, wire_type)
    return ConnectedInfo(cols, rows, session_id, client_id, initial_screen)


def parse_resize(payload: bytes) -> tuple[int, int]:
    """Resize{ cols = 1, rows = 2 }."""
    cols = rows = 0
    pos = 0
    while pos < len(payload):
        field, wire_type, pos = _read_tag(payload, pos)
        if field == 1 and wire_type == 0:
            cols, pos = _read_varint(payload, pos)
        elif field == 2 and wire_type == 0:
            rows, pos = _read_varint(payload, pos)
        else:
            pos = _skip_field(payload, pos, wire_type)
    return cols, rows


def parse_output_data(payload: bytes) -> bytes:
    """Output{ data = 1 } (fields besides data are ignored by the test clients)."""
    pos = 0
    while pos < len(payload):
        field, wire_type, pos = _read_tag(payload, pos)
        if field == 1 and wire_type == 2:
            length, pos = _read_varint(payload, pos)
            return payload[pos : pos + length]
        pos = _skip_field(payload, pos, wire_type)
    return b""


async def recv_server_frame(ws: Any) -> tuple[int, bytes]:
    """Receive the next ServerMessage frame as (field, payload), decompressed."""
    raw = await asyncio.wait_for(ws.recv(), timeout=RECV_TIMEOUT_S)
    if isinstance(raw, str):
        raise TypeError("expected a binary protobuf frame, got text")
    return parse_top_level(decode_frame(raw))


async def recv_connected(ws: Any) -> ConnectedInfo:
    """Read frames until the Connected message arrives."""
    while True:
        field, payload = await recv_server_frame(ws)
        if field == SM_CONNECTED:
            return parse_connected(payload)


async def recv_output_frames(ws: Any) -> AsyncIterator[bytes]:
    """Yield Output.data payloads, skipping other ServerMessage members."""
    while True:
        field, payload = await recv_server_frame(ws)
        if field == SM_OUTPUT:
            yield parse_output_data(payload)


async def collect_output_until(
    ws: Any, needle: str, max_wait_s: float = RECV_TIMEOUT_S
) -> str | None:
    """Collect Output payloads until `needle` appears; return accumulated text."""
    seen = ""
    loop = asyncio.get_event_loop()
    deadline = loop.time() + max_wait_s
    while loop.time() < deadline:
        try:
            async for chunk in recv_output_frames(ws):
                seen += chunk.decode("utf-8", errors="replace")
                if needle in seen:
                    return seen
                if loop.time() >= deadline:
                    break
        except TimeoutError:
            break
    return None
