
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use map_viewer::{
    place_frame_inputs, wing_placement_input_from_world_row, AnchorProvenance,
    AnchorProvinceSource, DivisionPlacementInput,
    FleetPlacementInput, GameFileResolver, PlacementStatus, ProvinceAnchorTable, UnitPlacement,
    UnitPlacementSet, WingPlacementInput,
};
use replay_core::{
    open_recording, DivisionUnitRow, FleetUnitRow, Leaf, WingUnitRow, WorldFrame, WorldTimeline,
};
use serde_json::Value;


fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}

fn fixture_path(name: &str) -> PathBuf {
    let manifest_path = fixture_root().join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read manifest {}: {e}", manifest_path.display()));
    let manifest: Value = serde_json::from_str(&text).expect("manifest json");
    let rel = manifest["positive"]
        .as_array()
        .expect("positive list")
        .iter()
        .find(|e| e["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("fixture {name:?} not listed"))["path"]
        .as_str()
        .expect("path")
        .to_string();
    fixture_root().join(rel)
}

fn open_fixture(name: &str) -> WorldTimeline {
    let source = open_recording(&fixture_path(name)).unwrap_or_else(|e| panic!("open {name}: {e}"));
    WorldTimeline::open(source).unwrap_or_else(|e| panic!("timeline {name}: {e}"))
}

fn frames(t: &WorldTimeline) -> Vec<Arc<WorldFrame>> {
    t.hours()
        .iter()
        .map(|&h| t.frame(h).unwrap_or_else(|e| panic!("frame {h}: {e}")))
        .collect()
}


fn leaf_string(l: &Leaf<String>) -> Option<String> {
    l.measured().cloned()
}
fn leaf_u32(l: &Leaf<u32>) -> Option<u32> {
    l.measured().copied()
}

/// The TAG of an admitted identity. DERIVED: the census was asked where the
/// row was decoded, so a placement input may spell the name.
fn leaf_identity_tag(l: &replay_core::Leaf<replay_core::CountryIdentityRef>) -> Option<String> {
    l.measured().map(|owner| owner.tag().to_string())
}

fn division_input(row: &DivisionUnitRow) -> DivisionPlacementInput {
    DivisionPlacementInput {
        uid: row.div_uid.clone(),
        owner: leaf_identity_tag(&row.owner),
        location_province: leaf_u32(&row.location_province),
    }
}
fn fleet_input(row: &FleetUnitRow) -> FleetPlacementInput {
    FleetPlacementInput {
        uid: row.task_force_uid.clone(),
        owner: leaf_identity_tag(&row.owner),
        location_province: leaf_u32(&row.location_province),
    }
}
fn wing_input(row: &WingUnitRow) -> WingPlacementInput {
    wing_placement_input_from_world_row(row)
}

fn placement_of(
    frame: &WorldFrame,
    table: &ProvinceAnchorTable,
    state_anchor: &BTreeMap<u32, u32>,
) -> UnitPlacementSet {
    let divisions: Vec<_> = frame.division_units().iter().map(division_input).collect();
    let fleets: Vec<_> = frame.fleets().iter().map(fleet_input).collect();
    let wings: Vec<_> = frame.wings().iter().map(wing_input).collect();
    place_frame_inputs(
        frame.hour_index(),
        &divisions,
        &fleets,
        &wings,
        table,
        state_anchor,
    )
}

fn find<'a>(set: &'a [UnitPlacement], uid: &str) -> &'a UnitPlacement {
    set.iter()
        .find(|p| p.unit_uid == uid)
        .unwrap_or_else(|| panic!("unit {uid} not in placement set"))
}

fn movement_table() -> ProvinceAnchorTable {
    let mut t = ProvinceAnchorTable::new();
    t.insert_synthetic(1000, 842.0, 9.5, 73.0);
    t.insert_synthetic(1001, 4651.0, 9.7, 1196.0);
    t.insert_synthetic(2000, 3753.0, 13.52, 1249.0);
    t
}
fn naval_air_table() -> ProvinceAnchorTable {
    let mut t = ProvinceAnchorTable::new();
    t.insert_synthetic(3000, 3612.0, 10.0, 1751.0);
    t.insert_synthetic(2500, 1200.0, 9.0, 800.0);
    t
}


#[test]
fn division_movement_placement_follows_fixture_path() {
    let t = open_fixture("fx_division_movement");
    let table = movement_table();
    let state_anchor = BTreeMap::new();

    let mut path: Vec<u32> = Vec::new();
    for f in frames(&t) {
        let set = placement_of(&f, &table, &state_anchor);

        let mut uids: Vec<&str> = set.divisions.iter().map(|p| p.unit_uid.as_str()).collect();
        uids.sort_unstable();
        assert_eq!(
            uids,
            ["101:1", "201:1"],
            "division uids @ {}",
            f.hour_index()
        );

        let mover = find(&set.divisions, "101:1");
        let certified = f
            .division_units()
            .iter()
            .find(|d| d.div_uid == "101:1")
            .expect("cert 101:1")
            .location_province
            .measured()
            .copied();
        assert_eq!(
            mover.anchor_province,
            certified,
            "anchor province tracks recording @ {}",
            f.hour_index()
        );
        assert_eq!(mover.status, PlacementStatus::Placed);
        assert_eq!(
            mover.anchor_province_source,
            AnchorProvinceSource::LocationProvince
        );
        let world = mover.world_anchor.expect("world anchor");
        assert!(
            world.in_map_bounds(),
            "world point {:?} in bounds @ {}",
            world,
            f.hour_index()
        );
        let pid = mover.anchor_province.unwrap();
        assert_eq!(mover.world_anchor, Some(table.anchor(pid).unwrap().anchor));

        let cur = mover.anchor_province.unwrap();
        if path.last() != Some(&cur) {
            path.push(cur);
        }
    }
    assert_eq!(path, [1000, 1001, 2000], "fixture province path");
}


#[test]
fn naval_air_placement_anchors_fleet_and_wing() {
    let t = open_fixture("fx_naval_air_activity");
    let table = naval_air_table();
    let mut state_anchor = BTreeMap::new();
    state_anchor.insert(30u32, 2500u32);

    for f in frames(&t) {
        let set = placement_of(&f, &table, &state_anchor);

        let fleet = find(&set.fleets, "501");
        assert_eq!(
            fleet.status,
            PlacementStatus::Placed,
            "fleet @ {}",
            f.hour_index()
        );
        assert_eq!(fleet.anchor_province, Some(3000));
        assert_eq!(
            fleet.anchor_province_source,
            AnchorProvinceSource::LocationProvince
        );
        assert_eq!(fleet.owner.as_deref(), Some("ENG"));
        assert!(fleet.world_anchor.unwrap().in_map_bounds());

        let wing = find(&set.wings, "800");
        assert_eq!(
            wing.status,
            PlacementStatus::Placed,
            "wing @ {}",
            f.hour_index()
        );
        assert_eq!(wing.anchor_province, Some(2500));
        assert_eq!(
            wing.anchor_province_source,
            AnchorProvinceSource::AirBaseStateRepresentative
        );
        assert!(wing.world_anchor.unwrap().in_map_bounds());
    }
}

#[test]
fn wing_without_state_mapping_is_unavailable() {
    let t = open_fixture("fx_naval_air_activity");
    let table = naval_air_table();
    let empty = BTreeMap::new();
    let f = t.frame(t.first_hour()).unwrap();
    let set = placement_of(&f, &table, &empty);
    let wing = find(&set.wings, "800");
    assert_eq!(wing.status, PlacementStatus::AnchorProvinceUnavailable);
    assert_eq!(wing.world_anchor, None);
}


#[test]
fn seek_equals_play_placement_and_step_holds() {
    for name in ["fx_division_movement", "fx_naval_air_activity"] {
        let t = open_fixture(name);
        let (table, mut state_anchor) = if name == "fx_division_movement" {
            (movement_table(), BTreeMap::new())
        } else {
            let mut sa = BTreeMap::new();
            sa.insert(30u32, 2500u32);
            (naval_air_table(), sa)
        };
        let _ = &mut state_anchor;
        let hours = t.hours().to_vec();

        for &h in hours.iter().chain(hours.iter().rev()) {
            let s = placement_of(&t.seek(h).unwrap(), &table, &state_anchor);
            let p = placement_of(&t.play_to(h).unwrap(), &table, &state_anchor);
            assert_eq!(s, p, "{name}: seek==play @ {h}");
        }

        for w in hours.windows(2) {
            let mid = w[0] + (w[1] - w[0]) / 2;
            if mid == w[0] {
                continue;
            }
            let scrub = placement_of(&t.seek(mid).unwrap(), &table, &state_anchor);
            let anchor = placement_of(&t.frame(w[0]).unwrap(), &table, &state_anchor);
            assert_eq!(
                UnitPlacementSet {
                    hour_index: w[0],
                    ..scrub.clone()
                },
                anchor,
                "{name}: step-hold scrub h{mid} -> anchor {}",
                w[0]
            );
        }
    }
}


#[test]
#[cfg_attr(not(gate_hoi4_install), ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path")]
fn gated_vanilla_unitstacks_anchor_table_places_fixture_provinces() {
    let root = replay_test_gates::require_vanilla_game_root();
    let resolver = GameFileResolver::vanilla(root).expect("vanilla resolver");
    let table = ProvinceAnchorTable::from_resolver(&resolver).expect("anchor table");
    assert!(table.len() > 13_000, "expected the full stack-centre table");

    for pid in [1000u32, 1001, 2000, 3000] {
        let a = table
            .anchor(pid)
            .unwrap_or_else(|| panic!("province {pid} missing from unitstacks"));
        assert_eq!(
            a.provenance,
            AnchorProvenance::UnitStackBaseSlot,
            "province {pid} uses the stack-centre slot"
        );
        assert!(a.anchor.in_map_bounds(), "province {pid} anchor in bounds");
    }

    let t = open_fixture("fx_division_movement");
    let state_anchor = BTreeMap::new();
    for f in frames(&t) {
        let set = placement_of(&f, &table, &state_anchor);
        let mover = find(&set.divisions, "101:1");
        assert_eq!(mover.status, PlacementStatus::Placed);
        assert_eq!(
            mover.anchor_provenance,
            Some(AnchorProvenance::UnitStackBaseSlot)
        );
        assert!(mover.world_anchor.unwrap().in_map_bounds());
    }
}
