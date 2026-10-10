//! CLandCombatMapIcon presentation layout, with no hourly state walker.
//! Position uses the combat province and first attacker with a live map entity.
//! Direction is written through the virtual position pointer, so direct writes
//! to the icon field do not describe the complete writer path.

/// CLandCombatMapIcon primary vtable.
pub const VTABLE_RVA: usize = 0x029f_d690;
/// CLandCombatMapIcon secondary base vptr offset.
pub const SECONDARY_BASE: usize = 0x80;
/// CLandCombatMapIcon constructor.
pub const CONSTRUCTOR_RVA: usize = 0x018a_5d50;
/// CLandCombatMapIcon world-position method.
pub const WORLD_POSITION_RVA: usize = 0x018a_94b0;
/// CLandCombatMapIcon direction-pointer method.
pub const DIRECTION_OUT_RVA: usize = 0x018a_8470;
/// CLandCombatMapIcon per-frame control update.
pub const UPDATE_RVA: usize = 0x018b_4bd0;
/// CLandCombatMapIcon category.
pub const MAP_ICON_BUCKET: u32 = 0xb;
/// Packed combat reference u64; both words gate resolution.
pub const COMBAT_REFERENCE: usize = 0x590;
/// CLandCombatMapIcon main control pointer.
pub const MAIN_CONTROL: usize = 0x598;
/// CLandCombatMapIcon battle_indicator_direction control pointer.
pub const DIRECTION_CONTROL: usize = 0x5c8;
/// CLandCombatMapIcon state-frame control pointer.
pub const STATE_FRAME_CONTROL: usize = 0x5d0;
/// CLandCombatMapIcon combat-kind frame control pointer.
pub const COMBAT_KIND_CONTROL: usize = 0x5d8;
/// CLandCombatMapIcon conditional indicator pointer.
pub const CONDITIONAL_INDICATOR_CONTROL: usize = 0x5e0;
/// CLandCombatMapIcon progress text control pointer.
pub const PROGRESS_TEXT_CONTROL: usize = 0x5e8;
/// CLandCombatMapIcon direction f32 radians.
pub const DIRECTION: usize = 0x5f0;
/// CLandCombatMapIcon direction child virtual setter offset.
pub const DIRECTION_CONTROL_SET_ANGLE_VTABLE_SLOT: usize = 0x328;
/// CLandCombatMapIcon base anchor slot.
pub const BASE_STACK_SLOT: usize = 10;
/// CLandCombatMapIcon midpoint multiplier storage.
pub const MIDPOINT_FACTOR_RVA: usize = 0x0270_9c34;
/// CLandCombatMapIcon direction sign-mask storage.
pub const DIRECTION_SIGN_MASK_RVA: usize = 0x0271_15d0;
