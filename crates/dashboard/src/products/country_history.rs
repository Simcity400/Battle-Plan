//! Shared country-row identity and ambiguity handling for branch histories.

use super::country_groups::HourSpan;
use super::country_measure::Measure;
use crate::metadata::{CountryKeys, IdentityTrack, link_slot};
use replay::rows::{CountryLinkRow, CountryRow};
use std::collections::{HashMap, HashSet};

use crate::{
    Source,
    metadata::{TaggedCountry, tagged_countries, validate_campaign},
};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, SharedView, ViewIdentity},
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NumericRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value: i64,
    pub starts_segment: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NumericBalanceDrop {
    pub hour: i32,
    pub before: i64,
    pub after: i64,
}

/// Recorded political and command power are signed fixed5 balances.
/// A net drop keeps both exact stored amounts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BalanceHistory {
    pub runs: Vec<NumericRun>,
    pub drops: Vec<NumericBalanceDrop>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RecordedCountry {
    pub identities: Arc<Vec<CountryIdentityRun>>,
    pub segments: Arc<Vec<HourSpan>>,
    pub political_power: Arc<BalanceHistory>,
    pub command_power: Arc<BalanceHistory>,
    pub territory: Arc<Vec<NumericRun>>,
    last_identity: IdentityTrack,
}

impl RecordedCountry {
    fn observe(
        &mut self,
        row: &CountryRow,
        slot: u32,
        hour: i32,
        measures: &[Measure],
        territory: Option<Option<i64>>,
    ) {
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
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RecordedHistories {
    pub countries: BTreeMap<String, RecordedCountry>,
    /// Complete native identity reads, usable as proof that a holder is absent.
    pub identity_reads: Vec<HourSpan>,
    pub unreadable_territory: usize,
    pub unresolved_countries: usize,
    pub unnamed: usize,
    measures: BTreeSet<Measure>,
    territory_loaded: bool,
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
pub(crate) fn branch_histories(
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
pub(crate) fn share(
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
pub(crate) fn selected_identities(
    history: &RecordedHistories,
    tags: &[String],
) -> BTreeSet<(u32, u32)> {
    tags.iter()
        .filter_map(|tag| history.countries.get(tag))
        .flat_map(|country| country.identities.iter())
        .map(|identity| (identity.slot, identity.country_id))
        .collect()
}

pub(crate) fn selected_aliases(history: &RecordedHistories, tags: &[String]) -> Vec<String> {
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

pub(crate) fn scan_histories(
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
pub(crate) struct HistoryScan<'m> {
    out: RecordedHistories,
    measures: &'m [Measure],
    names: HashMap<String, usize>,
    countries: Vec<RecordedCountry>,
    read: Option<CountryRead>,
}

/// What `tagged_countries` resolves from one set of country keys and links. The
/// same keys and links allocation resolve the same rows, so later hours reuse it.
struct CountryRead {
    keys: CountryKeys,
    links: ViewIdentity,
    /// Each named holder's history, row position and slot.
    holders: Vec<(usize, usize, u32)>,
    /// Every row has a named identity: proof that an unlisted holder is absent.
    complete: bool,
    unresolved: usize,
    unnamed: usize,
}

impl CountryRead {
    fn new(
        countries: &[CountryRow],
        links: &[CountryLinkRow],
        view: &SharedView,
        names: &mut HashMap<String, usize>,
        histories: &mut Vec<RecordedCountry>,
    ) -> Self {
        let tagged = tagged_countries(countries, links);
        let mut read = Self {
            keys: CountryKeys::read(countries),
            links: view.identity(),
            holders: Vec::new(),
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
            read.holders.push((history, position, slot));
        }
        read.complete = countries
            .iter()
            .all(|row| known.contains(&(row.canonical_slot, row.id)));
        read
    }
}

impl<'m> HistoryScan<'m> {
    pub(crate) fn new(territory: bool, measures: &'m [Measure]) -> Self {
        Self {
            out: RecordedHistories {
                measures: measures.iter().copied().collect(),
                territory_loaded: territory,
                ..Default::default()
            },
            measures,
            names: HashMap::new(),
            countries: Vec::new(),
            read: None,
        }
    }

    pub(crate) fn observe(&mut self, frame: &DecodedFrame) {
        let (Ok(countries), Ok(links)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
        ) else {
            return;
        };
        let territory = self.out.territory_loaded;
        let counts = territory
            .then(|| {
                frame
                    .complete::<tables::CountryCounterInputs>()
                    .ok()
                    .map(|rows| unique_by_slot(rows, links, |row| row.country))
            })
            .flatten();
        if let Some(rows) = &counts {
            self.out.unreadable_territory += rows.unresolved_rows + rows.duplicate_rows;
        }
        // Hourly balances change Countries; its keys and the links decide the rows.
        let view = &frame.tables[TableId::CountryLinks.index()];
        if !self
            .read
            .as_ref()
            .is_some_and(|read| read.links.is(view) && read.keys.same(countries))
        {
            self.read = Some(CountryRead::new(
                countries,
                links,
                view,
                &mut self.names,
                &mut self.countries,
            ));
        }
        let read = self.read.as_ref().expect("country read");
        if read.complete {
            super::country_groups::mark_unobserved(
                &mut self.out.identity_reads,
                frame.hour,
                frame.hour,
            );
        }
        self.out.unresolved_countries += read.unresolved;
        self.out.unnamed += read.unnamed;
        for &(history, position, slot) in &read.holders {
            let amount = counts
                .as_ref()
                .and_then(|counts| counts.rows.get(&slot))
                .and_then(|row| *row)
                .map(|row| i64::from(row.owned_state_count));
            self.countries[history].observe(
                &countries[position],
                slot,
                frame.hour,
                self.measures,
                territory.then_some(amount),
            );
        }
    }

    pub(crate) fn finish(mut self) -> RecordedHistories {
        self.out.countries = self
            .names
            .into_iter()
            .map(|(tag, history)| (tag, std::mem::take(&mut self.countries[history])))
            .collect();
        self.out
    }
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
pub(crate) type Identity = (u32, u32);

pub(crate) struct SlotRows<'a, T> {
    pub rows: HashMap<u32, Option<&'a T>>,
    pub unresolved_rows: usize,
    /// Rows after the first row naming the same slot.
    pub duplicate_rows: usize,
    /// First rows that also become unknown when their slot is repeated.
    pub ambiguous_slots: usize,
}

/// No row wins when several raw links resolve to the same country slot.
pub(crate) fn unique_by_slot<'a, T>(
    rows: impl IntoIterator<Item = &'a T>,
    links: &[CountryLinkRow],
    country: impl Fn(&T) -> u32,
) -> SlotRows<'a, T> {
    let mut result = SlotRows {
        rows: HashMap::new(),
        unresolved_rows: 0,
        duplicate_rows: 0,
        ambiguous_slots: 0,
    };
    for row in rows {
        let Some(slot) = link_slot(links, country(row)) else {
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
pub(crate) fn observe_identity(
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
    use crate::test_fixtures::{CAMPAIGN, records, source};
    use replay::frame::{DecodedFrame, TableView};
    use replay::rows::{CountryCounterInputRow, TableRows};

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
}
