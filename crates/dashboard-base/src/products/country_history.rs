//! Shared country-row identity and ambiguity handling for branch histories.

use super::country_groups::HourSpan;
use super::country_measure::Measure;
use crate::metadata::{CountryKeys, IdentityTrack, link_slot};
use replay_data::{rows::CountryLinkRow, rows::CountryRow};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    Source,
    metadata::{TaggedCountry, tagged_countries, tagged_countries_indexed, validate_campaign},
};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::tables};
use replay_frame::{
    frame::DecodedFrame, frame::FrameDecoder, frame::SharedView, frame::ViewIdentity,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

mod native;
#[cfg(any(test, feature = "test-support"))]
pub use native::Names as NativeNames;
pub mod prepared;

/// A native identity, kept independently of a measure's availability or rank.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryIdentityRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: u32,
    pub country_id: u32,
}

/// Exact native amounts stay numeric until a product builds its reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: i64,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericBalanceDrop {
    pub hour: i32,
    pub before: i64,
    pub after: i64,
}

/// Recorded political and command power are signed fixed5 balances.
/// A net drop keeps both exact stored amounts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceHistory {
    pub runs: Vec<NumericRun>,
    pub drops: Vec<NumericBalanceDrop>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecordedCountry {
    pub identities: Arc<Vec<CountryIdentityRun>>,
    pub segments: Arc<Vec<HourSpan>>,
    pub political_power: Arc<BalanceHistory>,
    pub command_power: Arc<BalanceHistory>,
    pub territory: Arc<Vec<NumericRun>>,
    last_identity: IdentityTrack,
}

impl RecordedCountry {
    /// A country recorded with only these identities.
    #[cfg(any(test, feature = "test-support"))]
    pub fn identified(identities: Vec<CountryIdentityRun>) -> Self {
        Self {
            identities: Arc::new(identities),
            ..Default::default()
        }
    }

    fn observe(
        &mut self,
        row: &CountryRow,
        slot: u32,
        hour: i32,
        measures: &[Measure],
        territory: Option<Option<i64>>,
    ) -> bool {
        let contiguous = observe_identity(
            Arc::make_mut(&mut self.segments),
            &mut self.last_identity,
            (slot, row.id),
            hour,
        );
        if contiguous {
            Arc::make_mut(&mut self.identities)
                .last_mut()
                .expect("identity")
                .last_hour = hour;
        } else {
            Arc::make_mut(&mut self.identities).push(CountryIdentityRun {
                first_hour: hour,
                last_hour: hour,
                slot,
                country_id: row.id,
            });
        }
        if measures.contains(&Measure::PoliticalPower) {
            append_balance(
                Arc::make_mut(&mut self.political_power),
                row.political_power_raw,
                hour,
                contiguous,
            );
        }
        if measures.contains(&Measure::CommandPower) {
            append_balance(
                Arc::make_mut(&mut self.command_power),
                Some(row.command_power_raw),
                hour,
                contiguous,
            );
        }
        if let Some(amount) = territory {
            append_fixed(Arc::make_mut(&mut self.territory), amount, hour, contiguous);
        }
        contiguous
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecordedHistories {
    pub countries: BTreeMap<String, RecordedCountry>,
    /// Complete native identity reads, usable as proof that a holder is absent.
    pub identity_reads: Vec<HourSpan>,
    pub unreadable_territory: usize,
    pub unresolved_countries: usize,
    pub unnamed: usize,
    measures: BTreeSet<Measure>,
    territory_loaded: bool,
}

impl RecordedHistories {
    pub fn bytes(&self) -> usize {
        use crate::prepared::prepass::HeldBytes;
        let mut bytes = HeldBytes::default();
        bytes
            .add(std::mem::size_of::<Self>())
            .tree::<String, RecordedCountry>(self.countries.len());
        bytes
            .heap(&self.identity_reads)
            .tree::<Measure, ()>(self.measures.len());
        for (tag, country) in &self.countries {
            bytes.heap(tag);
            for (pointer, capacity, size) in [
                (
                    Arc::as_ptr(&country.identities) as *const (),
                    country.identities.capacity(),
                    std::mem::size_of::<CountryIdentityRun>(),
                ),
                (
                    Arc::as_ptr(&country.segments) as *const (),
                    country.segments.capacity(),
                    std::mem::size_of::<HourSpan>(),
                ),
                (
                    Arc::as_ptr(&country.territory) as *const (),
                    country.territory.capacity(),
                    std::mem::size_of::<NumericRun>(),
                ),
            ] {
                bytes.shared(
                    pointer,
                    2 * std::mem::size_of::<usize>()
                        + std::mem::size_of::<Vec<()>>()
                        + capacity * size,
                );
            }
            for balance in [&country.political_power, &country.command_power] {
                bytes.shared(
                    Arc::as_ptr(balance),
                    2 * std::mem::size_of::<usize>()
                        + std::mem::size_of::<BalanceHistory>()
                        + balance.runs.capacity() * std::mem::size_of::<NumericRun>()
                        + balance.drops.capacity() * std::mem::size_of::<NumericBalanceDrop>(),
                );
            }
        }
        bytes.total() as usize
    }
}

/// Decode and seal the published identity page once per page and clipping bound.
/// The retained result belongs to the current query's bounded observation entry.
pub(crate) fn read_prepared_histories(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    through: Option<i32>,
) -> io::Result<Option<Arc<RecordedHistories>>> {
    let Some(file) = store.read(key, &format!("country-identities-{branch}"), 0)? else {
        return Ok(None);
    };
    let pages = vec![file.id().cache_key()];
    let history = crate::prepared::series::reuse_value(
        pages.clone(),
        &None::<i32>,
        RecordedHistories::bytes,
        || {
            let scan: HistoryScan = serde_json::from_reader(io::BufReader::new(file))?;
            Ok(scan.finish())
        },
    )?;
    match through {
        None => Ok(Some(history)),
        Some(last) => {
            crate::prepared::series::reuse_value(pages, &through, RecordedHistories::bytes, || {
                Ok((*history).clone().through(last))
            })
            .map(Some)
        }
    }
}

#[derive(Default)]
struct HistoriesCache {
    entries: BTreeMap<u64, Arc<RecordedHistories>>,
    /// One walk of a branch at a time, outside the cache lock: a request never
    /// waits on another branch's walk.
    gates: BTreeMap<u64, Arc<Mutex<()>>>,
}

/// Countries and country links supply identity and only requested balance fields
/// in one shared walk. The optional owned-state roster count excludes empty
/// state templates. No player, AI or land-holding status narrows this inventory.
pub fn branch_histories(
    source: &Source,
    branch: u64,
    territory: bool,
    measures: &[Measure],
    cancelled: &AtomicBool,
) -> io::Result<Arc<RecordedHistories>> {
    let state = source.state::<HistoriesCache>();
    let lock = || state.lock().unwrap_or_else(|error| error.into_inner());
    let covers = |found: &RecordedHistories| {
        (found.territory_loaded || !territory)
            && measures
                .iter()
                .all(|measure| found.measures.contains(measure))
    };
    let gate = {
        let mut cache = lock();
        if let Some(found) = cache.entries.get(&branch).filter(|found| covers(found)) {
            return Ok(Arc::clone(found));
        }
        Arc::clone(cache.gates.entry(branch).or_default())
    };
    // A failed or cancelled walk publishes nothing and frees the next request.
    let _walk = gate.lock().unwrap_or_else(|error| error.into_inner());
    let base = lock().entries.get(&branch).cloned();
    if let Some(found) = base.as_ref().filter(|found| covers(found)) {
        return Ok(Arc::clone(found));
    }
    let found = if let Some(base) = base {
        // Cloning the branch index shares all field histories. An upgrade only
        // writes newly requested empty fields; existing histories never copy.
        let mut found = (*base).clone();
        let missing: Vec<_> = measures
            .iter()
            .copied()
            .filter(|measure| !found.measures.contains(measure))
            .collect();
        if !missing.is_empty() {
            add_measures(&source.replay(), branch, &mut found, &missing, cancelled)?;
        }
        if territory && !found.territory_loaded {
            add_territory(&source.replay(), branch, &mut found, cancelled)?;
        }
        Arc::new(found)
    } else {
        Arc::new(scan_histories(
            &source.replay(),
            branch,
            territory,
            measures,
            cancelled,
        )?)
    };
    lock().entries.insert(branch, Arc::clone(&found));
    Ok(found)
}

/// Keep another walk's histories of the same frames for later requests. Histories
/// already kept for the branch are returned instead.
pub fn share(
    source: &Source,
    branch: u64,
    found: Arc<RecordedHistories>,
) -> Arc<RecordedHistories> {
    let state = source.state::<HistoriesCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    Arc::clone(cache.entries.entry(branch).or_insert(found))
}

/// Add only newly requested balance fields. Existing field histories and their
/// identities are preserved; every later request shares this one walk.
fn add_measures(
    replay: &Replay,
    branch: u64,
    out: &mut RecordedHistories,
    measures: &[Measure],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let selected = [TableId::Campaign, TableId::Countries, TableId::CountryLinks];
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Balance history cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let (Ok(countries), Ok(links)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                    ) else {
                        continue;
                    };
                    for (tag, lookup) in tagged_countries(countries, links) {
                        let TaggedCountry::Found(row, slot) = lookup else {
                            continue;
                        };
                        let Some(found) = std::str::from_utf8(tag)
                            .ok()
                            .and_then(|tag| out.countries.get_mut(tag))
                        else {
                            continue;
                        };
                        let index = found
                            .identities
                            .partition_point(|identity| identity.first_hour <= frame.hour);
                        let Some(identity) = index
                            .checked_sub(1)
                            .and_then(|index| found.identities.get(index))
                            .filter(|identity| {
                                identity.last_hour >= frame.hour
                                    && identity.slot == slot
                                    && identity.country_id == row.id
                            })
                        else {
                            continue;
                        };
                        let contiguous = frame.hour > identity.first_hour;
                        if measures.contains(&Measure::PoliticalPower) {
                            append_balance(
                                Arc::make_mut(&mut found.political_power),
                                row.political_power_raw,
                                frame.hour,
                                contiguous,
                            );
                        }
                        if measures.contains(&Measure::CommandPower) {
                            append_balance(
                                Arc::make_mut(&mut found.command_power),
                                Some(row.command_power_raw),
                                frame.hour,
                                contiguous,
                            );
                        }
                    }
                }
            }
            Ok(())
        },
    )?;
    out.measures.extend(measures.iter().copied());
    Ok(())
}

/// Upgrade an already cached country history with the requested roster count.
/// Only the current frame's tagged countries are joined; historical countries
/// absent this hour are not visited. Existing balance histories remain shared.
fn add_territory(
    replay: &Replay,
    branch: u64,
    out: &mut RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryCounterInputs,
    ];
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "State count history cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let (Ok(countries), Ok(links), Ok(rows)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::CountryCounterInputs>(),
                    ) else {
                        continue;
                    };
                    let rows = unique_by_slot(rows, links, |row| row.country);
                    out.unreadable_territory += rows.unresolved_rows + rows.duplicate_rows;
                    for (tag, lookup) in tagged_countries(countries, links) {
                        let TaggedCountry::Found(row, slot) = lookup else {
                            continue;
                        };
                        let Some(country) = std::str::from_utf8(tag)
                            .ok()
                            .and_then(|tag| out.countries.get_mut(tag))
                        else {
                            continue;
                        };
                        let index = country
                            .identities
                            .partition_point(|identity| identity.first_hour <= frame.hour);
                        let Some(identity) = index
                            .checked_sub(1)
                            .and_then(|index| country.identities.get(index))
                            .filter(|identity| {
                                identity.last_hour >= frame.hour
                                    && identity.slot == slot
                                    && identity.country_id == row.id
                            })
                        else {
                            continue;
                        };
                        let amount = rows
                            .rows
                            .get(&identity.slot)
                            .and_then(|row| *row)
                            .map(|row| i64::from(row.owned_state_count));
                        append_fixed(
                            Arc::make_mut(&mut country.territory),
                            amount,
                            frame.hour,
                            frame.hour > identity.first_hour,
                        );
                    }
                }
            }
            Ok(())
        },
    )?;
    out.territory_loaded = true;
    Ok(())
}

/// Explicit picks follow their recorded native identities through a tag rename.
/// Tags remain opaque; no tag spelling or player status establishes an alias.
pub fn selected_identities(history: &RecordedHistories, tags: &[String]) -> BTreeSet<(u32, u32)> {
    tags.iter()
        .filter_map(|tag| history.countries.get(tag))
        .flat_map(|country| country.identities.iter())
        .map(|identity| (identity.slot, identity.country_id))
        .collect()
}

pub fn selected_aliases(history: &RecordedHistories, tags: &[String]) -> Vec<String> {
    let selected: BTreeSet<_> = tags.iter().map(String::as_str).collect();
    let identities = selected_identities(history, tags);
    history
        .countries
        .iter()
        .filter(|(tag, country)| {
            selected.contains(tag.as_str())
                || country
                    .identities
                    .iter()
                    .any(|identity| identities.contains(&(identity.slot, identity.country_id)))
        })
        .map(|(tag, _)| tag.clone())
        .collect()
}

/// Each of `tags`' aliases ([`selected_aliases`]) with the hours one of the selected native
/// identities holds it, cut to the caller's `within` spans of that tag where it gives them: an
/// alias's other holders are not the selection.
pub fn selected_spans(
    history: &RecordedHistories,
    tags: &[String],
    within: &BTreeMap<String, Vec<HourSpan>>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<HourSpan>>> {
    let selected = selected_identities(history, tags);
    let mut spans = BTreeMap::new();
    for tag in selected_aliases(history, tags) {
        let native: Vec<_> = history
            .countries
            .get(&tag)
            .into_iter()
            .flat_map(|country| country.identities.iter())
            .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
            .map(|identity| HourSpan {
                first_hour: identity.first_hour,
                last_hour: identity.last_hour,
            })
            .collect();
        let scopes = match within.get(&tag) {
            Some(bounds) => {
                let mut scopes = Vec::new();
                crate::ranking::for_each_piece(
                    &native,
                    bounds,
                    |span| (span.first_hour, span.last_hour),
                    cancelled,
                    |_, first_hour, last_hour| {
                        scopes.push(HourSpan {
                            first_hour,
                            last_hour,
                        });
                        Ok(())
                    },
                )?;
                scopes
            }
            None => native,
        };
        spans.insert(tag, scopes);
    }
    Ok(spans)
}

pub fn scan_histories(
    replay: &Replay,
    branch: u64,
    territory: bool,
    measures: &[Measure],
    cancelled: &AtomicBool,
) -> io::Result<RecordedHistories> {
    let mut selected = vec![TableId::Campaign, TableId::Countries, TableId::CountryLinks];
    if territory {
        selected.push(TableId::CountryCounterInputs);
    }
    let mut scan = HistoryScan::new(territory, measures);
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Country history cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    scan.observe(decoded);
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.finish())
}

/// A branch walk's histories, fed every frame that decodes and passes the
/// campaign check, so another scan of the same frames can build them too.
#[derive(Default, Serialize, Deserialize)]
pub struct HistoryScan {
    witnesses: native::History,
    hour: Option<i32>,
    open: BTreeSet<usize>,
    pending_hours: usize,
    rates: [usize; 3],
    #[serde(skip)]
    counts: Option<BTreeMap<u32, Option<i64>>>,
    #[serde(skip)]
    counts_ready: bool,
    #[serde(skip)]
    count_refusals: usize,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
    out: RecordedHistories,
    measures: Vec<Measure>,
    // The iterator feeds supply's persistent actor registration order.
    names: BTreeMap<String, usize>,
    countries: Vec<RecordedCountry>,
    #[serde(skip)]
    read: Option<CountryRead>,
}

/// What `tagged_countries` resolves from one set of country keys and links. The
/// same keys and links allocation resolve the same rows, so later hours reuse it.
struct CountryRead {
    keys: CountryKeys,
    links: ViewIdentity,
    slots: Arc<HashMap<u32, u32>>,
    /// Each named holder's history, row position and slot.
    holders: Vec<(usize, usize, u32, u32, String)>,
    holders_by_slot: HashMap<u32, Vec<usize>>,
    /// Every row has a named identity: proof that an unlisted holder is absent.
    complete: bool,
    unresolved: usize,
    unnamed: usize,
}

impl CountryRead {
    fn new(
        countries: &[CountryRow],
        slots: Arc<HashMap<u32, u32>>,
        view: &SharedView,
        names: &mut BTreeMap<String, usize>,
        histories: &mut Vec<RecordedCountry>,
    ) -> Self {
        let tagged = tagged_countries_indexed(countries, &slots);
        let mut read = Self {
            keys: CountryKeys::read(countries),
            links: view.identity(),
            slots,
            holders: Vec::new(),
            holders_by_slot: HashMap::new(),
            complete: false,
            unresolved: tagged
                .values()
                .filter(|lookup| !matches!(lookup, TaggedCountry::Found(..)))
                .count(),
            unnamed: 0,
        };
        let mut known = HashSet::new();
        for (position, row) in countries.iter().enumerate() {
            let Some(&TaggedCountry::Found(found, slot)) = tagged.get(row.tag.as_slice()) else {
                continue;
            };
            if !std::ptr::eq(found, row) {
                continue;
            }
            let Ok(tag) = std::str::from_utf8(&row.tag) else {
                read.unnamed += 1;
                continue;
            };
            if !tag.is_empty() {
                known.insert((slot, row.id));
            }
            let history = *names.entry(tag.to_owned()).or_insert_with(|| {
                histories.push(RecordedCountry::default());
                histories.len() - 1
            });
            read.holders_by_slot
                .entry(slot)
                .or_default()
                .push(read.holders.len());
            read.holders
                .push((history, position, slot, row.id, tag.to_owned()));
        }
        read.complete = countries
            .iter()
            .all(|row| known.contains(&(row.canonical_slot, row.id)));
        read
    }
}

impl HistoryScan {
    pub fn new(territory: bool, measures: &[Measure]) -> Self {
        Self {
            out: RecordedHistories {
                measures: measures.iter().copied().collect(),
                territory_loaded: territory,
                ..Default::default()
            },
            measures: measures.to_vec(),
            names: BTreeMap::new(),
            countries: Vec::new(),
            read: None,
            witnesses: Default::default(),
            ..Default::default()
        }
    }

    pub fn native_tag(
        &self,
        hour: i32,
        country: replay_data::event_feed_rows::EventFeedCountry,
    ) -> Option<String> {
        self.witnesses.tag(hour, country)
    }

    /// Canonical slot of a captured scope reference at the exact occurrence hour.
    /// A missing fresh identity read remains unavailable, including aliases.
    pub fn native_scope_slot(
        &self,
        hour: i32,
        country: replay_data::event_feed_rows::EventFeedCountry,
    ) -> Option<u32> {
        self.witnesses.scope_slot(hour, country)
    }

    pub fn observe(&mut self, frame: &DecodedFrame) {
        self.observe_with_counts(frame, false, None, |_, _, _| {});
    }

    /// Publish the native count from the identity join without retaining its history here.
    pub fn observe_owned_states(
        &mut self,
        frame: &DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
        observed: impl FnMut(&str, Option<i64>, bool),
    ) {
        self.observe_with_counts(frame, true, changes, observed);
    }

    /// [`Self::observe_owned_states`] of an hour at which no country key, link, counter row
    /// key, gain key or table availability changed, made without its rows when it can be:
    /// false, with nothing changed, when it cannot.
    pub fn advance_unchanged(&mut self, hour: i32, witnessed: bool) -> bool {
        self.advance_quiet(hour, hour, witnessed)
    }

    fn account(&mut self) {
        self.out.unreadable_territory += self.rates[0] * self.pending_hours;
        self.out.unresolved_countries += self.rates[1] * self.pending_hours;
        self.out.unnamed += self.rates[2] * self.pending_hours;
        self.pending_hours = 0;
    }

    fn seal_country(&mut self, history: usize, hour: i32) {
        let country = &mut self.countries[history];
        let before = country.identities.last().map(|identity| identity.last_hour);
        country.last_identity.advance(hour);
        if let Some(run) = Arc::make_mut(&mut country.identities).last_mut() {
            run.last_hour = hour;
        }
        if let Some(span) = Arc::make_mut(&mut country.segments).last_mut() {
            span.last_hour = hour;
        }
        if self.out.territory_loaded
            && let Some(run) = Arc::make_mut(&mut country.territory).last_mut()
            && Some(run.last_hour) == before
        {
            run.last_hour = hour;
        }
    }

    fn observe_with_counts(
        &mut self,
        frame: &DecodedFrame,
        owned_states: bool,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
        mut observed: impl FnMut(&str, Option<i64>, bool),
    ) {
        let _cpu = replay_frame::cpu::scope("identities-read", None);
        self.witnesses.observe(frame, changes);
        let contiguous = self.hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let previous_hour = self.hour;
        let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            self.account();
            let open = match previous_hour {
                Some(hour) => {
                    let open = std::mem::take(&mut self.open);
                    for &history in &open {
                        self.seal_country(history, hour);
                    }
                    open
                }
                None => BTreeSet::new(),
            };
            if let Some(read) = &self.read {
                for (_, _, _, _, tag) in &read.holders {
                    if !tag.is_empty() {
                        observed(tag, None, false);
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.emitted += 1;
                        }
                    }
                }
            } else {
                // A restored scan keeps no join; its open histories are the
                // holders that join last named.
                for (tag, history) in &self.names {
                    if open.contains(history) && !tag.is_empty() {
                        observed(tag, None, false);
                        #[cfg(test)]
                        {
                            self.work.emitted += 1;
                        }
                    }
                }
            }
            self.hour = None;
            self.read = None;
            self.counts_ready = false;
            return;
        };
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let touched = |table: TableId| {
            changes.is_none_or(|changes| {
                changes.rows_changed[table.index()] || changes.table_read[table.index()]
            })
        };
        let view = &frame.tables[TableId::CountryLinks.index()];
        let country_delta = changes.and_then(|changes| changes.rows(TableId::Countries));
        #[cfg(any(test, feature = "test-support"))]
        if touched(TableId::Countries) && country_delta.is_none() {
            self.work.row_visits += countries.len();
        }
        let same = self.read.as_ref().is_some_and(|read| {
            read.links.is(view)
                && (!touched(TableId::Countries) || {
                    #[cfg(any(test, feature = "test-support"))]
                    let before = read.keys.row_visits();
                    let same = read.keys.same_changes(countries, country_delta);
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        if country_delta.is_some() {
                            self.work.row_visits += read.keys.row_visits() - before;
                        }
                    }
                    same
                })
        });
        let slots = self
            .read
            .as_ref()
            .filter(|read| read.links.is(view))
            .map(|read| Arc::clone(&read.slots))
            .unwrap_or_else(|| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += links.len();
                }
                Arc::new(crate::metadata::link_slots(links))
            });
        let refresh_counts = !self.counts_ready
            || changed(TableId::CountryCounterInputs)
            || changed(TableId::CountryLinks);
        let old_counts = if refresh_counts {
            self.counts.take()
        } else {
            None
        };
        if refresh_counts {
            self.account();
            self.count_refusals = 0;
            self.counts = if self.out.territory_loaded || owned_states {
                frame
                    .complete::<tables::CountryCounterInputs>()
                    .ok()
                    .map(|rows| {
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.row_visits += rows.len();
                        }
                        let joined = unique_by_slots(rows, &slots, |row| row.country);
                        self.count_refusals = joined.unresolved_rows + joined.duplicate_rows;
                        joined
                            .rows
                            .into_iter()
                            .map(|(slot, row)| {
                                (slot, row.map(|row| i64::from(row.owned_state_count)))
                            })
                            .collect()
                    })
            } else {
                None
            };
            self.counts_ready = true;
        }
        let old_read = if !same {
            self.account();
            self.read.replace(CountryRead::new(
                countries,
                slots,
                view,
                &mut self.names,
                &mut self.countries,
            ))
        } else {
            None
        };
        let read = self.read.as_ref().expect("country read");
        self.rates = [self.count_refusals, read.unresolved, read.unnamed];
        self.pending_hours += 1;
        if read.complete {
            super::country_groups::mark_unobserved(
                &mut self.out.identity_reads,
                frame.hour,
                frame.hour,
            );
        }
        // The join is runtime-only. Its old/new native holders and amounts are
        // the candidates; no retained country histories are scanned here.
        if changes.is_some() && same && !refresh_counts && contiguous && self.measures.is_empty() {
            self.hour = Some(frame.hour);
            return;
        }
        let holders = if changes.is_some() && same && contiguous && self.measures.is_empty() {
            // A changed counter table may contain only a few countries. Resolve
            // its changed slots through the current join, without cloning or
            // checking every unchanged country holder.
            let counts = self.counts.as_ref();
            let before = old_counts.as_ref();
            let positions: BTreeSet<_> = counts
                .into_iter()
                .flat_map(|counts| counts.keys())
                .chain(before.into_iter().flat_map(|counts| counts.keys()))
                .filter(|slot| {
                    counts
                        .and_then(|counts| counts.get(*slot))
                        .copied()
                        .flatten()
                        != before
                            .and_then(|counts| counts.get(*slot))
                            .copied()
                            .flatten()
                })
                .flat_map(|slot| {
                    read.holders_by_slot
                        .get(slot)
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect();
            positions
                .into_iter()
                .map(|position| read.holders[position].clone())
                .collect()
        } else {
            read.holders.clone()
        };
        let previous: BTreeMap<_, _> = old_read
            .as_ref()
            .into_iter()
            .flat_map(|read| &read.holders)
            .map(|(history, _, slot, id, _)| (*history, (*slot, *id)))
            .collect();
        if let Some(old) = &old_read {
            let present: BTreeSet<_> = holders.iter().map(|(history, ..)| *history).collect();
            for (history, _, _, _, tag) in &old.holders {
                if !present.contains(history) {
                    if let Some(hour) = previous_hour {
                        self.seal_country(*history, hour);
                    }
                    self.open.remove(history);
                    if !tag.is_empty() {
                        observed(tag, None, false);
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.emitted += 1;
                        }
                    }
                }
            }
        }
        if changes.is_some() && !same && old_read.is_none() {
            let present: BTreeSet<_> = holders.iter().map(|(history, ..)| *history).collect();
            let removed: Vec<_> = self
                .names
                .iter()
                .filter(|(_, history)| self.open.contains(*history) && !present.contains(*history))
                .map(|(tag, history)| (tag.clone(), *history))
                .collect();
            for (tag, history) in removed {
                if let Some(hour) = previous_hour {
                    self.seal_country(history, hour);
                }
                self.open.remove(&history);
                if !tag.is_empty() {
                    observed(&tag, None, false);
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.emitted += 1;
                    }
                }
            }
        }
        for (history, position, slot, id, tag) in holders {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.history_visits += 1;
            }
            let amount = self
                .counts
                .as_ref()
                .and_then(|counts| counts.get(&slot))
                .copied()
                .flatten();
            let before = if refresh_counts {
                old_counts
                    .as_ref()
                    .and_then(|counts| counts.get(&slot))
                    .copied()
                    .flatten()
            } else {
                amount
            };
            let identity_changed =
                !contiguous || !same && previous.get(&history) != Some(&(slot, id));
            let count_changed = refresh_counts && before != amount;
            if changes.is_some() && !identity_changed && !count_changed && self.measures.is_empty()
            {
                continue;
            }
            if changes.is_some()
                && self.open.contains(&history)
                && let Some(hour) = previous_hour
            {
                self.seal_country(history, hour);
            }
            let contiguous = self.countries[history].observe(
                &countries[position],
                slot,
                frame.hour,
                &self.measures,
                self.out.territory_loaded.then_some(amount),
            );
            self.open.insert(history);
            if !tag.is_empty() && (changes.is_none() || identity_changed || count_changed) {
                observed(&tag, amount, !contiguous);
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.emitted += 1;
                }
            }
        }
        self.hour = Some(frame.hour);
    }

    pub fn seal(&mut self) {
        self.account();
        self.witnesses.seal();
        if let Some(hour) = self.hour {
            let open: Vec<_> = self.open.iter().copied().collect();
            for history in open {
                self.seal_country(history, hour);
            }
        }
    }

    pub fn finish(mut self) -> RecordedHistories {
        self.seal();
        self.out.countries = self
            .names
            .into_iter()
            .map(|(tag, history)| (tag, std::mem::take(&mut self.countries[history])))
            .collect();
        self.out
    }

    /// The histories through `last_hour`, as a walk sealed there holds them,
    /// for a reader whose own coverage ends there while this scan went
    /// further: runs, segments and reads end there, and tags first held
    /// later are left out. Counts of unreadable rows remain every observed
    /// hour's.
    #[cfg(test)]
    fn finish_through(self, last_hour: i32) -> RecordedHistories {
        self.finish().through(last_hour)
    }
}

impl RecordedHistories {
    fn through(mut self, last_hour: i32) -> Self {
        fn clip<T>(runs: &mut Vec<T>, last_hour: i32, span: impl Fn(&mut T) -> (i32, &mut i32)) {
            runs.retain_mut(|run| span(run).0 <= last_hour);
            if let Some(run) = runs.last_mut() {
                let (_, last) = span(run);
                *last = (*last).min(last_hour);
            }
        }
        let out = &mut self;
        clip(&mut out.identity_reads, last_hour, |span| {
            (span.first_hour, &mut span.last_hour)
        });
        out.countries.retain(|_, country| {
            if country
                .identities
                .last()
                .is_some_and(|run| run.last_hour > last_hour)
            {
                clip(Arc::make_mut(&mut country.identities), last_hour, |run| {
                    (run.first_hour, &mut run.last_hour)
                });
            }
            if country
                .segments
                .last()
                .is_some_and(|span| span.last_hour > last_hour)
            {
                clip(Arc::make_mut(&mut country.segments), last_hour, |span| {
                    (span.first_hour, &mut span.last_hour)
                });
            }
            if country
                .territory
                .last()
                .is_some_and(|run| run.last_hour > last_hour)
            {
                clip(Arc::make_mut(&mut country.territory), last_hour, |run| {
                    (run.first_hour, &mut run.last_hour)
                });
            }
            for balance in [&mut country.political_power, &mut country.command_power] {
                if balance
                    .runs
                    .last()
                    .is_none_or(|run| run.last_hour <= last_hour)
                    && balance
                        .drops
                        .last()
                        .is_none_or(|drop| drop.hour <= last_hour)
                {
                    continue;
                }
                let balance = Arc::make_mut(balance);
                clip(&mut balance.runs, last_hour, |run| {
                    (run.first_hour, &mut run.last_hour)
                });
                balance.drops.retain(|drop| drop.hour <= last_hour);
            }
            !country.identities.is_empty()
        });
        self
    }
}

impl HistoryScan {
    /// What an unchanged hour's observation depends on besides the hour's
    /// read evidence: two scans with equal inputs advance alike over the same
    /// quiet hours ([`Self::advance_quiet`]).
    pub(crate) fn quiet_inputs(&self) -> QuietInputs {
        QuietInputs {
            hour: self.hour,
            joined: self.read.is_some(),
            witnesses: self.witnesses.quiet_inputs(),
        }
    }

    /// Whether the next hour reading every witness table needs its rows,
    /// because the held native names are not the last hour's.
    pub(crate) fn stale_witnesses(&self) -> bool {
        self.witnesses.stale()
    }

    /// Hours `first..=last`, following the last hour observed, at which no
    /// country key, link, counter row key, gain key or table availability
    /// changed, and which read all three witness tables at every hour
    /// (`witnessed`) or not all three at any: each hour's observation of
    /// owned states, made without its rows. Counter values moving there
    /// change no retained history; the holders they would visit are sealed
    /// at the same hours by the next observation or seal. False, with
    /// nothing changed, when the scan cannot step without rows: it holds
    /// balances or territory, it did not observe the hour before `first`, or
    /// a read hour would name stale witnesses.
    pub(crate) fn advance_quiet(&mut self, first: i32, last: i32, witnessed: bool) -> bool {
        if !self.measures.is_empty() || self.out.territory_loaded || first > last {
            return false;
        }
        let joined = self.read.as_ref().map(|read| read.complete);
        if joined.is_some() && self.hour.and_then(|hour| hour.checked_add(1)) != Some(first) {
            return false;
        }
        if witnessed && self.witnesses.stale() {
            return false;
        }
        if !self.witnesses.advance_quiet(first, last, witnessed) {
            return false;
        }
        // Without complete countries and links each hour only closes what
        // the hour observed closed; with them, each continues the same join.
        if let Some(complete) = joined {
            let Ok(hours) = usize::try_from(i64::from(last) - i64::from(first) + 1) else {
                return true;
            };
            self.pending_hours += hours;
            if complete {
                super::country_groups::mark_unobserved(&mut self.out.identity_reads, first, last);
            }
            self.hour = Some(last);
        }
        true
    }
}

/// What a [`HistoryScan`]'s unchanged hours depend on besides their read
/// evidence ([`HistoryScan::quiet_inputs`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QuietInputs {
    hour: Option<i32>,
    joined: bool,
    witnesses: (Option<i32>, bool, bool),
}

/// The prepared identity history's query product: the country identities'
/// own coverage and refusals answer for every reader of the history.
pub const IDENTITIES_PRODUCT: &str = "country-identities";

/// The shared identity-only walk, available once the identity history covers
/// the whole branch: the identities' own coverage when they are prepared on
/// their own, ahead of the hourly walk, or the branch's. A refused history
/// answers with its refusal, never with its earlier pages. Missing
/// preparation remains pending; it never starts a recording scan.
pub fn prepared_branch_histories(
    source: &Source,
    branch: u64,
    content: Option<&super::ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Arc<RecordedHistories>>> {
    crate::metadata::check_cancel(cancelled)?;
    crate::prepared::coverage::refusal(source, content, branch, IDENTITIES_PRODUCT, cancelled)?;
    // Complete for this recording: through the branch's last recorded hour,
    // not a page completed for a shorter recording of the same store.
    let end = {
        let replay = source.replay_ref();
        replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.last_hour)
            .chain(
                replay
                    .gaps
                    .iter()
                    .filter(|gap| gap.branch == branch)
                    .map(|gap| gap.last_hour),
            )
            .max()
    };
    let reaches = |coverage: Option<crate::prepared::coverage::Coverage>| {
        coverage.is_some_and(|coverage| {
            coverage.complete && end.is_none_or(|end| coverage.last_hour >= end)
        })
    };
    // A store the hourly walk published alone covers the identities with
    // the branch.
    let complete = reaches(crate::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        IDENTITIES_PRODUCT,
        cancelled,
    )?) || reaches(crate::prepared::coverage::coverage(
        source, content, branch, cancelled,
    )?);
    if !complete {
        return Ok(None);
    }
    let (store, key) = crate::prepared::files(source, content)?;
    let history = read_prepared_histories(&store, &key, branch, None)?;
    crate::metadata::check_cancel(cancelled)?;
    Ok(history)
}

fn append_fixed(
    runs: &mut Vec<NumericRun>,
    amount: Option<i64>,
    hour: i32,
    identity_contiguous: bool,
) {
    let Some(amount) = amount else {
        return;
    };
    let contiguous = identity_contiguous
        && runs
            .last()
            .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour));
    match runs.last_mut() {
        Some(run) if contiguous && run.value == amount => run.last_hour = hour,
        _ => {
            runs.push(NumericRun {
                first_hour: hour,
                last_hour: hour,
                value: amount,
                starts_segment: !contiguous,
            });
        }
    }
}

fn append_balance(series: &mut BalanceHistory, amount: Option<i64>, hour: i32, contiguous: bool) {
    if contiguous
        && let (Some(before), Some(after)) = (
            series
                .runs
                .last()
                .filter(|run| run.last_hour.checked_add(1) == Some(hour))
                .map(|run| run.value),
            amount,
        )
        && after < before
    {
        series.drops.push(NumericBalanceDrop {
            hour,
            before,
            after,
        });
    }
    append_fixed(&mut series.runs, amount, hour, contiguous);
}

/// Canonical country slot and native country id.
pub type Identity = (u32, u32);

pub struct SlotRows<'a, T> {
    pub rows: HashMap<u32, Option<&'a T>>,
    pub unresolved_rows: usize,
    /// Rows after the first row naming the same slot.
    pub duplicate_rows: usize,
    /// First rows that also become unknown when their slot is repeated.
    pub ambiguous_slots: usize,
}

/// No row wins when several raw links resolve to the same country slot.
pub fn unique_by_slot<'a, T>(
    rows: impl IntoIterator<Item = &'a T>,
    links: &[CountryLinkRow],
    country: impl Fn(&T) -> u32,
) -> SlotRows<'a, T> {
    unique_by(rows, |row| link_slot(links, country(row)))
}

/// The same native alias/duplicate rule with an already resolved current join.
pub(crate) fn unique_by_slots<'a, T>(
    rows: impl IntoIterator<Item = &'a T>,
    slots: &HashMap<u32, u32>,
    country: impl Fn(&T) -> u32,
) -> SlotRows<'a, T> {
    unique_by(rows, |row| slots.get(&country(row)).copied())
}

fn unique_by<'a, T>(
    rows: impl IntoIterator<Item = &'a T>,
    slot: impl Fn(&T) -> Option<u32>,
) -> SlotRows<'a, T> {
    let mut result = SlotRows {
        rows: HashMap::new(),
        unresolved_rows: 0,
        duplicate_rows: 0,
        ambiguous_slots: 0,
    };
    for row in rows {
        let Some(slot) = slot(row) else {
            result.unresolved_rows += 1;
            continue;
        };
        result
            .rows
            .entry(slot)
            .and_modify(|seen| {
                result.duplicate_rows += 1;
                result.ambiguous_slots += usize::from(seen.is_some());
                *seen = None;
            })
            .or_insert(Some(row));
    }
    result
}

/// Return whether this hour extends the preceding identity segment. Missing
/// hours and a different native country under the tag always start a new one.
pub fn observe_identity(
    segments: &mut Vec<HourSpan>,
    last_identity: &mut IdentityTrack,
    identity: Identity,
    hour: i32,
) -> bool {
    let contiguous = last_identity.step(Some(identity), hour);
    if contiguous {
        segments.last_mut().expect("identity segment").last_hour = hour;
    } else {
        segments.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
    contiguous
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_spans_keep_an_alias_only_while_a_selected_country_holds_it() {
        let held = |runs: &[(i32, i32, u32, u32)]| RecordedCountry {
            identities: Arc::new(
                runs.iter()
                    .map(|&(first_hour, last_hour, slot, country_id)| CountryIdentityRun {
                        first_hour,
                        last_hour,
                        slot,
                        country_id,
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        let mut history = RecordedHistories::default();
        // A renames to B at 10; another country takes tag B from 30.
        history
            .countries
            .insert("A".into(), held(&[(0, 9, 7, 100)]));
        history
            .countries
            .insert("B".into(), held(&[(10, 19, 7, 100), (30, 40, 8, 200)]));
        let span = |first_hour, last_hour| HourSpan {
            first_hour,
            last_hour,
        };
        let never = AtomicBool::new(false);
        let tags = ["A".to_owned()];
        assert_eq!(
            selected_spans(&history, &tags, &BTreeMap::new(), &never).unwrap(),
            BTreeMap::from([
                ("A".to_owned(), vec![span(0, 9)]),
                ("B".to_owned(), vec![span(10, 19)]),
            ])
        );
        // The caller's spans cut them further.
        let within = BTreeMap::from([("B".to_owned(), vec![span(15, 35)])]);
        assert_eq!(
            selected_spans(&history, &tags, &within, &never).unwrap()["B"],
            [span(15, 19)]
        );
    }

    #[test]
    fn published_identity_projection_reuses_the_full_page_across_clipping_bounds() {
        let fixture = crate::prepared::series::tests::Fixture::new();
        let mut scan = HistoryScan::new(false, &[]);
        for hour in 0..=6 {
            scan.observe(&crate::test_fixtures::scale_frame(1, hour, 3));
        }
        let encoded = serde_json::to_vec(&scan).unwrap();
        let mut writer = fixture.writer();
        writer
            .write_at("country-identities-1", 0, encoded.as_slice())
            .unwrap();
        writer.commit().unwrap();
        let scope = crate::prepared::series::cache_scope(None);
        let whole = read_prepared_histories(&fixture.store, &fixture.key, 1, None)
            .unwrap()
            .unwrap();
        let held = scope.finish();
        for last in [1, 3, 5] {
            let scope = crate::prepared::series::cache_scope(Some(&held));
            let clipped = read_prepared_histories(&fixture.store, &fixture.key, 1, Some(last))
                .unwrap()
                .unwrap();
            let again = read_prepared_histories(&fixture.store, &fixture.key, 1, None)
                .unwrap()
                .unwrap();
            assert!(
                Arc::ptr_eq(&whole, &again),
                "the full history was decoded again"
            );
            let expected: HistoryScan = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(
                serde_json::to_vec(&*clipped).unwrap(),
                serde_json::to_vec(&expected.finish_through(last)).unwrap()
            );
            assert!(scope.finish().bytes() >= whole.bytes());
        }
        scan.observe(&crate::test_fixtures::scale_frame(1, 7, 3));
        let mut writer = fixture.writer();
        let replacement = serde_json::to_vec(&scan).unwrap();
        writer
            .write_at("country-identities-1", 0, replacement.as_slice())
            .unwrap();
        writer.commit().unwrap();
        let scope = crate::prepared::series::cache_scope(Some(&held));
        let changed = read_prepared_histories(&fixture.store, &fixture.key, 1, None)
            .unwrap()
            .unwrap();
        assert!(!Arc::ptr_eq(&whole, &changed));
        assert_eq!(
            serde_json::to_vec(&*changed).unwrap(),
            serde_json::to_vec(&scan.finish()).unwrap()
        );
        drop(scope);
    }
    use crate::test_fixtures::{CAMPAIGN, records, source};
    use replay_data::{rows::CountryCounterInputRow, rows::TableRows};
    use replay_frame::{frame::DecodedFrame, frame::TableView};

    fn opened(branches: &[(u64, Vec<DecodedFrame>)]) -> Source {
        let records: Vec<_> = branches
            .iter()
            .map(|(branch, frames)| records(*branch, frames.clone()))
            .collect();
        Source::from_replay(
            crate::SourceKey {
                source_id: "country-history".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(Replay::from_records(CAMPAIGN.into(), &records).unwrap()),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn territory_frame(hour: i32, tag: &[u8], country_id: u32, amount: i32) -> DecodedFrame {
        let mut frame = source(1, hour, Some(i64::from(amount)));
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].id = country_id;
            rows[0].tag = tag.to_vec();
        }
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: country_id,
                canonical_slot: 7,
            }]));
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: country_id,
                    owned_state_count: amount,
                    ..Default::default()
                },
            ]));
        frame
    }

    #[test]
    fn country_history_shared_changes_keep_skipped_spans_resume_append_and_counter_proofs_exact() {
        use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut prepared = HistoryScan::new(true, &[]);
        let mut oracle = HistoryScan::new(true, &[]);
        let mut tracker = FrameChangeTracker::new();
        for (commit, (first, last)) in [(100, 112), (113, 124), (126, 139)].into_iter().enumerate()
        {
            let mut frame = territory_frame(first, b"original", 9001, 7);
            if commit == 2 {
                frame = territory_frame(first, b"renamed", 9011, -2);
            }
            for hour in first..=last {
                frame.hour = hour;
                if hour == 118 {
                    *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
                        reason: "counter unavailable".into(),
                    };
                }
                if hour == 121 {
                    frame = territory_frame(hour, b"original", 9001, 3);
                }
                prepared.observe_owned_states(
                    &frame,
                    Some(&tracker.observe(&frame, format)),
                    |_, _, _| {},
                );
                oracle.observe_owned_states(&frame, None, |_, _, _| {});
            }
            prepared.seal();
            oracle.seal();
            let snapshot: HistoryScan =
                serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            let reference: HistoryScan =
                serde_json::from_slice(&serde_json::to_vec(&oracle).unwrap()).unwrap();
            observer_tests::assert_reply_parity(&snapshot.finish(), &reference.finish()).unwrap();
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            tracker.invalidate();
        }
    }

    #[test]
    fn country_history_shared_driver_is_flat_over_unchanged_hours_and_retained_aliases() {
        use crate::prepared::{
            frame_changes::FrameChangeTracker,
            observer_tests::{self, Work},
        };
        let format = replay_data::format::FormatVersion::CURRENT;
        for history in [1, 100] {
            let work = observer_tests::scaled_work(10, |hours| {
                let mut prepared = HistoryScan::new(true, &[]);
                let mut tracker = FrameChangeTracker::new();
                let mut frame = territory_frame(100, b"initial", 9001, 7);
                for index in 0..history {
                    frame =
                        territory_frame(100 + index, format!("alias {index}").as_bytes(), 9001, 7);
                    prepared.observe_owned_states(
                        &frame,
                        Some(&tracker.observe(&frame, format)),
                        |_, _, _| {},
                    );
                }
                let delta = observer_tests::unchanged_frames(
                    prepared.work,
                    &frame,
                    hours as i32,
                    format,
                    |frame, changes| {
                        prepared.observe_owned_states(frame, Some(changes), |_, _, _| {});
                        Ok(prepared.work)
                    },
                )?;
                assert_eq!(delta, Work::default());
                Ok(delta)
            })
            .unwrap();
            observer_tests::assert_flat(work);
        }
    }

    #[test]
    fn country_history_one_changed_counter_emits_one_key_without_visiting_closed_aliases() {
        use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let format = replay_data::format::FormatVersion::CURRENT;
        let work = observer_tests::scaled_work(10, |history| {
            let mut prepared = HistoryScan::new(true, &[]);
            let mut oracle = HistoryScan::new(true, &[]);
            let mut tracker = FrameChangeTracker::new();
            let mut frame = territory_frame(100, b"initial", 9001, 7);
            for index in 0..history {
                frame = territory_frame(
                    100 + index as i32,
                    format!("alias {index}").as_bytes(),
                    9001,
                    7,
                );
                prepared.observe_owned_states(
                    &frame,
                    Some(&tracker.observe(&frame, format)),
                    |_, _, _| {},
                );
                oracle.observe(&frame);
            }
            let slots = Arc::clone(&prepared.read.as_ref().unwrap().slots);
            let before = prepared.work;
            frame.hour += 1;
            if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
                &mut *frame.tables[TableId::CountryCounterInputs.index()]
            {
                rows[0].owned_state_count = 9;
            }
            prepared.observe_owned_states(
                &frame,
                Some(&tracker.observe(&frame, format)),
                |_, _, _| {},
            );
            oracle.observe(&frame);
            assert!(Arc::ptr_eq(&slots, &prepared.read.as_ref().unwrap().slots));
            assert_eq!(prepared.work.row_visits - before.row_visits, 1);
            assert_eq!(prepared.work.history_visits - before.history_visits, 1);
            assert_eq!(prepared.work.emitted - before.emitted, 1);
            let delta = observer_tests::Work {
                row_visits: prepared.work.row_visits - before.row_visits,
                history_visits: prepared.work.history_visits - before.history_visits,
                emitted: prepared.work.emitted - before.emitted,
                ..Default::default()
            };
            observer_tests::assert_reply_parity(&prepared.finish(), &oracle.finish())?;
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(work);
    }

    #[test]
    fn country_history_country_operand_refresh_reuses_link_map_and_rebuilds_after_resume() {
        use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut prepared = HistoryScan::new(true, &[]);
        let mut oracle = HistoryScan::new(true, &[]);
        let mut tracker = FrameChangeTracker::new();
        let mut frame = territory_frame(100, b"original", 9001, 7);
        prepared.observe_owned_states(&frame, Some(&tracker.observe(&frame, format)), |_, _, _| {});
        oracle.observe(&frame);
        let slots = Arc::clone(&prepared.read.as_ref().unwrap().slots);
        frame.hour = 101;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"recorded alias".to_vec();
        }
        let before = prepared.work;
        prepared.observe_owned_states(&frame, Some(&tracker.observe(&frame, format)), |_, _, _| {});
        oracle.observe(&frame);
        assert!(Arc::ptr_eq(&slots, &prepared.read.as_ref().unwrap().slots));
        assert_eq!(
            prepared.work.row_visits - before.row_visits,
            1,
            "only changed country operands are visited"
        );
        assert_eq!(
            prepared.work.emitted - before.emitted,
            2,
            "the old and new aliases are the only emissions"
        );
        prepared.seal();
        oracle.seal();
        let snapshot: HistoryScan =
            serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        let reference: HistoryScan =
            serde_json::from_slice(&serde_json::to_vec(&oracle).unwrap()).unwrap();
        observer_tests::assert_reply_parity(&snapshot.finish(), &reference.finish()).unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        tracker.invalidate();
        frame.hour = 102;
        prepared.observe_owned_states(&frame, Some(&tracker.observe(&frame, format)), |_, _, _| {});
        oracle.observe(&frame);
        let restored = Arc::clone(&prepared.read.as_ref().unwrap().slots);
        assert!(!Arc::ptr_eq(&slots, &restored));
        frame.hour = 103;
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 9002,
                canonical_slot: 7,
            });
        }
        prepared.observe_owned_states(&frame, Some(&tracker.observe(&frame, format)), |_, _, _| {});
        oracle.observe(&frame);
        assert!(!Arc::ptr_eq(
            &restored,
            &prepared.read.as_ref().unwrap().slots
        ));
        assert_eq!(prepared.read.as_ref().unwrap().slots.get(&9002), Some(&7));
        observer_tests::assert_reply_parity(&prepared.finish(), &oracle.finish()).unwrap();
    }

    #[test]
    fn country_history_sparse_counter_refresh_visits_only_its_current_holders() {
        use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut frame = crate::test_fixtures::scale_frame(1, 100, 64);
        frame.metrics = Default::default();
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 100_000,
                    owned_state_count: 7,
                    ..Default::default()
                },
            ]));
        let mut prepared = HistoryScan::new(true, &[]);
        let mut oracle = HistoryScan::new(true, &[]);
        let mut tracker = FrameChangeTracker::new();
        prepared.observe_owned_states(&frame, Some(&tracker.observe(&frame, format)), |_, _, _| {});
        oracle.observe(&frame);
        for hour in [101, 102, 104] {
            frame.hour = hour;
            if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
                &mut *frame.tables[TableId::CountryCounterInputs.index()]
            {
                if hour == 102 {
                    rows.clear();
                } else {
                    *rows = vec![CountryCounterInputRow {
                        country: 100_000,
                        owned_state_count: hour,
                        ..Default::default()
                    }];
                }
            }
            let before = prepared.work;
            prepared.observe_owned_states(
                &frame,
                Some(&tracker.observe(&frame, format)),
                |_, _, _| {},
            );
            oracle.observe(&frame);
            assert_eq!(
                prepared.work.row_visits - before.row_visits,
                usize::from(hour != 102)
            );
            assert_eq!(prepared.work.history_visits - before.history_visits, 1);
            assert_eq!(prepared.work.emitted - before.emitted, 1);
            prepared.seal();
            oracle.seal();
            let snapshot: HistoryScan =
                serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            let reference: HistoryScan =
                serde_json::from_slice(&serde_json::to_vec(&oracle).unwrap()).unwrap();
            observer_tests::assert_reply_parity(&snapshot.finish(), &reference.finish()).unwrap();
            if hour == 102 {
                prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
                tracker.invalidate();
                // Rebuild before measuring the next sparse refresh.
                frame.hour = 103;
                prepared.observe_owned_states(
                    &frame,
                    Some(&tracker.observe(&frame, format)),
                    |_, _, _| {},
                );
                oracle.observe(&frame);
            }
        }
    }

    #[test]
    fn country_history_shared_driver_keeps_all_native_owned_counters_without_hourly_work() {
        use crate::prepared::{
            frame_changes::FrameChangeTracker,
            observer_tests::{self, Work},
        };
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut frame = crate::test_fixtures::scale_frame(1, 100, 24);
        let counts = frame
            .complete::<tables::Countries>()
            .unwrap()
            .iter()
            .map(|row| CountryCounterInputRow {
                country: row.id,
                owned_state_count: row.canonical_slot as i32,
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(counts));
        let mut prepared = HistoryScan::new(true, &[]);
        let changes = FrameChangeTracker::new().observe(&frame, format);
        prepared.observe_owned_states(&frame, Some(&changes), |_, _, _| {});
        assert_eq!(prepared.open.len(), 24);
        let delta = observer_tests::unchanged_frames(
            prepared.work,
            &frame,
            100,
            format,
            |frame, changes| {
                prepared.observe_owned_states(frame, Some(changes), |_, _, _| {});
                Ok(prepared.work)
            },
        )
        .unwrap();
        assert_eq!(delta, Work::default());
        assert!(
            prepared
                .finish()
                .countries
                .values()
                .all(|country| country.territory[0].last_hour == 200)
        );
    }

    #[test]
    fn absence_proof_requires_every_native_holder_to_have_a_resolved_identity() {
        let mut unresolved = territory_frame(101, b"initial", 9001, 7);
        *unresolved.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![]));
        let opened = opened(&[(
            1,
            vec![
                territory_frame(100, b"initial", 9001, 7),
                unresolved,
                territory_frame(102, &[0xff], 9001, 7),
                territory_frame(103, b"new alias", 9001, 7),
            ],
        )]);
        let history = branch_histories(&opened, 1, false, &[], &AtomicBool::new(false)).unwrap();
        assert_eq!(
            history.identity_reads,
            [
                HourSpan {
                    first_hour: 100,
                    last_hour: 100
                },
                HourSpan {
                    first_hour: 103,
                    last_hour: 103
                },
            ]
        );
        let upgraded = branch_histories(&opened, 1, true, &[], &AtomicBool::new(false)).unwrap();
        assert_eq!(
            upgraded.identity_reads, history.identity_reads,
            "adding an amount never manufactures identity absence evidence"
        );
    }

    #[test]
    fn country_history_cache_upgrades_share_fields_and_retain_every_branch() {
        let opened = opened(&[
            (1, vec![territory_frame(100, b"initial", 9001, 7)]),
            (2, vec![source(2, 100, Some(2))]),
            (3, vec![source(3, 100, Some(3))]),
        ]);
        let cancelled = AtomicBool::new(false);
        let pp =
            branch_histories(&opened, 1, false, &[Measure::PoliticalPower], &cancelled).unwrap();
        let cp = branch_histories(&opened, 1, false, &[Measure::CommandPower], &cancelled).unwrap();
        let before = &pp.countries["initial"];
        let after = &cp.countries["initial"];
        assert!(Arc::ptr_eq(&before.identities, &after.identities));
        assert!(Arc::ptr_eq(&before.segments, &after.segments));
        assert!(Arc::ptr_eq(&before.political_power, &after.political_power));
        assert!(before.command_power.runs.is_empty());
        assert_eq!(after.command_power.runs[0].value, i64::MAX);
        let territory = branch_histories(&opened, 1, true, &[], &cancelled).unwrap();
        let with_territory = &territory.countries["initial"];
        assert!(Arc::ptr_eq(
            &after.political_power,
            &with_territory.political_power
        ));
        assert!(Arc::ptr_eq(
            &after.command_power,
            &with_territory.command_power
        ));
        assert!(Arc::ptr_eq(&after.identities, &with_territory.identities));
        assert_eq!(with_territory.territory[0].value, 7);
        for branch in [2, 3] {
            branch_histories(&opened, branch, false, &[], &cancelled).unwrap();
        }
        let again =
            branch_histories(&opened, 1, true, &[Measure::PoliticalPower], &cancelled).unwrap();
        assert!(Arc::ptr_eq(&territory, &again));
    }

    #[test]
    fn country_history_walk_of_one_branch_never_delays_another_branch() {
        let opened = opened(&[
            (1, vec![source(1, 100, Some(1))]),
            (2, vec![source(2, 100, Some(2))]),
        ]);
        let gate = Arc::clone(
            opened
                .state::<HistoriesCache>()
                .lock()
                .unwrap()
                .gates
                .entry(1)
                .or_default(),
        );
        let walking = gate.lock().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let other = branch_histories(&opened, 2, false, &[], &AtomicBool::new(false));
                sent.send(other.is_ok()).unwrap();
            });
            let answered = received.recv_timeout(std::time::Duration::from_secs(30));
            drop(walking);
            assert_eq!(
                answered,
                Ok(true),
                "branch 2 answers during branch 1's walk"
            );
        });
    }

    #[test]
    fn country_history_territory_upgrade_matches_initial_scan_across_identity_changes() {
        let mut absent = territory_frame(102, b"renamed", 9001, 9);
        *absent.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![]));
        let frames = vec![
            territory_frame(100, b"initial", 9001, 7),
            territory_frame(101, b"renamed", 9001, 8),
            absent,
            territory_frame(103, b"initial", 9002, 10),
        ];
        let opened = opened(&[(1, frames)]);
        let cancelled = AtomicBool::new(false);
        let base = branch_histories(&opened, 1, false, &[], &cancelled).unwrap();
        let upgraded = branch_histories(&opened, 1, true, &[], &cancelled).unwrap();
        let fresh = scan_histories(&opened.replay(), 1, true, &[], &cancelled).unwrap();
        for (tag, country) in &fresh.countries {
            assert_eq!(upgraded.countries[tag].territory, country.territory);
            assert!(Arc::ptr_eq(
                &base.countries[tag].identities,
                &upgraded.countries[tag].identities
            ));
        }
        assert_eq!(
            upgraded.countries["initial"]
                .territory
                .iter()
                .map(|run| (run.first_hour, run.last_hour, run.value, run.starts_segment))
                .collect::<Vec<_>>(),
            [(100, 100, 7, true), (103, 103, 10, true)]
        );
        assert_eq!(upgraded.countries["renamed"].territory[0].last_hour, 101);
        assert_eq!(
            selected_aliases(&upgraded, &["initial".into()]),
            ["initial", "renamed"]
        );
    }

    #[test]
    fn country_history_refuses_every_ambiguous_row_and_unresolved_link() {
        let links = [
            CountryLinkRow {
                id: 1,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 3,
                canonical_slot: 8,
            },
        ];
        let rows = [1, 2, 1, 3, 99];
        let result = unique_by_slot(&rows, &links, |country| *country);
        assert_eq!(result.rows[&7], None);
        assert_eq!(result.rows[&8], Some(&3));
        assert_eq!(result.unresolved_rows, 1);
        assert_eq!(result.duplicate_rows, 2);
        assert_eq!(result.ambiguous_slots, 1);
        let slots = crate::metadata::link_slots(&links);
        let resolved = unique_by_slots(&rows, &slots, |country| *country);
        assert_eq!(resolved.rows, result.rows);
        assert_eq!(resolved.unresolved_rows, result.unresolved_rows);
        assert_eq!(resolved.duplicate_rows, result.duplicate_rows);
        assert_eq!(resolved.ambiguous_slots, result.ambiguous_slots);
    }

    #[test]
    fn country_history_identity_segments_break_on_gaps_and_native_id_changes() {
        let mut segments = Vec::new();
        let mut last = IdentityTrack::default();
        assert!(!observe_identity(&mut segments, &mut last, (7, 1), 10));
        assert!(observe_identity(&mut segments, &mut last, (7, 1), 11));
        assert!(!observe_identity(&mut segments, &mut last, (7, 1), 13));
        assert!(!observe_identity(&mut segments, &mut last, (7, 2), 14));
        assert!(!observe_identity(&mut segments, &mut last, (8, 2), 15));
        assert_eq!(
            segments
                .iter()
                .map(|span| (span.first_hour, span.last_hour))
                .collect::<Vec<_>>(),
            [(10, 11), (13, 13), (14, 14), (15, 15)]
        );
        assert!(!last.step(None, 16));
        assert!(!observe_identity(&mut segments, &mut last, (8, 2), 17));
        assert!(observe_identity(&mut segments, &mut last, (8, 2), 18));
        assert_eq!(segments.last().unwrap().first_hour, 17);
        assert_eq!(segments.last().unwrap().last_hour, 18);
        assert!(!observe_identity(
            &mut segments,
            &mut last,
            (8, 2),
            i32::MAX
        ));
        assert!(!observe_identity(
            &mut segments,
            &mut last,
            (8, 2),
            i32::MIN
        ));
    }

    const WITNESSED: [TableId; 3] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperienceGains,
    ];

    /// `count` countries at hour 100 with their counter rows (and one whose
    /// country no link resolves) and gains, every witness table read.
    fn quiet_base(count: u32) -> DecodedFrame {
        use replay_data::rows::CountryExperienceGainRow;
        let mut frame = crate::test_fixtures::scale_frame(1, 100, count);
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                (0..count)
                    .map(|index| CountryCounterInputRow {
                        country: 100_000 + index,
                        ..Default::default()
                    })
                    .chain([CountryCounterInputRow {
                        country: 999_999,
                        ..Default::default()
                    }])
                    .collect(),
            ));
        *frame.tables[TableId::CountryExperienceGains.index()] =
            TableView::Complete(TableRows::CountryExperienceGains(
                (0..count)
                    .map(|index| CountryExperienceGainRow {
                        country: 100_000 + index,
                        instance: 7 + index,
                        ..Default::default()
                    })
                    .collect(),
            ));
        for table in WITNESSED {
            frame.metrics.table_ns[table.index()] = 1;
        }
        frame
    }

    /// `previous` one hour on, at `hour`: counter counts and gain amounts
    /// move, so both tables are rebuilt; nothing the history reads changes.
    /// The gains are not read at 104.
    fn quiet_hour(previous: &DecodedFrame, hour: i32) -> DecodedFrame {
        let mut frame = previous.clone();
        frame.hour = hour;
        if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        {
            for (index, row) in rows.iter_mut().enumerate() {
                row.owned_state_count = (hour + index as i32).rem_euclid(5);
            }
        }
        if let TableView::Complete(TableRows::CountryExperienceGains(rows)) =
            &mut *frame.tables[TableId::CountryExperienceGains.index()]
        {
            for row in rows {
                row.army_raw = i64::from(hour);
            }
        }
        frame.metrics.table_ns[TableId::CountryExperienceGains.index()] = u64::from(hour != 104);
        frame
    }

    /// Quiet hours stepped without their rows leave the history, its native
    /// witnesses and its unreadable-row counts exactly as observing each hour
    /// does, although counter counts move every hour and a gain read is
    /// missing in between.
    fn quiet_steps_equal_hourly_observation(count: u32) {
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
        let mut frame = quiet_base(count);
        let changes = tracker.observe(&frame, format);
        let mut hourly = HistoryScan::new(false, &[]);
        let mut quiet = HistoryScan::new(false, &[]);
        hourly.observe_owned_states(&frame, Some(&changes), |_, _, _| {});
        quiet.observe_owned_states(&frame, Some(&changes), |_, _, _| {});
        for hour in 101..=110 {
            frame = quiet_hour(&frame, hour);
            let changes = tracker.observe(&frame, format);
            assert!(changes.rows_changed[TableId::CountryCounterInputs.index()]);
            assert!(!changes.rows_changed[TableId::CountryLinks.index()]);
            hourly.observe_owned_states(&frame, Some(&changes), |_, _, _| {});
        }
        assert!(quiet.advance_quiet(101, 103, true));
        assert!(quiet.advance_quiet(104, 104, false));
        assert!(quiet.advance_quiet(105, 110, true));
        assert_eq!(quiet.quiet_inputs(), hourly.quiet_inputs());
        hourly.seal();
        quiet.seal();
        assert_eq!(
            serde_json::to_vec(&quiet).unwrap(),
            serde_json::to_vec(&hourly).unwrap()
        );
        assert!(hourly.out.unreadable_territory > 0);
        let actor = replay_data::event_feed_rows::EventFeedCountry {
            reference: 100_000,
            id: 100_000,
            instance: 7,
        };
        assert_eq!(quiet.native_tag(110, actor), hourly.native_tag(110, actor));
        assert!(quiet.native_tag(110, actor).is_some());
        assert!(quiet.native_tag(104, actor).is_none());
    }

    #[test]
    fn country_history_quiet_hours_step_as_observed_hours() {
        quiet_steps_equal_hourly_observation(3);
    }

    /// An hour that does not read the countries while their keys change
    /// leaves the held native names stale: the next read hour needs its rows
    /// and is not stepped, while unread hours still are.
    #[test]
    fn country_history_stale_witnesses_refuse_a_quiet_read_hour() {
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
        let base = quiet_base(3);
        let mut scan = HistoryScan::new(false, &[]);
        scan.observe_owned_states(&base, Some(&tracker.observe(&base, format)), |_, _, _| {});
        assert!(!scan.stale_witnesses());
        let mut renamed = quiet_hour(&base, 101);
        renamed.metrics.table_ns[TableId::Countries.index()] = 0;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *renamed.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"mod/renamed".to_vec();
        }
        scan.observe_owned_states(
            &renamed,
            Some(&tracker.observe(&renamed, format)),
            |_, _, _| {},
        );
        assert!(scan.stale_witnesses());
        let before = serde_json::to_vec(&scan).unwrap();
        let inputs = scan.quiet_inputs();
        assert!(!scan.advance_quiet(102, 102, true));
        assert_eq!(serde_json::to_vec(&scan).unwrap(), before);
        assert_eq!(scan.quiet_inputs(), inputs);
        assert!(scan.advance_quiet(102, 102, false));
        // A scan holding balances never steps without rows.
        let mut balances = HistoryScan::new(false, &[Measure::PoliticalPower]);
        balances.observe_owned_states(&base, None, |_, _, _| {});
        assert!(!balances.advance_quiet(101, 101, true));
    }

    /// Finished through an hour, a history that went further holds what one
    /// sealed at that hour holds: its runs, segments and complete reads end
    /// there, and tags first held later are absent.
    #[test]
    fn country_history_finished_through_an_hour_is_the_history_sealed_there() {
        let frames: Vec<_> = (0..=6)
            .map(|hour| {
                let mut frame = crate::test_fixtures::scale_frame(1, hour, 3);
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    if hour >= 2 {
                        rows[1].tag = b"mod/renamed".to_vec();
                    }
                    if hour >= 5 {
                        rows[2].id = 200_002;
                    }
                }
                if hour == 4 {
                    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                        reason: "fixture".into(),
                    };
                }
                frame
            })
            .collect();
        let mut whole = HistoryScan::new(false, &[]);
        for frame in &frames {
            whole.observe(frame);
        }
        let whole = serde_json::to_vec(&whole).unwrap();
        for through in 0..=6_i32 {
            let mut prefix = HistoryScan::new(false, &[]);
            for frame in &frames[..=through as usize] {
                prefix.observe(frame);
            }
            let sealed = prefix.finish();
            let clipped = serde_json::from_slice::<HistoryScan>(&whole)
                .unwrap()
                .finish_through(through);
            let runs = |history: &RecordedHistories| {
                history
                    .countries
                    .iter()
                    .map(|(tag, country)| {
                        (
                            tag.clone(),
                            (*country.identities).clone(),
                            (*country.segments).clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(runs(&clipped), runs(&sealed), "through {through}");
            assert_eq!(
                clipped.identity_reads, sealed.identity_reads,
                "through {through}"
            );
        }
    }
}
