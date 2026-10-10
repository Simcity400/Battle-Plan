//! Transient receivers of the original nullable naval ledger pass.
//! Full identity and mission-header failures are memoized by original pointer;
//! numerical projection cannot heal them with a second source read.
use crate::table_capture::{Arena, grow_capacity, grow_vec};
use engine::{Memory, ctask_force, read, transport_ledger_observation as native};
use replay::rows::TransportMissionReceiver;

type Fact<T> = Result<T, &'static str>;

#[derive(Default)]
struct Mission {
    result: Option<Fact<Option<native::Receiver>>>,
}

#[derive(Default)]
pub(crate) struct Control {
    pub pointer: usize,
    pub packet: TransportMissionReceiver,
    pub counter_attempted: bool,
}

pub(crate) struct Captured {
    identities: Arena<native::ReceiverIdentity>,
    identity_slots: Vec<native::PointerSlot>,
    missions: Arena<Mission>,
    mission_slots: Vec<native::PointerSlot>,
    pub controls: Arena<Control>,
}

impl Captured {
    pub fn new(capacity: usize) -> Self {
        let slots = if capacity == 0 {
            0
        } else {
            native::index_capacity(capacity).unwrap_or(0)
        };
        Self {
            identities: Arena::new(capacity),
            missions: Arena::new(capacity),
            controls: Arena::new(capacity),
            identity_slots: engine::allocation::owned_vec(engine::allocation::default_vec(slots)),
            mission_slots: engine::allocation::owned_vec(engine::allocation::default_vec(slots)),
        }
    }

    pub fn reset(&mut self) {
        self.retire();
        self.identities.reset();
        self.missions.reset();
        self.controls.reset();
    }

    pub fn retire(&mut self) {
        self.identities.first.fill(Default::default());
        for mission in &mut self.missions.first {
            *mission = Mission::default();
        }
        self.identity_slots.fill(Default::default());
        self.mission_slots.fill(Default::default());
        for control in &mut self.controls.first {
            control.pointer = 0;
        }
    }

    pub fn grow(&mut self, capacity: &mut usize) {
        grow_capacity(
            capacity,
            self.identities
                .required
                .max(self.missions.required)
                .max(self.controls.required),
        );
        self.identities.grow(capacity);
        self.missions.grow(capacity);
        self.controls.grow(capacity);
        if let Some(slots) = native::index_capacity(*capacity) {
            grow_vec(&mut self.identity_slots, slots);
            grow_vec(&mut self.mission_slots, slots);
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.identities.allocated_bytes()
            + self.missions.allocated_bytes()
            + self.controls.allocated_bytes()
            + (self.identity_slots.capacity() + self.mission_slots.capacity())
                * std::mem::size_of::<native::PointerSlot>()
    }

    pub fn observer<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
    ) -> Fact<Observer<'a, M>> {
        // A vacant holder owns no storage and refuses before any native read.
        if self.identities.first.is_empty() {
            return Err("transport pointer index arena exhausted");
        }
        Ok(Observer {
            memory,
            base,
            identities: native::IdentityReader::new(
                memory,
                base,
                &mut self.identities.first,
                &mut self.identity_slots,
            )?,
            mission_index: native::PointerIndex::new(
                &mut self.mission_slots,
                self.missions.first.len(),
            )?,
            missions: &mut self.missions,
            controls: &mut self.controls,
            growing: false,
        })
    }

    pub fn observer_growing<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
    ) -> Fact<Observer<'a, M>> {
        Ok(Observer {
            memory,
            base,
            identities: native::IdentityReader::new_growing(
                memory,
                base,
                &mut self.identities.first,
                &mut self.identity_slots,
            )?,
            mission_index: native::PointerIndex::new_growing(
                &mut self.mission_slots,
                self.missions.first.len(),
            )?,
            missions: &mut self.missions,
            controls: &mut self.controls,
            growing: true,
        })
    }

    pub fn finish_identities(&mut self, required: usize) {
        self.identities.required = self.identities.required.max(required);
        self.controls.first[..self.controls.len].sort_unstable_by_key(|control| control.pointer);
    }

    pub fn refuse_join(&mut self, pointer: usize) {
        if let Ok(index) = self.controls.first[..self.controls.len]
            .binary_search_by_key(&pointer, |control| control.pointer)
        {
            let control = &mut self.controls.first[index];
            control.counter_attempted = true;
            control.packet = TransportMissionReceiver {
                task_force: control.packet.task_force,
                ..Default::default()
            };
        }
    }

    pub fn supply_counter(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        handle: u64,
        original: Option<Fact<ctask_force::CounterRow>>,
    ) -> Fact<bool> {
        let Ok(index) = self.controls.first[..self.controls.len]
            .binary_search_by_key(&pointer, |control| control.pointer)
        else {
            return Ok(false);
        };
        let control = &mut self.controls.first[index];
        if control.packet.task_force != handle {
            return Err("transport original fleet and receiver full handles disagree");
        }
        if control.counter_attempted {
            return Err("transport original counter prefix attempted twice");
        }
        control.counter_attempted = true;
        match original {
            Some(original) => {
                control.packet.in_regions = original.map(|row| row.is_in_regions).ok();
                control.packet.received_supply_ratio =
                    original.map(|row| row.received_supply_ratio).ok();
            }
            None => {
                // Row capacity did not attempt either scalar. Each independent
                // native receiver branch may acquire it once, without a fake row.
                control.packet.in_regions = pointer
                    .checked_add(ctask_force::NAVAL_MISSION)
                    .and_then(|root| root.checked_add(0x30))
                    .and_then(|at| read(memory, at));
                control.packet.received_supply_ratio = pointer
                    .checked_add(engine::cunit::RECEIVED_SUPPLY_RATIO)
                    .and_then(|at| read(memory, at));
            }
        }
        Ok(true)
    }
}

pub(crate) struct Observer<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    identities: native::IdentityReader<'a, M>,
    mission_index: native::PointerIndex<'a>,
    missions: &'a mut Arena<Mission>,
    controls: &'a mut Arena<Control>,
    growing: bool,
}

impl<M: Memory> Observer<'_, M> {
    #[cfg(test)]
    pub fn observe(
        &mut self,
        mission: usize,
        original_header: Option<Fact<[u8; 16]>>,
    ) -> Fact<u64> {
        self.observe_transport(mission, original_header, None)
    }

    pub fn observe_transport(
        &mut self,
        mission: usize,
        original_header: Option<Fact<[u8; 16]>>,
        original_owner: Option<Fact<u32>>,
    ) -> Fact<u64> {
        if mission == 0 {
            return Ok(0);
        }
        if let Some(index) = self.mission_index.find(mission) {
            return self.missions.first[index]
                .result
                .expect("original mission observed")
                .map(|receiver| receiver.map_or(0, |receiver| receiver.full_handle));
        }
        self.missions.required = self
            .missions
            .required
            .max(self.missions.len.saturating_add(1));
        let index = self.missions.len;
        if self.growing {
            engine::walk::Buffer::Growing(&mut self.missions.first).ensure(index + 1)?;
        }
        if index >= self.missions.first.len() {
            return Err("transport mission observation arena exhausted");
        }
        self.mission_index.insert(mission, index)?;
        let header = original_header.unwrap_or_else(|| {
            read::<[u8; 16]>(self.memory, mission)
                .ok_or("transport original mission header unreadable")
        });
        let mut is_new = false;
        let result = native::receiver(
            self.base,
            mission,
            header
                .as_ref()
                .map(|bytes| bytes.as_slice())
                .map_err(|reason| *reason),
            |pointer| {
                is_new = self.identities.find(pointer).is_none();
                self.identities.observe(pointer)
            },
        );
        if let Ok(Some(receiver)) = result
            && is_new
        {
            self.controls.required = self.controls.required.saturating_add(1);
            if self.growing {
                engine::walk::Buffer::Growing(&mut self.controls.first)
                    .ensure(self.controls.len + 1)?;
            }
            if let Some(row) = self.controls.first.get_mut(self.controls.len) {
                *row = Control {
                    pointer: receiver.pointer,
                    packet: TransportMissionReceiver {
                        task_force: receiver.full_handle,
                        mission_owner: original_owner
                            .unwrap_or_else(|| {
                                mission
                                    .checked_add(engine::naval_mission_inputs::MISSION_OWNER)
                                    .and_then(|at| read(self.memory, at))
                                    .ok_or("transport mission owner unreadable")
                            })
                            .ok(),
                        ..Default::default()
                    },
                    counter_attempted: false,
                };
                self.controls.len += 1;
            } else {
                self.missions.first[index] = Mission {
                    result: Some(Err("transport receiver control arena exhausted")),
                };
                self.missions.len += 1;
                return Err("transport receiver control arena exhausted");
            }
        }
        self.missions.first[index] = Mission {
            result: Some(result),
        };
        self.missions.len += 1;
        result.map(|receiver| receiver.map_or(0, |receiver| receiver.full_handle))
    }

    pub fn required(&self) -> usize {
        self.identities.required()
    }
}

#[cfg(test)]
#[path = "transport_receivers_tests.rs"]
mod tests;
