//! Shared deployed modifier roots carried by the existing tree writers.
use collector_observers::division_panel_changes as operands;
use collector_observers::naval_tree_changes as changes;
use collector_projection::certified::{Snapshot, Store as Rows};
use engine_native::{
    Memory, carmy::DivisionSeed, carmy_leader, ccountry, modifier_breakdown, read,
};
use replay_data::rows::{DivisionPanelSourceRow, SharedDivisionPanelSourceRow};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
type RowSnapshot = Snapshot<SharedDivisionPanelSourceRow>;
const VARIANT_WITHDRAWN: &str = "held variant terrain availability capability expired";
pub struct SourceSnapshot {
    rows: Arc<RowSnapshot>,
    availability: Option<u64>,
    withdrawn: OnceLock<Result<Arc<RowSnapshot>, &'static str>>,
}
impl SourceSnapshot {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    fn current(&self) -> bool {
        self.availability
            .is_some_and(operands::variant_capability_is_current)
            || !self.rows.has_range(key(3, 0, 0)..key(4, 0, 0))
    }
    fn withdrawn(&self) -> Result<&RowSnapshot, &'static str> {
        self.withdrawn
            .get_or_init(|| {
                let mut rows = Rows::from_snapshot(Arc::clone(&self.rows));
                let mut previous = None;
                let mut failure = None;
                self.rows.visit_range(key(3, 0, 0)..key(4, 0, 0), |row| {
                    if row.node.is_none() || previous == Some(row.owner) {
                        return;
                    }
                    previous = Some(row.owner);
                    operands::refused();
                    let result = (|| {
                        rows.remove_range(owner_range(3, row.owner))?;
                        rows.insert(Arc::new(DivisionPanelSourceRow {
                            kind: 3,
                            owner: row.owner,
                            reason: engine_allocation::copy_vec(VARIANT_WITHDRAWN.as_bytes())
                                .map_err(|_| "variant withdrawal allocation failed")?,
                            ..Default::default()
                        }))
                    })();
                    if let Err(reason) = result {
                        failure = Some(reason);
                    }
                });
                if let Some(reason) = failure {
                    return Err(reason);
                }
                rows.finish()
            })
            .as_ref()
            .map(Arc::as_ref)
            .map_err(|&reason| reason)
    }
    /// Validate on either side of projection, including unchanged-row carries.
    /// A reset never erases independent source roots, membership or state terms.
    pub fn publish<T>(&self, project: impl Fn(&RowSnapshot) -> T) -> Result<T, &'static str> {
        if !self.current() {
            return Ok(project(self.withdrawn()?));
        }
        let result = project(&self.rows);
        if self.current() {
            Ok(result)
        } else {
            Ok(project(self.withdrawn()?))
        }
    }
}
struct Pool {
    army: usize,
    certificate: Option<(u64, u64)>,
    count: u32,
    variants: Arc<Vec<(u32, u64, usize)>>,
    seen: u64,
    context: Option<(usize, u32, u32)>,
    context_refusal: Option<&'static str>,
}
struct Variant {
    refs: usize,
    pointer: usize,
    certificate: Option<(u64, u64)>,
    count: u32,
    availability: Option<u64>,
}
struct State {
    pointer: usize,
    certificate: Option<(u64, u64)>,
    seen: u64,
}
struct Held {
    receiver: usize,
    accumulator: usize,
    certificate: Option<(u64, u64)>,
    count: u32,
    seen: u64,
}
pub struct Store {
    roots: BTreeMap<(u8, u64), Held>,
    countries: BTreeMap<u32, (u64, Result<ccountry::Country, &'static str>)>,
    leaders: BTreeMap<usize, (u64, u64)>,
    rows: Rows<SharedDivisionPanelSourceRow>,
    turn: u64,
    world_epoch: u64,
    memo: carmy_leader::LeaderMemo,
    capacity: usize,
    pools: BTreeMap<u64, Pool>,
    variants: BTreeMap<u64, Variant>,
    states: BTreeMap<(u32, u32), State>,
    variant_handles: BTreeMap<usize, u64>,
    provinces: BTreeMap<usize, Option<(u32, usize)>>,
    membership_scratch: Vec<(u32, u64, usize)>,
    membership_changed: bool,
    membership_bytes: usize,
    context_changed: bool,
    dirty: Vec<usize>,
    variant_epoch: Option<u64>,
    retired: Vec<usize>,
    lifetime_epoch: Option<u64>,
    published: Option<Arc<SourceSnapshot>>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            roots: BTreeMap::new(),
            countries: BTreeMap::new(),
            leaders: BTreeMap::new(),
            rows: Rows::default(),
            turn: 0,
            world_epoch: 0,
            memo: carmy_leader::LeaderMemo::vacant(),
            capacity: 0,
            pools: BTreeMap::new(),
            variants: BTreeMap::new(),
            states: BTreeMap::new(),
            variant_handles: BTreeMap::new(),
            provinces: BTreeMap::new(),
            membership_scratch: Vec::new(),
            membership_changed: false,
            membership_bytes: 0,
            context_changed: false,
            dirty: Vec::new(),
            variant_epoch: None,
            retired: Vec::new(),
            lifetime_epoch: None,
            published: None,
        }
    }
}
fn key(kind: u8, owner: u64, ordinal: u32) -> u128 {
    ((kind as u128) << 96) | ((owner as u128) << 32) | ordinal as u128
}
fn owner_range(kind: u8, owner: u64) -> std::ops::Range<u128> {
    let start = key(kind, owner, 0);
    start..start + (1u128 << 32)
}
impl Store {
    pub fn forget(&mut self) {
        self.roots.clear();
        self.countries.clear();
        self.leaders.clear();
        self.rows.clear();
        self.memo.clear();
        self.pools.clear();
        self.membership_bytes = 0;
        self.variants.clear();
        self.states.clear();
        self.variant_handles.clear();
        self.provinces.clear();
        self.variant_epoch = None;
        self.lifetime_epoch = None;
        self.retired.clear();
        self.published = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.retired.capacity() * std::mem::size_of::<usize>()
            + self.memo.allocated_bytes()
            + self.roots.len() * std::mem::size_of::<((u8, u64), Held)>()
            + self.countries.len()
                * std::mem::size_of::<(u32, (u64, Result<ccountry::Country, &'static str>))>()
            + self.leaders.len() * std::mem::size_of::<(usize, (u64, u64))>()
            + self.pools.len() * std::mem::size_of::<(u64, Pool)>()
            + self.membership_bytes
            + self.variants.len() * std::mem::size_of::<(u64, Variant)>()
            + self.variant_handles.len() * std::mem::size_of::<(usize, u64)>()
            + self.states.len() * std::mem::size_of::<((u32, u32), State)>()
            + self.provinces.len() * std::mem::size_of::<(usize, Option<(u32, usize)>)>()
            + self.membership_scratch.capacity() * std::mem::size_of::<(u32, u64, usize)>()
            + self.dirty.capacity() * std::mem::size_of::<usize>()
    }
    fn root(
        &mut self,
        memory: &impl Memory,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
    ) -> Result<(), &'static str> {
        // Shared roots already visited this pass keep the first selection,
        // including its refusal, without another registry lookup.
        if self
            .roots
            .get(&(kind, owner))
            .is_some_and(|held| held.seen == self.turn)
        {
            return Ok(());
        }
        self.root_with_certificate(
            memory,
            kind,
            owner,
            receiver,
            accumulator,
            changes::sequence(receiver),
        )
    }
    fn root_with_certificate(
        &mut self,
        memory: &impl Memory,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
        certificate: Option<(u64, u64)>,
    ) -> Result<(), &'static str> {
        if self
            .roots
            .get(&(kind, owner))
            .is_some_and(|held| held.seen == self.turn)
        {
            return Ok(());
        }
        if let Some(held) = self.roots.get_mut(&(kind, owner))
            && held.certificate == certificate
            && held.receiver == receiver
            && held.accumulator == accumulator
        {
            held.seen = self.turn;
            return Ok(());
        }
        let old_count = self.roots.get(&(kind, owner)).map_or(0, |held| held.count);
        let mut count = 0;
        let result = (|| {
            certificate.ok_or("division modifier source writer certificate unavailable")?;
            modifier_breakdown::source_nodes(memory, accumulator, |ordinal, node| {
                count = ordinal
                    .checked_add(1)
                    .ok_or("division modifier source ordinal overflow")?;
                self.rows.insert(Arc::new(DivisionPanelSourceRow {
                    kind,
                    owner,
                    ordinal,
                    node: Some(node),
                    reason: Vec::new(),
                }))
            })
        })();
        if let Err(reason) = result {
            operands::refused();
            self.rows
                .remove_range(key(kind, owner, 1)..key(kind, owner, old_count.max(count).max(1)))?;
            self.refusal_row(
                kind,
                owner,
                reason,
                "division source refusal allocation failed",
            )?;
            count = 1;
        } else if count < old_count {
            self.rows
                .remove_range(key(kind, owner, count)..key(kind, owner, old_count))?;
        }
        self.roots.insert(
            (kind, owner),
            Held {
                receiver,
                accumulator,
                certificate,
                count,
                seen: self.turn,
            },
        );
        Ok(())
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        selected: &mut Vec<(u64, Option<[u64; 3]>)>,
    ) -> Result<Arc<SourceSnapshot>, &'static str> {
        let availability = operands::variant_capability().unwrap_or(0);
        let world_epoch = operands::world_epoch();
        if self.world_epoch != world_epoch {
            self.forget();
            self.world_epoch = world_epoch;
        }
        self.turn = self.turn.wrapping_add(1);
        selected.clear();
        engine_allocation::reserve(selected, seeds.len())
            .map_err(|_| "division source selection allocation failed")?;
        if seeds.len() > self.capacity {
            self.memo = carmy_leader::LeaderMemo::try_with_capacity(seeds.len())
                .map_err(|_| "division source memo allocation failed")?;
            self.capacity = seeds.len();
        }
        self.memo.clear();
        self.membership_changed = self.pools.len() != seeds.len();
        self.context_changed = self.membership_changed;
        self.dirty.clear();

        self.retire_memberships()?;
        let mut failed = false;
        let operand_epoch = operands::take(|pointer| {
            if engine_allocation::reserve(&mut self.dirty, 1).is_err() {
                failed = true;
            } else {
                self.dirty.push(pointer);
            }
        });
        self.dirty.sort_unstable();

        if failed {
            return Err("attrition dirty ranges allocation failed");
        }
        // A domain epoch change revokes all its held operands; this is a
        // recovery acquisition, never a quiet-hour membership scan.
        let variant_recovery = self.variant_epoch != operand_epoch;
        self.variant_epoch = operand_epoch;
        let countries = ccountry::Reader::new(memory, base, state)?;
        for seed in seeds {
            let selection = (|| {
                let owner = seed
                    .expeditionary_owner
                    .filter(|&owner| (owner as i32) > 0)
                    .or(seed.roster_owner)
                    .ok_or("division source owner unavailable")?;
                let country = match self.countries.get(&owner) {
                    Some(&(turn, country)) if turn == self.turn => country,
                    _ => {
                        let country = countries.resolve(owner);
                        self.countries.insert(owner, (self.turn, country));
                        country
                    }
                };
                let country = country?;
                self.root(
                    memory,
                    0,
                    country.id as u64,
                    country.pointer,
                    country
                        .pointer
                        .checked_add(ccountry::MODIFIER_ACCUMULATOR)
                        .ok_or("division source country overflow")?,
                )?;
                let selection = (|| {
                    let Some((accumulator, hq)) =
                        carmy_leader::movement_block_memoized(memory, base, seed, &mut self.memo)?
                    else {
                        return Ok([country.id as u64, 0, 0]);
                    };
                    let offset = if hq {
                        carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER
                    } else {
                        carmy_leader::MOVEMENT_MODIFIER
                    } + carmy_leader::MODIFIER_ACCUMULATOR;
                    let receiver = accumulator
                        .checked_sub(offset)
                        .ok_or("division source commander overflow")?;
                    let handle = match self.leaders.get(&receiver) {
                        Some(&(turn, handle)) if turn == self.turn => handle,
                        _ => {
                            let handle =
                                read::<u64>(memory, receiver + engine_native::cunit_leader::TYPE)
                                    .ok_or("division source commander identity unreadable")?;
                            self.leaders.insert(receiver, (self.turn, handle));
                            handle
                        }
                    };
                    if handle as u32 != engine_native::cunit_leader::REFERENCE_TYPE {
                        return Err("division source commander identity mismatch");
                    }
                    let kind = if hq { 2 } else { 1 };
                    self.root(memory, kind, handle, receiver, accumulator)?;
                    Ok::<_, &'static str>([country.id as u64, kind as u64, handle])
                })()
                .unwrap_or([country.id as u64, u64::MAX, 0]);
                Ok::<_, &'static str>(selection)
            })();
            selected.push((seed.division, selection.ok()));
            let selection_failure = selection.err();

            self.membership(memory, base, seed)?;
            let context_result = (|| {
                let province =
                    engine_native::division_panel_operands::selected_province(memory, state, seed)?;
                let current = (
                    province,
                    seed.roster_owner.unwrap_or(0),
                    seed.logical_owner.unwrap_or(0),
                );
                if self
                    .pools
                    .get_mut(&seed.division)
                    .unwrap()
                    .context
                    .replace(current)
                    != Some(current)
                {
                    self.context_changed = true;
                }
                let context = match self.provinces.get(&province) {
                    Some(context) => *context,
                    None => {
                        let context = engine_native::division_panel_operands::province_state(
                            memory, base, province,
                        )?;
                        self.provinces.insert(province, context);
                        context
                    }
                };
                if let Some((id, pointer)) = context {
                    for country in [seed.roster_owner, seed.logical_owner]
                        .into_iter()
                        .flatten()
                    {
                        self.state(memory, base, state, id, pointer, country)?;
                    }
                }
                Ok::<_, &'static str>(())
            })();
            self.context_refusal(seed.division, selection_failure.or(context_result.err()))?;
        }
        // Variant pointers touched by a rebuild refresh only that shared
        // operand. New memberships seed their variants in membership().
        if self.membership_changed || self.context_changed {
            self.prune()?;
        }
        if variant_recovery {
            self.dirty.clear();
            engine_allocation::reserve(&mut self.dirty, self.variants.len())
                .map_err(|_| "variant certificate recovery allocation failed")?;
            self.dirty
                .extend(self.variants.values().map(|held| held.pointer));
        }
        for index in 0..self.dirty.len() {
            let pointer = self.dirty[index];
            if let Some(&handle) = self.variant_handles.get(&pointer) {
                self.variant(memory, base, handle, pointer)?;
            }
        }

        operands::demand(self.pools.len(), self.variants.len(), self.states.len());
        changes::division_trees(self.roots.len());
        // Roots no longer demanded publish only their removed ranges.
        let mut gone = Vec::new();
        for (&identity, held) in &self.roots {
            if held.seen != self.turn {
                engine_allocation::reserve(&mut gone, 1)
                    .map_err(|_| "division source removal allocation failed")?;
                gone.push(identity);
            }
        }
        for (kind, owner) in gone {
            let held = self.roots.remove(&(kind, owner)).unwrap();
            self.rows
                .remove_range(key(kind, owner, 0)..key(kind, owner, held.count))?;
        }
        let rows = self.rows.finish()?;
        let availability = Some(availability);
        if let Some(held) = &self.published
            && Arc::ptr_eq(&held.rows, &rows)
            && held.availability == availability
        {
            return Ok(Arc::clone(held));
        }
        let snapshot = Arc::new(SourceSnapshot {
            rows,
            availability,
            withdrawn: OnceLock::new(),
        });
        self.published = Some(Arc::clone(&snapshot));
        Ok(snapshot)
    }
    fn retire_memberships(&mut self) -> Result<(), &'static str> {
        self.retired.clear();
        let mut failed = false;
        let epoch = operands::take_lifetimes(|pointer| {
            if engine_allocation::reserve(&mut self.retired, 1).is_err() {
                failed = true;
            } else {
                self.retired.push(pointer);
            }
        });
        if failed {
            self.lifetime_epoch = None;
            return Err("variant lifetime ranges allocation failed");
        }
        self.retired.sort_unstable();
        let recovery = self.lifetime_epoch != epoch;
        self.lifetime_epoch = epoch;
        if recovery || !self.retired.is_empty() {
            for held in self.pools.values_mut() {
                if recovery
                    || held
                        .variants
                        .iter()
                        .any(|&(_, _, pointer)| self.retired.binary_search(&pointer).is_ok())
                {
                    // The cached address no longer proves a live binding.
                    // membership() authenticates the current pool before any
                    // dirty variant address can be followed, and prune drops
                    // removed bindings before the dirty-operand pass.
                    held.certificate = None;
                }
            }
        }
        Ok(())
    }
    fn context_refusal(
        &mut self,
        division: u64,
        reason: Option<&'static str>,
    ) -> Result<(), &'static str> {
        if self.pools[&division].context_refusal == reason {
            return Ok(());
        }
        if let Some(reason) = reason {
            self.refusal(6, division, reason)?;
        } else {
            self.rows
                .remove_range(key(6, division, 0)..key(6, division, 1))?;
        }
        self.pools.get_mut(&division).unwrap().context_refusal = reason;
        Ok(())
    }
    fn refusal(&mut self, kind: u8, owner: u64, reason: &'static str) -> Result<(), &'static str> {
        if self
            .rows
            .get(key(kind, owner, 0))
            .is_some_and(|row| row.node.is_none() && row.reason == reason.as_bytes())
        {
            return Ok(());
        }
        operands::refused();
        self.refusal_row(
            kind,
            owner,
            reason,
            "division operand refusal allocation failed",
        )
    }
    fn refusal_row(
        &mut self,
        kind: u8,
        owner: u64,
        reason: &'static str,
        allocation_failure: &'static str,
    ) -> Result<(), &'static str> {
        if self
            .rows
            .get(key(kind, owner, 0))
            .is_some_and(|row| row.node.is_none() && row.reason == reason.as_bytes())
        {
            return Ok(());
        }
        self.rows.insert(Arc::new(DivisionPanelSourceRow {
            kind,
            owner,
            reason: engine_allocation::copy_vec(reason.as_bytes())
                .map_err(|_| allocation_failure)?,
            ..Default::default()
        }))
    }
    fn operand(
        &mut self,
        kind: u8,
        owner: u64,
        ordinal: u32,
        values: Vec<engine_native::modifier_breakdown::SourceValue>,
    ) -> Result<(), &'static str> {
        self.rows.insert(Arc::new(DivisionPanelSourceRow {
            kind,
            owner,
            ordinal,
            node: Some(engine_native::modifier_breakdown::SourceNode {
                children: 0,
                name: Vec::new(),
                values,
            }),
            reason: Vec::new(),
        }))
    }
    fn variant(
        &mut self,
        memory: &impl Memory,
        base: usize,
        handle: u64,
        pointer: usize,
    ) -> Result<(), &'static str> {
        let availability = operands::variant_capability();
        let certificate = operands::variant_sequence(pointer);
        if let Some(held) = self.variants.get(&handle)
            && held.certificate == certificate
            && held.pointer == pointer
            && held.availability == availability
            && (availability.is_some_and(operands::variant_capability_is_current)
                || self
                    .rows
                    .get(key(3, handle, 0))
                    .is_some_and(|row| row.node.is_none()))
        {
            return Ok(());
        }
        let old = self.variants.get(&handle).map_or(0, |held| held.count);
        let result = (|| {
            certificate.ok_or("held variant terrain writer certificate unavailable")?;
            engine_native::division_panel_operands::terrain(
                memory,
                base,
                pointer,
                handle,
                |ordinal, raw| {
                    self.operand(
                        3,
                        handle,
                        ordinal,
                        vec![engine_native::modifier_breakdown::SourceValue { key: 0, raw }],
                    )
                },
            )
            .and_then(|count| {
                if availability.is_some_and(operands::variant_capability_is_current)
                    && operands::variant_sequence(pointer) == certificate
                {
                    Ok(count)
                } else {
                    Err(VARIANT_WITHDRAWN)
                }
            })
        })();
        let count = match result {
            Ok(count) => {
                if count < old {
                    self.rows
                        .remove_range(key(3, handle, count)..key(3, handle, old))?;
                }
                count
            }
            Err(reason) => {
                operands::refused();
                // The failed attempt may have emitted more rows than the
                // previous baseline. Withdraw its complete ordinal range.
                self.rows
                    .remove_range(key(3, handle, 1)..owner_range(3, handle).end)?;
                self.refusal_row(3, handle, reason, "attrition refusal allocation failed")?;
                1
            }
        };
        if let Some(old) = self.variants.get(&handle)
            && old.pointer != pointer
            && self.variant_handles.get(&old.pointer) == Some(&handle)
        {
            self.variant_handles.remove(&old.pointer);
        }
        self.variant_handles.insert(pointer, handle);
        self.variants.insert(
            handle,
            Variant {
                refs: self.variants.get(&handle).map_or(0, |held| held.refs),
                pointer,
                certificate,
                count,
                availability,
            },
        );
        Ok(())
    }
    fn membership(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<(), &'static str> {
        let pool = seed
            .pointer
            .checked_add(engine_native::carmy::EQUIPMENT_POOL)
            .ok_or("attrition pool overflow")?;
        let certificate = operands::sequence(pool);
        if let Some(held) = self.pools.get_mut(&seed.division)
            && held.certificate == certificate
            && held.army == seed.pointer
        {
            held.seen = self.turn;
            return Ok(());
        }
        let old_count = self.pools.get(&seed.division).map_or(0, |held| held.count);
        self.membership_scratch.clear();
        let result = (|| {
            certificate.ok_or("held variant membership writer certificate unavailable")?;
            let scratch = &mut self.membership_scratch;
            let count = engine_native::division_panel_operands::held_variants(
                memory,
                base,
                seed,
                |entry| {
                    engine_allocation::reserve(scratch, 1)
                        .map_err(|_| "attrition membership scratch allocation failed")?;
                    scratch.push((
                        entry.source_ordinal,
                        entry.variant_handle,
                        entry.variant_pointer,
                    ));
                    Ok(())
                },
            )?;
            Ok::<_, &'static str>(count + 1)
        })();
        if let (Ok(count), Some(held)) = (result, self.pools.get_mut(&seed.division))
            && held.army == seed.pointer
            && self
                .rows
                .get(key(5, seed.division, 0))
                .is_some_and(|summary| summary.node.is_some())
            && held.variants.as_ref() == &self.membership_scratch
        {
            // Only a successful summary can carry equal membership. A refusal's
            // empty list must be replaced by a newly observed successful empty list.
            // Quantity-only changes have no membership delta or allocation.
            held.certificate = certificate;
            held.count = count;
            held.seen = self.turn;
            return Ok(());
        }
        self.membership_changed = true;
        let mut scratch = std::mem::take(&mut self.membership_scratch);
        let count = match result {
            Ok(count) => {
                let mut current = scratch.iter().peekable();
                for ordinal in 0..old_count.max(count) {
                    if ordinal == 0 {
                        self.operand(5, seed.division, 0, Vec::new())?;
                    } else if current.peek().is_some_and(|(at, _, _)| *at + 1 == ordinal) {
                        let (_, handle, _) = current.next().unwrap();
                        self.operand(
                            5,
                            seed.division,
                            ordinal,
                            vec![engine_native::modifier_breakdown::SourceValue {
                                key: 0,
                                raw: *handle as i64,
                            }],
                        )?;
                    } else {
                        self.rows.remove_range(
                            key(5, seed.division, ordinal)..key(5, seed.division, ordinal + 1),
                        )?;
                    }
                }
                for &(_, handle, pointer) in &scratch {
                    self.variant(memory, base, handle, pointer)?;
                }
                count
            }
            Err(reason) => {
                operands::refused();
                scratch.clear();
                self.rows.remove_range(
                    key(5, seed.division, 1)..key(5, seed.division, old_count.max(1)),
                )?;
                self.refusal_row(
                    5,
                    seed.division,
                    reason,
                    "attrition refusal allocation failed",
                )?;
                1
            }
        };
        if let Some(old) = self.pools.get(&seed.division) {
            for (_, handle, _) in old.variants.iter() {
                if let Some(held) = self.variants.get_mut(handle) {
                    held.refs = held.refs.saturating_sub(1);
                }
            }
        }
        for (_, handle, _) in &scratch {
            if let Some(held) = self.variants.get_mut(handle) {
                held.refs += 1;
            }
        }
        let variants = match self.pools.get(&seed.division) {
            Some(held) if held.variants.as_ref() == &scratch => Arc::clone(&held.variants),
            _ => Arc::new(
                engine_allocation::copy_vec(&scratch)
                    .map_err(|_| "attrition membership ownership allocation failed")?,
            ),
        };
        self.membership_bytes = self.membership_bytes.saturating_sub(
            self.pools.get(&seed.division).map_or(0, |held| {
                held.variants.capacity() * std::mem::size_of::<(u32, u64, usize)>()
            }),
        ) + variants.capacity() * std::mem::size_of::<(u32, u64, usize)>();
        scratch.clear();
        self.membership_scratch = scratch;
        let previous = self.pools.get(&seed.division);
        let context = previous.and_then(|held| held.context);
        let context_refusal = previous.and_then(|held| held.context_refusal);
        self.pools.insert(
            seed.division,
            Pool {
                army: seed.pointer,
                certificate,
                count,
                variants,
                seen: self.turn,
                context,
                context_refusal,
            },
        );
        Ok(())
    }
    fn state(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        id: u32,
        pointer: usize,
        country: u32,
    ) -> Result<(), &'static str> {
        let certificate = operands::state_sequence(id);
        if let Some(held) = self.states.get_mut(&(id, country))
            && held.certificate == certificate
            && held.pointer == pointer
        {
            held.seen = self.turn;
            return Ok(());
        }
        let owner = ((id as u64) << 32) | country as u64;
        let result = (|| {
            certificate.ok_or("state attrition writer certificate unavailable")?;
            let mut values = Vec::new();
            for key in [
                engine_executable::modifier_keys::ATTRITION,
                engine_executable::modifier_keys::ATTRITION_FOR_CONTROLLER,
                engine_executable::modifier_keys::ENEMY_ATTRITION,
            ] {
                let raw = engine_native::cstate::movement_modifier(
                    memory, base, game_state, pointer, country, key,
                )?;
                engine_allocation::reserve(&mut values, 1)
                    .map_err(|_| "state attrition allocation failed")?;
                values.push(engine_native::modifier_breakdown::SourceValue { key, raw });
            }
            self.operand(4, owner, 0, values)
        })();
        if let Err(reason) = result {
            operands::refused();
            self.refusal_row(
                4,
                owner,
                reason,
                "state attrition refusal allocation failed",
            )?;
        }
        self.states.insert(
            (id, country),
            State {
                pointer,
                certificate,
                seen: self.turn,
            },
        );
        Ok(())
    }
    fn prune(&mut self) -> Result<(), &'static str> {
        let mut gone = Vec::new();
        for (&division, held) in &self.pools {
            if held.seen != self.turn {
                engine_allocation::reserve(&mut gone, 1)
                    .map_err(|_| "attrition membership removal allocation failed")?;
                gone.push(division);
            }
        }
        for division in gone {
            let pool = self.pools.remove(&division).unwrap();
            self.membership_bytes = self.membership_bytes.saturating_sub(
                pool.variants.capacity() * std::mem::size_of::<(u32, u64, usize)>(),
            );
            self.rows
                .remove_range(key(6, division, 0)..key(6, division, 1))?;
            self.rows
                .remove_range(key(5, division, 0)..key(5, division, pool.count))?;
            for (_, handle, _) in pool.variants.iter() {
                if let Some(held) = self.variants.get_mut(handle) {
                    held.refs = held.refs.saturating_sub(1);
                }
            }
        }
        let mut gone = Vec::new();
        for (&handle, held) in &self.variants {
            if held.refs == 0 {
                engine_allocation::reserve(&mut gone, 1)
                    .map_err(|_| "attrition variant removal allocation failed")?;
                gone.push(handle);
            }
        }
        for handle in gone {
            let held = self.variants.remove(&handle).unwrap();
            if self.variant_handles.get(&held.pointer) == Some(&handle) {
                self.variant_handles.remove(&held.pointer);
            }
            self.rows
                .remove_range(key(3, handle, 0)..key(3, handle, held.count))?;
        }
        let mut gone = Vec::new();
        for (&identity, held) in &self.states {
            if held.seen != self.turn {
                engine_allocation::reserve(&mut gone, 1)
                    .map_err(|_| "attrition context removal allocation failed")?;
                gone.push(identity);
            }
        }
        for (id, country) in gone {
            self.states.remove(&(id, country));
            let owner = ((id as u64) << 32) | country as u64;
            self.rows.remove_range(key(4, owner, 0)..key(4, owner, 1))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::RowCodec;
    use std::cell::Cell;
    static PROOF: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn current(stamp: u64) -> bool {
        stamp & 1 != 0 && PROOF.load(std::sync::atomic::Ordering::SeqCst) == stamp
    }
    fn observe(enabled: bool) {
        let next = PROOF
            .fetch_add(2, std::sync::atomic::Ordering::SeqCst)
            .wrapping_add(2);
        let stamp = if enabled { next | 1 } else { next & !1 };
        PROOF.store(stamp, std::sync::atomic::Ordering::SeqCst);
        operands::variant_availability(stamp, current);
    }
    static MEMBERSHIP_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    struct Enabled;
    impl Drop for Enabled {
        fn drop(&mut self) {
            operands::patched(false);
            observe(false);
        }
    }
    struct World {
        bytes: Vec<u8>,
        reads: Cell<usize>,
        deny: Cell<bool>,
    }
    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.deny.get() {
                return false;
            }
            let Some(bytes) = self.bytes.get(address..address + output.len()) else {
                return false;
            };
            output.copy_from_slice(bytes);
            true
        }
    }
    #[test]
    fn unchanged_contexts_and_changed_new_removed_divisions_match_fresh_publication() {
        use engine_native::{carmy, cequipment_variant_pool as pool, cgame_state};
        use replay_data::rows::tables;
        let _serial = MEMBERSHIP_TEST.lock().unwrap();
        const DIVISIONS: usize = 5_000;
        const STATE: usize = 0x100;
        const COUNTRY: usize = 0x1000;
        operands::demand(DIVISIONS, 0, 0);
        operands::presize();
        operands::boundary(true);
        operands::patched(true);
        let _enabled = Enabled;
        let mut world = World {
            bytes: vec![0; 0x4000 + DIVISIONS * 0x1000],
            reads: Cell::new(0),
            deny: Cell::new(false),
        };
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        for (header, data) in [
            (STATE + cgame_state::COUNTRIES, 0x800),
            (STATE + cgame_state::COUNTRY_LINKS, 0x900),
        ] {
            put(&mut world.bytes, header, data);
            put(&mut world.bytes, header + 8, 2 | (2 << 32));
        }
        put(&mut world.bytes, 0x808, COUNTRY as u64);
        put(&mut world.bytes, 0x900, 1 << 32);
        put(&mut world.bytes, COUNTRY, ccountry::VTABLE_RVA as u64);
        put(&mut world.bytes, COUNTRY + ccountry::ID, 1);
        let mut seeds: Vec<_> = (0..DIVISIONS)
            .map(|index| {
                let pointer = 0x4000 + index * 0x1000;
                put(
                    &mut world.bytes,
                    pointer + carmy::EQUIPMENT_POOL,
                    pool::VTABLE_RVA as u64,
                );
                DivisionSeed {
                    pointer,
                    division: index as u64 + 1,
                    roster_owner: Some(1),
                    logical_owner: Some(1),
                    orders_group: Some(0),
                    ordinary_route: Some([0; 16]),
                    location: Some(0),
                    ..Default::default()
                }
            })
            .collect();
        let mut store = Store::default();
        let mut selected = Vec::new();
        let first = store
            .capture(&world, 0, STATE, &seeds, &mut selected)
            .unwrap();
        assert_eq!(first.len(), DIVISIONS + 1);
        assert!(
            selected
                .iter()
                .all(|(_, selection)| *selection == Some([1, 0, 0]))
        );
        let complete = |snapshot: &SourceSnapshot| {
            snapshot
                .publish(|rows| rows.complete::<tables::DivisionPanelSources>().unwrap())
                .unwrap()
        };
        let assert_fresh = |snapshot: &SourceSnapshot, seeds: &[DivisionSeed]| {
            let mut fresh = Store::default();
            let mut selection = Vec::new();
            let expected = fresh
                .capture(&world, 0, STATE, seeds, &mut selection)
                .unwrap();
            assert_eq!(complete(snapshot), complete(&expected));
            selection
        };
        assert_eq!(selected, assert_fresh(&first, &seeds));
        let quiet = collector_test_support::with_allocations_denied(|| {
            store
                .capture(&world, 0, STATE, &seeds, &mut selected)
                .unwrap()
        });
        assert!(Arc::ptr_eq(&first, &quiet));

        // Context is still evaluated hourly: a new refusal changes exactly its
        // own row, repeated refusals carry, and recovery removes that row.
        seeds[0].ordinary_route = None;
        let refused = store
            .capture(&world, 0, STATE, &seeds, &mut selected)
            .unwrap();
        assert_eq!(selected, assert_fresh(&refused, &seeds));
        assert_eq!(refused.rows.changed.len(), 1);
        assert_eq!(refused.rows.changed[0].kind, 6);
        assert_eq!(
            refused.rows.changed[0].reason,
            b"attrition ordinary route unavailable"
        );
        let quiet = collector_test_support::with_allocations_denied(|| {
            store
                .capture(&world, 0, STATE, &seeds, &mut selected)
                .unwrap()
        });
        assert!(Arc::ptr_eq(&refused, &quiet));
        seeds[0].ordinary_route = Some([0; 16]);
        let recovered = store
            .capture(&world, 0, STATE, &seeds, &mut selected)
            .unwrap();
        assert_eq!(complete(&recovered), complete(&first));
        assert_eq!(recovered.rows.ranges.len(), 1);
        assert!(recovered.rows.changed.is_empty());

        // A replacement has the retired division's address, not its identity.
        let mut new = seeds.remove(1);
        new.division = DIVISIONS as u64 + 1;
        new.ordinary_route = None;
        seeds.push(new);
        let replaced = store
            .capture(&world, 0, STATE, &seeds, &mut selected)
            .unwrap();
        assert_eq!(selected, assert_fresh(&replaced, &seeds));
        assert!(!store.pools.contains_key(&2));
        assert!(store.rows.get(key(5, 2, 0)).is_none());
        assert_eq!(
            store.pools[&new.division].context_refusal,
            Some("attrition ordinary route unavailable")
        );
        seeds.last_mut().unwrap().roster_owner = None;
        let changed_reason = store
            .capture(&world, 0, STATE, &seeds, &mut selected)
            .unwrap();
        assert_eq!(selected, assert_fresh(&changed_reason, &seeds));
        assert_eq!(changed_reason.rows.changed.len(), 1);
        assert_eq!(
            changed_reason.rows.changed[0].reason,
            b"division source owner unavailable"
        );
        seeds.pop();
        let removed = store
            .capture(&world, 0, STATE, &seeds, &mut selected)
            .unwrap();
        assert_eq!(selected, assert_fresh(&removed, &seeds));
        assert!(store.rows.get(key(5, new.division, 0)).is_none());
        assert!(store.rows.get(key(6, new.division, 0)).is_none());
        assert_eq!(complete(&first), complete(&recovered));
    }

    #[test]
    fn repeated_dirty_refusals_keep_rows_and_membership_allocations_and_refusal_counts() {
        use engine_native::{carmy, cequipment_variant_pool};
        let _serial = MEMBERSHIP_TEST.lock().unwrap();
        operands::demand(1, 0, 0);
        operands::presize();
        operands::boundary(true);
        operands::patched(true);
        let _enabled = Enabled;
        let seed = DivisionSeed {
            pointer: 0x100,
            division: 7,
            ..Default::default()
        };
        let pool = seed.pointer + carmy::EQUIPMENT_POOL;
        let mut world = World {
            bytes: vec![0; 0x1000],
            reads: Cell::new(0),
            deny: Cell::new(true),
        };
        world.bytes[pool..pool + 8]
            .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes());
        let mut store = Store::default();
        store.membership(&world, 0, &seed).unwrap();
        store
            .root_with_certificate(&world, 0, 1, 0x800, 0x900, Some((1, 1)))
            .unwrap();
        let first = store.rows.finish().unwrap();
        let variants = Arc::clone(&store.pools[&seed.division].variants);
        let refused = || {
            serde_json::to_value(operands::status()).unwrap()["refused_operands"]
                .as_u64()
                .unwrap()
        };
        let before = refused();
        drop(operands::notify(pool));
        store.turn += 1;
        world.reads.set(0);
        collector_test_support::with_allocations_denied(|| {
            store.membership(&world, 0, &seed).unwrap();
            store
                .root_with_certificate(&world, 0, 1, 0x800, 0x900, Some((1, 2)))
                .unwrap();
            assert!(Arc::ptr_eq(&first, &store.rows.finish().unwrap()));
        });
        assert!(
            world.reads.get() > 0,
            "dirty failures must still attempt their native reads"
        );
        assert_eq!(refused(), before + 2);
        assert!(Arc::ptr_eq(
            &variants,
            &store.pools[&seed.division].variants
        ));
    }

    #[test]
    fn a_pool_clear_replaces_refused_membership_with_successful_empty_membership() {
        use engine_native::{carmy, cequipment_variant_pool};
        let _serial = MEMBERSHIP_TEST.lock().unwrap();
        operands::demand(1, 0, 0);
        operands::presize();
        operands::boundary(true);
        operands::patched(true);
        let _enabled = Enabled;
        let seed = DivisionSeed {
            pointer: 0x100,
            division: 7,
            ..Default::default()
        };
        let pool = seed.pointer + carmy::EQUIPMENT_POOL;
        let mut bytes = vec![0u8; 0x1000];
        bytes[pool..pool + 8]
            .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes());
        let world = World {
            bytes,
            reads: Cell::new(0),
            deny: Cell::new(true),
        };
        let mut store = Store::default();
        let summary = key(5, seed.division, 0);
        store.membership(&world, 0, &seed).unwrap();
        let refused = store.rows.finish().unwrap();
        assert!(store.rows.get(summary).unwrap().node.is_none());
        assert!(!store.rows.get(summary).unwrap().reason.is_empty());
        assert!(store.pools[&seed.division].variants.is_empty());

        // The real pool writer notification changes the held certificate.
        // Reading the now accessible empty pool must replace its refusal summary.
        drop(operands::notify(pool));
        store.turn += 1;
        world.deny.set(false);
        world.reads.set(0);
        store.membership(&world, 0, &seed).unwrap();
        assert!(world.reads.get() > 0);
        let summary_row = store.rows.get(summary).unwrap();
        assert!(summary_row.node.is_some());
        assert!(summary_row.reason.is_empty());
        assert!(store.pools[&seed.division].variants.is_empty());
        let recovered = store.rows.finish().unwrap();
        assert!(!Arc::ptr_eq(&refused, &recovered));
        assert_eq!(recovered.changed.len(), 1);
        assert_eq!(recovered.changed[0].key(), summary);
        assert!(recovered.changed[0].node.is_some());
        assert_eq!(
            recovered.ranges.as_slice(),
            std::slice::from_ref(&(summary..summary + 1))
        );

        // A subsequent successful empty read still takes the allocation-free
        // equality path, and quiet hours retain the recovered snapshot without reads.
        let membership = Arc::clone(&store.pools[&seed.division].variants);
        drop(operands::notify(pool));
        store.turn += 1;
        store.membership(&world, 0, &seed).unwrap();
        assert!(Arc::ptr_eq(
            &membership,
            &store.pools[&seed.division].variants
        ));
        assert!(Arc::ptr_eq(&recovered, &store.rows.finish().unwrap()));
        store.turn += 1;
        world.deny.set(true);
        world.reads.set(0);
        store.membership(&world, 0, &seed).unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(Arc::ptr_eq(&recovered, &store.rows.finish().unwrap()));
    }

    #[test]
    fn pruning_a_retired_identity_preserves_a_new_identity_at_the_same_address() {
        let mut store = Store::default();
        let pointer = 0x100;
        for (handle, refs) in [(7, 0), (8, 1)] {
            store.variants.insert(
                handle,
                Variant {
                    refs,
                    pointer,
                    certificate: Some((1, 1)),
                    count: 0,
                    availability: None,
                },
            );
        }
        store.variant_handles.insert(pointer, 8);
        store.prune().unwrap();
        assert!(!store.variants.contains_key(&7));
        assert_eq!(store.variants[&8].refs, 1);
        assert_eq!(store.variant_handles.get(&pointer), Some(&8));
    }

    #[test]
    fn first_multi_entry_failures_withdraw_all_ordinals_and_publication_checks_the_rows() {
        use engine_native::{cequipment_variant as variant, division_panel_operands};
        let _serial = MEMBERSHIP_TEST.lock().unwrap();
        operands::demand(0, 1, 0);
        operands::presize();
        operands::boundary(true);
        let _enabled = Enabled;
        for reset in [true, false] {
            observe(true);
            let pointer = 0x100;
            let handle = (19u64 << 32) | variant::REFERENCE_TYPE as u64;
            let values = 0x800;
            let count = if reset { 2u64 } else { 3 };
            let mut world = World {
                bytes: vec![0u8; 0x1000],
                reads: Cell::new(0),
                deny: Cell::new(false),
            };
            let put = |bytes: &mut [u8], at: usize, value: u64| {
                bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
            };
            put(&mut world.bytes, pointer, variant::VTABLE_RVA as u64);
            put(&mut world.bytes, pointer + 8, handle);
            let terrain = pointer + division_panel_operands::VARIANT_TERRAIN_ATTRITION;
            put(&mut world.bytes, terrain, values as u64);
            put(
                &mut world.bytes,
                terrain + engine_native::cpdx_array::CAPACITY,
                (count << 32) | count,
            );
            for ordinal in 0..count {
                put(&mut world.bytes, values + ordinal as usize * 8, ordinal + 5);
            }
            struct StopAt<'a>(&'a World, usize, bool);
            impl Memory for StopAt<'_> {
                fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                    if address == self.1 && !self.2 {
                        return false;
                    }
                    let copied = self.0.copy(address, output);
                    if address == self.1 {
                        PROOF.fetch_add(2, std::sync::atomic::Ordering::SeqCst);
                    }
                    copied
                }
            }
            let mut store = Store::default();
            store.operand(0, 7, 0, Vec::new()).unwrap();
            store
                .variant(
                    &StopAt(&world, values + (count as usize - 1) * 8, reset),
                    0,
                    handle,
                    pointer,
                )
                .unwrap();
            let refusal = store.rows.get(key(3, handle, 0)).unwrap();
            assert!(refusal.node.is_none());
            if reset {
                assert_eq!(refusal.reason, VARIANT_WITHDRAWN.as_bytes());
            }
            for ordinal in 1..count as u32 {
                assert!(
                    store.rows.get(key(3, handle, ordinal)).is_none(),
                    "partial ordinal {ordinal} survived"
                );
            }
            let acquired = SourceSnapshot {
                rows: store.rows.finish().unwrap(),
                availability: None,
                withdrawn: OnceLock::new(),
            };
            let published = acquired
                .publish(|rows| {
                    rows.complete::<replay_data::rows::tables::DivisionPanelSources>()
                        .unwrap()
                })
                .unwrap();
            let replay_data::rows::TableRows::DivisionPanelSources(rows) = published else {
                panic!()
            };
            assert_eq!(rows.iter().filter(|row| row.kind == 3).count(), 1);
            assert!(
                rows.iter()
                    .find(|row| row.kind == 3)
                    .unwrap()
                    .node
                    .is_none()
            );

            // Exercise the publication guard independently of acquisition bookkeeping:
            // a higher terrain row with no admitted baseline must never be exposed.
            store
                .operand(
                    3,
                    handle,
                    1,
                    vec![modifier_breakdown::SourceValue { key: 0, raw: 29 }],
                )
                .unwrap();
            let partial = SourceSnapshot {
                rows: store.rows.finish().unwrap(),
                availability: None,
                withdrawn: OnceLock::new(),
            };
            assert!(!partial.current());
            let published = partial
                .publish(|rows| {
                    rows.complete::<replay_data::rows::tables::DivisionPanelSources>()
                        .unwrap()
                })
                .unwrap();
            let replay_data::rows::TableRows::DivisionPanelSources(rows) = published else {
                panic!()
            };
            assert_eq!(rows.iter().filter(|row| row.kind == 3).count(), 1);
            let refusal = rows.iter().find(|row| row.kind == 3).unwrap();
            assert_eq!(refusal.ordinal, 0);
            assert!(refusal.node.is_none());
            assert_eq!(refusal.reason, VARIANT_WITHDRAWN.as_bytes());
            assert!(
                rows.iter()
                    .find(|row| row.kind == 0)
                    .unwrap()
                    .node
                    .is_some()
            );
            let withdrawn = partial.withdrawn().unwrap().revision;
            assert_eq!(partial.publish(|rows| rows.revision).unwrap(), withdrawn);
        }
    }

    #[test]
    fn source_invalidation_replaces_carried_terrain_and_quiet_values_need_no_reads() {
        use engine_native::{cequipment_variant as variant, division_panel_operands};
        let _serial = MEMBERSHIP_TEST.lock().unwrap();
        operands::demand(0, 1, 0);
        operands::presize();
        operands::boundary(true);
        observe(true);
        let _enabled = Enabled;
        let pointer = 0x100;
        let handle = (19u64 << 32) | variant::REFERENCE_TYPE as u64;
        let values = 0x800;
        let mut world = World {
            bytes: vec![0u8; 0x1000],
            reads: Cell::new(0),
            deny: Cell::new(false),
        };
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        put(&mut world.bytes, pointer, variant::VTABLE_RVA as u64);
        put(&mut world.bytes, pointer + 8, handle);
        let terrain = pointer + division_panel_operands::VARIANT_TERRAIN_ATTRITION;
        put(&mut world.bytes, terrain, values as u64);
        put(
            &mut world.bytes,
            terrain + engine_native::cpdx_array::CAPACITY,
            (1u64 << 32) | 1,
        );
        put(&mut world.bytes, values, 5);
        let mut store = Store::default();
        store.variant(&world, 0, handle, pointer).unwrap();
        assert_eq!(
            store
                .rows
                .get(key(3, handle, 0))
                .unwrap()
                .node
                .as_ref()
                .unwrap()
                .values[0]
                .raw,
            5
        );
        let first = store.rows.finish().unwrap();
        world.deny.set(true);
        world.reads.set(0);
        store.variant(&world, 0, handle, pointer).unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(Arc::ptr_eq(&first, &store.rows.finish().unwrap()));

        operands::variant_invalidation_before();
        store.variant(&world, 0, handle, pointer).unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(store.rows.get(key(3, handle, 0)).unwrap().node.is_none());
        put(&mut world.bytes, values, 8);
        operands::variant_invalidation_after(false);
        world.deny.set(false);
        store.variant(&world, 0, handle, pointer).unwrap();
        assert_eq!(
            store
                .rows
                .get(key(3, handle, 0))
                .unwrap()
                .node
                .as_ref()
                .unwrap()
                .values[0]
                .raw,
            8
        );
        let changed = store.rows.finish().unwrap();
        assert_eq!(
            changed.ranges.as_slice(),
            std::slice::from_ref(&(key(3, handle, 0)..key(3, handle, 1)))
        );
        world.deny.set(true);
        world.reads.set(0);
        store.variant(&world, 0, handle, pointer).unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(Arc::ptr_eq(&changed, &store.rows.finish().unwrap()));

        // Reset after acquisition, without delivering an enabled notification.
        // A suspended publisher may still hand us the old stamp afterward.
        store.operand(0, 7, 0, Vec::new()).unwrap();
        let acquired = SourceSnapshot {
            rows: store.rows.finish().unwrap(),
            availability: operands::variant_capability(),
            withdrawn: OnceLock::new(),
        };
        let stamp = acquired.availability.unwrap();
        let projected = acquired
            .publish(|rows| {
                let result = rows
                    .complete::<replay_data::rows::tables::DivisionPanelSources>()
                    .unwrap();
                PROOF.store(stamp + 2, std::sync::atomic::Ordering::SeqCst);
                result
            })
            .unwrap();
        let replay_data::rows::TableRows::DivisionPanelSources(rows) = projected else {
            panic!()
        };
        assert!(
            rows.iter()
                .find(|row| row.kind == 0)
                .unwrap()
                .node
                .is_some()
        );
        let refused = rows.iter().find(|row| row.kind == 3).unwrap();
        assert!(refused.node.is_none());
        assert_eq!(refused.reason, VARIANT_WITHDRAWN.as_bytes());
        operands::variant_availability(stamp, current);
        assert!(
            operands::variant_capability().is_none(),
            "late delivery cannot revive the proof"
        );
        let withdrawn = acquired.withdrawn().unwrap().revision;
        assert_eq!(acquired.publish(|rows| rows.revision).unwrap(), withdrawn);
        world.reads.set(0);
        store.variant(&world, 0, handle, pointer).unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(store.rows.get(key(3, handle, 0)).unwrap().node.is_none());
        let refused = store.rows.finish().unwrap();
        store.variant(&world, 0, handle, pointer).unwrap();
        assert!(Arc::ptr_eq(&refused, &store.rows.finish().unwrap()));
        observe(true);
        world.deny.set(false);
        store.variant(&world, 0, handle, pointer).unwrap();
        assert!(store.rows.get(key(3, handle, 0)).unwrap().node.is_some());
        assert_eq!(
            acquired.publish(|rows| rows.revision).unwrap(),
            withdrawn,
            "a later baseline cannot revive a prior snapshot"
        );
        struct ResetDuringRead<'a>(&'a World, usize);
        impl Memory for ResetDuringRead<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let copied = self.0.copy(address, output);
                if address == self.1 {
                    PROOF.fetch_add(2, std::sync::atomic::Ordering::SeqCst);
                }
                copied
            }
        }
        operands::variant_invalidation_before();
        operands::variant_invalidation_after(false);
        store
            .variant(&ResetDuringRead(&world, values), 0, handle, pointer)
            .unwrap();
        let refused = store.rows.get(key(3, handle, 0)).unwrap();
        assert!(
            refused.node.is_none(),
            "reset during acquisition must withdraw partial values"
        );
        assert_eq!(refused.reason, VARIANT_WITHDRAWN.as_bytes());
    }

    #[test]
    fn retirement_reauthenticates_only_affected_pools_before_following_variants() {
        use engine_native::{cequipment_variant as variant, cequipment_variant_pool as pool};
        let _serial = MEMBERSHIP_TEST.lock().unwrap();
        operands::demand(2, 1, 0);
        operands::presize();
        operands::boundary(true);
        operands::patched(true);
        observe(true);
        let _enabled = Enabled;
        let seeds = [
            DivisionSeed {
                pointer: 0x100,
                division: 7,
                ..Default::default()
            },
            DivisionSeed {
                pointer: 0x600,
                division: 8,
                ..Default::default()
            },
        ];
        let pointer = 0x1400;
        let handle = (19u64 << 32) | variant::REFERENCE_TYPE as u64;
        let entries = 0xb00;
        let mut world = World {
            bytes: vec![0u8; 0x2000],
            reads: Cell::new(0),
            deny: Cell::new(false),
        };
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        for seed in &seeds {
            put(
                &mut world.bytes,
                seed.pointer + engine_native::carmy::EQUIPMENT_POOL,
                pool::VTABLE_RVA as u64,
            );
        }
        let first_pool = seeds[0].pointer + engine_native::carmy::EQUIPMENT_POOL;
        put(&mut world.bytes, first_pool + pool::ITEMS, entries as u64);
        world.bytes[first_pool + pool::COUNT..first_pool + pool::COUNT + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        world.bytes[first_pool + pool::CAPACITY..first_pool + pool::CAPACITY + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        put(&mut world.bytes, entries, pointer as u64);
        put(&mut world.bytes, entries + 8, 1);
        put(&mut world.bytes, pointer, variant::VTABLE_RVA as u64);
        put(&mut world.bytes, pointer + 8, handle);
        let mut store = Store::default();
        store.retire_memberships().unwrap();
        for seed in &seeds {
            store.membership(&world, 0, seed).unwrap();
        }
        assert_eq!(store.variants[&handle].refs, 1);
        let unrelated = store.pools[&8].certificate;
        let membership = Arc::clone(&store.pools[&8].variants);

        // A consuming native writer leaves a zero occurrence. The cached
        // pointer remains deliberately unreadable after its Before phase.
        operands::variant_before(pointer);
        operands::variant_retiring(pointer);
        put(&mut world.bytes, entries + 8, 0);
        operands::variant_after(pointer, false);
        struct NoRetired<'a>(&'a World, usize);
        impl Memory for NoRetired<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert!(
                    address + output.len() <= self.1 || address >= self.1 + 0x500,
                    "retired storage was dereferenced"
                );
                self.0.copy(address, output)
            }
        }
        store.turn += 1;
        store.retire_memberships().unwrap();
        assert_eq!(store.pools[&7].certificate, None);
        assert_eq!(store.pools[&8].certificate, unrelated);
        store
            .membership(&NoRetired(&world, pointer), 0, &seeds[0])
            .unwrap();
        world.deny.set(true);
        world.reads.set(0);
        store.membership(&world, 0, &seeds[1]).unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(Arc::ptr_eq(&membership, &store.pools[&8].variants));
        store.prune().unwrap();
        assert!(!store.variant_handles.contains_key(&pointer));
        assert!(!store.variants.contains_key(&handle));
        assert!(store.pools[&7].variants.is_empty());
        store.retire_memberships().unwrap();
        store.membership(&world, 0, &seeds[0]).unwrap();
        assert_eq!(world.reads.get(), 0);
    }

    #[test]
    fn a_quiet_shared_root_carries_without_reads_and_one_changed_leaf_publishes_one_range() {
        let mut bytes = vec![0u8; 0x1000];
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        put(&mut bytes, 0x100, 0x800);
        put(&mut bytes, 0x108, 1u64 | (1u64 << 32));
        put(&mut bytes, 0x100 + 0x48 + 24, 15);
        put(&mut bytes, 0x800, 57);
        put(&mut bytes, 0x808, 37);
        let mut world = World {
            bytes,
            reads: Cell::new(0),
            deny: Cell::new(false),
        };
        let mut store = Store::default();
        store
            .root_with_certificate(&world, 0, 7, 0x900, 0x100, Some((1, 1)))
            .unwrap();
        let first = store.rows.finish().unwrap();
        store.turn += 1;
        world.reads.set(0);
        world.deny.set(true);
        store
            .root_with_certificate(&world, 0, 7, 0x900, 0x100, Some((1, 1)))
            .unwrap();
        assert_eq!(world.reads.get(), 0);
        assert!(Arc::ptr_eq(&first, &store.rows.finish().unwrap()));
        store.turn += 1;
        world.deny.set(false);
        put(&mut world.bytes, 0x808, (-7i64) as u64);
        store
            .root_with_certificate(&world, 0, 7, 0x900, 0x100, Some((1, 2)))
            .unwrap();
        let changed = store.rows.finish().unwrap();
        assert_eq!(changed.changed.len(), 1);
        assert_eq!(changed.ranges.len(), 1);
        assert_eq!(changed.changed[0].node.as_ref().unwrap().values[0].raw, -7);
    }
}
