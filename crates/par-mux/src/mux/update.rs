//! Update channel selection for the CLI. Fetching, verification, replacement,
//! and restart helpers live in `par_term_emu_core::self_update`.

/// Release channel used by `par-mux update`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum Channel {
    /// Latest non-prerelease release.
    Stable,
    /// Newest prerelease, falling back to stable when none exists.
    Preview,
}
