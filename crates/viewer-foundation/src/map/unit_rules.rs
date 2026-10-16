//! Shared recorded-unit admission and namespace operands.
/// Signed route and combat counts decide the idle action.
pub fn idle(path_count: u32, combat_count: u32) -> bool {
    (path_count as i32) <= 0 && (combat_count as i32) <= 0
}

/// A task force must be in its mission regions with an active mission kind.
/// Invasion support additionally needs a bombardment region.
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

/// When no explicit portrait path exists, combine [`LEADER_DIRECTORY`], the
/// recorded country tag, a slash and the picture name. An empty tag returns `None`.
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

/// Patrol mission kind, whose activity also depends on the task force's region.
pub const PATROL: u32 = 1;
/// Directory prefix for country-qualified leader pictures.
pub const LEADER_DIRECTORY: &str = "gfx/leaders/";
pub const STANDSTILL: u32 = 0;
pub const ATTACKING: u32 = 9;
pub const DEFENDING: u32 = 10;
pub const SHIP_IN_PORT: u32 = 19;
pub const PAIRED_ONLY_GROUP: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_directory_portrait_is_the_leaders_directory_tag_and_picture() {
        assert_eq!(
            leader_directory_path(b"ENG", b"portrait_europe_generic_land_17.dds").as_deref(),
            Some(&b"gfx/leaders/ENG/portrait_europe_generic_land_17.dds"[..])
        );
        assert_eq!(leader_directory_path(b"", b"x.dds"), None);
        assert_eq!(LEADER_DIRECTORY.len(), 12);
    }
}
