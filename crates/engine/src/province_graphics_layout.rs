//! Province-graphics presentation layout, excluded from hourly state acquisition.

/// Building-vector update entry.
pub const BUILDING_VECTOR_UPDATE_RVA: usize = 0x0127_b2e0;
/// Building-vector update entry sentinel.
pub const BUILDING_VECTOR_UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
/// Visual-record pointer array.
pub const BUILDING_RECORDS: usize = 0x1548;
/// Visual-record u32 count.
pub const BUILDING_RECORD_COUNT: usize = 0x1554;
/// Visual-record pointer-slot width.
pub const BUILDING_RECORD_POINTER_STRIDE: usize = 8;
/// Land/naval counter-position table.
pub const UNIT_STACKS: usize = 0x11d8;
/// Counter-position slot stride, three f32 components.
pub const UNIT_STACK_STRIDE: usize = 0x0c;
/// Owning state-graphics pointer.
pub const STATE_GRAPHICS: usize = 0x1508;
/// Stack-position table writer.
pub const FILL_STACK_TABLE_RVA: usize = 0x0127_a390;
/// Province-definition pointer selecting its authored locator index.
pub const PROVINCE_DEFINITION: usize = 0x11b8;
/// First locator-float array, used as yaw for stationary units.
pub const LOCATOR_FLOAT_A: usize = 0x13ac;
/// Second locator-float array; unit-matrix meaning is unresolved.
pub const LOCATOR_FLOAT_B: usize = 0x1448;
/// Locator-float element width.
pub const LOCATOR_FLOAT_STRIDE: usize = 4;
/// Special slot position, [f32; 3].
pub const SPECIAL_STACK_POSITION: usize = 0x13a0;
/// Special-slot first float.
pub const SPECIAL_LOCATOR_FLOAT_A: usize = 0x1444;
/// Special-slot second float.
pub const SPECIAL_LOCATOR_FLOAT_B: usize = 0x14e0;
/// Special-slot selector stored separately from ordinary arrays.
pub const SPECIAL_STACK_SLOT: usize = 0x26;
/// Land-combat icon producer.
pub const LAND_ICON_PRODUCER_RVA: usize = 0x0127_bfc0;
/// Supply-icon update following definition-based hub predicates.
pub const UPDATE_SUPPLY_ICONS_RVA: usize = 0x0127_dac0;

/// Thirty-eight source stack-icon pointers; ordinary counters skip slot nineteen.
pub const STACK_ICONS: usize = 0x1560;
