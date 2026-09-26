//! Terminal benchmarking service (ARC-021 phase 1).
//!
//! `Terminal` carried the benchmark suite as inherent methods even though
//! benchmarking is not VT state-machine behavior — every such method grows
//! the god-object's four-way sync surface (Rust impl, binding macros, .pyi,
//! API_REFERENCE). The service below owns that logic instead, operating on a
//! borrowed `Terminal`; the deprecated forwarding methods on `Terminal`
//! (metrics.rs) delegate here and will be removed in a future release.

use super::metrics::{BenchmarkCategory, BenchmarkResult, BenchmarkSuite};
use super::Terminal;

/// Benchmarks operations on a [`Terminal`] (rendering scan, parsing, grid ops).
///
/// Stateless by construction — call as `TerminalBenchmarks::benchmark_parsing(&mut term, ...)`.
pub struct TerminalBenchmarks;

impl TerminalBenchmarks {
    /// Run rendering benchmark
    pub fn benchmark_rendering(term: &mut Terminal, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let mut min_time = u64::MAX;
        let mut max_time = 0u64;

        for _ in 0..iterations {
            let iter_start = std::time::Instant::now();

            // Simulate rendering operation
            let grid = term.active_grid();
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
    pub fn benchmark_parsing(term: &mut Terminal, text: &str, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        let bytes = text.as_bytes();
        for _ in 0..iterations {
            term.process(bytes);
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
    pub fn benchmark_grid_ops(term: &mut Terminal, iterations: u64) -> BenchmarkResult {
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            // Perform various grid ops
            term.grid.clear();
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
    pub fn run_benchmark_suite(term: &mut Terminal, suite_name: String) -> BenchmarkSuite {
        let start = std::time::Instant::now();
        let results = vec![
            Self::benchmark_rendering(term, 10),
            Self::benchmark_grid_ops(term, 100),
        ];

        BenchmarkSuite {
            results,
            total_time_ms: start.elapsed().as_millis() as u64,
            suite_name,
        }
    }
}
