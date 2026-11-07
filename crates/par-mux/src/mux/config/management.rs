//! The window/pane management chord table: the keys matched after the
//! prefix that act on the focused pane or window.

use super::parse_prefix;

/// The window/pane management chords: the keys matched after the prefix
/// that act on the focused pane/window. Each defaults to its tmux
/// spelling (`%`, `"`, `x`, `c`); the resize mode's entry key defaults to
/// `R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Management {
    /// Split the focused pane right — `split-window -t <pane> -h`.
    pub split_right: u8,
    /// Split the focused pane down — `split-window -t <pane>`.
    pub split_down: u8,
    /// Kill the focused pane — `kill-pane -t <pane>`.
    pub kill_pane: u8,
    /// Break the focused pane out into a new window of its session —
    /// `break-pane -s <pane>` — and land on the fresh window.
    pub break_pane: u8,
    /// Open the window picker in join mode: the picked window receives
    /// the focused pane (`join-pane -s <pane> -t <pane>`).
    pub join_pane: u8,
    /// Open the command palette: every management action (and
    /// settings/keybinds/reload/detach) as one filterable list.
    pub palette: u8,
    /// New window in the focused pane's session — `new-window -t <session>`.
    pub new_window: u8,
    /// Enter the sticky resize mode — arrows adjust the focused pane's
    /// edges by `Chords::resize_step` per press until Enter/Escape/`q`.
    pub resize: u8,
    /// Swap the focused pane with the previous pane in layout order —
    /// `swap-pane -s <focused> -t <prev>`.
    pub swap_prev: u8,
    /// Swap the focused pane with the next pane in layout order —
    /// `swap-pane -s <focused> -t <next>`.
    pub swap_next: u8,
    /// Move the shown window one tab left in its session's order —
    /// `swap-window -s <shown> -t <prev>` (tmux's `<`).
    pub move_window_left: u8,
    /// Move the shown window one tab right in its session's order —
    /// `swap-window -s <shown> -t <next>` (tmux's `>`).
    pub move_window_right: u8,
    /// Scroll the overflowing tab strip's visible run one wheel-notch
    /// left — toward the earlier windows — without switching (render
    /// mode).
    pub tab_strip_left: u8,
    /// Scroll the overflowing tab strip's visible run one wheel-notch
    /// right — toward the later windows — without switching (render
    /// mode).
    pub tab_strip_right: u8,
    /// Select the next workspace in id order — `select-workspace -t +N`
    /// — and land the view on the workspace's session. Prefix `W` by
    /// default (prefix `w` is reserved for the workspace picker).
    pub workspace_next: u8,
    /// Select the previous workspace in id order —
    /// `select-workspace -t +N` — and land the view on the workspace's
    /// session. Prefix `C-w` by default.
    pub workspace_prev: u8,
    /// Open the bindings help panel — every chord at its effective
    /// binding, from the live config.
    pub help: u8,
    /// Open the window picker modal — every session's windows as one
    /// flat list, the shown one emphasized, keyboard and mouse
    /// navigable.
    pub picker: u8,
    /// Toggle the focused pane between its layout rect and the full
    /// window — tmux's zoom.
    pub zoom: u8,
    /// Open the rename prompt for the shown window (its `name`).
    pub rename_window: u8,
    /// Open the rename prompt for the focused pane's user title.
    pub rename_pane: u8,
    /// Cycle the divider/border glyph set.
    pub border_cycle: u8,
    /// Toggle pane titles embedded in the pane borders (render mode).
    pub label_toggle: u8,
    /// Open the workspace picker modal — the window picker's
    /// workspace sibling: the daemon's workspace roster, the current one
    /// marked, keyboard and mouse navigable, Enter lands on it.
    pub workspace_picker: u8,
    /// Toggle the workspace side panel (render mode): a left strip with
    /// the workspace list, click-to-land; more sections will follow.
    pub sidebar: u8,
    /// Toggle the bottom status bar (render mode): shown by default;
    /// hiding it hands the row back to the pane grid.
    pub status_bar: u8,
    /// Respawn the focused pane when it is held dead — `respawn-pane -t
    /// <pane>`. A live pane is left alone.
    pub respawn: u8,
    /// Focus the pane behind the most recent agent notification this
    /// client delivered (herdr's `open_notification_target`; render mode).
    pub notify_jump: u8,
}

impl Default for Management {
    fn default() -> Self {
        Self {
            split_right: b'%',
            split_down: b'"',
            kill_pane: b'x',
            break_pane: b'!',
            join_pane: b'J',
            palette: b':',
            new_window: b'c',
            resize: b'R',
            swap_prev: b'{',
            swap_next: b'}',
            move_window_left: b'<',
            move_window_right: b'>',
            tab_strip_left: b'H',
            tab_strip_right: b'L',
            workspace_next: b'W',
            workspace_prev: 0x17, // C-w
            help: b'?',
            picker: b'w',
            zoom: b'z',
            rename_window: b',',
            rename_pane: b'$',
            border_cycle: b'B',
            label_toggle: b'l',
            workspace_picker: b'g',
            sidebar: b's',
            status_bar: b'S',
            respawn: b'r',
            notify_jump: b'j',
        }
    }
}

/// One after-prefix management key: a single token parsed with the shared
/// prefix parser. A management chord is ONE key matched by byte after the
/// prefix (the reload chord's multi-token spelling exists for
/// documentation only; a management chord keeps the single-key shape).
/// Public for the settings dialog's chord validation, which must accept
/// exactly what a reload would.
pub fn management_key(chord: &str, name: &str) -> Result<u8, String> {
    let token = chord
        .split_whitespace()
        .last()
        .ok_or_else(|| format!("[client] {name} chord is empty"))?;
    for token in chord.split_whitespace() {
        if parse_prefix(token).is_none() {
            return Err(format!(
                "[client] {name} chord {token:?} does not parse (expected the tmux spelling, e.g. C-b or a single key)"
            ));
        }
    }
    parse_prefix(token).ok_or_else(|| {
        format!(
            "[client] {name} chord {token:?} does not parse (expected the tmux spelling, e.g. C-b or a single key)"
        )
    })
}
