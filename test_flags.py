#!/usr/bin/env python3
"""
Test flag emoji rendering
"""

import asyncio
import json
import websockets


async def test_flags():
    uri = "ws://127.0.0.1:8080"
    print(f"Connecting to {uri}...")

    try:
        async with websockets.connect(uri) as websocket:
            print("Connected!")

            # Receive initial connection message
            msg = await websocket.recv()
            data = json.loads(msg)
            print(f"\n[Initial] Type: {data.get('type')}")

            # Test flag emojis (Regional Indicators)
            test_flags = [
                ("🇨🇳", "China", "CN"),
                ("🇺🇸", "USA", "US"),
                ("🇯🇵", "Japan", "JP"),
                ("🇬🇧", "UK", "GB"),
                ("🇫🇷", "France", "FR"),
                ("🇩🇪", "Germany", "DE"),
            ]

            for flag, name, code in test_flags:
                # Send via echo command
                cmd = f"echo 'Flag {name}: {flag} (code: {code})'"
                print(f"\n[Testing] {name} flag: {flag}")

                await websocket.send(json.dumps({"type": "input", "data": cmd + "\n"}))

                # Collect output
                outputs = []
                for _ in range(30):
                    try:
                        msg = await asyncio.wait_for(websocket.recv(), timeout=0.1)
                        data = json.loads(msg)
                        if data.get("type") == "output":
                            outputs.append(data.get("data", ""))
                    except asyncio.TimeoutError:
                        break

                full_output = "".join(outputs)

                # Show bytes for debugging
                flag_bytes = flag.encode("utf-8")
                print(f"  UTF-8 bytes: {flag_bytes.hex()}")

                # Check if flag appears
                if flag in full_output:
                    print("  ✅ Flag transmitted correctly")
                else:
                    print("  ❌ Flag may be corrupted")
                    print(f"  Raw output: {repr(full_output)}")

            print("\n[Test complete]")

    except Exception as e:
        print(f"Error: {e}")
        import traceback

        traceback.print_exc()


if __name__ == "__main__":
    asyncio.run(test_flags())
