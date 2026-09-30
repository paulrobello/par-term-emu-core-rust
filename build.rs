//! Build script for par-term-emu-core-rust
//!
//! Protocol Buffer code is pre-generated in src/streaming/terminal.pb.rs
//! to avoid requiring protoc at build time.
//!
//! To regenerate protobuf code after modifying proto/terminal.proto, run
//! `make proto-rust` (requires protoc). The Makefile also stamps the
//! generated file with a checksum of the proto, which this build script
//! verifies — see `check_proto_staleness`.

fn main() {
    emit_build_stamp();

    // Regenerate protobuf code only when explicitly requested
    #[cfg(feature = "regenerate-proto")]
    {
        println!("cargo:rerun-if-changed=proto/terminal.proto");

        prost_build::Config::new()
            .compile_protos(&["proto/terminal.proto"], &["proto/"])
            .expect("Failed to compile Protocol Buffer schema. Make sure protoc is installed.");
    }

    // Catch wire-format drift (ARC-020): the checked-in
    // src/streaming/terminal.pb.rs is the build-time source of truth (no protoc
    // dependency), so an edited proto/terminal.proto that wasn't regenerated
    // would silently desync the Rust from the schema. `make proto-rust` stamps
    // the generated file with an FNV-1a checksum of the proto; a mismatch
    // means the proto changed without a regeneration. Content, not mtimes —
    // checkout and clone do not preserve mtime ordering, which made the
    // original mtime comparison fire on every build of an unmodified tree.
    check_proto_staleness();
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

/// FNV-1a 64-bit checksum, matching the stamp written by `make proto-rust`.
///
/// Every `\r` is dropped first: git can check the proto out with CRLF on
/// Windows, and the checksum must not depend on checkout settings. The proto
/// contains no bare `\r`, so this is exactly line-ending normalization.
fn fnv1a_normalized(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        if byte == b'\r' {
            continue;
        }
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The `// proto-fnv1a:<16 hex>` stamp on the generated file's first line.
fn embedded_checksum(bytes: &[u8]) -> Option<u64> {
    let first_line_end = bytes.iter().position(|&b| b == b'\n')?;
    let line = std::str::from_utf8(&bytes[..first_line_end]).ok()?;
    let hex = line.strip_prefix("// proto-fnv1a:")?;
    u64::from_str_radix(hex.trim(), 16).ok()
}

fn check_proto_staleness() {
    let proto = std::path::Path::new("proto/terminal.proto");
    let checked_in = std::path::Path::new("src/streaming/terminal.pb.rs");

    let (Ok(proto_bytes), Ok(pb_bytes)) = (std::fs::read(proto), std::fs::read(checked_in)) else {
        return;
    };

    println!("cargo:rerun-if-changed=proto/terminal.proto");
    println!("cargo:rerun-if-changed=src/streaming/terminal.pb.rs");

    // No stamp: the file predates checksum stamping (or was regenerated by
    // raw cargo rather than make proto-rust). Nothing to compare against.
    let Some(embedded) = embedded_checksum(&pb_bytes) else {
        return;
    };

    if fnv1a_normalized(&proto_bytes) != embedded {
        println!(
            "cargo:warning=ARC-020: proto/terminal.proto changed without regenerating \
             src/streaming/terminal.pb.rs (checksum mismatch). Run `make proto-rust` \
             and commit the result."
        );
    }
}
