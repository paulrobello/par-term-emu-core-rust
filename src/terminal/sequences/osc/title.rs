//! Title-related OSC sequence handling

use crate::terminal::Terminal;

impl Terminal {
    pub(crate) fn handle_osc_title(&mut self, command: &str, params: &[&[u8]]) {
        match command {
            "0" | "2" => {
                if params.len() >= 2 {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        let new_title = title.to_string();
                        if self.title != new_title {
                            self.title = new_title.clone();
                            self.terminal_events
                                .push(crate::terminal::TerminalEvent::TitleChanged(new_title));
                        }
                    }
                }
            }
            "21" => {
                if params.len() >= 2 {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        self.title_stack.push(title.to_string());
                    }
                } else {
                    self.title_stack.push(self.title.clone());
                }
            }
            "22" | "23" => {
                if let Some(title) = self.title_stack.pop() {
                    self.title = title;
                }
            }
            _ => {}
        }
    }
}
