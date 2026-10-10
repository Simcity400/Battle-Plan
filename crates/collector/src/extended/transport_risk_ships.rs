//! Original SHIPS occurrences with numerical model loans. No native pointer
//! leaves an observer callback; original failed attempts are never retried.
use crate::table_capture::{Arena, grow_vec};
use engine::{
    Memory, cship,
    ctask_force::OriginalRiskShips,
    transport_ledger_observation::{PointerIndex, PointerSlot, index_capacity},
};
use replay::rows::{TransportReceiverShips, TransportRiskShip, TransportRiskShips};

type Fact<T> = Result<T, &'static str>;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static MODEL_REFUSALS: AtomicU64 = AtomicU64::new(0);
static GRAPH_REFUSALS: AtomicU64 = AtomicU64::new(0);
static SOURCE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static GRAPH_REASONS: crate::status::RefusalReasons = crate::status::RefusalReasons::new();
fn refuse_graph(refused: &mut bool, refusals: &mut u64) {
    *refused = true;
    *refusals = refusals.saturating_add(1);
    GRAPH_REFUSALS.fetch_add(1, Relaxed);
}
pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "transport original risk ship model leaves refused": MODEL_REFUSALS.load(Relaxed),
        "transport original risk SHIPS graph refused": GRAPH_REFUSALS.load(Relaxed),
        "graph_reasons": &GRAPH_REASONS,
        "transport original ship-model source failed": SOURCE_REFUSALS.load(Relaxed),
        "scope": "cumulative reached current-hour carrier leaves and complete original SHIPS graph refusals; native skipped leaves are not failures",
    })
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Model {
    pub ship: u64,
    pub observation: cship::TransportModelObservation,
}
#[derive(Default)]
struct Receiver {
    handle: u64,
    count: Option<i32>,
    count_reason: Option<&'static str>,
    first: usize,
    len: usize,
    complete: bool,
}

pub(crate) struct Captured {
    models: Arena<Model>,
    // Allocation-free ownership loans from the joined model-detail workers.
    model_parts: [Arena<Model>; super::ship_models::MAX_PARTS],
    slots: Vec<PointerSlot>,
    receivers: Arena<Receiver>,
    ships: Arena<TransportRiskShip>,
    hour: Option<i32>,
    refused: bool,
    model_hour: Option<i32>,
    model_failed: bool,
    pub model_refusals: u64,
    pub graph_refusals: u64,
}

impl Captured {
    pub(crate) fn prepare(&mut self, receivers: usize, ships: usize) {
        self.models.required = self.models.required.max(ships);
        self.receivers.required = self.receivers.required.max(receivers);
        self.ships.required = self.ships.required.max(ships);
        self.grow();
    }

    pub fn new(receivers: usize, ships: usize) -> Self {
        Self {
            models: Arena::new(ships),
            model_parts: std::array::from_fn(|_| Arena::new(0)),
            slots: engine::allocation::owned_vec(engine::allocation::default_vec(
                index_capacity(ships).unwrap_or(0),
            )),
            receivers: Arena::new(receivers),
            ships: Arena::new(ships),
            hour: None,
            refused: false,
            model_hour: None,
            model_failed: false,
            model_refusals: 0,
            graph_refusals: 0,
        }
    }
    /// Reset once before the original model loans are supplied. Growth demand
    /// remains available even when the previous acquisition was terminal.
    pub fn reset(&mut self) {
        self.hour = None;
        self.refused = false;
        self.model_hour = None;
        self.model_failed = false;
        self.models.len = 0;
        for part in &mut self.model_parts {
            part.len = 0;
        }
        self.receivers.len = 0;
        self.ships.len = 0;
        self.model_refusals = 0;
        self.graph_refusals = 0;
        self.slots.fill(Default::default());
    }
    pub fn begin_models(&mut self, hour: i32) {
        self.reset();
        self.model_hour = Some(hour);
    }
    /// Keep only the exact current hour's original numerical model journal.
    pub fn begin(&mut self, hour: i32) {
        if self.model_hour != Some(hour) {
            self.models.len = 0;
            self.model_failed = true;
        }
        self.hour = None;
        self.receivers.len = 0;
        self.ships.len = 0;
        self.refused = self.model_failed;
        self.slots.fill(Default::default());
    }
    #[cfg(test)]
    pub fn model(&mut self, ship: u64, original: cship::TransportModelObservation) -> Fact<()> {
        self.models.required = self.models.required.max(self.models.len.saturating_add(1));
        let target = self
            .models
            .first
            .get_mut(self.models.len)
            .ok_or("transport model loan arena exhausted")?;
        *target = Model {
            ship,
            observation: original,
        };
        self.models.len += 1;
        Ok(())
    }
    pub fn retain_model_demand(&mut self, count: usize) {
        self.models.required = self.models.required.max(count);
    }
    pub(crate) fn loan_model_part(&mut self, index: usize, original: &mut Arena<Model>) {
        std::mem::swap(&mut self.model_parts[index], original);
    }
    pub fn fail(&mut self) {
        self.model_failed = true;
        // Terminal source failure can be reported by the hourly coordinator.
        // It has no model journal to lend; count that cause without allocating.
        SOURCE_REFUSALS.fetch_add(1, Relaxed);
        refuse_graph(&mut self.refused, &mut self.graph_refusals);
    }
    pub fn grow(&mut self) {
        let mut count = self.models.first.len();
        self.models.grow(&mut count);
        grow_vec(&mut self.slots, index_capacity(count).unwrap_or(0));
        let mut count = self.receivers.first.len();
        self.receivers.grow(&mut count);
        let mut count = self.ships.first.len();
        self.ships.grow(&mut count);
    }
    pub fn allocated_bytes(&self) -> usize {
        self.models.allocated_bytes()
            + self.receivers.allocated_bytes()
            + self.ships.allocated_bytes()
            + self.slots.capacity() * std::mem::size_of::<PointerSlot>()
            + self
                .model_parts
                .iter()
                .map(Arena::allocated_bytes)
                .sum::<usize>()
    }
    pub fn observer<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        hour: i32,
    ) -> Fact<Observer<'a, M>> {
        if self.hour.is_some() {
            GRAPH_REASONS.record("transport original SHIPS observer supplied twice");
            self.model_failed = true;
            refuse_graph(&mut self.refused, &mut self.graph_refusals);
            return Err("transport original SHIPS observer supplied twice");
        }
        self.hour = Some(hour);
        if self.model_hour != Some(hour) || self.model_failed {
            GRAPH_REASONS.record(
                "transport original ship-model attempt journal unavailable for current hour",
            );
            refuse_graph(&mut self.refused, &mut self.graph_refusals);
            return Err(
                "transport original ship-model attempt journal unavailable for current hour",
            );
        }
        // Ordinary is worker-only. Keep every original model attempt while
        // combining the ownership loans, before any risk callback can borrow.
        let combined = (|| {
            for part in &mut self.model_parts {
                let end = self
                    .models
                    .len
                    .checked_add(part.len)
                    .ok_or("transport model loan extent overflow")?;
                if !grow_vec(&mut self.models.first, end) {
                    return Err("transport model loan allocation failed");
                }
                self.models.first[self.models.len..end].copy_from_slice(&part.first[..part.len]);
                self.models.len = end;
                part.len = 0;
            }
            Ok(())
        })();
        if let Err(reason) = combined {
            GRAPH_REASONS.record(reason);
            self.model_failed = true;
            refuse_graph(&mut self.refused, &mut self.graph_refusals);
            return Err(reason);
        }
        self.models.required = self.models.required.max(self.models.len);
        let mut index = match PointerIndex::new_growing(&mut self.slots, self.models.first.len()) {
            Ok(index) => index,
            Err(reason) => {
                GRAPH_REASONS.record(reason);
                self.model_failed = true;
                refuse_graph(&mut self.refused, &mut self.graph_refusals);
                return Err(reason);
            }
        };
        for (ordinal, model) in self.models.first[..self.models.len].iter().enumerate() {
            let Ok(key) = usize::try_from(model.ship) else {
                GRAPH_REASONS.record("transport original ship handle overflow");
                self.model_failed = true;
                refuse_graph(&mut self.refused, &mut self.graph_refusals);
                return Err("transport original ship handle overflow");
            };
            // The first original model attempt owns the observation, including
            // failure; a repeated census occurrence cannot heal it.
            if index.find(key).is_none()
                && let Err(reason) = index.insert(key, ordinal)
            {
                GRAPH_REASONS.record(reason);
                self.model_failed = true;
                refuse_graph(&mut self.refused, &mut self.graph_refusals);
                return Err(reason);
            }
        }
        let original_models = self.models.len;
        Ok(Observer {
            memory,
            base,
            models: &mut self.models,
            index,
            receivers: &mut self.receivers,
            ships: &mut self.ships,
            model_refusals: &mut self.model_refusals,
            graph_refusals: &mut self.graph_refusals,
            refused: &mut self.refused,
            current: None,
            receiver_attempts: 0,
            ship_occurrences: 0,
            original_models,
        })
    }
    /// Off-thread projection, after the original pointer loans have retired.
    pub fn packet(&self, hour: i32) -> std::io::Result<Option<TransportRiskShips>> {
        if self.hour != Some(hour) {
            return Ok(None);
        }
        let receivers = if self.refused {
            None
        } else {
            let mut values = engine::allocation::vec(self.receivers.len)?;
            for receiver in &self.receivers.first[..self.receivers.len] {
                let ships = if receiver.complete {
                    Some(engine::allocation::copy_vec(
                        &self.ships.first[receiver.first..receiver.first + receiver.len],
                    )?)
                } else {
                    None
                };
                values.push(TransportReceiverShips {
                    task_force: receiver.handle,
                    count: receiver.count,
                    ships,
                });
            }
            values.sort_unstable_by_key(|receiver| receiver.task_force);
            Some(values)
        };
        Ok(Some(TransportRiskShips {
            sample_hour: hour,
            receivers,
        }))
    }
}

impl<M: Memory> Drop for Observer<'_, M> {
    fn drop(&mut self) {
        // One totals publication after the original loop; no per-ship atomic.
        MODEL_REFUSALS.fetch_add(*self.model_refusals, Relaxed);
        GRAPH_REFUSALS.fetch_add(*self.graph_refusals, Relaxed);
    }
}

pub(crate) struct Observer<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    models: &'a mut Arena<Model>,
    index: PointerIndex<'a>,
    receivers: &'a mut Arena<Receiver>,
    ships: &'a mut Arena<TransportRiskShip>,
    model_refusals: &'a mut u64,
    graph_refusals: &'a mut u64,
    current: Option<usize>,
    refused: &'a mut bool,
    receiver_attempts: usize,
    ship_occurrences: usize,
    original_models: usize,
}

impl<M: Memory> OriginalRiskShips for Observer<'_, M> {
    fn begin(&mut self, receiver: u64, count: Fact<i32>) {
        self.current = None;
        self.receiver_attempts = self.receiver_attempts.saturating_add(1);
        self.receivers.required = self.receivers.required.max(self.receiver_attempts);
        if let Ok(count) = count
            && count > 0
        {
            // 140D5AE30/140D5AB80 walk every original +348/+354 SHIPS member.
            // Keep rejected occurrences in this pass, independent of admission
            // and retained demand from earlier hours.
            self.ship_occurrences = self.ship_occurrences.saturating_add(count as usize);
            self.ships.required = self.ships.required.max(self.ship_occurrences);
            self.models.required = self
                .models
                .required
                .max(self.original_models.saturating_add(self.ship_occurrences));
        }
        grow_vec(&mut self.receivers.first, self.receivers.required);
        grow_vec(&mut self.ships.first, self.ships.required);
        grow_vec(&mut self.models.first, self.models.required);
        let Some(target) = self.receivers.first.get_mut(self.receivers.len) else {
            GRAPH_REASONS.record("transport original SHIPS receiver arena exhausted");
            *self.graph_refusals += 1;
            *self.refused = true;
            return;
        };
        *target = Receiver {
            handle: receiver,
            count: count.ok(),
            count_reason: count.err(),
            first: self.ships.len,
            len: 0,
            complete: false,
        };
        self.current = Some(self.receivers.len);
        self.receivers.len += 1;
    }
    fn ship(
        &mut self,
        receiver: u64,
        ordinal: u32,
        pointer: usize,
        handle: Fact<u64>,
        class: Fact<u32>,
    ) -> Fact<()> {
        let current = self
            .current
            .ok_or("transport original SHIPS receiver arena exhausted")?;
        let row = &mut self.receivers.first[current];
        if row.handle != receiver || row.len != ordinal as usize {
            return Err("transport original SHIPS order differs");
        }
        let ship = handle?;
        let key = usize::try_from(ship).map_err(|_| "transport original ship handle overflow")?;
        let model_index = if let Some(index) = self.index.find(key) {
            index
        } else {
            let index = self.models.len;
            self.models.required = self.models.required.max(index.saturating_add(1));
            grow_vec(&mut self.models.first, self.models.required);
            let target = self
                .models
                .first
                .get_mut(index)
                .ok_or("transport original model union arena exhausted")?;
            self.index.insert(key, index)?;
            *target = Model {
                ship,
                observation: Default::default(),
            };
            self.models.len += 1;
            index
        };
        let original = self.models.first[model_index].observation;
        let observed = cship::read_transport_model(self.memory, self.base, pointer, ship, original);
        self.models.first[model_index].observation = observed;
        let identity = observed
            .identity
            .unwrap_or(Err("transport original ship model identity unattempted"));
        let target = self
            .ships
            .first
            .get_mut(self.ships.len)
            .ok_or("transport original SHIPS arena exhausted")?;
        *target = TransportRiskShip {
            ship: Some(ship),
            class: class.ok(),
            formation_flags: identity
                .and(
                    observed
                        .formation_flags
                        .unwrap_or(Err("transport formation flags unreached")),
                )
                .ok(),
            positive_equipment_flags: identity
                .and(
                    observed
                        .positive_equipment_flags
                        .unwrap_or(Err("transport positive equipment flags unreached")),
                )
                .ok(),
            carrier_air_base: identity
                .and(
                    observed
                        .carrier_air_base
                        .unwrap_or(Err("transport carrier air base unreached")),
                )
                .ok(),
        };
        let flags = target.formation_flags;
        let positive_reached = flags.is_some_and(|flags| flags & (1 << 22) == 0);
        let carrier = flags.is_some_and(|flags| flags & (1 << 22) != 0)
            || positive_reached
                && target.positive_equipment_flags.is_some_and(|positive| {
                    positive & (1 << 22) != 0 || (flags.unwrap_or(0) | positive) & (1 << 37) != 0
                });
        *self.model_refusals += u64::from(flags.is_none())
            + u64::from(positive_reached && target.positive_equipment_flags.is_none())
            + u64::from(carrier && target.carrier_air_base.is_none());
        self.ships.len += 1;
        row.len += 1;
        Ok(())
    }
    fn finish(&mut self, receiver: u64, original: Fact<()>) {
        let Some(index) = self.current.take() else {
            GRAPH_REASONS.record("transport original SHIPS receiver arena exhausted");
            *self.graph_refusals += 1;
            return;
        };
        let row = &mut self.receivers.first[index];
        row.complete = row.handle == receiver
            && original.is_ok()
            && row
                .count
                .is_some_and(|count| count >= 0 && count as usize == row.len);
        if !row.complete {
            GRAPH_REASONS.record(original.err().or(row.count_reason).unwrap_or(
                if row.handle != receiver {
                    "transport original SHIPS receiver identity differs"
                } else {
                    "transport original SHIPS count differs"
                },
            ));
            *self.graph_refusals += 1;
        }
    }
}

#[cfg(test)]
#[path = "transport_risk_ships_tests.rs"]
mod tests;
