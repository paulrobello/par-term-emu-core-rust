"""Comprehensive tests for terminal streaming functionality.

Tests WebSocket-based terminal streaming with multiple clients, authentication,
resizing, and concurrent operations.

Clients connect with close_timeout=1: when a test closes while unread broadcast
messages are still queued client-side, the websockets library's assembler hits
its queue bounds and pauses its reader, so close() blocks for the full
close_timeout regardless of server behavior (the server breaks its session loop
and replies with a Close frame promptly). Keep close_timeout small so the
suite's 5s per-test timeout is not exceeded.
"""

from __future__ import annotations

import asyncio
import socket
from typing import TYPE_CHECKING, Any

import pytest
import pytest_asyncio
from conftest import wait_for

# Streaming is optional, so skip all tests if not available
pytest.importorskip("websockets")

try:
    import websockets  # type: ignore[import-not-found]
    from par_term_emu_core_rust import (
        PtyTerminal,
        StreamingConfig,
        StreamingServer,
        decode_server_message,
    )

    # Verify streaming feature is actually compiled (classes exist but raise
    # RuntimeError at construction if the feature wasn't enabled at build time)
    StreamingConfig()
    HAS_STREAMING = True
except (ImportError, RuntimeError, TypeError):
    HAS_STREAMING = False
    pytestmark = pytest.mark.skip(reason="Streaming feature not built")
    # Type checking stubs to avoid unbound errors
    if TYPE_CHECKING:
        import websockets  # type: ignore[assignment, import-not-found]
        from par_term_emu_core_rust import (  # type: ignore[assignment]
            PtyTerminal,
            StreamingConfig,
            StreamingServer,
            decode_server_message,
        )
    else:
        # Dummy classes to satisfy runtime when imports fail
        PtyTerminal = Any  # type: ignore[misc, assignment]
        StreamingConfig = Any  # type: ignore[misc, assignment]
        StreamingServer = Any  # type: ignore[misc, assignment]
        decode_server_message = Any  # type: ignore[misc, assignment]
        websockets = Any  # type: ignore[misc, assignment]


# Fixtures


def port_open(port: int) -> bool:
    """Whether the streaming server's TCP port accepts connections.

    The server binds on its own background thread, so this (polled via
    ``wait_for``) is the readiness observable; ``server.addr`` is only the
    configured address, set before the listener exists.
    """
    with socket.socket() as sock:
        sock.settimeout(0.1)
        return sock.connect_ex(("127.0.0.1", port)) == 0


@pytest.fixture
def streaming_port():
    """Get an available port for testing."""
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
    except Exception:  # noqa: BLE001, S110
        pass


# Async fixtures need pytest_asyncio's decorator in strict mode (the default);
# a plain @pytest.fixture async generator is left unhandled and every test
# requesting it errors at setup.
@pytest_asyncio.fixture
async def streaming_server(pty_terminal, streaming_port):
    """Create and start a streaming server."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()

    # The listener binds on a background thread; wait for it. Blocking the
    # test loop here is safe — the server runs on its own thread + runtime.
    wait_for(lambda: port_open(streaming_port))

    yield server, streaming_port

    # Cleanup
    try:
        server.shutdown("test shutdown")
    except Exception:  # noqa: BLE001, S110
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


def test_streaming_config_api_key_default_none():
    """Test that api_key defaults to None."""
    config = StreamingConfig()
    assert config.api_key is None


def test_streaming_config_api_key_constructor():
    """Test setting api_key via constructor."""
    config = StreamingConfig(api_key="my-secret-key")
    assert config.api_key == "my-secret-key"


def test_streaming_config_api_key_setter():
    """Test setting api_key via setter."""
    config = StreamingConfig()
    assert config.api_key is None
    config.api_key = "new-key"
    assert config.api_key == "new-key"
    config.api_key = None
    assert config.api_key is None


def test_streaming_config_api_key_repr():
    """Test that api_key is masked in repr."""
    config = StreamingConfig(api_key="secret")
    repr_str = repr(config)
    assert "api_key=***" in repr_str
    assert "secret" not in repr_str


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
    assert wait_for(lambda: port_open(streaming_port))

    # Server should be running (bound address is set once started)
    assert server.addr != ""

    # Stop server
    server.shutdown("test shutdown")

    # Server should be stopped (no clients remain)
    assert server.client_count() == 0


def test_server_client_count_no_clients(pty_terminal, streaming_port):
    """Test client count with no connected clients."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()

    # Readiness poll via a raw connect leaves a ghost connection the server
    # reaps at its next EOF check, so wait for the count to settle at 0
    # instead of asserting it the instant the port opens.
    assert wait_for(lambda: port_open(streaming_port) and server.client_count() == 0)

    server.shutdown("test shutdown")


@pytest.mark.asyncio
async def test_server_address(pty_terminal, streaming_port):
    """Test getting server address."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")

    # Address should be set
    addr = server.addr
    assert "127.0.0.1" in addr
    assert str(streaming_port) in addr


# WebSocket Connection Tests


@pytest.mark.asyncio
async def test_websocket_connection(streaming_server):
    """Test basic WebSocket connection."""
    _server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # Should be connected
        assert websocket.state.name == "OPEN"

        # Should receive initial screen (if configured)
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
            assert message is not None
        except TimeoutError:
            pass  # OK if no initial screen


@pytest.mark.asyncio
async def test_websocket_close_handshake(streaming_server):
    """Server replies with a Close frame instead of dropping TCP (RFC 6455).

    The server echoes the client's close code, so a normal close completes
    with 1000. 1006 (abnormal closure, bare EOF) is what happens when the
    server drops the stream without completing the closing handshake.

    Pending output is drained first: the websockets library pauses its reader
    once its recv queue exceeds its bounds, and close() then blocks for the
    full close_timeout regardless of server behavior (verified against the
    reference websockets.serve server — this is client-side backpressure, not
    a server stall).
    """
    _server, port = streaming_server
    uri = f"ws://127.0.0.1:{port}"

    websocket = await websockets.connect(uri, close_timeout=1)
    while True:
        try:
            await asyncio.wait_for(websocket.recv(), timeout=0.2)
        except TimeoutError:
            break  # queue drained, shell idle
    await websocket.close()
    assert websocket.close_code == 1000, (
        f"expected proper close handshake (1000), got {websocket.close_code}"
    )


@pytest.mark.asyncio
async def test_websocket_close_handshake_http(pty_terminal, streaming_port):
    """The axum HTTP path (/ws) also completes the closing handshake.

    enable_http=True swaps the raw tungstenite listener for the axum server
    serving web_term/; its session handler (handle_axum_websocket) must reply
    with a Close frame instead of dropping the sink — the same regression as
    the tungstenite path (see test_websocket_close_handshake).
    """
    config = StreamingConfig(enable_http=True)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()
    wait_for(lambda: port_open(streaming_port))

    try:
        uri = f"ws://127.0.0.1:{streaming_port}/ws"

        websocket = await websockets.connect(uri, close_timeout=1)
        while True:
            try:
                await asyncio.wait_for(websocket.recv(), timeout=0.2)
            except TimeoutError:
                break  # queue drained, shell idle
        await websocket.close()
        assert websocket.close_code == 1000, (
            f"expected proper close handshake (1000), got {websocket.close_code}"
        )
    finally:
        try:
            server.shutdown("test shutdown")
        except Exception:  # noqa: BLE001, S110
            pass


@pytest.mark.asyncio
async def test_websocket_receive_output(streaming_server):
    """Test receiving terminal output via WebSocket."""
    _server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # Send command to terminal
        # Note: We can't directly access pty_terminal here, so this is a simplified test

        # Wait for any output
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=2.0)
            # Messages are binary frames: 1-byte compression flag + protobuf
            assert isinstance(message, bytes)
            decoded = decode_server_message(message)
            assert decoded["type"]
        except TimeoutError:
            pytest.skip("No output received within timeout")


@pytest.mark.asyncio
async def test_websocket_multiple_clients(streaming_server):
    """Test multiple WebSocket clients connecting simultaneously."""
    _server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    # Connect multiple clients
    clients = []
    try:
        for i in range(3):
            client = await websockets.connect(uri, close_timeout=1)
            clients.append(client)

        # All should be connected
        assert len(clients) == 3
        for client in clients:
            assert client.state.name == "OPEN"

    finally:
        # Close all clients
        for client in clients:
            await client.close()


@pytest.mark.asyncio
async def test_websocket_client_disconnect(streaming_server):
    """Test client disconnect and reconnect."""
    _server, port = streaming_server

    uri = f"ws://127.0.0.1:{port}"

    # Connect
    websocket = await websockets.connect(uri, close_timeout=1)
    assert websocket.state.name == "OPEN"

    # Disconnect
    await websocket.close()
    assert websocket.state.name == "CLOSED"

    # Reconnect
    websocket = await websockets.connect(uri, close_timeout=1)
    assert websocket.state.name == "OPEN"
    await websocket.close()


# Output Broadcasting Tests


@pytest.mark.asyncio
async def test_broadcast_to_all_clients(pty_terminal, streaming_port):
    """Test broadcasting output to all connected clients."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    # Connect two clients
    client1 = await websockets.connect(uri, close_timeout=1)
    client2 = await websockets.connect(uri, close_timeout=1)

    try:
        # Send output through terminal
        test_message = "Hello, World!\n"
        pty_terminal.write_str(test_message)

        # Both clients should receive the output
        # This is a simplified test - actual output may include ANSI codes
        received = []
        try:
            msg1 = await asyncio.wait_for(client1.recv(), timeout=1.0)
            received.append(msg1)
        except TimeoutError:
            pass

        try:
            msg2 = await asyncio.wait_for(client2.recv(), timeout=1.0)
            received.append(msg2)
        except TimeoutError:
            pass

        # At least one client should have received something
        assert len(received) > 0

    finally:
        await client1.close()
        await client2.close()
        server.shutdown("test shutdown")


# Configuration and Limits Tests


@pytest.mark.asyncio
async def test_max_clients_limit(pty_terminal, streaming_port):
    """A third client is refused once max_clients are connected."""
    config = StreamingConfig(max_clients=2)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"
    client1 = await websockets.connect(uri, close_timeout=1)
    client2 = await websockets.connect(uri, close_timeout=1)
    try:
        assert client1.state.name == "OPEN"
        assert client2.state.name == "OPEN"
        wait_for(lambda: server.client_count() == 2)

        # The third handshake must be refused: the server closes the TCP
        # connection during the handshake (EOF before an HTTP response), or
        # closes the stream right after it completes.
        third_refused = False
        try:
            third = await websockets.connect(uri, close_timeout=1)
        except (websockets.exceptions.WebSocketException, EOFError, OSError):
            third_refused = True
        else:
            try:
                await asyncio.wait_for(third.recv(), timeout=2.0)
            except websockets.exceptions.ConnectionClosed:
                third_refused = True
        assert third_refused, "the third client is refused past max_clients=2"
        assert server.client_count() == 2
    finally:
        await client1.close()
        await client2.close()
        server.shutdown("test shutdown")


@pytest.mark.asyncio
async def test_send_initial_screen_enabled(pty_terminal, streaming_port):
    """Test initial screen sending when enabled."""
    config = StreamingConfig(send_initial_screen=True)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # The first message is `connected`; with send_initial_screen enabled
        # it carries the screen dump in its initial_screen field
        message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
        decoded = decode_server_message(message)
        assert decoded["type"] == "connected"
        assert decoded.get("initial_screen")

    server.shutdown("test shutdown")


@pytest.mark.asyncio
async def test_send_initial_screen_disabled(pty_terminal, streaming_port):
    """Test no initial screen when disabled."""
    config = StreamingConfig(send_initial_screen=False)
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}", config)
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # A client always receives the `connected` message; with
        # send_initial_screen disabled it must arrive without screen content
        message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
        decoded = decode_server_message(message)
        assert decoded["type"] == "connected"
        assert not decoded.get("initial_screen")

    server.shutdown("test shutdown")


# Terminal Resizing Tests


@pytest.mark.asyncio
async def test_terminal_resize_notification(pty_terminal, streaming_port):
    """Test terminal resize notifications through WebSocket."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # Resize terminal
        pty_terminal.resize(100, 30)

        # Client might receive resize notification
        # (Implementation-specific behavior)
        try:
            message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
            # If we received something, validate it
            assert message is not None
        except TimeoutError:
            pass  # Resize notifications may not be implemented

    server.shutdown("test shutdown")


# Error Handling Tests


def test_server_bind_error_duplicate_port(pty_terminal, streaming_port):
    """Test error when binding to already-used port."""
    server1 = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server1.start()
    wait_for(lambda: port_open(streaming_port))

    # Try to create another server on same port. A failed bind surfaces
    # only on the server thread's log, so the observable is that the FIRST
    # server keeps owning the port and serving clients afterwards.
    pty_terminal2 = PtyTerminal(80, 24)
    pty_terminal2.spawn_shell()
    try:
        server2 = StreamingServer(pty_terminal2, f"127.0.0.1:{streaming_port}")
        server2.start()
        server2.shutdown("test shutdown")
    except RuntimeError:
        # Also acceptable: PyStreamingServer's PyO3 bindings raise
        # RuntimeError (PyRuntimeError) for construction/lifecycle
        # failures (see src/python_bindings/streaming.rs).
        pass

    assert port_open(streaming_port), "server1 still owns the port"
    with socket.create_connection(("127.0.0.1", streaming_port), timeout=2.0):
        pass
    server1.shutdown("test shutdown")


def test_server_operations_after_stop(pty_terminal, streaming_port):
    """Test server operations after stopping."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

    server.shutdown("test shutdown")

    # Should report no clients after shutdown
    assert server.client_count() == 0

    # Client count should be 0
    assert server.client_count() == 0


# Performance and Stress Tests


@pytest.mark.asyncio
@pytest.mark.slow
async def test_high_throughput_output(pty_terminal, streaming_port):
    """Test streaming with high-throughput output."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # Generate lots of output (the 10ms pacing spreads the writes)
        for i in range(50):
            pty_terminal.write_str(f"Line {i}: " + "X" * 70 + "\n")
            await asyncio.sleep(0.01)

        # Should have received multiple messages
        messages_received = 0
        try:
            while messages_received < 10:
                await asyncio.wait_for(websocket.recv(), timeout=0.1)
                messages_received += 1
        except TimeoutError:
            pass

        # Should have received at least some messages
        assert messages_received > 0

    server.shutdown("test shutdown")


@pytest.mark.asyncio
@pytest.mark.slow
async def test_many_clients_sequential(pty_terminal, streaming_port):
    """Test many clients connecting and disconnecting sequentially."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    # Connect and disconnect many clients
    for i in range(20):
        client = await websockets.connect(uri, close_timeout=1)
        assert client.state.name == "OPEN"

        # Receive any initial data
        try:
            await asyncio.wait_for(client.recv(), timeout=0.1)
        except TimeoutError:
            pass

        await client.close()
        await asyncio.sleep(0.05)

    server.shutdown("test shutdown")


# Integration Tests


@pytest.mark.asyncio
async def test_full_session_workflow(pty_terminal, streaming_port):
    """Test a complete terminal session workflow with streaming."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

    uri = f"ws://127.0.0.1:{streaming_port}"

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # Execute some commands
        commands = [
            "echo 'Hello from streaming test'",
            "pwd",
            "echo 'Goodbye'",
        ]

        for cmd in commands:
            pty_terminal.write_str(cmd + "\n")

            # Collect output
            output_chunks = []
            try:
                for _ in range(3):
                    chunk = await asyncio.wait_for(websocket.recv(), timeout=1.0)
                    output_chunks.append(chunk)
            except TimeoutError:
                pass

            # Should have received some output
            assert len(output_chunks) > 0

    server.shutdown("test shutdown")


@pytest.mark.asyncio
async def test_concurrent_read_write(pty_terminal, streaming_port):
    """Test concurrent reading and writing with streaming."""
    server = StreamingServer(pty_terminal, f"127.0.0.1:{streaming_port}")
    server.start()
    wait_for(lambda: port_open(streaming_port))

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
        except TimeoutError:
            pass
        return messages

    async with websockets.connect(uri, close_timeout=1) as websocket:
        # Run concurrently
        write_task = asyncio.create_task(write_output())
        messages = await read_from_websocket(websocket)
        await write_task

        # Should have received multiple messages
        assert len(messages) > 0

    server.shutdown("test shutdown")


if __name__ == "__main__":
    pytest.main([__file__, "-v", "-m", "not slow"])
