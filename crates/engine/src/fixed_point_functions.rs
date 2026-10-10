//! Numeric serialization metadata; the passive collector calls none of these functions.

/// Signed numeric writer.
pub const WRITE_SIGNED_RVA: usize = 0x024a_e9a0;
/// Signed five-decimal reader.
pub const READ_FIXED5_RVA: usize = 0x024b_0480;
/// Five-decimal formatter; trims trailing fractional zeroes.
pub const FORMAT_FIXED5_RVA: usize = 0x024b_89f0;
/// Country numeric serialization wrapper.
pub const WRITE_COUNTRY_NUMBER_RVA: usize = 0x024a_e590;
/// Native numeric serialization helper.
pub const WRITE_NATIVE_NUMBER_RVA: usize = 0x024a_f9a0;

/// Unsigned decimal reader; returns zero when no prefix is consumed.
pub const READ_UNSIGNED_DECIMAL_TOKEN_RVA: usize = 0x024b_00c0;
/// Unsigned 64-bit radix conversion; overflow saturates to the maximum value.
pub const PARSE_INTEGER64_RADIX_RVA: usize = 0x0253_d1fc;
/// Signed-i32 assignment; accepts every nonzero formatted-scan result, including EOF.
pub const ASSIGN_SIGNED_INTEGER32_TOKEN_RVA: usize = 0x024b_02c0;
/// Signed 64-bit scanner; forwards conversion to a width-dependent destination writer.
pub const SCAN_SIGNED_INTEGER64_RVA: usize = 0x0254_6ccc;
/// Scan destination writer; four-byte destinations retain the low i32 of the signed 64-bit result.
pub const STORE_SCANNED_INTEGER_RVA: usize = 0x0254_77d0;
/// Signed decimal i32 reader.
pub const PARSE_SIGNED_DECIMAL32_RVA: usize = 0x0254_af28;
/// Formatted scanner; distinguishes negative EOF from zero matching failure.
pub const FORMATTED_SCAN_RVA: usize = 0x0254_681c;
/// Formatted-scan EOF sentinel, distinct from matching failure.
pub const SCAN_END_OF_INPUT: i32 = -1;
/// Matching-failure sentinel; only this result is rejected by authored fixed-point reading.
pub const SCAN_MATCHING_FAILURE: i32 = 0;
/// Lua fixed5 table loader; rounds in f32 before storing signed i64.
pub const LOAD_LUA_FIXED5_TABLE_RVA: usize = 0x0205_9cf0;
/// Lua numeric-to-fixed5 conversion; retains ordered f32 scaling and sign-dependent rounding before i64 conversion.
pub const LUA_NUMBER_TO_FIXED5_RVA: usize = 0x0205_d560;
