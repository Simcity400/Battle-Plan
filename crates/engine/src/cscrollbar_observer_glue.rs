//! Embedded scrollbar-observer glue for an instant text box.
//! This module installs no observer.

/// Callback receiver pointer storing the text-box owner.
pub const RECEIVER: usize = 0x8;
/// Scrollbar callback pointer.
pub const CALLBACK: usize = 0x10;
