//! The context menus: panel composition (rows paired with their
//! actions), open/leave, keys and bytes, click dispatch, and the
//! window/workspace close actions the menu rows trigger.

use super::{MenuAction, MenuState, MenuTarget, Modal, PromptTarget, WindowSession};
use crate::mux::attach::panels::HelpRow;
use crate::mux::attach::targets::{active_window_row, parse_session_line, parse_workspace_line};
use par_term_emu_core::keyboard::TermKeyEvent;

/// The context menus' footer controls line.
pub(super) const MENU_FOOTER: &str = " action click · close esc/q ";
/// The context menus' overlay title (the target's name rides the panel's
/// accent header row).
pub(super) const MENU_TITLE: &str = " menu ";
/// Compose the context menu's overlay panel for `target`: the target's
/// name as the accent header (the command menu has no target entity, so
/// no header), one row per action, the footer controls line —
/// [`compose_prompt_panel`]'s row/footer shape. Returns the panel rows
/// AND the action per row (`None` for the header and footer) — the
/// click dispatch's pure mapping, so what a click highlights is exactly
/// what painted there.
pub(super) fn compose_menu_panel(
    target: &MenuTarget,
    name: Option<&str>,
) -> (Vec<HelpRow>, Vec<Option<MenuAction>>) {
    let actions: &[MenuAction] = match target {
        MenuTarget::Tab(_) => &[MenuAction::Rename, MenuAction::Close, MenuAction::AddTab],
        MenuTarget::Workspace(_) => &[
            MenuAction::Rename,
            MenuAction::Close,
            MenuAction::NewWorkspace,
        ],
        MenuTarget::Commands => &[
            MenuAction::Settings,
            MenuAction::Keybinds,
            MenuAction::ReloadConfig,
            MenuAction::Detach,
        ],
    };
    let labels: &[&str] = match target {
        MenuTarget::Tab(_) => &["rename", "close", "add tab"],
        MenuTarget::Workspace(_) => &["rename", "close", "new"],
        MenuTarget::Commands => &["settings", "keybinds", "reload config", "detach"],
    };
    let mut rows = Vec::new();
    let mut mapping = Vec::new();
    if let Some(name) = name {
        rows.push(HelpRow {
            text: format!(" {name} "),
            accent: true,
            footer: false,
        });
        mapping.push(None);
    }
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
    /// Open the context menu for `target`: the tab menu (right-press on
    /// a tab), the workspace menu (a right-press on a workspace row), or
    /// the command menu (the panel's ` menu ` chip). The overlay rides
    /// the same modal machinery the picker uses.
    pub(super) fn open_menu(&mut self, target: MenuTarget) {
        let name = match &target {
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
        };
        let (rows, actions) = compose_menu_panel(&target, name.as_deref());
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
            (MenuTarget::Commands, MenuAction::Settings) => self.enter_settings(),
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
                    // refresh reports SessionGone and the view ends cleanly.
                    s.status_dirty = true;
                    return;
                };
                s.land_window(c, &survivor);
            },
        );
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
        let workspace = workspace.to_string();
        match self.status.session_id.clone() {
            Some(session) => {
                self.send_then(
                    conn,
                    &format!("list-sessions -t {workspace}"),
                    move |s, c, listed| {
                        let shown = listed.ok
                            && listed
                                .body
                                .iter()
                                .filter_map(|l| parse_session_line(l))
                                .any(|(sid, _)| sid == session);
                        s.kill_workspace_then_land(c, &workspace, shown);
                    },
                );
            }
            None => self.kill_workspace_then_land(conn, &workspace, false),
        }
    }
    fn kill_workspace_then_land(
        &mut self,
        conn: &mut crate::mux::attach::conn::AttachConn,
        workspace: &str,
        shown: bool,
    ) {
        // In-order daemon: the list-workspaces below observes the kill.
        if conn
            .send_forget(&format!("kill-workspace -t {workspace}"))
            .is_err()
        {
            return;
        }
        if !shown {
            self.status_dirty = true;
            return;
        }
        self.send_then(conn, "list-workspaces", |s, c, reply| {
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
                // No workspace remains: the view ends cleanly through the
                // next status refresh.
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
    }
}
