//! Copy mode's `e`: the focused pane's scrollback + screen in the user's
//! editor, ported from herdr's `edit_scrollback` (Apache-2.0, herdr
//! `src/app/custom_commands.rs` + `src/platform/macos.rs`).
//!
//! herdr's semantics, kept: the pane's text (soft-wrapped rows rejoined
//! into logical lines) lands in a private temp file, and a temporary
//! ZOOMED pane runs the editor on it, deleting the file when the editor
//! exits. Nothing round-trips — the file is a scratchpad to search, copy
//! from, or save elsewhere. The editor runs in a daemon pane rather than
//! on the attach client's own tty, so the client keeps owning its screen
//! and stdin reader throughout.

use super::WindowSession;
use crate::mux::attach::conn::AttachConn;

/// `$VISUAL`, then `$EDITOR`; an empty or whitespace-only value counts as
/// unset (herdr's filter). `None` refuses the key — unlike herdr there is
/// no `vi` fallback, so a host with neither set gets a clear refusal
/// instead of an editor it never chose.
pub(super) fn resolve_editor(visual: Option<String>, editor: Option<String>) -> Option<String> {
    [visual, editor]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

/// The process environment's `(VISUAL, EDITOR)` — the session's default
/// editor lookup (tests swap it, never the process env).
pub(super) fn env_editor_lookup() -> (Option<String>, Option<String>) {
    (std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok())
}

/// The pair of private files one edit uses: the text and the launcher
/// script the zoomed pane runs (it removes both when the editor exits).
#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScrollbackFiles {
    pub(super) text: std::path::PathBuf,
    pub(super) launcher: std::path::PathBuf,
}

/// Single-quote `value` for `/bin/sh`.
#[cfg(unix)]
pub(super) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The launcher: run the editor on the text file, then remove both files
/// and exit with the editor's status. `editor` is a shell word list the
/// user wrote (`code -w`, `nvim -R`), so it is spliced unquoted — herdr's
/// `eval "${EDITOR}" file` semantics.
#[cfg(unix)]
fn launcher_script(editor: &str, files: &ScrollbackFiles) -> String {
    let text = sh_quote(&files.text.to_string_lossy());
    let launcher = sh_quote(&files.launcher.to_string_lossy());
    format!("{editor} {text}\nstatus=$?\nrm -f {text} {launcher}\nexit $status\n")
}

/// Create `path` exclusively (never following or reusing an existing
/// file) with owner-only permissions and write `content`.
#[cfg(unix)]
fn write_private(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content.as_bytes())
}

/// Write the text and launcher into `dir` under fresh names. A name
/// collision retries with the next counter; any other failure removes
/// whatever was written.
#[cfg(unix)]
pub(super) fn write_scrollback_files(
    dir: &std::path::Path,
    text: &str,
    editor: &str,
) -> std::io::Result<ScrollbackFiles> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut last = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let stem = format!("par-mux-scrollback-{}-{n}", std::process::id());
        let files = ScrollbackFiles {
            text: dir.join(format!("{stem}.txt")),
            launcher: dir.join(format!("{stem}.sh")),
        };
        match write_private(&files.text, text) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                last = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        }
        if let Err(err) = write_private(&files.launcher, &launcher_script(editor, &files)) {
            let _ = std::fs::remove_file(&files.text);
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                last = Some(err);
                continue;
            }
            return Err(err);
        }
        return Ok(files);
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no free scrollback file name")))
}

/// A popup's private launcher in `dir`: it removes itself, then runs
/// `command` as shell text (herdr runs popup commands through the
/// shell). Same exclusive, owner-only creation as the scrollback files.
#[cfg(unix)]
pub(super) fn write_popup_launcher(
    dir: &std::path::Path,
    command: &str,
) -> std::io::Result<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut last = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("par-mux-popup-{}-{n}.sh", std::process::id()));
        let script = format!("rm -f {}\n{command}\n", sh_quote(&path.to_string_lossy()));
        match write_private(&path, &script) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => last = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no free popup launcher name")))
}

#[cfg(unix)]
fn remove_files(files: &ScrollbackFiles) {
    let _ = std::fs::remove_file(&files.text);
    let _ = std::fs::remove_file(&files.launcher);
}

impl WindowSession {
    /// Copy mode's `e`: dump the focused pane's scrollback + screen (the
    /// grid's plain-text export, wrapped rows rejoined) to a private temp
    /// file and open it in `$VISUAL`/`$EDITOR` in a temporary zoomed pane
    /// split from the focused one. Without an editor the key is refused
    /// on the flash and copy mode stays up.
    pub(super) fn edit_scrollback(&mut self, conn: &mut AttachConn) {
        let Some(pane) = self.renderer.focused() else {
            return;
        };
        let (visual, editor) = (self.editor_lookup)();
        let Some(editor) = resolve_editor(visual, editor) else {
            self.chrome
                .flash("edit scrollback: set $VISUAL or $EDITOR".to_string());
            self.draw_status_row();
            return;
        };
        self.launch_scrollback_editor(conn, pane, &editor);
    }

    #[cfg(not(unix))]
    fn launch_scrollback_editor(&mut self, _conn: &mut AttachConn, _pane: u32, _editor: &str) {
        self.chrome
            .flash("edit scrollback: needs a POSIX shell (unix only)".to_string());
        self.draw_status_row();
    }

    #[cfg(unix)]
    fn launch_scrollback_editor(&mut self, conn: &mut AttachConn, pane: u32, editor: &str) {
        let Some(text) = self
            .renderer
            .pane_terminal(pane)
            .map(|t| t.active_grid().export_text_buffer())
        else {
            return;
        };
        let files = match write_scrollback_files(&std::env::temp_dir(), &text, editor) {
            Ok(files) => files,
            Err(err) => {
                self.chrome.flash(format!("edit scrollback: {err}"));
                self.draw_status_row();
                return;
            }
        };
        self.leave_scroll_mode();
        // The launcher runs under /bin/sh whatever the pane shell is (fish
        // would reject a POSIX script). The start command names only the
        // quoted script path: split-window's flag scan reads `-c`/`-h`/`-p`
        // anywhere on the line, so an inline `sh -c '…'` would be misread.
        let launcher = sh_quote(&files.launcher.to_string_lossy());
        let expected = self.window.clone();
        self.send_then(
            conn,
            &format!("split-window -t %{pane} -- /bin/sh {launcher}"),
            move |s, c, reply| {
                if !reply.ok {
                    remove_files(&files);
                    let why = reply.body.first().cloned().unwrap_or_default();
                    s.chrome.flash(format!("edit scrollback failed: {why}"));
                    s.draw_status_row();
                    return;
                }
                if s.window != expected {
                    return;
                }
                let new_pane = reply
                    .body
                    .first()
                    .and_then(|id| id.trim().strip_prefix('%'))
                    .and_then(|n| n.parse::<u32>().ok());
                let window = s.window.clone();
                s.reseed_window(c, &window);
                let Some(new_pane) = new_pane else {
                    return;
                };
                s.renderer.focus(new_pane);
                let _ = c.send_forget(&format!("select-pane -t %{new_pane}"));
                // The select above cleared any zoom on another pane, so
                // the toggle always zooms the editor pane.
                s.send_then(
                    c,
                    &format!("resize-pane -t %{new_pane} -Z"),
                    |s, _c, reply| {
                        if reply.ok {
                            s.zoomed = true;
                        }
                    },
                );
                s.chrome
                    .flash("scrollback opened in the editor (a scratch copy)".to_string());
                s.draw_status_row();
            },
        );
    }
}
