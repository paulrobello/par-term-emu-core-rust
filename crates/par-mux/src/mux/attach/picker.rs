//! The target-less session picker (`[client] session-picker`): when a
//! target-less attach finds several sessions, the user picks one inline
//! instead of reading the refusal. It runs before either attach mode
//! touches the terminal, so it draws with plain cursor movement on the
//! main screen (no ratatui, no alternate screen) and erases itself when
//! it ends.
//!
//! The key handling ([`PickerState`]), the frame ([`frame_lines`]) and
//! the draw loop ([`run_picker`]) take their input and output as
//! arguments, so the tests drive them without a terminal; only
//! [`terminal_chooser`] reads the host's keys.

use super::conn;
use super::text_input::{display_width, truncate_to_width};
use crate::mux::attach::chrome_safe;
use std::io::Write;

/// One session the picker lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerRow {
    /// The session id, `$N`.
    pub id: String,
    /// The session name.
    pub name: String,
    /// Its window count, when the `list-windows -a` query answered.
    pub windows: Option<usize>,
}

/// One input the picker acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerKey {
    /// Up arrow or `k`.
    Up,
    /// Down arrow or `j`.
    Down,
    /// Home: the first row.
    First,
    /// End: the last row.
    Last,
    /// Enter: attach the selected session.
    Enter,
    /// Esc, `q`, Ctrl-C, or the end of input.
    Abort,
    /// The host terminal changed size.
    Resize,
    /// Anything else.
    Other,
}

/// What one key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerStep {
    /// The selection moved or the size changed: draw again.
    Redraw,
    /// Nothing to do.
    Ignore,
    /// Attach the session at this row.
    Pick(usize),
    /// Leave without attaching.
    Abort,
}

/// The picker's selection over `len` rows. Moves wrap at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PickerState {
    /// The selected row.
    pub selected: usize,
    len: usize,
}

impl PickerState {
    /// A picker over `len` rows (at least one) with the first selected.
    pub(crate) fn new(len: usize) -> Self {
        Self {
            selected: 0,
            len: len.max(1),
        }
    }

    /// Apply one key.
    pub(crate) fn handle(&mut self, key: PickerKey) -> PickerStep {
        let before = self.selected;
        match key {
            PickerKey::Up => self.selected = (self.selected + self.len - 1) % self.len,
            PickerKey::Down => self.selected = (self.selected + 1) % self.len,
            PickerKey::First => self.selected = 0,
            PickerKey::Last => self.selected = self.len - 1,
            PickerKey::Enter => return PickerStep::Pick(self.selected),
            PickerKey::Abort => return PickerStep::Abort,
            PickerKey::Resize => return PickerStep::Redraw,
            PickerKey::Other => return PickerStep::Ignore,
        }
        if self.selected == before {
            PickerStep::Ignore
        } else {
            PickerStep::Redraw
        }
    }
}

/// The picker's title line.
pub(crate) const PICKER_TITLE: &str = "par-mux: several sessions are running — choose one";

/// The picker's key footer.
pub(crate) const PICKER_FOOTER: &str = "↑/↓ j/k move · enter attach · esc/q cancel";

/// Whether the picker may run: the setting is on and BOTH stdin and
/// stdout are terminals. A piped stdin cannot answer it and a redirected
/// stdout cannot show it, so a script or agent shell keeps the plain
/// refusal even with the setting on.
pub(crate) fn picker_active(setting: bool, stdin_tty: bool, stdout_tty: bool) -> bool {
    setting && stdin_tty && stdout_tty
}

/// The first row the viewport shows so `selected` stays visible in
/// `visible` rows, scrolling as little as possible from `top`.
fn viewport_top(top: usize, selected: usize, visible: usize) -> usize {
    if selected < top {
        selected
    } else if selected >= top + visible {
        selected + 1 - visible
    } else {
        top
    }
}

/// The frame for a `cols`x`rows` host: the title, one line per visible
/// session, and the footer, each cut to fit one line so nothing wraps
/// and the cursor-up redraw stays exact. Session rows align their id
/// and name columns; the selected row is `>`-marked and bold, the
/// window counts and footer dim. `top` is the first visible row.
pub(crate) fn frame_lines(
    rows: &[PickerRow],
    selected: usize,
    top: usize,
    cols: u16,
    host_rows: u16,
) -> Vec<String> {
    let width = usize::from(cols).saturating_sub(1).max(1);
    let visible = usize::from(host_rows)
        .saturating_sub(3)
        .max(1)
        .min(rows.len());
    let names: Vec<String> = rows
        .iter()
        .map(|r| chrome_safe(&r.name).into_owned())
        .collect();
    let id_w = rows.iter().map(|r| display_width(&r.id)).max().unwrap_or(0);
    let name_w = names.iter().map(|n| display_width(n)).max().unwrap_or(0);
    let mut lines = Vec::with_capacity(visible + 2);
    lines.push(format!(
        "\x1b[1m{}\x1b[0m",
        truncate_to_width(PICKER_TITLE, width).0
    ));
    for (index, row) in rows.iter().enumerate().skip(top).take(visible) {
        let name = &names[index];
        let mut text = format!(
            "{} {}{}  {}{}",
            if index == selected { '>' } else { ' ' },
            row.id,
            " ".repeat(id_w - display_width(&row.id)),
            name,
            " ".repeat(name_w - display_width(name)),
        );
        let windows = match row.windows {
            Some(1) => "  1 window".to_string(),
            Some(n) => format!("  {n} windows"),
            None => String::new(),
        };
        let (cut, used) = truncate_to_width(&text, width);
        let keep = cut.len();
        text.truncate(keep);
        let windows = truncate_to_width(&windows, width - used).0;
        let windows = if windows.is_empty() {
            String::new()
        } else {
            format!("\x1b[2m{windows}\x1b[22m")
        };
        lines.push(if index == selected {
            format!("\x1b[1m{text}\x1b[22m{windows}")
        } else {
            format!("{text}{windows}")
        });
    }
    lines.push(format!(
        "\x1b[2m{}\x1b[0m",
        truncate_to_width(PICKER_FOOTER, width).0
    ));
    lines
}

/// Draw `lines`, first moving up over the `drawn` lines of the previous
/// frame. Every line clears itself first, so a shorter frame leaves
/// nothing behind, and the trailing clear drops leftovers below.
fn draw(out: &mut dyn Write, lines: &[String], drawn: usize) -> std::io::Result<()> {
    let mut bytes = String::new();
    if drawn > 0 {
        bytes.push_str(&format!("\r\x1b[{drawn}A"));
    }
    for line in lines {
        bytes.push_str("\r\x1b[2K");
        bytes.push_str(line);
        bytes.push_str("\r\n");
    }
    bytes.push_str("\x1b[J");
    out.write_all(bytes.as_bytes())?;
    out.flush()
}

/// Run the picker over `rows`: draw, apply each key from `next_key`, and
/// redraw until a pick or an abort. `size` reports the host grid (read
/// again on every draw, so a resize re-fits). The frame is erased
/// before returning either way. `Some(index)` is the picked row.
pub(crate) fn run_picker(
    rows: &[PickerRow],
    out: &mut dyn Write,
    size: &mut dyn FnMut() -> (u16, u16),
    next_key: &mut dyn FnMut() -> PickerKey,
) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let mut state = PickerState::new(rows.len());
    let mut top = 0;
    let mut drawn = 0;
    let mut frame = |state: &PickerState, top: &mut usize, drawn: &mut usize| {
        let (cols, host_rows) = size();
        let visible = usize::from(host_rows).saturating_sub(3).max(1);
        *top = viewport_top(*top, state.selected, visible);
        let lines = frame_lines(rows, state.selected, *top, cols, host_rows);
        let ok = draw(out, &lines, *drawn).is_ok();
        *drawn = lines.len();
        ok
    };
    if !frame(&state, &mut top, &mut drawn) {
        return None;
    }
    let outcome = loop {
        match state.handle(next_key()) {
            PickerStep::Redraw => {
                if !frame(&state, &mut top, &mut drawn) {
                    break None;
                }
            }
            PickerStep::Ignore => {}
            PickerStep::Pick(index) => break Some(index),
            PickerStep::Abort => break None,
        }
    };
    let _ = write!(out, "\r\x1b[{drawn}A\x1b[J");
    let _ = out.flush();
    outcome
}

/// Map a host key event to the picker's input. Release events (kitty
/// keyboard / Windows) are ignored so one press moves one row.
fn picker_key(event: &crossterm::event::Event) -> PickerKey {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
    let key = match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        Event::Resize(..) => return PickerKey::Resize,
        _ => return PickerKey::Other,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c' | 'd') if ctrl => PickerKey::Abort,
        _ if ctrl => PickerKey::Other,
        KeyCode::Up | KeyCode::Char('k') => PickerKey::Up,
        KeyCode::Down | KeyCode::Char('j') => PickerKey::Down,
        KeyCode::Home | KeyCode::PageUp | KeyCode::Char('g') => PickerKey::First,
        KeyCode::End | KeyCode::PageDown | KeyCode::Char('G') => PickerKey::Last,
        KeyCode::Enter => PickerKey::Enter,
        KeyCode::Esc | KeyCode::Char('q') => PickerKey::Abort,
        _ => PickerKey::Other,
    }
}

/// Raw mode plus a hidden cursor for the picker's lifetime; dropping it
/// (a return or an unwind) shows the cursor and leaves raw mode.
struct PickerGuard;

impl PickerGuard {
    fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        let mut out = std::io::stdout().lock();
        let _ = super::write_all_blocking(&mut out, b"\x1b[?25l");
        let _ = out.flush();
        Ok(Self)
    }
}

impl Drop for PickerGuard {
    fn drop(&mut self) {
        let mut out = std::io::stdout().lock();
        let _ = super::write_all_blocking(&mut out, b"\x1b[?25h");
        let _ = out.flush();
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// The picker on the host terminal: raw mode, keys from crossterm's
/// event reader (a read error ends it as an abort), the frame on stdout.
pub(crate) fn terminal_chooser(rows: &[PickerRow]) -> Option<usize> {
    let Ok(_guard) = PickerGuard::enter() else {
        return None;
    };
    let mut out = std::io::stdout();
    run_picker(
        rows,
        &mut out,
        &mut conn::terminal_grid,
        &mut || match crossterm::event::read() {
            Ok(event) => picker_key(&event),
            Err(_) => PickerKey::Abort,
        },
    )
}

/// The attach entry's half, run BEFORE the attach connection exists:
/// with no `-t`, `setting` on and both stdio streams terminals
/// ([`picker_active`]), a daemon holding several sessions shows the
/// picker on the host terminal; see [`super::targets::picked_target`]
/// for the outcomes. The queries ride a plain control connection that
/// closes before the attach handshake, because the handshake's size
/// report makes a connection a viewer (it resizes the newest session's
/// window and clears its panes' `done`) — state a picker held open for
/// minutes must not create. `Err(None)` is a failed connect (the caller
/// reports no daemon).
pub(crate) fn picked_target_on_host(
    socket: &std::path::Path,
    target: Option<&str>,
    setting: bool,
) -> Result<Option<String>, Option<String>> {
    use std::io::IsTerminal as _;
    let active = picker_active(
        setting,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    );
    if target.is_some() || !active {
        return Ok(target.map(str::to_string));
    }
    let mut client = crate::mux::client::MuxClient::connect(socket).map_err(|_| None)?;
    super::targets::picked_target(
        &mut |command| client.send_checked(command),
        None,
        true,
        &mut terminal_chooser,
    )
    .map_err(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<PickerRow> {
        vec![
            PickerRow {
                id: "$0".to_string(),
                name: "att".to_string(),
                windows: Some(1),
            },
            PickerRow {
                id: "$12".to_string(),
                name: "second".to_string(),
                windows: Some(3),
            },
            PickerRow {
                id: "$13".to_string(),
                name: "third".to_string(),
                windows: None,
            },
        ]
    }

    /// The SGR-free text of a frame line.
    fn plain(line: &str) -> String {
        let mut out = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The gate needs the setting AND both stdio streams on a terminal:
    /// every other combination keeps the refusal.
    #[test]
    fn picker_runs_only_with_the_setting_and_both_stdio_terminals() {
        for setting in [false, true] {
            for stdin_tty in [false, true] {
                for stdout_tty in [false, true] {
                    assert_eq!(
                        picker_active(setting, stdin_tty, stdout_tty),
                        setting && stdin_tty && stdout_tty,
                        "setting={setting} stdin={stdin_tty} stdout={stdout_tty}"
                    );
                }
            }
        }
    }

    /// Up/Down wrap, Home/End jump, Enter picks the selection, Abort
    /// aborts, and keys that change nothing ask for no redraw.
    #[test]
    fn picker_state_moves_wraps_picks_and_aborts() {
        let mut state = PickerState::new(3);
        assert_eq!(state.handle(PickerKey::Down), PickerStep::Redraw);
        assert_eq!(state.selected, 1);
        assert_eq!(state.handle(PickerKey::Down), PickerStep::Redraw);
        assert_eq!(state.handle(PickerKey::Down), PickerStep::Redraw);
        assert_eq!(state.selected, 0, "down wraps to the top");
        assert_eq!(state.handle(PickerKey::Up), PickerStep::Redraw);
        assert_eq!(state.selected, 2, "up wraps to the bottom");
        assert_eq!(state.handle(PickerKey::Last), PickerStep::Ignore);
        assert_eq!(state.handle(PickerKey::First), PickerStep::Redraw);
        assert_eq!(state.handle(PickerKey::Other), PickerStep::Ignore);
        assert_eq!(state.handle(PickerKey::Resize), PickerStep::Redraw);
        assert_eq!(state.handle(PickerKey::Down), PickerStep::Redraw);
        assert_eq!(state.handle(PickerKey::Enter), PickerStep::Pick(1));
        assert_eq!(state.handle(PickerKey::Abort), PickerStep::Abort);
    }

    /// The frame: a bold title, one aligned row per session with the
    /// selection `>`-marked and bold and the window counts dim, then the
    /// key footer — no line wider than the host.
    #[test]
    fn frame_lists_aligned_rows_with_the_selection_marked() {
        let lines = frame_lines(&rows(), 1, 0, 80, 24);
        let text: Vec<String> = lines.iter().map(|l| plain(l)).collect();
        assert_eq!(
            text,
            [
                PICKER_TITLE,
                "  $0   att     1 window",
                "> $12  second  3 windows",
                "  $13  third ",
                PICKER_FOOTER,
            ]
        );
        assert!(lines[0].starts_with("\x1b[1m"), "the title is bold");
        assert!(
            lines[2].starts_with("\x1b[1m> $12"),
            "the selected row is bold: {:?}",
            lines[2]
        );
        assert!(lines[1].contains("\x1b[2m  1 window"), "counts are dim");
        assert!(lines[4].starts_with("\x1b[2m"), "the footer is dim");
    }

    /// A narrow host cuts every line to its width, a short one scrolls
    /// the rows, and a name's control characters never reach the host.
    #[test]
    fn frame_fits_narrow_and_short_hosts_and_strips_controls() {
        let mut list = rows();
        list[0].name = "evil\x1b[2Jname".to_string();
        for line in frame_lines(&list, 0, 0, 12, 24) {
            assert!(display_width(&plain(&line)) <= 11, "{line:?}");
        }
        let text: Vec<String> = frame_lines(&list, 0, 0, 80, 24)
            .iter()
            .map(|l| plain(l))
            .collect();
        assert!(text[1].contains("evil[2Jname"), "{text:?}");
        // Four host rows: title, one session row, footer.
        let short = frame_lines(&list, 2, 2, 80, 4);
        assert_eq!(short.len(), 3);
        assert!(plain(&short[1]).starts_with("> $13"), "{short:?}");
        assert_eq!(viewport_top(0, 2, 1), 2);
        assert_eq!(viewport_top(2, 0, 1), 0);
        assert_eq!(viewport_top(1, 2, 2), 1);
    }

    /// The draw loop: each key that moves redraws over the previous
    /// frame (cursor up by its line count), Enter returns the selected
    /// row, and the frame is erased on the way out.
    #[test]
    fn run_picker_redraws_in_place_and_returns_the_pick() {
        let mut keys = [
            PickerKey::Down,
            PickerKey::Other,
            PickerKey::Down,
            PickerKey::Enter,
        ]
        .into_iter();
        let mut out: Vec<u8> = Vec::new();
        let picked = run_picker(&rows(), &mut out, &mut || (80, 24), &mut || {
            keys.next().unwrap_or(PickerKey::Abort)
        });
        assert_eq!(picked, Some(2));
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.matches(PICKER_TITLE).count(),
            3,
            "first draw + two moves"
        );
        assert_eq!(
            text.matches("\r\x1b[5A").count(),
            3,
            "two redraws + the erase"
        );
        assert!(text.ends_with("\r\x1b[5A\x1b[J"), "erased: {text:?}");
    }

    /// An abort (and the end of input) returns no pick, erased the same.
    #[test]
    fn run_picker_abort_returns_none() {
        let mut out: Vec<u8> = Vec::new();
        let picked = run_picker(&rows(), &mut out, &mut || (80, 24), &mut || {
            PickerKey::Abort
        });
        assert_eq!(picked, None);
        assert!(String::from_utf8(out).unwrap().ends_with("\x1b[J"));
    }

    /// Host keys: arrows and j/k move, g/G and Home/End jump, Enter
    /// picks, Esc/q/Ctrl-C/Ctrl-D abort, a release does nothing.
    #[test]
    fn host_keys_map_to_picker_keys() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
        let key = |code, modifiers| Event::Key(KeyEvent::new(code, modifiers));
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(picker_key(&key(KeyCode::Up, none)), PickerKey::Up);
        assert_eq!(picker_key(&key(KeyCode::Char('k'), none)), PickerKey::Up);
        assert_eq!(picker_key(&key(KeyCode::Down, none)), PickerKey::Down);
        assert_eq!(picker_key(&key(KeyCode::Char('j'), none)), PickerKey::Down);
        assert_eq!(picker_key(&key(KeyCode::Char('g'), none)), PickerKey::First);
        assert_eq!(picker_key(&key(KeyCode::End, none)), PickerKey::Last);
        assert_eq!(picker_key(&key(KeyCode::Enter, none)), PickerKey::Enter);
        assert_eq!(picker_key(&key(KeyCode::Esc, none)), PickerKey::Abort);
        assert_eq!(picker_key(&key(KeyCode::Char('q'), none)), PickerKey::Abort);
        assert_eq!(picker_key(&key(KeyCode::Char('c'), ctrl)), PickerKey::Abort);
        assert_eq!(picker_key(&key(KeyCode::Char('d'), ctrl)), PickerKey::Abort);
        assert_eq!(picker_key(&key(KeyCode::Char('j'), ctrl)), PickerKey::Other);
        assert_eq!(picker_key(&Event::Resize(10, 10)), PickerKey::Resize);
        let mut release = KeyEvent::new(KeyCode::Down, none);
        release.kind = KeyEventKind::Release;
        assert_eq!(picker_key(&Event::Key(release)), PickerKey::Other);
    }
}
