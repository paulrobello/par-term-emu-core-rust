//! Inline-image emission and ingestion: the bridge between a Terminal's
//! graphics events and the size-bounded [`ServerMessage::InlineImage`]
//! frames.
//!
//! [`TerminalEvent::GraphicsAdded`] carries a graphic id, not pixels — the
//! announce stays cheap for every subscriber. The frame fetch happens once
//! per event, here: pixels move into one message and the broadcaster fans
//! it out; the per-subscriber subscription filter (`EventType::Graphics`)
//! never touches the payload. A frame whose payload exceeds
//! [`MAX_INLINE_IMAGE_BYTES`] is logged and skipped — the wire encoder
//! would reject it anyway, and blocking the event pump is never right.

use crate::streaming::protocol::{
    terminal_event_to_server_message, InlineImageFrame, ServerMessage, MAX_INLINE_IMAGE_BYTES,
};
use crate::terminal::{Terminal, TerminalEvent};
use parking_lot::RwLock;
use std::sync::Arc;

/// Build the [`ServerMessage::InlineImage`] frame for a stored graphic —
/// the event-pump form of [`inline_image_frame_struct`].
pub fn inline_image_frame(
    term: &Terminal,
    image_id: u64,
    pane_id: Option<u32>,
) -> Option<ServerMessage> {
    Some(ServerMessage::from(inline_image_frame_struct(
        term, image_id, pane_id,
    )?))
}

/// [`InlineImageFrame`] form of [`inline_image_frame`] — the shape
/// `Connected.graphics` replays to a late joiner. Returns `None` when the id
/// is unknown or the payload would not fit the wire cap (logged, never
/// truncated). Store pixels are RGBA by construction — sixel and kitty
/// decode to RGBA and the iTerm2 path stores its decoder output — so the
/// frame's format tag is always `"rgba"`.
pub fn inline_image_frame_struct(
    term: &Terminal,
    image_id: u64,
    pane_id: Option<u32>,
) -> Option<InlineImageFrame> {
    let graphic = term.all_graphics().iter().find(|g| g.id == image_id)?;
    if graphic.pixels.len() > MAX_INLINE_IMAGE_BYTES {
        crate::debug_error!(
            "STREAMING",
            "inline image {} ({}x{}) not streamed: {} payload bytes exceed the {} wire cap",
            image_id,
            graphic.width,
            graphic.height,
            graphic.pixels.len(),
            MAX_INLINE_IMAGE_BYTES
        );
        return None;
    }
    let width = u32::try_from(graphic.width).ok()?;
    let height = u32::try_from(graphic.height).ok()?;
    let (col, row) = graphic.position;
    let col = u32::try_from(col).unwrap_or(0);
    let row = u32::try_from(row).unwrap_or(0);
    Some(InlineImageFrame {
        image_id,
        pane_id,
        format: "rgba".to_string(),
        width,
        height,
        // One copy per event, never per subscriber: the broadcaster hands
        // the finished frame to each matched client by reference.
        data: graphic.pixels.as_ref().clone(),
        col,
        row,
    })
}

/// Convert one terminal event into the server messages it streams, with
/// Terminal access for the frame-bearing event. `pane_id` tags mux-mirror
/// frames; the direct-PTY streamer passes `None`.
pub fn event_messages(
    term: &Arc<RwLock<Terminal>>,
    event: TerminalEvent,
    pane_id: Option<u32>,
) -> Vec<ServerMessage> {
    match event {
        TerminalEvent::GraphicsAdded { image_id, row } => {
            let mut msgs = vec![ServerMessage::graphics_added(row as u16)];
            if let Some(frame) = inline_image_frame(&term.read(), image_id, pane_id) {
                msgs.push(frame);
            }
            msgs
        }
        other => terminal_event_to_server_message(other)
            .into_iter()
            .collect(),
    }
}

/// Ingest a received [`ServerMessage::InlineImage`] frame into a mirror
/// terminal's graphics machinery — the render effect a streaming client
/// applies so the frame shows in the pane's emulator. The frame's own id is
/// kept as the store id (after dropping any previous ingest of the same
/// id), so a re-sent frame replaces rather than stacks. Returns the store
/// id, or `None` when the frame is not an InlineImage or its format is not
/// `"rgba"`: the store holds decoded pixels only, so a compressed format
/// is the sender's to decode.
pub fn apply_inline_image(term: &mut Terminal, message: &ServerMessage) -> Option<u64> {
    let ServerMessage::InlineImage {
        image_id,
        format,
        width,
        height,
        col,
        row,
        data,
        ..
    } = message
    else {
        return None;
    };
    if format != "rgba" {
        crate::debug_error!(
            "STREAMING",
            "inline image {} not ingested: format '{}' is not rgba",
            image_id,
            format
        );
        return None;
    }
    let width = usize::try_from(*width).ok()?;
    let height = usize::try_from(*height).ok()?;
    let col = usize::try_from(*col).unwrap_or(0);
    let row = usize::try_from(*row).unwrap_or(0);
    term.graphics_store_mut().remove_graphic(*image_id);
    let mut graphic = crate::graphics::TerminalGraphic::new(
        *image_id,
        crate::graphics::GraphicProtocol::ITermInline,
        (col, row),
        width,
        height,
        data.clone(),
    );
    graphic.set_cell_dimensions(
        term.graphics.cell_dimensions.0,
        term.graphics.cell_dimensions.1,
    );
    term.graphics_store_mut().add_graphic(graphic);
    Some(*image_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{next_graphic_id, GraphicProtocol, TerminalGraphic};

    fn make_graphic(id: u64, width: usize, height: usize) -> TerminalGraphic {
        TerminalGraphic::new(
            id,
            GraphicProtocol::Sixel,
            (0, 0),
            width,
            height,
            vec![7u8; width * height * 4],
        )
    }

    /// criterion (a): the frame fetch resolves the event's id to the stored
    /// pixels without any subscriber fan-out cost — one message, payload
    /// intact, format tagged rgba.
    #[test]
    fn frame_fetch_resolves_the_event_id_to_stored_pixels() {
        let mut term = Terminal::new(80, 24);
        let id = next_graphic_id();
        let mut graphic = make_graphic(id, 4, 2);
        graphic.set_cell_dimensions(8, 16);
        graphic.position = (3, 4);
        term.graphics_store_mut().add_graphic(graphic);

        let frame = inline_image_frame(&term, id, None).expect("stored id fetches a frame");
        let ServerMessage::InlineImage {
            image_id,
            pane_id,
            format,
            width,
            height,
            col,
            row,
            data,
        } = frame
        else {
            panic!("expected an InlineImage frame");
        };
        assert_eq!(image_id, id);
        assert_eq!(pane_id, None);
        assert_eq!(format, "rgba");
        assert_eq!((width, height), (4, 2));
        assert_eq!((col, row), (3, 4), "the frame carries the stored placement");
        assert_eq!(data, vec![7u8; 4 * 2 * 4]);

        // An unknown id fetches nothing rather than fabricating a frame.
        assert!(inline_image_frame(&term, id + 999, None).is_none());
    }

    /// criterion (a): an oversize payload is dropped with a diagnostic, not
    /// sent and not truncated — the wire would reject it at encode anyway.
    #[test]
    fn oversize_payload_is_dropped_not_sent() {
        let mut term = Terminal::new(80, 24);
        let id = next_graphic_id();
        // A 1-pixel graphic whose pixel buffer claims to exceed the cap is
        // not constructible honestly, so build the smallest graphic that
        // trips the check directly against the frame builder's contract:
        // any payload over MAX_INLINE_IMAGE_BYTES must fetch None.
        let mut graphic = make_graphic(id, 1, 1);
        graphic.pixels = Arc::new(vec![0u8; MAX_INLINE_IMAGE_BYTES + 1]);
        term.graphics_store_mut().add_graphic(graphic);
        assert!(inline_image_frame(&term, id, Some(3)).is_none());
    }

    /// criterion (b): the event pump turns GraphicsAdded into the cheap
    /// announce plus the frame, and every other event still converts through
    /// the single-source-of-truth mapping.
    #[test]
    fn event_pump_emits_announce_plus_frame() {
        let mut term = Terminal::new(80, 24);
        term.process(b"\x1b_Ga=T,f=24,s=2,v=2;AAAAAAAAAAAAAAAA\x1b\\");
        let shared = Arc::new(RwLock::new(term));
        let events = shared.write().poll_events();
        let added = events
            .into_iter()
            .find(|e| matches!(e, TerminalEvent::GraphicsAdded { .. }))
            .expect("kitty a=T raises GraphicsAdded");

        let msgs = event_messages(&shared, added, Some(9));
        assert!(
            matches!(msgs[0], ServerMessage::GraphicsAdded { .. }),
            "the announce rides first"
        );
        assert_eq!(msgs.len(), 2, "the frame follows the announce");
        let ServerMessage::InlineImage { pane_id, .. } = &msgs[1] else {
            panic!("expected the InlineImage frame");
        };
        assert_eq!(*pane_id, Some(9));

        // A non-graphics event converts unchanged.
        let msgs = event_messages(&shared, TerminalEvent::TitleChanged("hi".to_string()), None);
        assert_eq!(msgs.len(), 1);
        assert!(matches!(msgs[0], ServerMessage::Title { .. }));
    }

    /// criterion 2: the render half — a fabricated InlineImage message fed
    /// into a pane emulator's graphics machinery lands as a placement the
    /// renderer reads back, with replace-tracking on the frame's id.
    #[test]
    fn fabricated_frame_ingests_into_the_pane_emulator() {
        let mut term = Terminal::new(80, 24);
        let message = ServerMessage::inline_image(
            42,
            Some(1),
            "rgba".to_string(),
            2,
            2,
            vec![9u8; 2 * 2 * 4],
            5,
            6,
        );
        let store_id = apply_inline_image(&mut term, &message).expect("rgba frame ingests");
        assert_eq!(store_id, 42, "the frame's own id becomes the store id");
        let stored = term
            .all_graphics()
            .iter()
            .find(|g| g.id == 42)
            .expect("the placement is in the store");
        assert_eq!((stored.width, stored.height), (2, 2));
        assert_eq!(stored.position, (5, 6), "the frame's cell places the image");
        assert_eq!(stored.pixels.as_ref(), &vec![9u8; 16]);

        // A re-sent frame with the same id replaces instead of stacking.
        let replacement = ServerMessage::inline_image(
            42,
            Some(1),
            "rgba".to_string(),
            3,
            1,
            vec![1u8; 3 * 4],
            0,
            2,
        );
        apply_inline_image(&mut term, &replacement).expect("re-ingest");
        assert_eq!(term.all_graphics().iter().filter(|g| g.id == 42).count(), 1);
        let moved = term
            .all_graphics()
            .iter()
            .find(|g| g.id == 42)
            .expect("the replacement is in the store");
        assert_eq!(
            moved.position,
            (0, 2),
            "a re-sent frame re-places the image"
        );

        // A non-rgba frame is refused, not half-applied.
        let png =
            ServerMessage::inline_image(43, None, "png".to_string(), 1, 1, vec![0u8; 4], 0, 0);
        assert!(apply_inline_image(&mut term, &png).is_none());
        assert!(!term.all_graphics().iter().any(|g| g.id == 43));

        // A non-InlineImage message is not an ingest request.
        assert!(apply_inline_image(&mut term, &ServerMessage::bell()).is_none());
    }
}
