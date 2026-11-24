pub fn idle(path_count: u32, combat_count: u32) -> bool {
    (path_count as i32) <= 0 && (combat_count as i32) <= 0
}

pub fn mission_active(is_in_regions: u8, mission: u32, bombardment_region: u32) -> bool {
    if is_in_regions == 0 {
        return false;
    }
    match mission {
        0 | 2 | 8 => false,
        9 => bombardment_region != 0,
        1..=7 => true,
        _ => false,
    }
}

pub fn leader_directory_path(tag: &[u8], picture: &[u8]) -> Option<Vec<u8>> {
    if tag.is_empty() || picture.is_empty() {
        return None;
    }
    let mut path = LEADER_DIRECTORY.as_bytes().to_vec();
    path.extend_from_slice(tag);
    path.push(b'/');
    path.extend_from_slice(picture);
    Some(path)
}

pub const PATROL: u32 = 1;
pub const LEADER_DIRECTORY: &str = "gfx/leaders/";
pub const STANDSTILL: u32 = 0;
pub const ATTACKING: u32 = 9;
pub const DEFENDING: u32 = 10;
pub const SHIP_IN_PORT: u32 = 19;
pub const PAIRED_ONLY_GROUP: u32 = u32::MAX;

