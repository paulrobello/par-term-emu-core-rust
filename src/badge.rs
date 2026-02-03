//! Badge format support for iTerm2-style badges
//!
//! Implements OSC 1337 SetBadgeFormat sequence parsing and badge format evaluation.
//! Badges are text overlays that can display session information like hostname,
//! username, current directory, etc.
//!
//! Reference: <https://iterm2.com/documentation-badges.html>

use serde::{Deserialize, Serialize};

/// Session variables that can be interpolated into badge format strings.
///
/// Badge format strings can reference these variables using the syntax:
/// `\(session.variable_name)` or `\(variable_name)`
///
/// Example format: `\(session.username)@\(session.hostname):\(session.path)`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionVariables {
    /// The hostname of the current session
    pub hostname: Option<String>,
    /// The username of the current session
    pub username: Option<String>,
    /// The current working directory path
    pub path: Option<String>,
    /// The current job name (if available from shell integration)
    pub job: Option<String>,
    /// The last command that was executed
    pub last_command: Option<String>,
    /// The profile name being used
    pub profile_name: Option<String>,
    /// The TTY device path
    pub tty: Option<String>,
    /// Number of columns in the terminal
    pub columns: u16,
    /// Number of rows in the terminal
    pub rows: u16,
    /// Number of times the bell has rung
    pub bell_count: u32,
    /// Currently selected text (if any)
    pub selection: Option<String>,
    /// Tmux pane title (if in tmux)
    pub tmux_pane_title: Option<String>,
    /// The session name
    pub session_name: Option<String>,
    /// The window title
    pub title: Option<String>,
    /// Custom user-defined variables (key-value pairs)
    pub custom: std::collections::HashMap<String, String>,
}

impl SessionVariables {
    /// Create a new SessionVariables with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Create SessionVariables with terminal dimensions
    pub fn with_dimensions(cols: u16, rows: u16) -> Self {
        Self {
            columns: cols,
            rows,
            ..Default::default()
        }
    }

    /// Set the hostname
    pub fn set_hostname(&mut self, hostname: impl Into<String>) {
        self.hostname = Some(hostname.into());
    }

    /// Set the username
    pub fn set_username(&mut self, username: impl Into<String>) {
        self.username = Some(username.into());
    }

    /// Set the current path
    pub fn set_path(&mut self, path: impl Into<String>) {
        self.path = Some(path.into());
    }

    /// Set the current job
    pub fn set_job(&mut self, job: impl Into<String>) {
        self.job = Some(job.into());
    }

    /// Set the last command
    pub fn set_last_command(&mut self, cmd: impl Into<String>) {
        self.last_command = Some(cmd.into());
    }

    /// Set the profile name
    pub fn set_profile_name(&mut self, name: impl Into<String>) {
        self.profile_name = Some(name.into());
    }

    /// Set the TTY
    pub fn set_tty(&mut self, tty: impl Into<String>) {
        self.tty = Some(tty.into());
    }

    /// Set the terminal dimensions
    pub fn set_dimensions(&mut self, cols: u16, rows: u16) {
        self.columns = cols;
        self.rows = rows;
    }

    /// Increment the bell count
    pub fn increment_bell_count(&mut self) {
        self.bell_count = self.bell_count.saturating_add(1);
    }

    /// Set the selection text
    pub fn set_selection(&mut self, text: impl Into<String>) {
        self.selection = Some(text.into());
    }

    /// Clear the selection
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// Set the tmux pane title
    pub fn set_tmux_pane_title(&mut self, title: impl Into<String>) {
        self.tmux_pane_title = Some(title.into());
    }

    /// Set a custom variable
    pub fn set_custom(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.custom.insert(key.into(), value.into());
    }

    /// Get a variable value by name
    ///
    /// Supports both `session.variable` and just `variable` syntax
    pub fn get(&self, name: &str) -> Option<String> {
        // Strip "session." prefix if present
        let var_name = name.strip_prefix("session.").unwrap_or(name);

        match var_name {
            "hostname" => self.hostname.clone(),
            "username" => self.username.clone(),
            "path" => self.path.clone(),
            "job" => self.job.clone(),
            "lastCommand" | "last_command" => self.last_command.clone(),
            "profileName" | "profile_name" => self.profile_name.clone(),
            "tty" => self.tty.clone(),
            "columns" | "cols" => Some(self.columns.to_string()),
            "rows" => Some(self.rows.to_string()),
            "bellCount" | "bell_count" => Some(self.bell_count.to_string()),
            "selection" => self.selection.clone(),
            "tmuxPaneTitle" | "tmux_pane_title" => self.tmux_pane_title.clone(),
            "sessionName" | "session_name" | "name" => self.session_name.clone(),
            "title" => self.title.clone(),
            // Check custom variables
            other => self.custom.get(other).cloned(),
        }
    }
}

/// Error type for badge format validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadgeFormatError {
    /// Invalid base64 encoding
    Base64DecodeError(String),
    /// Invalid UTF-8 in decoded data
    Utf8Error(String),
    /// Format string contains potentially unsafe content
    UnsafeContent(String),
    /// Format string is too long
    TooLong(usize),
}

impl std::fmt::Display for BadgeFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BadgeFormatError::Base64DecodeError(e) => write!(f, "Base64 decode error: {}", e),
            BadgeFormatError::Utf8Error(e) => write!(f, "UTF-8 error: {}", e),
            BadgeFormatError::UnsafeContent(msg) => write!(f, "Unsafe content: {}", msg),
            BadgeFormatError::TooLong(len) => {
                write!(f, "Badge format too long: {} bytes (max 4096)", len)
            }
        }
    }
}

impl std::error::Error for BadgeFormatError {}

/// Maximum allowed badge format length in bytes
const MAX_BADGE_FORMAT_LENGTH: usize = 4096;

/// Decode and validate a base64-encoded badge format string
///
/// # Arguments
/// * `encoded` - Base64-encoded badge format string
///
/// # Returns
/// The decoded and validated badge format string, or an error
///
/// # Security
/// This function validates the format string to ensure it only contains
/// simple variable interpolations and literal text. It rejects formats
/// that could be used for injection attacks.
pub fn decode_badge_format(encoded: &str) -> Result<String, BadgeFormatError> {
    use base64::{engine::general_purpose::STANDARD, Engine};

    // Decode base64
    let decoded_bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|e| BadgeFormatError::Base64DecodeError(e.to_string()))?;

    // Check length limit
    if decoded_bytes.len() > MAX_BADGE_FORMAT_LENGTH {
        return Err(BadgeFormatError::TooLong(decoded_bytes.len()));
    }

    // Convert to UTF-8
    let format_str =
        String::from_utf8(decoded_bytes).map_err(|e| BadgeFormatError::Utf8Error(e.to_string()))?;

    // Validate the format string for safety
    validate_badge_format(&format_str)?;

    Ok(format_str)
}

/// Validate a badge format string for safety
///
/// Ensures the format only contains:
/// - Literal text
/// - Variable interpolations: `\(variable)` or `\(session.variable)`
///
/// Rejects formats containing:
/// - Shell command syntax (backticks, $(), etc.)
/// - Escape sequences that could be malicious
/// - Nested parentheses that could indicate complex expressions
fn validate_badge_format(format: &str) -> Result<(), BadgeFormatError> {
    // Check for shell command injection patterns
    let dangerous_patterns = [
        "`",    // Backtick command substitution
        "$(",   // Shell command substitution
        "${",   // Shell variable expansion with braces
        "$((",  // Arithmetic expansion
        "&&",   // Command chaining
        "||",   // Command chaining
        ";",    // Command separator
        "|",    // Pipe
        "<",    // Redirection
        ">",    // Redirection
        "\x1b", // Escape character
        "\x07", // Bell
        "\x00", // Null byte
    ];

    for pattern in &dangerous_patterns {
        if format.contains(pattern) {
            return Err(BadgeFormatError::UnsafeContent(format!(
                "Contains forbidden pattern: {:?}",
                pattern
            )));
        }
    }

    // Validate that \( sequences are properly formed variable references
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == '(' {
                    chars.next(); // consume '('
                                  // Read until closing ')'
                    let mut var_name = String::new();
                    let mut found_close = false;
                    for inner in chars.by_ref() {
                        if inner == ')' {
                            found_close = true;
                            break;
                        }
                        // Only allow alphanumeric, underscore, and dot in variable names
                        if !inner.is_alphanumeric() && inner != '_' && inner != '.' {
                            return Err(BadgeFormatError::UnsafeContent(format!(
                                "Invalid character '{}' in variable reference",
                                inner
                            )));
                        }
                        var_name.push(inner);
                    }
                    if !found_close {
                        return Err(BadgeFormatError::UnsafeContent(
                            "Unclosed variable reference".to_string(),
                        ));
                    }
                    if var_name.is_empty() {
                        return Err(BadgeFormatError::UnsafeContent(
                            "Empty variable reference".to_string(),
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Evaluate a badge format string by substituting session variables
///
/// # Arguments
/// * `format` - The badge format string with `\(variable)` placeholders
/// * `vars` - Session variables to substitute
///
/// # Returns
/// The evaluated string with all variables replaced by their values.
/// Unknown variables are replaced with empty strings.
///
/// # Example
/// ```
/// use par_term_emu_core_rust::badge::{SessionVariables, evaluate_badge_format};
///
/// let mut vars = SessionVariables::new();
/// vars.set_username("alice");
/// vars.set_hostname("server1");
///
/// let result = evaluate_badge_format(r"\(username)@\(hostname)", &vars);
/// assert_eq!(result, "alice@server1");
/// ```
pub fn evaluate_badge_format(format: &str, vars: &SessionVariables) -> String {
    let mut result = String::with_capacity(format.len());
    let mut chars = format.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == '(' {
                    chars.next(); // consume '('
                                  // Read variable name until ')'
                    let mut var_name = String::new();
                    for inner in chars.by_ref() {
                        if inner == ')' {
                            break;
                        }
                        var_name.push(inner);
                    }
                    // Substitute the variable
                    if let Some(value) = vars.get(&var_name) {
                        result.push_str(&value);
                    }
                    // Unknown variables are silently replaced with empty string
                    continue;
                } else if next == '\\' {
                    // Escaped backslash
                    chars.next();
                    result.push('\\');
                    continue;
                } else if next == 'n' {
                    // Newline
                    chars.next();
                    result.push('\n');
                    continue;
                } else if next == 't' {
                    // Tab
                    chars.next();
                    result.push('\t');
                    continue;
                }
            }
            // Unrecognized escape, keep as-is
            result.push(c);
        } else {
            result.push(c);
        }
    }

    result
}

/// Badge format change event
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadgeFormatChanged {
    /// The new badge format (None if cleared)
    pub format: Option<String>,
    /// Timestamp when the change occurred (Unix epoch milliseconds)
    pub timestamp: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_variables_new() {
        let vars = SessionVariables::new();
        assert!(vars.hostname.is_none());
        assert!(vars.username.is_none());
        assert_eq!(vars.columns, 0);
        assert_eq!(vars.rows, 0);
        assert_eq!(vars.bell_count, 0);
    }

    #[test]
    fn test_session_variables_setters() {
        let mut vars = SessionVariables::new();
        vars.set_hostname("myhost");
        vars.set_username("myuser");
        vars.set_path("/home/myuser");
        vars.set_dimensions(80, 24);
        vars.increment_bell_count();
        vars.increment_bell_count();

        assert_eq!(vars.hostname, Some("myhost".to_string()));
        assert_eq!(vars.username, Some("myuser".to_string()));
        assert_eq!(vars.path, Some("/home/myuser".to_string()));
        assert_eq!(vars.columns, 80);
        assert_eq!(vars.rows, 24);
        assert_eq!(vars.bell_count, 2);
    }

    #[test]
    fn test_session_variables_get() {
        let mut vars = SessionVariables::new();
        vars.set_hostname("myhost");
        vars.set_username("myuser");
        vars.set_dimensions(80, 24);

        // Direct variable names
        assert_eq!(vars.get("hostname"), Some("myhost".to_string()));
        assert_eq!(vars.get("username"), Some("myuser".to_string()));
        assert_eq!(vars.get("columns"), Some("80".to_string()));
        assert_eq!(vars.get("rows"), Some("24".to_string()));

        // With session. prefix
        assert_eq!(vars.get("session.hostname"), Some("myhost".to_string()));
        assert_eq!(vars.get("session.username"), Some("myuser".to_string()));

        // Unknown variable
        assert_eq!(vars.get("unknown"), None);
    }

    #[test]
    fn test_session_variables_custom() {
        let mut vars = SessionVariables::new();
        vars.set_custom("myvar", "myvalue");
        vars.set_custom("another", "test");

        assert_eq!(vars.get("myvar"), Some("myvalue".to_string()));
        assert_eq!(vars.get("another"), Some("test".to_string()));
    }

    #[test]
    fn test_decode_badge_format_valid() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        // Simple text
        let encoded = STANDARD.encode("Hello World");
        let result = decode_badge_format(&encoded);
        assert_eq!(result.unwrap(), "Hello World");

        // With variable interpolation
        let encoded = STANDARD.encode(r"\(username)@\(hostname)");
        let result = decode_badge_format(&encoded);
        assert_eq!(result.unwrap(), r"\(username)@\(hostname)");

        // With session prefix
        let encoded = STANDARD.encode(r"\(session.path)");
        let result = decode_badge_format(&encoded);
        assert_eq!(result.unwrap(), r"\(session.path)");
    }

    #[test]
    fn test_decode_badge_format_invalid_base64() {
        let result = decode_badge_format("not-valid-base64!!!");
        assert!(matches!(
            result,
            Err(BadgeFormatError::Base64DecodeError(_))
        ));
    }

    #[test]
    fn test_decode_badge_format_unsafe_content() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        // Shell command substitution with backticks
        let encoded = STANDARD.encode("`whoami`");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));

        // Shell command substitution with $()
        let encoded = STANDARD.encode("$(whoami)");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));

        // Pipe
        let encoded = STANDARD.encode("test | cat");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));

        // Escape sequence
        let encoded = STANDARD.encode("\x1b[31mred\x1b[0m");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));
    }

    #[test]
    fn test_decode_badge_format_invalid_variable_reference() {
        use base64::{engine::general_purpose::STANDARD, Engine};

        // Unclosed variable reference
        let encoded = STANDARD.encode(r"\(username");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));

        // Empty variable reference
        let encoded = STANDARD.encode(r"\()");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));

        // Invalid character in variable name
        let encoded = STANDARD.encode(r"\(user name)");
        let result = decode_badge_format(&encoded);
        assert!(matches!(result, Err(BadgeFormatError::UnsafeContent(_))));
    }

    #[test]
    fn test_evaluate_badge_format_simple() {
        let mut vars = SessionVariables::new();
        vars.set_username("alice");
        vars.set_hostname("server1");

        let result = evaluate_badge_format(r"\(username)@\(hostname)", &vars);
        assert_eq!(result, "alice@server1");
    }

    #[test]
    fn test_evaluate_badge_format_with_session_prefix() {
        let mut vars = SessionVariables::new();
        vars.set_username("bob");
        vars.set_path("/home/bob");

        let result = evaluate_badge_format(r"\(session.username): \(session.path)", &vars);
        assert_eq!(result, "bob: /home/bob");
    }

    #[test]
    fn test_evaluate_badge_format_unknown_variables() {
        let vars = SessionVariables::new();

        // Unknown variables are replaced with empty string
        let result = evaluate_badge_format(r"Hello \(unknown)!", &vars);
        assert_eq!(result, "Hello !");
    }

    #[test]
    fn test_evaluate_badge_format_escaped_backslash() {
        let vars = SessionVariables::new();

        let result = evaluate_badge_format(r"path\\file", &vars);
        assert_eq!(result, r"path\file");
    }

    #[test]
    fn test_evaluate_badge_format_escape_sequences() {
        let vars = SessionVariables::new();

        // Newline
        let result = evaluate_badge_format(r"line1\nline2", &vars);
        assert_eq!(result, "line1\nline2");

        // Tab
        let result = evaluate_badge_format(r"col1\tcol2", &vars);
        assert_eq!(result, "col1\tcol2");
    }

    #[test]
    fn test_evaluate_badge_format_mixed() {
        let mut vars = SessionVariables::new();
        vars.set_username("charlie");
        vars.set_path("/var/log");
        vars.set_dimensions(120, 40);

        let result = evaluate_badge_format(r"\(username) - \(path)\n\(columns)x\(rows)", &vars);
        assert_eq!(result, "charlie - /var/log\n120x40");
    }

    #[test]
    fn test_evaluate_badge_format_literal_text() {
        let vars = SessionVariables::new();

        // Just literal text with no variables
        let result = evaluate_badge_format("Production Server", &vars);
        assert_eq!(result, "Production Server");
    }

    #[test]
    fn test_validate_badge_format_valid() {
        // All these should be valid
        assert!(validate_badge_format("Hello World").is_ok());
        assert!(validate_badge_format(r"\(username)").is_ok());
        assert!(validate_badge_format(r"\(session.hostname)").is_ok());
        assert!(validate_badge_format(r"\(user_name)").is_ok());
        assert!(validate_badge_format(r"\(var123)").is_ok());
        assert!(validate_badge_format(r"Hello \(name)!").is_ok());
    }

    #[test]
    fn test_validate_badge_format_rejects_dangerous() {
        // These should all be rejected
        assert!(validate_badge_format("`cmd`").is_err());
        assert!(validate_badge_format("$(cmd)").is_err());
        assert!(validate_badge_format("${VAR}").is_err());
        assert!(validate_badge_format("a && b").is_err());
        assert!(validate_badge_format("a || b").is_err());
        assert!(validate_badge_format("a; b").is_err());
        assert!(validate_badge_format("a | b").is_err());
        assert!(validate_badge_format("a > b").is_err());
        assert!(validate_badge_format("a < b").is_err());
    }
}
