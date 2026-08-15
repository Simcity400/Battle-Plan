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

// ---------------------------------------------------------------------------
// Recording relocation, at the READER boundary.
//
// A `.battleplan` is the sole runtime interface between the extractor and the
// replayer (SINGLE-FILE LAW), so the bytes are the whole recording: where the
// file sits, and what it is called, must change nothing about what the reader
// answers. Nothing else in the workspace states that at this boundary.
// `replay_fixture_gen`'s `generated_fixture_set_is_byte_identical_after_relocation`
// proves the GENERATOR is relocation-stable (it regenerates from two roots and
// compares the produced bytes); it never reopens a produced file from a second
// path. `map_viewer`'s `relocated_replay_and_install_produce_identical_plan` and
// `game_ui`'s `portable_manifest_rebinds_after_game_profile_and_replay_relocation`
// relocate the INSTALL / content roots, not the recording.
// ---------------------------------------------------------------------------

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

/// Windows denies a delete/rename handle to a `.battleplan` for as long as the
/// container is open (`open_retained_container_file`), and a virus scanner that
/// is still reading a freshly written file produces the same `os error 32`.
/// Every reader in these tests is dropped before any removal; this only absorbs
/// the scanner window.
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

/// Write `bytes` to `path`, absorbing the same transient `os error 32` window
/// `remove_dir_all_with_retry` does: a scanner that opened the file this test
/// wrote a moment ago denies the next writer for as long as it holds it. This is
/// environment noise, not reader behaviour, and must not be able to fail a test
/// about relocation.
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

/// Count the entries of `dir`, absorbing the same transient window.
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

/// Place `bytes` at `<root>/<sub>/<name>`, in a directory of its own that holds
/// nothing else. A reader that REQUIRED anything beside the recording — a
/// sidecar index, a neighbouring `manifest.json`, a cache file — would find
/// nothing to read here; the shipped corpus directory it was copied from holds
/// twenty recordings and a `manifest.json`.
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

/// A SAMPLE of what the reader answers about one recording, in a form that
/// carries no path of its own. Exhaustive over the decoded content — the whole
/// raw body of every hour, and the `WorldTimeline` read-back of every top-level
/// domain in it — but NOT over the reader's whole surface: see the "does NOT
/// kill" list on `relocating_a_recording_changes_nothing_the_reader_answers`
/// for what is left out and why. Types without `PartialEq` are captured through
/// their derived `Debug`, which prints every field, so a path that reached one
/// of the sampled ones shows up as a difference.
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
    g2_engine_capture: Option<String>,
    dates: Vec<(i64, Option<String>)>,
    frames: Vec<RawFrameReading>,
    timeline_game_id: String,
    timeline_hours: Vec<i64>,
    timeline_warnings: Vec<String>,
    /// The metadata `WorldTimeline` re-exposes from its source, read through the
    /// timeline's own accessors rather than assumed to be pass-throughs.
    timeline_metadata: String,
    /// `WorldTimeline::admit_playback_content` against a fixed probe inventory.
    admission: String,
    /// `(hour, Debug of the timeline's own per-hour accessors)` at each RECORDED
    /// hour.
    timeline_per_hour: Vec<(i64, String)>,
    /// `(hour, "<route>.<domain>", value)` for every top-level domain of the
    /// frame each of the four routes to that hour resolves to, taken from that
    /// frame's OWN `WorldFrame::raw` map. Not a shared key set: a route that
    /// invented a domain, or dropped one, changes this list's length and its
    /// entries.
    timeline_domains: Vec<(i64, String, Option<Value>)>,
    /// `(hour, WorldFrame::coverage_warnings)` — the coverage OUTCOME, which
    /// the deleted renderer test reached only indirectly, through pixels.
    coverage_warnings: Vec<(i64, Vec<String>)>,
    /// `(hour, Debug of the typed views `typed_views_debug` names)` — a
    /// spot-check that the DERIVED layer is reached at all, not a survey of it.
    /// `WorldFrame` has many more typed accessors and no way to enumerate them;
    /// the exhaustive statement here is the raw/timeline-domain comparison the
    /// derived layer is computed from.
    typed_views: Vec<(i64, String)>,
}

impl RecordingReading {
    /// Short, located descriptions of every disagreement. Returning the list
    /// (rather than comparing two whole readings with `assert_eq!`) keeps a
    /// failure legible: these frames are tens of thousands of JSON nodes each.
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
            "g2_engine_capture",
            format!("{:?}", self.g2_engine_capture),
            format!("{:?}", other.g2_engine_capture),
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

/// FOUR SELECTED arms of [`RecordingReading::differences`], by the prefixes each
/// writes. Each of these four must be observed reporting a difference somewhere
/// in the corpus; see `relocating_a_recording_changes_nothing_the_reader_answers`.
///
/// This is a sample, not a census: the remaining scalar arms (warnings,
/// declared coverage, the manifests, the catalogs, `dates`, `hours`,
/// `coverage_warnings`) have no self-check, because two shipped fixtures do not
/// reliably differ in each of them and this table only records arms that a real
/// pair can demonstrate. Deleting one of those scalar comparisons would not be
/// caught here.
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

/// A fixed, deliberately non-matching playback inventory for
/// `WorldTimeline::admit_playback_content`. It is not a claim about any real
/// install: it is a constant probe, so the report that comes back is derived
/// entirely from the RECORDING's own content manifest — which is the thing that
/// must not change when the recording moves.
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

/// Open `path` and read back the sampled surfaces `RecordingReading` documents
/// — every accessor `ReplaySource` and `WorldTimeline` answer WITHOUT an
/// argument, plus per-hour dates, raw bodies, domains, coverage warnings and
/// the typed views `typed_views_debug` names, `WorldTimeline`'s own per-hour
/// `date_at` / `lookup_hour` / `declared_gap_at`, its `admit_playback_content`
/// against a fixed probe inventory, and all FOUR routes to a recorded hour —
/// `frame`, `state_at`, `seek` and `play_to`, which are not interchangeable
/// (`play_to` deliberately folds a v4 delta chain by an independent loop).
///
/// What it leaves out, and why — no claim of completeness beyond this list:
///   * `ReplaySource::static_artifact` and `WorldTimeline::static_artifact`:
///     name-keyed, with no enumeration API, so nothing can reach them.
///   * `WorldTimeline::coverage_state_at`, whose distinctive answer is the
///     DeclaredGap arm, and which reduces to the frame routes above at a
///     recorded hour. Its input, the declared coverage, is compared; the shipped
///     corpus declares no gaps, so nothing here would exercise the other arm.
///   * `WorldTimeline::open_with_config` at any budget other than the default.
///     `open` delegates to it, so the function IS executed, but only with
///     `TimelineConfig::default()`; a branch reachable only under another cache
///     budget is not covered.
///   * The scrub contract at UNRECORDED hours. Every hour asked here is one the
///     recording carries; how an unrecorded hour resolves against the spine is a
///     different property from relocation.
///
/// Every handle is dropped before this returns, so the caller may delete or
/// overwrite the file it just read.
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
    let g2_engine_capture = source.g2_engine_capture().map(|v| format!("{v:?}"));

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
    // `WorldTimeline` re-exposes the source's metadata through its own
    // accessors. They are read here rather than assumed to be pass-throughs: a
    // path reaching one of them alone would otherwise change nothing anyone
    // compares.
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
    // `admit_playback_content` answers about THIS RECORDING's content manifest,
    // measured against a supplied inventory. The inventory below is a fixed,
    // deliberately non-matching probe: what is compared is the recording-derived
    // report it produces, which names the recording's own requirements, and that
    // must not change when the recording moves.
    let admission = format!(
        "{:?}",
        timeline.admit_playback_content(&relocation_probe_inventory())
    );
    let mut timeline_domains = Vec::new();
    let mut timeline_per_hour = Vec::new();
    let mut coverage_warnings = Vec::new();
    let mut typed_views = Vec::new();
    for &hour in &timeline_hours {
        // All FOUR public routes to a recorded hour, not just `frame`. They are
        // not interchangeable: `play_to` deliberately runs an INDEPENDENT fold
        // for a v4 delta anchor rather than the accumulator `seek` uses, so a
        // path reaching only that fold would be invisible if this read `frame`
        // alone.
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
            // Each route's OWN complete map, through `WorldFrame::raw`, not a
            // key set borrowed from the source frame: a route that INVENTED a
            // domain the source never carried would be invisible to a borrowed
            // key set, because nothing would ever ask for the invented key.
            for (key, value) in resolved.raw() {
                timeline_domains.push((hour, format!("{label}.{key}"), Some(value.clone())));
            }
            typed_views.push((hour, format!("{label} {}", typed_views_debug(resolved))));
        }
        let frame = &routes[0].1;
        coverage_warnings.push((hour, frame.coverage_warnings().to_vec()));
        // The timeline's own per-hour accessors, for the same reason the
        // argument-free ones above are read: `WorldTimeline::date_at` and
        // `declared_gap_at` wrap the source's answers, and a path reaching one
        // of the wrappers alone would change nothing else compared here. Asked
        // only at RECORDED hours; the scrub primitives that resolve arbitrary
        // requested hours are a different property and are not this test's.
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
        g2_engine_capture,
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

/// Copying a shipped recording to a second directory, under a second name,
/// changes nothing the reader answers.
///
/// kills: any path-derived state in the reader.
///   * `game_id()` answered from the file STEM instead of the container index.
///     `positive_fixtures_load_and_match_manifest_probes` asserts
///     `source.game_id() == name` at the ONE path whose stem already equals
///     that name, so it passes for a stem-derived id; the copies here are
///     renamed `relocated-<name>.battleplan`, so a stem-derived id reads
///     `relocated-<name>` and both the `game_id` comparison and the two
///     `assert_eq!`s below fail.
///   * the opened path, or anything derived from it, reaching one of the
///     surfaces `read_recording` samples: `warnings`, `declared_coverage`, the
///     content manifest, campaign identity, product admission, prefold receipt,
///     the design / decision / G2 catalogs, `date_at`, the raw decoded body of
///     every hour, the `WorldTimeline` read-back of every one of those
///     top-level domains, `WorldFrame::coverage_warnings`, and the typed views
///     `typed_views_debug` names. `open_store_impl` already threads
///     `path.display()` in as `source_label`; one more `format!` from it
///     reaching any of those lands in the reading and differs between the two
///     locations. The raw-body comparison is exhaustive over the decoded
///     content — every top-level domain of every hour — so a path reaching
///     DECODED data is caught wherever it lands.
///   * a MANDATORY read of something beside the recording (a sidecar index, a
///     neighbouring `manifest.json`, a cache file next to the container): the
///     relocated copy is alone in its directory, so the open fails outright.
///   * a reader that LEAVES an extra directory entry beside the recording after
///     the read returns: the directory is re-counted at that point and must
///     still hold exactly one entry. A sibling created and removed inside the
///     read, or created asynchronously after the count, is not caught — this is
///     one observation in time, not a filesystem trace.
///
/// What it does NOT kill, stated so the comment does not claim more than the
/// assertions distinguish:
///   * a producer-side absolute path baked into the container BYTES. Those
///     bytes are copied verbatim, so a leaked recorder path is identical at
///     both locations. That is the recorder's guard, not this one.
///   * an OPTIONAL read of an external path (a sibling, the working directory,
///     an install root) whose fallback reproduces the same answer the bytes
///     would have given. Equality of two readings cannot separate "read the
///     bytes" from "read something else that happens to agree"; what makes that
///     distinction is `the_reader_follows_the_bytes_at_a_fixed_path`, and only
///     for the bytes it corrupts.
///   * `ReplaySource::static_artifact` / `WorldTimeline::static_artifact`,
///     which take an artifact NAME and expose no enumeration, so nothing here
///     samples them; and the typed accessors `typed_views_debug` does not name.
///     Note also that for the shipped corpus the design / decision / G2 /
///     prefold options are all `None` — those four comparisons are real but
///     currently compare absence to absence.
#[test]
fn relocating_a_recording_changes_nothing_the_reader_answers() {
    let root = fixture_root();
    let manifest = fixture_manifest();
    let positive = manifest["positive"]
        .as_array()
        .expect("positive fixture list");
    assert_eq!(positive.len(), 19, "the shipped positive corpus");

    let relocation_root = unique_relocation_root("positive");
    // Two shapes on purpose. The plain one is the ordinary case; the second
    // carries spaces and non-ASCII, which is where a Windows path that is
    // re-encoded on its way through the reader (a lossy `to_string_lossy`, an
    // ANSI round trip) stops naming the file it was built from.
    let locations: [&str; 2] = ["plain", "with spaces and unicode ünïcödé 日本語"];

    let outcome = std::panic::catch_unwind(|| {
        // The previous fixture's reading, kept only to prove that
        // `RecordingReading::differences` can report a difference at all. An
        // "identical" verdict from a comparator that always returns nothing is
        // not evidence, and no assertion below would notice.
        let mut previous: Option<(&str, RecordingReading)> = None;
        let mut families_that_fired: BTreeSet<&str> = BTreeSet::new();

        for entry in positive {
            let name = entry["name"].as_str().expect("fixture name");
            let rel = entry["path"].as_str().expect("fixture path");
            let canonical_path = root.join(rel);
            let canonical = read_recording(&canonical_path)
                .unwrap_or_else(|err| panic!("{name}: open canonical fixture: {err}"));

            // The reading must be substantive, or "identical" says nothing.
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
                // Not merely "non-empty": the four families in
                // `COMPARISON_FAMILIES` must each be shown to fire somewhere in
                // the corpus, or deleting all but one of them would still
                // satisfy the check above. (Only those four — the scalar arms
                // are unchecked; see the table.) Accumulated across the 18
                // adjacent pairs rather than required of each one, because two
                // fixtures can genuinely differ in their recorded bodies and
                // still present identical typed views, which is exactly what
                // `fx_dlc_language_variation` and `fx_research_focus` do.
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
                // Reading a recording must leave nothing behind beside it. A
                // spool or cache directory is only ever created through the
                // explicit spooled-decoder entry points, which take the caller's
                // own base; the capability-free `open_recording` used here must
                // write nothing.
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
                // Stated separately from the whole-reading comparison because a
                // reader that answered one CONSTANT id everywhere satisfies the
                // comparison above, and this is the field a stem-derived reader
                // would rewrite.
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

/// A recording that is refused is refused identically from any path, and the
/// refusal names no path.
///
/// kills: a refusal message built from the opened pathname. `open_store_impl`
/// formats `no game found ... under {source_label}` from `path.display()`
/// today; widening that spelling to the refusals these fixtures reach — or
/// adding the path to the `SourceError::Open` / `NotFound` renderings on the
/// routes they take — makes the two messages differ. Three checks, because
/// equality alone has holes:
///   * equality of the two messages — kills any leak of a path that differs
///     between the two locations, which is all of both paths but the drive;
///   * the marker, which is in the relocated copy's directory AND in its file
///     name and in neither canonical path — kills a leak of just the file name
///     or just the containing directory;
///   * no backslash anywhere in the message — kills a leak of a SHARED prefix,
///     which equality cannot see: `path.ancestors().last()` renders `C:\` at
///     both locations, and a partial-path leak of any depth carries a
///     separator.
/// The forward slash cannot be forbidden in the same way, because two of these
/// four refusals legitimately name an IN-CONTAINER entry
/// (`payload/<game>/000016959360.snapshot.json`, `payload/<game>/collector_raw/
/// compact_stream.jsonl`) and those are not pathnames on disk. So the backslash
/// check is Windows-shaped, and what escapes it is a leak that carries no
/// separator at all (a bare `C:`) or one rendered with `/` instead of `\`.
/// `negative_fixtures_are_rejected_with_precise_errors` sees none of this: it
/// opens each negative fixture once, at one path, and only asserts a substring
/// is present.
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
            // Same carve-out as `negative_fixtures_are_rejected_with_precise_errors`:
            // the capability-free entry point refuses a compact-only package
            // before it reaches the decoder-only schema the manifest records.
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
            // (`.battleplan` is deliberately NOT forbidden: `open .battleplan:`
            // is a fixed route label in the adapter's rendering, not a
            // pathname.)
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
            // The shared-prefix leak equality is blind to. Every path this test
            // opens is absolute, so any part of one that names a directory
            // carries a separator; the four refusals carry no backslash today.
            // The literal `'\\'` rather than `MAIN_SEPARATOR`: forbidding the
            // platform separator would forbid `/` on a Unix host, and two of
            // these refusals name an in-container entry with `/` in it, which
            // is not a pathname on disk.
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

/// The reader reads the BYTES, not the path: at one fixed path, changing the
/// bytes changes the answer, and restoring them restores it.
///
/// This is what stops the relocation test above from being satisfiable by a
/// reader that ignores its input — a constant reading, or one memoised per
/// process, is identical everywhere and would pass it. Here the pathname never
/// changes across the three opens, so nothing about the path can explain the
/// difference; and the corrupt bytes are then refused from a second path with
/// the identical message, so nothing about the path explains the refusal
/// either.
///
/// kills: a cache of the whole open result — the admitted source, or a complete
/// decoded reading — keyed by PATHNAME or by `(pathname, length)`, since the
/// corrupt copy has exactly the length of the pristine one (asserted below).
/// Such a cache answers `Ok` for the corrupt file and the `panic!` fires.
///
/// It does NOT kill:
///   * a cache that also keys on mtime, which every write here changes. A test
///     could hold mtime still — `File::set_times` with a saved `FileTimes` —
///     and this one does not.
///   * a cache of some LATER stage — an admission sub-decision, a decoded
///     frame — that is only consulted after the container structure is re-read.
///     The corruption here is byte 0, the ZIP local file header signature, so
///     the refusal happens before any such cache would be reached. The deep
///     form of that (a payload byte changed after hashing) is the shipped
///     `neg_bad_snapshot_hash` fixture, but it is refused at two DIFFERENT
///     paths in `a_refused_recording_is_refused_the_same_way_from_any_path`, so
///     it is not a fixed-path hit against a stage cache either.
#[test]
fn the_reader_follows_the_bytes_at_a_fixed_path() {
    let root = fixture_root();
    let pristine = std::fs::read(root.join("positive/fx_topbar_scalars_sweep.battleplan"))
        .expect("read fx_topbar_scalars_sweep");
    // A format-determined corruption, not a lucky offset: byte 0 of a ZIP is
    // the first byte of the mandatory local file header signature `PK\x03\x04`,
    // wherever the archive was repacked from and whatever it contains, so this
    // stays a corruption for any future regeneration of the corpus. (A deeper
    // corruption — a payload byte changed after hashing — is a shipped fixture
    // in its own right, `neg_bad_snapshot_hash`, and
    // `a_refused_recording_is_refused_the_same_way_from_any_path` relocates it.)
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

        // The same corrupt bytes, a different directory and a different name:
        // the refusal must be word for word the one above.
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
