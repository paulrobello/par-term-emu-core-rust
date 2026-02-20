use crate::terminal::metrics::ProfileCategory;
use crate::terminal::Terminal;

#[test]
fn test_performance_metrics_default() {
    let term = Terminal::new(80, 24);
    let m = term.get_performance_metrics();
    assert_eq!(m.frames_rendered, 0);
    assert_eq!(m.cells_updated, 0);
    assert_eq!(m.bytes_processed, 0);
    assert_eq!(m.total_processing_us, 0);
    assert_eq!(m.peak_frame_us, 0);
}

#[test]
fn test_record_frame_timing() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(1000, 100, 50);
    let m = term.get_performance_metrics();
    assert_eq!(m.frames_rendered, 1);
    assert_eq!(m.cells_updated, 100);
    assert_eq!(m.bytes_processed, 50);
    assert_eq!(m.total_processing_us, 1000);
    assert_eq!(m.peak_frame_us, 1000);
}

#[test]
fn test_record_frame_timing_accumulates() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(1000, 100, 50);
    term.record_frame_timing(2000, 200, 100);
    let m = term.get_performance_metrics();
    assert_eq!(m.frames_rendered, 2);
    assert_eq!(m.cells_updated, 300);
    assert_eq!(m.bytes_processed, 150);
    assert_eq!(m.total_processing_us, 3000);
    assert_eq!(m.peak_frame_us, 2000, "peak should track the max");
}

#[test]
fn test_reset_performance_metrics() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(1000, 100, 50);
    term.reset_performance_metrics();
    let m = term.get_performance_metrics();
    assert_eq!(m.frames_rendered, 0);
    assert_eq!(m.total_processing_us, 0);
}

#[test]
fn test_get_frame_timings_empty() {
    let term = Terminal::new(80, 24);
    let timings = term.get_frame_timings(None);
    assert!(timings.is_empty());
}

#[test]
fn test_get_frame_timings_all() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(100, 10, 5);
    term.record_frame_timing(200, 20, 10);
    term.record_frame_timing(300, 30, 15);
    let timings = term.get_frame_timings(None);
    assert_eq!(timings.len(), 3);
}

#[test]
fn test_get_frame_timings_limited() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(100, 10, 5);
    term.record_frame_timing(200, 20, 10);
    term.record_frame_timing(300, 30, 15);
    let timings = term.get_frame_timings(Some(2));
    assert_eq!(timings.len(), 2, "should return last 2 timings");
    assert!(
        timings
            .iter()
            .any(|t| t.processing_us == 200 || t.processing_us == 300),
        "last 2 timings should include the 200 and/or 300 us frames"
    );
}

#[test]
fn test_get_average_frame_time_empty() {
    let term = Terminal::new(80, 24);
    assert_eq!(term.get_average_frame_time(), 0);
}

#[test]
fn test_get_average_frame_time() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(100, 10, 5);
    term.record_frame_timing(300, 30, 15);
    assert_eq!(term.get_average_frame_time(), 200);
}

#[test]
fn test_get_fps_no_frames() {
    let term = Terminal::new(80, 24);
    let fps = term.get_fps();
    assert_eq!(fps, 0.0);
}

#[test]
fn test_get_fps_with_frames() {
    let mut term = Terminal::new(80, 24);
    term.record_frame_timing(1000, 100, 50);
    term.record_frame_timing(1000, 100, 50);
    let fps = term.get_fps();
    assert!(fps > 0.0, "fps should be positive");
}

#[test]
fn test_profiling_disabled_by_default() {
    let term = Terminal::new(80, 24);
    assert!(!term.is_profiling_enabled());
}

#[test]
fn test_enable_profiling() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    assert!(term.is_profiling_enabled());
}

#[test]
fn test_disable_profiling() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    term.disable_profiling();
    assert!(!term.is_profiling_enabled());
}

#[test]
fn test_set_profiling_enabled() {
    let mut term = Terminal::new(80, 24);
    term.set_profiling_enabled(true);
    assert!(term.is_profiling_enabled());
    term.set_profiling_enabled(false);
    assert!(!term.is_profiling_enabled());
}

#[test]
fn test_record_profiling_when_disabled_is_noop() {
    let mut term = Terminal::new(80, 24);
    assert!(!term.is_profiling_enabled());
    term.record_profiling(ProfileCategory::CSI, 500);
    assert!(term.get_profiling_data().is_none());
}

#[test]
fn test_record_profiling_when_enabled() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    term.record_profiling(ProfileCategory::CSI, 500);
    let data = term
        .get_profiling_data()
        .expect("profiling data should exist when enabled");
    let csi = data.categories.get(&ProfileCategory::CSI);
    assert!(csi.is_some(), "CSI category should be recorded");
    assert_eq!(csi.unwrap().count, 1);
}

#[test]
fn test_record_multiple_profiling_categories() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    term.record_profiling(ProfileCategory::CSI, 100);
    term.record_profiling(ProfileCategory::OSC, 200);
    term.record_profiling(ProfileCategory::CSI, 150);
    let data = term.get_profiling_data().unwrap();
    let csi = data.categories.get(&ProfileCategory::CSI).unwrap();
    assert_eq!(csi.count, 2);
    assert_eq!(csi.total_time_us, 250);
    let osc = data.categories.get(&ProfileCategory::OSC).unwrap();
    assert_eq!(osc.count, 1);
}

#[test]
fn test_record_allocation() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    term.record_allocation(1024);
    term.record_allocation(2048);
    let data = term.get_profiling_data().unwrap();
    assert_eq!(data.allocations, 2);
    assert_eq!(data.bytes_allocated, 3072);
}

#[test]
fn test_update_peak_memory() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    term.update_peak_memory(1000);
    term.update_peak_memory(5000);
    term.update_peak_memory(2000);
    let data = term.get_profiling_data().unwrap();
    assert_eq!(data.peak_memory, 5000);
}

#[test]
fn test_reset_profiling_data() {
    let mut term = Terminal::new(80, 24);
    term.enable_profiling();
    term.record_profiling(ProfileCategory::CSI, 100);
    term.reset_profiling_data();
    let data = term.get_profiling_data().unwrap();
    assert!(
        data.categories.is_empty(),
        "categories should be cleared after reset"
    );
}

#[test]
fn test_benchmark_rendering_returns_result() {
    let mut term = Terminal::new(80, 24);
    let result = term.benchmark_rendering(10);
    assert_eq!(result.iterations, 10);
}

#[test]
fn test_benchmark_parsing_returns_result() {
    let mut term = Terminal::new(80, 24);
    let result = term.benchmark_parsing("hello world\r\n", 5);
    assert_eq!(result.iterations, 5);
}

#[test]
fn test_benchmark_grid_ops_returns_result() {
    let mut term = Terminal::new(80, 24);
    let result = term.benchmark_grid_ops(5);
    assert_eq!(result.iterations, 5);
}

#[test]
fn test_run_benchmark_suite() {
    let mut term = Terminal::new(80, 24);
    let suite = term.run_benchmark_suite("test-suite".to_string());
    assert_eq!(suite.suite_name, "test-suite");
    assert!(
        !suite.results.is_empty(),
        "suite should have at least one result"
    );
}

#[test]
fn test_get_stats_dimensions() {
    let term = Terminal::new(80, 24);
    let stats = term.get_stats();
    assert_eq!(stats.cols, 80);
    assert_eq!(stats.rows, 24);
    // total_cells includes scrollback; at minimum it should cover the visible grid
    assert!(
        stats.total_cells >= 80 * 24,
        "total_cells should be at least rows*cols"
    );
}

#[test]
fn test_get_stats_initial_empty() {
    let term = Terminal::new(80, 24);
    let stats = term.get_stats();
    assert_eq!(stats.graphics_count, 0);
    assert_eq!(stats.hyperlink_count, 0);
}
