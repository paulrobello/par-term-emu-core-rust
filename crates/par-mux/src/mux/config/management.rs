//! The window/pane management chord table: the keys matched after the
//! prefix that act on the focused pane or window.

use super::{parse_chord, ChordKey};

/// The window/pane management chords: the keys matched after the prefix
/// that act on the focused pane/window. Each defaults to its tmux
/// spelling (`%`, `"`, `x`, `c`); the resize mode's entry key defaults to
/// `R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Management {
    /// Split the focused pane right — `split-window -t <pane> -h`.
    pub split_right: ChordKey,
    /// Split the focused pane down — `split-window -t <pane>`.
    pub split_down: ChordKey,
    /// Kill the focused pane — `kill-pane -t <pane>`.
    pub kill_pane: ChordKey,
    /// Break the focused pane out into a new window of its session —
    /// `break-pane -s <pane>` — and land on the fresh window.
    pub break_pane: ChordKey,
    /// Open the window picker in join mode: the picked window receives
    /// the focused pane (`join-pane -s <pane> -t <pane>`).
    pub join_pane: ChordKey,
    /// Open the command palette: every management action (and
    /// settings/keybinds/reload/detach) as one filterable list.
    pub palette: ChordKey,
    /// New window in the focused pane's session — `new-window -t <session>`.
    pub new_window: ChordKey,
    /// Enter the sticky resize mode — arrows adjust the focused pane's
    /// edges by `Chords::resize_step` per press until Enter/Escape/`q`.
    pub resize: ChordKey,
    /// Swap the focused pane with the previous pane in layout order —
    /// `swap-pane -s <focused> -t <prev>`.
    pub swap_prev: ChordKey,
    /// Swap the focused pane with the next pane in layout order —
    /// `swap-pane -s <focused> -t <next>`.
    pub swap_next: ChordKey,
    /// Move the shown window one tab left in its session's order —
    /// `swap-window -s <shown> -t <prev>` (tmux's `<`).
    pub move_window_left: ChordKey,
    /// Move the shown window one tab right in its session's order —
    /// `swap-window -s <shown> -t <next>` (tmux's `>`).
    pub move_window_right: ChordKey,
    /// Scroll the overflowing tab strip's visible run one wheel-notch
    /// left — toward the earlier windows — without switching (render
    /// mode).
    pub tab_strip_left: ChordKey,
    /// Scroll the overflowing tab strip's visible run one wheel-notch
    /// right — toward the later windows — without switching (render
    /// mode).
    pub tab_strip_right: ChordKey,
    /// Select the next workspace in id order — `select-workspace -t +N`
    /// — and land the view on the workspace's session. Prefix `W` by
    /// default (prefix `w` is reserved for the workspace picker).
    pub workspace_next: ChordKey,
    /// Select the previous workspace in id order —
    /// `select-workspace -t +N` — and land the view on the workspace's
    /// session. Prefix `C-w` by default.
    pub workspace_prev: ChordKey,
    /// Open the bindings help panel — every chord at its effective
    /// binding, from the live config.
    pub help: ChordKey,
    /// Open the window picker modal — every session's windows as one
    /// flat list, the shown one emphasized, keyboard and mouse
    /// navigable.
    pub picker: ChordKey,
    /// Toggle the focused pane between its layout rect and the full
    /// window — tmux's zoom.
    pub zoom: ChordKey,
    /// Open the rename prompt for the shown window (its `name`).
    pub rename_window: ChordKey,
    /// Open the rename prompt for the focused pane's user title.
    pub rename_pane: ChordKey,
    /// Cycle the divider/border glyph set.
    pub border_cycle: ChordKey,
    /// Toggle pane titles embedded in the pane borders (render mode).
    pub label_toggle: ChordKey,
    /// Open the workspace picker modal — the window picker's
    /// workspace sibling: the daemon's workspace roster, the current one
    /// marked, keyboard and mouse navigable, Enter lands on it.
    pub workspace_picker: ChordKey,
    /// Toggle the workspace side panel (render mode): a left strip with
    /// the workspace list, click-to-land; more sections will follow.
    pub sidebar: ChordKey,
    /// Toggle the bottom status bar (render mode): shown by default;
    /// hiding it hands the row back to the pane grid.
    pub status_bar: ChordKey,
    /// Respawn the focused pane when it is held dead — `respawn-pane -t
    /// <pane>`. A live pane is left alone.
    pub respawn: ChordKey,
    /// Focus the pane behind the most recent agent notification this
    /// client delivered (herdr's `open_notification_target`; render mode).
    pub notify_jump: ChordKey,
    /// Apply the even-horizontal layout preset (tmux `M-1`).
    pub layout_even_horizontal: ChordKey,
    /// Apply the even-vertical layout preset (tmux `M-2`).
    pub layout_even_vertical: ChordKey,
    /// Apply the main-horizontal layout preset (tmux `M-3`).
    pub layout_main_horizontal: ChordKey,
    /// Apply the main-vertical layout preset (tmux `M-4`).
    pub layout_main_vertical: ChordKey,
    /// Apply the tiled layout preset (tmux `M-5`).
    pub layout_tiled: ChordKey,
}

impl Default for Management {
    fn default() -> Self {
        Self {
            split_right: ChordKey::plain(b'%'),
            split_down: ChordKey::plain(b'"'),
            kill_pane: ChordKey::plain(b'x'),
            break_pane: ChordKey::plain(b'!'),
            join_pane: ChordKey::plain(b'J'),
            palette: ChordKey::plain(b':'),
            new_window: ChordKey::plain(b'c'),
            resize: ChordKey::plain(b'R'),
            swap_prev: ChordKey::plain(b'{'),
            swap_next: ChordKey::plain(b'}'),
            move_window_left: ChordKey::plain(b'<'),
            move_window_right: ChordKey::plain(b'>'),
            tab_strip_left: ChordKey::plain(b'H'),
            tab_strip_right: ChordKey::plain(b'L'),
            workspace_next: ChordKey::plain(b'W'),
            workspace_prev: ChordKey::plain(0x17), // C-w
            help: ChordKey::plain(b'?'),
            picker: ChordKey::plain(b'w'),
            zoom: ChordKey::plain(b'z'),
            rename_window: ChordKey::plain(b','),
            rename_pane: ChordKey::plain(b'$'),
            border_cycle: ChordKey::plain(b'B'),
            label_toggle: ChordKey::plain(b'l'),
            workspace_picker: ChordKey::plain(b'g'),
            sidebar: ChordKey::plain(b's'),
            status_bar: ChordKey::plain(b'S'),
            respawn: ChordKey::plain(b'r'),
            notify_jump: ChordKey::plain(b'j'),
            layout_even_horizontal: ChordKey::alt(b'1'),
            layout_even_vertical: ChordKey::alt(b'2'),
            layout_main_horizontal: ChordKey::alt(b'3'),
            layout_main_vertical: ChordKey::alt(b'4'),
            layout_tiled: ChordKey::alt(b'5'),
        }
    }
}

/// One after-prefix management key as a byte, for callers that cannot
/// carry Alt (popup keys). An `M-` spelling is rejected here.
pub fn management_key(chord: &str, name: &str) -> Result<u8, String> {
    let c = management_chord(chord, name)?;
    if c.alt {
        return Err(format!(
            "[client] {name} chord {chord:?} cannot use M- (Alt) here"
        ));
    }
    Ok(c.key)
}

/// One after-prefix management chord: a single token parsed with
/// [`parse_chord`] (plain, `C-x`, or `M-x`). A management chord is ONE key
/// matched after the prefix (the reload chord's multi-token spelling exists
/// for documentation only; a management chord keeps the single-key shape).
/// Public for the settings dialog's chord validation, which must accept
/// exactly what a reload would.
pub fn management_chord(chord: &str, name: &str) -> Result<ChordKey, String> {
    let token = chord
        .split_whitespace()
        .last()
        .ok_or_else(|| format!("[client] {name} chord is empty"))?;
    for token in chord.split_whitespace() {
        if parse_chord(token).is_none() {
            return Err(format!(
                "[client] {name} chord {token:?} does not parse (expected the tmux spelling, e.g. C-b, M-1 or a single key)"
            ));
        }
    }
    parse_chord(token).ok_or_else(|| {
        format!(
            "[client] {name} chord {token:?} does not parse (expected the tmux spelling, e.g. C-b, M-1 or a single key)"
        )
    })
}
