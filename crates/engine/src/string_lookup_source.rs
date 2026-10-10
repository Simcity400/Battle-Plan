//! Separate source object supplying string data to lookup stages. The
//! looked-up pointer slot supplies the source base and is forwarded as the
//! third argument. This does not identify the source as the caller object.

/// Embedded string at source +0x3b0 passed as the string helper's second
/// argument. Capacity and length fields establish string geometry.
pub const TEXT: usize = 0x3b0;
