use crate::{
    politics_changes::{self as hooks, Drained, Event, Kind},
    table_capture::grow_capacity,
};
use engine_allocation as allocation;
use engine_events::politics_sites::BalanceSnapshot;
use engine_native_memory::walk::Outcome;
use replay_data::{BranchId, rows::*};
use replay_frame::{frame::TableInput, frame::TableView};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::Arc,
};

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::CountryPowerDaily,
    TableId::CountryPowerChanges,
    TableId::CountryPowerDailyCosts,
    TableId::GovernmentChanges,
    TableId::PoliticsCaptureCoverage,
];
const ASSEMBLED: &str = "politics observations assembled by spool";
const EVENT_SEQUENCE_STRIDE: u64 = 5;
fn event_sequence(sequence: u64) -> u64 {
    let first = sequence
        .checked_mul(EVENT_SEQUENCE_STRIDE)
        .expect("politics sequence exhausted");
    first
        .checked_add(EVENT_SEQUENCE_STRIDE - 1)
        .expect("politics sequence exhausted");
    first
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    countries: usize,
    entries: usize,
    bytes: usize,
}

collector_primitives::capacity_max!(Capacities {
    countries,
    entries,
    bytes
});
impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        grow_capacity(&mut self.countries, slots);
        grow_capacity(&mut self.entries, slots);
    }
}
pub struct Captured {
    events: Vec<Event>,
    bytes: Vec<u8>,
    balances: Vec<BalanceSnapshot>,
    drained: Option<Result<Drained, &'static str>>,
    census: Outcome,
    handoff: Option<Result<hooks::Handoff, &'static str>>,
    cursors: Vec<hooks::LaneCursor>,
}
impl Captured {
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self {
            events: Vec::new(),
            bytes: Vec::new(),
            balances: Vec::new(),
            drained: None,
            handoff: None,
            cursors: Vec::new(),
            census: Outcome::Unavailable { reason: ASSEMBLED },
        }
    }
    fn worker(capacity: &Capacities) -> io::Result<Self> {
        Ok(Self {
            events: allocation::default_vec(capacity.entries)?,
            bytes: allocation::default_vec(capacity.bytes)?,
            balances: allocation::default_vec(capacity.countries)?,
            drained: None,
            handoff: None,
            cursors: Vec::new(),
            census: Outcome::Unavailable { reason: ASSEMBLED },
        })
    }
    pub fn reset(&mut self) {
        self.drained = None;
        self.handoff = None;
        self.census = Outcome::Unavailable { reason: ASSEMBLED };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.events.capacity() * size_of::<Event>()
            + self.bytes.capacity()
            + self.balances.capacity() * size_of::<BalanceSnapshot>()
            + self.cursors.capacity() * size_of::<hooks::LaneCursor>()
    }
    /// # Safety
    /// Caller holds exclusive `&mut` access to this frame while politics staging stays live for the
    /// active session (or an isolated test) and `politics_changes::release` is excluded. Different
    /// frames may grow concurrently across spool and sizing threads because `resize` locks `LOGS`.
    pub unsafe fn grow(&mut self, capacity: &mut Capacities) {
        if let Some(Ok(drain)) = self.drained {
            grow_capacity(&mut capacity.entries, drain.pending);
            grow_capacity(&mut capacity.bytes, drain.keys_required);
        }
        if let Outcome::Truncated { len, unread_rows } = self.census {
            grow_capacity(
                &mut capacity.countries,
                len.saturating_add(unread_rows as usize),
            );
        }
        // SAFETY: Caller guarantees live politics staging with `release` excluded; `resize` locks `LOGS`.
        unsafe { hooks::resize(capacity.entries, 0) };
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [Outcome::Unavailable { reason: ASSEMBLED }; N]
    }
    pub fn tables(&self) -> [TableView; N] {
        std::array::from_fn(|_| unavailable(ASSEMBLED))
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    /// # Safety
    /// Caller must own the active session's capture or publication failure path when invalidating
    /// live politics observation.
    pub unsafe fn fail(&mut self, reason: &'static str) {
        // SAFETY: Caller owns the politics capture or publication failure path.
        unsafe { hooks::invalidate() };
        self.handoff = Some(Err(reason));
    }
    /// # Safety
    /// Only the admitted hour's sole observer boundary (or an isolated test holding
    /// `staging_test()`) may seal an offered generation and capture a handoff while staging lives.
    pub unsafe fn capture(&mut self) -> u64 {
        let started = crate::clock::qpc();
        self.handoff = Some(hooks::handoff_growing(&mut self.cursors));
        crate::clock::qpc().saturating_sub(started).max(0) as u64
    }
}
fn unavailable(reason: &str) -> TableView {
    crate::table_conversion::unavailable(reason)
}
fn input(rows: TableRows) -> TableInput {
    TableInput::Sampled(TableView::Complete(rows))
}
#[cfg(test)]
fn rows(input: &TableInput) -> Option<&TableRows> {
    match input {
        TableInput::Sampled(TableView::Complete(rows)) => Some(rows),
        TableInput::SharedComplete(rows) => Some(rows),
        _ => None,
    }
}
#[derive(Default)]
struct DailyEvents {
    instance: u32,
    hour: i32,
    rate: Option<Event>,
    pp: Option<Event>,
    cp: Option<Event>,
    costs: Vec<(Event, PowerIdentity)>,
}
#[derive(Default)]
pub struct Ledger {
    branch: Option<BranchId>,
    epoch: u64,
    cursor: u64,
    next: u64,
    hour: Option<i32>,
    balances: HashMap<u32, BalanceSnapshot>,
    next_balances: HashMap<u32, BalanceSnapshot>,
    daily: HashMap<u32, CountryPowerDailyRow>,
    costs: HashMap<u32, Vec<CountryPowerDailyCostRow>>,
    cost_countries: HashSet<u32>,
    cached_daily: Option<Arc<TableRows>>,
    cached_costs: Option<Arc<TableRows>>,
    lexical_index: Vec<u32>,
    lexical_source: Option<Arc<TableRows>>,
    pending_daily: HashMap<u32, DailyEvents>,
    invalid_daily: HashSet<u32>,
    deltas: HashMap<(u32, u8), i128>,
    party_ends: HashMap<u32, Option<u32>>,
    updated_daily: HashSet<u32>,
    scratch: Option<Box<Captured>>,
}
impl Ledger {
    /// # Safety
    /// Only the originating session's sole spool consumer may assemble this captured handoff, while
    /// the politics log retained by `captured` remains live and before `politics_changes::release`.
    pub unsafe fn assemble_owned(
        &mut self,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        countries: &[engine_native::ccountry::Row],
        outcome: Outcome,
        tables: &mut [TableInput],
    ) -> io::Result<()> {
        if allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let pending = captured
            .handoff
            .and_then(Result::ok)
            .map_or(0, |h| h.pending());
        let bytes = captured
            .handoff
            .and_then(Result::ok)
            .map_or(0, |h| h.key_capacity());
        let mut scratch = match self.scratch.take() {
            Some(scratch) => scratch,
            None => Box::new(Captured::worker(&Capacities {
                countries: countries.len(),
                entries: pending,
                bytes,
            })?),
        };
        allocation::resize(&mut scratch.events, pending, Event::default())?;
        allocation::resize(&mut scratch.bytes, bytes, 0)?;
        allocation::resize(
            &mut scratch.balances,
            countries.len(),
            BalanceSnapshot::default(),
        )?;
        if allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        scratch.drained = Some(
            captured
                .handoff
                .unwrap_or(Err("politics handoff absent"))
                .and_then(|h| {
                    // SAFETY: Caller guarantees sole spool ownership while the politics log stays live.
                    unsafe { h.drain(&captured.cursors, &mut scratch.events, &mut scratch.bytes) }
                }),
        );
        scratch.census = match outcome {
            Outcome::Rows { len } => {
                let mut complete = true;
                for (row, dst) in countries[..len].iter().zip(&mut scratch.balances) {
                    if let Some((instance, party)) = row.politics_witness {
                        *dst = BalanceSnapshot {
                            country: row.id,
                            instance,
                            party,
                            pp: row.political_power_raw,
                            cp: row.command_power_raw,
                        };
                    } else {
                        complete = false;
                    }
                }
                if complete {
                    Outcome::Rows { len }
                } else {
                    Outcome::Unavailable {
                        reason: "politics country witness unreadable",
                    }
                }
            }
            _ => Outcome::Unavailable {
                reason: "politics country census incomplete",
            },
        };
        self.assemble_checked(branch, hour, &scratch, tables)?;
        if scratch.events.capacity() > 512 {
            scratch.events = Vec::new();
        }
        if scratch.bytes.capacity() > 32 * 1024 {
            scratch.bytes = Vec::new();
        }
        self.scratch = Some(scratch);
        Ok(())
    }
    #[cfg(test)]
    fn assemble(
        &mut self,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        tables: &mut [TableInput],
    ) {
        if self
            .assemble_checked(branch, hour, captured, tables)
            .is_err()
        {
            for table in TABLES {
                tables[table.index()] =
                    TableInput::Sampled(unavailable("politics assembly allocation failed"));
            }
        }
    }
    fn assemble_checked(
        &mut self,
        branch: BranchId,
        hour: i32,
        captured: &Captured,
        tables: &mut [TableInput],
    ) -> io::Result<()> {
        if allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        if self.branch != Some(branch) {
            *self = Self {
                branch: Some(branch),
                ..Default::default()
            };
        }
        match &tables[TableId::LexicalTokens.index()] {
            TableInput::SharedComplete(shared) => {
                if !self
                    .lexical_source
                    .as_ref()
                    .is_some_and(|prev| Arc::ptr_eq(prev, shared))
                    && let TableRows::LexicalTokens(tokens) = &**shared
                {
                    self.lexical_index = build_lexical_index(tokens)?;
                    self.lexical_source = Some(Arc::clone(shared));
                }
            }
            TableInput::Sampled(TableView::Complete(TableRows::LexicalTokens(tokens))) => {
                let mut copied = allocation::vec(tokens.len())?;
                for row in tokens {
                    copied.push(replay_data::rows::LexicalTokenRow {
                        token: row.token,
                        literal: allocation::copy_vec(&row.literal)?,
                    });
                }
                self.lexical_index = build_lexical_index(&copied)?;
                self.lexical_source = Some(Arc::new(TableRows::LexicalTokens(copied)));
            }
            _ => {}
        }
        let drain = captured.drained.and_then(Result::ok);
        let epoch = drain.map_or(self.epoch, |d| d.epoch);
        let continuous = drain.is_some_and(|d| {
            d.epoch == self.epoch && d.first <= self.cursor && d.first + d.copied as u64 >= d.cursor
        }) && self.hour.and_then(|h| h.checked_add(1)) == Some(hour);
        let mut daily_changed = self.cached_daily.is_none() || self.cached_costs.is_none();
        if epoch != self.epoch {
            self.cursor = 0;
            self.next = 0;
            self.daily.clear();
            self.costs.clear();
            self.cost_countries.clear();
            self.pending_daily.clear();
            self.invalid_daily.clear();
            self.cached_daily = None;
            self.cached_costs = None;
            daily_changed = true;
        }
        if drain.is_some_and(|d| d.copied < d.pending) {
            hooks::short_handoff();
        }
        let mut changes = Vec::new();
        let mut governments = Vec::new();
        let mut deltas = std::mem::take(&mut self.deltas);
        let mut party_ends = std::mem::take(&mut self.party_ends);
        let mut updated_daily = std::mem::take(&mut self.updated_daily);
        deltas.clear();
        party_ends.clear();
        updated_daily.clear();
        if let Some(drain) = drain {
            for event in &captured.events[..drain.copied] {
                if event.epoch != epoch || event.sequence <= self.next {
                    continue;
                }
                let sequence = event_sequence(event.sequence);
                match event.kind {
                    Kind::Power => {
                        let delta = i128::from(event.after) - i128::from(event.before);
                        if event.sequence > self.cursor {
                            let key = (event.country, event.currency as u8);
                            let additional = usize::from(!deltas.contains_key(&key));
                            allocation::reserve_map(&mut deltas, additional)?;
                            *deltas.entry(key).or_default() += delta;
                        }
                        let identity = self.identity(event, &captured.bytes)?;
                        allocation::reserve_push_batch(&mut changes, 1)?;
                        changes.push(CountryPowerChangeRow {
                            country: event.country,
                            instance: event.instance,
                            sequence,
                            epoch,
                            hour: event.hour,
                            currency: event.currency,
                            operation: event.operation,
                            balance_before_raw: event.before,
                            balance_after_raw: RetiredI64(0),
                            requested_delta_raw: event.requested,
                            cause: event.cause,
                            site: event.site,
                            identity,
                            realized_delta_raw: Recorded::Value(event.after.wrapping_sub(event.before)),
                        });
                        if let Some(requested) = event.requested
                            && matches!(
                                event.site,
                                PowerSite::AddPoliticalPower
                                    | PowerSite::AddPoliticalPowerNoDeficit
                                    | PowerSite::SetPoliticalPower
                                    | PowerSite::ClearPolitics
                                    | PowerSite::AddCommandPower
                                    | PowerSite::AllocateCommandPower
                            )
                        {
                            let mut correction = delta - i128::from(requested);
                            let mut ordinal = 1;
                            while correction != 0 {
                                let amount = correction
                                    .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                                    as i64;
                                allocation::reserve_push_batch(&mut changes, 1)?;
                                changes.push(CountryPowerChangeRow {
                                    country: event.country,
                                    instance: event.instance,
                                    sequence: sequence + ordinal,
                                    epoch,
                                    hour: event.hour,
                                    currency: event.currency,
                                    operation: PowerOperation::Clamp,
                                    balance_before_raw: event.after,
                                    balance_after_raw: RetiredI64(0),
                                    requested_delta_raw: Some(amount),
                                    cause: PowerCause::CapCorrection,
                                    site: event.site,
                                    identity: PowerIdentity::default(),
                                    realized_delta_raw: Recorded::Value(0),
                                });
                                correction -= i128::from(amount);
                                ordinal += 1;
                            }
                        }
                        if event.cause == PowerCause::DailyGain
                            && self.stage_daily(event, &captured.bytes)?
                        {
                            let additional = usize::from(!updated_daily.contains(&event.country));
                            allocation::reserve_set(&mut updated_daily, additional)?;
                            updated_daily.insert(event.country);
                        }
                    }
                    Kind::Daily | Kind::Cost => {
                        if self.stage_daily(event, &captured.bytes)? {
                            let additional = usize::from(!updated_daily.contains(&event.country));
                            allocation::reserve_set(&mut updated_daily, additional)?;
                            updated_daily.insert(event.country);
                        }
                    }
                    Kind::Government => {
                        if event.sequence > self.cursor {
                            let key = event.country;
                            let additional = usize::from(!party_ends.contains_key(&key));
                            allocation::reserve_map(&mut party_ends, additional)?;
                            party_ends.insert(key, event.parties[1]);
                        }
                        let key = |index| {
                            self.lexical_token(key_bytes(event, &captured.bytes, index))
                        };
                        allocation::reserve_push_batch(&mut governments, 1)?;
                        governments.push(GovernmentChangeRow {
                            country: event.country,
                            instance: event.instance,
                            sequence,
                            epoch,
                            hour: event.hour,
                            before_party: event.parties[0],
                            after_party: event.parties[1],
                            before_ideology: key(0),
                            after_ideology: key(1),
                            cause: event.government_cause,
                            actor_country: event.actor_country,
                            origin_kind: event.identity.origin_kind,
                            origin_token: event.identity.value(5),
                            origin_key: allocation::copy_vec(key_bytes(event, &captured.bytes, 2))?,
                            site: event.site,
                        });
                    }
                }
            }
        }
        if !updated_daily.is_empty() {
            daily_changed = true;
            for &country in &updated_daily {
                self.publish_daily(country, epoch)?;
            }
        }
        self.updated_daily = updated_daily;
        let mut coverage = Vec::new();
        if let Outcome::Rows { len } = captured.census {
            let mut current = std::mem::take(&mut self.next_balances);
            current.clear();
            for now in &captured.balances[..len] {
                let previous = self.balances.get(&now.country);
                let mut streams = 0;
                if continuous && let Some(old) = previous {
                    streams = PoliticsCaptureCoverageRow::PP
                        | PoliticsCaptureCoverageRow::CP
                        | PoliticsCaptureCoverageRow::GOVERNMENT;
                    for (currency, before, after, bit) in [
                        (
                            PowerCurrency::Political,
                            old.pp,
                            now.pp,
                            PoliticsCaptureCoverageRow::PP,
                        ),
                        (
                            PowerCurrency::Command,
                            Some(old.cp),
                            Some(now.cp),
                            PoliticsCaptureCoverageRow::CP,
                        ),
                    ] {
                        if let (Some(before), Some(after)) = (before, after) {
                            let observed = deltas
                                .get(&(now.country, currency as u8))
                                .copied()
                                .unwrap_or(0);
                            let missing = i128::from(after) - i128::from(before) - observed;
                            if missing != 0 {
                                streams &= !bit;
                                hooks::mismatch(now.country, hour, currency, missing);
                            }
                        } else {
                            streams &= !bit;
                        }
                    }
                    if party_ends.get(&now.country).copied().unwrap_or(old.party) != now.party {
                        streams &= !PoliticsCaptureCoverageRow::GOVERNMENT;
                        hooks::government_mismatch();
                    }
                    if self.cost_countries.contains(&now.country)
                        && !self.invalid_daily.contains(&now.country)
                    {
                        streams |= PoliticsCaptureCoverageRow::PP_DAILY_COSTS;
                    }
                }
                allocation::reserve_push_batch(&mut coverage, 1)?;
                coverage.push(PoliticsCaptureCoverageRow {
                    country: now.country,
                    instance: now.instance,
                    epoch,
                    since_hour: self
                        .hour
                        .filter(|_| continuous && previous.is_some())
                        .unwrap_or(hour),
                    through_hour: hour,
                    streams,
                });
                let additional = usize::from(!current.contains_key(&now.country));
                allocation::reserve_map(&mut current, additional)?;
                current.insert(now.country, *now);
            }
            let daily_before = self.daily.len();
            let costs_before = self.cost_countries.len();
            self.daily
                .retain(|country, _| current.contains_key(country));
            self.cost_countries
                .retain(|country| self.daily.contains_key(country));
            self.costs
                .retain(|country, _| self.daily.contains_key(country));
            daily_changed |=
                self.daily.len() != daily_before || self.cost_countries.len() != costs_before;
            self.pending_daily.retain(|country, batch| {
                current.contains_key(country)
                    && (batch.hour == hour || batch.hour.checked_add(1) == Some(hour))
            });
            self.invalid_daily
                .retain(|country| current.contains_key(country));
            self.next_balances = std::mem::replace(&mut self.balances, current);
        } else {
            self.balances.clear();
        }
        self.deltas = deltas;
        self.party_ends = party_ends;
        if daily_changed {
            let mut daily = allocation::collect_vec(self.daily.values().cloned())?;
            daily.sort_unstable_by_key(RowCodec::key);
            let added_count = self
                .costs
                .values()
                .try_fold(0, |count, rows| allocation::add(count, rows.len()))?;
            let keep_cached = |row: &CountryPowerDailyCostRow| {
                self.cost_countries.contains(&row.country)
                    && !self.updated_daily.contains(&row.country)
                    && !self.costs.contains_key(&row.country)
            };
            let mut costs = match self.cached_costs.take() {
                Some(previous) => match Arc::try_unwrap(previous) {
                    Ok(TableRows::CountryPowerDailyCosts(mut existing)) => {
                        existing.retain(keep_cached);
                        allocation::reserve(&mut existing, added_count)?;
                        existing
                    }
                    Ok(_) => allocation::vec(added_count)?,
                    Err(shared) => {
                        let retained = if let TableRows::CountryPowerDailyCosts(rows) = &*shared {
                            rows.iter().filter(|row| keep_cached(row)).count()
                        } else {
                            0
                        };
                        let mut copied = allocation::vec(allocation::add(retained, added_count)?)?;
                        if let TableRows::CountryPowerDailyCosts(rows) = &*shared {
                            for row in rows {
                                if keep_cached(row) {
                                    copied.push(copy_cost(row)?);
                                }
                            }
                        }
                        copied
                    }
                },
                None => allocation::vec(added_count)?,
            };
            for (_, rows) in self.costs.drain() {
                costs.extend(rows);
            }
            costs.sort_unstable_by_key(RowCodec::key);
            if allocation::capture_stopped() {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            self.cached_daily = Some(Arc::new(TableRows::CountryPowerDaily(daily)));
            self.cached_costs = Some(Arc::new(TableRows::CountryPowerDailyCosts(costs)));
        }
        if allocation::capture_stopped() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        self.hour = Some(hour);
        self.epoch = epoch;
        if let Some(drain) = drain {
            self.cursor = drain.cursor;
            self.next = drain.first + drain.copied as u64;
        }
        changes.sort_unstable_by_key(|row| (row.country, row.sequence));
        governments.sort_unstable_by_key(|row| (row.country, row.sequence));
        coverage.sort_unstable_by_key(RowCodec::key);
        let failed = captured.drained.and_then(Result::err);
        let publish = |rows: TableRows| {
            failed.map_or_else(
                || input(rows),
                |reason| TableInput::Sampled(unavailable(reason)),
            )
        };
        let publish_shared = |rows: &Option<Arc<TableRows>>| {
            failed.map_or_else(
                || TableInput::SharedComplete(Arc::clone(rows.as_ref().expect("cached daily"))),
                |reason| TableInput::Sampled(unavailable(reason)),
            )
        };
        tables[TableId::CountryPowerChanges.index()] =
            publish(TableRows::CountryPowerChanges(changes));
        tables[TableId::GovernmentChanges.index()] =
            publish(TableRows::GovernmentChanges(governments));
        tables[TableId::PoliticsCaptureCoverage.index()] =
            if matches!(captured.census, Outcome::Rows { .. }) {
                input(TableRows::PoliticsCaptureCoverage(coverage))
            } else {
                TableInput::Sampled(unavailable("politics balance census incomplete"))
            };
        tables[TableId::CountryPowerDaily.index()] = publish_shared(&self.cached_daily);
        tables[TableId::CountryPowerDailyCosts.index()] = publish_shared(&self.cached_costs);
        Ok(())
    }
    fn stage_daily(&mut self, event: &Event, bytes: &[u8]) -> io::Result<bool> {
        if event.hour.rem_euclid(24) != 0 {
            let additional = usize::from(!self.invalid_daily.contains(&event.country));
            allocation::reserve_set(&mut self.invalid_daily, additional)?;
            self.invalid_daily.insert(event.country);
            hooks::daily_failure();
            return Ok(false);
        }
        let identity = self.identity(event, bytes)?;
        let additional = usize::from(!self.pending_daily.contains_key(&event.country));
        allocation::reserve_map(&mut self.pending_daily, additional)?;
        let batch = self
            .pending_daily
            .entry(event.country)
            .or_insert_with(|| DailyEvents {
                instance: event.instance,
                hour: event.hour,
                ..Default::default()
            });
        if batch.hour != event.hour {
            *batch = DailyEvents {
                instance: event.instance,
                hour: event.hour,
                ..Default::default()
            };
        }
        match event.kind {
            Kind::Daily => batch.rate = Some(*event),
            Kind::Cost => {
                allocation::reserve_push_batch(&mut batch.costs, 1)?;
                batch.costs.push((*event, identity));
            }
            Kind::Power if event.currency == PowerCurrency::Political => batch.pp = Some(*event),
            Kind::Power => batch.cp = Some(*event),
            Kind::Government => unreachable!(),
        }
        Ok(true)
    }
    fn publish_daily(&mut self, country: u32, epoch: u64) -> io::Result<()> {
        let batch = &self.pending_daily[&country];
        let mut daily = CountryPowerDailyRow {
            country,
            instance: batch.instance,
            epoch,
            day_hour: batch.hour,
            ..Default::default()
        };
        self.invalid_daily.remove(&country);
        self.costs.remove(&country);
        self.cost_countries.remove(&country);
        if let (Some(rate), Some(pp)) = (batch.rate, batch.pp) {
            if pp.requested == Some(rate.net) {
                daily.pp_gross_gain_raw = Some(rate.gross);
                daily.pp_cost_raw = Some(rate.cost);
                daily.pp_net_gain_raw = Some(rate.net);
                daily.pp_realized_delta_raw = Some(pp.after.wrapping_sub(pp.before));
                daily.pp_sequence = Some(event_sequence(pp.sequence));
                let sum = batch
                    .costs
                    .iter()
                    .fold(0i64, |sum, (event, _)| sum.wrapping_add(event.cost));
                if sum == rate.cost {
                    let mut costs = allocation::vec(batch.costs.len())?;
                    for (ordinal, (event, identity)) in batch.costs.iter().enumerate() {
                        costs.push(CountryPowerDailyCostRow {
                            country,
                            instance: event.instance,
                            ordinal: ordinal as u32,
                            epoch,
                            day_hour: batch.hour,
                            pp_sequence: event_sequence(pp.sequence),
                            component: event.component,
                            cost_raw: event.cost,
                            cause: event.cause,
                            site: event.site,
                            identity: copy_identity(identity)?,
                        });
                    }
                    allocation::reserve_map(&mut self.costs, 1)?;
                    allocation::reserve_set(&mut self.cost_countries, 1)?;
                    self.costs.insert(country, costs);
                    self.cost_countries.insert(country);
                } else {
                    let additional = usize::from(!self.invalid_daily.contains(&country));
                    allocation::reserve_set(&mut self.invalid_daily, additional)?;
                    self.invalid_daily.insert(country);
                    hooks::daily_failure();
                }
            } else {
                let additional = usize::from(!self.invalid_daily.contains(&country));
                allocation::reserve_set(&mut self.invalid_daily, additional)?;
                self.invalid_daily.insert(country);
                hooks::daily_failure();
            }
        } else if batch.rate.is_some() || batch.pp.is_some() {
            let additional = usize::from(!self.invalid_daily.contains(&country));
            allocation::reserve_set(&mut self.invalid_daily, additional)?;
            self.invalid_daily.insert(country);
        }
        if let Some(cp) = batch.cp {
            daily.cp_gain_raw = cp.requested;
            daily.cp_realized_delta_raw = Some(cp.after.wrapping_sub(cp.before));
            daily.cp_sequence = Some(event_sequence(cp.sequence));
        }
        let additional = usize::from(!self.daily.contains_key(&country));
        allocation::reserve_map(&mut self.daily, additional)?;
        self.daily.insert(country, daily);
        Ok(())
    }
    fn lexical_token(&self, literal: &[u8]) -> Option<u32> {
        let Some(TableRows::LexicalTokens(tokens)) = self.lexical_source.as_deref() else {
            return None;
        };
        if self.lexical_index.is_empty() {
            return None;
        }
        let mask = self.lexical_index.len() - 1;
        let mut slot = hash_lexical(literal) as usize & mask;
        loop {
            let entry = self.lexical_index[slot];
            if entry == u32::MAX {
                return None;
            }
            let row = &tokens[entry as usize];
            if row.literal.as_slice() == literal {
                return Some(row.token);
            }
            slot = (slot + 1) & mask;
        }
    }
    fn identity(&self, event: &Event, bytes: &[u8]) -> io::Result<PowerIdentity> {
        let value = event.identity;
        let item_key = allocation::copy_vec(key_bytes(event, bytes, 0))?;
        Ok(PowerIdentity {
            action: value.action,
            item_token: value
                .value(0)
                .or_else(|| self.lexical_token(&item_key)),
            item_key,
            slot_key: allocation::copy_vec(key_bytes(event, bytes, 1))?,
            character: value.value(1),
            object_kind: value.object_kind,
            object_reference: value.object_reference(),
            target_country: value.value(2),
            target_state: value.value(3),
            target_region: value.value(4),
            origin_kind: value.origin_kind,
            origin_token: value.value(5),
            origin_key: allocation::copy_vec(key_bytes(event, bytes, 2))?,
        })
    }
}
fn hash_lexical(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash ^ (hash >> 32)
}
fn build_lexical_index(tokens: &[replay_data::rows::LexicalTokenRow]) -> io::Result<Vec<u32>> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let slots = tokens
        .len()
        .checked_mul(2)
        .and_then(usize::checked_next_power_of_two)
        .ok_or_else(allocation::oom)?
        .max(16);
    let mut table = allocation::filled(slots, u32::MAX)?;
    let mask = slots - 1;
    for (index, row) in tokens.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| allocation::oom())?;
        let mut slot = hash_lexical(&row.literal) as usize & mask;
        loop {
            let existing = table[slot];
            if existing == u32::MAX || tokens[existing as usize].literal == row.literal {
                table[slot] = index;
                break;
            }
            slot = (slot + 1) & mask;
        }
    }
    Ok(table)
}
fn copy_identity(value: &PowerIdentity) -> io::Result<PowerIdentity> {
    Ok(PowerIdentity {
        action: value.action,
        item_token: value.item_token,
        item_key: allocation::copy_vec(&value.item_key)?,
        slot_key: allocation::copy_vec(&value.slot_key)?,
        character: value.character,
        object_kind: value.object_kind,
        object_reference: value.object_reference,
        target_country: value.target_country,
        target_state: value.target_state,
        target_region: value.target_region,
        origin_kind: value.origin_kind,
        origin_token: value.origin_token,
        origin_key: allocation::copy_vec(&value.origin_key)?,
    })
}
fn copy_cost(value: &CountryPowerDailyCostRow) -> io::Result<CountryPowerDailyCostRow> {
    Ok(CountryPowerDailyCostRow {
        country: value.country,
        instance: value.instance,
        ordinal: value.ordinal,
        epoch: value.epoch,
        day_hour: value.day_hour,
        pp_sequence: value.pp_sequence,
        component: value.component,
        cost_raw: value.cost_raw,
        cause: value.cause,
        site: value.site,
        identity: copy_identity(&value.identity)?,
    })
}
fn key_bytes<'a>(event: &Event, bytes: &'a [u8], index: usize) -> &'a [u8] {
    let key = event.identity.keys[index];
    bytes
        .get(key.address..key.address.saturating_add(key.len))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_events::politics_sites as native;
    fn captured(
        epoch: u64,
        first: u64,
        events: &[Event],
        balances: &[BalanceSnapshot],
    ) -> Captured {
        let mut capture = Captured::worker(&Capacities {
            countries: balances.len(),
            entries: events.len(),
            bytes: 0,
        })
        .unwrap();
        capture.events.copy_from_slice(events);
        capture.balances.copy_from_slice(balances);
        capture.census = Outcome::Rows {
            len: balances.len(),
        };
        capture.drained = Some(Ok(Drained {
            epoch,
            first,
            cursor: first + events.len() as u64,
            copied: events.len(),
            bytes: 0,
            pending: events.len(),
            keys_required: 0,
        }));
        capture
    }
    fn tables() -> Vec<TableInput> {
        (0..replay_data::rows::TABLE_COUNT)
            .map(|_| TableInput::Sampled(unavailable("fixture")))
            .collect()
    }
    fn balance(country: u32) -> BalanceSnapshot {
        BalanceSnapshot {
            country,
            instance: 1,
            pp: Some(100),
            cp: 200,
            party: None,
        }
    }

    #[test]
    pub fn compact_handoff_rows_are_byte_identical_to_original_event_stream() {
        let mut native_bytes = vec![0u8; 64];
        native_bytes[..8].copy_from_slice(b"mod_item");
        native_bytes[16..24].copy_from_slice(&8u64.to_le_bytes());
        native_bytes[24..32].copy_from_slice(&15u64.to_le_bytes());
        let literal = native::Literal {
            address: 0x1000,
            len: 8,
        };
        let mut stream = vec![
            Event {
                kind: Kind::Cost,
                cost: 3,
                ..Default::default()
            },
            Event {
                kind: Kind::Daily,
                gross: 10,
                cost: 3,
                net: 7,
                ..Default::default()
            },
            Event {
                before: 100,
                after: 107,
                requested: Some(7),
                cause: PowerCause::DailyGain,
                ..Default::default()
            },
            Event {
                currency: PowerCurrency::Command,
                before: 200,
                after: 200,
                requested: Some(20),
                cause: PowerCause::DailyGain,
                site: PowerSite::AddCommandPower,
                ..Default::default()
            },
            Event {
                before: 107,
                after: 72,
                requested: Some(-35),
                cause: PowerCause::Decision,
                identity: hooks::Identity {
                    keys: [literal, literal, literal],

                    object_kind: PowerObjectKind::Leader,

                    origin_kind: PowerOriginKind::Decision,

                    ..hooks::Identity::from_optional(
                        [Some(0), Some(0), Some(5002), None, None, Some(0)],
                        Some(u64::MAX),
                    )
                },
                ..Default::default()
            },
            Event {
                kind: Kind::Government,
                government_cause: GovernmentCause::SetPoliticsEffect,
                parties: [Some(0), Some(7)],
                actor_country: Some(0),
                identity: hooks::Identity {
                    keys: [literal, literal, literal],
                    origin_kind: PowerOriginKind::Decision,
                    ..Default::default()
                },
                ..Default::default()
            },
        ];
        for (n, e) in stream.iter_mut().enumerate() {
            e.country = 5001;
            e.instance = 1;
            e.epoch = 1;
            e.sequence = n as u64 + 1;
        }
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &native_bytes,
        };
        let (_, compact_events, compact_keys) = hooks::stage_fixture(&stream, &memory);
        let mut original_keys = Vec::new();
        for event in &mut stream {
            for key in &mut event.identity.keys {
                let start = original_keys.len();
                original_keys.resize(start + key.len, 0);
                if key.len != 0 {
                    key.copy(&memory, &mut original_keys[start..]).unwrap();
                }
                key.address = start;
            }
        }
        native_bytes.fill(0);
        let now = BalanceSnapshot {
            pp: Some(72),
            party: Some(7),
            ..balance(5001)
        };
        let mut original = captured(1, 0, &stream, &[now]);
        original.bytes = original_keys;
        let mut compact = captured(1, 0, &compact_events, &[now]);
        compact.bytes = compact_keys;
        let mut a = Ledger::default();
        let mut b = Ledger::default();
        let mut left = tables();
        let mut right = tables();
        let before = BalanceSnapshot {
            party: Some(0),
            ..balance(5001)
        };
        a.assemble(1, -1, &captured(1, 0, &[], &[before]), &mut left);
        b.assemble(1, -1, &captured(1, 0, &[], &[before]), &mut right);
        a.assemble(1, 0, &original, &mut left);
        b.assemble(1, 0, &compact, &mut right);
        assert_eq!(left, right);
        let wire = |tables: Vec<TableInput>| {
            let source = replay_frame::frame::FrameInput {
                branch: 1,
                hour: 0,
                tables: tables.try_into().unwrap(),
                metrics: Default::default(),
            };
            let frame = replay_frame::frame::FrameEncoder::new()
                .encode_input(source, true)
                .unwrap();
            let mut bytes = Vec::new();
            frame.encode(&mut bytes).unwrap();
            bytes
        };
        assert_eq!(wire(left), wire(right));
    }

    #[test]
    fn conservation_clears_only_the_country_currency_and_never_balances_with_a_row() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(
            1,
            10,
            &captured(1, 0, &[], &[balance(5001), balance(5002)]),
            &mut tables,
        );
        let event = Event {
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: 1,
            hour: 10,
            before: 100,
            after: 75,
            requested: Some(-25),
            cause: PowerCause::Decision,
            ..Default::default()
        };
        let now = [
            BalanceSnapshot {
                pp: Some(75),
                ..balance(5001)
            },
            BalanceSnapshot {
                cp: 150,
                ..balance(5002)
            },
        ];
        ledger.assemble(1, 11, &captured(1, 0, &[event], &now), &mut tables);
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_eq!(
            coverage[0].streams,
            PoliticsCaptureCoverageRow::PP
                | PoliticsCaptureCoverageRow::CP
                | PoliticsCaptureCoverageRow::GOVERNMENT
        );
        assert_eq!(
            coverage[1].streams,
            PoliticsCaptureCoverageRow::PP | PoliticsCaptureCoverageRow::GOVERNMENT
        );
        let Some(TableRows::CountryPowerChanges(events)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cause, PowerCause::Decision);
        ledger.assemble(1, 12, &captured(1, 1, &[], &now), &mut tables);
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert!(
            coverage
                .iter()
                .all(|row| row.streams & PoliticsCaptureCoverageRow::CP != 0)
        );
    }
    #[test]
    fn native_country_reset_has_no_synthetic_cap_correction() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        let before = BalanceSnapshot {
            cp: 40,
            ..balance(5001)
        };
        ledger.assemble(1, 10, &captured(1, 0, &[], &[before]), &mut tables);
        let event = Event {
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: 1,
            hour: 10,
            currency: PowerCurrency::Command,
            operation: PowerOperation::Reset,
            before: 40,
            after: 10,
            requested: Some(-40),
            cause: PowerCause::ResetSetup,
            site: PowerSite::ResetCountry,
            ..Default::default()
        };
        ledger.assemble(
            1,
            11,
            &captured(1, 0, &[event], &[BalanceSnapshot { cp: 10, ..before }]),
            &mut tables,
        );
        let Some(TableRows::CountryPowerChanges(events)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].balance_after() - events[0].balance_before_raw,
            -30
        );
        assert_eq!(events[0].operation, PowerOperation::Reset);
    }

    #[test]
    fn clamps_are_explicit_accounting_entries_and_identity_reuse_begins_a_new_baseline() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(1, 10, &captured(1, 0, &[], &[balance(5001)]), &mut tables);
        let event = Event {
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: 1,
            hour: 10,
            currency: PowerCurrency::Command,
            before: 200,
            after: 200,
            requested: Some(25),
            cause: PowerCause::DailyGain,
            site: PowerSite::AddCommandPower,
            ..Default::default()
        };
        ledger.assemble(
            1,
            11,
            &captured(1, 0, &[event], &[balance(5001)]),
            &mut tables,
        );
        let Some(TableRows::CountryPowerChanges(events)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].cause, PowerCause::CapCorrection);
        assert_eq!(events[1].requested_delta_raw, Some(-25));
        assert_eq!(
            events
                .iter()
                .map(CountryPowerChangeRow::realized_delta_raw)
                .sum::<i128>(),
            0
        );
        ledger.assemble(
            1,
            12,
            &captured(
                1,
                1,
                &[],
                &[BalanceSnapshot {
                    instance: 2,
                    ..balance(5002)
                }],
            ),
            &mut tables,
        );
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_eq!(coverage[0].streams, 0);
    }
    #[test]
    fn negative_set_target_retains_a_separate_floor_correction() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(1, 10, &captured(1, 0, &[], &[balance(5001)]), &mut tables);
        let event = Event {
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: 1,
            hour: 10,
            before: 100,
            after: 0,
            requested: Some(-125),
            operation: PowerOperation::Set,
            cause: PowerCause::ScriptEffect,
            site: PowerSite::SetPoliticalPower,
            ..Default::default()
        };
        ledger.assemble(
            1,
            11,
            &captured(
                1,
                0,
                &[event],
                &[BalanceSnapshot {
                    pp: Some(0),
                    ..balance(5001)
                }],
            ),
            &mut tables,
        );
        let Some(TableRows::CountryPowerChanges(events)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].operation, PowerOperation::Set);
        assert_eq!(events[1].operation, PowerOperation::Clamp);
        assert_eq!(events[1].requested_delta_raw, Some(25));
        assert_eq!(
            events
                .iter()
                .map(CountryPowerChangeRow::realized_delta_raw)
                .sum::<i128>(),
            -100
        );
    }
    #[test]
    fn politics_tfix2_interleaved_countries_publish_rows_in_ascending_key_order() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        let first = [balance(5001), balance(5002)];
        ledger.assemble(1, 0, &captured(1, 0, &[], &first), &mut tables);
        let power = |country, sequence, before, after| Event {
            country,
            instance: 1,
            epoch: 1,
            sequence,
            hour: 1,
            before,
            after,
            requested: Some(after - before),
            cause: PowerCause::Decision,
            ..Default::default()
        };
        let government = |country, sequence| Event {
            kind: Kind::Government,
            country,
            instance: 1,
            epoch: 1,
            sequence,
            hour: 1,
            government_cause: GovernmentCause::SetPoliticsEffect,
            parties: [None, None],
            ..Default::default()
        };
        let events = [
            power(5002, 1, 100, 90),
            power(5001, 2, 100, 80),
            government(5002, 3),
            power(5002, 4, 90, 85),
            government(5001, 5),
        ];
        let now = [
            BalanceSnapshot {
                pp: Some(80),
                ..balance(5001)
            },
            BalanceSnapshot {
                pp: Some(85),
                ..balance(5002)
            },
        ];
        ledger.assemble(1, 1, &captured(1, 0, &events, &now), &mut tables);
        let Some(TableRows::CountryPowerChanges(changes)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        let keys: Vec<_> = changes.iter().map(|r| (r.country, r.sequence)).collect();
        assert_eq!(
            keys,
            [
                (5001, event_sequence(2)),
                (5002, event_sequence(1)),
                (5002, event_sequence(4))
            ]
        );
        let Some(TableRows::GovernmentChanges(governments)) =
            rows(&tables[TableId::GovernmentChanges.index()])
        else {
            panic!()
        };
        let keys: Vec<_> = governments
            .iter()
            .map(|r| (r.country, r.sequence))
            .collect();
        assert_eq!(keys, [(5001, event_sequence(5)), (5002, event_sequence(3))]);
    }
    #[test]
    fn daily_components_certify_the_native_reduction_and_cp_is_independently_available() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        let first = [
            balance(5001),
            BalanceSnapshot {
                pp: None,
                ..balance(5002)
            },
        ];
        ledger.assemble(1, -1, &captured(1, 0, &[], &first), &mut tables);
        let events = [
            Event {
                kind: Kind::Cost,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 1,
                hour: 0,
                cost: 3,
                ..Default::default()
            },
            Event {
                kind: Kind::Daily,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 2,
                hour: 0,
                gross: 10,
                cost: 3,
                net: 7,
                ..Default::default()
            },
            Event {
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 3,
                hour: 0,
                before: 100,
                after: 107,
                requested: Some(7),
                cause: PowerCause::DailyGain,
                ..Default::default()
            },
            Event {
                country: 5002,
                instance: 1,
                epoch: 1,
                sequence: 4,
                hour: 0,
                currency: PowerCurrency::Command,
                before: 200,
                after: 201,
                requested: Some(1),
                cause: PowerCause::DailyGain,
                ..Default::default()
            },
        ];
        let now = [
            BalanceSnapshot {
                pp: Some(107),
                ..balance(5001)
            },
            BalanceSnapshot {
                pp: None,
                cp: 201,
                ..balance(5002)
            },
        ];
        ledger.assemble(1, 0, &captured(1, 0, &events, &now), &mut tables);
        let Some(TableRows::CountryPowerDaily(daily)) =
            rows(&tables[TableId::CountryPowerDaily.index()])
        else {
            panic!()
        };
        assert_eq!(daily.len(), 2);
        assert_eq!((daily[0].country, daily[0].instance), (5001, 1));
        assert_eq!((daily[1].country, daily[1].instance), (5002, 1));
        assert_eq!(daily[0].pp_net_gain_raw, Some(7));
        assert_eq!(daily[1].pp_net_gain_raw, None);
        assert_eq!(daily[1].cp_gain_raw, Some(1));
    }
    #[test]
    fn daily_batches_keep_components_through_recipient_changes_and_reset_on_the_next_day() {
        let mut ledger = Ledger::default();
        for (index, event) in [
            Event {
                kind: Kind::Cost,
                cost: 3,
                ..Default::default()
            },
            Event {
                kind: Kind::Daily,
                gross: 10,
                cost: 3,
                net: 7,
                ..Default::default()
            },
            Event {
                kind: Kind::Power,
                currency: PowerCurrency::Political,
                requested: Some(7),
                before: 100,
                after: 107,
                ..Default::default()
            },
            Event {
                kind: Kind::Power,
                currency: PowerCurrency::Command,
                requested: Some(2),
                before: 200,
                after: 202,
                ..Default::default()
            },
        ]
        .into_iter()
        .enumerate()
        {
            let event = Event {
                country: 5001,
                instance: index as u32 + 1,
                hour: 0,
                sequence: index as u64 + 1,
                ..event
            };
            assert!(ledger.stage_daily(&event, &[]).unwrap());
        }
        ledger.publish_daily(5001, 1).unwrap();
        let daily = &ledger.daily[&5001];
        assert_eq!((daily.day_hour, daily.instance), (0, 1));
        assert_eq!(daily.pp_net_gain_raw, Some(7));
        assert_eq!(daily.pp_realized_delta_raw, Some(7));
        assert_eq!(daily.cp_gain_raw, Some(2));
        assert_eq!(daily.cp_realized_delta_raw, Some(2));
        assert_eq!(ledger.costs[&5001].len(), 1);
        assert_eq!(ledger.costs[&5001][0].instance, 1);
        assert!(!ledger.invalid_daily.contains(&5001));
        assert!(
            ledger
                .stage_daily(
                    &Event {
                        kind: Kind::Power,
                        country: 5001,
                        instance: 7,
                        hour: 24,
                        sequence: 5,
                        currency: PowerCurrency::Command,
                        requested: Some(9),
                        before: 202,
                        after: 211,
                        ..Default::default()
                    },
                    &[],
                )
                .unwrap()
        );
        ledger.publish_daily(5001, 1).unwrap();
        let daily = &ledger.daily[&5001];
        assert_eq!((daily.day_hour, daily.instance), (24, 7));
        assert_eq!(daily.pp_net_gain_raw, None);
        assert_eq!(daily.pp_sequence, None);
        assert_eq!(daily.cp_gain_raw, Some(9));
        assert_eq!(daily.cp_realized_delta_raw, Some(9));
        assert!(!ledger.costs.contains_key(&5001));
        assert!(!ledger.invalid_daily.contains(&5001));
    }
    #[test]
    fn replacement_log_restarts_sequences_without_dropping_new_epoch_events() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(1, 10, &captured(1, 100, &[], &[balance(5001)]), &mut tables);
        let event = Event {
            country: 5001,
            instance: 1,
            epoch: 2,
            sequence: 1,
            hour: 10,
            before: 100,
            after: 75,
            requested: Some(-25),
            cause: PowerCause::Decision,
            ..Default::default()
        };
        ledger.assemble(
            1,
            11,
            &captured(
                2,
                0,
                &[event],
                &[BalanceSnapshot {
                    pp: Some(75),
                    ..balance(5001)
                }],
            ),
            &mut tables,
        );
        let Some(TableRows::CountryPowerChanges(events)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].epoch, 2);
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_eq!(coverage[0].streams, 0);
    }
    #[test]
    fn failed_handoff_publishes_every_hook_stream_unavailable() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        let mut capture = captured(1, 0, &[], &[balance(5001)]);
        capture.drained = Some(Err("politics capture disabled"));
        ledger.assemble(1, 0, &capture, &mut tables);
        for id in [
            TableId::CountryPowerChanges,
            TableId::GovernmentChanges,
            TableId::CountryPowerDaily,
            TableId::CountryPowerDailyCosts,
        ] {
            assert!(matches!(
                &tables[id.index()],
                TableInput::Sampled(TableView::Unavailable { reason })
                    if reason == "politics capture disabled"
            ));
        }
    }
    #[test]
    fn deferred_cost_keys_and_rates_survive_short_handoffs() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(1, -1, &captured(1, 0, &[], &[balance(5001)]), &mut tables);
        let cost = Event {
            kind: Kind::Cost,
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: 1,
            hour: 0,
            cost: 3,
            identity: hooks::Identity {
                keys: [
                    native::Literal { address: 0, len: 7 },
                    native::Literal::default(),
                    native::Literal::default(),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let mut first = captured(
            1,
            0,
            &[cost],
            &[BalanceSnapshot {
                pp: Some(107),
                ..balance(5001)
            }],
        );
        first.bytes = b"mod_key".to_vec();
        first.drained.as_mut().unwrap().as_mut().unwrap().cursor = 3;
        first.drained.as_mut().unwrap().as_mut().unwrap().pending = 3;
        ledger.assemble(1, 0, &first, &mut tables);
        first.bytes.fill(0);
        let tail = [
            Event {
                kind: Kind::Daily,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 2,
                hour: 0,
                gross: 10,
                cost: 3,
                net: 7,
                ..Default::default()
            },
            Event {
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 3,
                hour: 0,
                before: 100,
                after: 107,
                requested: Some(7),
                cause: PowerCause::DailyGain,
                ..Default::default()
            },
        ];
        ledger.assemble(
            1,
            1,
            &captured(
                1,
                1,
                &tail,
                &[BalanceSnapshot {
                    pp: Some(107),
                    ..balance(5001)
                }],
            ),
            &mut tables,
        );
        let Some(TableRows::CountryPowerDaily(daily)) =
            rows(&tables[TableId::CountryPowerDaily.index()])
        else {
            panic!()
        };
        assert_eq!(daily[0].pp_net_gain_raw, Some(7));
        let Some(TableRows::CountryPowerDailyCosts(costs)) =
            rows(&tables[TableId::CountryPowerDailyCosts.index()])
        else {
            panic!()
        };
        assert_eq!(costs[0].identity.item_key, b"mod_key");
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_ne!(coverage[0].streams & PoliticsCaptureCoverageRow::PP, 0);
    }
    #[test]
    fn mutable_surrender_targets_do_not_hide_power_or_government_writer_coverage() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(1, 10, &captured(1, 0, &[], &[balance(5001)]), &mut tables);
        let event = Event {
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: 1,
            hour: 10,
            currency: PowerCurrency::Command,
            before: 200,
            after: 207,
            ..Default::default()
        };
        let mut now = balance(5001);
        now.instance = 2;
        now.cp = 207;
        ledger.assemble(1, 11, &captured(1, 0, &[event], &[now]), &mut tables);
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_eq!(coverage[0].instance, 2);
        assert_eq!(coverage[0].since_hour, 10);
        assert_eq!(
            coverage[0].streams
                & (PoliticsCaptureCoverageRow::PP
                    | PoliticsCaptureCoverageRow::CP
                    | PoliticsCaptureCoverageRow::GOVERNMENT),
            PoliticsCaptureCoverageRow::PP
                | PoliticsCaptureCoverageRow::CP
                | PoliticsCaptureCoverageRow::GOVERNMENT
        );
    }
    #[test]
    fn cached_costs_merges_updates_and_prunes_stale_pending_daily() {
        let mut ledger = Ledger::default();
        let mut tables = tables();
        ledger.assemble(
            1,
            0,
            &captured(1, 0, &[], &[balance(5001), balance(5002)]),
            &mut tables,
        );
        let first_day = [
            Event {
                kind: Kind::Cost,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 1,
                hour: 0,
                cost: 5,
                ..Default::default()
            },
            Event {
                kind: Kind::Daily,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 2,
                hour: 0,
                gross: 10,
                cost: 5,
                net: 5,
                ..Default::default()
            },
            Event {
                kind: Kind::Power,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 3,
                hour: 0,
                currency: PowerCurrency::Political,
                cause: PowerCause::DailyGain,
                before: 100,
                after: 105,
                requested: Some(5),
                ..Default::default()
            },
            Event {
                kind: Kind::Cost,
                country: 5002,
                instance: 1,
                epoch: 1,
                sequence: 4,
                hour: 0,
                cost: 9,
                ..Default::default()
            },
            Event {
                kind: Kind::Daily,
                country: 5002,
                instance: 1,
                epoch: 1,
                sequence: 5,
                hour: 0,
                gross: 12,
                cost: 9,
                net: 3,
                ..Default::default()
            },
            Event {
                kind: Kind::Power,
                country: 5002,
                instance: 1,
                epoch: 1,
                sequence: 6,
                hour: 0,
                currency: PowerCurrency::Political,
                cause: PowerCause::DailyGain,
                before: 200,
                after: 203,
                requested: Some(3),
                ..Default::default()
            },
            Event {
                kind: Kind::Cost,
                country: 5099,
                instance: 1,
                epoch: 1,
                sequence: 7,
                hour: 0,
                cost: 1,
                ..Default::default()
            },
        ];
        ledger.assemble(
            1,
            1,
            &captured(
                1,
                0,
                &first_day,
                &[balance(5001), balance(5002), balance(5099)],
            ),
            &mut tables,
        );
        let Some(TableRows::CountryPowerDailyCosts(costs)) =
            rows(&tables[TableId::CountryPowerDailyCosts.index()])
        else {
            panic!()
        };
        assert_eq!(costs.len(), 2);
        assert_eq!(costs[0].country, 5001);
        assert_eq!(costs[1].country, 5002);
        assert!(ledger.costs.is_empty());
        assert!(ledger.pending_daily.contains_key(&5099));

        let second_day = [
            Event {
                kind: Kind::Daily,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 8,
                hour: 24,
                gross: 10,
                cost: 0,
                net: 10,
                ..Default::default()
            },
            Event {
                kind: Kind::Power,
                country: 5001,
                instance: 1,
                epoch: 1,
                sequence: 9,
                hour: 24,
                currency: PowerCurrency::Political,
                cause: PowerCause::DailyGain,
                before: 105,
                after: 115,
                requested: Some(10),
                ..Default::default()
            },
        ];
        ledger.assemble(
            1,
            24,
            &captured(
                1,
                0,
                &second_day,
                &[balance(5001), balance(5002), balance(5099)],
            ),
            &mut tables,
        );
        let Some(TableRows::CountryPowerDailyCosts(costs)) =
            rows(&tables[TableId::CountryPowerDailyCosts.index()])
        else {
            panic!()
        };
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].country, 5002);
        assert!(!ledger.pending_daily.contains_key(&5099));
    }
}

#[cfg(test)]
#[path = "politics_encoding_tests.rs"]
mod encoding_tests;
