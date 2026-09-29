# ENH-031: Allocation-free hex encoding in the mux-mirror `send_keys_line` input path

> Filed from the 2026-09-28 /opus-audit enhancement pass (cycle `audit-2026-09-28`). Board card: `[ENH-031]`.
> Independent. It touches `src/streaming/mux_factory.rs`, which AUDIT QA-161/QA-170 also edit, so run it after those land, or rebase onto them.

## Goal

Streaming input for par-mux-backed sessions is forwarded as `send-keys -t %N -H <hex>` control lines. `send_keys_line` (`src/streaming/mux_factory.rs:205-211`) calls `format!(" {byte:02x}")` once per input byte, allocating one heap `String` for every byte. A 256 KiB paste costs about 262K small allocations on the input path. The goal is to encode into one pre-sized buffer with a nibble table.

## Current state

```rust
fn send_keys_line(pane: u32, chunk: &[u8]) -> String {
    let mut line = format!("send-keys -t %{pane} -H");
    for byte in chunk {
        line.push_str(&format!(" {byte:02x}"));
    }
    line
}
```
- Called from `SendKeysWriter::write` (`:215-230`), which holds the `LocalStream` mutex while writing.
- The output format (a space-separated lowercase 2-digit hex list) is the daemon's `send-keys -H` grammar, parsed in `src/mux/command.rs`. It must stay byte-identical.

## Implementation

1. Rewrite:
   ```rust
   fn send_keys_line(pane: u32, chunk: &[u8]) -> String {
       const HEX: &[u8; 16] = b"0123456789abcdef";
       let mut line = String::with_capacity(24 + 3 * chunk.len());
       use std::fmt::Write as _;
       let _ = write!(line, "send-keys -t %{pane} -H");
       for &b in chunk {
           line.push(' ');
           line.push(HEX[(b >> 4) as usize] as char);
           line.push(HEX[(b & 0x0f) as usize] as char);
       }
       line
   }
   ```
2. Unit test: for all 256 byte values plus a mixed chunk, the output equals the old implementation's output. Keep the old function as `#[cfg(test)] fn send_keys_line_reference`.
3. Optional (measure first): let `SendKeysWriter` reuse one `String` buffer across writes (`clear()` + write). Do this only if the benchmark in Verify shows the per-call allocation still matters.

## Files to touch

- `src/streaming/mux_factory.rs`

## Verify

- `cargo test --lib --no-default-features --features pyo3/auto-initialize,streaming mux_factory`: the equivalence test passes for all 256 values.
- A micro-benchmark (a criterion bench in `benches/`, or a `#[test] #[ignore]` timing loop) encoding a 256 KiB chunk shows at least a 5× speed-up over the reference. Report the numbers. A smaller win still ships if equivalence holds, but say so.
- The streaming mux-backed session tests pass: `make test-rust-streaming` and `cargo test --test test_streaming …` per the Makefile invocation.
- `make checkall` is green.

## Rollback

Revert the function. The format is unchanged, so no compatibility concerns.
