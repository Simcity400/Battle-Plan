//! Range-local observations; ordered certificate/store admission stays in the parent.
use super::*;
use collector_observers::unit_history_changes as history_changes;
use std::{ops::Range, sync::Arc};

#[derive(Clone, Copy)]
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
    orders: [ScalarOrder; 4],
    supply: Vec<Option<Arc<crate::supply_inputs::Sample>>>,
    supply_order: ScalarOrder,
    sources: crate::panel_sources::Preparation,
    source_ready: bool,
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
    }
    pub fn get(&self, index: usize) -> Option<PreparedDivision> {
        self.rows.get(index.checked_sub(self.range.start)?).copied()
    }
    /// An absent entry selects ordered lookup; a present `None` is a sampled miss.
    pub fn daily_supply(&self, index: usize) -> Option<&Option<Arc<crate::supply_inputs::Sample>>> {
        self.supply.get(index.checked_sub(self.range.start)?)
    }
    pub fn can_settle_sources(&self) -> bool {
        self.source_ready && self.rows.len() == self.range.len() && self.sources.can_settle_owned()
    }
    pub fn has_local_source_refusal(&self) -> bool {
        self.source_ready && self.sources.has_local_province_refusal()
    }
    pub fn source_reads(&self) -> Option<&crate::panel_sources::Preparation> {
        self.source_ready.then_some(&self.sources)
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
        // A failed optional staging allocation performs no native read. The
        // ordered consumer uses its original acquisition for this range alone.
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.rows, seeds.len()))
            .is_err()
        {
            return;
        }
        // This immutable daily baseline cannot change until the range leases
        // return. Optional scratch failure keeps the original ordered lookup.
        let supply_ready = store.holds_daily_progress()
            && engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.supply, seeds.len())
            })
            .is_ok();
        let source_reads = self.sources.begin(seeds.len());
        let countries = source_reads
            .then(|| engine_native::ccountry::Reader::new(memory, base, state).ok())
            .flatten();
        // The ordered source pass authenticates these headers even if no
        // division resolves an owner. Decode must not bypass that failure gate.
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
            // These receivers are already registered. The passive query performs
            // the same epoch/quiet checks without a second, no-op registration.
            // Cold receivers still reach the original ordered admission.
            let experience_certificate = changes.experience_observed_sequence(seed.pointer);
            let progression_certificate = changes.progression_observed_sequence(seed.pointer);
            let experience_acquired = experience_certificate.is_some();
            let progression_acquired = progression_certificate.is_some();
            let officer_receiver = seed
                .pointer
                .checked_add(engine_native::unit_history::OFFICER);
            let officer_certificate = officer_receiver.and_then(history_changes::observed_sequence);
            let officer_acquired = officer_certificate.is_some();
            let career_certificate = experience_certificate
                .and_then(|_| history_changes::observed_sequence(seed.pointer));
            let career_acquired = career_certificate.is_some();
            let experience = store.experience.prefetch_value(
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
            );
            let progression = store.progression.prefetch_value(
                base,
                index,
                seed,
                progression_certificate,
                &mut self.orders[1],
                || engine_native::carmy::pending_template(memory, base, seed.pointer),
            );
            let officer = store.officer.prefetch_value(
                base,
                index,
                seed,
                officer_certificate,
                &mut self.orders[2],
                || engine_native::unit_history::officer_experience(memory, base, seed.pointer),
            );
            let career = store.career.prefetch_value(
                base,
                index,
                seed,
                career_certificate,
                &mut self.orders[3],
                || engine_native::unit_history::career_counter(memory, seed.pointer),
            );
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
                history: store.history.prefetch_certificates(index, seed),
            });
        }
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
        fn experience_observed_sequence(&self, _: usize) -> Option<(u64, u64)> {
            self.experience.fetch_add(1, Relaxed);
            self.present.then_some((1, 1))
        }
        fn progression_observed_sequence(&self, _: usize) -> Option<(u64, u64)> {
            self.progression.fetch_add(1, Relaxed);
            self.present.then_some((1, 1))
        }
        fn experience_sequence(&self, _: usize) -> Option<(u64, u64)> {
            panic!("range must neither register cold receivers nor re-query warm ones")
        }
        fn progression_sequence(&self, _: usize) -> Option<(u64, u64)> {
            panic!("range must neither register cold receivers nor re-query warm ones")
        }
        fn take(&self, _: &mut dyn FnMut(usize)) -> Option<u64> {
            panic!("range must not consume marks")
        }
        fn register(&self, _: usize) -> bool {
            panic!("range must not admit stat receivers")
        }
    }

    #[test]
    fn preparation_queries_each_warm_scalar_once_and_leaves_cold_admission_ordered() {
        const COUNT: usize = 5_000;
        let seeds: Vec<_> = (1..=COUNT)
            .map(|id| DivisionSeed {
                division: id as u64,
                pointer: id * 0x1000,
                handle: Some(id as u64),
                class: Some(0),
                primary_vtable: Some(engine_native::carmy::VTABLE_RVA as u64),
                secondary_vtable: Some(engine_native::carmy::SECONDARY_VTABLE_RVA as u64),
                ..Default::default()
            })
            .collect();
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

/// Fresh attack reads planned after ordered certification. Each entry retains
/// its terrain buffers; neither a carried block nor an invalid seed is read.
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
        captured: &Captured,
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
                    captured
                        .held_index
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
