use replay_core::{open_recording, Leaf, WorldFrame, WorldTimeline};
use serde_json::Value;
use std::path::{Path, PathBuf};

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

fn open_fixture(name: &str) -> WorldTimeline {
    let root = fixture_root();
    let manifest = fixture_manifest();
    let positive = manifest["positive"]
        .as_array()
        .expect("positive fixture list");
    let entry = positive
        .iter()
        .find(|entry| entry["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("fixture {name:?} not listed"));
    let rel = entry["path"].as_str().expect("fixture path");
    let source = open_recording(&root.join(rel))
        .unwrap_or_else(|err| panic!("open fixture {name} through ReplaySource: {err}"));
    WorldTimeline::open(source).unwrap_or_else(|err| panic!("timeline fixture {name}: {err}"))
}

fn frame_probe<'a>(frame: &'a WorldFrame, path: &[Value]) -> Option<&'a Value> {
    let first = path.first()?.as_str()?;
    let mut cur = frame.domain(first)?;
    for seg in &path[1..] {
        let key = seg.as_str()?;
        match cur {
            Value::Object(map) => cur = map.get(key)?,
            Value::Array(items) => cur = items.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        }
    }
    Some(cur)
}

#[test]
fn positive_fixtures_load_and_match_manifest_probes() {
    let root = fixture_root();
    let manifest = fixture_manifest();
    assert_eq!(
        manifest["format"].as_str(),
        Some("hoi4replay_fixture_manifest")
    );
    let positive = manifest["positive"]
        .as_array()
        .expect("positive fixture list");
    assert_eq!(
        positive.len(),
        19,
        "12 A2 fixtures + 7 fx-streams data-stream fixtures"
    );

    for entry in positive {
        let name = entry["name"].as_str().expect("fixture name");
        let rel = entry["path"].as_str().expect("fixture path");
        let path = root.join(rel);
        assert!(path.is_file(), "fixture file missing: {}", path.display());

        let source = open_recording(&path)
            .unwrap_or_else(|err| panic!("{name}: open_recording failed: {err}"));
        assert_eq!(source.game_id(), name);
        assert!(source.warnings().is_empty(), "{name}: source warnings");
        let timeline = WorldTimeline::open(source)
            .unwrap_or_else(|err| panic!("{name}: WorldTimeline::open failed: {err}"));

        let expected_hours = entry["hours"]
            .as_array()
            .expect("fixture hours")
            .iter()
            .map(|h| h.as_i64().expect("integer hour"))
            .collect::<Vec<_>>();
        assert_eq!(timeline.hours(), expected_hours.as_slice(), "{name}: hours");
        assert_eq!(
            timeline.hours().len(),
            entry["snapshot_count"].as_u64().unwrap() as usize,
            "{name}: snapshot count"
        );

        let first = timeline.frame(timeline.first_hour()).unwrap();
        assert!(first.domain("countries").is_some(), "{name}: countries");
        assert!(first.domain("states").is_some(), "{name}: states");
        assert!(first.domain("provinces").is_some(), "{name}: provinces");
        assert!(first.domain("map").is_some(), "{name}: map");

        for probe in entry["probes"].as_array().expect("fixture probes") {
            let hour = probe["hour"].as_i64().expect("probe hour");
            let frame = timeline
                .frame(hour)
                .unwrap_or_else(|err| panic!("{name}: probe hour {hour}: {err}"));
            let path = probe["path"].as_array().expect("probe path");
            let got = frame_probe(&frame, path)
                .unwrap_or_else(|| panic!("{name}: missing probe path {:?}", probe["path"]));
            assert_eq!(
                got,
                &probe["equals"],
                "{name}: probe failed ({})",
                probe["note"].as_str().unwrap_or("<no note>")
            );
        }
    }
}

#[test]
fn selected_positive_fixtures_feed_current_typed_views() {
    let owner = open_fixture("fx_owner_controller_flip");
    let before = owner.frame(owner.hours()[0]).unwrap();
    let after = owner.frame(owner.hours()[5]).unwrap();
    assert_eq!(before.owner_by_state()[&10], Leaf::Measured("GER".into()));
    assert_eq!(after.owner_by_state()[&10], Leaf::Measured("SOV".into()));
    assert_eq!(
        after.control_by_province()[&1000],
        Leaf::Measured("SOV".into())
    );

    let ideology = open_fixture("fx_cosmetic_ideology");
    let changed = ideology.frame(ideology.hours()[4]).unwrap();
    assert_eq!(
        changed.country("GER").unwrap().ruling_party,
        Leaf::Measured("democratic".into())
    );

    let topbar = open_fixture("fx_topbar_scalars_sweep");
    let topbar_end = topbar.frame(topbar.last_hour()).unwrap();
    let ger = topbar_end.country("GER").unwrap();
    assert_eq!(ger.political_power, Leaf::Measured(33.0));
    assert_eq!(ger.fuel_current, Leaf::Measured(775.0));
    assert_eq!(ger.division_count, Leaf::Measured(1));

    let movement = open_fixture("fx_division_movement");
    let moved = movement.frame(movement.hours()[8]).unwrap();
    let div = moved
        .divisions()
        .iter()
        .find(|div| div.div_uid == "101:1")
        .expect("division 101:1");
    assert_eq!(div.location_province, Leaf::Measured(2000));
    assert_eq!(div.in_combat, Leaf::Measured(true));
}

#[test]
fn negative_fixtures_are_rejected_with_precise_errors() {
    let root = fixture_root();
    let manifest = fixture_manifest();
    let negative = manifest["negative"]
        .as_array()
        .expect("negative fixture list");
    assert_eq!(negative.len(), 4);

    for entry in negative {
        let name = entry["name"].as_str().expect("negative name");
        let rel = entry["path"].as_str().expect("negative path");
        // The manifest records the shipped product-open expectation. The
        // capability-free replay_core entry point intentionally refuses a
        // compact-only package before inspecting the decoder-only schema.
        let expected = if name == "neg_empty_snapshot_index" {
            "empty snapshot index"
        } else {
            entry["expected_error_contains"]
                .as_str()
                .expect("expected error substring")
        };
        let path = root.join(rel);
        assert!(Path::new(&path).is_file(), "{name}: fixture missing");

        let err = match open_recording(&path) {
            Ok(_) => panic!("{name}: negative fixture unexpectedly opened"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(
            msg.contains(expected),
            "{name}: expected error containing {expected:?}, got {msg:?}"
        );
    }
}
