//! Popup command windows over the attach TUI (herdr's `type = "popup"`
//! custom commands, Apache-2.0, herdr `src/app/popup.rs`): prefix + a
//! configured key runs a command in a floating, bordered box over the
//! panes; the tiled layout underneath never changes.
//!
//! The command runs on the DAEMON: `new-window` creates a scratch window
//! next to the shown one (par-mux's `new-window` never selects it), and a
//! `control-pane` stream on a second connection mirrors its one pane into
//! a client-side emulator painted in the box. The stream's seed and its
//! narrowing are atomic daemon-side, so no output is lost or doubled
//! between them, and the stream's size report makes the scratch window
//! exactly the box's inner grid. The popup closes when its command exits
//! (the stream ends) or on prefix + the popup's key again; closing kills
//! the scratch window.

use super::paint::{map_color, map_flags};
use super::{PaneEmulator, PaneRenderer, WindowSession};
use crate::mux::attach::conn::AttachConn;
use crate::mux::pane_stream::StreamEvent;
use crate::mux::popup::{resolve_geometry, CellRect, Popup};
use par_term_emu_core::color::{Color as CoreColor, NamedColor};
use ratatui::buffer::CellDiffOption;
use ratatui::style::Style as RtStyle;
use std::sync::mpsc::{Receiver, TryRecvError};

/// The scratch window's name (other clients see it in their tab lists
/// while the popup is up).
const POPUP_WINDOW_NAME: &str = "popup";

/// The size-report half of a popup's pane stream.
pub(super) trait PopupControl {
    /// Report the box's inner grid as the scratch window's size.
    fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()>;
}

impl PopupControl for crate::mux::pane_stream::StreamWriter {
    fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        crate::mux::pane_stream::StreamWriter::resize(self, cols, rows)
    }
}

/// An open popup pane stream: its events in wire order and its size
/// reports.
pub(super) struct PopupLink {
    pub(super) events: Receiver<StreamEvent>,
    pub(super) control: Box<dyn PopupControl>,
}

/// Opens the `control-pane` stream on `pane` (`%N`) at the daemon's
/// socket; tests swap it for a channel-backed fake.
pub(super) type PopupOpener = fn(Option<&std::path::Path>, &str) -> std::io::Result<PopupLink>;

/// The real opener: a second connection, its reader on a thread feeding
/// the event channel (the thread ends with the stream).
pub(super) fn open_stream(
    socket: Option<&std::path::Path>,
    pane: &str,
) -> std::io::Result<PopupLink> {
    let socket = socket.ok_or_else(|| std::io::Error::other("no daemon socket known"))?;
    let (mut reader, writer) = crate::mux::pane_stream::open(socket, pane, true)?;
    let mut writer = writer.ok_or_else(|| std::io::Error::other("no control writer"))?;
    writer.set_pane(pane);
    let (tx, events) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(Some(event)) = reader.next_event() {
            if tx.send(event).is_err() {
                return;
            }
        }
    });
    Ok(PopupLink {
        events,
        control: Box::new(writer),
    })
}

/// The popup on screen (or opening).
pub(super) struct OpenPopup {
    /// The popup's config entry (its key closes it again).
    pub(super) popup: Popup,
    /// The scratch window, once `new-window` answered.
    pub(super) window: Option<String>,
    /// The scratch window's pane, once `list-panes` answered.
    pub(super) pane: Option<u32>,
    /// The pane stream, once opened.
    pub(super) link: Option<PopupLink>,
}

/// What the renderer paints for a popup: the box, its title, and the
/// mirrored pane.
pub(in crate::mux::attach) struct PopupView {
    /// The outer box in frame cells.
    pub(super) outer: CellRect,
    pub(super) title: String,
    /// `None` until the stream's seed arrives (the box paints empty).
    pub(super) emulator: Option<PaneEmulator>,
}

impl PopupView {
    /// The terminal grid inside the border.
    pub(super) fn inner(&self) -> CellRect {
        CellRect {
            x: self.outer.x + 1,
            y: self.outer.y + 1,
            width: self.outer.width.saturating_sub(2),
            height: self.outer.height.saturating_sub(2),
        }
    }
}

impl PaneRenderer {
    /// The pane content area in frame cells: right of the side panel,
    /// the frame's full height.
    pub(super) fn popup_area(&self) -> CellRect {
        let x = self.geometry.sidebar_w.min(self.width);
        CellRect {
            x,
            y: 0,
            width: self.width - x,
            height: self.height,
        }
    }

    /// Show, re-place, or clear the popup box.
    pub(super) fn set_popup(&mut self, popup: Option<PopupView>) {
        self.popup = popup;
        self.dirty = true;
    }

    /// The popup box, if one is up.
    pub(super) fn popup_mut(&mut self) -> Option<&mut PopupView> {
        self.dirty = true;
        self.popup.as_mut()
    }

    /// The popup's terminal (input modes for key re-encoding).
    pub(super) fn popup_terminal(&self) -> Option<&par_term_emu_core::terminal::Terminal> {
        self.popup
            .as_ref()
            .and_then(|p| p.emulator.as_ref())
            .map(PaneEmulator::terminal)
    }

    /// Where the host cursor goes while the popup is up: its tracked
    /// cell inside the box (frame cells), `None` when the popup hid it.
    pub(super) fn popup_cursor(
        &self,
    ) -> Option<(u16, u16, par_term_emu_core::cursor::CursorStyle)> {
        let popup = self.popup.as_ref()?;
        let cursor = popup.emulator.as_ref()?.terminal().cursor();
        let inner = popup.inner();
        if !cursor.visible
            || cursor.col >= usize::from(inner.width)
            || cursor.row >= usize::from(inner.height)
        {
            return None;
        }
        Some((
            inner.x + cursor.col as u16,
            inner.y + cursor.row as u16,
            cursor.style,
        ))
    }

    /// Paint the popup box over the frame: a cleared, accent-bordered
    /// box with the title in its top edge and the mirrored grid inside.
    pub(super) fn paint_popup(&mut self) {
        let Some(popup) = self.popup.as_ref() else {
            return;
        };
        let outer = popup.outer;
        let inner = popup.inner();
        let title = popup.title.clone();
        let (width, height) = (self.width, self.height);
        let accent = RtStyle::default().fg(self.accent());
        let glyphs = self.glyphs;
        let bg = self.bg;
        for y in outer.y..(outer.y + outer.height).min(height) {
            for x in outer.x..(outer.x + outer.width).min(width) {
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                if let Some(bg) = bg {
                    cell.set_bg(bg);
                }
                let (left, right) = (x == outer.x, x + 1 == outer.x + outer.width);
                let (top, bottom) = (y == outer.y, y + 1 == outer.y + outer.height);
                let symbol = match (left, right, top, bottom) {
                    (true, _, true, _) => glyphs.corner_top_left(),
                    (_, true, true, _) => glyphs.corner_top_right(),
                    (true, _, _, true) => glyphs.corner_bottom_left(),
                    (_, true, _, true) => glyphs.corner_bottom_right(),
                    (true, ..) | (_, true, ..) => glyphs.vertical(),
                    (_, _, true, _) | (_, _, _, true) => glyphs.horizontal(),
                    _ => continue,
                };
                cell.set_symbol(symbol);
                cell.set_style(accent);
            }
        }
        let label = format!(" {} ", crate::mux::attach::chrome_safe(&title));
        let room = usize::from(outer.width.saturating_sub(4));
        for (i, ch) in label.chars().take(room).enumerate() {
            let x = outer.x + 2 + i as u16;
            if x < width && outer.y < height {
                let cell = &mut self.buffer[(x, outer.y)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(accent);
            }
        }
        let Some(emulator) = self.popup.as_ref().and_then(|p| p.emulator.as_ref()) else {
            return;
        };
        let grid = emulator.terminal().active_grid();
        let rows = inner.height.min(grid.rows() as u16);
        let cols = inner.width.min(grid.cols() as u16);
        for row in 0..rows {
            for col in 0..cols {
                let (x, y) = (inner.x + col, inner.y + row);
                if x >= width || y >= height {
                    continue;
                }
                let Some(core) = grid.get(col as usize, row as usize) else {
                    continue;
                };
                if core.flags().wide_char_spacer() {
                    continue;
                }
                let cell = &mut self.buffer[(x, y)];
                let mut symbol = String::from(core.c());
                for comb in core.combining() {
                    symbol.push(*comb);
                }
                cell.set_symbol(&symbol);
                cell.set_fg(map_color(core.fg()));
                if !matches!(core.bg(), CoreColor::Named(NamedColor::Black)) {
                    cell.set_bg(map_color(core.bg()));
                }
                cell.set_style(map_flags(core.flags()));
                if core.width() == 2 && col + 1 < cols && x + 1 < width {
                    self.buffer[(x + 1, y)].set_diff_option(CellDiffOption::Skip);
                }
            }
        }
    }
}

impl WindowSession {
    /// The popup bound to the after-prefix `byte`, if any.
    pub(super) fn popup_for_key(&self, byte: u8) -> Option<&Popup> {
        self.popups.iter().find(|p| p.key == byte)
    }

    /// Whether a popup owns the input (open, or still opening).
    pub(super) fn popup_active(&self) -> bool {
        self.popup.is_some()
    }

    /// prefix + a popup's key: open it — or, when that popup is the one
    /// up, close it. The box is herdr's geometry over the pane area; an
    /// area under the 6x4 minimum refuses on the flash.
    pub(super) fn open_popup(&mut self, conn: &mut AttachConn, byte: u8) {
        if self
            .popup
            .as_ref()
            .is_some_and(|open| open.popup.key == byte)
        {
            self.close_popup(conn);
            return;
        }
        let Some(popup) = self.popup_for_key(byte).cloned() else {
            return;
        };
        if self.popup.is_some() {
            return;
        }
        let Some(geometry) = resolve_geometry(
            popup.entry.width(),
            popup.entry.height(),
            self.renderer.popup_area(),
        ) else {
            self.chrome
                .flash("popup: the pane area is too small".to_string());
            self.draw_status_row();
            return;
        };
        let start = match popup_start_command(&popup.entry.command) {
            Ok(start) => start,
            Err(err) => {
                self.chrome.flash(format!("popup failed: {err}"));
                self.draw_status_row();
                return;
            }
        };
        if matches!(self.modal, super::Modal::Scroll) {
            self.leave_scroll_mode();
        }
        self.renderer.set_popup(Some(PopupView {
            outer: geometry.outer,
            title: popup.title().to_string(),
            emulator: None,
        }));
        self.popup = Some(OpenPopup {
            popup,
            window: None,
            pane: None,
            link: None,
        });
        let command = format!(
            "new-window -t {} -n {POPUP_WINDOW_NAME} -- {start}",
            self.window
        );
        self.send_then(conn, &command, |s, c, reply| {
            let window = reply.body.first().map(|w| w.trim().to_string());
            match window.filter(|w| reply.ok && w.starts_with('@')) {
                Some(window) => {
                    if let Some(open) = s.popup.as_mut() {
                        open.window = Some(window.clone());
                    } else {
                        // Closed while opening: the window is unwanted.
                        let _ = c.send_forget(&format!("kill-window -t {window}"));
                        return;
                    }
                    s.draw_tab_strip();
                    s.popup_find_pane(c, &window);
                }
                None => {
                    let why = reply.body.first().cloned().unwrap_or_default();
                    s.drop_popup(c, &format!("popup failed: {why}"));
                }
            }
        });
    }

    /// Resolve the scratch window's pane, then open its stream.
    fn popup_find_pane(&mut self, conn: &mut AttachConn, window: &str) {
        let expected = window.to_string();
        self.send_then(
            conn,
            &format!("list-panes -t {window}"),
            move |s, c, reply| {
                if s.popup.as_ref().and_then(|o| o.window.as_deref()) != Some(expected.as_str()) {
                    return;
                }
                let pane = reply
                    .body
                    .first()
                    .and_then(|line| line.split_whitespace().next())
                    .and_then(|id| id.strip_prefix('%'))
                    .and_then(|n| n.parse::<u32>().ok());
                let Some(pane) = pane.filter(|_| reply.ok) else {
                    s.drop_popup(c, "popup: the command exited");
                    return;
                };
                match (s.popup_opener)(s.socket.as_deref(), &format!("%{pane}")) {
                    Ok(link) => {
                        if let Some(open) = s.popup.as_mut() {
                            open.pane = Some(pane);
                            open.link = Some(link);
                        }
                    }
                    Err(err) => s.drop_popup(c, &format!("popup failed: {err}")),
                }
            },
        );
    }

    /// Pump step 1c: apply the popup stream's events in order — the seed
    /// builds the mirror and sizes the scratch window to the box, output
    /// feeds it, and the command's exit (or the stream ending) closes the
    /// popup.
    pub(super) fn drain_popup(&mut self, conn: &mut AttachConn) {
        loop {
            let event = match self
                .popup
                .as_ref()
                .and_then(|o| o.link.as_ref())
                .map(|link| link.events.try_recv())
            {
                None | Some(Err(TryRecvError::Empty)) => return,
                Some(Err(TryRecvError::Disconnected)) => {
                    self.close_popup(conn);
                    return;
                }
                Some(Ok(event)) => event,
            };
            match event {
                StreamEvent::Seed {
                    size, exited, seed, ..
                } => {
                    if exited.is_some() {
                        self.close_popup(conn);
                        return;
                    }
                    let (cols, rows) = (size.0.max(1), size.1.max(1));
                    let mut emulator = PaneEmulator::new(cols, rows);
                    emulator.feed(&seed);
                    if let Some(view) = self.renderer.popup_mut() {
                        view.emulator = Some(emulator);
                    }
                    self.fit_popup(conn);
                }
                StreamEvent::Output(bytes) => {
                    if let Some(emulator) =
                        self.renderer.popup_mut().and_then(|v| v.emulator.as_mut())
                    {
                        emulator.feed(&bytes);
                    }
                }
                StreamEvent::Exited(_) | StreamEvent::Closed => {
                    self.close_popup(conn);
                    return;
                }
                StreamEvent::Respawned | StreamEvent::Reply { .. } => {}
            }
        }
    }

    /// Re-place the box over the current pane area (a host resize, a
    /// side-panel toggle), re-fit the mirror, and report the inner grid
    /// as the scratch window's size. An area under the minimum closes
    /// the popup.
    pub(super) fn fit_popup(&mut self, conn: &mut AttachConn) {
        let Some(open) = self.popup.as_ref() else {
            return;
        };
        let (width, height) = (open.popup.entry.width(), open.popup.entry.height());
        let Some(geometry) = resolve_geometry(width, height, self.renderer.popup_area()) else {
            self.close_popup(conn);
            return;
        };
        let inner = geometry.inner;
        if let Some(view) = self.renderer.popup_mut() {
            view.outer = geometry.outer;
            if let Some(emulator) = view.emulator.as_mut() {
                emulator.resize(inner.width, inner.height);
            }
        }
        if let Some(link) = self.popup.as_mut().and_then(|o| o.link.as_mut()) {
            let _ = link.control.resize(inner.width, inner.height);
        }
    }

    /// Close the popup: kill its scratch window (the stream ends with
    /// it), clear the box, and hand input back to the panes.
    pub(super) fn close_popup(&mut self, conn: &mut AttachConn) {
        let Some(open) = self.popup.take() else {
            return;
        };
        if let Some(window) = &open.window {
            let _ = conn.send_forget(&format!("kill-window -t {window}"));
        }
        self.renderer.set_popup(None);
        self.arm_prefix(false);
        self.status_dirty = true;
        self.draw_tab_strip();
    }

    /// [`Self::close_popup`] with a reason on the flash.
    fn drop_popup(&mut self, conn: &mut AttachConn, why: &str) {
        self.close_popup(conn);
        self.chrome.flash(why.to_string());
        self.draw_status_row();
    }

    /// The session is ending (detach, `%exit`, EOF): kill a popup's
    /// scratch window, waiting briefly for the daemon to take it.
    pub(super) fn popup_teardown(&mut self, conn: &mut AttachConn) {
        if let Some(window) = self.popup.take().and_then(|o| o.window) {
            let _ = conn.send_checked_timeout(
                &format!("kill-window -t {window}"),
                std::time::Duration::from_millis(500),
            );
        }
    }

    /// The scratch window to leave out of this client's tab strip.
    pub(super) fn popup_window(&self) -> Option<&str> {
        self.popup.as_ref().and_then(|o| o.window.as_deref())
    }

    /// A plain run while a popup owns the input: prefix + the popup's key
    /// closes it, prefix prefix sends the literal prefix, prefix `d`
    /// closes it and detaches, and every other chord is consumed (the
    /// panes and the window switches stay out of reach). Everything else
    /// goes to the popup's pane. Returns true on detach.
    pub(super) fn popup_route_plain(&mut self, conn: &mut AttachConn, bytes: &[u8]) -> bool {
        let mut to_send = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if self.prefix_pending {
                self.arm_prefix(false);
                let key = self.popup.as_ref().map(|o| o.popup.key);
                if Some(byte) == key {
                    self.popup_send(conn, &to_send);
                    self.close_popup(conn);
                    return false;
                }
                if byte == b'd' {
                    self.close_popup(conn);
                    return true;
                }
                if byte == self.literal {
                    to_send.push(byte);
                }
            } else if byte == self.prefix {
                self.arm_prefix(true);
            } else {
                to_send.push(byte);
            }
        }
        self.popup_send(conn, &to_send);
        false
    }

    /// A functional key while a popup owns the input: re-encoded against
    /// the popup's own input modes (DECCKM arrows in a TUI popup). A
    /// pending prefix consumes it.
    pub(super) fn popup_route_key(
        &mut self,
        conn: &mut AttachConn,
        ev: &par_term_emu_core::keyboard::TermKeyEvent,
    ) {
        if self.prefix_pending {
            self.arm_prefix(false);
            return;
        }
        let bytes = self
            .renderer
            .popup_terminal()
            .map(|term| par_term_emu_core::keyboard::encode_key(ev, term))
            .unwrap_or_default();
        self.popup_send(conn, &bytes);
    }

    /// A bracketed paste while a popup owns the input: framed by the
    /// popup's own DECSET 2004 state.
    pub(super) fn popup_route_paste(&mut self, conn: &mut AttachConn, body: &[u8]) {
        self.arm_prefix(false);
        let body = crate::mux::attach::input::strip_paste_end(body);
        let mut bytes = Vec::with_capacity(body.len() + 12);
        match self.renderer.popup_terminal() {
            Some(term) => {
                bytes.extend_from_slice(term.bracketed_paste_start());
                bytes.extend_from_slice(&body);
                bytes.extend_from_slice(term.bracketed_paste_end());
            }
            None => bytes.extend_from_slice(&body),
        }
        self.popup_send(conn, &bytes);
    }

    /// Forward input to the popup's pane (dropped until the pane is
    /// known — the box is still opening).
    fn popup_send(&mut self, conn: &mut AttachConn, bytes: &[u8]) {
        let Some(pane) = self.popup.as_ref().and_then(|o| o.pane) else {
            return;
        };
        if !bytes.is_empty() {
            self.forward_keys(conn, format!("%{pane}"), bytes);
        }
    }
}

/// The start command `new-window` runs, as written: since the parser's
/// start-command cut (f567569f) stops the flag scan at the `--`, an
/// inline `lazygit -c x` is no longer misread and no launcher script is
/// needed.
fn popup_start_command(command: &str) -> std::io::Result<String> {
    Ok(command.to_string())
}

/// The settings dialog's popup row label, and the add row's.
const POPUP_ROW_LABEL: &str = "popup";
const ADD_POPUP_LABEL: &str = "+ add popup";

impl super::modal_settings::SettingsState {
    /// The general section's popup rows: one per popup plus the add row
    /// (0 elsewhere).
    pub(super) fn popup_rows(&self) -> usize {
        if self.section == super::modal_settings::SettingsSection::General {
            self.popups.len() + 1
        } else {
            0
        }
    }

    /// The popup rows' texts, laid out like the fixed rows (`first` is
    /// the first popup row's body index, for the selection pointer).
    pub(super) fn popup_row_texts(
        &self,
        first: usize,
        label_w: usize,
        panel_w: usize,
    ) -> Vec<String> {
        let value_w = panel_w - 4 - label_w;
        let pointer = |at: usize| {
            if self.selected == first + at {
                "\u{25B8}"
            } else {
                " "
            }
        };
        let mut rows: Vec<String> = self
            .popups
            .iter()
            .enumerate()
            .map(|(at, entry)| {
                let spelled = crate::mux::popup::spell_entry(entry);
                let len = spelled.chars().count();
                let value = if len > value_w {
                    let tail: String = spelled.chars().skip(len - (value_w - 1)).collect();
                    format!("\u{2026}{tail}")
                } else {
                    spelled
                };
                format!(
                    " {} {POPUP_ROW_LABEL:<label_w$} {value:>value_w$}",
                    pointer(at)
                )
            })
            .collect();
        rows.push(format!(
            " {} {ADD_POPUP_LABEL:<label_w$} {:>value_w$}",
            pointer(self.popups.len()),
            "KEY [WxH] COMMAND"
        ));
        rows
    }
}

impl WindowSession {
    /// The popups as the config file spells them (the dialog's rows).
    pub(super) fn popup_entries(&self) -> Vec<crate::mux::popup::PopupEntry> {
        self.popups.iter().map(|p| p.entry.clone()).collect()
    }

    /// Enter on a popup row (`index` past the end = the add row): the
    /// prompt opens seeded with the entry's `KEY [WxH] COMMAND` spelling.
    pub(super) fn edit_popup_row(
        &mut self,
        index: usize,
        back: super::modal_settings::SettingsState,
    ) {
        let (seed, slot, label) = match back.popups.get(index) {
            Some(entry) => (
                crate::mux::popup::spell_entry(entry),
                index,
                POPUP_ROW_LABEL,
            ),
            None => (String::new(), usize::MAX, ADD_POPUP_LABEL),
        };
        let spec = super::modal_settings::SettingSpec {
            key: "popup",
            label,
            kind: super::modal_settings::SettingKind::Popup(slot),
        };
        self.enter_setting_prompt(&spec, spec.kind, seed, back);
    }

    /// Commit a popup row's prompt: an empty text deletes the popup (on
    /// the add row it cancels), anything else must parse as `KEY [WxH]
    /// COMMAND` with a key no other popup holds (a bad one flashes and
    /// the prompt stays up). The whole `[[client.popup]]` array is
    /// rewritten, then the reload applies it. Returns whether the prompt
    /// closed.
    pub(super) fn popup_setting_commit(
        &mut self,
        conn: &mut AttachConn,
        edit: &super::modal_settings::SettingEdit,
        index: usize,
        text: &str,
    ) -> bool {
        let mut entries = edit.back.popups.clone();
        if text.is_empty() {
            if index < entries.len() {
                entries.remove(index);
            }
        } else {
            let entry = match crate::mux::popup::parse_spelling(text, entries.get(index)) {
                Ok(entry) => entry,
                Err(err) => {
                    self.chrome.flash(format!("popup: {err}"));
                    self.draw_status_row();
                    return false;
                }
            };
            if index < entries.len() {
                entries[index] = entry;
            } else {
                entries.push(entry);
            }
            if let Err(err) = crate::mux::popup::validate(&entries) {
                self.chrome.flash(err);
                self.draw_status_row();
                return false;
            }
        }
        let tables = entries.iter().map(popup_table).collect();
        let mut back = edit.back.clone();
        if self.settings_write_and_reload(
            conn,
            "popup",
            crate::mux::attach::settings_write::SettingValue::Tables(tables),
        ) {
            back.popups = self.popup_entries();
            back.selected = back
                .selected
                .min(back.section.row_count() + back.popup_rows() - 1);
        }
        self.open_modal(super::Modal::Settings(back));
        self.refresh_settings();
        true
    }
}

/// One entry as `[[client.popup]]` table fields.
fn popup_table(
    entry: &crate::mux::popup::PopupEntry,
) -> Vec<(String, crate::mux::attach::settings_write::TableField)> {
    use crate::mux::attach::settings_write::TableField;
    let size = |s: crate::mux::popup::PopupSize| match s {
        crate::mux::popup::PopupSize::Cells(n) => TableField::Int(i64::from(n)),
        crate::mux::popup::PopupSize::Percent(_) => TableField::Str(s.spell()),
    };
    let mut fields = vec![
        ("key".to_string(), TableField::Str(entry.key.clone())),
        (
            "command".to_string(),
            TableField::Str(entry.command.clone()),
        ),
    ];
    if let Some(w) = entry.width() {
        fields.push(("width".to_string(), size(w)));
    }
    if let Some(h) = entry.height() {
        fields.push(("height".to_string(), size(h)));
    }
    if let Some(d) = &entry.description {
        fields.push(("description".to_string(), TableField::Str(d.clone())));
    }
    fields
}
