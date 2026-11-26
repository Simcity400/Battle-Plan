use crate::clock::qpc;
use collector_observers::division_panel_changes as operands;

mod owned;
mod preparation;
use preparation::NativeReads;

pub const PHASE_NAMES: [&str; 4] = [
    "sources_ordered",
    "sources_prune",
    "sources_variants",
    "sources_publish",
];
use collector_observers::naval_tree_changes as changes;
use collector_projection::certified::{Snapshot, Store as Rows};
use engine_native_memory::{Memory, read};
use engine_native::{carmy::DivisionSeed, carmy_leader, ccountry};
use engine_native_politics::modifier_breakdown;
use replay_data::rows::{DivisionPanelSourceRow, SharedDivisionPanelSourceRow};
use std::sync::{Arc, OnceLock};
type RowSnapshot = Snapshot<SharedDivisionPanelSourceRow>;
type Map<K, V> = std::collections::HashMap<K, V, foldhash::fast::FixedState>;
fn table_bytes<K, V>(capacity: usize) -> usize {
    const GROUP: usize = 16;
    if capacity == 0 {
        return 0;
    }
    let buckets = if capacity < 8 {
        capacity + 1
    } else {
        capacity / 7 * 8
    };
    let align = std::mem::align_of::<(K, V)>().max(GROUP);
    (std::mem::size_of::<(K, V)>() * buckets).next_multiple_of(align) + buckets + GROUP
}
struct Table<K, V> {
    map: Map<K, V>,
    bytes: usize,
}
impl<K, V> Default for Table<K, V> {
    fn default() -> Self {
        Self {
            map: Map::default(),
            bytes: 0,
        }
    }
}
impl<K: Eq + std::hash::Hash, V> Table<K, V> {
    fn put(&mut self, key: K, value: V) {
        match self.map.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut held) => {
                held.insert(value);
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(value);
                self.bytes = self.bytes.max(table_bytes::<K, V>(self.map.capacity()));
            }
        }
    }
    fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.map.get_mut(key)
    }
    fn remove(&mut self, key: &K) -> Option<V> {
        self.map.remove(key)
    }
    fn values_mut(&mut self) -> std::collections::hash_map::ValuesMut<'_, K, V> {
        self.map.values_mut()
    }
    fn clear(&mut self) {
        self.map.clear();
    }
}
impl<K, V> std::ops::Deref for Table<K, V> {
    type Target = Map<K, V>;
    fn deref(&self) -> &Map<K, V> {
        &self.map
    }
}
impl<'a, K, V> IntoIterator for &'a Table<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::hash_map::Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}
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
#[derive(Clone, Copy)]
pub struct Selection {
    country: Option<(u32, Result<engine_native_memory::country_identity::Country, &'static str>)>,
    movement: Option<carmy_leader::MovementPreparation>,
    commander: Option<(usize, Result<u64, &'static str>)>,
    province: Result<usize, &'static str>,
    context: Option<Result<Option<(u32, usize)>, &'static str>>,
    membership: Option<((u64, u64), usize)>,
}

pub struct Preparation {
    memo: carmy_leader::LeaderMemo,
    memo_used: bool,
    countries: Map<u32, Result<engine_native_memory::country_identity::Country, &'static str>>,
    commanders: Map<usize, u64>,
    provinces: Map<usize, Option<(u32, usize)>>,
    route_provinces: Map<u32, usize>,
    last_country: Option<(u32, Result<engine_native_memory::country_identity::Country, &'static str>)>,
    last_province: Option<(usize, bool, Option<(u32, usize)>)>,
    roster: Option<engine_native_memory::cpdx_array::Array>,
    reads: NativeReads,
    local_province_refusal: bool,
}
impl Default for Preparation {
    fn default() -> Self {
        Self {
            memo: carmy_leader::LeaderMemo::vacant(),
            memo_used: false,
            countries: Map::default(),
            commanders: Map::default(),
            provinces: Map::default(),
            route_provinces: Map::default(),
            last_country: None,
            last_province: None,
            roster: None,
            reads: NativeReads::default(),
            local_province_refusal: false,
        }
    }
}
impl Preparation {
    pub fn can_settle_owned(&self) -> bool {
        self.reads.complete
    }
    pub fn has_local_province_refusal(&self) -> bool {
        self.local_province_refusal
    }
    pub fn has_variant(&self, handle: u64, pointer: usize) -> bool {
        self.reads.has_variant(handle, pointer)
    }
    #[cfg(test)]
    fn staged_reads(&self) -> usize {
        self.reads.staged()
    }
    pub fn allocated_bytes(&self) -> usize {
        self.memo.allocated_bytes()
            + self.reads.allocated_bytes()
            + table_bytes::<u32, Result<engine_native_memory::country_identity::Country, &'static str>>(self.countries.capacity())
            + table_bytes::<usize, u64>(self.commanders.capacity())
            + table_bytes::<usize, Option<(u32, usize)>>(self.provinces.capacity())
            + table_bytes::<u32, usize>(self.route_provinces.capacity())
    }
    pub fn begin(&mut self, divisions: usize) -> bool {
        self.begin_part(divisions, true)
    }
    pub fn begin_part(&mut self, divisions: usize, read_dirty: bool) -> bool {
        self.local_province_refusal = false;
        self.last_country = None;
        self.last_province = None;
        if !self.countries.is_empty() {
            self.countries.clear();
        }
        if !self.commanders.is_empty() {
            self.commanders.clear();
        }
        if !self.provinces.is_empty() {
            self.provinces.clear();
        }
        if !self.route_provinces.is_empty() {
            self.route_provinces.clear();
        }
        self.roster = None;
        if std::mem::take(&mut self.memo_used) {
            self.memo.clear();
        }
        engine_allocation::advisory(|| {
            engine_allocation::reserve_map(&mut self.countries, divisions.min(8))?;
            self.reads.begin_part(divisions, read_dirty)
        })
        .is_ok()
    }
    pub fn capture_dirty(&mut self, memory: &impl Memory, base: usize, store: &Store) {
        self.reads.capture_dirty(memory, base, store);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        countries: Option<&ccountry::Reader<'_, impl Memory>>,
        store: &Store,
        seed: &DivisionSeed,
    ) -> Selection {
        if countries.is_none() {
            self.reads.incomplete();
        }
        let held = store.admitted().unwrap_or(store);
        let country = source_owner(seed).ok().and_then(|owner| {
            countries.map(|countries| {
                let country = if let Some((last_owner, last_country)) = self.last_country
                    && last_owner == owner
                {
                    last_country
                } else if let Some(&country) = self.countries.get(&owner) {
                    self.last_country = Some((owner, country));
                    country
                } else {
                    let country = countries.resolve(owner);
                    if engine_allocation::advisory(|| {
                        engine_allocation::reserve_map(&mut self.countries, 1)
                    })
                    .is_ok()
                    {
                        self.countries.insert(owner, country);
                    }
                    self.last_country = Some((owner, country));
                    country
                };
                (owner, country)
            })
        });
        let led = country.is_some_and(|(_, country)| country.is_ok());
        let movement = (led && seed.selected.is_none()).then(|| {
            if self.memo.capacity() == 0
                && let Ok(memo) =
                    engine_allocation::advisory(|| carmy_leader::LeaderMemo::try_with_capacity(64))
            {
                self.memo = memo;
            }
            self.memo_used = true;
            carmy_leader::MovementPreparation::capture(memory, base, seed, &mut self.memo)
        });
        let commander = match seed.selected {
            Some(selected) => selected
                .leader
                .and_then(Result::ok)
                .filter(|&leader| led && leader != 0)
                .map(|leader| {
                    let handle = selected
                        .handle
                        .ok_or("division source commander identity unreadable");
                    (leader, handle)
                }),
            None => movement
                .and_then(|movement| movement.leader())
                .map(|receiver| {
                    let handle = if !self.commanders.is_empty()
                        && let Some(&handle) = self.commanders.get(&receiver)
                    {
                        Ok(handle)
                    } else {
                        let handle =
                            read::<u64>(memory, receiver + engine_native::cunit_leader::TYPE)
                                .ok_or("division source commander identity unreadable");
                        if let Ok(handle) = handle
                            && engine_allocation::advisory(|| {
                                engine_allocation::reserve_map(&mut self.commanders, 1)
                            })
                            .is_ok()
                        {
                            self.commanders.insert(receiver, handle);
                        }
                        handle
                    };
                    (receiver, handle)
                }),
        };
        let province = engine_native_land::division_panel_operands::selected_province_cached(
            memory,
            &mut self.roster,
            state,
            seed,
            |id, read| {
                if !self.route_provinces.is_empty()
                    && let Some(&province) = self.route_provinces.get(&id)
                {
                    return Ok(province);
                }
                let province = read()?;
                if engine_allocation::advisory(|| {
                    engine_allocation::reserve_map(&mut self.route_provinces, 1)
                })
                .is_ok()
                {
                    self.route_provinces.insert(id, province);
                }
                Ok(province)
            },
        );
        self.local_province_refusal |= province.is_err();
        if let Err(reason) = province {
            let reason = source_owner(seed).err().unwrap_or(reason);
            if !held
                .pools
                .get(&seed.division)
                .is_some_and(|held| held.context_refusal == Some(reason))
            {
                self.reads.incomplete();
            }
        }
        let (context, resolved_context) = match province {
            Ok(province) => {
                if let Some((last_p, in_held, last_ctx)) = self.last_province
                    && last_p == province
                {
                    ((!in_held).then_some(Ok(last_ctx)), last_ctx)
                } else if let Some(&ctx) = held.provinces.get(&province) {
                    self.last_province = Some((province, true, ctx));
                    (None, ctx)
                } else if !self.provinces.is_empty()
                    && let Some(&ctx) = self.provinces.get(&province)
                {
                    self.last_province = Some((province, false, ctx));
                    (Some(Ok(ctx)), ctx)
                } else {
                    let result = engine_native_land::division_panel_operands::province_state(
                        memory, base, province,
                    );
                    if let Ok(ctx) = result {
                        if engine_allocation::advisory(|| {
                            engine_allocation::reserve_map(&mut self.provinces, 1)
                        })
                        .is_ok()
                        {
                            self.provinces.insert(province, ctx);
                        }
                        self.last_province = Some((province, false, ctx));
                    }
                    (Some(result), result.ok().flatten())
                }
            }
            Err(_) => (None, None),
        };
        let mut selection = Selection {
            country,
            movement,
            commander,
            province,
            context,
            membership: None,
        };
        let block = match seed.selected {
            Some(selected) if led => Some(selected.block),
            Some(_) => None,
            None => movement.map(|movement| {
                carmy_leader::movement_block_prepared(
                    memory,
                    base,
                    seed,
                    &mut self.memo,
                    Some(movement),
                )
            }),
        }
        .and_then(|block| match block {
            Ok(block) => block,
            Err(_) => {
                self.reads.incomplete();
                None
            }
        });
        if country.is_some_and(|(_, value)| value.is_err())
            || commander.is_some_and(|(_, value)| value.is_err())
            || context.is_some_and(|value| value.is_err())
        {
            self.reads.incomplete();
        }
        self.reads.capture(
            memory,
            base,
            state,
            store,
            seed,
            selection,
            block,
            resolved_context,
        );
        selection.membership = self.reads.membership_observation;
        selection
    }
}

fn source_owner(seed: &DivisionSeed) -> Result<u32, &'static str> {
    seed.expeditionary_owner
        .filter(|&owner| (owner as i32) > 0)
        .or(seed.roster_owner)
        .ok_or("division source owner unavailable")
}

struct Pool {
    army: usize,
    certificate: Option<(u64, u64)>,
    count: u32,
    variants: Arc<Vec<(u32, u64, usize)>>,
    seen: u64,
    context: Option<(usize, u32, u32)>,
    context_refusal: Option<&'static str>,
    slot: usize,
}
struct Variant {
    refs: usize,
    lifetime: Option<(u64, u64)>,
    pointer: usize,
    certificate: Option<(u64, u64)>,
    count: u32,
    availability: Option<u64>,
    values: Vec<i64>,
}
struct State {
    slot: usize,
    pointer: usize,
    certificate: Option<(u64, u64)>,
    seen: u64,
}
struct Held {
    slot: usize,
    receiver: usize,
    accumulator: usize,
    certificate: Option<(u64, u64)>,
    count: u32,
    seen: u64,
}
pub struct Store {
    roots: Table<(u8, u64), Held>,
    countries: Table<u32, (u64, Result<engine_native_memory::country_identity::Country, &'static str>, bool)>,
    leaders: Table<usize, (u64, u64, u8)>,
    rows: Rows<SharedDivisionPanelSourceRow>,
    turn: u64,
    world_epoch: u64,
    memo: carmy_leader::LeaderMemo,
    capacity: usize,
    pools: Table<u64, Pool>,
    variants: Table<u64, Variant>,
    states: Table<(u32, u32), State>,
    variant_handles: Table<usize, u64>,
    provinces: Table<usize, Option<(u32, usize)>>,
    membership_scratch: Vec<(u32, u64, usize)>,
    membership_changed: bool,
    membership_bytes: usize,
    variant_value_bytes: usize,
    context_changed: bool,
    dirty: Vec<usize>,
    variant_epoch: Option<u64>,
    variant_observation: Option<((u64, u64), u64)>,
    retired: Vec<usize>,
    lifetime_epoch: Option<u64>,
    published: Option<Arc<SourceSnapshot>>,
    phase_ticks: [u64; PHASE_NAMES.len()],
    boundary: owned::Boundary,
    settling_owned: bool,
    settling_availability: Option<u64>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            roots: Table::default(),
            countries: Table::default(),
            leaders: Table::default(),
            rows: Rows::default(),
            turn: 0,
            world_epoch: 0,
            memo: carmy_leader::LeaderMemo::vacant(),
            capacity: 0,
            pools: Table::default(),
            variants: Table::default(),
            states: Table::default(),
            variant_handles: Table::default(),
            provinces: Table::default(),
            membership_scratch: Vec::new(),
            membership_changed: false,
            membership_bytes: 0,
            variant_value_bytes: 0,
            context_changed: false,
            dirty: Vec::new(),
            variant_epoch: None,
            variant_observation: None,
            retired: Vec::new(),
            lifetime_epoch: None,
            published: None,
            phase_ticks: [0; PHASE_NAMES.len()],
            boundary: Default::default(),
            settling_owned: false,
            settling_availability: None,
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
static FORGOTTEN: std::sync::LazyLock<Store> = std::sync::LazyLock::new(Store::default);
impl Store {
    fn admitted(&self) -> Option<&Store> {
        let world = operands::world_epoch();
        if self.world_epoch == world {
            Some(self)
        } else if self.boundary.active && self.boundary.world_epoch == world {
            Some(&FORGOTTEN)
        } else {
            None
        }
    }
    pub fn forget(&mut self) {
        self.boundary.active = false;
        self.settling_owned = false;
        self.settling_availability = None;
        self.roots.clear();
        self.countries.clear();
        self.leaders.clear();
        self.rows.clear();
        self.memo.clear();
        self.pools.clear();
        self.membership_bytes = 0;
        self.variant_value_bytes = 0;
        self.variants.clear();
        self.states.clear();
        self.variant_handles.clear();
        self.provinces.clear();
        self.variant_epoch = None;
        self.variant_observation = None;
        self.lifetime_epoch = None;
        self.retired.clear();
        self.published = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.boundary.allocated_bytes()
            + self.retired.capacity() * std::mem::size_of::<usize>()
            + self.memo.allocated_bytes()
            + self.roots.bytes
            + self.countries.bytes
            + self.leaders.bytes
            + self.pools.bytes
            + self.membership_bytes
            + self.variant_value_bytes
            + self.variants.bytes
            + self.variant_handles.bytes
            + self.states.bytes
            + self.provinces.bytes
            + self.membership_scratch.capacity() * std::mem::size_of::<(u32, u64, usize)>()
            + self.dirty.capacity() * std::mem::size_of::<usize>()
    }
    fn root_prepared(
        &mut self,
        memory: &impl Memory,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
        reads: Option<&NativeReads>,
    ) -> Result<(), &'static str> {
        if self
            .roots
            .get(&(kind, owner))
            .is_some_and(|held| held.seen == self.turn)
        {
            return Ok(());
        }
        if let Some(held) = self.roots.get_mut(&(kind, owner))
            && let Some(slot) = reads.and_then(|reads| reads.root_slot(kind, owner))
        {
            held.slot = slot;
        }
        let certificate = if self.settling_owned {
            reads.and_then(|reads| reads.root_certificate(kind, owner))
        } else {
            changes::sequence(receiver)
        };
        self.root_with_observation(
            memory,
            kind,
            owner,
            receiver,
            accumulator,
            certificate,
            reads,
        )
    }
    #[cfg(test)]
    fn root_with_certificate(
        &mut self,
        memory: &impl Memory,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
        certificate: Option<(u64, u64)>,
    ) -> Result<(), &'static str> {
        self.root_with_observation(
            memory,
            kind,
            owner,
            receiver,
            accumulator,
            certificate,
            None,
        )
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "one root observation names its kind, owner, receiver, accumulator, certificate and reads"
    )]
    fn root_with_observation(
        &mut self,
        memory: &impl Memory,
        kind: u8,
        owner: u64,
        receiver: usize,
        accumulator: usize,
        certificate: Option<(u64, u64)>,
        reads: Option<&NativeReads>,
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
            if let Some(observed) =
                reads.and_then(|reads| reads.root(kind, owner, receiver, accumulator, certificate))
            {
                for row in &observed.rows {
                    count = row
                        .ordinal
                        .checked_add(1)
                        .ok_or("division modifier source ordinal overflow")?;
                    self.rows.insert(Arc::clone(row))?;
                }
                return observed.result;
            }
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
        self.roots.put(
            (kind, owner),
            Held {
                slot: reads
                    .and_then(|reads| reads.root_slot(kind, owner))
                    .unwrap_or(usize::MAX),
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
        self.capture_prepared(memory, base, state, seeds, selected, |_| None)
    }
    pub fn capture_prepared(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        selected: &mut Vec<(u64, Option<[u64; 3]>)>,
        prepared: impl Fn(usize) -> Option<Selection>,
    ) -> Result<Arc<SourceSnapshot>, &'static str> {
        self.capture_staged(
            memory,
            base,
            state,
            seeds,
            selected,
            prepared,
            |_| None,
            |_, _| None,
        )
    }
    pub fn phase_ticks(&self) -> [u64; PHASE_NAMES.len()] {
        self.phase_ticks
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "the staged capture takes the seeds, selection, preparation and both read sets separately"
    )]
    pub fn capture_staged<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        selected: &mut Vec<(u64, Option<[u64; 3]>)>,
        prepared: impl Fn(usize) -> Option<Selection>,
        reads: impl Fn(usize) -> Option<&'a Preparation>,
        variant_reads: impl Fn(u64, usize) -> Option<&'a Preparation>,
    ) -> Result<Arc<SourceSnapshot>, &'static str> {
        self.capture_boundary();
        let boundary = std::mem::take(&mut self.boundary);
        let result = self.capture_inner(
            memory,
            base,
            state,
            seeds,
            selected,
            prepared,
            reads,
            variant_reads,
            &boundary,
        );
        self.boundary = boundary;
        self.boundary.active = false;
        result
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "the inner capture takes the seeds, selection, preparation, both read sets and the boundary separately"
    )]
    fn capture_inner<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        selected: &mut Vec<(u64, Option<[u64; 3]>)>,
        prepared: impl Fn(usize) -> Option<Selection>,
        reads: impl Fn(usize) -> Option<&'a Preparation>,
        variant_reads: impl Fn(u64, usize) -> Option<&'a Preparation>,
        boundary: &owned::Boundary,
    ) -> Result<Arc<SourceSnapshot>, &'static str> {
        self.phase_ticks = [0; PHASE_NAMES.len()];
        let setup = qpc();
        let availability = boundary.availability.unwrap_or(0);
        let world_epoch = boundary.world_epoch;
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

        boundary.result?;
        self.retire_from(boundary.lifetime_epoch, &boundary.retired)?;
        engine_allocation::reserve(&mut self.dirty, boundary.dirty.len())
            .map_err(|_| "attrition dirty ranges allocation failed")?;
        self.dirty.extend_from_slice(&boundary.dirty);
        let operand_epoch = boundary.variant_epoch;
        let variant_recovery = self.variant_epoch != operand_epoch;
        self.variant_epoch = operand_epoch;
        let countries = if self.settling_owned {
            None
        } else {
            Some(ccountry::Reader::new(memory, base, state)?)
        };
        let mut roster = None;
        for (index, seed) in seeds.iter().enumerate() {
            let prepared = prepared(index);
            let reads = reads(index).map(|preparation| &preparation.reads);
            let selection = (|| {
                let owner = source_owner(seed)?;
                let (country, root_seen) = match self.countries.get(&owner) {
                    Some(&(turn, country, root_seen)) if turn == self.turn => (country, root_seen),
                    _ => {
                        let country = prepared
                            .and_then(|p| p.country)
                            .filter(|(selected, _)| *selected == owner)
                            .map(|(_, country)| country)
                            .unwrap_or_else(|| {
                                countries
                                    .as_ref()
                                    .ok_or("division source owned selection missing")?
                                    .resolve(owner)
                            });
                        self.countries.put(owner, (self.turn, country, false));
                        (country, false)
                    }
                };
                let country = country?;
                if !root_seen {
                    self.root_prepared(
                        memory,
                        0,
                        country.id as u64,
                        country.pointer,
                        country
                            .pointer
                            .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
                            .ok_or("division source country overflow")?,
                        reads,
                    )?;
                    if let Some(entry) = self.countries.get_mut(&owner) {
                        entry.2 = true;
                    }
                }
                let selection = (|| {
                    let block = match seed.selected {
                        Some(selected) => selected.block,
                        None => carmy_leader::movement_block_prepared(
                            memory,
                            base,
                            seed,
                            &mut self.memo,
                            prepared.and_then(|p| p.movement),
                        ),
                    };
                    let Some((accumulator, hq)) = block? else {
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
                    let kind = if hq { 2 } else { 1 };
                    let kind_bit = 1u8 << kind;
                    let selected_handle = seed.selected.and_then(|selected| selected.handle);
                    let (handle, leader_root_seen) = match self.leaders.get(&receiver) {
                        Some(&(turn, cached, mask))
                            if turn == self.turn
                                && selected_handle.is_none_or(|handle| handle == cached) =>
                        {
                            (cached, mask & kind_bit != 0)
                        }
                        _ => {
                            let handle = match selected_handle {
                                Some(handle) => handle,
                                None => prepared
                                    .and_then(|p| p.commander)
                                    .filter(|(selected, _)| *selected == receiver)
                                    .map(|(_, handle)| handle)
                                    .unwrap_or_else(|| {
                                        read::<u64>(
                                            memory,
                                            receiver + engine_native::cunit_leader::TYPE,
                                        )
                                        .ok_or("division source commander identity unreadable")
                                    })?,
                            };
                            self.leaders.put(receiver, (self.turn, handle, 0));
                            (handle, false)
                        }
                    };
                    if handle as u32 != engine_native::cunit_leader::REFERENCE_TYPE {
                        return Err("division source commander identity mismatch");
                    }
                    if !leader_root_seen {
                        self.root_prepared(memory, kind, handle, receiver, accumulator, reads)?;
                        if let Some(entry) = self.leaders.get_mut(&receiver)
                            && entry.0 == self.turn
                            && entry.1 == handle
                        {
                            entry.2 |= kind_bit;
                        }
                    }
                    Ok::<_, &'static str>([country.id as u64, kind as u64, handle])
                })()
                .unwrap_or([country.id as u64, u64::MAX, 0]);
                Ok::<_, &'static str>(selection)
            })();
            selected.push((seed.division, selection.ok()));
            let selection_failure = selection.err();

            self.membership_prepared(
                memory,
                base,
                seed,
                reads,
                prepared.and_then(|p| p.membership),
                (!self.settling_owned).then_some(&boundary.operands),
            )?;
            let context_result = (|| {
                let province = match prepared {
                    Some(prepared) => prepared.province,
                    None => engine_native_land::division_panel_operands::selected_province_in(
                        memory,
                        &mut roster,
                        state,
                        seed,
                    ),
                }?;
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
                        let context = prepared.and_then(|p| p.context).unwrap_or_else(|| {
                            engine_native_land::division_panel_operands::province_state(
                                memory, base, province,
                            )
                        })?;
                        self.provinces.put(province, context);
                        context
                    }
                };
                if let Some((id, pointer)) = context {
                    if let Some(country) = seed.roster_owner {
                        self.state_prepared(memory, base, state, id, pointer, country, reads)?;
                    }
                    if let Some(country) = seed.logical_owner
                        && Some(country) != seed.roster_owner
                    {
                        self.state_prepared(memory, base, state, id, pointer, country, reads)?;
                    }
                }
                Ok::<_, &'static str>(())
            })();
            let result =
                self.context_refusal(seed.division, selection_failure.or(context_result.err()));
            result?;
        }
        let begin = qpc();
        let result = if self.membership_changed || self.context_changed {
            self.prune()
        } else {
            Ok(())
        };
        self.phase_ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        result?;
        if variant_recovery {
            self.dirty.clear();
            engine_allocation::reserve(&mut self.dirty, self.variants.len())
                .map_err(|_| "variant certificate recovery allocation failed")?;
            let mut handles = Vec::new();
            engine_allocation::reserve(&mut handles, self.variants.len())
                .map_err(|_| "variant certificate recovery allocation failed")?;
            handles.extend(
                self.variants
                    .iter()
                    .map(|(&handle, held)| (handle, held.pointer)),
            );
            handles.sort_unstable_by_key(|&(handle, _)| handle);
            self.dirty
                .extend(handles.into_iter().map(|(_, pointer)| pointer));
        }
        let variants_begin = qpc();
        for index in 0..self.dirty.len() {
            let pointer = self.dirty[index];
            if let Some(&handle) = self.variant_handles.get(&pointer) {
                let observed = variant_reads(handle, pointer);
                self.variant_prepared(memory, base, handle, pointer, observed.map(|p| &p.reads))?;
            }
        }

        self.phase_ticks[2] = qpc().saturating_sub(variants_begin).max(0) as u64;
        let begin = qpc();
        operands::demand(self.pools.len(), self.variants.len(), self.states.len());
        changes::division_trees(self.roots.len());
        let mut gone = Vec::new();
        for (&identity, held) in &self.roots {
            if held.seen != self.turn {
                engine_allocation::reserve(&mut gone, 1)
                    .map_err(|_| "division source removal allocation failed")?;
                gone.push(identity);
            }
        }
        gone.sort_unstable();
        for (kind, owner) in gone {
            let held = self.roots.remove(&(kind, owner)).unwrap();
            self.rows
                .remove_range(key(kind, owner, 0)..key(kind, owner, held.count))?;
        }
        self.phase_ticks[1] += qpc().saturating_sub(begin).max(0) as u64;
        let begin = qpc();
        let result = (|| {
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
        })();
        let finished = qpc();
        self.phase_ticks[3] += finished.saturating_sub(begin).max(0) as u64;
        self.phase_ticks[0] = (finished.saturating_sub(setup).max(0) as u64)
            .saturating_sub(self.phase_ticks[1..].iter().sum());
        if result.is_ok() {
            self.variant_observation = boundary.stamp;
        }
        result
    }
    #[cfg(test)]
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
        let retired = std::mem::take(&mut self.retired);
        let result = self.retire_from(epoch, &retired);
        self.retired = retired;
        result
    }
    fn retire_from(&mut self, epoch: Option<u64>, retired: &[usize]) -> Result<(), &'static str> {
        let recovery = self.lifetime_epoch != epoch;
        self.lifetime_epoch = epoch;
        if recovery || !retired.is_empty() {
            for held in self.pools.values_mut() {
                if recovery
                    || held
                        .variants
                        .iter()
                        .any(|&(_, _, pointer)| retired.binary_search(&pointer).is_ok())
                {
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
        values: Vec<game_model::modifier_breakdown::SourceValue>,
    ) -> Result<(), &'static str> {
        self.rows.insert(Arc::new(DivisionPanelSourceRow {
            kind,
            owner,
            ordinal,
            node: Some(game_model::modifier_breakdown::SourceNode {
                children: 0,
                name: Vec::new(),
                values,
            }),
            reason: Vec::new(),
        }))
    }
    fn terrain_operand(&mut self, owner: u64, ordinal: u32, raw: i64) -> Result<(), &'static str> {
        let value = game_model::modifier_breakdown::SourceValue { key: 0, raw };
        if self.rows.get(key(3, owner, ordinal)).is_some_and(|row| {
            row.reason.is_empty()
                && row.node.as_ref().is_some_and(|node| {
                    node.children == 0
                        && node.name.is_empty()
                        && node.values.as_slice() == std::slice::from_ref(&value)
                })
        }) {
            return Ok(());
        }
        self.operand(3, owner, ordinal, vec![value])
    }
    #[cfg(test)]
    fn variant(
        &mut self,
        memory: &impl Memory,
        base: usize,
        handle: u64,
        pointer: usize,
    ) -> Result<(), &'static str> {
        self.variant_prepared(memory, base, handle, pointer, None)
    }
    fn variant_prepared(
        &mut self,
        memory: &impl Memory,
        base: usize,
        handle: u64,
        pointer: usize,
        reads: Option<&NativeReads>,
    ) -> Result<(), &'static str> {
        self.variant_observed(memory, base, handle, pointer, reads)
    }
    fn variant_observed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        handle: u64,
        pointer: usize,
        reads: Option<&NativeReads>,
    ) -> Result<(), &'static str> {
        let availability = if self.settling_owned {
            self.settling_availability
        } else {
            operands::variant_capability()
        };
        let certificate = if self.settling_owned {
            reads.and_then(|reads| reads.variant_certificate(handle, pointer))
        } else {
            operands::variant_sequence(pointer)
        };
        if let Some(held) = self.variants.get(&handle)
            && held.certificate == certificate
            && held.pointer == pointer
            && held.availability == availability
            && (self.settling_owned
                || availability.is_some_and(operands::variant_capability_is_current)
                || self
                    .rows
                    .get(key(3, handle, 0))
                    .is_some_and(|row| row.node.is_none()))
        {
            return Ok(());
        }
        let (old, mut cached_values) = match self.variants.get_mut(&handle) {
            Some(held) => {
                let values = std::mem::take(&mut held.values);
                self.variant_value_bytes = self
                    .variant_value_bytes
                    .saturating_sub(values.capacity() * std::mem::size_of::<i64>());
                (held.count, values)
            }
            None => (0, Vec::new()),
        };
        let mut next_values = Vec::new();
        let result = (|| {
            certificate.ok_or("held variant terrain writer certificate unavailable")?;
            let result = if let Some(observed) = reads.and_then(|reads| {
                reads.variant_observed(
                    handle,
                    pointer,
                    certificate,
                    availability,
                    self.settling_owned,
                )
            }) {
                let same = observed.result.is_ok()
                    && cached_values.len() == observed.values.len()
                    && cached_values.as_slice() == observed.values.as_slice();
                if same {
                    next_values = std::mem::take(&mut cached_values);
                } else {
                    for (ordinal, &raw) in observed.values.iter().enumerate() {
                        if cached_values.get(ordinal).copied() != Some(raw) {
                            self.terrain_operand(handle, ordinal as u32, raw)?;
                        }
                    }
                    if observed.result.is_ok() {
                        cached_values.clear();
                        if engine_allocation::reserve(&mut cached_values, observed.values.len())
                            .is_ok()
                        {
                            cached_values.extend_from_slice(&observed.values);
                            next_values = std::mem::take(&mut cached_values);
                        }
                    }
                }
                observed.result
            } else {
                let old_slice = cached_values.as_slice();
                let mut recorded = Vec::new();
                let mut record_ok = true;
                let res = engine_native_land::division_panel_operands::terrain(
                    memory,
                    base,
                    pointer,
                    handle,
                    |ordinal, raw| {
                        if old_slice.get(ordinal as usize).copied() != Some(raw) {
                            self.terrain_operand(handle, ordinal as u32, raw)?;
                        }
                        if record_ok {
                            if recorded.len() == ordinal as usize
                                && (recorded.len() < recorded.capacity()
                                    || engine_allocation::reserve(&mut recorded, 1).is_ok())
                            {
                                recorded.push(raw);
                            } else {
                                record_ok = false;
                            }
                        }
                        Ok(())
                    },
                );
                if res.is_ok() && record_ok {
                    next_values = recorded;
                }
                res
            };
            result.and_then(|count| {
                if self.settling_owned
                    || (availability.is_some_and(operands::variant_capability_is_current)
                        && operands::variant_sequence(pointer) == certificate)
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
                next_values.clear();
                operands::refused();
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
        self.variant_handles.put(pointer, handle);
        self.variant_value_bytes = self
            .variant_value_bytes
            .saturating_add(next_values.capacity() * std::mem::size_of::<i64>());
        self.variants.put(
            handle,
            Variant {
                refs: self.variants.get(&handle).map_or(0, |held| held.refs),
                lifetime: if self.settling_owned {
                    reads.and_then(|reads| reads.variant_lifetime(handle, pointer))
                } else {
                    operands::observed_lifetime_sequence(pointer)
                },
                pointer,
                certificate,
                count,
                availability,
                values: next_values,
            },
        );
        Ok(())
    }
    #[cfg(test)]
    fn membership(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<(), &'static str> {
        self.membership_prepared(memory, base, seed, None, None, None)
    }
    fn membership_prepared(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        reads: Option<&NativeReads>,
        observation: Option<((u64, u64), usize)>,
        quiet: Option<&crate::observations::Taken>,
    ) -> Result<(), &'static str> {
        let pool = seed
            .pointer
            .checked_add(engine_native::carmy::EQUIPMENT_POOL)
            .ok_or("attrition pool overflow")?;
        let certificate = if self.settling_owned {
            observation.map(|(certificate, _)| certificate)
        } else {
            match self.pools.get_mut(&seed.division) {
                Some(held)
                    if held.army == seed.pointer
                        && held
                            .certificate
                            .is_some_and(|cert| quiet.is_some_and(|quiet| quiet.quiet(pool, cert.0))) =>
                {
                    held.seen = self.turn;
                    if let Some((_, slot)) = observation {
                        held.slot = slot;
                    }
                    return Ok(());
                }
                _ => operands::sequence(pool),
            }
        };
        if let Some(held) = self.pools.get_mut(&seed.division)
            && held.certificate == certificate
            && held.army == seed.pointer
        {
            held.seen = self.turn;
            if let Some((_, slot)) = observation {
                held.slot = slot;
            }
            return Ok(());
        }
        let old_count = self.pools.get(&seed.division).map_or(0, |held| held.count);
        self.membership_scratch.clear();
        let result = (|| {
            certificate.ok_or("held variant membership writer certificate unavailable")?;
            let scratch = &mut self.membership_scratch;
            if let Some(observed) = reads.and_then(|reads| reads.membership(seed, certificate)) {
                engine_allocation::reserve(scratch, observed.variants.len())
                    .map_err(|_| "attrition membership scratch allocation failed")?;
                scratch.extend_from_slice(&observed.variants);
                return observed.result;
            }
            let count = engine_native_land::division_panel_operands::held_variants(
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
                            vec![game_model::modifier_breakdown::SourceValue {
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
                    self.variant_prepared(memory, base, handle, pointer, reads)?;
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
        self.pools.put(
            seed.division,
            Pool {
                army: seed.pointer,
                certificate,
                count,
                variants,
                seen: self.turn,
                context,
                context_refusal,
                slot: observation.map_or(usize::MAX, |(_, slot)| slot),
            },
        );
        Ok(())
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "one state preparation names its id, pointer, country and reads beside the read operands"
    )]
    fn state_prepared(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        id: u32,
        pointer: usize,
        country: u32,
        reads: Option<&NativeReads>,
    ) -> Result<(), &'static str> {
        let certificate = if self.settling_owned {
            reads.and_then(|reads| reads.state_certificate(id, country))
        } else {
            operands::state_sequence(id)
        };
        if let Some(held) = self.states.get_mut(&(id, country))
            && held.certificate == certificate
            && held.pointer == pointer
        {
            held.seen = self.turn;
            if let Some(slot) = reads.and_then(|reads| reads.state_slot(id, country)) {
                held.slot = slot;
            }
            return Ok(());
        }
        let owner = ((id as u64) << 32) | country as u64;
        let result = (|| {
            certificate.ok_or("state attrition writer certificate unavailable")?;
            let mut values = Vec::new();
            let observed = reads.and_then(|reads| reads.state(id, pointer, country, certificate));
            for (index, key) in [
                engine_executable::modifier_keys::ATTRITION,
                engine_executable::modifier_keys::ATTRITION_FOR_CONTROLLER,
                engine_executable::modifier_keys::ENEMY_ATTRITION,
            ]
            .into_iter()
            .enumerate()
            {
                let raw = match observed {
                    Some(observed) => observed.values[index],
                    None => engine_native::cstate::movement_modifier(
                        memory, base, game_state, pointer, country, key,
                    ),
                }?;
                engine_allocation::reserve(&mut values, 1)
                    .map_err(|_| "state attrition allocation failed")?;
                values.push(game_model::modifier_breakdown::SourceValue { key, raw });
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
        self.states.put(
            (id, country),
            State {
                slot: reads
                    .and_then(|reads| reads.state_slot(id, country))
                    .unwrap_or(usize::MAX),
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
        gone.sort_unstable();
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
        gone.sort_unstable();
        for handle in gone {
            let held = self.variants.remove(&handle).unwrap();
            self.variant_value_bytes = self
                .variant_value_bytes
                .saturating_sub(held.values.capacity() * std::mem::size_of::<i64>());
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
        gone.sort_unstable();
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
    use crate::army_stat_changes::{MEMBERSHIP_TEST, serial};
    use replay_data::rows::RowCodec;
    use std::cell::Cell;
    static PROOF: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn current(stamp: u64) -> bool {
        stamp & 1 != 0 && PROOF.load(std::sync::atomic::Ordering::SeqCst) == stamp
    }
    pub(super) fn observe(enabled: bool) {
        let next = PROOF
            .fetch_add(2, std::sync::atomic::Ordering::SeqCst)
            .wrapping_add(2);
        let stamp = if enabled { next | 1 } else { next & !1 };
        PROOF.store(stamp, std::sync::atomic::Ordering::SeqCst);
        operands::variant_availability(stamp, current);
    }
    #[test]
    fn held_keys_replace_in_a_full_table_without_allocating() {
        assert_eq!(table_bytes::<u64, u64>(0), 0);
        assert_eq!(table_bytes::<u64, u64>(3), 4 * 16 + 4 + 16);
        assert_eq!(table_bytes::<u64, u64>(14), 16 * 16 + 16 + 16);
        let mut table = Table::<u64, u64>::default();
        table.put(0, 0);
        let full = table.capacity() as u64;
        for key in 1..full {
            table.put(key, 0);
        }
        assert_eq!(table.len() as u64, full);
        assert_eq!(table.bytes, table_bytes::<u64, u64>(full as usize));
        collector_test_support::with_allocations_denied(|| {
            for key in 0..full {
                table.put(key, key + 1);
            }
        });
        assert_eq!(table.capacity() as u64, full);
        assert!((0..full).all(|key| table[&key] == key + 1));
        let bytes = table.bytes;
        table.put(full, 0);
        assert!(table.bytes > bytes);
        assert_eq!(table.bytes, table_bytes::<u64, u64>(table.capacity()));
        for key in 0..=full {
            table.remove(&key);
        }
        table.clear();
        assert_eq!(table.bytes, table_bytes::<u64, u64>(table.capacity()));
    }
    pub(super) struct Enabled;
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
        use engine_native::{carmy, cequipment_variant::pool as pool, cgame_state};
        use replay_data::rows::tables;
        let _serial = serial(&MEMBERSHIP_TEST);
        const DIVISIONS: usize = 64;
        const STATE: usize = 0x100;
        const COUNTRY: usize = 0x1000;
        operands::demand(DIVISIONS, 0, 0);
        operands::presize();
        operands::boundary(true);
        operands::patched(true);
        observe(true);
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
        put(&mut world.bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        put(&mut world.bytes, COUNTRY + ccountry::ID, 1);
        let mut seeds: Vec<_> = (0..DIVISIONS)
            .map(|index| {
                let pointer = 0x4000 + index * 0x1000;
                put(
                    &mut world.bytes,
                    pointer + carmy::EQUIPMENT_POOL,
                    pool::VTABLE_RVA.0 as u64,
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
            for parts in [1, 2, 7, 31] {
                let mut contexts = vec![None; seeds.len()];
                let mut staged = Store::default();
                let mut memos: Vec<_> = (0..parts).map(|_| Preparation::default()).collect();
                let mut staged_selection = Vec::new();
                let mut first = None;
                let mut bytes = 0;
                for pass in 0..3 {
                    staged.capture_boundary();
                    let countries = ccountry::Reader::new(&world, 0, STATE).unwrap();
                    for part in (0..parts).rev() {
                        let range = seeds.len() * part / parts..seeds.len() * (part + 1) / parts;
                        assert!(memos[part].begin(range.len()));
                        for index in range {
                            contexts[index] = Some(memos[part].capture(
                                &world,
                                0,
                                STATE,
                                Some(&countries),
                                &staged,
                                &seeds[index],
                            ));
                        }
                    }
                    let owned = staged.can_settle_owned()
                        && memos.iter().all(Preparation::can_settle_owned);
                    if pass == 0 {
                        assert!(!owned, "cold source baseline keeps ordered admission");
                    }
                    if pass > 0 && seeds.iter().all(|seed| seed.roster_owner.is_none()) {
                        assert!(owned, "warm owned source settlement must be exercised");
                        assert_eq!(
                            memos.iter().any(Preparation::has_local_province_refusal),
                            seeds.iter().any(|seed| seed.ordinary_route.is_none()),
                        );
                    }
                    let mut capture = || {
                        let selection = |index| contexts[index];
                        let reads = |index| {
                            memos
                                .iter()
                                .enumerate()
                                .find(|(part, _)| {
                                    (seeds.len() * part / parts..seeds.len() * (part + 1) / parts)
                                        .contains(&index)
                                })
                                .map(|(_, memo)| memo)
                        };
                        let variants = |handle, pointer| {
                            memos
                                .iter()
                                .find(|reads| reads.has_variant(handle, pointer))
                        };
                        if owned {
                            staged.settle_owned(
                                0,
                                STATE,
                                seeds,
                                &mut staged_selection,
                                selection,
                                reads,
                                variants,
                            )
                        } else {
                            staged.capture_staged(
                                &world,
                                0,
                                STATE,
                                seeds,
                                &mut staged_selection,
                                selection,
                                reads,
                                variants,
                            )
                        }
                        .unwrap()
                    };
                    let actual = if pass == 0 {
                        capture()
                    } else {
                        collector_test_support::with_allocations_denied(capture)
                    };
                    assert_eq!(complete(&actual), complete(&expected));
                    assert_eq!(staged_selection, selection);
                    let current_bytes = memos
                        .iter()
                        .map(Preparation::allocated_bytes)
                        .sum::<usize>();
                    if let Some(first) = &first {
                        assert!(Arc::ptr_eq(first, &actual));
                        assert_eq!(current_bytes, bytes, "range scratch grew on a quiet hour");
                    } else {
                        first = Some(actual);
                        bytes = current_bytes;
                    }
                }
            }
            selection
        };
        assert_eq!(selected, assert_fresh(&first, &seeds));
        let quiet = collector_test_support::with_allocations_denied(|| {
            store
                .capture(&world, 0, STATE, &seeds, &mut selected)
                .unwrap()
        });
        assert!(Arc::ptr_eq(&first, &quiet));

        let mut serial_refusal = Store::default();
        let mut staged_refusal = Store::default();
        let mut serial_selected = Vec::new();
        let mut staged_selected = Vec::new();
        serial_refusal
            .capture(&world, 0, STATE, &seeds[..1], &mut serial_selected)
            .unwrap();
        staged_refusal
            .capture(&world, 0, STATE, &seeds[..1], &mut staged_selected)
            .unwrap();
        let mut changed = [seeds[0].clone()];
        changed[0].roster_owner = None;
        changed[0].ordinary_route = None;
        staged_refusal.capture_boundary();
        let countries = ccountry::Reader::new(&world, 0, STATE).unwrap();
        let mut preparation = Preparation::default();
        assert!(preparation.begin(1));
        let selection = preparation.capture(
            &world,
            0,
            STATE,
            Some(&countries),
            &staged_refusal,
            &changed[0],
        );
        assert!(preparation.has_local_province_refusal());
        assert!(!preparation.can_settle_owned());
        let serial_result = collector_test_support::with_allocations_denied(|| {
            serial_refusal.capture(&world, 0, STATE, &changed, &mut serial_selected)
        });
        let staged_result = collector_test_support::with_allocations_denied(|| {
            staged_refusal.capture_staged(
                &world,
                0,
                STATE,
                &changed,
                &mut staged_selected,
                |_| Some(selection),
                |_| Some(&preparation),
                |_, _| None,
            )
        });
        assert_eq!(
            serial_result.err(),
            Some("division operand refusal allocation failed")
        );
        assert_eq!(
            staged_result.err(),
            Some("division operand refusal allocation failed")
        );
        assert_eq!(staged_selected, serial_selected);
        assert_eq!(staged_refusal.turn, serial_refusal.turn);

        let mut ownerless = seeds.clone();
        for seed in &mut ownerless {
            seed.roster_owner = None;
        }
        let mut ownerless_store = Store::default();
        let ownerless_rows = ownerless_store
            .capture(&world, 0, STATE, &ownerless, &mut selected)
            .unwrap();
        assert_eq!(selected, assert_fresh(&ownerless_rows, &ownerless));

        for index in [0, DIVISIONS / 2, DIVISIONS - 1] {
            ownerless[index].ordinary_route = None;
        }
        let ownerless_refused = ownerless_store
            .capture(&world, 0, STATE, &ownerless, &mut selected)
            .unwrap();
        assert_eq!(selected, assert_fresh(&ownerless_refused, &ownerless));

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

        const GROUP: usize = 0x2000;
        const LEADER: usize = 0x2800;
        const PROVINCE: usize = 0x3000;
        const CONTEXT: usize = 0x3800;
        put(
            &mut world.bytes,
            GROUP,
            engine_native::corders_group::VTABLE_RVA.0 as u64,
        );
        put(
            &mut world.bytes,
            GROUP + engine_native::corders_group::HANDLE,
            engine_native::corders_group::REFERENCE_TYPE as u64,
        );
        put(
            &mut world.bytes,
            GROUP + engine_native::corders_group::LEADER,
            LEADER as u64,
        );
        put(&mut world.bytes, LEADER, carmy_leader::VTABLE_RVA.0 as u64);
        put(
            &mut world.bytes,
            LEADER + engine_native::cunit_leader::TYPE,
            (9 << 32) | engine_native::cunit_leader::REFERENCE_TYPE as u64,
        );
        put(
            &mut world.bytes,
            PROVINCE,
            engine_native::cprovince::VTABLE_RVA.0 as u64,
        );
        put(
            &mut world.bytes,
            PROVINCE + engine_native::cprovince::STATE,
            CONTEXT as u64,
        );
        put(
            &mut world.bytes,
            CONTEXT,
            engine_native::cstate::VTABLE_RVA.0 as u64,
        );
        put(&mut world.bytes, CONTEXT + engine_native::cstate::ID, 7);
        for seed in &mut seeds {
            seed.orders_group = Some(GROUP);
            seed.class = Some(1);
            seed.location = Some(PROVINCE);
        }
        struct Fault<'a> {
            world: &'a World,
            address: usize,
            len: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                !(address == self.address && out.len() == self.len) && self.world.copy(address, out)
            }
        }
        for (address, len) in [
            (usize::MAX, 0),
            (COUNTRY, 8),
            (GROUP, 8),
            (LEADER + engine_native::cunit_leader::TYPE, 8),
            (PROVINCE, 8),
            (PROVINCE + engine_native::cprovince::STATE, 8),
            (CONTEXT + engine_native::cstate::ID, 4),
        ] {
            let source = Fault {
                world: &world,
                address,
                len,
            };
            let mut serial = Store::default();
            let mut expected_selection = Vec::new();
            let expected = serial
                .capture(&source, 0, STATE, &seeds, &mut expected_selection)
                .unwrap();
            for parts in [1, 7] {
                let mut staged = Store::default();
                let mut selected = Vec::new();
                let mut observations = vec![None; seeds.len()];
                let countries = ccountry::Reader::new(&source, 0, STATE).unwrap();
                for part in (0..parts).rev() {
                    let range = seeds.len() * part / parts..seeds.len() * (part + 1) / parts;
                    let mut memo = Preparation::default();
                    assert!(memo.begin(range.len()));
                    for index in range {
                        observations[index] = Some(memo.capture(
                            &source,
                            0,
                            STATE,
                            Some(&countries),
                            &staged,
                            &seeds[index],
                        ));
                    }
                }
                let actual = staged
                    .capture_prepared(&source, 0, STATE, &seeds, &mut selected, |index| {
                        observations[index]
                    })
                    .unwrap();
                assert_eq!(complete(&actual), complete(&expected));
                assert_eq!(selected, expected_selection);
                for (division, held) in &serial.pools {
                    let actual = &staged.pools[division];
                    assert_eq!(actual.certificate, held.certificate);
                    assert_eq!(actual.context, held.context);
                    assert_eq!(actual.context_refusal, held.context_refusal);
                }
                for (identity, held) in &serial.roots {
                    let actual = &staged.roots[identity];
                    assert_eq!(actual.certificate, held.certificate);
                    assert_eq!(actual.receiver, held.receiver);
                    assert_eq!(actual.accumulator, held.accumulator);
                }
            }
        }
    }

    #[test]
    fn a_store_of_an_earlier_world_is_staged_as_empty_and_admitted_without_pool_reads() {
        use engine_native::{carmy, cequipment_variant::pool as pool, cgame_state};
        use replay_data::rows::tables;
        let _serial = serial(&MEMBERSHIP_TEST);
        const DIVISIONS: usize = 64;
        const STATE: usize = 0x100;
        const COUNTRY: usize = 0x1000;
        operands::demand(DIVISIONS, 0, 0);
        operands::presize();
        operands::boundary(true);
        operands::patched(true);
        observe(true);
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
        put(&mut world.bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        put(&mut world.bytes, COUNTRY + ccountry::ID, 1);
        let seeds: Vec<_> = (0..DIVISIONS)
            .map(|index| {
                let pointer = 0x4000 + index * 0x1000;
                put(
                    &mut world.bytes,
                    pointer + carmy::EQUIPMENT_POOL,
                    pool::VTABLE_RVA.0 as u64,
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
        struct Pools<'a> {
            world: &'a World,
            seeds: &'a [DivisionSeed],
            reads: Cell<usize>,
        }
        impl Memory for Pools<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if self.seeds.iter().any(|seed| {
                    (seed.pointer + carmy::EQUIPMENT_POOL..seed.pointer + 0x1000).contains(&address)
                }) {
                    self.reads.set(self.reads.get() + 1);
                }
                self.world.copy(address, output)
            }
        }
        let complete = |snapshot: &SourceSnapshot| {
            snapshot
                .publish(|rows| rows.complete::<tables::DivisionPanelSources>().unwrap())
                .unwrap()
        };
        for parts in [1, 2, 7] {
            let range = |part: usize| seeds.len() * part / parts..seeds.len() * (part + 1) / parts;
            let mut staged = Store::default();
            let mut selected = Vec::new();
            staged
                .capture(&world, 0, STATE, &seeds, &mut selected)
                .unwrap();
            operands::boundary(true);
            let mut serial = Store::default();
            let mut expected_selection = Vec::new();
            let expected = serial
                .capture(&world, 0, STATE, &seeds, &mut expected_selection)
                .unwrap();
            staged.capture_boundary();
            assert!(!staged.can_settle_owned());
            let countries = ccountry::Reader::new(&world, 0, STATE).unwrap();
            let mut memos: Vec<_> = (0..parts).map(|_| Preparation::default()).collect();
            let mut selections = vec![None; seeds.len()];
            for part in (0..parts).rev() {
                assert!(memos[part].begin(range(part).len()));
                for index in range(part) {
                    selections[index] = Some(memos[part].capture(
                        &world,
                        0,
                        STATE,
                        Some(&countries),
                        &staged,
                        &seeds[index],
                    ));
                }
                assert!(
                    memos[part].staged_reads() >= range(part).len(),
                    "parts={parts}: range {part} read its memberships"
                );
            }
            let memory = Pools {
                world: &world,
                seeds: &seeds,
                reads: Cell::new(0),
            };
            let actual = staged
                .capture_staged(
                    &memory,
                    0,
                    STATE,
                    &seeds,
                    &mut selected,
                    |index| selections[index],
                    |index| {
                        memos
                            .iter()
                            .enumerate()
                            .find(|(part, _)| range(*part).contains(&index))
                            .map(|(_, memo)| memo)
                    },
                    |handle, pointer| memos.iter().find(|memo| memo.has_variant(handle, pointer)),
                )
                .unwrap();
            assert_eq!(memory.reads.get(), 0, "parts={parts}: the ordered admission read a pool");
            assert_eq!(complete(&actual), complete(&expected), "parts={parts}");
            assert_eq!(selected, expected_selection, "parts={parts}");
            for (division, held) in &serial.pools {
                assert_eq!(staged.pools[division].certificate, held.certificate);
                assert_eq!(staged.pools[division].count, held.count);
            }
        }
    }

    #[test]
    fn repeated_dirty_refusals_keep_rows_and_membership_allocations() {
        use engine_native::{carmy, cequipment_variant::pool as cequipment_variant_pool};
        let _serial = serial(&MEMBERSHIP_TEST);
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
            .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes());
        let mut store = Store::default();
        store.membership(&world, 0, &seed).unwrap();
        store
            .root_with_certificate(&world, 0, 1, 0x800, 0x900, Some((1, 1)))
            .unwrap();
        let first = store.rows.finish().unwrap();
        let variants = Arc::clone(&store.pools[&seed.division].variants);
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
        assert!(Arc::ptr_eq(
            &variants,
            &store.pools[&seed.division].variants
        ));
    }

    #[test]
    fn a_pool_clear_replaces_refused_membership_with_successful_empty_membership() {
        use engine_native::{carmy, cequipment_variant::pool as cequipment_variant_pool};
        let _serial = serial(&MEMBERSHIP_TEST);
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
            .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes());
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
            store.variants.put(
                handle,
                Variant {
                    lifetime: None,
                    refs,
                    pointer,
                    certificate: Some((1, 1)),
                    count: 0,
                    availability: None,
                    values: Vec::new(),
                },
            );
        }
        store.variant_handles.put(pointer, 8);
        store.prune().unwrap();
        assert!(!store.variants.contains_key(&7));
        assert_eq!(store.variants[&8].refs, 1);
        assert_eq!(store.variant_handles.get(&pointer), Some(&8));
    }

    #[test]
    fn first_multi_entry_failures_withdraw_all_ordinals_and_publication_checks_the_rows() {
        use engine_native::cequipment_variant as variant;
        use engine_native_land::division_panel_operands;
        let _serial = serial(&MEMBERSHIP_TEST);
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
            put(&mut world.bytes, pointer, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
            put(&mut world.bytes, pointer + 8, handle);
            let terrain = pointer + division_panel_operands::VARIANT_TERRAIN_ATTRITION;
            put(&mut world.bytes, terrain, values as u64);
            put(
                &mut world.bytes,
                terrain + engine_native_memory::cpdx_array::CAPACITY,
                (count << 32) | count,
            );
            for ordinal in 0..count {
                put(&mut world.bytes, values + ordinal as usize * 8, ordinal + 5);
            }
            struct StopAt<'a>(&'a World, usize, bool);
            impl Memory for StopAt<'_> {
                fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                    let hit = (address..address.saturating_add(output.len())).contains(&self.1);
                    if hit && !self.2 {
                        return false;
                    }
                    let copied = self.0.copy(address, output);
                    if hit {
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
        use engine_native::cequipment_variant as variant;
        use engine_native_land::division_panel_operands;
        let _serial = serial(&MEMBERSHIP_TEST);
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
        put(&mut world.bytes, pointer, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        put(&mut world.bytes, pointer + 8, handle);
        let terrain = pointer + division_panel_operands::VARIANT_TERRAIN_ATTRITION;
        put(&mut world.bytes, terrain, values as u64);
        put(
            &mut world.bytes,
            terrain + engine_native_memory::cpdx_array::CAPACITY,
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
    fn an_equal_terrain_reread_allocates_nothing_and_each_change_publishes_only_its_rows() {
        use engine_native::cequipment_variant as variant;
        use engine_native_land::division_panel_operands;
        let _serial = serial(&MEMBERSHIP_TEST);
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
        let terrain = pointer + division_panel_operands::VARIANT_TERRAIN_ATTRITION;
        let resize = |bytes: &mut [u8], count: u64| {
            put(
                bytes,
                terrain + engine_native_memory::cpdx_array::CAPACITY,
                (count << 32) | count,
            )
        };
        put(&mut world.bytes, pointer, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        put(&mut world.bytes, pointer + 8, handle);
        put(&mut world.bytes, terrain, values as u64);
        resize(&mut world.bytes, 3);
        for ordinal in 0..3 {
            put(&mut world.bytes, values + ordinal * 8, ordinal as u64 + 5);
        }
        let fresh = |world: &World| {
            let mut fresh = Store::default();
            fresh.variant(world, 0, handle, pointer).unwrap();
            fresh
                .rows
                .finish()
                .unwrap()
                .complete::<replay_data::rows::tables::DivisionPanelSources>()
                .unwrap()
        };
        let moved = || {
            operands::variant_before(pointer);
            operands::variant_after(pointer, false);
        };
        let mut store = Store::default();
        store.variant(&world, 0, handle, pointer).unwrap();
        let first = store.rows.finish().unwrap();
        assert_eq!(
            first
                .complete::<replay_data::rows::tables::DivisionPanelSources>()
                .unwrap(),
            fresh(&world)
        );

        moved();
        world.reads.set(0);
        collector_test_support::with_allocations_denied(|| {
            store.variant(&world, 0, handle, pointer).unwrap();
            assert!(Arc::ptr_eq(&first, &store.rows.finish().unwrap()));
        });
        assert!(world.reads.get() > 0, "a moved certificate must reread");

        put(&mut world.bytes, values + 8, (-3i64) as u64);
        moved();
        store.variant(&world, 0, handle, pointer).unwrap();
        let changed = store.rows.finish().unwrap();
        assert_eq!(
            changed.ranges.as_slice(),
            std::slice::from_ref(&(key(3, handle, 1)..key(3, handle, 2)))
        );
        assert_eq!(changed.changed.len(), 1);
        assert_eq!(
            changed
                .complete::<replay_data::rows::tables::DivisionPanelSources>()
                .unwrap(),
            fresh(&world)
        );

        resize(&mut world.bytes, 4);
        put(&mut world.bytes, values + 24, 11);
        moved();
        store.variant(&world, 0, handle, pointer).unwrap();
        let grown = store.rows.finish().unwrap();
        assert_eq!(
            grown.ranges.as_slice(),
            std::slice::from_ref(&(key(3, handle, 3)..key(3, handle, 4)))
        );
        resize(&mut world.bytes, 3);
        moved();
        store.variant(&world, 0, handle, pointer).unwrap();
        let shrunk = store.rows.finish().unwrap();
        assert_eq!(
            shrunk.ranges.as_slice(),
            std::slice::from_ref(&(key(3, handle, 3)..key(3, handle, 4)))
        );
        assert!(shrunk.changed.is_empty());
        assert_eq!(
            shrunk
                .complete::<replay_data::rows::tables::DivisionPanelSources>()
                .unwrap(),
            fresh(&world)
        );

        world.deny.set(true);
        moved();
        store.variant(&world, 0, handle, pointer).unwrap();
        let refused = store.rows.get(key(3, handle, 0)).unwrap();
        assert!(refused.node.is_none());
        assert!(store.rows.get(key(3, handle, 1)).is_none());
        let refused = store.rows.finish().unwrap();
        world.deny.set(false);
        moved();
        store.variant(&world, 0, handle, pointer).unwrap();
        let recovered = store.rows.finish().unwrap();
        assert_eq!(recovered.parent, refused.revision);
        assert_eq!(recovered.changed.len(), 3);
        assert_eq!(
            recovered
                .complete::<replay_data::rows::tables::DivisionPanelSources>()
                .unwrap(),
            shrunk
                .complete::<replay_data::rows::tables::DivisionPanelSources>()
                .unwrap()
        );
    }

    #[test]
    fn retirement_reauthenticates_only_affected_pools_before_following_variants() {
        use engine_native::cequipment_variant::{self as variant, pool as pool};
        let _serial = serial(&MEMBERSHIP_TEST);
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
                pool::VTABLE_RVA.0 as u64,
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
        put(&mut world.bytes, pointer, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        put(&mut world.bytes, pointer + 8, handle);
        let mut store = Store::default();
        store.retire_memberships().unwrap();
        for seed in &seeds {
            store.membership(&world, 0, seed).unwrap();
        }
        assert_eq!(store.variants[&handle].refs, 1);
        let unrelated = store.pools[&8].certificate;
        let membership = Arc::clone(&store.pools[&8].variants);

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
