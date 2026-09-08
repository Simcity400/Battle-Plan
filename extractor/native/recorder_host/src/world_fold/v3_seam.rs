//! H4CP v3 reducer seam: reducers mark the country members, divisions,
//! states and provinces they write; the hour boundary renders only those,
//! through the builder code, into grain upserts/removals for the prototype.
//! Invariant: the projection of the seam's grains equals the v2 section bytes
//! every hour (the prototype's oracle and world_fold/tests.rs check it).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use fold_v3::{decompose_subtree, Path, RootKind, Seg};
use serde_json::{Map, Value};
use snapshot::canon_convert::from_value_shared;

use super::*;

pub const V3_SECTION_COUNTRIES: &str = "countries";
pub const V3_SECTION_DIVISIONS: &str = "divisions";
pub const V3_SECTION_STATES: &str = "states";
pub const V3_SECTION_PROVINCES: &str = "provinces";

/// A private member each staged row carries through `emit_countries_by_tag`, so
/// the emitted key can be joined back to the id; stripped before any grain.
const V3_ID_MARKER: &str = "__v3_tag_db_id";

const V3_OUTPUT_CAP: usize = 16;

/// The seam's grain depths, one per covered section, as the prototype's
/// grain-depth spec names them.
#[derive(Debug, Clone)]
pub struct V3SeamConfig {
    pub countries_depth: usize,
    pub divisions_depth: usize,
    pub states_depth: usize,
    pub provinces_depth: usize,
    /// Render exactly the reducer marks: no widening, no projections. Not a
    /// legal seam (the oracle fails); it prices the write set alone.
    pub census_only: bool,
}

impl V3SeamConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.countries_depth < 2 {
            return Err(format!(
                "v3 seam: countries grain depth {} is below 2; the seam renders members, so a grain must lie at or below a member",
                self.countries_depth
            ));
        }
        for (name, depth) in [
            ("divisions", self.divisions_depth),
            ("states", self.states_depth),
            ("provinces", self.provinces_depth),
        ] {
            if depth == 0 {
                return Err(format!("v3 seam: {name} grain depth 0"));
            }
        }
        Ok(())
    }
}

/// One hour's grain mutations for one covered section.
#[derive(Clone, Debug)]
pub struct V3SectionHour {
    pub section: &'static str,
    /// Root shape supplied by the producer; apply after removals, before upserts.
    pub root_kind: RootKind,
    /// Paths whose grain is gone; applied before the upserts.
    pub removals: Vec<Path>,
    /// Canonical JSON bytes per grain path, produced by serializing a
    /// [`CanonValue`], which is what `HourTxn::upsert_canonical_bytes` admits.
    pub upserts: Vec<(Path, Vec<u8>)>,
    /// Wall time the seam spent rendering this section this hour.
    pub render_ns: u64,
    /// Bytes of grain text the seam serialized this hour (upserts included
    /// the ones the state will find unchanged).
    pub rendered_bytes: u64,
    pub rows_rendered: u64,
    pub members_rendered: u64,
    /// The reducers' own write set this hour, before the seam widened it:
    /// rows the reducers named (for countries, distinct (row, member) pairs).
    pub reducer_marked_rows: u64,
    pub reducer_marked_members: u64,
    /// Whether the seam rendered every row of the section this hour, and why.
    pub widened_all: Option<&'static str>,
    /// What the seam decided this hour (flags, projections run, marks), for
    /// reading a divergence.
    pub diagnostics: String,
}

/// Everything the seam produced for one hour.
#[derive(Clone, Debug)]
pub struct V3HourOutput {
    pub hour_index: i64,
    pub sections: Vec<V3SectionHour>,
    pub mark_ns: u64,
    pub marks: u64,
    /// Wall time of the whole hour-boundary render, every section included
    /// (the per-section `render_ns` sum to slightly less).
    pub render_ns: u64,
    /// Hours the seam dropped because nothing drained its outputs.
    pub outputs_dropped: u64,
}

impl V3HourOutput {
    /// Heap the output holds (the consumer charges it against the hour).
    pub fn approx_bytes(&self) -> u64 {
        let seg = std::mem::size_of::<Seg>() as u64;
        let upsert = std::mem::size_of::<(Path, Vec<u8>)>() as u64;
        let removal = std::mem::size_of::<Path>() as u64;
        (self.sections.capacity() * std::mem::size_of::<V3SectionHour>()) as u64
            + self
                .sections
                .iter()
                .map(|section| {
                    section.upserts.capacity() as u64 * upsert
                        + section
                            .upserts
                            .iter()
                            .map(|(path, bytes)| {
                                path.capacity() as u64 * seg + bytes.capacity() as u64
                            })
                            .sum::<u64>()
                        + section.removals.capacity() as u64 * removal
                        + section
                            .removals
                            .iter()
                            .map(|path| path.capacity() as u64 * seg)
                            .sum::<u64>()
                        + section.diagnostics.capacity() as u64
                })
                .sum::<u64>()
    }
}

/// The whole-map passes `build_snapshot` runs over the staged countries, in
/// its order, with the domains whose records change their output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Projection {
    /// The base loop's identity layer: witness, claims, cores, is_major,
    /// cosmetic tag.
    Identity,
    Collaboration,
    AceRoster,
    DynamicModifierNull,
    Frd,
    Domain53,
    ProductionLicenses,
    Capitulation,
    MarketTrade,
    ScriptedGui,
    NukeStockpile,
    PreferredTactics,
    ArmyUpgradeRequests,
    ActiveCommandAbilities,
    Domain48,
    /// Written from the `diplomacy_world` build into `countries`.
    LendLease,
}

const PROJECTIONS: &[(Projection, &[u16])] = &[
    (Projection::Identity, &[18, 53]),
    (Projection::Collaboration, &[52, 18]),
    (Projection::AceRoster, &[46, 18]),
    (Projection::DynamicModifierNull, &[67]),
    (Projection::Frd, &[45]),
    (Projection::Domain53, &[53]),
    (Projection::ProductionLicenses, &[53, 18, 11]),
    (Projection::Capitulation, &[60, 18]),
    (Projection::MarketTrade, &[43, 18]),
    (Projection::ScriptedGui, &[84]),
    (Projection::NukeStockpile, &[83]),
    (Projection::PreferredTactics, &[92, 96, 18]),
    (Projection::ArmyUpgradeRequests, &[93]),
    (Projection::ActiveCommandAbilities, &[88]),
    (Projection::Domain48, &[48]),
    (Projection::LendLease, &[43, 18]),
];

const CO_OWNED: &[(Projection, &[&str])] = &[
    (Projection::Identity, &["politics", "identity"]),
    (Projection::DynamicModifierNull, &["dynamic_modifier_instances"]),
    (Projection::Frd, &["focus", "research", "decisions"]),
    (Projection::Domain53, &["extra", "diplomacy", "intel"]),
    (Projection::ProductionLicenses, &["production"]),
    (Projection::MarketTrade, &["economy", "r2_market_trade_live"]),
    (Projection::LendLease, &["diplomacy"]),
    (Projection::AceRoster, &["extra"]),
    (Projection::ScriptedGui, &["extra"]),
    (Projection::NukeStockpile, &["extra"]),
    (Projection::PreferredTactics, &["extra", "active_abilities"]),
    (Projection::ArmyUpgradeRequests, &["extra"]),
    (Projection::ActiveCommandAbilities, &["extra", "active_abilities"]),
    (Projection::Domain48, &["extra", "production"]),
    (Projection::Collaboration, &["collaboration"]),
];

fn co_owned(projection: Projection) -> &'static [&'static str] {
    CO_OWNED
        .iter()
        .find(|(candidate, _)| *candidate == projection)
        .map(|(_, members)| *members)
        .unwrap_or(&[])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CountryFlags {
    identity_known: bool,
    scripted_unknown: bool,
    dynamic_modifier_null: bool,
    collaboration_present: bool,
    ace_roster: bool,
    frd_gate: bool,
    domain53_known: bool,
    market_unknown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DivisionFlags {
    unknown: bool,
    equipment_masked: bool,
    equipment_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateFlags {
    map_control_unknown: bool,
    occupation_unknown: bool,
    map_detail_unknown: bool,
    scripted_unknown: bool,
}

/// The grains standing for one country member, and the ids it could not
/// name (the key plan reads them as long as the member stands).
#[derive(Debug, Default, Clone)]
struct MemberInstalled {
    paths: Vec<Path>,
    unresolved: BTreeSet<u32>,
}

#[derive(Debug, Default, Clone)]
struct CountryInstalled {
    key: String,
    members: BTreeMap<String, MemberInstalled>,
    /// Projections whose probe named this row when it was last rendered
    /// whole; one of them running again makes the row a target.
    projections: BTreeSet<Projection>,
}

#[derive(Default)]
struct RowMarks {
    all: bool,
    members: BTreeSet<String>,
}

/// A rendered row of a keyed section: its grain paths and the ids it could
/// not name.
#[derive(Debug, Default, Clone)]
struct RowInstalled {
    paths: Vec<Path>,
    unresolved: BTreeSet<u32>,
}

/// The seam's state: this hour's marks and what it has installed.
pub struct V3Seam {
    config: V3SeamConfig,
    first_hour: bool,
    // ---- marks this hour --------------------------------------------------
    countries_all: Option<&'static str>,
    country_marks: BTreeMap<u32, RowMarks>,
    country_removed: BTreeSet<u32>,
    divisions_all: Option<&'static str>,
    division_marks: BTreeSet<String>,
    division_removed: BTreeSet<String>,
    states_all: Option<&'static str>,
    state_marks: BTreeSet<u32>,
    provinces_all: Option<&'static str>,
    province_marks: BTreeSet<u32>,
    /// Whole-section marks that came from a REDUCER (a whole-table record),
    /// counted in the census; the widening the render adds is not.
    reducer_all_divisions: bool,
    reducer_all_states: bool,
    reducer_all_provinces: bool,
    mark_ns: u64,
    marks: u64,
    // ---- what stands ------------------------------------------------------
    installed_countries: BTreeMap<u32, CountryInstalled>,
    unresolved_by_projection: BTreeMap<Projection, BTreeSet<u32>>,
    installed_divisions: BTreeMap<String, RowInstalled>,
    installed_states: BTreeMap<String, RowInstalled>,
    installed_provinces: BTreeMap<String, RowInstalled>,
    division_key_of: BTreeMap<String, Option<DivisionEquipmentKey>>,
    division_buckets: BTreeMap<DivisionEquipmentKey, BTreeSet<String>>,
    // ---- last hour's inputs, for change detection ------------------------
    last_domains: BTreeSet<u16>,
    last_tag_by_db_id: BTreeMap<u32, String>,
    last_emitted_keys: BTreeMap<u32, String>,
    last_country_flags: Option<CountryFlags>,
    last_division_flags: Option<DivisionFlags>,
    last_state_flags: Option<StateFlags>,
    last_map_control_unknown: Option<bool>,
    /// The states and provinces the D48 projection wrote last time it ran, so
    /// one it drops this hour is re-rendered without the member.
    last_d48_state_targets: BTreeSet<u32>,
    last_d48_province_targets: BTreeSet<u32>,
    // ---- output -------------------------------------------------------------
    outputs: Vec<V3HourOutput>,
    outputs_dropped: u64,
}

impl V3Seam {
    pub fn new(config: V3SeamConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            config,
            first_hour: true,
            countries_all: None,
            country_marks: BTreeMap::new(),
            country_removed: BTreeSet::new(),
            divisions_all: None,
            division_marks: BTreeSet::new(),
            division_removed: BTreeSet::new(),
            states_all: None,
            state_marks: BTreeSet::new(),
            provinces_all: None,
            province_marks: BTreeSet::new(),
            reducer_all_divisions: false,
            reducer_all_states: false,
            reducer_all_provinces: false,
            mark_ns: 0,
            marks: 0,
            installed_countries: BTreeMap::new(),
            unresolved_by_projection: BTreeMap::new(),
            installed_divisions: BTreeMap::new(),
            installed_states: BTreeMap::new(),
            installed_provinces: BTreeMap::new(),
            division_key_of: BTreeMap::new(),
            division_buckets: BTreeMap::new(),
            last_domains: BTreeSet::new(),
            last_tag_by_db_id: BTreeMap::new(),
            last_emitted_keys: BTreeMap::new(),
            last_country_flags: None,
            last_division_flags: None,
            last_state_flags: None,
            last_map_control_unknown: None,
            last_d48_state_targets: BTreeSet::new(),
            last_d48_province_targets: BTreeSet::new(),
            outputs: Vec::new(),
            outputs_dropped: 0,
        })
    }

    fn mark_country_member(&mut self, id: u32, member: &str) {
        let marks = self.country_marks.entry(id).or_default();
        if !marks.all {
            marks.members.insert(member.to_string());
        }
    }

    /// For a reducer iterating `states` by reference (it cannot call
    /// `v3_touch_state` while it holds the map).
    pub(super) fn mark_state(&mut self, state_id: u32) {
        self.marks += 1;
        if self.states_all.is_none() {
            self.state_marks.insert(state_id);
        }
    }

    fn mark_country_row(&mut self, id: u32) {
        let marks = self.country_marks.entry(id).or_default();
        marks.all = true;
        marks.members.clear();
    }

    /// Marks a division and every division sharing its equipment join key,
    /// before and after the change.
    fn mark_division_keyed(&mut self, uid: &str, key: Option<DivisionEquipmentKey>) {
        if self.divisions_all.is_none() {
            self.division_marks.insert(uid.to_string());
        }
        let previous = self.division_key_of.insert(uid.to_string(), key).flatten();
        if let Some(previous) = previous {
            if Some(previous) != key {
                if let Some(bucket) = self.division_buckets.get_mut(&previous) {
                    bucket.remove(uid);
                }
                self.mark_bucket(previous);
            }
        }
        if let Some(key) = key {
            self.division_buckets
                .entry(key)
                .or_default()
                .insert(uid.to_string());
            self.mark_bucket(key);
        }
    }

    fn mark_bucket(&mut self, key: DivisionEquipmentKey) {
        if self.divisions_all.is_some() {
            return;
        }
        if let Some(bucket) = self.division_buckets.get(&key) {
            for uid in bucket {
                self.division_marks.insert(uid.clone());
            }
        }
    }

    fn clear_marks(&mut self) {
        self.countries_all = None;
        self.country_marks.clear();
        self.country_removed.clear();
        self.divisions_all = None;
        self.division_marks.clear();
        self.division_removed.clear();
        self.states_all = None;
        self.state_marks.clear();
        self.provinces_all = None;
        self.province_marks.clear();
        self.reducer_all_divisions = false;
        self.reducer_all_states = false;
        self.reducer_all_provinces = false;
        self.mark_ns = 0;
        self.marks = 0;
    }
}

impl WorldFold {
    pub fn enable_v3_seam(&mut self, config: V3SeamConfig) -> Result<(), String> {
        if self.file_sink.is_some() {
            return Err("v3 seam: not for a strided file-sink fold".to_string());
        }
        self.v3 = Some(V3Seam::new(config)?);
        Ok(())
    }

    pub fn v3_seam_enabled(&self) -> bool {
        self.v3.is_some()
    }

    /// The hours' outputs since the last call, oldest first.
    pub fn take_v3_hour_outputs(&mut self) -> Vec<V3HourOutput> {
        match self.v3.as_mut() {
            Some(seam) => std::mem::take(&mut seam.outputs),
            None => Vec::new(),
        }
    }

    /// Runs `mark` on the seam when there is one: a disabled seam costs the
    /// reducer one branch, nothing is computed for it.
    #[inline]
    fn v3_mark(&mut self, mark: impl FnOnce(&mut V3Seam)) {
        if let Some(seam) = self.v3.as_mut() {
            let started = Instant::now();
            mark(seam);
            seam.marks += 1;
            seam.mark_ns += started.elapsed().as_nanos() as u64;
        }
    }

    /// A reducer wrote these members of country `tag_db_id`.
    #[inline]
    pub(super) fn v3_touch_country_members(&mut self, tag_db_id: u32, members: &[&str]) {
        if members.contains(&"research") {
            self.invalidate_research_root(tag_db_id);
        }
        self.v3_mark(|seam| {
            for member in members {
                seam.mark_country_member(tag_db_id, member);
            }
        });
    }

    /// A reducer wrote the members named by the keys of `values`.
    #[inline]
    pub(super) fn v3_touch_country_member_keys(
        &mut self,
        tag_db_id: u32,
        values: &Map<String, Value>,
    ) {
        if values.contains_key("research") {
            self.invalidate_research_root(tag_db_id);
        }
        self.v3_mark(|seam| {
            for member in values.keys() {
                seam.mark_country_member(tag_db_id, member);
            }
        });
    }

    /// A reducer rewrote country `tag_db_id` wholesale (or created it).
    #[inline]
    pub(super) fn v3_touch_country_row(&mut self, tag_db_id: u32) {
        self.invalidate_research_root(tag_db_id);
        self.v3_mark(|seam| seam.mark_country_row(tag_db_id));
    }

    /// A reducer removed country `tag_db_id`.
    #[inline]
    pub(super) fn v3_remove_country(&mut self, tag_db_id: u32) {
        let key = tag_db_id.to_string();
        self.canonical_research.remove(&key);
        self.dirty_canonical_research.remove(&key);
        self.v3_mark(|seam| {
            seam.country_marks.remove(&tag_db_id);
            seam.country_removed.insert(tag_db_id);
        });
    }

    /// A reducer inserted or replaced division `uid`, whose equipment join
    /// key is now `key`.
    #[inline]
    pub(super) fn v3_touch_division(&mut self, uid: &str, key: Option<DivisionEquipmentKey>) {
        self.v3_mark(|seam| seam.mark_division_keyed(uid, key));
    }

    #[inline]
    pub(super) fn v3_remove_division(&mut self, uid: &str) {
        self.v3_mark(|seam| {
            seam.mark_division_keyed(uid, None);
            seam.division_key_of.remove(uid);
            seam.division_marks.remove(uid);
            seam.division_removed.insert(uid.to_string());
        });
    }

    /// A reducer changed the equipment under `key`: every division joined to
    /// it renders differently.
    #[inline]
    pub(super) fn v3_touch_division_bucket(&mut self, key: DivisionEquipmentKey) {
        self.v3_mark(|seam| seam.mark_bucket(key));
    }

    #[inline]
    pub(super) fn v3_touch_all_divisions(&mut self, why: &'static str) {
        self.v3_mark(|seam| {
            seam.divisions_all = Some(why);
            seam.reducer_all_divisions = true;
            seam.division_marks.clear();
            seam.division_key_of.clear();
            seam.division_buckets.clear();
        });
    }

    #[inline]
    pub(super) fn v3_touch_state(&mut self, state_id: u32) {
        self.v3_mark(|seam| {
            if seam.states_all.is_none() {
                seam.state_marks.insert(state_id);
            }
        });
    }

    #[inline]
    pub(super) fn v3_touch_all_states(&mut self, why: &'static str) {
        self.v3_mark(|seam| {
            seam.states_all = Some(why);
            seam.reducer_all_states = true;
            seam.state_marks.clear();
        });
    }

    #[inline]
    pub(super) fn v3_touch_province(&mut self, province_id: u32) {
        self.v3_mark(|seam| {
            if seam.provinces_all.is_none() {
                seam.province_marks.insert(province_id);
            }
        });
    }

    #[inline]
    pub(super) fn v3_touch_all_provinces(&mut self, why: &'static str) {
        self.v3_mark(|seam| {
            seam.provinces_all = Some(why);
            seam.reducer_all_provinces = true;
            seam.province_marks.clear();
        });
    }
}

/// Grains of one rendered subtree: canonical bytes per path.
fn grains_of(value: Value, depth_left: usize, prefix: Path) -> Vec<(Path, Vec<u8>)> {
    let canonical = from_value_shared(&value, None);
    let mut parts = Vec::new();
    decompose_subtree(&canonical, depth_left, prefix, &mut parts);
    parts
        .into_iter()
        .map(|(path, part)| (path, part.to_json_bytes()))
        .collect()
}

/// Replaces what stood at `installed` with `fresh`: paths no longer present
/// become removals, every fresh grain an upsert (the state byte-compares).
fn replace_grains(
    installed: &mut Vec<Path>,
    fresh: Vec<(Path, Vec<u8>)>,
    out: &mut V3SectionHour,
) {
    let fresh_paths: BTreeSet<&Path> = fresh.iter().map(|(path, _)| path).collect();
    for old in installed.iter() {
        if !fresh_paths.contains(old) {
            out.removals.push(old.clone());
        }
    }
    *installed = fresh.iter().map(|(path, _)| path.clone()).collect();
    for (path, bytes) in fresh {
        out.rendered_bytes += bytes.len() as u64;
        out.upserts.push((path, bytes));
    }
}

/// `resolve_country_collaboration_park` on an emitted row object.
fn resolve_country_collaboration_park_object(row: &mut Map<String, Value>, plan: &CountryKeyPlan) {
    if let Some(collaboration) = row.get_mut("collaboration").and_then(Value::as_object_mut) {
        resolve_parked_country_rows(collaboration, plan);
    }
}

fn empty_section(section: &'static str) -> V3SectionHour {
    V3SectionHour {
        section,
        root_kind: if section == V3_SECTION_DIVISIONS { RootKind::Array } else { RootKind::Object },
        removals: Vec::new(),
        upserts: Vec::new(),
        render_ns: 0,
        rendered_bytes: 0,
        rows_rendered: 0,
        members_rendered: 0,
        reducer_marked_rows: 0,
        reducer_marked_members: 0,
        widened_all: None,
        diagnostics: String::new(),
    }
}

impl WorldFold {
    pub(super) fn v3_render_hour(&mut self, hour_index: i64) {
        let Some(mut seam) = self.v3.take() else {
            return;
        };
        let started = Instant::now();
        let mark_ns = seam.mark_ns;
        let marks = seam.marks;
        let mut sections = Vec::with_capacity(4);
        sections.push(self.v3_render_divisions(&mut seam));
        sections.push(self.v3_render_states(&mut seam));
        sections.push(self.v3_render_provinces(&mut seam));
        sections.push(self.v3_render_countries(&mut seam, hour_index));
        seam.last_domains = self.domains_this_hour.iter().copied().collect();
        seam.last_tag_by_db_id = self.tag_by_db_id.clone();
        seam.first_hour = false;
        seam.clear_marks();
        if seam.outputs.len() >= V3_OUTPUT_CAP {
            seam.outputs.remove(0);
            seam.outputs_dropped += 1;
        }
        let outputs_dropped = seam.outputs_dropped;
        seam.outputs.push(V3HourOutput {
            hour_index,
            sections,
            mark_ns,
            marks,
            render_ns: started.elapsed().as_nanos() as u64,
            outputs_dropped,
        });
        self.v3 = Some(seam);
    }

    fn v3_domain_ran(&self, seam: &V3Seam, domain: u16) -> bool {
        self.domains_this_hour.contains(&domain) || seam.last_domains.contains(&domain)
    }

    fn v3_country_flags(&self) -> CountryFlags {
        let d45_malformed_payload = self.domain_problems_this_hour.iter().any(|problem| {
            problem.domain_id == FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID
                && problem.problem == "malformed_payload"
        });
        CountryFlags {
            identity_known: self.country_identity_known_this_hour,
            scripted_unknown: self.fold_domain_unknown(SCRIPTED_STATE_DOMAIN_ID),
            dynamic_modifier_null: self.dynamic_modifier_v2_seen
                && !self.dynamic_modifier_v2_known_this_hour,
            collaboration_present: self.collaboration_extension_present_this_hour,
            ace_roster: self.air_aces_fresh_this_hour && self.country_identity_known_this_hour,
            frd_gate: !self.fold_domain_unknown(FOCUS_RESEARCH_DECISIONS_DEPTH_DOMAIN_ID)
                || d45_malformed_payload,
            domain53_known: self.domain53_known_this_hour,
            market_unknown: self.market_trade_unknown_this_hour,
        }
    }

    /// The base render of one country member: the reducer's value with the
    /// per-row fixups `build_snapshot` applies before any projection.
    fn v3_base_member(
        &self,
        base: &Map<String, Value>,
        member: &str,
        scripted_unknown: bool,
        unresolved: &mut BTreeSet<u32>,
    ) -> Option<Value> {
        let value = base.get(member)?.clone();
        let mut row = Map::new();
        row.insert(member.to_string(), value);
        if member == "diplomacy" {
            self.resolve_country_fields(&mut row, unresolved);
        }
        if member == "extra" && scripted_unknown {
            mask_scripted_country_extra(&mut row);
        }
        row.remove(member)
    }

    /// The identity layer of the base loop, for one staged row.
    fn v3_identity_layer(&self, row: &mut Map<String, Value>, id: u32) {
        if self.attach_country_identity_and_names(row, id) {
            if let Some(claims) = self
                .country_claims_this_hour
                .as_ref()
                .and_then(|rows| rows.get(&id))
            {
                ensure_json_object_mut(row, "identity").insert("claims".into(), json!(claims));
            }
            if let Some(cores) = self
                .country_cores_this_hour
                .as_ref()
                .and_then(|rows| rows.get(&id))
            {
                ensure_json_object_mut(row, "identity").insert("cores".into(), json!(cores));
            }
            if let Some(is_major) = self
                .country_major_this_hour
                .as_ref()
                .and_then(|rows| rows.get(&id))
            {
                ensure_json_object_mut(row, "politics").insert("is_major".into(), json!(is_major));
            }
        }
        if let Some(cosmetic) = self.cosmetic_by_db_id.get(&id) {
            row.insert(
                "cosmetic_tag".into(),
                match cosmetic {
                    Some(tag) => json!(tag),
                    None => Value::Null,
                },
            );
        }
    }

    /// Runs one projection over `staging` exactly as `build_snapshot` does.
    fn v3_run_projection(
        &self,
        projection: Projection,
        staging: &mut Map<String, Value>,
        flags: CountryFlags,
        hour_index: i64,
        unresolved: &mut BTreeSet<u32>,
    ) {
        match projection {
            Projection::Identity => {
                for numeric_key in self.countries.keys() {
                    let Ok(id) = numeric_key.parse::<u32>() else {
                        continue;
                    };
                    let row = ensure_country_projection_object(staging, id);
                    self.v3_identity_layer(row, id);
                }
            }
            Projection::Collaboration => {
                if !flags.collaboration_present {
                    return;
                }
                match &self.collaboration_relations_this_hour {
                    Some(rows) => {
                        for numeric_key in self.countries.keys() {
                            if let Ok(id) = numeric_key.parse::<u32>() {
                                ensure_country_projection_object(staging, id).insert(
                                    "collaboration".into(),
                                    json!({"collaboration": {}}),
                                );
                            }
                        }
                        for &tag_db_id in self.tag_by_db_id.keys() {
                            let country = ensure_country_projection_object(staging, tag_db_id);
                            self.attach_country_identity_witness(country, tag_db_id);
                            country
                                .insert("collaboration".into(), json!({"collaboration": {}}));
                        }
                        let mut relations_by_owner: BTreeMap<u32, BTreeMap<u32, Value>> =
                            BTreeMap::new();
                        for &(owner, target, progress_raw) in rows {
                            relations_by_owner.entry(owner).or_default().insert(
                                target,
                                json!({"value": progress_raw as f64 / FIXED_SCALE}),
                            );
                        }
                        for (owner, targets) in relations_by_owner {
                            let country = ensure_country_projection_object(staging, owner);
                            let collaboration = ensure_json_object_mut(country, "collaboration");
                            defer_country_rows(
                                collaboration,
                                self,
                                "collaboration",
                                "_r2_collaboration_by_tag_db_id",
                                targets,
                                unresolved,
                            );
                        }
                    }
                    None => {
                        for numeric_key in self.countries.keys() {
                            if let Ok(id) = numeric_key.parse::<u32>() {
                                ensure_country_projection_object(staging, id)
                                    .insert("collaboration".into(), Value::Null);
                            }
                        }
                        for &tag_db_id in self.tag_by_db_id.keys() {
                            ensure_country_projection_object(staging, tag_db_id)
                                .insert("collaboration".into(), Value::Null);
                        }
                    }
                }
            }
            Projection::AceRoster => {
                if !flags.ace_roster {
                    return;
                }
                if let Some(authority) = self.air_ace_roster_authority_this_hour {
                    if authority.observed_country_rows as usize == self.tag_by_db_id.len() {
                        for &owner_tag_db_id in self.tag_by_db_id.keys() {
                            let country = ensure_country_projection_object(staging, owner_tag_db_id);
                            self.attach_country_identity_witness(country, owner_tag_db_id);
                            let extra = ensure_json_object_mut(country, "extra");
                            extra.entry(DOMAIN46_ACE_R2_KEY).or_insert_with(|| {
                                build_air_ace_roster_sidecar(authority, owner_tag_db_id, Vec::new())
                            });
                        }
                    }
                }
            }
            Projection::DynamicModifierNull => {
                if !flags.dynamic_modifier_null {
                    return;
                }
                for numeric_key in self.countries.keys() {
                    if let Ok(id) = numeric_key.parse::<u32>() {
                        ensure_country_projection_object(staging, id)
                            .insert("dynamic_modifier_instances".into(), Value::Null);
                    }
                }
            }
            Projection::Frd => {
                if flags.frd_gate {
                    self.project_frd_annotations(staging, unresolved);
                }
            }
            Projection::Domain53 => {
                if flags.domain53_known {
                    self.project_domain53_country_overlays(staging, unresolved);
                    self.project_domain53_faction_memberships(staging, unresolved);
                    self.project_domain53_subject_overlords(staging);
                }
            }
            Projection::ProductionLicenses => {
                self.project_production_license_agreements(staging, unresolved);
            }
            Projection::Capitulation => self.project_capitulation_country_state(staging),
            Projection::MarketTrade => {
                self.project_market_trade_resource_flow(staging);
                self.project_market_trade_factory_headers(staging);
                self.project_market_trade_resource_rows(staging);
                self.project_market_trade_classic_deals(staging, unresolved);
                self.project_market_trade_stockpiles(staging);
            }
            Projection::ScriptedGui => self.project_scripted_gui_runtime(staging, hour_index),
            Projection::NukeStockpile => self.project_nuke_stockpile(staging, hour_index),
            Projection::PreferredTactics => self.project_country_preferred_tactics(staging),
            Projection::ArmyUpgradeRequests => {
                self.project_country_army_upgrade_requests(staging, hour_index)
            }
            Projection::ActiveCommandAbilities => {
                self.project_active_command_abilities(staging, hour_index)
            }
            Projection::Domain48 => self.project_domain48_countries(staging),
            Projection::LendLease => {
                if flags.market_unknown {
                    if let Some(contracts) = &self.market_trade_contracts {
                        self.project_market_trade_lend_lease(staging, contracts, true);
                    }
                } else if let Some(contracts) = &self.market_trade_contracts {
                    let mut contracts = contracts.clone();
                    self.project_market_trade_relation_context(&mut contracts, unresolved);
                    self.resolve_contract_list(&mut contracts, unresolved);
                    if !contracts.is_empty() {
                        self.project_market_trade_lend_lease(staging, &contracts, false);
                    }
                }
            }
        }
    }

    fn v3_render_countries(&self, seam: &mut V3Seam, hour_index: i64) -> V3SectionHour {
        let started = Instant::now();
        let mut out = empty_section(V3_SECTION_COUNTRIES);
        let depth_left = seam.config.countries_depth - 2;
        let flags = self.v3_country_flags();
        let every_id = || {
            self.countries
                .keys()
                .filter_map(|key| key.parse::<u32>().ok())
                .collect::<Vec<u32>>()
        };

        let census_only = seam.config.census_only;
        out.reducer_marked_rows = (seam.country_marks.len() + seam.country_removed.len()) as u64;
        out.reducer_marked_members = seam
            .country_marks
            .iter()
            .map(|(id, marks)| {
                if marks.all {
                    self.countries
                        .get(&id.to_string())
                        .map_or(0, |base| base.len())
                } else {
                    marks.members.len()
                }
            })
            .sum::<usize>() as u64;

        // Global changes: the whole section, or one member of every row.
        if seam.first_hour {
            seam.countries_all = Some("first hour");
        } else if !census_only && seam.last_tag_by_db_id != self.tag_by_db_id {
            seam.countries_all = Some("tag map changed");
        }
        if let (false, Some(last)) = (census_only, seam.last_country_flags) {
            if last.identity_known != flags.identity_known {
                seam.countries_all = Some("identity gate flipped");
            }
            if last.scripted_unknown != flags.scripted_unknown {
                for id in every_id() {
                    seam.mark_country_member(id, "extra");
                }
            }
        }

        // Projections whose input domains ran or whose gate flipped.
        let mut ran_set: BTreeSet<Projection> = BTreeSet::new();
        for (projection, domains) in PROJECTIONS {
            let ran_now = domains.iter().any(|&domain| self.v3_domain_ran(seam, domain));
            let gate_flipped = seam.last_country_flags.map_or(true, |last| match projection {
                Projection::Identity => {
                    last.identity_known != flags.identity_known
                        || last.domain53_known != flags.domain53_known
                },
                Projection::Collaboration => {
                    last.collaboration_present != flags.collaboration_present
                }
                Projection::AceRoster => last.ace_roster != flags.ace_roster,
                Projection::DynamicModifierNull => {
                    last.dynamic_modifier_null != flags.dynamic_modifier_null
                }
                Projection::Frd => last.frd_gate != flags.frd_gate,
                Projection::Domain53 => last.domain53_known != flags.domain53_known,
                Projection::LendLease => last.market_unknown != flags.market_unknown,
                _ => false,
            });
            if !census_only && (seam.countries_all.is_some() || ran_now || gate_flipped) {
                ran_set.insert(*projection);
            }
        }
        let mut probes: BTreeMap<Projection, BTreeSet<u32>> = BTreeMap::new();
        while !census_only {
            let mut changed = false;
            for (projection, _) in PROJECTIONS {
                if ran_set.contains(projection) {
                    continue;
                }
                let needed = co_owned(*projection).iter().any(|member| {
                    seam.country_marks
                        .values()
                        .any(|marks| marks.all || marks.members.contains(*member))
                });
                if needed {
                    ran_set.insert(*projection);
                    changed = true;
                }
            }
            for (projection, _) in PROJECTIONS {
                if !ran_set.contains(projection) || probes.contains_key(projection) {
                    continue;
                }
                let mut probe: Map<String, Value> = Map::new();
                let mut scratch = BTreeSet::new();
                self.v3_run_projection(*projection, &mut probe, flags, hour_index, &mut scratch);
                probes.insert(
                    *projection,
                    probe.keys().filter_map(|key| key.parse().ok()).collect(),
                );
                changed = true;
            }
            if !changed {
                break;
            }
        }
        let ran: Vec<Projection> = PROJECTIONS
            .iter()
            .map(|(projection, _)| *projection)
            .filter(|projection| ran_set.contains(projection))
            .collect();
        let mut whole_ids: BTreeSet<u32> = BTreeSet::new();
        if seam.countries_all.is_some() {
            whole_ids.extend(every_id());
            whole_ids.extend(seam.installed_countries.keys().copied());
        }
        for (id, marks) in &seam.country_marks {
            if marks.all {
                whole_ids.insert(*id);
            }
        }
        for (projection, ids) in &probes {
            whole_ids.extend(ids.iter().copied());
            for (id, installed) in &seam.installed_countries {
                if installed.projections.contains(projection) {
                    whole_ids.insert(*id);
                }
            }
        }

        // The key plan decides which rows change address; those are rendered
        // whole too. Every unresolved reference that stands feeds it.
        let mut unresolved: BTreeSet<u32> = BTreeSet::new();
        for installed in seam.installed_countries.values() {
            for record in installed.members.values() {
                unresolved.extend(record.unresolved.iter().copied());
            }
        }
        for ids in seam.unresolved_by_projection.values() {
            unresolved.extend(ids.iter().copied());
        }
        for rows in [
            &seam.installed_divisions,
            &seam.installed_states,
            &seam.installed_provinces,
        ] {
            for row in rows.values() {
                unresolved.extend(row.unresolved.iter().copied());
            }
        }
        for id in every_id() {
            if !self.tag_by_db_id.contains_key(&id) {
                unresolved.insert(id);
            }
        }
        let contended = self.contended_country_tags();
        let pre_plan = self.country_key_plan(&contended, &unresolved);
        for id in every_id() {
            if census_only {
                break;
            }
            let wanted = pre_plan.emitted_tag.get(&id).cloned().or_else(|| {
                (!flags.identity_known && !self.tag_by_db_id.contains_key(&id))
                    .then(|| id.to_string())
            });
            let Some(wanted) = wanted else {
                continue;
            };
            match seam.installed_countries.get(&id) {
                Some(installed) if installed.key == wanted => {}
                _ => {
                    whole_ids.insert(id);
                }
            }
        }
        // Parked collaboration rows are keyed by the plan.
        if !census_only && !seam.first_hour && seam.last_emitted_keys != pre_plan.emitted_tag {
            for id in every_id() {
                if self
                    .countries
                    .get(&id.to_string())
                    .is_some_and(|base| base.contains_key("collaboration"))
                {
                    seam.mark_country_member(id, "collaboration");
                }
            }
        }

        let mut base_rendered: BTreeMap<u32, BTreeMap<String, BTreeSet<u32>>> = BTreeMap::new();
        let mut staging: Map<String, Value> = Map::new();
        for id in &whole_ids {
            let row = ensure_country_projection_object(&mut staging, *id);
            let rendered = base_rendered.entry(*id).or_default();
            if let Some(base) = self.countries.get(&id.to_string()) {
                for member in base.keys() {
                    let mut u = BTreeSet::new();
                    if let Some(value) =
                        self.v3_base_member(base, member, flags.scripted_unknown, &mut u)
                    {
                        row.insert(member.clone(), value);
                    }
                    rendered.insert(member.clone(), u);
                }
            }
        }
        seam.unresolved_by_projection.clear();
        for (projection, _) in PROJECTIONS {
            if census_only {
                break;
            }
            let mut u = BTreeSet::new();
            self.v3_run_projection(*projection, &mut staging, flags, hour_index, &mut u);
            seam.unresolved_by_projection.insert(*projection, u);
        }
        // Rows nothing wrote are not in the build's map either.
        staging.retain(|key, row| {
            key.parse::<u32>().is_ok_and(|id| whole_ids.contains(&id))
                && (self.countries.contains_key(key)
                    || row.as_object().is_some_and(|row| !row.is_empty()))
        });
        let mut partial_members: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        for (id, marks) in &seam.country_marks {
            if whole_ids.contains(id) {
                continue;
            }
            let Some(base) = self.countries.get(&id.to_string()) else {
                continue;
            };
            let row = ensure_country_projection_object(&mut staging, *id);
            let rendered = base_rendered.entry(*id).or_default();
            for member in &marks.members {
                let mut u = BTreeSet::new();
                if let Some(value) =
                    self.v3_base_member(base, member, flags.scripted_unknown, &mut u)
                {
                    row.insert(member.clone(), value);
                }
                rendered.insert(member.clone(), u);
            }
            partial_members.insert(*id, marks.members.clone());
        }
        // The CURRENT unresolved set: members standing from earlier hours keep
        // theirs, members rendered this hour contribute what they found now,
        // the projections what they found now.
        let mut unresolved: BTreeSet<u32> = BTreeSet::new();
        for (id, installed) in &seam.installed_countries {
            if whole_ids.contains(id) {
                continue;
            }
            for (member, record) in &installed.members {
                let rendered_now = base_rendered
                    .get(id)
                    .is_some_and(|members| members.contains_key(member));
                if !rendered_now {
                    unresolved.extend(record.unresolved.iter().copied());
                }
            }
        }
        for members in base_rendered.values() {
            for ids in members.values() {
                unresolved.extend(ids.iter().copied());
            }
        }
        for ids in seam.unresolved_by_projection.values() {
            unresolved.extend(ids.iter().copied());
        }
        for rows in [
            &seam.installed_divisions,
            &seam.installed_states,
            &seam.installed_provinces,
        ] {
            for row in rows.values() {
                unresolved.extend(row.unresolved.iter().copied());
            }
        }
        for id in every_id() {
            if !self.tag_by_db_id.contains_key(&id) {
                unresolved.insert(id);
            }
        }

        // Emission: the staged rows go through `emit_countries_by_tag`
        // carrying their id, and come back keyed.
        let mut staged_ids: BTreeSet<u32> = whole_ids.clone();
        staged_ids.extend(partial_members.keys().copied());
        for (key, row) in staging.iter_mut() {
            if let (Ok(id), Some(row)) = (key.parse::<u32>(), row.as_object_mut()) {
                row.insert(V3_ID_MARKER.into(), json!(id));
            }
        }
        let CountryEmission {
            countries: emitted,
            withheld: _,
            key_plan: plan,
        } = self.emit_countries_by_tag(std::mem::take(&mut staging), &contended, &mut unresolved);
        let mut emitted_rows: BTreeMap<u32, (String, Map<String, Value>)> = BTreeMap::new();
        for (key, row) in emitted {
            let Value::Object(mut row) = row else {
                continue;
            };
            let Some(id) = row
                .remove(V3_ID_MARKER)
                .and_then(|marker| marker.as_u64())
                .map(|id| id as u32)
            else {
                continue;
            };
            resolve_country_collaboration_park_object(&mut row, &plan);
            emitted_rows.insert(id, (key, row));
        }

        // Rows gone: removed by a reducer, staged but withheld, or no longer
        // in the plan.
        let removed_ids: Vec<u32> = seam
            .installed_countries
            .keys()
            .copied()
            .filter(|id| {
                seam.country_removed.contains(id)
                    || (staged_ids.contains(id) && !emitted_rows.contains_key(id))
                    || (!plan.emitted_tag.contains_key(id)
                        && (flags.identity_known || self.tag_by_db_id.contains_key(id)))
            })
            .collect();
        for id in removed_ids {
            if let Some(installed) = seam.installed_countries.remove(&id) {
                for record in installed.members.values() {
                    out.removals.extend(record.paths.iter().cloned());
                }
            }
        }

        let mut unplanned_rekeys = 0u32;
        for (id, (tag, row)) in &emitted_rows {
            let id = *id;
            let installed = seam.installed_countries.entry(id).or_default();
            if installed.key != *tag {
                if !installed.key.is_empty() && !whole_ids.contains(&id) {
                    unplanned_rekeys += 1;
                }
                for record in installed.members.values() {
                    out.removals.extend(record.paths.iter().cloned());
                }
                installed.members.clear();
                installed.key = tag.clone();
            }
            out.rows_rendered += 1;
            let rendered = base_rendered.get(&id);
            let whole = whole_ids.contains(&id);
            let members: Vec<String> = if whole {
                row.keys().cloned().collect()
            } else {
                partial_members
                    .get(&id)
                    .map(|members| members.iter().cloned().collect())
                    .unwrap_or_default()
            };
            for member in members {
                let record = installed.members.entry(member.clone()).or_default();
                match row.get(&member) {
                    Some(value) => {
                        let prefix: Path = vec![Seg::key(tag), Seg::key(&member)];
                        let fresh = grains_of(value.clone(), depth_left, prefix);
                        replace_grains(&mut record.paths, fresh, &mut out);
                        out.members_rendered += 1;
                    }
                    None => {
                        out.removals.extend(record.paths.drain(..));
                    }
                }
                record.unresolved = rendered
                    .and_then(|members| members.get(&member))
                    .cloned()
                    .unwrap_or_default();
            }
            if whole {
                let gone: Vec<String> = installed
                    .members
                    .keys()
                    .filter(|member| !row.contains_key(*member))
                    .cloned()
                    .collect();
                for member in gone {
                    if let Some(record) = installed.members.remove(&member) {
                        out.removals.extend(record.paths);
                    }
                }
                let mut projections: BTreeSet<Projection> = installed
                    .projections
                    .iter()
                    .copied()
                    .filter(|projection| !ran_set.contains(projection))
                    .collect();
                for (projection, ids) in &probes {
                    if ids.contains(&id) {
                        projections.insert(*projection);
                    }
                }
                installed.projections = projections;
            }
            installed.members.retain(|_, record| !record.paths.is_empty());
        }
        let empty: Vec<u32> = seam
            .installed_countries
            .iter()
            .filter(|(_, installed)| installed.members.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for id in empty {
            seam.installed_countries.remove(&id);
        }

        out.widened_all = seam.countries_all;
        out.diagnostics = format!(
            "census_only={census_only} flags={flags:?} countries_all={:?} reducer_marked_rows={} reducer_marked_members={} whole_rows={} partial_rows={} unplanned_rekeys={unplanned_rekeys} ran={ran:?} probes={:?} domains_this_hour={:?}",
            seam.countries_all,
            out.reducer_marked_rows,
            out.reducer_marked_members,
            whole_ids.len(),
            partial_members.len(),
            probes
                .iter()
                .map(|(projection, ids)| (*projection, ids.len()))
                .collect::<Vec<_>>(),
            self.domains_this_hour
        );
        seam.last_emitted_keys = plan.emitted_tag.clone();
        seam.last_country_flags = Some(flags);
        out.render_ns = started.elapsed().as_nanos() as u64;
        out
    }

    fn v3_render_divisions(&self, seam: &mut V3Seam) -> V3SectionHour {
        let started = Instant::now();
        let mut out = empty_section(V3_SECTION_DIVISIONS);
        let depth_left = seam.config.divisions_depth - 1;
        let equipment_masked = self.fold_domain_unknown(DIVISION_EQUIPMENT_DOMAIN_ID)
            || (self.fold_domain_seen_in_stream(DIVISION_EQUIPMENT_DOMAIN_ID)
                && !self.fold_domain_fresh_this_hour(DIVISION_EQUIPMENT_DOMAIN_ID));
        let equipment_present = self.fold_domain_seen_in_stream(DIVISION_EQUIPMENT_DOMAIN_ID)
            || !self.division_equipment.is_empty();
        let flags = DivisionFlags {
            unknown: self.divisions_unknown_this_hour,
            equipment_masked,
            equipment_present,
        };
        let census_only = seam.config.census_only;
        out.reducer_marked_rows = if seam.reducer_all_divisions {
            (self.divisions.len()
                + seam
                    .installed_divisions
                    .keys()
                    .filter(|uid| !self.divisions.contains_key(*uid))
                    .count()) as u64
        } else {
            (seam.division_marks.len() + seam.division_removed.len()) as u64
        };
        out.reducer_marked_members = out.reducer_marked_rows;
        if seam.first_hour {
            seam.divisions_all = Some("first hour");
            // The join index starts from the retained set.
            seam.division_key_of.clear();
            seam.division_buckets.clear();
            for (uid, row) in &self.divisions {
                let key = row.equipment_join_key();
                seam.division_key_of.insert(uid.clone(), key);
                if let Some(key) = key {
                    seam.division_buckets
                        .entry(key)
                        .or_default()
                        .insert(uid.clone());
                }
            }
        } else if seam.last_division_flags.is_some_and(|previous| previous.unknown) && !flags.unknown {
            seam.divisions_all = Some("D20 availability restored");
        } else if census_only {
        } else if seam.last_tag_by_db_id != self.tag_by_db_id {
            seam.divisions_all = Some("tag map changed");
        } else if seam.last_division_flags != Some(flags) {
            seam.divisions_all = Some("equipment gate flipped");
        }
        if flags.unknown {
            // Withhold published grains, not the reducer's retained baseline.
            for (_, row) in std::mem::take(&mut seam.installed_divisions) {
                out.removals.extend(row.paths);
            }
            out.root_kind = RootKind::Scalar;
            out.upserts.push((Vec::new(), b"null".to_vec()));
            out.rendered_bytes = 4;
            out.widened_all = Some("D20 unavailable");
            out.diagnostics = format!("flags={flags:?}; retained baseline withheld");
            seam.last_division_flags = Some(flags);
            out.render_ns = started.elapsed().as_nanos() as u64;
            return out;
        }
        if seam.last_division_flags.is_some_and(|previous| previous.unknown) {
            // Remove the null grain before restoring the array, including [].
            out.removals.push(Vec::new());
        }
        // D26 touched or pruned a key: its bucket's divisions join differently.
        if seam.divisions_all.is_none() && !census_only {
            let keys: Vec<DivisionEquipmentKey> = self
                .division_equipment_touched_this_hour
                .iter()
                .chain(self.division_equipment_deleted_division_keys_this_hour.iter())
                .copied()
                .collect();
            for key in keys {
                seam.mark_bucket(key);
            }
        }
        // Rows gone.
        let gone: Vec<String> = seam
            .installed_divisions
            .keys()
            .filter(|uid| !self.divisions.contains_key(*uid))
            .cloned()
            .collect();
        for uid in gone {
            if let Some(row) = seam.installed_divisions.remove(&uid) {
                out.removals.extend(row.paths);
            }
        }
        let uids: Vec<&String> = if seam.divisions_all.is_some() {
            self.divisions.keys().collect()
        } else {
            seam.division_marks
                .iter()
                .filter(|uid| self.divisions.contains_key(*uid))
                .collect()
        };
        out.widened_all = seam.divisions_all;
        if uids.is_empty() {
            seam.last_division_flags = Some(flags);
            out.render_ns = started.elapsed().as_nanos() as u64;
            return out;
        }
        let mut join_index: BTreeMap<DivisionEquipmentKey, Vec<&str>> = BTreeMap::new();
        for (uid, row) in &self.divisions {
            if let Some(key) = row.equipment_join_key() {
                join_index.entry(key).or_default().push(uid.as_str());
            }
        }
        for uid in uids {
            let row = &self.divisions[uid];
            let mut document = Self::fold_division_document(row);
            if equipment_masked {
                if equipment_present {
                    document.insert("equipment".into(), Value::Null);
                }
            } else if let Some(key) = row.equipment_join_key() {
                if join_index.get(&key).map(Vec::as_slice) == Some(&[uid.as_str()][..]) {
                    if let Some(equipment) = self.division_equipment.get(&key) {
                        document.insert(
                            "equipment".into(),
                            Value::Array(
                                equipment
                                    .iter()
                                    .map(DivisionEquipmentEntryState::to_json)
                                    .collect(),
                            ),
                        );
                    }
                }
            }
            let mut unresolved = BTreeSet::new();
            self.resolve_country_value(document.get_mut("owner"), &mut unresolved);
            let fresh = grains_of(Value::Object(document), depth_left, vec![Seg::key(uid)]);
            let installed = seam.installed_divisions.entry(uid.clone()).or_default();
            replace_grains(&mut installed.paths, fresh, &mut out);
            installed.unresolved = unresolved;
            out.rows_rendered += 1;
            out.members_rendered += 1;
        }
        seam.last_division_flags = Some(flags);
        out.diagnostics = format!(
            "flags={flags:?} divisions_all={:?} reducer_marked={}",
            seam.divisions_all, out.reducer_marked_rows
        );
        out.render_ns = started.elapsed().as_nanos() as u64;
        out
    }

    fn v3_state_flags(&self) -> StateFlags {
        StateFlags {
            map_control_unknown: self.map_control_unknown_this_hour,
            occupation_unknown: self.occupation_unknown_this_hour,
            map_detail_unknown: self.map_detail_unknown_this_hour,
            scripted_unknown: self.fold_domain_unknown(SCRIPTED_STATE_DOMAIN_ID),
        }
    }

    fn v3_state_projection_targets(&self) -> BTreeSet<u32> {
        let mut probe: Map<String, Value> = Map::new();
        self.project_domain48_states(&mut probe);
        probe.keys().filter_map(|key| key.parse().ok()).collect()
    }

    fn v3_province_projection_targets(&self) -> BTreeSet<u32> {
        let mut probe: Map<String, Value> = Map::new();
        self.project_domain48_provinces(&mut probe);
        probe.keys().filter_map(|key| key.parse().ok()).collect()
    }

    fn v3_render_states(&self, seam: &mut V3Seam) -> V3SectionHour {
        let started = Instant::now();
        let mut out = empty_section(V3_SECTION_STATES);
        let depth_left = seam.config.states_depth - 1;
        let flags = self.v3_state_flags();
        let census_only = seam.config.census_only;
        if seam.first_hour {
            seam.states_all = Some("first hour");
        } else if census_only {
        } else if seam.last_tag_by_db_id != self.tag_by_db_id {
            seam.states_all = Some("tag map changed");
        } else if seam.last_state_flags != Some(flags) {
            seam.states_all = Some("a gate flipped");
        }
        let mut projection_targets: BTreeSet<u32> = BTreeSet::new();
        if !census_only && self.v3_domain_ran(seam, CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID) {
            let now = self.v3_state_projection_targets();
            projection_targets.extend(seam.last_d48_state_targets.iter().copied());
            projection_targets.extend(now.iter().copied());
            seam.last_d48_state_targets = now;
        }
        if seam.states_all.is_none() {
            for sid in &projection_targets {
                seam.state_marks.insert(*sid);
            }
        }
        let state_keys: BTreeSet<String> = self
            .states
            .keys()
            .chain(self.map_control_states.keys())
            .cloned()
            .collect();
        out.reducer_marked_rows = if seam.reducer_all_states {
            (state_keys.len()
                + seam
                    .installed_states
                    .keys()
                    .filter(|sid| !state_keys.contains(*sid))
                    .count()) as u64
        } else {
            seam.state_marks.len() as u64
        };
        out.reducer_marked_members = out.reducer_marked_rows;
        let gone: Vec<String> = seam
            .installed_states
            .keys()
            .filter(|sid| !state_keys.contains(*sid))
            .cloned()
            .collect();
        for sid in gone {
            if let Some(row) = seam.installed_states.remove(&sid) {
                out.removals.extend(row.paths);
            }
        }
        let sids: Vec<String> = if seam.states_all.is_some() {
            state_keys.iter().cloned().collect()
        } else {
            seam.state_marks
                .iter()
                .map(|sid| sid.to_string())
                .filter(|sid| state_keys.contains(sid))
                .collect()
        };
        out.widened_all = seam.states_all;
        if sids.is_empty() {
            seam.last_state_flags = Some(flags);
            out.render_ns = started.elapsed().as_nanos() as u64;
            return out;
        }
        let mut states: Map<String, Value> = Map::new();
        let mut unresolved_by_sid: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for sid in &sids {
            let mut unresolved = BTreeSet::new();
            let mut row = self.states.get(sid).cloned().unwrap_or_default();
            if let Some(map_control) = self.map_control_states.get(sid) {
                for key in ["owner", "controller"] {
                    if let Some(value) = map_control.get(key) {
                        row.insert(
                            key.into(),
                            if flags.map_control_unknown {
                                Value::Null
                            } else {
                                value.clone()
                            },
                        );
                    }
                }
            }
            if !flags.map_control_unknown {
                self.resolve_country_value(row.get_mut("owner"), &mut unresolved);
                self.resolve_country_value(row.get_mut("controller"), &mut unresolved);
            }
            if flags.occupation_unknown {
                if row.contains_key("occupation") {
                    row.insert("occupation".into(), Value::Null);
                }
                if row.contains_key("occupation_law") {
                    row.insert("occupation_law".into(), Value::Null);
                }
            } else {
                self.resolve_occupation_fields(&mut row, &mut unresolved);
            }
            if flags.map_detail_unknown {
                for key in ["resources", "buildings"] {
                    if row.contains_key(key) {
                        row.insert(key.into(), Value::Null);
                    }
                }
            }
            if flags.scripted_unknown && row.contains_key("_r2_scripted_state") {
                row.insert("_r2_scripted_state".into(), Value::Null);
            }
            states.insert(sid.clone(), Value::Object(row));
            unresolved_by_sid.insert(sid.clone(), unresolved);
        }
        if !census_only {
            self.project_map_detail_province_building_totals(&mut states);
            self.project_domain48_states(&mut states);
        }
        for sid in sids {
            let Some(value) = states.remove(&sid) else {
                continue;
            };
            let fresh = grains_of(value, depth_left, vec![Seg::key(&sid)]);
            let unresolved = unresolved_by_sid.remove(&sid).unwrap_or_default();
            let installed = seam.installed_states.entry(sid).or_default();
            replace_grains(&mut installed.paths, fresh, &mut out);
            installed.unresolved = unresolved;
            out.rows_rendered += 1;
            out.members_rendered += 1;
        }
        seam.last_state_flags = Some(flags);
        out.diagnostics = format!(
            "flags={flags:?} states_all={:?} reducer_marked={} projection_targets={}",
            seam.states_all,
            out.reducer_marked_rows,
            projection_targets.len()
        );
        out.render_ns = started.elapsed().as_nanos() as u64;
        out
    }

    fn v3_render_provinces(&self, seam: &mut V3Seam) -> V3SectionHour {
        let started = Instant::now();
        let mut out = empty_section(V3_SECTION_PROVINCES);
        let depth_left = seam.config.provinces_depth - 1;
        let map_control_unknown = self.map_control_unknown_this_hour;
        let census_only = seam.config.census_only;
        if seam.first_hour {
            seam.provinces_all = Some("first hour");
        } else if census_only {
        } else if seam.last_tag_by_db_id != self.tag_by_db_id {
            seam.provinces_all = Some("tag map changed");
        } else if seam.last_map_control_unknown != Some(map_control_unknown) {
            seam.provinces_all = Some("map control gate flipped");
        }
        seam.last_map_control_unknown = Some(map_control_unknown);
        let mut projection_targets: BTreeSet<u32> = BTreeSet::new();
        if !census_only && self.v3_domain_ran(seam, CONSTRUCTION_BUILDINGS_STATE_DAMAGE_DOMAIN_ID) {
            let now = self.v3_province_projection_targets();
            projection_targets.extend(seam.last_d48_province_targets.iter().copied());
            projection_targets.extend(now.iter().copied());
            seam.last_d48_province_targets = now;
        }
        if seam.provinces_all.is_none() {
            for pid in &projection_targets {
                seam.province_marks.insert(*pid);
            }
        }
        let province_keys: BTreeSet<String> = self
            .provinces
            .keys()
            .chain(self.map_control_provinces.keys())
            .cloned()
            .collect();
        out.reducer_marked_rows = if seam.reducer_all_provinces {
            (province_keys.len()
                + seam
                    .installed_provinces
                    .keys()
                    .filter(|pid| !province_keys.contains(*pid))
                    .count()) as u64
        } else {
            seam.province_marks.len() as u64
        };
        out.reducer_marked_members = out.reducer_marked_rows;
        let gone: Vec<String> = seam
            .installed_provinces
            .keys()
            .filter(|pid| !province_keys.contains(*pid))
            .cloned()
            .collect();
        for pid in gone {
            if let Some(row) = seam.installed_provinces.remove(&pid) {
                out.removals.extend(row.paths);
            }
        }
        let pids: Vec<String> = if seam.provinces_all.is_some() {
            province_keys.iter().cloned().collect()
        } else {
            seam.province_marks
                .iter()
                .map(|pid| pid.to_string())
                .filter(|pid| province_keys.contains(pid))
                .collect()
        };
        out.widened_all = seam.provinces_all;
        if pids.is_empty() {
            out.render_ns = started.elapsed().as_nanos() as u64;
            return out;
        }
        let mut provinces: Map<String, Value> = Map::new();
        let mut unresolved_by_pid: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
        for pid in &pids {
            let mut unresolved = BTreeSet::new();
            let mut row = self.provinces.get(pid).cloned().unwrap_or_default();
            if let Some(map_control) = self.map_control_provinces.get(pid) {
                for key in ["controller", "state_id"] {
                    if let Some(value) = map_control.get(key) {
                        row.insert(
                            key.into(),
                            if map_control_unknown {
                                Value::Null
                            } else {
                                value.clone()
                            },
                        );
                    }
                }
            }
            if !map_control_unknown {
                self.resolve_country_value(row.get_mut("controller"), &mut unresolved);
            }
            provinces.insert(pid.clone(), Value::Object(row));
            unresolved_by_pid.insert(pid.clone(), unresolved);
        }
        if !census_only {
            self.project_domain48_provinces(&mut provinces);
        }
        for pid in pids {
            let Some(value) = provinces.remove(&pid) else {
                continue;
            };
            let fresh = grains_of(value, depth_left, vec![Seg::key(&pid)]);
            let unresolved = unresolved_by_pid.remove(&pid).unwrap_or_default();
            let installed = seam.installed_provinces.entry(pid).or_default();
            replace_grains(&mut installed.paths, fresh, &mut out);
            installed.unresolved = unresolved;
            out.rows_rendered += 1;
            out.members_rendered += 1;
        }
        out.diagnostics = format!(
            "provinces_all={:?} reducer_marked={} projection_targets={}",
            seam.provinces_all,
            out.reducer_marked_rows,
            projection_targets.len()
        );
        out.render_ns = started.elapsed().as_nanos() as u64;
        out
    }
}
