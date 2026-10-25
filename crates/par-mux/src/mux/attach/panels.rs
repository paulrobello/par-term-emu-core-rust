//! Overlay and sidebar composition shared by passthrough and render
//! modes: key spelling, help rows and filtering, prompt panels, the
//! session/window picker, and the workspace sidebar.

use super::ManagementKey;

/// The tmux spelling of a chord byte: `C-x` for control bytes (0 = the
/// Space spelling), the literal character otherwise.
pub(crate) fn spell_key(byte: u8) -> String {
    match byte {
        0 => "C-Space".to_string(),
        b if (1..27).contains(&b) => format!("C-{}", (b - 1 + b'a') as char),
        other => (other as char).to_string(),
    }
}

/// One help panel row: the text and whether it renders as an accent row
/// (category headers). Data for [`help_rows`]/[`compose_help_panel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpRow {
    pub text: String,
    pub accent: bool,
    /// The modal's controls band: painted on a dark-grey strip across the
    /// panel's inner width (the help/picker/prompt footers).
    pub footer: bool,
    /// Render-mode-only binding: the passthrough help dump filters these
    /// out (the pane's own output owns the screen there — the scroll
    /// viewport and its editor never exist).
    pub render_only: bool,
}

/// The passthrough help dump's byte payload: a leading blank line so the
/// first header never prints on the cursor's current line (it used to land
/// on the prompt row), bold category headers, one CRLF per row.
pub(crate) fn help_dump_text(rows: &[HelpRow]) -> String {
    let mut out = String::from("\r\n");
    for row in rows {
        if row.accent {
            out.push_str(&format!("\x1b[1m{}\x1b[0m", row.text));
        } else {
            out.push_str(row.text.as_str());
        }
        out.push_str("\r\n");
    }
    out
}

/// The bindings help panel's category rows from the LIVE chord state — a
/// remapped chord shows its remapped key. Render mode composes the modal
/// panel over these ([`compose_help_panel`]); passthrough prints the same
/// rows as plain text (the documented shape).
pub(crate) fn help_rows(
    prefix: u8,
    reload: u8,
    m: crate::mux::config::Management,
    resize_step: u32,
) -> Vec<HelpRow> {
    let p = spell_key(prefix);
    let mut rows: Vec<HelpRow> = Vec::new();
    let push_cat = |rows: &mut Vec<HelpRow>, title: &str, entries: Vec<(String, String)>| {
        rows.push(HelpRow {
            text: format!(" {title} "),
            accent: true,
            footer: false,
            render_only: false,
        });
        let width = entries
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0);
        for (key, desc) in entries {
            rows.push(HelpRow {
                text: format!(" {:width$}  {}", key, desc, width = width),
                accent: false,
                footer: false,
                render_only: false,
            });
        }
    };
    push_cat(
        &mut rows,
        "global",
        vec![
            (format!("{p} {p}"), "type a literal prefix".to_string()),
            (format!("{p} d"), "detach".to_string()),
            (format!("{p} {}", spell_key(m.help)), "keybinds".to_string()),
            (
                format!("{p} {}", spell_key(reload)),
                "reload the config".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.palette)),
                "command palette".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.respawn)),
                "respawn the held-dead pane".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.notify_jump)),
                "jump to the last notification's pane".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "workspaces",
        vec![
            (
                format!("{p} {}", spell_key(m.workspace_next)),
                "next workspace".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.workspace_prev)),
                "previous workspace".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.workspace_picker)),
                "workspace picker".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.sidebar)),
                "toggle the workspace side panel".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.status_bar)),
                "toggle the status bar".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "tabs / windows / sessions",
        vec![
            (
                format!("{p} {}", spell_key(m.new_window)),
                "new tab (window)".to_string(),
            ),
            (
                format!("{p} n / p"),
                "next / previous tab (window)".to_string(),
            ),
            (format!("{p} ( / )"), "previous / next session".to_string()),
            (
                format!("{p} {}", spell_key(m.picker)),
                "session/window picker".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.rename_window)),
                "rename the tab (window)".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "panes",
        vec![
            (
                format!("{p} {}", spell_key(m.split_right)),
                "split right".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.split_down)),
                "split down".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.kill_pane)),
                "kill the focused pane".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.break_pane)),
                "break the pane out to a new tab".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.join_pane)),
                "join the pane into a picked tab".to_string(),
            ),
            (format!("{p} o"), "cycle panes".to_string()),
            (
                format!(
                    "{p} {} / {}",
                    spell_key(m.swap_prev),
                    spell_key(m.swap_next)
                ),
                "swap pane prev/next".to_string(),
            ),
            (
                format!(
                    "{p} {} / {}",
                    spell_key(m.move_window_left),
                    spell_key(m.move_window_right)
                ),
                "move tab left/right".to_string(),
            ),
            (
                format!(
                    "{p} {} / {}",
                    spell_key(m.tab_strip_left),
                    spell_key(m.tab_strip_right)
                ),
                "scroll tab strip left/right".to_string(),
            ),
            (
                format!("{p} S-arrows"),
                "swap with the pane in that direction".to_string(),
            ),
            (
                format!("{p} {} arrows", spell_key(m.resize)),
                format!("resize mode, edge moves by {resize_step}"),
            ),
            (
                format!("{p} {}", spell_key(m.zoom)),
                "zoom the focused pane (toggle)".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.border_cycle)),
                "cycle the border style (herdr = per-pane boxes)".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.label_toggle)),
                "toggle pane labels".to_string(),
            ),
            (
                format!("{p} {}", spell_key(m.rename_pane)),
                "rename the focused pane".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "navigation",
        vec![
            (
                "click".to_string(),
                "focus the pane under the pointer".to_string(),
            ),
            (
                format!("{p} arrows"),
                "select the pane in that direction".to_string(),
            ),
            (format!("{p} ["), "scroll the pane's history".to_string()),
            (
                format!("{p} [ then e"),
                "open the scrollback in $VISUAL/$EDITOR".to_string(),
            ),
            (
                "wheel".to_string(),
                "scrollback; forwarded when the pane owns mouse".to_string(),
            ),
        ],
    );
    push_cat(
        &mut rows,
        "mouse",
        vec![
            (
                "click near divider".to_string(),
                "focuses (a bare click still focuses)".to_string(),
            ),
            (
                "drag divider".to_string(),
                "resize the adjacent split".to_string(),
            ),
            (
                "click +".to_string(),
                "new tab — prompts for its name".to_string(),
            ),
            (
                "panel new".to_string(),
                "new workspace — prompts for its name".to_string(),
            ),
            (
                "panel menu".to_string(),
                "settings, keybinds, reload, detach".to_string(),
            ),
        ],
    );
    // The scroll viewport and its editor are render-mode-only bindings:
    // the passthrough dump filters them out (marking here keeps the two
    // spellings beside their entries above).
    for row in &mut rows {
        let key = row.text.trim();
        if key == format!("{p} [") || key == format!("{p} [ then e") {
            row.render_only = true;
        }
    }

    rows
}

/// The help panel's footer/controls line (render mode).
pub(crate) const HELP_FOOTER: &str = " search / · scroll j/k/arrows/pgup/pgdn · close esc/enter ";

/// The modal overlay's title for the bindings panel (the render-mode
/// chrome embeds it in the top border, left-aligned).
pub(crate) const HELP_OVERLAY_TITLE: &str = " keybinds ";

/// The modal overlay's title for the session/window picker.
pub(crate) const PICKER_OVERLAY_TITLE: &str = " picker ";

/// The workspace picker modal's title.
pub(crate) const WORKSPACE_PICKER_OVERLAY_TITLE: &str = " workspaces ";

/// The modal overlay's title for the rename-window prompt.
pub(crate) const PROMPT_WINDOW_OVERLAY_TITLE: &str = " rename window ";

/// The modal overlay's title in the border for the rename-pane prompt.
pub(crate) const PROMPT_PANE_OVERLAY_TITLE: &str = " rename pane ";

/// The rename prompt's footer controls line.
pub(crate) const PROMPT_FOOTER: &str = " enter rename · esc cancel ";

/// The new-tab prompt's footer controls line (herdr's chip vocabulary).
pub(crate) const NEW_PROMPT_FOOTER: &str = " enter save · ^c clear · esc cancel ";

/// The modal overlay's title for the new-window prompt (the tab strip's
/// `+` button).
pub(crate) const PROMPT_NEW_WINDOW_OVERLAY_TITLE: &str = " new tab ";

/// The modal overlay's title for the rename-workspace prompt.
pub(crate) const PROMPT_WORKSPACE_OVERLAY_TITLE: &str = " rename workspace ";

/// The modal overlay's title for the new-workspace prompt (the panel's
/// ` new ` chip).
pub(crate) const PROMPT_NEW_WORKSPACE_OVERLAY_TITLE: &str = " new workspace ";
pub(crate) const PROMPT_SETTING_OVERLAY_TITLE: &str = " setting ";

/// Spell a NAME for the control wire: unconditional single quotes, the
/// embedded-quote `'\''` idiom — the same bounded quoting grammar the
/// daemon's parser (`shell_split`) and `agent_resume::render_argv` use,
/// so a name with spaces or quotes survives as one word.
pub(crate) fn wire_quote(name: &str) -> String {
    format!("'{}'", name.replace('\'', "'\\''"))
}

/// The new-window prompt's editable default: the shown session's next
/// free index — one past the highest window ordinal, bumped past any
/// window NAME that already claims the number (the manual-pass ask: the
/// next non-conflicting index). Pure over the queried windows — the
/// unit-test surface.
pub(crate) fn next_window_name(windows: &[(String, String)]) -> String {
    let used: std::collections::HashSet<&str> =
        windows.iter().map(|(_, name)| name.as_str()).collect();
    let mut candidate = windows
        .iter()
        .filter_map(|(id, _)| id.trim_start_matches('@').parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
    while used.contains(candidate.to_string().as_str()) {
        candidate += 1;
    }
    candidate.to_string()
}

/// The new-workspace prompt's editable default: the roster's next free
/// index — one past the highest workspace ordinal, bumped past any
/// workspace NAME that already claims the number (the same
/// non-conflicting rule [`next_window_name`] runs over the windows).
/// Pure over the queried workspaces — the unit-test surface.
pub(crate) fn next_workspace_name(workspaces: &[(String, String)]) -> String {
    let used: std::collections::HashSet<&str> =
        workspaces.iter().map(|(_, name)| name.as_str()).collect();
    let mut candidate = workspaces
        .iter()
        .filter_map(|(id, _)| id.trim_start_matches('+').parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
    while used.contains(candidate.to_string().as_str()) {
        candidate += 1;
    }
    candidate.to_string()
}

/// The prompt's content rows: the input line (`> text▌`, the ▌ is the
/// insert point — the frame hides the host cursor under the overlay), a
/// spacer, and the footer controls line — `footer` spells the controls
/// (the rename and new-tab prompts differ). Pure over its input — the
/// unit-test surface.
pub(crate) fn compose_prompt_panel(text: &str, footer: &str) -> Vec<HelpRow> {
    vec![
        HelpRow {
            text: format!(" > {text}▌"),
            accent: true,
            footer: false,
            render_only: false,
        },
        HelpRow {
            text: String::new(),
            accent: false,
            footer: false,
            render_only: false,
        },
        HelpRow {
            text: footer.to_string(),
            accent: false,
            footer: true,
            render_only: false,
        },
    ]
}

/// The help panel's content rows after the filter: headers hide when
/// nothing beneath them matches. Shared by the panel composer and the
/// renderer's scroll-state math.
pub(crate) fn help_content(rows: &[HelpRow], filter: &str) -> Vec<HelpRow> {
    let lower = filter.to_lowercase();
    let mut content: Vec<HelpRow> = Vec::new();
    let mut pending_header: Option<HelpRow> = None;
    for row in rows {
        if row.accent {
            pending_header = Some(row.clone());
        } else if lower.is_empty() || row.text.to_lowercase().contains(&lower) {
            if let Some(header) = pending_header.take() {
                content.push(header);
            }
            content.push(row.clone());
        }
    }
    content
}

/// The windowing start the help panel shows: the scroll clamped so the
/// last `visible` rows fill the panel.
pub(crate) fn help_window_start(content_len: usize, visible: usize, scroll: usize) -> usize {
    scroll.min(content_len.saturating_sub(visible))
}

/// Compose the help panel's CONTENT rows: the filter line when a filter
/// is open or set (` /query▌`; the footer already advertises `search /`,
/// so no placeholder when idle — the round-5 report), the filter's
/// matching rows windowed to `visible` rows at `scroll`, and the footer
/// controls line. The renderer's `PaneRenderer::paint_overlay` wraps
/// these in the themed border box (ring, title, badge, background, and
/// the overflow thumb). Pure over its inputs — the unit-test surface for
/// the panel.
pub(crate) fn compose_help_panel(
    rows: &[HelpRow],
    filter: &str,
    filtering: bool,
    visible: usize,
    scroll: usize,
) -> Vec<HelpRow> {
    let _ = filtering; // the cursor glyph rides the filter text below
    let content = help_content(rows, filter);
    let content_len = content.len();
    let start = help_window_start(content_len, visible, scroll);
    let window: Vec<HelpRow> = content[start..(start + visible.min(content_len - start))].to_vec();

    let mut panel: Vec<HelpRow> = Vec::new();
    if filtering || !filter.is_empty() {
        panel.push(HelpRow {
            text: format!(" /{filter}▌"),
            accent: false,
            footer: false,
            render_only: false,
        });
    }
    panel.extend(window);
    panel.push(HelpRow {
        text: HELP_FOOTER.to_string(),
        accent: false,
        footer: true,
        render_only: false,
    });
    panel
}

/// The session/window picker's footer/controls line (render mode).
pub(crate) const PICKER_FOOTER: &str =
    " navigate arrows/j/k · select enter/click · filter / · close esc/q ";

/// The command palette's footer/controls line (render mode): the filter
/// is live from the open, so the footer never advertises `filter /`.
pub(crate) const PALETTE_FOOTER: &str =
    " type to filter · arrows/j/k navigate · enter run · click select · esc close ";

/// One session in the picker's queried state: its windows as
/// `(window_id, name)` in window order, the session's active window id,
/// and whether the session is the one the view currently shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerEntry {
    pub session_id: String,
    pub session_name: String,
    pub windows: Vec<(String, String)>,
    pub active_window: Option<String>,
    pub current: bool,
}

/// Where one picker display row came from: a session (its header row) or
/// one of its windows. The compose returns these parallel to the
/// filtered rows so a selection or a click maps back to a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerRef {
    Session(usize),
    Window(usize, usize),
    /// One workspace row (the workspace picker): the workspace's index
    /// into the picker's queried workspace list.
    Workspace(usize),
    /// One command row (the keyboard palette): the action's index into
    /// [`PALETTE_ACTIONS`].
    Command(usize),
}

/// The keyboard palette's actions (`prefix :`): every management chord
/// plus the chordless paths — settings/keybinds/reload/detach (the
/// table's first four, fixed order) and the tab moves to start/end
/// (the last two, shown-window targets) — type-to-filter from the
/// open (the picker's `/` machinery, with the filter live at the
/// open). The labels show beside their EFFECTIVE chords
/// ([`palette_action_chord`]); the action dispatches through
/// [`crate::mux::attach::ManagementKey`] where one exists.
pub(crate) const PALETTE_ACTIONS: &[(Option<ManagementKey>, &str)] = &[
    (None, "settings"),
    (None, "keybinds"),
    (None, "reload config"),
    (None, "detach"),
    (Some(ManagementKey::NewWindow), "new window"),
    (Some(ManagementKey::BreakPane), "break pane out"),
    (Some(ManagementKey::JoinPane), "join pane into tab"),
    (Some(ManagementKey::SplitRight), "split right"),
    (Some(ManagementKey::SplitDown), "split down"),
    (Some(ManagementKey::KillPane), "kill pane"),
    (Some(ManagementKey::Zoom), "zoom pane"),
    (Some(ManagementKey::RenameWindow), "rename window"),
    (Some(ManagementKey::RenamePane), "rename pane"),
    (Some(ManagementKey::WorkspacePicker), "workspace picker"),
    (Some(ManagementKey::Sidebar), "toggle sidebar"),
    (Some(ManagementKey::StatusBar), "toggle status bar"),
    (Some(ManagementKey::WorkspaceNext), "next workspace"),
    (Some(ManagementKey::WorkspacePrev), "previous workspace"),
    (Some(ManagementKey::NotifyJump), "jump to notification"),
    (Some(ManagementKey::MoveWindowLeft), "move tab left"),
    (Some(ManagementKey::MoveWindowRight), "move tab right"),
    (None, "move tab to start"),
    (None, "move tab to end"),
];

/// A palette action's chord at its EFFECTIVE binding, the help panel's
/// spelling (`C-b x`); `None` for the actions with no chord (settings)
/// or an index past the table. `reload` is the reload chord's key byte
/// (it lives outside [`crate::mux::config::Management`]).
pub(crate) fn palette_action_chord(
    action: usize,
    prefix: u8,
    reload: u8,
    m: &crate::mux::config::Management,
) -> Option<String> {
    let key = match action {
        // The click-only paths the table carries with `None` keys:
        // keybinds opens at the help chord, reload at the reload chord,
        // detach at its fixed prefix chord.
        1 => m.help,
        2 => reload,
        3 => b'd',
        _ => match PALETTE_ACTIONS.get(action)?.0? {
            ManagementKey::SplitRight => m.split_right,
            ManagementKey::SplitDown => m.split_down,
            ManagementKey::KillPane => m.kill_pane,
            ManagementKey::BreakPane => m.break_pane,
            ManagementKey::JoinPane => m.join_pane,
            ManagementKey::NewWindow => m.new_window,
            ManagementKey::Zoom => m.zoom,
            ManagementKey::RenameWindow => m.rename_window,
            ManagementKey::RenamePane => m.rename_pane,
            ManagementKey::WorkspacePicker => m.workspace_picker,
            ManagementKey::Sidebar => m.sidebar,
            ManagementKey::StatusBar => m.status_bar,
            ManagementKey::WorkspaceNext => m.workspace_next,
            ManagementKey::WorkspacePrev => m.workspace_prev,
            ManagementKey::NotifyJump => m.notify_jump,
            ManagementKey::MoveWindowLeft => m.move_window_left,
            ManagementKey::MoveWindowRight => m.move_window_right,
            _ => return None,
        },
    };
    Some(format!("{} {}", spell_key(prefix), spell_key(key)))
}

/// The picker's display rows from the queried entries: one accent
/// header per session (` $N: name`, `>`-marked when the session is the
/// one the view shows), its windows nested beneath it as `   @N: name`
/// rows (`>`-marked for the shown window, `*` suffixed for the
/// session's active window). The parallel refs list maps each row to
/// its selection target.
pub(crate) fn picker_rows(
    entries: &[PickerEntry],
    current_window: Option<&str>,
) -> (Vec<HelpRow>, Vec<PickerRef>) {
    let mut rows: Vec<HelpRow> = Vec::new();
    let mut refs: Vec<PickerRef> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let marker = if entry.current { ">" } else { " " };
        rows.push(HelpRow {
            text: format!(
                " {marker}{0}: {1}",
                entry.session_id,
                super::chrome_safe(&entry.session_name)
            ),
            accent: true,
            footer: false,
            render_only: false,
        });
        refs.push(PickerRef::Session(i));
        for (w, (window_id, name)) in entry.windows.iter().enumerate() {
            let marker = if current_window == Some(window_id.as_str()) {
                ">"
            } else {
                " "
            };
            let active = entry.active_window.as_deref() == Some(window_id.as_str());
            let star = if active { " *" } else { "" };
            rows.push(HelpRow {
                text: format!(" {marker}  {window_id}: {}{star}", super::chrome_safe(name)),
                accent: false,
                footer: false,
                render_only: false,
            });
            refs.push(PickerRef::Window(i, w));
        }
    }
    (rows, refs)
}

/// One row of an edit's filtering: `true` keeps the row.
pub(super) fn picker_row_matches(row: &HelpRow, lower: &str) -> bool {
    lower.is_empty() || row.text.to_lowercase().contains(lower)
}

/// The listbox panning rule the picker's content window uses: `start`
/// moves only when `selected` leaves the visible window `[start,
/// start+visible)`. Pure — the session keeps the running `start` and
/// passes it back each compose.
pub(crate) fn listbox_scroll(start: usize, selected: usize, visible: usize) -> usize {
    if visible == 0 {
        return 0;
    }
    if selected < start {
        selected
    } else if selected >= start + visible {
        selected + 1 - visible
    } else {
        start
    }
}

/// Compose the picker panel's CONTENT rows: the always-visible filter
/// line (the same shape [`compose_help_panel`] draws — placeholder when
/// inactive, ` /query▌` while typing), the filter's matching rows with
/// headers hiding when nothing beneath them matches, the selection
/// cursor `▸` prefixed to the selected row (clamped into range), the
/// content windowed to `visible` rows at the panned `start`, and the
/// footer controls line. Returns the panel rows, the FILTERED refs (the
/// selection/click target for each content row, in content order), and
/// the window's content start index, so a click at composed row `i`
/// maps to content row `start + i - 1`. `footer` is the modal's
/// controls line (the session/workspace pickers and the palette spell
/// different ones). Pure over its inputs — the unit-test surface for
/// the picker.
#[allow(clippy::too_many_arguments)] // the pure compose's positional test surface
pub(crate) fn compose_picker_panel(
    rows: &[HelpRow],
    refs: &[PickerRef],
    filter: &str,
    filtering: bool,
    selected: usize,
    visible: usize,
    start: usize,
    footer: &str,
) -> (Vec<HelpRow>, Vec<PickerRef>, usize) {
    let lower = filter.to_lowercase();
    let mut content: Vec<HelpRow> = Vec::new();
    let mut content_refs: Vec<PickerRef> = Vec::new();
    let mut pending: Option<(HelpRow, PickerRef)> = None;
    for (row, r#ref) in rows.iter().zip(refs.iter()) {
        if row.accent {
            pending = Some((row.clone(), *r#ref));
        } else if picker_row_matches(row, &lower) {
            if let Some((header, header_ref)) = pending.take() {
                content.push(header);
                content_refs.push(header_ref);
            }
            content.push(row.clone());
            content_refs.push(*r#ref);
        }
    }
    // The selection cursor clamps into the filtered range (a narrowed
    // filter can shrink the list under the cursor).
    let selected = selected.min(content.len().saturating_sub(1));
    let start = start.min(selected);
    let start = listbox_scroll(start, selected, visible);
    for (i, row) in content.iter_mut().enumerate() {
        if i == selected {
            row.text = format!("▸{}", row.text);
        }
    }
    let content_len = content.len();
    let end = (start + visible).min(content_len);
    let window: Vec<HelpRow> = content[start..end].to_vec();

    let mut panel: Vec<HelpRow> = Vec::new();
    if filtering || !filter.is_empty() {
        panel.push(HelpRow {
            text: format!(" /{filter}▌"),
            accent: false,
            footer: false,
            render_only: false,
        });
    }
    panel.extend(window);
    panel.push(HelpRow {
        text: footer.to_string(),
        accent: false,
        footer: true,
        render_only: false,
    });
    (panel, content_refs, start)
}

/// One section of the workspace side panel: the entries top-down. The
/// panel renders sections top-down — workspaces first; more sections
/// slot in after. The section title moved to the tab strip's lead
/// segment (round 6), so the section carries only its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SidebarSection {
    /// The area's identity: `workspaces` selects the workspace area
    /// (whose title the tab strip's lead segment paints), `agents` the
    /// agent area (whose header row the panel paints) — herdr's
    /// ` spaces` lead row.
    pub title: &'static str,
    pub rows: Vec<SidebarRow>,
}

/// What a side-panel row activates on click (ARC-140).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SidebarRowId {
    /// A workspace row: its id, `+N`.
    Workspace(String),
    /// An agent row: the agent's pane.
    Pane(u32),
    /// The footer's ` new ` chip: opens the new-workspace prompt.
    New,
    /// The footer's ` menu ` chip: opens the command menu (keybinds,
    /// reload config, detach).
    Menu,
    /// The bottom-right chevron: collapses the panel to the status rail
    /// (or expands the rail back).
    Collapse,
    /// The agents header's sort toggle (herdr's `agent_sort_toggle`):
    /// the right-aligned `grouped`/`priority` label, a click flipping
    /// the roster's ordering.
    AgentsSort,
}

/// One side-panel entry: its click target, first-line label, any
/// continuation lines (a multi-line agent entry, config `agent-rows`),
/// active flag, the status glyph the collapsed rail paints for it, and
/// whether its first continuation line is the git badge (painted blue,
/// not dim).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SidebarRow {
    pub id: SidebarRowId,
    pub label: String,
    pub active: bool,
    pub status: AgentStatus,
    pub continuation: Vec<String>,
    pub badge: bool,
}

/// One agent's facts the entry tokens render from.
pub(crate) struct AgentFacts<'a> {
    pub pane: u32,
    pub agent: &'a str,
    pub state: &'a str,
    pub workspace: Option<&'a str>,
    pub project: Option<&'a str>,
    /// The state's message (the blocked reason); the `message` token
    /// renders it, and drops out when absent.
    pub message: Option<&'a str>,
}

/// The status-glyph placeholder an `agent-rows` line leads with: the
/// panel paints the entry's status glyph in its color there.
pub(crate) const STATE_ICON: char = '\u{25CF}';

/// Render an agent entry's lines from the `agent-rows` token lines
/// (herdr's `[ui.sidebar.agents] rows`): each line joins its tokens'
/// texts with ` · `, a `state_icon` with one space; a token with nothing
/// to show (an unprobed project, an unknown workspace) drops out, and a
/// line left empty drops entirely. Never returns no lines — an entry
/// whose every line is empty falls back to the agent name. Pure — the
/// unit-test surface.
pub(crate) fn agent_entry_lines(rows: &[Vec<String>], facts: &AgentFacts<'_>) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for tokens in rows {
        let mut line = String::new();
        let mut after_icon = false;
        for token in tokens {
            let text = match token.as_str() {
                "state_icon" => Some(STATE_ICON.to_string()),
                "state" => Some(facts.state.to_string()),
                "workspace" => facts.workspace.map(str::to_string),
                "project" => facts.project.map(str::to_string),
                "message" => facts.message.filter(|m| !m.is_empty()).map(str::to_string),
                "agent" => Some(facts.agent.to_string()),
                "pane" => Some(format!("%{}", facts.pane)),
                _ => None,
            };
            let Some(text) = text.filter(|t| !t.is_empty()) else {
                continue;
            };
            if !line.is_empty() {
                line.push_str(if after_icon { " " } else { " \u{b7} " });
            }
            after_icon = token == "state_icon";
            line.push_str(&text);
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        lines.push(facts.agent.to_string());
    }
    lines
}

/// An agent's state as the side panel shows it (herdr's status dot): a
/// workspace row carries its agents' most urgent state. Priority is
/// declaration order: `question` (a dialog asking the user to choose)
/// outranks everything, then `blocked`, `done` (finished while out of
/// view), `working`, `idle`, none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum AgentStatus {
    /// No agent, or a state the panel does not know.
    #[default]
    None,
    Idle,
    Working,
    /// Finished while out of view (herdr's priority: above working, below
    /// blocked).
    Done,
    Blocked,
    /// A dialog asks the user to choose (the scrape tier's `question`
    /// rules); the magenta glyph is the roster's "you are wanted" flag.
    Question,
}

impl AgentStatus {
    /// The roster's state word (`list-agents`' third column).
    pub(crate) fn from_state(state: &str) -> Self {
        match state {
            "idle" => Self::Idle,
            "working" => Self::Working,
            "done" => Self::Done,
            "blocked" => Self::Blocked,
            "question" => Self::Question,
            _ => Self::None,
        }
    }

    /// The rail glyph: herdr's dots with the working half-circle
    /// (`●` busy or unseen, `◐` working, `○` idle, `·` nothing).
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::None => "·",
            Self::Idle => "○",
            Self::Working => "◐",
            Self::Done | Self::Blocked | Self::Question => "●",
        }
    }
}

/// The side panel's default area split: the workspace area takes this
/// share of the panel's rows (herdr's 0.5).
pub(crate) const SIDEBAR_SPLIT_DEFAULT: f32 = 0.5;

/// The collapsed panel's width: an index column and a status column.
pub(crate) const SIDEBAR_RAIL_WIDTH: u16 = 2;

/// The expanded panel's chevron (collapse) and the rail's (expand).
pub(crate) const CHEVRON_COLLAPSE: &str = "«";
pub(crate) const CHEVRON_EXPAND: &str = "»";

/// The agents area's ordering (herdr's `agent_panel_sort`, config
/// `agents-sort`). `Grouped` is herdr's spaces view: workspace groups in
/// daemon id order, agents beneath by status priority. `Priority` is
/// herdr's attention queue: status first, most urgent at the top,
/// workspace beneath.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AgentSortMode {
    #[default]
    Grouped,
    Priority,
}

impl AgentSortMode {
    /// The header toggle's label — the mode's own name, herdr's
    /// `grouped` / `priority`.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Grouped => "grouped",
            Self::Priority => "priority",
        }
    }

    /// The config word (`[client] agents-sort`) — the same vocabulary
    /// the settings dialog's row writes.
    pub(crate) fn config_value(self) -> &'static str {
        self.label()
    }

    /// The flipped mode — the header toggle's click.
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Grouped => Self::Priority,
            Self::Priority => Self::Grouped,
        }
    }
}

/// The workspace area's row count for a panel `height` rows tall at
/// `split` — herdr's `sidebar_section_heights`: each area keeps at
/// least 3 rows (a header, one entry, its last row), a short panel
/// halves.
pub(crate) fn sidebar_workspace_rows(height: u16, split: f32) -> u16 {
    if height < 6 {
        return height.div_ceil(2);
    }
    let rows = (f32::from(height) * split.clamp(0.1, 0.9)).round() as u16;
    rows.clamp(3, height - 3)
}

/// One composed sidebar line: buffer-row-relative `y`, the clickable
/// column span `[span.0, span.1)` within the strip (body rows span the
/// content width, the footer chips exactly their cells), the text,
/// whether it paints from a non-zero column, and the id a click
/// activates (`None` for filler).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SidebarLine {
    pub y: u16,
    /// The text's leftmost column (0 for body rows, the chips' cells
    /// for the footer row).
    pub x: u16,
    /// The clickable columns `[x, x_end)` — the click hit-test's span.
    pub x_end: u16,
    pub text: String,
    pub id: Option<SidebarRowId>,
    /// The active entry: painted as herdr's full-width inverted block.
    pub active: bool,
    /// The row's kind: body rows are clickable; a header paints dim +
    /// bold (herdr's ` spaces` lead); the rule is the draggable
    /// area-divider.
    pub kind: SidebarLineKind,
}

/// One composed side-panel row's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarLineKind {
    /// A workspace/agent entry row (an entry's first line).
    Row,
    /// A multi-line entry's later line: it clicks and highlights with
    /// its entry but stays dim, like herdr's secondary lines.
    Continuation,
    /// The git badge continuation line (` main ↑2`): clickable like a
    /// continuation but painted blue instead of dim.
    Badge,
    /// An area header: ` agents` (the workspace area carries no
    /// header — the tab strip's lead segment labels it).
    Header,
    /// The full-width rule dividing the areas — the split-drag handle.
    Rule,
}

/// Compose the side panel's lines from its sections, herdr's separated
/// panel shape split by `split` (the workspace area's share of the rows,
/// [`sidebar_workspace_rows`]): the workspace area opens directly with
/// its rows (the tab strip's lead segment carries the panel title — no
/// in-panel header) and ends with the footer chips on its LAST row —
/// ` new ` bottom-left, ` menu ` bottom-right, each its own clickable
/// span. A full-width rule (herdr's `─`.repeat(width) section divider)
/// opens the agents area, then its header (` agents`) and rows; the
/// panel's last row carries the collapse chevron `«` at the right edge.
/// Every text is clipped to `width - 1` columns (the divider column
/// owns the strip's right edge). Pure — the unit-test surface.
pub(crate) fn compose_sidebar(
    sections: &[SidebarSection],
    width: u16,
    height: u16,
    split: f32,
    agents_sort: AgentSortMode,
) -> Vec<SidebarLine> {
    let mut lines: Vec<SidebarLine> = Vec::new();
    if height == 0 {
        return lines;
    }
    let text_width = usize::from(width.saturating_sub(1));
    let content_end = width.saturating_sub(1);
    let ws_rows = sidebar_workspace_rows(height, split);
    let filler = |lines: &mut Vec<SidebarLine>, y: u16, text: String, kind: SidebarLineKind| {
        lines.push(SidebarLine {
            y,
            x: 0,
            x_end: content_end,
            text,
            id: None,
            active: false,
            kind,
        });
    };
    let clip = |text: String| -> String { text.chars().take(text_width).collect() };
    // The entries of one area written from `top` while below `end`:
    // each entry's first line carries the status glyph (a workspace
    // row's agents' most urgent state) in the marker column; an agent
    // row keeps the `▸` cursor when active (its state already leads the
    // entry text as the state icon), and its continuation lines indent
    // under the label. Every line keeps the entry's click target and
    // active block. An entry that does not fit whole still shows the
    // lines that do.
    let entries = |lines: &mut Vec<SidebarLine>, rows: &[SidebarRow], top: u16, end: u16| {
        let mut y = top;
        for row in rows {
            let marker = if row.active { "▸" } else { " " };
            // A workspace's status glyph rides NEXT TO THE NAME on the
            // first line, as the STATE_ICON placeholder the renderer
            // paints in the status's color — not in the marker column,
            // which stays the active pointer (and never on the
            // continuation lines). A workspace with no agent state
            // carries no icon at all.
            let status_dot = match (&row.id, row.status) {
                (SidebarRowId::Workspace(_), status) if status != AgentStatus::None => {
                    format!(" {STATE_ICON}")
                }
                _ => String::new(),
            };
            let first = format!(" {marker} {}{status_dot}", super::chrome_safe(&row.label));
            let rest = row
                .continuation
                .iter()
                .map(|line| format!("   {}", super::chrome_safe(line)));
            for (i, text) in std::iter::once(first).chain(rest).enumerate() {
                if y >= end {
                    return;
                }
                lines.push(SidebarLine {
                    y,
                    x: 0,
                    x_end: content_end,
                    text: clip(text),
                    id: Some(row.id.clone()),
                    active: row.active,
                    kind: if i == 0 {
                        SidebarLineKind::Row
                    } else if row.badge && i == 1 {
                        SidebarLineKind::Badge
                    } else {
                        SidebarLineKind::Continuation
                    },
                });
                y += 1;
            }
        }
    };
    let workspaces = sections.iter().find(|s| s.title == "workspaces");
    let agents = sections.iter().find(|s| s.title == "agents");

    // The workspace area: rows from the top (the tab strip's lead
    // segment names the panel), footer chips on its last row.
    let footer_y = ws_rows.saturating_sub(1);
    if let Some(section) = workspaces {
        entries(&mut lines, &section.rows, 0, footer_y);
    }
    let new_chip = " new ";
    let menu_chip = " menu ";
    if footer_y > 0 && text_width >= new_chip.len() {
        lines.push(SidebarLine {
            y: footer_y,
            x: 0,
            x_end: new_chip.len() as u16,
            text: new_chip.to_string(),
            id: Some(SidebarRowId::New),
            active: false,
            kind: SidebarLineKind::Row,
        });
        if text_width >= new_chip.len() + menu_chip.len() {
            let mx = text_width - menu_chip.len();
            lines.push(SidebarLine {
                y: footer_y,
                x: mx as u16,
                x_end: text_width as u16,
                text: menu_chip.to_string(),
                id: Some(SidebarRowId::Menu),
                active: false,
                kind: SidebarLineKind::Row,
            });
        }
    }

    // The agents area: the rule (the split-drag handle), its header,
    // then rows above the panel's last row (the chevron's).
    if ws_rows < height {
        filler(
            &mut lines,
            ws_rows,
            "─".repeat(text_width),
            SidebarLineKind::Rule,
        );
    }
    if ws_rows + 1 < height {
        filler(
            &mut lines,
            ws_rows + 1,
            clip(" agents".to_string()),
            SidebarLineKind::Header,
        );
        // The sort toggle rides the header's right edge (herdr's
        // `agent_sort_toggle`): the label names the CURRENT mode, the
        // click flips it. Painted after the header filler so it wins
        // the right-edge cells; ` new `/` menu ` use the same
        // same-row-two-lines shape.
        let label = agents_sort.label();
        if text_width >= label.len() {
            lines.push(SidebarLine {
                y: ws_rows + 1,
                x: (text_width - label.len()) as u16,
                x_end: content_end,
                text: label.to_string(),
                id: Some(SidebarRowId::AgentsSort),
                active: false,
                kind: SidebarLineKind::Row,
            });
        }
    }
    let last = height - 1;
    if let Some(section) = agents {
        entries(&mut lines, &section.rows, ws_rows + 2, last);
    }
    if text_width >= 2 && last > ws_rows {
        let cx = content_end - 1;
        lines.push(SidebarLine {
            y: last,
            x: cx,
            x_end: content_end,
            text: CHEVRON_COLLAPSE.to_string(),
            id: Some(SidebarRowId::Collapse),
            active: false,
            kind: SidebarLineKind::Row,
        });
    }
    lines
}

/// Compose the collapsed panel — herdr's status rail — `height` rows of
/// a [`SIDEBAR_RAIL_WIDTH`]-column strip: per workspace its 1-based
/// index (a digit; `+` past 9) and its agents' most urgent status glyph,
/// the same split's rule, per agent its index and glyph, and the expand
/// chevron `»` on the last row. Every entry stays clickable (land on the
/// workspace, focus the agent's pane). Pure — the unit-test surface.
pub(crate) fn compose_sidebar_rail(
    sections: &[SidebarSection],
    height: u16,
    split: f32,
) -> Vec<SidebarLine> {
    let mut lines: Vec<SidebarLine> = Vec::new();
    if height == 0 {
        return lines;
    }
    let ws_rows = sidebar_workspace_rows(height, split);
    let last = height - 1;
    let index = |i: usize| {
        if i < 9 {
            char::from(b'1' + i as u8)
        } else {
            '+'
        }
    };
    let entries = |lines: &mut Vec<SidebarLine>, rows: &[SidebarRow], top: u16, end: u16| {
        for (i, (y, row)) in (top..end).zip(rows).enumerate() {
            lines.push(SidebarLine {
                y,
                x: 0,
                x_end: SIDEBAR_RAIL_WIDTH,
                text: format!("{}{}", index(i), row.status.glyph()),
                id: Some(row.id.clone()),
                active: row.active,
                kind: SidebarLineKind::Row,
            });
        }
    };
    if let Some(section) = sections.iter().find(|s| s.title == "workspaces") {
        entries(&mut lines, &section.rows, 0, ws_rows.min(last));
    }
    if ws_rows < last {
        lines.push(SidebarLine {
            y: ws_rows,
            x: 0,
            x_end: SIDEBAR_RAIL_WIDTH,
            text: "──".to_string(),
            id: None,
            active: false,
            kind: SidebarLineKind::Rule,
        });
        if let Some(section) = sections.iter().find(|s| s.title == "agents") {
            entries(&mut lines, &section.rows, ws_rows + 1, last);
        }
    }
    lines.push(SidebarLine {
        y: last,
        x: 0,
        x_end: SIDEBAR_RAIL_WIDTH,
        text: format!(" {CHEVRON_EXPAND}"),
        id: Some(SidebarRowId::Collapse),
        active: false,
        kind: SidebarLineKind::Row,
    });
    lines
}
