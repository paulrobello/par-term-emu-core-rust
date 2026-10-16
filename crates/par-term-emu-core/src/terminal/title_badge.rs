//! Window title, OSC 1337 badge format, and user/session variables (ARC-003).

use super::Terminal;
use std::collections::HashMap;

impl Terminal {
    /// Get the title
    pub fn title(&self) -> &str {
        &self.title_state.title
    }

    /// Set the title
    pub fn set_title(&mut self, title: String) {
        // Also update session variables for badge evaluation
        self.badge_state.session_variables.title = Some(title.clone());
        self.title_state.title = title;
    }

    // === Badge Format Support ===

    /// Get the current badge format template
    ///
    /// Returns the badge format string if one has been set via OSC 1337 SetBadgeFormat.
    /// The format may contain `\(variable)` placeholders for session variables.
    pub fn badge_format(&self) -> Option<&str> {
        self.badge_state.badge_format.as_deref()
    }

    /// Set the badge format template
    ///
    /// This method is typically called when processing OSC 1337 SetBadgeFormat sequences.
    /// The format string should contain `\(variable)` placeholders.
    pub fn set_badge_format(&mut self, format: Option<String>) {
        self.badge_state.badge_format = format;
    }

    /// Clear the badge format
    pub fn clear_badge_format(&mut self) {
        self.badge_state.badge_format = None;
    }

    /// Get a reference to the session variables
    ///
    /// Session variables are used for badge format evaluation.
    pub fn session_variables(&self) -> &crate::badge::SessionVariables {
        &self.badge_state.session_variables
    }

    /// Get a mutable reference to the session variables
    ///
    /// Use this to update session variables that will be used in badge evaluation.
    pub fn session_variables_mut(&mut self) -> &mut crate::badge::SessionVariables {
        &mut self.badge_state.session_variables
    }

    /// Evaluate the current badge format with session variables
    ///
    /// Returns the evaluated badge string with all variables substituted,
    /// or None if no badge format is set.
    ///
    /// # Example
    /// ```ignore
    /// terminal.set_badge_format(Some(r"\(username)@\(hostname)".to_string()));
    /// terminal.session_variables_mut().set_username("alice");
    /// terminal.session_variables_mut().set_hostname("server1");
    /// assert_eq!(terminal.evaluate_badge(), Some("alice@server1".to_string()));
    /// ```
    pub fn evaluate_badge(&self) -> Option<String> {
        self.badge_state.evaluate()
    }

    /// Get a user variable by name
    ///
    /// Returns the value of a user variable set via OSC 1337 SetUserVar,
    /// or None if the variable is not set.
    pub fn get_user_var(&self, name: &str) -> Option<&str> {
        self.badge_state
            .session_variables
            .custom
            .get(name)
            .map(|s| s.as_str())
    }

    /// Get all user variables as a reference to the HashMap
    pub fn get_user_vars(&self) -> &HashMap<String, String> {
        &self.badge_state.session_variables.custom
    }

    /// Set a user variable, emitting a UserVarChanged event if the value changed
    pub fn set_user_var(&mut self, name: String, value: String) {
        self.badge_state.set_user_var(name, value, &mut self.events);
    }
}
