#!/usr/bin/env python3
"""Generate the Kitty ``o=z`` (zlib-compressed) corpus seeds for the kitty
fuzz target (ENH-020).

SEC-109's zlib bomb went unnoticed because the kitty target never exercised
``o=z``: no seed drove the decompression path, and the default RSS limit
(2048 MB) sat above the bomb's 581 MB peak. These seeds give the fuzzer
valid compressed transmissions to mutate — plain, ``m=1`` chunked, and a
high-expansion stream — and the fuzz runs themselves are memory-bounded
with ``-rss_limit_mb=512`` (Makefile ``fuzz-*`` targets and fuzz.yml).

Run from anywhere: paths resolve relative to this file. Idempotent —
regenerating overwrites the corpus files in place.

Committed outputs (fuzz/corpus/kitty/):
  zlib.seed          plain o=z transmit, 16 payload bytes
  zlib_chunked.seed  the same payload as an m=1 (continuation) chunk
  zlib_expansion.seed ~1 KiB zlib stream expanding to 1 MiB of zeros
"""

import base64
import pathlib
import zlib

CORPUS = pathlib.Path(__file__).resolve().parents[1] / "corpus" / "kitty"


def apc(payload: str) -> bytes:
    """Wrap a Kitty TGP payload in the ESC _ G ... ST framing the fuzz
    target splits on."""
    return b"\x1b_G" + payload.encode("ascii") + b"\x1b\\"


def write(name: str, payload: str) -> None:
    path = CORPUS / name
    path.write_bytes(apc(payload))
    print(f"{name}: {len(apc(payload))} bytes")


def main() -> None:
    CORPUS.mkdir(parents=True, exist_ok=True)

    # Plain o=z transmit: 16 raw bytes, control keys declaring a tiny
    # 1-pixel-tall RGBA image (s is the decompressed byte count).
    small = zlib.compress(b"\x00" * 16)
    keys = "a=T,f=32,s=16,v=1,o=z"
    write("zlib.seed", f"{keys};{base64.b64encode(small).decode('ascii')}")

    # The same payload as an m=1 chunk: parse_chunk returns Ok(true) and the
    # parser keeps its state — the continuation arm the plain seed skips.
    write(
        "zlib_chunked.seed",
        f"a=T,m=1,f=32,s=16,v=1,o=z;{base64.b64encode(small).decode('ascii')}",
    )

    # Expansion-ratio seed: a ~1 KiB zlib stream that decompresses to 1 MiB
    # of zeros. Small enough to be a mutation-friendly corpus file, big
    # enough that the o=z decompression path does real allocation work —
    # mutations of this seed are what -rss_limit_mb=512 sits under.
    big = zlib.compress(b"\x00" * (1024 * 1024))
    assert len(big) <= 4096, f"expansion seed too large to mutate well: {len(big)}"
    write(
        "zlib_expansion.seed",
        f"a=T,f=32,s={1024 * 1024},v=1,o=z;{base64.b64encode(big).decode('ascii')}",
    )


if __name__ == "__main__":
    main()
