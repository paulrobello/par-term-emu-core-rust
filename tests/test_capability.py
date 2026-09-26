"""Native build-capability surface: HAS_STREAMING and its Python mirror.

Runs in every build variant (unlike tests/test_streaming.py, which skips
entirely when the streaming feature is absent), so the honest-detection
contract is asserted on both legs.
"""

import par_term_emu_core_rust as ptec
import pytest
from par_term_emu_core_rust import _native

STREAMING_EXPORTS = (
    "StreamingServer",
    "StreamingConfig",
    "encode_server_message",
    "decode_server_message",
    "encode_client_message",
    "decode_client_message",
)


def test_has_streaming_constant_exists():
    assert isinstance(_native.HAS_STREAMING, bool)


def test_has_streaming_matches_native_constant():
    assert ptec._has_streaming == _native.HAS_STREAMING


def test_streaming_exports_track_capability():
    for name in STREAMING_EXPORTS:
        assert hasattr(ptec, name)
        if _native.HAS_STREAMING:
            assert getattr(ptec, name) is not None
            assert name in ptec.__all__
        else:
            assert getattr(ptec, name) is None
            assert name not in ptec.__all__


def test_streaming_constructs_when_advertised():
    if not _native.HAS_STREAMING:
        pytest.skip("streaming feature not built")
    # The stub's constructor raises, so a bare construct is the probe that
    # HAS_STREAMING=True is not over-advertising a stub build.
    config = ptec.StreamingConfig()
    assert config is not None
