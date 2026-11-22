use crate::{history_events, history_projection::Held, history_reducer::Store};
use engine_land_inputs::{combat_log_events as event, combat_log_history as native};
use engine_native::{Memory, walk::Outcome};
use replay_data::rows::*;
use replay_frame::frame::{TableInput, TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
pub const N: usize = 11;
pub const TABLES: [TableId; N] = [
    TableId::CombatLogGroups,
    TableId::CombatLogMonths,
    TableId::CombatLogManpower,
    TableId::CombatLogTemplates,
    TableId::CombatLogReferences,
    TableId::CombatLogBattles,
    TableId::CombatLogEnemyEquipment,
    TableId::CombatLogBattleSides,
    TableId::CombatLogBattleEquipment,
    TableId::CombatLogBattleModifiers,
    TableId::CombatLogBattleCountries,
];
const UNREAD: &str = "combat log retained-history baseline unavailable";
pub(crate) type Views = [Result<Arc<TableRows>, &'static str>; N];
fn push<T>(rows: &mut Vec<T>, value: T) -> Result<(), &'static str> {
    engine_allocation::reserve_push_batch(rows, 1)
        .map_err(|_| "combat log batch allocation failed")?;
    rows.push(value);
    Ok(())
}
#[derive(Default)]
struct Batch {
    facts: Vec<event::Fact>,
    counts: Vec<(u32, i32)>,
}
impl Batch {
    fn clear(&mut self) {
        self.facts.clear();
        self.counts.clear();
    }
}
fn seed_battle(batch: &mut Batch, id: u32, b: &native::Battle) -> Result<(), &'static str> {
    let token = id as u64;
    let mut emit = |kind, item, values| {
        push(
            &mut batch.facts,
            event::Fact {
                kind,
                root: token,
                item,
                values,
            },
        )
    };
    emit(
        event::BATTLE,
        0,
        [
            b.date as u64,
            b.province as u64,
            u32::from_le_bytes(b.flags) as u64,
            0,
            0,
        ],
    )?;
    for (side, s) in [&b.attacker, &b.defender].into_iter().enumerate() {
        let side = side as u64;
        emit(
            event::SIDE,
            side,
            [s.manpower as u64, s.air_factor_raw as u64, s.leader, 0, 0],
        )?;
        for (kind, p) in [&s.equipment, &s.captured_by_enemy, &s.recovered]
            .into_iter()
            .enumerate()
        {
            let key = side * 3 + kind as u64;
            emit(
                event::POOL,
                key,
                [u64::from(p.serialized), u64::from(p.allow_zero), 0, 0, 0],
            )?;
            for (ordinal, e) in p.equipment.iter().enumerate() {
                emit(
                    event::EQUIPMENT,
                    key,
                    [ordinal as u64, e.variant, e.amount_raw as u64, 0, 0],
                )?;
            }
        }
        for (ordinal, m) in s.modifiers.iter().enumerate() {
            emit(
                event::MODIFIER,
                side,
                [ordinal as u64, m.modifier as u64, m.hours as u64, 0, 0],
            )?;
        }
        for (ordinal, &country) in s.countries.iter().enumerate() {
            emit(
                event::COUNTRY,
                side,
                [ordinal as u64, country as u64, 0, 0, 0],
            )?;
        }
    }
    emit(event::SLOT, id as u64, [0; 5])?;
    push(&mut batch.counts, (id, b.ref_count))
}

#[derive(Clone, Copy, Default, serde::Serialize)]
struct RecoveryCall {
    sequence: u64,
    reason: &'static str,
    elapsed_ns: u64,
    groups_visited: u64,
    battles_read: u64,
    facts: u64,
    complete: bool,
}
#[derive(Default, serde::Serialize)]
struct RecoveryStatus {
    calls: u64,
    total_ns: u64,
    last: RecoveryCall,
}
static RECOVERY: Mutex<RecoveryStatus> = Mutex::new(RecoveryStatus {
    calls: 0,
    total_ns: 0,
    last: RecoveryCall {
        sequence: 0,
        reason: "not called",
        elapsed_ns: 0,
        groups_visited: 0,
        battles_read: 0,
        facts: 0,
        complete: false,
    },
});
fn record_recovery(call: RecoveryCall) {
    let mut status = RECOVERY.lock().unwrap_or_else(|e| e.into_inner());
    status.calls = status.calls.saturating_add(1);
    status.total_ns = status.total_ns.saturating_add(call.elapsed_ns);
    status.last = call;
}
pub fn recovery_status() -> impl serde::Serialize {
    let status = RECOVERY.lock().unwrap_or_else(|e| e.into_inner());
    RecoveryStatus {
        calls: status.calls,
        total_ns: status.total_ns,
        last: status.last,
    }
}

#[derive(Clone, Copy, Default)]
struct SlotWitness {
    count: Option<i32>,
    counted_at: u64,
    allocated_at: u64,
}

pub struct State {
    signal: Option<i64>,
    available: bool,
    full: bool,
    sequence: u64,
    batch: Batch,
    requested: Vec<u32>,
    witness: Option<history_events::Witness>,
    certificate: Option<history_events::Witness>,
    drained: Vec<history_events::Logged>,
    slots: Vec<SlotWitness>,
    recovery_reason: &'static str,
    failure: Option<&'static str>,
}
static REBASE_REQUIRED: AtomicBool = AtomicBool::new(false);
impl State {
    pub const fn new() -> Self {
        Self {
            signal: None,
            available: false,
            full: false,
            sequence: 0,
            batch: Batch {
                facts: Vec::new(),
                counts: Vec::new(),
            },
            requested: Vec::new(),
            witness: None,
            certificate: None,
            drained: Vec::new(),
            slots: Vec::new(),
            recovery_reason: UNREAD,
            failure: Some(UNREAD),
        }
    }
    pub(crate) fn begin(&mut self, signal: i64, full: bool) {
        let retry = REBASE_REQUIRED.swap(false, Ordering::AcqRel);
        let drained = history_events::drain(&mut self.drained);
        self.begin_observed(signal, full || retry, drained.as_ref().ok().copied());
        if let Err(reason) = drained {
            self.recover(reason);
        } else if !self.full {
            let mut entries = std::mem::take(&mut self.drained);
            let result = self.incremental(signal, &entries);
            entries.clear();
            self.drained = entries;
            if let Err(reason) = result {
                self.recover(reason);
            }
        }
        // SAFETY: Admitted combat-log capture prepares history-event storage while storage lives.
        unsafe { history_events::prepare(0) };
    }
    fn begin_observed(
        &mut self,
        signal: i64,
        full: bool,
        witness: Option<history_events::Witness>,
    ) {
        let Some(next) = self.sequence.checked_add(1) else {
            self.refuse("combat log batch sequence exhausted");
            return;
        };
        self.sequence = next;
        self.batch.clear();
        self.requested.clear();
        self.full = full
            || !self.available
            || self.signal != Some(signal)
            || self.certificate.map(|w| w.lost) != witness.map(|w| w.lost);
        self.recovery_reason = if full {
            "combat log hour or conversion continuity lost"
        } else if !self.available || self.signal != Some(signal) {
            UNREAD
        } else {
            "combat log writer occurrence loss"
        };
        self.signal = Some(signal);
        self.witness = witness;
        self.available = false;
        self.failure = None;
    }
    fn recover(&mut self, reason: &'static str) {
        self.full = true;
        self.recovery_reason = reason;
        self.batch.clear();
        self.requested.clear();
    }
    fn incremental(
        &mut self,
        signal: i64,
        entries: &[history_events::Logged],
    ) -> Result<(), &'static str> {
        engine_allocation::advisory(|| {
            for logged in entries {
                if logged.signal != signal {
                    return Err("combat log occurrence belongs to another session");
                }
                let fact = logged.fact;
                match fact.kind {
                    event::RESET => {
                        return Err("combat log reset requires retained-state recovery");
                    }
                    event::SLOT => {
                        let sequence = self.sequence;
                        let slot = self.slot(fact.item as u32)?;
                        if (slot.count.is_some_and(|n| n > 0) || slot.allocated_at == sequence)
                            && slot.counted_at != sequence
                        {
                            return Err("combat log slot reuse lacks a release witness");
                        }
                        slot.allocated_at = sequence;
                        slot.counted_at = 0;
                        push(&mut self.requested, fact.item as u32)?;
                    }
                    event::RELEASE => {
                        let sequence = self.sequence;
                        self.slot(fact.root as u32)?.counted_at = sequence;
                        push(&mut self.requested, fact.root as u32)?;
                    }
                    event::COUNT => push(&mut self.requested, fact.root as u32)?,
                    _ => (),
                }
                push(&mut self.batch.facts, fact)?;
            }
            Ok(())
        })
    }
    fn slot(&mut self, id: u32) -> Result<&mut SlotWitness, &'static str> {
        let slots = id as usize + 1;
        if slots > self.slots.len() {
            let additional = slots - self.slots.len();
            engine_allocation::advisory(|| engine_allocation::reserve(&mut self.slots, additional))
                .map_err(|_| "combat log slot lifetime allocation failed")?;
            self.slots.resize(slots, SlotWitness::default());
        }
        Ok(&mut self.slots[id as usize])
    }
    pub(crate) fn acquire(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        if let Err(reason) = self.acquire_observed(memory, base, game_state, history_events::quiet)
        {
            self.refuse(reason);
        } else if self.full {
            history_events::prepare_seed(self.batch.facts.len());
            if history_events::drain(&mut self.drained).is_err() {
                REBASE_REQUIRED.store(true, Ordering::Release);
            }
            if let Some(witness) = history_events::quiet().filter(|w| {
                self.witness
                    .is_some_and(|previous| previous.revision == w.revision)
            }) {
                if self.certificate.is_some() {
                    self.certificate = Some(witness);
                }
                self.witness = Some(witness);
            } else {
                self.refuse("combat log writer changed after retained-state recovery");
            }
            // SAFETY: Admitted combat-log capture prepares history-event storage while storage lives.
        unsafe { history_events::prepare(0) };
        }
    }
    fn acquire_observed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        observe: impl Fn() -> Option<history_events::Witness>,
    ) -> Result<(), &'static str> {
        engine_allocation::advisory(|| {
            if let Some(reason) = self.failure {
                return Err(reason);
            }
            let witness = observe().ok_or("combat log writers not quiescent or unavailable")?;
            if self.witness != Some(witness) {
                self.recover("combat log writer changed after occurrence drain");
                self.witness = Some(witness);
            }
            if !self.full {
                self.requested.sort_unstable();
                self.requested.dedup();
                let mut result = Ok(());
                let counts =
                    event::settled_counts(memory, base, &self.requested, &mut |id, count| {
                        if result.is_ok() {
                            result = push(&mut self.batch.counts, (id, i32::from(count)));
                        }
                    })
                    .and(result);
                if let Err(reason) = counts {
                    self.recover(reason);
                }
            }
            if self.full {
                self.batch.clear();
                self.requested.clear();
                let started = std::time::Instant::now();
                let mut call = RecoveryCall {
                    sequence: self.sequence,
                    reason: self.recovery_reason,
                    ..Default::default()
                };
                let result = self.rebuild(memory, base, game_state, &mut call);
                call.elapsed_ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                call.complete = result.is_ok() && observe() == self.witness;
                call.facts = self.batch.facts.len() as u64;
                record_recovery(call);
                result?;
            }
            if observe() != self.witness {
                return Err("combat log writer changed during history acquisition");
            }
            if self.full {
                self.slots.fill(SlotWitness::default());
            }
            let mut tracked = true;
            let full = self.full;
            for index in 0..self.batch.counts.len() {
                let (id, count) = self.batch.counts[index];
                match self.slot(id) {
                    Ok(slot) => slot.count = Some(count),
                    Err(reason) if !full => return Err(reason),
                    Err(_) => {
                        tracked = false;
                        break;
                    }
                }
            }
            self.available = true;
            self.certificate = if tracked { self.witness } else { None };
            Ok(())
        })
    }
    fn rebuild(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        call: &mut RecoveryCall,
    ) -> Result<(), &'static str> {
        engine_land_inputs::ncombat_log_cmanager::visit_retained_groups(
            memory,
            base,
            game_state,
            |country, pointer, ordinal, bytes| {
                call.groups_visited += 1;
                event::seed_group(
                    memory,
                    base,
                    pointer,
                    country,
                    ordinal,
                    bytes,
                    &mut |fact| {
                        if self.failure.is_none() {
                            let result = (|| {
                                if fact.kind == event::REFERENCE {
                                    push(&mut self.requested, fact.values[0] as u32)?;
                                }
                                push(&mut self.batch.facts, fact)
                            })();
                            if let Err(reason) = result {
                                self.failure = Some(reason);
                            }
                        }
                    },
                )?;
                self.failure.map_or(Ok(()), Err)
            },
        )?;
        self.requested.sort_unstable();
        self.requested.dedup();
        if !self.requested.is_empty() {
            let pool = native::BattlePool::read(memory, base)?;
            for &id in &self.requested {
                let record = pool
                    .read_record(memory, base, id as i32)?
                    .ok_or("combat log seeded reference has no record")?;
                call.battles_read += 1;
                seed_battle(&mut self.batch, id, &record)?;
            }
        }
        Ok(())
    }
    pub fn refuse(&mut self, reason: &'static str) {
        self.available = false;
        self.failure = Some(reason);
    }
    pub fn snapshot(&mut self, target: &mut Captured) {
        std::mem::swap(&mut self.batch, &mut target.batch);
        self.batch.clear();
        target.signal = self.signal;
        target.sequence = self.sequence;
        target.full = self.full;
        target.failure = self
            .failure
            .or(if self.available { None } else { Some(UNREAD) });
        target.converted = OnceLock::new();
    }
}
impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
pub struct Captured {
    batch: Batch,
    signal: Option<i64>,
    sequence: u64,
    full: bool,
    failure: Option<&'static str>,
    converted: OnceLock<[usize; N]>,
}
impl Captured {
    pub fn new() -> Self {
        Self {
            batch: Batch::default(),
            signal: None,
            sequence: 0,
            full: false,
            failure: Some(UNREAD),
            converted: OnceLock::new(),
        }
    }
    pub fn fail(&mut self, reason: &'static str) {
        REBASE_REQUIRED.store(true, Ordering::Release);
        self.reset(reason);
    }
    pub fn reset(&mut self, reason: &'static str) {
        self.failure = Some(reason);
        self.batch.clear();
        self.converted = OnceLock::new();
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        if let Some(lengths) = self.converted.get() {
            return std::array::from_fn(|i| Outcome::Rows { len: lengths[i] });
        }
        [self.failure
            .map_or(Outcome::Rows { len: 0 }, |reason| Outcome::Unavailable {
                reason,
            }); N]
    }
    fn views(&self) -> Views {
        if let Some(reason) = self.failure {
            return std::array::from_fn(|_| Err(reason));
        }
        let mut conversion = CONVERSION.lock().unwrap_or_else(|e| e.into_inner());
        self.convert(&mut conversion, &REBASE_REQUIRED)
    }
    fn convert(&self, conversion: &mut Conversion, retry: &AtomicBool) -> Views {
        engine_allocation::advisory(|| {
            if let Some(reason) = self.failure {
                return std::array::from_fn(|_| Err(reason));
            }
            if ((self.full && self.converted.get().is_none())
                || conversion.sequence != self.sequence
                || conversion.signal != self.signal)
                && let Err(reason) = conversion.apply(self)
            {
                conversion.signal = None;
                retry.store(true, Ordering::Release);
                return std::array::from_fn(|_| Err(reason));
            }
            let views = conversion.held.snapshot();
            if views.iter().any(Result::is_err) {
                conversion.signal = None;
                retry.store(true, Ordering::Release);
                return views;
            }
            let _ = self.converted.set(std::array::from_fn(|i| {
                views[i].as_ref().map_or(0, |r| r.len())
            }));
            views
        })
    }
    pub fn inputs(&self) -> [TableInput; N] {
        self.views().map(|view| match view {
            Ok(rows) => TableInput::SharedComplete(rows),
            Err(reason) => TableInput::Sampled(TableView::Unavailable {
                reason: reason.into(),
            }),
        })
    }
    pub fn tables(&self) -> [TableView; N] {
        self.views().map(|view| match view {
            Ok(rows) => TableView::Complete((*rows).clone()),
            Err(reason) => TableView::Unavailable {
                reason: reason.into(),
            },
        })
    }
}
impl Default for Captured {
    fn default() -> Self {
        Self::new()
    }
}

struct Conversion {
    signal: Option<i64>,
    sequence: u64,
    store: Store,
    held: Held,
}
impl Conversion {
    const fn new() -> Self {
        Self {
            signal: None,
            sequence: 0,
            store: Store {
                groups: BTreeMap::new(),
                battles: BTreeMap::new(),
                order: BTreeMap::new(),
                members: BTreeMap::new(),
                touched_references: BTreeSet::new(),
                pending_battles: BTreeMap::new(),
                changed_counts: Vec::new(),
            },
            held: Held::new(),
        }
    }
    fn apply(&mut self, batch: &Captured) -> Result<usize, &'static str> {
        if batch.full {
            self.store.clear();
            self.held = Held::new();
        } else if self.signal != batch.signal
            || self.sequence.checked_add(1) != Some(batch.sequence)
        {
            return Err("combat log conversion batch missing; full history required");
        }
        let mut details = BTreeSet::new();
        for &fact in &batch.batch.facts {
            if fact.kind == event::SLOT {
                details.insert(fact.item as u32);
            }
            self.held.apply(&mut self.store, fact)?;
        }
        for &(id, count) in &batch.batch.counts {
            let b = self
                .store
                .battles
                .get_mut(&id)
                .ok_or("combat log count has no staged record")?;
            let changed_presence = (b.ref_count > 0) != (count > 0);
            let changed_count = b.ref_count != count;
            b.ref_count = count;
            if changed_count || details.contains(&id) {
                self.held
                    .battle(id, b, changed_presence || details.contains(&id));
            }
        }
        let visited = self.store.validate()?;
        self.store.changed_counts.clear();
        self.store.touched_references.clear();
        self.signal = batch.signal;
        self.sequence = batch.sequence;
        Ok(visited)
    }
}
static CONVERSION: Mutex<Conversion> = Mutex::new(Conversion::new());
pub(crate) fn release_conversion() {
    *CONVERSION.lock().unwrap_or_else(|e| e.into_inner()) = Conversion::new();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn combat_log_acquisition_handoff_reuses_flat_arenas_without_projection_or_allocation() {
        let mut state = State::new();
        state.signal = Some(123);
        state.sequence = 7;
        state.available = true;
        state.failure = None;
        state.batch.facts.push(event::Fact {
            kind: event::COUNT,
            root: 4,
            ..Default::default()
        });
        let mut target = Captured::new();
        target.batch.facts = vec![event::Fact::default(); 5000];
        let capacity = target.batch.facts.capacity();
        collector_test_support::with_allocations_denied(|| state.snapshot(&mut target));
        assert_eq!(target.batch.facts.len(), 1);
        assert_eq!(target.sequence, 7);
        assert_eq!(state.batch.facts.capacity(), capacity);
        assert!(state.batch.facts.is_empty());
        assert!(target.converted.get().is_none());
    }
    #[test]
    fn combat_log_one_battle_edits_only_changed_rows_with_large_retained_history() {
        let mut c = Conversion::new();
        c.held
            .apply(
                &mut c.store,
                event::Fact {
                    kind: event::GROUP,
                    root: 1,
                    values: [7, 53 | 7 << 32, 0, 0, 0],
                    ..Default::default()
                },
            )
            .unwrap();
        for id in 0..5000 {
            let b = native::Battle {
                ref_count: 1,
                ..Default::default()
            };
            c.held.battle(id, &b, true);
            c.store.battles.insert(id, b);
            c.held
                .apply(
                    &mut c.store,
                    event::Fact {
                        kind: event::REFERENCE,
                        root: 1,
                        item: id as u64,
                        values: [id as u64, 1, 0, 0, 0],
                    },
                )
                .unwrap();
        }
        c.store.validate().unwrap();
        c.store.touched_references.clear();
        let before = c.held.edits;
        let mut batch = Captured::new();
        batch.failure = None;
        batch.signal = None;
        batch.sequence = 1;
        batch.batch.facts.extend([
            event::Fact {
                kind: event::BATTLE,
                root: 9000,
                ..Default::default()
            },
            event::Fact {
                kind: event::SLOT,
                root: 9000,
                item: 5000,
                ..Default::default()
            },
            event::Fact {
                kind: event::REFERENCE,
                root: 1,
                item: 5000,
                values: [5000, 1, 0, 0, 0],
            },
        ]);
        batch.batch.counts.push((5000, 1));
        assert_eq!(
            c.apply(&batch).unwrap(),
            2,
            "validate only the new reference and changed slot"
        );
        assert_eq!(
            c.held.edits - before,
            4,
            "one reference, one scalar and its two sides, independent of retained history"
        );
        assert_eq!(c.store.battles.len(), 5001);
        assert!(
            batch.converted.get().is_none(),
            "acquisition does not project replay rows"
        );
        let before = c.held.edits;
        batch.sequence = 2;
        batch.batch.facts.clear();
        batch.batch.counts.clear();
        c.apply(&batch).unwrap();
        assert_eq!(c.held.edits, before, "quiet hours touch no retained rows");
    }
    #[test]
    fn combat_log_conversion_refuses_skipped_owned_batches_until_a_full_seed() {
        let mut c = Conversion::new();
        let mut batch = Captured::new();
        batch.failure = None;
        batch.sequence = 3;
        assert!(c.apply(&batch).is_err());
        batch.full = true;
        c.apply(&batch).unwrap();
        batch.full = false;
        batch.sequence = 5;
        assert!(c.apply(&batch).is_err());
    }
}

#[cfg(test)]
#[path = "history_capture_tests.rs"]
mod native_tests;
