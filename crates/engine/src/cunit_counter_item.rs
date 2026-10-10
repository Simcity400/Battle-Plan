//! Unit-counter presentation layout, excluded from hourly capture.
//! The tooltip builder uses the secondary base. Per-frame control updates are
//! separate; offsets below are relative to the primary object.

/// CUnitCounterItem primary vtable.
pub const VTABLE_RVA: usize = 0x029f_f890;
/// CUnitCounterItem secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x029f_f930;
/// Secondary subobject displacement.
pub const SECONDARY_SUBOBJECT: usize = 0x18;
/// Counter kind, i32; read relative to the primary object.
pub const KIND: usize = 0x180;
/// GUI-root pointer; read relative to the primary object.
pub const GUI_ROOT: usize = 0x10;
/// CUnitCounterItem allocation size.
pub const SIZE: usize = 0xbb0;
/// CUnitCounterItem deleting destructor.
pub const DELETING_DESTRUCTOR_RVA: usize = 0x018b_e910;
/// CUnitCounterItem tooltip builder.
pub const TOOLTIP_BUILDER_RVA: usize = 0x018c_00d0;
/// CUnitCounterItem member append.
pub const APPEND_MEMBER_RVA: usize = 0x018b_e9e0;
/// CUnitCounterItem grouping predicate.
pub const GROUPING_COMPATIBILITY_RVA: usize = 0x018b_ef20;
/// CUnitCounterItem per-frame control update.
pub const UPDATE_RVA: usize = 0x018c_4b50;
/// CUnitCounterItem land/railway-gun control update.
pub const LAND_UPDATE_RVA: usize = 0x018c_5ce0;
/// CUnitCounterItem naval control update.
pub const NAVAL_UPDATE_RVA: usize = 0x018c_69c0;
/// CUnitCounterItem authored child-slot binder.
pub const BIND_CONTROLS_RVA: usize = 0x018c_70c0;
/// CUnitCounterItem representative typed reference, two u32 words.
pub const REPRESENTATIVE_REFERENCE: usize = 0x30;
/// CUnitCounterItem member-reference array pointer.
pub const MEMBER_REFERENCES: usize = 0x38;
/// CUnitCounterItem i32 member-array capacity.
pub const MEMBER_CAPACITY: usize = 0x40;
/// CUnitCounterItem i32 member count.
pub const MEMBER_COUNT: usize = 0x44;
/// CUnitCounterItem member-array allocator pointer.
pub const MEMBER_ALLOCATOR: usize = 0x48;
/// CUnitCounterItem packed typed-reference stride.
pub const MEMBER_REFERENCE_STRIDE: usize = 8;
/// Count-background control pointer.
pub const COUNT_BACKGROUND: usize = 0x60;
/// Selected-count control pointer.
pub const COUNT_SELECTED: usize = 0x68;
/// Counter-overlay control pointer.
pub const COUNTER_OVERLAY: usize = 0x70;
/// Ideology control pointer.
pub const COUNTER_IDEOLOGY: usize = 0x78;
/// No-orders control pointer.
pub const NO_ORDERS: usize = 0x80;
/// Invalid-orders control pointer; assigned only when kind differs from two.
pub const INVALID_ORDERS: usize = 0x88;
/// Group-marker control pointer.
pub const GROUP_MARKER: usize = 0x90;
/// Low-supply control pointer.
pub const LOW_SUPPLY: usize = 0x98;
/// Unit-type control pointer.
pub const TYPE_CONTROL: usize = 0xa0;
/// Count-text control pointer.
pub const COUNT_TEXT: usize = 0xa8;
/// Strength-bar control pointer.
pub const STRENGTH_BAR: usize = 0xb0;
/// Organization-bar control pointer; assigned only when kind differs from two.
pub const ORGANISATION_BAR: usize = 0xb8;
/// Flag control pointer.
pub const FLAG: usize = 0xc0;
/// Large-flag control pointer.
pub const LARGE_FLAG: usize = 0xc8;
/// General-portrait control pointer; visibility is enabled for kind zero.
pub const GENERAL_PORTRAIT: usize = 0xd0;
/// Entrenchment-icon control pointer; assigned only for kind zero.
pub const DIG_IN_ICON: usize = 0xd8;
/// Action-icon control pointer; assigned only when kind differs from one.
pub const ACTION_ICON: usize = 0xe0;
/// Defending-area control pointer; assigned only for kind zero.
pub const DEFENDING_AREA: usize = 0xe8;
/// Attrition control pointer; assigned only when kind differs from one.
pub const ATTRITION: usize = 0xf0;
/// Training-icon control pointer; assigned only for kind zero.
pub const TRAINING_ICON: usize = 0xf8;
/// HQ-badge control pointer; assigned only for kind zero.
pub const HQ_BADGE: usize = 0x100;
/// Leader-enroute control pointer; assigned only for kind zero.
pub const LEADER_EN_ROUTE: usize = 0x108;
/// HQ horizontal-offset value-object pointer; assigned only for kind zero.
pub const HQ_X_OFFSET_VALUE: usize = 0x110;
/// Active-abilities control pointer; assigned only for kind zero.
pub const ACTIVE_ABILITIES: usize = 0x118;
/// Capital-count text control pointer; assigned only for kind one.
pub const CAPITAL_COUNT_TEXT: usize = 0x120;
/// Air-base icon control pointer; assigned only for kind one.
pub const AIR_BASE_ICON: usize = 0x128;
/// Fleet-pride control pointer; assigned only for kind one.
pub const PRIDE_OF_THE_FLEET: usize = 0x130;
/// Capital-ship control pointer; assigned only for kind one.
pub const CAPITAL_SHIP: usize = 0x138;
