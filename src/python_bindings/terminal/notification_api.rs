//! Notification API methods for `PyTerminal` (ARC-002: split out of the
//! monolithic `#[pymethods]` block in `mod.rs`). Pure relocation — no Python API
//! or behavior change; these methods remain on the same `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 37: Terminal Notifications ===

    /// Get notification configuration
    ///
    /// Returns:
    ///     NotificationConfig: Current notification settings
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
    ///     message: Optional message string
    fn trigger_notification(
        &mut self,
        trigger: &str,
        alert: &str,
        message: Option<String>,
    ) -> PyResult<()> {
        use crate::terminal::{NotificationAlert, NotificationTrigger};

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

        let alert_parsed = if alert.to_lowercase() == "desktop" {
            NotificationAlert::Desktop
        } else if alert.starts_with("Sound(") && alert.ends_with(')') {
            let vol_str = &alert[6..alert.len() - 1];
            let vol: u8 = vol_str
                .parse()
                .map_err(|_| PyValueError::new_err("Invalid sound volume"))?;
            NotificationAlert::Sound(vol)
        } else if alert.to_lowercase() == "visual" {
            NotificationAlert::Visual
        } else {
            return Err(PyValueError::new_err(
                "Invalid alert type (use 'Desktop', 'Sound(volume)', or 'Visual')",
            ));
        };

        self.inner
            .trigger_notification(trigger_parsed, alert_parsed, message);
        Ok(())
    }

    /// Get notification events
    ///
    /// Returns:
    ///     List of NotificationEvent objects
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
    fn clear_notification_events(&mut self) -> PyResult<()> {
        self.inner.clear_notification_events();
        Ok(())
    }

    /// Set maximum number of OSC 9/777 notifications to retain (0 disables buffering)
    fn set_max_notifications(&mut self, max: usize) -> PyResult<()> {
        self.inner.set_max_notifications(max);
        Ok(())
    }

    /// Get maximum retained OSC 9/777 notifications
    fn get_max_notifications(&self) -> PyResult<usize> {
        Ok(self.inner.max_notifications())
    }

    /// Mark a notification as delivered
    ///
    /// Args:
    ///     index: Index of the notification event
    fn mark_notification_delivered(&mut self, index: usize) -> PyResult<()> {
        self.inner.mark_notification_delivered(index);
        Ok(())
    }

    /// Update activity timestamp
    fn update_activity(&mut self) -> PyResult<()> {
        self.inner.update_activity();
        Ok(())
    }

    /// Check for silence and trigger notification if needed
    fn check_silence(&mut self) -> PyResult<()> {
        self.inner.check_silence();
        Ok(())
    }

    /// Check for activity and trigger notification if needed
    fn check_activity(&mut self) -> PyResult<()> {
        self.inner.check_activity();
        Ok(())
    }

    /// Register a custom notification trigger
    ///
    /// Args:
    ///     id: Trigger ID
    ///     message: Message for the trigger
    fn register_custom_trigger(&mut self, id: u32, message: String) -> PyResult<()> {
        self.inner.register_custom_trigger(id, message);
        Ok(())
    }

    /// Trigger a custom notification
    ///
    /// Args:
    ///     id: Trigger ID
    ///     alert: Alert type ("Desktop", "Sound(volume)", "Visual")
    fn trigger_custom_notification(&mut self, id: u32, alert: &str) -> PyResult<()> {
        use crate::terminal::NotificationAlert;

        let alert_parsed = if alert.to_lowercase() == "desktop" {
            NotificationAlert::Desktop
        } else if alert.starts_with("Sound(") && alert.ends_with(')') {
            let vol_str = &alert[6..alert.len() - 1];
            let vol: u8 = vol_str
                .parse()
                .map_err(|_| PyValueError::new_err("Invalid sound volume"))?;
            NotificationAlert::Sound(vol)
        } else if alert.to_lowercase() == "visual" {
            NotificationAlert::Visual
        } else {
            return Err(PyValueError::new_err(
                "Invalid alert type (use 'Desktop', 'Sound(volume)', or 'Visual')",
            ));
        };

        self.inner.trigger_custom_notification(id, alert_parsed);
        Ok(())
    }

    /// Handle bell event with notification
    fn handle_bell_notification(&mut self) -> PyResult<()> {
        self.inner.handle_bell_notification();
        Ok(())
    }
}
