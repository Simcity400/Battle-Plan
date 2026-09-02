
use replay_core::{
    open_recording, Leaf, PlayerCountry, SnowMudCell, ViewStatus, WarRelationRow, WorldFrame,
    WorldTimeline,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;


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

fn exp_string(v: Option<&Value>) -> Leaf<String> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::String(s)) => Leaf::Measured(s.clone()),
        other => panic!("exp_string: not a string: {other:?}"),
    }
}
fn exp_nullable_string(v: Option<&Value>) -> Leaf<Option<String>> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Measured(None),
        Some(Value::String(s)) => Leaf::Measured(Some(s.clone())),
        other => panic!("exp_nullable_string: not a string or null: {other:?}"),
    }
}
fn exp_id_string(v: Option<&Value>) -> Leaf<String> {
    match v {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::String(s)) => Leaf::Measured(s.clone()),
        Some(n) => Leaf::Measured(n.as_i64().expect("integer id").to_string()),
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

fn frames(t: &WorldTimeline) -> Vec<std::sync::Arc<WorldFrame>> {
    t.hours()
        .iter()
        .map(|&h| t.frame(h).unwrap_or_else(|e| panic!("frame {h}: {e}")))
        .collect()
}


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
            exp_nullable_string(raw(&f, &["countries", "GER", "extra", "cosmetic_tag"])),
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
        assert_eq!(view.by_province_status, ViewStatus::Present);
        assert_eq!(
            view.scale_raw,
            exp_u32(raw(&f, &["map", "_r2_weather_live", "scale_raw"])),
            "declared scale @ {}",
            f.hour_index()
        );
        let provinces = raw(&f, &["map", "_r2_weather_live", "provinces"])
            .and_then(Value::as_object)
            .expect("province carrier");
        assert!(!provinces.is_empty(), "carrier rows @ {}", f.hour_index());
        for (key, row) in provinces {
            let pid = key.parse::<u32>().expect("numeric province key");
            let got: &SnowMudCell = view.by_province.get(&pid).expect("province cell");
            assert_eq!(
                got.snow_raw,
                exp_i64(row.get("snow_raw")),
                "snow_raw p{pid} @ {}",
                f.hour_index()
            );
            let expected_mud = if row.get("mud_present") == Some(&Value::Bool(true)) {
                exp_bool(row.get("mud_active"))
            } else {
                Leaf::Unobtained
            };
            assert_eq!(got.mud, expected_mud, "mud p{pid} @ {}", f.hour_index());
        }
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
            let convoys = p(&["military", "convoys_count"]);
            let want = convoys
                .as_ref()
                .and_then(Value::as_f64)
                .map(Leaf::Measured)
                .unwrap();
            assert_eq!(s.convoys, want, "{tag} convoys @ {}", f.hour_index());
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
        let tf = f
            .fleets()
            .iter()
            // FOUND BY THE WHOLE KEY, not by the number: two clausewitz
            // registries mint task forces and their numbers collide. The uid is
            // the GAME's number and is display text only.
            .find(|r| {
                r.task_force_key()
                    == Some(replay_core::TaskForceKey {
                        id: 501,
                        id_space: 61,
                    })
            })
            .expect("task force 501 in the fleet registry");
        assert_eq!(
            tf.task_force_uid, "501",
            "the rendered uid is what the game shows and carries no registry"
        );
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
        let status_path = |leaf_key: &str| {
            raw(
                &f,
                &[
                    "order_of_battle",
                    "extra",
                    "naval_missions_status_live",
                    "task_forces",
                    "0",
                    leaf_key,
                ],
            )
        };
        assert_eq!(tf.mission, exp_string(status_path("mission_name")));
        assert_eq!(tf.mission_enum, exp_i64(status_path("mission_enum")));
        assert_eq!(tf.mission_state, exp_i64(status_path("mission_state")));
        assert_eq!(
            tf.detached_activity,
            exp_string(status_path("detached_activity"))
        );
        assert_eq!(tf.ship_count, exp_i64(probe("ship_count").as_ref()));
        assert_eq!(
            tf.dominant_hull_class,
            exp_string(probe("dominant_hull_class").as_ref())
        );
        assert_eq!(tf.owner, exp_string(probe("owner").as_ref()));
        let w = f
            .wings()
            .iter()
            .find(|w| w.wing_uid == "800")
            .expect("wing 800");
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
fn rt_air_superiority() {
    let t = open_fixture("fx_naval_air_activity");
    for f in frames(&t) {
        assert_eq!(
            f.air_superiority_status(),
            ViewStatus::Present,
            "air superiority present @ {}",
            f.hour_index()
        );
        let row = f.air_superiority(301).expect("region 301 air superiority");
        let p = |k: &str| raw(&f, &["naval_air", "air_superiority", "301", k]).cloned();
        assert_eq!(
            row.controller,
            exp_string(p("controller").as_ref()),
            "air controller @ {}",
            f.hour_index()
        );
        assert_eq!(row.mission_mask, exp_i64(p("mission_mask").as_ref()));
        assert_eq!(row.wing_count, exp_i64(p("wing_count").as_ref()));
        assert_eq!(row.plane_count, exp_i64(p("plane_count").as_ref()));
    }
}

#[test]
fn rt_air_superiority_unobtained_when_null() {
    let t = open_fixture("fx_owner_controller_flip");
    for f in frames(&t) {
        assert!(
            matches!(
                raw(&f, &["naval_air", "air_superiority"]),
                Some(serde_json::Value::Null)
            ),
            "fixture parks air_superiority null @ {}",
            f.hour_index()
        );
        assert_eq!(
            f.air_superiority_status(),
            ViewStatus::DomainUnavailable,
            "air superiority unavailable @ {}",
            f.hour_index()
        );
        assert!(f.air_superiority_by_region().is_empty());
    }
}

#[test]
fn rt_diplomacy_live_surfaces() {
    let t = open_fixture("fx_war_faction_matrix");
    for f in frames(&t) {
        let dip = f.diplomacy();

        match raw(&f, &["diplomacy_live", "war_relations"]) {
            Some(serde_json::Value::Null) => {
                assert_eq!(
                    dip.war_relations,
                    Leaf::Unobtained,
                    "war_relations unobtained @ {}",
                    f.hour_index()
                );
            }
            Some(serde_json::Value::Array(items)) => {
                let want: Vec<WarRelationRow> = items
                    .iter()
                    .map(|it| WarRelationRow {
                        first: exp_string(it.get("first")),
                        second: exp_string(it.get("second")),
                        first_casualties: exp_i64(it.get("first_casualties")),
                        second_casualties: exp_i64(it.get("second_casualties")),
                        threat_fixed5_raw: exp_i64(it.get("threat_fixed5_raw")),
                        threat_fixed_scale: exp_i64(it.get("threat_fixed_scale")),
                        first_was_instigator: exp_bool(it.get("first_was_instigator")),
                        hostility_reason_instigator_raw: exp_i64(
                            it.get("hostility_reason_instigator_raw"),
                        ),
                        hostility_reason_instigator: exp_string(
                            it.get("hostility_reason_instigator"),
                        ),
                        hostility_reason_defender_raw: exp_i64(
                            it.get("hostility_reason_defender_raw"),
                        ),
                        hostility_reason_defender: exp_string(
                            it.get("hostility_reason_defender"),
                        ),
                    })
                    .collect();
                assert_eq!(
                    dip.war_relations,
                    Leaf::Measured(want),
                    "war_relations @ {}",
                    f.hour_index()
                );
            }
            other => panic!("unexpected war_relations {other:?} @ {}", f.hour_index()),
        }

        match raw(&f, &["diplomacy_live", "faction_members"]) {
            Some(serde_json::Value::Null) => {
                assert_eq!(
                    dip.faction_members,
                    Leaf::Unobtained,
                    "faction_members unobtained @ {}",
                    f.hour_index()
                );
            }
            Some(serde_json::Value::Object(map)) => {
                let want: BTreeMap<String, Vec<String>> = map
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_array()
                                .expect("member array")
                                .iter()
                                .map(|m| m.as_str().expect("member tag").to_string())
                                .collect(),
                        )
                    })
                    .collect();
                assert_eq!(
                    dip.faction_members,
                    Leaf::Measured(want),
                    "faction_members @ {}",
                    f.hour_index()
                );
            }
            other => panic!("unexpected faction_members {other:?} @ {}", f.hour_index()),
        }

        match raw(&f, &["countries", "GER", "diplomacy", "relation_flags"]) {
            Some(serde_json::Value::Null) => {
                assert_eq!(
                    dip.relation_flags.get("GER"),
                    Some(&Leaf::Unobtained),
                    "GER relation_flags unobtained @ {}",
                    f.hour_index()
                );
            }
            Some(serde_json::Value::Object(map)) => {
                let want: BTreeMap<String, Vec<String>> = map
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_array()
                                .expect("flag array")
                                .iter()
                                .map(|s| s.as_str().expect("flag string").to_string())
                                .collect(),
                        )
                    })
                    .collect();
                assert_eq!(
                    dip.relation_flags.get("GER"),
                    Some(&Leaf::Measured(want)),
                    "GER relation_flags @ {}",
                    f.hour_index()
                );
            }
            other => panic!("unexpected relation_flags {other:?} @ {}", f.hour_index()),
        }

        if let Some(pair) = f.diplomatic_pair_state("GER", "ITA") {
            assert_eq!(
                pair.military_access,
                Leaf::Absent,
                "relation_flags must NOT fold into pairs @ {}",
                f.hour_index()
            );
        }
    }
}

#[test]
fn rt_topbar_ratios_alerts() {
    let t = open_fixture("fx_topbar_scalars_sweep");
    for f in frames(&t) {
        assert_eq!(
            f.topbar_extras_status(),
            ViewStatus::Present,
            "topbar extras present @ {}",
            f.hour_index()
        );
        for tag in ["GER", "ENG", "SOV"] {
            let r = f
                .topbar_ratios(tag)
                .unwrap_or_else(|| panic!("{tag} ratios"));
            let a = f
                .topbar_alerts(tag)
                .unwrap_or_else(|| panic!("{tag} alerts"));
            let p = |k: &[&str]| {
                let mut path = vec!["countries", tag, "scalars"];
                path.extend_from_slice(k);
                raw(&f, &path).cloned()
            };
            assert_eq!(
                r.industrial_ratio,
                exp_f64(p(&["industrial_ratio"]).as_ref())
            );
            assert_eq!(r.convoy_ratio, exp_f64(p(&["convoy_ratio"]).as_ref()));
            assert_eq!(r.fuel_ratio, exp_f64(p(&["fuel_ratio"]).as_ref()));
            assert_eq!(r.xp_ratio, exp_f64(p(&["xp_ratio"]).as_ref()));
            assert_eq!(a.initiative, exp_i64(p(&["alerts", "initiative"]).as_ref()));
            assert_eq!(
                a.decisions_available,
                exp_i64(p(&["alerts", "decisions_available"]).as_ref())
            );
            assert_eq!(a.intel, exp_i64(p(&["alerts", "intel"]).as_ref()));
            assert_eq!(
                a.market_contracts,
                exp_i64(p(&["alerts", "market_contracts"]).as_ref()),
                "{tag} market_contracts @ {}",
                f.hour_index()
            );
        }
    }
}

#[test]
fn rt_topbar_ratios_alerts_absent_without_scalars() {
    let t = open_fixture("fx_owner_controller_flip");
    for f in frames(&t) {
        for tag in f.topbar_ratios_by_country().keys() {
            let r = f.topbar_ratios(tag).unwrap();
            let a = f.topbar_alerts(tag).unwrap();
            assert_eq!(r.industrial_ratio, Leaf::Absent);
            assert_eq!(r.convoy_ratio, Leaf::Absent);
            assert_eq!(a.initiative, Leaf::Absent);
            assert_eq!(a.market_contracts, Leaf::Absent);
        }
    }
}

#[test]
fn rt_building_damage() {
    let t = open_fixture("fx_building_damage");
    for f in frames(&t) {
        let sb = f.buildings_by_state().get(&10).expect("state 10 buildings");
        assert_eq!(
            sb.state_buildings["infrastructure"].level,
            exp_f64(raw(&f, &["states", "10", "buildings", "infrastructure"])),
            "infra level @ {}",
            f.hour_index()
        );
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
        let can = f.country_identity("CAN").expect("CAN identity");
        assert_eq!(
            can.cosmetic_tag,
            exp_nullable_string(raw(&f, &["countries", "CAN", "misc", "cosmetic_tag"])),
            "CAN cosmetic @ {}",
            f.hour_index()
        );
    }
}

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
                        // The legacy shapes carry no engine id.
                        tag_db_id: Leaf::Absent,
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
fn rt_research_focus() {
    let t = open_fixture("fx_research_focus");
    for f in frames(&t) {
        let r = f.research("GER").expect("GER research");
        assert_eq!(
            r.tech_count,
            exp_i64(raw(&f, &["countries", "GER", "research", "tech_count"])),
            "tech_count @ {}",
            f.hour_index()
        );
        assert_eq!(
            r.research_slots,
            exp_i64(raw(&f, &["countries", "GER", "research", "research_slots"]))
        );
        let ip = raw(&f, &["countries", "GER", "research", "in_progress"])
            .and_then(Value::as_array)
            .expect("in_progress");
        let want: Vec<replay_core::ResearchProgressRow> = ip
            .iter()
            .map(|o| replay_core::ResearchProgressRow {
                name: exp_string(o.get("name")),
                points_factor: exp_f64(o.get("points_factor")),
                progress_points: exp_f64(o.get("progress_points")),
                required_points_base: exp_f64(o.get("required_points_base")),
                remaining_points_base: exp_f64(o.get("remaining_points_base")),
                progress_pct: exp_f64(o.get("progress_pct")),
                eta_days: exp_i64(o.get("eta_days")),
                progress_basis: exp_string(o.get("progress_basis")),
                progress_unavailable_reason: exp_string(o.get("progress_unavailable_reason")),
                eta_unavailable_reason: exp_string(o.get("eta_unavailable_reason")),
                ahead_of_time_years: exp_f64(o.get("ahead_of_time_years")),
                ahead_of_time_penalty_pct: exp_f64(o.get("ahead_of_time_penalty_pct")),
                ahead_of_time_reduction_years: exp_f64(o.get("ahead_of_time_reduction_years")),
                ahead_of_time_reduced_penalty_pct: exp_f64(
                    o.get("ahead_of_time_reduced_penalty_pct"),
                ),
            })
            .collect();
        assert_eq!(
            r.in_progress,
            Leaf::Measured(want),
            "research in_progress @ {}",
            f.hour_index()
        );
        let techs = raw(&f, &["countries", "GER", "research", "technologies"])
            .and_then(Value::as_array)
            .expect("technologies");
        let want_techs: Vec<replay_core::ResearchTechnologyRow> = techs
            .iter()
            .map(|o| replay_core::ResearchTechnologyRow {
                name: exp_string(o.get("name")),
                level: exp_i64(o.get("level")),
                date: exp_string(o.get("date")),
                research_points: exp_f64(o.get("research_points")),
            })
            .collect();
        assert_eq!(
            r.technologies,
            Leaf::Measured(want_techs),
            "research technologies @ {}",
            f.hour_index()
        );
        assert_eq!(
            r.doctrine,
            Leaf::Absent,
            "research doctrine @ {}",
            f.hour_index()
        );

        let focus = f.focus("GER").expect("GER focus");
        assert_eq!(
            focus.focus_tree,
            exp_string(raw(&f, &["countries", "GER", "focus", "focus_tree"]))
        );
        assert_eq!(
            focus.current,
            exp_string(raw(&f, &["countries", "GER", "focus", "current"])),
            "focus current @ {}",
            f.hour_index()
        );
        assert_eq!(
            focus.current_set,
            exp_string_list(raw(&f, &["countries", "GER", "focus", "current_set"]))
        );
        assert_eq!(
            focus.current_continuous,
            exp_string(raw(
                &f,
                &["countries", "GER", "focus", "current_continuous"]
            ))
        );
        assert_eq!(
            focus.current_continuous_set,
            exp_string_list(raw(
                &f,
                &["countries", "GER", "focus", "current_continuous_set"]
            ))
        );
        assert_eq!(
            focus.completed_count,
            exp_i64(raw(&f, &["countries", "GER", "focus", "completed_count"]))
        );
        assert_eq!(
            focus.completed,
            exp_string_list(raw(&f, &["countries", "GER", "focus", "completed"]))
        );
        assert_eq!(
            focus.progress,
            exp_f64(raw(&f, &["countries", "GER", "focus", "progress"]))
        );
        assert_eq!(
            focus.available,
            exp_string_list(raw(&f, &["countries", "GER", "focus", "available"]))
        );
        assert_eq!(
            focus.visible_order,
            exp_string_list(raw(&f, &["countries", "GER", "focus", "visible_order"]))
        );
        assert_eq!(
            focus.paused,
            exp_bool(raw(&f, &["countries", "GER", "focus", "paused"]))
        );
    }
}

#[test]
fn rt_country_management() {
    let t = open_fixture("fx_country_management");
    for f in frames(&t) {
        let pd = f.politics_detail("GER").expect("GER politics detail");
        assert_eq!(
            pd.ideas,
            exp_string_list(raw(&f, &["countries", "GER", "politics", "ideas"]))
        );
        assert_eq!(
            pd.law_economy,
            exp_string(raw(
                &f,
                &["countries", "GER", "politics", "laws", "economy"]
            )),
            "law_economy @ {}",
            f.hour_index()
        );
        assert_eq!(
            pd.appointed_advisors,
            exp_string_list(raw(
                &f,
                &["countries", "GER", "politics", "appointed_advisors"]
            ))
        );
        let pp = raw(&f, &["countries", "GER", "politics", "party_popularities"])
            .and_then(Value::as_object)
            .expect("party_popularities");
        let want_pp: BTreeMap<String, f64> = pp
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect();
        assert_eq!(pd.party_popularities, Leaf::Measured(want_pp));
        let ab = raw(&f, &["countries", "GER", "abilities"])
            .and_then(Value::as_array)
            .expect("abilities");
        let want_ab: Vec<String> = ab
            .iter()
            .map(|o| o.get("ability").unwrap().as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            pd.command_abilities,
            Leaf::Measured(want_ab),
            "command abilities @ {}",
            f.hour_index()
        );

        let prod = f.production("GER").expect("GER production");
        assert_eq!(
            prod.stockpile_total,
            exp_f64(raw(
                &f,
                &["countries", "GER", "production", "stockpile_total"]
            )),
            "stockpile_total @ {}",
            f.hour_index()
        );
        let eqs = raw(&f, &["countries", "GER", "military", "equipment_stockpile"])
            .and_then(Value::as_array)
            .expect("equipment_stockpile");
        let want_eqs: Vec<replay_core::EquipmentStock> = eqs
            .iter()
            .map(|o| replay_core::EquipmentStock {
                archetype: exp_string(o.get("archetype")),
                amount: exp_f64(o.get("amount")),
            })
            .collect();
        assert_eq!(prod.equipment_stockpile, Leaf::Measured(want_eqs));
        assert_eq!(
            prod.stockpile_complete,
            exp_bool(raw(
                &f,
                &["countries", "GER", "production", "stockpile_complete"]
            ))
        );
        assert_eq!(
            prod.military_lines_complete,
            exp_bool(raw(
                &f,
                &["countries", "GER", "production", "military_lines_complete"]
            ))
        );
        assert_eq!(
            prod.naval_lines_complete,
            exp_bool(raw(
                &f,
                &["countries", "GER", "production", "naval_lines_complete"]
            ))
        );
        for (path, actual) in [
            (
                ["countries", "GER", "production", "military_lines"],
                &prod.military_lines,
            ),
            (
                ["countries", "GER", "production", "naval_lines"],
                &prod.naval_lines,
            ),
        ] {
            let expected = match raw(&f, &path) {
                None => Leaf::Absent,
                Some(Value::Null) => Leaf::Unobtained,
                Some(Value::Array(rows)) => Leaf::Measured(
                    rows.iter()
                        .map(|value| {
                            let o = value.as_object().expect("production line object");
                            replay_core::EquipmentProductionLineRow {
                                id: exp_i64(o.get("id")),
                                priority: exp_i64(o.get("priority")),
                                line_subtype: exp_string(o.get("line_subtype")),
                                equipment_variant_id: exp_id_string(o.get("equipment_variant_id")),
                                active_factories: exp_i64(o.get("active_factories")),
                                requested_factories: exp_i64(o.get("requested_factories")),
                                efficiency: exp_f64(o.get("efficiency")),
                                efficiency_cap: exp_f64(o.get("efficiency_cap")),
                                efficiency_growth: exp_f64(o.get("efficiency_growth")),
                                cost: exp_f64(o.get("cost")),
                                speed: exp_f64(o.get("speed")),
                                produced: exp_f64(o.get("produced")),
                                amount: exp_i64(o.get("amount")),
                                progress_ratio: exp_f64(o.get("progress_ratio")),
                                daily_output: exp_f64(o.get("daily_output")),
                                days_to_next_unit: exp_f64(o.get("days_to_next_unit")),
                                mio_id: exp_id_string(o.get("mio_id")),
                            }
                        })
                        .collect(),
                ),
                other => panic!("production line list has wrong type: {other:?}"),
            };
            assert_eq!(*actual, expected, "production lines @ {}", f.hour_index());
        }

        let intel = f.intel("GER").expect("GER intel");
        assert_eq!(
            intel.agency_name,
            exp_string(raw(&f, &["countries", "GER", "intel", "agency", "name"]))
        );
        assert_eq!(
            intel.agency_upgrade_progress,
            exp_f64(raw(
                &f,
                &["countries", "GER", "intel", "agency", "upgrade_progress"]
            ))
        );
        let running = raw(&f, &["countries", "GER", "intel", "operations", "running"])
            .and_then(Value::as_array)
            .expect("operations running");
        let want_ops: Vec<replay_core::OperationRow> = running
            .iter()
            .map(|o| replay_core::OperationRow {
                op_id: exp_i64(o.get("op_id")),
                op_type: exp_string(o.get("type")),
                target: exp_string(o.get("target")),
            })
            .collect();
        assert_eq!(
            intel.operations_running,
            Leaf::Measured(want_ops),
            "operations running @ {}",
            f.hour_index()
        );

        let trade = f.trade("GER").expect("GER trade");
        let prod_res = raw(
            &f,
            &["countries", "GER", "economy", "resources", "produced"],
        )
        .and_then(Value::as_object)
        .expect("produced");
        let want_prod: BTreeMap<String, f64> = prod_res
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect();
        assert_eq!(trade.produced, Leaf::Measured(want_prod));
        let ll = raw(&f, &["countries", "GER", "diplomacy", "lend_lease"])
            .and_then(Value::as_array)
            .expect("lend_lease");
        let want_ll: Vec<replay_core::TradeContractRow> = ll
            .iter()
            .map(|o| replay_core::TradeContractRow {
                market_trade_projection: exp_bool(o.get("r2_market_trade_projection")),
                contract_id: exp_string(o.get("contract_id")),
                counterpart: exp_string(o.get("counterpart")),
                direction: exp_string(o.get("direction")),
                convoys_assigned: exp_i64(o.get("convoys").and_then(|c| c.get("assigned"))),
                visible_order: exp_i64(o.get("visible_order")),
                equipment_purchase_relation_complete: exp_bool(
                    o.get("equipment_purchase_relation_complete"),
                ),
                relation_agreement_id: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("agreement_id")),
                ),
                seller: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("seller")),
                ),
                seller_tag_db_id: exp_u32(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("seller_tag_db_id")),
                ),
                buyer: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("buyer")),
                ),
                buyer_tag_db_id: exp_u32(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("buyer_tag_db_id")),
                ),
                relation_first: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("relation_first")),
                ),
                relation_first_tag_db_id: exp_u32(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("relation_first_tag_db_id")),
                ),
                relation_second: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("relation_second")),
                ),
                relation_second_tag_db_id: exp_u32(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("relation_second_tag_db_id")),
                ),
                relation_start_date_raw: exp_i64(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("start_date_raw")),
                ),
                relation_start_date: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("start_date")),
                ),
                relation_end_date_raw: exp_i64(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("end_date_raw")),
                ),
                relation_end_date: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("end_date")),
                ),
                relation_source: exp_string(
                    o.get("equipment_purchase_relation").and_then(|r| r.get("source")),
                ),
            })
            .collect();
        assert_eq!(
            trade.contracts,
            Leaf::Measured(want_ll),
            "trade contracts @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_designs_catalog() {
    let t = open_fixture("fx_designs_catalog");
    for f in frames(&t) {
        let d = f.designs();
        assert_eq!(d.status, ViewStatus::Present);
        let tmpl0 = &d.division_templates[0];
        assert_eq!(tmpl0.template_uid, "1:1");
        assert_eq!(
            tmpl0.n_regiments,
            exp_i64(raw(
                &f,
                &["designs", "division_templates", "0", "n_regiments"]
            )),
            "n_regiments @ {}",
            f.hour_index()
        );
        assert_eq!(
            tmpl0.country,
            exp_string(raw(&f, &["designs", "division_templates", "0", "country"]))
        );
        let var = d.equipment_variants.get("200:12").expect("variant 200:12");
        assert_eq!(
            var.obsolete,
            exp_bool(raw(
                &f,
                &["designs", "equipment_variants", "200:12", "obsolete"]
            )),
            "obsolete @ {}",
            f.hour_index()
        );
        let art = d.equipment_variants.get("100:3").expect("variant 100:3");
        assert_eq!(
            art.name,
            exp_string(raw(&f, &["designs", "equipment_variants", "100:3", "name"])),
            "variant name trichotomy @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_occupation_sweep() {
    let t = open_fixture("fx_occupation_sweep");
    for f in frames(&t) {
        assert_eq!(
            f.occupation_status(),
            ViewStatus::Present,
            "occupation present @ {}",
            f.hour_index()
        );
        let occ = f.occupation(20).expect("state 20 occupation");
        let base = &["states", "20", "occupation"];
        let p = |k: &str| {
            let mut path = base.to_vec();
            path.push(k);
            raw(&f, &path).cloned()
        };
        assert_eq!(
            occ.resistance,
            exp_f64(p("resistance").as_ref()),
            "resistance @ {}",
            f.hour_index()
        );
        assert_eq!(occ.compliance, exp_f64(p("compliance").as_ref()));
        assert_eq!(
            occ.occupied_country_tag,
            exp_string(p("occupied_country_tag").as_ref())
        );
        assert_eq!(
            occ.operational_status,
            exp_string(p("operational_status").as_ref())
        );
        assert_eq!(
            occ.occupation_law,
            exp_string(raw(&f, &["states", "20", "occupation_law"])),
            "occupation_law @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_leader_roster() {
    let t = open_fixture("fx_leader_roster");
    for f in frames(&t) {
        let lr = f.leaders("GER").expect("GER leaders");
        assert_eq!(
            lr.country_leader_name,
            exp_string(raw(
                &f,
                &["countries", "GER", "commanders", "leader", "name"]
            ))
        );
        assert_eq!(
            lr.roster_size,
            exp_i64(raw(
                &f,
                &["countries", "GER", "commanders", "counts", "roster_size"]
            )),
            "roster_size @ {}",
            f.hour_index()
        );
        let fms = raw(&f, &["countries", "GER", "commanders", "field_marshals"])
            .and_then(Value::as_array)
            .expect("field_marshals");
        let want: Vec<replay_core::LeaderRow> = fms
            .iter()
            .map(|o| replay_core::LeaderRow {
                name: exp_string(o.get("name")),
                token: exp_string(o.get("token")),
                level: exp_i64(o.get("level")),
                traits: exp_string_list(o.get("traits")),
                assignment: match o.get("assignment") {
                    None => Leaf::Absent,
                    Some(Value::Null) => Leaf::Unobtained,
                    Some(Value::Object(a)) => exp_string(a.get("name")),
                    other => panic!("assignment {other:?}"),
                },
            })
            .collect();
        assert_eq!(
            lr.field_marshals,
            Leaf::Measured(want),
            "field marshals @ {}",
            f.hour_index()
        );
    }
}

#[test]
fn rt_power_balance_combat() {
    let t = open_fixture("fx_power_balance_combat");
    for f in frames(&t) {
        let pb = f.power_balance();
        assert_eq!(pb.status, ViewStatus::Present);
        let row = &pb.balances[0];
        assert_eq!(
            row.trending_side,
            exp_string(raw(&f, &["world", "power_balances", "0", "trending_side"])),
            "trending_side @ {}",
            f.hour_index()
        );
        assert_eq!(
            row.value,
            exp_f64(raw(&f, &["world", "power_balances", "0", "value"]))
        );
        assert_eq!(
            row.countries,
            exp_string_list(raw(&f, &["world", "power_balances", "0", "countries"]))
        );

        let nac = f.naval_air_combat();
        let nc = &nac.naval_combats[0];
        assert_eq!(
            nc.result_type,
            exp_string(raw(&f, &["naval_combats", "0", "result_type"])),
            "naval result_type @ {}",
            f.hour_index()
        );
        assert_eq!(
            nc.progress,
            exp_f64(raw(&f, &["naval_combats", "0", "progress"]))
        );
        assert_eq!(
            nc.location_province,
            exp_u32(raw(&f, &["naval_combats", "0", "location_province"]))
        );
        assert_eq!(
            nac.sunk_convoys,
            Leaf::Absent,
            "the positive fixture must not fabricate B4 sunk-convoy history without exact authority @ {}",
            f.hour_index()
        );

        assert_eq!(f.raids().priorities.get("ENG").copied(), Some(2));
        assert_eq!(
            f.raids().next_target,
            exp_i64(raw(&f, &["raids", "counters", "next_target"]))
        );
    }
}

#[test]
fn rt_scripted_gui_vars() {
    let t = open_fixture("fx_scripted_gui_vars");
    for f in frames(&t) {
        let sg = f.scripted_gui();
        assert_eq!(
            sg.status,
            ViewStatus::Present,
            "scripted gui present @ {}",
            f.hour_index()
        );
        let gvars = raw(&f, &["countries", "GER", "extra", "_r2_script_variables"])
            .and_then(Value::as_object)
            .expect("GER production d51 script variables");
        let want_g: BTreeMap<String, f64> = gvars
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect();
        assert_eq!(
            sg.by_country.get("GER"),
            Some(&Leaf::Measured(want_g)),
            "GER script vars @ {}",
            f.hour_index()
        );
        let wvars = raw(&f, &["world", "variables"])
            .and_then(Value::as_object)
            .expect("world variables");
        let want_w: BTreeMap<String, f64> = wvars
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect();
        assert_eq!(sg.world_variables, Leaf::Measured(want_w));
        assert_eq!(
            sg.country_flags.get("GER"),
            Some(&Leaf::Measured(vec!["inner_circle_active".to_string()]))
        );
        assert_eq!(
            sg.runtime_by_country.get("GER"),
            Some(&Leaf::Absent),
            "the variable-state fixture must not fabricate d84 runtime materialization @ {}",
            f.hour_index()
        );
        assert_eq!(sg.runtime_authority, Leaf::Absent);
        let wf = raw(&f, &["world", "flags"])
            .and_then(Value::as_object)
            .expect("world flags");
        let mut want_flags: Vec<String> = wf.keys().cloned().collect();
        want_flags.sort();
        assert_eq!(sg.world_flags, Leaf::Measured(want_flags));
    }
}

#[test]
fn rt_fx_streams_domain_absence() {
    let t = open_fixture("fx_owner_controller_flip");
    for f in frames(&t) {
        assert_eq!(
            f.occupation_status(),
            ViewStatus::DomainAbsent,
            "occupation absent @ {}",
            f.hour_index()
        );
        assert!(f.occupation_by_state().is_empty());
        assert!(f.power_balance().balances.is_empty());
        assert!(f.naval_air_combat().naval_combats.is_empty());
        assert_eq!(f.research("GER").unwrap().tech_count, Leaf::Measured(0));
        assert_eq!(
            f.scripted_gui().by_country.get("GER"),
            Some(&Leaf::Measured(BTreeMap::new()))
        );
    }
}

#[test]
fn well_formed_fixtures_emit_no_coverage_warnings() {
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

const POSITIVE_FIXTURES: [&str; 19] = [
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
    "fx_research_focus",
    "fx_country_management",
    "fx_designs_catalog",
    "fx_occupation_sweep",
    "fx_leader_roster",
    "fx_power_balance_combat",
    "fx_scripted_gui_vars",
];


fn assert_state_equal(a: &WorldFrame, b: &WorldFrame, ctx: &str) {
    assert_eq!(a.hour_index(), b.hour_index(), "{ctx}: hour");
    assert_eq!(a.date(), b.date(), "{ctx}: date");
    assert_eq!(a.save_version(), b.save_version(), "{ctx}: save_version");
    assert_eq!(
        a.raw_for_consumer_tests(),
        b.raw_for_consumer_tests(),
        "{ctx}: raw domain map"
    );
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
    assert_eq!(
        a.air_superiority_by_region(),
        b.air_superiority_by_region(),
        "{ctx}: air_superiority"
    );
    assert_eq!(
        a.topbar_ratios_by_country(),
        b.topbar_ratios_by_country(),
        "{ctx}: topbar_ratios"
    );
    assert_eq!(
        a.topbar_alerts_by_country(),
        b.topbar_alerts_by_country(),
        "{ctx}: topbar_alerts"
    );
    assert_eq!(a.session_metadata(), b.session_metadata(), "{ctx}: session");
    for pov in ["GER", "ENG", "SOV"] {
        assert_eq!(
            a.visibility_for(pov),
            b.visibility_for(pov),
            "{ctx}: visibility {pov}"
        );
    }
    assert_eq!(
        a.research_by_country(),
        b.research_by_country(),
        "{ctx}: research"
    );
    assert_eq!(a.focus_by_country(), b.focus_by_country(), "{ctx}: focus");
    assert_eq!(
        a.politics_detail_by_country(),
        b.politics_detail_by_country(),
        "{ctx}: politics_detail"
    );
    assert_eq!(
        a.production_by_country(),
        b.production_by_country(),
        "{ctx}: production"
    );
    assert_eq!(a.intel_by_country(), b.intel_by_country(), "{ctx}: intel");
    assert_eq!(a.trade_by_country(), b.trade_by_country(), "{ctx}: trade");
    assert_eq!(
        a.leaders_by_country(),
        b.leaders_by_country(),
        "{ctx}: leaders"
    );
    assert_eq!(
        a.occupation_by_state(),
        b.occupation_by_state(),
        "{ctx}: occupation"
    );
    assert_eq!(a.power_balance(), b.power_balance(), "{ctx}: power_balance");
    assert_eq!(a.designs(), b.designs(), "{ctx}: designs");
    assert_eq!(
        a.naval_air_combat(),
        b.naval_air_combat(),
        "{ctx}: naval_air_combat"
    );
    assert_eq!(a.scripted_gui(), b.scripted_gui(), "{ctx}: scripted_gui");
    assert_eq!(a.raids(), b.raids(), "{ctx}: raids");
}

#[test]
fn seek_equals_play_on_every_fixture() {
    for name in POSITIVE_FIXTURES {
        let t = open_fixture(name);
        let hours = t.hours().to_vec();

        for &h in &hours {
            let s = t.seek(h).unwrap();
            let p = t.play_to(h).unwrap();
            assert_state_equal(&s, &p, &format!("{name} fwd h{h}"));
        }
        for &h in hours.iter().rev() {
            let s = t.seek(h).unwrap();
            let p = t.play_to(h).unwrap();
            assert_state_equal(&s, &p, &format!("{name} bwd h{h}"));
        }
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


#[test]
fn absence_honesty_absent_vs_unavailable_vs_present() {
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

    assert_eq!(
        pf.visibility_for("USA").status,
        ViewStatus::DomainAbsent,
        "unlisted POV in a present domain -> absent"
    );
}


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


#[test]
fn adversarial_wrong_oracle_makes_gate_fail() {
    let owner = open_fixture("fx_owner_controller_flip");
    let f = owner.frame(owner.hours()[5]).unwrap();

    assert_eq!(f.owner_by_state()[&10], Leaf::Measured("SOV".into()));
    assert_ne!(f.owner_by_state()[&10], Leaf::Measured("GER".into()));

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

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        assert_eq!(f.owner_by_state()[&10], Leaf::<String>::Unobtained);
    }));
    std::panic::set_hook(prev);
    assert!(outcome.is_err(), "Measured must not equal Unobtained");
}
