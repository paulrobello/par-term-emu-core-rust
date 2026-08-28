//! Recording, macro, and coprocess types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Coprocess configuration (constructable from Python)
#[pyclass(name = "CoprocessConfig", from_py_object)]
#[derive(Clone)]
pub struct PyCoprocessConfig {
    /// Command to execute
    #[pyo3(get, set)]
    pub command: String,
    /// Command arguments
    #[pyo3(get, set)]
    pub args: Vec<String>,
    /// Working directory (None = inherit)
    #[pyo3(get, set)]
    pub cwd: Option<String>,
    /// Environment variables for the coprocess
    #[pyo3(get, set)]
    pub env: std::collections::HashMap<String, String>,
    /// Whether terminal output is piped to the coprocess stdin
    #[pyo3(get, set)]
    pub copy_terminal_output: bool,
    /// Restart policy: "never" (default), "always", or "on_failure"
    #[pyo3(get, set)]
    pub restart_policy: String,
    /// Delay in milliseconds before restarting (0 = immediate)
    #[pyo3(get, set)]
    pub restart_delay_ms: u64,
}

#[pymethods]
impl PyCoprocessConfig {
    /// Create a new coprocess configuration
    ///
    /// Args:
    ///     command: Command to execute
    ///     args: Optional list of command arguments
    ///     cwd: Optional working directory
    ///     env: Optional environment variables dictionary
    ///     copy_terminal_output: Whether to pipe terminal output to stdin (default: True)
    ///     restart_policy: Restart policy - "never" (default), "always", or "on_failure"
    ///     restart_delay_ms: Delay in milliseconds before restarting (default: 0)
    ///
    /// Returns:
    ///     A new CoprocessConfig instance
    ///
    /// Example:
    ///     >>> config = CoprocessConfig("grep", args=["ERROR"])
    ///     >>> config = CoprocessConfig("cat", copy_terminal_output=True)
    ///     >>> config = CoprocessConfig("watcher", restart_policy="always", restart_delay_ms=1000)
    #[new]
    #[pyo3(signature = (command, args=None, cwd=None, env=None, copy_terminal_output=true, restart_policy="never", restart_delay_ms=0))]
    fn new(
        command: String,
        args: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<std::collections::HashMap<String, String>>,
        copy_terminal_output: bool,
        restart_policy: &str,
        restart_delay_ms: u64,
    ) -> Self {
        PyCoprocessConfig {
            command,
            args: args.unwrap_or_default(),
            cwd,
            env: env.unwrap_or_default(),
            copy_terminal_output,
            restart_policy: restart_policy.to_string(),
            restart_delay_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "CoprocessConfig(command={}, args={:?}, copy_output={}, restart_policy={}, restart_delay_ms={})",
            self.command, self.args, self.copy_terminal_output, self.restart_policy, self.restart_delay_ms
        )
    }
}

impl From<&PyCoprocessConfig> for crate::coprocess::CoprocessConfig {
    fn from(config: &PyCoprocessConfig) -> Self {
        use crate::coprocess::RestartPolicy;
        let restart_policy = match config.restart_policy.as_str() {
            "always" => RestartPolicy::Always,
            "on_failure" => RestartPolicy::OnFailure,
            _ => RestartPolicy::Never,
        };
        crate::coprocess::CoprocessConfig {
            command: config.command.clone(),
            args: config.args.clone(),
            cwd: config.cwd.clone(),
            env: config.env.clone(),
            copy_terminal_output: config.copy_terminal_output,
            restart_policy,
            restart_delay_ms: config.restart_delay_ms,
        }
    }
}

/// Recording event
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "RecordingEvent", from_py_object)]
#[derive(Clone)]
pub struct PyRecordingEvent {
    /// Milliseconds since recording start
    pub timestamp: u64,
    /// Event kind: "Input", "Output", "Resize", "Metadata", or "Marker"
    pub event_type: String,
    /// Raw event payload bytes
    pub data: Vec<u8>,
    /// Event-specific metadata, e.g. (cols, rows) for resize events
    pub metadata: Option<(usize, usize)>,
}

#[pymethods]
impl PyRecordingEvent {
    fn __repr__(&self) -> String {
        format!(
            "RecordingEvent(type={}, timestamp={}ms, data_len={})",
            self.event_type,
            self.timestamp,
            self.data.len()
        )
    }

    /// Get event data as string
    fn get_data_str(&self) -> String {
        String::from_utf8_lossy(&self.data).to_string()
    }
}

impl From<&crate::terminal::RecordingEvent> for PyRecordingEvent {
    fn from(event: &crate::terminal::RecordingEvent) -> Self {
        let event_type = match event.event_type {
            crate::terminal::RecordingEventType::Input => "Input".to_string(),
            crate::terminal::RecordingEventType::Output => "Output".to_string(),
            crate::terminal::RecordingEventType::Resize => "Resize".to_string(),
            crate::terminal::RecordingEventType::Metadata => "Metadata".to_string(),
            crate::terminal::RecordingEventType::Marker => "Marker".to_string(),
        };

        PyRecordingEvent {
            timestamp: event.timestamp,
            event_type,
            data: event.data.clone(),
            metadata: event.metadata,
        }
    }
}

/// Recording session
#[pyclass(name = "RecordingSession", from_py_object)]
#[derive(Clone)]
pub struct PyRecordingSession {
    pub(crate) inner: crate::terminal::RecordingSession,
}

#[pymethods]
impl PyRecordingSession {
    fn __repr__(&self) -> String {
        format!(
            "RecordingSession(duration={}ms, size={:?}, events={})",
            self.inner.duration,
            self.inner.initial_size,
            self.inner.events.len()
        )
    }

    /// Get recording size (cols, rows)
    fn get_size(&self) -> (usize, usize) {
        self.inner.initial_size
    }

    /// Get duration in seconds
    fn get_duration_seconds(&self) -> f64 {
        self.inner.duration as f64 / 1000.0
    }

    /// Unix epoch milliseconds when recording started
    #[getter]
    fn created_at(&self) -> u64 {
        self.inner.created_at
    }

    /// Terminal size when recording started, as (cols, rows)
    #[getter]
    fn initial_size(&self) -> (usize, usize) {
        self.inner.initial_size
    }

    /// Recording duration in milliseconds
    #[getter]
    fn duration(&self) -> u64 {
        self.inner.duration
    }

    /// Recording title
    #[getter]
    fn title(&self) -> Option<String> {
        Some(self.inner.title.clone())
    }

    /// Number of events in the recording
    #[getter]
    fn event_count(&self) -> usize {
        self.inner.events.len()
    }

    /// Get all recorded events
    #[getter]
    fn events(&self) -> Vec<PyRecordingEvent> {
        self.inner
            .events
            .iter()
            .map(PyRecordingEvent::from)
            .collect()
    }

    /// Get captured environment variables
    #[getter]
    fn env(&self) -> std::collections::HashMap<String, String> {
        self.inner.env.clone()
    }
}

impl From<&crate::terminal::RecordingSession> for PyRecordingSession {
    fn from(session: &crate::terminal::RecordingSession) -> Self {
        PyRecordingSession {
            inner: session.clone(),
        }
    }
}

impl From<crate::terminal::RecordingSession> for PyRecordingSession {
    fn from(session: crate::terminal::RecordingSession) -> Self {
        PyRecordingSession { inner: session }
    }
}

/// Macro event
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "MacroEvent", from_py_object)]
#[derive(Clone)]
pub struct PyMacroEvent {
    /// Event kind: "KeyPress", "Delay", or "Screenshot"
    pub event_type: String,
    /// Milliseconds since macro start
    pub timestamp: u64,
    /// Key name for KeyPress events (e.g. "enter", "ctrl+c")
    pub key: Option<String>,
    /// Delay duration in milliseconds for Delay events
    pub duration: Option<u64>,
    /// Label for Screenshot events
    pub label: Option<String>,
}

#[pymethods]
impl PyMacroEvent {
    fn __repr__(&self) -> String {
        match self.event_type.as_str() {
            "key" => format!(
                "MacroEvent(key={}, timestamp={}ms)",
                self.key.as_ref().unwrap(),
                self.timestamp
            ),
            "delay" => format!(
                "MacroEvent(delay={}ms, timestamp={}ms)",
                self.duration.unwrap(),
                self.timestamp
            ),
            "screenshot" => format!(
                "MacroEvent(screenshot, label={:?}, timestamp={}ms)",
                self.label, self.timestamp
            ),
            _ => "MacroEvent(unknown)".to_string(),
        }
    }
}

impl From<&crate::macros::MacroEvent> for PyMacroEvent {
    fn from(event: &crate::macros::MacroEvent) -> Self {
        match event {
            crate::macros::MacroEvent::KeyPress { key, timestamp } => PyMacroEvent {
                event_type: "key".to_string(),
                timestamp: *timestamp,
                key: Some(key.clone()),
                duration: None,
                label: None,
            },
            crate::macros::MacroEvent::Delay {
                duration,
                timestamp,
            } => PyMacroEvent {
                event_type: "delay".to_string(),
                timestamp: *timestamp,
                key: None,
                duration: Some(*duration),
                label: None,
            },
            crate::macros::MacroEvent::Screenshot { label, timestamp } => PyMacroEvent {
                event_type: "screenshot".to_string(),
                timestamp: *timestamp,
                key: None,
                duration: None,
                label: label.clone(),
            },
        }
    }
}

/// Macro recording
#[pyclass(name = "Macro", from_py_object)]
#[derive(Clone)]
pub struct PyMacro {
    pub(crate) inner: crate::macros::Macro,
}

#[pymethods]
impl PyMacro {
    /// Create a new macro
    #[new]
    fn new(name: String) -> Self {
        PyMacro {
            inner: crate::macros::Macro::new(name),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Macro(name={}, duration={}ms, events={})",
            self.inner.name,
            self.inner.duration,
            self.inner.events.len()
        )
    }

    /// Add a key press event
    fn add_key(&mut self, key: String) {
        self.inner.add_key(key);
    }

    /// Add a delay event
    fn add_delay(&mut self, duration_ms: u64) {
        self.inner.add_delay(duration_ms);
    }

    /// Add a screenshot trigger
    fn add_screenshot(&mut self, label: Option<String>) {
        self.inner.add_screenshot_labeled(label);
    }

    /// Set description
    fn set_description(&mut self, description: String) {
        self.inner.description = Some(description);
    }

    /// Save to YAML file
    fn save_yaml(&self, path: String) -> PyResult<()> {
        self.inner
            .save_yaml(path)
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Load from YAML file
    #[staticmethod]
    fn load_yaml(path: String) -> PyResult<Self> {
        crate::macros::Macro::load_yaml(path)
            .map(|inner| PyMacro { inner })
            .map_err(|e| pyo3::exceptions::PyIOError::new_err(e.to_string()))
    }

    /// Convert to YAML string
    fn to_yaml(&self) -> PyResult<String> {
        self.inner
            .to_yaml()
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    /// Parse from YAML string
    #[staticmethod]
    fn from_yaml(yaml: String) -> PyResult<Self> {
        crate::macros::Macro::from_yaml(&yaml)
            .map(|inner| PyMacro { inner })
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    /// Get macro name
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Get description
    #[getter]
    fn description(&self) -> Option<String> {
        self.inner.description.clone()
    }

    /// Get duration in milliseconds
    #[getter]
    fn duration(&self) -> u64 {
        self.inner.duration
    }

    /// Get terminal size (cols, rows)
    #[getter]
    fn terminal_size(&self) -> Option<(usize, usize)> {
        self.inner.terminal_size
    }

    /// Get number of events
    #[getter]
    fn event_count(&self) -> usize {
        self.inner.events.len()
    }

    /// Get all events
    #[getter]
    fn events(&self) -> Vec<PyMacroEvent> {
        self.inner.events.iter().map(PyMacroEvent::from).collect()
    }
}

impl From<crate::macros::Macro> for PyMacro {
    fn from(macro_data: crate::macros::Macro) -> Self {
        PyMacro { inner: macro_data }
    }
}
