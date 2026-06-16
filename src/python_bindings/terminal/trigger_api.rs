//! Trigger & automation API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 18: Triggers & Automation ===

    /// Add a new trigger with a regex pattern and actions
    ///
    /// Args:
    ///     name: Human-readable trigger name
    ///     pattern: Regex pattern to match against terminal output lines
    ///     actions: List of TriggerAction objects defining what happens on match
    ///
    /// Returns:
    ///     int: Trigger ID for future reference
    ///
    /// Example:
    ///     >>> action = TriggerAction("highlight", {"bg_r": "255", "bg_g": "0", "bg_b": "0"})
    ///     >>> trigger_id = term.add_trigger("errors", r"ERROR:\s+(.+)", [action])
    fn add_trigger(
        &mut self,
        name: String,
        pattern: String,
        actions: Vec<crate::python_bindings::types::PyTriggerAction>,
    ) -> PyResult<u64> {
        let rust_actions: Result<Vec<_>, _> =
            actions.iter().map(|a| a.to_trigger_action()).collect();
        let rust_actions =
            rust_actions.map_err(|e| PyValueError::new_err(format!("Invalid action: {}", e)))?;
        self.inner
            .add_trigger(name, pattern, rust_actions)
            .map_err(PyValueError::new_err)
    }

    /// Remove a trigger by ID
    ///
    /// Args:
    ///     trigger_id: ID of the trigger to remove
    ///
    /// Returns:
    ///     bool: True if trigger was found and removed
    fn remove_trigger(&mut self, trigger_id: u64) -> PyResult<bool> {
        Ok(self.inner.remove_trigger(trigger_id))
    }

    /// Enable or disable a trigger
    ///
    /// Args:
    ///     trigger_id: ID of the trigger
    ///     enabled: Whether to enable (True) or disable (False)
    ///
    /// Returns:
    ///     bool: True if trigger was found and updated
    fn set_trigger_enabled(&mut self, trigger_id: u64, enabled: bool) -> PyResult<bool> {
        Ok(self.inner.set_trigger_enabled(trigger_id, enabled))
    }

    /// List all registered triggers
    ///
    /// Returns:
    ///     list[Trigger]: List of all triggers
    fn list_triggers(&self) -> PyResult<Vec<crate::python_bindings::types::PyTrigger>> {
        Ok(self
            .inner
            .list_triggers()
            .iter()
            .map(|t| crate::python_bindings::types::PyTrigger::from(*t))
            .collect())
    }

    /// Get a trigger by ID
    ///
    /// Args:
    ///     trigger_id: ID of the trigger
    ///
    /// Returns:
    ///     Trigger | None: Trigger if found, None otherwise
    fn get_trigger(
        &self,
        trigger_id: u64,
    ) -> PyResult<Option<crate::python_bindings::types::PyTrigger>> {
        Ok(self
            .inner
            .get_trigger(trigger_id)
            .map(crate::python_bindings::types::PyTrigger::from))
    }

    /// Drain all pending trigger match events
    ///
    /// Returns:
    ///     list[TriggerMatch]: List of matches since last poll
    ///
    /// Example:
    ///     >>> matches = term.poll_trigger_matches()
    ///     >>> for m in matches:
    ///     ...     print(f"Trigger {m.trigger_id} matched '{m.text}' at row {m.row}")
    fn poll_trigger_matches(
        &mut self,
    ) -> PyResult<Vec<crate::python_bindings::types::PyTriggerMatch>> {
        Ok(self
            .inner
            .poll_trigger_matches()
            .iter()
            .map(crate::python_bindings::types::PyTriggerMatch::from)
            .collect())
    }

    /// Process trigger scans on dirty rows
    ///
    /// Called automatically in PTY mode. Use manually for non-PTY terminals.
    fn process_trigger_scans(&mut self) -> PyResult<()> {
        self.inner.process_trigger_scans();
        Ok(())
    }

    /// Get active trigger highlights (filters expired ones)
    ///
    /// Returns:
    ///     list[tuple]: List of (row, col_start, col_end, fg, bg) tuples
    ///         where fg and bg are optional (r, g, b) tuples
    #[allow(clippy::type_complexity)]
    fn get_trigger_highlights(
        &self,
    ) -> PyResult<
        Vec<(
            usize,
            usize,
            usize,
            Option<(u8, u8, u8)>,
            Option<(u8, u8, u8)>,
        )>,
    > {
        Ok(self
            .inner
            .get_trigger_highlights()
            .iter()
            .map(|h| (h.row, h.col_start, h.col_end, h.fg, h.bg))
            .collect())
    }

    /// Clear all trigger highlights
    fn clear_trigger_highlights(&mut self) -> PyResult<()> {
        self.inner.clear_trigger_highlights();
        Ok(())
    }

    /// Drain pending action results for frontend consumption
    ///
    /// Returns:
    ///     list[dict]: List of action result dicts with 'type' and action-specific fields
    fn poll_action_results(&mut self) -> PyResult<Vec<std::collections::HashMap<String, String>>> {
        use crate::terminal::trigger::ActionResult;
        Ok(self
            .inner
            .poll_action_results()
            .iter()
            .map(|ar| {
                let mut map = std::collections::HashMap::new();
                match ar {
                    ActionResult::RunCommand {
                        trigger_id,
                        command,
                        args,
                    } => {
                        map.insert("type".to_string(), "run_command".to_string());
                        map.insert("trigger_id".to_string(), trigger_id.to_string());
                        map.insert("command".to_string(), command.clone());
                        map.insert("args".to_string(), args.join(","));
                    }
                    ActionResult::PlaySound {
                        trigger_id,
                        sound_id,
                        volume,
                    } => {
                        map.insert("type".to_string(), "play_sound".to_string());
                        map.insert("trigger_id".to_string(), trigger_id.to_string());
                        map.insert("sound_id".to_string(), sound_id.clone());
                        map.insert("volume".to_string(), volume.to_string());
                    }
                    ActionResult::SendText {
                        trigger_id,
                        text,
                        delay_ms,
                    } => {
                        map.insert("type".to_string(), "send_text".to_string());
                        map.insert("trigger_id".to_string(), trigger_id.to_string());
                        map.insert("text".to_string(), text.clone());
                        map.insert("delay_ms".to_string(), delay_ms.to_string());
                    }
                    ActionResult::Notify {
                        trigger_id,
                        title,
                        message,
                    } => {
                        map.insert("type".to_string(), "notify".to_string());
                        map.insert("trigger_id".to_string(), trigger_id.to_string());
                        map.insert("title".to_string(), title.clone());
                        map.insert("message".to_string(), message.clone());
                    }
                    ActionResult::MarkLine {
                        trigger_id,
                        row,
                        label,
                        color,
                    } => {
                        map.insert("type".to_string(), "mark_line".to_string());
                        map.insert("trigger_id".to_string(), trigger_id.to_string());
                        map.insert("row".to_string(), row.to_string());
                        if let Some(l) = label {
                            map.insert("label".to_string(), l.clone());
                        }
                        if let Some((r, g, b)) = color {
                            map.insert("color".to_string(), format!("{},{},{}", r, g, b));
                        }
                    }
                    ActionResult::SplitPane {
                        trigger_id,
                        direction,
                        focus_new_pane,
                        target,
                        source_pane_id,
                        ..
                    } => {
                        use crate::terminal::trigger::{TriggerSplitDirection, TriggerSplitTarget};
                        map.insert("type".to_string(), "split_pane".to_string());
                        map.insert("trigger_id".to_string(), trigger_id.to_string());
                        map.insert(
                            "direction".to_string(),
                            match direction {
                                TriggerSplitDirection::Horizontal => "horizontal".to_string(),
                                TriggerSplitDirection::Vertical => "vertical".to_string(),
                            },
                        );
                        map.insert("focus_new_pane".to_string(), focus_new_pane.to_string());
                        map.insert(
                            "target".to_string(),
                            match target {
                                TriggerSplitTarget::Active => "active".to_string(),
                                TriggerSplitTarget::Source => "source".to_string(),
                            },
                        );
                        if let Some(pane_id) = source_pane_id {
                            map.insert("source_pane_id".to_string(), pane_id.to_string());
                        }
                    }
                }
                map
            })
            .collect())
    }
}
