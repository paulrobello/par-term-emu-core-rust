//! Panes: PTY ownership, output plumbing, and the factory seam.

use crate::mux::ids::PaneId;
use crate::pty_error::PtyError;
use crate::pty_session::PtySession;
use crate::terminal::Terminal;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Errors raised while creating or driving a pane.
#[derive(Debug)]
pub enum MuxError {
    /// The underlying PTY layer failed.
    Pty(PtyError),
    /// The requested pane does not exist.
    NoSuchPane(PaneId),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MuxError::Pty(err) => write!(f, "pty error: {err}"),
            MuxError::NoSuchPane(id) => write!(f, "no such pane: {id}"),
        }
    }
}

impl std::error::Error for MuxError {}

impl From<PtyError> for MuxError {
    fn from(err: PtyError) -> Self {
        MuxError::Pty(err)
    }
}

/// Default scrollback retained per pane.
const DEFAULT_SCROLLBACK: usize = 10_000;

/// One pane: a PTY, its terminal emulator, and its metadata.
///
/// `metadata` is extension seam S2 — empty in the base server, and the place an
/// agent layer later records `agent`, `agent_status`, and session identity
/// without changing this struct.
pub struct MuxPane {
    id: PaneId,
    session: PtySession,
    metadata: HashMap<String, String>,
}

impl MuxPane {
    /// This pane's identifier.
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// The terminal emulator backing this pane.
    pub fn terminal(&self) -> Arc<RwLock<Terminal>> {
        self.session.terminal()
    }

    /// Whether the pane's child process is still running.
    pub fn is_running(&self) -> bool {
        self.session.is_running()
    }

    /// The pane's child process id, if it has been spawned.
    pub fn child_pid(&self) -> Option<u32> {
        self.session.child_pid()
    }

    /// Read-only view of this pane's metadata (seam S2).
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Record a metadata entry (seam S2).
    pub fn set_metadata(&mut self, key: &str, value: &str) {
        self.metadata.insert(key.to_string(), value.to_string());
    }

    /// Install the sink that receives raw PTY output for this pane.
    ///
    /// The server wires this to the control-mode emitter so bytes become
    /// `%output` lines as they are produced — the push path that a snapshot
    /// API cannot provide.
    pub fn on_output<F>(&mut self, callback: F)
    where
        F: Fn(&[u8]) + Send + Sync + 'static,
    {
        // OutputCallback is Arc<dyn Fn(&[u8]) + Send + Sync>, so the sink is
        // shared, not boxed.
        self.session.set_output_callback(Arc::new(callback));
    }

    /// Write client input to the pane's PTY.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), MuxError> {
        self.session.write(bytes).map_err(MuxError::from)
    }

    /// Resize the pane.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), MuxError> {
        self.session.resize(cols, rows).map_err(MuxError::from)
    }

    /// Terminate the pane's child process.
    pub fn kill(&mut self) -> Result<(), MuxError> {
        self.session.kill().map_err(MuxError::from)
    }
}

/// Creates panes on demand — extension seam S1.
///
/// Mirrors [`crate::streaming::SessionFactory`] deliberately. An agent layer
/// later ships an implementation that spawns an agent CLI, seeds the child
/// environment, and tags [`MuxPane::metadata`], with no change to this trait
/// or to the server that calls it.
pub trait PaneFactory: Send + Sync {
    /// Create a pane, spawning its process.
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
    ) -> Result<MuxPane, MuxError>;
}

/// The default factory: spawns the user's shell, or an explicit command.
#[derive(Debug, Default)]
pub struct ShellPaneFactory {
    /// Working directory for new panes; the process default when `None`.
    pub cwd: Option<std::path::PathBuf>,
}

impl PaneFactory for ShellPaneFactory {
    fn create_pane(
        &self,
        id: PaneId,
        cols: u16,
        rows: u16,
        command: Option<&str>,
    ) -> Result<MuxPane, MuxError> {
        let mut session = PtySession::new(cols as usize, rows as usize, DEFAULT_SCROLLBACK);

        if let Some(cwd) = &self.cwd {
            session.set_cwd(cwd);
        }
        // The pane id is exported so hooks running inside the pane can identify
        // themselves back to the server — the mechanism seam S1's agent layer
        // will rely on.
        session.set_env("PAR_MUX_PANE_ID", &id.to_string());

        match command {
            Some(cmd) => {
                let shell = PtySession::get_default_shell();
                session.spawn(&shell, &["-c", cmd])?;
            }
            None => session.spawn_shell()?,
        }

        Ok(MuxPane {
            id,
            session,
            metadata: HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn shell_factory_creates_a_running_pane() {
        let factory = ShellPaneFactory::default();
        let pane = factory
            .create_pane(PaneId(0), 80, 24, None)
            .expect("shell pane should spawn");
        assert_eq!(pane.id(), PaneId(0));
        assert!(
            pane.is_running(),
            "a freshly spawned shell should be running"
        );
        assert!(pane.child_pid().is_some(), "a spawned pane has a child pid");
    }

    #[test]
    fn factory_honors_an_explicit_command() {
        let factory = ShellPaneFactory::default();
        let pane = factory
            .create_pane(PaneId(1), 80, 24, Some("echo par-mux"))
            .expect("command pane should spawn");
        assert!(pane.child_pid().is_some());
    }

    #[test]
    fn output_callback_receives_pty_bytes() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory
            .create_pane(PaneId(2), 80, 24, Some("echo par-mux-marker"))
            .expect("pane should spawn");

        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen);
        pane.on_output(move |bytes: &[u8]| {
            if !bytes.is_empty() {
                counter.fetch_add(bytes.len(), Ordering::Relaxed);
            }
        });

        // The shell needs a moment to run and flush. Poll rather than sleep a
        // fixed duration so a fast machine does not wait and a slow one does.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while seen.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert!(
            seen.load(Ordering::Relaxed) > 0,
            "output callback should have received PTY bytes within 5s"
        );
    }

    #[test]
    fn metadata_starts_empty_and_accepts_entries() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory.create_pane(PaneId(3), 80, 24, None).unwrap();
        assert!(
            pane.metadata().is_empty(),
            "metadata starts empty (seam S2)"
        );
        pane.set_metadata("agent", "claude");
        assert_eq!(
            pane.metadata().get("agent").map(String::as_str),
            Some("claude")
        );
    }

    #[test]
    fn resize_updates_the_terminal_dimensions() {
        let factory = ShellPaneFactory::default();
        let mut pane = factory.create_pane(PaneId(4), 80, 24, None).unwrap();
        pane.resize(100, 30).expect("resize should succeed");
        let terminal = pane.terminal();
        let guard = terminal.read();
        assert_eq!(guard.size(), (100, 30));
    }
}
