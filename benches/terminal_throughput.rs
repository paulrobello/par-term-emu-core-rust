//! Criterion benchmarks for the VTE processing hot path (ENH-007).
//!
//! Every benchmark drives the real ingestion entry point
//! ([`Terminal::process`]) so the numbers cover the whole pipeline: the APC
//! pre-filter, the vte parser, sequence dispatch, grid writes, scrolling, and
//! graphics ingestion (Sixel DCS / Kitty APC). Payloads are generated
//! deterministically in-process — no fixture files.
//!
//! Run with `make bench` (cargo bench --no-default-features --features
//! rust-only). Throughput is reported as bytes/second via
//! `Throughput::Bytes`, so criterion prints MB/s per benchmark. Compare
//! against a saved baseline with `--save-baseline` / `--baseline` (see
//! CONTRIBUTING.md, "Benchmarks").

use base64::Engine as _;
use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use par_term_emu_core_rust::terminal::Terminal;

/// Total payload sizes per iteration. Small enough that a full suite run
/// stays in the low minutes, large enough that per-iteration overhead is
/// noise. SGR/unicode streams are denser work per byte, so they use less.
const PLAIN_BYTES: usize = 1 << 20; // 1 MiB
const SGR_BYTES: usize = 256 << 10; // 256 KiB
const UNICODE_BYTES: usize = 256 << 10; // 256 KiB
const SCROLL_BYTES: usize = 1 << 20; // 1 MiB
const CURSOR_BYTES: usize = 512 << 10; // 512 KiB
const SIXEL_IMAGES: usize = 8; // 128x96 sixel images per iteration
const KITTY_IMAGES: usize = 4; // 96x96 RGB kitty images per iteration

const SIXEL_WIDTH: usize = 128;
const SIXEL_HEIGHT: usize = 96;
const KITTY_SIZE: usize = 96;

/// Tiny deterministic LCG — avoids a `rand` dev-dependency for payload
/// generation while still producing varied, reproducible bytes.
struct Lcg(u64);

impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n
    }
}

const WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
];

/// Plain ASCII text in <=79-column lines — the baseline case.
fn plain_ascii_payload(target: usize) -> Vec<u8> {
    let mut rng = Lcg(0x5EED_0001);
    let mut out = Vec::with_capacity(target + 96);
    while out.len() < target {
        let mut line_len = 0;
        loop {
            let word = WORDS[rng.below(WORDS.len())];
            if line_len + word.len() + 1 > 79 {
                break;
            }
            if line_len > 0 {
                out.push(b' ');
                line_len += 1;
            }
            out.extend_from_slice(word.as_bytes());
            line_len += word.len();
        }
        out.push(b'\n');
    }
    out
}

/// Alternating truecolor SGR escapes and short words — the shape of
/// `ls --color` / compiler diagnostic output.
fn sgr_heavy_payload(target: usize) -> Vec<u8> {
    let mut rng = Lcg(0x5EED_0002);
    let mut out = Vec::with_capacity(target + 64);
    while out.len() < target {
        let (r, g, b) = (
            rng.next_u32() as u8,
            rng.next_u32() as u8,
            rng.next_u32() as u8,
        );
        let word = WORDS[rng.below(WORDS.len())];
        out.extend_from_slice(format!("\x1b[38;2;{r};{g};{b}m{word}\x1b[0m ").as_bytes());
        if rng.below(8) == 0 {
            out.push(b'\n');
        }
    }
    out
}

/// CJK, emoji, ZWJ sequences, and combining marks — exercises the wide-char
/// and grapheme-cluster hard paths in `write_char`.
fn unicode_wide_payload(target: usize) -> Vec<u8> {
    let units: [&str; 6] = [
        "你好世界测试终端",                 // CJK, width 2 per char
        "👍",                               // emoji
        "👨\u{200d}👩\u{200d}👧\u{200d}👦", // ZWJ family
        "e\u{301}",                         // combining acute accent
        "cafe\u{301} au lait",              // mixed ASCII + combiner
        " ",                                // separator
    ];
    let mut rng = Lcg(0x5EED_0003);
    let mut out = Vec::with_capacity(target + 64);
    while out.len() < target {
        let unit = units[rng.below(units.len())];
        out.extend_from_slice(unit.as_bytes());
        if rng.below(12) == 0 {
            out.push(b'\n');
        }
    }
    out
}

/// Long run of short lines on a small grid with scrollback — pure
/// scroll-region/scrollback churn.
fn scroll_payload(target: usize) -> Vec<u8> {
    plain_ascii_payload(target)
}

/// Full-screen repaint pattern — CUP to each row then a full line of text,
/// the shape of a vim redraw.
fn cursor_addressing_payload(target: usize) -> Vec<u8> {
    let mut rng = Lcg(0x5EED_0004);
    let mut line = String::with_capacity(80);
    let mut out = Vec::with_capacity(target + 128);
    while out.len() < target {
        for row in 1..=24 {
            line.clear();
            while line.len() < 78 {
                let word = WORDS[rng.below(WORDS.len())];
                if line.len() + word.len() + 1 > 78 {
                    break;
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            out.extend_from_slice(format!("\x1b[{row};1H{line}").as_bytes());
        }
    }
    out
}

/// One 128x96 sixel image: 16 color definitions, then 16 bands of sixel
/// data characters (one band = 6 pixel rows), colors separated by `$`
/// (carriage return), bands by `-` (newline).
fn sixel_image() -> Vec<u8> {
    let bands = SIXEL_HEIGHT / 6;
    let mut img = Vec::with_capacity(SIXEL_WIDTH * bands * 10 + 256);
    img.extend_from_slice(b"\x1bP0;1;0q");
    img.extend_from_slice(format!("\"1;1;{SIXEL_WIDTH};{SIXEL_HEIGHT}").as_bytes());
    for i in 0..16u32 {
        let (r, g, b) = (i * 16, 255 - i * 12, i * 8 + 20);
        img.extend_from_slice(format!("#{i};2;{r};{g};{b}").as_bytes());
    }
    for band in 0..bands {
        for color in 0..16usize {
            img.extend_from_slice(format!("#{color}").as_bytes());
            for x in 0..SIXEL_WIDTH {
                let sixel = 0x3Fu8 + ((x * 7 + band * 13 + color * 11) % 64) as u8;
                img.push(sixel);
            }
            img.push(b'$');
        }
        img.push(b'-');
    }
    img.extend_from_slice(b"\x1b\\");
    img
}

/// One 96x96 uncompressed RGB Kitty image (`a=T,f=24`), base64-encoded as
/// the APC payload requires.
fn kitty_image() -> Vec<u8> {
    let pixels = KITTY_SIZE * KITTY_SIZE * 3;
    let mut rgb = Vec::with_capacity(pixels);
    for y in 0..KITTY_SIZE {
        for x in 0..KITTY_SIZE {
            rgb.push((x * 255 / KITTY_SIZE) as u8);
            rgb.push((y * 255 / KITTY_SIZE) as u8);
            rgb.push(((x + y) * 255 / (KITTY_SIZE * 2)) as u8);
        }
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&rgb);
    let mut payload = Vec::with_capacity(b64.len() + 64);
    payload.extend_from_slice(b"\x1b_Ga=T,f=24,s=");
    payload.extend_from_slice(format!("{KITTY_SIZE},v={KITTY_SIZE};").as_bytes());
    payload.extend_from_slice(b64.as_bytes());
    payload.extend_from_slice(b"\x1b\\");
    payload
}

fn bench_plain_ascii(c: &mut Criterion) {
    let payload = plain_ascii_payload(PLAIN_BYTES);
    let mut group = c.benchmark_group("plain_ascii");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("1MiB_lorem_80x24", |b| {
        b.iter_batched(
            || Terminal::new(80, 24),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_sgr_heavy(c: &mut Criterion) {
    let payload = sgr_heavy_payload(SGR_BYTES);
    let mut group = c.benchmark_group("sgr_heavy");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("256KiB_truecolor_80x24", |b| {
        b.iter_batched(
            || Terminal::new(80, 24),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_unicode_wide(c: &mut Criterion) {
    let payload = unicode_wide_payload(UNICODE_BYTES);
    let mut group = c.benchmark_group("unicode_wide");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("256KiB_cjk_emoji_zwj_80x24", |b| {
        b.iter_batched(
            || Terminal::new(80, 24),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_scroll(c: &mut Criterion) {
    let payload = scroll_payload(SCROLL_BYTES);
    let mut group = c.benchmark_group("scroll");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("1MiB_lines_80x24_scrollback10k", |b| {
        b.iter_batched(
            || Terminal::with_scrollback(80, 24, 10_000),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_cursor_addressing(c: &mut Criterion) {
    let payload = cursor_addressing_payload(CURSOR_BYTES);
    let mut group = c.benchmark_group("cursor_addressing");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("512KiB_fullscreen_repaint_80x24", |b| {
        b.iter_batched(
            || Terminal::new(80, 24),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_sixel_decode(c: &mut Criterion) {
    let image = sixel_image();
    let payload: Vec<u8> = (0..SIXEL_IMAGES)
        .flat_map(|_| image.iter().copied())
        .collect();
    let mut group = c.benchmark_group("sixel_decode");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("8x_128x96_16color_dcs", |b| {
        b.iter_batched(
            || Terminal::new(80, 24),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

fn bench_kitty_decode(c: &mut Criterion) {
    let image = kitty_image();
    let payload: Vec<u8> = (0..KITTY_IMAGES)
        .flat_map(|_| image.iter().copied())
        .collect();
    let mut group = c.benchmark_group("kitty_decode");
    group.throughput(Throughput::Bytes(payload.len() as u64));
    group.bench_function("4x_96x96_rgb_apc", |b| {
        b.iter_batched(
            || Terminal::new(80, 24),
            |mut term| term.process(&payload),
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_plain_ascii,
    bench_sgr_heavy,
    bench_unicode_wide,
    bench_scroll,
    bench_cursor_addressing,
    bench_sixel_decode,
    bench_kitty_decode,
);
criterion_main!(benches);
