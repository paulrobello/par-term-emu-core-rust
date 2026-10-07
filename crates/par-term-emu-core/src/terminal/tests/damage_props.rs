//! Damage-completeness property test (ENH-025 phase 1).
//!
//! Invariant: for any VT byte stream, every row of the active grid whose
//! visible content (cells, including attributes) changed during `process()`
//! is reported by `get_dirty_rows()`. This guards the dirty-row contract
//! that renderers (FFI dirty ranges, Python `get_dirty_rows`/`mark_clean`)
//! consume, and is the test that lets marking move into the Grid mutators
//! in phase 2.

use crate::terminal::Terminal;
use proptest::prelude::*;

const COLS: usize = 24;
const ROWS: usize = 10;

/// Clone the rows the renderer would show: the active grid, primary or
/// alternate depending on the current mode at snapshot time.
fn snapshot_rows(term: &Terminal) -> Vec<Vec<crate::cell::Cell>> {
    let grid = term.active_grid();
    (0..ROWS)
        .map(|r| grid.row(r).map(|cells| cells.to_vec()).unwrap_or_default())
        .collect()
}

/// One printable ASCII byte.
fn ascii_chunk() -> impl Strategy<Value = Vec<u8>> {
    (0x20u8..=0x7e).prop_map(|b| vec![b])
}

/// A wide character.
fn wide_chunk() -> impl Strategy<Value = Vec<u8>> {
    Just("あ".as_bytes().to_vec())
}

fn crlf_chunk() -> impl Strategy<Value = Vec<u8>> {
    Just(b"\r\n".to_vec())
}

/// CSI with random small params and one of the finals that mutate the
/// screen: ICH(@) DCH(P) IL(L) DL(M) ECH(X) EL(K) ED(J) SU(S) SD(T)
/// DECSTBM(r) CUP(H) VPA(d) CHA(G) SGR(m).
fn csi_chunk() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::collection::vec(0u16..=25, 0..=4),
        prop_oneof![
            Just('@'),
            Just('P'),
            Just('L'),
            Just('M'),
            Just('X'),
            Just('K'),
            Just('J'),
            Just('S'),
            Just('T'),
            Just('r'),
            Just('H'),
            Just('d'),
            Just('G'),
            Just('m'),
        ],
    )
        .prop_map(|(params, final_byte)| {
            let mut out = b"\x1b[".to_vec();
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    out.push(b';');
                }
                out.extend_from_slice(p.to_string().as_bytes());
            }
            out.push(final_byte as u8);
            out
        })
}

/// DEC rectangle ops: DECFRA($x) DECSERA($v) DECERA($z) DECCARA($r)
/// DECRARA($t) with top;left;bottom;right params.
fn rect_chunk() -> impl Strategy<Value = Vec<u8>> {
    (
        0u16..=(ROWS as u16 + 3),
        0u16..=(COLS as u16 + 3),
        0u16..=(ROWS as u16 + 3),
        0u16..=(COLS as u16 + 3),
        prop_oneof![Just('x'), Just('v'), Just('z'), Just('r'), Just('t')],
    )
        .prop_map(|(top, left, bottom, right, final_byte)| {
            format!("\x1b[{top};{left};{bottom};{right}${final_byte}").into_bytes()
        })
}

fn esc_chunk() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just(b"\x1bc".to_vec()),       // RIS
        Just(b"\x1b7".to_vec()),       // DECSC
        Just(b"\x1b8".to_vec()),       // DECRC
        Just(b"\x1b[?1049h".to_vec()), // enter alt screen
        Just(b"\x1b[?1049l".to_vec()), // leave alt screen
        Just(b"\x1bD".to_vec()),       // IND
        Just(b"\x1bM".to_vec()),       // RI
        Just(b"\x1bE".to_vec()),       // NEL
    ]
}

fn vt_stream() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![
            10 => ascii_chunk(),
            2 => wide_chunk(),
            4 => crlf_chunk(),
            6 => csi_chunk(),
            2 => rect_chunk(),
            3 => esc_chunk(),
        ],
        0..120,
    )
    .prop_map(|chunks| chunks.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn damage_covers_every_changed_row(bytes in vt_stream()) {
        let mut term = Terminal::new(COLS, ROWS);
        // Seed some content so erase/scroll ops have something to change.
        term.process(b"seed line one\r\nseed line two\r\nseed line three");
        term.mark_clean();

        let before = snapshot_rows(&term);
        term.process(&bytes);
        let after = snapshot_rows(&term);
        let dirty = term.get_dirty_rows();

        for row in 0..ROWS {
            if before[row] != after[row] {
                prop_assert!(
                    dirty.contains(&row),
                    "row {row} changed but is not dirty; stream={:?}",
                    String::from_utf8_lossy(&bytes)
                );
            }
        }
    }

    /// ENH-038: a renderer holding the previous frame rebuilds the screen
    /// by blitting the reported region by the reported delta and re-reading
    /// only the content-dirty rows. The rebuild must be cell-for-cell
    /// identical to a fresh read; any sentinel case falls back to a full
    /// read.
    #[test]
    fn scroll_blit_plus_content_dirty_rebuilds_the_screen(bytes in vt_stream()) {
        let mut term = Terminal::new(COLS, ROWS);
        term.process(b"seed line one\r\nseed line two\r\nseed line three");
        let mut prev = snapshot_rows(&term);
        let mut since = term.damage_generation();

        // Two frames per stream so composed scroll ops across a frame
        // boundary are exercised too.
        let mid = bytes.len() / 2;
        for chunk in [&bytes[..mid], &bytes[mid..]] {
            term.process(chunk);
            let report = term.scroll_damage_since(since);
            let fresh = snapshot_rows(&term);
            let blit = if report.full_redraw {
                // Sentinel: the renderer treats every row as dirty and
                // re-reads the whole screen.
                fresh.clone()
            } else {
                let mut frame = prev.clone();
                for row in report.top..=report.bottom {
                    let src = i64::from(row) + i64::from(report.delta);
                    if src >= i64::from(report.top) && src <= i64::from(report.bottom) {
                        frame[row as usize] = prev[src as usize].clone();
                    }
                }
                term.for_each_content_dirty_range_since(since, |start, end| {
                    let grid = term.active_grid();
                    for row in start..=end {
                        if let Some(cells) = grid.row(row as usize) {
                            frame[row as usize] = cells.to_vec();
                        }
                    }
                });
                frame
            };
            prop_assert!(
                blit == fresh,
                "blit+content-dirty rebuild diverged; stream={:?}",
                String::from_utf8_lossy(&bytes)
            );
            prev = fresh;
            since = term.damage_generation();
        }
    }
}
