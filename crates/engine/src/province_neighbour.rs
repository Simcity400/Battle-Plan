//! One native province-definition neighbor entry; the old reader does not establish its C++ class name.

pub const STRIDE: usize = 0x30;
/// Province-definition neighbor i32 province id; old land_combat_province_heading_index reads entry+0x08.
pub const PROVINCE_ID: usize = 0x08;
/// Province-definition adjacency crossing-kind byte; old map_viewer/province_adjacencies.rs distinguishes it from the following flag.
pub const CROSSING_KIND: usize = 0x10;
/// Province-definition adjacency flag byte; old map_viewer/province_adjacencies.rs preserves +0x11 separately from crossing kind.
pub const FLAG: usize = 0x11;
