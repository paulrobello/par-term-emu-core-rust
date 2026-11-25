#!/usr/bin/env python3
"""
Test client for terminal resize functionality
"""

# STALE GATE (2026-11-25 script audit): JSON-era streaming test client.
# The streamer's wire protocol is now protobuf (src/streaming/protocol/ ->
# proto/terminal.proto); the server's first frame is a binary protobuf
# `Connected` message, so json.loads(await ws.recv()) fails with
# UnicodeDecodeError and the old broad except exits 0 anyway. Gated so a
# run cannot be mistaken for a live signal; rewrite against the protobuf
# protocol or remove via the follow-up kanban card filed 2026-11-25.
import sys

print(
    "STALE: this helper predates the protobuf streaming protocol; its JSON "
    "client cannot decode the server's protobuf frames. See the kanban "
    "follow-up card (2026-11-25 script audit).",
    file=sys.stderr,
)
sys.exit(1)

import asyncio
import json

import websockets  # type: ignore


async def test_resize():
    uri = "ws://127.0.0.1:8080"
    print(f"Connecting to {uri}...")

    try:
        async with websockets.connect(uri) as websocket:
            print("Connected!")

            # Receive initial connection message
            msg = await websocket.recv()
            data = json.loads(msg)
            print(
                f"\n[Initial] Type: {data.get('type')}, Size: {data.get('cols')}x{data.get('rows')}"
            )

            # Send resize to 120x30
            print("\n[Sending] Resize to 120x30")
            resize_msg = json.dumps({"type": "resize", "cols": 120, "rows": 30})
            await websocket.send(resize_msg)

            # Wait a moment
            await asyncio.sleep(0.2)

            # Send a command to see output
            print("[Sending] Command: 'tput cols; tput lines'")
            await websocket.send(
                json.dumps({"type": "input", "data": "tput cols; tput lines\n"})
            )

            # Receive output
            print("\n[Waiting for output...]")
            for i in range(20):
                try:
                    msg = await asyncio.wait_for(websocket.recv(), timeout=0.1)
                    data = json.loads(msg)
                    if data.get("type") == "output":
                        output = data.get("data", "")
                        print(f"[Output] {output!r}")
                except TimeoutError:
                    continue

            # Send another resize to 80x24
            print("\n[Sending] Resize to 80x24")
            resize_msg = json.dumps({"type": "resize", "cols": 80, "rows": 24})
            await websocket.send(resize_msg)

            await asyncio.sleep(0.2)

            print("\n[Test complete]")

    except Exception as e:  # noqa: BLE001
        print(f"Error: {e}")
        import traceback

        traceback.print_exc()


if __name__ == "__main__":
    asyncio.run(test_resize())
