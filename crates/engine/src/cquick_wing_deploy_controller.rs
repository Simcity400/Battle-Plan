//! Quick-wing-deployment candidate filters and bucket vectors.
//! These presentation inputs do not establish an hourly state-table walk.

/// Deployment-status pointer supplied by the country.
pub const CANDIDATE_OWNER: usize = 0x28;
/// Production-status pointer supplied by the country.
pub const ARCHETYPE_CONTEXT: usize = 0x30;
/// Controller allocation size.
pub const SIZE: usize = 0xf8;
/// Constructor receiving deployment and production status pointers.
pub const CONSTRUCTOR_RVA: usize = 0x0195_18e0;
/// Bucket accessor.
pub const GET_OPTIONS_RVA: usize = 0x0195_1a10;
/// Forwarding entry.
pub const GET_OPTIONS_THUNK_RVA: usize = 0x0195_1ba0;
/// Option builder populating four buckets.
pub const BUILD_OPTIONS_RVA: usize = 0x0195_2420;
/// Bucket-zero option-array data.
pub const BUCKET_0: usize = 0x38;
/// Bucket-one option-array data.
pub const BUCKET_1: usize = 0x50;
/// Bucket-two option-array data.
pub const BUCKET_2: usize = 0x68;
/// Bucket-three option-array data.
pub const BUCKET_3: usize = 0x80;
/// Bucket-zero option-array count, i32.
pub const BUCKET_0_COUNT: usize = 0x44;
/// Bucket-one option-array count, i32.
pub const BUCKET_1_COUNT: usize = 0x5c;
/// Bucket-two option-array count, i32.
pub const BUCKET_2_COUNT: usize = 0x74;
/// Bucket-three option-array count, i32.
pub const BUCKET_3_COUNT: usize = 0x8c;
/// Common air-mask branch.
pub const AIR_MASK_TEST_RVA: usize = 0x0195_261b;
/// Exclusion-mask branch.
pub const EXCLUDED_MASK_TEST_RVA: usize = 0x0195_2624;
/// Flag-33 rejection branch.
pub const BIT_33_TEST_RVA: usize = 0x0195_2631;
/// Bucket-one byte predicate for the candidate field at +0x5b4.
pub const BUCKET_1_TEST_RVA: usize = 0x0195_2837;
/// Bucket-2 flag-20 predicate site.
pub const BUCKET_2_TEST_RVA: usize = 0x0195_2704;
/// Accepted candidate flags require a nonzero intersection with this u64 mask.
pub const AIR_FLAGS_MASK: u64 = 0x001f_0037_fc00;
/// Excluded candidate flags reject any intersection with this u64 mask.
pub const EXCLUDED_FLAGS_MASK: u64 = 0x001c_0001_0000;
/// Common rejected candidate flag, bit 33.
pub const REJECTED_FLAG_33: u64 = 1 << 33;
/// Bucket-two rejected candidate flag, bit 20.
pub const BUCKET_2_REJECTED_FLAG: u64 = 1 << 20;
