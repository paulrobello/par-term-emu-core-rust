//! Shared Terminal access + method macros (ARC-003 / QA-001).
//!
//! `PyTerminal` owns a `Terminal` directly; `PyPtyTerminal` holds one behind an
//! `Arc<Mutex<Terminal>>` (via `PtySession`). Historically every shared method
//! was hand-duplicated between the two — ~155 copies that drift on every PR.
//!
//! [`TerminalAccess`] abstracts the "give me the terminal" step (returning a
//! `Deref<Target=Terminal>` guard for each type's native form), and the macros
//! below emit the shared `#[pymethods]` definitions ONCE, invoked per type.
//! Behavior is preserved (the macros use the clean `Terminal`-method form; the
//! duplicated `Ok::<_, ()>(lock())` dead-fallback copies in `pty.rs` are
//! dropped, which is safe because `parking_lot` locks never fail).

use crate::terminal::Terminal;

/// Unified read/write access to the underlying [`Terminal`] for both Python
/// wrapper types.
///
/// `term_ref` / `term_mut` return opaque `Deref`/`DerefMut` guards so each impl
/// can return its native borrow form (`&Terminal` for `PyTerminal`,
/// `MutexGuard<Terminal>` for `PyPtyTerminal`) while callers see a uniform
/// `Deref<Target = Terminal>`.
pub(crate) trait TerminalAccess {
    /// Shared (immutable) access to the terminal.
    fn term_ref(&self) -> impl std::ops::Deref<Target = Terminal>;
    /// Exclusive (mutable) access to the terminal.
    #[allow(dead_code)] // used once setters/mutating methods are migrated (ARC-003/QA-001 scaling)
    fn term_mut(&mut self) -> impl std::ops::DerefMut<Target = Terminal>;
}

/// Emit a small set of simple read-only getters for `$ty`, using
/// [`TerminalAccess::term_ref`]. Validates the shared-method macro pattern
/// (ARC-003/QA-001); the same shape scales to the full duplicated set.
#[macro_export]
macro_rules! impl_terminal_simple_getters {
    ($ty:ty) => {
        #[pymethods]
        impl $ty {
            /// Get the cursor color (OSC 12)
            fn cursor_color(&self) -> pyo3::PyResult<(u8, u8, u8)> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.cursor_color().to_rgb())
            }

            /// Whether the cursor is visible (DECTCE)
            fn cursor_visible(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.cursor().visible)
            }

            /// Whether bracketed-paste mode is active
            fn bracketed_paste(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.bracketed_paste())
            }

            /// Whether OSC 52 clipboard reads are allowed
            fn allow_clipboard_read(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.allow_clipboard_read())
            }

            /// Whether OSC 7 directory-tracking sequences are accepted
            fn accept_osc7(&self) -> pyo3::PyResult<bool> {
                let t = $crate::python_bindings::common::TerminalAccess::term_ref(self);
                Ok(t.accept_osc7())
            }
        }
    };
}
