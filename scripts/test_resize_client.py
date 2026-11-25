#!/usr/bin/env python3
"""
Test client for terminal resize functionality against the par-term-streamer
(protobuf wire protocol; run the server as `par-term-streamer --port 8080`).

Rewritten 2026-11-25 from the JSON-era client: the server's frames are
protobuf ServerMessage messages (proto/terminal.proto), hand-encoded in
`_ws_pb_test_common.py`.
"""

import asyncio
import sys

sys.path.insert(0, __file__.rsplit("/", 1)[0])

import websockets  # type: ignore
from _ws_pb_test_common import (
    SM_RESIZE,
    STREAMER_WS_URL,
    encode_client_message_resize,
    parse_resize,
    recv_connected,
    recv_server_frame,
)


async def test_resize() -> bool:
    print(f"Connecting to {STREAMER_WS_URL}...")

    async with websockets.connect(STREAMER_WS_URL) as websocket:
        connected = await recv_connected(websocket)
        print(f"Connected! Initial size: {connected.cols}x{connected.rows}")

        print("\n[Sending] Resize to 120x30")
        await websocket.send(encode_client_message_resize(120, 30))

        # The server reports the applied size in a Resize ServerMessage; skip
        # other members (output reflow etc.) until it arrives.
        while True:
            field, payload = await recv_server_frame(websocket)
            if field == SM_RESIZE:
                cols, rows = parse_resize(payload)
                print(f"\n[Received] Resize: {cols}x{rows}")
                break

        if (cols, rows) != (120, 30):
            print(f"✗ Resize not applied: got {cols}x{rows}, expected 120x30")
            return False
        return True


async def main() -> int:
    try:
        ok = await test_resize()
    except Exception as exc:  # noqa: BLE001
        print(f"\n✗ Resize test failed: {exc}", file=sys.stderr)
        return 1
    if not ok:
        return 1
    print("\n✓ Resize works")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
