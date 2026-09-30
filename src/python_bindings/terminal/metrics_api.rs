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
    ///
    /// Returns:
    ///     PerformanceMetrics: Aggregate counters since the last reset
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.record_frame_timing(1500, 120, 64)
    ///     m = term.get_performance_metrics()
    ///     (m.frames_rendered, m.cells_updated, m.bytes_processed, m.peak_frame_us)
    ///     # (1, 120, 64, 1500)
    ///     ```
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
    ///
    /// Also clears the buffered frame timings.
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.record_frame_timing(1500, 120, 64)
    ///     term.reset_performance_metrics()
    ///     term.get_performance_metrics().frames_rendered   # 0
    ///     term.get_frame_timings()                         # []
    ///     ```
    fn reset_performance_metrics(&mut self) -> PyResult<()> {
        self.inner.reset_performance_metrics();
        Ok(())
    }

    /// Record a frame timing
    ///
    /// Args:
    ///     processing_us: Frame processing time in microseconds
    ///     cells_updated: Number of cells updated in the frame
    ///     bytes_processed: Number of input bytes processed in the frame
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.record_frame_timing(1500, 120, 64)
    ///     term.get_frame_timings()   # [FrameTiming(frame=1, time=1500us, cells=120)]
    ///     ```
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
    ///
    /// Args:
    ///     count: Maximum timings to return (None for all buffered)
    ///
    /// Returns:
    ///     list[FrameTiming]: Most recent frame timings, oldest first
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.record_frame_timing(1500, 120, 64)
    ///     term.record_frame_timing(2500, 80, 32)
    ///     [t.processing_us for t in term.get_frame_timings()]    # [1500, 2500]
    ///     [t.processing_us for t in term.get_frame_timings(1)]   # [2500]
    ///     ```
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
    ///
    /// Returns:
    ///     int: Mean frame processing time in microseconds
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.record_frame_timing(1500, 120, 64)
    ///     term.record_frame_timing(2500, 80, 32)
    ///     term.get_average_frame_time()   # 2000
    ///     ```
    fn get_average_frame_time(&self) -> PyResult<u64> {
        Ok(self.inner.get_average_frame_time())
    }

    /// Get frames per second
    ///
    /// Returns:
    ///     float: Recent frame rate in frames per second (0.0 with no timings)
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_fps()   # 0.0
    ///     term.record_frame_timing(1500, 120, 64)
    ///     term.record_frame_timing(2500, 80, 32)
    ///     term.get_fps()   # 500.0
    ///     ```
    fn get_fps(&self) -> PyResult<f64> {
        Ok(self.inner.get_fps())
    }

    // === Feature 16: Performance Profiling ===

    /// Enable performance profiling
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.enable_profiling()
    ///     term.get_profiling_data()   # ProfilingData(categories=0, allocations=0, peak_memory=0)
    ///     ```
    fn enable_profiling(&mut self) -> PyResult<()> {
        self.inner.enable_profiling();
        Ok(())
    }

    /// Disable performance profiling
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.enable_profiling()
    ///     term.disable_profiling()
    ///     term.is_profiling_enabled()   # False
    ///     ```
    fn disable_profiling(&mut self) -> PyResult<()> {
        self.inner.disable_profiling();
        Ok(())
    }

    /// Check if profiling is enabled
    ///
    /// Returns:
    ///     bool: True if performance profiling is collecting data
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.is_profiling_enabled()   # False
    ///     term.enable_profiling()
    ///     term.is_profiling_enabled()   # True
    ///     ```
    fn is_profiling_enabled(&self) -> PyResult<bool> {
        Ok(self.inner.is_profiling_enabled())
    }

    /// Get profiling data
    ///
    /// Returns:
    ///     ProfilingData | None: Collected profiling data, or None when no data
    ///     is held (profiling never enabled, or reset while disabled)
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.get_profiling_data()   # None
    ///     term.enable_profiling()
    ///     term.record_escape_sequence("csi", 12)
    ///     term.get_profiling_data().categories
    ///     # {'csi': EscapeSequenceProfile(count=1, avg_us=12, peak_us=12)}
    ///     ```
    fn get_profiling_data(
        &self,
    ) -> PyResult<Option<crate::python_bindings::types::PyProfilingData>> {
        Ok(self
            .inner
            .get_profiling_data()
            .map(|d| crate::python_bindings::types::PyProfilingData::from(&d)))
    }

    /// Reset profiling data
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.enable_profiling()
    ///     term.record_allocation(4096)
    ///     term.reset_profiling_data()
    ///     term.get_profiling_data()   # ProfilingData(categories=0, allocations=0, peak_memory=0)
    ///     ```
    fn reset_profiling_data(&mut self) -> PyResult<()> {
        self.inner.reset_profiling_data();
        Ok(())
    }

    /// Record an escape sequence execution
    ///
    /// Args:
    ///     category: One of "csi", "osc", "esc", "dcs", "print", "control"
    ///         (case-insensitive)
    ///     time_us: Execution time in microseconds
    ///
    /// Raises:
    ///     ValueError: If category is not one of the names above
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.enable_profiling()
    ///     term.record_escape_sequence("csi", 12)
    ///     term.record_escape_sequence("CSI", 8)
    ///     term.get_profiling_data().categories["csi"]
    ///     # EscapeSequenceProfile(count=2, avg_us=10, peak_us=12)
    ///     ```
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
    ///
    /// Args:
    ///     bytes: Number of bytes allocated
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.enable_profiling()
    ///     term.record_allocation(4096)
    ///     data = term.get_profiling_data()
    ///     (data.allocations, data.bytes_allocated)   # (1, 4096)
    ///     ```
    fn record_allocation(&mut self, bytes: u64) -> PyResult<()> {
        self.inner.record_allocation(bytes);
        Ok(())
    }

    /// Update peak memory usage
    ///
    /// Args:
    ///     current_bytes: Current total memory usage in bytes; recorded only
    ///         when it exceeds the stored peak
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.enable_profiling()
    ///     term.update_peak_memory(1048576)
    ///     term.update_peak_memory(4096)
    ///     term.get_profiling_data().peak_memory   # 1048576
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     result = term.benchmark_rendering(10)
    ///     (result.name, result.iterations)   # ('Text Rendering', 10)
    ///     result.avg_time_us                 # timing varies by machine
    ///     ```
    fn benchmark_rendering(
        &mut self,
        iterations: u64,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkResult> {
        let result =
            crate::terminal::TerminalBenchmarks::benchmark_rendering(&mut self.inner, iterations);
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     result = term.benchmark_parsing("\x1b[1mbold\x1b[0m\r\n", 10)
    ///     (result.name, result.iterations)   # ('Parsing', 10)
    ///     ```
    fn benchmark_parsing(
        &mut self,
        text: &str,
        iterations: u64,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkResult> {
        let result = crate::terminal::TerminalBenchmarks::benchmark_parsing(
            &mut self.inner,
            text,
            iterations,
        );
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     result = term.benchmark_grid_ops(10)
    ///     (result.name, result.iterations)   # ('Grid Ops', 10)
    ///     ```
    fn benchmark_grid_ops(
        &mut self,
        iterations: u64,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkResult> {
        let result =
            crate::terminal::TerminalBenchmarks::benchmark_grid_ops(&mut self.inner, iterations);
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     suite = term.run_benchmark_suite("smoke")
    ///     [r.name for r in suite.results]   # ['Text Rendering', 'Grid Ops']
    ///     ```
    fn run_benchmark_suite(
        &mut self,
        suite_name: String,
    ) -> PyResult<crate::python_bindings::types::PyBenchmarkSuite> {
        let suite =
            crate::terminal::TerminalBenchmarks::run_benchmark_suite(&mut self.inner, suite_name);
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
    ///
    /// Raises:
    ///     ValueError: If level is not one of the names above
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     term = Terminal(80, 24)
    ///     term.test_compliance("vt100")
    ///     # ComplianceReport(level=vt100, passed=1/1, compliance=100.0%)
    ///     ```
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
    ///
    /// Example:
    ///     ```python
    ///     from par_term_emu_core_rust import Terminal
    ///     report = Terminal(80, 24).test_compliance("vt100")
    ///     Terminal.format_compliance_report(report).splitlines()[:3]
    ///     # ['Compliance Report for par-term-emu-core-rust', 'Level: VT100', 'Score: 100.0% (1 passed, 0 failed)']
    ///     ```
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
