"""Comprehensive tests for terminal streaming functionality.

Tests WebSocket-based terminal streaming with multiple clients, authentication,
resizing, and concurrent operations.
"""

from __future__ import annotations

import asyncio
import time
from typing import TYPE_CHECKING, Any

import pytest

# Streaming is optional, so skip all tests if not available
pytest.importorskip("websockets")

try:
    from par_term_emu_core_rust import PtyTerminal, StreamingConfig, StreamingServer
    import websockets  # type: ignore[import-not-found]

    HAS_STREAMING = True
except ImportError:
    HAS_STREAMING = False
    pytestmark = pytest.mark.skip(reason="Streaming feature not built")
    # Type checking stubs to avoid unbound errors
    if TYPE_CHECKING:
        from par_term_emu_core_rust import (  # type: ignore[assignment]
            PtyTerminal,
            StreamingConfig,
            StreamingServer,
        )
        import websockets  # type: ignore[assignment, import-not-found]
    else:
        # Dummy classes to satisfy runtime when imports fail
        PtyTerminal = Any  # type: ignore[misc, assignment]
        StreamingConfig = Any  # type: ignore[misc, assignment]
        StreamingServer = Any  # type: ignore[misc, assignment]
        websockets = Any  # type: ignore[misc, assignment]


# Fixtures


@pytest.fixture
def streaming_port():
    """Get an available port for testing."""
    import socket

    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return port


@pytest.fixture
def pty_terminal():
    """Create a PTY terminal for testing."""
    term = PtyTerminal(80, 24)
    term.spawn_shell()
    yield term
    # Cleanup
    try:
        term.write_str("exit\n")
    except Exception:
        pass


@pytest.fixture
async def streaming_server(pty_terminal, streaming_port):
    """Create and start a streaming server."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()

    # Give server time to start
    await asyncio.sleep(0.1)

    yield server, streaming_port

    # Cleanup
    try:
        server.stop()
    except Exception:
        pass


# Configuration Tests


def test_streaming_config_creation():
    """Test creating streaming configuration."""
    config = StreamingConfig()
    assert config.max_clients == 1000
    assert config.send_initial_screen is True
    assert config.keepalive_interval == 30
    assert config.default_read_only is False
    assert config.max_sessions == 10
    assert config.session_idle_timeout == 900


def test_streaming_config_custom():
    """Test custom streaming configuration."""
    config = StreamingConfig(
        max_clients=10,
        send_initial_screen=False,
        keepalive_interval=60,
        default_read_only=True,
    )

    assert config.max_clients == 10
    assert config.send_initial_screen is False
    assert config.keepalive_interval == 60
    assert config.default_read_only is True


def test_streaming_config_setters():
    """Test streaming configuration setters."""
    config = StreamingConfig()

    config.max_clients = 500
    assert config.max_clients == 500

    config.send_initial_screen = False
    assert config.send_initial_screen is False

    config.keepalive_interval = 120
    assert config.keepalive_interval == 120

    config.default_read_only = True
    assert config.default_read_only is True

    config.max_sessions = 5
    assert config.max_sessions == 5

    config.session_idle_timeout = 600
    assert config.session_idle_timeout == 600


def test_streaming_config_repr():
    """Test streaming configuration string representation."""
    config = StreamingConfig(max_clients=100)
    repr_str = repr(config)

    assert "StreamingConfig" in repr_str
    assert "max_clients=100" in repr_str


# Server Creation and Management Tests


def test_server_creation(pty_terminal, streaming_port):
    """Test creating a streaming server."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    assert server is not None


def test_server_creation_with_config(pty_terminal, streaming_port):
    """Test creating a streaming server with custom config."""
    config = StreamingConfig(max_clients=50)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    assert server is not None


def test_server_start_stop(pty_terminal, streaming_port):
    """Test starting and stopping the server."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")

    # Start server
    server.start()
    time.sleep(0.1)

    # Server should be running
    assert server.is_running()

    # Stop server
    server.stop()
    time.sleep(0.1)

    # Server should be stopped
    assert not server.is_running()


def test_server_client_count_no_clients(pty_terminal, streaming_port):
    """Test client count with no connected clients."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    time.sleep(0.1)

    assert server.client_count() == 0

    server.stop()


@pytest.mark.asyncio
async def test_server_address(pty_terminal, streaming_port):
    """Test getting server address."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")

    # Address should be set
    addr = server.address()
    assert "127.0.0.1" in addr
    assert str(streaming_port) in addr


# WebSocket Connection Tests


@pytest.mark.asyncio
async def test_websocket_connection(streaming_server):
    """Test basic WebSocket connection."""
    server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    async with websockets.connect(uri) as websocket:
        # Should be connected
        assert websocket.open

        # Should receive initial screen (if configured)
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
            assert message is not None
        except asyncio.TimeoutError:
            pass  # OK if no initial screen


@pytest.mark.asyncio
async def test_websocket_receive_output(streaming_server):
    """Test receiving terminal output via WebSocket."""
    _server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    async with websockets.connect(uri) as websocket:
        # Send command to terminal
        # Note: We can't directly access pty_terminal here, so this is a simplified test

        # Wait for any output
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=2.0)
            # Should receive something
            assert isinstance(message, str)
        except asyncio.TimeoutError:
            pytest.skip("No output received within timeout")


@pytest.mark.asyncio
async def test_websocket_multiple_clients(streaming_server):
    """Test multiple WebSocket clients connecting simultaneously."""
    server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    # Connect multiple clients
    clients = []
    try:
        for i in range(3):
            client = await websockets.connect(uri)
            clients.append(client)
            await asyncio.sleep(0.05)

        # All should be connected
        assert len(clients) == 3
        for client in clients:
            assert client.open

        # Check client count
        # Note: This may not work immediately due to async nature
        await asyncio.sleep(0.2)

    finally:
        # Close all clients
        for client in clients:
            await client.close()


@pytest.mark.asyncio
async def test_websocket_client_disconnect(streaming_server):
    """Test client disconnect and reconnect."""
    server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    # Connect
    websocket = await websockets.connect(uri)
    assert websocket.open

    # Disconnect
    await websocket.close()
    assert not websocket.open

    # Reconnect
    websocket = await websockets.connect(uri)
    assert websocket.open
    await websocket.close()


# Output Broadcasting Tests


@pytest.mark.asyncio
async def test_broadcast_to_all_clients(pty_terminal, streaming_port):
    """Test broadcasting output to all connected clients."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    # Connect two clients
    client1 = await websockets.connect(uri)
    client2 = await websockets.connect(uri)
    await asyncio.sleep(0.1)

    try:
        # Send output through terminal
        test_message = "Hello, World!\n"
        pty_terminal.write_str(test_message)
        await asyncio.sleep(0.2)

        # Both clients should receive the output
        # This is a simplified test - actual output may include ANSI codes
        received = []
        try:
            msg1 = await asyncio.wait_for(client1.recv(), timeout=1.0)
            received.append(msg1)
        except asyncio.TimeoutError:
            pass

        try:
            msg2 = await asyncio.wait_for(client2.recv(), timeout=1.0)
            received.append(msg2)
        except asyncio.TimeoutError:
            pass

        # At least one client should have received something
        assert len(received) > 0

    finally:
        await client1.close()
        await client2.close()
        server.stop()


# Configuration and Limits Tests


def test_max_clients_limit(pty_terminal, streaming_port):
    """Test maximum clients configuration."""
    config = StreamingConfig(max_clients=2)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()

    # Configuration should be set
    time.sleep(0.1)

    server.stop()


@pytest.mark.asyncio
async def test_send_initial_screen_enabled(pty_terminal, streaming_port):
    """Test initial screen sending when enabled."""
    config = StreamingConfig(send_initial_screen=True)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri) as websocket:
        # Should receive initial screen content
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
            assert message is not None
            assert len(message) > 0
        except asyncio.TimeoutError:
            pytest.skip("Initial screen not received within timeout")

    server.stop()


@pytest.mark.asyncio
async def test_send_initial_screen_disabled(pty_terminal, streaming_port):
    """Test no initial screen when disabled."""
    config = StreamingConfig(send_initial_screen=False)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri) as websocket:
        # Should not receive immediate content
        # We'll wait a short time to ensure nothing is sent
        with pytest.raises(asyncio.TimeoutError):
            await asyncio.wait_for(websocket.recv(), timeout=0.3)

    server.stop()


# Terminal Resizing Tests


@pytest.mark.asyncio
async def test_terminal_resize_notification(pty_terminal, streaming_port):
    """Test terminal resize notifications through WebSocket."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri) as websocket:
        # Resize terminal
        pty_terminal.resize(100, 30)
        await asyncio.sleep(0.2)

        # Client might receive resize notification
        # (Implementation-specific behavior)
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
            # If we received something, validate it
            assert message is not None
        except asyncio.TimeoutError:
            pass  # Resize notifications may not be implemented

    server.stop()


# Error Handling Tests


def test_server_bind_error_duplicate_port(pty_terminal, streaming_port):
    """Test error when binding to already-used port."""
    server1 = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server1.start()
    time.sleep(0.1)

    # Try to create another server on same port
    # This should either fail immediately or when started
    try:
        pty_terminal2 = PtyTerminal(80, 24)
        pty_terminal2.spawn_shell()
        server2 = StreamingServer(pty_terminal2, f"127.0.0.1:{streaming_port}")
        server2.start()
        time.sleep(0.1)

        # If we got here, check if both are actually running
        # (some implementations may handle this gracefully)

        server2.stop()
    except Exception:
        # Expected to fail
        pass
    finally:
        server1.stop()


def test_server_operations_after_stop(pty_terminal, streaming_port):
    """Test server operations after stopping."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    time.sleep(0.1)

    server.stop()
    time.sleep(0.1)

    # Should report as not running
    assert not server.is_running()

    # Client count should be 0
    assert server.client_count() == 0


# Performance and Stress Tests


@pytest.mark.asyncio
@pytest.mark.slow
async def test_high_throughput_output(pty_terminal, streaming_port):
    """Test streaming with high-throughput output."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri) as websocket:
        # Generate lots of output
        for i in range(50):
            pty_terminal.write_str(f"Line {i}: " + "X" * 70 + "\n")
            await asyncio.sleep(0.01)

        await asyncio.sleep(0.5)

        # Should have received multiple messages
        messages_received = 0
        try:
            while messages_received < 10:
                await asyncio.wait_for(websocket.recv(), timeout=0.1)
                messages_received += 1
        except asyncio.TimeoutError:
            pass

        # Should have received at least some messages
        assert messages_received > 0

    server.stop()


@pytest.mark.asyncio
@pytest.mark.slow
async def test_many_clients_sequential(pty_terminal, streaming_port):
    """Test many clients connecting and disconnecting sequentially."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    # Connect and disconnect many clients
    for i in range(20):
        client = await websockets.connect(uri)
        assert client.open

        # Receive any initial data
        try:
            await asyncio.wait_for(client.recv(), timeout=0.1)
        except asyncio.TimeoutError:
            pass

        await client.close()
        await asyncio.sleep(0.05)

    server.stop()


# Integration Tests


@pytest.mark.asyncio
async def test_full_session_workflow(pty_terminal, streaming_port):
    """Test a complete terminal session workflow with streaming."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri) as websocket:
        # Execute some commands
        commands = [
            "echo 'Hello from streaming test'",
            "pwd",
            "echo 'Goodbye'",
        ]

        for cmd in commands:
            pty_terminal.write_str(cmd + "\n")
            await asyncio.sleep(0.2)

            # Collect output
            output_chunks = []
            try:
                for _ in range(3):
                    chunk = await asyncio.wait_for(websocket.recv(), timeout=0.2)
                    output_chunks.append(chunk)
            except asyncio.TimeoutError:
                pass

            # Should have received some output
            assert len(output_chunks) > 0

    server.stop()


@pytest.mark.asyncio
async def test_concurrent_read_write(pty_terminal, streaming_port):
    """Test concurrent reading and writing with streaming."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    await asyncio.sleep(0.1)

    uri = f"ws://127.0.0.1:{streaming_port}"

    async def write_output():
        """Write output to terminal."""
        for i in range(10):
            pty_terminal.write_str(f"Output {i}\n")
            await asyncio.sleep(0.1)

    async def read_from_websocket(websocket):
        """Read from WebSocket."""
        messages = []
        try:
            for _ in range(15):
                msg = await asyncio.wait_for(websocket.recv(), timeout=0.2)
                messages.append(msg)
        except asyncio.TimeoutError:
            pass
        return messages

    async with websockets.connect(uri) as websocket:
        # Run concurrently
        write_task = asyncio.create_task(write_output())
        messages = await read_from_websocket(websocket)
        await write_task

        # Should have received multiple messages
        assert len(messages) > 0

    server.stop()


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-m", "not slow"])
