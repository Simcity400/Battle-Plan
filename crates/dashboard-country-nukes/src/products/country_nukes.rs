//! Every country's nuclear stockpile as the game's top bar shows it.
//!
//! The top bar displays a signed integer: each nuclear amount truncates,
//! then the amounts add with signed-dword wrapping. Ready count instead
//! counts queued drops whose delay elapsed; it is not the available arsenal.
//! Every value is dated to a complete hourly Nukes read. Missing rows,
//! incomplete tables, gaps and identity changes never become zero.

use dashboard_base::{
    Source,
    catalogue::{self, Catalogue},
};
use dashboard_base::{
    products::LoadContext,
    products::MAX_PRODUCT_REPLY_BYTES,
    products::Product,
    products::country_groups::HourSpan,
    products::country_history::{
        RecordedHistories, branch_histories, observe_identity, selected_identities,
    },
    products::validate_tag,
};
use dashboard_text::metadata::{
    IdentityTrack, link_slot, slot_read, tagged_countries, validate_campaign,
};
use engine_native::{
    raid_tooltip_operands::NUCLEAR_BOMB_TOKEN, raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN,
};
use replay::container::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::NukeRow, rows::TableId, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct CountryNukes;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NukesParams {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub top: Option<u32>,
    /// A drill selects the native country holding its tag at this recorded hour.
    #[serde(default)]
    pub origin_hour: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeTypeInfo {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub supported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeCount {
    pub ordinal: u32,
    pub value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub counts: Vec<NukeCount>,
    /// Unknown for a hole in this country's native array; unnamed types still count.
    pub total: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeSeries {
    pub tag: String,
    pub runs: Vec<NukeRun>,
    pub segments: Vec<HourSpan>,
    pub membership: Option<Vec<HourSpan>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryNukesReply {
    pub branch_id: String,
    /// Every fresh complete Nukes read, including reads supplying unchanged runs.
    pub read_hours: Vec<i32>,
    /// Complete reads with no nonzero or unknown array among countries present.
    /// Absent rows are outside the race, never supplied as synthetic zero values.
    pub empty_read_hours: Vec<i32>,
    pub types: Vec<NukeTypeInfo>,
    pub series: Vec<NukeSeries>,
    pub warnings: Vec<String>,
}

/// Recorded nuclear amount to displayed whole stockpile, including its sentinel
/// and machine-width multiply/divide/narrowing.
pub fn displayed_amount(raw: i64) -> i32 {
    if dashboard_base::products::finite_fixed64(raw) {
        (raw.wrapping_mul(100_000) / 10_000_000_000_000) as i32
    } else {
        0xa7c5
    }
}

#[allow(clippy::type_complexity)]
fn counts_by_slot(
    rows: &[NukeRow],
    links: &[CountryLinkRow],
) -> (HashMap<u32, BTreeMap<u32, Option<i32>>>, usize, bool) {
    let mut result = HashMap::<u32, BTreeMap<u32, Option<i32>>>::new();
    let mut unreadable = 0;
    let mut resolved = true;
    for row in rows {
        let Some(slot) = link_slot(links, row.country) else {
            resolved = false;
            unreadable += 1;
            continue;
        };
        match result.entry(slot).or_default().entry(row.nuke_type) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(displayed_amount(row.amount_raw)));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
                unreadable += 1;
            }
        }
    }
    (result, unreadable, resolved)
}

fn stockpile(values: &BTreeMap<u32, Option<i32>>) -> Option<(Vec<NukeCount>, Option<i32>)> {
    let counts: Vec<_> = values
        .iter()
        .map(|(&ordinal, value)| value.map(|value| NukeCount { ordinal, value }))
        .collect::<Option<_>>()?;
    // Only a hole inside this country's recorded array prefix makes its total unknown.
    let total = values
        .keys()
        .enumerate()
        .all(|(index, ordinal)| usize::try_from(*ordinal) == Ok(index))
        .then(|| {
            counts
                .iter()
                .fold(0i32, |total, count| total.wrapping_add(count.value))
        });
    Some((counts, total))
}

fn empty_stockpiles(
    countries: &[CountryRow],
    links: &[CountryLinkRow],
    by_slot: &HashMap<u32, BTreeMap<u32, Option<i32>>>,
    resolved: bool,
) -> bool {
    let tagged = tagged_countries(countries, links);
    let valid = tagged
        .iter()
        .all(|(tag, lookup)| lookup.found().is_some() && std::str::from_utf8(tag).is_ok());
    let slots = tagged
        .values()
        .filter_map(|lookup| lookup.found().map(|(_, slot)| slot))
        .collect::<BTreeSet<_>>();
    empty_resolved(by_slot, |slot| slots.contains(&slot), valid, resolved)
}
fn empty_resolved(
    by_slot: &HashMap<u32, BTreeMap<u32, Option<i32>>>,
    selected: impl Fn(u32) -> bool,
    valid: bool,
    resolved: bool,
) -> bool {
    resolved
        && valid
        && by_slot
            .iter()
            .filter(|(slot, _)| selected(**slot))
            .all(|(_, values)| stockpile(values).is_some_and(|(_, total)| total == Some(0)))
}

/// A fresh complete supplying read, with the native empty-census decision.
fn supplying_read(
    decoded: &DecodedFrame,
    format: replay_data::format::FormatVersion,
) -> Option<bool> {
    if !slot_read(format, &decoded.metrics, TableId::Nukes, decoded.hour) {
        return None;
    }
    let rows = decoded.complete::<tables::Nukes>().ok()?;
    let (Ok(countries), Ok(links)) = (
        decoded.complete::<tables::Countries>(),
        decoded.complete::<tables::CountryLinks>(),
    ) else {
        return Some(false);
    };
    let (by_slot, _, resolved) = counts_by_slot(rows, links);
    Some(empty_stockpiles(countries, links, &by_slot, resolved))
}

/// Maps recorded nuclear ordinals to their name tokens. The token spelling
/// and displayed name come from recorded content.
fn name_token(ordinal: u32) -> Option<u32> {
    match ordinal {
        0 => Some(NUCLEAR_BOMB_TOKEN),
        1 => Some(THERMONUCLEAR_BOMB_TOKEN),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedNukes {
    pub read_hours: Vec<i32>,
    pub empty_read_hours: Vec<i32>,
    pub series: BTreeMap<String, Vec<NukeRun>>,
    pub segments: BTreeMap<String, Vec<HourSpan>>,
    pub types: BTreeSet<u32>,
    pub tokens: BTreeMap<u32, String>,
    pub refused: BTreeMap<u32, usize>,
    pub unreadable: usize,
    pub unknown_hours: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Track {
    runs: Vec<NukeRun>,
    segments: Vec<HourSpan>,
    last: Option<((u32, u32), i32)>,
    last_identity: IdentityTrack,
}

pub mod prepared;
#[derive(Clone, PartialEq, Eq)]
struct Current {
    identity: (u32, u32),
    values: Option<(Vec<NukeCount>, Option<i32>)>,
}
#[derive(Default)]
struct Rates {
    unreadable: usize,
    refused: BTreeMap<u32, usize>,
    unknown: u64,
}
impl Track {
    fn advance(&mut self, hour: i32) {
        let Some(segment) = self.segments.last_mut() else {
            return;
        };
        let before = segment.last_hour;
        segment.last_hour = hour;
        self.last_identity.advance(hour);
        if let Some((_, at)) = &mut self.last
            && *at == before
        {
            *at = hour;
            if let Some(run) = self.runs.last_mut() {
                run.last_hour = hour;
            }
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Scan {
    scanned: ScannedNukes,
    tracks: BTreeMap<String, Track>,
    frontier: Option<i32>,
    live: BTreeSet<String>,
    #[serde(skip)]
    countries: dashboard_base::products::force_census::Countries,
    #[serde(skip)]
    current: BTreeMap<String, Current>,
    #[serde(skip)]
    supplied_slots: BTreeSet<u32>,
    #[serde(skip)]
    rates: Rates,
    #[serde(skip)]
    read: Option<bool>,
    #[serde(skip)]
    empty: bool,
    /// Whether the last hour observed joined its countries and links, and
    /// whether its stockpile table was complete.
    #[serde(skip)]
    joined: bool,
    #[serde(skip)]
    complete: bool,
    #[serde(skip)]
    observed: Vec<String>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
impl Scan {
    /// What observing `hour` does when it follows the last hour observed with
    /// the same country keys, links, name tokens, availability and stockpile
    /// rows, the stockpile table read (`read`) or not as then: the read hour
    /// it dates and the unchanged hour's counts, in constant time. False, with
    /// nothing changed, when observing it would take another path: the hour
    /// does not follow, its read flag differs, the countries and links did
    /// not join, or a read of a stockpile table that is not complete names
    /// its reason from the frame.
    fn advance_quiet(&mut self, hour: i32, read: bool) -> bool {
        let continuous = self
            .frontier
            .is_some_and(|at| at.checked_add(1) == Some(hour));
        if !continuous || self.read != Some(read) || !self.joined || read && !self.complete {
            return false;
        }
        self.observed.clear();
        if read {
            self.scanned.read_hours.push(hour);
        }
        self.unchanged(hour, read);
        true
    }
    /// An hour whose stockpile inputs are the hour before's: the rates of the
    /// last read count again, and a complete read of all-empty stockpiles is
    /// dated empty again.
    fn unchanged(&mut self, hour: i32, complete_read: bool) {
        self.scanned.unreadable += self.rates.unreadable;
        self.scanned.unknown_hours += self.rates.unknown;
        for (&ordinal, &count) in &self.rates.refused {
            *self.scanned.refused.entry(ordinal).or_default() += count;
        }
        if complete_read && self.empty {
            self.scanned.empty_read_hours.push(hour);
        }
        self.frontier = Some(hour);
    }
    pub(crate) fn observe(
        &mut self,
        decoded: &DecodedFrame,
        format: replay_data::format::FormatVersion,
    ) {
        self.observe_changes(decoded, format, None);
        self.seal();
    }
    fn seal(&mut self) {
        if let Some(hour) = self.frontier {
            for tag in &self.live {
                self.tracks
                    .get_mut(tag)
                    .expect("live nuclear holder")
                    .advance(hour);
            }
        }
    }
    fn observe_changes(
        &mut self,
        decoded: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) {
        self.observed.clear();
        let hour = decoded.hour;
        if self.scanned.tokens.len() < 2
            && changes.is_none_or(|changes| changes.rows_changed[TableId::LexicalTokens.index()])
            && let Ok(lexical) = decoded.complete::<tables::LexicalTokens>()
        {
            for ordinal in 0..=1 {
                if self.scanned.tokens.contains_key(&ordinal) {
                    continue;
                }
                let token = name_token(ordinal).expect("native mapping");
                if let Ok(index) = lexical.binary_search_by_key(&token, |row| row.token)
                    && let Ok(text) = std::str::from_utf8(&lexical[index].literal)
                {
                    self.scanned.tokens.insert(ordinal, text.to_owned());
                }
            }
        }
        let continuous = self
            .frontier
            .is_some_and(|at| at.checked_add(1) == Some(hour));
        let read = slot_read(format, &decoded.metrics, TableId::Nukes, hour);
        let rows = decoded.complete::<tables::Nukes>().ok();
        if read && rows.is_some() {
            self.scanned.read_hours.push(hour);
        }
        let joined = self.countries.update(decoded);
        self.joined = joined.is_ok();
        self.complete = rows.is_some();
        let changed = !continuous
            || self.read != Some(read)
            || !matches!(joined, Ok(false))
            || changes.is_none_or(|changes| changes.rows_changed[TableId::Nukes.index()]);
        if !changed {
            self.unchanged(hour, read && rows.is_some());
            return;
        }
        if !continuous {
            self.seal();
        }
        let partial = continuous && matches!(joined, Ok(false)) && !self.current.is_empty();
        self.read = Some(read);
        let mut rates = Rates::default();
        let mut current = BTreeMap::new();
        self.empty = false;
        if joined.is_ok()
            && let (Ok(_countries), Ok(links)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            )
        {
            let (by_slot, unreadable, resolved) =
                counts_by_slot(rows.filter(|_| read).unwrap_or_default(), links);
            rates.unreadable += unreadable;
            #[cfg(test)]
            {
                self.work.row_visits += rows.filter(|_| read).map_or(0, |rows| rows.len());
            }
            if read {
                for row in rows.into_iter().flatten() {
                    self.scanned.types.insert(row.nuke_type);
                    if name_token(row.nuke_type).is_none() {
                        *rates.refused.entry(row.nuke_type).or_default() += 1;
                    }
                }
            }
            self.empty = read
                && rows.is_some()
                && empty_resolved(
                    &by_slot,
                    |slot| self.countries.contains_slot(slot),
                    self.countries.tags_valid(),
                    resolved,
                );
            let mut slots = self.supplied_slots.clone();
            if read {
                slots.extend(by_slot.keys().copied());
            }
            let candidates = if partial {
                self.countries.in_slots(&slots)
            } else {
                self.countries.tags.clone()
            };
            rates.unreadable += self.countries.unreadable_tags;
            for (tag, identity) in &candidates {
                let Some(identity) = *identity else {
                    continue;
                };
                let Ok(tag) = std::str::from_utf8(tag) else {
                    continue;
                };
                let values = if rows.is_none() {
                    None
                } else if read {
                    by_slot.get(&identity.0).and_then(stockpile)
                } else {
                    self.current
                        .get(tag)
                        .filter(|before| continuous && before.identity == identity)
                        .and_then(|before| before.values.clone())
                        .or_else(|| {
                            self.tracks
                                .get(tag)
                                .filter(|_| continuous && self.live.contains(tag))
                                .and_then(|track| {
                                    track
                                        .last
                                        .filter(|(seen, at)| {
                                            *seen == identity && Some(*at) == self.frontier
                                        })
                                        .and_then(|_| {
                                            track
                                                .runs
                                                .last()
                                                .map(|run| (run.counts.clone(), run.total))
                                        })
                                })
                        })
                };
                rates.unknown +=
                    u64::from(values.as_ref().is_some_and(|(_, total)| total.is_none()));
                current.insert(tag.to_owned(), Current { identity, values });
            }
            if read || rows.is_none() {
                self.supplied_slots = by_slot.keys().copied().collect();
            }
        }
        for (tag, after) in &current {
            if continuous && self.current.get(tag) == Some(after) {
                continue;
            }
            let track = self.tracks.entry(tag.clone()).or_default();
            if self.live.contains(tag)
                && let Some(before) = self.frontier
            {
                track.advance(before);
            }
            observe_identity(
                &mut track.segments,
                &mut track.last_identity,
                after.identity,
                hour,
            );
            if let Some((counts, total)) = &after.values {
                let continues = track.last.is_some_and(|(seen, at)| {
                    seen == after.identity && at.checked_add(1) == Some(hour)
                });
                match track.runs.last_mut() {
                    Some(run) if continues && run.counts == *counts && run.total == *total => {
                        run.last_hour = hour
                    }
                    _ => track.runs.push(NukeRun {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment: !continues,
                        counts: counts.clone(),
                        total: *total,
                    }),
                }
                track.last = Some((after.identity, hour));
            }
            self.observed.push(tag.clone());
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
        }
        for tag in self
            .live
            .iter()
            .filter(|tag| !partial && !current.contains_key(*tag))
        {
            if let Some(before) = self.frontier {
                self.tracks
                    .get_mut(tag)
                    .expect("departed nuclear holder")
                    .advance(before);
            }
            self.observed.push(tag.clone());
        }
        if partial {
            self.current.extend(current);
        } else {
            self.live = current.keys().cloned().collect();
            self.current = current;
        }
        self.scanned.unreadable += rates.unreadable;
        self.scanned.unknown_hours += rates.unknown;
        for (&ordinal, &count) in &rates.refused {
            *self.scanned.refused.entry(ordinal).or_default() += count;
        }
        self.rates = rates;
        if self.empty {
            self.scanned.empty_read_hours.push(hour);
        }
        self.frontier = Some(hour);
    }
    fn finish(mut self) -> ScannedNukes {
        self.seal();
        // Stockpile totals use this country's own array count; independent countries
        // can have different counts. A complete contiguous prefix proves that array,
        // even when another country records more types.
        let Self {
            mut scanned,
            tracks,
            ..
        } = self;
        for (tag, track) in tracks {
            if track.runs.is_empty() {
                continue;
            }
            scanned.segments.insert(tag.clone(), track.segments);
            scanned.series.insert(tag, track.runs);
        }
        scanned.read_hours.sort_unstable();
        scanned.read_hours.dedup();
        scanned.empty_read_hours.sort_unstable();
        scanned.empty_read_hours.dedup();
        scanned
    }
}

pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedNukes> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Nukes,
        TableId::LexicalTokens,
    ];
    let mut observer = Scan::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error());
                }
                let Ok(frames) = frames else {
                    continue;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancelled_error());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    observer.observe(decoded, replay.format);
                }
            }
            Ok(())
        },
    )?;
    Ok(observer.finish())
}

fn cancelled_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "Nuclear stockpile request cancelled",
    )
}

pub fn type_info(
    scanned: &ScannedNukes,
    localise: impl Fn(&str) -> Option<String>,
) -> Vec<NukeTypeInfo> {
    scanned
        .types
        .iter()
        .map(|&ordinal| info(scanned, ordinal, &localise))
        .collect()
}

/// Every type the game names, spelled from the recorded lexical tokens
/// whether or not a stockpile read saw it.
pub fn native_type_info(
    scanned: &ScannedNukes,
    localise: impl Fn(&str) -> Option<String>,
) -> Vec<NukeTypeInfo> {
    (0..)
        .take_while(|&ordinal| name_token(ordinal).is_some())
        .map(|ordinal| info(scanned, ordinal, &localise))
        .collect()
}

fn info(
    scanned: &ScannedNukes,
    ordinal: u32,
    localise: impl Fn(&str) -> Option<String>,
) -> NukeTypeInfo {
    let token = scanned.tokens.get(&ordinal).cloned();
    NukeTypeInfo {
        ordinal,
        name: token.as_deref().and_then(&localise),
        token,
        supported: name_token(ordinal).is_some(),
    }
}

pub(crate) fn requested_series(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<NukeSeries>> {
    let membership = membership(scanned, params, history, cancelled)?;
    requested_selected(scanned, params, history, cancelled, membership)
}
fn membership(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
) -> io::Result<Option<BTreeMap<String, Vec<HourSpan>>>> {
    if params.origin_hour.is_some() && history.is_none() {
        return Err(io::Error::other(
            "Nuclear drill identity history is unavailable",
        ));
    }
    let membership = if let Some(history) = history.filter(|_| params.top.is_none()) {
        let selected = if let Some(hour) = params.origin_hour {
            params
                .tags
                .first()
                .and_then(|tag| history.countries.get(tag))
                .and_then(|country| {
                    country
                        .identities
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                })
                .map(|run| (run.slot, run.country_id))
                .into_iter()
                .collect()
        } else {
            selected_identities(history, &params.tags)
        };
        let aliases: BTreeMap<String, Vec<HourSpan>> = history
            .countries
            .iter()
            .filter_map(|(tag, country)| {
                let spans: Vec<_> = country
                    .identities
                    .iter()
                    .filter(|run| selected.contains(&(run.slot, run.country_id)))
                    .map(|run| HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    })
                    .collect();
                (!spans.is_empty()).then(|| (tag.clone(), spans))
            })
            .collect();
        if params.origin_hour.is_some() {
            let mut spans: Vec<_> = aliases.values().flatten().collect();
            spans.sort_by_key(|span| span.first_hour);
            if spans
                .windows(2)
                .any(|pair| pair[0].last_hour >= pair[1].first_hour)
            {
                return Err(io::Error::other(
                    "NuclearDrillIdentityAmbiguous: multiple recorded tags hold the selected native country at one hour",
                ));
            }
        }
        Some(aliases)
    } else {
        params
            .top
            .map(|count| {
                dashboard_base::ranking::top_membership(
                    &scanned.series,
                    count as usize,
                    |run| (run.first_hour, run.last_hour),
                    |run| run.total.filter(|total| *total != 0).map(i64::from),
                    cancelled,
                )
            })
            .transpose()?
    };
    Ok(membership)
}
fn requested_selected(
    scanned: &ScannedNukes,
    params: &NukesParams,
    history: Option<&RecordedHistories>,
    cancelled: &AtomicBool,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<Vec<NukeSeries>> {
    let tags: BTreeSet<&str> = match &membership {
        Some(top) => top.keys().map(String::as_str).collect(),
        None => params.tags.iter().map(String::as_str).collect(),
    };
    let mut series = Vec::new();
    // Measure the serialized selection before copying histories. Only the
    // complete reply's transport budget bounds a recording's country/run count.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SeriesMeasure<'a> {
        tag: &'a str,
        runs: &'a [NukeRun],
        segments: &'a [HourSpan],
        membership: Option<&'a [HourSpan]>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct RunMeasure<'a> {
        first_hour: i32,
        last_hour: i32,
        starts_segment: bool,
        counts: &'a [NukeCount],
        total: Option<i32>,
    }
    let mut bytes = 2usize;
    for tag in &tags {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        let Some(runs) = scanned.series.get(*tag) else {
            continue;
        };
        let spans = membership.as_ref().and_then(|top| top.get(*tag));
        let measure = SeriesMeasure {
            tag,
            runs: if spans.is_some() { &[] } else { runs },
            segments: if params.top.is_none() && history.is_some() {
                spans.map_or(&[][..], Vec::as_slice)
            } else {
                scanned.segments.get(*tag).map_or(&[][..], Vec::as_slice)
            },
            membership: spans.map(Vec::as_slice),
        };
        if bytes > 2 {
            bytes = bytes.saturating_add(1);
        }
        bytes = bytes.saturating_add(
            dashboard_core::transport::json_len(&measure, MAX_PRODUCT_REPLY_BYTES)
                .ok_or_else(reply_too_large)?,
        );
        if let Some(spans) = spans {
            let mut first_piece = true;
            dashboard_base::ranking::for_each_piece(
                runs,
                spans,
                |run| (run.first_hour, run.last_hour),
                cancelled,
                |run, first_hour, last_hour| {
                    let piece = RunMeasure {
                        first_hour,
                        last_hour,
                        starts_segment: run.starts_segment && first_hour == run.first_hour,
                        counts: &run.counts,
                        total: run.total,
                    };
                    if !first_piece {
                        bytes = bytes.saturating_add(1);
                    }
                    first_piece = false;
                    bytes = bytes.saturating_add(
                        dashboard_core::transport::json_len(&piece, MAX_PRODUCT_REPLY_BYTES)
                            .ok_or_else(reply_too_large)?,
                    );
                    if bytes > MAX_PRODUCT_REPLY_BYTES {
                        return Err(reply_too_large());
                    }
                    Ok(())
                },
            )?;
        }
        if bytes > MAX_PRODUCT_REPLY_BYTES {
            return Err(reply_too_large());
        }
    }
    for tag in tags {
        let Some(runs) = scanned.series.get(tag) else {
            continue;
        };
        let spans = membership.as_ref().and_then(|top| top.get(tag));
        let kept = match spans {
            None => runs.clone(),
            Some(spans) => {
                let mut kept = Vec::new();
                dashboard_base::ranking::for_each_piece(
                    runs,
                    spans,
                    |run| (run.first_hour, run.last_hour),
                    cancelled,
                    |run, first_hour, last_hour| {
                        let mut piece = run.clone();
                        piece.first_hour = first_hour;
                        piece.last_hour = last_hour;
                        piece.starts_segment = run.starts_segment && first_hour == run.first_hour;
                        kept.push(piece);
                        Ok(())
                    },
                )?;
                kept
            }
        };
        series.push(NukeSeries {
            tag: tag.to_owned(),
            runs: kept,
            segments: if params.top.is_none() && history.is_some() {
                spans.cloned().unwrap_or_default()
            } else {
                scanned.segments.get(tag).cloned().unwrap_or_default()
            },
            membership: spans.cloned(),
        });
    }
    Ok(series)
}

fn reply_too_large() -> io::Error {
    io::Error::other(
        "NuclearReplyTooLarge: the complete nuclear stockpile reply exceeds the transport budget; no countries or runs were omitted",
    )
}

#[derive(Default)]
struct Scans {
    branch: Option<(u64, Arc<ScannedNukes>)>,
}

pub fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedNukes>> {
    let state = source.state::<Scans>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((id, scanned)) = &cache.branch
        && *id == branch
    {
        return Ok(Arc::clone(scanned));
    }
    let scanned = Arc::new(scan(&source.replay(), branch, cancelled)?);
    cache.branch = Some((branch, Arc::clone(&scanned)));
    Ok(scanned)
}

fn names(catalogue: &Catalogue, scanned: &ScannedNukes) -> Vec<NukeTypeInfo> {
    type_info(scanned, |token| {
        catalogue.localisation.get(token)?;
        catalogue.localised(token, catalogue::View::Plain)
    })
}

impl Product for CountryNukes {
    const KEY: &'static str = "country-nukes";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NukesParams;
    type Reply = CountryNukesReply;

    fn types() -> Vec<String> {
        vec![
            NukesParams::decl(),
            NukeTypeInfo::decl(),
            NukeCount::decl(),
            NukeRun::decl(),
            NukeSeries::decl(),
            CountryNukesReply::decl(),
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
            .is_some_and(|count| count == 0 || !params.tags.is_empty())
        {
            return Err("Invalid top countries");
        }
        if params.origin_hour.is_some() && (params.top.is_some() || params.tags.len() != 1) {
            return Err("A nuclear drill needs one tag at its origin hour");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let scanned = scanned_branch(
            source,
            params.branch_id.parse().expect("validated branch"),
            cancelled,
        )?;
        let history = (params.origin_hour.is_some() || !params.tags.is_empty())
            .then(|| {
                branch_histories(
                    source,
                    params.branch_id.parse().expect("validated branch"),
                    false,
                    &[],
                    cancelled,
                )
            })
            .transpose()?;
        present(
            source,
            params,
            context,
            cancelled,
            &scanned,
            history.as_deref(),
            None,
        )
    }
}

#[cfg(test)]
mod tests;

fn present(
    source: &Source,
    params: &NukesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
    scanned: &ScannedNukes,
    history: Option<&RecordedHistories>,
    prepared_membership: Option<BTreeMap<String, Vec<HourSpan>>>,
) -> io::Result<CountryNukesReply> {
    let mut warnings = Vec::new();
    let types = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => {
            let types = names(&catalogue, scanned);
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            warnings.extend(catalogue.refusal_lines());
            types
        }
        Err(error) => {
            warnings.push(format!("Nuclear type names are unavailable: {error}"));
            type_info(scanned, |_| None)
        }
    };
    for (ordinal, count) in &scanned.refused {
        warnings.push(format!("Unknown type {ordinal}: {count} recorded nuclear rows have no proven native name mapping; their recorded amounts still count in complete country totals."));
    }
    for kind in &types {
        if kind.supported && kind.name.is_none() {
            warnings.push(format!(
                "Nuclear type {}: no name in the recorded content; shown as unknown.",
                kind.token.as_deref().unwrap_or("unknown token")
            ));
        }
    }
    if scanned.unreadable > 0 {
        warnings.push(format!(
            "{} nuclear rows or country tags could not be resolved uniquely and are left out.",
            scanned.unreadable
        ));
    }
    let unknown_hours = scanned.unknown_hours;
    if unknown_hours > 0 {
        warnings.push(format!("{unknown_hours} country-hours have a hole in their native nuclear array; totals stay unknown."));
    }
    let series = if let Some(membership) = prepared_membership {
        requested_selected(scanned, params, history, cancelled, Some(membership))?
    } else {
        requested_series(scanned, params, history, cancelled)?
    };
    if params.origin_hour.is_some() && series.is_empty() {
        warnings.push("No nuclear stockpile or native country identity was recorded for the selected country at its origin hour.".into());
    }
    let reply = CountryNukesReply {
        branch_id: params.branch_id.clone(),
        read_hours: scanned.read_hours.clone(),
        empty_read_hours: scanned.empty_read_hours.clone(),
        types,
        series,
        warnings,
    };
    if dashboard_core::transport::json_len(&reply, MAX_PRODUCT_REPLY_BYTES).is_none() {
        return Err(reply_too_large());
    }
    Ok(reply)
}
