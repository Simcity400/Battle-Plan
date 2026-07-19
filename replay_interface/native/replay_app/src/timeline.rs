//! Timeline abstraction: who owns and controls each state at each hour.
//!
//! Scrubbing consumes an [`OwnerProvider`]; the renderer rewrites only the
//! compact state -> color table from `controllers_at(hour)` for validation
//! while visible political recoloring remains blocked. `owners_at(hour)` stays
//! exposed because occupied-territory striping needs both maps.
//!
//! Frames are returned as `Arc` so a recording-backed provider can load
//! them lazily (a real recording is far too large to materialize every
//! hour up front) while the static provider stays allocation-free on repeat
//! calls.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use replay_core::{Leaf, WorldFrame, WorldTimeline};

use crate::anomaly::{scan_timeline, AnomalyPolicy};
use crate::bundle::MapBundle;
use crate::{Result, TimelineMode};

/// A headless recording selection. Canonical hours are absolute
/// `REPLAY_SCHEMA` hour_index values; ordinals are zero-based positions in the
/// verified recording index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DumpHourRequest {
    Canonical(i64),
    Ordinal(usize),
}

/// Loud, typed failures for recording-hour selection and frame loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineSelectionError {
    Empty,
    BeforeFirst {
        requested: i64,
        first: i64,
        last: i64,
    },
    AfterLast {
        requested: i64,
        first: i64,
        last: i64,
    },
    MissingCanonical {
        requested: i64,
        first: i64,
        last: i64,
    },
    OrdinalOutOfRange {
        requested: usize,
        count: usize,
        first: i64,
        last: i64,
    },
    FrameLoad {
        requested: i64,
        first: i64,
        last: i64,
        detail: String,
    },
}

impl std::fmt::Display for TimelineSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "recording has no canonical hours"),
            Self::BeforeFirst { requested, first, last } => write!(
                f,
                "requested canonical hour {requested} is before the recording's valid canonical range {first}..={last}"
            ),
            Self::AfterLast { requested, first, last } => write!(
                f,
                "requested canonical hour {requested} is after the recording's valid canonical range {first}..={last}"
            ),
            Self::MissingCanonical { requested, first, last } => write!(
                f,
                "requested canonical hour {requested} is not a recorded snapshot in the recording's valid canonical range {first}..={last}"
            ),
            Self::OrdinalOutOfRange { requested, count, first, last } => write!(
                f,
                "requested dump snapshot ordinal {requested} is outside the recording's valid ordinal range 0..={} (canonical range {first}..={last})",
                count - 1
            ),
            Self::FrameLoad { requested, first, last, detail } => write!(
                f,
                "failed to load requested canonical hour {requested} from the recording's valid canonical range {first}..={last}: {detail}"
            ),
        }
    }
}

impl std::error::Error for TimelineSelectionError {}

fn canonical_range(hours: &[i64]) -> std::result::Result<(i64, i64), TimelineSelectionError> {
    let first = *hours.first().ok_or(TimelineSelectionError::Empty)?;
    let last = *hours.last().ok_or(TimelineSelectionError::Empty)?;
    Ok((first, last))
}

/// Resolve either public dump selector to an exact canonical recorded hour.
/// This is deliberately stricter than timeline scrubbing, which may resolve
/// an in-between time to the preceding frame.
pub fn resolve_dump_hour(
    hours: &[i64],
    request: DumpHourRequest,
) -> std::result::Result<i64, TimelineSelectionError> {
    let (first, last) = canonical_range(hours)?;
    match request {
        DumpHourRequest::Ordinal(requested) => {
            hours
                .get(requested)
                .copied()
                .ok_or(TimelineSelectionError::OrdinalOutOfRange {
                    requested,
                    count: hours.len(),
                    first,
                    last,
                })
        }
        DumpHourRequest::Canonical(requested) if requested < first => {
            Err(TimelineSelectionError::BeforeFirst {
                requested,
                first,
                last,
            })
        }
        DumpHourRequest::Canonical(requested) if requested > last => {
            Err(TimelineSelectionError::AfterLast {
                requested,
                first,
                last,
            })
        }
        DumpHourRequest::Canonical(requested) => {
            if hours.binary_search(&requested).is_ok() {
                Ok(requested)
            } else {
                Err(TimelineSelectionError::MissingCanonical {
                    requested,
                    first,
                    last,
                })
            }
        }
    }
}

/// Build the owner provider + mode label for a timeline mode. Shared by the
/// interactive window loop and the headless frame-dump path.
pub fn build_provider(
    bundle: Option<&MapBundle>,
    mode: &TimelineMode,
) -> Result<(Box<dyn OwnerProvider>, String)> {
    match mode {
        TimelineMode::Recording {
            path,
            timeline: config,
        } => {
            let source = replay_core::open_recording(path)
                .map_err(|err| format!("open recording {}: {err}", path.display()))?;
            // F2: the decoded-frame LRU budget rides the mode (default 16;
            // `--frame-cache` shrinks it for weak-PC profiles).
            let timeline = replay_core::WorldTimeline::open_with_config(source, *config)
                .map_err(|err| format!("open timeline {}: {err}", path.display()))?;
            for warning in timeline.warnings() {
                eprintln!("replay_app: recording warning: {warning}");
            }
            // F4 mid-playback anomaly policy: scan the recording at open, log
            // every mid-stream anomaly once (log + hold-last-good, never
            // invent). The known-province set (for out-of-range province ids)
            // comes from the bundle's province->state map.
            // F2 (2026-07-10) audited this scan: MEMORY is bounded (frames go
            // through the LRU-capped cache; the policy stores change-points,
            // not per-hour snapshots) — but it still DECODES every frame once
            // at open, an O(hours) LATENCY residual on multi-year recordings
            // (plan F2 row). An incremental/lazy scan is future work.
            let known_provinces = bundle.map(|bundle| {
                bundle
                    .province_to_state
                    .keys()
                    .copied()
                    .collect::<BTreeSet<u32>>()
            });
            let policy = scan_timeline(&timeline, known_provinces);
            for diagnostic in policy.diagnostics() {
                eprintln!("{}", diagnostic.log_line());
            }
            let recording = RecordingTimeline::new_with_policy(timeline, Some(policy));
            let label = format!("recording {}", recording.game_id());
            Ok((Box::new(recording), label))
        }
        TimelineMode::Static1936 => {
            let bundle = bundle.ok_or(
                "static 1936 debug mode requires --bundle; production playback uses --recording and never requires a pre-generated map bundle",
            )?;
            Ok((
                Box::new(Static1936::new(bundle.state_owner_1936.clone())),
                String::from("static 1936"),
            ))
        }
    }
}

/// Hour-indexed state ownership. Hours are ascending and non-empty.
pub trait OwnerProvider {
    fn hours(&self) -> &[i64];
    fn owners_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>>;
    fn controllers_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>>;
    /// C7 render wiring: the typed replay_core frame for this hour. Static
    /// providers return `None`; consumers never parse recording bytes.
    fn world_frame_at(&self, _hour: i64) -> Result<Option<Arc<WorldFrame>>> {
        Ok(None)
    }
    /// Human-readable label for the title bar; recording-backed providers
    /// add the in-game date.
    fn hour_label(&self, hour: i64) -> Result<String> {
        Ok(format!("hour {hour}"))
    }
    /// The recorded `date` string for an exact recorded hour, when the frame
    /// carried one. Used to build the transport's date spine (seek-bar ticks +
    /// jump-to-date). Defaults to `None` (dateless providers degrade honestly).
    fn date_at(&self, _hour: i64) -> Result<Option<String>> {
        Ok(None)
    }
    /// E6: the observer country picker source, derived from the recording's
    /// session view. Defaults to fail-closed (no recording session).
    fn pov_picker(&self) -> crate::observer::PovPicker {
        crate::observer::PovPicker::Unavailable(
            "no recording session (static baseline has no player countries)".to_string(),
        )
    }

    /// F4: the marked held-value note (mid-playback anomaly holds) for `hour`,
    /// or `None` when nothing is held. Defaults to `None` (no anomaly policy).
    fn anomaly_note(&self, _hour: i64) -> Option<String> {
        None
    }

    fn content_manifest(&self) -> Option<&replay_core::ContentManifest> {
        None
    }

    fn admit_playback_content(
        &self,
        _inventory: &replay_core::PlaybackContentInventory,
    ) -> std::result::Result<
        replay_core::ValidatedContentAdmission,
        replay_core::ContentValidationReport,
    > {
        Err(replay_core::ContentValidationReport {
            compatible: false,
            verified_required_content: 0,
            issues: vec![replay_core::ContentIssue {
                code: replay_core::ContentIssueCode::InvalidManifest,
                subject: "timeline".into(),
                message: "static timeline has no portable content manifest".into(),
            }],
        })
    }
}

/// Single-frame provider: the grounded 1936 start owners from
/// `state_owner_1936.json`.
pub struct Static1936 {
    hours: Vec<i64>,
    owners: Arc<BTreeMap<u32, String>>,
}

impl Static1936 {
    pub fn new(owners: BTreeMap<u32, String>) -> Static1936 {
        Static1936 {
            hours: vec![0],
            owners: Arc::new(owners),
        }
    }
}

impl OwnerProvider for Static1936 {
    fn hours(&self) -> &[i64] {
        &self.hours
    }

    fn owners_at(&self, _hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(Arc::clone(&self.owners))
    }

    fn controllers_at(&self, _hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(Arc::clone(&self.owners))
    }
}

/// Recording-backed provider over a `replay_core::WorldTimeline`.
///
/// Trichotomy rule: only a `Leaf::Measured` owner becomes a map entry;
/// `Absent` and `Unobtained` states stay off the table, so the renderer
/// shows the no-owner sentinel (the grounded underlay) instead of a
/// fabricated owner color.
pub struct RecordingTimeline {
    timeline: WorldTimeline,
    /// One-slot cache: (requested hour, owners, controllers, date label). The
    /// WorldTimeline keeps its own LRU of decoded frames, so a miss here is
    /// only a cheap re-conversion, but the probe path calls per mouse-move
    /// and must not re-walk the leaf map every event.
    cache: Mutex<Option<CachedFrame>>,
    /// F4 mid-playback anomaly policy (populated at open by `build_provider`).
    /// Holds the marked last-good state so the UI can surface held domains.
    anomaly: Option<AnomalyPolicy>,
}

struct CachedFrame {
    hour: i64,
    owners: Arc<BTreeMap<u32, String>>,
    controllers: Arc<BTreeMap<u32, String>>,
    date: Option<String>,
}

impl RecordingTimeline {
    pub fn new_with_policy(
        timeline: WorldTimeline,
        anomaly: Option<AnomalyPolicy>,
    ) -> RecordingTimeline {
        RecordingTimeline {
            timeline,
            cache: Mutex::new(None),
            anomaly,
        }
    }

    /// F4: the marked held-value note for the frame at-or-before `hour`, e.g.
    /// "anomaly: HELD weather from hour 41", or `None` when nothing is held
    /// (pre-anomaly hours, or a clean recording). An hour that lands inside a
    /// detected gap is reported as unavailable, never interpolated.
    fn anomaly_note_at(&self, hour: i64) -> Option<String> {
        use crate::anomaly::HourResolution;
        let policy = self.anomaly.as_ref()?;
        // A requested hour inside a gap is unavailable (never interpolated).
        if let HourResolution::GapUnavailable { from, to, missing } = policy.classify_hour(hour) {
            return Some(format!(
                "anomaly: hour {hour} unavailable (gap {from}->{to}, missing {missing:?}); not interpolated"
            ));
        }
        let frame_hour = self.timeline.state_at(hour).ok()?.hour_index();
        let mut parts: Vec<String> = Vec::new();
        // Note a gap immediately preceding this frame (the after-gap hour).
        if let Some(gap) = policy.gaps().iter().find(|g| g.to == frame_hour) {
            parts.push(format!("gap before (missing {:?})", gap.missing));
        }
        for hold in policy.active_holds_at(frame_hour) {
            parts.push(hold.marker);
        }
        if parts.is_empty() {
            None
        } else {
            Some(format!("anomaly: {}", parts.join("; ")))
        }
    }

    pub fn game_id(&self) -> &str {
        self.timeline.game_id()
    }

    /// E1 wiring: random-access seek through the `replay_core` hook. Returns
    /// the resolved recorded hour (at-or-before), or `None` before the first
    /// frame. Playback uses [`WorldTimeline::seek`] / [`WorldTimeline::play_to`]
    /// which resolve identically today (see the `seek_equals_play_to`
    /// conformance gate); the interactive scrub reads owners through the same
    /// `state_at` primitive.
    pub fn seek_hour(&self, hour: i64) -> Option<i64> {
        self.timeline
            .seek(hour)
            .ok()
            .map(|frame| frame.hour_index())
    }

    /// E1 wiring: linear play-to through the `replay_core` hook.
    pub fn play_to_hour(&self, hour: i64) -> Option<i64> {
        self.timeline
            .play_to(hour)
            .ok()
            .map(|frame| frame.hour_index())
    }

    fn selected_frame(&self, hour: i64) -> Result<Arc<WorldFrame>> {
        let (first, last) = canonical_range(self.timeline.hours())?;
        if hour < first {
            return Err(TimelineSelectionError::BeforeFirst {
                requested: hour,
                first,
                last,
            }
            .into());
        }
        if hour > last {
            return Err(TimelineSelectionError::AfterLast {
                requested: hour,
                first,
                last,
            }
            .into());
        }
        self.timeline.state_at(hour).map_err(|err| {
            TimelineSelectionError::FrameLoad {
                requested: hour,
                first,
                last,
                detail: err.to_string(),
            }
            .into()
        })
    }

    fn frame_maps(
        &self,
        hour: i64,
    ) -> Result<(
        Arc<BTreeMap<u32, String>>,
        Arc<BTreeMap<u32, String>>,
        Option<String>,
    )> {
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.hour == hour {
                    return Ok((
                        Arc::clone(&cached.owners),
                        Arc::clone(&cached.controllers),
                        cached.date.clone(),
                    ));
                }
            }
        }
        // Any selection or snapshot-body load failure remains an error all the
        // way to the renderer; an empty political table is valid data only
        // when the decoded typed view itself is genuinely empty.
        let frame = self.selected_frame(hour)?;
        let owners = Arc::new(owners_from_leaves(frame.owner_by_state()));
        let controllers = Arc::new(owners_from_leaves(frame.control_by_state()));
        let date = frame.date().map(str::to_string);
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some(CachedFrame {
                hour,
                owners: Arc::clone(&owners),
                controllers: Arc::clone(&controllers),
                date: date.clone(),
            });
        }
        Ok((owners, controllers, date))
    }
}

impl OwnerProvider for RecordingTimeline {
    fn hours(&self) -> &[i64] {
        self.timeline.hours()
    }

    fn owners_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(self.frame_maps(hour)?.0)
    }

    fn controllers_at(&self, hour: i64) -> Result<Arc<BTreeMap<u32, String>>> {
        Ok(self.frame_maps(hour)?.1)
    }

    fn world_frame_at(&self, hour: i64) -> Result<Option<Arc<WorldFrame>>> {
        self.selected_frame(hour).map(Some)
    }

    fn hour_label(&self, hour: i64) -> Result<String> {
        // D6 replay-native clock: format the recording date with the FILE
        // topbar clock law (`interface/topbar.gui` DateText placeholder +
        // `core_l_english.yml` months) via game_ui; fall back to the raw
        // Clausewitz string when it does not parse (fail-closed, never a
        // fabricated date). Closes the former raw-date GAP note in app.rs.
        Ok(match self.frame_maps(hour)?.2 {
            Some(date) => match game_ui::binding::format_topbar_date_english(&date) {
                Some(formatted) => format!("{formatted} (hour {hour})"),
                None => format!("{date} (hour {hour})"),
            },
            None => format!("hour {hour}"),
        })
    }

    fn date_at(&self, hour: i64) -> Result<Option<String>> {
        Ok(self.frame_maps(hour)?.2)
    }

    fn pov_picker(&self) -> crate::observer::PovPicker {
        // Player countries are session-level; read them from the first frame's
        // session view through the typed seam (never a raw snapshot parse).
        match self.timeline.state_at(self.timeline.first_hour()) {
            Ok(frame) => crate::observer::povs_from_session(frame.session_metadata()),
            Err(err) => {
                crate::observer::PovPicker::Unavailable(format!("session view unavailable: {err}"))
            }
        }
    }

    fn anomaly_note(&self, hour: i64) -> Option<String> {
        self.anomaly_note_at(hour)
    }

    fn content_manifest(&self) -> Option<&replay_core::ContentManifest> {
        self.timeline.content_manifest()
    }

    fn admit_playback_content(
        &self,
        inventory: &replay_core::PlaybackContentInventory,
    ) -> std::result::Result<
        replay_core::ValidatedContentAdmission,
        replay_core::ContentValidationReport,
    > {
        self.timeline.admit_playback_content(inventory)
    }
}

/// Measured owners only; `Absent`/`Unobtained` stay off the map (sentinel).
pub fn owners_from_leaves(leaves: &BTreeMap<u32, Leaf<String>>) -> BTreeMap<u32, String> {
    leaves
        .iter()
        .filter_map(|(state_id, leaf)| leaf.measured().map(|tag| (*state_id, tag.clone())))
        .collect()
}

#[cfg(test)]
fn month_day_from_clausewitz_date(date: &str) -> Option<(u32, u32)> {
    let mut parts = date.split('.');
    let _year = parts.next()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    Some((month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_owners() -> BTreeMap<u32, String> {
        let mut owners = BTreeMap::new();
        for state_id in 1..=30u32 {
            let tag = if state_id % 2 == 0 { "GER" } else { "FRA" };
            owners.insert(state_id, tag.to_string());
        }
        owners
    }

    #[test]
    fn static_1936_has_single_hour_and_constant_owners() {
        let provider = Static1936::new(fixture_owners());
        assert_eq!(provider.hours(), &[0]);
        assert_eq!(*provider.owners_at(0).expect("owners"), fixture_owners());
        assert_eq!(
            provider.controllers_at(0).expect("controllers"),
            provider.owners_at(0).expect("owners")
        );
        assert!(provider.world_frame_at(0).expect("world frame").is_none());
        // Out-of-range hours still resolve to the single frame.
        assert_eq!(
            *provider.owners_at(9999).expect("constant owners"),
            fixture_owners()
        );
        assert_eq!(
            provider.controllers_at(9999).expect("constant controllers"),
            provider.owners_at(9999).expect("constant owners")
        );
    }

    #[test]
    fn dump_hour_selection_distinguishes_canonical_hours_from_ordinals() {
        let hours = [16_959_372, 16_959_373, 16_959_375];
        assert_eq!(
            resolve_dump_hour(&hours, DumpHourRequest::Canonical(16_959_373)),
            Ok(16_959_373)
        );
        assert_eq!(
            resolve_dump_hour(&hours, DumpHourRequest::Ordinal(2)),
            Ok(16_959_375)
        );
        assert!(matches!(
            resolve_dump_hour(&hours, DumpHourRequest::Canonical(71)),
            Err(TimelineSelectionError::BeforeFirst {
                requested: 71,
                first: 16_959_372,
                last: 16_959_375,
            })
        ));
        assert!(matches!(
            resolve_dump_hour(&hours, DumpHourRequest::Canonical(16_959_374)),
            Err(TimelineSelectionError::MissingCanonical { .. })
        ));
        assert!(matches!(
            resolve_dump_hour(&hours, DumpHourRequest::Ordinal(3)),
            Err(TimelineSelectionError::OrdinalOutOfRange { .. })
        ));
    }

    // The historical branch supplied a synthetic failing ReplaySource for this
    // test. That branch-local source is preserved in the Git bundle; the
    // consolidated active gates cover the same public invalid-hour boundary.
    #[cfg(any())]
    #[test]
    fn recording_provider_never_replaces_selection_or_load_errors_with_empty_maps() {
        let timeline = WorldTimeline::open(Box::new(MidstreamLoadFailureSource::new()))
            .expect("failing timeline");
        let recording = RecordingTimeline::new_with_policy(timeline, None);

        let before = recording.owners_at(-1).expect_err("before-first must fail");
        assert!(
            before.to_string().contains(
                "requested canonical hour -1 is before the recording's valid canonical range 0..=2"
            ),
            "{before}"
        );
        let after = recording
            .controllers_at(3)
            .expect_err("after-last must fail");
        assert!(
            after.to_string().contains(
                "requested canonical hour 3 is after the recording's valid canonical range 0..=2"
            ),
            "{after}"
        );
        let load = recording
            .owners_at(2)
            .expect_err("snapshot load failure must fail");
        assert!(
            load.to_string().contains(
                "failed to load requested canonical hour 2 from the recording's valid canonical range 0..=2"
            ),
            "{load}"
        );
    }

    #[test]
    fn owners_from_leaves_keeps_only_measured() {
        let mut leaves: BTreeMap<u32, Leaf<String>> = BTreeMap::new();
        leaves.insert(1, Leaf::Measured("GER".to_string()));
        leaves.insert(2, Leaf::Unobtained);
        leaves.insert(3, Leaf::Absent);
        let owners = owners_from_leaves(&leaves);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners.get(&1).map(String::as_str), Some("GER"));
        // Unobtained/Absent must NOT appear (they render as the underlay
        // sentinel, never as a fabricated owner).
        assert!(!owners.contains_key(&2));
        assert!(!owners.contains_key(&3));
    }

    fn fixture_path(rel: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
            .join(rel)
    }

    // Retained historical regression corpus for the superseded incremental
    // anomaly scanner. The consolidated product uses the eager AnomalyPolicy
    // constructed in build_provider, so these implementation-specific tests
    // remain preserved but are not part of this architecture's test build.
    #[cfg(any())]
    mod retired_incremental_anomaly_tests {
        fn committed_playable_fixtures() -> Vec<(String, std::path::PathBuf)> {
            let manifest_path = fixture_path("manifest.json");
            let bytes = std::fs::read(&manifest_path).expect("read fixture manifest");
            let manifest: serde_json::Value =
                serde_json::from_slice(&bytes).expect("parse fixture manifest");
            let mut fixtures = Vec::new();
            for section in ["positive", "anomaly"] {
                for entry in manifest[section]
                    .as_array()
                    .expect("playable fixture section")
                {
                    let name = entry["name"].as_str().expect("fixture name");
                    let rel = entry["path"].as_str().expect("fixture path");
                    fixtures.push((name.to_string(), fixture_path(rel)));
                }
            }
            assert!(!fixtures.is_empty(), "manifest has no playable fixtures");
            fixtures
        }

        fn open_counted_recording(path: &std::path::Path) -> (RecordingTimeline, Arc<AtomicUsize>) {
            let source = replay_core::open_recording(path)
                .unwrap_or_else(|error| panic!("open counted fixture {}: {error}", path.display()));
            let (source, decodes) = CountingReplaySource::new(source);
            let timeline = WorldTimeline::open(Box::new(source)).unwrap_or_else(|error| {
                panic!("open counted timeline {}: {error}", path.display())
            });
            let scan = IncrementalAnomalyScan::new(timeline.hours(), None);
            (
                RecordingTimeline::new_with_incremental_scan(timeline, scan),
                decodes,
            )
        }

        /// Gate E1 (app-open latency): run the exact `app::run` index-date
        /// collection over the largest committed playable fixture. Container open,
        /// timeline open, anomaly setup, and the complete date-spine pass perform
        /// zero snapshot-body decodes and leave F4's watermark untouched.
        #[test]
        fn app_open_date_spine_is_zero_decode_on_largest_committed_fixture() {
            let (name, path) = committed_playable_fixtures()
                .into_iter()
                .max_by_key(|(_, path)| {
                    std::fs::metadata(path)
                        .expect("playable fixture metadata")
                        .len()
                })
                .expect("largest playable fixture");
            let fixture_bytes = std::fs::metadata(&path)
                .expect("largest fixture metadata")
                .len();

            let started = Instant::now();
            let (recording, decodes) = open_counted_recording(&path);
            let dates: Vec<Option<String>> = recording
                .hours()
                .iter()
                .map(|hour| recording.date_at_indexed(*hour))
                .collect();
            let elapsed = started.elapsed();

            assert_eq!(dates.len(), recording.hours().len(), "complete spine");
            assert_eq!(
                decodes.load(Ordering::SeqCst),
                0,
                "app-open index-date collection decoded a frame"
            );
            assert_eq!(
                recording.anomaly_scanned_through(),
                None,
                "index-only dates must not advance the F4 watermark"
            );
            eprintln!(
            "[e1-index-date-open] fixture={name} bytes={fixture_bytes} hours={} frame_decodes=0 open_and_spine_us={}",
            dates.len(),
            elapsed.as_micros()
        );
        }

        /// Gate E1 (content): for every committed fixture that opens for playback,
        /// the verified INDEX date bytes equal the decoded snapshot-body date bytes
        /// at every canonical hour. The index pass itself remains zero-decode.
        #[test]
        fn indexed_dates_equal_decoded_dates_on_every_committed_playable_fixture() {
            let fixtures = committed_playable_fixtures();
            let mut compared_hours = 0usize;
            for (name, path) in &fixtures {
                let (recording, decodes) = open_counted_recording(path);
                let hours = recording.hours().to_vec();
                let indexed: Vec<Option<String>> = hours
                    .iter()
                    .map(|hour| recording.date_at_indexed(*hour))
                    .collect();
                assert_eq!(
                    decodes.load(Ordering::SeqCst),
                    0,
                    "{name}: indexed date pass decoded a frame"
                );
                assert_eq!(
                    recording.anomaly_scanned_through(),
                    None,
                    "{name}: indexed date pass advanced F4 watermark"
                );

                let decoded: Vec<Option<String>> = hours
                    .iter()
                    .map(|hour| recording.date_at(*hour).expect("decoded date"))
                    .collect();
                assert_eq!(
                    indexed, decoded,
                    "{name}: INDEX and decoded date bytes differ"
                );
                compared_hours += hours.len();
            }
            eprintln!(
            "[e1-index-date-equivalence] playable_fixtures={} compared_hours={compared_hours} byte_differences=0",
            fixtures.len()
        );
        }

        #[derive(Clone, Copy)]
        enum PublicQuery {
            Anomaly(i64),
            Date(i64),
            Owners(i64),
            Controllers(i64),
            HourLabel(i64),
            Seek(i64),
            PlayTo(i64),
        }

        fn public_query_answer(recording: &RecordingTimeline, query: PublicQuery) -> String {
            match query {
                PublicQuery::Anomaly(hour) => {
                    format!("anomaly({hour})={:?}", recording.anomaly_note(hour))
                }
                PublicQuery::Date(hour) => format!("date({hour})={:?}", recording.date_at(hour)),
                PublicQuery::Owners(hour) => {
                    format!("owners({hour})={:?}", recording.owners_at(hour))
                }
                PublicQuery::Controllers(hour) => {
                    format!("controllers({hour})={:?}", recording.controllers_at(hour))
                }
                PublicQuery::HourLabel(hour) => {
                    format!("hour_label({hour})={:?}", recording.hour_label(hour))
                }
                PublicQuery::Seek(hour) => format!("seek({hour})={:?}", recording.seek_hour(hour)),
                PublicQuery::PlayTo(hour) => {
                    format!("play_to({hour})={:?}", recording.play_to_hour(hour))
                }
            }
        }

        #[test]
        fn recording_timeline_public_queries_match_eager_for_high_then_low_sequence() {
            let path = fixture_path("anomaly/fx_anomaly_missing_domain.battleplan");
            let eager_timeline = WorldTimeline::open(
                replay_core::open_recording(&path).expect("open eager anomaly fixture"),
            )
            .expect("eager anomaly timeline");
            let eager_policy = crate::anomaly::scan_timeline(
                &eager_timeline,
                Some([1000u32, 1001, 2000, 3000].into_iter().collect()),
            );
            let eager = RecordingTimeline::new_with_policy(eager_timeline, Some(eager_policy));

            let lazy_timeline = WorldTimeline::open(
                replay_core::open_recording(&path).expect("open lazy anomaly fixture"),
            )
            .expect("lazy anomaly timeline");
            let lazy_scan = IncrementalAnomalyScan::new(
                lazy_timeline.hours(),
                Some([1000u32, 1001, 2000, 3000].into_iter().collect()),
            );
            let lazy = RecordingTimeline::new_with_incremental_scan(lazy_timeline, lazy_scan);

            let hours = eager.hours();
            let first = hours[0];
            let crossing = hours[6];
            let last = *hours.last().expect("fixture hours");
            let queries = [
                // A decoded date query is still a genuine playback visit and must
                // advance every preceding frame-derived diagnostic. `app::run`
                // now uses the separately gated index-only date path at open.
                PublicQuery::Date(last),
                PublicQuery::Anomaly(first),
                PublicQuery::Owners(crossing),
                PublicQuery::Date(crossing - 1),
                PublicQuery::Controllers(last - 1),
                PublicQuery::Anomaly(crossing),
                PublicQuery::HourLabel(first + 1),
                PublicQuery::Seek(last),
                PublicQuery::PlayTo(first),
                PublicQuery::Owners(last - 2),
            ];

            let mut eager_transcript = String::new();
            let mut lazy_transcript = String::new();
            for (index, query) in queries.into_iter().enumerate() {
                let eager_answer = public_query_answer(&eager, query);
                let lazy_answer = public_query_answer(&lazy, query);
                assert_eq!(
                    lazy_answer.as_bytes(),
                    eager_answer.as_bytes(),
                    "query {index}"
                );
                writeln!(&mut eager_transcript, "{eager_answer}").unwrap();
                writeln!(&mut lazy_transcript, "{lazy_answer}").unwrap();
                if index == 0 {
                    assert_eq!(
                        lazy.anomaly_scanned_through(),
                        Some(last),
                        "decoded date_at(last) must remain eager-policy-equivalent"
                    );
                }
            }
            assert_eq!(lazy_transcript.into_bytes(), eager_transcript.into_bytes());
            assert_eq!(
                lazy.anomaly_diagnostic_log_lines(),
                eager.anomaly_diagnostic_log_lines(),
                "stderr-visible diagnostic lines must end byte-identical"
            );
        }

        #[test]
        fn unreadable_recorded_frame_suppresses_index_only_anomaly_note() {
            let eager_timeline = WorldTimeline::open(Box::new(MidstreamLoadFailureSource::new()))
                .expect("eager failing timeline");
            let eager_policy = crate::anomaly::scan_timeline(&eager_timeline, None);
            let eager = RecordingTimeline::new_with_policy(eager_timeline, Some(eager_policy));

            let lazy_timeline = WorldTimeline::open(Box::new(MidstreamLoadFailureSource::new()))
                .expect("lazy failing timeline");
            let lazy_scan = IncrementalAnomalyScan::new(lazy_timeline.hours(), None);
            let lazy = RecordingTimeline::new_with_incremental_scan(lazy_timeline, lazy_scan);

            assert_eq!(eager.anomaly_note(2), None, "eager load-failure answer");
            assert_eq!(
                lazy.anomaly_note(2),
                None,
                "lazy must not return the index-derived gap-before note for an unreadable frame"
            );
            assert_eq!(
                lazy.anomaly_diagnostic_log_lines(),
                eager.anomaly_diagnostic_log_lines(),
                "load-failure diagnostics must remain eager-equivalent"
            );
            assert!(
                lazy.anomaly_diagnostic_log_lines()
                    .iter()
                    .any(|line| line.contains("recorded frame failed to load")),
                "constructed fixture must actually exercise the mid-stream load failure"
            );
        }
    }

    #[test]
    fn seek_equals_play_to_on_every_recorded_hour() {
        // E1/A4: the replay_core seek/play_to hooks resolve identically, and
        // date_at surfaces the recorded date for the transport spine.
        let path = fixture_path("positive/fx_owner_controller_flip.battleplan");
        let source = replay_core::open_recording(&path).expect("open fixture");
        let timeline = replay_core::WorldTimeline::open(source).expect("timeline");
        let recording = RecordingTimeline::new_with_policy(timeline, None);
        for &hour in recording.hours() {
            assert_eq!(
                recording.seek_hour(hour),
                recording.play_to_hour(hour),
                "seek != play_to at hour {hour}"
            );
            assert_eq!(recording.seek_hour(hour), Some(hour), "exact-hour resolves");
            // Every recorded frame in this fixture carries a Clausewitz date.
            assert!(
                recording.date_at(hour).expect("date query").is_some(),
                "date at hour {hour}"
            );
            assert_eq!(
                recording
                    .world_frame_at(hour)
                    .expect("world frame query")
                    .map(|frame| frame.hour_index()),
                Some(hour),
                "typed WorldFrame is exposed at the exact recorded hour"
            );
        }
    }

    #[test]
    fn clausewitz_date_parser_extracts_month_and_day() {
        assert_eq!(month_day_from_clausewitz_date("1936.1.1.12"), Some((1, 1)));
        assert_eq!(
            month_day_from_clausewitz_date("1941.10.15.0"),
            Some((10, 15))
        );
        assert_eq!(month_day_from_clausewitz_date("hour 12"), None);
    }
}
