# ENH-046: cargo-fuzz target for the attach client's `InputParser`

> Filed from the 2026-10-08 /opus-audit enhancement pass (cycle `audit-2026-10-08`). Board card: `[ENH-046]`.
> Sequencing: land this **after SEC-208**. SEC-208 keeps one `InputParser` per `WindowSession` and adds `Token::Paste(Vec<u8>)`, emitting paste bodies as that token instead of `Token::Bytes`. The target below is written against that post-SEC-208 API. It does not compile before SEC-208 because `Token::Paste` does not exist yet. SEC-208's unit tests pin specific splits, and this target generalizes them to fuzzer-chosen splits through one long-lived parser, which is the exact state SEC-208 introduces. The SGR-overflow fix in step 1 has no dependency and can land first as its own commit.

**Priority**: medium · **Estimate**: S

## Goal

The attach client's stdin tokenizer (`InputParser`) consumes bytes from the host terminal, including pasted text, which may be hostile. It is stateful across `feed` calls (`pending` partial sequences and the `paste` flag), and SEC-208 makes that state live for the whole session. No fuzz target covers it, and the existing unit tests exercise hand-picked splits only.

Add a `attach_input` cargo-fuzz target that drives **one long-lived parser** through fuzzer-chosen feed splits and asserts three things:
- `feed` never panics;
- the parser never emits more bytes than it was given;
- a bracketed-paste body (`ESC[200~` … `ESC[201~`) comes out only as `Token::Paste` and never as `Token::Key`, `Token::Mouse`, or a non-empty `Token::Bytes`. `Token::Bytes` is what reaches the prefix and chord scan. The concatenated paste bytes equal the body whatever the split points.

Wire the target into `fuzz/Cargo.toml`, the Makefile, the nightly CI matrix and CONTRIBUTING, the same way the other seven targets are wired.

## Current state

- **Parser.** `crates/par-mux/src/mux/attach/input.rs`:
  - `pub enum Token` at `:40-52` has `Bytes`, `Key` and `Mouse` (SEC-208 adds `Paste`).
  - `pub struct InputParser { pending, paste }` at `:138-142`, with `pub fn feed(&mut self, bytes: &[u8]) -> Vec<Token>` at `:146-210`.
  - Paste-body handling is at `:154-176`. `PASTE_END` is at `:124`, and `PASTE_HELD_CAP = 1 << 20` is at `:130`.
  - `scan_csi` consumes the `ESC[200~` opener and returns an empty `Token::Bytes` at `:326-333`.
- **Designed divergence (lone ESC).** At `:192-198` and in the doc at `:132-137`, a burst that ends on a bare ESC is emitted immediately as the Escape key. If a split falls right after the opener's ESC, `[200~` therefore arrives as plain bytes. That is by design, so the paste invariant must never cut at stream offset 1. The unit-test module documents the same rule at `:670-673`.
- **SGR overflow (a crash the target finds at once).** `input.rs:286` reads `b'0'..=b'9' => fields[field] = fields[field] * 10 + u32::from(data[i] - b'0'),`.
  - `ESC[<` followed by 11 or more digits overflows `u32`. `cargo fuzz` builds with debug assertions and overflow checks on by default (`cargo +nightly fuzz run --help`: "default if not -O"), so this panics within seconds of the first run.
  - Under `cargo test` (dev profile, overflow checks on) it also panics. In a release build it wraps silently.
- **Visibility. No fuzz hook is needed.**
  - `crates/par-mux/src/mux/mod.rs:12-13`: `#[cfg(feature = "attach")] pub mod attach;`.
  - `attach/mod.rs:15`: `pub mod input;`.
  - The root re-exports the member: `src/lib.rs:106-107` (`#[cfg(feature = "mux")] pub use par_mux::mux;`).
  - So `par_term_emu_core_rust::mux::attach::input::{InputParser, Token}` is reachable once the fuzz crate enables the root's `attach` feature (`Cargo.toml:221`, `attach = ["mux-bin", "par-mux/attach"]`).
  - This mirrors `fuzz/fuzz_targets/mux_parse_command.rs:11`, which imports `par_term_emu_core_rust::mux::command::{parse_command, parse_line}` straight from the public API. `apc_filter` is the only target that needs a `cfg(fuzzing)` hook, and that is because its function is private.
- **Fuzz crate.**
  - `fuzz/Cargo.toml` is its own workspace (an empty `[workspace]` table at `:12`).
  - Its dependency is `par-term-emu-core-rust` with `features = ["rust-only", "mux"]` (`:18-21`). The comment block is at `:23-26`.
  - There are seven `[[bin]]` entries, and `mux_hook_report` is last at `:70-75`.
  - Seeds live in `fuzz/corpus/<target>/*.seed`. They are tracked through `fuzz/.gitignore:9-11`.
- **Makefile.**
  - Help lines at `:76-84`: `:77` says "Run all seven fuzz targets".
  - Recipes at `:1069-1088` follow the shape `cargo +nightly fuzz run <t> -- -max_total_time=$(FUZZ_SECONDS) -rss_limit_mb=512`.
  - The `fuzz-all` aggregate is at `:1090`, and its `##` comment says "seven".
  - Fuzz targets are not in `.PHONY`, and that stays as it is.
- **CI.**
  - `.github/workflows/fuzz.yml`: the header comment is at `:3-6`, and the matrix at `:19` is `target: [terminal_process, sixel, kitty, apc_filter, tmux_control, mux_parse_command, mux_hook_report]`.
  - `.github/workflows/README.md:12` says "over the four untrusted-byte parsers". That is already stale, since there are seven.
- **CONTRIBUTING.** `CONTRIBUTING.md:84` lists the targets and ends "All seven have a `make fuzz-<target>` entry and a slot in the CI matrix."

## Implementation

1. **Fix the SGR overflow first, as its own commit with a regression test.** At `crates/par-mux/src/mux/attach/input.rs:286`, replace the digit arm with:
   ```rust
   b'0'..=b'9' => {
       fields[field] = fields[field]
           .saturating_mul(10)
           .saturating_add(u32::from(data[i] - b'0'))
   }
   ```
   The existing `u8::try_from(..).unwrap_or(u8::MAX)` and `u16::try_from(..).unwrap_or(u16::MAX)` at `:291-293` already clamp the saturated value. Add the test to the `#[cfg(test)]` module in the same file:
   ```rust
   #[test]
   fn sgr_mouse_huge_fields_saturate_instead_of_overflowing() {
       let mut parser = InputParser::default();
       let tokens = parser.feed(b"\x1b[<99999999999;99999999999;99999999999M");
       assert_eq!(
           tokens,
           vec![Token::Mouse(SgrMouse { cb: u8::MAX, col: u16::MAX, row: u16::MAX, release: false })]
       );
   }
   ```
   It panics with "attempt to multiply with overflow" before the fix and passes after it.
2. **`fuzz/Cargo.toml`.**
   - Change `features = ["rust-only", "mux"]` to `features = ["rust-only", "mux", "attach"]`.
   - Extend the comment block at `:23-26` to end with: `…grammars, and (ENH-046, +attach) the attach client's stdin tokenizer.`
   - Append:
     ```toml
     [[bin]]
     name = "attach_input"
     path = "fuzz_targets/attach_input.rs"
     test = false
     doc = false
     bench = false
     ```
3. **Create `fuzz/fuzz_targets/attach_input.rs`** with exactly this content:
   ```rust
   #![no_main]

   //! par-mux attach client stdin tokenizer (`InputParser`). The parser is
   //! stateful across feeds (`pending` partial sequences, the `paste` flag)
   //! and, since SEC-208, lives for the whole session, so the target drives ONE
   //! parser through fuzzer-chosen burst splits.
   //!
   //! Input layout: the first `CTRL` bytes are burst lengths (each byte + 1),
   //! the rest is the stdin payload.
   //!
   //! Invariants:
   //! 1. `feed` never panics, and the parser never emits more bytes than it
   //!    was given (pending bytes are re-scanned, never re-emitted).
   //! 2. A bracketed-paste body comes out only as `Token::Paste`. It is never
   //!    a `Key` or `Mouse`, and never a non-empty `Bytes` (the run the prefix
   //!    and chord scanner sees). Empty `Bytes` are the marker-consumption
   //!    drop shape and are allowed.
   //! 3. The concatenated paste bytes equal the body for every split and for
   //!    the whole-stream feed. The one designed divergence, a burst ending
   //!    on the opener's bare ESC (the Escape key, xterm's no-timeout trade),
   //!    is excluded by never cutting at stream offset 1.

   use libfuzzer_sys::fuzz_target;
   use par_term_emu_core_rust::mux::attach::input::{InputParser, Token};

   const PASTE_START: &[u8] = b"\x1b[200~";
   const PASTE_END: &[u8] = b"\x1b[201~";
   const CTRL: usize = 8;

   /// Feed `stream` through one parser in bursts whose lengths `ctrl` encodes
   /// (each byte + 1), then the remainder. A cut landing on `forbid` moves one
   /// byte later.
   fn feed_split(stream: &[u8], ctrl: &[u8], forbid: Option<usize>) -> Vec<Token> {
       let mut parser = InputParser::default();
       let mut tokens = Vec::new();
       let mut at = 0;
       for &c in ctrl {
           let mut cut = (at + usize::from(c) + 1).min(stream.len());
           if Some(cut) == forbid {
               cut = (cut + 1).min(stream.len());
           }
           tokens.extend(parser.feed(&stream[at..cut]));
           at = cut;
       }
       tokens.extend(parser.feed(&stream[at..]));
       tokens
   }

   fn emitted_len(tokens: &[Token]) -> usize {
       tokens
           .iter()
           .map(|t| match t {
               Token::Bytes(b) | Token::Paste(b) => b.len(),
               _ => 0,
           })
           .sum()
   }

   fn paste_bytes(tokens: &[Token]) -> Vec<u8> {
       let mut out = Vec::new();
       for t in tokens {
           match t {
               Token::Paste(b) => out.extend_from_slice(b),
               Token::Bytes(b) if b.is_empty() => {}
               other => panic!("paste body leaked out of Token::Paste as {other:?}"),
           }
       }
       out
   }

   /// Remove every `ESC[201~` until none remain (a removal can splice a new one).
   fn strip_terminators(mut body: Vec<u8>) -> Vec<u8> {
       while let Some(p) = body.windows(PASTE_END.len()).position(|w| w == PASTE_END) {
           body.drain(p..p + PASTE_END.len());
       }
       body
   }

   fuzz_target!(|data: &[u8]| {
       let (ctrl, payload) = data.split_at(data.len().min(CTRL));

       // 1. Arbitrary stdin, arbitrary bursts, one parser.
       let tokens = feed_split(payload, ctrl, None);
       assert!(
           emitted_len(&tokens) <= payload.len(),
           "parser emitted more bytes than it was fed: {tokens:?}"
       );

       // 2 + 3. The payload as a bracketed-paste body.
       let body = strip_terminators(payload.to_vec());
       let mut stream = PASTE_START.to_vec();
       stream.extend_from_slice(&body);
       stream.extend_from_slice(PASTE_END);

       let whole = InputParser::default().feed(&stream);
       assert_eq!(paste_bytes(&whole), body, "whole-stream paste body mismatch");

       let split = feed_split(&stream, ctrl, Some(1));
       assert_eq!(paste_bytes(&split), body, "split-feed paste body mismatch");
   });
   ```
   - Leave the default `-max_len` (4096) alone. Bodies stay far below `PASTE_HELD_CAP` (1 MiB), so the cap-flush path is out of scope for invariant 3.
   - If SEC-208 named the variant something other than `Paste(Vec<u8>)`, rename it in the two `match` arms and nowhere else.
4. **Seeds.** Create `fuzz/corpus/attach_input/` with these four files. Each starts with the 8 control bytes.
   ```bash
   d=fuzz/corpus/attach_input; mkdir -p $d
   printf '\x00\x00\x00\x00\x00\x00\x00\x00\x1b[200~echo hi\x02\x1b[3~\x1b[1;5\x1b tail\x1b[201~x' > $d/paste_hostile.seed
   printf '\x03\x03\x03\x03\x03\x03\x03\x03\x1b[A\x1b[1;5C\x1bOP\x1b[15~\x1bx' > $d/keys.seed
   printf '\x05\x01\x07\x02\x00\x00\x00\x00\x1b[<0;10;5M\x1b[<64;1;1m' > $d/mouse.seed
   printf '\x01\x01\x01\x01\x01\x01\x01\x01\x1b]11;rgb:0000/0000/0000\x07abc' > $d/osc11_reply.seed
   ```
5. **Makefile.**
   - After `:84` add the help line `@echo "  fuzz-attach_input     - Fuzz the par-mux attach stdin tokenizer (paste + split invariants)"`.
   - At `:77` and in the `##` comment of `fuzz-all` at `:1090`, change "seven" to "eight".
   - After the `fuzz-mux_hook_report` recipe (`:1087-1088`) add:
     ```make
     fuzz-attach_input: ## Fuzz the par-mux attach stdin tokenizer (ENH-046)
     	cargo +nightly fuzz run attach_input -- -max_total_time=$(FUZZ_SECONDS) -rss_limit_mb=512
     ```
   - Append ` fuzz-attach_input` to the `fuzz-all:` prerequisite list at `:1090`.
6. **CI.**
   - `.github/workflows/fuzz.yml:19`: append `, attach_input` to the matrix list.
   - In the header comment at `:3-4`, after "the par-mux control-line and hook-report parsers in ARC-121", add ", +the attach stdin tokenizer in ENH-046".
   - `.github/workflows/README.md:12`: change "over the four untrusted-byte parsers" to "over every target in the matrix". The old count was already stale.
7. **CONTRIBUTING.md:84.**
   - Before ` (ENH-018)`, insert `, and the attach client's stdin tokenizer \`attach_input\` (ENH-046: one long-lived parser across fuzzer-chosen splits, asserting paste bodies never leak out of \`Token::Paste\`)`.
   - Change "All seven" to "All eight".

## Files to touch

- `crates/par-mux/src/mux/attach/input.rs` (`:286` saturating arithmetic, plus one `#[test]`)
- `fuzz/Cargo.toml` (`attach` feature, comment, `[[bin]]`)
- `fuzz/fuzz_targets/attach_input.rs` (new)
- `fuzz/corpus/attach_input/{paste_hostile,keys,mouse,osc11_reply}.seed` (new)
- `Makefile` (help line, "seven" → "eight" twice, recipe, `fuzz-all`)
- `.github/workflows/fuzz.yml` (matrix, header comment)
- `.github/workflows/README.md` (`:12` count wording)
- `CONTRIBUTING.md` (`:84`)

## Verify

- `cargo test -p par-mux --features attach --lib sgr_mouse_huge_fields_saturate_instead_of_overflowing > /tmp/enh046-t.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and the log shows `1 passed`.
- `cargo +nightly fuzz build attach_input > /tmp/enh046-b.log 2>&1; echo EXIT=$?`, run from the repo root, prints `EXIT=0`.
- `cargo +nightly fuzz run attach_input -- -max_total_time=120 -rss_limit_mb=512 > /tmp/enh046-r.log 2>&1; echo EXIT=$?` prints `EXIT=0`, and `ls fuzz/artifacts/attach_input/ 2>/dev/null | grep -c crash` prints `0`.
- **Mutation check (the invariant bites).**
  - Temporarily change the post-SEC-208 paste-body emit in `input.rs` from `Token::Paste(..)` to `Token::Bytes(..)`.
  - Then `cargo +nightly fuzz run attach_input fuzz/corpus/attach_input/paste_hostile.seed > /tmp/enh046-m.log 2>&1; echo EXIT=$?` prints a non-zero exit, and the log contains `paste body leaked out of Token::Paste`.
  - Revert the change. `git diff --stat crates/par-mux/src/mux/attach/input.rs` then shows only step 1's edit.
- `grep -c attach_input .github/workflows/fuzz.yml` prints `1`.
- `make -n fuzz-all > /tmp/enh046-n.log 2>&1; echo EXIT=$?; grep -c 'fuzz run' /tmp/enh046-n.log` prints `EXIT=0` and then `8`.
- `make help > /tmp/enh046-h.log 2>&1; grep -c 'fuzz-attach_input' /tmp/enh046-h.log` prints `1`, and `grep -c 'seven' /tmp/enh046-h.log` prints `0`.
- `grep -c 'All eight' CONTRIBUTING.md` prints `1`, and `grep -c 'four untrusted' .github/workflows/README.md` prints `0`.
- `make checkall > /tmp/enh046-c.log 2>&1; echo EXIT=$?` prints `EXIT=0`. The fuzz crate is outside the workspace, so checkall does not build it, and this proves step 1 did not regress the attach tests.

## Rollback

- Delete `fuzz/fuzz_targets/attach_input.rs` and `fuzz/corpus/attach_input/`.
- Revert the `fuzz/Cargo.toml`, Makefile, `fuzz.yml`, workflows README and CONTRIBUTING edits.
- Keep the step 1 overflow fix and its test. It is an independent correctness fix.
