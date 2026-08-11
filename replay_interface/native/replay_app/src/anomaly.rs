//! Mid-playback data-anomaly policy: **log + hold-last-good, never invent**.
//!
//! Startup fail-closed diagnostics (the `--validate-recording` path in
//! `main.rs`, the negative-fixture rejection matrix) reject a *malformed*
//! recording at open. This module covers a recording that opens cleanly but
//! develops an anomaly
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
//! recording bytes or `replay_core` internals. Declared recording gaps remain
//! [`HourResolution::GapUnavailable`]; hold-last-good never substitutes a stale
//! frame for a declared gap.

use std::collections::{BTreeMap, BTreeSet};

use replay_core::{Leaf, ViewStatus, WorldFrame, WorldTimeline};
use serde_json::Value;

/// The five mid-playback anomaly classes.
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

/// A recorded index entry whose payload cannot be loaded and authenticated.
/// This is terminal, not a hold-last-good anomaly: holding across an integrity
/// failure would make an unverified recording appear playable.
///
/// Constructed only by [`scan_timeline`], the whole-timeline library mirror of
/// the product walk. The product path reports the same condition as
/// [`crate::timeline::TimelineSelectionError::FrameLoad`]; the two are
/// deliberately separate types because only this one can name the scan that
/// found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineScanError {
    pub hour: i64,
    pub detail: String,
}

impl std::fmt::Display for TimelineScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "recorded frame at canonical hour {} failed payload integrity/decode validation: {}; refusing hold-last-good across an unauthenticated frame",
            self.hour, self.detail
        )
    }
}

impl std::error::Error for TimelineScanError {}

/// Domains whose `ViewStatus` is watched for the Absent-after-Present
/// (MissingDomain) transition. Ordered for deterministic diagnostics.
const MONITORED_DOMAINS: &[&str] = &[
    "countries",
    "weather",
    "snow_mud",
    // The province ground-cover texture carries its own availability, split
    // from the snow_mud view status (which also covers the fade partition):
    // the texture can go absent/unobtained/wrong-typed mid-stream while the
    // view stays Present, and that downgrade must surface, not read as an
    // empty measured texture.
    "snow_mud.by_province",
    "visibility",
    "buildings",
    "topbar",
    "naval_supremacy",
    "country_identities",
    "diplomacy",
];

/// Country scalar fields watched for the number->non-number TypeFlip.
const TYPE_FLIP_FIELDS: &[&str] = &["political_power", "fuel_current"];

/// The raw JSON value backing a monitored domain, used only to remember the
/// last-good hour for the hold marker. Most monitors are top-level raw keys;
/// the snow_mud pair lives nested under `weather` (and the texture partition
/// one level deeper), so a plain top-level lookup would never record a last
/// good value and the hold would claim the FAILING hour as its `since_hour`.
fn monitored_domain_value<'a>(frame: &'a WorldFrame, domain: &str) -> Option<&'a Value> {
    match domain {
        "snow_mud" => frame.domain("weather")?.get("snow_mud"),
        "snow_mud.by_province" => frame.domain("weather")?.get("snow_mud")?.get("texture"),
        _ => frame.domain(domain),
    }
}

fn view_status(frame: &WorldFrame, domain: &str) -> ViewStatus {
    match domain {
        "countries" => frame.countries_status(),
        "weather" => frame.weather_status(),
        "snow_mud" => frame.snow_mud_status(),
        "snow_mud.by_province" => frame.snow_mud().by_province_status,
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
    /// Last recorded hour each monitored domain's TYPED view was `Present`.
    /// Presence of the typed view — not of a raw top-level key — is what the
    /// viewer holds: derived domains (e.g. `topbar`, which is keyed off
    /// `countries`) are `Present` with no raw key of their own, and the hold
    /// provenance must still name the real last-good hour, never the breaking
    /// hour.
    last_present_hour: BTreeMap<String, i64>,
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
            last_present_hour: BTreeMap::new(),
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

        // Record only active-hold change-points for scrub queries. Appending
        // every observed hour would grow O(recorded-hours) even on a clean
        // recording; an entry is appended only when the snapshot differs from
        // the previous change-point (re-observation of the same hour still
        // overwrites).
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
                    // The typed view is the presence authority. A raw-key
                    // probe here (`frame.domain(domain)`) would MISS derived
                    // domains such as `topbar` (Present from `countries`,
                    // no raw `topbar` key) and a later loss would then
                    // fabricate the breaking hour as the hold origin.
                    self.last_present_hour.insert(domain.to_string(), hour);
                    // A Present view clears its active hold; a later loss then
                    // emits a new transition.
                    self.active.remove(&key);
                }
                ViewStatus::DomainAbsent | ViewStatus::DomainUnavailable => {
                    // Only an anomaly if this domain was Present earlier; the
                    // hold provenance is the exact hour it was last Present,
                    // never invented from the breaking hour.
                    if let Some(&since) = self.last_present_hour.get(domain) {
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
                        // Coverage warnings are country-qualified JSON paths
                        // (world.rs emits `countries.{tag}.<walked path>: present
                        // but wrong-typed ...`), so match this country's exact
                        // path — matching the bare field name would attribute
                        // one country's flip to every other country whose leaf
                        // is plain-null Unobtained in the same frame.
                        let warn_path = match field {
                            "political_power" => {
                                format!("countries.{tag}.politics.political_power")
                            }
                            "fuel_current" => {
                                format!("countries.{tag}.economy.fuel.current")
                            }
                            _ => continue,
                        };
                        let wrong_typed = warnings
                            .iter()
                            .any(|w| w.starts_with(&warn_path) && w.contains("wrong-typed"));
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
            // The marker may claim a HELD value only when one was actually
            // captured. An out-of-range value with no prior good
            // capture is FLAGGED, never presented as held from the breaking
            // hour (that provenance would be invented).
            let last_good = self.last_good_scalar.get(monitor).map(|(h, _)| *h);
            let since = last_good.unwrap_or(hour);
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
            let marker = match last_good {
                Some(good_hour) => {
                    format!("HELD {monitor} from hour {good_hour} (last value out of range)")
                }
                None => format!("FLAGGED {monitor} out of range (no prior good value to hold)"),
            };
            self.active.insert(
                key,
                Hold {
                    class: AnomalyClass::OutOfRange,
                    monitor: monitor.to_string(),
                    since_hour: since,
                    marker,
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

/// Library convenience: run the policy over every recorded frame of a timeline
/// in order, and return the populated policy.
///
/// **This is not the product path.** The app never scans a whole timeline at
/// open; it observes frames incrementally through the requested hour in
/// [`crate::timeline::RecordingTimeline`] (`scan_anomalies_through`), which is
/// what every scrub entry point reaches. This function is the whole-timeline
/// mirror of that walk: same [`AnomalyPolicy`], same ordering, same terminal
/// treatment of a load failure. It exists so the policy can be exercised
/// against a fixture without standing up a `RecordingTimeline`, and the
/// anomaly-policy test battery below is its only caller.
///
/// The one behaviour it must keep identical to the product walk: a frame-load
/// integrity/decode error is terminal. It is returned to the caller and is
/// never downgraded to a hold-last-good anomaly.
pub fn scan_timeline(
    timeline: &WorldTimeline,
    known_province_ids: Option<BTreeSet<u32>>,
) -> Result<AnomalyPolicy, TimelineScanError> {
    let mut policy = AnomalyPolicy::with_known_provinces(timeline.hours(), known_province_ids);
    for &hour in timeline.hours() {
        match timeline.frame(hour) {
            Ok(frame) => {
                policy.observe(&frame);
            }
            Err(error) => {
                // SHA/CRC/decode failure is an authentication boundary, not a
                // semantic mid-stream anomaly. Never continue with a held
                // value after bytes failed verification.
                return Err(TimelineScanError {
                    hour,
                    detail: error.to_string(),
                });
            }
        }
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{open_recording, RawFrame, ReplaySource, SourceError};
    use serde_json::Value as JsonValue;
    use std::path::PathBuf;

    /// Minimal in-memory source for tests that need a precise frame shape
    /// (e.g. two countries diverging within one frame) that no on-disk
    /// anomaly fixture exercises.
    struct MemorySource {
        hours: Vec<i64>,
        frames: BTreeMap<i64, JsonValue>,
    }

    impl ReplaySource for MemorySource {
        fn game_id(&self) -> &str {
            "mem_test"
        }
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn warnings(&self) -> &[String] {
            &[]
        }
        fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
            let obj = self
                .frames
                .get(&hour_index)
                .and_then(JsonValue::as_object)
                .ok_or_else(|| SourceError::Open(format!("no frame at hour {hour_index}")))?;
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: obj.clone(),
            })
        }
    }

    struct IntegrityFailureSource {
        hours: Vec<i64>,
    }

    impl ReplaySource for IntegrityFailureSource {
        fn game_id(&self) -> &str {
            "integrity_failure"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
            if hour_index == 2 {
                return Err(SourceError::Open(
                    "snapshot sha256 mismatch: expected aa, observed bb".into(),
                ));
            }
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: Default::default(),
            })
        }
    }

    #[test]
    fn frame_integrity_failure_is_terminal_not_a_hold() {
        let timeline = WorldTimeline::open(Box::new(IntegrityFailureSource {
            hours: vec![0, 1, 2, 3],
        }))
        .expect("open synthetic timeline");
        let error = match scan_timeline(&timeline, None) {
            Err(error) => error,
            Ok(_) => panic!("SHA mismatch must reject scan"),
        };
        assert_eq!(error.hour, 2);
        assert!(error.detail.contains("sha256 mismatch"), "{error}");
        assert!(
            error
                .to_string()
                .contains("refusing hold-last-good across an unauthenticated frame"),
            "{error}"
        );
    }

    /// `topbar` is a DERIVED monitored domain — its typed
    /// view is `Present` off the `countries` object with no raw `topbar` key.
    /// When it later breaks, the hold provenance must name the hour it was
    /// last Present, never fabricate the BREAKING hour as the hold origin.
    #[test]
    fn derived_domain_hold_names_the_last_present_hour_not_the_breaking_hour() {
        let timeline = WorldTimeline::open(Box::new(MemorySource {
            hours: vec![0, 1, 2],
            frames: BTreeMap::from([
                (0, serde_json::json!({ "countries": { "GER": {} } })),
                (1, serde_json::json!({ "countries": { "GER": {} } })),
                // countries (and therefore derived topbar) lost at hour 2.
                (2, serde_json::json!({})),
            ]),
        }))
        .expect("open synthetic timeline");

        // The typed topbar view is Present while the raw `topbar` key is
        // absent.
        let first = timeline.frame(0).expect("first frame");
        assert_eq!(first.topbar_status(), replay_core::ViewStatus::Present);
        assert!(
            first.domain("topbar").is_none(),
            "fixture must exercise the derived-domain path (no raw topbar key)"
        );

        let policy = scan_timeline(&timeline, None).expect("scan");
        let holds = policy.active_holds_at(2);
        let topbar_hold = holds
            .iter()
            .find(|hold| hold.monitor == "topbar")
            .expect("topbar loss after Present must hold");
        assert_eq!(
            topbar_hold.since_hour, 1,
            "hold provenance is the last Present hour, not the breaking hour"
        );
        assert_eq!(topbar_hold.marker, "HELD topbar from hour 1");
        let diagnostic = diagnostics_of(&policy, AnomalyClass::MissingDomain)
            .into_iter()
            .find(|d| d.monitor == "topbar")
            .expect("topbar MissingDomain diagnostic");
        assert!(
            diagnostic.detail.contains("from hour 1"),
            "diagnostic names the honest hold origin: {}",
            diagnostic.detail
        );
        assert_eq!(diagnostic.hour, 2, "the transition is logged at the break");
    }

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
            let policy = scan_timeline(&timeline, Some(known_provinces())).expect("scan fixture");
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
        let policy = scan_timeline(&timeline, None).expect("scan fixture");

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
        let policy = scan_timeline(&timeline, None).expect("scan fixture");

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

    /// The wrong-typed coverage warning is country-qualified
    /// (`countries.{tag}.politics.political_power: present but wrong-typed
    /// ...`). A genuinely wrong-typed political_power in one country must NOT
    /// be attributed to another country whose political_power is plain-null
    /// Unobtained in the same frame (both with prior measured values):
    /// exactly one TypeFlip diagnostic and one held marker, for the
    /// wrong-typed country only.
    #[test]
    fn type_flip_is_not_cross_attributed_between_countries() {
        let frame0 = serde_json::json!({
            "countries": {
                "GER": { "politics": { "political_power": 12.5 } },
                "ITA": { "politics": { "political_power": 34.0 } }
            }
        });
        let frame1 = serde_json::json!({
            "countries": {
                // Genuinely wrong-typed: view goes Unobtained AND a
                // wrong-typed coverage warning is appended.
                "GER": { "politics": { "political_power": "banana" } },
                // Plain null: captured-but-unobtained, no warning, no flip.
                "ITA": { "politics": { "political_power": null } }
            }
        });
        let source = MemorySource {
            hours: vec![0, 1],
            frames: [(0, frame0), (1, frame1)].into_iter().collect(),
        };
        let timeline = WorldTimeline::open(Box::new(source)).expect("timeline");
        let policy = scan_timeline(&timeline, None).expect("scan fixture");

        let hits = diagnostics_of(&policy, AnomalyClass::TypeFlip);
        assert_eq!(hits.len(), 1, "exactly one TypeFlip diagnostic: {hits:?}");
        assert_eq!(hits[0].monitor, "countries.GER.political_power");
        assert_eq!(hits[0].hour, 1);

        assert!(policy.active_holds_at(0).is_empty());
        let held: Vec<Hold> = policy
            .active_holds_at(1)
            .into_iter()
            .filter(|h| h.class == AnomalyClass::TypeFlip)
            .collect();
        assert_eq!(held.len(), 1, "exactly one TypeFlip hold: {held:?}");
        assert_eq!(held[0].monitor, "countries.GER.political_power");
        assert_eq!(held[0].since_hour, 0);
        assert!(held[0].marker.contains("HELD"));
    }

    /// FW1 status consumer: the per-province snow/mud semantic collection has
    /// availability (`SnowMudView::by_province_status`), split from the view
    /// status that also covers the fade partition. A mid-stream collection
    /// downgrade under a still-`Present` snow_mud view must surface as a
    /// MissingDomain hold on the legacy-key monitor — not read as an empty
    /// measured semantic census, and not fire the whole-view monitor.
    #[test]
    fn snow_mud_texture_downgrade_surfaces_under_present_view() {
        let textured = serde_json::json!({
            "weather": { "snow_mud": {
                "texture": [
                    { "province_id": 1000, "region_id": 1,
                      "snow_level_0_255": 40, "mud_level_0_255": 0 }
                ],
                "fade": { "phase": 0.5, "fade_0_hour": 1, "fade_1_hour": 2 }
            } }
        });
        // The view stays Present (the snow_mud object and fade partition are
        // still there) while the texture partition goes explicitly unobtained.
        let texture_lost = serde_json::json!({
            "weather": { "snow_mud": {
                "texture": null,
                "fade": { "phase": 0.5, "fade_0_hour": 1, "fade_1_hour": 2 }
            } }
        });
        let source = MemorySource {
            hours: vec![0, 1],
            frames: [(0, textured), (1, texture_lost)].into_iter().collect(),
        };
        let timeline = WorldTimeline::open(Box::new(source)).expect("timeline");

        // Preconditions: the stale-retained-state shape really is Present at
        // the view root with a downgraded texture partition.
        let frame1 = timeline.frame(1).expect("frame 1");
        assert_eq!(frame1.snow_mud_status(), ViewStatus::Present);
        assert_eq!(
            frame1.snow_mud().by_province_status,
            ViewStatus::DomainUnavailable
        );

        let policy = scan_timeline(&timeline, None).expect("scan");
        let hits = diagnostics_of(&policy, AnomalyClass::MissingDomain);
        assert_eq!(hits.len(), 1, "exactly one texture diagnostic: {hits:?}");
        assert_eq!(hits[0].monitor, "snow_mud.by_province");
        assert_eq!(hits[0].hour, 1);
        assert!(
            hits[0].detail.contains("from hour 0"),
            "the diagnostic names the LAST GOOD hour, not the failing one: {}",
            hits[0].detail
        );

        assert!(policy.active_holds_at(0).is_empty());
        let held = policy.active_holds_at(1);
        let texture = held
            .iter()
            .find(|h| h.monitor == "snow_mud.by_province")
            .expect("texture held at the downgrade hour");
        assert_eq!(texture.class, AnomalyClass::MissingDomain);
        assert_eq!(
            texture.since_hour, 0,
            "the hold carries the last-good hour, never the failing hour"
        );
        assert!(texture.marker.contains("HELD"));
        assert!(
            !held.iter().any(|h| h.monitor == "snow_mud"),
            "the whole-view monitor stays clean while only the texture broke"
        );
    }

    #[test]
    fn out_of_range_flags_negative_manpower_and_stray_province() {
        let timeline = open_anomaly("fx_anomaly_out_of_range");
        let entry = anomaly_entry("fx_anomaly_out_of_range");
        let crossing = entry["anomaly"]["crossing_hour"].as_i64().unwrap();
        let last_good = entry["anomaly"]["last_good_hour"].as_i64().unwrap();
        let policy = scan_timeline(&timeline, Some(known_provinces())).expect("scan fixture");

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

    /// An out-of-range scalar with NO prior good capture is
    /// FLAGGED — the hold marker must not claim "HELD ... from hour X" where
    /// X is the breaking hour and nothing was ever captured to hold.
    #[test]
    fn out_of_range_without_prior_good_value_is_flagged_not_held() {
        let timeline = WorldTimeline::open(Box::new(MemorySource {
            hours: vec![0, 1],
            frames: BTreeMap::from([
                // Out of range from the very first observation.
                (
                    0,
                    serde_json::json!({
                        "countries": { "GER": { "politics": { "stability": 1.5 } } }
                    }),
                ),
                (
                    1,
                    serde_json::json!({
                        "countries": { "GER": { "politics": { "stability": 1.5 } } }
                    }),
                ),
            ]),
        }))
        .expect("open synthetic timeline");
        let policy = scan_timeline(&timeline, None).expect("scan");
        let holds = policy.active_holds_at(1);
        let hold = holds
            .iter()
            .find(|h| h.monitor == "countries.GER.stability")
            .expect("out-of-range stability is tracked");
        assert!(
            hold.marker.starts_with("FLAGGED"),
            "no prior good value: marker must flag, not claim a hold: {}",
            hold.marker
        );
        assert!(
            !hold.marker.contains("HELD"),
            "a HELD claim requires a captured value: {}",
            hold.marker
        );
        let diagnostic = diagnostics_of(&policy, AnomalyClass::OutOfRange)
            .into_iter()
            .find(|d| d.monitor == "countries.GER.stability")
            .expect("diagnostic fired");
        assert!(
            diagnostic.detail.contains("no prior good value to hold"),
            "{}",
            diagnostic.detail
        );
    }

    #[test]
    fn hour_gap_is_unavailable_not_interpolated() {
        let timeline = open_anomaly("fx_anomaly_hour_gap");
        let entry = anomaly_entry("fx_anomaly_hour_gap");
        let anomaly = &entry["anomaly"];
        let missing = anomaly["missing_hours"][0].as_i64().unwrap();
        let before = anomaly["before_gap_hour"].as_i64().unwrap();
        let after = anomaly["after_gap_hour"].as_i64().unwrap();

        let policy = scan_timeline(&timeline, None).expect("scan fixture");
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
        let policy = scan_timeline(&timeline, None).expect("scan fixture");

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
        let policy = scan_timeline(&timeline, Some(known_provinces())).expect("scan fixture");
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
