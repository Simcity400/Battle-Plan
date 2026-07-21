//! C2 — division / fleet / wing placement over time (the DATA + PLACEMENT half
//! of unit rendering; the counter VISUAL law is CAP6-gated and NOT in scope).
//!
//! This module turns per-hour unit views (division, fleet, air-wing rows served
//! by `replay_core`'s certified `WorldTimeline`) into **world-space placement
//! records**: each unit gets an anchor province and a world `(x, y, z)` point on
//! the 5632×2048 map plane. It is deliberately decoupled from `replay_core`: it
//! consumes plain input structs ([`DivisionPlacementInput`] etc.), exactly like
//! the other recording-fed `map_viewer` consumers (`naval_supremacy_tint`,
//! `map_furniture`) take primitive inputs and let `replay_app` adapt the typed
//! views. The A2 fixtures are the numeric oracle; the round-trip gate lives in
//! `map_viewer/tests/unit_placement_gates.rs` (opens the fixtures through the
//! real `replay_core` source, so the placement is proven against the generator
//! ground truth).
//!
//! # Placement law
//!
//! | Element | Class | Rule |
//! |---|---|---|
//! | Land division anchor province | FILE (recording) | the division's own `location_province` for the hour. |
//! | Fleet anchor province | FILE (recording) | the task force's own `location_province` (sea / naval-base province). |
//! | Air-wing anchor province | FIT | wings carry no on-map province today; anchored via the air-base **state**'s representative province (caller-supplied `state_id → province_id`). Exact air-base locator is UNKNOWN (see gap below). A future recording `location_province` (item 8 "airbase province") is honored first when present. |
//! | Province → world point | FILE | `map/unitstacks.txt` **stack-center** row (`type == 0`) for the province — the engine's own per-province unit-stack centre `(x, y, z)` in world space. When a province has no `type 0` row, the lowest available slot kind is used (still FILE, provenance records the kind). |
//! | Province → world point (fallback) | FIT | province pixel **centroid** (the `map_furniture` DERIVED_APPROXIMATION) may be injected via [`ProvinceAnchorTable::insert`] with [`AnchorProvenance::ProvinceCentroid`] for provinces missing from `unitstacks.txt`. Not auto-loaded here (raster load is heavy); it is an explicit, labelled extension. |
//! | Which unitstacks **slot** a specific unit occupies; multi-unit **stacking**; screen-space size / anchor offset / aggregation-by-zoom | UNKNOWN / CAP6 | `unitstacks.txt` ships ~20 slot kinds per province (`type 0..=38`) but the semantic enum (land/naval/air, sub-slot order) is NOT shipped; the counter visual law is capture-gated (CAP6). This module places every unit at the province **stack centre**; slotting/stacking slots in later without changing the province→anchor seam. |
//!
//! # Interpolation policy (plan C2)
//!
//! **Step function — hold the last hourly sample until the next hour.** The
//! recording carries `moving` / `move_target_province` / `move_progress`, but the
//! plan forbids invented smoothing without capture evidence, so position is the
//! `location_province` of the frame at-or-before the requested hour. Because
//! `replay_core::WorldTimeline::state_at` already resolves to the nearest
//! snapshot at-or-before, calling [`place_frame_inputs`] once per resolved frame
//! yields the step-hold for free, and `seek(h) == play_to(h)` placement equality
//! is inherited from the timeline (gated in the test). `move_*` fields are left
//! available for a FUTURE evidence-based interpolation; they do NOT move the
//! anchor today.
//!
//! # Fail-closed honesty
//!
//! Nothing is invented. A unit whose recording gives no anchor province is
//! [`PlacementStatus::AnchorProvinceUnavailable`]; a province with no entry in
//! the anchor table is [`PlacementStatus::AnchorProvinceUnresolved`] (no world
//! point). Only a resolved province yields [`PlacementStatus::Placed`] with a
//! world anchor and its [`AnchorProvenance`].
//!
//! # Recording-side gap (filed)
//!
//! Air wings need an on-map anchor. The schema (item 8) mentions an "airbase
//! province" but `WingUnitRow` exposes only `air_base` (an opaque id string),
//! `state_id`, and `strategic_region_id`; none is a clean province. Until the
//! recording carries an air-base province, wing placement is a state-centroid
//! FIT. Filed in `REPLAY_DATA_REQUIREMENTS.md` item 8 / `EXTRACTION_NEEDS.md`
//! A7.

use crate::*;
use std::collections::BTreeMap;

/// Map plane extent (world x), shared with the terrain/weather plane
/// (`weather_law::MAP_PLANE_X`); the province raster and unit-stack anchors live
/// in the same `[0, 5632] × [0, 2048]` world x/z space.
pub const UNIT_PLACEMENT_MAP_PLANE_X: f32 = MAP_PLANE_X;
/// Map plane extent (world z), shared with `weather_law::MAP_PLANE_Z`.
pub const UNIT_PLACEMENT_MAP_PLANE_Z: f32 = MAP_PLANE_Z;
/// `map/unitstacks.txt` slot kind for the per-province stack CENTRE row (the
/// canonical base anchor; rotation 0, the point the other slots cluster around).
pub const UNIT_STACK_BASE_SLOT_KIND: u32 = 0;

/// A point on the world map plane (x right, z down/vertical-map-axis, y = terrain
/// height). Same convention as `map/unitstacks.txt` / `map/buildings.txt`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldAnchor {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl WorldAnchor {
    /// Whether the point falls inside the `[0, plane] × [0, plane]` map bounds
    /// (the gate's "within map bounds" predicate). `y` (height) is unbounded.
    pub fn in_map_bounds(&self) -> bool {
        self.x >= 0.0
            && self.x <= UNIT_PLACEMENT_MAP_PLANE_X
            && self.z >= 0.0
            && self.z <= UNIT_PLACEMENT_MAP_PLANE_Z
    }
}

/// Provenance + fidelity label for a resolved province → world anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorProvenance {
    /// FILE: `map/unitstacks.txt` stack-centre row (`type == 0`) for the province.
    UnitStackBaseSlot,
    /// FILE: `map/unitstacks.txt` lowest-kind slot, used when a province has no
    /// `type 0` row. Still shipped-file-derived; the exact slot is not the
    /// centre, recorded for honesty.
    UnitStackLowestSlot { kind_raw: u32 },
    /// FIT: province pixel centroid (a DERIVED_APPROXIMATION; see module docs).
    ProvinceCentroid,
    /// Test / fixture injected anchor.
    Synthetic,
}

/// A resolved province anchor: the world point plus how it was derived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProvinceAnchor {
    pub anchor: WorldAnchor,
    pub provenance: AnchorProvenance,
}

/// `province_id → world anchor` table. Built FILE-first from `map/unitstacks.txt`
/// (the engine's per-province stack centres); callers may inject FIT centroids or
/// synthetic test anchors with [`ProvinceAnchorTable::insert`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProvinceAnchorTable {
    by_province: BTreeMap<u32, ProvinceAnchor>,
}

impl ProvinceAnchorTable {
    pub fn new() -> ProvinceAnchorTable {
        ProvinceAnchorTable::default()
    }

    /// Build from parsed `map/unitstacks.txt` rows. For each province, the
    /// stack-centre (`type 0`) row wins; failing that, the lowest slot kind.
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

    /// FILE constructor: load `map/unitstacks.txt` through the mod-aware
    /// resolver (mod override wins) and build the stack-centre table.
    pub fn from_resolver(resolver: &GameFileResolver) -> Result<ProvinceAnchorTable> {
        let rows = unit_stack_placements_from_resolver(resolver)?;
        Ok(ProvinceAnchorTable::from_unit_stacks(&rows))
    }

    /// Insert / override a province anchor (FIT centroid injection, tests).
    pub fn insert(&mut self, province_id: u32, anchor: ProvinceAnchor) {
        self.by_province.insert(province_id, anchor);
    }

    /// Convenience: inject a synthetic anchor (test / fixture harness).
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

/// Whether unitstacks slot `cand` is a better per-province base than `cur`
/// (stack centre `type 0` always wins; otherwise the lower kind).
fn better_slot(cand: u32, cur: u32) -> bool {
    if cur == UNIT_STACK_BASE_SLOT_KIND {
        false
    } else if cand == UNIT_STACK_BASE_SLOT_KIND {
        true
    } else {
        cand < cur
    }
}

/// Unit domain of a placement record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Division,
    Fleet,
    Wing,
}

/// How a placement's anchor province was obtained (fidelity of the province
/// choice, separate from the province → world anchor provenance).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorProvinceSource {
    /// FILE / recording: the unit's own `location_province`.
    LocationProvince,
    /// FIT: wing anchored via its air-base state's representative province.
    AirBaseStateRepresentative,
    /// No province could be determined for the unit.
    None,
}

/// Resolution outcome for one unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlacementStatus {
    /// Anchored: `world_anchor` and `anchor_provenance` are set.
    Placed,
    /// The recording gave no anchor province (or a state with no mapping) — the
    /// unit exists but cannot be placed. Fail-closed, never invented.
    AnchorProvinceUnavailable,
    /// A province was known but the anchor table has no world point for it.
    AnchorProvinceUnresolved,
}

/// One placed (or unplaceable) unit at one hour.
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

/// Minimal land-division input (adapt from `replay_core::DivisionUnitRow`:
/// `uid = div_uid`, `owner`/`location_province` from the `Leaf`s).
#[derive(Clone, Debug, PartialEq)]
pub struct DivisionPlacementInput {
    pub uid: String,
    pub owner: Option<String>,
    pub location_province: Option<u32>,
}

/// Minimal task-force / fleet input (adapt from `replay_core::FleetUnitRow`).
#[derive(Clone, Debug, PartialEq)]
pub struct FleetPlacementInput {
    pub uid: String,
    pub owner: Option<String>,
    pub location_province: Option<u32>,
}

/// Minimal air-wing input (adapt from `replay_core::WingUnitRow`). The exact
/// recorded air-base province is honored first. `state_id` is retained only
/// for legacy recordings that predate the A7 D34 placement carrier.
#[derive(Clone, Debug, PartialEq)]
pub struct WingPlacementInput {
    pub uid: String,
    pub owner: Option<String>,
    pub state_id: Option<u32>,
    pub location_province: Option<u32>,
}

/// Production replay adapter for the A7 air-base placement carrier.
///
/// `Leaf::Unobtained` and `Leaf::Absent` both remain `None`; no state centroid
/// or neighboring province is relabeled as exact recording data here.
pub fn wing_placement_input_from_world_row(
    row: &replay_core::WingUnitRow,
) -> WingPlacementInput {
    WingPlacementInput {
        uid: row.wing_uid.clone(),
        owner: row.owner.measured().cloned(),
        state_id: row.state_id.measured().copied(),
        location_province: row.air_base_province.measured().copied(),
    }
}

/// A whole hour's placement set (the per-frame product).
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

/// Place one land division at its `location_province` (step-hold; see module).
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

/// Place one fleet / task force at its `location_province` (sea province).
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

/// Place one air wing. Resolution order: exact recorded air-base province
/// from A7/D34 → air-base **state** representative province (legacy FIT,
/// via `state_anchor_provinces`) → unplaceable.
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

/// Place a whole hour's unit inputs into one [`UnitPlacementSet`]. Call once per
/// timeline-resolved frame (`state_at(hour)`) to get the step-hold placement.
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
        // Two slots for province 7: kind 3 first, then the centre kind 0.
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
            // Province 8 has no centre; lowest kind (5) wins over 9.
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
        state_anchor.insert(30u32, 2500u32); // state 30 -> representative province
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
            location_province: Some(3000), // exact A7 air-base province wins
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
            owner: Leaf::Measured("ENG".into()),
            name: Leaf::Absent,
            plane_count: Leaf::Measured(100),
            experience: Leaf::Absent,
            manpower: Leaf::Absent,
            mission_kind: Leaf::Absent,
            strategic_region_id: Leaf::Measured(68),
            air_base: Leaf::Measured("39:65".into()),
            air_base_province: Leaf::Measured(6508),
            state_id: Leaf::Measured(193),
            plane_definition: Leaf::Absent,
            effective_planes: Leaf::Absent,
            equipment: Leaf::Absent,
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
            state_id: Some(99), // not in the state map
            location_province: None,
        };
        let p = place_wing(&w, &table, &BTreeMap::new());
        assert_eq!(p.status, PlacementStatus::AnchorProvinceUnavailable);
        let _ = table; // table irrelevant when no province is chosen
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
