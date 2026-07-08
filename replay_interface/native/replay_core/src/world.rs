//! Scrubbable world model over a [`ReplaySource`].
//!
//! [`WorldTimeline`] owns the source and a small frame cache; [`WorldFrame`]
//! is one hour of world state with the raw domain map plus typed views for
//! the surfaces the replay UI consumes (map control, divisions, combats,
//! country core, provenance). Typed views never coerce: a wrong-typed value
//! becomes [`Leaf::Unobtained`] plus a [`WorldFrame::coverage_warnings`]
//! entry, never a guessed value and never a silent drop.

use crate::source::{RawFrame, ReplaySource, SourceError};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Frames kept hot for scrubbing. Small on purpose: a frame is rebuilt from
/// the verified source on miss, so eviction only costs a re-read.
const FRAME_CACHE_CAP: usize = 16;

/// The no-fabrication trichotomy, preserved end to end:
///
/// * `Absent`     — the key was never captured (absent from the JSON).
/// * `Unobtained` — the key is present with an explicit JSON `null`
///   (captured-but-unobtained), OR present with a wrong-typed value. A
///   wrong-typed value additionally appends to
///   [`WorldFrame::coverage_warnings`] — it is never coerced and never
///   dropped silently.
/// * `Measured(T)` — a typed value, including a measured `0` / `0.0`.
///
/// Consumers must render all three distinctly.
#[derive(Debug, Clone, PartialEq)]
pub enum Leaf<T> {
    Absent,
    Unobtained,
    Measured(T),
}

impl<T> Leaf<T> {
    pub fn is_measured(&self) -> bool {
        matches!(self, Leaf::Measured(_))
    }
    pub fn measured(&self) -> Option<&T> {
        match self {
            Leaf::Measured(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum WorldError {
    Source(SourceError),
    /// The recording's hour series is unusable (empty or out of order).
    Timeline(String),
    /// [`WorldTimeline::frame`] is exact-hour; this hour has no frame.
    NoSuchHour(i64),
    /// [`WorldTimeline::state_at`] before the first recorded frame.
    BeforeFirst {
        hour: i64,
        first: i64,
    },
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorldError::Source(e) => write!(f, "source: {e}"),
            WorldError::Timeline(m) => write!(f, "timeline: {m}"),
            WorldError::NoSuchHour(h) => write!(
                f,
                "no frame at hour_index {h} (frame() is exact-hour; state_at() scrubs)"
            ),
            WorldError::BeforeFirst { hour, first } => write!(
                f,
                "hour_index {hour} is before the first recorded frame ({first})"
            ),
        }
    }
}
impl std::error::Error for WorldError {}
impl From<SourceError> for WorldError {
    fn from(e: SourceError) -> Self {
        WorldError::Source(e)
    }
}

/// Where a map-control view came from (recorded so a consumer can tell a
/// writer-provided mirror from a fallback walk from a missing layer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapLayerSource {
    /// The `map.owner_by_state` / `map.control_by_province` mirror.
    MapMirror,
    /// Fallback walk of `states.*.owner` / `provinces.*.controller`.
    DomainWalk,
    /// Neither the mirror nor the domain existed; the view is empty.
    Missing,
}

/// One division row (REPLAY_SCHEMA.md §6).
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionRow {
    /// `"div_id:div_type"`, falling back to bare `"div_id"` when `div_type`
    /// is absent/null (schema §6 identity rule).
    pub div_uid: String,
    pub owner: Leaf<String>,
    pub location_province: Leaf<u32>,
    pub strength: Leaf<f64>,
    pub organisation: Leaf<f64>,
    pub in_combat: Leaf<bool>,
}

/// One active combat row (REPLAY_SCHEMA.md §7).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatRow {
    pub combat_id: Leaf<i64>,
    pub kind: Leaf<String>,
    pub location_province: Leaf<u32>,
    /// Participant division ids (bare `div_id`s per §7).
    pub attackers: Leaf<Vec<i64>>,
    pub defenders: Leaf<Vec<i64>>,
}

/// Per-TAG country core values for the topbar surface (REPLAY_SCHEMA.md
/// §3.1 fuel, §3.2 division_count, §3.3 politics).
#[derive(Debug, Clone, PartialEq)]
pub struct CountryCore {
    pub stability: Leaf<f64>,
    pub war_support: Leaf<f64>,
    pub political_power: Leaf<f64>,
    pub ruling_party: Leaf<String>,
    pub fuel_current: Leaf<f64>,
    pub fuel_max: Leaf<f64>,
    pub division_count: Leaf<i64>,
}

impl CountryCore {
    /// A present-but-null (or wrong-typed) country entry: every leaf is
    /// captured-but-unobtained, distinct from an absent country.
    fn all_unobtained() -> CountryCore {
        CountryCore {
            stability: Leaf::Unobtained,
            war_support: Leaf::Unobtained,
            political_power: Leaf::Unobtained,
            ruling_party: Leaf::Unobtained,
            fuel_current: Leaf::Unobtained,
            fuel_max: Leaf::Unobtained,
            division_count: Leaf::Unobtained,
        }
    }
}

/// Coverage of one provenance channel (REPLAY_SCHEMA.md §2).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelCoverage {
    pub captured: Leaf<bool>,
    pub domains: Vec<String>,
}

/// Per-frame provenance: which domains each channel (collector/memory/save)
/// captured, plus the writer's free-form notes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProvenanceView {
    pub channels: BTreeMap<String, ChannelCoverage>,
    pub notes: Vec<String>,
}

/// World state at one canonical hour: the raw domain map plus typed views.
pub struct WorldFrame {
    hour_index: i64,
    date: Option<String>,
    save_version: Option<String>,
    raw: Map<String, Value>,
    owner_by_state: BTreeMap<u32, Leaf<String>>,
    owner_by_state_source: MapLayerSource,
    control_by_state: BTreeMap<u32, Leaf<String>>,
    control_by_state_source: MapLayerSource,
    control_by_province: BTreeMap<u32, Leaf<String>>,
    control_by_province_source: MapLayerSource,
    divisions: Vec<DivisionRow>,
    combats: Vec<CombatRow>,
    countries: BTreeMap<String, CountryCore>,
    provenance: ProvenanceView,
    coverage_warnings: Vec<String>,
}

impl WorldFrame {
    pub fn hour_index(&self) -> i64 {
        self.hour_index
    }
    pub fn date(&self) -> Option<&str> {
        self.date.as_deref()
    }
    pub fn save_version(&self) -> Option<&str> {
        self.save_version.as_deref()
    }
    /// Every top-level domain of the snapshot, raw and untouched (unknown
    /// schema growth stays reachable here).
    pub fn raw(&self) -> &Map<String, Value> {
        &self.raw
    }
    pub fn domain(&self, key: &str) -> Option<&Value> {
        self.raw.get(key)
    }
    pub fn owner_by_state(&self) -> &BTreeMap<u32, Leaf<String>> {
        &self.owner_by_state
    }
    pub fn owner_by_state_source(&self) -> MapLayerSource {
        self.owner_by_state_source
    }
    /// Per-state controller (occupier when it differs from the owner).
    /// Schema §8 defines no `map.control_by_state` mirror today, so this
    /// normally comes from the `states.*.controller` domain walk; a future
    /// writer-provided mirror under that key is honored if present.
    pub fn control_by_state(&self) -> &BTreeMap<u32, Leaf<String>> {
        &self.control_by_state
    }
    pub fn control_by_state_source(&self) -> MapLayerSource {
        self.control_by_state_source
    }
    pub fn control_by_province(&self) -> &BTreeMap<u32, Leaf<String>> {
        &self.control_by_province
    }
    pub fn control_by_province_source(&self) -> MapLayerSource {
        self.control_by_province_source
    }
    pub fn divisions(&self) -> &[DivisionRow] {
        &self.divisions
    }
    pub fn combats(&self) -> &[CombatRow] {
        &self.combats
    }
    pub fn countries(&self) -> &BTreeMap<String, CountryCore> {
        &self.countries
    }
    pub fn country(&self, tag: &str) -> Option<&CountryCore> {
        self.countries.get(tag)
    }
    pub fn provenance(&self) -> &ProvenanceView {
        &self.provenance
    }
    /// Present-but-wrong-typed values and other typed-view findings. Each
    /// wrong-typed leaf is `Unobtained` in the view AND listed here.
    pub fn coverage_warnings(&self) -> &[String] {
        &self.coverage_warnings
    }

    fn from_raw(frame: RawFrame) -> WorldFrame {
        let mut warnings: Vec<String> = Vec::new();
        let (owner_by_state, owner_by_state_source) = build_tag_map(
            &frame.rest,
            "owner_by_state",
            "states",
            "owner",
            &mut warnings,
        );
        let (control_by_state, control_by_state_source) = build_tag_map(
            &frame.rest,
            "control_by_state",
            "states",
            "controller",
            &mut warnings,
        );
        let (control_by_province, control_by_province_source) = build_tag_map(
            &frame.rest,
            "control_by_province",
            "provinces",
            "controller",
            &mut warnings,
        );
        let divisions = build_divisions(&frame.rest, &mut warnings);
        let combats = build_combats(&frame.rest, &mut warnings);
        let countries = build_countries(&frame.rest, &mut warnings);
        let provenance = build_provenance(&frame.rest, &mut warnings);
        WorldFrame {
            hour_index: frame.hour_index,
            date: frame.date,
            save_version: frame.save_version,
            raw: frame.rest,
            owner_by_state,
            owner_by_state_source,
            control_by_state,
            control_by_state_source,
            control_by_province,
            control_by_province_source,
            divisions,
            combats,
            countries,
            provenance,
            coverage_warnings: warnings,
        }
    }
}

/// Ordered, scrubbable series of world frames over one recording.
pub struct WorldTimeline {
    source: Box<dyn ReplaySource>,
    hours: Vec<i64>,
    cache: Mutex<FrameCache>,
}

impl WorldTimeline {
    /// Wrap a source, validating its hour series (non-empty, strictly
    /// increasing — the source contract, revalidated here because everything
    /// downstream scrubs by binary search over it).
    pub fn open(source: Box<dyn ReplaySource>) -> Result<WorldTimeline, WorldError> {
        let hours = source.hours().to_vec();
        if hours.is_empty() {
            return Err(WorldError::Timeline(format!(
                "recording {:?} has no playable hours",
                source.game_id()
            )));
        }
        for pair in hours.windows(2) {
            if pair[0] >= pair[1] {
                return Err(WorldError::Timeline(format!(
                    "hour series is not strictly increasing ({} then {})",
                    pair[0], pair[1]
                )));
            }
        }
        Ok(WorldTimeline {
            source,
            hours,
            cache: Mutex::new(FrameCache::new()),
        })
    }

    pub fn game_id(&self) -> &str {
        self.source.game_id()
    }
    pub fn hours(&self) -> &[i64] {
        &self.hours
    }
    pub fn first_hour(&self) -> i64 {
        self.hours[0]
    }
    pub fn last_hour(&self) -> i64 {
        self.hours[self.hours.len() - 1]
    }
    /// Ingest warnings from the source (skipped index entries, duplicates).
    pub fn warnings(&self) -> &[String] {
        self.source.warnings()
    }

    /// The frame at exactly `hour_index`; [`WorldError::NoSuchHour`] otherwise.
    pub fn frame(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        if self.hours.binary_search(&hour_index).is_err() {
            return Err(WorldError::NoSuchHour(hour_index));
        }
        self.load_cached(hour_index)
    }

    /// The nearest frame at-or-before `hour_index` (the scrub primitive);
    /// [`WorldError::BeforeFirst`] before the first recorded frame.
    pub fn state_at(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        let first = self.first_hour();
        if hour_index < first {
            return Err(WorldError::BeforeFirst {
                hour: hour_index,
                first,
            });
        }
        let at = match self.hours.binary_search(&hour_index) {
            Ok(i) => self.hours[i],
            Err(i) => self.hours[i - 1],
        };
        self.load_cached(at)
    }

    fn load_cached(&self, hour_index: i64) -> Result<Arc<WorldFrame>, WorldError> {
        if let Some(frame) = self.cache.lock().expect("frame cache lock").get(hour_index) {
            return Ok(frame);
        }
        let raw = self.source.frame(hour_index)?;
        let frame = Arc::new(WorldFrame::from_raw(raw));
        self.cache
            .lock()
            .expect("frame cache lock")
            .insert(hour_index, frame.clone());
        Ok(frame)
    }
}

/// Tiny LRU (oldest first); linear scans are fine at [`FRAME_CACHE_CAP`].
struct FrameCache {
    entries: Vec<(i64, Arc<WorldFrame>)>,
}

impl FrameCache {
    fn new() -> FrameCache {
        FrameCache {
            entries: Vec::with_capacity(FRAME_CACHE_CAP),
        }
    }
    fn get(&mut self, hour: i64) -> Option<Arc<WorldFrame>> {
        let pos = self.entries.iter().position(|(h, _)| *h == hour)?;
        let entry = self.entries.remove(pos);
        let frame = entry.1.clone();
        self.entries.push(entry);
        Some(frame)
    }
    fn insert(&mut self, hour: i64, frame: Arc<WorldFrame>) {
        if let Some(pos) = self.entries.iter().position(|(h, _)| *h == hour) {
            self.entries.remove(pos);
        }
        self.entries.push((hour, frame));
        if self.entries.len() > FRAME_CACHE_CAP {
            self.entries.remove(0);
        }
    }
}

// ---------------------------------------------------------------------------
// typed-view builders (no coercion; wrong types -> Unobtained + warning)
// ---------------------------------------------------------------------------

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Walk `path` under `obj`. `Absent` if any key on the path is absent;
/// `Unobtained` for an explicit null anywhere on the path (an unobtained
/// group makes its leaves unobtained, not absent); `Measured(&Value)` for a
/// non-null leaf. A wrong-typed intermediate is `Unobtained` + a warning.
fn walk_path<'a>(
    mut obj: &'a Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<&'a Value> {
    for (i, seg) in path.iter().enumerate() {
        let v = match obj.get(*seg) {
            None => return Leaf::Absent,
            Some(v) => v,
        };
        if i + 1 == path.len() {
            return if v.is_null() {
                Leaf::Unobtained
            } else {
                Leaf::Measured(v)
            };
        }
        match v {
            Value::Null => return Leaf::Unobtained,
            Value::Object(m) => obj = m,
            other => {
                warnings.push(format!(
                    "{ctx}.{}: expected object, found {}; treated as unobtained",
                    path[..=i].join("."),
                    json_type_name(other)
                ));
                return Leaf::Unobtained;
            }
        }
    }
    Leaf::Absent
}

fn typed_leaf<'a, T, F>(
    obj: &'a Map<String, Value>,
    path: &[&str],
    want: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
    convert: F,
) -> Leaf<T>
where
    F: FnOnce(&'a Value) -> Option<T>,
{
    match walk_path(obj, path, ctx, warnings) {
        Leaf::Absent => Leaf::Absent,
        Leaf::Unobtained => Leaf::Unobtained,
        Leaf::Measured(v) => match convert(v) {
            Some(t) => Leaf::Measured(t),
            None => {
                warnings.push(format!(
                    "{ctx}.{}: present but wrong-typed (expected {want}, found {}); \
                     treated as unobtained",
                    path.join("."),
                    json_type_name(v)
                ));
                Leaf::Unobtained
            }
        },
    }
}

fn leaf_f64(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<f64> {
    typed_leaf(obj, path, "number", ctx, warnings, Value::as_f64)
}

fn leaf_i64(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<i64> {
    typed_leaf(obj, path, "integer", ctx, warnings, Value::as_i64)
}

fn leaf_u32(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<u32> {
    typed_leaf(obj, path, "non-negative integer", ctx, warnings, |v| {
        v.as_u64().and_then(|n| u32::try_from(n).ok())
    })
}

fn leaf_bool(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<bool> {
    typed_leaf(obj, path, "bool", ctx, warnings, Value::as_bool)
}

fn leaf_string(
    obj: &Map<String, Value>,
    path: &[&str],
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<String> {
    typed_leaf(obj, path, "string", ctx, warnings, |v| {
        v.as_str().map(str::to_string)
    })
}

/// A `{id-string: TAG|null}` layer -> typed map. Used for the `map.*`
/// mirrors and for the fallback domain walk.
fn tag_map_from_object(
    layer: &Map<String, Value>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> BTreeMap<u32, Leaf<String>> {
    let mut out = BTreeMap::new();
    for (key, v) in layer {
        let id: u32 = match key.parse() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!("{ctx}: non-numeric id key {key:?} skipped"));
                continue;
            }
        };
        let leaf = match v {
            Value::Null => Leaf::Unobtained,
            Value::String(tag) => Leaf::Measured(tag.clone()),
            other => {
                warnings.push(format!(
                    "{ctx}.{key}: present but wrong-typed (expected string TAG, found {}); \
                     treated as unobtained",
                    json_type_name(other)
                ));
                Leaf::Unobtained
            }
        };
        out.insert(id, leaf);
    }
    out
}

/// Map-control view: prefer the `map.<mirror_key>` mirror; when it is absent
/// or null (or wrong-typed, with a warning), fall back to walking
/// `<domain>.*.<field>`. Returns the view and which source produced it.
fn build_tag_map(
    rest: &Map<String, Value>,
    mirror_key: &str,
    domain: &str,
    field: &str,
    warnings: &mut Vec<String>,
) -> (BTreeMap<u32, Leaf<String>>, MapLayerSource) {
    let mirror = match rest.get("map") {
        Some(Value::Object(map)) => map.get(mirror_key),
        Some(Value::Null) | None => None,
        Some(other) => {
            warnings.push(format!(
                "map: present but wrong-typed (expected object, found {}); \
                 falling back to walking {domain}.*.{field}",
                json_type_name(other)
            ));
            None
        }
    };
    match mirror {
        Some(Value::Object(layer)) => {
            let ctx = format!("map.{mirror_key}");
            return (
                tag_map_from_object(layer, &ctx, warnings),
                MapLayerSource::MapMirror,
            );
        }
        Some(Value::Null) | None => {}
        Some(other) => {
            warnings.push(format!(
                "map.{mirror_key}: present but wrong-typed (expected object, found {}); \
                 falling back to walking {domain}.*.{field}",
                json_type_name(other)
            ));
        }
    }
    // Fallback: walk the full domain objects.
    let entries = match rest.get(domain) {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) | None => return (BTreeMap::new(), MapLayerSource::Missing),
        Some(other) => {
            warnings.push(format!(
                "{domain}: present but wrong-typed (expected object, found {}); \
                 map view is empty",
                json_type_name(other)
            ));
            return (BTreeMap::new(), MapLayerSource::Missing);
        }
    };
    let mut out = BTreeMap::new();
    for (key, entry) in entries {
        let id: u32 = match key.parse() {
            Ok(id) => id,
            Err(_) => {
                warnings.push(format!("{domain}: non-numeric id key {key:?} skipped"));
                continue;
            }
        };
        let leaf = match entry {
            Value::Null => Leaf::Unobtained,
            Value::Object(obj) => {
                let ctx = format!("{domain}.{key}");
                leaf_string(obj, &[field], &ctx, warnings)
            }
            other => {
                warnings.push(format!(
                    "{domain}.{key}: expected object, found {}; treated as unobtained",
                    json_type_name(other)
                ));
                Leaf::Unobtained
            }
        };
        out.insert(id, leaf);
    }
    (out, MapLayerSource::DomainWalk)
}

fn build_divisions(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> Vec<DivisionRow> {
    let items = match rest.get("divisions") {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => return Vec::new(),
        Some(other) => {
            warnings.push(format!(
                "divisions: present but wrong-typed (expected array, found {}); \
                 division view is empty",
                json_type_name(other)
            ));
            return Vec::new();
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = match item.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "divisions[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        // Identity is required (schema §6): a row without an integer div_id
        // cannot be keyed and is skipped with a warning, never invented.
        let div_id = match obj.get("div_id").and_then(Value::as_i64) {
            Some(id) => id,
            None => {
                warnings.push(format!(
                    "divisions[{i}]: missing or non-integer div_id; row skipped"
                ));
                continue;
            }
        };
        let div_uid = match obj.get("div_type") {
            // absent div_type is the schema-sanctioned bare-id identity
            None => format!("{div_id}"),
            // null is captured-but-unobtained, not schema-absent: identity
            // still degrades to the bare id (§6 untyped-channel matching),
            // but the degradation is surfaced, never silent
            Some(Value::Null) => {
                warnings.push(format!(
                    "divisions[{i}].div_type is null (unobtained); falling back to \
                     bare div_id identity"
                ));
                format!("{div_id}")
            }
            Some(v) => match v.as_i64() {
                Some(t) => format!("{div_id}:{t}"),
                None => {
                    warnings.push(format!(
                        "divisions[{i}].div_type: present but wrong-typed (expected \
                         integer, found {}); falling back to bare div_id identity",
                        json_type_name(v)
                    ));
                    format!("{div_id}")
                }
            },
        };
        let ctx = format!("divisions[{i}]");
        rows.push(DivisionRow {
            div_uid,
            owner: leaf_string(obj, &["owner"], &ctx, warnings),
            location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
            strength: leaf_f64(obj, &["strength"], &ctx, warnings),
            organisation: leaf_f64(obj, &["organisation"], &ctx, warnings),
            in_combat: leaf_bool(obj, &["in_combat"], &ctx, warnings),
        });
    }
    rows
}

fn leaf_id_list(
    obj: &Map<String, Value>,
    key: &str,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<Vec<i64>> {
    match obj.get(key) {
        None => Leaf::Absent,
        Some(Value::Null) => Leaf::Unobtained,
        Some(Value::Array(items)) => {
            let mut ids = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                match item.as_i64() {
                    Some(id) => ids.push(id),
                    None => {
                        // A partial participant list would misrepresent the
                        // engagement as Measured; one malformed element
                        // poisons the whole leaf (fail-closed, with warning).
                        warnings.push(format!(
                            "{ctx}.{key}[{i}]: expected integer division id, found {}; \
                             whole list treated as unobtained",
                            json_type_name(item)
                        ));
                        return Leaf::Unobtained;
                    }
                }
            }
            Leaf::Measured(ids)
        }
        Some(other) => {
            warnings.push(format!(
                "{ctx}.{key}: present but wrong-typed (expected array, found {}); \
                 treated as unobtained",
                json_type_name(other)
            ));
            Leaf::Unobtained
        }
    }
}

fn build_combats(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> Vec<CombatRow> {
    let items = match rest.get("combats") {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => return Vec::new(),
        Some(other) => {
            warnings.push(format!(
                "combats: present but wrong-typed (expected array, found {}); \
                 combat view is empty",
                json_type_name(other)
            ));
            return Vec::new();
        }
    };
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let obj = match item.as_object() {
            Some(obj) => obj,
            None => {
                warnings.push(format!(
                    "combats[{i}]: expected object, found {}; row skipped",
                    json_type_name(item)
                ));
                continue;
            }
        };
        let ctx = format!("combats[{i}]");
        rows.push(CombatRow {
            combat_id: leaf_i64(obj, &["combat_id"], &ctx, warnings),
            kind: leaf_string(obj, &["kind"], &ctx, warnings),
            location_province: leaf_u32(obj, &["location_province"], &ctx, warnings),
            attackers: leaf_id_list(obj, "attackers", &ctx, warnings),
            defenders: leaf_id_list(obj, "defenders", &ctx, warnings),
        });
    }
    rows
}

fn build_countries(
    rest: &Map<String, Value>,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, CountryCore> {
    let entries = match rest.get("countries") {
        Some(Value::Object(entries)) => entries,
        Some(Value::Null) | None => return BTreeMap::new(),
        Some(other) => {
            warnings.push(format!(
                "countries: present but wrong-typed (expected object, found {}); \
                 country view is empty",
                json_type_name(other)
            ));
            return BTreeMap::new();
        }
    };
    let mut out = BTreeMap::new();
    for (tag, entry) in entries {
        let obj = match entry {
            Value::Object(obj) => obj,
            // A present-but-null country entry means its leaves are
            // captured-but-unobtained; the tag must not vanish from the view
            // (that would be indistinguishable from an absent country).
            Value::Null => {
                out.insert(tag.clone(), CountryCore::all_unobtained());
                continue;
            }
            other => {
                warnings.push(format!(
                    "countries.{tag}: expected object, found {}; leaves treated as \
                     unobtained",
                    json_type_name(other)
                ));
                out.insert(tag.clone(), CountryCore::all_unobtained());
                continue;
            }
        };
        let ctx = format!("countries.{tag}");
        out.insert(
            tag.clone(),
            CountryCore {
                stability: leaf_f64(obj, &["politics", "stability"], &ctx, warnings),
                war_support: leaf_f64(obj, &["politics", "war_support"], &ctx, warnings),
                political_power: leaf_f64(obj, &["politics", "political_power"], &ctx, warnings),
                ruling_party: leaf_string(obj, &["politics", "ruling_party"], &ctx, warnings),
                fuel_current: leaf_f64(obj, &["economy", "fuel", "current"], &ctx, warnings),
                fuel_max: leaf_f64(obj, &["economy", "fuel", "max"], &ctx, warnings),
                division_count: leaf_i64(obj, &["military", "division_count"], &ctx, warnings),
            },
        );
    }
    out
}

fn build_provenance(rest: &Map<String, Value>, warnings: &mut Vec<String>) -> ProvenanceView {
    let prov = match rest.get("provenance") {
        Some(Value::Object(prov)) => prov,
        Some(Value::Null) | None => return ProvenanceView::default(),
        Some(other) => {
            warnings.push(format!(
                "provenance: present but wrong-typed (expected object, found {}); \
                 provenance view is empty",
                json_type_name(other)
            ));
            return ProvenanceView::default();
        }
    };
    let mut channels = BTreeMap::new();
    for channel in ["collector", "memory", "save"] {
        match prov.get(channel) {
            None => {}
            Some(Value::Null) => {
                channels.insert(
                    channel.to_string(),
                    ChannelCoverage {
                        captured: Leaf::Unobtained,
                        domains: Vec::new(),
                    },
                );
            }
            Some(Value::Object(obj)) => {
                let ctx = format!("provenance.{channel}");
                let mut domains = Vec::new();
                match obj.get("domains") {
                    None | Some(Value::Null) => {}
                    Some(Value::Array(items)) => {
                        for (i, item) in items.iter().enumerate() {
                            match item.as_str() {
                                Some(s) => domains.push(s.to_string()),
                                None => warnings.push(format!(
                                    "{ctx}.domains[{i}]: expected string, found {}; \
                                     element skipped",
                                    json_type_name(item)
                                )),
                            }
                        }
                    }
                    Some(other) => warnings.push(format!(
                        "{ctx}.domains: present but wrong-typed (expected array, found {}); \
                         treated as empty",
                        json_type_name(other)
                    )),
                }
                channels.insert(
                    channel.to_string(),
                    ChannelCoverage {
                        captured: leaf_bool(obj, &["captured"], &ctx, warnings),
                        domains,
                    },
                );
            }
            Some(other) => {
                warnings.push(format!(
                    "provenance.{channel}: present but wrong-typed (expected object, \
                     found {}); treated as unobtained",
                    json_type_name(other)
                ));
                channels.insert(
                    channel.to_string(),
                    ChannelCoverage {
                        captured: Leaf::Unobtained,
                        domains: Vec::new(),
                    },
                );
            }
        }
    }
    let mut notes = Vec::new();
    match prov.get("notes") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                match item.as_str() {
                    Some(s) => notes.push(s.to_string()),
                    None => warnings.push(format!(
                        "provenance.notes[{i}]: expected string, found {}; element skipped",
                        json_type_name(item)
                    )),
                }
            }
        }
        Some(other) => warnings.push(format!(
            "provenance.notes: present but wrong-typed (expected array, found {}); \
             treated as empty",
            json_type_name(other)
        )),
    }
    ProvenanceView { channels, notes }
}
