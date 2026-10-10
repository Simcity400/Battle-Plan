//! Pdxmesh mesh-settings row offsets; no reader or capture path is enabled here.

/// Signed i32 object-name length; zero accepts immediately.
pub const OBJECT_NAME_LENGTH: usize = 0x10;
/// Signed i32 diffuse-override length; positive values replace the slot.
pub const DIFFUSE_OVERRIDE_LENGTH: usize = 0x38;
/// Signed i32 normal-override length; positive values replace the slot.
pub const NORMAL_OVERRIDE_LENGTH: usize = 0x58;
/// Signed i32 specular-override length; positive values replace the slot.
pub const SPECULAR_OVERRIDE_LENGTH: usize = 0x78;
/// Signed i32 mesh ordinal compared with the requested ordinal or wildcard.
pub const MESH_ORDINAL: usize = 0xcc;
/// Wildcard ordinal; other negative values do not match named rows.
pub const ANY_MESH_ORDINAL: i32 = -1;
