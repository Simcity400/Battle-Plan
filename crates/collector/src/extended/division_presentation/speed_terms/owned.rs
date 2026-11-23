use super::*;
use crate::acquisition::lock_unpoisoned;
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Condvar,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
};

struct Slot {
    state: Mutex<State>,
    returned: Condvar,
    generation: AtomicU64,
}
enum State {
    Vacant,
    Ready(Held),
    Loaned,
}
static SLOT: Slot = Slot {
    state: Mutex::new(State::Vacant),
    returned: Condvar::new(),
    generation: AtomicU64::new(0),
};

pub(super) fn forget() {
    let mut state = lock_unpoisoned(&SLOT.state);
    SLOT.generation.fetch_add(1, Relaxed);
    if let State::Ready(held) = &mut *state {
        held.clear();
    }
}

#[derive(Default)]
pub(super) struct Loan {
    value: Held,
    owner: Option<&'static Slot>,
    complete: bool,
    generation: u64,
}
impl Loan {
    pub(super) fn take() -> Result<Self, &'static str> {
        let mut state = lock_unpoisoned(&SLOT.state);
        while matches!(*state, State::Loaned) {
            if crate::allocation::failed() || crate::decode::panic_failure().is_some() {
                return Err("unit speed preceding settlement unavailable");
            }
            state = SLOT
                .returned
                .wait_timeout(state, std::time::Duration::from_millis(10))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        let value = match std::mem::replace(&mut *state, State::Loaned) {
            State::Vacant => Held::default(),
            State::Ready(value) => value,
            State::Loaned => unreachable!("one unit speed store loan"),
        };
        Ok(Self {
            value,
            owner: Some(&SLOT),
            complete: false,
            generation: SLOT.generation.load(Relaxed),
        })
    }
    pub(super) fn return_complete(&mut self) {
        if self.owner.is_some() {
            self.complete = true;
            drop(std::mem::take(self));
        }
    }
    pub(super) fn abandon(&mut self) {
        if self.owner.is_some() {
            drop(std::mem::take(self));
        }
    }
    #[cfg(test)]
    pub(super) fn replace(&mut self, value: Held) {
        self.value = value;
    }
}
impl Deref for Loan {
    type Target = Held;
    fn deref(&self) -> &Held {
        &self.value
    }
}
impl DerefMut for Loan {
    fn deref_mut(&mut self) -> &mut Held {
        &mut self.value
    }
}
impl Drop for Loan {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.take() {
            let mut state = lock_unpoisoned(&owner.state);
            if !self.complete || self.generation != owner.generation.load(Relaxed) {
                self.value.clear();
            }
            assert!(matches!(*state, State::Loaned), "one unit speed store loan");
            *state = State::Ready(std::mem::take(&mut self.value));
            owner.returned.notify_one();
        }
    }
}

pub(super) struct Country {
    pub owner: u32,
    pub country: Result<ccountry::Country, &'static str>,
    pub held: Option<HeldCountry>,
    pub start: usize,
    pub end: usize,
    pub read: Result<usize, &'static str>,
}

#[derive(Clone, Copy)]
pub(super) struct StateInputs {
    pub branch: u8,
    pub army: Value,
    pub relation: Option<Value>,
    pub attack: Result<Value, &'static str>,
}
impl StateInputs {
    fn terms(self) -> StateMovementTerms {
        StateMovementTerms {
            army_speed: self.army.present.then_some(self.army.raw),
            relation_speed: self
                .relation
                .and_then(|value| value.present.then_some(value.raw)),
            attack_speed: self
                .attack
                .ok()
                .and_then(|value| value.present.then_some(value.raw)),
            branch: self.branch
                | if self.attack.is_err() {
                    StateMovementTerms::ATTACK_REFUSED
                } else {
                    0
                },
        }
    }
}

#[derive(Default)]
#[allow(clippy::type_complexity)]
pub(super) struct Rows {
    pub states: Vec<((u32, u32, u32), Result<StateInputs, &'static str>)>,
    pub access: Vec<(u32, u32)>,
    pub access_areas: Vec<((u32, usize), Result<bool, &'static str>)>,
    pub access_ready: bool,
    pub rivers: Vec<((u32, u32), Result<Option<i64>, &'static str>)>,
    pub transport: Vec<(
        (u32, u32),
        Result<engine_naval_history::cnaval_unit_transfer::SpeedInputs, &'static str>,
    )>,
}
impl Rows {
    pub(super) fn clear(&mut self) {
        self.states.clear();
        self.access.clear();
        self.access_areas.clear();
        self.access_ready = false;
        self.rivers.clear();
        self.transport.clear();
    }
    pub(super) fn bytes(&self) -> usize {
        self.states.capacity() * size_of::<((u32, u32, u32), Result<StateInputs, &'static str>)>()
            + self.access.capacity() * size_of::<(u32, u32)>()
            + self.access_areas.capacity() * size_of::<((u32, usize), Result<bool, &'static str>)>()
            + self.rivers.capacity() * size_of::<((u32, u32), Result<Option<i64>, &'static str>)>()
            + self.transport.capacity()
                * size_of::<(
                    (u32, u32),
                    Result<engine_naval_history::cnaval_unit_transfer::SpeedInputs, &'static str>,
                )>()
    }
}

#[derive(Default)]
pub(super) struct Entries {
    pub pairs: Vec<(u32, usize, usize)>,
    pub wanted: Vec<(u32, u32)>,
    pub countries: Vec<Country>,
    pub values: Vec<Result<Value, &'static str>>,
    pub next: Held,
    pub pending: bool,
    pub epoch: Option<u64>,
    pub arrays: Option<[Array; 2]>,
    pub complete: bool,
    pub counts: Counts,
    pub result: Option<Result<(), &'static str>>,
}
impl Entries {
    pub(super) fn clear(&mut self) {
        self.pairs.clear();
        self.wanted.clear();
        self.countries.clear();
        self.values.clear();
        self.next.clear();
        self.pending = false;
        self.result = None;
    }
    pub(super) fn bytes(&self) -> usize {
        self.pairs.capacity() * size_of::<(u32, usize, usize)>()
            + self.wanted.capacity() * size_of::<(u32, u32)>()
            + self.countries.capacity() * size_of::<Country>()
            + self.values.capacity() * size_of::<Result<Value, &'static str>>()
            + self.next.bytes()
    }
}

impl SpeedTerms {
    pub(super) fn settle_family(&mut self, slot: usize) {
        let count = |counts: &mut [u64; 3], failed: bool| counts[usize::from(failed) * 2] += 1;
        match slot {
            0 => {
                for ((state, roster, owner), inputs) in self.observed_rows.states.drain(..) {
                    count(&mut self.counts[0], inputs.is_err());
                    if inputs.is_ok_and(|inputs| inputs.attack.is_err()) {
                        self.counts[0][2] += 1;
                    }
                    self.states.push(MovementStateTermRow {
                        state,
                        roster,
                        owner,
                        terms: inputs.ok().map(StateInputs::terms),
                    });
                }
            }
            1 => {
                for (owner, province) in self.observed_rows.access.drain(..) {
                    if !self.observed_rows.access_ready {
                        continue;
                    }
                    let access = super::province_at(&self.provinces, &self.province_slot, province)
                        .and_then(|facts| {
                            memo(
                                &self.observed_rows.access_areas,
                                (owner, facts.controller_area?),
                            )
                            .ok_or("core supply area unread")?
                        });
                    count(&mut self.counts[1], access.is_err());
                    self.access.push(CoreSupplyAccessRow {
                        owner,
                        province,
                        access: access.ok(),
                    });
                }
                self.observed_rows.access_areas.clear();
                self.observed_rows.access_ready = false;
            }
            3 => {
                for ((owner, controller), entry) in self.observed_rows.rivers.drain(..) {
                    count(&mut self.counts[3], entry.is_err());
                    self.rivers.push(RiverRelationRow {
                        owner,
                        controller,
                        entry: entry.ok(),
                    });
                }
            }
            4 => {
                for ((owner, region), speed) in self.observed_rows.transport.drain(..) {
                    count(&mut self.counts[4], speed.is_err());
                    self.transport.push(OwnerTransportSpeedRow {
                        owner,
                        region,
                        speed: speed
                            .ok()
                            .map(engine_naval_history::cnaval_unit_transfer::SpeedInputs::evaluate),
                    });
                }
            }
            _ => unreachable!("unit speed entries have an ordered settlement"),
        }
    }

    pub(super) fn settle_entries(&mut self) -> Outcome {
        let observed = &mut self.observed_entries;
        if !std::mem::take(&mut observed.pending) {
            return self.outcomes[2];
        }
        let next = &mut observed.next;
        let mut holdable =
            observed.complete && observed.epoch.is_some() && observed.arrays.is_some();
        let mut carried_countries = 0;
        let mut counts = std::mem::take(&mut observed.counts);
        let result = (|| {
            let acquired = observed
                .result
                .take()
                .expect("unit speed acquisition result");
            for group in &observed.countries {
                let country = group.country;
                let held = group.held;
                let wanted = &observed.wanted[group.start..group.end];
                let read = group.read.map(|start| {
                    &observed.values[start..start + if held.is_some() { 0 } else { wanted.len() }]
                });
                let held_value = |held: HeldCountry, key: u32| {
                    let keys = &self.held.keys[held.start..held.start + held.len];
                    keys.binary_search(&key)
                        .ok()
                        .map(|at| self.held.values[held.start + at])
                };
                if held.is_some() {
                    carried_countries += 1;
                } else {
                    counts.read += 1;
                }
                let start = next.keys.len();
                for (index, &(_, key)) in wanted.iter().enumerate() {
                    let entry = match (held, &read) {
                        (Some(held), _) => {
                            Some(held_value(held, key).ok_or("held unit speed entry missing")?)
                        }
                        (None, Ok(fresh)) => fresh[index]
                            .ok()
                            .map(|value| value.present.then_some(value.raw)),
                        (None, Err(_)) => None,
                    };
                    self.counts[2][if entry.is_none() {
                        2
                    } else if held.is_some() {
                        1
                    } else {
                        0
                    }] += 1;
                    self.entries.push(CountryUnitSpeedEntryRow {
                        country: group.owner,
                        key,
                        entry,
                    });
                    if holdable {
                        match entry {
                            Some(value)
                                if reserve(&mut next.keys, 1).is_ok()
                                    && reserve(&mut next.values, 1).is_ok() =>
                            {
                                next.keys.push(key);
                                next.values.push(value);
                            }
                            Some(_) => holdable = false,
                            None => {}
                        }
                    }
                }
                let len = next.keys.len() - start;
                match country {
                    Ok(country) if holdable && len == wanted.len() => {
                        if reserve(&mut next.countries, 1).is_ok() {
                            next.countries.push(HeldCountry {
                                owner: group.owner,
                                country,
                                start,
                                len,
                            });
                        } else {
                            holdable = false;
                        }
                    }
                    _ => {
                        next.keys.truncate(start);
                        next.values.truncate(start);
                    }
                }
            }
            acquired.map(|()| self.entries.len())
        })();
        if result.is_ok() && holdable {
            next.epoch = observed.epoch;
            next.arrays = observed.arrays.unwrap_or_default();
            next.template_token = self.held.template_token;
            std::mem::swap(&mut next.template_spans, &mut self.held.template_spans);
            std::mem::swap(&mut next.template_keys, &mut self.held.template_keys);
            std::mem::swap(&mut *self.held, next);
            next.clear();
        } else {
            self.held.clear();
        }
        changes::record(Table::UnitSpeeds, &counts, None);
        changes::record_carried(Table::UnitSpeeds, carried_countries);
        self.finish(result, 2)
    }
}
