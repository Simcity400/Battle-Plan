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
    Source,
    content::{self, ContentKey},
    metadata::{
        TaggedCountry, check_cancel, link_slots, slot_read, tagged_countries_indexed,
        validate_campaign,
    },
    protocol::MAX_EXACT_JS_INTEGER,
    transport::json_len,
};
use installed_content::province_area::ProvinceAreas;
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, ViewIdentity},
    rows::{CountryRow, ProvinceRow, StateRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
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
    fn index(self, rule: TerritoryRule) -> usize {
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

#[derive(Default)]
struct Track {
    runs: Vec<TerritoryRun>,
    segments: Vec<HourSpan>,
    present: Vec<HourSpan>,
    last: Option<((u32, u32), i32)>,
}
#[derive(Default)]
pub struct Scan {
    /// The recording's format, which dates its daily tables
    /// ([`crate::metadata::slot_read`]).
    pub(crate) format: replay::format::FormatVersion,
    world_totals: Vec<TerritoryRun>,
    overflowed: usize,
    ambiguous_slots: usize,
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

#[derive(Default)]
struct Builder {
    lineage: Option<Lineage>,
    previous: Vec<[Option<i64>; 8]>,
    current: Vec<[Option<i64>; 8]>,
    previous_hour: Option<i32>,
    /// The last land reads and, under the current lineage, their totals.
    reads: LandReads,
    totals: Option<Totals>,
    #[cfg(test)]
    aggregations: usize,
    incomplete: bool,
}

/// What a holder lineage reads of the country tables: each row's id, canonical
/// slot and tag, in order, and the links by identity, which holds none of their
/// rows. Countries change hourly in their balances alone.
pub(crate) struct CountryReads {
    links: ViewIdentity,
    countries: Vec<(u32, u32, Vec<u8>)>,
}

impl CountryReads {
    pub(crate) fn new(frame: &DecodedFrame, countries: &[CountryRow]) -> Self {
        Self {
            links: frame.tables[TableId::CountryLinks.index()].identity(),
            countries: countries
                .iter()
                .map(|country| (country.id, country.canonical_slot, country.tag.clone()))
                .collect(),
        }
    }

    pub(crate) fn matches(&self, frame: &DecodedFrame, countries: &[CountryRow]) -> bool {
        self.links.is(&frame.tables[TableId::CountryLinks.index()])
            && self.countries.len() == countries.len()
            && self
                .countries
                .iter()
                .zip(countries)
                .all(|((id, slot, tag), country)| {
                    *id == country.id && *slot == country.canonical_slot && *tag == country.tag
                })
    }
}

/// What territory measures read of a fresh States and Provinces read: each state's
/// id and holders, and each province's id, holders and victory points. Their
/// other fields change hourly.
#[derive(Default)]
pub(crate) struct LandReads {
    states: Option<Vec<[u32; 3]>>,
    provinces: Option<Vec<([u32; 3], i32)>>,
}

impl LandReads {
    /// Keeps this hour's reads, answering whether they equal the last ones.
    pub(crate) fn update(
        &mut self,
        states: Option<&[StateRow]>,
        provinces: Option<&[ProvinceRow]>,
    ) -> bool {
        let states = keep(&mut self.states, states, |state| {
            [state.id, state.controller, state.owner]
        });
        let provinces = keep(&mut self.provinces, provinces, |province| {
            (
                [province.id, province.controller, province.owner],
                province.victory_points,
            )
        });
        states && provinces
    }
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
pub(crate) fn fresh_tables(
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
    overflowed: usize,
}

/// The same exact province weights serve totals and geographic breakdowns.
pub(crate) fn province_values(
    province: &replay::rows::ProvinceRow,
    areas: Option<&ProvinceAreas>,
) -> [Option<i64>; 3] {
    [
        Some(1),
        Some(i64::from(province.victory_points)),
        areas.and_then(|areas| areas.land.get(province.id as usize).copied()),
    ]
}

fn totals(
    format: replay::format::FormatVersion,
    frame: &DecodedFrame,
    areas: Option<&ProvinceAreas>,
    raw_slots: &HashMap<u32, usize>,
    slot_count: usize,
    result: &mut Vec<[Option<i64>; 8]>,
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
    result.fill([Some(0); 8]);
    let mut world = [Some(0i64); 8];
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
    for state in states.into_iter().flatten() {
        add(state.controller, 0, &[Some(1)]);
        add(state.owner, 4, &[Some(1)]);
    }
    let mut missing_area = false;
    for province in provinces.into_iter().flatten() {
        let values = province_values(province, areas);
        missing_area |= areas.is_some() && values[2].is_none();
        for (raw, offset) in [(province.controller, 0), (province.owner, 4)] {
            add(raw, offset + 1, &values);
        }
    }
    let overflowed = sanitise_totals(available, result, &mut world, missing_area);
    Totals {
        missing_area,
        world,
        overflowed,
    }
}

/// A world-share ring also needs every candidate country to be rankable. A
/// safe signed world sum cannot conceal an overflowing country contribution.
fn sanitise_totals(
    available: [bool; 8],
    countries: &mut [[Option<i64>; 8]],
    world: &mut [Option<i64>; 8],
    missing_area: bool,
) -> usize {
    let mut overflowed = 0usize;
    for (column, known) in available.into_iter().enumerate() {
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
                overflowed += 1;
                sums[column] = None;
                country_overflow = true;
            }
        }
        if unsafe_value(world[column]) {
            overflowed += 1;
            world[column] = None;
        }
        if country_overflow {
            world[column] = None;
        }
    }
    overflowed
}

fn observe(
    scan: &mut Scan,
    frame: &DecodedFrame,
    areas: Option<&ProvinceAreas>,
    builder: &mut Builder,
) -> io::Result<()> {
    let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) else {
        scan.incomplete += 1;
        builder.previous_hour = None;
        return Ok(());
    };
    let same = builder
        .lineage
        .as_ref()
        .is_some_and(|index| index.reads.matches(frame, countries));
    if !same {
        let slots = link_slots(links);
        let mut entries = Vec::new();
        let mut present = Vec::new();
        let mut incomplete = false;
        let mut dense_slots = HashMap::new();
        for (tag, lookup) in tagged_countries_indexed(countries, &slots) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                incomplete = true;
                continue;
            };
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
            if let TaggedCountry::Found(country, slot) = lookup {
                let next = dense_slots.len();
                let index = *dense_slots.entry(slot).or_insert(next);
                entries.push((tag, (slot, country.id), index));
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
        let raw_slots = slots
            .into_iter()
            .filter(|(_, slot)| !ambiguous.contains(slot))
            .filter_map(|(raw, slot)| dense_slots.get(&slot).map(|&index| (raw, index)))
            .collect();
        let slot_count = dense_slots.len();
        builder.lineage = Some(Lineage {
            reads: CountryReads::new(frame, countries),
            raw_slots,
            entries,
            present,
            slot_count,
            incomplete,
        });
    }
    let index = builder.lineage.as_ref().expect("complete lineage");
    record_hour(&mut scan.rosters, frame.hour);
    for tag in &index.present {
        record_hour(
            &mut scan.series.get_mut(tag).expect("interned tag").present,
            frame.hour,
        );
    }
    let (states, provinces) = fresh_tables(scan.format, frame);
    let same_reads = builder.reads.update(states, provinces);
    let totals = if let Some(totals) = builder.totals.filter(|_| same && same_reads) {
        builder.current.clone_from(&builder.previous);
        totals
    } else {
        #[cfg(test)]
        {
            builder.aggregations += 1;
        }
        let totals = totals(
            scan.format,
            frame,
            areas,
            &index.raw_slots,
            index.slot_count,
            &mut builder.current,
        );
        builder.totals = Some(totals);
        totals
    };
    scan.overflowed = scan.overflowed.saturating_add(totals.overflowed);
    let world = if index.incomplete {
        [None; 8]
    } else {
        totals.world
    };
    let world_continues =
        builder.previous_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
    if let Some(previous) = scan.world_totals.last_mut()
        && world_continues
        && previous.values == world
    {
        previous.last_hour = frame.hour;
    } else {
        scan.world_totals.push(TerritoryRun {
            first_hour: frame.hour,
            last_hour: frame.hour,
            starts_segment: !world_continues,
            values: world,
        });
    }
    let unchanged = same
        && builder.previous_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
        && builder.current == builder.previous;
    if unchanged {
        // No tag lookups or allocations when both lineage and totals are unchanged.
        for track in scan.series.values_mut() {
            let Some((identity, hour)) = track.last else {
                continue;
            };
            if Some(hour) != builder.previous_hour {
                continue;
            }
            track.last = Some((identity, frame.hour));
            track
                .segments
                .last_mut()
                .expect("previous identity")
                .last_hour = frame.hour;
            if let Some(run) = track.runs.last_mut()
                && run.last_hour == hour
            {
                run.last_hour = frame.hour;
            }
        }
    } else {
        let mut incomplete = index.incomplete;
        for (tag, identity, slot) in &index.entries {
            let track = scan.series.get_mut(tag).expect("interned tag");
            let continues = track.last.is_some_and(|(previous, hour)| {
                previous == *identity && hour.checked_add(1) == Some(frame.hour)
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
            let values = builder.current[*slot];
            incomplete |= values
                .iter()
                .enumerate()
                .any(|(column, value)| value.is_none() && (column % 4 != 3 || areas.is_some()));
            if values.iter().all(Option::is_none) {
                continue;
            }
            match track.runs.last_mut() {
                Some(run)
                    if continues
                        && run.last_hour.checked_add(1) == Some(frame.hour)
                        && run.values == values =>
                {
                    run.last_hour = frame.hour;
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
    }
    builder.previous_hour = Some(frame.hour);
    std::mem::swap(&mut builder.current, &mut builder.previous);
    scan.incomplete += usize::from(builder.incomplete || totals.missing_area);
    Ok(())
}

/// Each readable hour of a branch's territory tables, in order. Returns how many
/// blocks or hours could not be read; an unreadable hour ends its block.
fn walk(
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

pub fn scan(
    replay: &Replay,
    branch: u64,
    areas: Option<&ProvinceAreas>,
    cancelled: &AtomicBool,
) -> io::Result<Scan> {
    scan_with(replay, branch, areas, cancelled, |_| Ok(()))
}

/// The scan, also handing each hour to another consumer of the same tables.
fn scan_with(
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
        observe(&mut result, frame, areas, &mut builder)
    })?;
    result.incomplete += failed;
    Ok(result)
}

/// Hands each readable hour of a branch's territory tables to a geographic
/// consumer, building this branch's cached scan in the same pass if none is.
pub(crate) fn walk_branch(
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

type CachedScan = (u64, Option<ContentKey>, Arc<Scan>);
type CachedAreas = (
    Arc<installed_content::ResolvedContent>,
    Result<Arc<ProvinceAreas>, String>,
);
#[derive(Default)]
struct Cache {
    scans: VecDeque<CachedScan>,
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

fn cached_scan(
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

pub(crate) struct AreaData {
    pub(crate) areas: Option<Arc<ProvinceAreas>>,
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

fn scan_key(areas: Option<&ProvinceAreas>, context: &LoadContext) -> Option<ContentKey> {
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
pub(crate) fn area_data(
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
