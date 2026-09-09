//! MSVC tree node instantiated for `NProject::CBreakthroughProgress`.
//!
//! No independent RTTI name was found for this container node. The offsets
//! come from archived `read_project_breakthrough_progress_rows` and its

/// Breakthrough tree node +0x00 left link; sentinel's left link points to the minimum node.
pub const LEFT: usize = 0x00;
/// Breakthrough tree node +0x08 parent link; used by engine-compatible successor traversal.
pub const PARENT: usize = 0x08;
/// Breakthrough tree node +0x10 right link; used by engine-compatible successor traversal.
pub const RIGHT: usize = 0x10;
/// Breakthrough tree node +0x19 nil byte; archived walker requires 1 for sentinel and 0 for entries.
pub const IS_NIL: usize = 0x19;
/// Breakthrough tree node +0x1c specialization handle u32; archived progress reader's key.
pub const SPECIALIZATION_HANDLE: usize = 0x1c;
/// Breakthrough tree node +0x20 points read as u32; native width and scaling need writer confirmation.
pub const POINTS: usize = 0x20;
