//! Performance, profiling, benchmark, and compliance types.
//!
//! Split from the former monolithic `types.rs`.

use pyo3::prelude::*;

/// Performance metrics
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "PerformanceMetrics", from_py_object)]
#[derive(Clone)]
pub struct PyPerformanceMetrics {
    /// Number of frames (process() batches) rendered
    pub frames_rendered: u64,
    /// Total number of cells updated
    pub cells_updated: u64,
    /// Total input bytes processed
    pub bytes_processed: u64,
    /// Total time spent processing, in microseconds
    pub total_processing_us: u64,
    /// Slowest single frame, in microseconds
    pub peak_frame_us: u64,
    /// Number of scroll operations performed
    pub scroll_count: u64,
    /// Number of line wraps performed
    pub wrap_count: u64,
    /// Number of escape sequences processed
    pub escape_sequences: u64,
}

#[pymethods]
impl PyPerformanceMetrics {
    fn __repr__(&self) -> String {
        format!(
            "PerformanceMetrics(frames={}, cells={}, fps={:.1})",
            self.frames_rendered,
            self.cells_updated,
            if self.total_processing_us > 0 {
                1_000_000.0 * self.frames_rendered as f64 / self.total_processing_us as f64
            } else {
                0.0
            }
        )
    }
}

/// Frame timing
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "FrameTiming", from_py_object)]
#[derive(Clone)]
pub struct PyFrameTiming {
    /// Sequential frame index
    pub frame_number: u64,
    /// Time to process this frame, in microseconds
    pub processing_us: u64,
    /// Number of cells updated in this frame
    pub cells_updated: usize,
    /// Number of bytes processed in this frame
    pub bytes_processed: usize,
}

#[pymethods]
impl PyFrameTiming {
    fn __repr__(&self) -> String {
        format!(
            "FrameTiming(frame={}, time={}us, cells={})",
            self.frame_number, self.processing_us, self.cells_updated
        )
    }
}

/// Escape sequence profile
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "EscapeSequenceProfile", from_py_object)]
#[derive(Clone)]
pub struct PyEscapeSequenceProfile {
    /// Number of sequences of this category processed
    pub count: u64,
    /// Total processing time, in microseconds
    pub total_time_us: u64,
    /// Slowest single sequence, in microseconds
    pub peak_time_us: u64,
    /// Average processing time, in microseconds
    pub avg_time_us: u64,
}

#[pymethods]
impl PyEscapeSequenceProfile {
    fn __repr__(&self) -> String {
        format!(
            "EscapeSequenceProfile(count={}, avg_us={}, peak_us={})",
            self.count, self.avg_time_us, self.peak_time_us
        )
    }
}

impl From<&crate::terminal::EscapeSequenceProfile> for PyEscapeSequenceProfile {
    fn from(profile: &crate::terminal::EscapeSequenceProfile) -> Self {
        PyEscapeSequenceProfile {
            count: profile.count,
            total_time_us: profile.total_time_us,
            peak_time_us: profile.peak_time_us,
            avg_time_us: profile.avg_time_us,
        }
    }
}

/// Profiling data
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ProfilingData", from_py_object)]
#[derive(Clone)]
pub struct PyProfilingData {
    /// Per-category escape sequence profiles
    pub categories: std::collections::HashMap<String, PyEscapeSequenceProfile>,
    /// Total number of allocations
    pub allocations: u64,
    /// Total bytes allocated
    pub bytes_allocated: u64,
    /// Peak memory usage in bytes
    pub peak_memory: usize,
}

#[pymethods]
impl PyProfilingData {
    fn __repr__(&self) -> String {
        format!(
            "ProfilingData(categories={}, allocations={}, peak_memory={})",
            self.categories.len(),
            self.allocations,
            self.peak_memory
        )
    }
}

impl From<&crate::terminal::ProfilingData> for PyProfilingData {
    fn from(data: &crate::terminal::ProfilingData) -> Self {
        use crate::terminal::ProfileCategory;

        let mut categories = std::collections::HashMap::new();
        for (cat, profile) in &data.categories {
            let key = match cat {
                ProfileCategory::CSI => "csi",
                ProfileCategory::OSC => "osc",
                ProfileCategory::ESC => "esc",
                ProfileCategory::DCS => "dcs",
                ProfileCategory::Print => "print",
                ProfileCategory::Control => "control",
            }
            .to_string();
            categories.insert(key, PyEscapeSequenceProfile::from(profile));
        }

        PyProfilingData {
            categories,
            allocations: data.allocations,
            bytes_allocated: data.bytes_allocated,
            peak_memory: data.peak_memory,
        }
    }
}

/// Benchmark result
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "BenchmarkResult", from_py_object)]
#[derive(Clone)]
pub struct PyBenchmarkResult {
    /// Benchmark category (e.g. "parsing", "rendering")
    pub category: String,
    /// Benchmark name
    pub name: String,
    /// Number of iterations run
    pub iterations: u64,
    /// Total benchmark time, in microseconds
    pub total_time_us: u64,
    /// Average iteration time, in microseconds
    pub avg_time_us: u64,
    /// Fastest iteration time, in microseconds
    pub min_time_us: u64,
    /// Slowest iteration time, in microseconds
    pub max_time_us: u64,
    /// Throughput in operations per second
    pub ops_per_sec: f64,
    /// Peak memory used during the benchmark, in bytes (if measured)
    pub memory_bytes: Option<usize>,
}

#[pymethods]
impl PyBenchmarkResult {
    fn __repr__(&self) -> String {
        format!(
            "BenchmarkResult(category={}, name={}, iterations={}, avg_us={}, ops/sec={:.0})",
            self.category, self.name, self.iterations, self.avg_time_us, self.ops_per_sec
        )
    }
}

impl From<&crate::terminal::BenchmarkResult> for PyBenchmarkResult {
    fn from(result: &crate::terminal::BenchmarkResult) -> Self {
        use crate::terminal::BenchmarkCategory;

        let category = match result.category {
            BenchmarkCategory::Rendering => "rendering",
            BenchmarkCategory::Parsing => "parsing",
            BenchmarkCategory::GridOps => "gridops",
            BenchmarkCategory::Scrollback => "scrollback",
            BenchmarkCategory::Memory => "memory",
            BenchmarkCategory::Throughput => "throughput",
        }
        .to_string();

        PyBenchmarkResult {
            category,
            name: result.name.clone(),
            iterations: result.iterations,
            total_time_us: result.total_time_us,
            avg_time_us: result.avg_time_us,
            min_time_us: result.min_time_us,
            max_time_us: result.max_time_us,
            ops_per_sec: result.ops_per_sec,
            memory_bytes: result.memory_bytes,
        }
    }
}

/// Benchmark suite
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "BenchmarkSuite", from_py_object)]
#[derive(Clone)]
pub struct PyBenchmarkSuite {
    /// Individual benchmark results
    pub results: Vec<PyBenchmarkResult>,
    /// Total suite wall time, in milliseconds
    pub total_time_ms: u64,
    /// Suite name
    pub suite_name: String,
}

#[pymethods]
impl PyBenchmarkSuite {
    fn __repr__(&self) -> String {
        format!(
            "BenchmarkSuite(name={}, tests={}, time={}ms)",
            self.suite_name,
            self.results.len(),
            self.total_time_ms
        )
    }
}

impl From<&crate::terminal::BenchmarkSuite> for PyBenchmarkSuite {
    fn from(suite: &crate::terminal::BenchmarkSuite) -> Self {
        PyBenchmarkSuite {
            results: suite.results.iter().map(PyBenchmarkResult::from).collect(),
            total_time_ms: suite.total_time_ms,
            suite_name: suite.suite_name.clone(),
        }
    }
}

/// Compliance test
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ComplianceTest", from_py_object)]
#[derive(Clone)]
pub struct PyComplianceTest {
    /// Test name
    pub name: String,
    /// Test category
    pub category: String,
    /// Whether the test passed
    pub passed: bool,
    /// Expected value (rendered as string)
    pub expected: String,
    /// Actual value (rendered as string)
    pub actual: String,
    /// Additional notes about the test
    pub notes: Option<String>,
}

#[pymethods]
impl PyComplianceTest {
    fn __repr__(&self) -> String {
        format!(
            "ComplianceTest(name={}, category={}, passed={})",
            self.name, self.category, self.passed
        )
    }
}

impl From<&crate::terminal::ComplianceTest> for PyComplianceTest {
    fn from(test: &crate::terminal::ComplianceTest) -> Self {
        PyComplianceTest {
            name: test.name.clone(),
            category: test.category.clone(),
            passed: test.passed,
            expected: test.expected.clone(),
            actual: test.actual.clone(),
            notes: test.notes.clone(),
        }
    }
}

/// Compliance report
#[par_term_emu_derive::pyo3_get_all]
#[pyclass(name = "ComplianceReport", from_py_object)]
#[derive(Clone)]
pub struct PyComplianceReport {
    /// Description of the terminal under test
    pub terminal_info: String,
    /// VT conformance level tested against
    pub level: String,
    /// Individual test results
    pub tests: Vec<PyComplianceTest>,
    /// Number of tests passed
    pub passed: usize,
    /// Number of tests failed
    pub failed: usize,
    /// Percentage of tests passed (0.0-100.0)
    pub compliance_percent: f64,
}

#[pymethods]
impl PyComplianceReport {
    fn __repr__(&self) -> String {
        format!(
            "ComplianceReport(level={}, passed={}/{}, compliance={:.1}%)",
            self.level,
            self.passed,
            self.passed + self.failed,
            self.compliance_percent
        )
    }
}

impl From<&crate::terminal::ComplianceReport> for PyComplianceReport {
    fn from(report: &crate::terminal::ComplianceReport) -> Self {
        use crate::terminal::ComplianceLevel;

        let level = match report.level {
            ComplianceLevel::VT52 => "vt52",
            ComplianceLevel::VT100 => "vt100",
            ComplianceLevel::VT220 => "vt220",
            ComplianceLevel::VT320 => "vt320",
            ComplianceLevel::VT420 => "vt420",
            ComplianceLevel::VT520 => "vt520",
            ComplianceLevel::XTerm => "xterm",
        }
        .to_string();

        PyComplianceReport {
            terminal_info: report.terminal_info.clone(),
            level,
            tests: report.tests.iter().map(PyComplianceTest::from).collect(),
            passed: report.passed,
            failed: report.failed,
            compliance_percent: report.compliance_percent,
        }
    }
}
