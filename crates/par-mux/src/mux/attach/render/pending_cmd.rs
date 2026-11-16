//! Reply continuations: write a daemon command now, run the rest of the
//! chord on the pump tick its reply arrives (ENH-049).
//!
//! The daemon serves one connection's lines strictly in order, so a
//! command written after another is applied after it whether or not the
//! first reply was awaited. What this buys is on the client: the pump keeps
//! reading stdin and painting while replies are owed.

use super::WindowSession;
use crate::mux::attach::conn::AttachConn;
use crate::mux::client::Reply;
use std::collections::HashMap;

/// What runs when a command's reply arrives.
pub(super) type Continuation = Box<dyn FnOnce(&mut WindowSession, &mut AttachConn, Reply)>;

/// Continuations parked by token, awaiting their replies.
#[derive(Default)]
pub(super) struct PendingCommands {
    by_token: HashMap<u64, Continuation>,
}

impl PendingCommands {
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.by_token.is_empty()
    }

    /// Whether the reply for `token` is still owed.
    pub(super) fn contains(&self, token: u64) -> bool {
        self.by_token.contains_key(&token)
    }
}

impl WindowSession {
    /// Write `command` now; run `then` on the pump tick its reply arrives.
    /// A `%error` reply reaches `then` as `Reply { ok: false, .. }`. A write
    /// failure drops `then` and flashes the "daemon busy" cue when it was a
    /// timeout (a closed daemon ends the pump through the notification
    /// channel).
    pub(super) fn send_then(
        &mut self,
        conn: &mut AttachConn,
        command: &str,
        then: impl FnOnce(&mut Self, &mut AttachConn, Reply) + 'static,
    ) {
        self.send_then_tracking(conn, command, then);
    }

    /// [`send_then`] that also reports the reply token, so the caller can
    /// suppress work that must not jump ahead of this reply (the roster
    /// click's landing versus its own broadcasts — the follow race the
    /// old nav_landing field covered). `None` on a write failure, where
    /// no reply will ever come.
    pub(super) fn send_then_tracking(
        &mut self,
        conn: &mut AttachConn,
        command: &str,
        then: impl FnOnce(&mut Self, &mut AttachConn, Reply) + 'static,
    ) -> Option<u64> {
        match conn.send_nowait(command) {
            Ok(token) => {
                self.pending_cmds.by_token.insert(token, Box::new(then));
                Some(token)
            }
            Err(err) => {
                self.flash_if_busy(&err);
                None
            }
        }
    }

    /// Pump step 1b: run the continuation of every reply that has arrived,
    /// in arrival order. A continuation may park further ones; those run on
    /// a later call.
    pub(super) fn dispatch_replies(&mut self, conn: &mut AttachConn) {
        for (token, reply) in conn.poll_replies() {
            if let Some(then) = self.pending_cmds.by_token.remove(&token) {
                then(self, conn, reply);
            }
        }
    }
}
