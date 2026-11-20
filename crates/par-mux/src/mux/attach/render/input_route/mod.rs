//! Input routing: plain-byte runs through the prefix scanner and the
//! modal owners, and mouse reports through the overlay hit-testers.

pub(super) mod keyboard;
pub(super) mod mouse;
pub(super) mod pointer;

pub(super) use keyboard::InputRouteState;
