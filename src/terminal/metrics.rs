//! Performance metrics and profiling
//!
//! Provides types for tracking terminal performance, profiling escape sequences,
//! and benchmarking various operations.

use std::collections::HashMap;

/// Performance metrics for tracking terminal rendering performance
#[derive(Debug, Clone, Default)]
pub struct PerformanceMetrics {
    /// Total number of frames rendered
    pub frames_rendered: u64,
    /// Total number of cells updated
    pub cells_updated: u64,
    /// Total number of bytes processed
    pub bytes_processed: u64,
    /// Total processing time in microseconds
    pub total_processing_us: u64,
    /// Peak processing time for a single frame in microseconds
    pub peak_frame_us: u64,
    /// Number of scrolls performed
    pub scroll_count: u64,
    /// Number of line wraps
    pub wrap_count: u64,
    /// Number of escape sequences processed
    pub escape_sequences: u64,
}

/// Frame timing information
#[derive(Debug, Clone)]
pub struct FrameTiming {
    /// Frame number
    pub frame_number: u64,
    /// Processing time in microseconds
    pub processing_us: u64,
    /// Number of cells updated this frame
    pub cells_updated: usize,
    /// Number of bytes processed this frame
    pub bytes_processed: usize,
}

/// Profiling data for escape sequences
#[derive(Debug, Clone, Default)]
pub struct EscapeSequenceProfile {
    /// Total count of this sequence type
    pub count: u64,
    /// Total time spent processing (microseconds)
    pub total_time_us: u64,
    /// Peak processing time (microseconds)
    pub peak_time_us: u64,
    /// Average processing time (microseconds)
    pub avg_time_us: u64,
}

/// Profiling category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileCategory {
    /// CSI sequences
    CSI,
    /// OSC sequences
    OSC,
    /// ESC sequences
    ESC,
    /// DCS sequences
    DCS,
    /// Plain text printing
    Print,
    /// Control characters
    Control,
}

/// Complete profiling data
#[derive(Debug, Clone, Default)]
pub struct ProfilingData {
    /// Per-category profiling
    pub categories: HashMap<ProfileCategory, EscapeSequenceProfile>,
    /// Memory allocations tracked
    pub allocations: u64,
    /// Total bytes allocated
    pub bytes_allocated: u64,
    /// Peak memory usage
    pub peak_memory: usize,
}

/// Benchmark category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BenchmarkCategory {
    /// Text rendering performance
    Rendering,
    /// Escape sequence parsing
    Parsing,
    /// Grid operations
    GridOps,
    /// Scrollback operations
    Scrollback,
    /// Memory operations
    Memory,
    /// Overall throughput
    Throughput,
}

/// Benchmark result
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Benchmark category
    pub category: BenchmarkCategory,
    /// Benchmark name
    pub name: String,
    /// Number of iterations
    pub iterations: u64,
    /// Total time in microseconds
    pub total_time_us: u64,
    /// Average time per iteration
    pub avg_time_us: u64,
    /// Minimum time
    pub min_time_us: u64,
    /// Maximum time
    pub max_time_us: u64,
    /// Operations per second
    pub ops_per_sec: f64,
    /// Memory used (bytes)
    pub memory_bytes: Option<usize>,
}

/// Benchmark suite results
#[derive(Debug, Clone)]
pub struct BenchmarkSuite {
    /// All benchmark results
    pub results: Vec<BenchmarkResult>,
    /// Total execution time
    pub total_time_ms: u64,
    /// Suite name
    pub suite_name: String,
}

use crate::terminal::Terminal;

impl Terminal {
    /// Get current performance metrics
    pub fn get_performance_metrics(&self) -> PerformanceMetrics {
        self.perf_metrics.clone()
    }

    /// Reset performance metrics
    pub fn reset_performance_metrics(&mut self) {
        self.perf_metrics = PerformanceMetrics::default();
        self.frame_timings.clear();
    }

    /// Record a frame timing
    pub fn record_frame_timing(
        &mut self,
        processing_us: u64,
        cells_updated: usize,
        bytes_processed: usize,
    ) {
        self.perf_metrics.frames_rendered += 1;
        self.perf_metrics.cells_updated += cells_updated as u64;
        self.perf_metrics.bytes_processed += bytes_processed as u64;
        self.perf_metrics.total_processing_us += processing_us;

        if processing_us > self.perf_metrics.peak_frame_us {
            self.perf_metrics.peak_frame_us = processing_us;
        }

        let frame_timing = FrameTiming {
            frame_number: self.perf_metrics.frames_rendered,
            processing_us,
            cells_updated,
            bytes_processed,
        };

        self.frame_timings.push(frame_timing);

        // Keep only last N frames
        if self.frame_timings.len() > self.max_frame_timings {
            self.frame_timings.remove(0);
        }
    }

    /// Get recent frame timings
    pub fn get_frame_timings(&self, count: Option<usize>) -> Vec<FrameTiming> {
        let count = count
            .unwrap_or(self.frame_timings.len())
            .min(self.frame_timings.len());
        self.frame_timings[self.frame_timings.len() - count..].to_vec()
    }

    /// Get average frame time in microseconds
    pub fn get_average_frame_time(&self) -> u64 {
        if self.perf_metrics.frames_rendered == 0 {
            0
        } else {
            self.perf_metrics.total_processing_us / self.perf_metrics.frames_rendered
        }
    }

    /// Get frames per second (based on average frame time)
    pub fn get_fps(&self) -> f64 {
        let avg_time = self.get_average_frame_time();
        if avg_time == 0 {
            0.0
        } else {
            1_000_000.0 / avg_time as f64
        }
    }

    // === Feature 16: Performance Profiling ===

    /// Enable or disable performance profiling
    pub fn set_profiling_enabled(&mut self, enabled: bool) {
        self.profiling_enabled = enabled;
        if enabled && self.profiling_data.is_none() {
            self.profiling_data = Some(ProfilingData::default());
        }
    }

    /// Check if profiling is enabled
    pub fn is_profiling_enabled(&self) -> bool {
        self.profiling_enabled
    }

    /// Enable profiling
    pub fn enable_profiling(&mut self) {
        self.set_profiling_enabled(true);
    }

    /// Disable profiling
    pub fn disable_profiling(&mut self) {
        self.set_profiling_enabled(false);
    }

    /// Record a memory allocation
    pub fn record_allocation(&mut self, bytes: u64) {
        if let Some(ref mut data) = self.profiling_data {
            data.allocations += 1;
            data.bytes_allocated += bytes;
        }
    }

    /// Update peak memory usage
    pub fn update_peak_memory(&mut self, current_bytes: usize) {
        if let Some(ref mut data) = self.profiling_data {
            if current_bytes > data.peak_memory {
                data.peak_memory = current_bytes;
            }
        }
    }

    /// Record profiling data for a category
    pub fn record_profiling(&mut self, category: ProfileCategory, micros: u64) {
        if !self.profiling_enabled {
            return;
        }

        if let Some(ref mut data) = self.profiling_data {
            let profile = data.categories.entry(category).or_default();
            profile.count += 1;
            profile.total_time_us += micros;
            if micros > profile.peak_time_us {
                profile.peak_time_us = micros;
            }
            profile.avg_time_us = profile.total_time_us / profile.count;
        }
    }

    /// Get current profiling data
    pub fn get_profiling_data(&self) -> Option<ProfilingData> {
        self.profiling_data.clone()
    }

    /// Reset profiling data
    pub fn reset_profiling_data(&mut self) {
        if self.profiling_enabled {
            self.profiling_data = Some(ProfilingData::default());
        } else {
            self.profiling_data = None;
        }
    }

    /// Record processing time for an escape sequence
    pub fn record_escape_sequence(&mut self, category: ProfileCategory, micros: u64) {
        self.record_profiling(category, micros);
    }

    // === Feature 28: Benchmarking Suite ===

    /// Run rendering benchmark
    pub fn benchmark_rendering(&mut self, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let mut min_time = u64::MAX;
        let mut max_time = 0u64;

        for _ in 0..iterations {
            let iter_start = std::time::Instant::now();

            // Simulate rendering operation
            let grid = self.active_grid();
            for row in 0..grid.rows() {
                if let Some(line) = grid.row(row) {
                    let _ = crate::terminal::cells_to_text(line);
                }
            }

            let iter_time = iter_start.elapsed().as_micros() as u64;
            min_time = min_time.min(iter_time);
            max_time = max_time.max(iter_time);
        }

        let total_time = start.elapsed().as_micros() as u64;
        let avg_time = total_time / iterations;

        BenchmarkResult {
            category: BenchmarkCategory::Rendering,
            name: "Text Rendering".to_string(),
            iterations,
            total_time_us: total_time,
            avg_time_us: avg_time,
            min_time_us: min_time,
            max_time_us: max_time,
            ops_per_sec: if avg_time > 0 {
                1_000_000.0 / avg_time as f64
            } else {
                0.0
            },
            memory_bytes: None,
        }
    }

    /// Run parsing benchmark
    pub fn benchmark_parsing(&mut self, text: &str, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let bytes = text.as_bytes();
        for _ in 0..iterations {
            self.process(bytes);
        }
        let total_time = start.elapsed().as_micros() as u64;
        let avg_time = total_time / iterations;

        BenchmarkResult {
            category: BenchmarkCategory::Parsing,
            name: "Parsing".to_string(),
            iterations,
            total_time_us: total_time,
            avg_time_us: avg_time,
            min_time_us: 0,
            max_time_us: 0,
            ops_per_sec: if avg_time > 0 {
                1_000_000.0 / avg_time as f64
            } else {
                0.0
            },
            memory_bytes: None,
        }
    }

    /// Run grid operations benchmark
    pub fn benchmark_grid_ops(&mut self, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            // Perform various grid ops
            self.grid.clear();
        }
        let total_time = start.elapsed().as_micros() as u64;
        let avg_time = total_time / iterations;

        BenchmarkResult {
            category: BenchmarkCategory::GridOps,
            name: "Grid Ops".to_string(),
            iterations,
            total_time_us: total_time,
            avg_time_us: avg_time,
            min_time_us: 0,
            max_time_us: 0,
            ops_per_sec: if avg_time > 0 {
                1_000_000.0 / avg_time as f64
            } else {
                0.0
            },
            memory_bytes: None,
        }
    }

    /// Run full benchmark suite
    pub fn run_benchmark_suite(&mut self, suite_name: String) -> BenchmarkSuite {
        let start = std::time::Instant::now();
        let results = vec![self.benchmark_rendering(10), self.benchmark_grid_ops(100)];

        BenchmarkSuite {
            results,
            total_time_ms: start.elapsed().as_millis() as u64,
            suite_name,
        }
    }

    /// Get comprehensive terminal statistics
    pub fn get_stats(&self) -> TerminalStats {
        let (cols, rows) = self.size();
        let scrollback_lines = self.grid.scrollback_len();
        let total_cells = (rows * cols) + (scrollback_lines * cols);

        TerminalStats {
            cols,
            rows,
            scrollback_lines,
            total_cells,
            non_whitespace_lines: self.count_non_whitespace_lines(),
            graphics_count: self.graphics_store.graphics_count(),
            estimated_memory_bytes: 0, // Should be calculated
            hyperlink_count: self.hyperlinks.len(),
            hyperlink_memory_bytes: 0, // Should be calculated
            color_stack_depth: self.color_stack.len(),
            title_stack_depth: self.title_stack.len(),
            keyboard_stack_depth: self.keyboard_stack.len(),
            response_buffer_size: self.response_buffer.len(),
            dirty_row_count: self.dirty_rows.len(),
            pending_bell_events: self.bell_events.len(),
            pending_terminal_events: self.terminal_events.len(),
        }
    }
}

pub struct TerminalStats {
    /// Number of columns
    pub cols: usize,
    /// Number of rows
    pub rows: usize,
    /// Number of scrollback lines currently used
    pub scrollback_lines: usize,
    /// Total number of cells (rows × cols + scrollback)
    pub total_cells: usize,
    /// Number of lines with non-whitespace content
    pub non_whitespace_lines: usize,
    /// Number of Sixel graphics
    pub graphics_count: usize,
    /// Estimated memory usage in bytes
    pub estimated_memory_bytes: usize,
    /// Number of hyperlinks stored
    pub hyperlink_count: usize,
    /// Estimated memory used by hyperlink storage (bytes)
    pub hyperlink_memory_bytes: usize,
    /// Color stack depth
    pub color_stack_depth: usize,
    /// Title stack depth
    pub title_stack_depth: usize,
    /// Keyboard flag stack depth (active screen)
    pub keyboard_stack_depth: usize,
    /// Response buffer size (bytes)
    pub response_buffer_size: usize,
    /// Number of dirty rows
    pub dirty_row_count: usize,
    /// Pending bell events count
    pub pending_bell_events: usize,
    /// Pending terminal events count
    pub pending_terminal_events: usize,
}
