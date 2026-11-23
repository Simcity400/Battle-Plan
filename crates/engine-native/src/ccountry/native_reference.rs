
use std::{
    cell::{Cell, RefCell},
    io,
};

use super::{AuthenticatedRoster, Country, Reader};
use engine_native_memory::{Memory, cpdx_array::Array};
use engine_allocation as allocation;

type CountryResult = Result<Country, &'static str>;
type SlotResult = Result<Option<Country>, &'static str>;
type LinkResult = Result<u32, &'static str>;

struct Results<T: Copy> {
    entries: Vec<Option<(u32, T)>>,
    capacity: usize,
    used: usize,
    required: usize,
    short: bool,
}

impl<T: Copy> Results<T> {
    const fn vacant() -> Self {
        Self {
            entries: Vec::new(),
            capacity: 0,
            used: 0,
            required: 0,
            short: false,
        }
    }

    fn new(capacity: usize) -> io::Result<Self> {
        let cells = if capacity == 0 {
            0
        } else {
            allocation::mul(capacity, 2)?
                .checked_next_power_of_two()
                .ok_or_else(allocation::oom)?
        };
        Ok(Self {
            entries: allocation::filled(cells, None)?,
            capacity,
            used: 0,
            required: 0,
            short: false,
        })
    }

    fn index(&self, key: u32) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let mask = self.entries.len() - 1;
        let mut index = (key as usize).wrapping_mul(0x9e37_79b1) & mask;
        for _ in 0..self.entries.len() {
            if self.entries[index].is_none_or(|(stored, _)| stored == key) {
                return Some(index);
            }
            index = (index + 1) & mask;
        }
        None
    }

    fn get(&self, key: u32) -> Option<T> {
        self.index(key)
            .and_then(|index| self.entries[index].map(|(_, value)| value))
    }

    fn admit(&mut self) -> bool {
        self.required = self.required.max(self.used.saturating_add(1));
        if self.used < self.capacity {
            true
        } else {
            if !self.short {
                let grown = (|| {
                    let capacity = allocation::mul(self.capacity.max(1), 2)?;
                    let mut grown = Self::new(capacity)?;
                    for (key, value) in self.entries.iter().flatten().copied() {
                        grown.insert(key, value);
                    }
                    grown.required = self.required;
                    Ok::<_, io::Error>(grown)
                })();
                if let Ok(grown) = grown {
                    *self = grown;
                    return true;
                }
            }
            self.short = true;
            false
        }
    }

    fn insert(&mut self, key: u32, value: T) {
        let index = self
            .index(key)
            .expect("an admitted result has an empty cell");
        if self.entries[index].is_none() {
            self.used += 1;
        }
        self.entries[index] = Some((key, value));
    }

    fn clear(&mut self) {
        self.entries.fill(None);
        self.used = 0;
        self.required = 0;
        self.short = false;
    }

    fn heap_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Option<(u32, T)>>()
    }
}

pub struct NativeReferenceMemo {
    identity: Option<(usize, usize)>,
    reserved: Cell<Option<super::ReservedCountryObservation>>,
    positive: Cell<Option<Result<[Array; 2], &'static str>>>,
    raw: RefCell<Results<CountryResult>>,
    canonical: RefCell<Results<SlotResult>>,
    links: RefCell<Vec<Option<LinkResult>>>,
    link_used: Cell<usize>,
    link_required: Cell<usize>,
}

impl Default for NativeReferenceMemo {
    fn default() -> Self {
        Self::vacant()
    }
}

impl NativeReferenceMemo {
    pub const fn vacant() -> Self {
        Self {
            identity: None,
            reserved: Cell::new(None),
            positive: Cell::new(None),
            raw: RefCell::new(Results::vacant()),
            canonical: RefCell::new(Results::vacant()),
            links: RefCell::new(Vec::new()),
            link_used: Cell::new(0),
            link_required: Cell::new(0),
        }
    }

    pub fn try_with_capacity(raw_references: usize, links: usize) -> io::Result<Self> {
        Ok(Self {
            identity: None,
            reserved: Cell::new(None),
            positive: Cell::new(None),
            raw: RefCell::new(Results::new(raw_references)?),
            canonical: RefCell::new(Results::new(raw_references)?),
            links: RefCell::new(allocation::filled(links, None)?),
            link_used: Cell::new(0),
            link_required: Cell::new(0),
        })
    }

    fn clear(&mut self) {
        self.identity = None;
        self.reserved.set(None);
        self.positive.set(None);
        self.raw.get_mut().clear();
        self.canonical.get_mut().clear();
        self.links.get_mut().fill(None);
        self.link_used.set(0);
        self.link_required.set(0);
    }

    pub fn reset(&mut self, base: usize, game_state: usize) {
        self.clear();
        self.identity = Some((base, game_state));
    }

    pub fn reserve(&mut self, raw_references: usize, links: usize) -> io::Result<()> {
        self.clear();
        if raw_references > self.raw.get_mut().capacity {
            let raw = Results::new(raw_references)?;
            let canonical = Results::new(raw_references)?;
            *self.raw.get_mut() = raw;
            *self.canonical.get_mut() = canonical;
        }
        if links > self.links.get_mut().len() {
            allocation::resize_default(self.links.get_mut(), links)?;
        }
        Ok(())
    }

    pub fn view<'a, M: Memory>(
        &'a self,
        memory: &'a M,
        base: usize,
        game_state: usize,
    ) -> NativeReferenceContext<'a, M> {
        NativeReferenceContext {
            memo: self,
            memory,
            base,
            game_state,
        }
    }

    pub fn raw_capacity(&self) -> usize {
        self.raw.borrow().capacity
    }
    pub fn raw_used(&self) -> usize {
        self.raw.borrow().used
    }
    pub fn raw_required(&self) -> usize {
        self.raw.borrow().required
    }
    pub fn raw_required_is_lower_bound(&self) -> bool {
        self.raw.borrow().short
    }
    pub fn canonical_used(&self) -> usize {
        self.canonical.borrow().used
    }
    pub fn link_capacity(&self) -> usize {
        self.links.borrow().len()
    }
    pub fn link_used(&self) -> usize {
        self.link_used.get()
    }
    pub fn link_required(&self) -> usize {
        self.link_required.get()
    }

    pub fn allocated_bytes(&self) -> usize {
        self.raw.borrow().heap_bytes()
            + self.canonical.borrow().heap_bytes()
            + self.links.borrow().capacity() * size_of::<Option<LinkResult>>()
    }
}

pub struct NativeReferenceContext<'a, M: Memory> {
    memo: &'a NativeReferenceMemo,
    memory: &'a M,
    base: usize,
    game_state: usize,
}

impl<'a, M: Memory> NativeReferenceContext<'a, M> {
    pub fn memory(&self) -> &'a M {
        self.memory
    }

    fn check(&self) -> Result<(), &'static str> {
        if self.memo.identity != Some((self.base, self.game_state)) {
            return Err("native country references belong to a different pass context");
        }
        Ok(())
    }

    fn reserved(&self) -> CountryResult {
        self.reserved_observation().result()
    }

    fn reserved_observation(&self) -> super::ReservedCountryObservation {
        if let Err(reason) = self.check() {
            return super::ReservedCountryObservation::from_result(Err(reason));
        }
        if let Some(observation) = self.memo.reserved.get() {
            return observation;
        }
        let observation =
            super::cached_reserved_observation(self.memory, self.base, self.game_state)
                .unwrap_or_else(|| {
                    super::ReservedCountryObservation::from_result(Err(
                        "native country reference lacks shared reserved source",
                    ))
                });
        let observation = observation.with_result(observation.result().and_then(|country| {
            if country.pointer == 0 || country.canonical_slot != 0 || country.id != 0 {
                Err("native country reserved source is not physical slot zero")
            } else {
                Ok(country)
            }
        }));
        self.memo.reserved.set(Some(observation));
        observation
    }

    fn arrays(&self) -> Result<[Array; 2], &'static str> {
        if let Some(result) = self.memo.positive.get() {
            return result;
        }
        let memory = ObservedLinks::new(self, None);
        let result = Reader::new(&memory, self.base, self.game_state).map(|reader| reader.arrays());
        if let Ok([_, links]) = result {
            self.memo.link_required.set(links.count as usize);
        }
        let result = result.and_then(|arrays| {
            let mut links = self.memo.links.borrow_mut();
            let required = arrays[1].count as usize;
            if required > links.len() {
                allocation::resize_default(&mut links, required)
                    .map_err(|_| "native country link observations do not fit arena")?;
            }
            Ok(arrays)
        });
        self.memo.positive.set(Some(result));
        result
    }

    pub fn canonical_slot(&self, raw: u32) -> LinkResult {
        self.check()?;
        if raw == 0 {
            return Ok(0);
        }
        if (raw as i32) < 0 {
            return Err("native country reference has a negative signed index");
        }
        let arrays = self.arrays()?;
        let [countries, links] = arrays;
        links
            .element(raw, 4)
            .ok_or("country link unreadable or outside country table")?;
        let stored = self
            .memo
            .links
            .borrow()
            .get(raw as usize)
            .copied()
            .ok_or("native country link observations do not fit arena")?;
        let result = if let Some(result) = stored {
            result
        } else {
            let memory = ObservedLinks::new(self, Some(links));
            let result = Reader::from_arrays(&memory, self.base, self.game_state, arrays)
                .and_then(|reader| reader.link(raw).map(|slot| slot.unwrap_or(0)));
            let result = memory.result(result);
            let mut entries = self.memo.links.borrow_mut();
            if entries[raw as usize].is_none() {
                entries[raw as usize] = Some(result);
                self.memo.link_used.set(self.memo.link_used.get() + 1);
            }
            result
        };
        let slot = result?;
        if slot >= countries.count {
            return Err("country link unreadable or outside country table");
        }
        Ok(slot)
    }

    pub fn resolve(&self, raw: u32) -> CountryResult {
        self.check()?;
        if raw == 0 {
            return self.reserved();
        }
        if let Some(result) = self.memo.raw.borrow().get(raw) {
            return result;
        }
        if !self.memo.raw.borrow_mut().admit() {
            return Err("native country raw results do not fit arena");
        }
        let result = (|| {
            let slot = self.canonical_slot(raw)?;
            if slot == 0 {
                return self.reserved();
            }
            let observed = self.memo.canonical.borrow().get(slot);
            let country = if let Some(result) = observed {
                result
            } else {
                if !self.memo.canonical.borrow_mut().admit() {
                    return Err("native country canonical results do not fit arena");
                }
                let arrays = self.arrays()?;
                let memory = ObservedLinks::new(self, Some(arrays[1]));
                let result = Reader::from_arrays(&memory, self.base, self.game_state, arrays)
                    .and_then(|reader| reader.at(slot));
                let result = memory.result(result);
                self.memo.canonical.borrow_mut().insert(slot, result);
                result
            }?;
            let country = country.ok_or("country link resolves to null country")?;
            if country.canonical_slot != slot {
                return Err("country link does not resolve to its canonical object");
            }
            Ok(country)
        })();
        self.memo.raw.borrow_mut().insert(raw, result);
        result
    }
}

struct ObservedLinks<'c, 'a, M: Memory> {
    context: &'c NativeReferenceContext<'a, M>,
    links: Option<Array>,
    refusal: Cell<Option<&'static str>>,
}

impl<'c, 'a, M: Memory> ObservedLinks<'c, 'a, M> {
    fn new(context: &'c NativeReferenceContext<'a, M>, links: Option<Array>) -> Self {
        Self {
            context,
            links,
            refusal: Cell::new(None),
        }
    }

    fn result<T>(&self, result: Result<T, &'static str>) -> Result<T, &'static str> {
        result.map_err(|reason| self.refusal.get().unwrap_or(reason))
    }
}

impl<M: Memory> Memory for ObservedLinks<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let index = self.links.and_then(|links| {
            let offset = address.checked_sub(links.data)?;
            (destination.len() == 4 && offset % 4 == 0 && offset / 4 < links.count as usize)
                .then_some(offset / 4)
        });
        let Some(index) = index else {
            return self.context.memory.copy(address, destination);
        };
        let result = {
            let mut entries = self.context.memo.links.borrow_mut();
            let Some(cell) = entries.get_mut(index) else {
                self.refusal
                    .set(Some("native country link observations do not fit arena"));
                return false;
            };
            if let Some(result) = *cell {
                result
            } else {
                let result = engine_native_memory::read::<u32>(self.context.memory, address)
                    .ok_or("country link unreadable or outside country table");
                *cell = Some(result);
                self.context
                    .memo
                    .link_used
                    .set(self.context.memo.link_used.get() + 1);
                result
            }
        };
        match result {
            Ok(word) => {
                destination.copy_from_slice(&word.to_le_bytes());
                true
            }
            Err(reason) => {
                self.refusal.set(Some(reason));
                false
            }
        }
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<AuthenticatedRoster<'_>> {
        self.context
            .memory
            .authenticated_countries(base, game_state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<CountryResult> {
        Some(
            if (base, game_state) == (self.context.base, self.context.game_state) {
                self.context.reserved()
            } else {
                Err("native country reserved source belongs to a different context")
            },
        )
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<super::ReservedCountryObservation> {
        Some(
            if (base, game_state) == (self.context.base, self.context.game_state) {
                self.context.reserved_observation()
            } else {
                super::ReservedCountryObservation::from_result(Err(
                    "native country reserved source belongs to a different context",
                ))
            },
        )
    }
}

#[cfg(test)]
mod tests;
