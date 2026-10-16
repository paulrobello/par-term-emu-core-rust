//! Tmux control protocol support
//!
//! Implements parsing and handling of tmux control mode protocol.
//! Control mode is enabled with `tmux -C` (or `-CC` for no echo).
//!
//! # Protocol Overview
//!
//! - Commands sent to tmux are standard tmux commands
//! - Output from tmux consists of:
//!   - Command output wrapped in `%begin`/`%end` or `%begin`/`%error`
//!   - Asynchronous notifications starting with `%`
//!   - Pane output in format `%output %pane-id data`
//!
//! # References
//!
//! - [Tmux Control Mode Wiki](https://github.com/tmux/tmux/wiki/Control-Mode)

mod notification;
mod parser;

pub use notification::TmuxNotification;
pub use parser::TmuxControlParser;

#[cfg(test)]
mod tests;
