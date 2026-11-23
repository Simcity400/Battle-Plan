
use super::{DivisionSeed, Row, capture_division_seeds, walk, walk_with};
use engine_native_memory::{Memory, msvc_string, read, walk::Outcome};
use std::cell::RefCell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PresentationDemand {
    pub rows: usize,
    pub cached_name_text: usize,
    pub supply_origins: usize,
    pub failure: Option<&'static str>,
}

impl PresentationDemand {
    pub fn max(self, other: Self) -> Self {
        Self {
            rows: self.rows.max(other.rows),
            cached_name_text: self.cached_name_text.max(other.cached_name_text),
            supply_origins: self.supply_origins.max(other.supply_origins),
            failure: other.failure,
        }
    }
}

struct Headers<'a, M> {
    memory: &'a M,
    observed: RefCell<Vec<(usize, [u8; 16])>>,
    strings: RefCell<Vec<(usize, [u8; msvc_string::STRIDE])>>,
}

impl<M: Memory> Memory for Headers<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if !self.memory.copy(address, output) {
            return false;
        }
        if let Ok(header) = <[u8; 16]>::try_from(&*output) {
            let mut observed = self.observed.borrow_mut();
            if observed.try_reserve(1).is_err() {
                return false;
            }
            observed.push((address, header));
        } else if let Ok(header) = <[u8; msvc_string::STRIDE]>::try_from(&*output) {
            let mut strings = self.strings.borrow_mut();
            if strings.try_reserve(1).is_err() {
                return false;
            }
            strings.push((address, header));
        }
        true
    }
}

impl<M: Memory> Headers<'_, M> {
    fn validate(&self) -> Result<(), &'static str> {
        for &(address, header) in &*self.observed.borrow() {
            if read::<[u8; 16]>(self.memory, address) != Some(header) {
                return Err("division presentation demand selecting header changed");
            }
        }
        for &(address, header) in &*self.strings.borrow() {
            if read::<[u8; msvc_string::STRIDE]>(self.memory, address) != Some(header) {
                return Err("division presentation demand string header changed");
            }
        }
        Ok(())
    }
}

fn storage<T: Default + Clone>(count: usize) -> Result<Vec<T>, &'static str> {
    let mut storage = Vec::new();
    storage
        .try_reserve_exact(count)
        .map_err(|_| "division presentation sizing allocation skipped")?;
    storage.resize(count, T::default());
    Ok(storage)
}

fn add(total: &mut usize, count: usize) -> Result<(), &'static str> {
    *total = total
        .checked_add(count)
        .ok_or("division presentation demand overflow")?;
    Ok(())
}

pub fn presentation_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<PresentationDemand, &'static str> {
    engine_allocation::advisory(|| presentation_demand_advisory(memory, base, state))
}

fn presentation_demand_advisory(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<PresentationDemand, &'static str> {
    let count = match walk(memory, base, state, &mut []) {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("division presentation demand row overflow")?,
        Outcome::Unavailable { reason } => return Err(reason),
    };
    let headers = Headers {
        memory,
        observed: RefCell::default(),
        strings: RefCell::default(),
    };
    let mut rows = storage::<Row>(count)?;
    let mut seeds = storage::<DivisionSeed>(count)?;
    let mut visited = 0;
    let outcome = walk_with(&headers, base, state, &mut rows, |pointer, row| {
        seeds[visited] = DivisionSeed {
            pointer,
            division: row.id,
            ..Default::default()
        };
        visited += 1;
    });
    match outcome {
        Outcome::Rows { len } if len == visited => (),
        Outcome::Rows { .. } | Outcome::Truncated { .. } => {
            return Err("division presentation demand army census changed");
        }
        Outcome::Unavailable { reason } => return Err(reason),
    }
    seeds.truncate(visited);
    capture_division_seeds(&headers, &mut seeds);
    let mut demand = PresentationDemand {
        rows: visited,
        ..Default::default()
    };
    for seed in &seeds {
        if let Some(member) = seed.name_member.filter(|member| *member != 0) {
            match crate::cname_group_member::cached_name_demand(&headers, base, member) {
                Ok(bytes) => add(&mut demand.cached_name_text, bytes)?,
                Err(reason) => {
                    demand.failure.get_or_insert(reason);
                }
            }
        }
        match engine_native_parts::cunit_supply_inputs::origin_demand(&headers, seed.pointer) {
            Ok(count) => add(&mut demand.supply_origins, count)?,
            Err(reason) => {
                demand.failure.get_or_insert(reason.reason());
            }
        }
    }
    headers.validate()?;
    Ok(demand)
}
