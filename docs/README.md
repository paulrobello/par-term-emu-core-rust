# Documentation Index

An index of the guides and references in `docs/`, grouped by task. Start with the project [README](../README.md) and [QUICKSTART](../QUICKSTART.md); contributors also read [CONTRIBUTING](../CONTRIBUTING.md).

## Table of Contents

- [Getting Started](#getting-started)
- [API References](#api-references)
- [Terminal Features](#terminal-features)
- [Streaming and Multiplexer](#streaming-and-multiplexer)
- [Internals and Contributing](#internals-and-contributing)
- [Testing](#testing)
- [Archives](#archives)

## Getting Started

- [BUILDING.md](BUILDING.md) - Build and install the library from source
- [CROSS_PLATFORM.md](CROSS_PLATFORM.md) - Platform compatibility status and how to keep code portable
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) - Fixes for the most common build, test, streaming, and par-mux failures

## API References

- [API_REFERENCE.md](API_REFERENCE.md) - Complete Python API
- [RUST_USAGE.md](RUST_USAGE.md) - Using the library from pure Rust, including the feature flags
- [FFI_GUIDE.md](FFI_GUIDE.md) - Embedding the emulator from C, C++, Swift, or any language with a C FFI
- [CONFIG_REFERENCE.md](CONFIG_REFERENCE.md) - Terminal configuration options, modes, and settings
- [VT_SEQUENCES.md](VT_SEQUENCES.md) - Quick reference for the supported ANSI/VT escape sequences
- [VT_TECHNICAL_REFERENCE.md](VT_TECHNICAL_REFERENCE.md) - Implementation details, compatibility matrices, and behavior specifications for the sequences

## Terminal Features

- [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) - Advanced terminal emulation features
- [OBSERVERS.md](OBSERVERS.md) - Push-based event delivery through the observer pattern (Rust, Python, FFI)
- [INSTANT_REPLAY.md](INSTANT_REPLAY.md) - Snapshot-based state capture with delta replay through terminal history
- [MACROS.md](MACROS.md) - Keyboard macro recording, YAML serialization, and timed playback
- [FONTS.md](FONTS.md) - The embedded fonts the screenshot renderer uses
- [REGIONAL_FLAG_LIMITATION.md](REGIONAL_FLAG_LIMITATION.md) - Why flag emoji can render wrongly in the web terminal clients

## Streaming and Multiplexer

- [STREAMING.md](STREAMING.md) - WebSocket terminal streaming and the browser frontend
- [MUX.md](MUX.md) - par-mux multiplexer daemon operational reference
- [MUX_DECISIONS.md](MUX_DECISIONS.md) - One-line summaries of the numbered par-mux design decisions cited in code
- [par-mux.md](par-mux.md) - Pointer to the full par-mux design document, maintained in the `par-agent-os` repository
- [MANUAL-PASS.md](MANUAL-PASS.md) - Manual checklist for the attach client in real terminal emulators
- [SECURITY.md](SECURITY.md) - PTY, streaming, and multiplexer security considerations

## Internals and Contributing

- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture: crate layout, components, data flow, build, and CI
- [MATURIN_BEST_PRACTICES.md](MATURIN_BEST_PRACTICES.md) - The project's compliance with Maturin packaging best practices
- [BENCHMARKING.md](BENCHMARKING.md) - Running the throughput benches and the benchmark regression gate
- [DOCUMENTATION_STYLE_GUIDE.md](DOCUMENTATION_STYLE_GUIDE.md) - Standards for writing these docs

## Testing

- [GRAPHICS_TESTING.md](GRAPHICS_TESTING.md) - Quick reference for testing Sixel, iTerm2, and Kitty graphics
- [TESTING_KITTY_ANIMATIONS.md](TESTING_KITTY_ANIMATIONS.md) - Testing Kitty graphics animation support

## Archives

These directories hold point-in-time records. They are kept for history and are not maintained against the current tree, so source paths in them can be stale.

- `fable/` - Committed benchmark baselines (`BENCH-BASELINE-2026-08.md`, `BENCH-BASELINE-2026-09.md`)
- `opus/` - Enhancement implementation plans (`ENH-048` to `ENH-061`) from audit passes
- `superpowers/plans/` - Feature implementation plans
- `research/` - Protocol research notes (OSC 9;4 progress bar)
- `images/` - Image assets used by the docs
