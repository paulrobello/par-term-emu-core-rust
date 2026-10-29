//! The durable pane lifecycle event ring (par-director's needs).
//!
//! Pushed notifications are lossy by design — `--cmd` discards them while a
//! reply is pending, a control client is evicted at 4096 queued lines, and
//! the plugin tier drops them past 8 in flight — so a consumer that must not
//! miss a pane exit replays this ring instead: every lifecycle notification
//! the daemon broadcasts (except the geometry churn of `%layout-change`;
//! `%output` never flows through the broadcast seam) is recorded here under
//! a daemon-wide monotonically increasing sequence number for the daemon's
//! lifetime. A client that reconnects sends `events --since <seq>` and
//! reads everything it missed, plus the current head so it knows where to
//! resume. The ring is in-memory only: a daemon restart clears it, and a
//! consumer that resumes `--since <seq>` past eviction sees the gap as
//! holes in the sequence numbers.
//!
//! State lives on the tree (per-daemon, hermetic per test), recorded at the
//! `broadcast_notification` seam where the documented lock discipline
//! ("releases the tree before broadcasting") guarantees no caller holds the
//! tree lock.

use par_term_emu_core::tmux_control::TmuxNotification;
use std::collections::VecDeque;

/// Bounded ring: the newest 1024 events. At lifecycle-event rates (pane
/// spawns, exits, renames, agent state changes) this is hours of history;
/// an eviction shows up on the wire as a hole in `seq`.
/// cap: Lifecycle events retained by the daemon's replay ring.
pub(crate) const EVENT_RING_CAPACITY: usize = 1024;

/// One replayable event: its sequence number, its epoch-ms timestamp, and
/// the emitted control-mode wire line (trailing newline stripped, leading
/// `%` included) — exactly the line a live client would have received on
/// the push stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MuxEvent {
    pub(crate) seq: u64,
    pub(crate) at_ms: u64,
    pub(crate) line: String,
}

/// The bounded in-memory ring itself: `next_seq` is the sequence number the
/// NEXT recorded event gets, so `head()` is `next_seq - 1` and 0 before the
/// first event (no event ever carries seq 0 — the first is seq 1).
#[derive(Debug, Default)]
pub struct EventRing {
    next_seq: u64,
    events: VecDeque<MuxEvent>,
}

impl EventRing {
    /// Record one already-emitted wire line (trailing newline optional):
    /// stamp it, push it, and evict the oldest past the capacity. Returns
    /// the recorded event's sequence number.
    pub(crate) fn record_line(&mut self, line: &str) -> u64 {
        let line = line.trim_end_matches('\n');
        if line.is_empty() {
            return self.head();
        }
        let seq = self.next_seq + 1;
        self.next_seq = seq;
        self.events.push_back(MuxEvent {
            seq,
            at_ms: now_epoch_ms(),
            line: line.to_string(),
        });
        while self.events.len() > EVENT_RING_CAPACITY {
            self.events.pop_front();
        }
        seq
    }

    /// The highest recorded sequence number (0 = nothing recorded yet).
    pub(crate) fn head(&self) -> u64 {
        self.next_seq
    }

    /// Every retained event with `seq > after`, in sequence order.
    pub(crate) fn since(&self, after: u64) -> Vec<MuxEvent> {
        self.events
            .iter()
            .filter(|event| event.seq > after)
            .cloned()
            .collect()
    }
}

/// Record the notification's wire line into the tree's ring, when the
/// notification is ring-worthy at all: everything that flows the broadcast
/// seam is lifecycle-shaped except `%layout-change`, the geometry churn
/// every resize report broadcasts — recording it would evict real events.
pub(crate) fn wants(notification: &TmuxNotification) -> bool {
    !matches!(notification, TmuxNotification::LayoutChange { .. })
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring_with(lines: &[&str]) -> EventRing {
        let mut ring = EventRing::default();
        for line in lines {
            ring.record_line(line);
        }
        ring
    }

    #[test]
    fn recording_assigns_monotonic_sequence_numbers_from_one() {
        let mut ring = EventRing::default();
        assert_eq!(ring.head(), 0, "an empty ring's head is 0");
        assert_eq!(ring.record_line("%pane-exited %3 7"), 1);
        assert_eq!(ring.record_line("%window-add @2\n"), 2);
        assert_eq!(ring.head(), 2);
    }

    #[test]
    fn the_newline_is_stripped_and_empty_lines_record_nothing() {
        let mut ring = EventRing::default();
        ring.record_line("%pane-exited %3 7\n");
        assert_eq!(
            ring.since(0)[0].line,
            "%pane-exited %3 7",
            "the trailing newline does not ride"
        );
        assert_eq!(ring.record_line("\n"), 1, "an empty line records nothing");
        assert_eq!(ring.record_line(""), 1);
        assert_eq!(ring.since(0).len(), 1, "only the real event is stored");
    }

    #[test]
    fn since_returns_only_events_after_the_watermark_in_order() {
        let ring = ring_with(&["%a 1", "%b 2", "%c 3"]);
        let seqs: Vec<u64> = ring.since(0).iter().map(|event| event.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        assert_eq!(
            ring.since(1).iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![2, 3],
            "since(s) excludes the event AT s"
        );
        assert!(ring.since(3).is_empty(), "since(head) is empty");
        assert!(ring.since(99).is_empty(), "since(past-head) is empty");
    }

    #[test]
    fn the_ring_evicts_the_oldest_past_the_capacity() {
        let mut ring = EventRing::default();
        for i in 0..EVENT_RING_CAPACITY + 25 {
            ring.record_line(&format!("%e {i}"));
        }
        assert_eq!(ring.since(0).len(), EVENT_RING_CAPACITY, "bounded");
        // The first recorded seq is 1, but the oldest RETAINED event's seq
        // is 26: eviction shows up as holes, not renumbering.
        assert_eq!(ring.since(0)[0].seq, 26);
        assert_eq!(ring.since(0)[0].line, "%e 25");
        assert_eq!(ring.head(), EVENT_RING_CAPACITY as u64 + 25);
    }

    #[test]
    fn layout_change_is_not_ring_worthy() {
        let notification = TmuxNotification::LayoutChange {
            window_id: "@0".to_string(),
            window_layout: "0000,80x24,0,0,1".to_string(),
            window_visible_layout: "0000,80x24,0,0,1".to_string(),
            window_raw_flags: String::new(),
        };
        assert!(!wants(&notification));
        assert!(wants(&TmuxNotification::PaneExited {
            pane_id: "%3".to_string(),
            exit_code: Some(7),
        }));
    }
}
