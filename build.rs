//! Build script for par-term-emu-core-rust
//!
//! Protocol Buffer code is pre-generated in src/streaming/terminal.pb.rs
//! to avoid requiring protoc at build time.
//!
//! To regenerate protobuf code after modifying proto/terminal.proto:
//! 1. Install protoc (e.g., `brew install protobuf` or `apt-get install protobuf-compiler`)
//! 2. Run: cargo build --features streaming,regenerate-proto
//! 3. Copy output from target/debug/build/.../out/terminal.rs to src/streaming/terminal.pb.rs

fn main() {
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
    // would silently desync the Rust from the schema. Warn when the proto is
    // newer than the checked-in Rust.
    check_proto_staleness();
}

fn check_proto_staleness() {
    let proto = std::path::Path::new("proto/terminal.proto");
    let checked_in = std::path::Path::new("src/streaming/terminal.pb.rs");

    let (Ok(proto_meta), Ok(pb_meta)) = (std::fs::metadata(proto), std::fs::metadata(checked_in))
    else {
        return;
    };

    println!("cargo:rerun-if-changed=proto/terminal.proto");
    println!("cargo:rerun-if-changed=src/streaming/terminal.pb.rs");

    if let (Ok(proto_mtime), Ok(pb_mtime)) = (proto_meta.modified(), pb_meta.modified()) {
        if proto_mtime > pb_mtime {
            println!(
                "cargo:warning=ARC-020: proto/terminal.proto is newer than the checked-in \
                 src/streaming/terminal.pb.rs. The generated Rust may be stale — run \
                 `make proto-rust` to regenerate and commit the result."
            );
        }
    }
}
