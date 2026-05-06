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

    let img = term.graphics_store.get_kitty_image(42);
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

    let placements = term.graphics_store.all_virtual_placements();
    assert!(
        placements.contains_key(&(42, 0)),
        "virtual placement for (image=42, placement=0) should exist; got keys: {:?}",
        placements.keys().collect::<Vec<_>>()
    );
}

#[test]
fn handles_apc_split_across_process_calls() {
    let mut term = Terminal::new(80, 24);

    // Split mid-payload — first half ends inside the base64 data.
    term.process(b"\x1b_Ga=t,f=24,i=99,s=2,v=2;AAAA");
    term.process(b"AAAAAAAAAAAA\x1b\\");

    let img = term.graphics_store.get_kitty_image(99);
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

#[test]
fn passes_surrounding_text_through() {
    let mut term = Terminal::new(80, 24);

    // 4 chars of 'A' base64 = 3 zero bytes = 1 * 1 * 3 (1x1 RGB pixel)
    term.process(b"hi\x1b_Ga=t,f=24,i=7,s=1,v=1;AAAA\x1b\\bye");

    // Image should have been stored.
    assert!(
        term.graphics_store.get_kitty_image(7).is_some(),
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
    assert!(term.graphics_store.get_kitty_image(42).is_none());
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
fn transmit_does_not_emit_response() {
    let mut term = Terminal::new(80, 24);

    // Phase 1-style transmit: must register the image and produce no reply bytes.
    term.process(b"\x1b_Ga=t,f=24,i=77,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");

    assert!(
        term.graphics_store.get_kitty_image(77).is_some(),
        "transmit should still register the image"
    );
    assert!(
        !term.has_pending_responses(),
        "non-query actions must not write to response_buffer"
    );
}

#[test]
fn non_kitty_apc_is_left_alone() {
    let mut term = Terminal::new(80, 24);

    // ESC _ X ... ST is a non-Kitty APC. Must not panic, must not register
    // any image, and (per vte semantics) should leave the screen untouched.
    term.process(b"\x1b_Xstuff\x1b\\");

    // No image stored.
    assert!(term.graphics_store.get_kitty_image(0).is_none());
    assert!(term.graphics_store.all_virtual_placements().is_empty());

    // First cell should still be empty/blank (default).
    let grid = term.active_grid();
    let cell = grid.get(0, 0).unwrap();
    assert_eq!(cell.c, ' ');
}
