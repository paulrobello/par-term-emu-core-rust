//! The session and workspace pickers: entering from daemon state,
//! the filter/selection model, activation through the select+resync
//! contract, and their keys and bytes.

use super::{Modal, PickerState, WindowSession};
use crate::mux::attach::panels::{
    compose_picker_panel, palette_action_chord, picker_rows, HelpRow, PickerEntry, PickerRef,
    PALETTE_ACTIONS, PALETTE_FOOTER, PICKER_FOOTER, PICKER_OVERLAY_TITLE,
    WORKSPACE_PICKER_OVERLAY_TITLE,
};
use crate::mux::attach::targets::{parse_session_line, parse_workspace_line};
use par_term_emu_core::keyboard::TermKeyEvent;

impl WindowSession {
    /// Open the window picker (the `picker` chord): every session\'s
    /// windows as one flat row list, composed into the same themed
    /// modal the help panel uses (title `picker`, `esc close` badge,
    /// footer controls line), the shown window `>`-marked. A session is
    /// not a row (landing on one meant its active window); a window
    /// name two sessions share is prefixed with its session\'s name.
    /// The filter is live from the open: typing filters immediately (no
    /// `/` first); arrows move the selection without leaving the
    /// filter; Enter selects through [`Self::picker_activate`]; esc
    /// dismisses and the prior frame repaints. Keys and the mouse never
    /// reach the pane while it is up.
    pub(super) fn enter_picker(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        self.send_then(conn, "list-sessions", |s, c, reply| {
            if !reply.ok {
                return;
            }
            let sessions: Vec<(String, String)> = reply
                .body
                .iter()
                .filter_map(|line| parse_session_line(line))
                .collect();
            if sessions.is_empty() {
                return;
            }
            // Every session's window list is written now and answered in
            // order; the picker opens when the last one lands.
            let slots: std::rc::Rc<std::cell::RefCell<Vec<Option<PickerEntry>>>> = std::rc::Rc::new(
                std::cell::RefCell::new((0..sessions.len()).map(|_| None).collect()),
            );
            for (index, (sid, name)) in sessions.into_iter().enumerate() {
                let slots = std::rc::Rc::clone(&slots);
                let command = format!("list-windows -t {sid}");
                s.send_then(c, &command, move |s, _c, listed| {
                    let mut windows: Vec<(String, String)> = Vec::new();
                    let mut active_window: Option<String> = None;
                    if listed.ok {
                        for wl in &listed.body {
                            let mut fields = wl.split_whitespace();
                            let Some(id) = fields.next().filter(|id| id.starts_with('@')) else {
                                continue;
                            };
                            let marker = fields.next().unwrap_or("-");
                            let rest = fields.collect::<Vec<_>>().join(" ");
                            let wname = if rest.is_empty() { id } else { &rest };
                            if marker == "*" {
                                active_window = Some(id.to_string());
                            }
                            windows.push((id.to_string(), wname.to_string()));
                        }
                    }
                    let current = s.status.session_id.as_deref() == Some(sid.as_str());
                    let mut slots = slots.borrow_mut();
                    slots[index] = Some(PickerEntry {
                        session_id: sid,
                        session_name: name,
                        windows,
                        active_window,
                        current,
                    });
                    if slots.iter().any(Option::is_none) {
                        return;
                    }
                    let entries: Vec<PickerEntry> = slots.drain(..).flatten().collect();
                    drop(slots);
                    s.open_session_picker(entries);
                });
            }
        });
    }
    fn open_session_picker(&mut self, entries: Vec<PickerEntry>) {
        if entries.is_empty() {
            return;
        }
        // The selection opens on the shown window's row — else the
        // current session's first window — else the first row (the
        // view's window is unknown to the roster).
        let (_rows, refs) = picker_rows(&entries, Some(&self.window));
        let selected = refs
            .iter()
            .position(|r| {
                matches!(r, PickerRef::Window(i, w)
                    if entries[*i].windows.get(*w).is_some_and(|(id, _)| id == &self.window))
            })
            .or_else(|| {
                refs.iter()
                    .position(|r| matches!(r, PickerRef::Window(i, _) if entries[*i].current))
            })
            .unwrap_or(0);
        self.open_modal(Modal::Picker(PickerState {
            entries,
            selected,
            filtering: true,
            ..PickerState::default()
        }));
        self.refresh_picker();
    }

    /// The palette overlay's title.
    pub(super) const PALETTE_TITLE: &str = " command ";

    /// Open the command palette (the `palette` chord): every management
    /// action plus settings/keybinds/reload/detach as a filterable picker
    /// list — the keyboard path to the click-only menu chips and every
    /// chord, one list (the TUI audit's R2). The filter is live from the
    /// open: typing filters immediately (no `/` first), arrows/j/k move,
    /// Enter runs the selected action, esc dismisses. Each row carries
    /// its action's EFFECTIVE chord, spelled like the keybinds panel.
    pub(super) fn enter_palette(&mut self) {
        let commands: Vec<(usize, String)> = PALETTE_ACTIONS
            .iter()
            .enumerate()
            .map(|(index, (_, label))| (index, label.to_string()))
            .collect();
        self.open_modal(Modal::Picker(PickerState {
            commands: Some(commands),
            filtering: true,
            ..PickerState::default()
        }));
        self.refresh_picker();
    }

    /// Compose the palette's overlay: one row per action — the label in
    /// a left column, its EFFECTIVE chord (the live config's binding,
    /// the help panel's spelling) in a second column — through the
    /// picker's filter, cursor, and windowing. The filter matches the
    /// whole row, so a chord fragment finds its action too.
    fn refresh_palette(&mut self, commands: &[(usize, String)]) {
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        let width = commands
            .iter()
            .map(|(_, label)| label.chars().count())
            .max()
            .unwrap_or(0);
        let rows: Vec<HelpRow> = commands
            .iter()
            .map(|(i, label)| {
                let chord =
                    palette_action_chord(*i, self.prefix, self.reload_key, &self.management);
                let text = match chord {
                    Some(chord) => format!(" {label:<width$}  {chord} "),
                    None => format!(" {label:<width$} "),
                };
                HelpRow {
                    text,
                    accent: false,
                    footer: false,
                    render_only: false,
                }
            })
            .collect();
        let refs: Vec<PickerRef> = commands
            .iter()
            .map(|(i, _)| PickerRef::Command(*i))
            .collect();
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = compose_picker_panel(
            &rows,
            &refs,
            &picker.filter,
            picker.filtering,
            picker.selected,
            visible,
            picker.start,
            PALETTE_FOOTER,
        );
        self.store_picker_compose(filtered_refs, start, lines.len());
        self.renderer
            .set_overlay(Some((Self::PALETTE_TITLE, lines, None)));
    }

    /// Dispatch one palette action: the table's first four rows are the
    /// non-chord paths (fixed order), the last four the tab moves
    /// (left/right as chords, start/end on the SHOWN window through
    /// `move-window -s {shown} -t {slot}`), the rest through the
    /// management chord table.
    fn run_palette_action(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        action: usize,
    ) {
        match action {
            0 => self.enter_settings(),
            1 => self.enter_help(),
            2 => self.reload_config(conn),
            3 => self.detach_requested = true,
            19 => self.move_palette_window(conn, super::modal_menu::TabMove::Left),
            20 => self.move_palette_window(conn, super::modal_menu::TabMove::Right),
            21 => self.move_palette_window(conn, super::modal_menu::TabMove::Start),
            22 => self.move_palette_window(conn, super::modal_menu::TabMove::End),
            _ => {
                if let Some((Some(key), _)) = PALETTE_ACTIONS.get(action) {
                    self.management_chord(*key, conn);
                }
            }
        }
    }
    /// The palette's `move tab to start`/`move tab to end`: the shown
    /// window through the tab menu's move helper.
    fn move_palette_window(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        to: super::modal_menu::TabMove,
    ) {
        let shown = self.window.clone();
        self.move_window_slot(conn, &shown, to);
    }
    /// Open the workspace picker (the workspace-picker chord): the
    /// daemon's workspace roster — id, name, the active one `>`-marked —
    /// in the same themed modal, filter (live from the open), cursor,
    /// and click handling the window picker runs. Enter lands on
    /// the workspace through the same select-then-resync every switch
    /// follows.
    pub(super) fn enter_ws_picker(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        self.send_then(conn, "list-workspaces", |s, _c, reply| {
            if !reply.ok {
                return;
            }
            let workspaces: Vec<(String, String, bool)> = reply
                .body
                .iter()
                .filter_map(|l| parse_workspace_line(l))
                .collect();
            if workspaces.is_empty() {
                return;
            }
            let open_on = workspaces.iter().position(|(_, _, active)| *active);
            s.open_modal(Modal::Picker(PickerState {
                selected: open_on.unwrap_or(0),
                workspaces: Some(workspaces),
                filtering: true,
                ..PickerState::default()
            }));
            s.refresh_ws_picker();
        });
    }
    /// Dismiss the picker: the next frame\'s pane repaint restores the
    /// covered cells.
    pub(super) fn leave_picker(&mut self) {
        if matches!(self.modal, Modal::Picker(_)) {
            self.modal = Modal::None;
        }
        self.join_source = None;
        self.renderer.set_overlay(None);
    }
    /// Re-compose the picker overlay from the live state (filter,
    /// selection, pan). The workspace picker composes its own rows and
    /// rides the same filter/window/footer machinery.
    pub(super) fn refresh_picker(&mut self) {
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        if let Some(commands) = picker.commands.clone() {
            self.refresh_palette(&commands);
            return;
        }
        if picker.workspaces.is_some() {
            self.refresh_ws_picker();
            return;
        }
        let (rows, refs) = picker_rows(&picker.entries, Some(&self.window));
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = compose_picker_panel(
            &rows,
            &refs,
            &picker.filter,
            picker.filtering,
            picker.selected,
            visible,
            picker.start,
            PICKER_FOOTER,
        );
        self.store_picker_compose(filtered_refs, start, lines.len());
        self.renderer
            .set_overlay(Some((PICKER_OVERLAY_TITLE, lines, None)));
    }
    /// The picker's visible content-row count (the same window the
    /// compose uses; the filter line joins the chrome only while a
    /// filter is open or set).
    pub(super) fn picker_visible(&self) -> usize {
        let filtering = match &self.modal {
            Modal::Picker(picker) => picker.filtering || !picker.filter.is_empty(),
            _ => false,
        };
        let filter_lines = usize::from(filtering);
        usize::from(
            self.renderer
                .window_size()
                .1
                .saturating_sub(3 + filter_lines as u16),
        )
        .max(1)
    }
    /// Move the picker\'s selection cursor by `delta` content rows (the
    /// cursor wraps at the ends of the filtered list) and re-compose.
    pub(super) fn picker_move(&mut self, delta: isize) {
        let Modal::Picker(picker) = &mut self.modal else {
            return;
        };
        let count = picker.refs.len();
        if count == 0 {
            return;
        }
        picker.selected = (picker.selected as isize + delta).rem_euclid(count as isize) as usize;
        self.refresh_picker();
    }
    /// Activate the selected picker row: a window row lands on that
    /// window — daemon-side `select-window`, then the select+resync
    /// re-seed every switch follows.
    pub(super) fn picker_activate(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        let idx = picker.selected.min(picker.refs.len().saturating_sub(1));
        let Some(item) = picker.refs.get(idx).copied() else {
            return;
        };
        // Palette mode: the row's action runs instead of a landing.
        if let PickerRef::Command(action) = item {
            self.leave_picker();
            self.run_palette_action(conn, action);
            return;
        }
        // Join mode: the picked window receives the offered pane,
        // instead of a landing.
        if let Some(source) = self.join_source.take() {
            let window = match item {
                PickerRef::Window(i, w) => picker
                    .entries
                    .get(i)
                    .and_then(|e| e.windows.get(w).map(|(id, _)| id.clone())),
                PickerRef::Workspace(_) | PickerRef::Command(_) => None,
            };
            let Some(window) = window else {
                self.leave_picker();
                return;
            };
            self.leave_picker();
            self.join_pane_into(conn, source, window);
            return;
        }
        let PickerRef::Workspace(index) = item else {
            return self.ws_picker_activate(conn, item);
        };
        let Some(workspaces) = picker.workspaces.clone() else {
            return;
        };
        let Some((id, _, _)) = workspaces.get(index) else {
            self.leave_picker();
            return;
        };
        let id = id.clone();
        self.leave_picker();
        self.land_on_workspace(conn, &id);
    }
    /// Activate a window picker row: land the view on that window.
    pub(super) fn ws_picker_activate(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        item: PickerRef,
    ) {
        let PickerRef::Window(entry_index, w) = item else {
            return;
        };
        let window = {
            let Modal::Picker(picker) = &self.modal else {
                return;
            };
            picker
                .entries
                .get(entry_index)
                .and_then(|e| e.windows.get(w).map(|(id, _)| id.clone()))
        };
        let Some(window) = window else {
            self.leave_picker();
            return;
        };
        self.leave_picker();
        self.land_window(conn, &window);
    }
    /// Compose the workspace picker's overlay: one row per workspace
    /// (`>`- and `*`-marked when active), through the same filter,
    /// cursor, windowing, and footer machinery as the session picker.
    pub(super) fn refresh_ws_picker(&mut self) {
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        let Some(workspaces) = &picker.workspaces else {
            return;
        };
        let mut rows: Vec<HelpRow> = Vec::new();
        let mut refs: Vec<PickerRef> = Vec::new();
        for (i, (id, name, active)) in workspaces.iter().enumerate() {
            let marker = if *active { ">" } else { " " };
            let star = if *active { " *" } else { "" };
            rows.push(HelpRow {
                text: format!(" {marker}{id}  {name}{star}"),
                accent: false,
                footer: false,
                render_only: false,
            });
            refs.push(PickerRef::Workspace(i));
        }
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = compose_picker_panel(
            &rows,
            &refs,
            &picker.filter,
            picker.filtering,
            picker.selected,
            visible,
            picker.start,
            PICKER_FOOTER,
        );
        self.store_picker_compose(filtered_refs, start, lines.len());
        self.renderer
            .set_overlay(Some((WORKSPACE_PICKER_OVERLAY_TITLE, lines, None)));
    }
    /// One key while the picker is up: the same modal-key shape the help
    /// panel runs, plus the selection cursor. Every key is consumed —
    /// nothing leaks into the pane.
    pub(super) fn picker_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        let _ = conn;
        use par_term_emu_core::keyboard::TermKey;
        // Every picker's filter is live from the open: arrows move the
        // selection without leaving the filter (typing owns every other
        // printable key).
        if let Modal::Picker(picker) = &self.modal {
            if picker.filtering {
                match ev.key() {
                    TermKey::Up => return self.picker_move(-1),
                    TermKey::Down => return self.picker_move(1),
                    _ => {}
                }
            }
        }
        let Modal::Picker(picker) = &mut self.modal else {
            return;
        };
        if picker.filtering {
            match ev.key() {
                TermKey::Char => {
                    if let Some(ch) = char::from_u32(ev.codepoint) {
                        picker.filter.push(ch);
                    }
                }
                TermKey::Escape => {
                    self.leave_picker();
                    return;
                }
                _ => picker.filtering = false,
            }
            self.refresh_picker();
            return;
        }
        match (ev.key(), ev.modifiers) {
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'/') => {
                picker.filtering = true;
                self.refresh_picker();
            }
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'j') => self.picker_move(1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'k') => self.picker_move(-1),
            (TermKey::Up, 0) => self.picker_move(-1),
            (TermKey::Down, 0) => self.picker_move(1),
            _ => self.leave_picker(),
        }
    }
    /// One plain byte while the picker is up — the byte spellings of the
    /// same controls (Backspace pops the filter, `/` opens it, Enter
    /// selects, `q`/Escape close).
    pub(super) fn picker_byte(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        byte: u8,
    ) -> bool {
        let Modal::Picker(picker) = &mut self.modal else {
            return false;
        };
        if picker.filtering {
            match byte {
                0x7f => {
                    picker.filter.pop();
                    self.refresh_picker();
                }
                0x1b => {
                    self.leave_picker();
                    return false;
                }
                b'\r' => {
                    // Enter runs the selected row straight from the live
                    // filter — the palette's semantics, now every
                    // picker's.
                    self.picker_activate(conn);
                }
                b if b.is_ascii_graphic() || b == b' ' => {
                    picker.filter.push(b as char);
                    self.refresh_picker();
                }
                b if b >= 0x80 => {
                    if let Some(ch) = self.text_utf8.push_printable(b) {
                        picker.filter.push(ch);
                        self.refresh_picker();
                    }
                }
                _ => {}
            }
            return matches!(self.modal, Modal::Picker(_));
        }
        match byte {
            b'/' => {
                picker.filtering = true;
                self.refresh_picker();
            }
            b'j' => self.picker_move(1),
            b'k' => self.picker_move(-1),
            b'\r' => self.picker_activate(conn),
            b'q' | 0x1b => self.leave_picker(),
            _ => {}
        }
        matches!(self.modal, Modal::Picker(_))
    }
    /// Record a picker compose's results on the open picker: the
    /// filtered refs (the selection/click map), the panned window start,
    /// and the panel length (the click hit-test's footer boundary).
    fn store_picker_compose(&mut self, refs: Vec<PickerRef>, start: usize, len: usize) {
        if let Modal::Picker(picker) = &mut self.modal {
            picker.refs = refs;
            picker.start = start;
            picker.panel_len = len;
        }
    }
}
