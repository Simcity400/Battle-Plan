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

use replay_core::{Leaf, WorldTimeline};

use crate::anomaly::{scan_timeline, AnomalyPolicy};
use crate::bundle::MapBundle;
use crate::{Result, TimelineMode};

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
    fn owners_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>>;
    fn controllers_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>>;
    /// Human-readable label for the title bar; recording-backed providers
    /// add the in-game date.
    fn hour_label(&self, hour: i64) -> String {
        format!("hour {hour}")
    }
    /// The recorded `date` string for an exact recorded hour, when the frame
    /// carried one. Used to build the transport's date spine (seek-bar ticks +
    /// jump-to-date). Defaults to `None` (dateless providers degrade honestly).
    fn date_at(&self, _hour: i64) -> Option<String> {
        None
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

    /// Exact typed frame for production UI binding. Static baselines and
    /// failed timeline reads return `None`; callers omit data-bound UI rather
    /// than parsing raw snapshots or manufacturing defaults.
    fn world_frame_at(&self, _hour: i64) -> Option<Arc<replay_core::WorldFrame>> {
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

    fn owners_at(&self, _hour: i64) -> Arc<BTreeMap<u32, String>> {
        Arc::clone(&self.owners)
    }

    fn controllers_at(&self, _hour: i64) -> Arc<BTreeMap<u32, String>> {
        Arc::clone(&self.owners)
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

    fn frame_maps(
        &self,
        hour: i64,
    ) -> (
        Arc<BTreeMap<u32, String>>,
        Arc<BTreeMap<u32, String>>,
        Option<String>,
    ) {
        if let Ok(guard) = self.cache.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.hour == hour {
                    return (
                        Arc::clone(&cached.owners),
                        Arc::clone(&cached.controllers),
                        cached.date.clone(),
                    );
                }
            }
        }
        // state_at scrubs to the frame at-or-before `hour`; hours() only
        // hands out recorded hours, so this errors only on a recording that
        // changed under us — render the empty (all-underlay) map, honestly.
        let (owners, controllers, date) = match self.timeline.state_at(hour) {
            Ok(frame) => (
                Arc::new(owners_from_leaves(frame.owner_by_state())),
                Arc::new(owners_from_leaves(frame.control_by_state())),
                frame.date().map(str::to_string),
            ),
            Err(_) => (Arc::new(BTreeMap::new()), Arc::new(BTreeMap::new()), None),
        };
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some(CachedFrame {
                hour,
                owners: Arc::clone(&owners),
                controllers: Arc::clone(&controllers),
                date: date.clone(),
            });
        }
        (owners, controllers, date)
    }
}

impl OwnerProvider for RecordingTimeline {
    fn hours(&self) -> &[i64] {
        self.timeline.hours()
    }

    fn owners_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>> {
        self.frame_maps(hour).0
    }

    fn controllers_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>> {
        self.frame_maps(hour).1
    }

    fn hour_label(&self, hour: i64) -> String {
        // D6 replay-native clock: format the recording date with the FILE
        // topbar clock law (`interface/topbar.gui` DateText placeholder +
        // `core_l_english.yml` months) via game_ui; fall back to the raw
        // Clausewitz string when it does not parse (fail-closed, never a
        // fabricated date). Closes the former raw-date GAP note in app.rs.
        match self.frame_maps(hour).2 {
            Some(date) => match game_ui::binding::format_topbar_date_english(&date) {
                Some(formatted) => format!("{formatted} (hour {hour})"),
                None => format!("{date} (hour {hour})"),
            },
            None => format!("hour {hour}"),
        }
    }

    fn date_at(&self, hour: i64) -> Option<String> {
        self.frame_maps(hour).2
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

    fn world_frame_at(&self, hour: i64) -> Option<Arc<replay_core::WorldFrame>> {
        self.timeline.state_at(hour).ok()
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
        assert_eq!(*provider.owners_at(0), fixture_owners());
        assert_eq!(provider.controllers_at(0), provider.owners_at(0));
        // Out-of-range hours still resolve to the single frame.
        assert_eq!(*provider.owners_at(9999), fixture_owners());
        assert_eq!(provider.controllers_at(9999), provider.owners_at(9999));
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
            assert!(recording.date_at(hour).is_some(), "date at hour {hour}");
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
