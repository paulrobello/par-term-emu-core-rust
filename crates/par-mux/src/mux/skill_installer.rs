//! par-mux agent skill installer.
//!
//! The skill that teaches coding agents to drive this CLI ships in the
//! source tree (`skills/par-mux/SKILL.md`) and is embedded in the binary
//! via `include_str!`, so every package — the par-term bundle, a
//! `cargo install`, `cargo run` — carries the copy that matches its own
//! CLI. The binary's `--install-skill` flag writes it into the Claude Code
//! skills directory; par-term's integrations dialog invokes that flag
//! instead of embedding its own copy, so the skill cannot drift from the
//! binary an agent actually drives.

use std::io;
use std::path::{Path, PathBuf};

/// The bundled par-mux agent skill, embedded at compile time.
pub const PAR_MUX_SKILL_MD: &str = include_str!("../../skills/par-mux/SKILL.md");

/// The Claude Code skills directory: `~/.claude/skills`.
pub fn claude_skills_dir() -> io::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".claude").join("skills"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))
}

/// Install the skill under `skills_dir`, the skills *root*: the file
/// lands at `skills_dir/par-mux/SKILL.md`. The seam shared by
/// [`install_skill`] and the bin `--skills-dir` flag.
pub fn install_skill_into(skills_dir: &Path) -> io::Result<PathBuf> {
    let dir = skills_dir.join("par-mux");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, PAR_MUX_SKILL_MD)?;
    Ok(path)
}

/// Install (or refresh) the bundled agent skill into the Claude Code
/// skills directory.
///
/// Always overwrites: the embedded copy is the source of truth, so an
/// install after a par-mux upgrade refreshes the skill to the shipping
/// version.
pub fn install_skill() -> io::Result<PathBuf> {
    install_skill_into(&claude_skills_dir()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_the_embedded_asset_under_the_skills_root() {
        let root = tempfile::tempdir().expect("temp dir");

        let path = install_skill_into(root.path()).expect("install");

        assert_eq!(path, root.path().join("par-mux").join("SKILL.md"));
        let content = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(content, PAR_MUX_SKILL_MD);
        assert!(content.starts_with("---\nname: par-mux\n"));
    }

    #[test]
    fn reinstall_overwrites_an_older_copy() {
        let root = tempfile::tempdir().expect("temp dir");
        let path = install_skill_into(root.path()).expect("install");
        std::fs::write(&path, "# an older par-term-era copy").expect("age the copy");

        install_skill_into(root.path()).expect("reinstall");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            PAR_MUX_SKILL_MD
        );
    }
}
