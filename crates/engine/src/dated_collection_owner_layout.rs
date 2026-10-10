//! Owner layout for an indexed collection of dated records.
//! Collection and flag offsets are relative to the collection owner.

/// Collection member containing the data pointer used by indexed updates.
pub const RECORDS: usize = 0x3b0;

/// Byte set to one after the collection operation.
pub const FLAG: usize = 0x3f9;
