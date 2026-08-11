//! Exact-a729 map-mode registry and viewer-owned selection law.
//!
//! hard-coded registry. `CMapmodesInterface` constructs the visible button
//! strip in a different order: button 2 selects strategic navy (engine 2)
//! while button 3 selects strategic air (engine 1). Keeping those identities
//! separate prevents a UI ordinal from being mistaken for an engine enum.

use crate::Result;

pub const XMAP18_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const XMAP18_REGISTRY_GHIDRA_MANIFEST_SHA256: &str =
    "293068ab17fe5e223a4747bce0eb249cd29f08a539ea4fe96ce64dc03f66f8db";
pub const FIRST_SCRIPTED_MAP_MODE: u32 = 0x28;
pub const POLITICAL_MAP_MODE_ENGINE_ALIAS: u32 = 0x1c;
pub const STRATEGIC_AIR_MAP_MODE_ENGINE_ALIAS: u32 = 0x20;
pub const RAID_MAP_MODE_ENGINE_ALIAS: u32 = 0x26;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HardcodedMapMode {
    Political,
    StrategicAir,
    StrategicNavy,
    Operatives,
    States,
    Supply,
    Resistance,
    Compliance,
    Resources,
    Diplomacy,
    Factions,
    Players,
    Infrastructure,
    Manpower,
    Ideology,
    Terrain,
    Raids,
}

impl HardcodedMapMode {
    pub const ALL: [Self; 17] = [
        Self::Political,
        Self::StrategicAir,
        Self::StrategicNavy,
        Self::Operatives,
        Self::States,
        Self::Supply,
        Self::Resistance,
        Self::Compliance,
        Self::Resources,
        Self::Diplomacy,
        Self::Factions,
        Self::Players,
        Self::Infrastructure,
        Self::Manpower,
        Self::Ideology,
        Self::Terrain,
        Self::Raids,
    ];

    pub const fn engine_code(self) -> u32 {
        match self {
            Self::Political => 0,
            Self::StrategicAir => 1,
            Self::StrategicNavy => 2,
            Self::Operatives => 3,
            Self::States => 4,
            Self::Supply => 5,
            Self::Resistance => 6,
            Self::Compliance => 7,
            Self::Resources => 8,
            Self::Diplomacy => 9,
            Self::Factions => 10,
            Self::Players => 11,
            Self::Infrastructure => 12,
            Self::Manpower => 13,
            Self::Ideology => 14,
            Self::Terrain => 15,
            Self::Raids => 16,
        }
    }

    pub const fn token(self) -> &'static str {
        match self {
            Self::Political => "political",
            Self::StrategicAir => "strategic_air",
            Self::StrategicNavy => "strategic_navy",
            Self::Operatives => "operatives",
            Self::States => "states",
            Self::Supply => "supply",
            Self::Resistance => "resistance",
            Self::Compliance => "compliance",
            Self::Resources => "resources",
            Self::Diplomacy => "diplomacy",
            Self::Factions => "factions",
            Self::Players => "players",
            Self::Infrastructure => "infrastructure",
            Self::Manpower => "manpower",
            Self::Ideology => "ideology",
            Self::Terrain => "terrain",
            Self::Raids => "raids",
        }
    }

    pub const fn localisation_key(self) -> &'static str {
        match self {
            Self::Political => "MAPMODE_DEFAULT",
            Self::StrategicAir => "MAPMODE_STRATEGIC_AIR",
            Self::StrategicNavy => "MAPMODE_STRATEGIC_NAVY",
            Self::Operatives => "MAPMODE_OPERATIVES",
            Self::States => "MAPMODE_STATES",
            Self::Supply => "MAPMODE_SUPPLY_MAP_MODE",
            Self::Resistance => "MAPMODE_RESISTANCE",
            Self::Compliance => "MAPMODE_COMPLIANCE",
            Self::Resources => "MAPMODE_RESOURCES",
            Self::Diplomacy => "MAPMODE_DIPLOMACY",
            Self::Factions => "MAPMODE_FACTIONS",
            Self::Players => "MAPMODE_PLAYERS",
            Self::Infrastructure => "MAPMODE_INFRASTRUCTURE",
            Self::Manpower => "MAPMODE_MANPOWER",
            Self::Ideology => "MAPMODE_IDEOLOGY",
            Self::Terrain => "MAPMODE_TERRAIN",
            Self::Raids => "MAPMODE_RAIDS",
        }
    }

    pub const fn visible_button(self) -> Option<u8> {
        match self {
            Self::Political => Some(1),
            Self::StrategicNavy => Some(2),
            Self::StrategicAir => Some(3),
            Self::Operatives => Some(4),
            Self::Supply => Some(5),
            Self::Terrain => Some(6),
            Self::Resistance => Some(7),
            Self::Compliance => Some(8),
            Self::Resources => Some(9),
            Self::Infrastructure => Some(10),
            Self::Factions => Some(11),
            Self::States
            | Self::Diplomacy
            | Self::Players
            | Self::Manpower
            | Self::Ideology
            | Self::Raids => None,
        }
    }

    pub fn from_engine_code(code: u32) -> Option<Self> {
        match code {
            POLITICAL_MAP_MODE_ENGINE_ALIAS => return Some(Self::Political),
            STRATEGIC_AIR_MAP_MODE_ENGINE_ALIAS => return Some(Self::StrategicAir),
            RAID_MAP_MODE_ENGINE_ALIAS => return Some(Self::Raids),
            _ => {}
        }
        Self::ALL
            .into_iter()
            .find(|mode| mode.engine_code() == code)
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.token().eq_ignore_ascii_case(token))
    }

    pub fn from_visible_button(button: u8) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.visible_button() == Some(button))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerMapModeSelection {
    Hardcoded(HardcodedMapMode),
    Scripted(String),
}

impl Default for ViewerMapModeSelection {
    fn default() -> Self {
        Self::Hardcoded(HardcodedMapMode::Political)
    }
}

impl ViewerMapModeSelection {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.is_empty() {
            return Err("viewer map-mode selection is blank".into());
        }
        let token = value
            .strip_prefix("viewer.map_mode.")
            .unwrap_or(value)
            .trim();
        if let Some(mode) = HardcodedMapMode::from_token(token) {
            return Ok(Self::Hardcoded(mode));
        }
        let scripted = token
            .strip_prefix("scripted:")
            .ok_or_else(|| format!("unknown viewer map-mode token {value}"))?
            .trim();
        validate_scripted_id(scripted)?;
        Ok(Self::Scripted(scripted.to_string()))
    }

    pub fn viewer_token(&self) -> String {
        match self {
            Self::Hardcoded(mode) => format!("viewer.map_mode.{}", mode.token()),
            Self::Scripted(id) => format!("viewer.map_mode.scripted:{id}"),
        }
    }

    pub fn engine_code(&self, scripted_index: Option<u32>) -> Result<u32> {
        match self {
            Self::Hardcoded(mode) => Ok(mode.engine_code()),
            Self::Scripted(_) => FIRST_SCRIPTED_MAP_MODE
                .checked_add(
                    scripted_index
                        .ok_or("scripted map-mode selection has no effective-VFS database index")?,
                )
                .ok_or_else(|| "scripted map-mode engine code overflow".into()),
        }
    }

    pub const fn visible_button(&self) -> Option<u8> {
        match self {
            Self::Hardcoded(mode) => mode.visible_button(),
            Self::Scripted(_) => None,
        }
    }
}

/// Cycle the complete effective viewer-owned database. Function-key buttons
/// cover only the engine's visible strip; this route deliberately includes the
/// hidden hard-coded modes and every effective scripted definition.
pub fn cycle_viewer_map_mode(
    current: &ViewerMapModeSelection,
    effective_scripted_ids: &[String],
    forward: bool,
) -> Result<ViewerMapModeSelection> {
    let mut ordered = HardcodedMapMode::ALL
        .into_iter()
        .map(ViewerMapModeSelection::Hardcoded)
        .collect::<Vec<_>>();
    let mut scripted_seen = std::collections::BTreeSet::new();
    for id in effective_scripted_ids {
        validate_scripted_id(id)?;
        if !scripted_seen.insert(id.as_str()) {
            return Err("effective scripted map-mode database contains a duplicate id".into());
        }
        ordered.push(ViewerMapModeSelection::Scripted(id.clone()));
    }
    let current_index = ordered
        .iter()
        .position(|candidate| candidate == current)
        .ok_or("current viewer map mode is absent from the effective database")?;
    let next_index = if forward {
        (current_index + 1) % ordered.len()
    } else if current_index == 0 {
        ordered.len() - 1
    } else {
        current_index - 1
    };
    Ok(ordered[next_index].clone())
}

fn validate_scripted_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("scripted map-mode id is blank or contains a non-portable character".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_engine_registry_is_dense_through_raids() {
        for (expected, mode) in HardcodedMapMode::ALL.into_iter().enumerate() {
            assert_eq!(mode.engine_code(), expected as u32);
            assert_eq!(
                HardcodedMapMode::from_engine_code(expected as u32),
                Some(mode)
            );
        }
        assert_eq!(HardcodedMapMode::from_engine_code(17), None);
        assert_eq!(
            HardcodedMapMode::from_engine_code(RAID_MAP_MODE_ENGINE_ALIAS),
            Some(HardcodedMapMode::Raids)
        );
        assert_eq!(
            HardcodedMapMode::from_engine_code(POLITICAL_MAP_MODE_ENGINE_ALIAS),
            Some(HardcodedMapMode::Political)
        );
        assert_eq!(
            HardcodedMapMode::from_engine_code(STRATEGIC_AIR_MAP_MODE_ENGINE_ALIAS),
            Some(HardcodedMapMode::StrategicAir)
        );
        assert_eq!(
            HardcodedMapMode::from_engine_code(FIRST_SCRIPTED_MAP_MODE),
            None
        );
    }

    #[test]
    fn visible_navy_and_air_order_is_not_engine_enum_order() {
        assert_eq!(
            HardcodedMapMode::from_visible_button(2),
            Some(HardcodedMapMode::StrategicNavy)
        );
        assert_eq!(
            HardcodedMapMode::from_visible_button(3),
            Some(HardcodedMapMode::StrategicAir)
        );
        assert_eq!(HardcodedMapMode::StrategicNavy.engine_code(), 2);
        assert_eq!(HardcodedMapMode::StrategicAir.engine_code(), 1);
        assert_eq!(HardcodedMapMode::Raids.visible_button(), None);
        assert_eq!(HardcodedMapMode::from_visible_button(12), None);
    }

    #[test]
    fn viewer_tokens_are_canonical_and_scripted_modes_fail_closed() {
        let selection =
            ViewerMapModeSelection::parse("viewer.map_mode.scripted:mod_weather").unwrap();
        assert_eq!(
            selection.viewer_token(),
            "viewer.map_mode.scripted:mod_weather"
        );
        assert_eq!(selection.engine_code(Some(3)).unwrap(), 0x2b);
        assert!(selection.engine_code(None).is_err());
        assert!(ViewerMapModeSelection::parse("scripted:../bad").is_err());
    }

    #[test]
    fn complete_cycle_reaches_hidden_and_scripted_modes_in_both_directions() {
        let scripts = vec!["mod_alpha".to_string(), "mod_beta".to_string()];
        assert_eq!(
            cycle_viewer_map_mode(
                &ViewerMapModeSelection::Hardcoded(HardcodedMapMode::Operatives),
                &scripts,
                true,
            )
            .unwrap(),
            ViewerMapModeSelection::Hardcoded(HardcodedMapMode::States)
        );
        assert_eq!(
            cycle_viewer_map_mode(
                &ViewerMapModeSelection::Hardcoded(HardcodedMapMode::Raids),
                &scripts,
                true,
            )
            .unwrap(),
            ViewerMapModeSelection::Scripted("mod_alpha".into())
        );
        assert_eq!(
            cycle_viewer_map_mode(
                &ViewerMapModeSelection::Hardcoded(HardcodedMapMode::Political),
                &scripts,
                false,
            )
            .unwrap(),
            ViewerMapModeSelection::Scripted("mod_beta".into())
        );
        assert!(cycle_viewer_map_mode(
            &ViewerMapModeSelection::Scripted("stale".into()),
            &scripts,
            true,
        )
        .is_err());
    }
}
