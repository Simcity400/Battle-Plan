//! Depth/stencil descriptor; no reader or hook is installed.
//! Lowering duplicates one stencil face into both API faces. Masks are full
//! u32 values even though lowering consumes only each low byte.

/// Descriptor initializer.
pub const INITIALIZE_RVA: usize = 0x023f_7140;
/// Descriptor-copy and API-lowering operation.
pub const LOWER_RVA: usize = 0x023f_cfc0;
/// Comparison and stencil-encoding translator.
pub const LOWER_COMPARISON_RVA: usize = 0x0241_1060;
/// Descriptor byte size.
pub const COPIED_BYTES: usize = 0x2c;
/// Depth-enable byte.
pub const DEPTH_ENABLED: usize = 0x00;
/// Depth-write u32; enables writing only when equal to one.
pub const DEPTH_WRITE_MODE: usize = 0x04;
/// Depth-comparison encoding, u32.
pub const DEPTH_COMPARISON: usize = 0x08;
/// Stencil-enable byte.
pub const STENCIL_ENABLED: usize = 0x0c;
/// Stencil read mask, u32; initialized to all ones.
pub const STENCIL_READ_MASK: usize = 0x10;
/// Stencil write mask, u32; initialized to all ones.
pub const STENCIL_WRITE_MASK: usize = 0x14;
/// Stencil-fail operation encoding, u32.
pub const STENCIL_FAIL_OPERATION: usize = 0x18;
/// Stencil-depth-fail operation encoding, u32.
pub const STENCIL_DEPTH_FAIL_OPERATION: usize = 0x1c;
/// Stencil-pass operation encoding, u32.
pub const STENCIL_PASS_OPERATION: usize = 0x20;
/// Stencil comparison encoding, u32.
pub const STENCIL_COMPARISON: usize = 0x24;
/// Uninterpreted u32; initialized to zero and copied unchanged.
pub const UNCLASSIFIED_28: usize = 0x28;
