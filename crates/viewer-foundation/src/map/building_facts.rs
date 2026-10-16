//! Authored building reads shared by world admission and counter/province hovers.
use crate::table_access::TokenKeys;
use installed_content::{
    buildings::{BuildingType, SpawnPoint},
    map::{MapData, ProvinceKind},
    mesh::Roster,
    positions::BuildingPosition,
    states::StateDefinition,
};
use std::collections::BTreeMap;

pub struct BuildingFacts {
    pub ordered: Vec<BuildingType>,
    pub province_sites: BTreeMap<(u32, u32), [bool; 3]>,
    pub keys: TokenKeys,
    pub index: BTreeMap<String, usize>,
}
impl BuildingFacts {
    /// The game ORs specialization flags of every candidate in an admitted slot.
    pub fn province_sites(&self, state: u32, province: u32) -> [bool; 3] {
        self.province_sites
            .get(&(state, province))
            .copied()
            .unwrap_or_default()
    }

    /// The game checks the template's specialization count.
    pub fn is_facility(&self, name: &str) -> Result<bool, String> {
        let key = self
            .keys
            .resolve(name)?
            .ok_or_else(|| format!("building {name} has no content template"))?;
        self.index
            .get(key)
            .map(|&index| &self.ordered[index])
            .map(|b| !b.specialization.is_empty())
            .ok_or_else(|| format!("building {name} has no content template"))
    }
}

/// How the ordinary arm of the game treats one slot whose
/// name reached `spawn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotAdmission {
    /// The slot becomes a record of its bitmap province.
    Admitted,
    /// The bucket registers no entity (`0x28C`): the slot is ignored.
    Unregistered,
    /// The bitmap province is no land province (`0x2A7`), or the declared state does not
    /// exist (`0x2AE`) or does not hold it (`0x2CC`): the slot is ignored.
    Outside,
}

/// The ordinary arm's admission of one slot, in the loader's order: the bucket's entity
/// registration, then [`admitted_position`]. `province` is the province under the slot's own position ([`ProvinceMap::at`]). Both
/// the building layer and the naval-HQ and facility anchors (both
/// search only admitted records) use it.
pub fn admit_slot(
    spawn: &SpawnPoint,
    position: &BuildingPosition,
    province: Option<u32>,
    map: &MapData,
    states: &BTreeMap<u32, StateDefinition>,
    roster: &Roster,
) -> SlotAdmission {
    if !bucket_registered(spawn, roster) {
        SlotAdmission::Unregistered
    } else if !admitted_position(position, province, map, states) {
        SlotAdmission::Outside
    } else {
        SlotAdmission::Admitted
    }
}

/// The ordinary arm of the game admits a slot only when its bitmap
/// province is land and belongs to the declared state.
fn admitted_position(
    position: &BuildingPosition,
    province: Option<u32>,
    map: &MapData,
    states: &BTreeMap<u32, StateDefinition>,
) -> bool {
    province.is_some_and(|province| {
        map.definitions
            .get(&province)
            .is_some_and(|definition| definition.kind == ProvinceKind::Land)
            && states
                .get(&position.state)
                .is_some_and(|state| state.provinces.contains(&province))
    })
}

/// Whether the game registers a spawn bucket: its signed max and mesh
/// count must be positive and an entity `building_<bucket>` (one mesh) or
/// `building_<bucket>_<k>` (`k` in `1..=meshes`) must be registered.
fn bucket_registered(spawn: &SpawnPoint, roster: &Roster) -> bool {
    if spawn.max <= 0 || spawn.meshes <= 0 {
        return false;
    }
    let base = format!("{}{}", installed_content::buildings::ENTITY_PREFIX, spawn.name);
    let names: Vec<String> = if spawn.meshes == 1 {
        vec![base]
    } else {
        (1..=spawn.meshes).map(|k| format!("{base}_{k}")).collect()
    };
    names
        .iter()
        .any(|name| roster.descriptors.entities.contains_key(name))
}

pub const FLOATING_HARBOR_SLOT: &str = "floating_harbor";
/// Whether the game routes a slot to the floating-harbour arm rather than to its
/// named bucket.
pub fn is_floating_harbor_slot(kind: &str) -> bool {
    kind == FLOATING_HARBOR_SLOT
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_bucket_needs_a_registered_entity_of_its_own_name() {
        let mut roster = Roster::default();
        for name in ["building_one", "building_many_2"] {
            roster
                .descriptors
                .entities
                .insert(name.into(), Default::default());
        }
        let spawn = |name: &str, max, meshes| SpawnPoint {
            name: name.into(),
            max,
            meshes,
            provincial: false,
            is_port: false,
        };
        assert!(bucket_registered(&spawn("one", 1, 1), &roster));
        assert!(!bucket_registered(&spawn("one", 0, 1), &roster));
        assert!(bucket_registered(&spawn("many", 1, 3), &roster));
        assert!(!bucket_registered(&spawn("many", 1, 1), &roster));
        assert!(!bucket_registered(&spawn("absent", 1, 1), &roster));
    }

    #[test]
    fn ordinary_slots_require_land_in_their_declared_state() {
        use installed_content::map::{CountryColors, ProvinceDefinition};
        let definition = |id, kind| ProvinceDefinition {
            id,
            rgb: [0, 0, 0],
            kind,
            continent: 0,
        };
        let map = MapData {
            width: 2,
            height: 1,
            province_ids: vec![7, 8],
            definitions: BTreeMap::from([
                (7, definition(7, ProvinceKind::Land)),
                (8, definition(8, ProvinceKind::Sea)),
            ]),
            country_colors: CountryColors::default(),
        };
        let state = |id, provinces| StateDefinition {
            id,
            provinces,
            ..Default::default()
        };
        let states = BTreeMap::from([(3, state(3, vec![7])), (4, state(4, vec![8]))]);
        let mut position = BuildingPosition {
            state: 3,
            kind: "arms_factory".into(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            rotation: 0.0,
            adjacent_sea: 0,
        };
        assert!(admitted_position(&position, Some(7), &map, &states));
        assert!(!admitted_position(&position, None, &map, &states));
        assert!(!admitted_position(&position, Some(8), &map, &states));
        position.state = 4;
        assert!(!admitted_position(&position, Some(7), &map, &states));
    }
}
