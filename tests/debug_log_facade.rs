//! ARC-111: core diagnostics reach the `log` facade.
//!
//! Its own test binary because a process installs one global logger.

use std::sync::Mutex;

static RECORDS: Mutex<Vec<(log::Level, String, String)>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        RECORDS.lock().unwrap().push((
            record.level(),
            record.target().to_string(),
            record.args().to_string(),
        ));
    }

    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;

fn records() -> Vec<(log::Level, String, String)> {
    RECORDS.lock().unwrap().clone()
}

#[test]
fn core_records_reach_the_installed_logger_at_its_level() {
    use par_term_emu_core_rust::debug::{self, DebugLevel};

    // No DEBUG_LEVEL: the file sink stays off, so only the facade decides.
    assert!(std::env::var_os("DEBUG_LEVEL").is_none());
    log::set_logger(&CAPTURE).expect("the first logger in this process");

    log::set_max_level(log::LevelFilter::Off);
    assert!(!debug::is_enabled(DebugLevel::Error));
    par_term_emu_core_rust::debug_error!("CAT", "silent {}", 0);
    assert!(records().is_empty(), "no logger level, no record");

    log::set_max_level(log::LevelFilter::Info);
    assert!(debug::is_enabled(DebugLevel::Info));
    assert!(!debug::is_enabled(DebugLevel::Debug));
    par_term_emu_core_rust::debug_error!("CAT", "boom {}", 1);
    par_term_emu_core_rust::debug_log!("CAT", "filtered {}", 2);
    debug::log_screen_switch(true, "test");

    let got = records();
    assert_eq!(
        got,
        vec![
            (
                log::Level::Error,
                debug::LOG_TARGET.to_string(),
                "[CAT] boom 1".to_string()
            ),
            (
                log::Level::Info,
                debug::LOG_TARGET.to_string(),
                "[SCREEN_SWITCH] switched to ALTERNATE screen (test)".to_string()
            ),
        ]
    );

    log::set_max_level(log::LevelFilter::Trace);
    debug::log_buffer_snapshot("snap", 2, 3, "abc");
    let last = records().pop().expect("a snapshot record");
    assert_eq!(last.0, log::Level::Trace);
    assert!(
        last.2.starts_with("[BUFFER_SNAPSHOT] snap (2x3)"),
        "{}",
        last.2
    );

    // The file sink is off without DEBUG_LEVEL: nothing was written.
    let file = std::env::temp_dir().join(format!(
        "par_term_emu_core_rust_debug_rust_{}.log",
        std::process::id()
    ));
    assert!(!file.exists(), "{} must not exist", file.display());
}
