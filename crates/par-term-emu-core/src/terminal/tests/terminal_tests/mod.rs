//! Terminal unit tests, split by topic.

use crate::color::{Color, NamedColor};
use crate::mouse::{MouseEncoding, MouseEvent, MouseMode};
use crate::terminal::*;

mod basics;
mod cursor_and_sgr;
mod device_queries;
mod dirty_and_introspection;
mod editing_regions_tabs;
mod export_and_restore;
mod metrics_colors_snapshots;
mod modes_and_export;
mod modify_other_keys;
mod osc52_clipboard;
mod osc_colors;
mod recording_and_tmux;
mod rectangle_ops;
mod reset_and_triggers;
mod search_and_detection;
mod sync_paste_keyboard;
mod underline_cursor_styles;
mod vt520;
