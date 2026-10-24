//! Country strategic-operative object owning intel networks. Its owner
//! field is a raw country reference.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x29a_2358;
/// Strategic-operative allocation size.
pub const SIZE: usize = 0x1c8;
/// Constructor initializing vtable and owner.
pub const CONSTRUCTOR_RVA: usize = 0x11f_1ae0;
/// Raw owner country reference, u32.
pub const OWNER: usize = 0x08;
/// Intel-network pointer-vector data.
pub const NETWORKS_DATA: usize = 0x10;
/// Control-trade influence map on the target country's object, keyed by the
/// influencing country (`strategic_operatives::TRADE_*` entries). Saved as
/// `trade_influence_modifier_of_buyers`.
pub const TRADE_INFLUENCE: usize = 0xd0;
/// Diplomatic-pressure map on the target country's object, keyed by the
/// pressuring country (`strategic_operatives::PRESSURE_*` entries). Saved as
/// `diplomatic_pressure_from_countries`.
pub const DIPLOMATIC_PRESSURE: usize = 0x110;
/// Mission activity, signed i64 fixed5, saved as
/// `subversive_activity_level.value`.
pub const MISSION_ACTIVITY: usize = 0x170;
/// Operation activity, signed i64 fixed5, saved as `danger_level.value`.
pub const OPERATION_ACTIVITY: usize = 0x1a8;
