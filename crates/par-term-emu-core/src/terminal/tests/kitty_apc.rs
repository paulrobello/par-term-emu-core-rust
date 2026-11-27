//! Phase 1 Kitty TGP ingestion tests.
//!
//! Verifies that `Terminal::process` strips Kitty APC sequences from the
//! raw byte stream, feeds them to `KittyParser`, and stores results in the
//! terminal's `GraphicsStore`. Surrounding text and non-Kitty APCs must be
//! unaffected.

use crate::terminal::Terminal;

#[test]
fn ingests_complete_kitty_transmit_apc() {
    let mut term = Terminal::new(80, 24);

    // 16 chars of base64 'A' decode to 12 zero bytes = 2 * 2 * 3 (s*v*RGB)
    term.process(b"\x1b_Ga=t,f=24,i=42,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");

    let img = term.graphics.graphics_store.get_kitty_image(42);
    assert!(img.is_some(), "image id 42 should be stored");
    let (w, h, data) = img.unwrap();
    assert_eq!(w, 2);
    assert_eq!(h, 2);
    // 12 bytes RGB are normalized to 16 bytes RGBA on storage.
    assert_eq!(data.len(), 16);
}

#[test]
fn ingests_virtual_placement_apc() {
    let mut term = Terminal::new(80, 24);

    // First transmit the image so it exists.
    term.process(b"\x1b_Ga=t,f=24,i=42,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");
    // Then place it virtually.
    term.process(b"\x1b_Ga=p,U=1,i=42,c=10,r=5\x1b\\");

    let placements = term.graphics.graphics_store.all_virtual_placements();
    assert!(
        placements.contains_key(&(42, 0)),
        "virtual placement for (image=42, placement=0) should exist; got keys: {:?}",
        placements.keys().collect::<Vec<_>>()
    );
}

#[test]
fn ingests_transmit_then_regular_placement_apc() {
    let mut term = Terminal::new(80, 24);

    // Transmit a 2x2 RGB image under id 42 (16 base64 'A' chars decode to
    // 12 zero bytes = 2*2*3). Transmit-only stores the shared image.
    term.process(b"\x1b_Ga=t,f=24,i=42,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");

    // Regular (non-virtual) placement of the transmitted image.
    // `KittyParser::build_graphic` returns `KittyGraphicResult::Graphic`
    // WITHOUT storing it; the terminal layer must commit it via
    // `GraphicsStore::add_graphic` or the graphic is silently dropped.
    term.process(b"\x1b_Ga=p,i=42\x1b\\");

    let graphics = term.graphics.graphics_store.all_graphics();
    let placed: Vec<_> = graphics
        .iter()
        .filter(|g| g.kitty_image_id == Some(42))
        .collect();
    assert_eq!(
        placed.len(),
        1,
        "placement APC must store exactly one graphic for image 42; got {}",
        placed.len()
    );
    assert_eq!(
        (placed[0].width, placed[0].height),
        (2, 2),
        "stored graphic must carry the transmitted pixel dimensions"
    );
    assert!(
        !placed[0].pixels.is_empty(),
        "stored graphic must carry the transmitted pixels"
    );
}

#[test]
fn handles_apc_split_across_process_calls() {
    let mut term = Terminal::new(80, 24);

    // Split mid-payload — first half ends inside the base64 data.
    term.process(b"\x1b_Ga=t,f=24,i=99,s=2,v=2;AAAA");
    term.process(b"AAAAAAAAAAAA\x1b\\");

    let img = term.graphics.graphics_store.get_kitty_image(99);
    assert!(
        img.is_some(),
        "image id 99 should be stored after split APC"
    );
    let (w, h, data) = img.unwrap();
    assert_eq!(w, 2);
    assert_eq!(h, 2);
    // 12 bytes RGB are normalized to 16 bytes RGBA on storage.
    assert_eq!(data.len(), 16);
}

/// Chunked transmission (m=1 continuation frames) delivered one APC frame
/// per `process()` call — the shape a real PTY read loop produces — must
/// retain the Kitty assembler state across the calls: the final frame
/// dispatches with the concatenated payload, not an empty one.
#[test]
fn chunked_transmission_across_separate_process_calls() {
    let mut term = Terminal::new(80, 24);

    // A 2x2 RGBA image is 16 bytes: 24 base64 chars. Split at a base64
    // block boundary (12 + 12 chars = 9 + 7 decoded bytes).
    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);
    let (first, rest) = b64.split_at(12);

    // Control: the identical transmission delivered in one process() call.
    term.process(format!("\x1b_Ga=T,f=32,i=41,s=2,v=2,q=1;{b64}\x1b\\").as_bytes());
    let control = term.graphics.graphics_store.get_kitty_image(41);
    assert!(
        control.is_some(),
        "single-frame control must pass; the divergence under test is chunking"
    );

    // Frame 1 (a=T header frame with m=1) arrives in its own process() call.
    term.process(format!("\x1b_Ga=T,f=32,i=42,s=2,v=2,q=1,m=1;{first}\x1b\\").as_bytes());
    // Frame 2 (m=0 final) arrives in the next process() call.
    term.process(format!("\x1b_Gm=0;{rest}\x1b\\").as_bytes());

    let img = term.graphics.graphics_store.get_kitty_image(42);
    let (w, h, data) = img.unwrap_or_else(|| {
        panic!(
            "chunked transmission across process() calls must store image 42; \
             got none — assembler state lost across APC frames"
        )
    });
    assert_eq!(w, 2);
    assert_eq!(h, 2);
    assert_eq!(
        data.as_ref(),
        pixel_data.as_slice(),
        "assembled chunked payload must match the single-frame payload"
    );
}

/// The spec allows other escape codes between chunks of a chunked
/// transmission. An interleaved Kitty command — here an icat-style `a=q`
/// capability probe — must not disturb the open transmission: it runs to
/// its own completion while the accumulated chunks stay parked, and the
/// final `m=0` frame still assembles the full payload. Pre-fix, the probe
/// overwrote `action`/params on the shared parser, so the final frame
/// dispatched the query branch and silently discarded the pixel data.
#[test]
fn chunked_transmission_survives_interleaved_probe() {
    let mut term = Terminal::new(80, 24);

    let pixel_data: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
    ];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);
    let (first, rest) = b64.split_at(12);

    term.process(format!("\x1b_Ga=T,f=32,i=42,s=2,v=2,q=1,m=1;{first}\x1b\\").as_bytes());
    // Capability probe while the transmission is in flight.
    term.process(b"\x1b_Gi=42,a=q,q=1\x1b\\");
    term.process(format!("\x1b_Gm=0;{rest}\x1b\\").as_bytes());

    let (w, h, data) = term
        .graphics
        .graphics_store
        .get_kitty_image(42)
        .unwrap_or_else(|| {
            panic!(
                "chunked transmission must survive an interleaved a=q probe; \
                 image 42 missing — assembler state lost across APC frames"
            )
        });
    assert_eq!(w, 2);
    assert_eq!(h, 2);
    assert_eq!(data.as_ref(), pixel_data.as_slice());
}

/// Same contract for an interleaved `a=p` put: pre-fix, a put with an
/// explicit `m=0` dispatched against the open transmission's parser and its
/// reset wiped the accumulated chunks, so the final frame could never
/// assemble them.
#[test]
fn chunked_transmission_survives_interleaved_put() {
    let mut term = Terminal::new(80, 24);

    let pixel_data: Vec<u8> = vec![9u8; 16];
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pixel_data);
    let (first, rest) = b64.split_at(12);

    term.process(format!("\x1b_Ga=T,f=32,i=43,s=2,v=2,q=1,m=1;{first}\x1b\\").as_bytes());
    // Placement attempt (explicit m=0) while the transmission is in flight;
    // pre-fix its reset wiped the parked chunks.
    term.process(b"\x1b_Ga=p,i=43,q=1,m=0\x1b\\");
    term.process(format!("\x1b_Gm=0;{rest}\x1b\\").as_bytes());

    let (w, h, data) = term
        .graphics
        .graphics_store
        .get_kitty_image(43)
        .unwrap_or_else(|| {
            panic!(
                "chunked transmission must survive an interleaved a=p put; \
                 image 43 missing — assembler state lost across APC frames"
            )
        });
    assert_eq!(w, 2);
    assert_eq!(h, 2);
    assert_eq!(data.as_ref(), pixel_data.as_slice());
}

#[test]
fn passes_surrounding_text_through() {
    let mut term = Terminal::new(80, 24);

    // 4 chars of 'A' base64 = 3 zero bytes = 1 * 1 * 3 (1x1 RGB pixel)
    term.process(b"hi\x1b_Ga=t,f=24,i=7,s=1,v=1;AAAA\x1b\\bye");

    // Image should have been stored.
    assert!(
        term.graphics.graphics_store.get_kitty_image(7).is_some(),
        "image id 7 should be stored"
    );

    // Surrounding text should appear on the screen.
    let grid = term.active_grid();
    let cells: String = (0..5)
        .map(|c| grid.get(c, 0).map(|cell| cell.c).unwrap_or(' '))
        .collect();
    assert_eq!(cells, "hibye");
}

#[test]
fn query_emits_ok_response_on_response_buffer() {
    let mut term = Terminal::new(80, 24);

    // Standard kitty TGP detection probe: a=q with a tiny RGB blob.
    // 16 'A' chars decode to 12 zero bytes = 2x2 RGB.
    term.process(b"\x1b_Gi=42,a=q,s=2,v=2,f=24;AAAAAAAAAAAAAAAA\x1b\\");

    assert!(
        term.has_pending_responses(),
        "query should populate response_buffer"
    );
    let response = term.drain_responses();
    assert_eq!(
        response, b"\x1b_Gi=42;OK\x1b\\",
        "response should be APC G i=42;OK ST"
    );

    // Query must not register an image.
    assert!(term.graphics.graphics_store.get_kitty_image(42).is_none());
}

#[test]
fn quiet_mode_query_suppresses_response() {
    let mut term = Terminal::new(80, 24);

    // q=2 means "suppress all replies".
    term.process(b"\x1b_Gi=42,a=q,q=2,s=2,v=2,f=24;AAAAAAAAAAAAAAAA\x1b\\");

    assert!(
        !term.has_pending_responses(),
        "q=2 must suppress response (got: {:?})",
        term.drain_responses()
    );
}

#[test]
fn query_without_image_id_emits_ok_without_id() {
    let mut term = Terminal::new(80, 24);

    // No i= parameter; spec allows omitting i= in the response.
    term.process(b"\x1b_Ga=q,s=2,v=2,f=24;AAAAAAAAAAAAAAAA\x1b\\");

    assert!(term.has_pending_responses());
    let response = term.drain_responses();
    assert_eq!(response, b"\x1b_G;OK\x1b\\");
}

#[test]
fn transmit_emits_ok_response_and_q1_suppresses_it() {
    let mut term = Terminal::new(80, 24);

    // Phase 1-style transmit: registers the image and, at q=0, replies OK
    // per the quietness matrix. q=1 suppresses the OK reply.
    term.process(b"\x1b_Ga=t,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");
    assert!(
        term.graphics.graphics_store.get_kitty_image(77).is_some(),
        "transmit should still register the image"
    );
    assert_eq!(
        term.drain_responses(),
        b"\x1b_Gi=77;OK\x1b\\",
        "q=0 successful commands reply OK"
    );

    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b_Ga=t,f=24,i=77,q=1,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");
    assert!(term.graphics.graphics_store.get_kitty_image(77).is_some());
    assert!(
        !term.has_pending_responses(),
        "q=1 must suppress the OK reply (got: {:?})",
        term.drain_responses()
    );
}

#[test]
fn failed_put_replies_enoent_and_q2_suppresses_it() {
    let mut term = Terminal::new(80, 24);

    // a=p of an image that was never transmitted: ENOENT, per the spec's
    // `_Gi=<id>;ENOENT:<some detailed error msg>` response example.
    term.process(b"\x1b_Ga=p,i=9,q=0;\x1b\\");
    let response = term.drain_responses();
    assert!(
        response.starts_with(b"\x1b_Gi=9;ENOENT:"),
        "missing-id put must reply ENOENT, got: {:?}",
        String::from_utf8_lossy(&response)
    );

    // q=1 keeps error replies (it suppresses OK replies only).
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b_Ga=p,i=9,q=1;\x1b\\");
    assert!(
        term.drain_responses().starts_with(b"\x1b_Gi=9;ENOENT:"),
        "q=1 must keep error replies"
    );

    // q=2 suppresses all replies.
    let mut term = Terminal::new(80, 24);
    term.process(b"\x1b_Ga=p,i=9,q=2;\x1b\\");
    assert!(
        !term.has_pending_responses(),
        "q=2 must suppress error replies (got: {:?})",
        term.drain_responses()
    );
}

#[test]
fn placeholder_cells_skip_unicode_normalization() {
    // Placeholder cells (U+10EEEE base + combining diacritics) encode an image
    // ID, not real text. The print/write_char path must accept them without
    // running NFC normalization or grapheme width recalc — both would allocate
    // strings per char and dominate frame time for an N×M placement.
    use crate::graphics::placeholder::{number_to_diacritic, PLACEHOLDER_CHAR};

    let mut term = Terminal::new(80, 24);

    let row_diac = number_to_diacritic(0).unwrap();
    let col_diac = number_to_diacritic(0).unwrap();
    let id_diac = number_to_diacritic(0).unwrap();

    let mut placeholder = String::new();
    placeholder.push(PLACEHOLDER_CHAR);
    placeholder.push(row_diac);
    placeholder.push(col_diac);
    placeholder.push(id_diac);

    term.process(placeholder.as_bytes());

    let grid = term.active_grid();
    let cell = grid.get(0, 0).unwrap();
    assert_eq!(cell.c, PLACEHOLDER_CHAR, "base char preserved");
    assert_eq!(
        cell.combining(),
        [row_diac, col_diac, id_diac],
        "combining marks preserved verbatim (no normalization rewrite)"
    );
    assert_eq!(cell.width(), 1, "no width recalc applied");
}

#[test]
fn placeholder_cells_render_at_scale_quickly() {
    // Smoke check: a 40x20 placeholder rectangle (800 cells × 4 chars =
    // 3200 prints) must complete promptly. Before the fast path, NFC
    // normalization on each char + per-combining-mark grapheme normalize
    // pushed this into hundreds of milliseconds; after, it should be
    // sub-10ms. We assert << 200ms to leave headroom for slow CI.
    use crate::graphics::placeholder::{number_to_diacritic, PLACEHOLDER_CHAR};

    let mut term = Terminal::new(80, 24);
    let mut buf = String::with_capacity(40 * 20 * 8);
    for r in 0..20u16 {
        for c in 0..40u16 {
            buf.push(PLACEHOLDER_CHAR);
            buf.push(number_to_diacritic(r).unwrap());
            buf.push(number_to_diacritic(c).unwrap());
            buf.push(number_to_diacritic(0).unwrap());
        }
        buf.push('\r');
        buf.push('\n');
    }

    let start = std::time::Instant::now();
    term.process(buf.as_bytes());
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "40x20 placeholder rect took {:?}, expected < 200ms",
        elapsed
    );
}

#[test]
fn scaled_kitty_placement_uses_c_r_for_rows_and_scroll() {
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(5, 4);
    for _ in 0..20 {
        term.process(b"\n");
    }

    // 20x12 RGB pixels (720 bytes, encoded as 960 base64 zero bytes) are
    // deliberately larger than the explicit two-column, one-row footprint.
    let payload = "A".repeat(960);
    let apc = format!("\x1b_Ga=T,f=24,i=7,s=20,v=12,c=2,r=1;{}\x1b\\", payload);
    term.process(apc.as_bytes());

    let graphic = &term.graphics.graphics_store.all_graphics()[0];
    assert_eq!(graphic.cell_dimensions, Some((5, 4)));
    assert_eq!(graphic.cell_span(5, 4), (2, 1));
    assert_eq!(graphic.height_in_rows(1, 2), 1);
    assert_eq!(term.graphics_at_row(20).len(), 1);
    assert_eq!(term.graphics_at_row(21).len(), 0);

    // A 21-line scroll fully removes the one-row placement. Pixel-derived
    // math would incorrectly retain this native 3-row image.
    term.graphics
        .graphics_store
        .adjust_for_scroll_up_with_scrollback(21, 0, 23, 0);
    assert_eq!(term.graphics_count(), 0);
    assert_eq!(term.graphics.graphics_store.scrollback_count(), 1);
}

#[test]
fn non_kitty_apc_is_left_alone() {
    let mut term = Terminal::new(80, 24);

    // ESC _ X ... ST is a non-Kitty APC. Must not panic, must not register
    // any image, and (per vte semantics) should leave the screen untouched.
    term.process(b"\x1b_Xstuff\x1b\\");

    // No image stored.
    assert!(term.graphics.graphics_store.get_kitty_image(0).is_none());
    assert!(term
        .graphics
        .graphics_store
        .all_virtual_placements()
        .is_empty());

    // First cell should still be empty/blank (default).
    let grid = term.active_grid();
    let cell = grid.get(0, 0).unwrap();
    assert_eq!(cell.c, ' ');
}

/// Herdr sends `CSI row;col H` before each `a=p` APC. The APC filter must
/// advance the passthrough bytes (cursor moves) that precede each APC so
/// slices at different rows get distinct terminal positions.
#[test]
fn interleaved_cursor_moves_produce_distinct_graphic_rows() {
    let mut term = Terminal::new(80, 24);

    // Transmit a 2×4 image (24 bytes RGB → 32 base64 chars).
    term.process(b"\x1b_Ga=t,f=24,i=9,s=2,v=4;AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\x1b\\");

    // Two placement APCs, each preceded by a cursor move to a different row.
    // Stream: CSI 5;10H → a=p(y=0,h=2) → CSI 7;10H → a=p(y=2,h=2)
    term.process(
        b"\x1b[5;10H\x1b_Ga=p,i=9,p=1,c=2,r=1,y=0,h=2\x1b\\\
          \x1b[7;10H\x1b_Ga=p,i=9,p=2,c=2,r=1,y=2,h=2\x1b\\",
    );

    let graphics = term.graphics.graphics_store.all_graphics();
    assert_eq!(graphics.len(), 2, "two slices should be stored");

    // Slice 1 should be at row 4 (0-indexed from CSI row 5).
    assert_eq!(
        graphics[0].position.1, 4,
        "first slice should be at row 4 (CSI row 5, 0-indexed)"
    );
    // Slice 2 should be at row 6 (0-indexed from CSI row 7).
    assert_eq!(
        graphics[1].position.1, 6,
        "following slice should be at row 6 (CSI row 7, 0-indexed)"
    );
}

/// A default Kitty placement moves the cursor to the first line below the
/// image (Kitty TGP cursor movement), spanning the placement's row count.
#[test]
fn placement_advances_cursor_to_line_below() {
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(1, 1);

    // 1×3 pixel image: cell dimensions (1,1) make the span 1 col × 3 rows.
    term.process(b"\x1b_Ga=t,f=24,i=5,s=1,v=3;AAAAAAAAAAAA\x1b\\");
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 0),
        "transmit alone never moves the cursor"
    );

    term.process(b"\x1b_Ga=p,i=5\x1b\\");
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 3),
        "cursor must land on the first line below the 3-row image"
    );
}

/// `C=1` suppresses the post-placement cursor move; `C=0` and an omitted `C=`
/// keep the default advance. Covers the parser field and the end-to-end path.
#[test]
fn placement_c_key_controls_cursor_movement() {
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(1, 1);
    term.process(b"\x1b_Ga=t,f=24,i=5,s=1,v=3;AAAAAAAAAAAA\x1b\\");

    term.process(b"\x1b_Ga=p,i=5,C=1\x1b\\");
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 0),
        "C=1 must suppress the cursor move"
    );
    assert!(
        term.graphics.graphics_store.all_graphics().len() == 1,
        "C=1 suppresses only the cursor move, not the placement"
    );

    term.process(b"\x1b_Ga=p,i=5,C=0\x1b\\");
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 3),
        "C=0 keeps the default (cursor advances)"
    );

    term.process(b"\x1b_Ga=p,i=5\x1b\\");
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 6),
        "omitted C= keeps the default; advances accumulate across placements"
    );
}

/// The post-placement advance flows through the ordinary newline path, so a
/// placement that runs into the scroll region's bottom margin scrolls the
/// region instead of moving the cursor past it.
#[test]
fn placement_advance_respects_scroll_region() {
    let mut term = Terminal::new(80, 24);
    // Scroll region rows 0..=9 (1-indexed 1;10). Park the cursor on the
    // region's last row and place a 3-row image: two of the three advance
    // newlines scroll the region.
    term.set_cell_dimensions(1, 1);
    term.process(b"\x1b[1;10r\x1b[10;1H");
    assert_eq!((term.cursor().col, term.cursor().row), (0, 9));

    term.process(b"\x1b_Ga=t,f=24,i=5,s=1,v=3;AAAAAAAAAAAA\x1b\\");
    term.process(b"\x1b_Ga=p,i=5\x1b\\");

    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 9),
        "cursor clamps to the scroll region's bottom margin after scrolling"
    );
    // The scrolled placement rides the scroll like any multi-row write:
    // placed at the region's bottom row, each advance newline scrolls it up.
    assert_eq!(
        term.graphics_at_row(6).len(),
        1,
        "placement moved up with the scroll"
    );
    assert_eq!(term.graphics_at_row(9).len(), 0);
}

/// Virtual placements (U=1) occupy cells via placeholder bookkeeping but do
/// not move the cursor — kitty applies the same rule to unicode placements.
#[test]
fn virtual_placement_does_not_advance_cursor() {
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(1, 1);
    term.process(b"\x1b_Ga=t,f=24,i=5,s=1,v=3;AAAAAAAAAAAA\x1b\\");

    term.process(b"\x1b_Ga=p,i=5,U=1,c=2,r=2\x1b\\");
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 0),
        "U=1 virtual placement must not move the cursor"
    );
    assert!(
        !term
            .graphics
            .graphics_store
            .all_virtual_placements()
            .is_empty(),
        "virtual placement is still registered"
    );
}

/// a=T (transmit-and-display) and a=p (put) must emit GraphicsAdded like the
/// Sixel (DCS) and iTerm2 inline paths do — otherwise observers subscribed to
/// graphics_added and the streaming server's graphics subscribers never hear
/// kitty placements (a=t is transmit-only and displays nothing, so it stays
/// silent). The event carries the placement row, captured before the cursor
/// advance, plus the store id the frame's pixels are fetched by.
#[test]
fn display_paths_emit_graphics_added() {
    use crate::terminal::TerminalEvent;

    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(1, 1);

    // a=T: transmit a 2x2 image and display it at the cursor (0, 0).
    term.process(b"\x1b_Ga=T,f=24,i=42,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");
    let (image_id, row) = term
        .poll_events()
        .into_iter()
        .find_map(|e| match e {
            TerminalEvent::GraphicsAdded { image_id, row } => Some((image_id, row)),
            _ => None,
        })
        .expect("a=T must emit GraphicsAdded");
    assert_eq!(row, 0, "a=T at cursor (0, 0) reports the placement row");
    assert!(
        term.all_graphics().iter().any(|g| g.id == image_id),
        "the event's image_id must resolve to the stored graphic"
    );

    // a=p: re-displaying the stored image is a new placement at the
    // current cursor row.
    let row_before = term.cursor().row;
    term.process(b"\x1b_Ga=p,i=42\x1b\\");
    let (image_id, row) = term
        .poll_events()
        .into_iter()
        .find_map(|e| match e {
            TerminalEvent::GraphicsAdded { image_id, row } => Some((image_id, row)),
            _ => None,
        })
        .expect("a=p must emit GraphicsAdded");
    assert_eq!(row, row_before, "a=p reports the placement row");
    assert!(
        term.all_graphics().iter().any(|g| g.id == image_id),
        "the re-display's image_id must resolve too"
    );
}

/// Card 01a0d9e6f012, criterion 3: after displaying an image the cursor row
/// must match between a daemon-side terminal and the client-side mirror
/// rendering it. Both derive rows from the cell size, so a terminal whose
/// host reported real text-area pixels (`set_pixel_size`, what the daemon's
/// cell-pixel resize drives on `refresh-client -p`) advances
/// by pixel-height / cell-height — not by the (1, 2) no-information default,
/// which would put the daemon dozens of rows past the mirror.
#[test]
fn image_cursor_advance_uses_the_reported_cell_size() {
    let mut term = Terminal::new(80, 24);
    // The client's metrics: an 80x24 grid at 12x24 px cells.
    term.set_pixel_size(80 * 12, 24 * 24);
    assert_eq!(
        term.graphics.cell_dimensions,
        (12, 24),
        "reporting text-area pixels derives the graphics cell size"
    );

    // a=T transmits and displays a 2x24 px image: 2*24*3 = 144 zero bytes
    // = 192 base64 'A' chars. The length is computed, not hand-counted —
    // a payload short of the declared dimensions is silently rejected (see
    // `short_payload_for_declared_dims_is_silently_rejected` below).
    let sequence = format!("\x1b_Ga=T,f=24,i=7,s=2,v=24;{}\x1b\\", "A".repeat(192));
    term.process(sequence.as_bytes());
    assert_eq!(
        term.cursor().row, 1,
        "a 24 px image over 24 px cells advances one row — the mirror's advance, where the (1, 2) default would advance 12 and desync the grids"
    );
}

/// Card 01a0db7800637762b6c3189dee5c9189: a 1-wide, 24-tall RGB image
/// (`s=1,v=24`, 1*24*3 = 72 bytes = exactly 96 base64 chars) must store a
/// graphic and emit GraphicsAdded like every other well-formed shape — the
/// card's "silently produces no graphic" did not reproduce at HEAD in any
/// context (fresh terminal, with/without `set_pixel_size`, parser-direct).
/// Pins the shape so a regression of exactly this dimension pair is caught.
#[test]
fn w1_h24_exact_payload_stores_graphic_and_emits_event() {
    use crate::terminal::TerminalEvent;

    let mut term = Terminal::new(80, 24);
    term.set_pixel_size(80 * 12, 24 * 24);

    let sequence = format!("\x1b_Ga=T,f=24,i=7,s=1,v=24;{}\x1b\\", "A".repeat(96));
    term.process(sequence.as_bytes());

    let graphics = term.graphics.graphics_store.all_graphics();
    assert_eq!(graphics.len(), 1, "the 1x24 placement must be stored");
    assert_eq!(
        (graphics[0].width, graphics[0].height),
        (1, 24),
        "stored graphic carries the transmitted pixel dimensions"
    );
    assert!(
        term.poll_events()
            .into_iter()
            .any(|e| matches!(e, TerminalEvent::GraphicsAdded { .. })),
        "a=T must emit GraphicsAdded for the 1x24 shape"
    );
    assert_eq!(
        term.cursor().row,
        1,
        "24 px over 24 px cells advances one row, same rule as the 2x24 shape"
    );
}

/// The rejection the filing session actually hit (misattributed to 1x24):
/// a payload whose decoded length disagrees with the declared dimensions is
/// rejected by `KittyParser::decode_pixels` ("Data size mismatch") inside
/// `build_graphic`, whose error the terminal layer swallows by design
/// ("Errors here are non-fatal" — a malformed graphic must never kill the
/// emulator). The rejection is silent on purpose: no graphic, no event, no
/// cursor move. Kitty TGP error replies (`_G...;ENOCI`-style APC responses)
/// are a separate, unimplemented contract.
#[test]
fn short_payload_for_declared_dims_is_silently_rejected() {
    use crate::terminal::TerminalEvent;

    let mut term = Terminal::new(80, 24);
    term.set_pixel_size(80 * 12, 24 * 24);

    // 96 chars decode to 72 bytes, but s=2,v=24 RGB declares 2*24*3 = 144.
    let sequence = format!("\x1b_Ga=T,f=24,i=7,s=2,v=24;{}\x1b\\", "A".repeat(96));
    term.process(sequence.as_bytes());

    assert!(
        term.graphics.graphics_store.all_graphics().is_empty(),
        "mismatched payload must not produce a placement"
    );
    assert!(
        !term
            .poll_events()
            .into_iter()
            .any(|e| matches!(e, TerminalEvent::GraphicsAdded { .. })),
        "no GraphicsAdded for a rejected graphic"
    );
    assert_eq!(
        (term.cursor().col, term.cursor().row),
        (0, 0),
        "a rejected graphic never moves the cursor"
    );
}

/// A program that animates one image by editing its root frame, as kitty-doom
/// does (`a=T` sized in cells with `C=1`, then `a=f,r=1` and `a=a,c=1` for every frame, all
/// sent in chunks), keeps one placement at home with its cell size, and the cursor stays.
#[test]
fn root_frame_edits_repaint_the_placement_in_place() {
    const ST: &str = "\x1b\\";
    let mut term = Terminal::new(80, 24);
    term.set_cell_dimensions(10, 20);
    // A 4×2 RGB image is 24 bytes, 32 base64 characters: two chunks of 12 bytes each.
    let chunks = |value: u8| {
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [value; 24]);
        (b64[..16].to_string(), b64[16..].to_string())
    };
    let (first, rest) = chunks(1);
    term.process(b"\x1b[2J\x1b[H");
    term.process(
        format!("\x1b_Ga=T,i=9,f=24,s=4,v=2,q=2,c=80,r=24,C=1,m=1;{first}{ST}\x1b_Gm=0;{rest}{ST}")
            .as_bytes(),
    );
    let placed = term.graphics.graphics_store.all_graphics()[0].id;

    for value in 2..=4u8 {
        let (first, rest) = chunks(value);
        term.process(
            format!(
                "\x1b_Ga=f,r=1,i=9,f=24,q=2,x=0,y=0,s=4,v=2,m=1;{first}{ST}\
                 \x1b_Ga=f,r=1,q=2,m=0;{rest}{ST}\x1b_Ga=a,q=2,c=1,i=9;{ST}"
            )
            .as_bytes(),
        );
        let graphics = term.graphics.graphics_store.all_graphics();
        assert_eq!(graphics.len(), 1, "frame {value}: still one placement");
        let graphic = &graphics[0];
        assert_eq!(graphic.id, placed);
        assert_eq!(graphic.position, (0, 0));
        assert_eq!(
            (graphic.placement.columns, graphic.placement.rows),
            (Some(80), Some(24))
        );
        assert!(
            graphic
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [value, value, value, 255]),
            "frame {value} shows"
        );
        assert_eq!((term.cursor().col, term.cursor().row), (0, 0));
    }
}

#[test]
fn frame_edits_and_frame_switches_freeze_the_scrollback_copy() {
    // Kitty's model freezes history: a scrollback copy keeps the frame it
    // showed at scroll time, even after the image is edited or switches
    // frames, while live placements follow the current frame.
    let mut term = Terminal::new(80, 24);
    let red = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        [255u8, 0, 0, 255],
    );
    let green = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        [0u8, 255, 0, 255],
    );
    let blue = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        [0u8, 0, 255, 255],
    );

    // Place a red image (row 0) and scroll it into history.
    term.process(format!("\x1b_Ga=T,f=32,s=1,v=1,i=7;{red}\x1b\\").as_bytes());
    term.process(b"\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n");
    assert!(
        term.graphics
            .graphics_store
            .scrollback_entries()
            .iter()
            .any(|g| g.kitty_image_id == Some(7)),
        "the placement should have scrolled into history"
    );

    // A second, live placement of the same image on the new screen.
    term.process(b"\x1b_Ga=p,i=7,p=2;\x1b\\");

    // Edit the root frame in place: the live placement follows, the
    // scrollback copy stays frozen at red.
    term.process(format!("\x1b_Ga=f,f=32,s=1,v=1,i=7,r=1,X=1;{green}\x1b\\").as_bytes());
    let store = &term.graphics.graphics_store;
    let live = store
        .all_graphics()
        .iter()
        .find(|g| g.kitty_image_id == Some(7))
        .expect("live placement");
    assert_eq!(
        live.pixels.as_slice(),
        [0, 255, 0, 255],
        "live placement shows the edit"
    );
    let scrolled = store
        .scrollback_entries()
        .iter()
        .find(|g| g.kitty_image_id == Some(7))
        .expect("scrollback copy");
    assert_eq!(
        scrolled.pixels.as_slice(),
        [255, 0, 0, 255],
        "scrollback copy must stay frozen at the frame it showed at scroll time"
    );

    // A frame switch (a=a,c=N) likewise leaves the scrollback copy alone.
    // Frame 2 carries blue so a follow would repaint the scrollback copy to
    // blue and the freeze assertion below would catch it.
    term.process(format!("\x1b_Ga=f,f=32,s=1,v=1,i=7;{blue}\x1b\\").as_bytes());
    term.process(b"\x1b_Ga=a,i=7,c=2;\x1b\\");
    let store = &term.graphics.graphics_store;
    let live = store
        .all_graphics()
        .iter()
        .find(|g| g.kitty_image_id == Some(7))
        .unwrap();
    assert_eq!(
        live.pixels.as_slice(),
        [0, 0, 255, 255],
        "frame switch shows frame 2"
    );
    let scrolled = store
        .scrollback_entries()
        .iter()
        .find(|g| g.kitty_image_id == Some(7))
        .unwrap();
    assert_eq!(
        scrolled.pixels.as_slice(),
        [255, 0, 0, 255],
        "scrollback copy still frozen at its original frame"
    );
}
