//! The host POINTER's shape: the divider resize cue (config
//! `drag-cursor-shape`) written as OSC 22 — the pointer-shape escape
//! ghostty/kitty/xterm accept (CSS cursor names, best-effort: a
//! terminal without it keeps its own pointer; the DECSCUSR
//! steady-block during a live drag remains the fallback cue there).

use super::FlushSink;

/// A vertical divider column drags left/right.
pub(super) const RESIZE_COL: &str = "col-resize";
/// A horizontal divider row drags up/down.
pub(super) const RESIZE_ROW: &str = "row-resize";
/// The restore shape: the terminal's own hover-over-text pointer.
const RESTORE: &str = "text";

#[derive(Default)]
pub(super) struct PointerState {
    /// The shape the routing wants, `None` for the terminal's own.
    want: Option<&'static str>,
    /// The shape the last OSC 22 sent (with `None` meaning "text sent,
    /// nothing shaped"): the guard that keeps a quiet frame from
    /// re-emitting the escape.
    sent: Option<&'static str>,
}

impl PointerState {
    /// Record the shape the routing wants; [`Self::emit`] writes the
    /// change.
    pub(super) fn want(&mut self, shape: Option<&'static str>) {
        self.want = shape;
    }

    /// Write the OSC 22 for the wanted shape when it differs from the
    /// last one sent. Idempotent per frame: an unchanged shape writes
    /// nothing.
    pub(super) fn emit(&mut self, sink: &mut dyn FlushSink) {
        if self.want == self.sent {
            return;
        }
        let name = self.want.unwrap_or(RESTORE);
        let mut bytes = Vec::with_capacity(name.len() + 8);
        bytes.extend_from_slice(b"\x1b]22;");
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(b"\x1b\\");
        sink.write_raw(&bytes);
        self.sent = self.want;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use par_term_emu_core::cursor::CursorStyle;
    use ratatui::buffer::Cell as RtCell;

    #[derive(Default)]
    struct RawSink {
        raw: Vec<u8>,
    }

    impl FlushSink for RawSink {
        fn flush(&mut self, _diff: &[(u16, u16, RtCell)]) {}
        fn repaint_all(&mut self) {}
        fn place_cursor(&mut self, _cursor: Option<(u16, u16, CursorStyle)>) {}
        fn write_raw(&mut self, bytes: &[u8]) {
            self.raw.extend_from_slice(bytes);
        }
    }

    /// The emit guard: a change writes exactly one OSC 22, an
    /// unchanged want writes nothing, and `None` restores `text`.
    #[test]
    fn emit_writes_only_changes_and_restores_with_text() {
        let mut pointer = PointerState::default();
        let mut sink = RawSink::default();
        pointer.emit(&mut sink);
        assert!(sink.raw.is_empty(), "nothing wanted, nothing sent");

        pointer.want(Some(RESIZE_COL));
        pointer.emit(&mut sink);
        pointer.emit(&mut sink);
        assert_eq!(sink.raw, b"\x1b]22;col-resize\x1b\\");

        sink.raw.clear();
        pointer.want(None);
        pointer.emit(&mut sink);
        assert_eq!(sink.raw, b"\x1b]22;text\x1b\\");
    }
}
