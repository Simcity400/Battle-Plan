//! Land army totals and the Army size race. Country attribution is shared with
//! Scoreboard through `visit_army_runs`; all views use the same per-source scan.
//! Missing reads and country identities end runs; they never produce zeros.
//!
//! SendExpeditionaryForceAction::Execute@0x1410F4750 keeps the source via
//! SetExpeditionarySender@0x140BF4F70, then AddArmy@0x1406C6480 sets the target's
//! roster/logical links through 0x140BF5480 / 0x140BF5290. Owner therefore means
//! positive signed expeditionary owner, otherwise roster owner; host is roster.
//! The hourly census uses that same owner rule, adding grouped and ungrouped.
//! Manpower uses CArmyManpower::GetTotal@0x140C5A040 /
//! CArmyManpowerValues::GetTotal@0x140C5A0F0: wrapping signed i32 contributor
//! sum per division, then an exact country sum. Contributor origins never
//! replace the division owner/host.
//! Combined strength is current HP, not a percentage or template maximum:
//! CArmy::set_hit_points@0x140C7FA80 writes +0x420, clamps it to 0..maximum,
//! and formats it through FormatFixed5Trimmed@0x1424B60D0 (fixed5).
//!
//! Division classification was deferred because the counter
//! representative (0x140B94FA0: signed wrapping priority * count, largest
//! positive score, first wins ties; getter 0x140B90370 with brigade (0,0)/null
//! fallback) differs from the AI representative (same builder, ai_priority *
//! count). Category getter 0x140B8F7F0 uses the AI representative, counter
//! fallback, a template override and fixed executable category predicates.
//! Authored group is a sub-unit's dynamic token (0x141008E80 case 0x3F,
//! +0x5A4, token reader 0x1424A6500); the designer reads it per regiment at
//! 0x141678490. The user chose battalions only: grouped template sub-units
//! (regiments and support companies) use their authored content group, never
//! either representative. No per-battalion HP is recorded, so HP stays unsplit.

use super::{
    LoadContext, Product,
    country_groups::HourSpan,
    country_history::{Identity, observe_identity},
    validate_tag,
};
use crate::{
    Source,
    metadata::{
        IdentityTrack, TaggedCountry, link_slot, slot_read, tagged_countries, validate_campaign,
    },
};
use replay::{
    container::Replay,
    frame::{FrameDecoder, TableUpdate},
    rows::{CountryLinkRow, DivisionRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct ArmySize;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArmyMeasure {
    Manpower,
    Divisions,
    Strength,
    Battalions,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArmyAttribution {
    Owner,
    Host,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmySizeParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<usize>,
    pub measure: ArmyMeasure,
    pub count_for: ArmyAttribution,
    pub by_type: bool,
    /// None is the capability/window manifest; histories use a replay-block index.
    #[serde(default)]
    pub window: Option<usize>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyType {
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArmyRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub total: Option<f64>,
    pub raw: String,
}

// Compact hourly totals: [first hour, geometry total, last hour or null, exact raw].
impl Serialize for ArmyRun {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        (
            self.first_hour,
            self.total,
            (self.last_hour != self.first_hour).then_some(self.last_hour),
            &self.raw,
        )
            .serialize(serializer)
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyMixRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub values: Vec<Option<f64>>,
    /// Decimal counts retain exact labels; values are chart geometry only.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub raw_values: Option<Vec<Option<String>>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmySeries {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub identities: Option<Vec<super::country_history::CountryIdentityRun>>,
    #[ts(type = "Array<ArmyRun>")]
    pub runs: Vec<ArmyRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mix: Option<Vec<ArmyMixRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmySizeReply {
    pub branch_id: String,
    pub windows: Vec<HourSpan>,
    pub window: Option<usize>,
    pub interval: Option<HourSpan>,
    pub available: bool,
    pub measures: Vec<ArmyMeasure>,
    pub types: Vec<ArmyType>,
    pub series: Vec<ArmySeries>,
    pub warnings: Vec<String>,
}

/// Value and supplying read for an hour. None from `army_value_at` is unknown or
/// absent, never zero. Strength is current hit points, in ordinary HP units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmyValue {
    pub value: f64,
    pub raw: i128,
    pub read_hour: i32,
}

fn channel(measure: ArmyMeasure, attribution: ArmyAttribution) -> usize {
    match (measure, attribution) {
        (ArmyMeasure::Divisions, ArmyAttribution::Owner) => 0,
        (ArmyMeasure::Strength, ArmyAttribution::Owner) => 1,
        (ArmyMeasure::Divisions, ArmyAttribution::Host) => 2,
        (ArmyMeasure::Strength, ArmyAttribution::Host) => 3,
        (ArmyMeasure::Battalions, ArmyAttribution::Owner) => 4,
        (ArmyMeasure::Battalions, ArmyAttribution::Host) => 5,
        (ArmyMeasure::Manpower, ArmyAttribution::Owner) => 6,
        (ArmyMeasure::Manpower, ArmyAttribution::Host) => 7,
    }
}

fn amount(raw: Option<i128>, measure: ArmyMeasure) -> Option<f64> {
    raw.map(|raw| {
        raw as f64
            / if measure == ArmyMeasure::Strength {
                100_000.0
            } else {
                1.0
            }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Totals {
    // Owner/host divisions, HP, battalions, then native division manpower totals.
    values: [Option<i128>; 8],
}

#[derive(Clone, Debug)]
struct RawRun {
    first: i32,
    last: i32,
    starts: bool,
    totals: Totals,
}

#[derive(Default)]
struct Track {
    runs: Vec<RawRun>,
    last: Option<(Identity, i32)>,
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    mix: Vec<RawMixRun>,
    mix_version: Option<(u64, u64, [bool; 2])>,
}

#[derive(Default)]
struct Scan {
    tracks: BTreeMap<String, Track>,
    divisions: bool,
    strength: bool,
    manpower: bool,
    inventory_fallback: bool,
    unreadable: usize,
    battalions: bool,
    composition_unreadable: usize,
    invalid_table_reads: BTreeMap<String, usize>,
}

/// Census owner rule: collector/src/division_census.rs:601, active_damage@140C7BFC0.
fn owner(row: &DivisionRow) -> u32 {
    if row.expeditionary_owner as i32 > 0 {
        row.expeditionary_owner
    } else {
        row.roster_owner
    }
}

fn add(target: &mut Option<i64>, value: i64) {
    *target = target.and_then(|current| current.checked_add(value));
}

/// Detailed tables use native sparse deltas; only country/census tables need
/// full decoder reconstruction. Sparse tables validate keys and native deltas
/// before updating their own baselines; failed reads stay gaps.
fn scan_tables(detailed: bool) -> Vec<TableId> {
    let mut tables = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::DivisionCensus,
    ];
    if detailed {
        tables.push(TableId::LexicalTokens);
    }
    tables
}

fn scan(replay: &Replay, branch: u64, detailed: bool, cancelled: &AtomicBool) -> io::Result<Scan> {
    let selected = scan_tables(detailed);
    let mut wire_selected = selected.clone();
    wire_selected.extend([
        TableId::Divisions,
        TableId::DivisionTemplateIcons,
        TableId::DivisionLandInputs,
    ]);
    let mut result = Scan::default();
    let mut state = ScanState::new(detailed);
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        crate::metadata::check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &wire_selected) else {
            continue;
        };
        for frame in frames {
            crate::metadata::check_cancel(cancelled)?;
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            state.apply_changes(&frame, decoded);
            state.compose(decoded);
            observe_totals(&mut result, decoded, &frame, replay, branch, &mut state);
        }
    }
    Ok(result)
}

#[derive(Default)]
struct ScanState {
    inventory: Option<Inventory>,
    identities: Vec<(String, Identity)>,
    countries: Option<replay::frame::ViewIdentity>,
    links: Option<replay::frame::ViewIdentity>,
    unreadable_tags: usize,
}

impl ScanState {
    fn new(detailed: bool) -> Self {
        Self {
            inventory: detailed.then(Inventory::default),
            ..Default::default()
        }
    }
    fn apply_changes(
        &mut self,
        frame: &replay::frame::Frame,
        decoded: &replay::frame::DecodedFrame,
    ) {
        if let Some(inventory) = &mut self.inventory {
            inventory.apply_changes(frame, decoded);
        }
    }
    fn compose(&mut self, decoded: &replay::frame::DecodedFrame) {
        if let Some(inventory) = &mut self.inventory {
            inventory.compose(decoded);
        }
    }
    fn identities(&mut self, decoded: &replay::frame::DecodedFrame) -> bool {
        let (Ok(countries), Ok(links)) = (
            decoded.complete::<tables::Countries>(),
            decoded.complete::<tables::CountryLinks>(),
        ) else {
            self.countries = None;
            self.links = None;
            return false;
        };
        let country_view = &decoded.tables[TableId::Countries.index()];
        let link_view = &decoded.tables[TableId::CountryLinks.index()];
        if self
            .countries
            .as_ref()
            .is_some_and(|old| old.is(country_view))
            && self.links.as_ref().is_some_and(|old| old.is(link_view))
        {
            return true;
        }
        let tagged = tagged_countries(countries, links);
        self.unreadable_tags = tagged
            .iter()
            .filter(|(tag, lookup)| {
                matches!(lookup, TaggedCountry::Found(_, _)) && std::str::from_utf8(tag).is_err()
            })
            .count();
        self.identities = tagged
            .into_iter()
            .filter_map(|(tag, lookup)| {
                let TaggedCountry::Found(country, slot) = lookup else {
                    return None;
                };
                std::str::from_utf8(tag)
                    .ok()
                    .map(|tag| (tag.to_owned(), (slot, country.id)))
            })
            .collect();
        self.countries = Some(country_view.identity());
        self.links = Some(link_view.identity());
        true
    }
}

fn observe_totals(
    result: &mut Scan,
    decoded: &replay::frame::DecodedFrame,
    frame: &replay::frame::Frame,
    replay: &Replay,
    branch: u64,
    state: &mut ScanState,
) {
    let hour = decoded.hour;
    if let Some(inventory) = &state.inventory {
        for (table, reason) in inventory.errors() {
            *result
                .invalid_table_reads
                .entry(format!("{table:?}: {reason}"))
                .or_default() += 1;
        }
    }
    let division_read = matches!(
        &frame.tables[TableId::Divisions.index()],
        TableUpdate::Snapshot(_) | TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. }
    ) && slot_read(&decoded.metrics, TableId::Divisions, hour);
    let template_read = matches!(
        &frame.tables[TableId::DivisionTemplateIcons.index()],
        TableUpdate::Snapshot(_) | TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. }
    ) && slot_read(&decoded.metrics, TableId::DivisionTemplateIcons, hour);
    if division_read {
        result.strength = true;
        result.divisions = true;
        result.manpower |= slot_read(&decoded.metrics, TableId::DivisionLandInputs, hour)
            && matches!(
                &frame.tables[TableId::DivisionLandInputs.index()],
                TableUpdate::Snapshot(_)
                    | TableUpdate::Delta { .. }
                    | TableUpdate::Unchanged { .. }
            );
        result.battalions |= template_read;
    }
    if replay
        .gaps
        .iter()
        .any(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
        || !state.identities(decoded)
    {
        return;
    }
    result.unreadable += state.unreadable_tags;
    let Ok(links) = decoded.complete::<tables::CountryLinks>() else {
        return;
    };
    let census = decoded
        .complete::<tables::DivisionCensus>()
        .ok()
        .filter(|_| slot_read(&decoded.metrics, TableId::DivisionCensus, hour));
    result.inventory_fallback |= division_read && census.is_none();
    let mut counts = HashMap::<u32, Option<i64>>::new();
    let mut invalid = false;
    if let Some(census) = census {
        result.divisions = true;
        for row in census {
            if let Some(slot) = link_slot(links, row.country) {
                add(
                    counts.entry(slot).or_insert(Some(0)),
                    i64::from(row.grouped) + i64::from(row.ungrouped),
                );
            } else if row.grouped != 0 || row.ungrouped != 0 {
                invalid = true;
                result.unreadable += 1;
            }
        }
    }
    let inventory = state
        .inventory
        .as_ref()
        .filter(|inventory| division_read && inventory.known());
    let composition = inventory.filter(|inventory| template_read && inventory.composition_known());
    if let Some(inventory) = inventory {
        result.unreadable += inventory.unreadable();
    }
    for (tag, identity) in &state.identities {
        let slot = identity.0;
        let mut total = inventory.map_or_else(Totals::default, |inventory| {
            inventory.totals(slot, composition.is_some())
        });
        if !slot_read(&decoded.metrics, TableId::DivisionLandInputs, hour) {
            total.values[6] = None;
            total.values[7] = None;
        }
        if census.is_some() {
            total.values[0] = (!invalid)
                .then(|| counts.get(&slot).copied().flatten().map(i128::from))
                .flatten();
        }
        // Identity coverage is independent of measure availability.
        if !result.tracks.contains_key(tag) {
            result.tracks.insert(tag.clone(), Track::default());
        }
        let Some(track) = result.tracks.get_mut(tag) else {
            continue;
        };
        observe_identity(
            &mut track.segments,
            &mut track.last_identity,
            *identity,
            hour,
        );
        if census.is_none() && inventory.is_none() {
            continue;
        }
        let continues = track
            .last
            .is_some_and(|(last, at)| last == *identity && at.checked_add(1) == Some(hour));
        match track.runs.last_mut() {
            Some(run) if continues && run.totals == total => run.last = hour,
            _ => track.runs.push(RawRun {
                first: hour,
                last: hour,
                starts: !continues,
                totals: total,
            }),
        }
        if let Some(inventory) = composition {
            let version = inventory.parts_version(slot);
            result.composition_unreadable += version.2.iter().filter(|known| !**known).count();
            if let Some(run) = track.mix.last_mut()
                && continues
                && track.mix_version == Some(version)
                && run.last.checked_add(1) == Some(hour)
            {
                run.last = hour;
            } else {
                let parts = inventory.parts_at(slot);
                match track.mix.last_mut() {
                    Some(run)
                        if continues
                            && run.last.checked_add(1) == Some(hour)
                            && run.parts == parts =>
                    {
                        run.last = hour
                    }
                    _ => track.mix.push(RawMixRun {
                        first: hour,
                        last: hour,
                        parts,
                    }),
                }
            }
            track.mix_version = Some(version);
        }
        track.last = Some((*identity, hour));
    }
}

#[derive(Default)]
struct Scans {
    entries: VecDeque<(u64, bool, Arc<Scan>)>,
    runs: usize,
}

fn scanned(
    source: &Source,
    branch: u64,
    detailed: bool,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Scan>> {
    crate::metadata::check_cancel(cancelled)?;
    let state = source.state::<Scans>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if !source
        .replay_ref()
        .branches
        .iter()
        .any(|known| known.id == branch)
    {
        return Err(crate::invalid("Unknown branch"));
    }
    if let Some((_, detail, scan)) = cache.entries.iter().find(|(id, _, _)| *id == branch)
        && (*detail || !detailed)
    {
        return Ok(Arc::clone(scan));
    }
    cache.runs += 1;
    let mut result = scan(source.replay_ref(), branch, detailed, cancelled)?;
    let fallback = !detailed && result.inventory_fallback;
    if fallback {
        cache.runs += 1;
        result = scan(source.replay_ref(), branch, true, cancelled)?;
    }
    let scan = Arc::new(result);
    cache.entries.retain(|(id, _, _)| *id != branch);
    cache
        .entries
        .push_back((branch, detailed || fallback, Arc::clone(&scan)));
    while cache.entries.len() > 2 {
        cache.entries.pop_front();
    }
    Ok(scan)
}

/// Scoreboard's army column: exact per-country value at one recorded hour,
/// using the same cached scan and owner/host rules as the widget. Unavailable
/// tables, missing country rows, invalid HP and gaps return None.
pub fn army_value_at(
    source: &Source,
    branch: u64,
    tag: &str,
    hour: i32,
    measure: ArmyMeasure,
    count_for: ArmyAttribution,
    cancelled: &AtomicBool,
) -> io::Result<Option<ArmyValue>> {
    let scan = scanned(
        source,
        branch,
        measure != ArmyMeasure::Divisions || count_for == ArmyAttribution::Host,
        cancelled,
    )?;
    let value = scan.tracks.get(tag).and_then(|track| {
        let index = track
            .runs
            .partition_point(|run| run.first <= hour)
            .checked_sub(1)?;
        let run = &track.runs[index];
        (hour <= run.last)
            .then(|| run.totals.values[channel(measure, count_for)])
            .flatten()
    });
    Ok(value.map(|raw| ArmyValue {
        value: amount(Some(raw), measure).expect("recorded total"),
        raw,
        read_hour: hour,
    }))
}

/// Exact cached runs for the requested tags; an empty selection visits all tags.
/// Inclusive hours are hourly supplying reads. None stays unknown, and starts
/// preserve native identity/gap boundaries without expanding campaign hours.
pub fn visit_army_runs(
    source: &Source,
    branch: u64,
    tags: &[String],
    measure: ArmyMeasure,
    attribution: ArmyAttribution,
    cancelled: &AtomicBool,
    mut visitor: impl FnMut(&str, i32, i32, bool, Option<i128>) -> io::Result<()>,
) -> io::Result<()> {
    let scan = scanned(
        source,
        branch,
        measure != ArmyMeasure::Divisions || attribution == ArmyAttribution::Host,
        cancelled,
    )?;
    let index = channel(measure, attribution);
    let mut visit = |tag: &str, track: &Track| -> io::Result<()> {
        let mut pending: Option<(i32, i32, bool, Option<i128>)> = None;
        for run in &track.runs {
            crate::metadata::check_cancel(cancelled)?;
            let raw = run.totals.values[index];
            if let Some(previous) = pending.as_mut()
                && !run.starts
                && previous.1.checked_add(1) == Some(run.first)
                && previous.3 == raw
            {
                previous.1 = run.last;
            } else {
                if let Some((first, last, starts, raw)) = pending.take() {
                    visitor(tag, first, last, starts, raw)?;
                }
                pending = Some((run.first, run.last, run.starts, raw));
            }
        }
        if let Some((first, last, starts, raw)) = pending {
            visitor(tag, first, last, starts, raw)?;
        }
        Ok(())
    };
    if tags.is_empty() {
        for (tag, track) in &scan.tracks {
            visit(tag, track)?;
        }
    } else {
        for tag in tags.iter().collect::<std::collections::BTreeSet<_>>() {
            if let Some(track) = scan.tracks.get(tag) {
                visit(tag, track)?;
            }
        }
    }
    Ok(())
}

fn for_each_reply_run(
    track: &Track,
    params: &ArmySizeParams,
    range: &HourSpan,
    spans: Option<&[HourSpan]>,
    cancelled: &AtomicBool,
    mut output: impl FnMut(ArmyRun) -> io::Result<()>,
) -> io::Result<()> {
    let index = channel(params.measure, params.count_for);
    let mut pending: Option<ArmyRun> = None;
    let mut visit = |raw: &RawRun, first, last| -> io::Result<()> {
        let Some(exact) = raw.totals.values[index] else {
            return Ok(());
        };
        let starts_segment = (raw.starts && first == raw.first)
            || pending
                .as_ref()
                .is_none_or(|run| run.last_hour.checked_add(1) != Some(first));
        if let Some(previous) = pending.as_mut()
            && !starts_segment
            && previous.raw == exact.to_string()
        {
            previous.last_hour = last;
        } else {
            if let Some(previous) = pending.take() {
                output(previous)?;
            }
            pending = Some(ArmyRun {
                first_hour: first,
                last_hour: last,
                total: amount(Some(exact), params.measure),
                raw: exact.to_string(),
            });
        }
        Ok(())
    };
    let runs = window_runs(track, range);
    let whole = [range.clone()];
    crate::ranking::for_each_piece(
        runs,
        spans.unwrap_or(&whole),
        |run| (run.first, run.last),
        cancelled,
        &mut visit,
    )?;
    if let Some(last) = pending {
        output(last)?;
    }
    Ok(())
}

fn reply_bound() -> io::Error {
    io::Error::other("Army size reply exceeds the transport reply budget; choose fewer countries")
}

fn window_runs<'a>(track: &'a Track, range: &HourSpan) -> &'a [RawRun] {
    let first = track
        .runs
        .partition_point(|run| run.last < range.first_hour);
    let last = track
        .runs
        .partition_point(|run| run.first <= range.last_hour);
    &track.runs[first..last]
}

fn window_mix<'a>(track: &'a Track, range: &HourSpan) -> &'a [RawMixRun] {
    let first = track.mix.partition_point(|run| run.last < range.first_hour);
    let last = track
        .mix
        .partition_point(|run| run.first <= range.last_hour);
    &track.mix[first..last]
}

fn window_segments(track: &Track, range: &HourSpan) -> Vec<HourSpan> {
    // Keep the original identity start for country looks; omit other windows.
    let first = track
        .segments
        .partition_point(|span| span.last_hour < range.first_hour);
    let last = track
        .segments
        .partition_point(|span| span.first_hour <= range.last_hour);
    track.segments[first..last]
        .iter()
        .map(|span| HourSpan {
            first_hour: span.first_hour,
            last_hour: span.last_hour.min(range.last_hour),
        })
        .collect()
}

fn window_identities(
    country: &super::country_history::RecordedCountry,
    range: &HourSpan,
) -> Vec<super::country_history::CountryIdentityRun> {
    let first = country
        .identities
        .partition_point(|identity| identity.last_hour < range.first_hour);
    let last = country
        .identities
        .partition_point(|identity| identity.first_hour <= range.last_hour);
    country.identities[first..last]
        .iter()
        .map(|identity| super::country_history::CountryIdentityRun {
            first_hour: identity.first_hour.max(range.first_hour),
            last_hour: identity.last_hour.min(range.last_hour),
            slot: identity.slot,
            country_id: identity.country_id,
        })
        .collect()
}

fn reply_series(
    scan: &Scan,
    params: &ArmySizeParams,
    range: &HourSpan,
    selected: Option<&BTreeMap<String, Vec<super::country_history::CountryIdentityRun>>>,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<ArmySeries>, usize)> {
    let transport_budget = super::MAX_PRODUCT_REPLY_BYTES;
    let index = channel(params.measure, params.count_for);
    let membership = if let Some(top) = params.top {
        let runs: BTreeMap<_, _> = scan
            .tracks
            .iter()
            .map(|(tag, track)| {
                (
                    tag.clone(),
                    window_runs(track, range)
                        .iter()
                        .map(|run| RawRun {
                            first: run.first.max(range.first_hour),
                            last: run.last.min(range.last_hour),
                            starts: run.starts,
                            totals: run.totals,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        Some(crate::ranking::top_membership(
            &runs,
            top,
            |run| (run.first, run.last),
            |run| run.totals.values[index],
            cancelled,
        )?)
    } else {
        None
    };
    let mut seen = std::collections::BTreeSet::new();
    let wanted: Vec<_> = membership.as_ref().map_or_else(
        || {
            params
                .tags
                .iter()
                .filter(|tag| seen.insert(*tag))
                .map(|tag| (tag.clone(), None))
                .collect()
        },
        |members| {
            members
                .iter()
                .map(|(tag, spans)| (tag.clone(), Some(spans.clone())))
                .collect()
        },
    );
    let wanted: Vec<_> = wanted
        .into_iter()
        .filter_map(|(tag, membership)| {
            let track = scan.tracks.get(&tag)?;
            let identities = selected.map(|countries| countries[&tag].clone());
            let spans = identities.as_ref().map(|identities| {
                identities
                    .iter()
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect::<Vec<_>>()
            });
            let mut segments = window_segments(track, range);
            if let Some(spans) = &spans {
                segments.retain(|segment| {
                    let first = spans.partition_point(|span| span.last_hour < segment.first_hour);
                    spans
                        .get(first)
                        .is_some_and(|span| span.first_hour <= segment.last_hour)
                });
            }
            Some((
                ArmySeries {
                    tag,
                    identities,
                    runs: Vec::new(),
                    segments,
                    membership,
                    mix: None,
                },
                track,
                spans,
            ))
        })
        .collect();
    // Count the selected serialized series before copying its history.
    let mut bytes = 2usize; // Array brackets.
    let mut series_comma = 0;
    for (series, track, spans) in &wanted {
        crate::metadata::check_cancel(cancelled)?;
        bytes += series_comma
            + crate::transport::json_len(series, transport_budget).ok_or_else(reply_bound)?;
        series_comma = 1;
        if bytes > transport_budget {
            return Err(reply_bound());
        }
        let mut run_comma = 0;
        for_each_reply_run(
            track,
            params,
            range,
            spans.as_deref().or(series.membership.as_deref()),
            cancelled,
            |run| {
                bytes += run_comma
                    + crate::transport::json_len(&run, transport_budget).ok_or_else(reply_bound)?;
                run_comma = 1;
                if bytes > transport_budget {
                    return Err(reply_bound());
                }
                Ok(())
            },
        )?;
    }
    let series = wanted
        .into_iter()
        .map(|(mut series, track, spans)| {
            for_each_reply_run(
                track,
                params,
                range,
                spans.as_deref().or(series.membership.as_deref()),
                cancelled,
                |run| {
                    series.runs.push(run);
                    Ok(())
                },
            )?;
            Ok(series)
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok((series, bytes))
}

impl Product for ArmySize {
    const KEY: &'static str = "army-size";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ArmySizeParams;
    type Reply = ArmySizeReply;
    fn types() -> Vec<String> {
        vec![
            ArmyMeasure::decl(),
            ArmyAttribution::decl(),
            ArmySizeParams::decl(),
            ArmyType::decl(),
            "type ArmyRun = [number, number | null, number | null, string?];".into(),
            ArmyMixRun::decl(),
            ArmySeries::decl(),
            ArmySizeReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        if params
            .top
            .is_some_and(|top| top == 0 || !params.tags.is_empty())
        {
            return Err("Invalid country selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| crate::invalid("Invalid branch"))?;
        if !source
            .replay_ref()
            .branches
            .iter()
            .any(|known| known.id == branch)
        {
            return Err(crate::invalid("Unknown branch"));
        }
        let windows: Vec<_> = source
            .replay_ref()
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| HourSpan {
                first_hour: block.first_hour,
                last_hour: block.last_hour,
            })
            .collect();
        let interval = params
            .window
            .map(|index| {
                windows
                    .get(index)
                    .cloned()
                    .ok_or_else(|| crate::invalid("Unknown army history window"))
            })
            .transpose()?;
        let detailed =
            params.measure != ArmyMeasure::Divisions || params.count_for == ArmyAttribution::Host;
        let scan = scanned(source, branch, detailed, cancelled)?;
        let mut warnings = Vec::new();
        warnings.extend(scan.invalid_table_reads.iter().map(|(reason, count)| {
            format!("{count} army table reads rejected ({reason}); affected values stay gaps.")
        }));
        if scan.unreadable > 0 {
            warnings.push(format!("{} division/country attribution reads could not be resolved; affected totals remain unknown.", scan.unreadable));
        }
        if scan.composition_unreadable > 0 && params.measure == ArmyMeasure::Battalions {
            warnings.push(format!(
                "{} country attribution-hours have unresolved division ownership or missing/invalid template composition; battalion totals stay unknown.",
                scan.composition_unreadable));
        }
        let measures = [
            ArmyMeasure::Manpower,
            ArmyMeasure::Divisions,
            ArmyMeasure::Strength,
            ArmyMeasure::Battalions,
        ]
        .into_iter()
        .filter(|measure| match measure {
            ArmyMeasure::Divisions => scan.divisions,
            ArmyMeasure::Strength => scan.strength,
            ArmyMeasure::Manpower => scan.manpower,
            ArmyMeasure::Battalions => scan.battalions,
        })
        .collect();
        let mut projection = params.clone();
        let identities = if params.top.is_none() && !params.tags.is_empty() {
            let identities =
                super::country_history::branch_histories(source, branch, false, &[], cancelled)?;
            let selected = super::country_history::selected_identities(&identities, &params.tags);
            projection.tags = super::country_history::selected_aliases(&identities, &params.tags);
            interval.as_ref().map(|range| {
                projection
                    .tags
                    .iter()
                    .map(|tag| {
                        let runs = identities
                            .countries
                            .get(tag)
                            .map_or_else(Vec::new, |country| {
                                window_identities(country, range)
                                    .into_iter()
                                    .filter(|identity| {
                                        selected.contains(&(identity.slot, identity.country_id))
                                    })
                                    .collect()
                            });
                        (tag.clone(), runs)
                    })
                    .collect::<BTreeMap<_, _>>()
            })
        } else {
            None
        };
        let (mut series, series_bytes) = interval.as_ref().map_or_else(
            || Ok((Vec::new(), 2)),
            |range| reply_series(&scan, &projection, range, identities.as_ref(), cancelled),
        )?;
        let types = if let Some(range) = interval.as_ref()
            && params.by_type
            && params.measure == ArmyMeasure::Battalions
        {
            composition_reply(
                source,
                context,
                &scan,
                params,
                range,
                &mut series,
                &mut warnings,
                cancelled,
            )?
        } else {
            Vec::new()
        };
        let mut reply = ArmySizeReply {
            branch_id: params.branch_id.clone(),
            windows: if params.window.is_none() {
                windows
            } else {
                Vec::new()
            },
            window: params.window,
            interval,
            available: match params.measure {
                ArmyMeasure::Divisions if params.count_for == ArmyAttribution::Host => {
                    scan.strength
                }
                ArmyMeasure::Divisions => scan.divisions,
                ArmyMeasure::Strength => scan.strength,
                ArmyMeasure::Manpower => scan.manpower,
                ArmyMeasure::Battalions => scan.battalions,
            },
            measures,
            types,
            series: Vec::new(),
            warnings,
        };
        let envelope_bytes = crate::transport::json_len(&reply, super::MAX_PRODUCT_REPLY_BYTES)
            .ok_or_else(reply_bound)?;
        let bytes = if series
            .iter()
            .any(|series| series.mix.is_some() || series.identities.is_some())
        {
            crate::transport::json_len(&series, super::MAX_PRODUCT_REPLY_BYTES)
                .ok_or_else(reply_bound)?
        } else {
            series_bytes
        };
        if envelope_bytes + bytes - 2 > super::MAX_PRODUCT_REPLY_BYTES {
            return Err(reply_bound());
        }
        reply.series = series;
        Ok(reply)
    }
}

mod composition;
pub(crate) use composition::check_content;
use composition::{RawMixRun, composition_reply};
mod inventory;
use inventory::Inventory;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scale;
