//! Overlay and sidebar composition shared by passthrough and render
//! modes: key spelling, help rows and filtering, prompt panels, the
//! session/window picker, and the workspace sidebar.

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
            (format!("{p} r"), "respawn the held-dead pane".to_string()),
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
                "keybinds / reload config / detach".to_string(),
            ),
        ],
    );
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
        },
        HelpRow {
            text: String::new(),
            accent: false,
            footer: false,
        },
        HelpRow {
            text: footer.to_string(),
            accent: false,
            footer: true,
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
        });
    }
    panel.extend(window);
    panel.push(HelpRow {
        text: HELP_FOOTER.to_string(),
        accent: false,
        footer: true,
    });
    panel
}

/// The session/window picker's footer/controls line (render mode).
pub(crate) const PICKER_FOOTER: &str =
    " navigate arrows/j/k · select enter/click · filter / · close esc/q ";

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
            text: format!(" {marker}{0}: {1}", entry.session_id, entry.session_name),
            accent: true,
            footer: false,
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
                text: format!(" {marker}  {window_id}: {name}{star}"),
                accent: false,
                footer: false,
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
/// maps to content row `start + i - 1`. Pure over its inputs — the
/// unit-test surface for the picker.
pub(crate) fn compose_picker_panel(
    rows: &[HelpRow],
    refs: &[PickerRef],
    filter: &str,
    filtering: bool,
    selected: usize,
    visible: usize,
    start: usize,
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
        });
    }
    panel.extend(window);
    panel.push(HelpRow {
        text: PICKER_FOOTER.to_string(),
        accent: false,
        footer: true,
    });
    (panel, content_refs, start)
}

/// One section of the workspace side panel: the entries top-down. The
/// panel renders sections top-down — workspaces first; more sections
/// slot in after. The section title moved to the tab strip's lead
/// segment (round 6), so the section carries only its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SidebarSection {
    /// `(id, label, active)` per row; `id` is what a click lands on.
    pub rows: Vec<(String, String, bool)>,
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
    pub id: Option<String>,
    /// The active entry: painted as herdr's full-width inverted block.
    pub active: bool,
}

/// The side panel's footer chips: ` new ` opens the new-workspace
/// prompt, ` menu ` the command menu (keybinds, reload config, detach).
/// The ids flow through
/// [`SidebarLine.id`] into the click dispatch.
pub(crate) const SIDEBAR_NEW_ID: &str = "panel:new";

pub(crate) const SIDEBAR_MENU_ID: &str = "panel:menu";

/// Compose the side panel's lines from its sections: the rows top-down
/// (NO section header rows — the title lives in the tab strip's lead
/// segment since round 6, so workspace rows start at composed row 0 =
/// host row 1), clipped to `width - 1` columns (the divider column
/// owns the strip's right edge), plus a footer row pinned to the
/// panel's LAST row: ` new ` bottom-left and ` menu ` bottom-right
/// (the mock's panel shape; each chip its own clickable span). Pure —
/// the unit-test surface.
pub(crate) fn compose_sidebar(
    sections: &[SidebarSection],
    width: u16,
    height: u16,
) -> Vec<SidebarLine> {
    let mut lines: Vec<SidebarLine> = Vec::new();
    let max = usize::from(height);
    if max == 0 {
        return lines;
    }
    let text_width = usize::from(width.saturating_sub(1));
    // Body rows fill everything above the footer row.
    let body_cap = max - 1;
    for section in sections {
        for (id, label, active) in &section.rows {
            if lines.len() >= body_cap {
                break;
            }
            let marker = if *active { "▸" } else { " " };
            let text: String = format!(" {marker} {label}")
                .chars()
                .take(text_width)
                .collect();
            lines.push(SidebarLine {
                y: lines.len() as u16,
                x: 0,
                x_end: width.saturating_sub(1),
                text,
                id: Some(id.clone()),
                active: *active,
            });
        }
    }
    // The footer chips pin to the panel's last row; a strip too narrow
    // for a chip drops it.
    let footer_y = (max - 1) as u16;
    let new_chip = " new ";
    if text_width >= new_chip.len() {
        lines.push(SidebarLine {
            y: footer_y,
            x: 0,
            x_end: new_chip.len() as u16,
            text: new_chip.to_string(),
            id: Some(SIDEBAR_NEW_ID.to_string()),
            active: false,
        });
        let menu_chip = " menu ";
        if text_width >= new_chip.len() + menu_chip.len() {
            let mx = text_width - menu_chip.len();
            lines.push(SidebarLine {
                y: footer_y,
                x: mx as u16,
                x_end: text_width as u16,
                text: menu_chip.to_string(),
                id: Some(SIDEBAR_MENU_ID.to_string()),
                active: false,
            });
        }
    }
    lines
}
