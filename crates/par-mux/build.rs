//! Build script for par-mux: bakes the build identity `mux::build_stamp()`
//! reports (`PAR_TERM_CORE_BUILD_SHA`).

fn main() {
    emit_build_stamp();
}

/// Bake a build identity into the crate so a daemon and the clients linked
/// against it can tell whether they were built from the same source.
///
/// The stamp is the short git sha of the checkout the crate was compiled
/// from, plus `-dirty` when tracked files carry uncommitted changes. A
/// crates.io source tarball has no `.git`, so the stamp falls back to a
/// content digest of the crate source (`src-<fnv1a-16hex>`): deterministic
/// for identical source, different the moment any hashed file differs, so
/// same-version drift stays detectable in release builds instead of
/// collapsing to `+unknown` (see `mux::build_stamp`).
///
/// Rerun-on-`.git/HEAD` keeps the stamp moving with commits: without it the
/// env would be frozen at the first build of the checkout and every later
/// commit would silently keep the old identity. The reflog-style dance of
/// branch switches also updates HEAD, which is exactly when a stale stamp
/// would lie. The digest fallback instead reruns on the hashed inputs.
fn emit_build_stamp() {
    let head = std::path::Path::new(".git/HEAD");
    if head.exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
        // HEAD is usually a symref; the ref it names changes without HEAD
        // itself changing, so watch the resolved ref too when it is one.
        if let Ok(content) = std::fs::read_to_string(head) {
            if let Some(ref_path) = content.trim().strip_prefix("ref: ") {
                let ref_file = std::path::Path::new(".git").join(ref_path);
                if ref_file.exists() {
                    println!("cargo:rerun-if-changed={}", ref_file.display());
                }
            }
        }
    }
    let sha = git_short_sha().unwrap_or_else(|| {
        // No git identity (crates.io tarball): the source digest IS the
        // identity, so its inputs must re-run this script when they change —
        // without these, a rebuild after editing a hashed file would keep
        // serving the stale env.
        println!("cargo:rerun-if-changed=src");
        println!("cargo:rerun-if-changed=Cargo.toml");
        source_digest().unwrap_or_else(|| "unknown".to_string())
    });
    println!("cargo:rustc-env=PAR_TERM_CORE_BUILD_SHA={sha}");
}

/// Deterministic FNV-1a digest over the crate's own source: every file
/// under `src/` plus `Cargo.toml` and `build.rs`, walked in sorted path
/// order, hashing each path and each CR-normalized content (the same
/// line-ending normalization as the proto checksum — a tarball must hash
/// identically regardless of checkout settings). `None` only when there is
/// no `src/` tree to hash, which never holds for a real build.
fn source_digest() -> Option<String> {
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from("src")];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    if files.is_empty() {
        return None;
    }
    files.push(std::path::PathBuf::from("Cargo.toml"));
    files.push(std::path::PathBuf::from("build.rs"));
    files.sort();

    let mut hash: u64 = 0xcbf29ce484222325;
    let mix = |hash: &mut u64, bytes: &[u8]| {
        for &byte in bytes {
            if byte == b'\r' {
                continue;
            }
            *hash ^= u64::from(byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    for path in &files {
        let bytes = std::fs::read(path).ok()?;
        // Forward-slash rel path, so the digest is separator-independent.
        let rel = path.to_string_lossy().replace('\\', "/");
        mix(&mut hash, rel.as_bytes());
        mix(&mut hash, &bytes);
    }
    Some(format!("src-{hash:016x}"))
}

/// The checkout's short sha, with `-dirty` appended when tracked files have
/// uncommitted changes. `None` when git is absent, the checkout is not a
/// repository (crates.io tarballs), or the enclosing repository is not this
/// crate's own — never an error, the stamp degrades to the source digest.
fn git_short_sha() -> Option<String> {
    // ARC-120: a crate vendored inside another repository (or a crates.io
    // tarball extracted in one) would otherwise stamp the OUTER repo's
    // commit. Git identity describes this crate only when the toplevel is
    // the manifest dir; canonicalize both (macOS /tmp -> /private/tmp).
    let toplevel = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !toplevel.status.success() {
        return None;
    }
    let toplevel = String::from_utf8(toplevel.stdout).ok()?;
    let toplevel = std::fs::canonicalize(toplevel.trim()).ok()?;
    let manifest_dir = std::fs::canonicalize(std::env::var_os("CARGO_MANIFEST_DIR")?).ok()?;
    if toplevel != manifest_dir {
        return None;
    }

    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() {
        return None;
    }
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    let dirty = status.status.success()
        && String::from_utf8(status.stdout)
            // Any porcelain line that is not untracked (`??`) is a staged or
            // unstaged change to a tracked file.
            .map(|out| out.lines().any(|l| !l.starts_with("??")))
            .unwrap_or(false);
    Some(if dirty { format!("{sha}-dirty") } else { sha })
}
