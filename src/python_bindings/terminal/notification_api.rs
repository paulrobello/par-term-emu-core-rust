//! Notification API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

/// Parse a Python notification alert string ("Desktop", "Sound(volume)",
/// "Visual"), shared by `trigger_notification` and
/// `trigger_custom_notification`.
fn parse_notification_alert(alert: &str) -> PyResult<crate::terminal::NotificationAlert> {
    use crate::terminal::NotificationAlert;

    if alert.to_lowercase() == "desktop" {
        Ok(NotificationAlert::Desktop)
    } else if alert.starts_with("Sound(") && alert.ends_with(')') {
        let vol_str = &alert[6..alert.len() - 1];
        let vol: u8 = vol_str
            .parse()
            .map_err(|_| PyValueError::new_err("Invalid sound volume"))?;
        Ok(NotificationAlert::Sound(vol))
    } else if alert.to_lowercase() == "visual" {
        Ok(NotificationAlert::Visual)
    } else {
        Err(PyValueError::new_err(
            "Invalid alert type (use 'Desktop', 'Sound(volume)', or 'Visual')",
        ))
    }
}

#[pymethods]
impl PyTerminal {
    // === Feature 37: Terminal Notifications ===

    /// Get notification configuration
    ///
    /// Returns:
    ///     NotificationConfig: Current notification settings
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_notification_config()
    ///     # NotificationConfig(bell_desktop=false, bell_visual=true, activity=false, silence=false)
    ///     ```
    fn get_notification_config(
        &self,
    ) -> PyResult<crate::python_bindings::types::PyNotificationConfig> {
        Ok(crate::python_bindings::types::PyNotificationConfig::from(
            &self.inner.get_notification_config(),
        ))
    }

    /// Set notification configuration
    ///
    /// Args:
    ///     config: NotificationConfig object with settings
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     config = term.get_notification_config()
    ///     config.bell_desktop = True
    ///     term.set_notification_config(config)
    ///     term.get_notification_config().bell_desktop   # True
    ///     ```
    fn set_notification_config(
        &mut self,
        config: &crate::python_bindings::types::PyNotificationConfig,
    ) -> PyResult<()> {
        self.inner
            .set_notification_config(crate::terminal::NotificationConfig::from(config));
        Ok(())
    }

    /// Trigger a notification
    ///
    /// Args:
    ///     trigger: Trigger type ("Bell", "Activity", "Silence", "Custom(id)")
    ///     alert: Alert type ("Desktop", "Sound(volume)", "Visual")
    ///     message: Message string, or None; the argument itself is required
    ///
    /// Raises:
    ///     ValueError: If the trigger or alert string is not recognized
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.trigger_notification("Bell", "Sound(50)", "build done")
    ///     term.get_notification_events()
    ///     # [NotificationEvent(trigger=Bell, alert=Sound(50), delivered=false)]
    ///     ```
    fn trigger_notification(
        &mut self,
        trigger: &str,
        alert: &str,
        message: Option<String>,
    ) -> PyResult<()> {
        use crate::terminal::NotificationTrigger;

        let trigger_parsed = if trigger.to_lowercase() == "bell" {
            NotificationTrigger::Bell
        } else if trigger.to_lowercase() == "activity" {
            NotificationTrigger::Activity
        } else if trigger.to_lowercase() == "silence" {
            NotificationTrigger::Silence
        } else if trigger.starts_with("Custom(") && trigger.ends_with(')') {
            let id_str = &trigger[7..trigger.len() - 1];
            let id: u32 = id_str
                .parse()
                .map_err(|_| PyValueError::new_err("Invalid custom trigger ID"))?;
            NotificationTrigger::Custom(id)
        } else {
            return Err(PyValueError::new_err(
                "Invalid trigger type (use 'Bell', 'Activity', 'Silence', or 'Custom(id)')",
            ));
        };

        let alert_parsed = parse_notification_alert(alert)?;

        self.inner
            .trigger_notification(trigger_parsed, alert_parsed, message);
        Ok(())
    }

    /// Get notification events
    ///
    /// Returns:
    ///     List of NotificationEvent objects
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.trigger_notification("Custom(7)", "Desktop", "deploy finished")
    ///     [(e.trigger, e.alert, e.message) for e in term.get_notification_events()]
    ///     # [('Custom(7)', 'Desktop', 'deploy finished')]
    ///     ```
    fn get_notification_events(
        &self,
    ) -> PyResult<Vec<crate::python_bindings::types::PyNotificationEvent>> {
        Ok(self
            .inner
            .get_notification_events()
            .iter()
            .map(crate::python_bindings::types::PyNotificationEvent::from)
            .collect())
    }

    /// Clear notification events
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.trigger_notification("Bell", "Visual", None)
    ///     term.clear_notification_events()
    ///     term.get_notification_events()   # []
    ///     ```
    fn clear_notification_events(&mut self) -> PyResult<()> {
        self.inner.clear_notification_events();
        Ok(())
    }

    /// Set maximum number of OSC 9/777 notifications to retain (0 disables buffering)
    ///
    /// Args:
    ///     max: Maximum notifications to buffer (0 disables buffering)
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.set_max_notifications(2)
    ///     term.process_str("\x1b]9;one\x07\x1b]9;two\x07\x1b]9;three\x07")
    ///     term.take_notifications()   # [('', 'two'), ('', 'three')]
    ///     ```
    fn set_max_notifications(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_notifications(max);
        Ok(())
    }

    /// Get maximum retained OSC 9/777 notifications
    ///
    /// Returns:
    ///     int: Maximum notifications buffered (0 means buffering is disabled)
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_max_notifications()   # 128
    ///     ```
    fn get_max_notifications(&self) -> PyResult<usize> {
        Ok(self.inner.max_notifications())
    }

    /// Mark a notification as delivered
    ///
    /// Args:
    ///     index: Index of the notification event; out-of-range indices are ignored
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.trigger_notification("Bell", "Visual", None)
    ///     term.mark_notification_delivered(0)
    ///     term.get_notification_events()[0].delivered   # True
    ///     ```
    fn mark_notification_delivered(&mut self, index: usize) -> PyResult<()> {
        self.inner.mark_notification_delivered(index);
        Ok(())
    }

    /// Update activity timestamp
    ///
    /// Resets the silence timer that `check_silence` measures against.
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.update_activity()
    ///     ```
    fn update_activity(&mut self) -> PyResult<()> {
        self.inner.update_activity();
        Ok(())
    }

    /// Check for silence and trigger notification if needed
    ///
    /// Queues a Silence/Visual event when silence notifications are enabled
    /// and more than `silence_threshold` seconds have passed since the last
    /// `update_activity` call and the last silence notification.
    ///
    /// Example:
    ///     ```python
    ///     import time
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     config = term.get_notification_config()
    ///     config.silence_enabled = True
    ///     config.silence_threshold = 0
    ///     term.set_notification_config(config)
    ///     time.sleep(0.01)
    ///     term.check_silence()
    ///     [(e.trigger, e.message) for e in term.get_notification_events()]
    ///     # [('Silence', 'Terminal is silent')]
    ///     ```
    fn check_silence(&mut self) -> PyResult<()> {
        self.inner.check_silence();
        Ok(())
    }

    /// Check for activity notifications
    ///
    /// Queues an Activity/Visual event when `activity_enabled` is set and
    /// `update_activity` was called since the previous event, rate-limited to
    /// at most one event per `activity_threshold` seconds.
    ///
    /// Example:
    ///     ```python
    ///     import time
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     config = term.get_notification_config()
    ///     config.activity_enabled = True
    ///     config.activity_threshold = 0
    ///     term.set_notification_config(config)
    ///     term.update_activity()
    ///     time.sleep(0.01)
    ///     term.check_activity()
    ///     [(e.trigger, e.message) for e in term.get_notification_events()]
    ///     # [('Activity', 'Terminal activity detected')]
    ///     ```
    fn check_activity(&mut self) -> PyResult<()> {
        self.inner.check_activity();
        Ok(())
    }

    /// Register a custom notification trigger
    ///
    /// Args:
    ///     id: Trigger ID
    ///     message: Message for the trigger
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.register_custom_trigger(1, "tests finished")
    ///     term.trigger_custom_notification(1, "Visual")
    ///     term.get_notification_events()[0].message   # 'tests finished'
    ///     ```
    fn register_custom_trigger(&mut self, id: u32, message: String) -> PyResult<()> {
        self.inner.register_custom_trigger(id, message);
        Ok(())
    }

    /// Trigger a custom notification
    ///
    /// Args:
    ///     id: Trigger ID
    ///     alert: Alert type ("Desktop", "Sound(volume)", "Visual")
    ///
    /// Raises:
    ///     ValueError: If the alert string is not recognized
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.register_custom_trigger(1, "tests finished")
    ///     term.trigger_custom_notification(1, "Visual")
    ///     [(e.trigger, e.alert, e.message) for e in term.get_notification_events()]
    ///     # [('Custom(1)', 'Visual', 'tests finished')]
    ///     ```
    fn trigger_custom_notification(&mut self, id: u32, alert: &str) -> PyResult<()> {
        let alert_parsed = parse_notification_alert(alert)?;

        self.inner.trigger_custom_notification(id, alert_parsed);
        Ok(())
    }

    /// Handle bell event with notification
    ///
    /// Queues a Bell event whose alert follows the config: Desktop when
    /// `bell_desktop` is set, else Sound when `bell_sound` > 0, else Visual.
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.handle_bell_notification()
    ///     [(e.trigger, e.alert, e.message) for e in term.get_notification_events()]
    ///     # [('Bell', 'Visual', 'Bell rang')]
    ///     ```
    fn handle_bell_notification(&mut self) -> PyResult<()> {
        self.inner.handle_bell_notification();
        Ok(())
    }
}
