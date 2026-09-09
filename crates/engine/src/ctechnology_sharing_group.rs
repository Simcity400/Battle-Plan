//! CTechnologySharingGroup, including fields reached through its primary base.
//! RTTI proves the CTechnologySharing base at zero; field ownership within that
//! base is not separated by the archived walk. No hourly consistency is claimed.

/// CTechnologySharingGroup primary vtable; build_profile.rs:309, confirmed by pinned-image RTTI.
pub const VTABLE_RVA: usize = 0x0270_b720;
/// CTechnologySharingGroup extent declared by the old profile; build_profile.rs:311, allocation-size proof not retained there.
pub const PROFILE_OBJECT_SIZE: usize = 0x50;
/// CTechnologySharingGroup lexer ordinal u32; build_profile.rs:312 and collector read_compact_tech_sharing_groups:147134.
pub const ID_TOKEN: usize = 0x08;
/// CTechnologySharingGroup raw base bonus i64/100000; build_profile.rs:313, declared field not emitted by the final old walk.
pub const BASE_BONUS: usize = 0x10;
/// CTechnologySharingGroup member u32 country-link array; build_profile.rs:314 and collector D69 paired-vector walk.
pub const MEMBERS: usize = 0x18;
/// CTechnologySharingGroup member capacity u32; build_profile.rs:315 and collector D69 paired-vector walk.
pub const MEMBER_CAPACITY: usize = 0x20;
/// CTechnologySharingGroup member count u32; build_profile.rs:316 and collector D69 paired-vector walk.
pub const MEMBER_COUNT: usize = 0x24;
/// CTechnologySharingGroup member allocator slot; build_profile.rs:317, retained layout declaration.
pub const MEMBER_ALLOCATOR: usize = 0x28;
/// CTechnologySharingGroup dynamic bonus i64 array; build_profile.rs:318 and collector D69 paired-vector walk.
pub const BONUSES: usize = 0x30;
/// CTechnologySharingGroup bonus capacity u32; build_profile.rs:319 and collector D69 paired-vector walk.
pub const BONUS_CAPACITY: usize = 0x38;
/// CTechnologySharingGroup bonus count u32; build_profile.rs:320 and collector D69 paired-vector walk.
pub const BONUS_COUNT: usize = 0x3c;
/// CTechnologySharingGroup bonus allocator slot; build_profile.rs:321, retained layout declaration.
pub const BONUS_ALLOCATOR: usize = 0x40;
/// CTechnologySharingGroup template pointer; build_profile.rs:322, pointee class not established by the old walk.
pub const TEMPLATE: usize = 0x48;
/// CTechnologySharingGroup country-handle element width; build_profile.rs:324 and collector D69 canonicalization.
pub const MEMBER_STRIDE: usize = 4;
/// CTechnologySharingGroup signed bonus element width; build_profile.rs:325 and collector D69 reads full i64 values.
pub const BONUS_STRIDE: usize = 8;
/// CTechnologySharingGroup raw bonus denominator; compact_contract/tech_sharing_groups.rs:10, old D69 preserves i64.
pub const BONUS_SCALE: i64 = 100_000;
