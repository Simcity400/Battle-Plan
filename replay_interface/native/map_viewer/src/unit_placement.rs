
use crate::*;
use std::collections::BTreeMap;

pub const UNIT_PLACEMENT_MAP_PLANE_X: f32 = MAP_PLANE_X;
pub const UNIT_PLACEMENT_MAP_PLANE_Z: f32 = MAP_PLANE_Z;
pub const UNIT_STACK_BASE_SLOT_KIND: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldAnchor {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl WorldAnchor {
    pub fn in_map_bounds(&self) -> bool {
        self.x >= 0.0
            && self.x <= UNIT_PLACEMENT_MAP_PLANE_X
            && self.z >= 0.0
            && self.z <= UNIT_PLACEMENT_MAP_PLANE_Z
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorProvenance {
    UnitStackBaseSlot,
    UnitStackLowestSlot { kind_raw: u32 },
    ProvinceCentroid,
    Synthetic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProvinceAnchor {
    pub anchor: WorldAnchor,
    pub provenance: AnchorProvenance,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProvinceAnchorTable {
    by_province: BTreeMap<u32, ProvinceAnchor>,
}

impl ProvinceAnchorTable {
    pub fn new() -> ProvinceAnchorTable {
        ProvinceAnchorTable::default()
    }

    pub fn from_unit_stacks(rows: &[UnitStackPlacement]) -> ProvinceAnchorTable {
        let mut best: BTreeMap<u32, (u32, WorldAnchor)> = BTreeMap::new();
        for row in rows {
            let anchor = WorldAnchor {
                x: row.pos[0],
                y: row.pos[1],
                z: row.pos[2],
            };
            best.entry(row.province_id)
                .and_modify(|(kind, existing)| {
                    if better_slot(row.kind_raw, *kind) {
                        *kind = row.kind_raw;
                        *existing = anchor;
                    }
                })
                .or_insert((row.kind_raw, anchor));
        }
        let by_province = best
            .into_iter()
            .map(|(province_id, (kind, anchor))| {
                let provenance = if kind == UNIT_STACK_BASE_SLOT_KIND {
                    AnchorProvenance::UnitStackBaseSlot
                } else {
                    AnchorProvenance::UnitStackLowestSlot { kind_raw: kind }
                };
                (province_id, ProvinceAnchor { anchor, provenance })
            })
            .collect();
        ProvinceAnchorTable { by_province }
    }

    pub fn from_resolver(resolver: &GameFileResolver) -> Result<ProvinceAnchorTable> {
        let rows = unit_stack_placements_from_resolver(resolver)?;
        Ok(ProvinceAnchorTable::from_unit_stacks(&rows))
    }

    pub fn insert(&mut self, province_id: u32, anchor: ProvinceAnchor) {
        self.by_province.insert(province_id, anchor);
    }

    pub fn insert_synthetic(&mut self, province_id: u32, x: f32, y: f32, z: f32) {
        self.insert(
            province_id,
            ProvinceAnchor {
                anchor: WorldAnchor { x, y, z },
                provenance: AnchorProvenance::Synthetic,
            },
        );
    }

    pub fn anchor(&self, province_id: u32) -> Option<&ProvinceAnchor> {
        self.by_province.get(&province_id)
    }

    pub fn len(&self) -> usize {
        self.by_province.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_province.is_empty()
    }
}

fn better_slot(cand: u32, cur: u32) -> bool {
    if cur == UNIT_STACK_BASE_SLOT_KIND {
        false
    } else if cand == UNIT_STACK_BASE_SLOT_KIND {
        true
    } else {
        cand < cur
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Division,
    Fleet,
    Wing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorProvinceSource {
    LocationProvince,
    AirBaseStateRepresentative,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementStatus {
    Placed,
    AnchorProvinceUnavailable,
    AnchorProvinceUnresolved,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitPlacement {
    pub unit_uid: String,
    pub kind: UnitKind,
    pub owner: Option<String>,
    pub anchor_province: Option<u32>,
    pub anchor_province_source: AnchorProvinceSource,
    pub world_anchor: Option<WorldAnchor>,
    pub anchor_provenance: Option<AnchorProvenance>,
    pub status: PlacementStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DivisionPlacementInput {
    pub uid: String,
    pub owner: Option<String>,
    pub location_province: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FleetPlacementInput {
    /// The task force's number as the GAME shows it -- the marker's label.
    ///
    /// **NOT UNIQUE, and nothing here may assume it is.** Two clausewitz
    /// registries mint task forces and their numbers collide, so a frame can
    /// place two markers carrying this same text; they are two different task
    /// forces in two different provinces. Placement keeps a positional list for
    /// exactly that reason -- do not turn it into a map keyed on this field. A
    /// caller that needs the identity has `FleetUnitRow::task_force_key()`.
    pub uid: String,
    pub owner: Option<String>,
    pub location_province: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WingPlacementInput {
    pub uid: String,
    pub owner: Option<String>,
    pub state_id: Option<u32>,
    pub location_province: Option<u32>,
}

pub fn wing_placement_input_from_world_row(row: &replay_core::WingUnitRow) -> WingPlacementInput {
    WingPlacementInput {
        uid: row.wing_uid.clone(),
        owner: row.owner.measured().map(|owner| owner.tag().to_string()),
        state_id: row.state_id.measured().copied(),
        location_province: row.air_base_province.measured().copied(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitPlacementSet {
    pub hour_index: i64,
    pub divisions: Vec<UnitPlacement>,
    pub fleets: Vec<UnitPlacement>,
    pub wings: Vec<UnitPlacement>,
}

fn resolve(
    unit_uid: String,
    kind: UnitKind,
    owner: Option<String>,
    province: Option<u32>,
    province_source: AnchorProvinceSource,
    table: &ProvinceAnchorTable,
) -> UnitPlacement {
    match province {
        None => UnitPlacement {
            unit_uid,
            kind,
            owner,
            anchor_province: None,
            anchor_province_source: AnchorProvinceSource::None,
            world_anchor: None,
            anchor_provenance: None,
            status: PlacementStatus::AnchorProvinceUnavailable,
        },
        Some(pid) => match table.anchor(pid) {
            Some(pa) => UnitPlacement {
                unit_uid,
                kind,
                owner,
                anchor_province: Some(pid),
                anchor_province_source: province_source,
                world_anchor: Some(pa.anchor),
                anchor_provenance: Some(pa.provenance),
                status: PlacementStatus::Placed,
            },
            None => UnitPlacement {
                unit_uid,
                kind,
                owner,
                anchor_province: Some(pid),
                anchor_province_source: province_source,
                world_anchor: None,
                anchor_provenance: None,
                status: PlacementStatus::AnchorProvinceUnresolved,
            },
        },
    }
}

pub fn place_division(
    input: &DivisionPlacementInput,
    table: &ProvinceAnchorTable,
) -> UnitPlacement {
    resolve(
        input.uid.clone(),
        UnitKind::Division,
        input.owner.clone(),
        input.location_province,
        AnchorProvinceSource::LocationProvince,
        table,
    )
}

pub fn place_fleet(input: &FleetPlacementInput, table: &ProvinceAnchorTable) -> UnitPlacement {
    resolve(
        input.uid.clone(),
        UnitKind::Fleet,
        input.owner.clone(),
        input.location_province,
        AnchorProvinceSource::LocationProvince,
        table,
    )
}

pub fn place_wing(
    input: &WingPlacementInput,
    table: &ProvinceAnchorTable,
    state_anchor_provinces: &BTreeMap<u32, u32>,
) -> UnitPlacement {
    let (province, source) = match input.location_province {
        Some(pid) => (Some(pid), AnchorProvinceSource::LocationProvince),
        None => match input
            .state_id
            .and_then(|s| state_anchor_provinces.get(&s).copied())
        {
            Some(pid) => (Some(pid), AnchorProvinceSource::AirBaseStateRepresentative),
            None => (None, AnchorProvinceSource::None),
        },
    };
    resolve(
        input.uid.clone(),
        UnitKind::Wing,
        input.owner.clone(),
        province,
        source,
        table,
    )
}

pub fn place_divisions(
    inputs: &[DivisionPlacementInput],
    table: &ProvinceAnchorTable,
) -> Vec<UnitPlacement> {
    inputs.iter().map(|d| place_division(d, table)).collect()
}

pub fn place_fleets(
    inputs: &[FleetPlacementInput],
    table: &ProvinceAnchorTable,
) -> Vec<UnitPlacement> {
    inputs.iter().map(|f| place_fleet(f, table)).collect()
}

pub fn place_wings(
    inputs: &[WingPlacementInput],
    table: &ProvinceAnchorTable,
    state_anchor_provinces: &BTreeMap<u32, u32>,
) -> Vec<UnitPlacement> {
    inputs
        .iter()
        .map(|w| place_wing(w, table, state_anchor_provinces))
        .collect()
}

pub fn place_frame_inputs(
    hour_index: i64,
    divisions: &[DivisionPlacementInput],
    fleets: &[FleetPlacementInput],
    wings: &[WingPlacementInput],
    table: &ProvinceAnchorTable,
    state_anchor_provinces: &BTreeMap<u32, u32>,
) -> UnitPlacementSet {
    UnitPlacementSet {
        hour_index,
        divisions: place_divisions(divisions, table),
        fleets: place_fleets(fleets, table),
        wings: place_wings(wings, table, state_anchor_provinces),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_with(provinces: &[(u32, f32, f32, f32)]) -> ProvinceAnchorTable {
        let mut t = ProvinceAnchorTable::new();
        for (pid, x, y, z) in provinces {
            t.insert_synthetic(*pid, *x, *y, *z);
        }
        t
    }

    #[test]
    fn unitstacks_prefers_stack_centre_slot() {
        let rows = vec![
            UnitStackPlacement {
                province_id: 7,
                kind_raw: 3,
                pos: [100.0, 9.0, 200.0],
                rotation: 1.0,
                offset: 0.3,
            },
            UnitStackPlacement {
                province_id: 7,
                kind_raw: 0,
                pos: [110.0, 9.5, 210.0],
                rotation: 0.0,
                offset: 0.3,
            },
            UnitStackPlacement {
                province_id: 8,
                kind_raw: 9,
                pos: [300.0, 9.0, 400.0],
                rotation: 0.0,
                offset: 0.3,
            },
            UnitStackPlacement {
                province_id: 8,
                kind_raw: 5,
                pos: [305.0, 9.0, 405.0],
                rotation: 0.0,
                offset: 0.3,
            },
        ];
        let table = ProvinceAnchorTable::from_unit_stacks(&rows);
        let a7 = table.anchor(7).expect("province 7");
        assert_eq!(a7.provenance, AnchorProvenance::UnitStackBaseSlot);
        assert_eq!(
            a7.anchor,
            WorldAnchor {
                x: 110.0,
                y: 9.5,
                z: 210.0
            }
        );
        let a8 = table.anchor(8).expect("province 8");
        assert_eq!(
            a8.provenance,
            AnchorProvenance::UnitStackLowestSlot { kind_raw: 5 }
        );
        assert_eq!(
            a8.anchor,
            WorldAnchor {
                x: 305.0,
                y: 9.0,
                z: 405.0
            }
        );
    }

    #[test]
    fn division_places_at_location_province() {
        let table = table_with(&[(1000, 842.0, 9.5, 73.0)]);
        let d = DivisionPlacementInput {
            uid: "101:1".into(),
            owner: Some("GER".into()),
            location_province: Some(1000),
        };
        let p = place_division(&d, &table);
        assert_eq!(p.status, PlacementStatus::Placed);
        assert_eq!(p.kind, UnitKind::Division);
        assert_eq!(p.anchor_province, Some(1000));
        assert_eq!(
            p.anchor_province_source,
            AnchorProvinceSource::LocationProvince
        );
        assert_eq!(
            p.world_anchor,
            Some(WorldAnchor {
                x: 842.0,
                y: 9.5,
                z: 73.0
            })
        );
        assert_eq!(p.anchor_provenance, Some(AnchorProvenance::Synthetic));
        assert!(p.world_anchor.unwrap().in_map_bounds());
    }

    #[test]
    fn division_movement_path_follows_provinces() {
        let table = table_with(&[
            (1000, 842.0, 9.5, 73.0),
            (1001, 4651.0, 9.7, 1196.0),
            (2000, 3753.0, 13.5, 1249.0),
        ]);
        for (loc, want) in [
            (
                1000u32,
                WorldAnchor {
                    x: 842.0,
                    y: 9.5,
                    z: 73.0,
                },
            ),
            (
                1001,
                WorldAnchor {
                    x: 4651.0,
                    y: 9.7,
                    z: 1196.0,
                },
            ),
            (
                2000,
                WorldAnchor {
                    x: 3753.0,
                    y: 13.5,
                    z: 1249.0,
                },
            ),
        ] {
            let d = DivisionPlacementInput {
                uid: "101:1".into(),
                owner: Some("GER".into()),
                location_province: Some(loc),
            };
            let p = place_division(&d, &table);
            assert_eq!(p.anchor_province, Some(loc));
            assert_eq!(p.world_anchor, Some(want));
        }
    }

    #[test]
    fn missing_province_is_unavailable_not_invented() {
        let table = table_with(&[(1000, 842.0, 9.5, 73.0)]);
        let d = DivisionPlacementInput {
            uid: "101:1".into(),
            owner: Some("GER".into()),
            location_province: None,
        };
        let p = place_division(&d, &table);
        assert_eq!(p.status, PlacementStatus::AnchorProvinceUnavailable);
        assert_eq!(p.world_anchor, None);
        assert_eq!(p.anchor_provenance, None);
        assert_eq!(p.anchor_province_source, AnchorProvinceSource::None);
    }

    #[test]
    fn known_province_absent_from_table_is_unresolved() {
        let table = table_with(&[(1000, 842.0, 9.5, 73.0)]);
        let d = DivisionPlacementInput {
            uid: "101:1".into(),
            owner: Some("GER".into()),
            location_province: Some(9999),
        };
        let p = place_division(&d, &table);
        assert_eq!(p.status, PlacementStatus::AnchorProvinceUnresolved);
        assert_eq!(p.anchor_province, Some(9999));
        assert_eq!(p.world_anchor, None);
    }

    #[test]
    fn fleet_places_at_sea_province() {
        let table = table_with(&[(3000, 3612.0, 10.0, 1751.0)]);
        let f = FleetPlacementInput {
            uid: "501:1".into(),
            owner: Some("ENG".into()),
            location_province: Some(3000),
        };
        let p = place_fleet(&f, &table);
        assert_eq!(p.status, PlacementStatus::Placed);
        assert_eq!(p.kind, UnitKind::Fleet);
        assert_eq!(
            p.world_anchor,
            Some(WorldAnchor {
                x: 3612.0,
                y: 10.0,
                z: 1751.0
            })
        );
        assert!(p.world_anchor.unwrap().in_map_bounds());
    }

    #[test]
    fn wing_places_via_air_base_state_fit() {
        let table = table_with(&[(2500, 1200.0, 9.0, 800.0)]);
        let mut state_anchor = BTreeMap::new();
        state_anchor.insert(30u32, 2500u32);
        let w = WingPlacementInput {
            uid: "800:1".into(),
            owner: Some("ENG".into()),
            state_id: Some(30),
            location_province: None,
        };
        let p = place_wing(&w, &table, &state_anchor);
        assert_eq!(p.status, PlacementStatus::Placed);
        assert_eq!(p.kind, UnitKind::Wing);
        assert_eq!(p.anchor_province, Some(2500));
        assert_eq!(
            p.anchor_province_source,
            AnchorProvinceSource::AirBaseStateRepresentative
        );
        assert!(p.world_anchor.unwrap().in_map_bounds());
    }

    #[test]
    fn wing_prefers_recording_location_province_over_state_fit() {
        let table = table_with(&[(2500, 1200.0, 9.0, 800.0), (3000, 3612.0, 10.0, 1751.0)]);
        let mut state_anchor = BTreeMap::new();
        state_anchor.insert(30u32, 2500u32);
        let w = WingPlacementInput {
            uid: "800:1".into(),
            owner: Some("ENG".into()),
            state_id: Some(30),
            location_province: Some(3000),
        };
        let p = place_wing(&w, &table, &state_anchor);
        assert_eq!(p.anchor_province, Some(3000));
        assert_eq!(
            p.anchor_province_source,
            AnchorProvinceSource::LocationProvince
        );
    }

    #[test]
    fn world_wing_adapter_carries_exact_air_base_province() {
        use replay_core::{Leaf, WingUnitRow};

        let row = WingUnitRow {
            wing_uid: "800:69".into(),
            raw_wing_id: Leaf::Measured(800),
            // ABSENT, and it has to be: nothing outside `replay_core` can
            // build a country identity, which is the closure working. This
            // test is about the air base anchor and never reads the owner.
            owner: Leaf::Absent,
            name: Leaf::Absent,
            plane_count: Leaf::Measured(100),
            experience: Leaf::Absent,
            manpower: Leaf::Absent,
            mission_kind: Leaf::Absent,
            mission_mask: Leaf::Unobtained,
            strategic_region_id: Leaf::Measured(68),
            air_base: Leaf::Measured("39:65".into()),
            air_base_province: Leaf::Measured(6508),
            state_id: Leaf::Measured(193),
            air_base_anchor: Leaf::Measured(replay_core::AirBaseAnchor::State),
            plane_definition: Leaf::Absent,
            effective_planes: Leaf::Absent,
            equipment: Leaf::Absent,
            exact_equipment: Leaf::Absent,
        };
        let input = wing_placement_input_from_world_row(&row);
        assert_eq!(input.location_province, Some(6508));
        assert_eq!(input.state_id, Some(193));
    }

    #[test]
    fn wing_without_state_or_province_is_unavailable() {
        let table = table_with(&[(2500, 1200.0, 9.0, 800.0)]);
        let w = WingPlacementInput {
            uid: "800:1".into(),
            owner: Some("ENG".into()),
            state_id: Some(99),
            location_province: None,
        };
        let p = place_wing(&w, &table, &BTreeMap::new());
        assert_eq!(p.status, PlacementStatus::AnchorProvinceUnavailable);
        let _ = table;
    }

    #[test]
    fn out_of_bounds_anchor_is_flagged() {
        let anchor = WorldAnchor {
            x: 6000.0,
            y: 9.0,
            z: 100.0,
        };
        assert!(!anchor.in_map_bounds());
        let inb = WorldAnchor {
            x: 5632.0,
            y: 9.0,
            z: 2048.0,
        };
        assert!(inb.in_map_bounds());
    }
}
