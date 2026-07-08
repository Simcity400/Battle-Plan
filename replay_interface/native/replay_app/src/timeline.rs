//! Timeline abstraction: who owns and controls each state at each hour.
//!
//! Scrubbing consumes an [`OwnerProvider`]; the renderer rewrites only the
//! compact state -> color table from `controllers_at(hour)` plus the generated
//! runtime masks (never the big map textures). `owners_at(hour)` stays exposed
//! because occupied-territory striping needs both maps.
//!
//! Frames are returned as `Arc` so a recording-backed provider can load
//! them lazily (a real recording is far too large to materialize every
//! hour up front) while the static/synthetic providers stay allocation-free
//! on repeat calls.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use replay_core::{Leaf, WorldTimeline};

use crate::bundle::MapBundle;
use crate::{Result, TimelineMode};

/// Build the owner provider + mode label for a timeline mode. Shared by the
/// interactive window loop and the headless frame-dump path.
pub fn build_provider(
    bundle: &MapBundle,
    mode: &TimelineMode,
) -> Result<(Box<dyn OwnerProvider>, String)> {
    match mode {
        TimelineMode::Recording(path) => {
            let source = replay_core::open_recording(path)
                .map_err(|err| format!("open recording {}: {err}", path.display()))?;
            let timeline = replay_core::WorldTimeline::open(source)
                .map_err(|err| format!("open timeline {}: {err}", path.display()))?;
            for warning in timeline.warnings() {
                eprintln!("replay_app: recording warning: {warning}");
            }
            let recording = RecordingTimeline::new(timeline);
            let label = format!("recording {}", recording.game_id());
            Ok((Box::new(recording), label))
        }
        TimelineMode::DemoScrub => Ok((
            Box::new(SyntheticScrubDemo::new(
                &bundle.state_owner_1936,
                &bundle.country_colors,
            )),
            String::from("SYNTHETIC DEMO (not replay data)"),
        )),
        TimelineMode::Static1936 => Ok((
            Box::new(Static1936::new(bundle.state_owner_1936.clone())),
            String::from("static 1936"),
        )),
    }
}

/// Hour-indexed state ownership. Hours are ascending and non-empty.
pub trait OwnerProvider {
    fn hours(&self) -> &[i64];
    fn owners_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>>;
    fn controllers_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>>;
    /// Replay date as `(month, day)` when the backing timeline has a measured
    /// Clausewitz date. Static and synthetic modes return `None`, so
    /// date-driven visual experiments are documented no-ops there.
    fn month_day_at(&self, _hour: i64) -> Option<(u32, u32)> {
        None
    }
    /// Human-readable label for the title bar; recording-backed providers
    /// add the in-game date.
    fn hour_label(&self, hour: i64) -> String {
        format!("hour {hour}")
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

/// SYNTHETIC DEMO (behind `--demo-scrub`): NOT replay data. A
/// deterministic triangle sweep used only to exercise the scrub path:
/// over hours 0..=48, up to [`SWEEP_STATE_COUNT`] of the lowest-numbered
/// owned states flip one per hour to a deterministic other tag and then
/// flip back, ending identical to the 1936 baseline. No randomness.
pub struct SyntheticScrubDemo {
    hours: Vec<i64>,
    frames: Vec<Arc<BTreeMap<u32, String>>>,
}

/// Number of states swept by the synthetic demo.
pub const SWEEP_STATE_COUNT: usize = 24;

/// Total demo hours: 0..=48 (flip out for 24 hours, flip back for 24).
pub const DEMO_LAST_HOUR: i64 = 48;

impl SyntheticScrubDemo {
    pub fn new(
        base_owners: &BTreeMap<u32, String>,
        country_colors: &BTreeMap<String, [u8; 3]>,
    ) -> SyntheticScrubDemo {
        // Deterministic sweep set: the lowest state ids that have a 1936
        // owner and for which some *different* tag exists in
        // country_colors.json (so the flip is visible).
        let mut sweep: Vec<(u32, String)> = Vec::new();
        for (state_id, owner) in base_owners {
            if sweep.len() >= SWEEP_STATE_COUNT {
                break;
            }
            let target = country_colors.keys().find(|tag| *tag != owner);
            if let Some(target) = target {
                sweep.push((*state_id, target.clone()));
            }
        }

        let hours: Vec<i64> = (0..=DEMO_LAST_HOUR).collect();
        let mut frames = Vec::with_capacity(hours.len());
        for &hour in &hours {
            // Triangle sweep: hour 0 -> 0 flipped, hour 24 -> all flipped,
            // hour 48 -> 0 flipped again.
            let flipped = (hour.min(DEMO_LAST_HOUR - hour).max(0) as usize).min(sweep.len());
            let mut frame = base_owners.clone();
            for (state_id, target) in sweep.iter().take(flipped) {
                frame.insert(*state_id, target.clone());
            }
            frames.push(Arc::new(frame));
        }
        SyntheticScrubDemo { hours, frames }
    }

    fn frame_index(&self, hour: i64) -> usize {
        // Hours are contiguous 0..=48; clamp out-of-range scrub requests.
        hour.clamp(0, DEMO_LAST_HOUR) as usize
    }
}

impl OwnerProvider for SyntheticScrubDemo {
    fn hours(&self) -> &[i64] {
        &self.hours
    }

    fn owners_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>> {
        Arc::clone(&self.frames[self.frame_index(hour)])
    }

    fn controllers_at(&self, hour: i64) -> Arc<BTreeMap<u32, String>> {
        // SEMANTIC-ASSUMPTION: the synthetic scrub demo has no occupation;
        // controller and owner are the same synthetic visible political map.
        // Validation plan is the recording-backed occupied-territory path.
        Arc::clone(&self.frames[self.frame_index(hour)])
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
}

struct CachedFrame {
    hour: i64,
    owners: Arc<BTreeMap<u32, String>>,
    controllers: Arc<BTreeMap<u32, String>>,
    date: Option<String>,
}

impl RecordingTimeline {
    pub fn new(timeline: WorldTimeline) -> RecordingTimeline {
        RecordingTimeline {
            timeline,
            cache: Mutex::new(None),
        }
    }

    pub fn game_id(&self) -> &str {
        self.timeline.game_id()
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

    fn month_day_at(&self, hour: i64) -> Option<(u32, u32)> {
        let (_, _, date) = self.frame_maps(hour);
        month_day_from_clausewitz_date(date.as_deref()?)
    }

    fn hour_label(&self, hour: i64) -> String {
        match self.frame_maps(hour).2 {
            Some(date) => format!("{date} (hour {hour})"),
            None => format!("hour {hour}"),
        }
    }
}

/// Measured owners only; `Absent`/`Unobtained` stay off the map (sentinel).
pub fn owners_from_leaves(leaves: &BTreeMap<u32, Leaf<String>>) -> BTreeMap<u32, String> {
    leaves
        .iter()
        .filter_map(|(state_id, leaf)| leaf.measured().map(|tag| (*state_id, tag.clone())))
        .collect()
}

pub fn month_day_from_clausewitz_date(date: &str) -> Option<(u32, u32)> {
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

    fn fixture_colors() -> BTreeMap<String, [u8; 3]> {
        let mut colors = BTreeMap::new();
        colors.insert("FRA".to_string(), [1, 2, 3]);
        colors.insert("GER".to_string(), [4, 5, 6]);
        colors.insert("ITA".to_string(), [7, 8, 9]);
        colors
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
    fn demo_is_deterministic() {
        let a = SyntheticScrubDemo::new(&fixture_owners(), &fixture_colors());
        let b = SyntheticScrubDemo::new(&fixture_owners(), &fixture_colors());
        assert_eq!(a.hours(), b.hours());
        for &hour in a.hours() {
            assert_eq!(a.owners_at(hour), b.owners_at(hour), "hour {hour}");
            assert_eq!(
                a.controllers_at(hour),
                b.controllers_at(hour),
                "hour {hour}"
            );
        }
    }

    #[test]
    fn demo_first_and_last_hours_match_baseline() {
        let base = fixture_owners();
        let demo = SyntheticScrubDemo::new(&base, &fixture_colors());
        assert_eq!(demo.hours().len(), 49);
        assert_eq!(*demo.owners_at(0), base);
        assert_eq!(demo.controllers_at(0), demo.owners_at(0));
        assert_eq!(*demo.owners_at(DEMO_LAST_HOUR), base);
        assert_eq!(
            demo.controllers_at(DEMO_LAST_HOUR),
            demo.owners_at(DEMO_LAST_HOUR)
        );
    }

    #[test]
    fn demo_peak_flips_sweep_states_to_other_tags() {
        let base = fixture_owners();
        let demo = SyntheticScrubDemo::new(&base, &fixture_colors());
        let peak = demo.owners_at(24);
        assert_eq!(demo.controllers_at(24), peak);
        let changed: Vec<u32> = peak
            .iter()
            .filter(|(state_id, tag)| base.get(*state_id) != Some(*tag))
            .map(|(state_id, _)| *state_id)
            .collect();
        assert_eq!(changed.len(), SWEEP_STATE_COUNT);
        // Lowest owned state ids are swept first, deterministically.
        assert_eq!(changed[0], 1);
        for state_id in &changed {
            assert_ne!(peak.get(state_id), base.get(state_id));
        }
    }

    #[test]
    fn demo_ramps_one_state_per_hour() {
        let base = fixture_owners();
        let demo = SyntheticScrubDemo::new(&base, &fixture_colors());
        for hour in 0..=DEMO_LAST_HOUR {
            let expected = (hour.min(DEMO_LAST_HOUR - hour) as usize).min(SWEEP_STATE_COUNT);
            let frame = demo.owners_at(hour);
            let changed = frame
                .iter()
                .filter(|(state_id, tag)| base.get(*state_id) != Some(*tag))
                .count();
            assert_eq!(changed, expected, "hour {hour}");
        }
    }

    #[test]
    fn demo_clamps_out_of_range_hours() {
        let base = fixture_owners();
        let demo = SyntheticScrubDemo::new(&base, &fixture_colors());
        assert_eq!(demo.owners_at(-5), demo.owners_at(0));
        assert_eq!(demo.owners_at(500), demo.owners_at(DEMO_LAST_HOUR));
    }

    #[test]
    fn demo_with_empty_baseline_stays_empty() {
        let demo = SyntheticScrubDemo::new(&BTreeMap::new(), &fixture_colors());
        for &hour in demo.hours() {
            assert!(demo.owners_at(hour).is_empty());
        }
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
