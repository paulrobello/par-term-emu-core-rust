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
}
