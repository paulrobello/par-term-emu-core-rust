//! The context menus: panel composition (rows paired with their
//! actions), open/leave, keys and bytes, click dispatch, and the
//! window/workspace close actions the menu rows trigger.

use super::{
    GroupCloseArm, MenuAction, MenuState, MenuTarget, Modal, PromptTarget, WindowSession,
    GROUP_CLOSE_WINDOW,
};
use crate::mux::attach::panels::HelpRow;
use crate::mux::attach::targets::{active_window_row, parse_workspace_line};
use crate::mux::ipc::SessionRow;
use par_term_emu_core::keyboard::TermKeyEvent;

/// Where a tab-menu/palette move row sends its window: one slot
/// left/right, or the first/last slot outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TabMove {
    Left,
    Right,
    Start,
    End,
}

/// The context menus' footer controls line.
pub(super) const MENU_FOOTER: &str = " navigate arrows/j/k · action enter/click · close esc/q ";
/// The context menus' overlay title (the target's name rides the panel's
/// accent header row).
pub(super) const MENU_TITLE: &str = " menu ";
/// Compose the context menu's overlay panel for `target`: the target's
/// name as the accent header (the command menu has no target entity, so
/// no header), one row per action, the footer controls line —
/// [`compose_prompt_panel`]'s row/footer shape. Returns the panel rows
/// AND the action per row (`None` for the header and footer) — the
/// click dispatch's pure mapping, so what a click highlights is exactly
/// what painted there. `group_armed` swaps the workspace menu's `close`
/// for `close group` in the same row (a refused close armed it).
pub(super) fn compose_menu_panel(
    target: &MenuTarget,
    name: Option<&str>,
    group_armed: bool,
) -> (Vec<HelpRow>, Vec<Option<MenuAction>>) {
    let close = if group_armed {
        MenuAction::CloseGroup
    } else {
        MenuAction::Close
    };
    let ws_actions = [
        MenuAction::Rename,
        close,
        MenuAction::NewWorkspace,
        MenuAction::RespawnToggle,
        MenuAction::ExitAction("remove"),
        MenuAction::ExitAction("hold"),
        MenuAction::ExitAction("shell"),
    ];
    let actions: &[MenuAction] = match target {
        MenuTarget::Tab(_) => &[
            MenuAction::Rename,
            MenuAction::Close,
            MenuAction::AddTab,
            MenuAction::MoveLeft,
            MenuAction::MoveRight,
            MenuAction::MoveFirst,
            MenuAction::MoveLast,
        ],
        MenuTarget::Workspace(_) => &ws_actions,
        MenuTarget::Commands => &[
            MenuAction::Settings,
            MenuAction::Layouts,
            MenuAction::Keybinds,
            MenuAction::ReloadConfig,
            MenuAction::Detach,
        ],
    };
    let labels: &[&str] = match target {
        MenuTarget::Tab(_) => &[
            "rename",
            "close",
            "add tab",
            "move left",
            "move right",
            "move to start",
            "move to end",
        ],
        MenuTarget::Workspace(_) if group_armed => &[
            "rename",
            "close group",
            "new",
            "respawn on exit",
            "exit: remove",
            "exit: hold",
            "exit: shell",
        ],
        MenuTarget::Workspace(_) => &[
            "rename",
            "close",
            "new",
            "respawn on exit",
            "exit: remove",
            "exit: hold",
            "exit: shell",
        ],
        MenuTarget::Commands => &[
            "settings",
            "Layouts…",
            "keybinds",
            "reload config",
            "detach",
        ],
    };
    let mut rows = Vec::new();
    let mut mapping = Vec::new();
    if let Some(name) = name {
        rows.push(HelpRow {
            text: format!(" {name} "),
            accent: true,
            footer: false,
            render_only: false,
        });
        mapping.push(None);
    }
    for (action, label) in actions.iter().zip(labels.iter()) {
        rows.push(HelpRow {
            text: format!(" {label} "),
            accent: false,
            footer: false,
            render_only: false,
        });
        mapping.push(Some(*action));
    }
    rows.push(HelpRow {
        text: MENU_FOOTER.to_string(),
        accent: false,
        footer: true,
        render_only: false,
    });
    mapping.push(None);
    (rows, mapping)
}

impl WindowSession {
    /// The target's display name for the menu header (tab menus and
    /// workspace menus carry one; the command menu does not).
    fn menu_name(&self, target: &MenuTarget) -> Option<String> {
        match target {
            MenuTarget::Tab(id) => Some(
                self.status
                    .windows()
                    .iter()
                    .find(|(wid, _)| wid == id)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| id.clone()),
            ),
            MenuTarget::Workspace(id) => Some(
                self.status
                    .workspaces()
                    .iter()
                    .find(|(wsid, _)| wsid == id)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| id.clone()),
            ),
            MenuTarget::Commands => None,
        }
    }
    /// Whether `target` is the workspace an unexpired refused close armed
    /// for `close group`.
    fn group_close_armed(&self, target: &MenuTarget) -> bool {
        match (target, &self.group_close) {
            (MenuTarget::Workspace(id), Some(arm)) => {
                arm.workspace == *id && std::time::Instant::now() < arm.until
            }
            _ => false,
        }
    }
    /// Open the context menu for `target`: the tab menu (right-press on
    /// a tab), the workspace menu (a right-press on a workspace row), or
    /// the command menu (the panel's ` menu ` chip). The overlay rides
    /// the same modal machinery the picker uses.
    pub(super) fn open_menu(&mut self, target: MenuTarget) {
        let name = self.menu_name(&target);
        let armed = self.group_close_armed(&target);
        let (_rows, actions) = compose_menu_panel(&target, name.as_deref(), armed);
        // The keyboard cursor opens on the first action row (past the
        // name header a tab/workspace menu carries), highlighted like a
        // hover.
        let selected = actions
            .iter()
            .position(|action| action.is_some())
            .unwrap_or(0);
        self.open_modal(Modal::Menu(MenuState {
            target,
            actions,
            selected,
        }));
        self.repaint_menu();
    }
    /// One no-button motion while the menu is up, at window-relative
    /// `(x, y)`: the action row under the pointer becomes the selected
    /// row — the same index the arrow keys move, so one reversed-label
    /// highlight serves hover and keyboard. A motion off the panel or
    /// onto the header/footer leaves the selection where it was. No-op
    /// when the selection did not move rows.
    pub(super) fn menu_hover(&mut self, x: u16, y: u16) {
        let row = match &self.modal {
            Modal::Menu(state) => self.renderer.overlay_row_at(x, y).filter(|&row| {
                state
                    .actions
                    .get(row)
                    .is_some_and(|action| action.is_some())
            }),
            _ => return,
        };
        let Some(row) = row else {
            return;
        };
        if let Modal::Menu(state) = &mut self.modal {
            if state.selected == row {
                return;
            }
            state.selected = row;
        }
        self.repaint_menu();
    }
    /// Recompose the open menu's overlay rows with the current selection
    /// highlight. The highlight spans the selected row's own label, not
    /// the panel width — the subtle cue, one visual language for the
    /// pointer's hover and the keyboard's cursor.
    pub(super) fn repaint_menu(&mut self) {
        // The label follows the click mapping fixed at open, not the
        // clock, so a row that still dispatches `close group` says so.
        let (target, selected, armed) = match &self.modal {
            Modal::Menu(state) => (
                state.target.clone(),
                state.selected,
                state.actions.contains(&Some(MenuAction::CloseGroup)),
            ),
            _ => return,
        };
        let name = self.menu_name(&target);
        let (rows, _actions) = compose_menu_panel(&target, name.as_deref(), armed);
        let on_action = matches!(
            self.modal,
            Modal::Menu(ref state) if matches!(state.actions.get(selected), Some(Some(_)))
        );
        let highlights = match rows.get(selected) {
            Some(label) if on_action => vec![(
                selected,
                0,
                crate::mux::attach::text_input::display_width(&label.text),
            )],
            _ => Vec::new(),
        };
        self.renderer.set_overlay(Some((MENU_TITLE, rows, None)));
        self.renderer.set_overlay_highlights(highlights);
    }
    /// Move the menu's selection by `delta` action rows (the cursor
    /// wraps, the picker's convention) and re-compose. The header and
    /// footer rows are not stops: the index jumps them.
    pub(super) fn menu_move(&mut self, delta: isize) {
        let Modal::Menu(state) = &mut self.modal else {
            return;
        };
        let rows: Vec<usize> = state
            .actions
            .iter()
            .enumerate()
            .filter_map(|(row, action)| action.is_some().then_some(row))
            .collect();
        let count = rows.len();
        if count == 0 {
            return;
        }
        let at = rows
            .iter()
            .position(|&row| row == state.selected)
            .unwrap_or(0);
        state.selected = rows[(at as isize + delta).rem_euclid(count as isize) as usize];
        self.repaint_menu();
    }
    /// Dismiss the menu: the next frame's pane repaint restores the
    /// covered cells.
    pub(super) fn leave_menu(&mut self) {
        if matches!(self.modal, Modal::Menu(_)) {
            self.modal = Modal::None;
        }
        self.renderer.set_overlay(None);
    }
    /// One key while the menu is up: Escape closes, Up/Down move the
    /// selection (wrapping, skipping the header and footer), Enter
    /// dispatches the selected action through [`Self::menu_click`], and
    /// everything else is consumed (a modal owns the keyboard).
    pub(super) fn menu_key(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        ev: &TermKeyEvent,
    ) {
        use par_term_emu_core::keyboard::TermKey;
        match (ev.key(), ev.modifiers) {
            (TermKey::Escape, _) => self.leave_menu(),
            (TermKey::Up, 0) => self.menu_move(-1),
            (TermKey::Down, 0) => self.menu_move(1),
            // A CSI-u Enter (kitty keyboard protocol) rides the key
            // route; the plain CR byte lands in `menu_byte`.
            (TermKey::Enter, 0) => self.menu_enter(conn),
            _ => {}
        }
    }
    /// One plain byte while the menu is up: `j`/`k` move the selection,
    /// Enter (CR) dispatches the selected action, `q`/Escape close, the
    /// rest consumed.
    pub(super) fn menu_byte(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        byte: u8,
    ) -> bool {
        match byte {
            b'q' | 0x1b => self.leave_menu(),
            b'j' => self.menu_move(1),
            b'k' => self.menu_move(-1),
            b'\r' => self.menu_enter(conn),
            _ => {}
        }
        matches!(self.modal, Modal::Menu(_))
    }
    /// Dispatch the selected action through the click path: `menu_click`
    /// reads the action off the open state's row mapping, closes the
    /// menu first (spends any armed group-close offer), and the action's
    /// own overlay (a rename prompt, the settings dialog) replaces it.
    fn menu_enter(&mut self, conn: &mut crate::mux::attach::conn::AttachConn) {
        let Modal::Menu(state) = &self.modal else {
            return;
        };
        let row = state.selected;
        self.menu_click(conn, row);
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
        // Any menu action spends the offer: a stale arm must never
        // group-close by surprise.
        let arm = self.group_close.take();
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
            (MenuTarget::Tab(window), MenuAction::MoveLeft) => {
                self.move_window_slot(conn, &window, TabMove::Left);
            }
            (MenuTarget::Tab(window), MenuAction::MoveRight) => {
                self.move_window_slot(conn, &window, TabMove::Right);
            }
            (MenuTarget::Tab(window), MenuAction::MoveFirst) => {
                self.move_window_slot(conn, &window, TabMove::Start);
            }
            (MenuTarget::Tab(window), MenuAction::MoveLast) => {
                self.move_window_slot(conn, &window, TabMove::End);
            }
            (MenuTarget::Workspace(workspace), MenuAction::Rename) => {
                self.enter_prompt(PromptTarget::Workspace(workspace));
            }
            (MenuTarget::Workspace(_), MenuAction::NewWorkspace) => {
                self.enter_prompt(PromptTarget::NewWorkspace);
            }
            (MenuTarget::Workspace(workspace), MenuAction::Close) => {
                self.close_workspace_from_menu(conn, &workspace, None);
            }
            (MenuTarget::Workspace(workspace), MenuAction::CloseGroup) => {
                let members = arm
                    .filter(|arm| arm.workspace == workspace)
                    .map(|arm| arm.members)
                    .unwrap_or_default();
                self.close_workspace_from_menu(conn, &workspace, Some(members));
            }
            (MenuTarget::Workspace(workspace), MenuAction::RespawnToggle) => {
                self.toggle_workspace_respawn(conn, &workspace);
            }
            (MenuTarget::Workspace(workspace), MenuAction::ExitAction(value)) => {
                self.set_workspace_exit_action(conn, &workspace, value);
            }
            (MenuTarget::Commands, MenuAction::Settings) => self.enter_settings(),
            (MenuTarget::Commands, MenuAction::Layouts) => self.enter_layouts(),
            (MenuTarget::Commands, MenuAction::Keybinds) => self.enter_help(),
            (MenuTarget::Commands, MenuAction::ReloadConfig) => self.reload_config(conn),
            (MenuTarget::Commands, MenuAction::Detach) => self.detach_requested = true,
            // The cross pairs never compose (each menu carries only its
            // own actions).
            _ => {}
        }
    }
    /// The tab menu's move rows and the palette's move-tab rows: send
    /// `move-window -s {id} -t {slot}` for `window` — one slot
    /// left/right (an edge flashes, the move chords' rule) or the
    /// first/last slot outright (the daemon clamps out-of-range slots
    /// to the ends). A no-op move (already at the slot) sends nothing.
    pub(super) fn move_window_slot(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
        to: TabMove,
    ) {
        let windows: Vec<String> = self
            .status
            .windows()
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        let Some(at) = windows.iter().position(|id| id == window) else {
            return;
        };
        let slot = match to {
            TabMove::Start => 0,
            TabMove::End => windows.len().saturating_sub(1),
            TabMove::Left => {
                if at == 0 {
                    self.chrome.flash("already the first tab".to_string());
                    self.draw_status_row();
                    return;
                }
                at - 1
            }
            TabMove::Right => {
                if at + 1 >= windows.len() {
                    self.chrome.flash("already the last tab".to_string());
                    self.draw_status_row();
                    return;
                }
                at + 1
            }
        };
        if slot == at {
            return;
        }
        let _ = conn.send_forget(&format!("move-window -s {window} -t {slot}"));
        self.status_dirty = true;
    }
    /// The workspace menu's `respawn on exit`: flip the workspace's
    /// keep-one-tab respawn option (`set-workspace-respawn -t {id}
    /// toggle`) and flash the state the daemon replies.
    pub(super) fn toggle_workspace_respawn(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
    ) {
        self.send_then(
            conn,
            &format!("set-workspace-respawn -t {workspace} toggle"),
            |s, _, reply| {
                let text = if reply.ok {
                    let state = reply.body.first().map(String::as_str).unwrap_or("?");
                    format!("respawn on exit: {state}")
                } else {
                    format!(
                        "respawn on exit: {}",
                        reply.body.first().map(String::as_str).unwrap_or("failed")
                    )
                };
                s.chrome.flash(text);
                s.draw_status_row();
            },
        );
    }

    /// The workspace menu's `exit:` rows: set the workspace's on-exit
    /// behavior (`set-exit-action -t {id} {value}`) and flash the
    /// resulting state the daemon replies.
    pub(super) fn set_workspace_exit_action(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
        value: &'static str,
    ) {
        self.send_then(
            conn,
            &format!("set-exit-action -t {workspace} {value}"),
            |s, _, reply| {
                let text = if reply.ok {
                    let state = reply.body.first().map(String::as_str).unwrap_or("?");
                    format!("exit action: {state}")
                } else {
                    format!(
                        "exit action: {}",
                        reply.body.first().map(String::as_str).unwrap_or("failed")
                    )
                };
                s.chrome.flash(text);
                s.draw_status_row();
            },
        );
    }

    /// The tab menu's close: `kill-window -t {id}`; when the killed
    /// window is the SHOWN one, land on the session's surviving
    /// daemon-active (or first) window through the select+resync — a
    /// session that died with its last window lands up the tree through
    /// the next status refresh's SessionGone. Any other target just marks
    /// the status stale.
    pub(super) fn close_window_from_menu(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        window: &str,
    ) {
        let shown = self.window == window;
        // In-order daemon: the list-windows below observes the kill.
        if conn
            .send_forget(&format!("kill-window -t {window}"))
            .is_err()
        {
            return;
        }
        if !shown {
            self.status_dirty = true;
            return;
        }
        let Some(session) = self.status.session_id.clone() else {
            self.status_dirty = true;
            return;
        };
        self.send_then(
            conn,
            &format!("list-windows -t {session}"),
            |s, c, reply| {
                if !reply.ok {
                    return;
                }
                let Some(survivor) = active_window_row(&reply.body) else {
                    // The session died with the window: the next status
                    // refresh reports SessionGone and lands up the tree.
                    s.status_dirty = true;
                    return;
                };
                s.land_window(c, &survivor);
            },
        );
    }
    /// The workspace menu's close: `kill-workspace -t {id}` (with `-g`
    /// when `group` carries the armed offer's linked members); when the
    /// killed workspace — or, for a group, any member — held the view's
    /// session, land on the first surviving workspace through the
    /// select+land contract; none remaining ends the view cleanly (the
    /// daemon kills the session, the refresh's SessionGone fires). Any
    /// other target just marks the status stale. A refusal flashes its
    /// reason ([`Self::workspace_close_refused`]).
    pub(super) fn close_workspace_from_menu(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
        group: Option<Vec<String>>,
    ) {
        let workspace = workspace.to_string();
        let grouped = group.is_some();
        let mut closing = group.unwrap_or_default();
        closing.push(workspace.clone());
        match self.status.session_id.clone() {
            Some(session) => {
                // A group's view may sit in a linked member, so the group
                // close lists every session and matches the owner's
                // workspace against the whole group.
                let query = if grouped {
                    "list-sessions".to_string()
                } else {
                    format!("list-sessions -t {workspace}")
                };
                self.send_then(conn, &query, move |s, c, listed| {
                    let shown = listed.ok
                        && listed
                            .body
                            .iter()
                            .filter_map(|l| SessionRow::parse(l))
                            .any(|row| {
                                row.id == session
                                    && (!grouped
                                        || row
                                            .workspace
                                            .as_ref()
                                            .is_some_and(|(ws, _)| closing.contains(ws)))
                            });
                    s.kill_workspace_then_land(c, &workspace, grouped, shown);
                });
            }
            None => self.kill_workspace_then_land(conn, &workspace, grouped, false),
        }
    }
    fn kill_workspace_then_land(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
        group: bool,
        shown: bool,
    ) {
        let command = if group {
            format!("kill-workspace -t {workspace} -g")
        } else {
            format!("kill-workspace -t {workspace}")
        };
        let workspace = workspace.to_string();
        self.send_then(conn, &command, move |s, c, reply| {
            if !reply.ok {
                s.workspace_close_refused(&workspace, &reply.body);
                return;
            }
            if !shown {
                s.status_dirty = true;
                return;
            }
            s.send_then(c, "list-workspaces", |s, c, reply| {
                if !reply.ok {
                    return;
                }
                let survivor = reply
                    .body
                    .iter()
                    .filter_map(|l| parse_workspace_line(l))
                    .map(|(id, _, _)| id)
                    .next();
                let Some(survivor) = survivor else {
                    // No workspace remains: the view ends cleanly through
                    // the next status refresh.
                    s.status_dirty = true;
                    return;
                };
                if c.send_forget(&format!("select-workspace -t {survivor}"))
                    .is_err()
                {
                    return;
                }
                s.land_on_workspace(c, &survivor);
            });
        });
    }
    /// A refused `kill-workspace`: the group rule's refusal (a repo
    /// workspace with open linked worktree workspaces) flashes the linked
    /// count and the `-g` remedy, and arms `close group` in that
    /// workspace's menu for [`GROUP_CLOSE_WINDOW`]; any other refusal
    /// flashes the daemon's reason.
    fn workspace_close_refused(&mut self, workspace: &str, body: &[String]) {
        let reason = body.first().map(String::as_str).unwrap_or("refused");
        match parse_group_refusal(reason) {
            Some(members) => {
                let name = self
                    .menu_name(&MenuTarget::Workspace(workspace.to_string()))
                    .unwrap_or_else(|| workspace.to_string());
                self.chrome.flash(group_refusal_flash(&name, members.len()));
                self.group_close = Some(GroupCloseArm {
                    workspace: workspace.to_string(),
                    members,
                    until: std::time::Instant::now() + GROUP_CLOSE_WINDOW,
                });
            }
            None => self.chrome.flash(format!("close: {reason}")),
        }
        self.draw_status_row();
    }
}

/// The linked worktree workspaces a `kill-workspace` group refusal names
/// (`workspace +N has linked worktree workspaces (+A, +B); use -g …`);
/// `None` for any other reply.
pub(super) fn parse_group_refusal(reason: &str) -> Option<Vec<String>> {
    let (_, rest) = reason.split_once("has linked worktree workspaces (")?;
    let (list, tail) = rest.split_once(')')?;
    if !tail.contains("-g") {
        return None;
    }
    let members: Vec<String> = list
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    (!members.is_empty()).then_some(members)
}

/// The status-row flash for a group refusal: the workspace, its linked
/// worktree count, and the `-g` remedy the menu now offers.
pub(super) fn group_refusal_flash(name: &str, count: usize) -> String {
    let noun = if count == 1 { "worktree" } else { "worktrees" };
    format!("{name}: {count} linked {noun} - menu 'close group' sends -g")
}
