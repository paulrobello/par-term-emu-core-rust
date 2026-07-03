//! Regex trigger and trigger-action types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Trigger information (read-only view)
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "Trigger", from_py_object)]
#[derive(Clone)]
pub struct PyTrigger {
    pub id: u64,
    pub name: String,
    pub pattern: String,
    pub enabled: bool,
    pub fire_once_per_line: bool,
    pub match_count: usize,
}

#[pymethods]
impl PyTrigger {
    fn __repr__(&self) -> String {
        format!(
            "Trigger(id={}, name={}, pattern={}, enabled={}, matches={})",
            self.id, self.name, self.pattern, self.enabled, self.match_count
        )
    }
}

impl From<&crate::terminal::trigger::Trigger> for PyTrigger {
    fn from(t: &crate::terminal::trigger::Trigger) -> Self {
        PyTrigger {
            id: t.id,
            name: t.name.clone(),
            pattern: t.pattern.clone(),
            enabled: t.enabled,
            fire_once_per_line: t.fire_once_per_line,
            match_count: t.match_count,
        }
    }
}

/// Trigger match result
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "TriggerMatch", from_py_object)]
#[derive(Clone)]
pub struct PyTriggerMatch {
    pub trigger_id: u64,
    pub row: usize,
    pub col: usize,
    pub end_col: usize,
    pub text: String,
    pub captures: Vec<String>,
    pub timestamp: u64,
}

#[pymethods]
impl PyTriggerMatch {
    fn __repr__(&self) -> String {
        format!(
            "TriggerMatch(trigger_id={}, row={}, col={}..{}, text={})",
            self.trigger_id, self.row, self.col, self.end_col, self.text
        )
    }
}

impl From<&crate::terminal::trigger::TriggerMatch> for PyTriggerMatch {
    fn from(m: &crate::terminal::trigger::TriggerMatch) -> Self {
        PyTriggerMatch {
            trigger_id: m.trigger_id,
            row: m.row,
            col: m.col,
            end_col: m.end_col,
            text: m.text.clone(),
            captures: m.captures.clone(),
            timestamp: m.timestamp,
        }
    }
}

/// Trigger action configuration (constructable from Python)
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "TriggerAction", from_py_object)]
#[derive(Clone)]
pub struct PyTriggerAction {
    /// Action type: "highlight", "notify", "mark_line", "set_variable",
    /// "run_command", "play_sound", "send_text", "split_pane", "stop"
    pub action_type: String,
    /// Action parameters (key-value pairs, type-specific)
    pub params: std::collections::HashMap<String, String>,
}

#[pymethods]
impl PyTriggerAction {
    /// Create a new trigger action
    ///
    /// Args:
    ///     action_type: Action type string (highlight, notify, mark_line,
    ///         set_variable, run_command, play_sound, send_text, split_pane, stop)
    ///     params: Dictionary of action parameters
    ///
    /// Returns:
    ///     A new TriggerAction instance
    ///
    /// Example:
    ///     >>> action = TriggerAction("highlight", {"bg_r": "255", "bg_g": "0", "bg_b": "0"})
    ///     >>> action = TriggerAction("notify", {"title": "Alert", "message": "Error found: $1"})
    ///     >>> action = TriggerAction("split_pane", {"direction": "horizontal", "focus_new_pane": "true"})
    #[new]
    #[pyo3(signature = (action_type, params=None))]
    fn new(action_type: String, params: Option<std::collections::HashMap<String, String>>) -> Self {
        PyTriggerAction {
            action_type,
            params: params.unwrap_or_default(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "TriggerAction(type={}, params={:?})",
            self.action_type, self.params
        )
    }
}

impl PyTriggerAction {
    /// Convert to Rust TriggerAction
    pub fn to_trigger_action(&self) -> Result<crate::terminal::trigger::TriggerAction, String> {
        use crate::terminal::trigger::TriggerAction;
        match self.action_type.as_str() {
            "highlight" => {
                let fg = self.parse_color("fg");
                let bg = self.parse_color("bg");
                let duration_ms = self
                    .params
                    .get("duration_ms")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                Ok(TriggerAction::Highlight {
                    fg,
                    bg,
                    duration_ms,
                })
            }
            "notify" => Ok(TriggerAction::Notify {
                title: self.params.get("title").cloned().unwrap_or_default(),
                message: self.params.get("message").cloned().unwrap_or_default(),
            }),
            "mark_line" => Ok(TriggerAction::MarkLine {
                label: self.params.get("label").cloned(),
                color: self.params.get("color").and_then(|c| {
                    let parts: Vec<u8> =
                        c.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                    if parts.len() == 3 {
                        Some((parts[0], parts[1], parts[2]))
                    } else {
                        None
                    }
                }),
            }),
            "set_variable" => Ok(TriggerAction::SetVariable {
                name: self.params.get("name").cloned().unwrap_or_default(),
                value: self.params.get("value").cloned().unwrap_or_default(),
            }),
            "run_command" => {
                let args: Vec<String> = self
                    .params
                    .get("args")
                    .map(|a| a.split(',').map(|s| s.trim().to_string()).collect())
                    .unwrap_or_default();
                Ok(TriggerAction::RunCommand {
                    command: self.params.get("command").cloned().unwrap_or_default(),
                    args,
                })
            }
            "play_sound" => Ok(TriggerAction::PlaySound {
                sound_id: self.params.get("sound_id").cloned().unwrap_or_default(),
                volume: self
                    .params
                    .get("volume")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(100),
            }),
            "send_text" => Ok(TriggerAction::SendText {
                text: self.params.get("text").cloned().unwrap_or_default(),
                delay_ms: self
                    .params
                    .get("delay_ms")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            }),
            "split_pane" => {
                use crate::terminal::trigger::{
                    TriggerSplitCommand, TriggerSplitDirection, TriggerSplitTarget,
                };
                let direction = match self
                    .params
                    .get("direction")
                    .map(|s| s.as_str())
                    .unwrap_or("horizontal")
                {
                    "vertical" => TriggerSplitDirection::Vertical,
                    _ => TriggerSplitDirection::Horizontal,
                };
                let focus_new_pane = self
                    .params
                    .get("focus_new_pane")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(false);
                let target = match self
                    .params
                    .get("target")
                    .map(|s| s.as_str())
                    .unwrap_or("active")
                {
                    "source" => TriggerSplitTarget::Source,
                    _ => TriggerSplitTarget::Active,
                };
                let command = match self.params.get("command_type").map(|s| s.as_str()) {
                    Some("send_text") => {
                        let text = self.params.get("command_text").cloned().unwrap_or_default();
                        let delay_ms = self
                            .params
                            .get("command_delay_ms")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        Some(TriggerSplitCommand::SendText { text, delay_ms })
                    }
                    Some("initial_command") => {
                        let command_str =
                            self.params.get("command_text").cloned().unwrap_or_default();
                        let args: Vec<String> = self
                            .params
                            .get("command_args")
                            .map(|a| a.split(',').map(|s| s.trim().to_string()).collect())
                            .unwrap_or_default();
                        Some(TriggerSplitCommand::InitialCommand {
                            command: command_str,
                            args,
                        })
                    }
                    _ => None,
                };
                Ok(TriggerAction::SplitPane {
                    direction,
                    command,
                    focus_new_pane,
                    target,
                })
            }
            "stop" => Ok(TriggerAction::StopPropagation),
            _ => Err(format!("Unknown action type: {}", self.action_type)),
        }
    }

    fn parse_color(&self, prefix: &str) -> Option<(u8, u8, u8)> {
        let r = self
            .params
            .get(&format!("{}_r", prefix))
            .and_then(|v| v.parse().ok());
        let g = self
            .params
            .get(&format!("{}_g", prefix))
            .and_then(|v| v.parse().ok());
        let b = self
            .params
            .get(&format!("{}_b", prefix))
            .and_then(|v| v.parse().ok());
        match (r, g, b) {
            (Some(r), Some(g), Some(b)) => Some((r, g, b)),
            _ => None,
        }
    }
}
