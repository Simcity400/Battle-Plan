//! Compact focus history. Dates bound observed completions, never pre-recording events.
use super::{CountryParams, LoadContext, Product, artwork};
use crate::{
    Artwork, FocusEntry, FocusHistory, FocusOrder, FocusProgress, FocusSegment, Source,
    metadata::{CountryKeys, validate_campaign},
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, ViewIdentity},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

#[cfg(test)]
pub fn focus_order(
    replay: &Replay,
    branch: u64,
    hour: i32,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<FocusOrder> {
    focus_history(replay, branch, tag, cancelled)?.at(hour)
}

/// A branch projection of the scanned countries: one requested country, or
/// every country when no tag is selected.
pub struct FocusBranch {
    pub countries: BTreeMap<String, FocusHistory>,
    pub retained_bytes: usize,
    /// The first hour whose focus progress was not kept because progress
    /// reached its own budget. Order history is unaffected; progress after it
    /// is absent, never carried forward.
    pub progress_stopped: Option<i32>,
}

/// Retained bytes for focus progress runs, apart from the order history's
/// budget: progress changes daily for every country, so a long or heavily
/// modded campaign stops recording progress instead of failing the product.
const PROGRESS_BUDGET_BYTES: usize = 48 * 1024 * 1024;
/// A progress run's retained size: the struct plus its decimal string.
const PROGRESS_RUN_BYTES: usize = std::mem::size_of::<FocusProgress>() + 20;

const MAX_CACHED_BYTES: usize = 64 * 1024 * 1024;
const MAX_CACHED_COUNTRIES: usize = 16;

/// Each request scans only its own country, so the memory budget and the work
/// never grow with how many countries the recording holds. Recently viewed
/// countries stay cached within one shared byte budget.
#[derive(Default)]
pub struct FocusCache {
    countries: std::collections::VecDeque<CachedCountry>,
}

struct CachedCountry {
    branch: u64,
    tag: String,
    history: FocusHistory,
    bytes: usize,
    progress_stopped: Option<i32>,
}

impl FocusCache {
    pub fn get(
        &mut self,
        replay: &Replay,
        branch: u64,
        tag: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<(FocusHistory, Option<i32>)> {
        self.get_observing(replay, branch, tag, cancelled, &mut |_| Ok(()))
    }

    fn get_observing(
        &mut self,
        replay: &Replay,
        branch: u64,
        tag: &str,
        cancelled: &AtomicBool,
        observe: &mut dyn FnMut(&DecodedFrame) -> io::Result<()>,
    ) -> io::Result<(FocusHistory, Option<i32>)> {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Focus request cancelled",
            ));
        }
        if let Some(index) = self
            .countries
            .iter()
            .position(|entry| entry.branch == branch && entry.tag == tag)
        {
            let entry = self.countries.remove(index).unwrap();
            self.countries.push_back(entry);
        } else {
            let mut result = focus_branch_observing(
                replay,
                branch,
                Some(tag),
                cancelled,
                PROGRESS_BUDGET_BYTES,
                observe,
            )?;
            let history = result
                .countries
                .remove(tag)
                .ok_or_else(|| io::Error::other("No recorded focus history for this country"))?;
            let bytes = result.retained_bytes;
            while !self.countries.is_empty()
                && (self.countries.len() >= MAX_CACHED_COUNTRIES
                    || self
                        .countries
                        .iter()
                        .map(|entry| entry.bytes)
                        .sum::<usize>()
                        + bytes
                        > MAX_CACHED_BYTES)
            {
                self.countries.pop_front();
            }
            self.countries.push_back(CachedCountry {
                branch,
                tag: tag.to_owned(),
                history,
                bytes,
                progress_stopped: result.progress_stopped,
            });
        }
        let entry = self.countries.back().unwrap();
        Ok((entry.history.clone(), entry.progress_stopped))
    }
}

#[cfg(test)]
impl FocusHistory {
    pub fn at(&self, hour: i32) -> io::Result<FocusOrder> {
        let index = self.segments.partition_point(|s| s.first_hour <= hour);
        self.segments
            .get(index.wrapping_sub(1))
            .filter(|s| hour <= s.last_hour)
            .map(|s| s.order.clone())
            .ok_or_else(|| {
                io::Error::other("No complete recorded focus data for this country at this hour")
            })
    }
}

#[cfg(test)]
pub fn focus_history(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<FocusHistory> {
    focus_branch(replay, branch, Some(tag), cancelled)?
        .countries
        .remove(tag)
        .ok_or_else(|| io::Error::other("No recorded focus history for this country"))
}

/// A successful read's accepted tags. Held focus tables and unchanged country
/// keys repeat the same validation; only consecutive hours extend its runs.
#[derive(Default)]
struct HeldFocusRead {
    views: Option<[ViewIdentity; 4]>,
    keys: CountryKeys,
    hour: Option<i32>,
    tags: BTreeSet<String>,
    repeatable: bool,
}

impl HeldFocusRead {
    const TABLES: [TableId; 4] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Focus,
        TableId::CompletedFocuses,
    ];

    fn can_extend(&self, frame: &DecodedFrame, countries: &[replay::rows::CountryRow]) -> bool {
        let Some([country_view, views @ ..]) = &self.views else {
            return false;
        };
        self.repeatable
            && self.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            && views
                .iter()
                .zip(&Self::TABLES[1..])
                .all(|(view, table)| view.is(&frame.tables[table.index()]))
            && (country_view.is(&frame.tables[TableId::Countries.index()])
                || self.keys.same(countries))
    }

    fn begin(&mut self, frame: &DecodedFrame, countries: &[replay::rows::CountryRow]) {
        self.views = Some(Self::TABLES.map(|table| frame.tables[table.index()].identity()));
        self.keys = CountryKeys::read(countries);
        self.hour = Some(frame.hour);
        self.tags.clear();
        self.repeatable = true;
    }

    fn accept(&mut self, tag: &str) {
        // Duplicate tags are processed in recorded row order. They may select
        // different identities, so that read cannot extend as one held order.
        self.repeatable &= self.tags.insert(tag.to_owned());
    }

    fn extend(&mut self, result: &mut FocusBranch, frame: &DecodedFrame) {
        for tag in &self.tags {
            let history = result
                .countries
                .get_mut(tag)
                .expect("accepted focus history");
            history
                .segments
                .last_mut()
                .expect("accepted focus order")
                .last_hour = frame.hour;
            if result.progress_stopped.is_none() {
                history
                    .progress
                    .last_mut()
                    .expect("accepted focus progress")
                    .last_hour = frame.hour;
            }
        }
        self.hour = Some(frame.hour);
        if let Some(views) = &mut self.views {
            views[0] = frame.tables[TableId::Countries.index()].identity();
        }
    }
}

/// Scans one branch for `selected_tag`, or for every country when it is None.
/// The memory budget applies to the scanned countries only.
pub fn focus_branch(
    replay: &Replay,
    branch: u64,
    selected_tag: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<FocusBranch> {
    focus_branch_within(
        replay,
        branch,
        selected_tag,
        cancelled,
        PROGRESS_BUDGET_BYTES,
    )
}

/// [`focus_branch`] with the progress budget given, so tests can reach it.
pub(crate) fn focus_branch_within(
    replay: &Replay,
    branch: u64,
    selected_tag: Option<&str>,
    cancelled: &AtomicBool,
    progress_budget: usize,
) -> io::Result<FocusBranch> {
    focus_branch_observing(
        replay,
        branch,
        selected_tag,
        cancelled,
        progress_budget,
        &mut |_| Ok(()),
    )
}

/// New selected-country order segments expose only already verified frames.
/// The observer retains its own bounded projection, never a second frame list.
fn focus_branch_observing(
    replay: &Replay,
    branch: u64,
    selected_tag: Option<&str>,
    cancelled: &AtomicBool,
    progress_budget: usize,
    observe: &mut dyn FnMut(&DecodedFrame) -> io::Result<()>,
) -> io::Result<FocusBranch> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Focus request cancelled",
        ));
    }
    let selected = [
        replay::rows::TableId::Campaign,
        replay::rows::TableId::Countries,
        replay::rows::TableId::CountryLinks,
        replay::rows::TableId::Focus,
        replay::rows::TableId::CompletedFocuses,
    ];
    let mut result = FocusBranch {
        countries: BTreeMap::new(),
        retained_bytes: 0,
        progress_stopped: None,
    };
    let mut identities = BTreeMap::new();
    let mut held_read = HeldFocusRead::default();
    let mut segment_count = 0;
    let mut progress_bytes = 0_usize;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|b| b.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "Focus request cancelled",
                    ));
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(_) => continue,
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Focus request cancelled",
                        ));
                    }
                    let decoded = match decoder.advance(&frame) {
                        Ok(frame) => frame,
                        Err(_) => break,
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let (Ok(countries), Ok(links), Ok(focuses), Ok(completed)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::Focus>(),
                        decoded.complete::<tables::CompletedFocuses>(),
                    ) else {
                        continue;
                    };
                    if held_read.can_extend(decoded, countries) {
                        held_read.extend(&mut result, decoded);
                        continue;
                    }
                    held_read.begin(decoded, countries);
                    // Only the scanned countries' rows are indexed: a selected tag
                    // costs the same however many countries the frame holds.
                    let wanted: Vec<_> = countries
                        .iter()
                        .filter(|country| {
                            selected_tag.is_none_or(|tag| tag.as_bytes() == country.tag)
                        })
                        .filter_map(|country| {
                            crate::metadata::mapped_slot(country, links)
                                .ok()
                                .map(|slot| (country, slot))
                        })
                        .collect();
                    if wanted.is_empty() {
                        continue;
                    }
                    let slots: BTreeSet<u32> = wanted.iter().map(|(_, slot)| *slot).collect();
                    let by_slot: BTreeMap<_, _> = focuses
                        .iter()
                        .filter_map(|focus| {
                            links
                                .binary_search_by_key(&focus.country, |l| l.id)
                                .ok()
                                .map(|i| (links[i].canonical_slot, focus))
                        })
                        .filter(|(slot, _)| slots.contains(slot))
                        .collect();
                    let mut new_selected_segment = false;
                    for (country, slot) in wanted {
                        let Some(focus) = by_slot.get(&slot) else {
                            continue;
                        };
                        // Complete decoded tables have unique ascending (country, ordinal)
                        // keys. Keep the raw focus owner: its alias need not equal the slot.
                        let rows = completed_for_owner(completed, focus.country);
                        if !valid_order(rows.iter(), focus.completed_count) {
                            continue;
                        }
                        let Ok(tag) = std::str::from_utf8(&country.tag) else {
                            continue;
                        };
                        if tag.is_empty() || tag.len() > 512 {
                            continue;
                        }
                        held_read.accept(tag);
                        let identity = (slot, country.id);
                        let same_identity = identities.get(tag) == Some(&identity);
                        let history = result.countries.entry(tag.to_owned()).or_default();
                        let current = focus.current.as_deref().filter(|s| !s.is_empty());
                        let continuous = focus.continuous.as_deref().filter(|s| !s.is_empty());
                        // Lossy output differs from invalid raw bytes, so the normal
                        // order comparison must still start a segment each hour.
                        held_read.repeatable &= current
                            .is_none_or(|bytes| std::str::from_utf8(bytes).is_ok())
                            && continuous.is_none_or(|bytes| std::str::from_utf8(bytes).is_ok())
                            && rows
                                .iter()
                                .all(|row| std::str::from_utf8(&row.token).is_ok());
                        // Progress changes once a day; runs never bridge a gap or an identity change.
                        match history.progress.last_mut() {
                            Some(run)
                                if result.progress_stopped.is_none()
                                    && same_identity
                                    && run.last_hour.checked_add(1) == Some(frame.hour)
                                    && run.progress_raw.parse::<i64>() == Ok(focus.progress) =>
                            {
                                run.last_hour = frame.hour;
                            }
                            _ if result.progress_stopped.is_some() => {}
                            _ if progress_bytes + PROGRESS_RUN_BYTES > progress_budget => {
                                result.progress_stopped = Some(frame.hour);
                            }
                            _ => {
                                progress_bytes += PROGRESS_RUN_BYTES;
                                history.progress.push(FocusProgress {
                                    first_hour: frame.hour,
                                    last_hour: frame.hour,
                                    progress_raw: focus.progress.to_string(),
                                });
                            }
                        }
                        if let Some(previous) = history.segments.last_mut() {
                            let order = &previous.order;
                            // Compare borrowed rows first: unchanged hours allocate no focus strings.
                            if same_identity
                                && previous.last_hour.checked_add(1) == Some(frame.hour)
                                && order.current.as_ref().map(|s| s.as_bytes()) == current
                                && order.continuous.as_ref().map(|s| s.as_bytes()) == continuous
                                && order.paused == (focus.paused != 0)
                                && order.completed.len() == rows.len()
                                && order
                                    .completed
                                    .iter()
                                    .zip(rows)
                                    .all(|(a, b)| a.token.as_bytes() == b.token)
                            {
                                previous.last_hour = frame.hour;
                                continue;
                            }
                        }
                        let mut order = FocusOrder {
                            completed: rows
                                .iter()
                                .map(|row| FocusEntry {
                                    ordinal: row.ordinal,
                                    token: String::from_utf8_lossy(&row.token).into_owned(),
                                    completed_by_hour: None,
                                })
                                .collect(),
                            current: current.map(|s| String::from_utf8_lossy(s).into_owned()),
                            continuous: continuous.map(|s| String::from_utf8_lossy(s).into_owned()),
                            paused: focus.paused != 0,
                        };
                        if same_identity && let Some(previous) = history.segments.last() {
                            date_completions(&mut order, &previous.order, frame.hour);
                        }
                        result.retained_bytes += 256
                            + tag.len()
                            + order.current.as_ref().map_or(0, String::len)
                            + order.continuous.as_ref().map_or(0, String::len)
                            + order
                                .completed
                                .iter()
                                .map(|r| 64 + r.token.len())
                                .sum::<usize>();
                        segment_count += 1;
                        if result.retained_bytes > 64 * 1024 * 1024 || segment_count > 100000 {
                            return Err(io::Error::other(
                                "Focus history exceeds its memory budget",
                            ));
                        }
                        history.segments.push(FocusSegment {
                            first_hour: frame.hour,
                            last_hour: frame.hour,
                            order,
                        });
                        identities.insert(tag.to_owned(), identity);
                        new_selected_segment = selected_tag.is_some();
                    }
                    if new_selected_segment {
                        // scan_blocks hands out frames only after read_block_selected
                        // has verified the whole block, including its checksum.
                        observe(decoded)?;
                    }
                }
            }
            Ok(())
        },
    )?;
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Focus request cancelled",
        ));
    }
    // The cache's shared budget counts both; only order history can fail the scan.
    result.retained_bytes += progress_bytes;
    Ok(result)
}

pub(crate) fn date_completions(order: &mut FocusOrder, previous: &FocusOrder, hour: i32) {
    // Only an unchanged prefix establishes the identity of existing ordinals.
    // A reset/reorder starts another undated list rather than reusing old dates.
    if order.completed.len() < previous.completed.len()
        || !order
            .completed
            .iter()
            .zip(&previous.completed)
            .all(|(a, b)| a.token == b.token)
    {
        return;
    }
    for (index, entry) in order.completed.iter_mut().enumerate() {
        entry.completed_by_hour = previous
            .completed
            .get(index)
            .map_or(Some(hour), |old| old.completed_by_hour);
    }
}

/// Only these tables are needed to date focus completions. A consumer can union
/// this selection with its presentation dependencies in one forward block pass.
pub(crate) const FOCUS_COMPLETION_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Focus,
    TableId::CompletedFocuses,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FocusCompletion {
    pub tag: String,
    pub token: String,
    pub hour: i32,
}

/// One latest ordered completion list per recorded identity; no historical full
/// lists, progress, active focus or UI segments are retained. Completed entries
/// append only ordinal/token, so emitted hours are observation bounds.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct FocusCompletionProjection {
    #[serde(with = "crate::prepared_production::map_pairs")]
    previous: BTreeMap<(u32, u32), FocusOrder>,
    #[serde(skip)]
    views: Option<[ViewIdentity; 4]>,
    /// The last read's country keys, and the refusals a read of the same tables repeats.
    #[serde(skip)]
    last_read: Option<(CountryKeys, BTreeMap<&'static str, usize>)>,
    /// The last frame's hour, so a break in the recording is noticed.
    last_hour: Option<i32>,
    #[cfg(test)]
    changed_frames: usize,
    pub observed: bool,
    pub unavailable: BTreeMap<String, usize>,
}

impl FocusCompletionProjection {
    fn refuse(&mut self, reason: &'static str, count: usize) {
        *self.unavailable.entry(reason.to_owned()).or_default() += count;
        if let Some((_, repeated)) = &mut self.last_read {
            *repeated.entry(reason).or_default() += count;
        }
    }

    pub(crate) fn observe(&mut self, frame: &DecodedFrame) -> Vec<FocusCompletion> {
        use crate::metadata::{TaggedCountry, link_slot, tagged_countries};
        // A completion is dated only by consecutive reads. Across a break in the
        // recording the first read after it is a new baseline, so a focus completed
        // inside the break is not listed; each such break is counted once.
        if self
            .last_hour
            .is_some_and(|last| last.checked_add(1) != Some(frame.hour))
        {
            if !self.previous.is_empty() {
                *self
                    .unavailable
                    .entry("Focus completion observation: completions inside a break in the recording are not dated".to_owned())
                    .or_default() += 1;
            }
            self.previous.clear();
            self.views = None;
        }
        self.last_hour = Some(frame.hour);
        const VIEWS: [TableId; 4] = [
            TableId::Countries,
            TableId::Focus,
            TableId::CompletedFocuses,
            TableId::CountryLinks,
        ];
        let countries = frame.complete::<tables::Countries>();
        if let Some([country_view, views @ ..]) = &mut self.views
            && views
                .iter()
                .zip(&VIEWS[1..])
                .all(|(view, table)| view.is(&frame.tables[table.index()]))
        {
            let view = &frame.tables[TableId::Countries.index()];
            if country_view.is(view) {
                return Vec::new();
            }
            // Hourly balances change Countries, of which this reads only the keys:
            // the same keys read as the last read did, counting its refusals again.
            if let (Ok(rows), Some((keys, repeated))) = (&countries, &self.last_read)
                && keys.same(rows)
            {
                *country_view = view.identity();
                for (reason, count) in repeated {
                    *self.unavailable.entry((*reason).to_owned()).or_default() += count;
                }
                return Vec::new();
            }
        }
        self.views = Some(VIEWS.map(|table| frame.tables[table.index()].identity()));
        self.last_read = countries
            .as_ref()
            .ok()
            .map(|rows| (CountryKeys::read(rows), BTreeMap::new()));
        #[cfg(test)]
        {
            self.changed_frames += 1;
        }
        let (Ok(countries), Ok(links), Ok(focuses), Ok(completed)) = (
            countries,
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::Focus>(),
            frame.complete::<tables::CompletedFocuses>(),
        ) else {
            self.refuse(
                "Focus completion observation: required tables incomplete",
                1,
            );
            return Vec::new();
        };
        let identities = tagged_countries(countries, links);
        let present: BTreeSet<_> = identities
            .values()
            .filter_map(|lookup| lookup.found().map(|(country, slot)| (slot, country.id)))
            .collect();
        self.previous
            .retain(|identity, _| present.contains(identity));
        let mut by_slot = BTreeMap::<u32, &replay::rows::FocusRow>::new();
        let mut duplicate_slots = BTreeSet::new();
        for focus in focuses {
            let Some(slot) = link_slot(links, focus.country) else {
                self.refuse("Focus completion observation: unresolved country link", 1);
                continue;
            };
            if by_slot.insert(slot, focus).is_some() {
                duplicate_slots.insert(slot);
                self.refuse(
                    "Focus completion observation: duplicate linked focus row",
                    1,
                );
            }
        }
        let mut by_owner = BTreeMap::<u32, Vec<&replay::rows::CompletedFocusRow>>::new();
        for row in completed {
            by_owner.entry(row.country).or_default().push(row);
        }
        for rows in by_owner.values_mut() {
            rows.sort_by_key(|row| row.ordinal);
        }
        let mut events = Vec::new();
        for (tag, lookup) in identities {
            let TaggedCountry::Found(country, slot) = lookup else {
                self.refuse(
                    "Focus completion observation: unresolved country identity",
                    1,
                );
                continue;
            };
            if duplicate_slots.contains(&slot) {
                continue;
            }
            let Some(focus) = by_slot.get(&slot) else {
                self.refuse("Focus completion observation: country focus row missing", 1);
                continue;
            };
            let rows = by_owner.get(&focus.country).map_or(&[][..], Vec::as_slice);
            if !valid_order(rows.iter().copied(), focus.completed_count) {
                self.refuse(
                    "Focus completion observation: completed order incomplete",
                    1,
                );
                continue;
            }
            let Ok(tag) = std::str::from_utf8(tag) else {
                self.refuse("Focus completion observation: non-UTF-8 country tag", 1);
                continue;
            };
            if rows
                .iter()
                .any(|row| std::str::from_utf8(&row.token).is_err())
            {
                self.refuse("Focus completion observation: non-UTF-8 focus token", 1);
                continue;
            }
            self.observed = true;
            let identity = (slot, country.id);
            // Unchanged lists allocate no tokens and no historical copies.
            if self.previous.get(&identity).is_some_and(|old| {
                old.completed.len() == rows.len()
                    && old
                        .completed
                        .iter()
                        .zip(rows)
                        .all(|(entry, row)| entry.token.as_bytes() == row.token)
            }) {
                continue;
            }
            let mut current = FocusOrder {
                completed: rows
                    .iter()
                    .map(|row| FocusEntry {
                        ordinal: row.ordinal,
                        token: std::str::from_utf8(&row.token)
                            .expect("validated UTF-8 token")
                            .to_owned(),
                        completed_by_hour: None,
                    })
                    .collect(),
                current: None,
                continuous: None,
                paused: false,
            };
            if let Some(previous) = self.previous.get(&identity) {
                date_completions(&mut current, previous, frame.hour);
                events.extend(
                    current
                        .completed
                        .iter()
                        .filter(|entry| entry.completed_by_hour == Some(frame.hour))
                        .map(|entry| FocusCompletion {
                            tag: tag.to_owned(),
                            token: entry.token.clone(),
                            hour: frame.hour,
                        }),
                );
                if current.completed.len() < previous.completed.len()
                    || !current
                        .completed
                        .iter()
                        .zip(&previous.completed)
                        .all(|(a, b)| a.token == b.token)
                {
                    // Only a changed list resets: reading the same tables never repeats this.
                    *self
                        .unavailable
                        .entry(
                            "Focus completion observation: reset or reordered completion list has no event dates".to_owned())
                        .or_default() += 1;
                }
            }
            self.previous.insert(identity, current);
        }
        events
    }
}

/// Whether a country's completed focuses are whole and in order. Any number is
/// valid: a modded tree may complete more focuses than any base game tree has.
pub(crate) fn valid_order<'a>(
    rows: impl ExactSizeIterator<Item = &'a replay::rows::CompletedFocusRow>,
    count: i32,
) -> bool {
    count >= 0
        && count as usize == rows.len()
        && rows.enumerate().all(|(index, row)| {
            row.ordinal == index as u32 && !row.token.is_empty() && row.token.len() <= 512
        })
}

/// Borrow one raw owner's range from a complete, key-validated decoded table.
pub(crate) fn completed_for_owner(
    rows: &[replay::rows::CompletedFocusRow],
    owner: u32,
) -> &[replay::rows::CompletedFocusRow] {
    let first = rows.partition_point(|row| completed_owner_key(row) < owner);
    let remaining = &rows[first..];
    let end = remaining.partition_point(|row| completed_owner_key(row) <= owner);
    &remaining[..end]
}

#[cfg(test)]
std::thread_local! {
    static COMPLETED_OWNER_INSPECTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn completed_owner_key(row: &replay::rows::CompletedFocusRow) -> u32 {
    #[cfg(test)]
    COMPLETED_OWNER_INSPECTIONS.with(|count| count.set(count.get() + 1));
    row.country
}

#[cfg(test)]
fn ordered(rows: &[&replay::rows::CompletedFocusRow], count: i32) -> io::Result<Vec<FocusEntry>> {
    if count < 0 || count as usize != rows.len() {
        return Err(io::Error::other("Completed focus order is incomplete"));
    }
    let mut rows = rows.to_vec();
    rows.sort_by_key(|row| row.ordinal);
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            if row.ordinal != index as u32 || row.token.is_empty() || row.token.len() > 512 {
                return Err(io::Error::other(
                    "Completed focus order has missing or invalid entries",
                ));
            }
            Ok(FocusEntry {
                ordinal: row.ordinal,
                token: String::from_utf8_lossy(&row.token).into_owned(),
                completed_by_hour: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;

pub struct NationalFocus;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FocusReply {
    pub history: FocusHistory,
    pub artwork: Vec<Artwork>,
    /// Country-independent text only; scoped titles still require dated reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub static_text: Option<Vec<Artwork>>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct State {
    artwork: Option<artwork::Context>,
    history: FocusCache,
}

/// Cold content preparation is independent of the verified recording scan.
/// Join it before returning, while retaining the scan's refusal precedence.
fn prepare_while_scanning<T>(
    cancelled: &AtomicBool,
    scan: impl FnOnce() -> io::Result<T>,
    prepare: impl FnOnce() -> io::Result<()> + Send,
) -> io::Result<T> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Focus request cancelled",
        ));
    }
    let (scanned, prepared) = std::thread::scope(|scope| {
        let preparation = scope.spawn(prepare);
        let scanned = scan();
        (scanned, preparation.join())
    });
    // Preparation did not run on a failed scan in the serial path. Its error
    // or panic must not replace that scan's existing refusal.
    let scanned = scanned?;
    prepared.unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
    Ok(scanned)
}

impl Product for NationalFocus {
    const KEY: &'static str = "national-focus";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = FocusReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let content = context.content()?;
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let replay = source.replay();
        let State { history, artwork } = &mut *state;
        let mut scan = || {
            history.get_observing(
                &replay,
                params.branch(),
                &params.tag,
                cancelled,
                &mut |frame| super::focus_artwork::retain_basic(source, frame, cancelled),
            )
        };
        let (history, progress_stopped) = if artwork.is_none() {
            prepare_while_scanning(cancelled, scan, || {
                artwork::ensure(artwork, source, content, artwork::Kind::Focus).map(|_| ())
            })?
        } else {
            let scanned = scan()?;
            artwork::ensure(artwork, source, content, artwork::Kind::Focus)?;
            scanned
        };
        let art = artwork.as_mut().expect("focus artwork context prepared");
        let tokens = history
            .segments
            .iter()
            .flat_map(|segment| {
                segment
                    .order
                    .completed
                    .iter()
                    .map(|r| r.token.as_str())
                    .chain(segment.order.current.as_deref())
                    .chain(segment.order.continuous.as_deref())
            })
            .collect::<BTreeSet<_>>();
        let (static_text, _) = art.text_catalogue(tokens.clone(), cancelled)?;
        let (artwork, mut warnings) = art.focus_metadata(tokens, cancelled)?;
        if let Some(hour) = progress_stopped {
            warnings.push(format!(
                "Focus progress is too large to keep in full; it is unavailable from hour {hour}."
            ));
        }
        // Dated artwork requests report their own selector/localisation refusals.
        let reply = FocusReply {
            history,
            artwork,
            static_text: Some(static_text),
            warnings,
        };
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
