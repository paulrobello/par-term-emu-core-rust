#!/usr/bin/env python3
"""
Simple WebSocket client to test bidirectional streaming against the
par-term-streamer (protobuf wire protocol; run the server as
`par-term-streamer --port 8080`).

Rewritten 2026-11-25 from the JSON-era client: the server's frames are
protobuf ServerMessage messages (proto/terminal.proto), hand-encoded in
`_ws_pb_test_common.py`.
"""

import asyncio
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import websockets  # type: ignore
from _ws_pb_test_common import (
    STREAMER_WS_URL,
    collect_output_until,
    encode_client_message_input,
    recv_connected,
)


async def test_streaming() -> bool:
    print(f"Connecting to {STREAMER_WS_URL}...")

    async with websockets.connect(STREAMER_WS_URL) as websocket:
        print("Connected!")

        connected = await recv_connected(websocket)
        print("\n[Received] Connected")
        print(f"  Size: {connected.cols}x{connected.rows}")
        print(f"  Session ID: {connected.session_id}")
        print(f"  Client ID: {connected.client_id}")
        if connected.initial_screen is not None:
            print(f"  Initial screen: {len(connected.initial_screen)} chars")
            print(f"  Screen preview: {connected.initial_screen[:100]!r}")

        print("\n[Sending] Input: 'echo hello\\n'")
        await websocket.send(encode_client_message_input(b"echo hello\n"))

        seen = await collect_output_until(websocket, "hello")
        if seen is None:
            print("\n✗ No output containing 'hello' came back")
            return False
        print(f"\n[Received] Output echo ({len(seen)} chars accumulated)")
        print(f"  Last chunk preview: {seen[-120:]!r}")
        return True


async def main() -> int:
    try:
        ok = await test_streaming()
    except Exception as exc:  # noqa: BLE001
        print(f"\n✗ Streaming test failed: {exc}", file=sys.stderr)
        return 1
    if not ok:
        return 1
    print("\n✓ Bidirectional streaming works")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
