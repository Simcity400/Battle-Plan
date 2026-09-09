//! CMapArrowDefinition native authored arrow parameters in a729.

/// Definition constructor; map_viewer/src/arrow_submission_order.rs:350,797-799 identifies CMapArrowDefinition and confirms zero initialization.
pub const CONSTRUCT_RVA: usize = 0x0124_ff40;
/// Definition parser; arrow_submission_order.rs:800-803 confirms token 0x2efb targets this definition's byte+200 field.
pub const READ_TOKEN_RVA: usize = 0x0125_5760;
/// Width-scale f32; arrow_submission_order.rs:350,798-806 confirms constructor zero and getter read at byte+200.
pub const WIDTH_SCALE: usize = 0xc8;
/// Native default width-scale bits; arrow_submission_order.rs:798 checks the constructor's literal zero store.
pub const DEFAULT_WIDTH_SCALE_BITS: u32 = 0;
