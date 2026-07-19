//! F4 — mid-playback data-anomaly policy: **log + hold-last-good, never invent**.
//!
//! Startup fail-closed diagnostics (the `--validate-recording` path in
//! `main.rs`, the negative-fixture rejection matrix) reject a *malformed*
//! recording at open. This module covers the other case the plan (Phase F4)
//! calls out: a recording that OPENS cleanly but develops an anomaly
//! **mid-stream** — a domain that earlier snapshots carried goes missing, a
//! field flips type, a value falls out of range, an hourly snapshot is missing,
//! or the recording's identity mutates.
//!
//! Policy (Constraint 3, fail-closed honesty):
//!
//! * **Log** — every anomaly emits one structured [`Diagnostic`] the first time
//!   it is observed (it does not re-fire every hour it persists).
//! * **Hold last good, marked** — the last measured value is retained and
//!   surfaced as [`Hold`] (carrying the hour it is held *from* and the class),
//!   so a consumer renders it VISIBLY MARKED as held, never as if it were live.
//! * **Never invent** — an out-of-range value is flagged, never clamped; an
//!   hour gap scrubs as [`HourResolution::GapUnavailable`], never interpolated;
//!   a domain that never had a good value holds nothing.
//!
//! This lives entirely at the **consumer seam** (`replay_app`): it reads the
//! typed `replay_core::WorldFrame` views (`ViewStatus`, `Leaf`,
//! `coverage_warnings`, `save_version`, raw `provenance`) and never touches
//! recording bytes or `replay_core` internals. `replay_core` is read-only for
//! this lane; no additive `replay_core` API was required (see LANE_REPORT.md).

use std::collections::{BTreeMap, BTreeSet};

use replay_core::{Leaf, ViewStatus, WorldFrame, WorldTimeline};
use serde_json::Value;

/// The five mid-playback anomaly classes (plan F4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnomalyClass {
    /// A domain (typed view) that was `Present` in earlier snapshots is now
    /// `DomainAbsent`/`DomainUnavailable` (Absent-after-Present).
    MissingDomain,
    /// A field's JSON type flips mid-stream; the typed view goes `Unobtained`
    /// with a wrong-typed coverage warning.
    TypeFlip,
    /// A measured value falls outside its valid domain (negative manpower/fuel,
    /// a ratio outside `[0,1]`, or a province id absent from the map).
    OutOfRange,
    /// An hourly snapshot is missing from the middle of the series.
    HourGap,
    /// The recording's invariant identity (modset id / save version) mutates.
    IdentityMutation,
}

impl AnomalyClass {
    pub fn label(self) -> &'static str {
        match self {
            AnomalyClass::MissingDomain => "MISSING_DOMAIN",
            AnomalyClass::TypeFlip => "TYPE_FLIP",
            AnomalyClass::OutOfRange => "OUT_OF_RANGE",
            AnomalyClass::HourGap => "HOUR_GAP",
            AnomalyClass::IdentityMutation => "IDENTITY_MUTATION",
        }
    }
}

/// One structured anomaly diagnostic — emitted once per anomaly episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub class: AnomalyClass,
    /// The hour at which the anomaly was first observed.
    pub hour: i64,
    /// The monitored view / field / id.
    pub monitor: String,
    /// Human-readable detail (says what was held / flagged).
    pub detail: String,
}

impl Diagnostic {
    /// A single structured log line for stderr (the "log" side of the policy).
    pub fn log_line(&self) -> String {
        format!(
            "replay_app: ANOMALY {} @hour {} [{}]: {}",
            self.class.label(),
            self.hour,
            self.monitor,
            self.detail
        )
    }
}

/// A last-good value being held VISIBLY MARKED because its monitor is currently
/// anomalous. The consumer renders it as held (never as a live value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub class: AnomalyClass,
    pub monitor: String,
    /// The recorded hour the held value is carried from (the last good hour).
    pub since_hour: i64,
    /// A short marker string for the UI, e.g. "HELD weather from hour 41".
    pub marker: String,
}

/// Resolution of an arbitrary requested hour against the recorded series, so a
/// consumer that seeks to a non-recorded hour degrades honestly instead of
/// interpolating across a gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HourResolution {
    /// The requested hour is a recorded snapshot.
    Exact,
    /// The requested hour is before the first recorded frame.
    BeforeFirst,
    /// The requested hour lies inside a detected gap; it is unavailable and
    /// must NOT be interpolated. Carries the surrounding recorded hours and the
    /// missing hour(s).
    GapUnavailable {
        from: i64,
        to: i64,
        missing: Vec<i64>,
    },
}

/// A detected hole in the hourly series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub from: i64,
    pub to: i64,
    pub missing: Vec<i64>,
}

/// Domains whose `ViewStatus` is watched for the Absent-after-Present
/// (MissingDomain) transition. Ordered for deterministic diagnostics.
const MONITORED_DOMAINS: &[&str] = &[
    "countries",
    "weather",
    "snow_mud",
    "visibility",
    "buildings",
    "topbar",
    "naval_supremacy",
    "country_identities",
    "diplomacy",
];

/// Country scalar fields watched for the number->non-number TypeFlip.
const TYPE_FLIP_FIELDS: &[&str] = &["political_power", "fuel_current"];

fn view_status(frame: &WorldFrame, domain: &str) -> ViewStatus {
    match domain {
        "countries" => frame.countries_status(),
        "weather" => frame.weather_status(),
        "snow_mud" => frame.snow_mud_status(),
        "visibility" => frame.visibility_status(),
        "buildings" => frame.buildings_status(),
        "topbar" => frame.topbar_status(),
        "naval_supremacy" => frame.naval_supremacy_status(),
        "country_identities" => frame.country_identities_status(),
        "diplomacy" => frame.diplomacy().status,
        _ => ViewStatus::DomainAbsent,
    }
}

/// The recording's invariant identity fingerprint at one frame.
fn identity_fingerprint(frame: &WorldFrame) -> (Option<String>, Option<String>) {
    let save_version = frame.save_version().map(str::to_string);
    let modset = frame
        .domain("provenance")
        .and_then(|p| p.get("mod"))
        .and_then(|m| m.get("modset_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (save_version, modset)
}

/// The mid-playback anomaly policy. Observe recorded frames in order; it emits
/// a diagnostic once per anomaly episode and tracks the marked last-good holds.
pub struct AnomalyPolicy {
    hours: Vec<i64>,
    gaps: Vec<Gap>,
    /// Province ids that exist in the map (from the bundle). When set, a
    /// controlled province id absent from it is OutOfRange. `None` disables the
    /// province-id check (e.g. the static baseline with no recording map).
    known_province_ids: Option<BTreeSet<u32>>,

    // rolling state
    domain_present_seen: BTreeSet<String>,
    last_good_domain: BTreeMap<String, (i64, Value)>,
    last_good_scalar: BTreeMap<String, (i64, f64)>,
    identity_baseline: Option<(i64, (Option<String>, Option<String>))>,
    /// Monitors currently in an anomalous episode (so we log the transition
    /// once, not every hour it persists).
    active: BTreeMap<String, Hold>,

    // outputs
    diagnostics: Vec<Diagnostic>,
    /// Active holds as-of each observed hour, for scrub-time queries.
    hold_timeline: Vec<(i64, Vec<Hold>)>,
}

impl AnomalyPolicy {
    /// Build a policy for a recorded hour series. Gaps are precomputed so the
    /// hour-gap diagnostics are available immediately and are order-independent.
    pub fn with_known_provinces(
        hours: &[i64],
        known_province_ids: Option<BTreeSet<u32>>,
    ) -> AnomalyPolicy {
        let mut gaps = Vec::new();
        for pair in hours.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if b - a > 1 {
                gaps.push(Gap {
                    from: a,
                    to: b,
                    missing: ((a + 1)..b).collect(),
                });
            }
        }
        let mut policy = AnomalyPolicy {
            hours: hours.to_vec(),
            gaps,
            known_province_ids,
            domain_present_seen: BTreeSet::new(),
            last_good_domain: BTreeMap::new(),
            last_good_scalar: BTreeMap::new(),
            identity_baseline: None,
            active: BTreeMap::new(),
            diagnostics: Vec::new(),
            hold_timeline: Vec::new(),
        };
        // Hour-gap diagnostics are a property of the series, not of any frame.
        for gap in policy.gaps.clone() {
            policy.diagnostics.push(Diagnostic {
                class: AnomalyClass::HourGap,
                hour: gap.to,
                monitor: "timeline".to_string(),
                detail: format!(
                    "hourly snapshot(s) {:?} missing between hour {} and hour {}; \
                     the gap scrubs as unavailable, never interpolated",
                    gap.missing, gap.from, gap.to
                ),
            });
        }
        policy
    }

    pub fn gaps(&self) -> &[Gap] {
        &self.gaps
    }

    /// All diagnostics accumulated so far (hour-gap ones up front, then the
    /// per-frame anomalies in observation order).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Classify an arbitrary requested hour against the recorded series.
    pub fn classify_hour(&self, requested: i64) -> HourResolution {
        if self.hours.binary_search(&requested).is_ok() {
            return HourResolution::Exact;
        }
        match self.hours.first() {
            Some(&first) if requested < first => return HourResolution::BeforeFirst,
            None => return HourResolution::BeforeFirst,
            _ => {}
        }
        for gap in &self.gaps {
            if requested > gap.from && requested < gap.to {
                return HourResolution::GapUnavailable {
                    from: gap.from,
                    to: gap.to,
                    missing: gap.missing.clone(),
                };
            }
        }
        // Past the last recorded hour, or a sub-hour position between two
        // contiguous hours: hold-last-good is honest there (no gap).
        HourResolution::Exact
    }

    /// The marked holds active as of `hour` (the frame at-or-before it). Empty
    /// before the first observed frame. `hold_timeline` stores change-points
    /// in ascending hour order, so the last entry at-or-before `hour` is the
    /// active snapshot (binary search; one clone of the answer only).
    pub fn active_holds_at(&self, hour: i64) -> Vec<Hold> {
        let idx = self.hold_timeline.partition_point(|(h, _)| *h <= hour);
        if idx == 0 {
            Vec::new()
        } else {
            self.hold_timeline[idx - 1].1.clone()
        }
    }

    /// Observe one recorded frame (call in ascending recorded-hour order).
    /// Returns the diagnostics newly emitted for this frame.
    pub fn observe(&mut self, frame: &WorldFrame) -> Vec<Diagnostic> {
        let hour = frame.hour_index();
        let before = self.diagnostics.len();

        self.check_missing_domains(frame, hour);
        self.check_type_flips(frame, hour);
        self.check_out_of_range(frame, hour);
        self.check_identity(frame, hour);

        // Record the active-hold snapshot for scrub queries — CHANGE-POINTS
        // only (F2 memory audit): the old code appended one entry per
        // observed hour, an O(recorded-hours) growth even on a clean
        // multi-year recording (~26k entries for 3 years). An entry is now
        // appended only when the snapshot differs from the previous
        // change-point (re-observation of the same hour still overwrites).
        // `active_holds_at` answers by last-entry-at-or-before, so skipping
        // an identical snapshot is query-equivalent.
        let holds: Vec<Hold> = self.active.values().cloned().collect();
        match self.hold_timeline.last_mut() {
            Some(last) if last.0 == hour => last.1 = holds,
            Some(last) if last.1 == holds => {}
            Some(_) => self.hold_timeline.push((hour, holds)),
            // An initial all-clear snapshot is the same as no entry at all
            // (`active_holds_at` returns empty before the first entry).
            None if holds.is_empty() => {}
            None => self.hold_timeline.push((hour, holds)),
        }

        self.diagnostics[before..].to_vec()
    }

    fn fire(&mut self, class: AnomalyClass, hour: i64, monitor: String, detail: String) {
        self.diagnostics.push(Diagnostic {
            class,
            hour,
            monitor,
            detail,
        });
    }

    fn check_missing_domains(&mut self, frame: &WorldFrame, hour: i64) {
        for &domain in MONITORED_DOMAINS {
            let status = view_status(frame, domain);
            let key = format!("domain:{domain}");
            match status {
                ViewStatus::Present => {
                    self.domain_present_seen.insert(domain.to_string());
                    if let Some(value) = frame.domain(domain) {
                        self.last_good_domain
                            .insert(domain.to_string(), (hour, value.clone()));
                    }
                    // Recovered: clear an active hold so a later re-break re-logs.
                    self.active.remove(&key);
                }
                ViewStatus::DomainAbsent | ViewStatus::DomainUnavailable => {
                    // Only an anomaly if this domain was Present earlier.
                    if self.domain_present_seen.contains(domain) {
                        let since = self
                            .last_good_domain
                            .get(domain)
                            .map(|(h, _)| *h)
                            .unwrap_or(hour);
                        if !self.active.contains_key(&key) {
                            self.fire(
                                AnomalyClass::MissingDomain,
                                hour,
                                domain.to_string(),
                                format!(
                                    "view was Present earlier, now {}; holding last-good \
                                     {domain} from hour {since} marked HELD (never invented)",
                                    status_label(status)
                                ),
                            );
                        }
                        self.active.insert(
                            key,
                            Hold {
                                class: AnomalyClass::MissingDomain,
                                monitor: domain.to_string(),
                                since_hour: since,
                                marker: format!("HELD {domain} from hour {since}"),
                            },
                        );
                    }
                }
            }
        }
    }

    fn check_type_flips(&mut self, frame: &WorldFrame, hour: i64) {
        let warnings = frame.coverage_warnings();
        for (tag, country) in frame.countries() {
            for &field in TYPE_FLIP_FIELDS {
                let leaf = match field {
                    "political_power" => &country.political_power,
                    "fuel_current" => &country.fuel_current,
                    _ => continue,
                };
                let monitor = format!("countries.{tag}.{field}");
                let key = format!("typeflip:{monitor}");
                match leaf {
                    Leaf::Measured(v) => {
                        self.last_good_scalar.insert(monitor.clone(), (hour, *v));
                        self.active.remove(&key);
                    }
                    Leaf::Unobtained => {
                        // A wrong-typed value (not a plain null) is the flip: it
                        // both goes Unobtained AND appends a coverage warning.
                        let wrong_typed = warnings
                            .iter()
                            .any(|w| w.contains(field) && w.contains("wrong-typed"));
                        // Only after we had a good value here.
                        if wrong_typed && self.last_good_scalar.contains_key(&monitor) {
                            let (since, held) = self.last_good_scalar[&monitor];
                            if !self.active.contains_key(&key) {
                                self.fire(
                                    AnomalyClass::TypeFlip,
                                    hour,
                                    monitor.clone(),
                                    format!(
                                        "field type flipped (view now Unobtained + wrong-typed \
                                         warning); holding last-good value {held} from hour \
                                         {since} marked HELD (never coerced)"
                                    ),
                                );
                            }
                            self.active.insert(
                                key,
                                Hold {
                                    class: AnomalyClass::TypeFlip,
                                    monitor: monitor.clone(),
                                    since_hour: since,
                                    marker: format!("HELD {monitor}={held} from hour {since}"),
                                },
                            );
                        }
                    }
                    Leaf::Absent => {
                        self.active.remove(&key);
                    }
                }
            }
        }
    }

    fn check_out_of_range(&mut self, frame: &WorldFrame, hour: i64) {
        // Scalar ranges: manpower & fuel are non-negative; stability &
        // war-support are ratios in [0,1].
        for (tag, topbar) in frame.topbar_scalars_by_country() {
            if let Leaf::Measured(n) = topbar.manpower_available {
                let monitor = format!("countries.{tag}.manpower_available");
                self.range_check(hour, &monitor, n < 0, n as f64, |v| {
                    format!("manpower_available {v} is negative (out of range)")
                });
            }
        }
        for (tag, country) in frame.countries() {
            if let Leaf::Measured(f) = country.fuel_current {
                let monitor = format!("countries.{tag}.fuel_current");
                self.range_check(hour, &monitor, f < 0.0, f, |v| {
                    format!("fuel_current {v} is negative (out of range)")
                });
            }
            if let Leaf::Measured(s) = country.stability {
                let monitor = format!("countries.{tag}.stability");
                self.range_check(hour, &monitor, !(0.0..=1.0).contains(&s), s, |v| {
                    format!("stability {v} is outside the [0,1] ratio range")
                });
            }
            if let Leaf::Measured(w) = country.war_support {
                let monitor = format!("countries.{tag}.war_support");
                self.range_check(hour, &monitor, !(0.0..=1.0).contains(&w), w, |v| {
                    format!("war_support {v} is outside the [0,1] ratio range")
                });
            }
        }
        // Province ids: any controlled province absent from the map universe.
        // Collect first (immutable borrow of self) before mutating (fire/active).
        let stray: Vec<u32> = match &self.known_province_ids {
            Some(known) => frame
                .control_by_province()
                .keys()
                .copied()
                .filter(|id| !known.contains(id))
                .collect(),
            None => Vec::new(),
        };
        for province_id in stray {
            let monitor = format!("map.control_by_province.{province_id}");
            let key = format!("range:{monitor}");
            if !self.active.contains_key(&key) {
                self.fire(
                    AnomalyClass::OutOfRange,
                    hour,
                    monitor.clone(),
                    format!(
                        "province id {province_id} is not in the map; flagged, \
                         not rendered (never invented)"
                    ),
                );
            }
            self.active.insert(
                key,
                Hold {
                    class: AnomalyClass::OutOfRange,
                    monitor,
                    since_hour: hour,
                    marker: format!("FLAGGED stray province {province_id}"),
                },
            );
        }
    }

    fn range_check<F>(
        &mut self,
        hour: i64,
        monitor: &str,
        out_of_range: bool,
        value: f64,
        detail: F,
    ) where
        F: FnOnce(f64) -> String,
    {
        let key = format!("range:{monitor}");
        if out_of_range {
            let since = self
                .last_good_scalar
                .get(monitor)
                .map(|(h, _)| *h)
                .unwrap_or(hour);
            if !self.active.contains_key(&key) {
                let held = self
                    .last_good_scalar
                    .get(monitor)
                    .map(|(_, v)| format!("; holding last-good {v} from hour {since} marked HELD"))
                    .unwrap_or_else(|| "; no prior good value to hold".to_string());
                self.fire(
                    AnomalyClass::OutOfRange,
                    hour,
                    monitor.to_string(),
                    format!("{}{held}", detail(value)),
                );
            }
            self.active.insert(
                key,
                Hold {
                    class: AnomalyClass::OutOfRange,
                    monitor: monitor.to_string(),
                    since_hour: since,
                    marker: format!("HELD {monitor} from hour {since} (last value out of range)"),
                },
            );
        } else {
            self.last_good_scalar
                .insert(monitor.to_string(), (hour, value));
            self.active.remove(&key);
        }
    }

    fn check_identity(&mut self, frame: &WorldFrame, hour: i64) {
        let fp = identity_fingerprint(frame);
        match &self.identity_baseline {
            None => self.identity_baseline = Some((hour, fp)),
            Some((base_hour, baseline)) => {
                let base_hour = *base_hour;
                if fp != *baseline {
                    let key = "identity".to_string();
                    if !self.active.contains_key(&key) {
                        self.fire(
                            AnomalyClass::IdentityMutation,
                            hour,
                            "recording_identity".to_string(),
                            format!(
                                "identity mutated from {baseline:?} (hour {base_hour}) to {fp:?}; \
                                 holding baseline identity marked HELD (recording identity must be \
                                 invariant)"
                            ),
                        );
                    }
                    self.active.insert(
                        key,
                        Hold {
                            class: AnomalyClass::IdentityMutation,
                            monitor: "recording_identity".to_string(),
                            since_hour: base_hour,
                            marker: format!("HELD baseline identity from hour {base_hour}"),
                        },
                    );
                }
            }
        }
    }
}

fn status_label(status: ViewStatus) -> &'static str {
    match status {
        ViewStatus::Present => "Present",
        ViewStatus::DomainAbsent => "DomainAbsent",
        ViewStatus::DomainUnavailable => "DomainUnavailable",
    }
}

/// Convenience: run the policy over every recorded frame of a timeline in order
/// (the "scan at open" the app uses to log mid-playback anomalies). Returns the
/// populated policy; frame-load errors are surfaced as a coverage note and
/// never panic.
pub fn scan_timeline(
    timeline: &WorldTimeline,
    known_province_ids: Option<BTreeSet<u32>>,
) -> AnomalyPolicy {
    let mut policy = AnomalyPolicy::with_known_provinces(timeline.hours(), known_province_ids);
    for &hour in timeline.hours() {
        match timeline.frame(hour) {
            Ok(frame) => {
                policy.observe(&frame);
            }
            Err(_) => {
                // A load error at a recorded hour is itself an anomaly; hold
                // (skip) rather than invent a frame.
                policy.fire(
                    AnomalyClass::MissingDomain,
                    hour,
                    "frame".to_string(),
                    "recorded frame failed to load; skipped (no fabrication)".to_string(),
                );
            }
        }
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::open_recording;
    use serde_json::Value as JsonValue;
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures")
    }

    fn manifest() -> JsonValue {
        let path = fixture_root().join("manifest.json");
        let text = std::fs::read_to_string(&path).expect("read manifest");
        serde_json::from_str(&text).expect("manifest json")
    }

    fn anomaly_entry(name: &str) -> JsonValue {
        manifest()["anomaly"]
            .as_array()
            .expect("anomaly section")
            .iter()
            .find(|e| e["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("anomaly fixture {name} not listed"))
            .clone()
    }

    fn open_anomaly(name: &str) -> WorldTimeline {
        let entry = anomaly_entry(name);
        let rel = entry["path"].as_str().expect("path");
        let source = open_recording(&fixture_root().join(rel)).unwrap_or_else(|e| {
            panic!("{name}: anomaly fixture must OPEN (degrades, not rejects): {e}")
        });
        WorldTimeline::open(source).unwrap_or_else(|e| panic!("{name}: timeline: {e}"))
    }

    fn known_provinces() -> BTreeSet<u32> {
        [1000u32, 1001, 2000, 3000].into_iter().collect()
    }

    fn diagnostics_of(policy: &AnomalyPolicy, class: AnomalyClass) -> Vec<&Diagnostic> {
        policy
            .diagnostics()
            .iter()
            .filter(|d| d.class == class)
            .collect()
    }

    /// Every anomaly fixture OPENS (the anomaly/ set degrades, it does not
    /// reject like the negative/ set) and playback crosses every hour with no
    /// panic.
    #[test]
    fn all_anomaly_fixtures_open_and_play_without_panic() {
        let names = [
            "fx_anomaly_missing_domain",
            "fx_anomaly_type_flip",
            "fx_anomaly_out_of_range",
            "fx_anomaly_hour_gap",
            "fx_anomaly_identity_mutation",
        ];
        for name in names {
            let timeline = open_anomaly(name);
            let policy = scan_timeline(&timeline, Some(known_provinces()));
            // Crossed every recorded hour; holds queryable at each.
            for &hour in timeline.hours() {
                let _ = policy.active_holds_at(hour);
            }
        }
    }

    #[test]
    fn missing_domain_fires_once_and_holds_marked() {
        let timeline = open_anomaly("fx_anomaly_missing_domain");
        let entry = anomaly_entry("fx_anomaly_missing_domain");
        let crossing = entry["anomaly"]["crossing_hour"].as_i64().unwrap();
        let last_good = entry["anomaly"]["last_good_hour"].as_i64().unwrap();
        let policy = scan_timeline(&timeline, None);

        let hits = diagnostics_of(&policy, AnomalyClass::MissingDomain);
        assert_eq!(hits.len(), 1, "exactly one MissingDomain diagnostic");
        assert_eq!(hits[0].monitor, "weather");
        assert_eq!(hits[0].hour, crossing, "fires at the crossing hour");

        // Pre-anomaly hours unaffected: no holds active before the crossing.
        assert!(
            policy.active_holds_at(last_good).is_empty(),
            "pre-anomaly hour has no holds"
        );
        // At and after the crossing, the weather hold is present and MARKED.
        let held = policy.active_holds_at(crossing);
        let weather = held
            .iter()
            .find(|h| h.monitor == "weather")
            .expect("weather held at crossing");
        assert_eq!(weather.class, AnomalyClass::MissingDomain);
        assert_eq!(weather.since_hour, last_good);
        assert!(weather.marker.contains("HELD"), "held value is marked");
    }

    #[test]
    fn type_flip_fires_once_and_holds_last_number() {
        let timeline = open_anomaly("fx_anomaly_type_flip");
        let entry = anomaly_entry("fx_anomaly_type_flip");
        let crossing = entry["anomaly"]["crossing_hour"].as_i64().unwrap();
        let last_good = entry["anomaly"]["last_good_hour"].as_i64().unwrap();
        let policy = scan_timeline(&timeline, None);

        let hits = diagnostics_of(&policy, AnomalyClass::TypeFlip);
        assert_eq!(hits.len(), 1, "exactly one TypeFlip diagnostic");
        assert_eq!(hits[0].monitor, "countries.GER.political_power");
        assert_eq!(hits[0].hour, crossing);

        assert!(policy.active_holds_at(last_good).is_empty());
        let held = policy.active_holds_at(crossing);
        let flip = held
            .iter()
            .find(|h| h.monitor == "countries.GER.political_power")
            .expect("political_power held");
        assert_eq!(flip.class, AnomalyClass::TypeFlip);
        assert_eq!(flip.since_hour, last_good);
        assert!(flip.marker.contains("HELD"));
    }

    #[test]
    fn out_of_range_flags_negative_manpower_and_stray_province() {
        let timeline = open_anomaly("fx_anomaly_out_of_range");
        let entry = anomaly_entry("fx_anomaly_out_of_range");
        let crossing = entry["anomaly"]["crossing_hour"].as_i64().unwrap();
        let last_good = entry["anomaly"]["last_good_hour"].as_i64().unwrap();
        let policy = scan_timeline(&timeline, Some(known_provinces()));

        let hits = diagnostics_of(&policy, AnomalyClass::OutOfRange);
        // Two distinct out-of-range monitors: negative manpower + stray province.
        let manpower = hits
            .iter()
            .find(|d| d.monitor == "countries.GER.manpower_available")
            .expect("manpower out-of-range diagnostic");
        assert_eq!(manpower.hour, crossing);
        let province = hits
            .iter()
            .find(|d| d.monitor == "map.control_by_province.987654")
            .expect("stray province diagnostic");
        assert_eq!(province.hour, crossing);
        // Each fires exactly once (does not re-fire every subsequent hour).
        assert_eq!(
            hits.iter()
                .filter(|d| d.monitor == "countries.GER.manpower_available")
                .count(),
            1
        );
        assert_eq!(
            hits.iter()
                .filter(|d| d.monitor == "map.control_by_province.987654")
                .count(),
            1
        );

        // Pre-anomaly unaffected; manpower held from the last good hour after.
        assert!(policy.active_holds_at(last_good).is_empty());
        let held = policy.active_holds_at(crossing);
        assert!(held
            .iter()
            .any(|h| h.monitor == "countries.GER.manpower_available"
                && h.class == AnomalyClass::OutOfRange
                && h.since_hour == last_good));
    }

    #[test]
    fn hour_gap_is_unavailable_not_interpolated() {
        let timeline = open_anomaly("fx_anomaly_hour_gap");
        let entry = anomaly_entry("fx_anomaly_hour_gap");
        let anomaly = &entry["anomaly"];
        let missing = anomaly["missing_hours"][0].as_i64().unwrap();
        let before = anomaly["before_gap_hour"].as_i64().unwrap();
        let after = anomaly["after_gap_hour"].as_i64().unwrap();

        let policy = scan_timeline(&timeline, None);
        let hits = diagnostics_of(&policy, AnomalyClass::HourGap);
        assert_eq!(hits.len(), 1, "exactly one HourGap diagnostic");
        assert_eq!(hits[0].hour, after);

        // The gap detector saw exactly the missing hour.
        assert_eq!(policy.gaps().len(), 1);
        assert_eq!(policy.gaps()[0].missing, vec![missing]);

        // Recorded hours resolve exactly; the missing hour is GapUnavailable,
        // never interpolated to the neighbour frame.
        assert_eq!(policy.classify_hour(before), HourResolution::Exact);
        assert_eq!(policy.classify_hour(after), HourResolution::Exact);
        match policy.classify_hour(missing) {
            HourResolution::GapUnavailable {
                from,
                to,
                missing: m,
            } => {
                assert_eq!(from, before);
                assert_eq!(to, after);
                assert_eq!(m, vec![missing]);
            }
            other => panic!("expected GapUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn identity_mutation_fires_once_and_holds_baseline() {
        let timeline = open_anomaly("fx_anomaly_identity_mutation");
        let entry = anomaly_entry("fx_anomaly_identity_mutation");
        let crossing = entry["anomaly"]["crossing_hour"].as_i64().unwrap();
        let baseline_hour = timeline.first_hour();
        let policy = scan_timeline(&timeline, None);

        let hits = diagnostics_of(&policy, AnomalyClass::IdentityMutation);
        assert_eq!(hits.len(), 1, "exactly one IdentityMutation diagnostic");
        assert_eq!(hits[0].hour, crossing);

        assert!(policy
            .active_holds_at(entry["anomaly"]["last_good_hour"].as_i64().unwrap())
            .is_empty());
        let held = policy.active_holds_at(crossing);
        let id = held
            .iter()
            .find(|h| h.monitor == "recording_identity")
            .expect("identity held");
        assert_eq!(id.class, AnomalyClass::IdentityMutation);
        assert_eq!(id.since_hour, baseline_hour);
        assert!(id.marker.contains("HELD"));
    }

    /// A clean positive fixture raises no anomalies (no false positives): the
    /// pre-anomaly behaviour of the policy is silent on well-formed recordings.
    #[test]
    fn clean_positive_fixture_raises_no_anomalies() {
        let root = fixture_root();
        let rel = manifest()["positive"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"].as_str() == Some("fx_owner_controller_flip"))
            .unwrap()["path"]
            .as_str()
            .unwrap()
            .to_string();
        let source = open_recording(&root.join(rel)).unwrap();
        let timeline = WorldTimeline::open(source).unwrap();
        let policy = scan_timeline(&timeline, Some(known_provinces()));
        assert!(
            policy.diagnostics().is_empty(),
            "clean recording must raise no anomalies, got {:?}",
            policy.diagnostics()
        );
        for &hour in timeline.hours() {
            assert!(policy.active_holds_at(hour).is_empty());
        }
    }

    #[test]
    fn diagnostic_log_line_is_structured() {
        let d = Diagnostic {
            class: AnomalyClass::MissingDomain,
            hour: 42,
            monitor: "weather".to_string(),
            detail: "held".to_string(),
        };
        let line = d.log_line();
        assert!(line.contains("ANOMALY MISSING_DOMAIN"));
        assert!(line.contains("@hour 42"));
        assert!(line.contains("[weather]"));
    }
}
