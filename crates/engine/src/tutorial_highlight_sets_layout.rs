//! Tutorial-highlight owner reached through in-game idler +0x960.
//! Membership uses province, state and strategic-region IDs.

/// State-ID array compared with province +0xc0 then state +0x58.
pub const STATE_IDS: usize = 0xf90;
/// State-ID count, i32, bounding the array at +0xf90.
pub const STATE_ID_COUNT: usize = 0xf9c;
/// Province-ID array compared with province +0xa4.
pub const PROVINCE_IDS: usize = 0xfa8;
/// Province-ID count, i32, bounding the array at +0xfa8.
pub const PROVINCE_ID_COUNT: usize = 0xfb4;
/// Strategic-region-ID array compared with province +0xc8 then region +0x58.
pub const STRATEGIC_REGION_IDS: usize = 0xfc0;
/// Strategic-region-ID count, i32, bounding the array at +0xfc0.
pub const STRATEGIC_REGION_ID_COUNT: usize = 0xfcc;
/// Highlight predicate comparing province, state and strategic-region membership.
pub const BORDER_MEMBERSHIP_SPLIT_RVA: usize = 0x0126_d450;
