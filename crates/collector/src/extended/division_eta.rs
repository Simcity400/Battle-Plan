//! Hourly native ETA operands from the shared division census. Normal speed
//! composition and route duration consume owned inputs on the spool thread.
use super::production::Sources;
use crate::{
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, carmy::DivisionSeed, cprovince, cunit, walk::Outcome};
use replay::{
    frame::TableInput,
    rows::{DivisionEtaRow, TableId, tables},
};
use std::{cell::RefCell, mem::size_of};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionEta];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Callback-local counts, accumulated across the one or two acquisition passes.
/// Journal sizes are peak bounded army-scope demand, not allocated pool bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CacheDiagnostics {
    pub hits: u64,
    pub retained_misses: u64,
    pub direct_reads: u64,
    pub witness_skips: u64,
    pub witness_fallbacks: u64,
    pub witness_entries_peak: usize,
    pub witness_bytes_peak: usize,
    pub witness_entries_required: usize,
    pub witness_bytes_required: usize,
    pub stages: StageDiagnostics,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StageCost {
    pub calls: u64,
    pub ticks: u64,
}
impl StageCost {
    fn record(&mut self, started: i64) {
        self.calls = self.calls.saturating_add(1);
        self.ticks = self
            .ticks
            .saturating_add(qpc().saturating_sub(started).max(0) as u64);
    }
    fn absorb(&mut self, next: Self) {
        self.calls = self.calls.saturating_add(next.calls);
        self.ticks = self.ticks.saturating_add(next.ticks);
    }
}

/// Nested acquisition service spans, plus disjoint pass-final verification.
/// No QPC call or trace is added to the scalar-memory read path. Capability
/// fallbacks execute in the engine outside the corresponding resolver span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StageDiagnostics {
    pub walk: StageCost,
    pub supply_acquire: StageCost,
    pub supply_verify: StageCost,
    pub transport_acquire: StageCost,
    pub transport_verify: StageCost,
    pub modifiers_acquire: StageCost,
    pub modifiers_verify: StageCost,
    pub primary_acquire: StageCost,
    pub primary_verify: StageCost,
    pub cache_verify: StageCost,
    pub scope_verify: StageCost,
    pub supply_builds: u64,
    pub transport_builds: u64,
    pub supply_fallbacks: u64,
    pub modifier_fallbacks: u64,
}
impl StageDiagnostics {
    pub(crate) fn absorb(&mut self, next: Self) {
        macro_rules! spans { ($($field:ident),+ $(,)?) => { $(self.$field.absorb(next.$field);)+ }; }
        spans!(
            walk,
            supply_acquire,
            supply_verify,
            transport_acquire,
            transport_verify,
            modifiers_acquire,
            modifiers_verify,
            primary_acquire,
            primary_verify,
            cache_verify,
            scope_verify
        );
        self.supply_builds = self.supply_builds.saturating_add(next.supply_builds);
        self.transport_builds = self.transport_builds.saturating_add(next.transport_builds);
        self.supply_fallbacks = self.supply_fallbacks.saturating_add(next.supply_fallbacks);
        self.modifier_fallbacks = self
            .modifier_fallbacks
            .saturating_add(next.modifier_fallbacks);
    }
}

fn measure_stage<T>(
    cache: &RefCell<SmallReadCache>,
    stage: impl FnOnce(&mut StageDiagnostics) -> &mut StageCost,
    work: impl FnOnce() -> T,
) -> T {
    let started = qpc();
    let result = work();
    stage(&mut cache.borrow_mut().diagnostics.stages).record(started);
    result
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    legs: usize,
    cache_entries: usize,
    witness_entries: usize,
    witness_bytes: usize,
    transport_entries: usize,
    modifier_accumulators: usize,
    modifier_bytes: usize,
    supply_entries: usize,
    air_defense_entries: usize,
    primary_roots: usize,
    primary_chunks: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 8192,
            legs: 32 * 1024,
            cache_entries: 128 * 1024,
            witness_entries: 1024,
            witness_bytes: 64 * 1024,
            transport_entries: 1024,
            modifier_accumulators: 1024,
            modifier_bytes: 32 * 1024 * 16,
            supply_entries: 1024,
            air_defense_entries: 1024,
            primary_roots: 1024,
            primary_chunks: 2048,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<cunit::EtaInputs>,
    legs: Arena<cunit::EtaLeg>,
    cache: RefCell<SmallReadCache>,
    transport: TransportMemo,
    modifiers: RefCell<ModifierSnapshots>,
    supply: RefCell<SupplyMemo>,
    air_defense: RefCell<AirDefenseMemo>,
    primary: RefCell<engine::carmy_leader::PrimaryRootCache>,
    completed: RefCell<Vec<DivisionEtaRow>>,
    seed_masks: [Vec<u64>; 2],
    seed_required: usize,
    geometry: cprovince::GeometryPacket,
    geometry_second: Option<cprovince::GeometryPacket>,
    geometry_required: cprovince::GeometryCapacities,
}
impl Captured {
    /// Off-thread mask storage follows the full shared census independently of
    /// ETA row and leg admission. Private row limits must remain truncation.
    pub(crate) fn reserve_sources(&mut self, capacity: usize) {
        crate::table_capture::grow_vec(&mut self.seed_masks[0], capacity);
        if self.geometry_second.is_some() {
            crate::table_capture::grow_vec(&mut self.seed_masks[1], capacity);
        }
    }

    pub(crate) fn reject_source_census(&mut self) -> bool {
        if !self.rows.checked {
            return true;
        }
        self.rows.outcome = Outcome::Unavailable {
            reason: "division ETA source census changed during double read",
        };
        self.rows.mismatches = self.rows.mismatches.saturating_add(1);
        false
    }

    pub(crate) fn consumed_seed_masks(&self, pass: usize) -> &[u64] {
        &self.seed_masks[pass]
    }

    /// The callback union verifier supplies fresh changed bits after every
    /// acquisition worker joins. Only fields consumed by ETA can reject ETA.
    pub(crate) fn reject_changed_seeds(&mut self, pass: usize, changed: &[u64]) -> bool {
        let masks = &self.seed_masks[pass];
        let valid = masks.iter().enumerate().all(|(index, &mask)| {
            mask == 0
                || changed
                    .get(index)
                    .is_some_and(|changed| changed & mask == 0)
        });
        if !valid {
            self.rows.outcome = Outcome::Unavailable {
                reason: "division ETA shared seed changed or unreadable",
            };
            if self.rows.checked {
                self.rows.mismatches = self.rows.mismatches.saturating_add(1);
            }
        }
        valid
    }

    pub(crate) fn cache_diagnostics(&self) -> CacheDiagnostics {
        self.cache.borrow().diagnostics
    }
    pub(crate) fn empty() -> Self {
        Self::with_capacities(
            &Capacities {
                rows: 0,
                legs: 0,
                cache_entries: 0,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        )
    }
    pub(crate) fn fail(&mut self, reason: &'static str, checked: bool) {
        self.reset();
        self.rows.outcome = Outcome::Unavailable { reason };
        self.rows.checked = checked;
    }
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(c.rows, successful[0]),
            legs: Arena::new(c.legs, successful[0]),
            cache: RefCell::new(SmallReadCache::with_witness(
                c.cache_entries,
                c.witness_entries,
                c.witness_bytes,
            )),
            transport: TransportMemo::new(c.transport_entries),
            modifiers: RefCell::new(ModifierSnapshots::new(
                c.modifier_accumulators,
                c.modifier_bytes,
            )),
            supply: RefCell::new(SupplyMemo::new(c.supply_entries)),
            air_defense: RefCell::new(AirDefenseMemo::new(c.air_defense_entries)),
            primary: RefCell::new(engine::carmy_leader::PrimaryRootCache::new(
                c.primary_roots,
                c.primary_chunks,
            )),
            completed: RefCell::new(Vec::with_capacity(c.rows)),
            seed_masks: [
                vec![0; c.rows],
                vec![
                    0;
                    if successful[0] < crate::sampling::CONSISTENCY_TARGET {
                        c.rows
                    } else {
                        0
                    }
                ],
            ],
            seed_required: 0,
            geometry: cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                provinces: c.rows.saturating_mul(2),
                neighbour_entries: c.legs.saturating_mul(4),
            }),
            geometry_second: (successful[0] < crate::sampling::CONSISTENCY_TARGET).then(|| {
                cprovince::GeometryPacket::with_capacities(cprovince::GeometryCapacities {
                    provinces: c.rows.saturating_mul(2),
                    neighbour_entries: c.legs.saturating_mul(4),
                })
            }),
            geometry_required: cprovince::GeometryCapacities {
                provinces: 0,
                neighbour_entries: 0,
            },
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.legs.reset();
        self.cache.get_mut().clear();
        self.transport.clear();
        self.modifiers.get_mut().clear();
        self.supply.get_mut().clear();
        self.air_defense.get_mut().clear();
        self.primary.get_mut().clear();
        self.completed.get_mut().clear();
        self.seed_masks.iter_mut().for_each(|masks| masks.fill(0));
        self.seed_required = 0;
        self.geometry.clear();
        if let Some(geometry) = &mut self.geometry_second {
            geometry.clear();
        }
        self.geometry_required = cprovince::GeometryCapacities {
            provinces: 0,
            neighbour_entries: 0,
        };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.legs.allocated_bytes()
            + self.cache.borrow().allocated_bytes()
            + self.transport.allocated_bytes()
            + self.modifiers.borrow().allocated_bytes()
            + self.supply.borrow().allocated_bytes()
            + self.air_defense.borrow().allocated_bytes()
            + self.primary.borrow().allocated_bytes()
            + self.completed.borrow().capacity() * size_of::<DivisionEtaRow>()
            + self
                .seed_masks
                .iter()
                .map(|masks| masks.capacity() * size_of::<u64>())
                .sum::<usize>()
            + self.geometry.allocated_bytes()
            + self
                .geometry_second
                .as_ref()
                .map_or(0, cprovince::GeometryPacket::allocated_bytes)
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.rows.grow(&mut c.rows, successful[0]);
        self.legs.grow(&mut c.legs, successful[0]);
        self.cache.get_mut().grow(c.cache_entries);
        self.cache
            .get_mut()
            .witness
            .grow(c.witness_entries, c.witness_bytes);
        self.transport.grow(c.transport_entries);
        self.modifiers
            .get_mut()
            .grow(c.modifier_accumulators, c.modifier_bytes);
        self.supply.get_mut().grow(c.supply_entries);
        self.air_defense.get_mut().grow(c.air_defense_entries);
        self.primary
            .get_mut()
            .grow(c.primary_roots, c.primary_chunks);
        let completed = self.completed.get_mut();
        completed.clear();
        completed.reserve(c.rows);
        let seed_capacity = self.seed_required.max(c.rows);
        crate::table_capture::grow_vec(&mut self.seed_masks[0], seed_capacity);
        crate::table_capture::comparison(&mut self.seed_masks[1], seed_capacity, successful[0]);
        let geometry_capacities = cprovince::GeometryCapacities {
            provinces: self
                .geometry_required
                .provinces
                .checked_next_power_of_two()
                .unwrap_or(self.geometry_required.provinces)
                .max(c.rows.saturating_mul(2)),
            neighbour_entries: self
                .geometry_required
                .neighbour_entries
                .checked_next_power_of_two()
                .unwrap_or(self.geometry_required.neighbour_entries)
                .max(c.legs.saturating_mul(4)),
        };
        self.geometry.grow(geometry_capacities);
        if successful[0] < crate::sampling::CONSISTENCY_TARGET {
            if let Some(geometry) = &mut self.geometry_second {
                geometry.grow(geometry_capacities);
            } else {
                self.geometry_second = Some(cprovince::GeometryPacket::with_capacities(
                    geometry_capacities,
                ));
            }
        } else {
            self.geometry_second = None;
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
        sources: Sources<'_, DivisionSeed>,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        self.reset();
        self.seed_required = sources.first_outcome.len().max(if needed[0] {
            sources.second_outcome.len()
        } else {
            0
        });
        let started = qpc();
        let cached = CacheRef {
            source: memory,
            cache: &self.cache,
            modifiers: Some(&self.modifiers),
            supply: Some(&self.supply),
            air_defense: Some(&self.air_defense),
            primary: Some(&self.primary),
        };
        let geometry_active = !matches!(sources.first_outcome, Outcome::Unavailable { .. })
            && sources.first_outcome.len() <= sources.first.len()
            && sources.first_outcome.len() <= self.seed_masks[0].len();
        let geometry_ready = if geometry_active {
            self.geometry.begin(memory, base, state)
        } else {
            match sources.first_outcome {
                Outcome::Unavailable { reason } => Err(reason),
                _ => Err("ETA division seed storage invalid"),
            }
        };
        let mut first = measure_stage(
            &self.cache,
            |stage| &mut stage.walk,
            || {
                geometry_ready.map_or_else(
                    |reason| cunit::EtaWalk {
                        outcome: Outcome::Unavailable { reason },
                        legs_len: 0,
                        required_legs: 0,
                    },
                    |()| {
                        cunit::walk_eta_with_transport(
                            &cached,
                            memory,
                            base,
                            state,
                            sources.first,
                            sources.first_outcome,
                            &mut self.rows.rows,
                            &mut self.legs.first,
                            &mut self.seed_masks[0],
                            &mut self.geometry,
                            |owner, region| {
                                measure_stage(
                                    &self.cache,
                                    |stage| &mut stage.transport_acquire,
                                    || {
                                        self.transport.resolve(owner, region, || {
                                            let mut diagnostics = self.cache.borrow_mut();
                                            diagnostics.diagnostics.stages.transport_builds =
                                                diagnostics
                                                    .diagnostics
                                                    .stages
                                                    .transport_builds
                                                    .saturating_add(1);
                                            drop(diagnostics);
                                            engine::cnaval_unit_transfer::movement_speed(
                                                &cached, base, state, owner, region,
                                            )
                                        })
                                    },
                                )
                            },
                        )
                    },
                )
            },
        );
        if geometry_active && let Err(reason) = self.geometry.verify(memory) {
            first.outcome = Outcome::Unavailable { reason };
        }
        self.geometry_required = self.geometry.required_capacities();
        if !measure_stage(
            &self.cache,
            |stage| &mut stage.transport_verify,
            || {
                self.transport.verify_and_clear(|owner, region| {
                    engine::cnaval_unit_transfer::movement_speed(memory, base, state, owner, region)
                })
            },
        ) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA transport speed changed or unreadable",
            };
        }
        if !measure_stage(
            &self.cache,
            |stage| &mut stage.supply_verify,
            || {
                self.supply
                    .borrow_mut()
                    .verify_and_clear(|base, state, owner, area| {
                        engine::ccountry::has_core_supply_access_direct(
                            memory, base, state, owner, area,
                        )
                    })
            },
        ) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA core supply access changed or unreadable",
            };
        }
        if !measure_stage(
            &self.cache,
            |stage| &mut stage.modifiers_verify,
            || self.modifiers.borrow_mut().verify_and_clear(memory),
        ) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA modifier snapshot changed or unreadable",
            };
        }
        self.air_defense.borrow_mut().clear();
        if !measure_stage(
            &self.cache,
            |stage| &mut stage.primary_verify,
            || self.primary.borrow_mut().verify_and_clear(memory),
        ) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA primary order dependencies changed or unreadable",
            };
        }
        if !measure_stage(
            &self.cache,
            |stage| &mut stage.cache_verify,
            || self.cache.borrow_mut().verify_and_clear(memory),
        ) {
            first.outcome = Outcome::Unavailable {
                reason: "division ETA cached operands changed or unreadable",
            };
        }
        self.rows.outcome = first.outcome;
        self.legs.len = first.legs_len;
        self.legs.required = first.required_legs;
        let mut pass = false;
        if needed[0] {
            self.rows.checked = true;
            let Some(geometry) = &mut self.geometry_second else {
                self.rows.mismatches = self.rows.mismatches.saturating_add(1);
                return ([false], [qpc().saturating_sub(started).max(0) as u64]);
            };
            let geometry_active = !matches!(sources.second_outcome, Outcome::Unavailable { .. })
                && sources.second_outcome.len() <= sources.second.len()
                && sources.second_outcome.len() <= self.seed_masks[1].len();
            let geometry_ready = if geometry_active {
                geometry.begin(memory, base, state)
            } else {
                match sources.second_outcome {
                    Outcome::Unavailable { reason } => Err(reason),
                    _ => Err("ETA division seed storage invalid"),
                }
            };
            let mut second = measure_stage(
                &self.cache,
                |stage| &mut stage.walk,
                || {
                    geometry_ready.map_or_else(
                        |reason| cunit::EtaWalk {
                            outcome: Outcome::Unavailable { reason },
                            legs_len: 0,
                            required_legs: 0,
                        },
                        |()| {
                            cunit::walk_eta_with_transport(
                                &cached,
                                memory,
                                base,
                                state,
                                sources.second,
                                sources.second_outcome,
                                &mut self.rows.second,
                                &mut self.legs.second,
                                &mut self.seed_masks[1],
                                geometry,
                                |owner, region| {
                                    measure_stage(
                                        &self.cache,
                                        |stage| &mut stage.transport_acquire,
                                        || {
                                            self.transport.resolve(owner, region, || {
                                                let mut diagnostics = self.cache.borrow_mut();
                                                diagnostics.diagnostics.stages.transport_builds =
                                                    diagnostics
                                                        .diagnostics
                                                        .stages
                                                        .transport_builds
                                                        .saturating_add(1);
                                                drop(diagnostics);
                                                engine::cnaval_unit_transfer::movement_speed(
                                                    &cached, base, state, owner, region,
                                                )
                                            })
                                        },
                                    )
                                },
                            )
                        },
                    )
                },
            );
            if geometry_active && let Err(reason) = geometry.verify(memory) {
                second.outcome = Outcome::Unavailable { reason };
            }
            let geometry_same = self.geometry.same_observation(geometry);
            let required = geometry.required_capacities();
            self.geometry_required.provinces =
                self.geometry_required.provinces.max(required.provinces);
            self.geometry_required.neighbour_entries = self
                .geometry_required
                .neighbour_entries
                .max(required.neighbour_entries);
            if !measure_stage(
                &self.cache,
                |stage| &mut stage.transport_verify,
                || {
                    self.transport.verify_and_clear(|owner, region| {
                        engine::cnaval_unit_transfer::movement_speed(
                            memory, base, state, owner, region,
                        )
                    })
                },
            ) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA transport speed changed or unreadable",
                };
            }
            if !measure_stage(
                &self.cache,
                |stage| &mut stage.supply_verify,
                || {
                    self.supply
                        .borrow_mut()
                        .verify_and_clear(|base, state, owner, area| {
                            engine::ccountry::has_core_supply_access_direct(
                                memory, base, state, owner, area,
                            )
                        })
                },
            ) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA core supply access changed or unreadable",
                };
            }
            if !measure_stage(
                &self.cache,
                |stage| &mut stage.modifiers_verify,
                || self.modifiers.borrow_mut().verify_and_clear(memory),
            ) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA modifier snapshot changed or unreadable",
                };
            }
            self.air_defense.borrow_mut().clear();
            if !measure_stage(
                &self.cache,
                |stage| &mut stage.primary_verify,
                || self.primary.borrow_mut().verify_and_clear(memory),
            ) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA primary order dependencies changed or unreadable",
                };
            }
            if !measure_stage(
                &self.cache,
                |stage| &mut stage.cache_verify,
                || self.cache.borrow_mut().verify_and_clear(memory),
            ) {
                second.outcome = Outcome::Unavailable {
                    reason: "division ETA cached operands changed or unreadable",
                };
            }
            let rows = self.rows.compare(second.outcome);
            let legs = self.legs.compare(second.legs_len, second.required_legs);
            self.rows.mismatches += usize::from(!legs);
            self.rows.mismatches += usize::from(!geometry_same);
            pass = rows && legs && geometry_same;
        }
        ([pass], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "division ETA operands changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
    }
    pub(crate) fn table_inputs(
        &self,
        projection: Option<&mut crate::projection::Projection<'_>>,
    ) -> [TableInput; N] {
        let outcome = self.outcomes()[0];
        if let Outcome::Unavailable { reason } = outcome {
            return [unavailable(reason).into()];
        }
        let Some(inputs) = self.rows.rows.get(..outcome.len()) else {
            return [unavailable("division ETA row span invalid").into()];
        };
        let Some(legs) = self.legs.first.get(..self.legs.len) else {
            return [unavailable("division ETA leg arena invalid").into()];
        };
        let mut rows = self.completed.borrow_mut();
        rows.clear();
        for input in inputs {
            let (next_remaining, destination_remaining) = match cunit::eta_remaining(input, legs) {
                Ok(value) => value,
                Err(reason) => return [unavailable(reason).into()],
            };
            rows.push(DivisionEtaRow {
                division: input.division,
                next: input.next,
                destination: input.destination,
                path_total: input.path_total,
                next_remaining,
                destination_remaining,
            });
        }
        [crate::projection::scalar_table::<tables::DivisionEta, _>(
            projection,
            outcome,
            &rows,
            |row| u128::from(row.division),
            |row| row.clone(),
        )]
    }
    #[cfg(test)]
    pub fn tables(&self) -> [replay::frame::TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("unprojected ETA always returns a sample"),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AcquisitionReport {
    pub matched: bool,
    pub ticks: u64,
}

const SMALL_READ_BYTES: usize = 32;

#[derive(Clone, Copy, Default)]
struct ModifierSnapshot {
    address: usize,
    header: [u8; 16],
    start: usize,
    len: usize,
    occupied: bool,
}

/// Full raw sorted accumulators, owned only until this pass's direct final
/// verification. A collision or exhausted arena uses the original live reader;
/// no eviction, allocation, partial snapshot, or cross-pass reuse is allowed.
struct ModifierSnapshots {
    entries: Vec<ModifierSnapshot>,
    used: Vec<usize>,
    used_len: usize,
    bytes: Vec<u8>,
    bytes_len: usize,
}

impl ModifierSnapshots {
    fn new(capacity: usize, bytes: usize) -> Self {
        Self {
            entries: vec![ModifierSnapshot::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
            bytes: vec![0; bytes],
            bytes_len: 0,
        }
    }

    fn lookup(
        &mut self,
        source: &impl Memory,
        address: usize,
        key: u32,
    ) -> Option<Result<engine::modifier_accumulator::Value, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let slot = ((address >> 4) ^ (address >> 17)) % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.address == address {
                Some(engine::modifier_accumulator::lookup_snapshot(
                    &self.bytes[entry.start..entry.start + entry.len],
                    key,
                ))
            } else {
                None
            };
        }
        let Some(header) = engine::read::<[u8; 16]>(source, address) else {
            return Some(Err("modifier accumulator header unreadable or invalid"));
        };
        let Some(array) =
            engine::cpdx_array::Array::from_header(header, engine::modifier_entry::STRIDE)
        else {
            return Some(Err("modifier accumulator header unreadable or invalid"));
        };
        let Some(len) = (array.count as usize).checked_mul(engine::modifier_entry::STRIDE) else {
            return Some(Err("modifier snapshot byte length overflow"));
        };
        let end = self
            .bytes_len
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len())?;
        let bytes = &mut self.bytes[self.bytes_len..end];
        if len != 0 && !engine::read_slice(source, array.data, bytes) {
            return Some(Err("modifier snapshot entries unreadable"));
        }
        if let Err(reason) = engine::modifier_accumulator::validate_snapshot(bytes) {
            return Some(Err(reason));
        }
        if engine::read::<[u8; 16]>(source, address) != Some(header) {
            return Some(Err("modifier accumulator header changed"));
        }
        let result = engine::modifier_accumulator::lookup_snapshot(bytes, key);
        self.entries[slot] = ModifierSnapshot {
            address,
            header,
            start: self.bytes_len,
            len,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        self.bytes_len = end;
        Some(result)
    }

    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = ModifierSnapshot::default();
        }
        self.used_len = 0;
        self.bytes_len = 0;
    }

    fn verify_and_clear(&mut self, source: &impl Memory) -> bool {
        let mut matched = true;
        let mut fresh = [0_u8; 64 * 16];
        for &slot in &self.used[..self.used_len] {
            let entry = self.entries[slot];
            matched &= engine::read::<[u8; 16]>(source, entry.address) == Some(entry.header);
            // The stored header was fully bounded before insertion. Continue
            // checking all retained bytes even if a prior header/read failed.
            let data = u64::from_le_bytes(entry.header[..8].try_into().unwrap()) as usize;
            let mut offset = 0;
            while offset < entry.len {
                let count = (entry.len - offset).min(fresh.len());
                let copied = engine::read_slice(source, data + offset, &mut fresh[..count]);
                matched &= copied
                    && fresh[..count]
                        == self.bytes[entry.start + offset..entry.start + offset + count];
                offset += count;
            }
            matched &= engine::read::<[u8; 16]>(source, entry.address) == Some(entry.header);
            self.entries[slot] = ModifierSnapshot::default();
        }
        self.used_len = 0;
        self.bytes_len = 0;
        matched
    }

    fn grow(&mut self, capacity: usize, bytes: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, ModifierSnapshot::default());
            self.used.resize(capacity, 0);
        }
        if bytes > self.bytes.len() {
            self.bytes.resize(bytes, 0);
        }
    }

    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<ModifierSnapshot>()
            + self.used.capacity() * size_of::<usize>()
            + self.bytes.capacity()
    }
}

#[derive(Clone, Copy, Default)]
struct AirDefenseEntry {
    arguments: [i64; 3],
    value: i64,
    occupied: bool,
}

/// Pure calculation sharing, not an input snapshot. Every lookup follows the
/// original guarded reads; only the deterministic integer series is omitted on
/// a hit. No final recomputation is needed for equal complete scalar arguments.
struct AirDefenseMemo {
    entries: Vec<AirDefenseEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl AirDefenseMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![AirDefenseEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        arguments: [i64; 3],
        calculate: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Option<Result<i64, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let mut hash = 0_u64;
        for argument in arguments {
            hash ^= argument as u64;
            hash = hash.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(23);
        }
        let slot = hash as usize % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.arguments == arguments {
                Some(Ok(entry.value))
            } else {
                None
            };
        }
        let value = match calculate() {
            Ok(value) => value,
            Err(reason) => return Some(Err(reason)),
        };
        self.entries[slot] = AirDefenseEntry {
            arguments,
            value,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Some(Ok(value))
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = AirDefenseEntry::default();
        }
        self.used_len = 0;
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, AirDefenseEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<AirDefenseEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy, Default)]
struct SupplyEntry {
    base: usize,
    state: usize,
    area: usize,
    owner: u32,
    accessible: bool,
    occupied: bool,
}

/// Native country/area predicates shared only during one independently checked
/// pass. Occupied slots are never evicted; every retained result is recomputed
/// through the unshared engine reader before publication, including on errors.
struct SupplyMemo {
    entries: Vec<SupplyEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl SupplyMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![SupplyEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        base: usize,
        state: usize,
        owner: u32,
        area: usize,
        calculate: impl FnOnce() -> Result<bool, &'static str>,
    ) -> Option<Result<bool, &'static str>> {
        if self.entries.is_empty() {
            return None;
        }
        let hash = (owner as usize).wrapping_mul(0x9e37_79b9)
            ^ (area >> 3)
            ^ (area >> 19)
            ^ (base >> 16)
            ^ (state >> 3);
        let slot = hash % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if (entry.base, entry.state, entry.owner, entry.area)
                == (base, state, owner, area)
            {
                Some(Ok(entry.accessible))
            } else {
                None
            };
        }
        let accessible = match calculate() {
            Ok(value) => value,
            Err(reason) => return Some(Err(reason)),
        };
        self.entries[slot] = SupplyEntry {
            base,
            state,
            area,
            owner,
            accessible,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Some(Ok(accessible))
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = SupplyEntry::default();
        }
        self.used_len = 0;
    }
    fn verify_and_clear(
        &mut self,
        mut calculate: impl FnMut(usize, usize, u32, usize) -> Result<bool, &'static str>,
    ) -> bool {
        let mut matched = true;
        for &slot in &self.used[..self.used_len] {
            let entry = self.entries[slot];
            matched &=
                calculate(entry.base, entry.state, entry.owner, entry.area) == Ok(entry.accessible);
            self.entries[slot] = SupplyEntry::default();
        }
        self.used_len = 0;
        matched
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if capacity > self.entries.len() {
            self.entries.resize(capacity, SupplyEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<SupplyEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy, Default)]
struct TransportEntry {
    owner: u32,
    region: usize,
    speed: i64,
    occupied: bool,
}

/// Derived results are local to one base/game-state/pass. Even reads which
/// collided in SmallReadCache participate in the direct final recomputation of
/// every retained result. No eviction or inferred success on failed misses.
struct TransportMemo {
    entries: Vec<TransportEntry>,
    used: Vec<usize>,
    used_len: usize,
}
impl TransportMemo {
    fn new(capacity: usize) -> Self {
        Self {
            entries: vec![TransportEntry::default(); capacity],
            used: vec![0; capacity],
            used_len: 0,
        }
    }
    fn resolve(
        &mut self,
        owner: u32,
        region: usize,
        calculate: impl FnOnce() -> Result<i64, &'static str>,
    ) -> Result<i64, &'static str> {
        if self.entries.is_empty() {
            return calculate();
        }
        let hash = (owner as usize).wrapping_mul(0x9e37_79b9) ^ (region >> 3) ^ (region >> 19);
        let slot = hash % self.entries.len();
        let entry = self.entries[slot];
        if entry.occupied {
            return if entry.owner == owner && entry.region == region {
                Ok(entry.speed)
            } else {
                calculate()
            };
        }
        let speed = calculate()?;
        self.entries[slot] = TransportEntry {
            owner,
            region,
            speed,
            occupied: true,
        };
        self.used[self.used_len] = slot;
        self.used_len += 1;
        Ok(speed)
    }
    fn clear(&mut self) {
        for &slot in &self.used[..self.used_len] {
            self.entries[slot] = TransportEntry::default();
        }
        self.used_len = 0;
    }
    fn verify_and_clear(
        &mut self,
        mut calculate: impl FnMut(u32, usize) -> Result<i64, &'static str>,
    ) -> bool {
        let mut matched = true;
        for &slot in &self.used[..self.used_len] {
            let entry = &mut self.entries[slot];
            let result = calculate(entry.owner, entry.region);
            matched &= result == Ok(entry.speed);
            *entry = TransportEntry::default();
        }
        self.used_len = 0;
        matched
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if self.entries.len() < capacity {
            self.entries.resize(capacity, TransportEntry::default());
            self.used.resize(capacity, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<TransportEntry>()
            + self.used.capacity() * size_of::<usize>()
    }
}

// The largest bulk read in the normal-common graph is a 64-member pointer
// chunk. Larger future requests preserve the original verifier via fallback.
const WITNESS_MAX_READ: usize = 64 * 8;

#[derive(Clone, Copy, Default)]
struct WitnessRead {
    address: usize,
    start: usize,
    len: usize,
}

/// Verification-only observations omitted by the exact small-read cache.
/// Never serves acquired data: collisions and bulk reads remain fresh, in
/// their original order. A bounded shortfall disables graph-verifier elision
/// for every affected scope, without changing acquisition outcomes.
struct ReadWitnessJournal {
    entries: Vec<WitnessRead>,
    bytes: Vec<u8>,
    len: usize,
    bytes_len: usize,
    required_entries: usize,
    required_bytes: usize,
    generation: u64,
    uncovered: u64,
    exhausted_counter: bool,
}
impl ReadWitnessJournal {
    fn new(entries: usize, bytes: usize) -> Self {
        Self {
            entries: vec![WitnessRead::default(); entries],
            bytes: vec![0; bytes],
            len: 0,
            bytes_len: 0,
            required_entries: 0,
            required_bytes: 0,
            generation: 1,
            uncovered: 0,
            exhausted_counter: false,
        }
    }
    fn enabled(&self) -> bool {
        !self.entries.is_empty() && !self.bytes.is_empty() && !self.exhausted_counter
    }
    fn miss(&mut self) {
        if let Some(next) = self.uncovered.checked_add(1) {
            self.uncovered = next;
        } else {
            self.exhausted_counter = true;
        }
    }
    fn record(&mut self, address: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.required_entries = self.required_entries.saturating_add(1);
        self.required_bytes = self.required_bytes.saturating_add(bytes.len());
        let end = self.bytes_len.checked_add(bytes.len());
        if bytes.len() > WITNESS_MAX_READ
            || self.len == self.entries.len()
            || end.is_none_or(|end| end > self.bytes.len())
        {
            self.miss();
            return;
        }
        let end = end.unwrap();
        self.bytes[self.bytes_len..end].copy_from_slice(bytes);
        self.entries[self.len] = WitnessRead {
            address,
            start: self.bytes_len,
            len: bytes.len(),
        };
        self.len += 1;
        self.bytes_len = end;
    }
    fn clear(&mut self) {
        self.entries[..self.len].fill(WitnessRead::default());
        self.bytes[..self.bytes_len].fill(0);
        self.len = 0;
        self.bytes_len = 0;
        self.required_entries = 0;
        self.required_bytes = 0;
        self.uncovered = 0;
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            // A generation must never alias a checkpoint from a prior pass.
            self.exhausted_counter = true;
        }
    }
    fn verify_and_clear(&mut self, source: &impl Memory) -> bool {
        let mut matched = true;
        let mut fresh = [0; WITNESS_MAX_READ];
        for entry in &self.entries[..self.len] {
            let copied = source.copy(entry.address, &mut fresh[..entry.len]);
            matched &=
                copied && fresh[..entry.len] == self.bytes[entry.start..entry.start + entry.len];
        }
        self.clear();
        matched
    }
    fn grow(&mut self, entries: usize, bytes: usize) {
        self.clear();
        if entries > self.entries.len() {
            self.entries.resize(entries, WitnessRead::default());
        }
        if bytes > self.bytes.len() {
            self.bytes.resize(bytes, 0);
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<WitnessRead>() + self.bytes.capacity()
    }
}

#[derive(Clone, Copy, Default)]
struct CacheKey {
    address: usize,
    len: usize,
}

/// Callback-local exact reads only. Occupied slots are never evicted: a full
/// four-slot neighborhood is acquired normally, leaving retained observations
/// available for mandatory direct verification at the end of each pass.
struct SmallReadCache {
    // Dense metadata avoids fetching the 32-byte payload on misses/collisions.
    keys: Vec<CacheKey>,
    bytes: Vec<[u8; SMALL_READ_BYTES]>,
    used: Vec<usize>,
    used_len: usize,
    mask: Option<usize>,
    witness: ReadWitnessJournal,
    diagnostics: CacheDiagnostics,
}
impl SmallReadCache {
    fn new(capacity: usize) -> Self {
        Self {
            keys: vec![CacheKey::default(); capacity],
            bytes: vec![[0; SMALL_READ_BYTES]; capacity],
            used: vec![0; capacity],
            used_len: 0,
            mask: capacity
                .is_power_of_two()
                .then_some(capacity.wrapping_sub(1)),
            witness: ReadWitnessJournal::new(0, 0),
            diagnostics: CacheDiagnostics::default(),
        }
    }
    fn with_witness(capacity: usize, entries: usize, bytes: usize) -> Self {
        Self {
            witness: ReadWitnessJournal::new(entries, bytes),
            ..Self::new(capacity)
        }
    }
    fn slot(&self, address: usize, len: usize) -> Option<usize> {
        if self.keys.is_empty() || len == 0 || len > SMALL_READ_BYTES {
            None
        } else {
            let hash = (address >> 3) ^ (address >> 19) ^ len.wrapping_mul(0x9e37_79b9);
            Some(match self.mask {
                Some(mask) => hash & mask,
                None => hash % self.keys.len(),
            })
        }
    }
    fn clear(&mut self) {
        self.diagnostics = CacheDiagnostics::default();
        self.witness.clear();
        for &slot in &self.used[..self.used_len] {
            self.keys[slot] = CacheKey::default();
            self.bytes[slot] = [0; SMALL_READ_BYTES];
        }
        self.used_len = 0;
    }
    fn grow(&mut self, capacity: usize) {
        self.clear();
        if self.keys.len() < capacity {
            self.keys.resize(capacity, CacheKey::default());
            self.bytes.resize(capacity, [0; SMALL_READ_BYTES]);
            self.used.resize(capacity, 0);
            self.mask = capacity
                .is_power_of_two()
                .then_some(capacity.wrapping_sub(1));
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.keys.capacity() * size_of::<CacheKey>()
            + self.bytes.capacity() * size_of::<[u8; SMALL_READ_BYTES]>()
            + self.used.capacity() * size_of::<usize>()
            + self.witness.allocated_bytes()
    }
    fn observe_witness(&mut self) {
        self.diagnostics.witness_entries_peak =
            self.diagnostics.witness_entries_peak.max(self.witness.len);
        self.diagnostics.witness_bytes_peak = self
            .diagnostics
            .witness_bytes_peak
            .max(self.witness.bytes_len);
        self.diagnostics.witness_entries_required = self
            .diagnostics
            .witness_entries_required
            .max(self.witness.required_entries);
        self.diagnostics.witness_bytes_required = self
            .diagnostics
            .witness_bytes_required
            .max(self.witness.required_bytes);
    }
    fn verify_and_clear(&mut self, memory: &impl Memory) -> bool {
        self.observe_witness();
        let mut matched = self.witness.verify_and_clear(memory);
        let mut fresh = [0; SMALL_READ_BYTES];
        for &slot in &self.used[..self.used_len] {
            let key = self.keys[slot];
            // Do not short-circuit after a mismatch: every retained read must
            // be directly checked, even when the enclosing walk already failed.
            let copied = memory.copy(key.address, &mut fresh[..key.len]);
            matched &= copied && fresh[..key.len] == self.bytes[slot][..key.len];
            self.keys[slot] = CacheKey::default();
            self.bytes[slot] = [0; SMALL_READ_BYTES];
        }
        self.used_len = 0;
        matched
    }
}

struct CacheRef<'a, M> {
    source: &'a M,
    cache: &'a RefCell<SmallReadCache>,
    modifiers: Option<&'a RefCell<ModifierSnapshots>>,
    supply: Option<&'a RefCell<SupplyMemo>>,
    air_defense: Option<&'a RefCell<AirDefenseMemo>>,
    primary: Option<&'a RefCell<engine::carmy_leader::PrimaryRootCache>>,
}
impl<M: Memory> Memory for CacheRef<'_, M> {
    fn finish_read_witness(
        &self,
        witness: engine::memory::ReadWitness,
    ) -> Option<Result<(), &'static str>> {
        let mut cache = self.cache.borrow_mut();
        if !cache.witness.enabled()
            || witness.owner != self.cache as *const RefCell<SmallReadCache> as usize
            || witness.generation != cache.witness.generation
        {
            return None;
        }
        let complete = witness.uncovered == cache.witness.uncovered;
        cache.observe_witness();
        // Only uncached collision/bulk observations are scoped. Exact scalar
        // cache entries and typed shared dependencies retain their independent
        // terminal checks; this never evicts or restages them for every army.
        let started = qpc();
        let matched = cache.witness.verify_and_clear(self.source);
        cache.diagnostics.stages.scope_verify.record(started);
        if !matched {
            return Some(Err("division ETA owned dependencies changed or unreadable"));
        }
        if complete {
            cache.diagnostics.witness_skips = cache.diagnostics.witness_skips.saturating_add(1);
            Some(Ok(()))
        } else {
            // The original complete-check path records this fallback once.
            None
        }
    }
    fn movement_primary_root(
        &self,
        base: usize,
        member: usize,
    ) -> Option<Result<usize, &'static str>> {
        let primary = self.primary?;
        Some(measure_stage(
            self.cache,
            |stage| &mut stage.primary_acquire,
            || primary.borrow_mut().resolve(self, base, member),
        ))
    }
    fn read_witness(&self) -> Option<engine::memory::ReadWitness> {
        let cache = self.cache.borrow();
        cache
            .witness
            .enabled()
            .then_some(engine::memory::ReadWitness {
                owner: self.cache as *const RefCell<SmallReadCache> as usize,
                generation: cache.witness.generation,
                uncovered: cache.witness.uncovered,
            })
    }
    fn read_witness_complete(&self, witness: engine::memory::ReadWitness) -> bool {
        let mut cache = self.cache.borrow_mut();
        let complete = cache.witness.enabled()
            && witness.owner == self.cache as *const RefCell<SmallReadCache> as usize
            && witness.generation == cache.witness.generation
            && witness.uncovered == cache.witness.uncovered;
        if complete {
            cache.diagnostics.witness_skips = cache.diagnostics.witness_skips.saturating_add(1);
        } else {
            cache.diagnostics.witness_fallbacks =
                cache.diagnostics.witness_fallbacks.saturating_add(1);
        }
        complete
    }
    fn movement_air_defense_value(
        &self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        self.air_defense?
            .borrow_mut()
            .resolve([defense, denominator, reduction], || {
                engine::csub_unit_definition::movement_air_defense_value(
                    defense,
                    denominator,
                    reduction,
                )
            })
    }
    fn core_supply_access(
        &self,
        base: usize,
        state: usize,
        owner: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        let supply = self.supply?;
        let result = measure_stage(
            self.cache,
            |stage| &mut stage.supply_acquire,
            || {
                supply.borrow_mut().resolve(base, state, owner, area, || {
                    let mut cache = self.cache.borrow_mut();
                    cache.diagnostics.stages.supply_builds =
                        cache.diagnostics.stages.supply_builds.saturating_add(1);
                    drop(cache);
                    engine::ccountry::has_core_supply_access_direct(self, base, state, owner, area)
                })
            },
        );
        if result.is_none() {
            let mut cache = self.cache.borrow_mut();
            cache.diagnostics.stages.supply_fallbacks =
                cache.diagnostics.stages.supply_fallbacks.saturating_add(1);
        }
        result
    }
    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<engine::modifier_accumulator::Value, &'static str>> {
        let modifiers = self.modifiers?;
        let result = measure_stage(
            self.cache,
            |stage| &mut stage.modifiers_acquire,
            || modifiers.borrow_mut().lookup(self.source, address, key),
        );
        if result.is_none() {
            let mut cache = self.cache.borrow_mut();
            cache.diagnostics.stages.modifier_fallbacks = cache
                .diagnostics
                .stages
                .modifier_fallbacks
                .saturating_add(1);
        }
        result
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let slot = {
            let mut cache = self.cache.borrow_mut();
            let Some(mut slot) = cache.slot(address, destination.len()) else {
                if !destination.is_empty() {
                    cache.diagnostics.direct_reads =
                        cache.diagnostics.direct_reads.saturating_add(1);
                }
                drop(cache);
                let copied = self.source.copy(address, destination);
                let mut cache = self.cache.borrow_mut();
                if copied {
                    cache.witness.record(address, destination);
                } else {
                    cache.witness.miss();
                }
                return copied;
            };
            let mut vacant = false;
            // The home hash is unchanged. Probe only adjacent dense metadata,
            // without eviction or another division for non-power-of-two sizes.
            for _ in 0..cache.keys.len().min(4) {
                let key = &cache.keys[slot];
                if key.len == destination.len() && key.address == address {
                    destination.copy_from_slice(&cache.bytes[slot][..key.len]);
                    cache.diagnostics.hits = cache.diagnostics.hits.saturating_add(1);
                    return true;
                }
                if key.len == 0 {
                    // First-empty insertion and no in-pass deletion mean an
                    // exact key cannot occur later in this probe sequence.
                    vacant = true;
                    break;
                }
                slot += 1;
                if slot == cache.keys.len() {
                    slot = 0;
                }
            }
            if !vacant {
                cache.diagnostics.direct_reads = cache.diagnostics.direct_reads.saturating_add(1);
            }
            (slot, vacant)
        };
        if !self.source.copy(address, destination) {
            self.cache.borrow_mut().witness.miss();
            return false;
        }
        if slot.1 {
            let mut cache = self.cache.borrow_mut();
            let key = CacheKey {
                address,
                len: destination.len(),
            };
            let mut bytes = [0; SMALL_READ_BYTES];
            bytes[..destination.len()].copy_from_slice(destination);
            cache.bytes[slot.0] = bytes;
            cache.keys[slot.0] = key;
            let next = cache.used_len;
            cache.used[next] = slot.0;
            cache.used_len += 1;
            cache.diagnostics.retained_misses = cache.diagnostics.retained_misses.saturating_add(1);
        } else {
            self.cache.borrow_mut().witness.record(address, destination);
        }
        true
    }
    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.source.authenticated_countries(base, game_state)
    }
    // No immutable_view: callers must not bypass acquisition or final checks.
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::TableView;
    use replay::rows::TableRows;

    #[test]
    fn conversion_sorts_census_order_and_rejects_duplicate_handles() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 3,
                legs: 0,
                cache_entries: 0,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 0,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        for (row, division) in capture.rows.rows.iter_mut().zip([9, 2, 7]) {
            row.division = division;
        }
        for outcome in [
            Outcome::Rows { len: 3 },
            Outcome::Truncated {
                len: 3,
                unread_rows: 4,
            },
        ] {
            capture.rows.outcome = outcome;
            let [view] = capture.tables();
            let rows = match view {
                TableView::Complete(TableRows::DivisionEta(rows)) => rows,
                TableView::Truncated {
                    rows: TableRows::DivisionEta(rows),
                    missing: 4,
                } => rows,
                other => panic!("unexpected ETA view {other:?}"),
            };
            assert_eq!(
                rows.iter().map(|r| r.division).collect::<Vec<_>>(),
                [2, 7, 9]
            );
        }
        capture.rows.rows[2].division = 2;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 0,
        };
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }
    use crate::sampling::CONSISTENCY_TARGET;
    use std::cell::Cell;

    #[test]
    fn shared_seed_terminal_changes_reject_only_eta_consumed_fields() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0]);
        captured.rows.outcome = Outcome::Rows { len: 1 };
        captured.rows.checked = true;
        captured.seed_masks[0][0] = DivisionSeed::CLASS | DivisionSeed::ORDINARY_ROUTE;
        assert!(captured.reject_changed_seeds(0, &[DivisionSeed::ORGANISATION]));
        assert_eq!(captured.rows.outcome, Outcome::Rows { len: 1 });
        assert!(!captured.reject_changed_seeds(0, &[DivisionSeed::CLASS]));
        assert!(matches!(captured.rows.outcome, Outcome::Unavailable { .. }));
        assert_eq!(captured.rows.mismatches, 1);
    }

    #[test]
    fn eta_source_masks_are_independent_of_private_row_capacity() {
        let mut captured = Captured::empty();
        captured.reserve_sources(3);
        assert_eq!(captured.rows.rows.len(), 0);
        assert_eq!(captured.rows.second.len(), 0);
        assert_eq!(captured.consumed_seed_masks(0).len(), 3);
        assert_eq!(captured.consumed_seed_masks(1).len(), 3);
    }

    #[test]
    fn missing_eta_seed_source_does_not_acquire_geometry() {
        let source = Observed::new();
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0]);
        let missing = Outcome::Unavailable {
            reason: "missing authenticated seeds",
        };
        captured.capture(
            &source,
            0,
            usize::MAX,
            &[true],
            Sources {
                first: &[],
                first_outcome: missing,
                second: &[],
                second_outcome: missing,
            },
        );
        assert_eq!(captured.rows.outcome, missing);
        assert!(source.calls.borrow().is_empty());
    }

    struct Observed {
        bytes: RefCell<[u8; 256]>,
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Cell<Option<usize>>,
    }
    impl Observed {
        fn new() -> Self {
            Self {
                bytes: RefCell::new(std::array::from_fn(|i| i as u8)),
                calls: RefCell::new(Vec::new()),
                fault: Cell::new(None),
            }
        }
        fn accumulator(&self, address: usize, data: usize, values: &[(u32, i64)]) {
            let mut bytes = self.bytes.borrow_mut();
            bytes[address..address + 16].fill(0);
            bytes[address..address + 8].copy_from_slice(&(data as u64).to_le_bytes());
            bytes[address + 8..address + 12].copy_from_slice(&(values.len() as u32).to_le_bytes());
            bytes[address + 12..address + 16].copy_from_slice(&(values.len() as u32).to_le_bytes());
            for (index, (key, raw)) in values.iter().enumerate() {
                let at = data + index * 16;
                bytes[at..at + 16].fill(0);
                bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
                bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
            }
        }
    }
    impl Memory for Observed {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fault.get() == Some(address) {
                destination.fill(255);
                return false;
            }
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            let bytes = self.bytes.borrow();
            let Some(source) = bytes.get(address..end) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }

    #[test]
    fn scoped_dependency_witness_reuses_bounded_storage_without_replaying_graphs() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(0, 1, 8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let allocated = cache.borrow().allocated_bytes();
        let mut previous = None;
        for address in [8, 16, 24, 32] {
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(address, &mut [0; 8]));
            if let Some(stale) = previous {
                assert_eq!(memory.finish_read_witness(stale), None);
                assert_eq!(cache.borrow().witness.len, 1);
            }
            assert_eq!(memory.finish_read_witness(mark), Some(Ok(())));
            assert_eq!(cache.borrow().witness.len, 0);
            previous = Some(mark);
        }
        assert_eq!(
            &*source.calls.borrow(),
            &[
                (8, 8),
                (8, 8),
                (16, 8),
                (16, 8),
                (24, 8),
                (24, 8),
                (32, 8),
                (32, 8)
            ]
        );
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
        assert_eq!(cache.borrow().diagnostics.witness_skips, 4);
        assert_eq!(cache.borrow().diagnostics.witness_entries_peak, 1);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn scoped_dependency_witness_preserves_incomplete_and_failed_observations() {
        for fault in [false, true] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::with_witness(0, 1, 8));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(8, &mut [0; 8]));
            assert!(memory.copy(16, &mut [0; 8]));
            let mut foreign = mark;
            foreign.owner ^= 1;
            assert_eq!(memory.finish_read_witness(foreign), None);
            assert_eq!(cache.borrow().witness.len, 1);
            if fault {
                source.fault.set(Some(8));
            }
            assert_eq!(
                memory.finish_read_witness(mark),
                if fault {
                    Some(Err("division ETA owned dependencies changed or unreadable"))
                } else {
                    None
                }
            );
            assert_eq!(cache.borrow().witness.len, 0);
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(24, &mut [0; 8]));
            source.bytes.borrow_mut()[24] ^= 1;
            assert_eq!(
                memory.finish_read_witness(mark),
                Some(Err("division ETA owned dependencies changed or unreadable"))
            );
            assert_eq!(cache.borrow().witness.len, 0);
        }
    }

    #[test]
    fn primary_root_hook_uses_cache_ref_and_independent_terminal_checks_without_reentry() {
        use engine::{corder_instance as order, corders_group as group};
        const MEMBER: usize = 0x100;
        const GROUP: usize = 0x1000;
        const ROOT: usize = 0x2000;
        const ROOTS: usize = 0x3000;
        const MEMBERS: usize = 0x4000;
        struct Source {
            bytes: RefCell<Vec<u8>>,
            calls: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Source {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, destination.len()));
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let mut bytes = vec![0; 0x5000];
        for (address, value) in [
            (MEMBER + engine::corders_group_member::GROUP, GROUP as u64),
            (GROUP, group::VTABLE_RVA as u64),
            (GROUP + group::HANDLE, group::REFERENCE_TYPE as u64),
            (GROUP + group::PRIMARY_ORDER_DATA, ROOTS as u64),
            (GROUP + group::PRIMARY_ORDER_DATA + 8, (1_u64 << 32) | 1),
            (ROOTS, ROOT as u64),
            (ROOT, order::VTABLE_RVA as u64),
            (ROOT + order::OWNER, GROUP as u64),
            (ROOT + order::ASSIGNED_MEMBER_DATA, MEMBERS as u64),
            (ROOT + order::ASSIGNED_MEMBER_DATA + 8, (1_u64 << 32) | 1),
            (MEMBERS, MEMBER as u64),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[MEMBER + 0x28] = 1;
        let source = Source {
            bytes: RefCell::new(bytes),
            calls: RefCell::new(Vec::new()),
        };
        let cache = RefCell::new(SmallReadCache::with_witness(128, 8, 512));
        let primary = RefCell::new(engine::carmy_leader::PrimaryRootCache::new(8, 4));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: Some(&primary),
        };
        let allocated = primary.borrow().allocated_bytes() + cache.borrow().allocated_bytes();
        let witness = memory.read_witness().unwrap();
        assert_eq!(memory.movement_primary_root(0, MEMBER), Some(Ok(ROOT)));
        assert_eq!(memory.movement_primary_root(0, MEMBER), Some(Ok(ROOT)));
        assert!(memory.read_witness_complete(witness));
        assert_eq!(
            source
                .calls
                .borrow()
                .iter()
                .filter(|(address, _)| *address == MEMBERS)
                .count(),
            1
        );
        source.bytes.borrow_mut()[MEMBERS..MEMBERS + 8].fill(0);
        assert!(!primary.borrow_mut().verify_and_clear(&source));
        let before = source.calls.borrow().len();
        // A failed typed verifier cannot prevent the scalar verifier from
        // checking and releasing its overlapping retained observations.
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert!(source.calls.borrow().len() > before);
        assert_eq!(cache.borrow().used_len, 0);
        assert_eq!(memory.movement_primary_root(0, MEMBER), Some(Ok(0)));
        assert!(primary.borrow_mut().verify_and_clear(&source));
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            primary.borrow().allocated_bytes() + cache.borrow().allocated_bytes(),
            allocated
        );
    }

    #[test]
    fn witness_keeps_collision_and_bulk_reads_fresh_and_checks_every_observation() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(1, 8, 256));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        assert!(memory.copy(8, &mut [0; 8]));
        assert!(memory.copy(8, &mut [0; 8]));
        let mut bytes = [0; 8];
        assert!(memory.copy(16, &mut bytes));
        assert_eq!(bytes[0], 16);
        source.bytes.borrow_mut()[16] = 99;
        assert!(memory.copy(16, &mut bytes));
        assert_eq!(
            bytes[0], 99,
            "journal must never serve cached collision data"
        );
        assert!(memory.copy(64, &mut [0; 64]));
        assert_eq!(
            &*source.calls.borrow(),
            &[(8, 8), (16, 8), (16, 8), (64, 64)]
        );
        assert!(memory.read_witness_complete(mark));
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        let diagnostics = cache.borrow().diagnostics;
        assert_eq!(diagnostics.hits, 1);
        assert_eq!(diagnostics.retained_misses, 1);
        assert_eq!(diagnostics.direct_reads, 3);
        assert_eq!(diagnostics.witness_skips, 1);
        assert_eq!(diagnostics.witness_entries_peak, 3);
        assert_eq!(diagnostics.witness_bytes_peak, 80);
        assert_eq!(diagnostics.witness_entries_required, 3);
        assert_eq!(diagnostics.witness_bytes_required, 80);
        assert!(!memory.read_witness_complete(mark));
        assert_eq!(cache.borrow().witness.len, 0);
        assert!(cache.borrow().witness.bytes.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn witness_terminal_fault_does_not_skip_later_journal_or_small_cache_reads() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(1, 8, 256));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        for address in [8, 16, 24, 32] {
            assert!(memory.copy(address, &mut [0; 8]));
        }
        assert!(memory.read_witness_complete(mark));
        source.calls.borrow_mut().clear();
        source.fault.set(Some(16));
        source.bytes.borrow_mut()[24] ^= 1;
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            &*source.calls.borrow(),
            &[(16, 8), (24, 8), (32, 8), (8, 8)]
        );
        assert_eq!(cache.borrow().witness.len, 0);
        assert_eq!(cache.borrow().used_len, 0);
    }

    #[test]
    fn witness_capacity_and_failed_initial_reads_force_original_verifier_fallback() {
        for (entries, bytes) in [(1, 128), (8, 8)] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::with_witness(0, entries, bytes));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let mark = memory.read_witness().unwrap();
            assert!(memory.copy(16, &mut [0; 8]));
            assert!(memory.read_witness_complete(mark));
            assert!(memory.copy(32, &mut [0; 8]));
            assert!(!memory.read_witness_complete(mark));
            assert_eq!(cache.borrow().witness.len, 1);
            assert!(cache.borrow_mut().verify_and_clear(&source));
            let diagnostics = cache.borrow().diagnostics;
            assert_eq!(diagnostics.witness_entries_required, 2);
            assert_eq!(diagnostics.witness_bytes_required, 16);
            assert_eq!(diagnostics.witness_fallbacks, 1);
            assert!(!memory.read_witness_complete(mark));
            let next = memory.read_witness().unwrap();
            source.fault.set(Some(64));
            assert!(!memory.copy(64, &mut [0; 8]));
            assert!(!memory.read_witness_complete(next));
            assert_eq!(
                cache.borrow().witness.len,
                0,
                "failed partial bytes must not be retained"
            );
            assert!(cache.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn witness_maximum_span_and_disabled_or_foreign_checkpoints_fail_closed() {
        let bytes = [7; WITNESS_MAX_READ + 1];
        let source = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let cache = RefCell::new(SmallReadCache::with_witness(0, 4, 2 * bytes.len()));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        assert!(memory.copy(0, &mut [0; WITNESS_MAX_READ]));
        assert!(memory.read_witness_complete(mark));
        assert!(memory.copy(0, &mut [0; WITNESS_MAX_READ + 1]));
        assert!(!memory.read_witness_complete(mark));
        assert_eq!(cache.borrow().witness.len, 1);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        let other_cache = RefCell::new(SmallReadCache::with_witness(0, 4, 2 * bytes.len()));
        let other = CacheRef {
            source: &source,
            cache: &other_cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(!other.read_witness_complete(memory.read_witness().unwrap()));
        let disabled_cache = RefCell::new(SmallReadCache::new(4));
        let disabled = CacheRef {
            source: &source,
            cache: &disabled_cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(disabled.read_witness().is_none());
        assert!(!disabled.read_witness_complete(mark));
        cache.borrow_mut().witness.generation = u64::MAX;
        cache.borrow_mut().clear();
        assert!(memory.read_witness().is_none());
    }

    #[test]
    fn witness_does_not_replace_independent_modifier_snapshot_verification() {
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, 10), (5, 20)]);
        let cache = RefCell::new(SmallReadCache::with_witness(1, 8, 256));
        let modifiers = RefCell::new(ModifierSnapshots::new(8, 128));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: Some(&modifiers),
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mark = memory.read_witness().unwrap();
        assert_eq!(
            engine::modifier_accumulator::lookup_at(&memory, 16, 2)
                .unwrap()
                .raw,
            10
        );
        assert!(memory.read_witness_complete(mark));
        source.bytes.borrow_mut()[88] ^= 1; // Unselected key5 value still belongs to the snapshot.
        assert!(!modifiers.borrow_mut().verify_and_clear(&source));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn witness_growth_accounting_and_independent_pass_diagnostics_are_bounded() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(1, 1, 8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let allocated = cache.borrow().allocated_bytes();
        for _ in 0..2 {
            assert!(memory.copy(8, &mut [0; 8]));
            assert!(memory.copy(16, &mut [0; 8]));
            assert!(cache.borrow_mut().verify_and_clear(&source));
        }
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
        assert_eq!(cache.borrow().diagnostics.retained_misses, 2);
        assert_eq!(cache.borrow().diagnostics.direct_reads, 2);
        assert_eq!(cache.borrow().diagnostics.witness_entries_peak, 1);
        assert_eq!(cache.borrow().diagnostics.witness_bytes_peak, 8);
        cache.borrow_mut().clear();
        assert_eq!(cache.borrow().diagnostics, CacheDiagnostics::default());
        cache.borrow_mut().witness.grow(4, 128);
        assert!(cache.borrow().allocated_bytes() >= allocated + 3 * size_of::<WitnessRead>() + 120);
        let mark = memory.read_witness().unwrap();
        assert!(memory.copy(32, &mut [0; 64]));
        assert!(memory.read_witness_complete(mark));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_batches_all_entries_and_reuses_raw_presence_only() {
        use engine::modifier_accumulator::{Value, lookup_at};
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, i64::MIN), (5, 0), (9, i64::MAX)]);
        let cache = RefCell::new(SmallReadCache::new(1));
        let modifiers = RefCell::new(ModifierSnapshots::new(8, 128));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: Some(&modifiers),
            supply: None,
            air_defense: None,
            primary: None,
        };
        for _ in 0..3 {
            assert_eq!(
                lookup_at(&memory, 16, 2),
                Ok(Value {
                    raw: i64::MIN,
                    present: true
                })
            );
            assert_eq!(
                lookup_at(&memory, 16, 5),
                Ok(Value {
                    raw: 0,
                    present: true
                })
            );
            assert_eq!(
                lookup_at(&memory, 16, 9),
                Ok(Value {
                    raw: i64::MAX,
                    present: true
                })
            );
            assert_eq!(lookup_at(&memory, 16, 6), Ok(Value::default()));
        }
        assert_eq!(&*source.calls.borrow(), &[(16, 16), (64, 48), (16, 16)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(modifiers.borrow_mut().verify_and_clear(&source));
        assert_eq!(&source.calls.borrow()[3..], &[(16, 16), (64, 48), (16, 16)]);
        assert_eq!(modifiers.borrow().used_len, 0);
        assert_eq!(modifiers.borrow().bytes_len, 0);
        assert!(
            modifiers
                .borrow()
                .entries
                .iter()
                .all(|entry| !entry.occupied && entry.address == 0 && entry.header == [0; 16])
        );
    }

    #[test]
    fn modifier_snapshot_verifies_unused_keys_values_padding_and_all_header_fields() {
        for mutation in [16, 24, 28, 96, 100, 104] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 0), (9, 13)]);
            let mut snapshots = ModifierSnapshots::new(8, 128);
            let first = snapshots.lookup(&source, 16, 2).unwrap().unwrap();
            source.bytes.borrow_mut()[mutation] ^= 1;
            assert_eq!(snapshots.lookup(&source, 16, 2), Some(Ok(first)));
            assert!(
                !snapshots.verify_and_clear(&source),
                "missed mutation at {mutation}"
            );
            assert_eq!(snapshots.used_len, 0);
            assert_eq!(snapshots.bytes_len, 0);
        }
        let source = Observed::new();
        source.accumulator(16, 64, &[(2, 7)]);
        let mut snapshots = ModifierSnapshots::new(8, 128);
        assert_eq!(snapshots.lookup(&source, 16, 2).unwrap().unwrap().raw, 7);
        source.bytes.borrow_mut()[72..80].copy_from_slice(&11_i64.to_le_bytes());
        assert!(!snapshots.verify_and_clear(&source));
        assert_eq!(snapshots.lookup(&source, 16, 2).unwrap().unwrap().raw, 11);
        assert!(snapshots.verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_faults_check_every_retained_array_and_clear_failed_passes() {
        for fault in [16, 64, 32, 128] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 11)]);
            source.accumulator(32, 128, &[(3, 17)]);
            let mut snapshots = ModifierSnapshots::new(8, 128);
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
            assert!(snapshots.lookup(&source, 32, 3).unwrap().is_ok());
            source.calls.borrow_mut().clear();
            source.fault.set(Some(fault));
            assert!(!snapshots.verify_and_clear(&source));
            assert_eq!(
                &*source.calls.borrow(),
                &[(16, 16), (64, 32), (16, 16), (32, 16), (128, 16), (32, 16)]
            );
            assert_eq!(snapshots.used_len, 0);
            source.fault.set(None);
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
            // A later failed lookup must not discard prior retained inputs:
            // capture finalizes them even when the walk is already unavailable.
            source.fault.set(Some(32));
            assert!(snapshots.lookup(&source, 32, 3).unwrap().is_err());
            source.bytes.borrow_mut()[72] ^= 1;
            assert!(!snapshots.verify_and_clear(&source));
            assert_eq!(snapshots.used_len, 0);
        }
    }

    #[test]
    fn modifier_snapshot_rejects_unsorted_suffix_and_failed_initial_reads() {
        let source = Observed::new();
        let mut snapshots = ModifierSnapshots::new(8, 128);
        for suffix in [0, 1, 5] {
            source.accumulator(16, 64, &[(2, 7), (5, 11), (suffix, 13)]);
            assert_eq!(
                snapshots.lookup(&source, 16, 2),
                Some(Err("modifier accumulator keys not positive and increasing"))
            );
            assert_eq!(snapshots.used_len, 0);
            assert_eq!(snapshots.bytes_len, 0);
        }
        source.accumulator(16, 64, &[(2, 7)]);
        for fault in [16, 64] {
            source.fault.set(Some(fault));
            assert!(snapshots.lookup(&source, 16, 2).unwrap().is_err());
            assert_eq!(snapshots.used_len, 0);
        }
        source.fault.set(None);
        assert!(snapshots.lookup(&source, 16, 2).unwrap().is_ok());
        assert!(snapshots.verify_and_clear(&source));
    }

    #[test]
    fn modifier_snapshot_collisions_and_capacity_fallback_keep_full_original_validation() {
        use engine::modifier_accumulator::lookup_at;
        for (slots, arena) in [(0, 0), (1, 128), (8, 32)] {
            let source = Observed::new();
            source.accumulator(16, 64, &[(2, 7), (5, 11)]);
            source.accumulator(32, 128, &[(3, 17), (7, 19)]);
            let cache = RefCell::new(SmallReadCache::new(0));
            let modifiers = RefCell::new(ModifierSnapshots::new(slots, arena));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: Some(&modifiers),
                supply: None,
                air_defense: None,
                primary: None,
            };
            assert_eq!(lookup_at(&memory, 16, 2).unwrap().raw, 7);
            assert_eq!(lookup_at(&memory, 32, 3).unwrap().raw, 17);
            source.bytes.borrow_mut()[136..144].copy_from_slice(&23_i64.to_le_bytes());
            assert_eq!(lookup_at(&memory, 32, 3).unwrap().raw, 23);
            source.bytes.borrow_mut()[144..148].copy_from_slice(&3_u32.to_le_bytes());
            assert!(
                lookup_at(&memory, 32, 3).is_err(),
                "fallback skipped malformed suffix"
            );
            assert_eq!(modifiers.borrow().used_len, usize::from(slots != 0));
            assert!(modifiers.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn modifier_snapshot_empty_arrays_reset_growth_and_accounting_are_bounded() {
        let source = Observed::new();
        source.accumulator(16, 64, &[]);
        let mut snapshots = ModifierSnapshots::new(0, 0);
        assert!(snapshots.lookup(&source, 16, 1).is_none());
        assert_eq!(snapshots.allocated_bytes(), 0);
        snapshots.grow(8, 128);
        let bytes = snapshots.allocated_bytes();
        let pointers = (
            snapshots.entries.as_ptr(),
            snapshots.used.as_ptr(),
            snapshots.bytes.as_ptr(),
        );
        assert!(bytes >= 8 * (size_of::<ModifierSnapshot>() + size_of::<usize>()) + 128);
        for _ in 0..3 {
            assert_eq!(
                snapshots.lookup(&source, 16, 1),
                Some(Ok(engine::modifier_accumulator::Value::default()))
            );
            assert!(snapshots.verify_and_clear(&source));
            assert_eq!(
                (
                    snapshots.entries.as_ptr(),
                    snapshots.used.as_ptr(),
                    snapshots.bytes.as_ptr()
                ),
                pointers
            );
            assert_eq!(snapshots.allocated_bytes(), bytes);
        }
        assert!(
            source
                .calls
                .borrow()
                .iter()
                .all(|&(address, len)| (address, len) == (16, 16))
        );
        assert!(snapshots.lookup(&source, 16, 1).unwrap().is_ok());
        snapshots.clear();
        assert_eq!(snapshots.used_len, 0);
        assert_eq!(snapshots.bytes_len, 0);
        assert!(
            snapshots
                .entries
                .iter()
                .all(|entry| !entry.occupied && entry.address == 0)
        );
    }

    #[test]
    fn modifier_snapshot_initial_header_reread_rejects_changes_including_empty_arrays() {
        struct Changed {
            source: Observed,
            reads: Cell<usize>,
            changed_byte: usize,
        }
        impl Memory for Changed {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if !self.source.copy(address, destination) {
                    return false;
                }
                if address == 16 {
                    self.reads.set(self.reads.get() + 1);
                    if self.reads.get() > 1 {
                        destination[self.changed_byte] ^= 1;
                    }
                }
                true
            }
        }
        for count in [0, 1] {
            for changed_byte in [0, 8, 12] {
                let source = Changed {
                    source: Observed::new(),
                    reads: Cell::new(0),
                    changed_byte,
                };
                source.source.accumulator(16, 64, &[(2, 7)][..count]);
                let mut snapshots = ModifierSnapshots::new(8, 128);
                assert_eq!(
                    snapshots.lookup(&source, 16, 2),
                    Some(Err("modifier accumulator header changed"))
                );
                assert_eq!(snapshots.used_len, 0);
                assert_eq!(snapshots.bytes_len, 0);
            }
        }
    }

    #[test]
    fn modifier_snapshot_direct_verification_reaches_the_final_partial_batch() {
        struct Wide {
            bytes: RefCell<Vec<u8>>,
            calls: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Wide {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, destination.len()));
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let source = Wide {
            bytes: RefCell::new(vec![0; 64 + 65 * 16]),
            calls: RefCell::new(Vec::new()),
        };
        {
            let mut bytes = source.bytes.borrow_mut();
            bytes[16..24].copy_from_slice(&64_u64.to_le_bytes());
            bytes[24..28].copy_from_slice(&65_u32.to_le_bytes());
            bytes[28..32].copy_from_slice(&65_u32.to_le_bytes());
            for index in 0..65 {
                let at = 64 + index * 16;
                bytes[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            }
        }
        let mut snapshots = ModifierSnapshots::new(8, 65 * 16);
        assert!(snapshots.lookup(&source, 16, 1).unwrap().is_ok());
        source.bytes.borrow_mut()[64 + 64 * 16 + 8] = 1;
        source.calls.borrow_mut().clear();
        assert!(!snapshots.verify_and_clear(&source));
        assert_eq!(
            &*source.calls.borrow(),
            &[(16, 16), (64, 1024), (1088, 16), (16, 16)]
        );
        assert_eq!(snapshots.used_len, 0);
    }

    #[test]
    fn transport_memo_hits_are_exact_and_end_at_each_independent_pass() {
        let mut memo = TransportMemo::new(8);
        let calls = Cell::new(0);
        for _ in 0..4 {
            assert_eq!(
                memo.resolve(u32::MAX, 0x1000, || {
                    calls.set(calls.get() + 1);
                    Ok(i64::MIN)
                }),
                Ok(i64::MIN)
            );
        }
        assert_eq!(calls.get(), 1);
        assert!(memo.verify_and_clear(|owner, region| {
            assert_eq!((owner, region), (u32::MAX, 0x1000));
            calls.set(calls.get() + 1);
            Ok(i64::MIN)
        }));
        assert_eq!(calls.get(), 2);
        assert_eq!(memo.used_len, 0);
        assert!(
            memo.entries
                .iter()
                .all(|e| !e.occupied && e.region == 0 && e.owner == 0 && e.speed == 0)
        );
        assert_eq!(memo.resolve(u32::MAX, 0x1000, || Ok(17)), Ok(17));
        assert!(memo.verify_and_clear(|_, _| Ok(17)));
    }

    #[test]
    fn supply_memo_shares_true_and_false_only_until_independent_pass_verification() {
        for value in [false, true] {
            let mut memo = SupplyMemo::new(8);
            let calls = Cell::new(0);
            for _ in 0..4 {
                assert_eq!(
                    memo.resolve(0, 16, u32::MAX, 0, || {
                        calls.set(calls.get() + 1);
                        Ok(value)
                    }),
                    Some(Ok(value))
                );
            }
            assert_eq!(calls.get(), 1);
            assert!(memo.verify_and_clear(|base, state, owner, area| {
                assert_eq!((base, state, owner, area), (0, 16, u32::MAX, 0));
                calls.set(calls.get() + 1);
                Ok(value)
            }));
            assert_eq!(calls.get(), 2);
            assert_eq!(memo.used_len, 0);
            assert!(memo.entries.iter().all(|e| !e.occupied
                && e.base == 0
                && e.state == 0
                && e.area == 0
                && e.owner == 0
                && !e.accessible));
            assert_eq!(
                memo.resolve(0, 16, u32::MAX, 0, || Ok(!value)),
                Some(Ok(!value))
            );
            assert!(memo.verify_and_clear(|_, _, _, _| Ok(!value)));
        }
    }

    #[test]
    fn pure_air_memo_matches_native_function_for_signed_and_exceptional_arguments() {
        let mut memo = AirDefenseMemo::new(128);
        for arguments in [
            [0, 100_000, 50_000],
            [1, 100_000, 50_000],
            [50_000, 100_000, 50_000],
            [-50_000, 100_000, 50_000],
            [100_000, 100_000, 50_000],
            [150_000, 100_000, 50_000],
            [1_000_000, 100_000, 50_000],
            [100_000, -100_000, 50_000],
            [100_000, 0, 50_000],
            [100_000, 100_000, -50_000],
            [100_000, i64::MIN, i64::MAX],
            [i64::MIN, 100_000, 50_000],
            [i64::MAX, 100_000, 50_000],
        ] {
            let calculate = || {
                engine::csub_unit_definition::movement_air_defense_value(
                    arguments[0],
                    arguments[1],
                    arguments[2],
                )
            };
            let expected = calculate();
            for _ in 0..2 {
                assert_eq!(
                    memo.resolve(arguments, calculate).unwrap_or_else(calculate),
                    expected
                );
            }
        }
    }

    #[test]
    fn pure_air_memo_preserves_all_three_raw_keys_without_eviction_or_error_success() {
        let mut memo = AirDefenseMemo::new(1);
        let key = [i64::MIN, -1, i64::MAX];
        assert_eq!(
            memo.resolve(key, || Err("native calculation traps")),
            Some(Err("native calculation traps"))
        );
        assert_eq!(memo.used_len, 0);
        let calls = Cell::new(0);
        for _ in 0..3 {
            assert_eq!(
                memo.resolve(key, || {
                    calls.set(calls.get() + 1);
                    Ok(0)
                }),
                Some(Ok(0))
            );
        }
        assert_eq!(calls.get(), 1);
        for changed in [
            [i64::MIN + 1, -1, i64::MAX],
            [i64::MIN, 1, i64::MAX],
            [i64::MIN, -1, i64::MIN],
        ] {
            assert_eq!(
                memo.resolve(changed, || panic!("collision uses caller's exact fallback")),
                None
            );
        }
        assert_eq!(
            memo.resolve(key, || panic!("retained key must survive collisions")),
            Some(Ok(0))
        );
        assert_eq!(memo.used_len, 1);
        let mut empty = AirDefenseMemo::new(0);
        assert_eq!(
            empty.resolve(key, || panic!("empty cache falls back")),
            None
        );
        assert_eq!(empty.allocated_bytes(), 0);
    }

    #[test]
    fn pure_air_capability_fallback_errors_and_changed_arguments_are_exact() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let air = RefCell::new(AirDefenseMemo::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: Some(&air),
            primary: None,
        };
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, 50_000),
            Some(Ok(25_000))
        );
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, 50_000),
            Some(Ok(25_000))
        );
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, -50_000),
            None
        );
        let expected =
            engine::csub_unit_definition::movement_air_defense_value(100_000, 100_000, -50_000);
        assert_eq!(expected, Ok(-25_000));
        // The hook is pure and must never access source bytes itself.
        assert!(source.calls.borrow().is_empty());
        air.borrow_mut().clear();
        assert_eq!(
            memory.movement_air_defense_value(100_000, 100_000, -50_000),
            Some(expected)
        );
    }

    #[test]
    fn pure_air_memo_reset_growth_and_pool_accounting_are_bounded() {
        let mut memo = AirDefenseMemo::new(0);
        memo.grow(8);
        let bytes = memo.allocated_bytes();
        let pointers = (memo.entries.as_ptr(), memo.used.as_ptr());
        assert!(bytes >= 8 * (size_of::<AirDefenseEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve([1, 2, 3], || Ok(4)), Some(Ok(4)));
            memo.clear();
            assert_eq!(memo.used_len, 0);
            assert!(
                memo.entries
                    .iter()
                    .all(|e| !e.occupied && e.arguments == [0; 3] && e.value == 0)
            );
            assert_eq!((memo.entries.as_ptr(), memo.used.as_ptr()), pointers);
            assert_eq!(memo.allocated_bytes(), bytes);
        }
    }

    #[test]
    fn supply_memo_full_keys_collisions_capacity_and_faults_never_alias_or_evict() {
        let mut memo = SupplyMemo::new(1);
        assert_eq!(
            memo.resolve(8, 16, 1, 32, || Err("first fault")),
            Some(Err("first fault"))
        );
        assert_eq!(memo.used_len, 0);
        assert_eq!(memo.resolve(8, 16, 1, 32, || Ok(true)), Some(Ok(true)));
        for (base, state, owner, area) in [
            (9, 16, 1, 32),
            (8, 17, 1, 32),
            (8, 16, 2, 32),
            (8, 16, 1, 33),
        ] {
            assert_eq!(
                memo.resolve(base, state, owner, area, || panic!(
                    "collision must use ordinary reader"
                )),
                None
            );
        }
        assert_eq!(
            memo.resolve(8, 16, 1, 32, || panic!("expected retained hit")),
            Some(Ok(true))
        );
        assert_eq!(memo.used_len, 1);
        assert!(memo.verify_and_clear(|base, state, owner, area| {
            assert_eq!((base, state, owner, area), (8, 16, 1, 32));
            Ok(true)
        }));
        let mut empty = SupplyMemo::new(0);
        assert_eq!(
            empty.resolve(0, 0, 0, 0, || panic!("zero capacity must fall back")),
            None
        );
        assert_eq!(empty.used_len, 0);
    }

    #[test]
    fn supply_memo_verifies_all_retained_keys_after_mutation_fault_or_failed_miss() {
        for fault in [false, true] {
            let mut memo = SupplyMemo::new(8);
            assert_eq!(memo.resolve(0, 0, 1, 8, || Ok(true)), Some(Ok(true)));
            assert_eq!(memo.resolve(0, 0, 1, 16, || Ok(false)), Some(Ok(false)));
            assert_eq!(
                memo.resolve(0, 0, 1, 24, || Err("walk failed")),
                Some(Err("walk failed"))
            );
            let mut calls = 0;
            assert!(!memo.verify_and_clear(|_, _, _, area| {
                calls += 1;
                if area == 8 {
                    if fault {
                        Err("verify fault")
                    } else {
                        Ok(false)
                    }
                } else {
                    assert_eq!(area, 16);
                    Ok(false)
                }
            }));
            assert_eq!(calls, 2);
            assert_eq!(memo.used_len, 0);
            assert!(
                memo.entries
                    .iter()
                    .all(|entry| !entry.occupied && entry.area == 0)
            );
        }
    }

    #[test]
    fn supply_memo_direct_verification_covers_small_read_collisions() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut memo = SupplyMemo::new(1);
        assert_eq!(
            memo.resolve(0, 0, 1, 8, || {
                let a = engine::read::<u8>(&memory, 32).ok_or("a")?;
                let b = engine::read::<u8>(&memory, 64).ok_or("b")?;
                Ok(a < b)
            }),
            Some(Ok(true))
        );
        source.bytes.borrow_mut()[64] = 0;
        assert_eq!(
            memo.resolve(0, 0, 1, 8, || panic!("memo hit")),
            Some(Ok(true))
        );
        assert!(!memo.verify_and_clear(|_, _, _, _| {
            let a = engine::read::<u8>(&source, 32).ok_or("a")?;
            let b = engine::read::<u8>(&source, 64).ok_or("b")?;
            Ok(a < b)
        }));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn supply_memo_growth_reset_and_accounting_remain_off_callback_and_bounded() {
        let mut memo = SupplyMemo::new(0);
        assert_eq!(memo.allocated_bytes(), 0);
        memo.grow(8);
        let bytes = memo.allocated_bytes();
        let pointers = (memo.entries.as_ptr(), memo.used.as_ptr());
        assert!(bytes >= 8 * (size_of::<SupplyEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve(0, 0, 1, 8, || Ok(false)), Some(Ok(false)));
            assert!(memo.verify_and_clear(|_, _, _, _| Ok(false)));
            assert_eq!((memo.entries.as_ptr(), memo.used.as_ptr()), pointers);
            assert_eq!(memo.allocated_bytes(), bytes);
        }
        assert_eq!(memo.resolve(0, 0, 1, 8, || Ok(true)), Some(Ok(true)));
        memo.clear();
        assert_eq!(memo.used_len, 0);
        assert!(memo.entries.iter().all(|e| !e.occupied && e.area == 0));
        assert_eq!(memo.allocated_bytes(), bytes);
    }

    #[test]
    fn supply_capability_runs_real_native_reader_and_final_check_bypasses_its_hit() {
        struct Source {
            bytes: RefCell<Vec<u8>>,
            calls: Cell<usize>,
        }
        impl Memory for Source {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                engine::memory::SliceMemory {
                    base: 0,
                    bytes: &self.bytes.borrow(),
                }
                .copy(address, destination)
            }
        }
        let source = Source {
            bytes: RefCell::new(vec![0; 0x6000]),
            calls: Cell::new(0),
        };
        {
            let mut bytes = source.bytes.borrow_mut();
            for (header, data) in [
                (engine::cgame_state::COUNTRIES, 0x2000_u64),
                (engine::cgame_state::COUNTRY_LINKS, 0x2100),
            ] {
                let at = 0x1000 + header;
                bytes[at..at + 8].copy_from_slice(&data.to_le_bytes());
                bytes[at + 8..at + 12].copy_from_slice(&2_u32.to_le_bytes());
                bytes[at + 12..at + 16].copy_from_slice(&2_u32.to_le_bytes());
            }
            bytes[0x2008..0x2010].copy_from_slice(&0x3000_u64.to_le_bytes());
            bytes[0x2104..0x2108].copy_from_slice(&1_u32.to_le_bytes());
            bytes[0x3000..0x3008]
                .copy_from_slice(&(engine::ccountry::VTABLE_RVA as u64).to_le_bytes());
            let id = 0x3000 + engine::ccountry::ID;
            bytes[id..id + 4].copy_from_slice(&1_u32.to_le_bytes());
        }
        let cache = RefCell::new(SmallReadCache::new(0));
        let supply = RefCell::new(SupplyMemo::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: Some(&supply),
            air_defense: None,
            primary: None,
        };
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        let calls = source.calls.get();
        assert!(calls > 0);
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert_eq!(source.calls.get(), calls);
        let capital = 0x3000 + engine::ccountry::CAPITAL;
        source.bytes.borrow_mut()[capital..capital + 4].copy_from_slice(&1_i32.to_le_bytes());
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert!(
            !supply
                .borrow_mut()
                .verify_and_clear(|base, state, owner, area| {
                    engine::ccountry::has_core_supply_access_direct(
                        &source, base, state, owner, area,
                    )
                })
        );
        assert_eq!(supply.borrow().used_len, 0);
        assert!(source.calls.get() > calls);
        source.bytes.borrow_mut()[capital..capital + 4].fill(0);
        assert_eq!(
            engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert!(
            supply
                .borrow_mut()
                .verify_and_clear(|base, state, owner, area| {
                    engine::ccountry::has_core_supply_access_direct(
                        &source, base, state, owner, area,
                    )
                })
        );
        for _ in 0..2 {
            assert!(engine::ccountry::has_core_supply_access(&memory, 0, 0x1000, 0, 0).is_err());
            assert_eq!(supply.borrow().used_len, 0);
        }
        // Capacity exhaustion preserves the original guarded reader too.
        let empty = RefCell::new(SupplyMemo::new(0));
        let fallback = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: Some(&empty),
            air_defense: None,
            primary: None,
        };
        assert_eq!(
            engine::ccountry::has_core_supply_access(&fallback, 0, 0x1000, 1, 0),
            Ok(true)
        );
        assert_eq!(empty.borrow().used_len, 0);
    }

    #[test]
    fn transport_collisions_and_failed_misses_never_replace_a_retained_key() {
        let mut memo = TransportMemo::new(1);
        assert_eq!(
            memo.resolve(1, 8, || Err("initial fault")),
            Err("initial fault")
        );
        assert_eq!(memo.used_len, 0);
        assert_eq!(memo.resolve(1, 8, || Ok(11)), Ok(11));
        assert_eq!(memo.resolve(2, 8, || Ok(22)), Ok(22));
        assert_eq!(
            memo.resolve(1, 16, || Err("collision fault")),
            Err("collision fault")
        );
        assert_eq!(memo.resolve(2, 8, || Ok(23)), Ok(23));
        assert_eq!(
            memo.resolve(1, 8, || panic!("retained key recalculated early")),
            Ok(11)
        );
        assert_eq!(memo.used_len, 1);
        assert!(memo.verify_and_clear(|owner, region| {
            assert_eq!((owner, region), (1, 8));
            Ok(11)
        }));
    }

    #[test]
    fn transport_verification_checks_all_results_after_change_or_fault_and_clears_keys() {
        for fault in [false, true] {
            let mut memo = TransportMemo::new(8);
            assert_eq!(memo.resolve(1, 8, || Ok(11)), Ok(11));
            assert_eq!(memo.resolve(1, 16, || Ok(22)), Ok(22));
            let mut calls = 0;
            assert!(!memo.verify_and_clear(|_, region| {
                calls += 1;
                if region == 8 {
                    if fault { Err("verify fault") } else { Ok(12) }
                } else {
                    Ok(22)
                }
            }));
            assert_eq!(calls, 2);
            assert_eq!(memo.used_len, 0);
            assert!(memo.entries.iter().all(|e| !e.occupied && e.region == 0));
        }
    }

    #[test]
    fn transport_direct_verification_covers_operands_not_retained_by_small_read_cache() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let cached = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        // The first operand occupies the sole read-cache slot. The derived
        // speed's second operand collides, so small-read verification alone
        // cannot discover its later change.
        let mut memo = TransportMemo::new(1);
        assert_eq!(
            memo.resolve(7, 8, || {
                let a = engine::read::<u8>(&cached, 32).ok_or("a")?;
                let b = engine::read::<u8>(&cached, 64).ok_or("b")?;
                Ok(i64::from(a) + i64::from(b))
            }),
            Ok(96)
        );
        source.bytes.borrow_mut()[64] = 65;
        assert_eq!(memo.resolve(7, 8, || panic!("expected memo hit")), Ok(96));
        assert!(!memo.verify_and_clear(|_, _| {
            let a = engine::read::<u8>(&source, 32).ok_or("a")?;
            let b = engine::read::<u8>(&source, 64).ok_or("b")?;
            Ok(i64::from(a) + i64::from(b))
        }));
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn transport_capacity_reset_and_accounting_do_not_allocate_during_use() {
        let mut memo = TransportMemo::new(0);
        assert_eq!(memo.resolve(0, 0, || Ok(9)), Ok(9));
        assert_eq!(memo.allocated_bytes(), 0);
        memo.grow(2);
        let bytes = memo.allocated_bytes();
        let entry_pointer = memo.entries.as_ptr();
        let used_pointer = memo.used.as_ptr();
        assert!(bytes >= 2 * (size_of::<TransportEntry>() + size_of::<usize>()));
        for _ in 0..3 {
            assert_eq!(memo.resolve(0, 0, || Ok(9)), Ok(9));
            assert!(memo.verify_and_clear(|_, _| Ok(9)));
            assert_eq!(memo.entries.as_ptr(), entry_pointer);
            assert_eq!(memo.used.as_ptr(), used_pointer);
        }
        assert_eq!(memo.resolve(1, 8, || Ok(10)), Ok(10));
        memo.clear();
        assert_eq!(memo.used_len, 0);
        assert!(memo.entries.iter().all(|e| !e.occupied && e.region == 0));
        assert_eq!(memo.allocated_bytes(), bytes);
    }

    #[test]
    fn exact_small_reads_share_only_within_one_verified_pass() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut bytes = [0; 8];
        for _ in 0..4 {
            assert!(memory.copy(64, &mut bytes));
        }
        assert_eq!(bytes, [64, 65, 66, 67, 68, 69, 70, 71]);
        assert_eq!(source.calls.borrow().as_slice(), &[(64, 8)]);
        assert!(memory.immutable_view(64, 8).is_none());
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(source.calls.borrow().as_slice(), &[(64, 8), (64, 8)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(
            cache
                .borrow()
                .keys
                .iter()
                .all(|e| e.address == 0 && e.len == 0)
        );
        assert!(
            cache
                .borrow()
                .bytes
                .iter()
                .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
        );
        source.bytes.borrow_mut()[64] = 17;
        assert!(memory.copy(64, &mut bytes));
        assert_eq!(bytes[0], 17);
        assert_eq!(source.calls.borrow().len(), 3);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn small_read_mask_keeps_original_modulo_slots_for_arbitrary_capacities() {
        for capacity in [0, 1, 2, 3, 7, 8, 16, 31, 32, 1023, 1024, 32 * 1024] {
            let cache = SmallReadCache::new(capacity);
            assert_eq!(
                cache.mask,
                capacity
                    .is_power_of_two()
                    .then_some(capacity.wrapping_sub(1))
            );
            for address in [
                0,
                1,
                7,
                8,
                31,
                32,
                63,
                64,
                0x1234_5678,
                usize::MAX - 32,
                usize::MAX,
            ] {
                for len in [0, 1, 2, 4, 8, 16, 17, 24, 31, 32, 33, usize::MAX] {
                    let expected = if capacity == 0 || len == 0 || len > SMALL_READ_BYTES {
                        None
                    } else {
                        Some(
                            ((address >> 3) ^ (address >> 19) ^ len.wrapping_mul(0x9e37_79b9))
                                % capacity,
                        )
                    };
                    assert_eq!(
                        cache.slot(address, len),
                        expected,
                        "capacity={capacity}, address={address}, len={len}"
                    );
                }
            }
        }
    }

    #[test]
    fn small_read_growth_updates_mask_from_actual_length_and_clears_both_arrays() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut length = 0;
        for requested in [3, 8, 9, 16, 7, 0, 17, 32] {
            cache.borrow_mut().grow(requested);
            length = length.max(requested);
            {
                let cache = cache.borrow();
                assert_eq!(
                    (cache.keys.len(), cache.bytes.len(), cache.used.len()),
                    (length, length, length)
                );
                assert_eq!(
                    cache.mask,
                    length.is_power_of_two().then_some(length.wrapping_sub(1))
                );
                assert_eq!(cache.used_len, 0);
                assert!(
                    cache
                        .keys
                        .iter()
                        .all(|key| key.len == 0 && key.address == 0)
                );
                assert!(
                    cache
                        .bytes
                        .iter()
                        .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
                );
                for address in [32_usize, 64, 128] {
                    let expected =
                        ((address >> 3) ^ (address >> 19) ^ 8_usize.wrapping_mul(0x9e37_79b9))
                            % length;
                    assert_eq!(cache.slot(address, 8), Some(expected));
                }
            }
            let pointers = {
                let cache = cache.borrow();
                (
                    cache.keys.as_ptr(),
                    cache.bytes.as_ptr(),
                    cache.used.as_ptr(),
                )
            };
            let allocated = cache.borrow().allocated_bytes();
            assert!(memory.copy(64, &mut [0; 8]));
            assert!(memory.copy(64, &mut [0; 8]));
            assert!(cache.borrow_mut().verify_and_clear(&source));
            let cache = cache.borrow();
            assert_eq!(
                (
                    cache.keys.as_ptr(),
                    cache.bytes.as_ptr(),
                    cache.used.as_ptr()
                ),
                pointers
            );
            assert_eq!(cache.allocated_bytes(), allocated);
            assert_eq!(
                allocated,
                cache.keys.capacity() * size_of::<CacheKey>()
                    + cache.bytes.capacity() * SMALL_READ_BYTES
                    + cache.used.capacity() * size_of::<usize>()
            );
            drop(cache);
            // Leave an occupied slot for the next grow, including no-growth
            // requests: growth must clear both metadata and owned payload.
            assert!(memory.copy(64, &mut [0; 8]));
        }
        cache.borrow_mut().clear();
    }

    #[test]
    fn separated_small_read_metadata_preserves_collision_fault_and_change_semantics() {
        for capacity in [3, 7, 8, 16] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::new(capacity));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let slot = cache.borrow().slot(32, 8).unwrap();
            let mut first = [0; 8];
            assert!(memory.copy(32, &mut first));
            let retained = capacity.min(4);
            for address in 33..32 + retained {
                assert_eq!(cache.borrow().slot(address, 8), Some(slot));
                assert!(memory.copy(address, &mut [0; 8]));
            }
            let collision = 32 + retained;
            assert_eq!(cache.borrow().slot(collision, 8), Some(slot));
            let mut second = [0; 8];
            assert!(memory.copy(collision, &mut second));
            source.bytes.borrow_mut()[collision] ^= 1;
            assert!(memory.copy(collision, &mut second));
            assert_eq!(second[0], source.bytes.borrow()[collision]);
            source.fault.set(Some(collision));
            assert!(!memory.copy(collision, &mut second));
            assert_eq!(cache.borrow().used_len, retained);
            assert_eq!(cache.borrow().keys[slot].address, 32);
            assert_eq!(&cache.borrow().bytes[slot][..8], &first);
            source.bytes.borrow_mut()[32] ^= 1;
            let mut hit = [0; 8];
            assert!(memory.copy(32, &mut hit));
            assert_eq!(hit, first);
            assert!(!cache.borrow_mut().verify_and_clear(&source));
            assert_eq!(cache.borrow().used_len, 0);
            assert!(cache.borrow().keys.iter().all(|key| key.len == 0));
            assert!(
                cache
                    .borrow()
                    .bytes
                    .iter()
                    .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
            );
        }
    }

    #[test]
    fn four_probe_neighborhood_wraps_and_bounds_retention_for_all_small_capacities() {
        for capacity in [0, 1, 2, 3, 4, 5, 7, 8, 16] {
            let source = Observed::new();
            let cache = RefCell::new(SmallReadCache::with_witness(capacity, 16, 128));
            let memory = CacheRef {
                source: &source,
                cache: &cache,
                modifiers: None,
                supply: None,
                air_defense: None,
                primary: None,
            };
            let address = if capacity == 0 {
                32
            } else {
                (0..192)
                    .step_by(8)
                    .find(|&address| cache.borrow().slot(address, 8) == Some(capacity - 1))
                    .unwrap()
            };
            let retained = capacity.min(4);
            let allocated = cache.borrow().allocated_bytes();
            let witness = memory.read_witness().unwrap();
            for offset in 0..5 {
                assert!(memory.copy(address + offset, &mut [0; 8]));
            }
            for offset in 0..5 {
                assert!(memory.copy(address + offset, &mut [0; 8]));
            }
            assert_eq!(source.calls.borrow().len(), 10 - retained);
            {
                let cache = cache.borrow();
                assert_eq!(cache.used_len, retained);
                assert_eq!(cache.diagnostics.hits, retained as u64);
                assert_eq!(cache.diagnostics.retained_misses, retained as u64);
                assert_eq!(cache.diagnostics.direct_reads, (2 * (5 - retained)) as u64);
                assert_eq!(cache.witness.len, 2 * (5 - retained));
                for offset in 0..retained {
                    let slot = (capacity - 1 + offset) % capacity;
                    assert_eq!(cache.used[offset], slot);
                    assert_eq!(cache.keys[slot].address, address + offset);
                    assert_eq!(cache.keys[slot].len, 8);
                }
            }
            assert!(memory.read_witness_complete(witness));
            assert!(cache.borrow_mut().verify_and_clear(&source));
            assert_eq!(source.calls.borrow().len(), 2 * (10 - retained));
            assert_eq!(cache.borrow().allocated_bytes(), allocated);
            assert!(!memory.read_witness_complete(witness));
            source.bytes.borrow_mut()[address] ^= 1;
            let mut bytes = [0; 8];
            assert!(memory.copy(address, &mut bytes));
            assert_eq!(bytes[0], source.bytes.borrow()[address]);
            assert!(cache.borrow_mut().verify_and_clear(&source));
        }
    }

    #[test]
    fn probed_failed_insertion_stays_empty_and_final_fault_checks_every_retained_key() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(8, 8, 64));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let witness = memory.read_witness().unwrap();
        assert!(memory.copy(32, &mut [0; 8]));
        source.fault.set(Some(33));
        assert!(!memory.copy(33, &mut [0; 8]));
        assert_eq!(cache.borrow().used_len, 1);
        source.fault.set(None);
        for address in [34, 35, 33] {
            assert!(memory.copy(address, &mut [0; 8]));
        }
        assert_eq!(cache.borrow().used_len, 4);
        // Failed acquisition cannot become a complete witness after recovery.
        assert!(!memory.read_witness_complete(witness));
        source.fault.set(Some(32));
        let before = source.calls.borrow().len();
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            &source.calls.borrow()[before..],
            &[(32, 8), (34, 8), (35, 8), (33, 8)]
        );
        assert_eq!(cache.borrow().used_len, 0);
        assert!(cache.borrow().keys.iter().all(|key| key.len == 0));
    }

    #[test]
    fn probed_overlap_lengths_stay_distinct_and_full_neighborhood_exhausts_witness() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::with_witness(32, 1, 8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        for len in [1, 2, 4, 8, 16, 24, 32] {
            assert!(memory.copy(64, &mut [0; 32][..len]));
        }
        for len in [1, 2, 4, 8, 16, 24, 32] {
            let mut bytes = [0; 32];
            assert!(memory.copy(64, &mut bytes[..len]));
            assert_eq!(&bytes[..len], &source.bytes.borrow()[64..64 + len]);
        }
        assert_eq!(source.calls.borrow().len(), 7);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        let witness = memory.read_witness().unwrap();
        for address in 32..36 {
            assert!(memory.copy(address, &mut [0; 8]));
        }
        assert!(memory.copy(36, &mut [0; 8]));
        assert!(memory.read_witness_complete(witness));
        source.bytes.borrow_mut()[43] ^= 1;
        let mut direct = [0; 8];
        assert!(memory.copy(36, &mut direct));
        assert_eq!(direct[7], source.bytes.borrow()[43]);
        assert!(!memory.read_witness_complete(witness));
        assert_eq!(cache.borrow().witness.required_entries, 2);
        let before = source.calls.borrow().len();
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(
            &source.calls.borrow()[before..],
            &[(36, 8), (32, 8), (33, 8), (34, 8), (35, 8)]
        );
    }

    #[test]
    fn collisions_different_lengths_large_and_empty_reads_never_evict_or_alias() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(1));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(memory.copy(64, &mut [0; 8]));
        assert!(memory.copy(72, &mut [0; 8]));
        assert!(memory.copy(72, &mut [0; 8]));
        assert!(memory.copy(64, &mut [0; 4]));
        assert!(memory.copy(64, &mut [0; 4]));
        assert!(memory.copy(64, &mut [0; 8]));
        assert!(memory.copy(96, &mut [0; 33]));
        assert!(memory.copy(96, &mut [0; 33]));
        assert!(memory.copy(96, &mut []));
        assert_eq!(source.calls.borrow().len(), 8);
        assert_eq!(cache.borrow().used_len, 1);
        assert_eq!(cache.borrow().keys[0].address, 64);
        assert_eq!(cache.borrow().keys[0].len, 8);
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(source.calls.borrow().last(), Some(&(64, 8)));
    }

    #[test]
    fn failed_initial_reads_are_not_cached_and_verification_faults_fail_the_pass() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        source.fault.set(Some(32));
        assert!(!memory.copy(32, &mut [0; 8]));
        assert!(!memory.copy(32, &mut [0; 8]));
        assert_eq!(cache.borrow().used_len, 0);
        source.fault.set(None);
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(memory.copy(40, &mut [0; 8]));
        source.fault.set(Some(32));
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        // Verification must still inspect the second retained span after the
        // first faults, and must clear both before another independent pass.
        assert_eq!(&source.calls.borrow()[4..], &[(32, 8), (40, 8)]);
        assert_eq!(cache.borrow().used_len, 0);
        assert!(cache.borrow().keys.iter().all(|e| e.len == 0));
        assert!(
            cache
                .borrow()
                .bytes
                .iter()
                .all(|bytes| *bytes == [0; SMALL_READ_BYTES])
        );
    }

    #[test]
    fn final_direct_reread_exposes_a_mutation_hidden_by_an_intermediate_cache_hit() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(8));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        let mut bytes = [0; 8];
        assert!(memory.copy(32, &mut bytes));
        source.bytes.borrow_mut()[32] ^= 1;
        assert!(memory.copy(32, &mut bytes));
        assert_eq!(bytes[0], 32);
        assert!(!cache.borrow_mut().verify_and_clear(&source));
        assert!(memory.copy(32, &mut bytes));
        assert_eq!(bytes[0], 33);
        assert!(cache.borrow_mut().verify_and_clear(&source));
    }

    #[test]
    fn cache_reset_growth_and_accounting_are_off_callback_and_bounded() {
        let source = Observed::new();
        let cache = RefCell::new(SmallReadCache::new(0));
        let memory = CacheRef {
            source: &source,
            cache: &cache,
            modifiers: None,
            supply: None,
            air_defense: None,
            primary: None,
        };
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(memory.copy(32, &mut [0; 8]));
        assert_eq!(cache.borrow().allocated_bytes(), 0);
        cache.borrow_mut().grow(2);
        let allocated = cache.borrow().allocated_bytes();
        assert!(allocated >= 2 * (size_of::<CacheKey>() + SMALL_READ_BYTES + size_of::<usize>()));
        assert!(memory.copy(32, &mut [0; 8]));
        cache.borrow_mut().clear();
        assert_eq!(cache.borrow().used_len, 0);
        assert!(memory.copy(32, &mut [0; 8]));
        assert!(cache.borrow_mut().verify_and_clear(&source));
        assert_eq!(cache.borrow().allocated_bytes(), allocated);
    }

    #[test]
    fn conversion_uses_owned_edges_and_keeps_separate_leg_rounding() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                legs: 2,
                cache_entries: 2,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 2,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        capture.rows.rows[0] = cunit::EtaInputs {
            division: 7,
            next: 10,
            destination: 11,
            path_total: 2,
            leg_start: 0,
            leg_len: 2,
            next_leg_count: 1,
            numerator: 1,
            current_speed: (3_i64).into(),
            normal: None,
        };
        capture.legs.first[..2].copy_from_slice(&[
            cunit::EtaLeg {
                distance: 2,
                speed: (3_i64).into(),
            },
            cunit::EtaLeg {
                distance: 2,
                speed: (3_i64).into(),
            },
        ]);
        capture.legs.len = 2;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::DivisionEta(rows))] = capture.tables() else {
            panic!("expected exact ETA rows");
        };
        assert_eq!(rows[0].next_remaining, 33_333);
        assert_eq!(rows[0].destination_remaining, 99_999);
        assert_eq!(rows[0].path_total, 2);
        capture.legs.len = 1;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }

    #[test]
    fn failures_and_truncated_empty_prefix_are_not_standstill() {
        let mut capture = Captured::with_capacities(
            &Capacities {
                rows: 1,
                legs: 1,
                cache_entries: 2,
                witness_entries: 0,
                witness_bytes: 0,
                transport_entries: 2,
                modifier_accumulators: 0,
                modifier_bytes: 0,
                supply_entries: 0,
                air_defense_entries: 0,
                primary_roots: 0,
                primary_chunks: 0,
            },
            &[0],
        );
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 8,
        };
        assert!(matches!(
            capture.tables(),
            [TableView::Truncated { missing: 8, .. }]
        ));
        capture.rows.checked = true;
        capture.rows.mismatches = 1;
        assert!(matches!(capture.tables(), [TableView::Unavailable { .. }]));
    }

    #[test]
    fn independent_eta_window_retires_only_comparison_storage() {
        let mut caps = Capacities {
            rows: 1,
            legs: 1,
            cache_entries: 2,
            witness_entries: 0,
            witness_bytes: 0,
            transport_entries: 2,
            modifier_accumulators: 0,
            modifier_bytes: 0,
            supply_entries: 0,
            air_defense_entries: 0,
            primary_roots: 0,
            primary_chunks: 0,
        };
        let mut capture = Captured::with_capacities(&caps, &[0]);
        capture.legs.required = 3;
        capture.grow(&mut caps, &[CONSISTENCY_TARGET]);
        assert!(capture.legs.first.len() >= 3);
        assert!(capture.rows.second.is_empty());
        assert!(capture.legs.second.is_empty());
        assert!(!capture.rows.rows.is_empty());
    }
}
