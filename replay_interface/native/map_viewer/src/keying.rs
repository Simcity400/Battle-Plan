//! A5 — renderer keying source swap.
//!
//! The political fill, border, label, and map-furniture laws all key off three
//! things: **owner-by-state**, **controller-by-state**, and per-tag
//! **cosmetic-tag** (which drives the NAME1 color/name override). Historically
//! `map_viewer` obtained these by parsing the static 1936 `history/states` /
//! `history/countries` files (`game_data::load_states`,
//! `political_color_synthesis::load_initial_state_controls`,
//! `game_data::load_country_colors_from_game_resolver`). That is a single fixed
//! moment; a replay must key every rendered frame to the *recording's* state at
//! the scrubbed hour.
//!
//! [`KeyingTables`] is the neutral hand-off between the two sources. It is
//! produced either from the static files (the existing parse, unchanged) or
//! from a `replay_core::WorldFrame` via [`KeyingTables::from_world_frame`], and
//! is consumed by the `*_with_keying` law entry points
//! (`synthesize_political_colors_with_keying`,
//! `synthesize_border_world_mesh_with_keying`, `furniture_layer_with_keying`,
//! and the label law via [`KeyingTables::owner_map`]).
//!
//! Fail-closed honesty (Constraint 3 / trichotomy): only `Leaf::Measured`
//! values enter the tables. A state whose owner is `Leaf::Unobtained` /
//! `Leaf::Absent` is simply omitted, so downstream it renders the absent form
//! (`NoOwner` texel class, no label, no marked owner) — never a stale or
//! default tag.

use crate::InitialStateControl;
use replay_core::{Leaf, WorldFrame};
use std::collections::BTreeMap;

/// The keying a rendered political map needs at one moment, source-neutral.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyingTables {
    /// State id -> owner tag (only measured owners).
    pub state_owners: BTreeMap<u32, String>,
    /// State id -> {owner, controller} for the political-fill secondary-color
    /// (occupation) law. `owner` mirrors [`Self::state_owners`]; `controller`
    /// is the measured state controller when it differs / is present.
    pub state_controls: BTreeMap<u32, InitialStateControl>,
    /// Tag -> cosmetic tag (set_cosmetic_tag) for the NAME1 color/name
    /// override. Only tags with a measured cosmetic tag appear.
    pub cosmetic_by_tag: BTreeMap<String, String>,
}

impl KeyingTables {
    /// Build the keying for one hour from a scrubbable [`WorldFrame`], reading
    /// the typed `owner_by_state` / `control_by_state` / `country_identities`
    /// views. Only `Leaf::Measured` values are kept (fail-closed): an
    /// unobtained or absent owner/controller/cosmetic is omitted, never
    /// coerced to a default.
    pub fn from_world_frame(frame: &WorldFrame) -> KeyingTables {
        let mut state_owners = BTreeMap::new();
        let mut state_controls: BTreeMap<u32, InitialStateControl> = BTreeMap::new();

        for (state_id, leaf) in frame.owner_by_state() {
            if let Leaf::Measured(tag) = leaf {
                state_owners.insert(*state_id, tag.clone());
                state_controls
                    .entry(*state_id)
                    .or_insert_with(|| InitialStateControl {
                        owner: None,
                        controller: None,
                    })
                    .owner = Some(tag.clone());
            }
        }
        for (state_id, leaf) in frame.control_by_state() {
            if let Leaf::Measured(tag) = leaf {
                state_controls
                    .entry(*state_id)
                    .or_insert_with(|| InitialStateControl {
                        owner: None,
                        controller: None,
                    })
                    .controller = Some(tag.clone());
            }
        }

        let mut cosmetic_by_tag = BTreeMap::new();
        for (tag, identity) in frame.country_identities() {
            if let Leaf::Measured(cosmetic) = &identity.cosmetic_tag {
                cosmetic_by_tag.insert(tag.clone(), cosmetic.clone());
            }
        }

        KeyingTables {
            state_owners,
            state_controls,
            cosmetic_by_tag,
        }
    }

    /// The owner-by-state map the border / label / furniture laws consume.
    pub fn owner_map(&self) -> &BTreeMap<u32, String> {
        &self.state_owners
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::political_color_synthesis::{political_texel_colors, PoliticalColorTexelClass};
    use crate::types::{ProvinceDefinition, StateMeta};
    use crate::{compute_country_label_placements, compute_furniture_markers};
    use crate::{FurnitureLawInputs, LabelLawInputs, ProvinceKind};
    use replay_core::{open_recording, WorldTimeline};
    use serde_json::{json, Map, Value};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    // ------------------------------------------------------------------
    // Inline ReplaySource: lets a gate build an arbitrary owner/controller/
    // cosmetic snapshot (including explicit-null = Unobtained) and drive it
    // through the REAL replay_core WorldTimeline, exactly as a fixture would.
    // ------------------------------------------------------------------
    struct InlineSource {
        hours: Vec<i64>,
        frames: BTreeMap<i64, replay_core::RawFrame>,
        warnings: Vec<String>,
    }
    impl replay_core::ReplaySource for InlineSource {
        fn game_id(&self) -> &str {
            "inline"
        }
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn warnings(&self) -> &[String] {
            &self.warnings
        }
        fn frame(
            &self,
            hour_index: i64,
        ) -> Result<replay_core::RawFrame, replay_core::SourceError> {
            self.frames.get(&hour_index).cloned().ok_or_else(|| {
                replay_core::SourceError::Open(format!("no inline frame at {hour_index}"))
            })
        }
    }

    fn timeline_from_snapshots(snaps: Vec<Value>) -> WorldTimeline {
        let mut hours = Vec::new();
        let mut frames = BTreeMap::new();
        for snap in snaps {
            let hour = snap["hour_index"].as_i64().expect("hour_index");
            hours.push(hour);
            let rest: Map<String, Value> = snap.as_object().expect("obj").clone();
            frames.insert(
                hour,
                replay_core::RawFrame {
                    hour_index: hour,
                    date: rest.get("date").and_then(|v| v.as_str()).map(String::from),
                    save_version: None,
                    schema_version: None,
                    rest,
                },
            );
        }
        let source = InlineSource {
            hours,
            frames,
            warnings: Vec::new(),
        };
        WorldTimeline::open(Box::new(source)).expect("open inline timeline")
    }

    /// One minimal snapshot with the map layers the owner/control views read.
    fn snapshot(hour: i64, states: Value, provinces: Value, countries: Value) -> Value {
        // Build the map mirrors the way the recorder/fixture does.
        let mut owners = Map::new();
        for (sid, st) in states.as_object().unwrap() {
            owners.insert(sid.clone(), st["owner"].clone());
        }
        let mut controllers = Map::new();
        for (pid, pv) in provinces.as_object().unwrap() {
            controllers.insert(pid.clone(), pv["controller"].clone());
        }
        json!({
            "hour_index": hour,
            "date": "1936.1.1.0",
            "states": states,
            "provinces": provinces,
            "countries": countries,
            "map": { "owner_by_state": owners, "control_by_province": controllers },
        })
    }

    fn st(owner: &str, controller: &str) -> Value {
        json!({ "owner": owner, "controller": controller })
    }
    fn pv(controller: &str, state_id: i64) -> Value {
        json!({ "controller": controller, "state_id": state_id })
    }
    fn ctry_cosmetic(cosmetic: Value) -> Value {
        json!({ "misc": { "cosmetic_tag": cosmetic } })
    }

    // ==================================================================
    // GATE 1 (t0 equivalence): the timeline adapter yields exactly the
    // owner/controller/cosmetic tables the underlying snapshot encodes.
    // ==================================================================
    #[test]
    fn gate1_from_world_frame_table_equality() {
        let states = json!({ "10": st("GER", "GER"), "20": st("SOV", "SOV") });
        let provinces = json!({ "1000": pv("GER", 10), "2000": pv("SOV", 20) });
        let countries = json!({
            "GER": ctry_cosmetic(json!("GER_reichskommissariat")),
            "SOV": ctry_cosmetic(Value::Null), // captured-but-unobtained
        });
        let t = timeline_from_snapshots(vec![snapshot(0, states, provinces, countries)]);
        let frame = t.frame(0).expect("frame 0");
        let keying = KeyingTables::from_world_frame(&frame);

        let mut expected_owners = BTreeMap::new();
        expected_owners.insert(10u32, "GER".to_string());
        expected_owners.insert(20u32, "SOV".to_string());
        assert_eq!(keying.state_owners, expected_owners, "owner table");

        let mut expected_controls = BTreeMap::new();
        expected_controls.insert(
            10u32,
            InitialStateControl {
                owner: Some("GER".into()),
                controller: Some("GER".into()),
            },
        );
        expected_controls.insert(
            20u32,
            InitialStateControl {
                owner: Some("SOV".into()),
                controller: Some("SOV".into()),
            },
        );
        assert_eq!(keying.state_controls, expected_controls, "control table");

        // SOV cosmetic is explicit null (Unobtained) -> omitted (fail-closed).
        let mut expected_cosmetic = BTreeMap::new();
        expected_cosmetic.insert("GER".to_string(), "GER_reichskommissariat".to_string());
        assert_eq!(keying.cosmetic_by_tag, expected_cosmetic, "cosmetic table");
    }

    // A tiny 2x1 province raster / definition set shared by the fill gates so
    // political_texel_colors runs without any game file.
    fn land_def() -> ProvinceDefinition {
        ProvinceDefinition {
            rgb: [0, 0, 0],
            province_type: "land".to_string(),
            coastal: false,
            terrain: String::new(),
            continent: 0,
        }
    }
    fn fill_world() -> (
        BTreeMap<u32, ProvinceDefinition>,
        BTreeMap<u32, u32>,
        BTreeMap<u32, StateMeta>,
    ) {
        let defs = BTreeMap::from([(1000u32, land_def()), (2000u32, land_def())]);
        let p2s = BTreeMap::from([(1000u32, 10u32), (2000u32, 20u32)]);
        let states = BTreeMap::from([(10u32, StateMeta::default()), (20u32, StateMeta::default())]);
        (defs, p2s, states)
    }

    fn fill_colors(
        keying: &KeyingTables,
    ) -> BTreeMap<u32, (PoliticalColorTexelClass, Option<[u8; 4]>, Option<[u8; 4]>)> {
        let (defs, p2s, states) = fill_world();
        // Distinct base colors + a synthetic cosmetic color map so a cosmetic
        // flip is observable in the owner color.
        let mut colors = BTreeMap::from([
            ("GER".to_string(), [10u8, 20, 30]),
            ("SOV".to_string(), [40u8, 50, 60]),
        ]);
        let cosmetic_colors = BTreeMap::from([
            ("GER_reichskommissariat".to_string(), [200u8, 0, 0]),
            ("GER_civil_war_democratic".to_string(), [0u8, 200, 0]),
        ]);
        crate::game_data::apply_cosmetic_color_overrides(
            &mut colors,
            &keying.cosmetic_by_tag,
            &cosmetic_colors,
        );
        let mut out = BTreeMap::new();
        for province in [1000u32, 2000] {
            out.insert(
                province,
                political_texel_colors(
                    province,
                    &defs,
                    &p2s,
                    &states,
                    &keying.state_owners,
                    &keying.state_controls,
                    &colors,
                )
                .expect("texel colors"),
            );
        }
        out
    }

    // GATE 1 (rendered-output equality): two paths carrying the SAME
    // owner/controller/cosmetic data produce byte-identical fill output. Here
    // the "static" path is a KeyingTables built by hand and the "timeline"
    // path is one built from a WorldFrame that encodes the same data.
    #[test]
    fn gate1_fill_output_equivalence_same_data() {
        let states = json!({ "10": st("GER", "GER"), "20": st("SOV", "SOV") });
        let provinces = json!({ "1000": pv("GER", 10), "2000": pv("SOV", 20) });
        let countries = json!({ "GER": ctry_cosmetic(json!("GER_reichskommissariat")), "SOV": ctry_cosmetic(Value::Null) });
        let t = timeline_from_snapshots(vec![snapshot(0, states, provinces, countries)]);
        let timeline_keying = KeyingTables::from_world_frame(&t.frame(0).unwrap());

        let static_keying = KeyingTables {
            state_owners: BTreeMap::from([(10u32, "GER".to_string()), (20u32, "SOV".to_string())]),
            state_controls: BTreeMap::from([
                (
                    10u32,
                    InitialStateControl {
                        owner: Some("GER".into()),
                        controller: Some("GER".into()),
                    },
                ),
                (
                    20u32,
                    InitialStateControl {
                        owner: Some("SOV".into()),
                        controller: Some("SOV".into()),
                    },
                ),
            ]),
            cosmetic_by_tag: BTreeMap::from([(
                "GER".to_string(),
                "GER_reichskommissariat".to_string(),
            )]),
        };

        assert_eq!(
            fill_colors(&timeline_keying),
            fill_colors(&static_keying),
            "t0 fill output must match"
        );
    }

    // ==================================================================
    // GATE 2 (divergence): scrubbing a real flip fixture across the flip
    // hour changes EXACTLY the expected states/provinces/tags, nothing else.
    // Oracle: replay_fixture_gen/src/main.rs fx_owner_controller_flip /
    // fx_cosmetic_ideology mutate closures.
    // ==================================================================
    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/positive")
            .join(format!("{name}.battleplan"))
    }
    fn open_fixture(name: &str) -> WorldTimeline {
        WorldTimeline::open(open_recording(&fixture_path(name)).expect("open fixture"))
            .expect("timeline")
    }
    fn keying_at(t: &WorldTimeline, hour: i64) -> KeyingTables {
        KeyingTables::from_world_frame(&t.state_at(hour).expect("state_at"))
    }
    // Which state ids differ in owner between two keyings.
    fn owner_diff(a: &KeyingTables, b: &KeyingTables) -> BTreeSet<u32> {
        let mut d = BTreeSet::new();
        for k in a.state_owners.keys().chain(b.state_owners.keys()) {
            if a.state_owners.get(k) != b.state_owners.get(k) {
                d.insert(*k);
            }
        }
        d
    }
    fn control_diff(a: &KeyingTables, b: &KeyingTables) -> BTreeSet<u32> {
        let mut d = BTreeSet::new();
        for k in a.state_controls.keys().chain(b.state_controls.keys()) {
            if a.state_controls.get(k) != b.state_controls.get(k) {
                d.insert(*k);
            }
        }
        d
    }

    #[test]
    fn gate2_owner_controller_flip_divergence() {
        let t = open_fixture("fx_owner_controller_flip");
        let hours = t.hours().to_vec();
        // tick indices: base_offset shifts absolute hours, but scrubbing by
        // hour index is monotone in tick; use the recorded hour series.
        let h = |tick: usize| hours[tick];

        // Owner flip: state 10 GER->SOV at tick 5, and nothing else EVER.
        let before = keying_at(&t, h(4));
        let after = keying_at(&t, h(5));
        assert_eq!(
            owner_diff(&before, &after),
            BTreeSet::from([10]),
            "owner flip @ tick5 = {{10}}"
        );
        assert_eq!(after.state_owners.get(&10), Some(&"SOV".to_string()));
        // Controller flip rides the same hour for state 10.
        assert_eq!(
            control_diff(&before, &after),
            BTreeSet::from([10]),
            "control flip @ tick5 = {{10}}"
        );

        // Second flip: state 20 controller GER at tick 9 (owner UNCHANGED).
        let before9 = keying_at(&t, h(8));
        let after9 = keying_at(&t, h(9));
        assert_eq!(
            owner_diff(&before9, &after9),
            BTreeSet::new(),
            "no owner change @ tick9"
        );
        assert_eq!(
            control_diff(&before9, &after9),
            BTreeSet::from([20]),
            "control flip @ tick9 = {{20}}"
        );
        assert_eq!(
            after9.state_controls.get(&20),
            Some(&InitialStateControl {
                owner: Some("SOV".into()),
                controller: Some("GER".into())
            })
        );

        // Whole-timeline: the ONLY owner change across all hours is state 10.
        let mut all_owner_changes = BTreeSet::new();
        for w in hours.windows(2) {
            let a = keying_at(&t, w[0]);
            let b = keying_at(&t, w[1]);
            all_owner_changes.extend(owner_diff(&a, &b));
        }
        assert_eq!(
            all_owner_changes,
            BTreeSet::from([10]),
            "only state 10 ever changes owner"
        );
    }

    #[test]
    fn gate2_cosmetic_flip_divergence_reaches_fill() {
        let t = open_fixture("fx_cosmetic_ideology");
        let hours = t.hours().to_vec();
        let h = |tick: usize| hours[tick];

        // tick<4: GER cosmetic null (Unobtained) -> omitted.
        let k3 = keying_at(&t, h(3));
        assert_eq!(
            k3.cosmetic_by_tag.get("GER"),
            None,
            "GER cosmetic unobtained @ tick3"
        );
        // tick 4..8: GER_civil_war_democratic.
        let k4 = keying_at(&t, h(4));
        assert_eq!(
            k4.cosmetic_by_tag.get("GER").map(String::as_str),
            Some("GER_civil_war_democratic")
        );
        // tick>=8: GER_reichskommissariat.
        let k8 = keying_at(&t, h(8));
        assert_eq!(
            k8.cosmetic_by_tag.get("GER").map(String::as_str),
            Some("GER_reichskommissariat")
        );
        // No other tag ever carries a cosmetic in this fixture.
        for k in [&k3, &k4, &k8] {
            let tags: BTreeSet<&String> = k.cosmetic_by_tag.keys().collect();
            assert!(
                tags.iter().all(|t| *t == "GER"),
                "only GER has a cosmetic tag"
            );
        }

        // The cosmetic flip must reach the political FILL owner color. The
        // fixture GER owns state 10; feed a synthetic cosmetic color map and
        // assert the owner color changes exactly when the cosmetic changes.
        // (Owner table is stable in this fixture; we key state 10 to GER.)
        let ger_state = |k: &KeyingTables| -> KeyingTables {
            KeyingTables {
                state_owners: BTreeMap::from([
                    (10u32, "GER".to_string()),
                    (20u32, "SOV".to_string()),
                ]),
                state_controls: BTreeMap::from([
                    (
                        10u32,
                        InitialStateControl {
                            owner: Some("GER".into()),
                            controller: Some("GER".into()),
                        },
                    ),
                    (
                        20u32,
                        InitialStateControl {
                            owner: Some("SOV".into()),
                            controller: Some("SOV".into()),
                        },
                    ),
                ]),
                cosmetic_by_tag: k.cosmetic_by_tag.clone(),
            }
        };
        let fill3 = fill_colors(&ger_state(&k3)); // base GER color [10,20,30]
        let fill4 = fill_colors(&ger_state(&k4)); // cosmetic -> [0,200,0]
        let fill8 = fill_colors(&ger_state(&k8)); // cosmetic -> [200,0,0]
        let owner_color =
            |m: &BTreeMap<u32, (PoliticalColorTexelClass, Option<[u8; 4]>, Option<[u8; 4]>)>,
             p: u32| m[&p].1;
        assert_eq!(
            owner_color(&fill3, 1000),
            Some([10, 20, 30, 255]),
            "no cosmetic -> base color"
        );
        assert_eq!(
            owner_color(&fill4, 1000),
            Some([0, 200, 0, 255]),
            "civil_war cosmetic color"
        );
        assert_eq!(
            owner_color(&fill8, 1000),
            Some([200, 0, 0, 255]),
            "reichskommissariat cosmetic color"
        );
        // The SOV state (province 2000) never moves.
        assert_eq!(
            owner_color(&fill3, 2000),
            owner_color(&fill8, 2000),
            "SOV color stable"
        );
    }

    // Fill a solid rectangle [x0,x1) x [y0,y1) of a raster with a province id.
    fn fill_rect(ids: &mut [u32], w: usize, pid: u32, x0: usize, y0: usize, x1: usize, y1: usize) {
        for y in y0..y1 {
            for x in x0..x1 {
                ids[y * w + x] = pid;
            }
        }
    }

    // Divergence reaches labels + furniture (owner-keyed consumers): a 200x100
    // raster with province 1 (state 10, GER) and province 2 (state 20, SOV).
    // Flipping state 10's owner GER->SOV must drop GER's label and re-key its
    // marker, and leave state 20 untouched.
    #[test]
    fn gate2_owner_flip_reaches_labels_and_furniture() {
        let (w, h) = (200usize, 100usize);
        let mut province_ids = vec![0u32; w * h];
        fill_rect(&mut province_ids, w, 1, 20, 40, 90, 60); // province 1 -> state 10
        fill_rect(&mut province_ids, w, 2, 110, 40, 180, 60); // province 2 -> state 20
        let province_to_state = BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]);
        let importance = BTreeMap::new();
        let ptype = BTreeMap::from([(1u32, ProvinceKind::Land), (2u32, ProvinceKind::Land)]);
        let impassable = BTreeSet::new();

        let owners_before =
            BTreeMap::from([(10u32, "GER".to_string()), (20u32, "SOV".to_string())]);
        let owners_after = BTreeMap::from([(10u32, "SOV".to_string()), (20u32, "SOV".to_string())]);

        let label_tags = |owners: &BTreeMap<u32, String>| -> BTreeSet<String> {
            compute_country_label_placements(&LabelLawInputs {
                province_ids: &province_ids,
                width: w,
                height: h,
                province_to_state: &province_to_state,
                owners,
                impassable_states: &impassable,
                province_importance: &importance,
                province_type: &ptype,
            })
            .into_iter()
            .map(|p| p.tag)
            .collect()
        };
        let before = label_tags(&owners_before);
        let after = label_tags(&owners_after);
        assert!(
            before.contains("GER"),
            "GER labelled before flip: {before:?}"
        );
        assert!(
            !after.contains("GER"),
            "GER label gone after flip: {after:?}"
        );
        assert!(
            before.contains("SOV") && after.contains("SOV"),
            "SOV label stable"
        );

        // Furniture: give each state a VP province so a marker is emitted.
        let vps = BTreeMap::from([
            (10u32, BTreeMap::from([(1u32, 5i32)])),
            (20u32, BTreeMap::from([(2u32, 5i32)])),
        ]);
        let caps = BTreeMap::new();
        let loc = BTreeMap::new();
        let markers = |owners: &BTreeMap<u32, String>| -> BTreeMap<u32, String> {
            compute_furniture_markers(&FurnitureLawInputs {
                province_ids: &province_ids,
                width: w,
                height: h,
                province_to_state: &province_to_state,
                state_owners: owners,
                state_victory_points: &vps,
                capital_states: &caps,
                localisation: &loc,
                vp_icon_defines: None,
            })
            .into_iter()
            .map(|m| (m.province_id, m.owner))
            .collect()
        };
        let mb = markers(&owners_before);
        let ma = markers(&owners_after);
        assert_eq!(
            mb.get(&1).map(String::as_str),
            Some("GER"),
            "marker p1 owner GER before"
        );
        assert_eq!(
            ma.get(&1).map(String::as_str),
            Some("SOV"),
            "marker p1 owner SOV after"
        );
        assert_eq!(mb.get(&2), ma.get(&2), "state 20 marker unchanged");
    }

    // ==================================================================
    // GATE 3 (trichotomy honesty): an Unobtained owner renders the absent
    // form (NoOwner), never a stale/default color.
    // ==================================================================
    #[test]
    fn gate3_unobtained_owner_renders_absent_not_stale() {
        // State 10 owner is explicit null (captured-but-unobtained); state 20
        // is a normal measured owner.
        let states = json!({ "10": st_null_owner(), "20": st("SOV", "SOV") });
        let provinces = json!({ "1000": pv("SOV", 10), "2000": pv("SOV", 20) });
        let countries = json!({ "SOV": ctry_cosmetic(Value::Null) });
        let t = timeline_from_snapshots(vec![snapshot(0, states, provinces, countries)]);
        let keying = KeyingTables::from_world_frame(&t.frame(0).unwrap());

        // Fail-closed: state 10 omitted from the owner/control tables entirely.
        assert!(
            !keying.state_owners.contains_key(&10),
            "unobtained owner omitted"
        );
        assert!(
            keying
                .state_controls
                .get(&10)
                .map(|c| c.owner.is_none())
                .unwrap_or(true),
            "no invented owner for state 10"
        );

        // The fill law then classifies province 1000 (state 10) as NoOwner —
        // the absent form, not a stale/default color.
        let colors = fill_colors(&keying);
        assert_eq!(
            colors[&1000].0,
            PoliticalColorTexelClass::NoOwner,
            "absent form"
        );
        assert_eq!(colors[&1000].1, None, "no owner color emitted");
        // State 20 still renders normally.
        assert_eq!(
            colors[&2000].0,
            PoliticalColorTexelClass::OwnerControllerEqual
        );
    }

    fn st_null_owner() -> Value {
        json!({ "owner": Value::Null, "controller": Value::Null })
    }
}
