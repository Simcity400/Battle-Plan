//! String-helper metadata; this module implements no string operations or game calls.

/// Byte-exact suffix comparison without case folding.
pub const ENDS_WITH_RVA: usize = 0x024b_6ab0;
/// Substring replacement of the first occurrence only.
pub const REPLACE_FIRST_RVA: usize = 0x024b_7eb0;
/// Unsigned UTF-16 comparison through the first NUL or mismatch; returns the code-unit difference.
pub const COMPARE_UTF16_RVA: usize = 0x020a_f800;
