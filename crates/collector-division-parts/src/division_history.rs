use collector_observers::unit_history_changes as changes;
use collector_projection::certified::{Snapshot, Store as Rows};
use engine_allocation as allocation;
use engine_native_memory::Memory;
use engine_native::carmy::DivisionSeed;
use engine_native_land::unit_history;
use engine_naval_history::csunk_ship_info;
use replay_data::rows::{DivisionHistoryRow, RowCodec, SharedDivisionHistoryRow, SunkShipEntry};
use std::{collections::BTreeMap, sync::Arc};
pub type HistorySnapshot = Snapshot<SharedDivisionHistoryRow>;
#[derive(Clone, Copy)]
struct Held {
    division: u64,
    army: usize,
    history: Option<(u64, u64)>,
    history_slot: usize,
    medals_slot: usize,
    store: usize,
    medals: Option<(u64, u64)>,
    counts: [usize; 2],
    latest: Option<(usize, u32)>,
    available: bool,
    edit_epoch: Option<u64>,
}
#[derive(Default)]
pub struct Store {
    held: Vec<Held>,
    baseline: Option<u64>,
    next: Vec<Held>,
    sorted: Vec<(u64, usize)>,
    shared: Option<allocation::SharedBox<crate::division_experience::DivisionIndex>>,
    sorted_ready: bool,
    held_sorted: bool,
    unique: bool,
    shift: isize,
    seen: Vec<bool>,
    rows: Rows<SharedDivisionHistoryRow>,
    bindings: BTreeMap<u64, BTreeMap<(usize, u128), ()>>,
    binding_count: usize,
    edits: Vec<usize>,
    edit_epoch: Option<u64>,
    ship_text: Vec<u8>,
    pending: Vec<Edit>,
    defer_edits: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Certificates {
    history: ((u64, u64), usize),
    store: usize,
    medals: Option<((u64, u64), usize)>,
    latest_provinces: Option<(usize, Result<u32, &'static str>)>,
}
impl Certificates {
    pub(super) fn set_latest_provinces(
        &mut self,
        address: usize,
        count: Result<u32, &'static str>,
    ) {
        self.latest_provinces = Some((address, count));
    }
}
enum Edit {
    Remove(std::ops::Range<u128>),
    Insert(SharedDivisionHistoryRow),
    ClearBindings(u64, u8),
    Bind(u64, usize, u128),
}
fn push_edit(pending: &mut Vec<Edit>, edit: Edit) -> Result<(), &'static str> {
    allocation::reserve(pending, 1).map_err(|_| "division history edit allocation failed")?;
    pending.push(edit);
    Ok(())
}
fn insert(
    rows: &mut Rows<SharedDivisionHistoryRow>,
    pending: &mut Vec<Edit>,
    deferred: bool,
    row: SharedDivisionHistoryRow,
) -> Result<(), &'static str> {
    if deferred {
        push_edit(pending, Edit::Insert(row))
    } else {
        rows.insert(row)
    }
}
fn remove(
    rows: &mut Rows<SharedDivisionHistoryRow>,
    pending: &mut Vec<Edit>,
    deferred: bool,
    range: std::ops::Range<u128>,
) -> Result<(), &'static str> {
    if deferred {
        push_edit(pending, Edit::Remove(range))
    } else {
        rows.remove_range(range)
    }
}
fn quiet(held: &Held, seed: &DivisionSeed, taken: &crate::observations::Taken) -> bool {
    let Some(history) = held.history else {
        return false;
    };
    seed.pointer
        .checked_add(unit_history::DIVISION_HISTORY)
        .is_some_and(|address| taken.quiet(address, history.0))
        && (held.store == 0
            || held
                .medals
                .is_some_and(|medals| taken.quiet(held.store, medals.0)))
}
fn latest_unchanged(
    memory: &impl Memory,
    held: &Held,
    prepared: Option<Certificates>,
) -> bool {
    !held.available
        || held.latest.is_none_or(|(address, provinces)| {
            let current = prepared
                .and_then(|p| p.latest_provinces)
                .filter(|&(checked, _)| checked == address)
                .map(|(_, count)| count)
                .unwrap_or_else(|| unit_history::province_count(memory, address));
            current == Ok(provinces)
        })
}
fn key(division: u64, list: u8, ordinal: u32) -> u128 {
    ((division as u128) << 34) | ((list as u128) << 32) | ordinal as u128
}
impl Store {
    pub fn begin(&mut self, count: usize) -> Result<(), &'static str> {
        let mut edits = crate::observations::Taken::default();
        edits.take(|visit| changes::take_edits(visit));
        self.begin_with(count, &edits)
    }
    pub fn begin_with(
        &mut self,
        count: usize,
        edits: &crate::observations::Taken,
    ) -> Result<(), &'static str> {
        self.pending.clear();
        self.defer_edits = false;
        changes::demand(count);
        self.next.clear();
        self.shift = 0;
        self.edits.clear();
        self.seen.clear();
        allocation::reserve(&mut self.seen, self.held.len())
            .map_err(|_| "division history membership allocation failed")?;
        self.seen.resize(self.held.len(), false);
        self.edit_epoch = edits.epoch();
        if allocation::reserve(&mut self.edits, edits.touched_len()).is_err() {
            self.edit_epoch = None;
        } else {
            self.edits.extend_from_slice(edits.touched_receivers());
        }
        allocation::reserve(&mut self.next, count)
            .map_err(|_| "division history cache allocation failed")
    }
    pub fn begin_owned(&mut self, count: usize) -> Result<(), &'static str> {
        let result = self.begin(count);
        self.defer_edits = true;
        result
    }
    pub fn begin_owned_with(
        &mut self,
        count: usize,
        edits: &crate::observations::Taken,
    ) -> Result<(), &'static str> {
        let result = self.begin_with(count, edits);
        self.defer_edits = true;
        result
    }
    fn prefetch_held(
        &self,
        index: usize,
        seed: &DivisionSeed,
        order: &mut crate::division_experience::Order,
    ) -> Option<&Held> {
        if let Some(held) = self
            .held
            .get(index)
            .filter(|held| held.division == seed.division)
        {
            order.set_shift(self.baseline, 0);
            return Some(held);
        }
        if self.held.is_empty() {
            return None;
        }
        if self.unique
            && let Some(guess) = order.guess(self.baseline, index)
            && let Some(held) = self
                .held
                .get(guess)
                .filter(|held| held.division == seed.division)
        {
            return Some(held);
        }
        let at = if self.held_sorted {
            self.held
                .binary_search_by_key(&seed.division, |held| held.division)
                .ok()
        } else if let Some(shared) = &self.shared {
            shared
                .sorted
                .binary_search_by_key(&seed.division, |entry| entry.0)
                .ok()
                .map(|at| shared.sorted[at].1 as usize)
        } else if self.sorted_ready {
            self.sorted
                .binary_search_by_key(&seed.division, |entry| entry.0)
                .ok()
                .map(|at| self.sorted[at].1)
        } else {
            order
                .position(
                    self.baseline,
                    self.held.iter().map(|held| held.division),
                    seed.division,
                )
                .unwrap_or_else(|| {
                    self.held
                        .iter()
                        .position(|held| held.division == seed.division)
                })
        };
        if let Some(at) = at {
            order.set_shift(self.baseline, at as isize - index as isize);
        }
        at.and_then(|at| self.held.get(at))
            .filter(|held| held.division == seed.division)
    }
    pub(super) fn prefetch_quiet(
        &self,
        index: usize,
        seed: &DivisionSeed,
        taken: &crate::observations::Taken,
        order: &mut crate::division_experience::Order,
    ) -> Option<(Certificates, Option<usize>)> {
        let held = self
            .prefetch_held(index, seed, order)
            .filter(|held| held.army == seed.pointer && quiet(held, seed, taken))?;
        let latest = held
            .available
            .then_some(held.latest.map(|(address, _)| address))
            .flatten();
        Some((
            Certificates {
                history: (held.history?, held.history_slot),
                store: held.store,
                medals: held.medals.map(|medals| (medals, held.medals_slot)),
                latest_provinces: None,
            },
            latest,
        ))
    }
    pub(super) fn prefetch_certificates(
        &self,
        index: usize,
        seed: &DivisionSeed,
        order: &mut crate::division_experience::Order,
    ) -> Option<(Certificates, Option<usize>)> {
        let held = self
            .prefetch_held(index, seed, order)
            .filter(|held| held.army == seed.pointer);
        let history = changes::sequence_at(
            seed.pointer.checked_add(unit_history::DIVISION_HISTORY)?,
            held.map_or(usize::MAX, |held| held.history_slot),
        )?;
        let store = held.map_or(0, |held| held.store);
        let medals = if store != 0 {
            Some(changes::sequence_at(
                store,
                held.map_or(usize::MAX, |held| held.medals_slot),
            )?)
        } else {
            None
        };
        let latest = held
            .filter(|held| held.available)
            .and_then(|held| held.latest)
            .map(|(address, _)| address);
        Some((
            Certificates {
                history,
                store,
                medals,
                latest_provinces: None,
            },
            latest,
        ))
    }
    pub fn forget(&mut self) {
        self.pending.clear();
        self.defer_edits = false;
        self.held.clear();
        self.baseline = None;
        self.next.clear();
        self.sorted.clear();
        self.shared = None;
        self.sorted_ready = false;
        self.held_sorted = false;
        self.unique = false;
        self.shift = 0;
        self.rows.clear();
        self.bindings.clear();
        self.binding_count = 0;
        self.edits.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        (self.held.capacity() + self.next.capacity()) * std::mem::size_of::<Held>()
            + self.sorted.capacity() * std::mem::size_of::<(u64, usize)>()
            + self.ship_text.capacity()
            + self.pending.capacity() * std::mem::size_of::<Edit>()
            + self
                .pending
                .iter()
                .map(|edit| match edit {
                    Edit::Insert(row) => row.heap_bytes(),
                    _ => 0,
                })
                .sum::<usize>()
            + self.rows.allocated_bytes()
            + self.edits.capacity() * std::mem::size_of::<usize>()
            + self.binding_count * std::mem::size_of::<((usize, u128), ())>()
            + self.seen.capacity()
    }
    pub fn observe(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
    ) -> Result<(), &'static str> {
        self.observe_prepared(memory, base, seed, None, None)
    }
    pub(super) fn observe_prepared(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seed: &DivisionSeed,
        prepared: Option<Certificates>,
        taken: Option<&crate::observations::Taken>,
    ) -> Result<(), &'static str> {
        self.observe_prepared_with_vtables(
            memory,
            base,
            unit_history::SeedVtables::resolve(base),
            seed,
            prepared,
            taken,
        )
    }
    pub(super) fn observe_prepared_with_vtables(
        &mut self,
        memory: &impl Memory,
        base: usize,
        vtables: unit_history::SeedVtables,
        seed: &DivisionSeed,
        prepared: Option<Certificates>,
        taken: Option<&crate::observations::Taken>,
    ) -> Result<(), &'static str> {
        let index = self.next.len();
        let guess = index.checked_add_signed(self.shift);
        let held_at = if guess.is_some_and(|at| {
            self.held
                .get(at)
                .is_some_and(|held| held.division == seed.division)
        }) {
            guess
        } else if self.held.is_empty() {
            None
        } else if self.held_sorted {
            self.held
                .binary_search_by_key(&seed.division, |held| held.division)
                .ok()
        } else if let Some(shared) = &self.shared {
            shared
                .sorted
                .binary_search_by_key(&seed.division, |entry| entry.0)
                .ok()
                .map(|at| shared.sorted[at].1 as usize)
        } else {
            if !self.sorted_ready {
                self.sorted.clear();
                allocation::reserve(&mut self.sorted, self.held.len())
                    .map_err(|_| "division history index allocation failed")?;
                self.sorted.extend(
                    self.held
                        .iter()
                        .enumerate()
                        .map(|(at, held)| (held.division, at)),
                );
                self.sorted.sort_unstable();
                self.unique = self.sorted.windows(2).all(|pair| pair[0].0 != pair[1].0);
                self.sorted_ready = true;
            }
            self.sorted
                .binary_search_by_key(&seed.division, |entry| entry.0)
                .ok()
                .map(|at| self.sorted[at].1)
        };
        if let Some(at) = held_at {
            self.shift = at as isize - index as isize;
            if self.seen[at] {
                return Err("division history duplicate census identity");
            }
            self.seen[at] = true;
        }
        let mut held = held_at
            .map(|at| self.held[at])
            .filter(|held| held.army == seed.pointer);
        let identity = unit_history::validate_seed_with(vtables, seed);
        let history_address = seed
            .pointer
            .checked_add(unit_history::DIVISION_HISTORY)
            .ok_or("division history address overflow")?;
        let quiet = held
            .as_ref()
            .zip(taken)
            .is_some_and(|(held, taken)| quiet(held, seed, taken));
        let history = match (quiet, prepared) {
            (true, _) => held.as_ref().and_then(|held| held.history),
            (false, Some(p)) => Some(p.history.0),
            (false, None) => changes::sequence(history_address),
        };
        let medals = held
            .as_ref()
            .filter(|held| held.store != 0)
            .and_then(|held| match (quiet, prepared.filter(|p| p.store == held.store)) {
                (true, _) => held.medals,
                (false, Some(p)) => p.medals.map(|(certificate, _)| certificate),
                (false, None) => changes::sequence(held.store),
            });
        if let (Some(held), Some(p)) = (held.as_mut().filter(|_| !quiet), prepared) {
            held.history_slot = p.history.1;
            if p.store == held.store {
                held.medals_slot = p.medals.map_or(usize::MAX, |(_, slot)| slot);
            }
        }
        if let Some(held) = held.filter(|held| {
            held.history == history
                && held.medals == medals
                && (!held.available
                    || (held.edit_epoch.is_some() && held.edit_epoch == self.edit_epoch))
                && !self.edits.iter().any(|address| {
                    self.bindings.get(&seed.division).is_some_and(|bindings| {
                        bindings
                            .range((*address, 0)..=(*address, u128::MAX))
                            .next()
                            .is_some()
                    })
                })
                && (identity.is_ok() || !held.available)
                && latest_unchanged(memory, held, prepared)
        }) {
            self.next.push(held);
            changes::carried();
            return Ok(());
        }
        let mut next = Held {
            division: seed.division,
            army: seed.pointer,
            history,
            history_slot: prepared.map_or(usize::MAX, |p| p.history.1),
            medals_slot: usize::MAX,
            store: 0,
            medals: None,
            counts: [0; 2],
            latest: None,
            available: false,
            edit_epoch: self.edit_epoch,
        };
        let result = (|| {
            identity?;
            history.ok_or("division history writer certificate unavailable")?;
            let roots = unit_history::roots(memory, base, seed)?;
            next.store = roots.store;
            next.medals = match prepared.filter(|p| p.store == roots.store) {
                Some(p) => {
                    next.medals_slot = p.medals.map_or(usize::MAX, |(_, slot)| slot);
                    p.medals.map(|(certificate, _)| certificate)
                }
                None => changes::sequence(roots.store),
            };
            next.medals
                .ok_or("division medal writer certificate unavailable")?;
            changes::read();
            next.counts = [roots.count(false), roots.count(true)];
            let lifetime = self.edits.binary_search(&history_address).is_ok();
            let store_edit = self.edits.binary_search(&roots.store).is_ok();
            let prefix = held.as_ref().filter(|held| {
                held.available
                    && held.store == roots.store
                    && held.edit_epoch.is_some()
                    && held.edit_epoch == self.edit_epoch
                    && !lifetime
            });
            let kept = [0, 1].map(|list| {
                prefix
                    .filter(|held| {
                        !(list == 1 && store_edit) && held.counts[list] <= next.counts[list]
                    })
                    .map(|held| held.counts[list].saturating_sub(usize::from(list == 0)) as u32)
            });
            let starts = kept.map(|first| first.unwrap_or(0));
            for (index, first) in kept.iter().enumerate() {
                if first.is_none() {
                    let list = index as u8 + 1;
                    remove(
                        &mut self.rows,
                        &mut self.pending,
                        self.defer_edits,
                        key(seed.division, list, 0)..key(seed.division, list + 1, 0),
                    )?;
                    if self.defer_edits {
                        push_edit(&mut self.pending, Edit::ClearBindings(seed.division, list))?;
                    } else if let Some(bindings) = self.bindings.get_mut(&seed.division) {
                        let before = bindings.len();
                        bindings.retain(|(_, k), _| ((k >> 32) & 3) != u128::from(list));
                        self.binding_count -= before - bindings.len();
                    }
                }
            }
            insert(
                &mut self.rows,
                &mut self.pending,
                self.defer_edits,
                Arc::new(DivisionHistoryRow {
                    division: seed.division,
                    available: 1,
                    amount: roots.amount,
                    ..Default::default()
                }),
            )?;
            if prefix.is_some() {
                for &address in &self.edits {
                    let Some(bindings) = self.bindings.get(&seed.division) else {
                        continue;
                    };
                    for (&(_, entry_key), _) in bindings.range((address, 0)..=(address, u128::MAX))
                    {
                        let list = ((entry_key >> 32) & 3) as usize;
                        if starts[list - 1] == 0 {
                            continue;
                        }
                        let entry = unit_history::entry(memory, base, address)?;
                        insert(
                            &mut self.rows,
                            &mut self.pending,
                            self.defer_edits,
                            project(
                                memory,
                                base,
                                seed.division,
                                ((entry_key >> 32) & 3) == 2,
                                entry_key as u32,
                                entry,
                                &mut self.ship_text,
                            )?,
                        )?;
                    }
                }
            }
            for awards in [false, true] {
                let first = starts[usize::from(awards)];
                roots.visit_range(memory, base, awards, first, |ordinal, address, entry| {
                    if changes::edit_sequence(address).is_none() {
                        next.edit_epoch = None;
                    }
                    if !awards {
                        next.latest = Some((address, entry.provinces.len() as u32));
                    }
                    let row = project(
                        memory,
                        base,
                        seed.division,
                        awards,
                        ordinal,
                        entry,
                        &mut self.ship_text,
                    )?;
                    if self.defer_edits {
                        push_edit(
                            &mut self.pending,
                            Edit::Bind(seed.division, address, row.key()),
                        )?;
                    } else if self
                        .bindings
                        .entry(seed.division)
                        .or_default()
                        .insert((address, row.key()), ())
                        .is_none()
                    {
                        self.binding_count += 1;
                    }
                    insert(&mut self.rows, &mut self.pending, self.defer_edits, row)
                })?;
            }
            if changes::edit_sequence(history_address).is_none()
                || changes::edit_sequence(roots.store).is_none()
            {
                next.edit_epoch = None;
            }
            next.available = true;
            Ok::<_, &'static str>(())
        })();
        if let Err(reason) = result {
            changes::refused();
            remove(
                &mut self.rows,
                &mut self.pending,
                self.defer_edits,
                key(seed.division, 0, 0)..key(seed.division, 3, 0),
            )?;
            insert(
                &mut self.rows,
                &mut self.pending,
                self.defer_edits,
                Arc::new(DivisionHistoryRow {
                    division: seed.division,
                    reason: allocation::copy_vec(reason.as_bytes())
                        .map_err(|_| "division history refusal allocation failed")?,
                    ..Default::default()
                }),
            )?;
        }
        self.next.push(next);
        Ok(())
    }
    pub fn finish(&mut self) -> Result<Arc<HistorySnapshot>, &'static str> {
        self.finish_with_index(None)
    }
    pub fn finish_with_index(
        &mut self,
        shared: Option<&allocation::SharedBox<crate::division_experience::DivisionIndex>>,
    ) -> Result<Arc<HistorySnapshot>, &'static str> {
        self.defer_edits = false;
        let mut pending = std::mem::take(&mut self.pending);
        let result = (|| {
            for edit in pending.drain(..) {
                match edit {
                    Edit::Remove(range) => self.rows.remove_range(range)?,
                    Edit::Insert(row) => self.rows.insert(row)?,
                    Edit::ClearBindings(division, list) => {
                        if let Some(bindings) = self.bindings.get_mut(&division) {
                            let before = bindings.len();
                            bindings.retain(|(_, k), _| ((k >> 32) & 3) != u128::from(list));
                            self.binding_count -= before - bindings.len();
                        }
                    }
                    Edit::Bind(division, address, key) => {
                        if self
                            .bindings
                            .entry(division)
                            .or_default()
                            .insert((address, key), ())
                            .is_none()
                        {
                            self.binding_count += 1;
                        }
                    }
                }
            }
            Ok(())
        })();
        self.pending = pending;
        result?;
        for (index, held) in self.held.iter().enumerate() {
            if !self.seen[index] {
                self.rows
                    .remove_range(key(held.division, 0, 0)..key(held.division, 3, 0))?;
                if let Some(bindings) = self.bindings.remove(&held.division) {
                    self.binding_count -= bindings.len();
                }
            }
        }
        std::mem::swap(&mut self.held, &mut self.next);
        self.next.clear();
        self.sorted.clear();
        self.shared = None;
        if let Some(shared) = shared
            && self.held.len() == shared.divisions.len()
            && self
                .held
                .iter()
                .zip(&shared.divisions)
                .all(|(held, &division)| held.division == division)
        {
            self.baseline = Some(shared.baseline);
            self.held_sorted = shared.held_sorted;
            self.unique = shared.unique;
            self.sorted_ready = true;
            self.shared = Some(allocation::SharedBox::clone(shared));
        } else {
            self.baseline = crate::division_experience::next_baseline();
            self.held_sorted = self
                .held
                .windows(2)
                .all(|pair| pair[0].division < pair[1].division);
            if self.held_sorted {
                self.sorted_ready = true;
                self.unique = true;
            } else if allocation::advisory(|| allocation::reserve(&mut self.sorted, self.held.len()))
                .is_ok()
            {
                self.sorted.extend(
                    self.held
                        .iter()
                        .enumerate()
                        .map(|(at, held)| (held.division, at)),
                );
                self.sorted.sort_unstable();
                self.unique = self.sorted.windows(2).all(|pair| pair[0].0 != pair[1].0);
                self.sorted_ready = true;
            } else {
                self.sorted_ready = false;
                self.unique = false;
            }
        }
        changes::entry_demand(self.binding_count);
        self.rows.finish()
    }
}
fn project(
    memory: &impl Memory,
    base: usize,
    division: u64,
    awards: bool,
    ordinal: u32,
    entry: unit_history::Entry,
    ship_text: &mut Vec<u8>,
) -> Result<SharedDivisionHistoryRow, &'static str> {
    let sunk_ship = entry
        .sunk_ship
        .map(|pointer| {
            let (record, len) =
                csunk_ship_info::read_one(memory, base, pointer, ship_text)?;
            let text = |range: csunk_ship_info::TextRange| {
                allocation::copy_vec(&ship_text[..len][range.start..range.start + range.len])
                    .map_err(|_| "division history sunk-ship text allocation failed")
            };
            Ok::<_, &'static str>(SunkShipEntry {
                ship_name: text(record.ship_name)?,
                attribution_name: text(record.attribution_name)?,
                class_name: text(record.class_name)?,
                attribution_class_name: text(record.attribution_class_name)?,
                victim_country: record.victim_country,
                attacker_country: record.attacker_country,
                sink_hour: record.sink_hour,
                class_token: record.class_token,
                attribution_class_token: record.attribution_class_token,
                attribution_class_present: record.attribution_class_present,
                model_level: record.model_level,
                victim_equipment_variant: record.victim_equipment_variant,
                damage_source_reference: record.damage_source_reference,
                province: record.province,
                province_present: record.province_present,
                battle: record.battle,
                cause_record: record.cause_record,
                assisted_kill: record.assisted_kill,
            })
        })
        .transpose()?;
    Ok(Arc::new(DivisionHistoryRow {
        division,
        list: if awards {
            DivisionHistoryRow::AWARDS
        } else {
            DivisionHistoryRow::CAREER
        },
        ordinal,
        available: 1,
        name: entry.name,
        custom_name: entry.custom_name,
        kind: entry.kind,
        target_country: entry.target_country,
        hour: entry.hour,
        medal_token: entry.medal.map(|medal| medal.0),
        medal_definition: entry.medal.map(|medal| medal.1),
        medal_enabled: entry.medal_enabled,
        medal_count: entry.medal_count,
        inherit: entry.inherit,
        multiplier: entry.multiplier,
        orders: entry.orders,
        provinces: entry.provinces,
        sunk_ship,
        ..Default::default()
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::army_stat_changes::{HISTORY_TEST, serial};
    use engine_native::carmy;
    use engine_native_memory::cgame_date;
    use engine_native_land::unit_history as native;
    use replay_data::rows::{TableRows, tables};
    use std::cell::Cell;
    struct World {
        bytes: Vec<u8>,
        reads: Cell<usize>,
        blocked: Cell<Option<(usize, usize)>>,
    }
    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.blocked.get().is_some_and(|(start, end)| {
                address < end && address.saturating_add(output.len()) > start
            }) {
                return false;
            }
            let Some(bytes) = address
                .checked_add(output.len())
                .and_then(|end| self.bytes.get(address..end))
            else {
                return false;
            };
            output.copy_from_slice(bytes);
            true
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: impl AsRef<[u8]>) {
        let value = value.as_ref();
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn seed(army: usize, division: u64) -> DivisionSeed {
        DivisionSeed {
            pointer: army,
            division,
            handle: Some(division),
            class: Some(0),
            primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
            secondary_vtable: Some(carmy::SECONDARY_VTABLE_RVA.0 as u64),
            ..Default::default()
        }
    }
    fn root(bytes: &mut [u8], army: usize, store: usize, entries: usize) {
        let history = army + native::DIVISION_HISTORY;
        put(
            bytes,
            history,
            (native::HISTORY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            bytes,
            history + native::UNIT_BACKLINK,
            (army as u64).to_le_bytes(),
        );
        put(
            bytes,
            history + native::MEDAL_STORE,
            (store as u64).to_le_bytes(),
        );
        put(
            bytes,
            store,
            (native::STORE_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            bytes,
            history + native::HISTORY_ENTRIES,
            (entries as u64).to_le_bytes(),
        );
        put(
            bytes,
            history + native::HISTORY_ENTRIES + 8,
            2_i32.to_le_bytes(),
        );
        put(
            bytes,
            history + native::HISTORY_ENTRIES + 12,
            2_i32.to_le_bytes(),
        );
        put(bytes, entries, 0x7000_u64.to_le_bytes());
        put(bytes, entries + 8, 0x7000_u64.to_le_bytes());
    }
    #[test]
    fn experience_history_carries_on_one_read_each_and_refreshes_only_the_writer_dirty_division() {
        let _serial = serial(&HISTORY_TEST);
        for owned in [false, true] {
            let begin = |store: &mut Store| {
                if owned {
                    store.begin_owned(2)
                } else {
                    store.begin(2)
                }
                .unwrap();
            };
            changes::patched(true);
            changes::forwarder(true);
            changes::demand(2);
            changes::presize();
            changes::boundary(false);
            let mut world = World {
                bytes: vec![0; 0x9000],
                reads: Cell::new(0),
                blocked: Cell::new(None),
            };
            root(&mut world.bytes, 0x1000, 0x4000, 0x6000);
            root(&mut world.bytes, 0x2000, 0x5000, 0x6100);
            let entry = 0x7000;
            put(
                &mut world.bytes,
                entry,
                (native::ENTRY_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            for text in [8, 0x28] {
                put(&mut world.bytes, entry + text + 24, 15_u64.to_le_bytes());
            }
            put(
                &mut world.bytes,
                entry + 0x50,
                (cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(
                &mut world.bytes,
                entry + 0x60,
                (cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut world.bytes, entry + 0x138, 100_000_i64.to_le_bytes());
            let seeds = [seed(0x2000, 2), seed(0x1000, 1)];
            let mut store = Store::default();
            let mut order = crate::division_experience::Order::default();
            begin(&mut store);
            for (index, seed) in seeds.iter().enumerate() {
                let prepared = owned
                    .then(|| store.prefetch_certificates(index, seed, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store.observe_prepared(&world, 0, seed, prepared, None).unwrap();
            }
            let first = store.finish().unwrap();
            let first_rows = first.complete::<tables::DivisionHistory>().unwrap();
            let TableRows::DivisionHistory(rows) = &first_rows else {
                panic!()
            };
            assert_eq!(rows.len(), 6);
            assert_eq!(rows[2].ordinal, 1);
            assert_eq!(rows[0].available, 1);
            world.reads.set(0);
            begin(&mut store);
            for (index, seed) in seeds.iter().enumerate() {
                let prepared = owned
                    .then(|| store.prefetch_certificates(index, seed, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store.observe_prepared(&world, 0, seed, prepared, None).unwrap();
            }
            let quiet = store.finish().unwrap();
            assert!(Arc::ptr_eq(&first, &quiet));
            assert_eq!(world.reads.get(), 2);
            assert_eq!(
                order.rebuilds(),
                0,
                "stable unsorted census needs no locator rebuild"
            );
            assert_eq!(
                store
                    .held
                    .iter()
                    .map(|held| held.division)
                    .collect::<Vec<_>>(),
                [2, 1]
            );
            put(
                &mut world.bytes,
                0x4000 + native::AWARD_AMOUNT,
                8_i32.to_le_bytes(),
            );
            drop(changes::notify(0x4000));
            begin(&mut store);
            for (index, seed) in seeds.iter().enumerate() {
                let prepared = owned
                    .then(|| store.prefetch_certificates(index, seed, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store.observe_prepared(&world, 0, seed, prepared, None).unwrap();
            }
            let changed = store.finish().unwrap();
            let changed_rows = changed.complete::<tables::DivisionHistory>().unwrap();
            let TableRows::DivisionHistory(next) = &changed_rows else {
                panic!()
            };
            assert_eq!(next[0].amount, 8);
            assert!(Arc::ptr_eq(&rows[1], &next[1]));
            assert!(Arc::ptr_eq(&rows[3], &next[3]));
            let appended = world.bytes[entry..entry + 0x148].to_vec();
            put(&mut world.bytes, 0x7200, &appended);
            put(&mut world.bytes, 0x7400, &appended);
            put(&mut world.bytes, 0x6000, 0x7400u64.to_le_bytes());
            put(&mut world.bytes, 0x6010, 0x7200u64.to_le_bytes());
            put(
                &mut world.bytes,
                0x1000 + native::DIVISION_HISTORY + native::HISTORY_ENTRIES + 8,
                3u32.to_le_bytes(),
            );
            put(
                &mut world.bytes,
                0x1000 + native::DIVISION_HISTORY + native::HISTORY_ENTRIES + 12,
                3u32.to_le_bytes(),
            );
            world.blocked.set(Some((0x7400, 0x7400 + 0x148)));
            drop(changes::notify(0x1000 + native::DIVISION_HISTORY));
            begin(&mut store);
            for (index, seed) in seeds.iter().enumerate() {
                let prepared = owned
                    .then(|| store.prefetch_certificates(index, seed, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store.observe_prepared(&world, 0, seed, prepared, None).unwrap();
            }
            let appended = store.finish().unwrap();
            assert_eq!(appended.changed.len(), 1);
            assert_eq!(appended.ranges.len(), 1);
            let TableRows::DivisionHistory(appended_rows) =
                appended.complete::<tables::DivisionHistory>().unwrap()
            else {
                panic!()
            };
            assert_eq!(appended_rows[0].available, 1);
            assert_eq!(appended_rows[3].ordinal, 2);
            assert!(Arc::ptr_eq(&rows[1], &appended_rows[1]));
            let history = 0x1000 + native::DIVISION_HISTORY;
            world.blocked.set(Some((history, history + 8)));
            drop(changes::notify(history));
            begin(&mut store);
            for (index, seed) in seeds.iter().enumerate() {
                let prepared = owned
                    .then(|| store.prefetch_certificates(index, seed, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store.observe_prepared(&world, 0, seed, prepared, None).unwrap();
            }
            let refused = store.finish().unwrap();
            let refused_rows = refused.complete::<tables::DivisionHistory>().unwrap();
            let TableRows::DivisionHistory(rows) = &refused_rows else {
                panic!()
            };
            assert_eq!(rows[0].available, 0);
            assert!(!rows[0].reason.is_empty());
            assert_eq!(rows[1].division, 2);
            world.reads.set(0);
            begin(&mut store);
            for (index, seed) in seeds.iter().enumerate() {
                let prepared = owned
                    .then(|| store.prefetch_certificates(index, seed, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store.observe_prepared(&world, 0, seed, prepared, None).unwrap();
            }
            assert!(Arc::ptr_eq(&refused, &store.finish().unwrap()));
            assert_eq!(world.reads.get(), 1);
            changes::patched(false);
            changes::forwarder(false);
        }
    }
    #[test]
    fn a_latest_career_entry_grown_in_place_is_recorded_alone_and_beside_an_append() {
        let _serial = serial(&HISTORY_TEST);
        for owned in [false, true] {
            changes::patched(true);
            changes::forwarder(true);
            changes::demand(1);
            changes::presize();
            changes::boundary(false);
            let mut world = World {
                bytes: vec![0; 0x9000],
                reads: Cell::new(0),
                blocked: Cell::new(None),
            };
            let army = 0x1000;
            let history = army + native::DIVISION_HISTORY;
            root(&mut world.bytes, army, 0x4000, 0x6000);
            put(
                &mut world.bytes,
                history + native::HISTORY_ENTRIES + 8,
                4_u32.to_le_bytes(),
            );
            let (earlier, latest, appended) = (0x7000, 0x7200, 0x7400);
            for at in [earlier, latest, appended] {
                put(
                    &mut world.bytes,
                    at,
                    (native::ENTRY_VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                for text in [8, 0x28] {
                    put(&mut world.bytes, at + text + 24, 15_u64.to_le_bytes());
                }
                put(
                    &mut world.bytes,
                    at + 0x50,
                    (cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut world.bytes,
                    at + 0x60,
                    (cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(&mut world.bytes, at + 0x138, 100_000_i64.to_le_bytes());
            }
            put(&mut world.bytes, 0x6008, (latest as u64).to_le_bytes());
            let provinces = 0x7800;
            for (index, (province, id)) in [(0x8000, 9263_u32), (0x8100, 362), (0x8200, 7)]
                .into_iter()
                .enumerate()
            {
                put(
                    &mut world.bytes,
                    province + engine_native::cprovince::ID,
                    id.to_le_bytes(),
                );
                put(
                    &mut world.bytes,
                    provinces + index * 8,
                    (province as u64).to_le_bytes(),
                );
            }
            put(
                &mut world.bytes,
                latest + 0x120,
                (provinces as u64).to_le_bytes(),
            );
            put(&mut world.bytes, latest + 0x128, 4_u32.to_le_bytes());
            let grow = |world: &mut World, count: u32| {
                put(&mut world.bytes, latest + 0x12c, count.to_le_bytes());
            };
            grow(&mut world, 1);
            let division = seed(army, 1);
            let mut store = Store::default();
            let mut order = crate::division_experience::Order::default();
            let mut hour = |store: &mut Store, world: &World| {
                if owned {
                    store.begin_owned(1).unwrap();
                } else {
                    store.begin(1).unwrap();
                }
                let prepared = owned
                    .then(|| store.prefetch_certificates(0, &division, &mut order))
                    .flatten()
                    .map(|(certs, _)| certs);
                store
                    .observe_prepared(world, 0, &division, prepared, None)
                    .unwrap();
                let snapshot = store.finish().unwrap();
                let TableRows::DivisionHistory(rows) =
                    snapshot.complete::<tables::DivisionHistory>().unwrap()
                else {
                    panic!()
                };
                let careers: Vec<_> = rows
                    .iter()
                    .filter(|row| row.list == DivisionHistoryRow::CAREER)
                    .cloned()
                    .collect();
                (snapshot, careers)
            };
            let (_, first) = hour(&mut store, &world);
            assert_eq!(
                first
                    .iter()
                    .map(|row| row.provinces.clone())
                    .collect::<Vec<_>>(),
                [vec![], vec![9263]]
            );
            grow(&mut world, 2);
            world.blocked.set(Some((earlier, earlier + 0x148)));
            let (grown, careers) = hour(&mut store, &world);
            assert_eq!(careers[1].provinces, [9263, 362]);
            assert!(Arc::ptr_eq(&first[0], &careers[0]));
            assert_eq!(grown.changed.len(), 1);
            world.reads.set(0);
            let (quiet, _) = hour(&mut store, &world);
            assert!(Arc::ptr_eq(&grown, &quiet));
            assert_eq!(world.reads.get(), 1);
            grow(&mut world, 3);
            put(&mut world.bytes, 0x6010, (appended as u64).to_le_bytes());
            put(
                &mut world.bytes,
                history + native::HISTORY_ENTRIES + 12,
                3_u32.to_le_bytes(),
            );
            drop(changes::notify(history));
            let (both, careers) = hour(&mut store, &world);
            assert_eq!(
                careers
                    .iter()
                    .map(|row| (row.ordinal, row.provinces.clone()))
                    .collect::<Vec<_>>(),
                [(0, vec![]), (1, vec![9263, 362, 7]), (2, vec![])]
            );
            assert!(Arc::ptr_eq(&first[0], &careers[0]));
            assert_eq!(both.changed.len(), 2);
            changes::patched(false);
            changes::forwarder(false);
        }
    }
    #[test]
    fn a_shifted_census_finds_each_held_history_through_one_index() {
        const COUNT: u64 = 30_000;
        let held = |division: u64| Held {
            division,
            army: division as usize * 0x100,
            history: None,
            history_slot: usize::MAX,
            medals_slot: usize::MAX,
            store: 0,
            medals: None,
            counts: [0; 2],
            latest: None,
            available: false,
            edit_epoch: None,
        };
        let mut store = Store::default();
        store.held = (1..=COUNT).map(held).collect();
        store.baseline = crate::division_experience::next_baseline();
        let mut order = crate::division_experience::Order::default();
        for index in 1..=COUNT as usize {
            let division = index as u64;
            let found = store.prefetch_held(index, &seed(index * 0x100, division), &mut order);
            assert_eq!(found.map(|held| held.division), Some(division));
        }
        assert!(
            store
                .prefetch_held(0, &seed(0x100, COUNT + 1), &mut order)
                .is_none()
        );
        assert_eq!(order.rebuilds(), 1);
        let mut unchanged = crate::division_experience::Order::default();
        for (index, held) in store.held.iter().enumerate() {
            let census = seed(held.army, held.division);
            assert!(
                store
                    .prefetch_held(index, &census, &mut unchanged)
                    .is_some()
            );
        }
        assert_eq!(unchanged.rebuilds(), 0);
        store.baseline = crate::division_experience::next_baseline();
        assert!(
            store
                .prefetch_held(0, &seed(0x200, 2), &mut order)
                .is_some()
        );
        assert_eq!(order.rebuilds(), 2);
    }
    #[test]
    fn an_uncertified_entry_edit_rereads_every_history_holding_the_entry() {
        let _serial = serial(&HISTORY_TEST);
        changes::patched(true);
        changes::forwarder(true);
        changes::demand(2);
        changes::presize();
        changes::boundary(false);
        let mut world = World {
            bytes: vec![0; 0x9000],
            reads: Cell::new(0),
            blocked: Cell::new(None),
        };
        root(&mut world.bytes, 0x1000, 0x4000, 0x6000);
        root(&mut world.bytes, 0x2000, 0x5000, 0x6100);
        let entry = 0x7000;
        put(
            &mut world.bytes,
            entry,
            (native::ENTRY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        for text in [8, 0x28] {
            put(&mut world.bytes, entry + text + 24, 15_u64.to_le_bytes());
        }
        put(
            &mut world.bytes,
            entry + 0x50,
            (cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut world.bytes,
            entry + 0x60,
            (cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut world.bytes, entry + 0x138, 100_000_i64.to_le_bytes());
        let seeds = [seed(0x2000, 2), seed(0x1000, 1)];
        let observe = |store: &mut Store, world: &World| {
            for seed in &seeds {
                store.observe(world, 0, seed).unwrap();
            }
            store.finish().unwrap()
        };
        let mut store = Store::default();
        store.begin(2).unwrap();
        observe(&mut store, &world);
        put(&mut world.bytes, entry + 0x138, 200_000_i64.to_le_bytes());
        drop(changes::notify(0x4000));
        drop(changes::notify_edit(entry));
        let running = changes::notify_edit(0);
        store.begin(2).unwrap();
        drop(running);
        let TableRows::DivisionHistory(rows) = observe(&mut store, &world)
            .complete::<tables::DivisionHistory>()
            .unwrap()
        else {
            panic!()
        };
        let careers: Vec<_> = rows
            .iter()
            .filter(|row| row.list == DivisionHistoryRow::CAREER)
            .map(|row| (row.division, row.multiplier))
            .collect();
        assert_eq!(
            careers,
            [(1, 200_000), (1, 200_000), (2, 200_000), (2, 200_000)]
        );
        changes::take_edits(|_| {});
        changes::patched(false);
        changes::forwarder(false);
    }
}
