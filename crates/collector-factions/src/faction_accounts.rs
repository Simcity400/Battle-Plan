use crate::faction_companion_changes::Changes;
use engine_allocation as allocation;
use engine_native::{Memory, cfaction_member_status as native, cpdx_array::Array};
use replay_data::rows::{FactionAccountRow, TableRows, tables};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub account: u32,
    pub country: u32,
    pub contribution_subject_gain: Option<i64>,
    pub population_growth: Option<i64>,
    pub influence: Vec<i64>,
    pub projection: Vec<i64>,
    pub projection_sources: [i64; 7],
    pub projection_derived: u8,
    pub influence_numerator_raw: i64,
    pub influence_denominator_raw: i64,
    pub influence_refresh: u32,
    pub influence_faction: u64,
    pub influence_total_phase: u64,
    pub influence_total_raw: i64,
    pub rank_refresh: u64,
    pub rank_faction: u64,
    pub rank_input_raw: i64,
    pub rank_input_order: u32,
    pub rank_members: u32,
    pub projection_numerator_raw: i64,
    pub projection_denominator_raw: i64,
    pub projection_refresh: u32,
    pub influence_share_raw: i64,
    pub projection_share_raw: i64,
    pub projection_rank: i32,
    pub initiative_raw: i64,
    pub initiative_anchor_raw: i64,
    pub initiative_product_raw: i64,
    pub initiative_steps: u32,
    pub contribution_raw: i64,
    pub contribution_gain_a_raw: i64,
    pub contribution_gain_b_raw: i64,
    pub previous_war_score_raw: i64,
    pub upgrade_groups: Vec<u32>,
    pub upgrade_tokens: Vec<u32>,
    pub completed_goals: Vec<u64>,
}
use std::{collections::BTreeMap, sync::Arc};
pub mod mirror;
mod subject;
const FAMILY: usize = 4;
#[derive(Default)]
pub struct Store {
    header: usize,
    array: Array,
    epoch: Option<u64>,
    quiet: Option<(u64, u64, u64)>,
    values: Vec<Account>,
    scratch: native::Snapshot,
    marks: Vec<usize>,
    rank_cohorts: Vec<(u64, u64)>,
    influence_cohorts: Vec<((u64, u64), u32)>,
    failures: BTreeMap<u32, &'static str>,
    rows: Option<Arc<TableRows>>,
    spares: Vec<Arc<TableRows>>,
    failure: Option<&'static str>,
    mirror: mirror::Holder,
    pending: bool,
    factors: Option<Result<[i64; 7], &'static str>>,
    subjects: subject::Store,
}
fn copy<T: Copy>(target: &mut Vec<T>, source: &[T]) -> Result<(), &'static str> {
    allocation::reserve(target, source.len().saturating_sub(target.len()))
        .map_err(|_| "faction account allocation failed")?;
    target.clear();
    target.extend_from_slice(source);
    Ok(())
}
impl Store {
    /// # Safety
    /// Caller holds hourly capture admission (or isolated fixture control) so global
    /// `faction_companion_changes` account demand updates belong to this pass.
    #[cfg(test)]
    pub unsafe fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        force: bool,
        changes: &impl Changes,
    ) {
        // SAFETY: Caller upholds the hourly capture admission contract.
        unsafe { self.acquire_members(memory, base, state, force, changes, &[]) };
    }
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so global `faction_companion_changes` account demand updates belong to this pass.
    pub(crate) unsafe fn acquire_members(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        force: bool,
        changes: &impl Changes,
        members: &[engine_native::cfaction_system::MemberRow],
    ) {
        let retry = self.failure.take().is_some();
        let before = changes.quiet(FAMILY);
        let Some(before) = before else {
            self.failure = Some("faction members' accounts could not be followed in the game");
            return;
        };
        let mut capacity_failed = false;
        let taken = changes.take_accounts(&mut |address| {
            if allocation::reserve(&mut self.marks, 1).is_err() {
                capacity_failed = true;
            } else {
                self.marks.push(address);
            }
        });
        if taken != Some(before.0) || capacity_failed {
            self.epoch = None;
            self.failure = Some("faction member accounting marks unavailable");
            return;
        }
        self.marks.sort_unstable();
        self.marks.dedup();
        let baseline = force
            || retry
            || self.mirror.needs_baseline()
            || self.epoch != Some(before.0)
            || self.marks.contains(&self.header);
        let scalar_before = if baseline {
            mirror::baseline_stamp()
        } else {
            mirror::quiet()
        };
        if scalar_before.is_none() {
            self.failure = Some(
                "faction members' values could not be followed in the game, or changed while read",
            );
            return;
        }
        if baseline {
            self.subjects.invalidate();
            self.factors = None;
            self.epoch = None;
        }
        let mut changed = baseline || self.pending;
        let result = (|| {
            if baseline {
                let _reads = collector_memory::read_accounting::table(
                    replay_data::rows::TableId::FactionAccounts,
                );
                let system = state
                    .checked_add(engine_native::cgame_state::FACTION_SYSTEM)
                    .and_then(|at| engine_native::read::<u64>(memory, at))
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value != 0)
                    .ok_or("faction member accounting system unavailable")?;
                let vtable = engine_sites::at(base, engine_native::cfaction_system::VTABLE_RVA)
                    .ok_or("faction member accounting system vtable mismatch")?;
                if engine_native::read::<u64>(memory, system) != Some(vtable as u64) {
                    return Err("faction member accounting system vtable mismatch");
                }
                self.header = system
                    .checked_add(native::ACCOUNTS)
                    .ok_or("faction member array address overflow")?;
                self.array = Array::read(memory, self.header, native::STRIDE)
                    .ok_or("faction member array invalid")?;
                // SAFETY: Admitted faction-accounts capture records measured account demand.
                unsafe {
                    crate::faction_companion_changes::demand_accounts(
                        self.array.count as usize * 2 + 1,
                    );
                }
                changes
                    .sequence(FAMILY, self.header)
                    .ok_or("faction member array registration unavailable")?;
                allocation::resize_default(&mut self.values, self.array.count as usize)
                    .map_err(|_| "faction member account allocation failed")?;
                self.failures.clear();
                for index in 0..self.array.count {
                    self.read(memory, base, index, changes, true)?;
                }
            } else {
                for at in 0..self.marks.len() {
                    let address = self.marks[at];
                    if let Some(offset) = address.checked_sub(self.array.data) {
                        let lane = offset % native::STRIDE;
                        if matches!(lane, 0 | native::UPGRADES)
                            && let Ok(index) = u32::try_from(offset / native::STRIDE)
                            && index < self.array.count
                        {
                            self.read(memory, base, index, changes, lane == 0)?;
                            if lane == 0 {
                                self.mirror
                                    .storage()?
                                    .reset(index as usize, &self.values[index as usize])?;
                            }
                            changed = true;
                        }
                    }
                }
            }
            if baseline {
                self.mirror.install(mirror::Storage::new(
                    self.array.data,
                    native::STRIDE,
                    &self.values,
                )?)?;
            }
            changed |= self.mirror.storage()?.drain(&mut self.values)?;
            let _reads = collector_memory::read_accounting::table(
                replay_data::rows::TableId::FactionAccounts,
            );
            changed |= self
                .subjects
                .capture(memory, base, state, members, &mut self.values);
            self.pending |= changed;
            if mirror::quiet() != scalar_before {
                return Err("faction member scalar writer changed during acquisition");
            }
            if changes.quiet(FAMILY) != Some(before) {
                return Err("faction member writer changed during acquisition");
            }
            self.marks.clear();
            self.epoch = Some(before.0);
            self.quiet = Some(before);
            if changed || self.rows.is_none() {
                if self.factors.is_none()
                    && self.values.iter().any(|row| row.projection_derived != 0)
                {
                    let _reads = collector_memory::read_accounting::table(
                        replay_data::rows::TableId::FactionAccounts,
                    );
                    self.factors = Some(native::projection_factors(memory, base));
                }
                if let Some(Err(why)) = self.factors {
                    return Err(why);
                }
                self.publish()?;
                self.pending = false;
            }
            self.epoch = Some(before.0);
            self.quiet = Some(before);
            if let Some(&reason) = self.failures.values().next() {
                return Err(reason);
            }
            Ok(())
        })();
        if let Err(reason) = result {
            self.failure = Some(reason);
        }
    }
    fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        index: u32,
        changes: &impl Changes,
        balances: bool,
    ) -> Result<(), &'static str> {
        let address = self
            .array
            .element(index, native::STRIDE)
            .ok_or("faction member account address invalid")?;
        let receiver = if balances {
            address
        } else {
            address + native::UPGRADES
        };
        changes
            .sequence(FAMILY, address + native::UPGRADES)
            .ok_or("faction member selection registration unavailable")?;
        let before = changes
            .sequence(FAMILY, receiver)
            .ok_or("faction member account registration unavailable")?;
        let _reads =
            collector_memory::read_accounting::table(replay_data::rows::TableId::FactionAccounts);
        let result = if balances {
            native::read_into(memory, base, self.array, index, &mut self.scratch)
        } else {
            native::read_selections_into(memory, base, self.array, index, &mut self.scratch)
        };
        if let Err(reason) = result {
            self.failures.insert(index, reason);
            return Ok(());
        }
        if changes.sequence(FAMILY, receiver) != Some(before) {
            return Err("faction member account changed during acquisition");
        }
        let row = &mut self.values[index as usize];
        let value = &self.scratch;
        if balances {
            row.influence_refresh = 0;
            row.influence_faction = 0;
            row.rank_refresh = 0;
            row.influence_total_phase = 0;
            row.influence_total_raw = 0;
            row.projection_refresh = 0;
            row.projection_sources = [0; 7];
            row.projection_derived = 0;
            row.account = index;
            row.country = value.country;
            row.influence_share_raw = value.influence_share_raw;
            row.projection_share_raw = value.power_projection_share_raw;
            row.projection_rank = value.power_projection_rank;
            copy(&mut row.projection, &value.power_projection)?;
            row.initiative_raw = value.initiative_raw;
            row.contribution_raw = value.contribution_raw;
            row.contribution_gain_a_raw = value.contribution_gain[0];
            row.contribution_gain_b_raw = value.contribution_gain[1];
            row.previous_war_score_raw = value.previous_war_score_raw;
            copy(&mut row.influence, &value.influence)?;
        }
        allocation::resize_default(&mut row.upgrade_groups, value.upgrades.len())
            .map_err(|_| "faction member upgrade allocation failed")?;
        allocation::resize_default(&mut row.upgrade_tokens, value.upgrades.len())
            .map_err(|_| "faction member upgrade allocation failed")?;
        for (at, upgrade) in value.upgrades.iter().enumerate() {
            row.upgrade_groups[at] = upgrade.group;
            row.upgrade_tokens[at] = upgrade.token;
        }
        allocation::resize_default(&mut row.completed_goals, value.completed_goals.len())
            .map_err(|_| "faction member goal allocation failed")?;
        for (at, goal) in value.completed_goals.iter().enumerate() {
            row.completed_goals[at] = goal.map(u64::from).unwrap_or(u64::MAX);
        }
        if balances {
            row.initiative_anchor_raw = row.initiative_raw;
            row.initiative_product_raw = 0;
            row.initiative_steps = 0;
        }
        self.failures.remove(&index);
        Ok(())
    }
    fn publish(&mut self) -> Result<(), &'static str> {
        let additional = self.values.len().saturating_sub(self.rank_cohorts.len());
        allocation::reserve(&mut self.rank_cohorts, additional)
            .map_err(|_| "faction rank cohort allocation failed")?;
        let additional = self
            .values
            .len()
            .saturating_sub(self.influence_cohorts.len());
        allocation::reserve(&mut self.influence_cohorts, additional)
            .map_err(|_| "faction influence cohort index allocation failed")?;
        self.influence_cohorts.clear();
        self.influence_cohorts.extend(
            self.values
                .iter()
                .filter(|row| row.influence_total_phase != 0)
                .map(|row| {
                    (
                        (row.influence_faction, row.influence_total_phase),
                        row.account,
                    )
                }),
        );
        self.influence_cohorts.sort_unstable();
        self.rank_cohorts.clear();
        self.rank_cohorts.extend(
            self.values
                .iter()
                .filter(|row| row.rank_refresh != 0)
                .map(|row| (row.rank_faction, row.rank_refresh)),
        );
        self.rank_cohorts.sort_unstable();
        let mut table = crate::faction_goals::writable(&mut self.rows, &mut self.spares, || {
            TableRows::FactionAccounts(Vec::new())
        })?;
        let rows = Arc::get_mut(&mut table)
            .unwrap()
            .rows_mut::<tables::FactionAccounts>()
            .unwrap();
        let count = self
            .values
            .iter()
            .try_fold(0usize, |n, row| {
                n.checked_add(
                    1 + row.influence.len()
                        + row.projection.len()
                        + if row.projection_derived != 0 { 7 } else { 0 },
                )
            })
            .ok_or("faction account occurrence count overflow")?;
        let count = count
            + if self.factors.is_some() && !self.values.is_empty() {
                7
            } else {
                0
            };
        allocation::resize_default(rows, count)
            .map_err(|_| "faction account occurrence allocation failed")?;
        let mut at = 0;
        for source in &self.values {
            let target = &mut rows[at];
            target.account = source.account;
            target.kind = 0;
            target.ordinal = 0;
            target.statistic_raw = 0;
            target.country = source.country;
            target.contribution_subject_gain = source.contribution_subject_gain;
            target.population_growth = source.population_growth;
            target.initiative_raw = source.initiative_anchor_raw;
            target.initiative_product_raw = source.initiative_product_raw;
            target.initiative_steps = source.initiative_steps;
            target.contribution_raw = source.contribution_raw;
            target.contribution_gain_a_raw = source.contribution_gain_a_raw;
            target.contribution_gain_b_raw = source.contribution_gain_b_raw;
            target.previous_war_score_raw = source.previous_war_score_raw;
            target.influence_share_raw = if source.influence_refresh == 0 {
                source.influence_share_raw
            } else {
                0
            };
            target.projection_share_raw = if source.projection_refresh == 0 {
                source.projection_share_raw
            } else {
                0
            };
            let current = crate::faction_pools::rank_current(source.rank_faction);
            let key = (source.rank_faction, source.rank_refresh);
            let count = self.rank_cohorts.partition_point(|value| *value <= key)
                - self.rank_cohorts.partition_point(|value| *value < key);
            let derived = source.rank_refresh != 0
                && count == source.rank_members as usize
                && current == Some((source.rank_refresh, source.rank_members));
            target.projection_rank = if derived { 0 } else { source.projection_rank };
            target.rank_derived = u8::from(derived);
            target.rank_faction = source.rank_faction;
            target.rank_input_raw = source.rank_input_raw;
            target.rank_input_order = source.rank_input_order;
            target.rank_members = source.rank_members;
            target.influence_faction = source.influence_faction;
            target.influence_supplied = u8::from(source.influence_total_phase != 0);
            target.influence_total_raw = source.influence_total_raw;
            target.projection_derived = source.projection_derived;
            target.influence_numerator_raw = source.influence_numerator_raw;
            target.influence_denominator_raw = source.influence_denominator_raw;
            target.influence_refresh = u32::from(source.influence_refresh != 0);
            target.projection_numerator_raw = source.projection_numerator_raw;
            target.projection_denominator_raw = source.projection_denominator_raw;
            target.projection_refresh = u32::from(source.projection_refresh != 0);
            copy(&mut target.upgrade_groups, &source.upgrade_groups)?;
            copy(&mut target.upgrade_tokens, &source.upgrade_tokens)?;
            copy(&mut target.completed_goals, &source.completed_goals)?;
            at += 1;
            for (kind, values) in [
                (1, source.influence.as_slice()),
                (2, source.projection.as_slice()),
                (
                    3,
                    if source.projection_derived != 0 {
                        source.projection_sources.as_slice()
                    } else {
                        &[]
                    },
                ),
                (
                    4,
                    if source.account == 0 {
                        self.factors
                            .as_ref()
                            .and_then(|value| value.as_ref().ok())
                            .map(<[i64; 7]>::as_slice)
                            .unwrap_or_default()
                    } else {
                        &[]
                    },
                ),
            ] {
                for (ordinal, &raw) in values.iter().enumerate() {
                    let target = &mut rows[at];
                    target.upgrade_groups.clear();
                    target.upgrade_tokens.clear();
                    target.completed_goals.clear();
                    let groups = std::mem::take(&mut target.upgrade_groups);
                    let tokens = std::mem::take(&mut target.upgrade_tokens);
                    let completed = std::mem::take(&mut target.completed_goals);
                    *target = FactionAccountRow {
                        account: source.account,
                        kind,
                        ordinal: ordinal as u32,
                        statistic_raw: if kind == 2
                            && ordinal < 4
                            && source.projection_derived & (1 << ordinal) != 0
                        {
                            0
                        } else {
                            raw
                        },
                        upgrade_groups: groups,
                        upgrade_tokens: tokens,
                        completed_goals: completed,
                        ..Default::default()
                    };
                    at += 1;
                }
            }
        }
        self.rows = Some(table);
        Ok(())
    }
    pub fn influence_cohort(&self, faction: u64, phase: u64) -> Option<&[((u64, u64), u32)]> {
        if self.failure.is_some() {
            return None;
        }
        let key = (faction, phase);
        let first = self.influence_cohorts.partition_point(|row| row.0 < key);
        let end = self.influence_cohorts.partition_point(|row| row.0 <= key);
        Some(&self.influence_cohorts[first..end])
    }
    pub fn output(&self) -> (Option<Arc<TableRows>>, Option<&'static str>) {
        (
            self.failure
                .is_none()
                .then(|| self.rows.as_ref().map(Arc::clone))
                .flatten(),
            self.failure,
        )
    }
    pub fn allocated_bytes(&self) -> usize {
        self.subjects.allocated_bytes()
            + self.mirror.allocated_bytes()
            + (self.scratch.influence.capacity() + self.scratch.power_projection.capacity()) * 8
            + self.scratch.upgrades.capacity() * size_of::<native::Upgrade>()
            + self.scratch.completed_goals.capacity() * size_of::<Option<u32>>()
            + self.spares.capacity() * size_of::<Arc<TableRows>>()
            + self.values.capacity() * size_of::<Account>()
            + self.marks.capacity() * size_of::<usize>()
            + self.rank_cohorts.capacity() * size_of::<(u64, u64)>()
            + self.influence_cohorts.capacity() * size_of::<((u64, u64), u32)>()
            + self
                .values
                .iter()
                .map(|row| {
                    (row.influence.capacity() + row.projection.capacity()) * 8
                        + (row.upgrade_groups.capacity() + row.upgrade_tokens.capacity()) * 4
                        + row.completed_goals.capacity() * 8
                })
                .sum::<usize>()
            + self
                .rows
                .iter()
                .chain(&self.spares)
                .map(|rows| size_of::<TableRows>() + rows.allocated_bytes())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_certificates::watch_registry::WatchRegistry;
    use std::cell::Cell;
    const STATE: usize = 0x1000;
    const SYSTEM: usize = 0x2000;
    const ACCOUNTS: usize = 0x3000;
    pub(super) fn alias_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x7000];
        put(
            &mut bytes,
            STATE + engine_native::cgame_state::COUNTRIES,
            0x2000,
        );
        put(
            &mut bytes,
            STATE + engine_native::cgame_state::COUNTRIES + 8,
            (10u64 << 32) | 10,
        );
        put(
            &mut bytes,
            STATE + engine_native::cgame_state::COUNTRY_LINKS,
            0x2100,
        );
        put(
            &mut bytes,
            STATE + engine_native::cgame_state::COUNTRY_LINKS + 8,
            (10u64 << 32) | 10,
        );
        put(&mut bytes, 0x2000 + 3 * 8, 0x4000);
        for raw in [3, 5, 9] {
            bytes[0x2100 + raw * 4..0x2104 + raw * 4].copy_from_slice(&3u32.to_le_bytes());
        }
        put(
            &mut bytes,
            0x4000,
            engine_native::ccountry::VTABLE_RVA.0 as u64,
        );
        bytes[0x4000 + engine_native::ccountry::ID..0x4004 + engine_native::ccountry::ID]
            .copy_from_slice(&9u32.to_le_bytes());
        let accumulator = 0x4000 + engine_native::ccountry::MODIFIER_ACCUMULATOR;
        put(&mut bytes, accumulator, 0x6000);
        bytes[accumulator + engine_native::modifier_accumulator::COUNT
            ..accumulator + engine_native::modifier_accumulator::COUNT + 4]
            .copy_from_slice(&2i32.to_le_bytes());
        for (ordinal, (key, raw)) in [(0x48u32, 25_000i64), (0x288u32, -17_000i64)]
            .into_iter()
            .enumerate()
        {
            let at = 0x6000 + ordinal * 16;
            bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
        }
        bytes
    }
    struct Observed(WatchRegistry);
    impl Changes for Observed {
        fn take_accounts(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
            self.0.take(visit)
        }
        fn quiet(&self, _: usize) -> Option<(u64, u64, u64)> {
            self.0.quiet().map(|(a, b)| (a, b, 0))
        }
        fn sequence(&self, _: usize, receiver: usize) -> Option<(u64, u64, u64)> {
            self.0.sequence(receiver).map(|(a, b)| (a, b, 0))
        }
    }
    struct Counted<'a> {
        bytes: &'a [u8],
        reads: Cell<usize>,
    }
    impl Memory for Counted<'_> {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            engine_native::memory::SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, out)
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture(count: usize) -> Vec<u8> {
        let mut bytes = vec![0; ACCOUNTS + count * native::STRIDE];
        put(
            &mut bytes,
            STATE + engine_native::cgame_state::FACTION_SYSTEM,
            SYSTEM as u64,
        );
        put(
            &mut bytes,
            SYSTEM,
            engine_native::cfaction_system::VTABLE_RVA.0 as u64,
        );
        put(&mut bytes, SYSTEM + native::ACCOUNTS, ACCOUNTS as u64);
        put(
            &mut bytes,
            SYSTEM + native::ACCOUNTS + 8,
            (count as u64) << 32 | count as u64,
        );
        for index in 0..count {
            let at = ACCOUNTS + index * native::STRIDE;
            put(&mut bytes, at, index as u64);
            put(&mut bytes, at + native::INITIATIVE, index as u64 + 10);
        }
        bytes
    }
    fn observed(count: usize) -> Observed {
        let watches = WatchRegistry::marking();
        watches.offer(count * 2 + 1);
        watches.boundary(false);
        watches.set_enabled(true);
        Observed(watches)
    }
    fn statistics(bytes: &mut Vec<u8>, account: usize, offset: usize, values: &[i64]) {
        let data = bytes.len();
        bytes.resize(data + values.len() * 8, 0);
        let header = ACCOUNTS + account * native::STRIDE + offset;
        put(bytes, header, data as u64);
        put(
            bytes,
            header + 8,
            (values.len() as u64) << 32 | values.len() as u64,
        );
        for (ordinal, &value) in values.iter().enumerate() {
            put(bytes, data + ordinal * 8, value as u64);
        }
    }

    #[test]
    fn new_faction_ranks_publish_every_account_from_the_first_hour_without_a_pool_certificate() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = fixture(1500);
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = observed(1500);
        let mut store = Store::default();
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe { store.acquire(&memory, 0, STATE, false, &changes) };
        let reads = memory.reads.get();
        for hour in 0..3 {
            collector_test_support::with_allocations_denied(|| {
                crate::faction_pools::phase_call(0xdead0000, 1, || {
                    assert_eq!(crate::faction_pools::phase(1), None);
                    let _flight = mirror::Flight::new();
                    for account in 0..1500 {
                        let address = ACCOUNTS + account * native::STRIDE;
                        mirror::rank_receiver(address);
                        mirror::rank_input(900, account as u32);
                        mirror::rank_commit(address, (account + 1 + hour) as i32);
                    }
                    0
                });
            });
            // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe { store.acquire(&memory, 0, STATE, false, &changes) };
            let (rows, failure) = store.output();
            assert_eq!(failure, None);
            let rows = rows.unwrap();
            let rows = rows.rows::<tables::FactionAccounts>().unwrap();
            assert_eq!(rows.len(), 1500);
            for (account, row) in rows.iter().enumerate() {
                assert_eq!(row.projection_rank, (account + 1 + hour) as i32);
                assert_eq!(row.rank_derived, 0);
                assert_eq!(row.rank(rows).unwrap(), row.projection_rank);
            }
        }
        assert_eq!(memory.reads.get(), reads);
    }

    #[test]
    fn scalar_refusals_rebaseline_exactly_and_retry_unreadable_hours_without_forcing_a_keyframe() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for fault in 0..3 {
            for unreadable_hours in [0, 2] {
                let mut bytes = fixture(2);
                statistics(&mut bytes, 1, native::INFLUENCE, &[1, 2, 3, 4, 5]);
                statistics(&mut bytes, 1, native::POWER_PROJECTION, &[6, 7, 8, 9]);
                let changes = observed(2);
                let mut store = Store::default();
                let acquire = |store: &mut Store, bytes: &[u8]| {
                    // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
                    unsafe {
                        store.acquire(
                            &engine_native::memory::SliceMemory { base: 0, bytes },
                            0,
                            STATE,
                            false,
                            &changes,
                        )
                    }
                };
                acquire(&mut store, &bytes);
                let address = ACCOUNTS + native::STRIDE;
                collector_test_support::with_allocations_denied(|| {
                    let _flight = mirror::Flight::new();
                    match fault {
                        0 => mirror::observe(address, true, 5, 99, mirror::Operation::Set),
                        1 => {
                            mirror::projection_operand(5, 123);
                            mirror::projection_sources(address, 3, 999);
                        }
                        _ => mirror::refuse(address),
                    }
                });
                let expected = match fault {
                    0 => "faction operand writer selected an invalid statistic",
                    1 => "faction projection setter lacks its supplying getter operands",
                    _ => "faction share refresh supplying denominator unreadable",
                };
                assert_eq!(
                    store.mirror.storage().unwrap().drain(&mut store.values),
                    Err(expected)
                );
                assert!(store.mirror.needs_baseline());
                statistics(&mut bytes, 1, native::INFLUENCE, &[11, 12, 13, 14, 15, 16]);
                statistics(&mut bytes, 1, native::POWER_PROJECTION, &[21, 22, 23, 24]);
                put(&mut bytes, address + native::CONTRIBUTION, 777);
                put(&mut bytes, address + native::INITIATIVE, 888);
                put(&mut bytes, address + native::INFLUENCE_SHARE, 54321);
                put(&mut bytes, address + native::POWER_PROJECTION_SHARE, 12345);
                bytes[address + native::POWER_PROJECTION_RANK
                    ..address + native::POWER_PROJECTION_RANK + 4]
                    .copy_from_slice(&2i32.to_le_bytes());
                for _ in 0..unreadable_hours {
                    bytes[SYSTEM] ^= 1;
                    acquire(&mut store, &bytes);
                    assert_eq!(
                        store.output().1,
                        Some("faction member accounting system vtable mismatch")
                    );
                    assert!(store.output().0.is_none());
                    bytes[SYSTEM] ^= 1;
                }
                acquire(&mut store, &bytes);
                assert_eq!(store.output().1, None);
                let rows = store.output().0.unwrap();
                let rows = rows.rows::<tables::FactionAccounts>().unwrap();
                assert_eq!(
                    FactionAccountRow::statistics(rows, 1, 1).collect::<Vec<_>>(),
                    [11, 12, 13, 14, 15, 16]
                );
                let row = rows
                    .iter()
                    .find(|row| row.account == 1 && row.kind == 0)
                    .unwrap();
                assert_eq!(row.projection(rows).unwrap(), [21, 22, 23, 24]);
                assert_eq!(row.contribution_raw, 777);
                assert_eq!(row.initiative(), 888);
                assert_eq!(row.influence_share(), 54321);
                assert_eq!(row.projection_share(), 12345);
                assert_eq!(row.rank(rows).unwrap(), 2);
                assert!(!store.mirror.needs_baseline());
                let memory = Counted {
                    bytes: &bytes,
                    reads: Cell::new(0),
                };
                let held = Arc::clone(store.rows.as_ref().unwrap());
                // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe { store.acquire(&memory, 0, STATE, false, &changes) };
                assert_eq!(store.output().1, None);
                assert_eq!(memory.reads.get(), 0);
                assert!(Arc::ptr_eq(&held, store.rows.as_ref().unwrap()));
            }
        }
    }

    #[test]
    fn accounting_carries_accounts_without_native_reads_or_allocations_and_selects_one_writer() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = fixture(40);
        let memory = Counted {
            bytes: &bytes,
            reads: Cell::new(0),
        };
        let changes = observed(40);
        let mut store = Store::default();
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe { store.acquire(&memory, 0, STATE, false, &changes) };
        assert_eq!(store.failure, None);
        let first = memory.reads.get();
        let held = Arc::clone(store.rows.as_ref().unwrap());
        collector_test_support::with_allocations_denied(|| {
            // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
            unsafe { store.acquire(&memory, 0, STATE, false, &changes) }
        });
        assert_eq!(memory.reads.get(), first);
        assert!(Arc::ptr_eq(&held, store.rows.as_ref().unwrap()));
        drop(changes.0.notify(ACCOUNTS + 39 * native::STRIDE));
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe { store.acquire(&memory, 0, STATE, false, &changes) };
        assert_eq!(store.failure, None);
        assert!(memory.reads.get() > first && memory.reads.get() - first < 15);
        let next = memory.reads.get();
        drop(changes.0.notify(0x100));
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe { store.acquire(&memory, 0, STATE, false, &changes) };
        assert_eq!(store.failure, None);
        assert_eq!(memory.reads.get(), next);
        assert_eq!(store.values[39].initiative_raw, 49);
    }
    #[test]
    fn drained_reset_selection_and_array_marks_survive_scalar_overlap() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for receiver in [
            ACCOUNTS,
            ACCOUNTS + native::UPGRADES,
            SYSTEM + native::ACCOUNTS,
        ] {
            let mut bytes = fixture(2);
            let changes = observed(2);
            let mut store = Store::default();
            let acquire = |store: &mut Store, bytes: &[u8]| {
                // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
                unsafe {
                    store.acquire(
                        &engine_native::memory::SliceMemory { base: 0, bytes },
                        0,
                        STATE,
                        false,
                        &changes,
                    )
                }
            };
            acquire(&mut store, &bytes);
            let flight = mirror::Flight::new();
            drop(changes.0.notify(receiver));
            acquire(&mut store, &bytes);
            assert!(store.failure.is_some());
            assert_eq!(store.marks, vec![receiver]);
            drop(flight);
            if receiver == SYSTEM + native::ACCOUNTS {
                put(&mut bytes, SYSTEM + native::ACCOUNTS + 8, (1u64 << 32) | 1);
            } else if receiver == ACCOUNTS {
                put(&mut bytes, ACCOUNTS + native::CONTRIBUTION, 777);
            } else {
                let payload = bytes.len();
                bytes.resize(payload + 8, 0);
                put(
                    &mut bytes,
                    ACCOUNTS + native::COMPLETED_GOALS,
                    payload as u64,
                );
                put(
                    &mut bytes,
                    ACCOUNTS + native::COMPLETED_GOALS + 8,
                    (1u64 << 32) | 1,
                );
            }
            acquire(&mut store, &bytes);
            assert_eq!(store.failure, None);
            assert!(store.marks.is_empty());
            if receiver == SYSTEM + native::ACCOUNTS {
                assert_eq!(store.values.len(), 1);
            } else if receiver == ACCOUNTS {
                assert_eq!(store.values[0].contribution_raw, 777);
            } else {
                assert_eq!(store.values[0].completed_goals, vec![u64::MAX]);
            }
        }
    }
    #[test]
    fn array_lifetime_notifications_rebuild_the_registered_array_and_world_epochs_reauthenticate() {
        let _lock = crate::faction_companion_changes::TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut bytes = fixture(2);
        let changes = observed(2);
        let mut store = Store::default();
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe {
            store.acquire(
                &engine_native::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                STATE,
                false,
                &changes,
            );
        }
        put(&mut bytes, ACCOUNTS + native::CONTRIBUTION, 777);
        drop(changes.0.notify(SYSTEM + native::ACCOUNTS));
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe {
            store.acquire(
                &engine_native::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                STATE,
                false,
                &changes,
            );
        }
        assert_eq!(store.values[0].contribution_raw, 777);
        changes.0.boundary(true);
        bytes[SYSTEM] ^= 1;
        // SAFETY: Unit test holds `TEST_LOCK` and owns this local fixture store.
        unsafe {
            store.acquire(
                &engine_native::memory::SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                STATE,
                false,
                &changes,
            );
        }
        assert_eq!(
            store.failure,
            Some("faction member accounting system vtable mismatch")
        );
    }
}
