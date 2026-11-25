#!/usr/bin/env python3
"""
Test UTF-8 character handling in streaming against the par-term-streamer
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
    STREAMER_WS_URL,
    collect_output_until,
    encode_client_message_input,
    recv_connected,
)

# (input command, needle that must echo back)
TEST_COMMANDS: list[tuple[str, str]] = [
    ("echo 'Hello World'", "Hello World"),
    ("echo 'Café résumé naïve'", "Café"),
    ("echo '你好世界'", "你好世界"),
    ("echo '🚀 🎉 ✨'", "🚀"),
    ("echo 'Ünïcödé — mixed: Ω δ Ψ'", "Ünïcödé"),
    ("echo 'line1\nline2'", "line2"),
]


async def test_utf8() -> list[str]:
    print(f"Connecting to {STREAMER_WS_URL}...")
    failures: list[str] = []

    async with websockets.connect(STREAMER_WS_URL) as websocket:
        await recv_connected(websocket)
        print("Connected!")

        for command, needle in TEST_COMMANDS:
            print(f"\n[Sending] {command!r}  (expecting {needle!r})")
            await websocket.send(encode_client_message_input(command.encode() + b"\n"))
            seen = await collect_output_until(websocket, needle)
            if seen is not None:
                print(f"  ✓ echoed ({len(seen)} chars accumulated)")
            else:
                print("  ✗ needle not found in echoed output")
                failures.append(command)

    return failures


async def main() -> int:
    try:
        failures = await test_utf8()
    except Exception as exc:  # noqa: BLE001
        print(f"\n✗ UTF-8 streaming test failed: {exc}", file=sys.stderr)
        return 1
    if failures:
        print(
            f"\n✗ {len(failures)}/{len(TEST_COMMANDS)} commands failed to echo: {failures}"
        )
        return 1
    print(f"\n✓ All {len(TEST_COMMANDS)} UTF-8 commands echoed correctly")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
