//! logind shutdown inhibitor (Linux, `shutdown-inhibit` feature): hold a
//! delay lock and request an immediate state save when logind announces
//! `PrepareForShutdown`, so the session tree reaches disk before the host
//! terminates the daemon's panes.
//!
//! Ported from herdr's `platform/linux/shutdown.rs` (Apache-2.0) onto
//! par-mux's blocking persist worker: herdr's variant parks an async task;
//! this one parks a thread. The delay lock is held from registration until
//! the thread ends — which it never does on the shutdown path — because
//! logind caps the delay itself (`InhibitDelayMaxSec`, default 5 s): a
//! stuck daemon cannot wedge the host, it only spends the window.
//!
//! Best-effort by design: a headless container, a non-logind init, or a
//! missing system bus makes [`spawn_shutdown_inhibitor`] return `None` and
//! the daemon serves exactly as it would without the feature.

use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;

use crate::mux::persist::{PersistState, SaveOrigin};
use crate::mux::tree::MuxTree;

/// The tree plus the persist worker's inbox: what a save request needs.
pub struct SaveOnShutdown {
    tree: Arc<Mutex<MuxTree>>,
    persist_tx: std::sync::mpsc::Sender<(SaveOrigin, PersistState)>,
}

impl SaveOnShutdown {
    pub fn new(
        tree: Arc<Mutex<MuxTree>>,
        persist_tx: std::sync::mpsc::Sender<(SaveOrigin, PersistState)>,
    ) -> Self {
        Self { tree, persist_tx }
    }

    /// Capture the tree and hand the snapshot to the persist worker — the
    /// same shape every dispatch save takes, off the inhibitor's thread.
    fn request(&self) {
        let capture = self.tree.lock().collect_persist_capture();
        let state = capture.capture();
        if let Err(err) = self.persist_tx.send((SaveOrigin::Shutdown, state)) {
            log::warn!("par-mux: shutdown save request could not reach the persist worker: {err}");
        }
    }
}

/// Watch logind and save on host shutdown. `None` when the system bus or
/// logind is unreachable (the daemon keeps serving either way).
pub fn spawn_shutdown_inhibitor(saver: Arc<SaveOnShutdown>) -> Option<JoinHandle<()>> {
    let connection = match zbus::blocking::Connection::system() {
        Ok(connection) => connection,
        Err(err) => {
            log::info!("par-mux: no logind shutdown inhibitor (no system bus: {err})");
            return None;
        }
    };
    let manager = match zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    ) {
        Ok(manager) => manager,
        Err(err) => {
            log::info!("par-mux: no logind shutdown inhibitor (login1 unreachable: {err})");
            return None;
        }
    };
    // The delay lock, held for the thread's life: registration happens at
    // daemon start, release happens when the process dies (logind closes
    // the fd), and on the shutdown path the park below never returns.
    let inhibitor: zbus::zvariant::OwnedFd = match manager.call(
        "Inhibit",
        &(
            "shutdown",
            "par-mux",
            "Save terminal session state",
            "delay",
        ),
    ) {
        Ok(inhibitor) => inhibitor,
        Err(err) => {
            log::info!("par-mux: logind refused the shutdown inhibitor: {err}");
            return None;
        }
    };
    let mut signals = match manager.receive_signal("PrepareForShutdown") {
        Ok(signals) => signals,
        Err(err) => {
            log::info!("par-mux: logind shutdown signal unavailable: {err}");
            return None;
        }
    };
    log::info!("par-mux: logind shutdown inhibitor active");

    Some(
        std::thread::Builder::new()
            .name("mux-shutdown-inhibit".into())
            .spawn(move || {
                // Drop faults (logind gone, bus reset) end the watch quietly;
                // the inhibitor fd releases with it. The blocking proxy hands
                // back a plain iterator of signals.
                for signal in &mut signals {
                    let preparing = signal.body().deserialize::<bool>().unwrap_or(false);
                    if preparing {
                        log::info!("par-mux: host shutdown announced; saving the session tree");
                        saver.request();
                        // Hold the delay lock until the process dies: logind
                        // caps the wait, so a stuck save costs the window, not
                        // the shutdown.
                        loop {
                            std::thread::park();
                        }
                    }
                }
                drop(inhibitor);
            })
            .expect("spawns the inhibitor thread"),
    )
}
