
use std::collections::{BTreeMap, BTreeSet};

use replay_core::{Leaf, ViewStatus, WorldFrame, WorldTimeline};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnomalyClass {
    MissingDomain,
    TypeFlip,
    OutOfRange,
    HourGap,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub class: AnomalyClass,
    pub hour: i64,
    pub monitor: String,
    pub detail: String,
}

impl Diagnostic {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub class: AnomalyClass,
    pub monitor: String,
    pub since_hour: i64,
    pub marker: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HourResolution {
    Exact,
    BeforeFirst,
    GapUnavailable {
        from: i64,
        to: i64,
        missing: Vec<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    pub from: i64,
    pub to: i64,
    pub missing: Vec<i64>,
}

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

const MONITORED_DOMAINS: &[&str] = &[
    "countries",
    "weather",
    "snow_mud",
    "snow_mud.by_province",
    "visibility",
    "buildings",
    "topbar",
    "naval_supremacy",
    "country_identities",
    "diplomacy",
];

const TYPE_FLIP_FIELDS: &[&str] = &["political_power", "fuel_current"];

fn monitored_domain_value<'a>(frame: &'a WorldFrame, domain: &str) -> Option<&'a Value> {
    match domain {
        "snow_mud" => frame.domain("map")?.get("_r2_weather_live"),
        "snow_mud.by_province" => frame
            .domain("map")?
            .get("_r2_weather_live")?
            .get("provinces"),
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

pub struct AnomalyPolicy {
    hours: Vec<i64>,
    gaps: Vec<Gap>,
    known_province_ids: Option<BTreeSet<u32>>,

    last_present_hour: BTreeMap<String, i64>,
    last_good_scalar: BTreeMap<String, (i64, f64)>,
    identity_baseline: Option<(i64, (Option<String>, Option<String>))>,
    active: BTreeMap<String, Hold>,

    diagnostics: Vec<Diagnostic>,
    hold_timeline: Vec<(i64, Vec<Hold>)>,
}

impl AnomalyPolicy {
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

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

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
        HourResolution::Exact
    }

    pub fn active_holds_at(&self, hour: i64) -> Vec<Hold> {
        let idx = self.hold_timeline.partition_point(|(h, _)| *h <= hour);
        if idx == 0 {
            Vec::new()
        } else {
            self.hold_timeline[idx - 1].1.clone()
        }
    }

    pub fn observe(&mut self, frame: &WorldFrame) -> Vec<Diagnostic> {
        let hour = frame.hour_index();
        let before = self.diagnostics.len();

        self.check_missing_domains(frame, hour);
        self.check_type_flips(frame, hour);
        self.check_out_of_range(frame, hour);
        self.check_identity(frame, hour);

        let holds: Vec<Hold> = self.active.values().cloned().collect();
        match self.hold_timeline.last_mut() {
            Some(last) if last.0 == hour => last.1 = holds,
            Some(last) if last.1 == holds => {}
            Some(_) => self.hold_timeline.push((hour, holds)),
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
                    self.last_present_hour.insert(domain.to_string(), hour);
                    self.active.remove(&key);
                }
                ViewStatus::DomainAbsent | ViewStatus::DomainUnavailable => {
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
            let mut rest = self
                .frames
                .get(&hour_index)
                .and_then(JsonValue::as_object)
                .ok_or_else(|| SourceError::Open(format!("no frame at hour {hour_index}")))?
                .clone();
            if replay_core::COUNTRY_COMPOSED_MEMBERS
                .iter()
                .any(|member| rest.contains_key(*member))
            {
                rest.entry("tag_db_id_index")
                    .or_insert_with(|| serde_json::json!({"country_composition_version": 2}));
            }
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: rest.into(),
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

    #[test]
    fn derived_domain_hold_names_the_last_present_hour_not_the_breaking_hour() {
        let timeline = WorldTimeline::open(Box::new(MemorySource {
            hours: vec![0, 1, 2],
            frames: BTreeMap::from([
                (0, serde_json::json!({ "countries": { "GER": {} } })),
                (1, serde_json::json!({ "countries": { "GER": {} } })),
                (2, serde_json::json!({})),
            ]),
        }))
        .expect("open synthetic timeline");

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

        assert!(
            policy.active_holds_at(last_good).is_empty(),
            "pre-anomaly hour has no holds"
        );
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
                "GER": { "politics": { "political_power": "banana" } },
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

    #[test]
    fn snow_mud_texture_downgrade_surfaces_under_present_view() {
        let textured = serde_json::json!({
            "map": { "_r2_weather_live": {
                "source": "weather_live",
                "scale_raw": 100000,
                "observed_province_rows": 2,
                "provinces": {
                    "0": { "province_id": 0, "snow_raw": 4_000_000,
                           "mud_present": true, "mud_active": false },
                    "1": { "province_id": 1, "snow_raw": 250_000,
                           "mud_present": true, "mud_active": true }
                }
            } }
        });
        let texture_lost = serde_json::json!({
            "map": { "_r2_weather_live": {
                "source": "weather_live",
                "scale_raw": 100000,
                "observed_province_rows": 2,
                "provinces": {
                    "0": 7,
                    "1": { "province_id": 1, "snow_raw": 250_000,
                           "mud_present": true, "mud_active": true }
                }
            } }
        });
        let source = MemorySource {
            hours: vec![0, 1],
            frames: [(0, textured), (1, texture_lost)].into_iter().collect(),
        };
        let timeline = WorldTimeline::open(Box::new(source)).expect("timeline");

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

        assert!(policy.active_holds_at(last_good).is_empty());
        let held = policy.active_holds_at(crossing);
        assert!(held
            .iter()
            .any(|h| h.monitor == "countries.GER.manpower_available"
                && h.class == AnomalyClass::OutOfRange
                && h.since_hour == last_good));
    }

    #[test]
    fn out_of_range_without_prior_good_value_is_flagged_not_held() {
        let timeline = WorldTimeline::open(Box::new(MemorySource {
            hours: vec![0, 1],
            frames: BTreeMap::from([
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

        assert_eq!(policy.gaps().len(), 1);
        assert_eq!(policy.gaps()[0].missing, vec![missing]);

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
