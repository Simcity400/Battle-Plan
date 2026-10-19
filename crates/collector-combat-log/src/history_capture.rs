//! Demand-only native acquisition. Owned batches are reduced/projected only
//! by conversion, after the acquisition worker has returned to the game.
use crate::{history_events, history_projection::Held, history_reducer::Store};
use engine_land_inputs::{combat_log_events as event, combat_log_history as native};
use engine_native::{Memory, walk::Outcome};
use replay_data::rows::*;
use replay_frame::frame::{TableInput, TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
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

pub struct State {
    signal: Option<i64>,
    lost: u64,
    available: bool,
    full: bool,
    sequence: u64,
    batch: Batch,
    drained: Vec<history_events::Logged>,
    requested: Vec<u32>,
    witness: Option<u64>,
    failure: Option<&'static str>,
}
impl State {
    pub const fn new() -> Self {
        Self {
            signal: None,
            lost: 0,
            available: false,
            full: false,
            sequence: 0,
            batch: Batch {
                facts: Vec::new(),
                counts: Vec::new(),
            },
            drained: Vec::new(),
            requested: Vec::new(),
            witness: None,
            failure: Some(UNREAD),
        }
    }
    pub fn begin(&mut self, signal: i64, full: bool) {
        let Some(next) = self.sequence.checked_add(1) else {
            self.refuse("combat log batch sequence exhausted");
            return;
        };
        self.sequence = next;
        self.batch.clear();
        self.requested.clear();
        self.full = full;
        let drained = history_events::drain(&mut self.drained);
        history_events::prepare(0);
        self.witness = if full {
            history_events::quiet()
        } else {
            drained.as_ref().ok().map(|&(_, revision)| revision)
        };
        if full {
            self.signal = Some(signal);
            self.lost = history_events::losses();
            self.available = false;
            self.failure = None;
            return;
        }
        let result = (|| {
            if self.signal != Some(signal) || !self.available {
                return Err(UNREAD);
            }
            if drained?.0 != self.lost {
                return Err("combat log event loss revoked retained history");
            }
            for logged in &self.drained {
                if logged.signal != signal {
                    continue;
                }
                let fact = logged.fact;
                if fact.kind == event::RESET {
                    self.requested.clear();
                }
                if fact.kind == event::COUNT {
                    push(&mut self.requested, fact.root as u32)?;
                }
                if fact.kind == event::SLOT {
                    push(&mut self.requested, fact.item as u32)?;
                }
                push(&mut self.batch.facts, fact)?;
            }
            Ok(())
        })();
        if let Err(reason) = result {
            self.refuse(reason);
        }
    }
    pub fn seed_group(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: u32,
        pointer: usize,
        ordinal: u32,
        bytes: &[u8],
    ) -> Result<(), &'static str> {
        if !self.full || self.failure.is_some() {
            return Ok(());
        }
        let mut result = Ok(());
        event::seed_group(
            memory,
            base,
            pointer,
            country,
            ordinal,
            bytes,
            &mut |fact| {
                if result.is_ok() {
                    if fact.kind == event::REFERENCE {
                        result = push(&mut self.requested, fact.values[0] as u32);
                    }
                    if result.is_ok() {
                        result = push(&mut self.batch.facts, fact);
                    }
                }
            },
        )?;
        result
    }
    pub fn finish(
        &mut self,
        memory: &impl Memory,
        base: usize,
        complete: bool,
    ) -> Result<(), &'static str> {
        if let Some(reason) = self.failure {
            return Err(reason);
        }
        if self.witness.is_none() || history_events::quiet() != self.witness {
            return Err("combat log writer changed during history acquisition");
        }
        self.requested.sort_unstable();
        self.requested.dedup();
        if self.full {
            if !complete {
                return Err("combat log initial group walk incomplete");
            }
            if !self.requested.is_empty() {
                let pool = native::BattlePool::read(memory, base)?;
                for &id in &self.requested {
                    let record = pool
                        .read_record(memory, base, id as i32)?
                        .ok_or("combat log seeded reference has no record")?;
                    seed_battle(&mut self.batch, id, &record)?;
                }
            }
            history_events::prepare(self.batch.facts.len().max(1));
        } else {
            let mut result = Ok(());
            event::settled_counts(memory, base, &self.requested, &mut |id, count| {
                if result.is_ok() {
                    result = push(&mut self.batch.counts, (id, i32::from(count)));
                }
            })?;
            result?;
        }
        if history_events::quiet() != self.witness {
            return Err("combat log writer changed during count settlement");
        }
        self.available = true;
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
        self.failure = Some(reason);
        self.batch.clear();
        self.converted = OnceLock::new();
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        // This is an owned event batch, not an eagerly projected row census.
        // Complete row cardinalities are known only after conversion.
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
        if ((self.full && self.converted.get().is_none())
            || conversion.sequence != self.sequence
            || conversion.signal != self.signal)
            && let Err(reason) = conversion.apply(self)
        {
            conversion.signal = None;
            return std::array::from_fn(|_| Err(reason));
        }
        let views = conversion.held.snapshot();
        let _ = self.converted.set(std::array::from_fn(|i| {
            views[i].as_ref().map_or(0, |r| r.len())
        }));
        views
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
pub fn release_conversion() {
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
