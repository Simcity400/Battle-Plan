//! Anonymous province-to-i64 hash bucket instantiation; RTTI name unknown.
//! The disrupted-supply bucket has a two-word key and is a different instantiation.

/// Province supply hash bucket stride; old attach_log.rs:141248 BUCKET_STRIDE in d81_hash_table_rows.
pub const STRIDE: usize = 0x18;
/// Bucket probe marker, u8; old d81_hash_table_rows reads `bucket[4]`, zero empty and 0xff terminal.
pub const MARKER: usize = 0x04;
/// Bucket province key, u32; old d81_hash_table_rows reads bucket+8.
pub const PROVINCE_ID: usize = 0x08;
/// Bucket supply value, native i64; old d81_hash_table_rows reads bucket+0x10 without narrowing.
pub const VALUE: usize = 0x10;
