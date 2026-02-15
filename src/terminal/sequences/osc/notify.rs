//! Notification and progress OSC sequence handling

use crate::debug;
use crate::terminal::progress::{ProgressBar, ProgressBarCommand, ProgressState};
use crate::terminal::Notification;
use crate::terminal::Terminal;

impl Terminal {
    pub(crate) fn handle_osc_notify(&mut self, command: &str, params: &[&[u8]]) {
        match command {
            "9" => {
                if params.len() >= 2 {
                    if let Ok(param1) = std::str::from_utf8(params[1]) {
                        let param1 = param1.trim();
                        if param1 == "4" {
                            self.handle_osc9_progress(&params[2..]);
                        } else {
                            let notification = Notification::new(String::new(), param1.to_string());
                            self.enqueue_notification(notification);
                        }
                    }
                }
            }
            "777" => {
                if params.len() >= 4 {
                    if let Ok(action) = std::str::from_utf8(params[1]) {
                        if action == "notify" {
                            if let (Ok(title), Ok(message)) = (
                                std::str::from_utf8(params[2]),
                                std::str::from_utf8(params[3]),
                            ) {
                                let notification =
                                    Notification::new(title.to_string(), message.to_string());
                                self.enqueue_notification(notification);
                            }
                        }
                    }
                }
            }
            "934" => {
                self.handle_osc934(params);
            }
            _ => {}
        }
    }

    pub(crate) fn handle_osc9_progress(&mut self, params: &[&[u8]]) {
        if params.is_empty() {
            return;
        }

        let state_param = match std::str::from_utf8(params[0]) {
            Ok(s) => s.trim(),
            Err(_) => return,
        };

        let state_num: u8 = match state_param.parse() {
            Ok(n) => n,
            Err(_) => return,
        };

        let state = ProgressState::from_param(state_num);

        let progress = if state.requires_progress() && params.len() >= 2 {
            match std::str::from_utf8(params[1]) {
                Ok(s) => s.trim().parse::<u8>().unwrap_or(0).min(100),
                Err(_) => 0,
            }
        } else {
            0
        };

        self.progress_bar = ProgressBar::new(state, progress);

        debug::log(
            debug::DebugLevel::Debug,
            "OSC9",
            &format!(
                "Progress bar: state={}, progress={}",
                state.description(),
                progress
            ),
        );
    }

    pub(crate) fn handle_osc934(&mut self, params: &[&[u8]]) {
        match ProgressBarCommand::parse(params) {
            Some(ProgressBarCommand::Set(bar)) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    &format!(
                        "Set progress bar: id={}, state={}, percent={}, label={:?}",
                        bar.id,
                        bar.state.description(),
                        bar.percent,
                        bar.label
                    ),
                );
                self.set_named_progress_bar(bar);
            }
            Some(ProgressBarCommand::Remove(id)) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    &format!("Remove progress bar: id={}", id),
                );
                self.remove_named_progress_bar(&id);
            }
            Some(ProgressBarCommand::RemoveAll) => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    "Remove all progress bars",
                );
                self.remove_all_named_progress_bars();
            }
            None => {
                debug::log(
                    debug::DebugLevel::Debug,
                    "OSC934",
                    "Failed to parse OSC 934 sequence",
                );
            }
        }
    }
}
