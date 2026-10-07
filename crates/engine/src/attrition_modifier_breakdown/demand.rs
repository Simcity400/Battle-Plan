//! Off-hour sizing from the existing authenticated army census and exact
//! attrition source requests. Trees and string headers are read, never display
//! payloads. A changed selecting header discards this hint.

use super::{Buffers, Frame, RawNode, Request, Root, StackLimits, capture_root};
use crate::{Memory, carmy, carmy_attrition, msvc_string, read, walk::Outcome};
use std::cell::RefCell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
/// Minimum capacities from the known independent sections of one authenticated
/// census. A local native fault leaves only that section's remaining demand
/// unknown; these bounds never certify capture availability or a zero value.
pub struct Demand {
    pub rows: usize,
    pub terrain_text: usize,
    pub cached_name_text: usize,
    /// Successful origin-header demand only; a failed hint never records zero.
    pub supply_origins: usize,
    pub requests: usize,
    pub roots: usize,
    pub nodes: usize,
    pub text: usize,
    pub depth: usize,
}

impl Demand {
    pub fn max(self, other: Self) -> Self {
        Self {
            rows: self.rows.max(other.rows),
            terrain_text: self.terrain_text.max(other.terrain_text),
            cached_name_text: self.cached_name_text.max(other.cached_name_text),
            supply_origins: self.supply_origins.max(other.supply_origins),
            requests: self.requests.max(other.requests),
            roots: self.roots.max(other.roots),
            nodes: self.nodes.max(other.nodes),
            text: self.text.max(other.text),
            depth: self.depth.max(other.depth),
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
                return Err("attrition demand selecting header changed");
            }
        }
        for &(address, header) in &*self.strings.borrow() {
            if read::<[u8; msvc_string::STRIDE]>(self.memory, address) != Some(header) {
                return Err("attrition demand string header changed");
            }
        }
        Ok(())
    }
}

fn storage<T: Default + Clone>(count: usize) -> Result<Vec<T>, &'static str> {
    let mut storage = Vec::new();
    // A sizing hint may be dropped without terminally failing a recording.
    // Actual packet allocation uses the collector's checked allocation path.
    storage
        .try_reserve_exact(count)
        .map_err(|_| "attrition sizing allocation skipped")?;
    storage.resize(count, T::default());
    Ok(storage)
}

pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Result<Demand, &'static str> {
    let count = match carmy::walk(memory, base, state, &mut []) {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("attrition demand row overflow")?,
        Outcome::Unavailable { reason } => return Err(reason),
    };
    let headers = Headers {
        memory,
        observed: RefCell::default(),
        strings: RefCell::default(),
    };
    let mut rows = storage::<carmy::Row>(count)?;
    let mut seeds = storage::<carmy::DivisionSeed>(count)?;
    let mut visited = 0;
    let outcome = carmy::walk_with(&headers, base, state, &mut rows, |pointer, row| {
        seeds[visited] = carmy::DivisionSeed {
            pointer,
            division: row.id,
            ..Default::default()
        };
        visited += 1;
    });
    match outcome {
        Outcome::Rows { len } if len == visited => (),
        Outcome::Rows { .. } | Outcome::Truncated { .. } => {
            return Err("attrition demand army census changed");
        }
        Outcome::Unavailable { reason } => return Err(reason),
    }
    seeds.truncate(visited);
    carmy::capture_division_seeds(&headers, &mut seeds);
    let maximum_requests = visited
        .checked_mul(carmy_attrition::SOURCE_COUNT)
        .ok_or("attrition demand request overflow")?;
    let mut requests = storage::<Request>(maximum_requests)?;
    let mut request_len = 0;
    let mut demand = Demand {
        rows: visited,
        ..Default::default()
    };
    for seed in &seeds {
        if let Some(member) = seed.name_member.filter(|member| *member != 0)
            && let Ok(bytes) = crate::cname_group_member::cached_name_demand(&headers, base, member)
        {
            super::add(&mut demand.cached_name_text, bytes)?;
        }
        if let Ok(count) = crate::cunit_supply_inputs::origin_demand(&headers, seed.pointer) {
            super::add(&mut demand.supply_origins, count)?;
        }
        // Exercise affects only training, which has no modifier-tree source.
        let inputs = carmy_attrition::capture(
            &headers,
            base,
            state,
            seed,
            Err("attrition sizing excludes exercise"),
        );
        if inputs.province == Err("attrition army seed identity unavailable") {
            return Err("attrition demand army identity changed");
        }
        if let Ok(Some(terrain)) = inputs.terrain
            && let Ok(address) = terrain.name
            && let Some(header) = read::<[u8; msvc_string::STRIDE]>(&headers, address)
            && let Ok(text) = msvc_string::Descriptor::from_header(&headers, address, header)
        {
            super::add(&mut demand.terrain_text, text.len)?;
            if read::<[u8; msvc_string::STRIDE]>(&headers, address) != Some(header) {
                return Err("attrition terrain sizing header changed");
            }
        }
        for source in inputs.sources {
            if let Ok(Some(source)) = source {
                requests[request_len] = Request {
                    accumulator: source.accumulator,
                    key: source.key,
                };
                request_len += 1;
            }
        }
    }
    demand.requests = request_len;
    requests.truncate(request_len);
    requests.sort_unstable();
    requests.dedup();
    demand.roots = requests.len();
    let mut empty_requests: [Request; 0] = [];
    let mut roots: [Root; 0] = [];
    let mut nodes: [RawNode; 0] = [];
    let mut text = [];
    let mut stack: [Frame; 0] = [];
    let mut buffers = Buffers {
        requests: &mut empty_requests,
        request_len: 0,
        request_required: 0,
        roots: &mut roots,
        root_len: 0,
        root_required: 0,
        nodes: &mut nodes,
        node_len: 0,
        node_required: 0,
        text: &mut text,
        text_len: 0,
        text_required: 0,
        stack: &mut stack,
        stack_required: 0,
    };
    let limits = StackLimits::current();
    for request in requests {
        let result = capture_root(
            &headers,
            base,
            request,
            false,
            &mut buffers,
            &mut None,
            &limits,
        );
        // A root-local read fault leaves only its remaining topology unknown.
        // Changed selectors invalidate the complete off-hour observation.
        if let Err(
            reason @ ("attrition modifier display header changed"
            | "attrition modifier child header changed"
            | "attrition modifier demand overflow"),
        ) = result
        {
            return Err(reason);
        }
    }
    headers.validate()?;
    demand.nodes = buffers.node_required;
    demand.text = buffers.text_required;
    demand.depth = buffers.stack_required;
    Ok(demand)
}
