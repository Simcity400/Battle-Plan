use super::*;
use collector_division_parts::{
    attack_stats::{ArmyStatStore, ScalarOrder},
    supply_inputs::Sample,
};

#[derive(Default)]
pub(super) struct DailyStatus {
    enabled: bool,
    samples: Vec<Option<Arc<Sample>>>,
    requested: Vec<bool>,
    order: ScalarOrder,
}
impl DailyStatus {
    pub(super) fn reset(&mut self) {
        self.enabled = false;
        self.samples.clear();
        self.requested.clear();
        self.order.clear();
    }
    pub(super) fn allocated_bytes(&self) -> usize {
        self.samples.capacity() * std::mem::size_of::<Option<Arc<Sample>>>()
            + self.requested.capacity() * std::mem::size_of::<bool>()
            + self.order.allocated_bytes()
    }
    pub(super) fn pending(&self) -> bool {
        self.enabled
    }
    pub(super) fn prepare(
        &mut self,
        packet: &mut carmy::StatusPacket,
        store: Option<&mut ArmyStatStore>,
        base: usize,
        seeds: &[DivisionSeed],
        staged: impl Fn(usize) -> Option<Option<Arc<Sample>>>,
    ) {
        self.reset();
        let _ = packet.select_supply(&[]);
        let Some(store) = store.filter(|store| store.holds_daily_progress()) else {
            return;
        };
        let prepared = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.samples, seeds.len())?;
            engine_allocation::reserve(&mut self.requested, seeds.len())?;
            Ok::<_, std::io::Error>(())
        })
        .is_ok();
        if prepared {
            for (index, seed) in seeds.iter().enumerate() {
                let sample = staged(index)
                    .unwrap_or_else(|| store.daily_supply(base, index, seed, &mut self.order));
                self.requested.push(sample.is_none());
                self.samples.push(sample);
            }
        }
        if !prepared
            || engine_allocation::advisory(|| packet.select_supply(&self.requested)).is_err()
        {
            self.reset();
            store.forget_supply();
            return;
        }
        self.enabled = true;
    }
    pub(super) fn settle(
        &mut self,
        packet: &carmy::StatusPacket,
        store: &mut ArmyStatStore,
        base: usize,
        seeds: &[DivisionSeed],
        complete: bool,
    ) {
        if !self.enabled {
            return;
        }
        for (index, sample) in self.samples.iter_mut().enumerate() {
            if sample.is_none() {
                let (inputs, failure, origins) = packet.supply_at(index);
                *sample =
                    engine_allocation::advisory(|| Sample::owned(inputs, failure, origins)).ok();
            }
        }
        engine_allocation::advisory(|| store.remember_supply(base, seeds, &self.samples, complete));
    }
    pub(super) fn sample(&self, index: usize) -> Option<&Sample> {
        self.samples.get(index)?.as_deref()
    }
}
