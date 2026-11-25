#!/usr/bin/env python3
"""
Simple WebSocket client to test bidirectional streaming
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


async def test_streaming():
    uri = "ws://127.0.0.1:8080"
    print(f"Connecting to {uri}...")

    try:
        async with websockets.connect(uri) as websocket:
            print("Connected!")

            # Receive initial connection message
            msg = await websocket.recv()
            data = json.loads(msg)
            print(f"\n[Received] Type: {data.get('type')}")
            if data.get("type") == "connected":
                print(f"  Size: {data.get('cols')}x{data.get('rows')}")
                print(f"  Client ID: {data.get('client_id')}")
                if "screen" in data:
                    print(f"  Initial screen: {len(data['screen'])} chars")
                    print(f"  Screen preview: {data['screen'][:100]!r}")

            # Send a simple command: "echo hello"
            print("\n[Sending] Input: 'echo hello\\n'")
            input_msg = json.dumps({"type": "input", "data": "echo hello\n"})
            await websocket.send(input_msg)

            # Receive output for up to 5 seconds
            print("\n[Waiting for output...]")
            output_count = 0
            try:
                # Wait up to 5 seconds for output messages
                for i in range(50):  # 5 seconds worth of 100ms intervals
                    try:
                        msg = await asyncio.wait_for(websocket.recv(), timeout=0.1)
                        data = json.loads(msg)
                        if data.get("type") == "output":
                            output = data.get("data", "")
                            print(f"[Output {output_count + 1}] {output!r}")
                            output_count += 1
                    except TimeoutError:
                        # No message in this interval, continue waiting
                        continue
            except Exception as e:  # noqa: BLE001
                print(f"[Error receiving] {e}")

            print("\n[Test complete]")

    except Exception as e:  # noqa: BLE001
        print(f"Error: {e}")
        import traceback

        traceback.print_exc()


if __name__ == "__main__":
    asyncio.run(test_streaming())
