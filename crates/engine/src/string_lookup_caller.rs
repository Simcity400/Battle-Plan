//! String-lookup caller object. Nested lookup stages retain the same
//! caller base and read its +0xb8 member. The pointed-to class is not
//! established. String data comes from the separate `string_lookup_source`.

/// Pointer at caller +0xb8 passed as the string helper's first argument.
pub const MEMBER_POINTER: usize = 0xb8;
