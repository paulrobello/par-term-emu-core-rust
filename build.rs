//! Build script for par-term-emu-core-rust
//!
//! Generates Protocol Buffer code when the `streaming` feature is enabled.

fn main() {
    #[cfg(feature = "streaming")]
    {
        // Only rebuild if proto files change
        println!("cargo:rerun-if-changed=proto/terminal.proto");

        // Compile protobuf schema
        prost_build::Config::new()
            .compile_protos(&["proto/terminal.proto"], &["proto/"])
            .expect("Failed to compile Protocol Buffer schema");
    }
}
