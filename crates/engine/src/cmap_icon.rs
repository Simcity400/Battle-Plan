//! CMapIcon base fields and camera-dependent hourly counter state.

/// Map-icon base constructor.
pub const CONSTRUCT_RVA: usize = 0x0162_cd30;
/// CMapIcon vtable.
pub const VTABLE_RVA: usize = 0x029c_feb0;
/// Grouping predicate accepts a different icon object.
pub const GROUPING_PREDICATE_RVA: usize = 0x0162_cef0;
/// CMapIcon grouping virtual slot.
pub const GROUPING_PREDICATE_VSLOT: usize = 0x58;

/// Map-icon owning object pointer.
pub const OWNER: usize = 0x18;
/// Last-matched map-icon group pointer. Every compatible group replaces
/// it during the full loop; memberships need not be exclusive. Clearing
/// removes this retained pointer.
pub const OWNING_GROUP: usize = 0x30;
/// CMapIcon current world anchor f32[3].
pub const WORLD_ANCHOR: usize = 0x54;
/// CMapIcon displayed anchor f32[3].
pub const DISPLAYED_WORLD_ANCHOR: usize = 0x60;
/// Map-icon u8 type and live-list index.
pub const TYPE: usize = 0x6c;
/// Map-icon flag byte.
pub const FLAGS: usize = 0x74;
/// Map-icon in-use mask; clear permits pool reuse.
pub const IN_USE_MASK: u8 = 1;
/// Anchor-present flag, checked before interpolation.
pub const HAS_WORLD_ANCHOR_MASK: u8 = 0x04;
/// Map-icon u32 last successful projection pass.
pub const UPDATE_WATERMARK: usize = 0x78;
/// Projection-test dispatch displacement, reached before retaining membership.
pub const PROJECTION_TEST_VSLOT: usize = 0x10;
/// Map-icon release virtual slot.
pub const RELEASE_VSLOT: usize = 0x38;
/// Common map-icon world-position getter vslot.
pub const WORLD_POSITION_VSLOT: usize = 0x20;
/// Common map-icon direction-out getter vslot.
pub const DIRECTION_OUT_VSLOT: usize = 0x28;
/// Camera-dependent common map-icon base transform.
pub const BASE_TRANSFORM_RVA: usize = 0x0162_d5b0;
/// Common null-direction output; naval and result icons have no direction.
pub const NULL_DIRECTION_OUT_RVA: usize = 0x0012_0030;
/// Victory-point icon type.
pub const VICTORY_POINT_TYPE: u8 = 2;
/// Resource-state icon type.
pub const RESOURCE_STATE_TYPE: u8 = 3;
/// Raid icon type.
pub const RAID_TYPE: u8 = 0x1d;
