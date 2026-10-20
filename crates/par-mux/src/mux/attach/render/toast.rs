//! The in-app notification toast (herdr's `inside herdr` delivery): a
//! bordered card at the pane area's bottom-right — the state dot and a
//! bold title, the dim body below — shown for a few seconds over the
//! panes. A click on it focuses the agent's pane.

use super::PaneRenderer;
use crate::mux::attach::panels::AgentStatus;
use ratatui::style::{Color as RtColor, Modifier as RtModifier, Style as RtStyle};

/// One toast on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    pub title: String,
    pub body: String,
    /// The state's message (the blocked reason) — the card's third line,
    /// omitted when the push carried none.
    pub message: Option<String>,
    pub status: AgentStatus,
    /// The agent's pane, focused by a click on the card.
    pub pane: u32,
}

/// How long a toast stays up: herdr's 8 s for needs-attention, 5 s
/// otherwise.
pub(crate) fn toast_lifetime(status: AgentStatus) -> std::time::Duration {
    std::time::Duration::from_secs(if status == AgentStatus::Blocked { 8 } else { 5 })
}

impl PaneRenderer {
    /// Show (or clear) the toast. Repaints.
    pub(crate) fn set_toast(&mut self, toast: Option<Toast>) {
        if self.toast != toast {
            self.toast = toast;
            self.dirty = true;
        }
    }

    /// The toast's frame rect `(x, y, w, h)` while one is up: the card
    /// at the content area's bottom-right, its width the longer line plus
    /// the ring and padding, clipped to the content area. A message adds
    /// the card's fourth row.
    pub(crate) fn toast_rect(&self) -> Option<(u16, u16, u16, u16)> {
        let toast = self.toast.as_ref()?;
        let left = self.geometry.sidebar_w.min(self.width);
        let area_w = self.width - left;
        let lines = toast
            .title
            .chars()
            .count()
            .max(toast.body.chars().count() + 2)
            .max(toast.message.as_ref().map_or(0, |m| m.chars().count() + 2));
        let w = u16::try_from(lines + 6).unwrap_or(u16::MAX).min(area_w);
        let h = (4 + u16::from(toast.message.is_some())).min(self.height);
        if w < 8 || h < 3 {
            return None;
        }
        Some((self.width - w, self.height - h, w, h))
    }

    /// The pane a toast click focuses, when the frame cell `(x, y)` is on
    /// the card.
    pub(crate) fn toast_pane_at(&self, x: u16, y: u16) -> Option<u32> {
        let (tx, ty, tw, th) = self.toast_rect()?;
        (x >= tx && x < tx + tw && y >= ty && y < ty + th)
            .then(|| self.toast.as_ref().map(|t| t.pane))
            .flatten()
    }

    /// Paint the toast card: the theme background, a dim ring, the
    /// state's colored dot and the bold title, the dim body indented
    /// under the title.
    pub(super) fn paint_toast(&mut self) {
        let Some((x0, y0, w, h)) = self.toast_rect() else {
            return;
        };
        let Some(toast) = self.toast.clone() else {
            return;
        };
        let dim = RtStyle::default().add_modifier(RtModifier::DIM);
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let cell = &mut self.buffer[(x, y)];
                cell.reset();
                if let Some(bg) = self.bg {
                    cell.set_bg(bg);
                }
                let edge_x = x == x0 || x == x0 + w - 1;
                let edge_y = y == y0 || y == y0 + h - 1;
                let symbol = match (edge_x, edge_y) {
                    (true, true) => match (x == x0, y == y0) {
                        (true, true) => self.glyphs.corner_top_left(),
                        (false, true) => self.glyphs.corner_top_right(),
                        (true, false) => self.glyphs.corner_bottom_left(),
                        (false, false) => self.glyphs.corner_bottom_right(),
                    },
                    (true, false) => self.glyphs.vertical(),
                    (false, true) => self.glyphs.horizontal(),
                    (false, false) => continue,
                };
                cell.set_symbol(symbol);
                cell.set_style(dim);
            }
        }
        let mut put = |x: u16, y: u16, text: &str, style: RtStyle| {
            for (j, ch) in text.chars().enumerate() {
                let x = x + j as u16;
                if x >= x0 + w - 1 {
                    break;
                }
                let cell = &mut self.buffer[(x, y)];
                cell.set_symbol(&ch.to_string());
                cell.set_style(style);
            }
        };
        let dot = match toast.status {
            AgentStatus::Blocked => RtColor::Indexed(1),
            AgentStatus::Working => RtColor::Indexed(3),
            AgentStatus::Done => RtColor::Indexed(6),
            AgentStatus::Idle => RtColor::Indexed(4),
            AgentStatus::None => RtColor::Indexed(14),
        };
        put(x0 + 2, y0 + 1, "\u{25CF}", RtStyle::default().fg(dot));
        put(
            x0 + 4,
            y0 + 1,
            &toast.title,
            RtStyle::default().add_modifier(RtModifier::BOLD),
        );
        if h > 3 {
            put(x0 + 4, y0 + 2, &toast.body, dim);
        }
        if h > 4 {
            if let Some(message) = &toast.message {
                put(x0 + 4, y0 + 3, message, dim);
            }
        }
    }
}
