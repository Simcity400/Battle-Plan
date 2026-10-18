//! Resource balance prepared block by block. An extract observes a block from
//! its keyframe alone and records the calls it makes after the block's first
//! hour. The fold, on the carried state, observes that first hour itself,
//! where continuation differs from a fresh start (runs and deals joined
//! across the boundary, countries closed, versions counted on), then makes the
//! recorded calls again under the carried keys and carries the country scan
//! forward. Assumptions the fast fold relies on are checked first; when one
//! does not hold, the fold observes the rest of the block hour by hour.

use super::super::{CountryScan, PoolRun, Scan, ScannedBalance, TradeRun, same_pools, same_terms};
use super::{Membership, Prepared, publish_membership};
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{Extract, Failure, Family, Folded, Held, HeldBytes},
    series::{self, Accumulator, Recording, ReplayTarget},
};
use replay::{container::BlockIndex, container::Replay};
use replay_data::{format::FormatVersion, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{any::Any, collections::BTreeMap, io, sync::Arc};

/// The tables resource balance reads.
pub(crate) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
    TableId::ResourceExchanges,
];

pub static FAMILY: Family = Family {
    name: "resources",
    extract: "resources.extract",
    sequential: "resources.sequential",
    fallback: "resources.fallback",
    tables: TABLES,
    fields: None,
    start,
};

fn start(_: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        prepared: Box::default(),
        recording: Recording::default(),
        first: None,
        names_stable: true,
    })
}

/// What a block's extract held right after the block's first hour.
struct First {
    /// The first hour, observed again by the fold on the carried state.
    frame: DecodedFrame,
    /// Keys the first hour's calls named.
    keys: usize,
    /// The resources named in the first hour.
    names: Option<Vec<(u32, String)>>,
    countries: BTreeMap<String, AfterFirst>,
}

/// A country as the extract held it after the block's first hour.
struct AfterFirst {
    trades: usize,
    unnamed: usize,
    unresolved: usize,
    /// The resources it had learned, once it had any names.
    learned: Option<Vec<(u32, String)>>,
}

/// One block's extract: the family observed from the block's keyframe with
/// no state from earlier blocks.
struct BlockExtract {
    prepared: Box<Prepared>,
    recording: Recording,
    first: Option<First>,
    /// The scan's resource names stayed those of the first hour.
    names_stable: bool,
}

/// A block's facts for [`Prepared::fold`].
struct BlockFacts {
    first: First,
    recording: Recording,
    /// The extract's keys by number.
    names: Vec<String>,
    memberships: Vec<Membership>,
    maxima: BTreeMap<String, i128>,
    scan: Scan,
    names_stable: bool,
}

fn names_of(prepared: &Prepared) -> Option<Vec<(u32, String)>> {
    prepared
        .scan
        .current_names
        .as_ref()
        .map(|names| names.list())
}

impl Held for AfterFirst {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.learned);
    }
}

impl Held for PoolRun {
    fn held(&self, bytes: &mut HeldBytes) {
        for raw in [
            &self.produced_raw,
            &self.subjects_raw,
            &self.imported_raw,
            &self.to_export_raw,
            &self.exported_raw,
            &self.available_raw,
            &self.after_projects_raw,
            &self.production_raw,
        ] {
            bytes.heap(raw);
        }
    }
}

impl Held for TradeRun {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.deal)
            .heap(&self.partner)
            .heap(&self.delivered_raw);
    }
}

impl Held for ScannedBalance {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.resources)
            .heap(&self.series)
            .heap(&self.deal_hours)
            .heap(&self.trades);
    }
}

impl Held for CountryScan {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.result)
            .heap(&self.learned)
            .heap(&self.open_trades)
            .heap(&self.pool_keys)
            .heap(&self.dirty_pools)
            .heap(&self.dirty_deals)
            .heap(&self.pool_ids);
    }
}

/// What the facts keep of a scan ([`Scan::forget_runtime`]).
impl Held for Scan {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.selected)
            .heap(&self.countries)
            .heap(&self.active);
    }
}

/// A membership recorded for the fold: its country and members.
impl Held for Membership {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.name).heap(&self.members);
    }
}

impl Extract for BlockExtract {
    /// What the block's facts hold: its first frame and what the scan held
    /// after it, its recorded calls, the memberships recorded for the fold,
    /// its keys by name, its maxima and its scan at the block's end (every
    /// country's pool runs and trades).
    fn bytes(&self) -> u64 {
        let mut held = HeldBytes::default();
        if let Some(first) = &self.first {
            held.frame(&first.frame)
                .heap(&first.names)
                .heap(&first.countries);
        }
        let prepared = &self.prepared;
        // The keys by number, as the facts name them.
        held.add(prepared.keys.len() * size_of::<String>())
            .add(prepared.keys.keys().map(String::capacity).sum());
        held.heap(&self.recording)
            .heap(&prepared.recorded)
            .heap(&prepared.maxima)
            .value(&prepared.scan)
            .total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        if self.first.is_none() {
            self.prepared.observe_changed(frame, changes)?;
            let countries = self
                .prepared
                .scan
                .countries
                .iter()
                .map(|(tag, country)| {
                    (
                        tag.clone(),
                        AfterFirst {
                            trades: country.result.trades.len(),
                            unnamed: country.result.unnamed,
                            unresolved: country.result.unresolved,
                            learned: country.names_ready.then(|| country.learned.list()),
                        },
                    )
                })
                .collect();
            self.first = Some(First {
                frame: frame.clone(),
                keys: self.prepared.keys.len(),
                names: names_of(&self.prepared),
                countries,
            });
            self.prepared.record_calls();
            return Ok(());
        }
        let prepared = &mut self.prepared;
        series::with_recording(&mut self.recording, || {
            prepared.observe_changed(frame, changes)
        })?;
        if self.names_stable
            && (changes.rows_changed[TableId::ResourceDefinitions.index()]
                || changes.rows_changed[TableId::LexicalTokens.index()])
        {
            let names = names_of(&self.prepared);
            self.names_stable = self
                .first
                .as_ref()
                .is_some_and(|first| first.names == names);
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        let BlockExtract {
            prepared,
            recording,
            first,
            names_stable,
        } = *self;
        let first = first.ok_or_else(|| io::Error::other("Resource block has no first hour"))?;
        let prepared = *prepared;
        let names = series::catalogue_names(&prepared.keys)?;
        let mut scan = prepared.scan;
        scan.forget_runtime();
        Ok(Box::new(BlockFacts {
            first,
            recording,
            names,
            memberships: prepared.recorded.unwrap_or_default(),
            maxima: prepared.maxima,
            scan,
            names_stable,
        }))
    }
}

/// The carried accumulators and keys the extract's recorded calls are made on.
struct Target<'a> {
    prepared: &'a mut Prepared,
    keys: &'a [u64],
    memberships: Vec<Option<Membership>>,
}

impl ReplayTarget for Target<'_> {
    fn series(&mut self, series: u8) -> &mut Accumulator {
        self.prepared.series(series)
    }
    fn key(&self, _: u8, key: u64) -> u64 {
        self.keys[key as usize]
    }
    fn family(&mut self, index: usize) -> io::Result<()> {
        let mut membership = self
            .memberships
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| io::Error::other("Resource fold has no recorded publication"))?;
        membership.id = self.keys[membership.id as usize];
        for member in &mut membership.members {
            *member = self.keys[*member as usize];
        }
        membership.members.sort_unstable();
        publish_membership(
            &mut self.prepared.memberships,
            &mut self.prepared.deals,
            &mut self.prepared.known,
            membership,
        )
    }
}

#[cfg(test)]
thread_local! {
    /// Fold every block fast, without checking its assumptions.
    static UNCHECKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Fold without making the recorded calls again, as a broken fold would.
    static SKIP_REPLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Recorded calls folds made again on this thread.
    static REPLAYED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `run` with folds that drop their recorded calls.
#[cfg(any(test, feature = "test-support"))]
pub fn with_skipped_replay<T>(run: impl FnOnce() -> T) -> T {
    let was = SKIP_REPLAY.with(|skip| skip.replace(true));
    let result = run();
    SKIP_REPLAY.with(|skip| skip.set(was));
    result
}

/// Recorded calls folds made again on this thread so far.
#[cfg(any(test, feature = "test-support"))]
pub fn replayed() -> usize {
    REPLAYED.with(std::cell::Cell::get)
}

impl Scan {
    /// Drop what a scan rebuilds from its next frame, as a restored scan has
    /// none of it.
    fn forget_runtime(&mut self) {
        let mut forgotten = Scan {
            selected: self.selected.take(),
            countries: std::mem::take(&mut self.countries),
            frontier: self.frontier,
            active: std::mem::take(&mut self.active),
            ..Scan::default()
        };
        #[cfg(test)]
        {
            forgotten.row_visits = self.row_visits;
            forgotten.history_visits = self.history_visits;
        }
        for country in forgotten.countries.values_mut() {
            country.dirty_pools.clear();
            country.dirty_deals = None;
        }
        *self = forgotten;
    }
}

impl Prepared {
    /// The accumulator recorded as `series`.
    fn series(&mut self, series: u8) -> &mut Accumulator {
        match series {
            0 => &mut self.pools,
            1 => &mut self.deals,
            2 => &mut self.known,
            values => &mut self.values[usize::from(values - 3)],
        }
    }

    /// From here on, record every accumulator call and deal publication.
    fn record_calls(&mut self) {
        for series in 0..14 {
            self.series(series).record_as(series);
        }
        self.recorded = Some(Vec::new());
    }

    /// Fold one block's facts into the carried state, after the block's
    /// ordinary observation and before its publication. `inject` sees each
    /// of the family's own observations (test refusals).
    pub fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        replay: &Replay,
        index: &BlockIndex,
        inject: &dyn Fn(i32, io::Result<()>) -> io::Result<()>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> Result<Folded, Failure> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let facts = {
            let _cpu = ::replay_frame::cpu::scope("fold facts", None);
            facts.downcast::<BlockFacts>().map_err(|_| Failure {
                hour: index.first_hour,
                error: io::Error::other("Pre-pass facts of another family"),
            })?
        };
        let BlockFacts {
            first,
            recording,
            names,
            memberships,
            maxima,
            scan,
            names_stable,
        } = *facts;
        let hour = first.frame.hour;
        // The first hour, where continuation differs from a fresh start.
        let changes = FrameChanges::alone(&first.frame, replay.format);
        inject(hour, {
            let _cpu = ::replay_frame::cpu::scope("fold observe", None);
            self.observe_changed(&first.frame, &changes)
        })
        .map_err(|error| Failure { hour, error })?;
        #[cfg(test)]
        let unchecked = UNCHECKED.with(std::cell::Cell::get);
        #[cfg(not(test))]
        let unchecked = false;
        if !unchecked && !self.continues(&first, &scan, &names, names_stable) {
            self.observe_after_first(replay, index, hour, inject, check)?;
            return Ok(Folded::Fallback);
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            if SKIP_REPLAY.with(std::cell::Cell::get) {
                return Ok(Folded::Fast);
            }
            REPLAYED.with(|replayed| replayed.set(replayed.get() + recording.len()));
        }
        let failed = |error: io::Error| Failure {
            hour: index.last_hour,
            error: io::Error::other(format!("Resource block fold failed: {error}")),
        };
        let keys = series::number_names(&mut self.keys, &names);
        recording
            .replay(&mut Target {
                prepared: self,
                keys: &keys,
                memberships: memberships.into_iter().map(Some).collect(),
            })
            .map_err(failed)?;
        {
            let _cpu = ::replay_frame::cpu::scope("fold observe", None);
            for (name, maximum) in maxima {
                let carried = self.maxima.entry(name).or_default();
                *carried = (*carried).max(maximum);
            }
            self.carry(&first, scan).map_err(failed)?;
        }
        Ok(Folded::Fast)
    }

    /// Whether the carried state, after observing the block's first hour,
    /// continues as the extract did from it: the same resource names learned,
    /// every run, deal and known span the extract opened then continuing one
    /// observed, every country the extract met later closed, and every key
    /// the first hour named carried.
    fn continues(&self, first: &First, block: &Scan, names: &[String], names_stable: bool) -> bool {
        let hour = first.frame.hour;
        if !names[..first.keys.min(names.len())]
            .iter()
            .all(|name| self.keys.contains_key(name))
        {
            return false;
        }
        for (tag, extracted) in &block.countries {
            let carried = self.scan.countries.get(tag);
            let after = first.countries.get(tag);
            match (after, carried) {
                (Some(after), Some(carried)) => {
                    if let Some(learned) = &after.learned
                        && (!carried.names_ready || carried.learned.list() != *learned)
                    {
                        return false;
                    }
                    if carried.open_trades.len() != after.trades
                        || extracted.result.trades.len() < after.trades
                    {
                        return false;
                    }
                    for trade in &extracted.result.trades[..after.trades] {
                        let continued = trade
                            .deal
                            .parse::<u64>()
                            .ok()
                            .and_then(|deal| carried.open_trades.get(&deal))
                            .and_then(|&at| carried.result.trades.get(at))
                            .is_some_and(|open| open.last_hour == hour && same_terms(open, trade));
                        if !continued {
                            return false;
                        }
                    }
                    for (resource, runs) in &extracted.result.series {
                        if let Some(run) = runs.first().filter(|run| run.first_hour == hour) {
                            let continued = carried
                                .result
                                .series
                                .get(resource)
                                .and_then(|runs| runs.last())
                                .is_some_and(|open| {
                                    open.last_hour == hour && same_pools(open, run)
                                });
                            if !continued {
                                return false;
                            }
                        }
                    }
                    if extracted
                        .result
                        .deal_hours
                        .first()
                        .is_some_and(|span| span.first_hour == hour)
                        && !carried
                            .result
                            .deal_hours
                            .last()
                            .is_some_and(|span| span.last_hour == hour)
                    {
                        return false;
                    }
                }
                // The first hour observes every country with a row; the
                // carried state observes at least those.
                (Some(_), None) => return false,
                (None, Some(carried)) => {
                    if carried.pools_open
                        || carried.deals_open
                        || !carried.open_trades.is_empty()
                        || !carried.pool_keys.is_empty()
                        || carried.diagnostic_rate != [0; 3]
                    {
                        return false;
                    }
                }
                (None, None) => {}
            }
            // A country named only after the first hour learns the names the
            // carried country holds only when they are those names.
            if extracted.names_ready
                && after.is_none_or(|after| after.learned.is_none())
                && let Some(carried) = carried.filter(|carried| carried.names_ready)
                && (!names_stable || first.names.as_ref() != Some(&carried.learned.list()))
            {
                return false;
            }
        }
        true
    }

    /// Carry the scan forward from the extract's end of block: what the
    /// extract opened in the first hour keeps the carried first hours,
    /// positions and counts it continued, and resources, countries and
    /// counts the block never touched stay as carried.
    fn carry(&mut self, first: &First, block: Scan) -> io::Result<()> {
        let hour = first.frame.hour;
        let mut block = block;
        for (tag, extracted) in std::mem::take(&mut block.countries) {
            let after = first.countries.get(&tag);
            let carried = self.scan.countries.entry(tag).or_default();
            carry_country(carried, extracted, after, hour)?;
        }
        self.scan.frontier = block.frontier;
        self.scan.active = block.active;
        self.scan.forget_runtime();
        Ok(())
    }

    /// Observe the block again hour by hour after its first hour, which the
    /// fold already observed, with the block's own change facts.
    fn observe_after_first(
        &mut self,
        replay: &Replay,
        index: &BlockIndex,
        first: i32,
        inject: &dyn Fn(i32, io::Result<()>) -> io::Result<()>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> Result<(), Failure> {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        let mut tracker = FrameChangeTracker::new();
        let mut failure = None;
        let read = replay.read_block_alone(
            index,
            TABLES,
            |raw, frame| {
                let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                if frame.hour == first {
                    return Ok(());
                }
                if let Err(error) = inject(frame.hour, {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    self.observe_changed(frame, &changes)
                }) {
                    failure = Some(Failure {
                        hour: frame.hour,
                        error,
                    });
                    return Err(io::Error::other("resource observation stopped"));
                }
                Ok(())
            },
            check,
        );
        if let Some(failure) = failure {
            return Err(failure);
        }
        read.map_err(|error| Failure { hour: first, error })
    }
}

fn carry_country(
    carried: &mut CountryScan,
    extracted: CountryScan,
    after: Option<&AfterFirst>,
    hour: i32,
) -> io::Result<()> {
    let CountryScan {
        result,
        learned,
        names_ready,
        track,
        pools_open,
        deals_open,
        open_trades,
        diagnostic_rate,
        materialized,
        pool_keys,
        ..
    } = extracted;
    let ScannedBalance {
        resources,
        series,
        deal_hours,
        mut trades,
        unnamed,
        unresolved,
    } = result;
    let observed_first = after.is_some();
    for (resource, mut runs) in series {
        if observed_first
            && let Some(run) = runs.first_mut().filter(|run| run.first_hour == hour)
            && let Some(open) = carried
                .result
                .series
                .get(&resource)
                .and_then(|runs| runs.last())
        {
            run.first_hour = open.first_hour;
            run.starts_segment = open.starts_segment;
        }
        carried.result.series.insert(resource, runs);
    }
    if !deal_hours.is_empty() {
        let mut deal_hours = deal_hours;
        if observed_first
            && let Some(span) = deal_hours
                .first_mut()
                .filter(|span| span.first_hour == hour)
            && let Some(open) = carried.result.deal_hours.last()
        {
            span.first_hour = open.first_hour;
        }
        carried.result.deal_hours = deal_hours;
    }
    // The first hour's trades are the carried open ones it continued.
    let first_trades = after.map_or(0, |after| after.trades);
    let later = trades.split_off(first_trades.min(trades.len()));
    let base = carried.result.trades.len();
    let mut moved = Vec::with_capacity(trades.len());
    for trade in trades {
        let at = trade
            .deal
            .parse::<u64>()
            .ok()
            .and_then(|deal| carried.open_trades.get(&deal).copied())
            .ok_or_else(|| io::Error::other("first-hour deal is not carried open"))?;
        carried.result.trades[at].last_hour = trade.last_hour;
        moved.push(at);
    }
    carried.result.trades.extend(later);
    carried.open_trades = open_trades
        .into_iter()
        .map(|(deal, at)| {
            (
                deal,
                match moved.get(at) {
                    Some(&position) => position,
                    None => base + at - first_trades,
                },
            )
        })
        .collect();
    carried.result.unnamed += unnamed - after.map_or(0, |after| after.unnamed);
    carried.result.unresolved += unresolved - after.map_or(0, |after| after.unresolved);
    if names_ready {
        carried.learned = learned;
        carried.names_ready = true;
        carried.result.resources = resources;
    }
    carried.track = track;
    carried.pools_open = pools_open;
    carried.deals_open = deals_open;
    carried.diagnostic_rate = diagnostic_rate;
    carried.materialized = materialized;
    carried.pool_keys = pool_keys;
    carried.dirty_pools.clear();
    carried.dirty_deals = None;
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
mod tests;
