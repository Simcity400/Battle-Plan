use super::*;
use collector_observers::unit_history_changes as history_changes;
use std::{ops::Range, sync::Arc};

pub struct PreparedDivision {
    pub(super) experience_certificate: Option<(u64, u64)>,
    pub(super) progression_certificate: Option<(u64, u64)>,
    pub(super) officer_certificate: Option<(u64, u64)>,
    pub(super) career_certificate: Option<(u64, u64)>,
    pub(super) experience: Option<Result<i64, &'static str>>,
    pub(super) progression: Option<Result<u64, &'static str>>,
    pub(super) officer: Option<Result<i64, &'static str>>,
    pub(super) career: Option<Result<i32, &'static str>>,
    pub selection: Option<crate::panel_sources::Selection>,
    pub(super) acquired: [bool; 4],
    pub(super) history: Option<crate::division_history::Certificates>,
}

impl PreparedDivision {
    pub(super) fn owned_scalars(&self) -> bool {
        self.acquired[1..].iter().all(|&acquired| acquired)
            && self.progression.is_some()
            && self.officer.is_some()
            && self.career.is_some()
    }
}

pub struct PreparationPart {
    pub range: Range<usize>,
    rows: Vec<PreparedDivision>,
    orders: [ScalarOrder; 5],
    supply: Vec<Option<Arc<crate::supply_inputs::Sample>>>,
    supply_order: ScalarOrder,
    sources: crate::panel_sources::Preparation,
    source_ready: bool,
    maxima: Vec<Option<Result<DivisionMaxima, &'static str>>>,
    maxima_pointers: Vec<usize>,
    maxima_reads: Vec<Result<DivisionMaxima, &'static str>>,
}
impl Default for PreparationPart {
    fn default() -> Self {
        Self {
            range: 0..0,
            rows: Vec::new(),
            orders: std::array::from_fn(|_| ScalarOrder::default()),
            supply: Vec::new(),
            supply_order: Default::default(),
            sources: Default::default(),
            source_ready: false,
            maxima: Vec::new(),
            maxima_pointers: Vec::new(),
            maxima_reads: Vec::new(),
        }
    }
}
impl PreparationPart {
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<PreparedDivision>()
            + self.supply.capacity()
                * std::mem::size_of::<Option<Arc<crate::supply_inputs::Sample>>>()
            + self.supply_order.allocated_bytes()
            + self.sources.allocated_bytes()
            + self
                .orders
                .iter()
                .map(ScalarOrder::allocated_bytes)
                .sum::<usize>()
            + self.maxima.capacity()
                * std::mem::size_of::<Option<Result<DivisionMaxima, &'static str>>>()
            + self.maxima_pointers.capacity() * std::mem::size_of::<usize>()
            + self.maxima_reads.capacity()
                * std::mem::size_of::<Result<DivisionMaxima, &'static str>>()
    }
    pub fn get(&self, index: usize) -> Option<&PreparedDivision> {
        self.rows.get(index.checked_sub(self.range.start)?)
    }
    pub fn daily_supply(&self, index: usize) -> Option<&Option<Arc<crate::supply_inputs::Sample>>> {
        self.supply.get(index.checked_sub(self.range.start)?)
    }
    pub fn can_settle_sources(&self) -> bool {
        self.source_ready && self.rows.len() == self.range.len() && self.sources.can_settle_owned()
    }
    pub fn source_reads(&self) -> Option<&crate::panel_sources::Preparation> {
        self.source_ready.then_some(&self.sources)
    }
    pub fn maxima(&self, index: usize) -> Option<Result<DivisionMaxima, &'static str>> {
        self.maxima
            .get(index.checked_sub(self.range.start)?)
            .copied()
            .flatten()
    }
    fn stage_maxima(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[DivisionSeed],
        store: &ArmyStatStore,
    ) {
        self.maxima.clear();
        self.maxima_pointers.clear();
        self.maxima_reads.clear();
        if engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.maxima, seeds.len())?;
            engine_allocation::reserve(&mut self.maxima_pointers, seeds.len())?;
            engine_allocation::reserve(&mut self.maxima_reads, seeds.len())
        })
        .is_err()
        {
            return;
        }
        let identity =
            engine_sites::at(base, engine_native::carmy::VTABLE_RVA).map(|at| at as u64);
        let mut shift = 0;
        for (offset, seed) in seeds.iter().enumerate() {
            let pointer = seed.stat_block.filter(|&pointer| {
                pointer != 0
                    && seed.primary_vtable == identity
                    && seed.handle == Some(seed.division)
                    && seed.division != 0
                    && !store.may_carry_maxima(&mut shift, self.range.start + offset, seed)
            });
            if let Some(pointer) = pointer {
                self.maxima_pointers.push(pointer);
            }
            self.maxima
                .push(pointer.map(|_| Err("division maxima unstaged")));
        }
        self.maxima_reads
            .resize(self.maxima_pointers.len(), Err("division maxima unstaged"));
        engine_native::csub_unit_definition::stage_division_maxima(
            memory,
            base,
            &self.maxima_pointers,
            &mut self.maxima_reads,
        );
        for (slot, read) in self.maxima.iter_mut().flatten().zip(&self.maxima_reads) {
            *slot = *read;
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        store: Option<&ArmyStatStore>,
        changes: &dyn Changes,
    ) {
        self.rows.clear();
        self.supply.clear();
        self.supply_order.clear();
        self.maxima.clear();
        self.source_ready = false;
        for order in &mut self.orders {
            order.clear();
        }
        let Some(store) = store else {
            return;
        };
        let Some(seeds) = seeds.get(self.range.clone()) else {
            return;
        };
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.rows, seeds.len()))
            .is_err()
        {
            return;
        }
        let supply_ready = store.holds_daily_progress()
            && engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.supply, seeds.len())
            })
            .is_ok();
        let source_reads = self.sources.begin(seeds.len());
        let countries = source_reads
            .then(|| engine_native::ccountry::Reader::new(memory, base, state).ok())
            .flatten();
        self.source_ready = source_reads && countries.is_some();
        for (index, seed) in seeds.iter().enumerate() {
            let index = self.range.start + index;
            if supply_ready {
                self.supply
                    .push(store.daily_supply(base, index, seed, &mut self.supply_order));
            }
            let selection = self.source_ready.then(|| {
                self.sources.capture(
                    memory,
                    base,
                    state,
                    countries.as_ref(),
                    &store.sources,
                    seed,
                )
            });
            let observations = store.observations.is_taken().then_some(&store.observations);
            let officer_receiver = seed
                .pointer
                .checked_add(engine_native_land::unit_history::OFFICER);
            let experience_quiet = observations.and_then(|observations| {
                store.experience.prefetch_quiet(
                    index,
                    seed,
                    seed.pointer,
                    &observations.experience,
                    &mut self.orders[0],
                )
            });
            let progression_quiet = observations.and_then(|observations| {
                store.progression.prefetch_quiet(
                    index,
                    seed,
                    seed.pointer,
                    &observations.progression,
                    &mut self.orders[1],
                )
            });
            let officer_quiet = observations.zip(officer_receiver).and_then(|(observations, receiver)| {
                store.officer.prefetch_quiet(
                    index,
                    seed,
                    receiver,
                    &observations.history,
                    &mut self.orders[2],
                )
            });
            for (kind, quiet) in [
                experience_quiet.is_some(),
                progression_quiet.is_some(),
                officer_quiet.is_some(),
            ]
            .into_iter()
            .enumerate()
            {
                if quiet {
                    crate::observations::quiet(kind);
                }
            }
            let experience_certificate = match experience_quiet {
                Some((certificate, _)) => Some(certificate),
                None => changes.experience_range_sequence(seed.pointer),
            };
            let progression_certificate = match progression_quiet {
                Some((certificate, _)) => Some(certificate),
                None => changes.progression_range_sequence(seed.pointer),
            };
            let experience_acquired = experience_certificate.is_some();
            let progression_acquired = progression_certificate.is_some();
            let officer_certificate = match officer_quiet {
                Some((certificate, _)) => Some(certificate),
                None => officer_receiver.and_then(history_changes::range_sequence),
            };
            let officer_acquired = officer_certificate.is_some();
            let career_quiet = experience_certificate
                .and(observations)
                .and_then(|observations| {
                    store.career.prefetch_quiet(
                        index,
                        seed,
                        seed.pointer,
                        &observations.history,
                        &mut self.orders[3],
                    )
                });
            if career_quiet.is_some() {
                crate::observations::quiet(3);
            }
            let career_certificate = match career_quiet {
                Some((certificate, _)) => Some(certificate),
                None => experience_certificate
                    .and_then(|_| history_changes::range_sequence(seed.pointer)),
            };
            let career_acquired = career_certificate.is_some();
            let experience = match experience_quiet {
                Some((_, value)) => Some(Ok(value)),
                None => store.experience.prefetch_value(
                    base,
                    index,
                    seed,
                    experience_certificate,
                    &mut self.orders[0],
                    || {
                        engine_native::read::<i64>(
                            memory,
                            seed.pointer
                                .checked_add(engine_native::carmy::EXPERIENCE)
                                .ok_or("division experience address overflow")?,
                        )
                        .ok_or("division experience unreadable")
                    },
                ),
            };
            let progression = match progression_quiet {
                Some((_, value)) => Some(Ok(value)),
                None => store.progression.prefetch_value(
                    base,
                    index,
                    seed,
                    progression_certificate,
                    &mut self.orders[1],
                    || engine_native::carmy::pending_template(memory, base, seed.pointer),
                ),
            };
            let officer = match officer_quiet {
                Some((_, value)) => Some(Ok(value)),
                None => store.officer.prefetch_value(
                    base,
                    index,
                    seed,
                    officer_certificate,
                    &mut self.orders[2],
                    || engine_native_land::unit_history::officer_experience(memory, base, seed.pointer),
                ),
            };
            let career = match career_quiet {
                Some((_, value)) => Some(Ok(value)),
                None => store.career.prefetch_value(
                    base,
                    index,
                    seed,
                    career_certificate,
                    &mut self.orders[3],
                    || engine_native_land::unit_history::career_counter(memory, seed.pointer),
                ),
            };
            self.rows.push(PreparedDivision {
                experience_certificate,
                progression_certificate,
                officer_certificate,
                career_certificate,
                experience,
                progression,
                officer,
                career,
                selection,
                acquired: [
                    experience_acquired,
                    progression_acquired,
                    officer_acquired,
                    career_acquired,
                ],
                history: observations
                    .and_then(|observations| {
                        store.history.prefetch_quiet(
                            index,
                            seed,
                            &observations.history,
                            &mut self.orders[4],
                        )
                    })
                    .or_else(|| {
                        store
                            .history
                            .prefetch_certificates(index, seed, &mut self.orders[4])
                    }),
            });
        }
        self.stage_maxima(memory, base, seeds, store);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    struct Warm {
        present: bool,
        experience: AtomicUsize,
        progression: AtomicUsize,
    }
    impl Changes for Warm {
        fn experience_range_sequence(&self, _: usize) -> Option<(u64, u64)> {
            self.experience.fetch_add(1, Relaxed);
            self.present.then_some((1, 1))
        }
        fn progression_range_sequence(&self, _: usize) -> Option<(u64, u64)> {
            self.progression.fetch_add(1, Relaxed);
            self.present.then_some((1, 1))
        }
        fn experience_sequence(&self, _: usize) -> Option<(u64, u64)> {
            panic!("a range takes its certificates once, through its own lookup")
        }
        fn progression_sequence(&self, _: usize) -> Option<(u64, u64)> {
            panic!("a range takes its certificates once, through its own lookup")
        }
        fn take(&self, _: &mut dyn FnMut(usize)) -> Option<u64> {
            panic!("range must not consume marks")
        }
        fn register(&self, _: usize) -> bool {
            panic!("range must not admit stat receivers")
        }
    }

    fn seeds(count: usize) -> Vec<DivisionSeed> {
        (1..=count)
            .map(|id| DivisionSeed {
                division: id as u64,
                pointer: id * 0x1000,
                handle: Some(id as u64),
                class: Some(0),
                primary_vtable: Some(engine_native::carmy::VTABLE_RVA.0 as u64),
                secondary_vtable: Some(engine_native::carmy::SECONDARY_VTABLE_RVA.0 as u64),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn a_fresh_attach_registers_cold_receivers_across_the_ranges() {
        const COUNT: usize = 5_000;
        let seeds = seeds(COUNT);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        for parts in [1, 2, 7] {
            let watches = crate::army_stat_changes::Watches::enabled(COUNT);
            let mut store = ArmyStatStore::default();
            store.experience.begin(COUNT).unwrap();
            store.progression.begin(COUNT).unwrap();
            store.experience.finish(true);
            store.progression.finish(true);
            let mut captures: Vec<_> = (0..parts)
                .map(|part| PreparationPart {
                    range: COUNT * part / parts..COUNT * (part + 1) / parts,
                    ..Default::default()
                })
                .collect();
            std::thread::scope(|scope| {
                for capture in &mut captures {
                    let (memory, seeds, store, watches) = (&memory, &seeds, &store, &watches);
                    scope.spawn(move || capture.capture(memory, 0, 0, seeds, Some(store), watches));
                }
            });
            for capture in &captures {
                for index in capture.range.clone() {
                    let row = capture.get(index).unwrap();
                    let pointer = seeds[index].pointer;
                    assert_eq!(row.acquired[..2], [true; 2], "{parts} parts, {index}");
                    assert!(row.experience_certificate.is_some());
                    assert_eq!(row.experience_certificate, watches.experience_sequence(pointer));
                    assert_eq!(
                        row.progression_certificate,
                        watches.progression_sequence(pointer)
                    );
                }
            }
        }
    }

    #[test]
    fn preparation_queries_each_scalar_once_through_its_range_lookup() {
        const COUNT: usize = 5_000;
        let seeds = seeds(COUNT);
        let mut store = ArmyStatStore::default();
        store.experience.begin(COUNT).unwrap();
        store.progression.begin(COUNT).unwrap();
        for seed in &seeds {
            store.experience
                .observe_with(0, seed, Some((1, 1)), || Ok(-(seed.division as i64)))
                .unwrap();
            store.progression
                .observe_with(0, seed, Some((1, 1)), || Ok(seed.division + 1))
                .unwrap();
        }
        store.experience.finish(true);
        store.progression.finish(true);
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        for parts in [1, 7] {
            for present in [false, true] {
                let changes = Warm {
                    present,
                    experience: AtomicUsize::new(0),
                    progression: AtomicUsize::new(0),
                };
                for part in 0..parts {
                    let range = COUNT * part / parts..COUNT * (part + 1) / parts;
                    let mut capture = PreparationPart {
                        range: range.clone(),
                        ..Default::default()
                    };
                    capture.capture(&memory, 0, 0, &seeds, Some(&store), &changes);
                    for index in range {
                        let row = capture.get(index).unwrap();
                        assert_eq!(row.acquired[..2], [present; 2]);
                        assert_eq!(row.experience_certificate, present.then_some((1, 1)));
                        assert_eq!(row.progression_certificate, present.then_some((1, 1)));
                        assert_eq!(row.experience, present.then_some(Ok(-((index + 1) as i64))));
                        assert_eq!(row.progression, present.then_some(Ok((index + 2) as u64)));
                    }
                }
                assert_eq!(changes.experience.load(Relaxed), COUNT);
                assert_eq!(changes.progression.load(Relaxed), COUNT);
            }
        }
    }
}

#[derive(Default)]
pub struct AttackPart {
    pub range: Range<usize>,
    rows: Vec<AttackEntry>,
    len: usize,
}
#[derive(Default)]
struct AttackEntry {
    pointer: Option<usize>,
    maxima: Option<DivisionMaxima>,
    value: Option<Result<stats::AttackStats, &'static str>>,
    terrain: Vec<i64>,
    movement: Vec<i64>,
}
impl AttackPart {
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<AttackEntry>()
            + self
                .rows
                .iter()
                .map(|row| {
                    (row.terrain.capacity()
                        + row.movement.capacity()
                        + row
                            .value
                            .as_ref()
                            .and_then(|value| value.as_ref().ok())
                            .map_or(0, |value| {
                                value.terrain.capacity() + value.movement_terrain.capacity()
                            }))
                        * 8
                })
                .sum::<usize>()
    }
    pub fn prepare(
        &mut self,
        seeds: &[DivisionSeed],
        hour: i32,
        held_index: &[Option<u32>],
        store: Option<&ArmyStatStore>,
        maxima: impl Fn(usize) -> Option<DivisionMaxima>,
    ) {
        self.len = 0;
        let Some(seeds) = seeds.get(self.range.clone()) else {
            return;
        };
        let additional = seeds.len().saturating_sub(self.rows.len());
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.rows, additional))
            .is_err()
        {
            return;
        }
        self.rows
            .resize_with(self.rows.len().max(seeds.len()), AttackEntry::default);
        for (offset, seed) in seeds.iter().enumerate() {
            let index = self.range.start + offset;
            let row = &mut self.rows[offset];
            if let Some(Ok(value)) = row.value.take() {
                row.terrain = value.terrain;
                row.movement = value.movement_terrain;
            }
            let held = store
                .and_then(|store| {
                    held_index
                        .get(index)
                        .copied()
                        .flatten()
                        .and_then(|at| store.held.get(at as usize))
                })
                .filter(|held| Some(held.stat_block) == seed.stat_block);
            let carried = held.is_some() && !witnessed(hour, seed.division >> 32);
            row.pointer = seed.stat_block.filter(|_| seed.division != 0 && !carried);
            row.maxima = maxima(index);
        }
        self.len = seeds.len();
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        for row in &mut self.rows[..self.len] {
            if let Some(pointer) = row.pointer {
                row.value = Some(stats::capture_attack_stats_holding(
                    memory,
                    base,
                    pointer,
                    &mut row.terrain,
                    &mut row.movement,
                    row.maxima,
                ));
            }
        }
    }
    pub fn is_prepared(&self) -> bool {
        self.len == self.range.len()
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn get(&self, index: usize) -> Option<Result<&stats::AttackStats, &'static str>> {
        let index = index.checked_sub(self.range.start)?;
        if index >= self.len {
            return None;
        }
        self.rows
            .get(index)?
            .value
            .as_ref()
            .map(|value| value.as_ref().map_err(|&reason| reason))
    }
}
