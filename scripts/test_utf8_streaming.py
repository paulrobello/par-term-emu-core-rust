#!/usr/bin/env python3
"""
Test UTF-8 character handling in streaming
"""

import asyncio
import json

import websockets  # type: ignore


async def test_utf8():
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

            # Test various UTF-8 characters
            test_commands = [
                ("echo 'Hello World'", "Basic ASCII"),
                ("echo 'Café résumé naïve'", "Latin extended (accents)"),
                ("echo '你好世界'", "Chinese characters"),
                ("echo '🚀 🎉 ✨'", "Emojis"),
                ("echo 'Привет мир'", "Cyrillic"),
                ("echo 'مرحبا بالعالم'", "Arabic"),
                ("echo '日本語テスト'", "Japanese"),
            ]

            for cmd, description in test_commands:
                print(f"\n[Testing] {description}: {cmd}")

                # Send command
                await websocket.send(json.dumps({"type": "input", "data": cmd + "\n"}))

                # Collect output for this command
                outputs = []
                for _ in range(30):  # Wait up to 3 seconds
                    try:
                        msg = await asyncio.wait_for(websocket.recv(), timeout=0.1)
                        data = json.loads(msg)
                        if data.get("type") == "output":
                            outputs.append(data.get("data", ""))
                    except TimeoutError:
                        break

                # Print collected output
                full_output = "".join(outputs)
                print(f"[Output] {full_output!r}")

                # Check if our test string appears in output
                test_str = cmd.split("'")[1] if "'" in cmd else cmd
                if test_str in full_output:
                    print("✅ UTF-8 preserved correctly")
                else:
                    print("❌ UTF-8 may be corrupted")

            print("\n[Test complete]")

    except Exception as e:  # noqa: BLE001
        print(f"Error: {e}")
        import traceback

        traceback.print_exc()


if __name__ == "__main__":
    asyncio.run(test_utf8())
