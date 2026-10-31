//! The stockpiles read from a recording's rows, frame by frame: the tests' oracle of the
//! family's columnar preparation ([`super::facts`], [`super::emit`]), its whole reply, and what
//! the event feed reads of it.

use super::{
    query::{Feed, Increase},
    *,
};
use dashboard_base::products::country_history::{branch_histories, observe_identity};
use dashboard_text::metadata::{IdentityTrack, link_slot, slot_read, validate_campaign};
use replay::container::Replay;
use replay_data::{rows::CountryLinkRow, rows::NukeRow, rows::TableId, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::FrameDecoder};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
};

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

fn stockpile(
    values: &BTreeMap<u32, Option<i32>>,
) -> Option<(Vec<NukeCount>, Option<i32>)> {
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

#[derive(Default)]
struct Track {
    runs: Vec<NukeRun>,
    segments: Vec<HourSpan>,
    last: Option<((u32, u32), i32)>,
    last_identity: IdentityTrack,
}

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

/// The hourly census, observing every frame whole.
#[derive(Default)]
pub struct Census {
    scanned: ScannedNukes,
    tracks: BTreeMap<String, Track>,
    frontier: Option<i32>,
    live: BTreeSet<String>,
    countries: dashboard_base::products::force_census::Countries,
    current: BTreeMap<String, Current>,
    increases: Vec<Increase>,
    refusals: BTreeMap<String, Vec<HourSpan>>,
}

impl Census {
    pub fn observe(&mut self, decoded: &DecodedFrame, format: replay_data::format::FormatVersion) {
        self.observe_changes(decoded, format);
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
    ) {
        let hour = decoded.hour;
        if self.scanned.tokens.len() < 2
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
        if read && let Err(error) = decoded.complete::<tables::Nukes>() {
            let reason = match &*decoded.tables[TableId::Nukes.index()] {
                replay_frame::frame::TableView::Unavailable { reason } => reason.clone(),
                _ => {
                    let text = error.to_string();
                    text.strip_prefix(&format!("hour {hour} "))
                        .unwrap_or(&text)
                        .to_owned()
                }
            };
            dashboard_base::products::country_groups::mark_unobserved(
                self.refusals.entry(reason).or_default(),
                hour,
                hour,
            );
        }
        let joined = self.countries.update(decoded);
        if !continuous {
            self.seal();
        }
        let mut rates = Rates::default();
        let mut current = BTreeMap::new();
        let mut empty = false;
        if joined.is_ok()
            && let (Ok(_countries), Ok(links)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
            )
        {
            let (by_slot, unreadable, resolved) =
                counts_by_slot(rows.filter(|_| read).unwrap_or_default(), links);
            rates.unreadable += unreadable;
            if read {
                for row in rows.into_iter().flatten() {
                    self.scanned.types.insert(row.nuke_type);
                    if name_token(row.nuke_type).is_none() {
                        *rates.refused.entry(row.nuke_type).or_default() += 1;
                    }
                }
            }
            empty = read
                && rows.is_some()
                && empty_resolved(
                    &by_slot,
                    |slot| self.countries.contains_slot(slot),
                    self.countries.tags_valid(),
                    resolved,
                );
            rates.unreadable += self.countries.unreadable_tags;
            for (tag, identity) in &self.countries.tags {
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
                };
                rates.unknown +=
                    u64::from(values.as_ref().is_some_and(|(_, total)| total.is_none()));
                current.insert(tag.to_owned(), Current { identity, values });
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
                    held => {
                        // A stockpile growing within its segment is an increase.
                        if continues
                            && let (Some(before), Some(now)) =
                                (held.and_then(|run| run.total), *total)
                            && let Ok(amount) = u32::try_from(i64::from(now) - i64::from(before))
                            && amount > 0
                        {
                            self.increases.push(Increase {
                                tag: tag.clone(),
                                slot: after.identity.0,
                                country_id: after.identity.1,
                                amount,
                                hour,
                            });
                        }
                        track.runs.push(NukeRun {
                            first_hour: hour,
                            last_hour: hour,
                            starts_segment: !continues,
                            counts: counts.clone(),
                            total: *total,
                        });
                    }
                }
                track.last = Some((after.identity, hour));
            }
        }
        for tag in self.live.iter().filter(|tag| !current.contains_key(*tag)) {
            if let Some(before) = self.frontier {
                self.tracks
                    .get_mut(tag)
                    .expect("departed nuclear holder")
                    .advance(before);
            }
        }
        self.live = current.keys().cloned().collect();
        self.current = current;
        self.scanned.unreadable += rates.unreadable;
        self.scanned.unknown_hours += rates.unknown;
        for (&ordinal, &count) in &rates.refused {
            *self.scanned.refused.entry(ordinal).or_default() += count;
        }
        if empty {
            self.scanned.empty_read_hours.push(hour);
        }
        self.frontier = Some(hour);
    }

    /// What it observed: the whole branch's stockpiles, and what the event feed reads of them.
    pub fn finish(mut self) -> (ScannedNukes, Feed) {
        self.seal();
        // Stockpile totals use this country's own array count; independent countries
        // can have different counts. A complete contiguous prefix proves that array,
        // even when another country records more types.
        let Self {
            mut scanned,
            tracks,
            increases,
            refusals,
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
        let feed = Feed {
            increases,
            first_read: scanned.read_hours.first().copied(),
            refused: scanned.refused.clone(),
            tokens: scanned.tokens.clone(),
            types: scanned.types.clone(),
            refusals,
        };
        (scanned, feed)
    }
}

/// Every frame of `branch` observed in order, a block ending at the first frame it cannot read.
fn census(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Census> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Nukes,
        TableId::LexicalTokens,
    ];
    let mut observer = Census::default();
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
    Ok(observer)
}

pub fn scan(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<ScannedNukes> {
    Ok(census(replay, branch, cancelled)?.finish().0)
}

/// What the event feed reads of the stockpiles.
pub fn feed(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Feed> {
    Ok(census(replay, branch, cancelled)?.finish().1)
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

/// The whole reply from the recording's rows.
pub fn load(
    source: &Source,
    params: &NukesParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<CountryNukesReply> {
    let branch = params.branch_id.parse().expect("validated branch");
    let scanned = scanned_branch(source, branch, cancelled)?;
    let history = (params.origin_hour.is_some() || !params.tags.is_empty())
        .then(|| branch_histories(source, branch, false, &[], cancelled))
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
