//! The modal state machines layered over the frame: help, the session and
//! workspace pickers, the rename/create prompt, the context menus, resize
//! mode, and the scroll viewport's keys.

use super::*;

/// The context menus' footer controls line.
pub(super) const MENU_FOOTER: &str = " action click · close esc/q ";

/// The context menus' overlay title (the target's name rides the panel's
/// accent header row).
pub(super) const MENU_TITLE: &str = " menu ";

/// The command menu's accent header (it has no target entity to name).
pub(super) const COMMAND_MENU_HEADER: &str = "commands";

/// Compose the context menu's overlay panel for `target`: the target's
/// name as the accent header, one row per action, the footer controls
/// line — [`super::super::compose_prompt_panel`]'s row/footer shape. Returns
/// the panel rows AND the action per row (`None` for the header and
/// footer) — the click dispatch's pure mapping, so what a click
/// highlights is exactly what painted there.
pub(super) fn compose_menu_panel(
    target: &MenuTarget,
    name: &str,
) -> (Vec<HelpRow>, Vec<Option<MenuAction>>) {
    let actions: &[MenuAction] = match target {
        MenuTarget::Tab(_) => &[MenuAction::Rename, MenuAction::Close, MenuAction::AddTab],
        MenuTarget::Workspace(_) => &[
            MenuAction::Rename,
            MenuAction::Close,
            MenuAction::NewWorkspace,
        ],
        MenuTarget::Commands => &[
            MenuAction::Keybinds,
            MenuAction::ReloadConfig,
            MenuAction::Detach,
        ],
    };
    let labels: &[&str] = match target {
        MenuTarget::Tab(_) => &["rename", "close", "add tab"],
        MenuTarget::Workspace(_) => &["rename", "close", "new"],
        MenuTarget::Commands => &["keybinds", "reload config", "detach"],
    };
    let mut rows = vec![HelpRow {
        text: format!(" {name} "),
        accent: true,
        footer: false,
    }];
    let mut mapping = vec![None];
    for (action, label) in actions.iter().zip(labels.iter()) {
        rows.push(HelpRow {
            text: format!(" {label} "),
            accent: false,
            footer: false,
        });
        mapping.push(Some(*action));
    }
    rows.push(HelpRow {
        text: MENU_FOOTER.to_string(),
        accent: false,
        footer: true,
    });
    mapping.push(None);
    (rows, mapping)
}

impl WindowSession {
    /// Make `modal` the one that owns input, replacing any other. A
    /// replaced scroll viewport releases its hold on the focused pane
    /// (snapping it to live), since nothing would be left to exit it.
    pub(super) fn open_modal(&mut self, modal: Modal) {
        if matches!(self.modal, Modal::Scroll) && !matches!(modal, Modal::Scroll) {
            if let Some(id) = self.renderer.focused() {
                self.renderer.exit_scroll_mode(id);
            }
        }
        self.modal = modal;
    }

    /// Enter the sticky resize mode (the `resize` chord): arrows adjust
    /// the focused pane's edges until Enter/Escape/`q` — tmux's resize
    /// step with an explicit mode instead of repeat-time. The flash cue
    /// rides the frame cadence on the status row.
    pub(super) fn enter_resize(&mut self) {
        self.open_modal(Modal::Resize);
        self.flash = Some(format!(
            "resize — arrows move the edge by {}, Enter/q exits",
            self.resize_step
        ));
    }

    /// Open the bindings help panel (the `help` chord): the live chord
    /// state's categories composed into the bordered modal (title
    /// `keybinds`, `esc close` badge, footer controls line) painted
    /// centered over the frame. `/` opens a filter-as-you-type box, j/k
    /// and pgup/pgdn scroll, esc/Enter/q close and the prior frame
    /// repaints. Keys never reach the pane while it is up.
    pub(super) fn enter_help(&mut self) {
        self.open_modal(Modal::Help(HelpState::default()));
        self.refresh_help();
    }

    /// Dismiss the help panel: the next frame's pane repaint restores the
    /// covered cells (the frame buffer resets, then panes repaint).
    pub(super) fn leave_help(&mut self) {
        if matches!(self.modal, Modal::Help(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }

    /// Re-compose the overlay from the live panel state (filter/scroll).
    pub(super) fn refresh_help(&mut self) {
        let Modal::Help(help) = &self.modal else {
            return;
        };
        let (filter, filtering, scroll) = (help.filter.clone(), help.filtering, help.scroll);
        let rows = super::super::help_rows(
            self.prefix,
            self.reload_key,
            self.management,
            self.resize_step,
        );
        // The modal's own chrome (2 border rows) and the footer surround
        // the content window; the filter line joins them only while a
        // filter is open or set (the footer already advertises
        // `search /` — no idle placeholder row).
        let filter_lines = usize::from(filtering || !filter.is_empty());
        let visible = usize::from(
            self.renderer
                .window_size()
                .1
                .saturating_sub(3 + filter_lines as u16),
        )
        .max(1);
        let content_len = super::super::help_content(&rows, &filter).len();
        let start = super::super::help_window_start(content_len, visible, scroll);
        let lines = super::super::compose_help_panel(&rows, &filter, filtering, visible, scroll);
        self.renderer.set_overlay(Some((
            super::super::HELP_OVERLAY_TITLE,
            lines,
            Some((start, visible, content_len)),
        )));
    }

    /// One key while the help panel is up: `/` opens the filter box
    /// (typing edits it, Enter commits and keeps the filter, Esc closes
    /// the panel), j/k/arrows/pgup/pgdn scroll, esc/Enter/q close. Every
    /// key is consumed — nothing leaks into the pane.
    pub(super) fn help_key(&mut self, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        let Modal::Help(help) = &mut self.modal else {
            return;
        };
        if help.filtering {
            match ev.key() {
                TermKey::Char => {
                    if let Some(ch) = char::from_u32(ev.codepoint) {
                        help.filter.push(ch);
                    }
                }
                TermKey::Escape => {
                    self.leave_help();
                    return;
                }
                _ => help.filtering = false,
            }
            self.refresh_help();
            return;
        }
        match (ev.key(), ev.modifiers) {
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'/') => {
                help.filtering = true;
                self.refresh_help();
            }
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'j') => self.help_scroll_by(1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'k') => self.help_scroll_by(-1),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'q') => self.leave_help(),
            (TermKey::Up, 0) => self.help_scroll_by(-1),
            (TermKey::Down, 0) => self.help_scroll_by(1),
            (TermKey::PageUp, 0) => self.help_scroll_by(-10),
            (TermKey::PageDown, 0) => self.help_scroll_by(10),
            // Enter (arrives as a byte via route_plain) and everything
            // else close the panel.
            _ => self.leave_help(),
        }
    }

    /// Scroll the help panel's content window (clamped by the compose).
    pub(super) fn help_scroll_by(&mut self, delta: isize) {
        if let Modal::Help(help) = &mut self.modal {
            help.scroll = (help.scroll as isize + delta).max(0) as usize;
        }
        self.refresh_help();
    }

    /// One plain byte while the help panel is up — the same controls the
    /// key path takes, for the byte spellings (Backspace pops the filter,
    /// `/` opens it, Enter commits or closes, `q` closes).
    pub(super) fn help_byte(&mut self, byte: u8) -> bool {
        let Modal::Help(help) = &mut self.modal else {
            return false;
        };
        if help.filtering {
            match byte {
                0x7f => {
                    help.filter.pop();
                    self.refresh_help();
                }
                b'\r' => help.filtering = false,
                b if byte != 0x1b && (b.is_ascii_graphic() || b == b' ') => {
                    help.filter.push(b as char);
                    self.refresh_help();
                }
                _ => {}
            }
            return matches!(self.modal, Modal::Help(_));
        }
        match byte {
            b'/' => {
                help.filtering = true;
                self.refresh_help();
            }
            b'q' | b'\r' => self.leave_help(),
            _ => {}
        }
        matches!(self.modal, Modal::Help(_))
    }

    /// Open the session/window picker (the `picker` chord): the daemon\'s
    /// session roster with each session\'s windows nested, composed into
    /// the same themed modal the help panel uses (title `picker`, `esc
    /// close` badge, footer controls line), the current session/window
    /// `>`-marked. `/` opens the filter box; arrows/j/k move the
    /// selection; Enter selects through [`Self::picker_activate`];
    /// esc/q dismiss and the prior frame repaints. Keys and the mouse
    /// never reach the pane while it is up.
    pub(super) fn enter_picker(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Ok(reply) = conn.send_checked("list-sessions") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let mut entries: Vec<super::super::PickerEntry> = Vec::new();
        for line in &reply.body {
            let Some((sid, name)) = super::super::parse_session_line(line) else {
                continue;
            };
            let mut windows: Vec<(String, String)> = Vec::new();
            let mut active_window: Option<String> = None;
            if let Ok(r) = conn.send_checked(&format!("list-windows -t {sid}")) {
                if r.ok {
                    for wl in &r.body {
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
            }
            let current = self.status.session_id.as_deref() == Some(sid.as_str());
            entries.push(super::super::PickerEntry {
                session_id: sid,
                session_name: name,
                windows,
                active_window,
                current,
            });
        }
        if entries.is_empty() {
            return;
        }
        // The selection opens on the current session\'s header row (or the
        // first row when the view\'s session is unknown to the roster).
        let (_rows, refs) = super::super::picker_rows(&entries, Some(&self.window));
        let selected = refs
            .iter()
            .position(|r| matches!(r, super::super::PickerRef::Session(i) if entries[*i].current))
            .unwrap_or(0);
        self.open_modal(Modal::Picker(PickerState {
            entries,
            selected,
            ..PickerState::default()
        }));
        self.refresh_picker();
    }

    /// Open the workspace picker (the workspace-picker chord): the
    /// daemon's workspace roster — id, name, the active one `>`-marked —
    /// in the same themed modal, filter, cursor, and click handling the
    /// session/window picker runs. Enter lands on the workspace through
    /// the same select-then-resync every switch follows.
    pub(super) fn enter_ws_picker(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Ok(reply) = conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let workspaces: Vec<(String, String, bool)> = reply
            .body
            .iter()
            .filter_map(|l| super::super::parse_workspace_line(l))
            .collect();
        if workspaces.is_empty() {
            return;
        }
        let open_on = workspaces.iter().position(|(_, _, active)| *active);
        self.open_modal(Modal::Picker(PickerState {
            selected: open_on.unwrap_or(0),
            workspaces: Some(workspaces),
            ..PickerState::default()
        }));
        self.refresh_ws_picker();
    }

    /// Dismiss the picker: the next frame\'s pane repaint restores the
    /// covered cells.
    pub(super) fn leave_picker(&mut self) {
        if matches!(self.modal, Modal::Picker(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }

    /// Open the prompt modal for `target`: the rename flows seed the
    /// target's current name — the window's or workspace's own name (a
    /// context menu may target a window/workspace the view is not
    /// showing) — and the create flows seed the next free index.
    pub(super) fn enter_prompt(&mut self, target: PromptTarget) {
        let seed = match &target {
            PromptTarget::Pane => self.status.pane_title().to_string(),
            PromptTarget::Window(id) => self
                .status
                .windows()
                .iter()
                .find(|(wid, _)| wid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            PromptTarget::NewWindow => super::super::next_window_name(self.status.windows()),
            PromptTarget::Workspace(id) => self
                .status
                .workspaces()
                .iter()
                .find(|(wsid, _)| wsid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_default(),
            PromptTarget::NewWorkspace => {
                super::super::next_workspace_name(self.status.workspaces())
            }
        };
        self.open_modal(Modal::Prompt(PromptState { text: seed, target }));
        self.refresh_prompt();
    }

    /// Dismiss the prompt: the next frame's pane repaint restores the
    /// covered cells.
    pub(super) fn leave_prompt(&mut self) {
        if matches!(self.modal, Modal::Prompt(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }

    /// Open the context menu for `target`: the tab menu (right-press on
    /// a tab), the workspace menu (a right-press on a workspace row), or
    /// the command menu (the panel's ` menu ` chip). The overlay rides
    /// the same modal machinery the picker uses.
    pub(super) fn open_menu(&mut self, target: MenuTarget) {
        let name = match &target {
            MenuTarget::Tab(id) => self
                .status
                .windows()
                .iter()
                .find(|(wid, _)| wid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| id.clone()),
            MenuTarget::Workspace(id) => self
                .status
                .workspaces()
                .iter()
                .find(|(wsid, _)| wsid == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| id.clone()),
            MenuTarget::Commands => COMMAND_MENU_HEADER.to_string(),
        };
        let (rows, actions) = compose_menu_panel(&target, &name);
        self.open_modal(Modal::Menu(MenuState { target, actions }));
        self.renderer.set_overlay(Some((MENU_TITLE, rows, None)));
    }

    /// Dismiss the menu: the next frame's pane repaint restores the
    /// covered cells.
    pub(super) fn leave_menu(&mut self) {
        if matches!(self.modal, Modal::Menu(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }

    /// One key while the menu is up: Escape closes, everything else is
    /// consumed (a modal owns the keyboard).
    pub(super) fn menu_key(&mut self, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        if ev.key() == TermKey::Escape {
            self.leave_menu();
        }
    }

    /// One plain byte while the menu is up: `q`/Escape close, the rest
    /// consumed.
    pub(super) fn menu_byte(&mut self, byte: u8) -> bool {
        if byte == b'q' || byte == 0x1b {
            self.leave_menu();
        }
        matches!(self.modal, Modal::Menu(_))
    }

    /// One press click while the menu is up, at overlay row `row` (the
    /// `overlay_row_at` mapping): an action row dispatches, the header
    /// and footer consume. The menu closes first so an action's own
    /// overlay (the rename prompt, the new-tab prompt) replaces it.
    pub(super) fn menu_click(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        row: usize,
    ) {
        let Modal::Menu(state) = &self.modal else {
            return;
        };
        let Some(action) = state.actions.get(row).copied().flatten() else {
            return; // header/footer/off-panel: consumed
        };
        let target = state.target.clone();
        self.leave_menu();
        match (target, action) {
            (MenuTarget::Tab(window), MenuAction::Rename) => {
                self.enter_prompt(PromptTarget::Window(window));
            }
            (MenuTarget::Tab(_), MenuAction::AddTab) => {
                self.enter_prompt(PromptTarget::NewWindow);
            }
            (MenuTarget::Tab(window), MenuAction::Close) => {
                self.close_window_from_menu(conn, &window);
            }
            (MenuTarget::Workspace(workspace), MenuAction::Rename) => {
                self.enter_prompt(PromptTarget::Workspace(workspace));
            }
            (MenuTarget::Workspace(_), MenuAction::NewWorkspace) => {
                self.enter_prompt(PromptTarget::NewWorkspace);
            }
            (MenuTarget::Workspace(workspace), MenuAction::Close) => {
                self.close_workspace_from_menu(conn, &workspace);
            }
            (MenuTarget::Commands, MenuAction::Keybinds) => self.enter_help(),
            (MenuTarget::Commands, MenuAction::ReloadConfig) => self.reload_config(conn),
            (MenuTarget::Commands, MenuAction::Detach) => self.detach_requested = true,
            // The cross pairs never compose (each menu carries only its
            // own actions).
            _ => {}
        }
    }

    /// The tab menu's close: `kill-window -t {id}`; when the killed
    /// window is the SHOWN one, land on the session's surviving
    /// daemon-active (or first) window through the select+resync — a
    /// session that died with its last window ends the view through the
    /// next status refresh's SessionGone. Any other target just marks
    /// the status stale.
    pub(super) fn close_window_from_menu(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let shown = self.window == window;
        let _ = conn.send_checked(&format!("kill-window -t {window}"));
        if !shown {
            self.status_dirty = true;
            return;
        }
        let Some(session) = self.status.session_id.clone() else {
            self.status_dirty = true;
            return;
        };
        let Ok(reply) = conn.send_checked(&format!("list-windows -t {session}")) else {
            return;
        };
        if !reply.ok {
            return;
        }
        let Some(survivor) = active_window_row(&reply.body) else {
            // The session died with the window: the next status refresh
            // reports SessionGone and the view ends cleanly.
            self.status_dirty = true;
            return;
        };
        self.land_window(conn, &survivor);
    }

    /// The workspace menu's close: `kill-workspace -t {id}`; when the
    /// killed workspace was the SHOWN one (the view's session lives in
    /// it), land on the first surviving workspace through the
    /// select+land contract — none remaining ends the view cleanly
    /// (the daemon kills the session, the refresh's SessionGone fires).
    /// Any other target just marks the status stale.
    pub(super) fn close_workspace_from_menu(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
    ) {
        // Was the view's session inside this workspace?
        let shown = self.status.session_id.as_ref().is_some_and(|session| {
            conn.send_checked(&format!("list-sessions -t {workspace}"))
                .ok()
                .filter(|reply| reply.ok)
                .is_some_and(|reply| {
                    reply
                        .body
                        .iter()
                        .filter_map(|l| super::super::parse_session_line(l))
                        .any(|(sid, _)| sid == *session)
                })
        });
        let _ = conn.send_checked(&format!("kill-workspace -t {workspace}"));
        if !shown {
            self.status_dirty = true;
            return;
        }
        let Ok(reply) = conn.send_checked("list-workspaces") else {
            return;
        };
        if !reply.ok {
            return;
        }
        let survivor = reply
            .body
            .iter()
            .filter_map(|l| super::super::parse_workspace_line(l))
            .map(|(id, _, _)| id)
            .next();
        let Some(survivor) = survivor else {
            // No workspace remains: the view ends cleanly through the
            // next status refresh.
            self.status_dirty = true;
            return;
        };
        let _ = conn.send_checked(&format!("select-workspace -t {survivor}"));
        self.land_on_workspace(conn, &survivor);
    }

    /// Re-compose the prompt overlay from the edit buffer.
    pub(super) fn refresh_prompt(&mut self) {
        let Modal::Prompt(prompt) = &self.modal else {
            return;
        };
        let title = match &prompt.target {
            PromptTarget::Pane => super::super::PROMPT_PANE_OVERLAY_TITLE,
            PromptTarget::Window(_) => super::super::PROMPT_WINDOW_OVERLAY_TITLE,
            PromptTarget::NewWindow => super::super::PROMPT_NEW_WINDOW_OVERLAY_TITLE,
            PromptTarget::Workspace(_) => super::super::PROMPT_WORKSPACE_OVERLAY_TITLE,
            PromptTarget::NewWorkspace => super::super::PROMPT_NEW_WORKSPACE_OVERLAY_TITLE,
        };
        let footer = match prompt.target {
            PromptTarget::NewWindow | PromptTarget::NewWorkspace => super::super::NEW_PROMPT_FOOTER,
            _ => super::super::PROMPT_FOOTER,
        };
        self.renderer.set_overlay(Some((
            title,
            super::super::compose_prompt_panel(&prompt.text, footer),
            None,
        )));
    }

    /// One plain byte while the prompt is up: printable bytes and
    /// spaces append, Backspace pops, ^C clears the input (herdr's
    /// footer), Enter commits (the daemon command below), Escape
    /// cancels. Returns whether the mode is still up.
    pub(super) fn prompt_byte(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        byte: u8,
    ) -> bool {
        match byte {
            0x1b => self.leave_prompt(),
            b'\r' => self.commit_prompt(conn),
            0x03 => {
                if let Modal::Prompt(prompt) = &mut self.modal {
                    prompt.text.clear();
                }
                self.refresh_prompt();
            }
            0x7f => {
                if let Modal::Prompt(prompt) = &mut self.modal {
                    prompt.text.pop();
                }
                self.refresh_prompt();
            }
            b if b.is_ascii_graphic() || b == b' ' => {
                if let Modal::Prompt(prompt) = &mut self.modal {
                    prompt.text.push(b as char);
                }
                self.refresh_prompt();
            }
            _ => {}
        }
        matches!(self.modal, Modal::Prompt(_))
    }

    /// One key event while the prompt is up: characters append, Escape
    /// cancels, everything else is consumed (the byte path carries the
    /// controls).
    pub(super) fn prompt_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        let _ = conn;
        use par_term_emu_core::keyboard::TermKey;
        match ev.key() {
            TermKey::Char => {
                if let (Some(ch), Modal::Prompt(prompt)) =
                    (char::from_u32(ev.codepoint), &mut self.modal)
                {
                    prompt.text.push(ch);
                }
            }
            TermKey::Escape => {
                self.leave_prompt();
                return;
            }
            _ => {}
        }
        self.refresh_prompt();
    }

    /// Commit the prompt: the pane spelling sets the sticky user title
    /// (`select-pane -T`), the window and workspace spellings rename
    /// their target (`rename-window`/`rename-workspace`, quoted), the
    /// create flows make the window/workspace and land the view on it;
    /// an empty input cancels (an empty name is not expressible on the
    /// wire — the parses require one).
    pub(super) fn commit_prompt(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Modal::Prompt(prompt) = &self.modal else {
            return;
        };
        let text = prompt.text.trim().to_string();
        let target = prompt.target.clone();
        if text.is_empty() {
            self.leave_prompt();
            return;
        }
        match target {
            PromptTarget::Pane => {
                let _ = conn.send_checked(&format!(
                    "select-pane -t {} -T {}",
                    self.focused_pane(),
                    super::super::wire_quote(&text)
                ));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::Window(id) => {
                // rename-window takes the REST OF THE LINE verbatim (the
                // daemon grammar's documented exception) — quoting would
                // become part of the name (the manual-pass round-8
                // report: renamed tabs wore literal quotes).
                let _ = conn.send_checked(&format!("rename-window -t {id} {text}"));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::Workspace(id) => {
                let _ = conn.send_checked(&format!(
                    "rename-workspace -t {id} {}",
                    super::super::wire_quote(&text)
                ));
                self.leave_prompt();
                self.status_dirty = true;
            }
            PromptTarget::NewWindow => {
                let Some(session) = self.status.session_id.clone() else {
                    self.leave_prompt();
                    return;
                };
                let Ok(reply) = conn.send_checked(&format!(
                    "new-window -t {session} -n {}",
                    super::super::wire_quote(&text)
                )) else {
                    self.leave_prompt();
                    return;
                };
                self.leave_prompt();
                if !reply.ok {
                    return;
                }
                if let Some(window) = reply.body.first().map(|w| w.trim().to_string()) {
                    self.land_window(conn, &window);
                }
            }
            PromptTarget::NewWorkspace => {
                let Ok(reply) = conn.send_checked(&format!(
                    "new-workspace -n {}",
                    super::super::wire_quote(&text)
                )) else {
                    self.leave_prompt();
                    return;
                };
                self.leave_prompt();
                if !reply.ok {
                    return;
                }
                if let Some(workspace) = reply.body.first().map(|w| w.trim().to_string()) {
                    let _ = conn.send_checked(&format!("select-workspace -t {workspace}"));
                    self.land_on_workspace(conn, &workspace);
                }
            }
        }
    }

    /// Re-compose the picker overlay from the live state (filter,
    /// selection, pan). The workspace picker composes its own rows and
    /// rides the same filter/window/footer machinery.
    pub(super) fn refresh_picker(&mut self) {
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        if picker.workspaces.is_some() {
            self.refresh_ws_picker();
            return;
        }
        let (rows, refs) = super::super::picker_rows(&picker.entries, Some(&self.window));
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = super::super::compose_picker_panel(
            &rows,
            &refs,
            &picker.filter,
            picker.filtering,
            picker.selected,
            visible,
            picker.start,
        );
        self.store_picker_compose(filtered_refs, start, lines.len());
        self.renderer
            .set_overlay(Some((super::super::PICKER_OVERLAY_TITLE, lines, None)));
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

    /// Activate the selected picker row: a session header lands on the
    /// session\'s active window (the select-session-equivalent), a window
    /// row lands on that window — daemon-side `select-window`, then the
    /// select+resync re-seed every switch follows.
    pub(super) fn picker_activate(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        let idx = picker.selected.min(picker.refs.len().saturating_sub(1));
        let Some(item) = picker.refs.get(idx).copied() else {
            return;
        };
        let super::super::PickerRef::Workspace(index) = item else {
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

    /// Activate a session/window picker row: a session header lands on
    /// the session's active window, a window row on that window.
    pub(super) fn ws_picker_activate(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        item: super::super::PickerRef,
    ) {
        let entry_index = match item {
            super::super::PickerRef::Session(i) | super::super::PickerRef::Window(i, _) => i,
            super::super::PickerRef::Workspace(_) => return,
        };
        let Modal::Picker(picker) = &self.modal else {
            return;
        };
        let entry = &picker.entries[entry_index];
        let window = match item {
            // Unreachable: ws_picker_activate returned early on Workspace.
            super::super::PickerRef::Workspace(_) => None,
            super::super::PickerRef::Session(_) => entry
                .active_window
                .clone()
                .or_else(|| entry.windows.first().map(|(id, _)| id.clone())),
            super::super::PickerRef::Window(_, w) => entry.windows.get(w).map(|(id, _)| id.clone()),
        };
        let Some(window) = window else {
            // A session with no windows: nothing to land on; dismiss.
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
        let mut rows: Vec<super::super::HelpRow> = Vec::new();
        let mut refs: Vec<super::super::PickerRef> = Vec::new();
        for (i, (id, name, active)) in workspaces.iter().enumerate() {
            let marker = if *active { ">" } else { " " };
            let star = if *active { " *" } else { "" };
            rows.push(super::super::HelpRow {
                text: format!(" {marker}{id}  {name}{star}"),
                accent: false,
                footer: false,
            });
            refs.push(super::super::PickerRef::Workspace(i));
        }
        let visible = self.picker_visible();
        let (lines, filtered_refs, start) = super::super::compose_picker_panel(
            &rows,
            &refs,
            &picker.filter,
            picker.filtering,
            picker.selected,
            visible,
            picker.start,
        );
        self.store_picker_compose(filtered_refs, start, lines.len());
        self.renderer.set_overlay(Some((
            super::super::WORKSPACE_PICKER_OVERLAY_TITLE,
            lines,
            None,
        )));
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
                b'\r' => picker.filtering = false,
                b if b.is_ascii_graphic() || b == b' ' => {
                    picker.filter.push(b as char);
                    self.refresh_picker();
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
    fn store_picker_compose(
        &mut self,
        refs: Vec<super::super::PickerRef>,
        start: usize,
        len: usize,
    ) {
        if let Modal::Picker(picker) = &mut self.modal {
            picker.refs = refs;
            picker.start = start;
            picker.panel_len = len;
        }
    }

    /// One key while resize mode is up: arrows send one resize step for
    /// the focused pane (`resize-pane -t <pane> -L|-R|-U|-D <step>`, the
    /// wire's relative form); Escape and `q` exit; every other key exits
    /// the mode and is consumed (keys must not leak into the pane during
    /// a modal chord).
    pub(super) fn resize_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use par_term_emu_core::keyboard::TermKey;
        match ev.key() {
            TermKey::Up if ev.modifiers == 0 => self.send_resize(conn, "-U"),
            TermKey::Down if ev.modifiers == 0 => self.send_resize(conn, "-D"),
            TermKey::Right if ev.modifiers == 0 => self.send_resize(conn, "-R"),
            TermKey::Left if ev.modifiers == 0 => self.send_resize(conn, "-L"),
            // Escape, q, Enter (as bytes via route_plain), or anything
            // else: the mode exits and the key is consumed.
            _ => self.modal = Modal::None,
        }
    }

    /// One resize step for the focused pane, the wire's relative form.
    /// Best-effort — the %layout-change broadcast the resize queues
    /// re-seeds the window through the pump's parked-layout path (PendingWork).
    pub(super) fn send_resize(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        flag: &str,
    ) {
        let focused = self.focused_pane();
        if focused.is_empty() {
            return;
        }
        // Divider-direction semantics: the arrows move the SHARED divider
        // in the pressed direction regardless of which side is focused
        // (the manual-pass report — the right pane used to invert). The
        // wire's resize-pane GROWS the focused pane, so when the focused
        // pane sits on the far side of its boundary (a neighbor to its
        // left, or above), the wire direction inverts.
        let focused_id: u32 = focused[1..].parse().unwrap_or(u32::MAX);
        let layout = self.renderer.layout();
        let invert = match layout.iter().find(|r| r.pane == focused_id) {
            Some(f) => match flag {
                "-L" | "-R" => layout
                    .iter()
                    .any(|r| r.pane != f.pane && r.x + r.width == f.x && rows_overlap(r, f)),
                "-U" | "-D" => layout
                    .iter()
                    .any(|r| r.pane != f.pane && r.y + r.height == f.y && cols_overlap(r, f)),
                _ => false,
            },
            None => false,
        };
        let flag = match (flag, invert) {
            ("-L", true) => "-R",
            ("-R", true) => "-L",
            ("-U", true) => "-D",
            ("-D", true) => "-U",
            (other, _) => other,
        };
        let _ = conn.send_checked(&format!(
            "resize-pane -t {focused} {flag} {}",
            self.resize_step
        ));
    }

    /// Leave scroll mode: clear the hold and snap the focused pane to
    /// live.
    pub(super) fn leave_scroll_mode(&mut self) {
        if matches!(self.modal, Modal::Scroll) {
            self.modal = Modal::None;
        }
        if let Some(id) = self.renderer.focused() {
            self.renderer.exit_scroll_mode(id);
        }
    }

    /// One functional key while scroll mode is up: arrows line-scroll,
    /// PgUp/PgDn page by the pane's height, Home jumps to the top of
    /// history, End and q and Enter exit. Keys never reach the pane
    /// while the viewport is up.
    pub(super) fn scroll_mode_key(&mut self, ev: &TermKeyEvent) {
        use par_term_emu_core::keyboard::TermKey;
        let Some(id) = self.renderer.focused() else {
            return;
        };
        let rows = self
            .renderer
            .pane_terminal(id)
            .map(|t| t.active_grid().rows().max(1) as isize)
            .unwrap_or(1);
        match (ev.key(), ev.modifiers) {
            (TermKey::Up, 0) => {
                self.renderer.scroll_viewport(id, 1);
            }
            (TermKey::Down, 0) => {
                self.renderer.scroll_viewport(id, -1);
            }
            (TermKey::PageUp, 0) => {
                self.renderer.scroll_viewport(id, rows);
            }
            (TermKey::PageDown, 0) => {
                self.renderer.scroll_viewport(id, -rows);
            }
            (TermKey::Home, 0) => {
                let max = self
                    .renderer
                    .pane_terminal(id)
                    .map(|t| t.active_grid().scrollback_len() as isize)
                    .unwrap_or(0);
                self.renderer.scroll_viewport(id, max);
            }
            (TermKey::End, 0) => self.leave_scroll_mode(),
            (TermKey::Char, 0) if ev.codepoint == u32::from(b'q') => self.leave_scroll_mode(),
            _ => {}
        }
    }
}
