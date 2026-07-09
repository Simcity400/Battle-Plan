//! C2 placement gates — division / fleet / wing placement proven against the A2
//! fixtures through the real `replay_core` source.
//!
//! The A2 fixtures are the oracle: `replay_core`'s round-trip gates already
//! certify that the typed unit views equal the generator ground truth at every
//! hour, so this gate asserts that placement is a faithful function of
//! (certified view + province→world anchor table): unit IDs preserved, anchor
//! province == the recorded `location_province`, world point resolved and inside
//! the map bounds, the moving division's anchor follows the fixture province
//! path exactly, and `seek(h) == play_to(h)` placement equality (step-hold
//! interpolation inherited from the timeline).
//!
//! Anchor coordinates in the hermetic gates are SYNTHETIC (injected) so the gate
//! does not depend on the vanilla install; a separate gated test builds the
//! FILE anchor table from the real `map/unitstacks.txt` and checks the fixture
//! provinces resolve to in-bounds stack-centre points.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use map_viewer::{
    place_frame_inputs, AnchorProvenance, AnchorProvinceSource, DivisionPlacementInput,
    FleetPlacementInput, GameFileResolver, PlacementStatus, ProvinceAnchorTable, UnitPlacement,
    UnitPlacementSet, WingPlacementInput,
};
use replay_core::{
    open_recording, DivisionUnitRow, FleetUnitRow, Leaf, WingUnitRow, WorldFrame, WorldTimeline,
};
use serde_json::Value;

const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

// --------------------------------------------------------------------------
// fixture plumbing (mirrors replay_core/tests/roundtrip_gates.rs)
// --------------------------------------------------------------------------

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

// --------------------------------------------------------------------------
// adapters: certified replay_core rows -> placement inputs (the one-liners
// replay_app hosts in production; here so the gate feeds real timeline views)
// --------------------------------------------------------------------------

fn leaf_string(l: &Leaf<String>) -> Option<String> {
    l.measured().cloned()
}
fn leaf_u32(l: &Leaf<u32>) -> Option<u32> {
    l.measured().copied()
}

fn division_input(row: &DivisionUnitRow) -> DivisionPlacementInput {
    DivisionPlacementInput {
        uid: row.div_uid.clone(),
        owner: leaf_string(&row.owner),
        location_province: leaf_u32(&row.location_province),
    }
}
fn fleet_input(row: &FleetUnitRow) -> FleetPlacementInput {
    FleetPlacementInput {
        uid: row.task_force_uid.clone(),
        owner: leaf_string(&row.owner),
        location_province: leaf_u32(&row.location_province),
    }
}
fn wing_input(row: &WingUnitRow) -> WingPlacementInput {
    WingPlacementInput {
        uid: row.wing_uid.clone(),
        owner: leaf_string(&row.owner),
        state_id: leaf_u32(&row.state_id),
        // WingUnitRow carries no on-map province in today's schema (gap filed);
        // the state_id drives the FIT air-base-state anchor.
        location_province: None,
    }
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

// Synthetic anchors for the fixture provinces (real vanilla coordinates so the
// numbers are realistic, but injected so the gate needs no install).
fn movement_table() -> ProvinceAnchorTable {
    let mut t = ProvinceAnchorTable::new();
    t.insert_synthetic(1000, 842.0, 9.5, 73.0);
    t.insert_synthetic(1001, 4651.0, 9.7, 1196.0);
    t.insert_synthetic(2000, 3753.0, 13.52, 1249.0);
    t
}
fn naval_air_table() -> ProvinceAnchorTable {
    let mut t = ProvinceAnchorTable::new();
    t.insert_synthetic(3000, 3612.0, 10.0, 1751.0); // fleet sea province
    t.insert_synthetic(2500, 1200.0, 9.0, 800.0); // state-30 representative province
    t
}

// --------------------------------------------------------------------------
// GATE 1 — division movement: unit IDs, anchor province follows the path,
// world point in bounds.
// --------------------------------------------------------------------------

#[test]
fn division_movement_placement_follows_fixture_path() {
    let t = open_fixture("fx_division_movement");
    let table = movement_table();
    let state_anchor = BTreeMap::new();

    let mut path: Vec<u32> = Vec::new();
    for f in frames(&t) {
        let set = placement_of(&f, &table, &state_anchor);

        // Unit-ID set is preserved exactly (both divisions, no invention).
        let mut uids: Vec<&str> = set.divisions.iter().map(|p| p.unit_uid.as_str()).collect();
        uids.sort_unstable();
        assert_eq!(
            uids,
            ["101:1", "201:1"],
            "division uids @ {}",
            f.hour_index()
        );

        let mover = find(&set.divisions, "101:1");
        // Anchor province == the certified location_province (== generator truth).
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
        // world point == the anchor table entry for that province.
        let pid = mover.anchor_province.unwrap();
        assert_eq!(mover.world_anchor, Some(table.anchor(pid).unwrap().anchor));

        let cur = mover.anchor_province.unwrap();
        if path.last() != Some(&cur) {
            path.push(cur);
        }
    }
    // The moving division visits exactly 1000 -> 1001 -> 2000 in order.
    assert_eq!(path, [1000, 1001, 2000], "fixture province path");
}

// --------------------------------------------------------------------------
// GATE 2 — naval + air placement: fleet at its sea province, wing via the
// air-base state FIT.
// --------------------------------------------------------------------------

#[test]
fn naval_air_placement_anchors_fleet_and_wing() {
    let t = open_fixture("fx_naval_air_activity");
    let table = naval_air_table();
    let mut state_anchor = BTreeMap::new();
    state_anchor.insert(30u32, 2500u32); // wing state 30 -> representative province

    for f in frames(&t) {
        let set = placement_of(&f, &table, &state_anchor);

        // Fleet task force 501:1 at sea province 3000 (recording location).
        let fleet = find(&set.fleets, "501:1");
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

        // Air wing 800:1 anchored via its air-base state (FIT).
        let wing = find(&set.wings, "800:1");
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

// A wing with no state mapping is unplaceable, never invented (fail-closed).
#[test]
fn wing_without_state_mapping_is_unavailable() {
    let t = open_fixture("fx_naval_air_activity");
    let table = naval_air_table();
    let empty = BTreeMap::new();
    let f = t.frame(t.first_hour()).unwrap();
    let set = placement_of(&f, &table, &empty);
    let wing = find(&set.wings, "800:1");
    assert_eq!(wing.status, PlacementStatus::AnchorProvinceUnavailable);
    assert_eq!(wing.world_anchor, None);
}

// --------------------------------------------------------------------------
// GATE 3 — seek == play placement equality + step-hold interpolation.
// --------------------------------------------------------------------------

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

        // seek(h) == play_to(h) placement, forward and backward.
        for &h in hours.iter().chain(hours.iter().rev()) {
            let s = placement_of(&t.seek(h).unwrap(), &table, &state_anchor);
            let p = placement_of(&t.play_to(h).unwrap(), &table, &state_anchor);
            assert_eq!(s, p, "{name}: seek==play @ {h}");
        }

        // Step-hold: an hour strictly between two anchors resolves to the prior
        // anchor's placement (no invented smoothing).
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

// --------------------------------------------------------------------------
// GATE 4 — FILE anchor table from the real map/unitstacks.txt resolves the
// fixture provinces to in-bounds stack-centre points (gated on the install).
// --------------------------------------------------------------------------

#[test]
fn gated_vanilla_unitstacks_anchor_table_places_fixture_provinces() {
    let root = std::path::Path::new(VANILLA_GAME_DIR);
    if !root.is_dir() {
        eprintln!("skipping vanilla unitstacks anchor gate: {VANILLA_GAME_DIR} absent");
        return;
    }
    let resolver = GameFileResolver::vanilla(root.to_path_buf()).expect("vanilla resolver");
    let table = ProvinceAnchorTable::from_resolver(&resolver).expect("anchor table");
    assert!(table.len() > 13_000, "expected the full stack-centre table");

    // Every fixture province used by the A2 unit fixtures exists in the shipped
    // file and resolves to an in-bounds stack-centre (FILE) point.
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

    // Feed the real table through the division fixture: the mover is placed
    // FILE-derived and in-bounds at every hour.
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
