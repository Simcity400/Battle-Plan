//! A3/A4/A6 round-trip certification gates.
//!
//! Each positive fixture is opened through the real `ReplaySource` and, at
//! every recorded hour, the typed `WorldTimeline` view is asserted equal to the
//! generator's ground truth — which is exactly the value the packed snapshot
//! carries at that raw JSON path (the fixture is the oracle: what the generator
//! wrote == what `frame.raw()` returns == what the typed view must expose).
//!
//! * A3 — per-domain round-trip gates below, one test per consumer contract.
//! * A4 — `seek(h) == play_to(h)` on every fixture, forward/backward/mid-file
//!   plus in-between scrub.
//! * A6 — absence honesty: domain-absent vs domain-unavailable vs present.
//!
//! Delta==snapshot cross-check: the A1/A2 fixtures are **snapshot-flavor**
//! containers (per-hour `role: "snapshot"` entries); their `collector_raw`
//! stream is an audit placeholder, not a field-level delta encoding, and
//! `replay_core` has no delta-reconstruction path (Backend 2 is fail-closed in
//! `source.rs`). So there is no delta to reconstruct; instead
//! `snapshot_path_is_deterministic` gates the snapshot path (re-open yields
//! byte-identical world state).

use replay_core::{
    open_recording, Leaf, PlayerCountry, SnowMudCell, ViewStatus, WorldFrame, WorldTimeline,
};
use serde_json::Value;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// fixture plumbing
// ---------------------------------------------------------------------------

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}

fn fixture_manifest() -> Value {
    let path = fixture_root().join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture manifest {}: {err}", path.display()));
    serde_json::from_str(&text).expect("fixture manifest json")
}

fn fixture_path(name: &str) -> PathBuf {
    let manifest = fixture_manifest();
    let rel = manifest["positive"]
        .as_array()
        .expect("positive fixture list")
        .iter()
        .find(|entry| entry["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("fixture {name:?} not listed"))["path"]
        .as_str()
        .expect("fixture path")
        .to_string();
    fixture_root().join(rel)
}

fn open_fixture(name: &str) -> WorldTimeline {
    let source = open_recording(&fixture_path(name))
        .unwrap_or_else(|err| panic!("open fixture {name} through ReplaySource: {err}"));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline fixture {name}: {err}"))
}

/// Navigate a raw JSON path under a frame's untouched domain map (arrays keyed
/// by decimal-index segments), mirroring the manifest probe addressing.
fn raw<'a>(frame: &'a WorldFrame, path: &[&str]) -> Option<&'a Value> {
    let mut cur = frame.domain(path[0])?;
    for seg in &path[1..] {
        match cur {
            Value::Object(map) => cur = map.get(*seg)?,
            Value::Array(items) => cur = items.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        }
    }
    Some(cur)
}

// Expected-leaf builders: None => Absent, explicit null => Unobtained, a
// typed value => Measured. A wrong JSON type panics the test (fixtures are
// well-formed; a type mismatch means a broken fixture/test, never a coercion).
fn exp_string(v: Option<&Value>) -> Leaf<String> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::String(s)) => Leaf::Measured(s.clone()),
        other => panic!("exp_string: not a string: {other:?}"),
    }
}
fn exp_u32(v: Option<&Value>) -> Leaf<u32> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(n) => Leaf::Measured(u32::try_from(n.as_u64().expect("u64")).expect("u32")),
    }
}
fn exp_i64(v: Option<&Value>) -> Leaf<i64> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(n) => Leaf::Measured(n.as_i64().expect("i64")),
    }
}
fn exp_f64(v: Option<&Value>) -> Leaf<f64> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(n) => Leaf::Measured(n.as_f64().expect("f64")),
    }
}
fn exp_bool(v: Option<&Value>) -> Leaf<bool> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Bool(b)) => Leaf::Measured(*b),
        other => panic!("exp_bool: not a bool: {other:?}"),
    }
}
fn exp_string_list(v: Option<&Value>) -> Leaf<Vec<String>> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Array(items)) => Leaf::Measured(
            items
                .iter()
                .map(|it| it.as_str().expect("string item").to_string())
                .collect(),
        ),
        other => panic!("exp_string_list: not an array: {other:?}"),
    }
}

/// Every recorded hour's frame, in index order.
fn frames(t: &WorldTimeline) -> Vec<std::sync::Arc<WorldFrame>> {
    t.hours()
        .iter()
        .map(|&h| t.frame(h).unwrap_or_else(|e| panic!("frame {h}: {e}")))
        .collect()
}

// ---------------------------------------------------------------------------
// A3 — per-domain round-trip gates
// ---------------------------------------------------------------------------

#[test]
fn rt_owner_controller_flip() {
    let t = open_fixture("fx_owner_controller_flip");
    for f in frames(&t) {
        for sid in [10u32, 20, 30] {
            let key = sid.to_string();
            assert_eq!(
                f.owner_by_state()
                    .get(&sid)
                    .cloned()
                    .unwrap_or(Leaf::Absent),
                exp_string(raw(&f, &["states", &key, "owner"])),
                "owner state {sid} @ {}",
                f.hour_index()
            );
            assert_eq!(
                f.control_by_state()
                    .get(&sid)
                    .cloned()
                    .unwrap_or(Leaf::Absent),
                exp_string(raw(&f, &["states", &key, "controller"])),
                "control state {sid} @ {}",
                f.hour_index()
            );
        }
        for pid in [1000u32, 1001, 2000, 3000] {
            let key = pid.to_string();
            assert_eq!(
                f.control_by_province()
                    .get(&pid)
                    .cloned()
                    .unwrap_or(Leaf::Absent),
                exp_string(raw(&f, &["provinces", &key, "controller"])),
                "control province {pid} @ {}",
                f.hour_index()
            );
        }
    }
}

#[test]
fn rt_cosmetic_ideology() {
    let t = open_fixture("fx_cosmetic_ideology");
    for f in frames(&t) {
        let id = f.country_identity("GER").expect("GER identity");
        assert_eq!(
            id.cosmetic_tag,
            exp_string(raw(&f, &["countries", "GER", "extra", "cosmetic_tag"])),
            "cosmetic_tag @ {}",
            f.hour_index()
        );
        assert_eq!(
            id.ruling_ideology,
            exp_string(raw(&f, &["countries", "GER", "politics", "ruling_party"])),
            "ruling_ideology @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_war_faction_matrix() {
    let t = open_fixture("fx_war_faction_matrix");
    for f in frames(&t) {
        let dip = f.diplomacy();
        // Faction membership derives from politics.faction.{name,is_leader};
        // a null faction (pre-formation) is captured-but-unobtained.
        for tag in ["GER", "ITA", "ENG"] {
            let faction = raw(&f, &["countries", tag, "politics", "faction"]);
            let want_name = match faction {
                Some(Value::Object(o)) => exp_string(o.get("name")),
                Some(Value::Null) => Leaf::Unobtained,
                None => Leaf::Absent,
                other => panic!("faction: {other:?}"),
            };
            assert_eq!(
                dip.countries[tag].faction_name,
                want_name,
                "{tag} faction_name @ {}",
                f.hour_index()
            );
        }
        // War edges derive from diplomacy.wars[*].against.
        let ger_at_war_eng = matches!(
            raw(&f, &["countries", "GER", "diplomacy", "wars", "0", "against"]),
            Some(Value::String(s)) if s == "ENG"
        );
        let pair = f.diplomatic_pair_state("GER", "ENG");
        if ger_at_war_eng {
            assert_eq!(
                pair.expect("GER|ENG pair").at_war,
                Leaf::Measured(true),
                "GER at war ENG @ {}",
                f.hour_index()
            );
            assert_eq!(
                f.diplomatic_pair_state("ENG", "ITA")
                    .expect("ENG|ITA pair")
                    .at_war,
                Leaf::Measured(true),
                "ENG at war ITA @ {}",
                f.hour_index()
            );
        } else {
            assert!(
                pair.is_none_or(|p| p.at_war != Leaf::Measured(true)),
                "no GER|ENG war before wars[] present @ {}",
                f.hour_index()
            );
        }
        // Same-faction folds from equal faction names.
        let ger_axis = matches!(
            raw(&f, &["countries", "GER", "politics", "faction", "name"]),
            Some(Value::String(s)) if s == "Axis"
        );
        if ger_axis {
            assert_eq!(
                f.diplomatic_pair_state("GER", "ITA")
                    .expect("GER|ITA pair")
                    .same_faction,
                Leaf::Measured(true),
                "GER|ITA same faction @ {}",
                f.hour_index()
            );
        }
    }
}

#[test]
fn rt_capital_moves() {
    let t = open_fixture("fx_capital_moves");
    for f in frames(&t) {
        assert_eq!(
            f.diplomacy().countries["FRA"].capital_state,
            exp_u32(raw(&f, &["countries", "FRA", "politics", "capital_state"])),
            "FRA capital @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_weather_sweep() {
    let t = open_fixture("fx_weather_sweep");
    for f in frames(&t) {
        let row = f.weather_by_region().get(&101).expect("region 101");
        assert_eq!(
            row.weather_kind,
            exp_string(raw(&f, &["weather", "regions", "101", "phenom_name"])),
            "weather_kind @ {}",
            f.hour_index()
        );
        assert_eq!(
            row.intensity,
            exp_f64(raw(&f, &["weather", "regions", "101", "intensity"])),
            "intensity @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_snow_mud_accumulator() {
    let t = open_fixture("fx_snow_mud_accumulator");
    for f in frames(&t) {
        let view = f.snow_mud();
        assert_eq!(view.status, ViewStatus::Present, "snow_mud present");
        // Each texture cell round-trips per province.
        let texture = raw(&f, &["weather", "snow_mud", "texture"])
            .and_then(Value::as_array)
            .expect("texture array");
        for cell in texture {
            let pid = u32::try_from(cell["province_id"].as_u64().unwrap()).unwrap();
            let got: &SnowMudCell = view.by_province.get(&pid).expect("province cell");
            assert_eq!(
                got.snow_level_0_255,
                exp_u32(cell.get("snow_level_0_255")),
                "snow p{pid} @ {}",
                f.hour_index()
            );
            assert_eq!(
                got.mud_level_0_255,
                exp_u32(cell.get("mud_level_0_255")),
                "mud p{pid} @ {}",
                f.hour_index()
            );
            assert_eq!(got.region_id, exp_u32(cell.get("region_id")));
        }
        assert_eq!(
            view.fade_phase,
            exp_f64(raw(&f, &["weather", "snow_mud", "fade", "phase"])),
            "fade phase @ {}",
            f.hour_index()
        );
        assert_eq!(
            view.fade_0_hour,
            exp_i64(raw(&f, &["weather", "snow_mud", "fade", "fade_0_hour"]))
        );
        assert_eq!(
            view.fade_1_hour,
            exp_i64(raw(&f, &["weather", "snow_mud", "fade", "fade_1_hour"]))
        );
    }
}

#[test]
fn rt_topbar_scalars_sweep() {
    let t = open_fixture("fx_topbar_scalars_sweep");
    for f in frames(&t) {
        for tag in ["GER", "ENG", "SOV"] {
            let s = f
                .topbar_scalars(tag)
                .unwrap_or_else(|| panic!("{tag} topbar"));
            let p = |k: &[&str]| {
                let mut path = vec!["countries", tag];
                path.extend_from_slice(k);
                raw(&f, &path).cloned()
            };
            assert_eq!(
                s.political_power,
                exp_f64(p(&["politics", "political_power"]).as_ref())
            );
            assert_eq!(s.stability, exp_f64(p(&["politics", "stability"]).as_ref()));
            assert_eq!(
                s.war_support,
                exp_f64(p(&["politics", "war_support"]).as_ref())
            );
            assert_eq!(
                s.command_power,
                exp_f64(p(&["politics", "command_power"]).as_ref())
            );
            assert_eq!(
                s.manpower_available,
                exp_i64(p(&["military", "manpower", "available"]).as_ref())
            );
            assert_eq!(s.xp_army, exp_f64(p(&["military", "xp", "army"]).as_ref()));
            assert_eq!(s.xp_navy, exp_f64(p(&["military", "xp", "navy"]).as_ref()));
            assert_eq!(s.xp_air, exp_f64(p(&["military", "xp", "air"]).as_ref()));
            assert_eq!(
                s.factories_civ,
                exp_i64(p(&["economy", "factories", "civ"]).as_ref())
            );
            assert_eq!(
                s.factories_mil,
                exp_i64(p(&["economy", "factories", "mil"]).as_ref())
            );
            assert_eq!(
                s.factories_dockyard,
                exp_i64(p(&["economy", "factories", "dockyard"]).as_ref())
            );
            assert_eq!(
                s.fuel_current,
                exp_f64(p(&["economy", "fuel", "current"]).as_ref())
            );
            assert_eq!(s.fuel_max, exp_f64(p(&["economy", "fuel", "max"]).as_ref()));
            assert_eq!(
                s.nukes_total,
                exp_i64(p(&["extra", "nukes_total"]).as_ref())
            );
            assert_eq!(
                s.nukes_ready,
                exp_i64(p(&["extra", "nukes_ready_total"]).as_ref())
            );
            assert_eq!(
                s.supply_value,
                exp_f64(p(&["scalars", "supply_value"]).as_ref())
            );
            assert_eq!(
                s.supply_ratio,
                exp_f64(p(&["scalars", "supply_ratio"]).as_ref())
            );
            // convoys: the sweep reports the scalar `military.convoys_count`.
            let convoys = p(&["military", "convoys_count"]);
            let want = convoys
                .as_ref()
                .and_then(Value::as_f64)
                .map(Leaf::Measured)
                .unwrap();
            assert_eq!(s.convoys, want, "{tag} convoys @ {}", f.hour_index());
            // DOCUMENTED GAP (honest, fail-closed): the fixture parks world
            // tension under a non-canonical `world.world_tension_inputs.*`
            // fixture key, so the canonical topbar reader leaves it Absent —
            // never a fabricated default.
            assert_eq!(
                s.world_tension,
                Leaf::Absent,
                "{tag} world_tension gap @ {}",
                f.hour_index()
            );
        }
    }
}

#[test]
fn rt_division_movement() {
    let t = open_fixture("fx_division_movement");
    for f in frames(&t) {
        let d = f
            .division_units()
            .iter()
            .find(|d| d.div_uid == "101:1")
            .expect("division 101:1");
        assert_eq!(
            d.location_province,
            exp_u32(raw(&f, &["divisions", "0", "location_province"])),
            "loc @ {}",
            f.hour_index()
        );
        assert_eq!(d.moving, exp_bool(raw(&f, &["divisions", "0", "moving"])));
        assert_eq!(
            d.move_target_province,
            exp_u32(raw(&f, &["divisions", "0", "move_target_province"]))
        );
        assert_eq!(
            d.move_progress,
            exp_f64(raw(&f, &["divisions", "0", "move_progress"]))
        );
        assert_eq!(
            d.in_combat,
            exp_bool(raw(&f, &["divisions", "0", "in_combat"]))
        );
        assert_eq!(
            d.strength,
            exp_f64(raw(&f, &["divisions", "0", "strength"]))
        );
        // Active combats round-trip when present.
        if let Some(Value::Array(cs)) = f.domain("combats") {
            if !cs.is_empty() {
                let c = &f.combats()[0];
                assert_eq!(
                    c.combat_id,
                    exp_i64(raw(&f, &["combats", "0", "combat_id"]))
                );
                assert_eq!(c.kind, exp_string(raw(&f, &["combats", "0", "kind"])));
                assert_eq!(c.attackers, Leaf::Measured(vec![101]));
                assert_eq!(c.defenders, Leaf::Measured(vec![201]));
            }
        }
    }
}

#[test]
fn rt_naval_air_activity() {
    let t = open_fixture("fx_naval_air_activity");
    for f in frames(&t) {
        // Region naval supremacy (from naval_air.region_control array rows).
        let sup = f.naval_supremacy_by_region().get(&301).expect("region 301");
        assert_eq!(
            sup.controller,
            exp_string(raw(
                &f,
                &["naval_air", "region_control", "301", "0", "controller"]
            )),
            "naval controller @ {}",
            f.hour_index()
        );
        assert_eq!(
            sup.supremacy,
            exp_f64(raw(
                &f,
                &["naval_air", "region_control", "301", "0", "supremacy"]
            ))
        );
        // Fleet / task-force presence.
        let tf = f
            .fleets()
            .iter()
            .find(|r| r.task_force_uid == "501:1")
            .expect("task force 501:1");
        let tfp = &["order_of_battle", "fleets", "0", "task_forces", "0"];
        let probe = |leaf_key: &str| {
            let mut path = tfp.to_vec();
            path.push(leaf_key);
            raw(&f, &path).cloned()
        };
        assert_eq!(
            tf.location_province,
            exp_u32(probe("location_province").as_ref())
        );
        assert_eq!(tf.mission, exp_string(probe("mission").as_ref()));
        assert_eq!(tf.ship_count, exp_i64(probe("ship_count").as_ref()));
        assert_eq!(
            tf.dominant_hull_class,
            exp_string(probe("dominant_hull_class").as_ref())
        );
        assert_eq!(tf.owner, exp_string(probe("owner").as_ref()));
        // Air wing presence.
        let w = f
            .wings()
            .iter()
            .find(|w| w.wing_uid == "800:1")
            .expect("wing 800:1");
        let wp = &["order_of_battle", "air_wings", "0"];
        let wprobe = |k: &str| {
            let mut path = wp.to_vec();
            path.push(k);
            raw(&f, &path).cloned()
        };
        assert_eq!(w.plane_count, exp_i64(wprobe("count").as_ref()));
        assert_eq!(w.mission_kind, exp_string(wprobe("mission").as_ref()));
        assert_eq!(
            w.strategic_region_id,
            exp_u32(wprobe("strategic_region").as_ref())
        );
        assert_eq!(w.state_id, exp_u32(wprobe("state_id").as_ref()));
        assert_eq!(
            w.effective_planes,
            exp_i64(wprobe("effective_planes").as_ref())
        );
    }
}

#[test]
fn rt_building_damage() {
    let t = open_fixture("fx_building_damage");
    for f in frames(&t) {
        let sb = f.buildings_by_state().get(&10).expect("state 10 buildings");
        // Undamaged buildings round-trip as level-only from states.10.buildings.
        assert_eq!(
            sb.state_buildings["infrastructure"].level,
            exp_f64(raw(&f, &["states", "10", "buildings", "infrastructure"])),
            "infra level @ {}",
            f.hour_index()
        );
        // arms_factory carries the overlaid building_damage[0] status.
        let arms = &sb.state_buildings["arms_factory"];
        let dmg0 = &["states", "10", "building_damage", "0"];
        let d = |k: &str| {
            let mut path = dmg0.to_vec();
            path.push(k);
            raw(&f, &path).cloned()
        };
        assert_eq!(
            arms.healthy_level,
            exp_f64(d("healthy_level").as_ref()),
            "healthy @ {}",
            f.hour_index()
        );
        assert_eq!(arms.damaged, exp_bool(d("damaged").as_ref()));
        assert_eq!(arms.damage_fraction, exp_f64(d("damage_fraction").as_ref()));
        assert_eq!(arms.repair_progress, exp_f64(d("repair_progress").as_ref()));
        assert_eq!(
            arms.under_construction,
            exp_bool(d("under_construction").as_ref())
        );
        // Province-scope damage overlays into province_buildings[1000].
        let naval = sb
            .province_buildings
            .get(&1000)
            .and_then(|m| m.get("naval_base"))
            .expect("province 1000 naval_base");
        let dmg1 = &["states", "10", "building_damage", "1"];
        let d1 = |k: &str| {
            let mut path = dmg1.to_vec();
            path.push(k);
            raw(&f, &path).cloned()
        };
        assert_eq!(naval.healthy_level, exp_f64(d1("healthy_level").as_ref()));
        assert_eq!(naval.damaged, exp_bool(d1("damaged").as_ref()));
        assert_eq!(
            naval.damage_fraction,
            exp_f64(d1("damage_fraction").as_ref())
        );
    }
}

#[test]
fn rt_pov_visibility() {
    let t = open_fixture("fx_pov_visibility");
    for f in frames(&t) {
        assert_eq!(
            f.visibility_status(),
            ViewStatus::Present,
            "visibility present"
        );
        for pov in ["GER", "ENG", "SOV"] {
            let view = f.visibility_for(pov);
            assert_eq!(
                view.status,
                ViewStatus::Present,
                "{pov} pov present @ {}",
                f.hour_index()
            );
            // Every recorded province visibility level round-trips.
            let provinces = raw(&f, &["visibility", "povs", pov, "provinces"])
                .and_then(Value::as_object)
                .expect("pov provinces");
            for (pid_s, level) in provinces {
                let pid: u32 = pid_s.parse().unwrap();
                let got = view.provinces.get(&pid).expect("province vis");
                assert_eq!(
                    got,
                    &Leaf::Measured(replay_core::ProvinceVisibility {
                        level: level.as_str().unwrap().to_string()
                    }),
                    "{pov} p{pid} @ {}",
                    f.hour_index()
                );
            }
            // Spotted-unit sets round-trip from visible_foreign_units.
            let spot = &["visibility", "povs", pov, "visible_foreign_units"];
            let sp = |k: &str| {
                let mut path = spot.to_vec();
                path.push(k);
                raw(&f, &path).cloned()
            };
            assert_eq!(
                view.spotted_units.divisions,
                exp_string_list(sp("divisions").as_ref())
            );
            assert_eq!(
                view.spotted_units.fleets,
                exp_string_list(sp("fleets").as_ref())
            );
            assert_eq!(
                view.spotted_units.wings,
                exp_string_list(sp("wings").as_ref())
            );
        }
    }
}

#[test]
fn rt_dlc_language_variation() {
    let t = open_fixture("fx_dlc_language_variation");
    for f in frames(&t) {
        let meta = f.session_metadata();
        assert_eq!(
            meta.active_dlcs,
            exp_string_list(raw(&f, &["world", "scenario", "dlcs"])),
            "dlcs @ {}",
            f.hour_index()
        );
        assert_eq!(
            meta.localisation_language,
            exp_string(raw(&f, &["world", "scenario", "localisation_language"])),
            "language @ {}",
            f.hour_index()
        );
        // DLC-gated cosmetic (CAN carries it only under misc.cosmetic_tag).
        let can = f.country_identity("CAN").expect("CAN identity");
        assert_eq!(
            can.cosmetic_tag,
            exp_string(raw(&f, &["countries", "CAN", "misc", "cosmetic_tag"])),
            "CAN cosmetic @ {}",
            f.hour_index()
        );
    }
}

/// Player-country picker round-trip (observer-POV dependency): the list lives
/// under `world.scenario.players` in the recording/fixture schema, not the
/// internal `session.player_countries` key. The view must surface it so the
/// POV country picker is not fail-closed on every real recording.
///
/// Oracle derived from the fixture generator source
/// (`replay_fixture_gen/src/main.rs` `base_snapshot`): each player entry is
/// `{"tag": <TAG>, "slot": <n>, "label": <name>}`, so the view's `tag` is the
/// entry's `tag` and `player_name` falls back to `label`.
#[test]
fn rt_player_countries_from_world_scenario_players() {
    for name in POSITIVE_FIXTURES {
        let t = open_fixture(name);
        for f in frames(&t) {
            let players = raw(&f, &["world", "scenario", "players"])
                .and_then(Value::as_array)
                .unwrap_or_else(|| {
                    panic!(
                        "{name} @ {}: fixture must carry world.scenario.players",
                        f.hour_index()
                    )
                });
            let expected: Vec<PlayerCountry> = players
                .iter()
                .map(|entry| {
                    let obj = entry.as_object().expect("player entry object");
                    PlayerCountry {
                        tag: exp_string(obj.get("tag")),
                        player_name: exp_string(obj.get("label")),
                    }
                })
                .collect();
            assert_eq!(
                f.session_metadata().player_countries,
                Leaf::Measured(expected),
                "{name} @ {}: player_countries from world.scenario.players",
                f.hour_index()
            );
        }
    }
}

#[test]
fn well_formed_fixtures_emit_no_coverage_warnings() {
    // The fixtures are well-typed, so no view should ever coerce or warn.
    for name in POSITIVE_FIXTURES {
        let t = open_fixture(name);
        for f in frames(&t) {
            assert!(
                f.coverage_warnings().is_empty(),
                "{name} @ {}: unexpected coverage warnings: {:?}",
                f.hour_index(),
                f.coverage_warnings()
            );
        }
    }
}

const POSITIVE_FIXTURES: [&str; 12] = [
    "fx_owner_controller_flip",
    "fx_cosmetic_ideology",
    "fx_war_faction_matrix",
    "fx_capital_moves",
    "fx_weather_sweep",
    "fx_snow_mud_accumulator",
    "fx_topbar_scalars_sweep",
    "fx_division_movement",
    "fx_naval_air_activity",
    "fx_building_damage",
    "fx_pov_visibility",
    "fx_dlc_language_variation",
];

// ---------------------------------------------------------------------------
// A4 — seek(h) == play_to(h) equality (forward / backward / mid-file / scrub)
// ---------------------------------------------------------------------------

/// Compare all observable world state between two frames. The frame is a pure
/// function of (hour, date, save_version, raw), so raw-map equality is total;
/// a spread of typed views is asserted alongside it to lock the contract that
/// `seek` and `play_to` expose byte-identical *views*, not just identical raw.
fn assert_state_equal(a: &WorldFrame, b: &WorldFrame, ctx: &str) {
    assert_eq!(a.hour_index(), b.hour_index(), "{ctx}: hour");
    assert_eq!(a.date(), b.date(), "{ctx}: date");
    assert_eq!(a.save_version(), b.save_version(), "{ctx}: save_version");
    assert_eq!(a.raw(), b.raw(), "{ctx}: raw domain map");
    assert_eq!(
        a.owner_by_state(),
        b.owner_by_state(),
        "{ctx}: owner_by_state"
    );
    assert_eq!(
        a.control_by_state(),
        b.control_by_state(),
        "{ctx}: control_by_state"
    );
    assert_eq!(
        a.control_by_province(),
        b.control_by_province(),
        "{ctx}: control_by_province"
    );
    assert_eq!(
        a.division_units(),
        b.division_units(),
        "{ctx}: division_units"
    );
    assert_eq!(a.fleets(), b.fleets(), "{ctx}: fleets");
    assert_eq!(a.wings(), b.wings(), "{ctx}: wings");
    assert_eq!(a.combats(), b.combats(), "{ctx}: combats");
    assert_eq!(a.countries(), b.countries(), "{ctx}: countries");
    assert_eq!(
        a.country_identities(),
        b.country_identities(),
        "{ctx}: identities"
    );
    assert_eq!(a.diplomacy(), b.diplomacy(), "{ctx}: diplomacy");
    assert_eq!(
        a.topbar_scalars_by_country(),
        b.topbar_scalars_by_country(),
        "{ctx}: topbar"
    );
    assert_eq!(
        a.weather_by_region(),
        b.weather_by_region(),
        "{ctx}: weather"
    );
    assert_eq!(a.snow_mud(), b.snow_mud(), "{ctx}: snow_mud");
    assert_eq!(
        a.buildings_by_state(),
        b.buildings_by_state(),
        "{ctx}: buildings"
    );
    assert_eq!(
        a.naval_supremacy_by_region(),
        b.naval_supremacy_by_region(),
        "{ctx}: naval"
    );
    assert_eq!(a.session_metadata(), b.session_metadata(), "{ctx}: session");
    for pov in ["GER", "ENG", "SOV"] {
        assert_eq!(
            a.visibility_for(pov),
            b.visibility_for(pov),
            "{ctx}: visibility {pov}"
        );
    }
}

#[test]
fn seek_equals_play_on_every_fixture() {
    for name in POSITIVE_FIXTURES {
        let t = open_fixture(name);
        let hours = t.hours().to_vec();

        // Forward pass over every exact hour.
        for &h in &hours {
            let s = t.seek(h).unwrap();
            let p = t.play_to(h).unwrap();
            assert_state_equal(&s, &p, &format!("{name} fwd h{h}"));
        }
        // Backward pass (mid-file start is implicit: play_to is stateless, so
        // jumping straight to the last hour then walking back must still match
        // seek at each step).
        for &h in hours.iter().rev() {
            let s = t.seek(h).unwrap();
            let p = t.play_to(h).unwrap();
            assert_state_equal(&s, &p, &format!("{name} bwd h{h}"));
        }
        // In-between scrub: an arbitrary hour between two anchors resolves to
        // the nearest snapshot at-or-before, identically for seek and play,
        // and equals the exact anchor frame.
        for w in hours.windows(2) {
            let mid = w[0] + (w[1] - w[0]) / 2;
            if mid == w[0] {
                continue;
            }
            let s = t.seek(mid).unwrap();
            let p = t.play_to(mid).unwrap();
            assert_state_equal(&s, &p, &format!("{name} scrub h{mid}"));
            let anchor = t.frame(w[0]).unwrap();
            assert_state_equal(
                &s,
                &anchor,
                &format!("{name} scrub h{mid} -> anchor {}", w[0]),
            );
        }
        // Mid-file cold start: a fresh timeline whose first access is a middle
        // hour must resolve identically to a from-the-top one.
        let fresh = open_fixture(name);
        let mid_idx = hours.len() / 2;
        let cold = fresh.play_to(hours[mid_idx]).unwrap();
        let warm = t.seek(hours[mid_idx]).unwrap();
        assert_state_equal(
            &cold,
            &warm,
            &format!("{name} cold-start h{}", hours[mid_idx]),
        );
    }
}

// ---------------------------------------------------------------------------
// A6 — absence honesty: absent vs unavailable vs present, all distinct
// ---------------------------------------------------------------------------

#[test]
fn absence_honesty_absent_vs_unavailable_vs_present() {
    // DOMAIN ABSENT: a recording that never carried a domain reports absent,
    // not unavailable and not a default. `visibility` is only present in the
    // POV fixture; everywhere else the domain key is entirely missing.
    let owner = open_fixture("fx_owner_controller_flip");
    let of = owner.frame(owner.first_hour()).unwrap();
    assert_eq!(
        of.visibility_status(),
        ViewStatus::DomainAbsent,
        "visibility absent"
    );
    assert_eq!(
        of.visibility_for("GER").status,
        ViewStatus::DomainAbsent,
        "pov absent, not unavailable"
    );
    assert_eq!(
        of.snow_mud_status(),
        ViewStatus::DomainAbsent,
        "snow_mud absent"
    );

    // DOMAIN UNAVAILABLE: the domain key is present but its contents are an
    // explicit null (captured-but-unobtained) — distinct from absent. The base
    // snapshot carries `weather: {regions: null, ...}` and
    // `naval_air: {region_control: null, ...}`.
    assert_eq!(
        of.weather_status(),
        ViewStatus::DomainUnavailable,
        "weather null -> unavailable"
    );
    assert_eq!(
        of.naval_supremacy_status(),
        ViewStatus::DomainUnavailable,
        "naval null -> unavailable"
    );

    // PRESENT: the domains the fixture exercises report present.
    let pov = open_fixture("fx_pov_visibility");
    let pf = pov.frame(pov.first_hour()).unwrap();
    assert_eq!(
        pf.visibility_status(),
        ViewStatus::Present,
        "pov visibility present"
    );
    assert_eq!(pf.visibility_for("GER").status, ViewStatus::Present);

    let weather = open_fixture("fx_weather_sweep");
    assert_eq!(
        weather
            .frame(weather.first_hour())
            .unwrap()
            .weather_status(),
        ViewStatus::Present,
        "weather present"
    );
    let snow = open_fixture("fx_snow_mud_accumulator");
    assert_eq!(
        snow.frame(snow.first_hour()).unwrap().snow_mud_status(),
        ViewStatus::Present,
        "snow_mud present"
    );
    let naval = open_fixture("fx_naval_air_activity");
    assert_eq!(
        naval
            .frame(naval.first_hour())
            .unwrap()
            .naval_supremacy_status(),
        ViewStatus::Present,
        "naval present"
    );

    // A POV that the recording never listed is absent within a present domain
    // (distinct from the domain itself being absent).
    assert_eq!(
        pf.visibility_for("USA").status,
        ViewStatus::DomainAbsent,
        "unlisted POV in a present domain -> absent"
    );
}

// ---------------------------------------------------------------------------
// snapshot-path determinism (stands in for delta==snapshot; see module docs)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_path_is_deterministic() {
    for name in POSITIVE_FIXTURES {
        let a = open_fixture(name);
        let b = open_fixture(name);
        assert_eq!(a.hours(), b.hours(), "{name}: hours");
        for &h in a.hours() {
            let fa = a.frame(h).unwrap();
            let fb = b.frame(h).unwrap();
            assert_state_equal(&fa, &fb, &format!("{name} reopen h{h}"));
        }
    }
}

// ---------------------------------------------------------------------------
// adversarial: a wrong oracle MUST fail the gate (a gate that can't fail is
// worthless)
// ---------------------------------------------------------------------------

#[test]
fn adversarial_wrong_oracle_makes_gate_fail() {
    let owner = open_fixture("fx_owner_controller_flip");
    let f = owner.frame(owner.hours()[5]).unwrap();

    // The correct oracle passes.
    assert_eq!(f.owner_by_state()[&10], Leaf::Measured("SOV".into()));
    // The discriminating gate: the view is NOT the pre-flip value.
    assert_ne!(f.owner_by_state()[&10], Leaf::Measured("GER".into()));

    // A perturbed expectation must actually panic the assertion — prove the
    // gate can fail rather than silently pass on a wrong value.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(f.owner_by_state()[&10], Leaf::Measured("GER".into()));
    }));
    std::panic::set_hook(prev);
    assert!(
        outcome.is_err(),
        "a wrong oracle must fail the round-trip gate"
    );

    // Same for a Measured-vs-Unobtained perturbation (trichotomy honesty).
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(f.owner_by_state()[&10], Leaf::<String>::Unobtained);
    }));
    std::panic::set_hook(prev);
    assert!(outcome.is_err(), "Measured must not equal Unobtained");
}
