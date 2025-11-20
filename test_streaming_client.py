#!/usr/bin/env python3
"""
Simple WebSocket client to test bidirectional streaming
"""
import asyncio
import json
import sys
import websockets

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
            if data.get('type') == 'connected':
                print(f"  Size: {data.get('cols')}x{data.get('rows')}")
                print(f"  Client ID: {data.get('client_id')}")
                if 'screen' in data:
                    print(f"  Initial screen: {len(data['screen'])} chars")
                    print(f"  Screen preview: {repr(data['screen'][:100])}")

            # Send a simple command: "echo hello"
            print("\n[Sending] Input: 'echo hello\\n'")
            input_msg = json.dumps({
                "type": "input",
                "data": "echo hello\n"
            })
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
                        if data.get('type') == 'output':
                            output = data.get('data', '')
                            print(f"[Output {output_count+1}] {repr(output)}")
                            output_count += 1
                    except asyncio.TimeoutError:
                        # No message in this interval, continue waiting
                        continue
            except Exception as e:
                print(f"[Error receiving] {e}")

            print("\n[Test complete]")

    except Exception as e:
        print(f"Error: {e}")
        import traceback
        traceback.print_exc()

if __name__ == '__main__':
    asyncio.run(test_streaming())
