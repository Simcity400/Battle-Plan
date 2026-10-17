//! Territory totals from complete, freshly read States and Provinces tables.
//! A read whose measured fields are unchanged reuses its totals. A failed or
//! unrecorded read supplies no totals, including for countries that held zero
//! territory at the last read.
use super::{
    LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_groups::{HourSpan, record_hour},
    country_history::{CountryIdentityRun, RecordedHistories},
    validate_tag,
};
use crate::{
    Source, content,
    metadata::{check_cancel, slot_read},
    protocol::MAX_EXACT_JS_INTEGER,
    transport::json_len,
};
#[cfg(any(test, feature = "test-support"))]
use crate::{content::ContentKey, metadata::validate_campaign};
use installed_content::province_area::ProvinceAreas;
#[cfg(any(test, feature = "test-support"))]
use replay::{container::Replay, frame::FrameDecoder};
use replay::{
    frame::{DecodedFrame, ViewIdentity},
    rows::{CountryRow, ProvinceRow, StateRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::collections::VecDeque;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct Territory;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TerritoryMeasure {
    States,
    Provinces,
    VictoryPoints,
    Area,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TerritoryRule {
    Controlled,
    Owned,
}

impl TerritoryMeasure {
    pub fn index(self, rule: TerritoryRule) -> usize {
        let measure = match self {
            Self::States => 0,
            Self::Provinces => 1,
            Self::VictoryPoints => 2,
            Self::Area => 3,
        };
        measure
            + match rule {
                TerritoryRule::Controlled => 0,
                TerritoryRule::Owned => 4,
            }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryTop {
    #[ts(type = "number")]
    pub count: u64,
    pub measure: TerritoryMeasure,
    pub rule: TerritoryRule,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerritoryParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<TerritoryTop>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    /// States, provinces, victory points, area: controlled first, then owned.
    /// Null means that measure/rule was not recorded, never zero.
    #[ts(type = "Array<number | null>")]
    pub values: [Option<i64>; 8],
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritorySeries {
    pub tag: String,
    pub runs: Vec<TerritoryRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    /// Fixed picks follow native holder instances through their recorded aliases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<CountryIdentityRun>>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerritoryReply {
    pub branch_id: String,
    /// The complete world's totals, including unheld territory, independent of selection.
    pub world_totals: Vec<TerritoryRun>,
    pub series: Vec<TerritorySeries>,
    /// No physical calibration is present in the province map assets.
    pub area_unit: Option<String>,
    /// A named refusal hides the measure in the settings; other measures work.
    pub area_reason: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Track {
    runs: Vec<TerritoryRun>,
    segments: Vec<HourSpan>,
    present: Vec<HourSpan>,
    last: Option<((u32, u32), i32)>,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    #[serde(skip)]
    pub format: replay::format::FormatVersion,
    world_totals: Vec<TerritoryRun>,
    overflowed: usize,
    ambiguous_slots: usize,
    #[serde(serialize_with = "crate::prepared_production::map_pairs::ordered_map")]
    series: HashMap<Arc<str>, Track>,
    rosters: Vec<HourSpan>,
    incomplete: usize,
}

struct Lineage {
    reads: CountryReads,
    raw_slots: HashMap<u32, usize>,
    entries: Vec<(Arc<str>, (u32, u32), usize)>,
    present: Vec<Arc<str>>,
    slot_count: usize,
    incomplete: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Builder {
    #[serde(skip)]
    lineage: Option<Lineage>,
    #[serde(skip)]
    previous: Vec<[Option<i64>; 8]>,
    #[serde(skip)]
    current: Vec<[Option<i64>; 8]>,
    previous_hour: Option<i32>,
    /// The last land reads and, under the current lineage, their totals.
    #[serde(skip)]
    reads: LandReads,
    #[serde(skip)]
    totals: Option<Totals>,
    #[cfg(any(test, feature = "test-support"))]
    aggregations: usize,
    incomplete: bool,
    #[serde(skip)]
    changed: BTreeMap<Arc<str>, [bool; 8]>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}

/// What a holder lineage reads of the country tables: each row's id, canonical
/// slot and tag, in order, and the links by identity, which holds none of their
/// rows. Countries change hourly in their balances alone.
pub struct CountryReads {
    links: ViewIdentity,
    countries: Vec<(u32, u32, Vec<u8>)>,
    slots: Arc<HashMap<u32, u32>>,
    holders: BTreeMap<String, Option<(u32, u32)>>,
    partners: BTreeMap<u32, Option<String>>,
    #[cfg(test)]
    comparisons: std::sync::atomic::AtomicUsize,
}

impl CountryReads {
    pub fn new(frame: &DecodedFrame, countries: &[CountryRow]) -> Self {
        let slots = frame
            .complete::<tables::CountryLinks>()
            .map(crate::metadata::link_slots)
            .unwrap_or_default();
        Self::from_slots(frame, countries, Arc::new(slots))
    }

    /// Refresh country holders without rereading unchanged links. The caller
    /// supplies the slot projection of this frame's CountryLinks allocation.
    pub fn from_slots(
        frame: &DecodedFrame,
        countries: &[CountryRow],
        slots: Arc<HashMap<u32, u32>>,
    ) -> Self {
        let holders = crate::metadata::tagged_countries_indexed(countries, &slots)
            .into_iter()
            .filter_map(|(tag, lookup)| {
                std::str::from_utf8(tag).ok().map(|tag| {
                    (
                        tag.to_owned(),
                        lookup.found().map(|(country, slot)| (slot, country.id)),
                    )
                })
            })
            .collect();
        let mut partners = BTreeMap::new();
        for country in countries {
            partners
                .entry(country.canonical_slot)
                .or_insert_with(|| std::str::from_utf8(&country.tag).ok().map(str::to_owned));
        }
        Self {
            links: frame.tables[TableId::CountryLinks.index()].identity(),
            countries: countries
                .iter()
                .map(|country| (country.id, country.canonical_slot, country.tag.clone()))
                .collect(),
            slots,
            holders,
            partners,
            #[cfg(test)]
            comparisons: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn slots(&self) -> &Arc<HashMap<u32, u32>> {
        &self.slots
    }
    pub fn holders(&self) -> &BTreeMap<String, Option<(u32, u32)>> {
        &self.holders
    }
    pub fn partners(&self) -> &BTreeMap<u32, Option<String>> {
        &self.partners
    }

    pub fn matches(&self, frame: &DecodedFrame, countries: &[CountryRow]) -> bool {
        self.links.is(&frame.tables[TableId::CountryLinks.index()])
            && self.countries.len() == countries.len()
            && self
                .countries
                .iter()
                .zip(countries)
                .all(|((id, slot, tag), country)| {
                    #[cfg(test)]
                    self.comparisons
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    *id == country.id && *slot == country.canonical_slot && *tag == country.tag
                })
    }

    pub fn matches_changes(
        &self,
        frame: &DecodedFrame,
        countries: &[CountryRow],
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> bool {
        static FIELDS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
            crate::prepared::frame_changes::field_mask(
                TableId::Countries,
                &["canonical_slot", "tag"],
            )
        });
        self.links.is(&frame.tables[TableId::CountryLinks.index()])
            && (changes.is_some_and(|changes| {
                self.countries.len() == countries.len()
                    && changes
                        .rows(TableId::Countries)
                        .is_some_and(|(changed, deleted)| {
                            deleted.is_empty() && changed.iter().all(|row| row.mask & *FIELDS == 0)
                        })
            }) || self.matches(frame, countries))
    }
}

/// What territory measures read of a fresh States and Provinces read: each state's
/// id and holders, and each province's id, holders and victory points. Their
/// other fields change hourly.
#[derive(Default)]
pub struct LandReads {
    states: Option<Vec<[u32; 3]>>,
    provinces: Option<Vec<([u32; 3], i32)>>,
}

impl LandReads {
    /// Compare only changed/read tables; absence and freshness remain operands.
    pub fn update_changes(
        &mut self,
        states: Option<&[StateRow]>,
        provinces: Option<&[ProvinceRow]>,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> [bool; 2] {
        let compare = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        let states = (!compare(TableId::States) && self.states.is_some() == states.is_some())
            || keep(&mut self.states, states, |state| {
                [state.id, state.controller, state.owner]
            });
        let provinces = (!compare(TableId::Provinces)
            && self.provinces.is_some() == provinces.is_some())
            || keep(&mut self.provinces, provinces, |province| {
                (
                    [province.id, province.controller, province.owner],
                    province.victory_points,
                )
            });
        [states, provinces]
    }
}

impl Builder {
    fn seal_world(&self, scan: &mut Scan) {
        let Some(hour) = self.previous_hour else {
            return;
        };
        if let Some(run) = scan.world_totals.last_mut() {
            let elapsed = usize::try_from(hour.saturating_sub(run.last_hour)).unwrap_or(0);
            if let Some(totals) = self.totals {
                scan.overflowed = scan.overflowed.saturating_add(
                    totals
                        .overflowed
                        .iter()
                        .sum::<usize>()
                        .saturating_mul(elapsed),
                );
                scan.incomplete += usize::from(self.incomplete || totals.missing_area) * elapsed;
            }
            run.last_hour = hour;
        }
        if let Some(span) = scan.rosters.last_mut() {
            span.last_hour = hour;
        }
    }

    /// Publication materializes only the currently open native holders.
    fn seal(&mut self, scan: &mut Scan) {
        self.seal_world(scan);
        let Some(hour) = self.previous_hour else {
            return;
        };
        let Some(index) = &self.lineage else { return };
        for tag in &index.present {
            if let Some(span) = scan
                .series
                .get_mut(tag)
                .and_then(|track| track.present.last_mut())
            {
                span.last_hour = hour;
            }
        }
        for (tag, _, _) in &index.entries {
            seal_track(scan.series.get_mut(tag).expect("interned tag"), hour);
        }
    }
}

fn seal_track(track: &mut Track, hour: i32) {
    let Some((identity, before)) = track.last else {
        return;
    };
    if let Some(span) = track.segments.last_mut() {
        span.last_hour = hour;
    }
    if let Some(run) = track.runs.last_mut()
        && run.last_hour == before
    {
        run.last_hour = hour;
    }
    track.last = Some((identity, hour));
}

fn keep<R, T: Copy + PartialEq>(
    kept: &mut Option<Vec<T>>,
    rows: Option<&[R]>,
    read: impl Fn(&R) -> T,
) -> bool {
    let same = match (&*kept, rows) {
        (Some(kept), Some(rows)) => kept.iter().copied().eq(rows.iter().map(&read)),
        (kept, rows) => kept.is_none() && rows.is_none(),
    };
    if !same {
        *kept = rows.map(|rows| rows.iter().map(&read).collect());
    }
    same
}

/// This hour's complete States and Provinces when freshly read; a failed or
/// unrecorded read supplies none.
pub fn fresh_tables(
    format: replay::format::FormatVersion,
    frame: &DecodedFrame,
) -> (Option<&[StateRow]>, Option<&[ProvinceRow]>) {
    let fresh = |table| slot_read(format, &frame.metrics, table, frame.hour);
    (
        fresh(TableId::States)
            .then(|| frame.complete::<tables::States>().ok())
            .flatten(),
        fresh(TableId::Provinces)
            .then(|| frame.complete::<tables::Provinces>().ok())
            .flatten(),
    )
}

/// A non-null unresolved holder could belong to any country, so its rule is
/// unknown globally. Null holders, including with canonical slot zero, hold none.
#[derive(Clone, Copy)]
struct Totals {
    missing_area: bool,
    world: [Option<i64>; 8],
    overflowed: [usize; 8],
}

pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::Provinces,
];
pub const SAMPLE_FIELDS: [&str; 8] = [
    "controlledStates",
    "controlledProvinces",
    "controlledVictoryPoints",
    "controlledArea",
    "ownedStates",
    "ownedProvinces",
    "ownedVictoryPoints",
    "ownedArea",
];

/// The empty tag addresses independent world totals, never a country template.
/// Presence and complete-country-read proof also serve the existing group sum.
#[allow(clippy::type_complexity)]
pub fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
    areas: Option<&ProvinceAreas>,
    format: replay::format::FormatVersion,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    if requested.is_some_and(|keys| {
        keys.iter().any(|(_, field)| {
            !SAMPLE_FIELDS.contains(&field.as_str())
                && !["present", "roster"].contains(&field.as_str())
        })
    }) {
        return Err(crate::invalid("Unknown territory sample field"));
    }
    let mut result = BTreeMap::new();
    if let (Ok(countries), Ok(_)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) {
        let mut scan = Scan::default();
        let index = lineage(&mut scan, CountryReads::new(frame, countries));
        let mut values = Vec::new();
        let totals = sample_rows(
            format,
            frame,
            areas,
            &index.raw_slots,
            index.slot_count,
            &mut values,
            None,
            [true; 2],
        );
        for tag in &index.present {
            for field in SAMPLE_FIELDS {
                result.insert((tag.to_string(), field.to_owned()), (None, None));
            }
            result.insert((tag.to_string(), "present".into()), (None, Some(1)));
        }
        for (tag, identity, slot) in &index.entries {
            for (field, amount) in SAMPLE_FIELDS.iter().zip(values[*slot]) {
                result.insert(
                    (tag.to_string(), (*field).to_owned()),
                    (Some(*identity), amount.map(i128::from)),
                );
            }
            result.insert(
                (tag.to_string(), "present".into()),
                (Some(*identity), Some(1)),
            );
        }
        for (field, amount) in SAMPLE_FIELDS.iter().zip(totals.world) {
            result.insert(
                (String::new(), (*field).to_owned()),
                (
                    None,
                    (!index.incomplete)
                        .then_some(amount)
                        .flatten()
                        .map(i128::from),
                ),
            );
        }
        result.insert((String::new(), "roster".into()), (None, Some(1)));
    }
    if let Some(requested) = requested {
        let mut selected = BTreeMap::new();
        for key in requested {
            selected.insert(
                key.clone(),
                result.get(key).copied().unwrap_or((None, None)),
            );
        }
        return Ok(selected);
    }
    Ok(result)
}

/// The same exact province weights serve totals and geographic breakdowns.
pub fn province_values(
    province: &replay::rows::ProvinceRow,
    areas: Option<&ProvinceAreas>,
) -> [Option<i64>; 3] {
    [
        Some(1),
        Some(i64::from(province.victory_points)),
        areas.and_then(|areas| areas.land.get(province.id as usize).copied()),
    ]
}

#[allow(clippy::too_many_arguments)]
fn sample_rows(
    format: replay::format::FormatVersion,
    frame: &DecodedFrame,
    areas: Option<&ProvinceAreas>,
    raw_slots: &HashMap<u32, usize>,
    slot_count: usize,
    result: &mut Vec<[Option<i64>; 8]>,
    previous: Option<(Totals, &[[Option<i64>; 8]])>,
    refresh: [bool; 2],
) -> Totals {
    let (states, provinces) = fresh_tables(format, frame);
    let mut available = [
        states.is_some(),
        provinces.is_some(),
        provinces.is_some(),
        provinces.is_some() && areas.is_some(),
        states.is_some(),
        provinces.is_some(),
        provinces.is_some(),
        provinces.is_some() && areas.is_some(),
    ];
    result.resize(slot_count, [Some(0); 8]);
    let mut totals = previous.map_or(
        Totals {
            missing_area: false,
            world: [Some(0); 8],
            overflowed: [0; 8],
        },
        |(totals, _)| totals,
    );
    if let Some((_, values)) = previous {
        result.copy_from_slice(values);
    }
    for column in 0..8 {
        if refresh[usize::from(column % 4 != 0)] {
            for sums in result.iter_mut() {
                sums[column] = Some(0);
            }
            totals.world[column] = Some(0);
            totals.overflowed[column] = 0;
        }
    }
    let world = &mut totals.world;
    let mut add = |raw: u32, column: usize, values: &[Option<i64>]| {
        for (index, value) in values.iter().enumerate() {
            world[column + index] = world[column + index]
                .zip(*value)
                .and_then(|(sum, value)| sum.checked_add(value));
        }
        if raw == 0 {
            return;
        }
        let Some(&slot) = raw_slots.get(&raw) else {
            available[column..column + values.len()].fill(false);
            return;
        };
        let sums = &mut result[slot];
        for (index, value) in values.iter().enumerate() {
            sums[column + index] = sums[column + index]
                .zip(*value)
                .and_then(|(sum, value)| sum.checked_add(value));
        }
    };
    for state in states.filter(|_| refresh[0]).into_iter().flatten() {
        add(state.controller, 0, &[Some(1)]);
        add(state.owner, 4, &[Some(1)]);
    }
    let mut missing_area = !refresh[1] && totals.missing_area;
    for province in provinces.filter(|_| refresh[1]).into_iter().flatten() {
        let values = province_values(province, areas);
        missing_area |= areas.is_some() && values[2].is_none();
        for (raw, offset) in [(province.controller, 0), (province.owner, 4)] {
            add(raw, offset + 1, &values);
        }
    }
    let overflowed = sanitise_totals(available, result, world, missing_area, refresh);
    for column in 0..8 {
        if refresh[usize::from(column % 4 != 0)] {
            totals.overflowed[column] = overflowed[column];
        }
    }
    totals.missing_area = missing_area;
    totals
}

/// A world-share ring also needs every candidate country to be rankable. A
/// safe signed world sum cannot conceal an overflowing country contribution.
fn sanitise_totals(
    available: [bool; 8],
    countries: &mut [[Option<i64>; 8]],
    world: &mut [Option<i64>; 8],
    missing_area: bool,
    refresh: [bool; 2],
) -> [usize; 8] {
    let mut overflowed = [0usize; 8];
    for (column, known) in available.into_iter().enumerate() {
        if !refresh[usize::from(column % 4 != 0)] {
            continue;
        }
        if !known {
            world[column] = None;
            for sums in countries.iter_mut() {
                sums[column] = None;
            }
            continue;
        }
        let unsafe_value = |value: Option<i64>| {
            value.is_none() && (column % 4 != 3 || !missing_area)
                || value.is_some_and(|value| value.unsigned_abs() > MAX_EXACT_JS_INTEGER)
        };
        let mut country_overflow = false;
        for sums in countries.iter_mut() {
            if unsafe_value(sums[column]) {
                overflowed[column] += 1;
                sums[column] = None;
                country_overflow = true;
            }
        }
        if unsafe_value(world[column]) {
            overflowed[column] += 1;
            world[column] = None;
        }
        if country_overflow {
            world[column] = None;
        }
    }
    overflowed
}

/// One native holder join serves the cached observer and transient sampler.
fn lineage(scan: &mut Scan, reads: CountryReads) -> Lineage {
    let mut entries = Vec::new();
    let mut present = Vec::new();
    let mut incomplete = reads
        .countries
        .iter()
        .any(|(_, _, tag)| std::str::from_utf8(tag).is_err());
    let mut dense_slots = HashMap::new();
    for (tag, identity) in reads.holders() {
        let tag = tag.as_str();
        if tag.is_empty() {
            continue;
        }
        let tag = match scan.series.get_key_value(tag) {
            Some((tag, _)) => Arc::clone(tag),
            None => {
                let tag: Arc<str> = tag.into();
                scan.series.insert(Arc::clone(&tag), Track::default());
                tag
            }
        };
        present.push(Arc::clone(&tag));
        if let Some((slot, country)) = *identity {
            let next = dense_slots.len();
            let index = *dense_slots.entry(slot).or_insert(next);
            entries.push((tag, (slot, country), index));
        } else {
            incomplete = true;
        }
    }
    let mut slot_tags = HashMap::new();
    let mut ambiguous = HashSet::new();
    for (tag, (slot, _), _) in &entries {
        if slot_tags
            .insert(*slot, tag)
            .is_some_and(|previous| previous != tag)
        {
            ambiguous.insert(*slot);
        }
    }
    if !ambiguous.is_empty() {
        incomplete = true;
        scan.ambiguous_slots += ambiguous.len();
        entries.retain(|(_, (slot, _), _)| !ambiguous.contains(slot));
    }
    // A mapped slot without a named country still cannot attribute its
    // non-null holders. Exclude it before deciding each measure's coverage.
    // Canonical slots are recorded identities, not allocation sizes.
    let raw_slots = reads
        .slots()
        .iter()
        .filter(|(_, slot)| !ambiguous.contains(*slot))
        .filter_map(|(raw, slot)| dense_slots.get(slot).map(|&index| (*raw, index)))
        .collect();
    let slot_count = dense_slots.len();
    Lineage {
        reads,
        raw_slots,
        entries,
        present,
        slot_count,
        incomplete,
    }
}

fn observe(
    scan: &mut Scan,
    frame: &DecodedFrame,
    areas: Option<&ProvinceAreas>,
    builder: &mut Builder,
    changes: Option<&crate::prepared::frame_changes::FrameChanges>,
) -> io::Result<()> {
    builder.changed.clear();
    let deferred = changes.is_some();
    let contiguous = builder.previous_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
    if deferred && !contiguous {
        builder.seal(scan);
    }
    let (Ok(countries), Ok(_)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) else {
        if deferred {
            builder.seal(scan);
        }
        if let Some(hour) = builder.previous_hour {
            match &builder.lineage {
                Some(index) => builder
                    .changed
                    .extend(index.present.iter().cloned().map(|tag| (tag, [true; 8]))),
                // A restored builder has no join; publication sealed the present
                // spans of that hour's holders at it.
                None => builder.changed.extend(
                    scan.series
                        .iter()
                        .filter(|(_, track)| {
                            track
                                .present
                                .last()
                                .is_some_and(|span| span.last_hour == hour)
                        })
                        .map(|(tag, _)| (Arc::clone(tag), [true; 8])),
                ),
            }
        }
        scan.incomplete += 1;
        builder.previous_hour = None;
        return Ok(());
    };
    let same = builder
        .lineage
        .as_ref()
        .is_some_and(|index| index.reads.matches_changes(frame, countries, changes));
    let (states, provinces) = fresh_tables(scan.format, frame);
    #[cfg(any(test, feature = "test-support"))]
    {
        let compare = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        builder.work.row_visits += usize::from(compare(TableId::Countries)) * countries.len()
            + usize::from(compare(TableId::States)) * states.map_or(0, <[StateRow]>::len)
            + usize::from(compare(TableId::Provinces)) * provinces.map_or(0, <[ProvinceRow]>::len);
    }
    let reads = builder.reads.update_changes(states, provinces, changes);
    let same_reads = reads.into_iter().all(|same| same);
    if deferred && same && same_reads && contiguous && builder.totals.is_some() {
        builder.previous_hour = Some(frame.hour);
        return Ok(());
    }
    if deferred {
        builder.seal_world(scan);
    }
    let replaced = if same {
        None
    } else {
        let reads = builder.lineage.as_ref().filter(|index| {
            contiguous
                && changes
                    .is_some_and(|changes| !changes.rows_changed[TableId::CountryLinks.index()])
                && index
                    .reads
                    .links
                    .is(&frame.tables[TableId::CountryLinks.index()])
        });
        let reads = match reads {
            Some(index) => {
                CountryReads::from_slots(frame, countries, Arc::clone(index.reads.slots()))
            }
            None => CountryReads::new(frame, countries),
        };
        Some(builder.lineage.replace(lineage(scan, reads)))
    };
    let index = builder.lineage.as_ref().expect("complete lineage");
    if !same || !same_reads || builder.totals.is_none() {
        #[cfg(any(test, feature = "test-support"))]
        {
            builder.aggregations += 1;
        }
        builder.totals = Some(sample_rows(
            scan.format,
            frame,
            areas,
            &index.raw_slots,
            index.slot_count,
            &mut builder.current,
            (same && builder.previous.len() == index.slot_count)
                .then_some((builder.totals, builder.previous.as_slice()))
                .and_then(|(totals, values)| totals.map(|totals| (totals, values))),
            reads.map(|read| !same || builder.totals.is_none() || !read),
        ));
    } else {
        std::mem::swap(&mut builder.current, &mut builder.previous);
    }
    let totals = builder.totals.expect("current totals");
    scan.overflowed = scan
        .overflowed
        .saturating_add(totals.overflowed.iter().sum());
    let world = if index.incomplete {
        [None; 8]
    } else {
        totals.world
    };
    if let Some(previous) = scan.world_totals.last_mut()
        && contiguous
        && previous.values == world
    {
        previous.last_hour = frame.hour;
    } else {
        scan.world_totals.push(TerritoryRun {
            first_hour: frame.hour,
            last_hour: frame.hour,
            starts_segment: !contiguous,
            values: world,
        });
    }
    record_hour(&mut scan.rosters, frame.hour);
    let before = replaced.as_ref().and_then(Option::as_ref);
    let previous_present: BTreeSet<_> = before
        .into_iter()
        .flat_map(|old| &old.present)
        .cloned()
        .collect();
    let present: BTreeSet<_> = index.present.iter().cloned().collect();
    if !same {
        for tag in previous_present.difference(&present) {
            let track = scan.series.get_mut(tag).expect("previous tag");
            if let Some(hour) = builder.previous_hour {
                seal_track(track, hour);
                if let Some(span) = track.present.last_mut() {
                    span.last_hour = hour;
                }
            }
            builder.changed.insert(Arc::clone(tag), [true; 8]);
        }
        let resolved: BTreeSet<_> = index.entries.iter().map(|(tag, ..)| tag.as_ref()).collect();
        for (tag, ..) in before.into_iter().flat_map(|old| &old.entries) {
            if present.contains(tag) && !resolved.contains(tag.as_ref()) {
                let track = scan.series.get_mut(tag).expect("previous holder");
                if let Some(hour) = builder.previous_hour {
                    seal_track(track, hour);
                    if let Some(span) = track.present.last_mut() {
                        span.last_hour = hour;
                    }
                }
                record_hour(&mut track.present, frame.hour);
                builder.changed.insert(Arc::clone(tag), [true; 8]);
            }
        }
        // A restored builder has no previous join, but publication sealed it
        // into the tracks at `previous_hour`: that hour's holders end a present
        // span there and its resolved holders a native identity. Close exactly
        // what that join would close; other keys keep their published state.
        if before.is_none()
            && let Some(hour) = builder.previous_hour
        {
            for (tag, track) in &scan.series {
                let was_present = track
                    .present
                    .last()
                    .is_some_and(|span| span.last_hour == hour);
                let was_resolved = track.last.as_ref().is_some_and(|(_, seen)| *seen == hour);
                if (was_present && !present.contains(tag))
                    || (was_resolved && present.contains(tag) && !resolved.contains(tag.as_ref()))
                {
                    builder.changed.insert(Arc::clone(tag), [true; 8]);
                }
            }
        }
    }
    for tag in &index.present {
        if !deferred || !contiguous || !same && !previous_present.contains(tag) {
            record_hour(
                &mut scan.series.get_mut(tag).expect("interned tag").present,
                frame.hour,
            );
            builder.changed.entry(Arc::clone(tag)).or_insert([false; 8]);
        }
    }
    let previous: HashMap<_, _> = before
        .into_iter()
        .flat_map(|old| &old.entries)
        .map(|(tag, identity, slot)| {
            (
                tag.as_ref(),
                (*identity, builder.previous.get(*slot).copied()),
            )
        })
        .collect();
    let mut incomplete = index.incomplete;
    for (tag, identity, slot) in &index.entries {
        let values = builder.current[*slot];
        incomplete |= values
            .iter()
            .enumerate()
            .any(|(column, value)| value.is_none() && (column % 4 != 3 || areas.is_some()));
        let unchanged = contiguous
            && if same {
                builder.previous.get(*slot) == Some(&values)
            } else {
                previous.get(tag.as_ref()) == Some(&(*identity, Some(values)))
            };
        if deferred && unchanged && !builder.changed.contains_key(tag) {
            continue;
        }
        let track = scan.series.get_mut(tag).expect("interned tag");
        #[cfg(any(test, feature = "test-support"))]
        {
            builder.work.history_visits += 1;
        }
        if deferred {
            if contiguous
                && let Some(span) = track.present.last_mut()
                && span.first_hour < frame.hour
            {
                span.last_hour = frame.hour - 1;
            }
            record_hour(&mut track.present, frame.hour);
        }
        if deferred && let Some(hour) = builder.previous_hour {
            seal_track(track, hour);
        }
        let continues = track.last.is_some_and(|(seen, hour)| {
            seen == *identity && hour.checked_add(1) == Some(frame.hour)
        });
        if continues {
            track
                .segments
                .last_mut()
                .expect("previous identity")
                .last_hour = frame.hour;
        } else {
            track.segments.push(HourSpan {
                first_hour: frame.hour,
                last_hour: frame.hour,
            });
        }
        track.last = Some((*identity, frame.hour));
        let old = if same {
            builder.previous.get(*slot).copied()
        } else {
            previous.get(tag.as_ref()).and_then(|(_, values)| *values)
        };
        let identity_changed = !contiguous
            || !same
                && previous
                    .get(tag.as_ref())
                    .is_none_or(|(seen, _)| seen != identity);
        builder.changed.insert(
            Arc::clone(tag),
            std::array::from_fn(|column| {
                identity_changed || old.and_then(|values| values[column]) != values[column]
            }),
        );
        if values.iter().all(Option::is_none) {
            continue;
        }
        match track.runs.last_mut() {
            Some(run)
                if continues
                    && run.last_hour.checked_add(1) == Some(frame.hour)
                    && run.values == values =>
            {
                run.last_hour = frame.hour
            }
            _ => {
                let starts_segment = !continues
                    || track
                        .runs
                        .last()
                        .is_none_or(|run| run.last_hour.checked_add(1) != Some(frame.hour));
                track.runs.push(TerritoryRun {
                    first_hour: frame.hour,
                    last_hour: frame.hour,
                    starts_segment,
                    values,
                });
            }
        }
    }
    builder.incomplete = incomplete;
    builder.previous_hour = Some(frame.hour);
    std::mem::swap(&mut builder.current, &mut builder.previous);
    scan.incomplete += usize::from(builder.incomplete || totals.missing_area);
    Ok(())
}

/// Each readable hour of a branch's territory tables, in order. Returns how many
/// blocks or hours could not be read; an unreadable hour ends its block.
#[cfg(any(test, feature = "test-support"))]
pub fn walk(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<usize> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::Provinces,
    ];
    let mut failed = 0;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    failed += 1;
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        failed += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        failed += 1;
                        break;
                    }
                    visit(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(failed)
}

#[cfg(any(test, feature = "test-support"))]
pub fn scan(
    replay: &Replay,
    branch: u64,
    areas: Option<&ProvinceAreas>,
    cancelled: &AtomicBool,
) -> io::Result<Scan> {
    scan_with(replay, branch, areas, cancelled, |_| Ok(()))
}

/// The scan, also handing each hour to another consumer of the same tables.
#[cfg(any(test, feature = "test-support"))]
pub fn scan_with(
    replay: &Replay,
    branch: u64,
    areas: Option<&ProvinceAreas>,
    cancelled: &AtomicBool,
    mut also: impl FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<Scan> {
    let mut result = Scan {
        format: replay.format,
        ..Scan::default()
    };
    let mut builder = Builder::default();
    let failed = walk(replay, branch, cancelled, |frame| {
        also(frame)?;
        observe(&mut result, frame, areas, &mut builder, None)
    })?;
    result.incomplete += failed;
    Ok(result)
}

/// Hands each readable hour of a branch's territory tables to a geographic
/// consumer, building this branch's cached scan in the same pass if none is.
#[cfg(any(test, feature = "test-support"))]
pub fn walk_branch(
    source: &Source,
    branch: u64,
    areas: Option<&ProvinceAreas>,
    context: &LoadContext,
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<()> {
    let replay = source.replay();
    let key = scan_key(areas, context);
    let mut shared = false;
    cached_scan(&source.state::<Cache>(), branch, key, || {
        shared = true;
        scan_with(&replay, branch, areas, cancelled, &mut visit)
    })?;
    if !shared {
        walk(&replay, branch, cancelled, visit)?;
    }
    Ok(())
}

/// Merge dated absence with recorded measures without allocating zero histories.
fn for_each_dated_run(
    track: &Track,
    rosters: &[HourSpan],
    cancelled: &AtomicBool,
    mut visit: impl FnMut(&TerritoryRun) -> io::Result<()>,
) -> io::Result<()> {
    let mut recorded = track.runs.iter().peekable();
    super::faction_totals::for_each_absent(rosters, &track.present, cancelled, |first, last| {
        while recorded.peek().is_some_and(|run| run.first_hour < first) {
            check_cancel(cancelled)?;
            visit(recorded.next().expect("recorded run"))?;
        }
        visit(&TerritoryRun {
            first_hour: first,
            last_hour: last,
            starts_segment: true,
            values: [Some(0); 8],
        })
    })?;
    for run in recorded {
        check_cancel(cancelled)?;
        visit(run)?;
    }
    Ok(())
}

fn reply_budget_error() -> io::Error {
    io::Error::other("Territory reply exceeds its transport size budget; choose fewer countries")
}

fn charge_reply_bytes(value: &impl Serialize, bytes: &mut usize) -> io::Result<()> {
    let remaining = MAX_PRODUCT_REPLY_BYTES.saturating_sub(*bytes);
    *bytes += json_len(value, remaining).ok_or_else(reply_budget_error)?;
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
fn requested_series(
    scan: &Scan,
    params: &TerritoryParams,
    cancelled: &AtomicBool,
    histories: Option<&RecordedHistories>,
) -> io::Result<Vec<TerritorySeries>> {
    let membership = if let Some(top) = &params.top {
        // Borrow the runs: ranking events hold only country indices and values.
        let series: BTreeMap<_, _> = scan
            .series
            .iter()
            .map(|(tag, track)| (tag.to_string(), track.runs.iter().collect()))
            .collect();
        Some(crate::ranking::top_membership(
            &series,
            top.count.min(series.len() as u64) as usize,
            |run| (run.first_hour, run.last_hour),
            |run| run.values[top.measure.index(top.rule)],
            cancelled,
        )?)
    } else {
        None
    };
    selected_series(scan, params, cancelled, histories, membership)
}

fn selected_series(
    scan: &Scan,
    params: &TerritoryParams,
    cancelled: &AtomicBool,
    histories: Option<&RecordedHistories>,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<Vec<TerritorySeries>> {
    let aliases = histories.map(|history| {
        let mut aliases = super::country_history::selected_aliases(history, &params.tags);
        let mut seen: HashSet<_> = aliases.iter().cloned().collect();
        aliases.extend(
            params
                .tags
                .iter()
                .filter(|tag| seen.insert((*tag).clone()))
                .cloned(),
        );
        aliases
    });
    let tags: Vec<_> = membership.as_ref().map_or_else(
        || {
            let mut seen = HashSet::new();
            aliases
                .as_ref()
                .unwrap_or(&params.tags)
                .iter()
                .filter(|tag| seen.insert(*tag))
                .collect()
        },
        |membership| membership.keys().collect(),
    );
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SeriesMetadata<'a> {
        tag: &'a str,
        runs: &'a [TerritoryRun],
        segments: &'a [HourSpan],
        membership: Option<&'a [HourSpan]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        identities: Option<&'a [CountryIdentityRun]>,
    }
    let mut reply = Vec::with_capacity(tags.len());
    let mut bytes = 2;
    charge_reply_bytes(&scan.world_totals, &mut bytes)?;
    for tag in tags {
        check_cancel(cancelled)?;
        let empty = Track::default();
        let track = scan.series.get(tag.as_str()).unwrap_or(&empty);
        let country = histories.and_then(|history| history.countries.get(tag));
        let identities =
            histories.map(|_| country.map_or(&[][..], |country| country.identities.as_slice()));
        let segments = country.map_or(track.segments.as_slice(), |country| {
            country.segments.as_slice()
        });
        // Ranking uses existing countries only; fixed selections also need proven absence.
        let spans = membership.as_ref().map(|membership| &membership[tag]);
        bytes += usize::from(!reply.is_empty());
        charge_reply_bytes(
            &SeriesMetadata {
                tag,
                runs: &[],
                segments,
                membership: spans.map(Vec::as_slice),
                identities,
            },
            &mut bytes,
        )?;
        let mut runs = Vec::new();
        let mut append = |run: &TerritoryRun, first_hour: i32, last_hour: i32| -> io::Result<()> {
            let mut piece = run.clone();
            piece.first_hour = first_hour;
            piece.last_hour = last_hour;
            piece.starts_segment &= first_hour == run.first_hour;
            bytes += usize::from(!runs.is_empty());
            // Count the actual JSON before copying the next piece into the reply.
            charge_reply_bytes(&piece, &mut bytes)?;
            runs.push(piece);
            Ok(())
        };
        if spans.is_some() {
            crate::ranking::for_each_optional_piece(
                &track.runs,
                spans.map(Vec::as_slice),
                |run| (run.first_hour, run.last_hour),
                cancelled,
                &mut append,
            )?;
        } else {
            for_each_dated_run(track, &scan.rosters, cancelled, |run| {
                append(run, run.first_hour, run.last_hour)
            })?;
        }
        reply.push(TerritorySeries {
            tag: tag.clone(),
            runs,
            segments: segments.to_vec(),
            membership: spans.cloned(),
            identities: identities.map(|identities| identities.to_vec()),
        });
    }
    Ok(reply)
}

#[cfg(any(test, feature = "test-support"))]
pub type CachedScan = (u64, Option<ContentKey>, Arc<Scan>);
type CachedAreas = (
    Arc<installed_content::ResolvedContent>,
    Result<Arc<ProvinceAreas>, String>,
);
#[derive(Default)]
pub struct Cache {
    #[cfg(any(test, feature = "test-support"))]
    pub scans: VecDeque<CachedScan>,
    areas: Option<CachedAreas>,
}

fn cached_areas(
    state: &Mutex<Cache>,
    resolved: Arc<installed_content::ResolvedContent>,
    cancelled: &AtomicBool,
    read: impl FnOnce(&installed_content::ResolvedLayers) -> io::Result<ProvinceAreas>,
) -> io::Result<Arc<ProvinceAreas>> {
    check_cancel(cancelled)?;
    let find = |cache: &Cache| {
        cache
            .areas
            .as_ref()
            .filter(|(old, _)| Arc::ptr_eq(old, &resolved))
            .map(|(_, result)| result.clone())
    };
    if let Some(result) = find(&state.lock().unwrap_or_else(|error| error.into_inner())) {
        return result.map_err(io::Error::other);
    }
    // Raster work, including the settings probe, never holds the cache lock.
    let built = read(&resolved.layers);
    check_cancel(cancelled)?;
    if built
        .as_ref()
        .is_err_and(|error| error.kind() == io::ErrorKind::Interrupted)
    {
        return built.map(Arc::new);
    }
    let result = built.map(Arc::new).map_err(|error| error.to_string());
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(first) = find(&cache) {
        return first.map_err(io::Error::other);
    }
    cache.areas = Some((resolved, result.clone()));
    result.map_err(io::Error::other)
}

#[cfg(any(test, feature = "test-support"))]
pub fn cached_scan(
    state: &Mutex<Cache>,
    branch: u64,
    key: Option<ContentKey>,
    build: impl FnOnce() -> io::Result<Scan>,
) -> io::Result<Arc<Scan>> {
    let find = |cache: &Cache| {
        cache
            .scans
            .iter()
            .find(|(old_branch, old_key, _)| *old_branch == branch && *old_key == key)
            .map(|(_, _, scan)| Arc::clone(scan))
    };
    if let Some(scan) = find(&state.lock().unwrap_or_else(|error| error.into_inner())) {
        return Ok(scan);
    }
    // Concurrent misses may scan twice; the first completed insert is shared.
    // Neither a scan nor reply construction holds the cache lock.
    let scanned = Arc::new(build()?);
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(first) = find(&cache) {
        return Ok(first);
    }
    cache.scans.push_back((branch, key, Arc::clone(&scanned)));
    while cache.scans.len() > 2 {
        cache.scans.pop_front();
    }
    Ok(scanned)
}

pub struct AreaData {
    pub areas: Option<Arc<ProvinceAreas>>,
    reason: Option<String>,
    warnings: Vec<String>,
}

fn load_areas(
    source: &Source,
    state: &Mutex<Cache>,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<AreaData> {
    check_cancel(cancelled)?;
    let mut warnings = Vec::new();
    let areas = (|| -> io::Result<Arc<ProvinceAreas>> {
        let options = context.content()?;
        content::check_replay(&source.replay(), options.allow_unverified, &mut warnings)?;
        let resolved = source.content(&options.options)?;
        cached_areas(state, resolved, cancelled, |layers| {
            ProvinceAreas::read_cancellable(layers, cancelled)
        })
    })();
    let (areas, reason) = match areas {
        Ok(areas) => (Some(areas), None),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => (None, Some(format!("Map area unavailable: {error}"))),
    };
    check_cancel(cancelled)?;
    Ok(AreaData {
        areas,
        reason,
        warnings,
    })
}

#[cfg(any(test, feature = "test-support"))]
fn load_scan(
    source: &Source,
    state: &Mutex<Cache>,
    branch: u64,
    areas: Option<&ProvinceAreas>,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Scan>> {
    cached_scan(state, branch, scan_key(areas, context), || {
        scan(&source.replay(), branch, areas, cancelled)
    })
}

#[cfg(any(test, feature = "test-support"))]
pub fn scan_key(areas: Option<&ProvinceAreas>, context: &LoadContext) -> Option<ContentKey> {
    // Refused content is never confused with a previous successful map read.
    areas.and(content::content_key(context.content.as_ref()))
}

fn scan_warning(scanned: &Scan, warnings: &mut Vec<String>) {
    if scanned.ambiguous_slots > 0 {
        warnings.push(format!("{} canonical-slot conflicts (one slot named by more than one tag); affected totals are not recorded.", scanned.ambiguous_slots));
    }
    if scanned.overflowed > 0 {
        warnings.push(format!(
            "{} territory totals exceeded the exact integer range; affected totals are gaps.",
            scanned.overflowed
        ));
    }
    if scanned.incomplete > 0 {
        warnings.push(format!(
            "{} territory reads were incomplete or unavailable; affected totals are gaps.",
            scanned.incomplete
        ));
    }
}

/// Geographic consumers share the raster cache and its named refusals.
pub fn area_data(
    source: &Source,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<AreaData> {
    load_areas(source, &source.state::<Cache>(), context, cancelled)
}

impl Product for Territory {
    const KEY: &'static str = "territory";
    const CONTENT_DEPENDENT: bool = true;
    type Params = TerritoryParams;
    type Reply = TerritoryReply;
    fn types() -> Vec<String> {
        vec![
            TerritoryMeasure::decl(),
            TerritoryRule::decl(),
            TerritoryTop::decl(),
            TerritoryParams::decl(),
            TerritoryRun::decl(),
            TerritorySeries::decl(),
            TerritoryReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params
            .top
            .as_ref()
            .is_some_and(|top| !params.tags.is_empty() || top.count == 0)
        {
            return Err("Invalid top countries");
        }
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let state = source.state::<Cache>();
        let AreaData {
            areas,
            reason: area_reason,
            mut warnings,
        } = load_areas(source, &state, context, cancelled)?;
        let branch = params.branch_id.parse().expect("validated branch");
        let scanned = load_scan(source, &state, branch, areas.as_deref(), context, cancelled)?;
        scan_warning(&scanned, &mut warnings);
        let histories = if params.top.is_none() && !params.tags.is_empty() {
            Some(super::country_history::branch_histories(
                source,
                branch,
                false,
                &[],
                cancelled,
            )?)
        } else {
            None
        };
        let series = requested_series(&scanned, params, cancelled, histories.as_deref())?;
        let reply = TerritoryReply {
            world_totals: scanned.world_totals.clone(),
            branch_id: params.branch_id.clone(),
            series,
            area_unit: areas.map(|_| ProvinceAreas::UNIT.to_owned()),
            area_reason,
            warnings,
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn faction_tracks(
    source: &Source,
    branch: u64,
    tags: &[String],
    measure: TerritoryMeasure,
    rule: TerritoryRule,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<super::faction_totals::CountryMeasures> {
    let state = source.state::<Cache>();
    let AreaData {
        areas,
        reason: area_reason,
        mut warnings,
    } = load_areas(source, &state, context, cancelled)?;
    let scanned = load_scan(source, &state, branch, areas.as_deref(), context, cancelled)?;
    scan_warning(&scanned, &mut warnings);
    let column = measure.index(rule);
    let mut tracks = Vec::with_capacity(tags.len());
    for tag in tags {
        check_cancel(cancelled)?;
        let empty = Track::default();
        let track = scanned.series.get(tag.as_str()).unwrap_or(&empty);
        let mut runs: Vec<super::faction_totals::MeasureRun> = Vec::new();
        for_each_dated_run(track, &scanned.rosters, cancelled, |run| {
            let value = run.values[column];
            if let Some(previous) = runs.last_mut()
                && !run.starts_segment
                && previous.last_hour.checked_add(1) == Some(run.first_hour)
                && previous.value == value
            {
                previous.last_hour = run.last_hour;
            } else {
                runs.push(super::faction_totals::MeasureRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    starts_segment: run.starts_segment,
                    value,
                });
            }
            Ok(())
        })?;
        tracks.push(super::faction_totals::CountryTrack {
            tag: tag.clone(),
            runs,
        });
    }
    Ok(super::faction_totals::CountryMeasures {
        tracks,
        unit: match measure {
            TerritoryMeasure::States => Some("states".into()),
            TerritoryMeasure::Provinces => Some("provinces".into()),
            TerritoryMeasure::VictoryPoints => Some("victory points".into()),
            TerritoryMeasure::Area => areas.map(|_| ProvinceAreas::UNIT.to_owned()),
        },
        unavailable_reason: matches!(measure, TerritoryMeasure::Area)
            .then_some(area_reason)
            .flatten(),
        warnings,
    })
}

#[cfg(test)]
mod tests;

pub mod prepared;
