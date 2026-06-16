//! Metrics, profiling, benchmarking, and compliance API methods for `PyTerminal`
//! (ARC-002: split out of the monolithic `#[pymethods]` block in `mod.rs`). Pure
//! relocation — no Python API or behavior change; these methods remain on the same
//! `Terminal` Python class.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::PyTerminal;

#[pymethods]
impl PyTerminal {
    // === Feature 7: Performance Metrics ===

    /// Get current performance metrics
    fn get_performance_metrics(
        &self,
    ) -> PyResult<crate::python_bindings::types::PyPerformanceMetrics> {
        let m = self.inner.get_performance_metrics();
        Ok(crate::python_bindings::types::PyPerformanceMetrics {
            frames_rendered: m.frames_rendered,
            cells_updated: m.cells_updated,
            bytes_processed: m.bytes_processed,
            total_processing_us: m.total_processing_us,
            peak_frame_us: m.peak_frame_us,
            scroll_count: m.scroll_count,
            wrap_count: m.wrap_count,
            escape_sequences: m.escape_sequences,
        })
    }

    /// Reset performance metrics
    fn reset_performance_metrics(&mut self) -> PyResult<()> {
        self.inner.reset_performance_metrics();
        Ok(())
    }

    /// Record a frame timing
    fn record_frame_timing(
        &mut self,
        processing_us: u64,
        cells_updated: usize,
        bytes_processed: usize,
    ) -> PyResult<()> {
        self.inner
            .record_frame_timing(processing_us, cells_updated, bytes_processed);
        Ok(())
    }

    /// Get recent frame timings
    #[pyo3(signature = (count=None))]
    fn get_frame_timings(
        &self,
        count: Option<usize>,
    ) -> PyResult<Vec<crate::python_bindings::types::PyFrameTiming>> {
        let timings = self.inner.get_frame_timings(count);
        Ok(timings
            .iter()
            .map(|t| crate::python_bindings::types::PyFrameTiming {
                frame_number: t.frame_number,
                processing_us: t.processing_us,
                cells_updated: t.cells_updated,
                bytes_processed: t.bytes_processed,
            })
            .collect())
    }

    /// Get average frame time in microseconds
    fn get_average_frame_time(&self) -> PyResult<u64> {
        Ok(self.inner.get_average_frame_time())
    }

    /// Get frames per second
    fn get_fps(&self) -> PyResult<f64> {
        Ok(self.inner.get_fps())
    }

    // === Feature 16: Performance Profiling ===

    /// Enable performance profiling
    fn enable_profiling(&mut self) -> PyResult<()> {
        self.inner.enable_profiling();
        Ok(())
    }

    /// Disable performance profiling
    fn disable_profiling(&mut self) -> PyResult<()> {
        self.inner.disable_profiling();
        Ok(())
    }

    /// Check if profiling is enabled
    fn is_profiling_enabled(&self) -> PyResult<bool> {
        Ok(self.inner.is_profiling_enabled())
    }

    /// Get profiling data
    fn get_profiling_data(
        &self,
    ) -> PyResult<Option<crate::python_bindings::types::PyProfilingData>> {
        Ok(self
            .inner
            .get_profiling_data()
            .map(|d| crate::python_bindings::types::PyProfilingData::from(&d)))
    }

    /// Reset profiling data
    fn reset_profiling_data(&mut self) -> PyResult<()> {
        self.inner.reset_profiling_data();
        Ok(())
    }

    /// Record an escape sequence execution
    fn record_escape_sequence(&mut self, category: &str, time_us: u64) -> PyResult<()> {
        use crate::terminal::ProfileCategory;

        let category = match category.to_lowercase().as_str() {
            "csi" => ProfileCategory::CSI,
            "osc" => ProfileCategory::OSC,
            "esc" => ProfileCategory::ESC,
            "dcs" => ProfileCategory::DCS,
            "print" => ProfileCategory::Print,
            "control" => ProfileCategory::Control,
            _ => return Err(PyValueError::new_err("Invalid profile category")),
        };

        self.inner.record_escape_sequence(category, time_us);
        Ok(())
    }

    /// Record memory allocation
    fn record_allocation(&mut self, bytes: u64) -> PyResult<()> {
        self.inner.record_allocation(bytes);
        Ok(())
    }

    /// Update peak memory usage
    fn update_peak_memory(&mut self, current_bytes: usize) -> PyResult<()> {
        self.inner.update_peak_memory(current_bytes);
        Ok(())
    }

    // === Feature 28: Benchmarking Suite ===

    /// Run rendering benchmark
    ///
    /// Args:
    ///     iterations: Number of iterations to run
    ///
    /// Returns:
    ///     PyBenchmarkResult with timing statistics
    fn benchmark_rendering(
        &mut self,
        iterations: u64,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkResult> {
        let result = self.inner.benchmark_rendering(iterations);
        Ok(crate::python_bindings::types::PyBenchmarkResult::from(
            &result,
        ))
    }

    /// Run escape sequence parsing benchmark
    ///
    /// Args:
    ///     text: Text to parse
    ///     iterations: Number of iterations to run
    ///
    /// Returns:
    ///     PyBenchmarkResult with timing statistics
    fn benchmark_parsing(
        &mut self,
        text: &str,
        iterations: u64,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkResult> {
        let result = self.inner.benchmark_parsing(text, iterations);
        Ok(crate::python_bindings::types::PyBenchmarkResult::from(
            &result,
        ))
    }

    /// Run grid operations benchmark
    ///
    /// Args:
    ///     iterations: Number of iterations to run
    ///
    /// Returns:
    ///     PyBenchmarkResult with timing statistics
    fn benchmark_grid_ops(
        &mut self,
        iterations: u64,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkResult> {
        let result = self.inner.benchmark_grid_ops(iterations);
        Ok(crate::python_bindings::types::PyBenchmarkResult::from(
            &result,
        ))
    }

    /// Run full benchmark suite
    ///
    /// Args:
    ///     suite_name: Name for the benchmark suite
    ///
    /// Returns:
    ///     PyBenchmarkSuite with all benchmark results
    fn run_benchmark_suite(
        &mut self,
        suite_name: String,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkSuite> {
        let suite = self.inner.run_benchmark_suite(suite_name);
        Ok(crate::python_bindings::types::PyBenchmarkSuite::from(
            &suite,
        ))
    }

    // === Feature 29: Terminal Compliance Testing ===

    /// Run compliance tests for a specific level
    ///
    /// Args:
    ///     level: Compliance level to test ("vt52", "vt100", "vt220", "vt320", "vt420", "vt520", "xterm")
    ///
    /// Returns:
    ///     PyComplianceReport with test results
    fn test_compliance(
        &mut self,
        level: &str,
    ) -> PyResult<crate::python_bindings::types::PyComplianceReport> {
        use crate::terminal::ComplianceLevel;

        let rust_level = match level.to_lowercase().as_str() {
            "vt52" => ComplianceLevel::VT52,
            "vt100" => ComplianceLevel::VT100,
            "vt220" => ComplianceLevel::VT220,
            "vt320" => ComplianceLevel::VT320,
            "vt420" => ComplianceLevel::VT420,
            "vt520" => ComplianceLevel::VT520,
            "xterm" => ComplianceLevel::XTerm,
            _ => return Err(PyValueError::new_err("Invalid compliance level")),
        };

        let report = self.inner.test_compliance(rust_level);
        Ok(crate::python_bindings::types::PyComplianceReport::from(
            &report,
        ))
    }

    /// Generate compliance report as formatted string
    ///
    /// Args:
    ///     report: PyComplianceReport to format
    ///
    /// Returns:
    ///     Formatted compliance report string
    #[staticmethod]
    fn format_compliance_report(
        report: &crate::python_bindings::types::PyComplianceReport,
    ) -> PyResult<String> {
        use crate::terminal::{ComplianceLevel, ComplianceReport, ComplianceTest, Terminal};

        let rust_level = match report.level.as_str() {
            "vt52" => ComplianceLevel::VT52,
            "vt100" => ComplianceLevel::VT100,
            "vt220" => ComplianceLevel::VT220,
            "vt320" => ComplianceLevel::VT320,
            "vt420" => ComplianceLevel::VT420,
            "vt520" => ComplianceLevel::VT520,
            "xterm" => ComplianceLevel::XTerm,
            _ => return Err(PyValueError::new_err("Invalid compliance level")),
        };

        let rust_tests: Vec<ComplianceTest> = report
            .tests
            .iter()
            .map(|t| ComplianceTest {
                name: t.name.clone(),
                category: t.category.clone(),
                passed: t.passed,
                expected: t.expected.clone(),
                actual: t.actual.clone(),
                notes: t.notes.clone(),
            })
            .collect();

        let rust_report = ComplianceReport {
            terminal_info: report.terminal_info.clone(),
            level: rust_level,
            tests: rust_tests,
            passed: report.passed,
            failed: report.failed,
            compliance_percent: report.compliance_percent,
        };

        Ok(Terminal::format_compliance_report(&rust_report))
    }
}
