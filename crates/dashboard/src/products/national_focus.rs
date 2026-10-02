//! Compact focus history. Dates bound observed completions, never pre-recording events.
use super::{CountryParams, LoadContext, Product, artwork};
use crate::{
    Artwork, FocusEntry, FocusHistory, FocusOrder, FocusProgress, FocusSegment, Source,
    metadata::validate_campaign,
};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use serde::Serialize;
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
            let mut result = focus_branch(replay, branch, Some(tag), cancelled)?;
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
    let mut segment_count = 0;
    let mut progress_bytes = 0_usize;
    for block in replay.index.iter().filter(|b| b.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Focus request cancelled",
            ));
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = match replay.read_block_selected(block, &selected) {
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
            // Only the scanned countries' rows are indexed: a selected tag
            // costs the same however many countries the frame holds.
            let wanted: Vec<_> = countries
                .iter()
                .filter(|country| selected_tag.is_none_or(|tag| tag.as_bytes() == country.tag))
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
            let owners: BTreeSet<u32> = by_slot.values().map(|focus| focus.country).collect();
            let mut by_country: BTreeMap<_, Vec<_>> = BTreeMap::new();
            for row in completed.iter().filter(|row| owners.contains(&row.country)) {
                by_country.entry(row.country).or_default().push(row);
            }
            for rows in by_country.values_mut() {
                rows.sort_by_key(|r| r.ordinal);
            }
            for (country, slot) in wanted {
                let Some(focus) = by_slot.get(&slot) else {
                    continue;
                };
                let rows = by_country
                    .get(&focus.country)
                    .map_or(&[][..], Vec::as_slice);
                if !valid_order(rows, focus.completed_count) {
                    continue;
                }
                let Ok(tag) = std::str::from_utf8(&country.tag) else {
                    continue;
                };
                if tag.is_empty() || tag.len() > 512 {
                    continue;
                }
                let identity = (slot, country.id);
                let same_identity = identities.get(tag) == Some(&identity);
                let history = result.countries.entry(tag.to_owned()).or_default();
                let current = focus.current.as_deref().filter(|s| !s.is_empty());
                let continuous = focus.continuous.as_deref().filter(|s| !s.is_empty());
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
                    completed: ordered(rows, focus.completed_count)?,
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
                    return Err(io::Error::other("Focus history exceeds its memory budget"));
                }
                history.segments.push(FocusSegment {
                    first_hour: frame.hour,
                    last_hour: frame.hour,
                    order,
                });
                identities.insert(tag.to_owned(), identity);
            }
        }
    }
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

fn date_completions(order: &mut FocusOrder, previous: &FocusOrder, hour: i32) {
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

/// Whether a country's completed focuses are whole and in order. Any number is
/// valid: a modded tree may complete more focuses than any base game tree has.
fn valid_order(rows: &[&replay::rows::CompletedFocusRow], count: i32) -> bool {
    count >= 0
        && count as usize == rows.len()
        && rows.iter().enumerate().all(|(index, row)| {
            row.ordinal == index as u32 && !row.token.is_empty() && row.token.len() <= 512
        })
}

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
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct State {
    artwork: Option<artwork::Context>,
    history: FocusCache,
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
        let (history, progress_stopped) =
            state
                .history
                .get(&source.replay(), params.branch(), &params.tag, cancelled)?;
        let art = artwork::ensure(&mut state.artwork, source, content, artwork::Kind::Focus)?;
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
            .collect();
        let (artwork, mut warnings) = art.catalogue(tokens, cancelled)?;
        if let Some(hour) = progress_stopped {
            warnings.push(format!(
                "Focus progress is too large to keep in full; it is unavailable from hour {hour}."
            ));
        }
        // Names left out for a localisation refusal, by reason and key.
        warnings.extend(art.refusal_lines());
        Ok(FocusReply {
            history,
            artwork,
            warnings,
        })
    }
}
