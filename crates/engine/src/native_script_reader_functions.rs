//! Shared script-reader and generated-token layouts.
//! These descriptors supply locations, without implementing another parser or
//! invoking game code.

/// Boolean field reader preserving its destination pointer during parsing.
pub const READ_BOOL_FIELD_RVA: usize = 0x024a_bca0;
/// Boolean assignment loader advancing tokens and resolving variables.
pub const READ_BOOL_ASSIGNMENT_RVA: usize = 0x022f_0760;
/// Boolean token parser writing one byte on success, otherwise leaving the destination unchanged.
pub const READ_BOOL_TOKEN_RVA: usize = 0x024b_06e0;
/// String field reader reporting disallowed line breaks through its error path.
pub const READ_STRING_FIELD_RVA: usize = 0x024a_bb50;
/// Token-to-string reader selecting the current token and destination string.
pub const READ_STRING_TOKEN_RVA: usize = 0x024b_05e0;
/// Float token parser producing f32 values and reporting conversion success.
pub const READ_FLOAT_TOKEN_RVA: usize = 0x024b_0320;
/// Ordered vector reader with four-byte components and expected axis-token gates.
pub const READ_AXIS_COMPONENTS_RVA: usize = 0x00ac_ebe0;
/// Current-token resolver for authored variables, replacing the token with its resolved value.
pub const RESOLVE_VARIABLE_TOKEN_RVA: usize = 0x024a_b540;
/// Reader error-report helper for parse failure or disallowed line breaks.
pub const REPORT_FIELD_ERROR_RVA: usize = 0x024a_cee0;
/// Generated-token lookup scanning signed i32 IDs and falling back to its first entry.
pub const LOOKUP_TOKEN_RVA: usize = 0x0205_7e20;
/// Generated-token registry start.
pub const TOKEN_REGISTRY_RVA: usize = 0x0337_4210;
/// Generated-token registry exclusive end.
pub const TOKEN_REGISTRY_END_RVA: usize = 0x0343_15b8;
/// Generated-token registry entry stride.
pub const TOKEN_ENTRY_STRIDE: usize = 0x48;
/// Authored-key registration entry.
pub const REGISTER_AUTHORED_KEYS_RVA: usize = 0x0008_2cb0;
/// Individual token registration storing a 32-bit ID and copying a terminated key name.
pub const REGISTER_TOKEN_RVA: usize = 0x024a_fc10;
