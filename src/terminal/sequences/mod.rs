//! VTE sequence handling modules
//!
//! This module contains implementations of the `vte::Perform` trait methods,
//! split into logical submodules for maintainability:
//! - `csi`: CSI (Control Sequence Introducer) sequences
//! - `osc`: OSC (Operating System Command) sequences
//! - `esc`: ESC (Escape) sequences
//! - `dcs`: DCS (Device Control String) sequences (primarily Sixel graphics)

pub mod csi;
pub mod dcs;
pub mod esc;
pub mod osc;
