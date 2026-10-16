//! State template selected by the simulation state.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0292_ce90;
/// Province-ID array.
pub const PROVINCES: usize = 0x08;
/// Province count, signed i32.
pub const PROVINCE_COUNT: usize = 0x14;
/// Fallback display-name string used when both state names are empty.
pub const DISPLAY_NAME: usize = 0x20;
/// State ID, u32, copied into the simulation state.
pub const ID: usize = 0xa0;
/// Resource-entry pointer.
pub const RESOURCES: usize = 0xf8;
/// Resource-array signed capacity, grown before append.
pub const RESOURCE_CAPACITY: usize = 0x100;
/// Resource-array signed count, incremented on append.
pub const RESOURCE_COUNT: usize = 0x104;
/// Resource-array allocator pointer.
pub const RESOURCE_ALLOCATOR: usize = 0x108;
/// Bounding-box centre x, i32.
pub const BOUNDING_CENTER_X: usize = 0x148;
/// Bounding-box centre y, i32.
pub const BOUNDING_CENTER_Y: usize = 0x14c;
/// Maximum bounding-box width/height, i32, separate from member count.
pub const BOUNDING_EXTENT: usize = 0x150;
/// Bounds initializer preserving integer coordinates and wrapping.
pub const UPDATE_BOUNDING_BOXES_RVA: usize = 0x00aa_dd40;
/// Constructor initializing resource fields.
pub const CONSTRUCTOR_RVA: usize = 0x00aa_ce20;
/// Resource loader converting source integers to i64 fixed5.
pub const LOAD_RESOURCES_RVA: usize = 0x00aa_f580;
/// Border reason-7 exemption byte for mixed terrain-bit edges.
pub const BORDER_REASON_7_EXEMPTION: usize = 0x15e;

/// Retained continent-definition pointer initialized to the database null
/// continent. Province traversal does not clear it first.
pub const CONTINENT: usize = 0x140;
