use collector_hooks::hook::Hook;
use collector_observers::war_changes::{self, Changes};
use collector_primitives::raw_memory;
use collector_timing::hook_timing::{self, Family, Span};
use engine_allocation as allocation;
use engine_mutations::war_participation_sites::SITES;
use engine_native_memory::Memory;
use engine_native::cwar_relation::{overview::OwnedRow, participation as native};
use replay_data::rows::{
    TableRows, WarCapturedProvinceRow, WarLendLeaseHistoryRow, WarParticipationRow,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

static STORE: Mutex<Option<Store>> = Mutex::new(None);
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());
static ENABLED: AtomicBool = AtomicBool::new(false);
static LOST: AtomicBool = AtomicBool::new(true);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CALLS: [AtomicU64; 15] = [const { AtomicU64::new(0) }; 15];
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static READ_FIELDS: AtomicU64 = AtomicU64::new(0);
static CARRIED: AtomicU64 = AtomicU64::new(0);
static SEEDS: AtomicU64 = AtomicU64::new(0);
static SEED_ROSTERS: AtomicU64 = AtomicU64::new(0);
static SEED_OWNERS: AtomicU64 = AtomicU64::new(0);
static SEED_MATRIX: AtomicU64 = AtomicU64::new(0);
static SEED_STATUSES: AtomicU64 = AtomicU64::new(0);
static SEED_NONZERO: AtomicU64 = AtomicU64::new(0);
static REQUIRED_EVENTS: AtomicUsize = AtomicUsize::new(0);
static REQUIRED_HISTORY_EVENTS: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 15] = [const { AtomicUsize::new(0) }; 15];

struct Held {
    first: u32,
    second: u32,
    ledgers: [native::Ledger; 2],
    signatures: [Option<(i32, i32, bool)>; 2],
    seed: bool,
    seen: u64,
    revision: u64,
    dirty: [u16; 2],
}
struct History {
    country: u32,
    target: u32,
    value: native::DeliveryHistory,
    dirty: u8,
    seen: u64,
}
struct HistoryOwner {
    country: u32,
    diplomacy: usize,
    statuses: HashSet<usize>,
    seen: u64,
    seed: bool,
}
#[derive(Default)]
struct Store {
    scope: Option<(usize, usize)>,
    topology: Option<(u64, u64)>,
    generation: u64,
    held: HashMap<usize, Held>,
    histories: HashMap<usize, History>,
    events: Vec<(usize, i32, u64)>,
    pending_scores: Vec<usize>,
    pending_histories: Vec<usize>,
    unseen_histories: Vec<(usize, u32, u32, u8)>,
    history_owners: HashMap<usize, (u32, u32)>,
    history_slots: HashMap<u32, u32>,
    history_roots: HashMap<u32, HistoryOwner>,
    roster_owners: HashSet<u32>,
    pending_owner_seeds: Vec<u32>,
    history_generation: u64,
    rows: Option<[Arc<TableRows>; 3]>,
    scores_changed: bool,
    captures_changed: bool,
    histories_changed: bool,
    requires_seed: bool,
    charge: raw_memory::Charge,
}

pub fn invalidate() {
    refuse();
}
fn refuse() {
    LOST.store(true, Ordering::SeqCst);
    REFUSALS.fetch_add(1, Ordering::Relaxed);
}
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}
#[derive(serde::Serialize)]
pub struct Status {
    enabled: bool,
    lost: bool,
    active: usize,
    calls: [u64; 15],
    refusals: u64,
    changed_field_reads: u64,
    seeded_directions: u64,
    carried_hours: u64,
    seed_roster_entries: u64,
    seed_owners: u64,
    seed_matrix_entries: u64,
    seed_status_entries: u64,
    seed_nonzero_histories: u64,
}

fn held_mut(store: &mut Store, ledger: usize) -> Option<(&mut Held, usize)> {
    let first = ledger.checked_sub(native::DIRECTIONAL_LEDGERS[0])?;
    let (relation, direction) = if store.held.contains_key(&first) {
        (first, 0)
    } else {
        (ledger.checked_sub(native::DIRECTIONAL_LEDGERS[1])?, 1)
    };
    store.held.get_mut(&relation).map(|h| (h, direction))
}

struct Flight {
    timing: Span,
    complete: bool,
}
impl Flight {
    fn enter(site: usize) -> Self {
        let timing = hook_timing::enter(Family::WarParticipation);
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        CALLS[site].fetch_add(1, Ordering::Relaxed);
        Self {
            timing,
            complete: false,
        }
    }
    fn finish(mut self) {
        self.complete = true;
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        if !self.complete {
            refuse();
        }
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

fn casualty_changed(ledger: usize, known: i32, unknown: i32, capitulated: bool) -> u16 {
    let Ok(mut slot) = STORE.try_lock() else {
        refuse();
        return 0;
    };
    let Some(store) = slot.as_mut() else {
        return 0;
    };
    let Some((held, side)) = held_mut(store, ledger) else {
        return 0;
    };
    let signature = (known / 1000, unknown / 1000, capitulated);
    let changed = match held.signatures[side] {
        None => (1 << 6) | (1 << 7),
        Some(previous) => {
            (if previous.0 != signature.0 || previous.2 != signature.2 {
                1 << 6
            } else {
                0
            }) | (if previous.1 != signature.1 || previous.2 != signature.2 {
                1 << 7
            } else {
                0
            })
        }
    };
    held.signatures[side] = Some(signature);
    changed
}

fn changed(ledger: usize, columns: u16, province: Option<i32>) {
    let Ok(mut slot) = STORE.try_lock() else {
        refuse();
        return;
    };
    let Some(store) = slot.as_mut() else {
        return;
    };
    let Some((held, direction)) = held_mut(store, ledger) else {
        return;
    };
    if held.seed {
        return;
    }
    let revision = held.revision;
    let first_mark = province.is_none() && held.dirty[direction] == 0;
    if province.is_none() {
        held.dirty[direction] |= columns;
    }
    if first_mark {
        if store.pending_scores.len() == store.pending_scores.capacity() {
            refuse();
            return;
        }
        store.pending_scores.push(ledger);
    }
    if let Some(province) = province {
        if store.events.len() == store.events.capacity() {
            REQUIRED_EVENTS.fetch_max(
                store.events.len().saturating_add(1).saturating_mul(2),
                Ordering::Relaxed,
            );
            refuse();
        } else {
            store.events.push((ledger, province, revision));
        }
    }
}
fn history_changed(status: usize, field: usize) {
    let Ok(mut slot) = STORE.try_lock() else {
        refuse();
        return;
    };
    let Some(store) = slot.as_mut() else {
        return;
    };
    if let Some(history) = store.histories.get_mut(&status) {
        let first_mark = history.dirty == 0;
        history.dirty |= 1 << field;
        if first_mark {
            if store.pending_histories.len() == store.pending_histories.capacity() {
                refuse();
                return;
            }
            store.pending_histories.push(status);
        }
        return;
    }
    if store.history_owners.is_empty() || LOST.load(Ordering::SeqCst) {
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let Some(diplomacy) = status
        .checked_add(engine_native::crelation::status::DIPLOMACY_STATUS)
        .and_then(|p| engine_native_memory::read::<u64>(&memory, p))
    else {
        refuse();
        return;
    };
    let Some(&(_, country)) = store.history_owners.get(&(diplomacy as usize)) else {
        return;
    };
    let target = status
        .checked_add(engine_native::crelation::status::TARGET_COUNTRY)
        .and_then(|p| engine_native_memory::read::<u32>(&memory, p));
    let Some(target) = target else {
        refuse();
        return;
    };
    if store.unseen_histories.len() == store.unseen_histories.capacity() {
        REQUIRED_HISTORY_EVENTS.fetch_max(
            store
                .unseen_histories
                .len()
                .saturating_add(1)
                .saturating_mul(2),
            Ordering::Relaxed,
        );
        refuse();
        return;
    }
    store
        .unseen_histories
        .push((status, country, target, 1 << field));
}

fn replace_held(store: &mut Store, relation: usize) {
    if let Some(held) = store.held.get_mut(&relation) {
        held.seed = true;
        store.requires_seed = true;
        match held.revision.checked_add(1) {
            Some(revision) => held.revision = revision,
            None => refuse(),
        }
    }
}
fn replace(relation: usize) {
    let Ok(mut slot) = STORE.try_lock() else {
        refuse();
        return;
    };
    if let Some(store) = slot.as_mut() {
        replace_held(store, relation);
    }
}

fn lifetime_changed(site: war_changes::Site, receiver: usize, diplomacy: usize) {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let _timing = hook_timing::enter(Family::WarParticipation);
    let Ok(mut slot) = STORE.try_lock() else {
        refuse();
        return;
    };
    let Some(store) = slot.as_mut() else {
        return;
    };
    if store.held.is_empty() && store.history_owners.is_empty() {
        return;
    }
    if matches!(
        site,
        war_changes::Site::ReplaceRelation
            | war_changes::Site::StatusReset
            | war_changes::Site::StatusLoad
    ) && !store.held.is_empty()
    {
        let relation = receiver
            .checked_add(engine_native::crelation::status::WAR_RELATION)
            .and_then(|p| {
                engine_native_memory::read::<u64>(&unsafe { engine_native_memory::memory::LiveMemory::new() }, p)
            });
        match relation {
            Some(relation) if relation != 0 => replace_held(store, relation as usize),
            Some(_) => (),
            None => refuse(),
        }
    }
    if store.history_owners.is_empty() {
        return;
    }
    if site == war_changes::Site::StatusConstruct && diplomacy == 0 {
        return;
    }
    let diplomacy = if diplomacy != 0 {
        Some(diplomacy)
    } else {
        receiver
            .checked_add(engine_native::crelation::status::DIPLOMACY_STATUS)
            .and_then(|p| {
                engine_native_memory::read::<u64>(&unsafe { engine_native_memory::memory::LiveMemory::new() }, p)
            })
            .map(|p| p as usize)
    };
    let Some(diplomacy) = diplomacy else {
        refuse();
        return;
    };
    let Some(&(owner, _)) = store.history_owners.get(&diplomacy) else {
        return;
    };
    let Some(root) = store.history_roots.get_mut(&owner) else {
        refuse();
        return;
    };
    if !root.seed {
        if store.pending_owner_seeds.len() == store.pending_owner_seeds.capacity() {
            refuse();
            return;
        }
        root.seed = true;
        store.pending_owner_seeds.push(owner);
    }
}

fn allocation_error(_: std::io::Error) -> &'static str {
    "war participation storage allocation failed"
}

pub fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[OwnedRow],
    country_sources: impl Iterator<Item = engine_native::cwar_relation::overview::CountrySource>,
    full: bool,
) -> Result<[Arc<TableRows>; 3], &'static str> {
    let result = capture_inner(memory, base, state, sources, country_sources, full);
    if result.is_err() {
        refuse();
    }
    result
}
fn capture_inner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    sources: &[OwnedRow],
    country_sources: impl Iterator<Item = engine_native::cwar_relation::overview::CountrySource>,
    full: bool,
) -> Result<[Arc<TableRows>; 3], &'static str> {
    if !ENABLED.load(Ordering::SeqCst) {
        return Err("war participation hooks unavailable");
    }
    let topology = war_changes::Observer
        .quiet()
        .ok_or("war participation lifetime certificate unavailable")?;
    if active() != 0 {
        return Err("war participation writer overlaps acquisition");
    }
    let mut slot = STORE
        .try_lock()
        .map_err(|_| "war participation store busy")?;
    let store = slot.get_or_insert_with(Store::default);
    let topology_changed = store.topology != Some(topology);
    let reset = full
        || store.scope != Some((base, state))
        || store
            .topology
            .is_none_or(|previous| previous.0 != topology.0);
    if LOST.load(Ordering::SeqCst) && !reset {
        return Err("war participation observation lost; full refresh required");
    }
    if reset {
        store.scope = Some((base, state));
        store.topology = Some(topology);
        store.events.clear();
        store.pending_scores.clear();
        store.pending_histories.clear();
        store.unseen_histories.clear();
        for held in store.held.values_mut() {
            held.seed = true;
        }
        store.histories.clear();
        store.pending_owner_seeds.clear();
        for root in store.history_roots.values_mut() {
            root.seed = false;
            root.statuses.clear();
        }
        store.rows = None;
        store.scores_changed = true;
        store.captures_changed = true;
        store.histories_changed = true;
        LOST.store(false, Ordering::SeqCst);
    }
    if !reset
        && !topology_changed
        && store.pending_owner_seeds.is_empty()
        && !store.requires_seed
        && !store.scores_changed
        && !store.captures_changed
        && !store.histories_changed
        && store.events.is_empty()
        && store.pending_scores.is_empty()
        && store.pending_histories.is_empty()
        && store.unseen_histories.is_empty()
    {
        if active() != 0
            || LOST.load(Ordering::SeqCst)
            || war_changes::Observer.quiet() != Some(topology)
        {
            refuse();
            return Err("war participation writers changed during acquisition");
        }
        CARRIED.fetch_add(1, Ordering::Relaxed);
        return store
            .rows
            .clone()
            .ok_or("war participation retained tables absent");
    }
    let reserve = sources.len().max(REQUIRED_EVENTS.load(Ordering::Relaxed));
    if store.events.capacity() < reserve {
        let additional = reserve.saturating_sub(store.events.len());
        allocation::reserve(&mut store.events, additional).map_err(allocation_error)?;
    }
    if reset || topology_changed || store.requires_seed {
        store.generation = store
            .generation
            .checked_add(1)
            .ok_or("war participation capture generation exhausted")?;
        let generation = store.generation;
        for source in sources.iter().filter(|r| r.retired == 0) {
            let pointer = source.source.relation;
            if pointer == 0 {
                return Err("war participation census source absent");
            }
            if !store.held.contains_key(&pointer) {
                allocation::reserve_map(&mut store.held, 1).map_err(allocation_error)?;
                store.held.insert(
                    pointer,
                    Held {
                        first: source.first_country,
                        second: source.second_country,
                        ledgers: Default::default(),
                        signatures: [None; 2],
                        seed: true,
                        seen: generation,
                        revision: 0,
                        dirty: [0; 2],
                    },
                );
            }
            let held = store.held.get_mut(&pointer).unwrap();
            held.seen = generation;
            if held.first != source.first_country || held.second != source.second_country {
                held.first = source.first_country;
                held.second = source.second_country;
                held.seed = true;
            }
            if held.seed {
                for (side, offset) in native::DIRECTIONAL_LEDGERS.into_iter().enumerate() {
                    let ledger = pointer
                        .checked_add(offset)
                        .ok_or("war participation ledger address overflow")?;
                    native::read_into(memory, base, ledger, &mut held.ledgers[side])?;
                }
                SEEDS.fetch_add(2, Ordering::Relaxed);
                held.signatures = [None; 2];
                held.dirty = [0; 2];
                held.seed = false;
                store.scores_changed = true;
                store.captures_changed = true;
            }
        }
        let before = store.held.len();
        store.held.retain(|_, value| value.seen == generation);
        if before != store.held.len() {
            store.scores_changed = true;
            store.captures_changed = true;
        }
        store.requires_seed = false;
        let score_demand = store
            .held
            .len()
            .checked_mul(2)
            .ok_or("war participation receiver count overflow")?;
        let additional = score_demand.saturating_sub(store.pending_scores.len());
        allocation::reserve(&mut store.pending_scores, additional).map_err(allocation_error)?;
    }
    read_province_events(store)?;
    if reset || topology_changed {
        let roster_entries =
            reconcile_histories(sources.iter().copied(), country_sources, store, reset)?;
        SEED_ROSTERS.fetch_add(roster_entries as u64, Ordering::Relaxed);
    }
    seed_history_owners(memory, base, state, store)?;
    read_marked(memory, store)?;
    if active() != 0
        || LOST.load(Ordering::SeqCst)
        || war_changes::Observer.quiet() != Some(topology)
    {
        refuse();
        return Err("war participation writers changed during acquisition");
    }
    store.topology = Some(topology);
    if !store.scores_changed && !store.captures_changed && !store.histories_changed {
        CARRIED.fetch_add(1, Ordering::Relaxed);
        return store
            .rows
            .clone()
            .ok_or("war participation retained tables absent");
    }
    publish(store)
}

fn read_province_events(store: &mut Store) -> Result<(), &'static str> {
    for index in 0..store.events.len() {
        let (ledger, province, revision) = store.events[index];
        if let Some((held, side)) = held_mut(store, ledger) {
            if held.revision != revision {
                continue;
            }
            let members = &mut held.ledgers[side].captured_provinces;
            if let Err(index) = members.binary_search(&province) {
                allocation::reserve(members, 1).map_err(allocation_error)?;
                members.insert(index, province);
                let first_mark = held.dirty[side] == 0;
                held.dirty[side] |= SITES[4].columns;
                if first_mark {
                    allocation::reserve(&mut store.pending_scores, 1).map_err(allocation_error)?;
                    store.pending_scores.push(ledger);
                }
                store.captures_changed = true;
            }
        }
    }
    store.events.clear();
    Ok(())
}

fn reconcile_histories(
    sources: impl IntoIterator<Item = OwnedRow>,
    country_sources: impl Iterator<Item = engine_native::cwar_relation::overview::CountrySource>,
    store: &mut Store,
    full: bool,
) -> Result<usize, &'static str> {
    store.history_owners.clear();
    store.history_slots.clear();
    store.roster_owners.clear();
    let generation = store.generation;
    let mut visited = 0usize;
    for row in sources {
        visited += 1;
        let source = row.source;
        for slot in [source.owner_slot, source.first_slot, source.second_slot] {
            if !store.roster_owners.contains(&slot) {
                allocation::reserve_set(&mut store.roster_owners, 1).map_err(allocation_error)?;
                store.roster_owners.insert(slot);
            }
        }
    }
    for country in country_sources {
        if !store.history_slots.contains_key(&country.country) {
            allocation::reserve_map(&mut store.history_slots, 1).map_err(allocation_error)?;
        }
        store.history_slots.insert(country.country, country.slot);
        if !store.roster_owners.contains(&country.slot) {
            continue;
        }
        let new = !store.history_roots.contains_key(&country.slot);
        if new {
            allocation::reserve_map(&mut store.history_roots, 1).map_err(allocation_error)?;
            store.history_roots.insert(
                country.slot,
                HistoryOwner {
                    country: country.country,
                    diplomacy: country.diplomacy,
                    statuses: HashSet::new(),
                    seen: generation,
                    seed: false,
                },
            );
        }
        let root = store.history_roots.get_mut(&country.slot).unwrap();
        let replaced = root.country != country.country || root.diplomacy != country.diplomacy;
        let changed = full || new || replaced;
        if replaced {
            for status in &root.statuses {
                if store
                    .histories
                    .get(status)
                    .is_some_and(|history| history.country == root.country)
                {
                    store.histories.remove(status);
                    store.histories_changed = true;
                }
            }
            root.statuses.clear();
        }
        root.seen = generation;
        root.country = country.country;
        root.diplomacy = country.diplomacy;
        if !store.history_owners.contains_key(&country.diplomacy) {
            allocation::reserve_map(&mut store.history_owners, 1).map_err(allocation_error)?;
        }
        store
            .history_owners
            .insert(country.diplomacy, (country.slot, country.country));
        if changed {
            queue_owner_seed(store, country.slot)?;
        }
    }
    store.history_roots.retain(|_, owner| {
        if owner.seen == generation {
            return true;
        }
        for status in &owner.statuses {
            if store
                .histories
                .get(status)
                .is_some_and(|history| history.country == owner.country)
            {
                store.histories.remove(status);
                store.histories_changed = true;
            }
        }
        false
    });
    if store.history_roots.len() != store.roster_owners.len() {
        return Err("war history owner census source absent");
    }
    let additional = store
        .history_roots
        .len()
        .saturating_sub(store.pending_owner_seeds.len());
    allocation::reserve(&mut store.pending_owner_seeds, additional).map_err(allocation_error)?;
    let demand = store
        .history_roots
        .len()
        .max(REQUIRED_HISTORY_EVENTS.load(Ordering::Relaxed));
    let additional = demand.saturating_sub(store.unseen_histories.len());
    allocation::reserve(&mut store.unseen_histories, additional).map_err(allocation_error)?;
    Ok(visited)
}

fn queue_owner_seed(store: &mut Store, slot: u32) -> Result<(), &'static str> {
    let root = store
        .history_roots
        .get_mut(&slot)
        .ok_or("war history owner absent")?;
    if !root.seed {
        allocation::reserve(&mut store.pending_owner_seeds, 1).map_err(allocation_error)?;
        root.seed = true;
        store.pending_owner_seeds.push(slot);
    }
    Ok(())
}

fn seed_history_owner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    slot: u32,
    store: &mut Store,
) -> Result<native::DeliverySeedWork, &'static str> {
    store.history_generation = store
        .history_generation
        .checked_add(1)
        .ok_or("war delivery seed generation exhausted")?;
    let generation = store.history_generation;
    let root = store
        .history_roots
        .get_mut(&slot)
        .ok_or("war history owner absent")?;
    let country = root.country;
    let diplomacy = root.diplomacy;
    let slots = &store.history_slots;
    let histories = &mut store.histories;
    let changed = &mut store.histories_changed;
    let mut countries = None;
    let work = native::walk_delivery_histories(
        memory,
        base,
        diplomacy,
        |raw, slot| {
            if slots.get(&raw) == Some(&slot) {
                return true;
            }
            if countries.is_none() {
                countries = engine_native::ccountry::Reader::new(memory, base, state).ok();
            }
            countries
                .as_ref()
                .is_some_and(|c| c.resolve(raw).is_ok_and(|c| c.canonical_slot == slot))
        },
        |status, target, value| {
            *changed |= histories
                .get(&status)
                .is_none_or(|h| h.country != country || h.target != target || h.value != value);
            if !histories.contains_key(&status) {
                allocation::reserve_map(histories, 1).map_err(allocation_error)?;
            }
            histories.insert(
                status,
                History {
                    country,
                    target,
                    value,
                    dirty: 0,
                    seen: generation,
                },
            );
            if !root.statuses.contains(&status) {
                allocation::reserve_set(&mut root.statuses, 1).map_err(allocation_error)?;
                root.statuses.insert(status);
            }
            Ok(())
        },
    )?;
    root.statuses.retain(|status| {
        if histories
            .get(status)
            .is_some_and(|h| h.country == country && h.seen == generation)
        {
            return true;
        }
        if histories.get(status).is_some_and(|h| h.country == country) {
            histories.remove(status);
            *changed = true;
        }
        false
    });
    root.seed = false;
    Ok(work)
}

fn seed_history_owners(
    memory: &impl Memory,
    base: usize,
    state: usize,
    store: &mut Store,
) -> Result<native::DeliverySeedWork, &'static str> {
    store.unseen_histories.retain(|event| {
        store
            .history_slots
            .get(&event.1)
            .and_then(|slot| store.history_roots.get(slot))
            .is_some_and(|root| !root.seed)
    });
    let mut total = native::DeliverySeedWork::default();
    for index in 0..store.pending_owner_seeds.len() {
        let slot = store.pending_owner_seeds[index];
        if !store.history_roots.get(&slot).is_some_and(|root| root.seed) {
            continue;
        }
        let work = seed_history_owner(memory, base, state, slot, store)?;
        SEED_OWNERS.fetch_add(1, Ordering::Relaxed);
        SEED_MATRIX.fetch_add(work.matrix_entries as u64, Ordering::Relaxed);
        SEED_STATUSES.fetch_add(work.status_entries as u64, Ordering::Relaxed);
        SEED_NONZERO.fetch_add(work.nonzero_histories as u64, Ordering::Relaxed);
        total.matrix_entries += work.matrix_entries;
        total.status_entries += work.status_entries;
        total.nonzero_histories += work.nonzero_histories;
    }
    store.pending_owner_seeds.clear();
    let additional = store
        .histories
        .len()
        .saturating_sub(store.pending_histories.len());
    allocation::reserve(&mut store.pending_histories, additional).map_err(allocation_error)?;
    Ok(total)
}

fn read_marked(memory: &impl Memory, store: &mut Store) -> Result<(), &'static str> {
    for index in 0..store.pending_scores.len() {
        let ledger = store.pending_scores[index];
        if let Some((held, side)) = held_mut(store, ledger) {
            let mask = held.dirty[side];
            let scores = &mut held.ledgers[side].scores;
            let previous = *scores;
            native::read_changed_scores(memory, ledger, mask, scores)?;
            let different = *scores != previous;
            held.dirty[side] = 0;
            store.scores_changed |= different;
            READ_FIELDS.fetch_add(mask.count_ones() as u64, Ordering::Relaxed);
        }
    }
    store.pending_scores.clear();
    for index in 0..store.pending_histories.len() {
        let status = store.pending_histories[index];
        let Some(history) = store.histories.get_mut(&status) else {
            continue;
        };
        let value = read_history_columns(memory, status, history.dirty, history.value)?;
        store.histories_changed |= history.value != value;
        history.value = value;
        history.dirty = 0;
        if value == native::DeliveryHistory::default() {
            remove_history(store, status);
        }
    }
    store.pending_histories.clear();
    store.unseen_histories.sort_unstable_by_key(|event| event.0);
    let mut index = 0;
    while index < store.unseen_histories.len() {
        let (status, country, target, mut mask) = store.unseen_histories[index];
        index += 1;
        while index < store.unseen_histories.len() && store.unseen_histories[index].0 == status {
            let event = store.unseen_histories[index];
            if (event.1, event.2) != (country, target) {
                return Err("war delivery identity changed without lifetime invalidation");
            }
            mask |= event.3;
            index += 1;
        }
        let Some(&owner_slot) = store.history_slots.get(&country) else {
            continue;
        };
        if !store.history_roots.contains_key(&owner_slot) {
            continue;
        }
        let before = store
            .histories
            .get(&status)
            .map_or(native::DeliveryHistory::default(), |h| h.value);
        let value = read_history_columns(memory, status, mask, before)?;
        store.histories_changed |= before != value;
        if value == native::DeliveryHistory::default() {
            remove_history(store, status);
        } else {
            if !store.histories.contains_key(&status) {
                allocation::reserve_map(&mut store.histories, 1).map_err(allocation_error)?;
            }
            let owner = *store
                .history_slots
                .get(&country)
                .ok_or("war delivery owner census identity absent")?;
            let root = store
                .history_roots
                .get_mut(&owner)
                .ok_or("war delivery owner root absent")?;
            if !root.statuses.contains(&status) {
                allocation::reserve_set(&mut root.statuses, 1).map_err(allocation_error)?;
                root.statuses.insert(status);
            }
            store.histories.insert(
                status,
                History {
                    country,
                    target,
                    value,
                    dirty: 0,
                    seen: 0,
                },
            );
        }
    }
    store.unseen_histories.clear();
    let additional = store
        .histories
        .len()
        .saturating_sub(store.pending_histories.len());
    allocation::reserve(&mut store.pending_histories, additional).map_err(allocation_error)?;
    Ok(())
}

fn remove_history(store: &mut Store, status: usize) {
    if let Some(history) = store.histories.remove(&status)
        && let Some(root) = store
            .history_slots
            .get(&history.country)
            .and_then(|slot| store.history_roots.get_mut(slot))
    {
        root.statuses.remove(&status);
    }
}

fn read_history_columns(
    memory: &impl Memory,
    status: usize,
    mask: u8,
    mut value: native::DeliveryHistory,
) -> Result<native::DeliveryHistory, &'static str> {
    for (field, offset) in native::DELIVERY_FIELDS.into_iter().enumerate() {
        if mask & (1 << field) == 0 {
            continue;
        }
        let raw = status
            .checked_add(offset)
            .and_then(|p| engine_native_memory::read::<i64>(memory, p))
            .ok_or("changed war lend-lease history unreadable")?;
        match field {
            0 => value.industry_sent = raw,
            1 => value.fuel_sent = raw,
            2 => value.industry_received = raw,
            3 => value.fuel_received = raw,
            _ => unreachable!(),
        }
    }
    READ_FIELDS.fetch_add(mask.count_ones() as u64, Ordering::Relaxed);
    Ok(value)
}

fn publish(store: &mut Store) -> Result<[Arc<TableRows>; 3], &'static str> {
    let mut current = store.rows.take();
    let scores = if !store.scores_changed
        && let Some(current) = current.as_ref()
    {
        Arc::clone(&current[0])
    } else {
        let mut rows = allocation::vec(
            store
                .held
                .len()
                .checked_mul(2)
                .ok_or("war participation row count overflow")?,
        )
        .map_err(allocation_error)?;
        for held in store.held.values() {
            for (direction, ledger) in held.ledgers.iter().enumerate() {
                let s = ledger.scores;
                rows.push(WarParticipationRow {
                    war_first: held.first,
                    war_second: held.second,
                    direction: direction as u32,
                    first_country: s.first,
                    second_country: s.second,
                    equipment_damage: s.equipment_damage,
                    province_capture: s.province_capture,
                    air_damage: s.air_damage,
                    strategic_bombing: s.strategic_bombing,
                    sunk_ships: s.sunk_ships,
                    sunk_convoys: s.sunk_convoys,
                    casualties: s.casualties,
                    unknown_casualties: s.unknown_casualties,
                    lend_lease_sent: s.lend_lease_sent,
                    lend_lease_received: s.lend_lease_received,
                });
            }
        }
        rows.sort_unstable_by_key(|r| (r.war_first, r.war_second, r.direction));
        if rows.windows(2).any(|r| {
            (r[0].war_first, r[0].war_second, r[0].direction)
                == (r[1].war_first, r[1].war_second, r[1].direction)
        }) {
            return Err("duplicate live war participation endpoints");
        }
        allocation::boxed(TableRows::WarParticipation(rows))
            .map(Arc::from)
            .map_err(allocation_error)?
    };
    let captures = if !store.captures_changed
        && let Some(current) = current.as_ref()
    {
        Arc::clone(&current[1])
    } else {
        let count = store
            .held
            .values()
            .try_fold(0usize, |sum, h| {
                h.ledgers
                    .iter()
                    .try_fold(sum, |sum, l| sum.checked_add(l.captured_provinces.len()))
            })
            .ok_or("war captured-province row count overflow")?;
        let mut rows = allocation::vec(count).map_err(allocation_error)?;
        for held in store.held.values() {
            for (direction, ledger) in held.ledgers.iter().enumerate() {
                for &province in &ledger.captured_provinces {
                    rows.push(WarCapturedProvinceRow {
                        war_first: held.first,
                        war_second: held.second,
                        direction: direction as u32,
                        province,
                        credited: 1,
                    });
                }
            }
        }
        rows.sort_unstable_by_key(|r| (r.war_first, r.war_second, r.direction, r.province as u32));
        allocation::boxed(TableRows::WarCapturedProvinces(rows))
            .map(Arc::from)
            .map_err(allocation_error)?
    };
    let histories = if !store.histories_changed
        && let Some(current) = current.as_ref()
    {
        Arc::clone(&current[2])
    } else {
        let mut rows = allocation::vec(store.histories.len()).map_err(allocation_error)?;
        for history in store.histories.values() {
            let h = history.value;
            rows.push(WarLendLeaseHistoryRow {
                country: history.country,
                target: history.target,
                industry_sent: h.industry_sent,
                fuel_sent: h.fuel_sent,
                industry_received: h.industry_received,
                fuel_received: h.fuel_received,
            });
        }
        rows.sort_unstable_by_key(|r| (r.country, r.target));
        allocation::boxed(TableRows::WarLendLeaseHistories(rows))
            .map(Arc::from)
            .map_err(allocation_error)?
    };
    let rows = [scores, captures, histories];
    store.rows = Some(rows.clone());
    store.scores_changed = false;
    store.captures_changed = false;
    store.histories_changed = false;
    store.charge.set(raw_memory::Capacity {
        payload_vectors: store
            .held
            .values()
            .flat_map(|h| h.ledgers.iter())
            .map(|l| raw_memory::vector_bytes(&l.captured_provinces))
            .sum(),
        auxiliary_vectors: raw_memory::vector_bytes(&store.events)
            + raw_memory::vector_bytes(&store.pending_scores)
            + raw_memory::vector_bytes(&store.pending_histories)
            + raw_memory::vector_bytes(&store.unseen_histories)
            + raw_memory::vector_bytes(&store.pending_owner_seeds),
        index_map_entries_lower_bound: (store.held.capacity()
            * std::mem::size_of::<(usize, Held)>()
            + store.histories.capacity() * std::mem::size_of::<(usize, History)>()
            + store.history_owners.capacity() * std::mem::size_of::<(usize, (u32, u32))>()
            + store.history_slots.capacity() * std::mem::size_of::<(u32, u32)>()
            + store.history_roots.capacity() * std::mem::size_of::<(u32, HistoryOwner)>()
            + store.roster_owners.capacity() * std::mem::size_of::<u32>()
            + store
                .history_roots
                .values()
                .map(|r| r.statuses.capacity() * std::mem::size_of::<usize>())
                .sum::<usize>()) as u64,
        ..Default::default()
    });
    current.take();
    Ok(rows)
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// Caller owns this family and the authenticated process image, and retains
    /// every patch and callback until restoration and native quiescence.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if SITES.iter().all(|s| {
            engine_sites::hook(base, s.rva)
                .and_then(|code| code.sentinel::<16>())
                .is_ok()
        }) {
            let callbacks = [
                bombing_decay as *const () as usize,
                air_damage as *const () as usize,
                equipment_damage as *const () as usize,
                destroyed_unit as *const () as usize,
                province_capture as *const () as usize,
                strategic_bombing as *const () as usize,
                strength_damage as *const () as usize,
                sunk_convoys as *const () as usize,
                sunk_ships as *const () as usize,
                casualty_score as *const () as usize,
                replace_ledger as *const () as usize,
                fuel_sent as *const () as usize,
                industry_sent as *const () as usize,
                fuel_received as *const () as usize,
                industry_received as *const () as usize,
            ];
            allocation::reserve(&mut hooks, SITES.len()).map_err(allocation_error)?;
            for (index, site) in SITES.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| "war participation hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        ENABLED.store(false, Ordering::SeqCst);
        LOST.store(true, Ordering::SeqCst);
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "war participation hook state change failed")?
        {
            return Ok(false);
        }
        let observing = enabled && !self.hooks.is_empty();
        war_changes::observe_participation(observing.then_some(lifetime_changed));
        ENABLED.store(observing, Ordering::SeqCst);
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        if active() == 0 && self.hooks.iter().all(|h| !h.is_enabled()) {
            war_changes::observe_participation(None);
            if let Ok(mut slot) = STORE.try_lock() {
                *slot = None;
            }
        }
    }
}
macro_rules! scalar {
    ($name:ident,$index:expr, ($($arg:ident:$ty:ty),*)) => {
        unsafe extern "system-unwind" fn $name(ledger: usize, $($arg:$ty),*) {
            let flight = Flight::enter($index);
            let original: unsafe extern "system-unwind" fn(usize,$($ty),*) = unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            flight.timing.original(|| unsafe { original(ledger,$($arg),*) });
            changed(ledger,SITES[$index].columns,None);
            flight.finish();
        }
    }
}
scalar!(bombing_decay, 0, ());
scalar!(air_damage,1,(target:usize, losses:usize));
scalar!(equipment_damage,2,(equipment:usize));
scalar!(destroyed_unit,3,(unit:usize));
scalar!(strategic_bombing,5,(amount:i64));
scalar!(strength_damage,6,(amount:i64, first:u8, second:u8, country:u32));
scalar!(sunk_convoys,7,(losses:usize));
scalar!(sunk_ships,8,(ship:usize));
unsafe extern "system-unwind" fn province_capture(ledger: usize, province: usize, country: u32) {
    let flight = Flight::enter(4);
    let original: unsafe extern "system-unwind" fn(usize, usize, u32) =
        unsafe { std::mem::transmute(ORIGINAL[4].load(Ordering::Acquire)) };
    let wanted = match STORE.try_lock() {
        Ok(mut slot) => slot.as_mut().and_then(|s| held_mut(s, ledger)).is_some(),
        Err(_) => {
            refuse();
            false
        }
    };
    let id = if wanted {
        province
            .checked_add(engine_native::cprovince::ID)
            .and_then(|p| {
                engine_native_memory::read::<i32>(&unsafe { engine_native_memory::memory::LiveMemory::new() }, p)
            })
    } else {
        None
    };
    flight
        .timing
        .original(|| unsafe { original(ledger, province, country) });
    if wanted && id.is_none() {
        refuse();
    }
    changed(ledger, SITES[4].columns, id);
    flight.finish();
}
unsafe extern "system-unwind" fn casualty_score(
    ledger: usize,
    known: i32,
    unknown: i32,
    capitulated: u8,
) {
    let flight = Flight::enter(9);
    let changed_columns = casualty_changed(ledger, known, unknown, capitulated != 0);
    let original: unsafe extern "system-unwind" fn(usize, i32, i32, u8) =
        unsafe { std::mem::transmute(ORIGINAL[9].load(Ordering::Acquire)) };
    flight
        .timing
        .original(|| unsafe { original(ledger, known, unknown, capitulated) });
    if changed_columns != 0 {
        changed(ledger, changed_columns, None);
    }
    flight.finish();
}
unsafe extern "system-unwind" fn replace_ledger(
    relation: usize,
    first: u32,
    second: u32,
    source: usize,
) {
    let flight = Flight::enter(10);
    replace(relation);
    let original: unsafe extern "system-unwind" fn(usize, u32, u32, usize) =
        unsafe { std::mem::transmute(ORIGINAL[10].load(Ordering::Acquire)) };
    flight
        .timing
        .original(|| unsafe { original(relation, first, second, source) });
    replace(relation);
    flight.finish();
}
macro_rules! delivery {
    ($name:ident,$index:expr,$field:expr) => {
        unsafe extern "system-unwind" fn $name(status: usize, amount: i64) {
            let flight = Flight::enter($index);
            let original: unsafe extern "system-unwind" fn(usize, i64) =
                unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            flight
                .timing
                .original(|| unsafe { original(status, amount) });
            if amount != 0 {
                history_changed(status, $field);
            }
            flight.finish();
        }
    };
}
delivery!(fuel_sent, 11, 1);
delivery!(industry_sent, 12, 0);
delivery!(fuel_received, 13, 3);
delivery!(industry_received, 14, 2);

#[cfg(test)]
mod tests {
    use super::*;

    fn held(first: u32, second: u32) -> Held {
        Held {
            first,
            second,
            ledgers: Default::default(),
            signatures: [None; 2],
            seed: false,
            seen: 0,
            revision: 0,
            dirty: [0; 2],
        }
    }

    fn history_root(store: &mut Store, slot: u32, country: u32, diplomacy: usize) {
        store.history_owners.insert(diplomacy, (slot, country));
        store.history_slots.insert(country, slot);
        store.history_roots.insert(
            slot,
            HistoryOwner {
                country,
                diplomacy,
                statuses: HashSet::new(),
                seen: store.generation,
                seed: false,
            },
        );
    }

    struct SeedWorld {
        count: u32,
        matrices: std::cell::Cell<usize>,
        statuses: std::cell::Cell<usize>,
        roots: std::cell::Cell<usize>,
        allowed: std::cell::Cell<Option<u64>>,
        zero: std::cell::Cell<bool>,
        zero_owner: std::cell::Cell<Option<u32>>,
    }
    impl SeedWorld {
        const ROOT: usize = 0x10000;
        const ROOT_STRIDE: usize = 0x1000;
        const MATRIX: usize = 0x1000_0000;
        const MATRIX_STRIDE: usize = 0x20000;
        const STATUS: usize = 0x1_0000_0000;
        const STATUS_STRIDE: usize = 0x1000;
        fn new(count: u32) -> Self {
            Self {
                count,
                matrices: 0.into(),
                statuses: 0.into(),
                roots: 0.into(),
                allowed: None.into(),
                zero: false.into(),
                zero_owner: None.into(),
            }
        }
        fn root(slot: u32) -> usize {
            Self::ROOT + slot as usize * Self::ROOT_STRIDE
        }
        fn status(&self, owner: u32, target: u32) -> usize {
            Self::STATUS
                + (owner as usize * self.count as usize + target as usize) * Self::STATUS_STRIDE
        }
        fn check(&self, owner: u32) {
            assert!(
                owner < self.count,
                "native root outside recorded war rosters"
            );
            if let Some(mask) = self.allowed.get() {
                assert!(
                    owner < 64 && mask & (1 << owner) != 0,
                    "unaffected owner was re-read"
                );
            }
        }
        fn record(&self, pointer: usize) -> [u8; 52] {
            assert_eq!((pointer - Self::STATUS) % Self::STATUS_STRIDE, 0);
            let id = (pointer - Self::STATUS) / Self::STATUS_STRIDE;
            let (owner, target) = (
                (id / self.count as usize) as u32,
                (id % self.count as usize) as u32,
            );
            self.check(owner);
            let mut bytes = [0u8; 52];
            bytes[..8].copy_from_slice(
                &(engine_native::crelation::status::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            bytes[8..12].copy_from_slice(&(target + 1).to_le_bytes());
            bytes[12..20].copy_from_slice(&(Self::root(owner) as u64).to_le_bytes());
            if !self.zero.get()
                && self.zero_owner.get() != Some(owner)
                && [1, 2]
                    .into_iter()
                    .any(|n| target == (owner + n) % self.count)
            {
                bytes[20..28].copy_from_slice(&7i64.to_le_bytes());
            }
            bytes
        }
        fn countries(
            &self,
        ) -> impl Iterator<Item = engine_native::cwar_relation::overview::CountrySource> {
            (0..=self.count).map(
                |slot| engine_native::cwar_relation::overview::CountrySource {
                    slot,
                    country: slot + 1,
                    diplomacy: Self::root(slot),
                },
            )
        }
        fn row(owner: u32, target: u32, count: u32) -> OwnedRow {
            OwnedRow {
                owner: owner + 1,
                ordinal: target,
                first_country: owner + 1,
                second_country: target + 1,
                source: engine_native::cwar_relation::overview::Source {
                    relation: 0x1000 + (owner as usize * count as usize + target as usize) * 0x200,
                    diplomacy: Self::root(owner),
                    owner_slot: owner,
                    first_slot: owner,
                    second_slot: target,
                },
                ..Default::default()
            }
        }
    }
    impl Memory for SeedWorld {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            if address >= Self::STATUS {
                let offset = (address - Self::STATUS) % Self::STATUS_STRIDE;
                let bytes = self.record(address - offset);
                let at = match offset {
                    engine_native::crelation::status::TARGET_COUNTRY => 8,
                    engine_native::crelation::status::DIPLOMACY_STATUS => 12,
                    0 => 0,
                    offset
                        if (native::DELIVERY_FIELDS[0]..native::DELIVERY_FIELDS[0] + 32)
                            .contains(&offset) =>
                    {
                        20 + offset - native::DELIVERY_FIELDS[0]
                    }
                    _ => panic!("unexpected synthetic status field"),
                };
                out.copy_from_slice(&bytes[at..at + out.len()]);
                return true;
            }
            if address >= Self::MATRIX {
                let owner = ((address - Self::MATRIX) / Self::MATRIX_STRIDE) as u32;
                let first = ((address - Self::MATRIX) % Self::MATRIX_STRIDE) / 8;
                self.check(owner);
                self.matrices.set(self.matrices.get() + out.len() / 8);
                for (index, bytes) in out.chunks_exact_mut(8).enumerate() {
                    bytes.copy_from_slice(
                        &(self.status(owner, (first + index) as u32) as u64).to_le_bytes(),
                    );
                }
                return true;
            }
            let owner = ((address - Self::ROOT) / Self::ROOT_STRIDE) as u32;
            self.check(owner);
            assert_eq!(
                address,
                Self::root(owner) + engine_native_parts::cdiplomacy_status_layout::RELATIONS
            );
            assert_eq!(out.len(), 16);
            self.roots.set(self.roots.get() + 1);
            out[..8].copy_from_slice(
                &((Self::MATRIX + owner as usize * Self::MATRIX_STRIDE) as u64).to_le_bytes(),
            );
            out[8..12].copy_from_slice(&(self.count as i32).to_le_bytes());
            out[12..16].copy_from_slice(&(self.count as i32).to_le_bytes());
            true
        }
        fn copy_records(
            &self,
            sources: engine_native_memory::memory::RecordSources<'_>,
            layout: &engine_native_memory::memory::RecordLayout<'_>,
            out: &mut [u8],
        ) -> usize {
            assert_eq!(layout.stride(), 52);
            assert_eq!(layout.fields().len(), 4);
            self.statuses.set(self.statuses.get() + sources.len());
            for index in 0..sources.len() {
                out[index * 52..index * 52 + 52]
                    .copy_from_slice(&self.record(sources.pointer(index).unwrap()));
            }
            sources.len() * layout.fields().len()
        }
    }

    #[test]
    fn participation_seed_work_is_roster_plus_native_entries_for_overlapping_sides() {
        let count = 10u32;
        let world = SeedWorld::new(count);
        let half = count / 2;
        let rows = (0..count).flat_map(|owner| {
            (0..half)
                .filter(move |&member| member != owner % half)
                .map(move |member| {
                    SeedWorld::row(
                        owner,
                        if owner < half { half + member } else { member },
                        count,
                    )
                })
        });
        let mut store = Store {
            generation: 1,
            ..Default::default()
        };
        let roster_entries =
            reconcile_histories(rows, world.countries(), &mut store, true).unwrap();
        assert_eq!(roster_entries, count as usize * (half - 1) as usize);
        assert_eq!(store.history_roots.len(), count as usize);
        let work = seed_history_owners(&world, 0, 0, &mut store).unwrap();
        let native_entries = count as usize * count as usize;
        assert_eq!(work.matrix_entries, native_entries);
        assert_eq!(work.status_entries, native_entries);
        assert_eq!(world.matrices.get(), native_entries);
        assert_eq!(world.statuses.get(), native_entries);
        assert_eq!(world.roots.get(), count as usize);
        assert_eq!(
            roster_entries + work.status_entries,
            140
        );
        assert_eq!(work.nonzero_histories, count as usize * 2);
        assert_eq!(store.histories.len(), count as usize * 2);
        assert!(store.histories.capacity() < count as usize * 4);
        assert!(
            store
                .history_roots
                .values()
                .all(|root| root.statuses.len() == 2)
        );
        world.allowed.set(Some(1));
        world.zero_owner.set(Some(0));
        queue_owner_seed(&mut store, 0).unwrap();
        let work = seed_history_owners(&world, 0, 0, &mut store).unwrap();
        assert_eq!(work.status_entries, count as usize);
        assert_eq!(work.nonzero_histories, 0);
        assert_eq!(store.histories.len(), count as usize * 2 - 2);
    }

    #[test]
    fn participation_zero_history_seed_allocates_nothing_and_pure_revisions_carry_owners() {
        let world = SeedWorld::new(4);
        world.zero.set(true);
        let mut store = Store {
            generation: 1,
            ..Default::default()
        };
        let rows = [SeedWorld::row(0, 1, 4), SeedWorld::row(1, 0, 4)];
        reconcile_histories(rows, world.countries(), &mut store, true).unwrap();
        let work = collector_test_support::with_allocations_denied(|| {
            seed_history_owners(&world, 0, 0, &mut store)
        })
        .unwrap();
        assert_eq!(work.status_entries, 8);
        assert!(store.histories.is_empty());
        assert_eq!(store.histories.capacity(), 0);
        assert!(
            store
                .history_roots
                .values()
                .all(|root| root.statuses.capacity() == 0)
        );
        world.allowed.set(Some(0));
        store.generation += 1;
        reconcile_histories(rows, world.countries(), &mut store, false).unwrap();
        assert!(store.pending_owner_seeds.is_empty());
        assert_eq!(
            seed_history_owners(&world, 0, 0, &mut store).unwrap(),
            native::DeliverySeedWork::default()
        );
    }

    #[test]
    fn participation_lifetime_and_roster_changes_seed_only_affected_owners_and_wars() {
        let _lock = TEST_LOCK.lock().unwrap();
        let world = SeedWorld::new(4);
        let mut store = Store {
            generation: 1,
            ..Default::default()
        };
        let rows = [SeedWorld::row(0, 1, 4), SeedWorld::row(1, 0, 4)];
        reconcile_histories(rows, world.countries(), &mut store, true).unwrap();
        seed_history_owners(&world, 0, 0, &mut store).unwrap();
        let untouched = store.histories[&world.status(1, 2)].value;
        store
            .history_roots
            .get_mut(&0)
            .unwrap()
            .statuses
            .insert(world.status(1, 2));
        store.held.insert(0x10000, held(1, 2));
        store.held.insert(0x20000, held(3, 4));
        let mut status = vec![0u8; engine_native::crelation::status::WAR_RELATION + 8];
        status[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&(SeedWorld::root(0) as u64).to_le_bytes());
        status[engine_native::crelation::status::WAR_RELATION..][..8]
            .copy_from_slice(&0x10000u64.to_le_bytes());
        *STORE.lock().unwrap() = Some(store);
        ENABLED.store(true, Ordering::SeqCst);
        LOST.store(false, Ordering::SeqCst);
        collector_test_support::with_allocations_denied(|| {
            lifetime_changed(
                war_changes::Site::ReplaceRelation,
                status.as_ptr() as usize,
                0,
            );
            lifetime_changed(
                war_changes::Site::ReplaceRelation,
                status.as_ptr() as usize,
                0,
            );
        });
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.pending_owner_seeds, [0]);
        assert!(store.held[&0x10000].seed);
        assert!(!store.held[&0x20000].seed);
        world.allowed.set(Some(1));
        world.zero_owner.set(Some(0));
        store.generation += 1;
        reconcile_histories(rows, world.countries(), store, false).unwrap();
        assert_eq!(
            seed_history_owners(&world, 0, 0, store)
                .unwrap()
                .status_entries,
            4
        );
        assert_eq!(store.histories[&world.status(1, 2)].value, untouched);
        let expanded = rows.into_iter().chain([SeedWorld::row(0, 2, 4)]);
        store.generation += 1;
        reconcile_histories(expanded, world.countries(), store, false).unwrap();
        assert_eq!(store.pending_owner_seeds, [2]);
        world.allowed.set(Some(1 << 2));
        assert_eq!(
            seed_history_owners(&world, 0, 0, store)
                .unwrap()
                .status_entries,
            4
        );
        store.generation += 1;
        reconcile_histories(rows, world.countries(), store, false).unwrap();
        world.allowed.set(Some(0));
        assert_eq!(
            seed_history_owners(&world, 0, 0, store)
                .unwrap()
                .status_entries,
            0
        );
        assert!(!store.history_roots.contains_key(&2));
        assert!(store.histories.values().all(|history| history.country != 3));
        assert!(!LOST.load(Ordering::SeqCst));
        ENABLED.store(false, Ordering::SeqCst);
        *slot = None;
    }

    #[test]
    fn participation_token_revisions_preserve_unaffected_owner_and_war_certificates() {
        let _lock = TEST_LOCK.lock().unwrap();
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                ENABLED.store(false, Ordering::SeqCst);
                *STORE
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                for family in [
                    war_changes::Forwarder::CountryInputs,
                    war_changes::Forwarder::Opinions,
                    war_changes::Forwarder::Relations,
                ] {
                    war_changes::forwarder(family, false);
                }
            }
        }
        let _restore = Restore;
        for family in [
            war_changes::Forwarder::CountryInputs,
            war_changes::Forwarder::Opinions,
            war_changes::Forwarder::Relations,
        ] {
            war_changes::forwarder(family, true);
        }
        const WARS: usize = 0x8000_0000;
        const STRIDE: usize = 0x400;
        const TREE_NIL: usize = 0x19;
        struct World {
            histories: SeedWorld,
            bytes: Vec<u8>,
            scores: std::cell::Cell<usize>,
            allowed_war: std::cell::Cell<Option<usize>>,
        }
        impl Memory for World {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                if (WARS..WARS + self.bytes.len()).contains(&address) {
                    let index = (address - WARS) / STRIDE;
                    if let Some(allowed) = self.allowed_war.get() {
                        assert_eq!(index, allowed, "unaffected war was re-seeded");
                    }
                    if out.len() == 0x60 {
                        self.scores.set(self.scores.get() + 1);
                    }
                    return engine_native_memory::memory::SliceMemory {
                        base: WARS,
                        bytes: &self.bytes,
                    }
                    .copy(address, out);
                }
                self.histories.copy(address, out)
            }
            fn copy_records(
                &self,
                sources: engine_native_memory::memory::RecordSources<'_>,
                layout: &engine_native_memory::memory::RecordLayout<'_>,
                out: &mut [u8],
            ) -> usize {
                self.histories.copy_records(sources, layout, out)
            }
        }
        let mut bytes = vec![0u8; STRIDE * 2];
        for war in 0..2 {
            let start = war * STRIDE;
            let head = start + 0x200;
            bytes[head + TREE_NIL] = 1;
            for at in [0, 8, 16] {
                bytes[head + at..head + at + 8]
                    .copy_from_slice(&((WARS + head) as u64).to_le_bytes());
            }
            for (side, ledger) in native::DIRECTIONAL_LEDGERS.into_iter().enumerate() {
                let at = start + ledger;
                bytes[at..at + 8]
                    .copy_from_slice(&(native::LEDGER_VTABLE_RVA.0 as u64).to_le_bytes());
                bytes[at + native::FIRST..at + native::FIRST + 4]
                    .copy_from_slice(&((war * 2 + side + 1) as u32).to_le_bytes());
                bytes[at + native::SECOND..at + native::SECOND + 4]
                    .copy_from_slice(&((war * 2 + 2 - side) as u32).to_le_bytes());
                bytes[at + native::CAPTURED_PROVINCES..at + native::CAPTURED_PROVINCES + 8]
                    .copy_from_slice(&((WARS + head) as u64).to_le_bytes());
            }
        }
        let world = World {
            histories: SeedWorld::new(4),
            bytes,
            scores: 0.into(),
            allowed_war: None.into(),
        };
        let mut sources = [SeedWorld::row(0, 1, 4), SeedWorld::row(2, 3, 4)];
        for (index, row) in sources.iter_mut().enumerate() {
            row.source.relation = WARS + index * STRIDE;
        }
        *STORE.lock().unwrap() = None;
        ENABLED.store(true, Ordering::SeqCst);
        let first =
            capture_inner(&world, 0, 0, &sources, world.histories.countries(), true).unwrap();
        assert_eq!(world.scores.get(), 4);
        assert_eq!(world.histories.statuses.get(), 16);
        world.allowed_war.set(Some(usize::MAX));
        world.histories.allowed.set(Some(0));
        drop(war_changes::forwarded(war_changes::Site::SetInstigator));
        let carried =
            capture_inner(&world, 0, 0, &sources, world.histories.countries(), false).unwrap();
        assert_eq!(world.scores.get(), 4);
        assert_eq!(world.histories.statuses.get(), 16);
        for index in 0..3 {
            assert!(Arc::ptr_eq(&first[index], &carried[index]));
        }
        let mut status = vec![0u8; engine_native::crelation::status::WAR_RELATION + 8];
        status[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&(SeedWorld::root(0) as u64).to_le_bytes());
        let mutation = war_changes::forwarded(war_changes::Site::StatusReset);
        lifetime_changed(war_changes::Site::StatusReset, status.as_ptr() as usize, 0);
        world.histories.allowed.set(Some(1));
        world.histories.zero_owner.set(Some(0));
        drop(mutation);
        let changed =
            capture_inner(&world, 0, 0, &sources, world.histories.countries(), false).unwrap();
        assert_eq!(world.scores.get(), 4);
        assert_eq!(world.histories.statuses.get(), 20);
        assert_eq!(changed[2].len(), 6);
        assert!(Arc::ptr_eq(&first[0], &changed[0]));
        assert!(Arc::ptr_eq(&first[1], &changed[1]));
        world.allowed_war.set(Some(0));
        world.histories.allowed.set(Some(0));
        replace(WARS);
        let replaced =
            capture_inner(&world, 0, 0, &sources, world.histories.countries(), false).unwrap();
        assert_eq!(world.scores.get(), 6);
        assert_eq!(world.histories.statuses.get(), 20);
        assert!(Arc::ptr_eq(&changed[2], &replaced[2]));
    }

    #[test]
    fn participation_delivery_events_retain_non_same_side_histories_without_alias_or_graph_work() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut bytes = vec![0u8; 0x800];
        let status = bytes.as_ptr() as usize;
        let diplomacy = 0x12300usize;
        bytes[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&(diplomacy as u64).to_le_bytes());
        bytes[engine_native::crelation::status::TARGET_COUNTRY..][..4]
            .copy_from_slice(&99u32.to_le_bytes());
        bytes[native::DELIVERY_FIELDS[0]..][..8].copy_from_slice(&7i64.to_le_bytes());
        let mut store = Store::default();
        history_root(&mut store, 1, 11, diplomacy);
        store.unseen_histories.reserve(1);
        *STORE.lock().unwrap() = Some(store);
        LOST.store(false, Ordering::SeqCst);
        collector_test_support::with_allocations_denied(|| history_changed(status, 0));
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.unseen_histories, [(status, 11, 99, 1)]);
        read_marked(
            &engine_native_memory::memory::SliceMemory {
                base: status,
                bytes: &bytes,
            },
            store,
        )
        .unwrap();
        assert_eq!(store.histories[&status].target, 99);
        assert_eq!(store.histories[&status].value.industry_sent, 7);
        assert!(store.history_roots[&1].statuses.contains(&status));
        assert!(!LOST.load(Ordering::SeqCst));
        *slot = None;
    }

    #[test]
    fn participation_province_hook_queues_identity_and_worker_deduplicates_first_credit() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut store = Store::default();
        let mut value = held(1, 2);
        value.ledgers[0].captured_provinces = (0..5_000).collect();
        store.held.insert(0x10000, value);
        store.events.reserve(3);
        let events = store.events.as_ptr();
        *STORE.lock().unwrap() = Some(store);
        LOST.store(false, Ordering::SeqCst);
        let ledger = 0x10000 + native::DIRECTIONAL_LEDGERS[0];
        collector_test_support::with_allocations_denied(|| {
            changed(ledger, SITES[4].columns, Some(4_999))
        });
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.events, [(ledger, 4_999, 0)]);
        assert!(store.pending_scores.is_empty());
        read_province_events(store).unwrap();
        assert!(store.pending_scores.is_empty());
        assert!(!store.captures_changed);
        drop(slot);
        collector_test_support::with_allocations_denied(|| {
            changed(ledger, SITES[4].columns, Some(-7));
            changed(ledger, SITES[4].columns, Some(-7));
        });
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.events.as_ptr(), events);
        assert_eq!(store.events.len(), 2);
        read_province_events(store).unwrap();
        assert_eq!(store.pending_scores, [ledger]);
        let held = &store.held[&0x10000];
        assert_eq!(held.dirty[0], SITES[4].columns);
        assert_eq!(held.ledgers[0].captured_provinces.len(), 5_001);
        assert_eq!(held.ledgers[0].captured_provinces[0], -7);
        assert!(store.captures_changed);
        store.events.push((ledger, -8, 0));
        store.held.get_mut(&0x10000).unwrap().revision = 1;
        read_province_events(store).unwrap();
        assert!(
            store.held[&0x10000].ledgers[0]
                .captured_provinces
                .binary_search(&-8)
                .is_err()
        );
        assert!(!LOST.load(Ordering::SeqCst));
        *slot = None;
    }

    #[test]
    fn participation_delivery_events_admit_nonzero_histories_and_remove_zero_rows() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut bytes = vec![0u8; 0x800];
        let status = bytes.as_ptr() as usize;
        let diplomacy = 0x12300usize;
        bytes[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&(diplomacy as u64).to_le_bytes());
        bytes[engine_native::crelation::status::TARGET_COUNTRY..][..4]
            .copy_from_slice(&22u32.to_le_bytes());
        bytes[native::DELIVERY_FIELDS[0]..][..8].copy_from_slice(&7i64.to_le_bytes());
        bytes[native::DELIVERY_FIELDS[1]..][..8].copy_from_slice(&11i64.to_le_bytes());
        let mut store = Store::default();
        history_root(&mut store, 1, 11, diplomacy);
        store.history_slots.insert(22, 2);
        store.unseen_histories.reserve(3);
        let events = store.unseen_histories.as_ptr();
        *STORE.lock().unwrap() = Some(store);
        LOST.store(false, Ordering::SeqCst);
        bytes[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&((diplomacy + 8) as u64).to_le_bytes());
        bytes[engine_native::crelation::status::TARGET_COUNTRY..][..4].fill(0);
        collector_test_support::with_allocations_denied(|| history_changed(status, 0));
        assert!(!LOST.load(Ordering::SeqCst));
        bytes[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&(diplomacy as u64).to_le_bytes());
        bytes[engine_native::crelation::status::TARGET_COUNTRY..][..4]
            .copy_from_slice(&22u32.to_le_bytes());
        collector_test_support::with_allocations_denied(|| {
            history_changed(status, 0);
            history_changed(status, 0);
            history_changed(status, 1);
        });
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.unseen_histories.len(), 3);
        assert_eq!(store.unseen_histories.as_ptr(), events);
        read_marked(
            &engine_native_memory::memory::SliceMemory {
                base: status,
                bytes: &bytes,
            },
            store,
        )
        .unwrap();
        assert_eq!(store.histories.len(), 1);
        let rows = publish(store).unwrap();
        assert_eq!(rows[2].len(), 1);
        assert_eq!(store.histories[&status].value.industry_sent, 7);
        assert_eq!(store.histories[&status].value.fuel_sent, 11);
        drop(slot);
        bytes[native::DELIVERY_FIELDS[0]..][..8].fill(0);
        bytes[native::DELIVERY_FIELDS[1]..][..8].fill(0);
        collector_test_support::with_allocations_denied(|| {
            history_changed(status, 0);
            history_changed(status, 1);
        });
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.pending_histories, [status]);
        read_marked(
            &engine_native_memory::memory::SliceMemory {
                base: status,
                bytes: &bytes,
            },
            store,
        )
        .unwrap();
        assert!(store.histories.is_empty());
        assert_eq!(publish(store).unwrap()[2].len(), 0);
        assert!(!LOST.load(Ordering::SeqCst));
        *slot = None;
    }

    #[test]
    fn participation_delivery_event_with_an_unreadable_owner_refuses_rather_than_ignoring_it() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut store = Store::default();
        history_root(&mut store, 1, 11, 0x12300);
        store.unseen_histories.reserve(1);
        *STORE.lock().unwrap() = Some(store);
        LOST.store(false, Ordering::SeqCst);
        collector_test_support::with_allocations_denied(|| history_changed(0x1000, 0));
        assert!(LOST.load(Ordering::SeqCst));
        assert!(
            STORE
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .unseen_histories
                .is_empty()
        );
        LOST.store(false, Ordering::SeqCst);
        let mut bytes = vec![0u8; 0x800];
        let status = bytes.as_ptr() as usize;
        bytes[engine_native::crelation::status::DIPLOMACY_STATUS..][..8]
            .copy_from_slice(&0x45600u64.to_le_bytes());
        collector_test_support::with_allocations_denied(|| history_changed(status, 0));
        assert!(!LOST.load(Ordering::SeqCst));
        assert!(
            STORE
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .unseen_histories
                .is_empty()
        );
        *STORE.lock().unwrap() = None;
    }

    #[test]
    fn participation_publication_shares_membership_across_scalar_changes() {
        let count = 6;
        let mut store = Store::default();
        for index in 0..count {
            let mut value = held(index, index + 1);
            value.ledgers[0].captured_provinces = vec![-7, index as i32];
            store.held.insert(0x10000 + index as usize * 0x200, value);
        }
        let first = publish(&mut store).unwrap();
        assert_eq!(first[0].len(), count as usize * 2);
        assert_eq!(first[1].len(), count as usize * 2);
        store.held.get_mut(&0x10000).unwrap().ledgers[0]
            .scores
            .equipment_damage = 7;
        store.scores_changed = true;
        let changed = publish(&mut store).unwrap();
        assert!(!Arc::ptr_eq(&first[0], &changed[0]));
        assert!(Arc::ptr_eq(&first[1], &changed[1]));
        assert!(Arc::ptr_eq(&first[2], &changed[2]));
        let quiet = publish(&mut store).unwrap();
        for index in 0..3 {
            assert!(Arc::ptr_eq(&changed[index], &quiet[index]));
        }
    }

    #[test]
    fn participation_casualty_refresh_deduplicates_operands_and_replacement_revokes_them() {
        let _lock = TEST_LOCK.lock().unwrap();
        let mut store = Store::default();
        store.held.insert(0x10000, held(1, 2));
        *STORE.lock().unwrap() = Some(store);
        let ledger = 0x10000 + native::DIRECTIONAL_LEDGERS[0];
        assert_eq!(casualty_changed(ledger, 1050, 2077, false), 192);
        assert_eq!(casualty_changed(ledger, 1999, 2999, false), 0);
        assert_eq!(casualty_changed(ledger, 2050, 2077, false), 64);
        assert_eq!(casualty_changed(ledger, 2050, 3077, false), 128);
        assert_eq!(casualty_changed(ledger, 2050, 3077, true), 192);
        assert_eq!(casualty_changed(ledger + 0x100000, 2050, 3077, true), 0);
        replace(0x10000);
        let mut slot = STORE.lock().unwrap();
        let value = &slot.as_ref().unwrap().held[&0x10000];
        assert!(value.seed);
        assert_eq!(value.revision, 1);
        *slot = None;
    }

    #[test]
    fn participation_marks_coalesce_and_worker_reads_only_changed_columns_without_membership() {
        use std::cell::RefCell;
        let _lock = TEST_LOCK.lock().unwrap();
        let mut store = Store::default();
        let mut value = held(1, 2);
        value.ledgers[0].captured_provinces = vec![-1, 5];
        value.ledgers[0].scores.lend_lease_sent = 99;
        store.held.insert(0x10000, value);
        store.pending_scores.reserve(2);
        let allocation = store.pending_scores.as_ptr();
        *STORE.lock().unwrap() = Some(store);
        let ledger = 0x10000 + native::DIRECTIONAL_LEDGERS[0];
        changed(ledger, 1, None);
        changed(ledger, 1, None);
        changed(ledger, 4, None);
        struct Traced {
            bytes: Vec<u8>,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Traced {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, destination.len()));
                engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes,
                }
                .copy(address, destination)
            }
        }
        let mut memory = Traced {
            bytes: vec![0; ledger + native::AIR_DAMAGE + 8],
            reads: RefCell::default(),
        };
        memory.bytes[ledger + native::EQUIPMENT_DAMAGE..ledger + native::EQUIPMENT_DAMAGE + 8]
            .copy_from_slice(&7i64.to_le_bytes());
        memory.bytes[ledger + native::AIR_DAMAGE..ledger + native::AIR_DAMAGE + 8]
            .copy_from_slice(&11i64.to_le_bytes());
        let mut slot = STORE.lock().unwrap();
        let store = slot.as_mut().unwrap();
        assert_eq!(store.pending_scores, [ledger]);
        read_marked(&memory, store).unwrap();
        assert_eq!(
            *memory.reads.borrow(),
            [
                (ledger + native::EQUIPMENT_DAMAGE, 8),
                (ledger + native::AIR_DAMAGE, 8)
            ]
        );
        let value = &store.held[&0x10000].ledgers[0];
        assert_eq!(value.scores.equipment_damage, 7);
        assert_eq!(value.scores.air_damage, 11);
        assert_eq!(value.scores.lend_lease_sent, 99);
        assert_eq!(value.captured_provinces, [-1, 5]);
        assert_eq!(store.pending_scores.as_ptr(), allocation);
        *slot = None;
    }
}
