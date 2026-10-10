//! Blend descriptor; its concrete type is unknown.
//!
//! Presentation-only layout; no reader or hook is installed. Offsets are relative
//! to the descriptor before copying into a GPU state wrapper.

/// Descriptor initializer.
pub const INITIALIZE_RVA: usize = 0x023f_7110;
/// Descriptor copy and lowering function.
pub const LOWER_RVA: usize = 0x023f_cb30;
/// Blend-factor scanner.
pub const SCAN_FACTOR_RVA: usize = 0x023f_73b0;
/// Blend-operation scanner.
pub const SCAN_OPERATION_RVA: usize = 0x023f_75c0;
/// Write-mask scanner.
pub const SCAN_WRITE_MASK_RVA: usize = 0x023f_81b0;
/// Blend-factor translator.
pub const LOWER_FACTOR_RVA: usize = 0x0241_0f30;
/// Blend-operation translator.
pub const LOWER_OPERATION_RVA: usize = 0x0241_1000;
/// Descriptor byte extent copied into the wrapper.
pub const COPIED_BYTES: usize = 0x24;
/// Blend-enable byte.
pub const ENABLED: usize = 0x00;
/// Shared alpha-test/alpha-to-coverage byte, unused by lowering.
pub const ALPHA_TEST_OR_COVERAGE: usize = 0x01;
/// Source-color factor, u32.
pub const SOURCE_COLOR_FACTOR: usize = 0x04;
/// Destination-color factor, u32.
pub const DESTINATION_COLOR_FACTOR: usize = 0x08;
/// Color operation, u32.
pub const COLOR_OPERATION: usize = 0x0c;
/// Source-alpha factor, u32.
pub const SOURCE_ALPHA_FACTOR: usize = 0x10;
/// Destination-alpha factor, u32.
pub const DESTINATION_ALPHA_FACTOR: usize = 0x14;
/// Alpha operation, u32.
pub const ALPHA_OPERATION: usize = 0x18;
/// RGBA write mask, u32; lowering consumes its low four bits.
pub const WRITE_MASK: usize = 0x1c;
/// Unclassified u32 initialized to six and only copied by lowering.
pub const UNCLASSIFIED_20: usize = 0x20;
/// Alpha source/destination/operation tuple selecting inheritance from color.
pub const ALPHA_INHERIT_SENTINEL: [u32; 3] = [1, 0, 0];
