use replay_core::{open_recording, Leaf, WorldFrame, WorldTimeline};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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


fn unique_relocation_root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "replay_core_relocation_{}_{n}_{nanos}_{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root)
        .unwrap_or_else(|err| panic!("create relocation root {}: {err}", root.display()));
    root
}

fn remove_dir_all_with_retry(path: &Path) {
    let mut last = None;
    for attempt in 0..40u32 {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(err) => {
                last = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(
                    25 * u64::from((attempt + 1).min(8)),
                ));
            }
        }
    }
    panic!(
        "remove relocation dir {}: {}",
        path.display(),
        last.expect("a failure to report")
    );
}

fn write_with_retry(path: &Path, bytes: &[u8]) {
    let mut last = None;
    for attempt in 0..40u32 {
        match std::fs::write(path, bytes) {
            Ok(()) => return,
            Err(err) => {
                last = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(
                    25 * u64::from((attempt + 1).min(8)),
                ));
            }
        }
    }
    panic!(
        "write {}: {}",
        path.display(),
        last.expect("a failure to report")
    );
}

fn entry_count(dir: &Path) -> usize {
    let mut last = None;
    for attempt in 0..40u32 {
        match std::fs::read_dir(dir) {
            Ok(entries) => return entries.count(),
            Err(err) => {
                last = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(
                    25 * u64::from((attempt + 1).min(8)),
                ));
            }
        }
    }
    panic!(
        "read {}: {}",
        dir.display(),
        last.expect("a failure to report")
    );
}

fn place_alone(root: &Path, sub: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let dir = root.join(sub);
    std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("create {}: {err}", dir.display()));
    let path = dir.join(name);
    write_with_retry(&path, bytes);
    assert_eq!(
        entry_count(&dir),
        1,
        "{} must hold the relocated recording and nothing else",
        dir.display()
    );
    path
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= 240 {
        return value.to_string();
    }
    let head: String = value.chars().take(240).collect();
    format!("{head}...<{} chars>", value.chars().count())
}

#[derive(Debug)]
struct RawFrameReading {
    hour_index: i64,
    date: Option<String>,
    save_version: Option<String>,
    schema_version: Option<Value>,
    rest: Map<String, Value>,
}

#[derive(Debug)]
struct RecordingReading {
    game_id: String,
    hours: Vec<i64>,
    warnings: Vec<String>,
    declared_coverage: Option<String>,
    content_manifest: Option<String>,
    campaign_identity: Option<String>,
    product_admission: Option<String>,
    prefold_receipt: Option<String>,
    design_catalog: Option<String>,
    decision_catalog: Option<String>,
    dates: Vec<(i64, Option<String>)>,
    frames: Vec<RawFrameReading>,
    timeline_game_id: String,
    timeline_hours: Vec<i64>,
    timeline_warnings: Vec<String>,
    timeline_metadata: String,
    admission: String,
    timeline_per_hour: Vec<(i64, String)>,
    timeline_domains: Vec<(i64, String, Option<Value>)>,
    coverage_warnings: Vec<(i64, Vec<String>)>,
    typed_views: Vec<(i64, String)>,
}

impl RecordingReading {
    fn differences(&self, other: &RecordingReading) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut scalar = |field: &str, a: String, b: String| {
            if a != b {
                out.push(format!("{field}: {} != {}", truncate(&a), truncate(&b)));
            }
        };
        scalar("game_id", self.game_id.clone(), other.game_id.clone());
        scalar(
            "hours",
            format!("{:?}", self.hours),
            format!("{:?}", other.hours),
        );
        scalar(
            "warnings",
            format!("{:?}", self.warnings),
            format!("{:?}", other.warnings),
        );
        scalar(
            "declared_coverage",
            format!("{:?}", self.declared_coverage),
            format!("{:?}", other.declared_coverage),
        );
        scalar(
            "content_manifest",
            format!("{:?}", self.content_manifest),
            format!("{:?}", other.content_manifest),
        );
        scalar(
            "campaign_identity",
            format!("{:?}", self.campaign_identity),
            format!("{:?}", other.campaign_identity),
        );
        scalar(
            "product_admission",
            format!("{:?}", self.product_admission),
            format!("{:?}", other.product_admission),
        );
        scalar(
            "prefold_receipt",
            format!("{:?}", self.prefold_receipt),
            format!("{:?}", other.prefold_receipt),
        );
        scalar(
            "design_catalog",
            format!("{:?}", self.design_catalog),
            format!("{:?}", other.design_catalog),
        );
        scalar(
            "decision_catalog",
            format!("{:?}", self.decision_catalog),
            format!("{:?}", other.decision_catalog),
        );
        scalar(
            "dates",
            format!("{:?}", self.dates),
            format!("{:?}", other.dates),
        );
        scalar(
            "timeline_game_id",
            self.timeline_game_id.clone(),
            other.timeline_game_id.clone(),
        );
        scalar(
            "timeline_hours",
            format!("{:?}", self.timeline_hours),
            format!("{:?}", other.timeline_hours),
        );
        scalar(
            "timeline_warnings",
            format!("{:?}", self.timeline_warnings),
            format!("{:?}", other.timeline_warnings),
        );
        scalar(
            "timeline_metadata",
            self.timeline_metadata.clone(),
            other.timeline_metadata.clone(),
        );
        scalar("admission", self.admission.clone(), other.admission.clone());
        scalar(
            "timeline_per_hour",
            format!("{:?}", self.timeline_per_hour),
            format!("{:?}", other.timeline_per_hour),
        );
        scalar(
            "coverage_warnings",
            format!("{:?}", self.coverage_warnings),
            format!("{:?}", other.coverage_warnings),
        );

        if self.frames.len() != other.frames.len() {
            out.push(format!(
                "frame count: {} != {}",
                self.frames.len(),
                other.frames.len()
            ));
        }
        for (a, b) in self.frames.iter().zip(other.frames.iter()) {
            let hour = a.hour_index;
            if a.hour_index != b.hour_index {
                out.push(format!(
                    "frame hour_index: {} != {}",
                    a.hour_index, b.hour_index
                ));
            }
            if a.date != b.date {
                out.push(format!("hour {hour} date: {:?} != {:?}", a.date, b.date));
            }
            if a.save_version != b.save_version {
                out.push(format!(
                    "hour {hour} save_version: {:?} != {:?}",
                    a.save_version, b.save_version
                ));
            }
            if a.schema_version != b.schema_version {
                out.push(format!(
                    "hour {hour} schema_version: {:?} != {:?}",
                    a.schema_version, b.schema_version
                ));
            }
            let keys: BTreeSet<&String> = a.rest.keys().chain(b.rest.keys()).collect();
            for key in keys {
                if a.rest.get(key) != b.rest.get(key) {
                    out.push(format!(
                        "hour {hour} raw domain {key:?}: {} != {}",
                        truncate(&format!("{:?}", a.rest.get(key))),
                        truncate(&format!("{:?}", b.rest.get(key)))
                    ));
                }
            }
        }

        if self.timeline_domains.len() != other.timeline_domains.len() {
            out.push(format!(
                "timeline domain count: {} != {}",
                self.timeline_domains.len(),
                other.timeline_domains.len()
            ));
        }
        for (a, b) in self
            .timeline_domains
            .iter()
            .zip(other.timeline_domains.iter())
        {
            if a != b {
                out.push(format!(
                    "timeline hour {} domain {:?} (other: hour {} domain {:?}): {} != {}",
                    a.0,
                    a.1,
                    b.0,
                    b.1,
                    truncate(&format!("{:?}", a.2)),
                    truncate(&format!("{:?}", b.2))
                ));
            }
        }

        if self.typed_views.len() != other.typed_views.len() {
            out.push(format!(
                "typed view count: {} != {}",
                self.typed_views.len(),
                other.typed_views.len()
            ));
        }
        for (a, b) in self.typed_views.iter().zip(other.typed_views.iter()) {
            if a != b {
                out.push(format!(
                    "typed views at hour {} (other: hour {}): {} != {}",
                    a.0,
                    b.0,
                    truncate(&a.1),
                    truncate(&b.1)
                ));
            }
        }
        out
    }
}

const COMPARISON_FAMILIES: &[(&str, &[&str])] = &[
    ("game id", &["game_id:", "timeline_game_id:"]),
    (
        "raw frame bodies",
        &["hour ", "frame hour_index:", "frame count:"],
    ),
    (
        "timeline domains",
        &["timeline hour ", "timeline domain count:"],
    ),
    (
        "typed views",
        &["typed views at hour ", "typed view count:"],
    ),
];

fn relocation_probe_inventory() -> replay_core::PlaybackContentInventory {
    replay_core::PlaybackContentInventory {
        source_game: replay_core::SourceGameIdentity {
            version: "km-relocation-probe".into(),
            build: "km-relocation-probe".into(),
            executable_sha256: "km-relocation-probe".into(),
            build_profile: "km-relocation-probe".into(),
        },
        source_profile: replay_core::SourceProfileIdentity {
            profile_id: "km-relocation-probe".into(),
            playset_id: "km-relocation-probe".into(),
            selection_sha256: "km-relocation-probe".into(),
        },
        layers: Vec::new(),
        dlc_selection: Vec::new(),
        localisation_language: "km-relocation-probe".into(),
        settings: Default::default(),
        resolved_content: Vec::new(),
    }
}

fn typed_views_debug(frame: &WorldFrame) -> String {
    format!(
        "hour_index={:?} date={:?} save_version={:?} owner_by_state={:?}/{:?}/{:?} \
         control_by_province={:?}/{:?}/{:?} divisions={:?}",
        frame.hour_index(),
        frame.date(),
        frame.save_version(),
        frame.owner_by_state(),
        frame.owner_by_state_source(),
        frame.owner_by_state_status(),
        frame.control_by_province(),
        frame.control_by_province_source(),
        frame.control_by_province_status(),
        frame.divisions(),
    )
}

fn read_recording(path: &Path) -> Result<RecordingReading, String> {
    let source = open_recording(path).map_err(|err| err.to_string())?;
    let game_id = source.game_id().to_string();
    let hours = source.hours().to_vec();
    let warnings = source.warnings().to_vec();
    let declared_coverage = source.declared_coverage().map(|v| format!("{v:?}"));
    let content_manifest = source.content_manifest().map(|v| format!("{v:?}"));
    let campaign_identity = source.campaign_identity().map(|v| format!("{v:?}"));
    let product_admission = source.product_admission().map(|v| format!("{v:?}"));
    let prefold_receipt = source.prefold_receipt().map(|v| format!("{v:?}"));
    let design_catalog = source.design_catalog().map(|v| format!("{v:?}"));
    let decision_catalog = source.decision_catalog().map(|v| format!("{v:?}"));

    let mut dates = Vec::new();
    let mut frames = Vec::new();
    for &hour in &hours {
        dates.push((hour, source.date_at(hour).map_err(|err| err.to_string())?));
        let raw = source.frame(hour).map_err(|err| err.to_string())?;
        frames.push(RawFrameReading {
            hour_index: raw.hour_index,
            date: raw.date,
            save_version: raw.save_version,
            schema_version: raw.schema_version,
            rest: raw.rest,
        });
    }

    let timeline = WorldTimeline::open(source).map_err(|err| err.to_string())?;
    let timeline_game_id = timeline.game_id().to_string();
    let timeline_hours = timeline.hours().to_vec();
    let timeline_warnings = timeline.warnings().to_vec();
    let timeline_metadata = format!(
        "first_hour={:?} last_hour={:?} declared_coverage={:?} content_manifest={:?} \
         campaign_identity={:?} product_admission={:?} prefold_receipt={:?} \
         design_catalog={:?} decision_catalog={:?} frame_cache_entries={:?}",
        timeline.first_hour(),
        timeline.last_hour(),
        timeline.declared_coverage(),
        timeline.content_manifest(),
        timeline.campaign_identity(),
        timeline.product_admission(),
        timeline.prefold_receipt(),
        timeline.design_catalog(),
        timeline.decision_catalog(),
        timeline.frame_cache_entries(),
    );
    let admission = format!(
        "{:?}",
        timeline.admit_playback_content(&relocation_probe_inventory())
    );
    let mut timeline_domains = Vec::new();
    let mut timeline_per_hour = Vec::new();
    let mut coverage_warnings = Vec::new();
    let mut typed_views = Vec::new();
    for &hour in &timeline_hours {
        let routes = [
            (
                "frame",
                timeline.frame(hour).map_err(|err| err.to_string())?,
            ),
            (
                "state_at",
                timeline.state_at(hour).map_err(|err| err.to_string())?,
            ),
            ("seek", timeline.seek(hour).map_err(|err| err.to_string())?),
            (
                "play_to",
                timeline.play_to(hour).map_err(|err| err.to_string())?,
            ),
        ];
        for (label, resolved) in &routes {
            for (key, value) in resolved.raw_for_consumer_tests() {
                timeline_domains.push((hour, format!("{label}.{key}"), Some(value.clone())));
            }
            typed_views.push((hour, format!("{label} {}", typed_views_debug(resolved))));
        }
        let frame = &routes[0].1;
        coverage_warnings.push((hour, frame.coverage_warnings().to_vec()));
        timeline_per_hour.push((
            hour,
            format!(
                "date_at={:?} lookup_hour={:?} declared_gap_at={:?}",
                timeline.date_at(hour).map_err(|err| err.to_string())?,
                timeline.lookup_hour(hour).map_err(|err| err.to_string())?,
                timeline.declared_gap_at(hour),
            ),
        ));
    }

    Ok(RecordingReading {
        game_id,
        hours,
        warnings,
        declared_coverage,
        content_manifest,
        campaign_identity,
        product_admission,
        prefold_receipt,
        design_catalog,
        decision_catalog,
        dates,
        frames,
        timeline_game_id,
        timeline_hours,
        timeline_warnings,
        timeline_metadata,
        admission,
        timeline_per_hour,
        timeline_domains,
        coverage_warnings,
        typed_views,
    })
}

#[test]
fn relocating_a_recording_changes_nothing_the_reader_answers() {
    let root = fixture_root();
    let manifest = fixture_manifest();
    let positive = manifest["positive"]
        .as_array()
        .expect("positive fixture list");
    assert_eq!(positive.len(), 19, "the shipped positive corpus");

    let relocation_root = unique_relocation_root("positive");
    let locations: [&str; 2] = ["plain", "with spaces and unicode ünïcödé 日本語"];

    let outcome = std::panic::catch_unwind(|| {
        let mut previous: Option<(&str, RecordingReading)> = None;
        let mut families_that_fired: BTreeSet<&str> = BTreeSet::new();

        for entry in positive {
            let name = entry["name"].as_str().expect("fixture name");
            let rel = entry["path"].as_str().expect("fixture path");
            let canonical_path = root.join(rel);
            let canonical = read_recording(&canonical_path)
                .unwrap_or_else(|err| panic!("{name}: open canonical fixture: {err}"));

            assert!(!canonical.frames.is_empty(), "{name}: no frames read");
            assert!(
                canonical
                    .frames
                    .iter()
                    .all(|frame| frame.rest.contains_key("countries")
                        && frame.rest.contains_key("states")),
                "{name}: a frame decoded without the countries/states domains"
            );
            assert!(
                canonical
                    .timeline_domains
                    .iter()
                    .filter(|(_, _, value)| value.is_some())
                    .count()
                    >= canonical.frames.len(),
                "{name}: the timeline answered fewer domains than it has frames"
            );
            assert!(
                !canonical.typed_views.is_empty(),
                "{name}: no typed views read"
            );
            assert_eq!(canonical.game_id, name, "{name}: canonical game id");

            if let Some((previous_name, previous)) = &previous {
                let cross = previous.differences(&canonical);
                assert!(
                    !cross.is_empty(),
                    "the comparator called two different recordings ({previous_name} and {name}) \
                     identical; it cannot report a difference, so its verdicts below mean nothing"
                );
                for (family, prefixes) in COMPARISON_FAMILIES {
                    if cross
                        .iter()
                        .any(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)))
                    {
                        families_that_fired.insert(family);
                    }
                }
            }

            let bytes = std::fs::read(&canonical_path)
                .unwrap_or_else(|err| panic!("{name}: read fixture bytes: {err}"));

            for location in locations {
                let moved = place_alone(
                    &relocation_root,
                    &format!("{location}/{name}"),
                    &format!("relocated-{name}.battleplan"),
                    &bytes,
                );
                let relocated = read_recording(&moved)
                    .unwrap_or_else(|err| panic!("{name} at {location:?}: open relocated: {err}"));
                let parent = moved.parent().expect("the relocated copy has a parent");
                assert_eq!(
                    entry_count(parent),
                    1,
                    "{name} at {location:?}: reading the recording left something beside it in {}",
                    parent.display()
                );
                let differences = canonical.differences(&relocated);
                assert!(
                    differences.is_empty(),
                    "{name}: relocating to {} changed the reading:\n  {}",
                    moved.display(),
                    differences.join("\n  ")
                );
                assert_eq!(
                    relocated.game_id, name,
                    "{name}: the game id after relocation must still be the recorded one"
                );
                assert_eq!(relocated.timeline_game_id, name, "{name}: timeline game id");
            }

            previous = Some((name, canonical));
        }

        let expected: BTreeSet<&str> = COMPARISON_FAMILIES.iter().map(|(name, _)| *name).collect();
        let silent: Vec<&&str> = expected.difference(&families_that_fired).collect();
        assert!(
            silent.is_empty(),
            "these selected arms of `RecordingReading::differences` never reported a difference \
             anywhere in the corpus, so nothing shows they can fail: {silent:?}"
        );
    });
    remove_dir_all_with_retry(&relocation_root);
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn a_refused_recording_is_refused_the_same_way_from_any_path() {
    let root = fixture_root();
    let manifest = fixture_manifest();
    let negative = manifest["negative"]
        .as_array()
        .expect("negative fixture list");
    assert_eq!(negative.len(), 4);

    let relocation_root = unique_relocation_root("negative");
    let outcome = std::panic::catch_unwind(|| {
        for entry in negative {
            let name = entry["name"].as_str().expect("negative name");
            let rel = entry["path"].as_str().expect("negative path");
            let expected = if name == "neg_empty_snapshot_index" {
                "empty snapshot index"
            } else {
                entry["expected_error_contains"]
                    .as_str()
                    .expect("expected error substring")
            };
            let canonical_path = root.join(rel);
            let canonical = match read_recording(&canonical_path) {
                Ok(_) => panic!("{name}: negative fixture unexpectedly opened"),
                Err(err) => err,
            };
            assert!(
                canonical.contains(expected),
                "{name}: expected the refusal to name {expected:?}, got {canonical:?}"
            );

            let bytes = std::fs::read(&canonical_path)
                .unwrap_or_else(|err| panic!("{name}: read negative fixture: {err}"));
            let marker = "kmRelocatedRefusalMarker";
            let moved = place_alone(
                &relocation_root,
                &format!("{marker}/{name}"),
                &format!("{marker}-{name}.battleplan"),
                &bytes,
            );
            let relocated = match read_recording(&moved) {
                Ok(_) => panic!(
                    "{name}: the same bytes opened at {} after being refused at {}",
                    moved.display(),
                    canonical_path.display()
                ),
                Err(err) => err,
            };
            assert_eq!(
                canonical, relocated,
                "{name}: the refusal changed when the recording moved"
            );
            for must_not_appear in [
                marker.to_string(),
                moved.display().to_string(),
                canonical_path.display().to_string(),
            ] {
                assert!(
                    !relocated.contains(&must_not_appear),
                    "{name}: the refusal names {must_not_appear:?} — it is reporting a path, \
                     not the bytes: {relocated:?}"
                );
            }
            for message in [&canonical, &relocated] {
                assert!(
                    !message.contains('\\'),
                    "{name}: the refusal carries a Windows path separator, so it is reporting \
                     some part of a pathname: {message:?}"
                );
            }
        }
    });
    remove_dir_all_with_retry(&relocation_root);
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

#[test]
fn the_reader_follows_the_bytes_at_a_fixed_path() {
    let root = fixture_root();
    let pristine = std::fs::read(root.join("positive/fx_topbar_scalars_sweep.battleplan"))
        .expect("read fx_topbar_scalars_sweep");
    assert_eq!(
        &pristine[..4],
        b"PK\x03\x04",
        "the fixture is not a ZIP local file header; re-derive the corruption"
    );
    let mut corrupted = pristine.clone();
    corrupted[0] ^= 0xFF;
    assert_ne!(pristine, corrupted);
    assert_eq!(pristine.len(), corrupted.len());

    let relocation_root = unique_relocation_root("bytes");
    let outcome = std::panic::catch_unwind(|| {
        let dir = relocation_root.join("fixed");
        std::fs::create_dir_all(&dir).expect("create fixed dir");
        let path = dir.join("recording.battleplan");

        write_with_retry(&path, &pristine);
        let first = read_recording(&path).expect("the pristine copy must open");
        assert_eq!(first.game_id, "fx_topbar_scalars_sweep");
        assert!(!first.frames.is_empty());

        write_with_retry(&path, &corrupted);
        let refusal = match read_recording(&path) {
            Ok(_) => panic!(
                "the corrupted copy opened at the same path the pristine copy did; \
                 the reader is not reading the bytes"
            ),
            Err(err) => err,
        };
        assert!(!refusal.is_empty(), "the refusal must say something");

        write_with_retry(&path, &pristine);
        let restored = read_recording(&path).expect("the restored copy must open");
        let differences = first.differences(&restored);
        assert!(
            differences.is_empty(),
            "restoring the bytes at the same path did not restore the reading:\n  {}",
            differences.join("\n  ")
        );

        let elsewhere = place_alone(
            &relocation_root,
            "elsewhere/deeper",
            "some other name.battleplan",
            &corrupted,
        );
        let elsewhere_refusal = match read_recording(&elsewhere) {
            Ok(_) => panic!("the corrupted copy opened at {}", elsewhere.display()),
            Err(err) => err,
        };
        assert_eq!(
            refusal, elsewhere_refusal,
            "the same corrupt bytes were refused differently from a different path"
        );
    });
    remove_dir_all_with_retry(&relocation_root);
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}
