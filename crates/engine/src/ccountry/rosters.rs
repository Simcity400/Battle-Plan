//! Stored hourly country rosters; never sort, filter or refresh them.

use super::{CONTROLLED_STATES, CORE_STATES, Country, OWNED_STATES, Reader};
use crate::{Memory, ccountry_characters, cpdx_array, cpdx_array::Array, cstate, cunit_leader};

/// Army and navy CUnitLeader* arrays. 1406DF460/1406E6500 read them;
/// 1410DAF80 inserts, 1410D2540/1410D26B0 and 1410D90E0 remove.
const LEADERS: [usize; 2] = [0x70, 0x88];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RosterSpan {
    pub start: usize,
    pub count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RosterDemand {
    pub states: usize,
    pub leaders: usize,
}

pub(super) type Spans = ([RosterSpan; 3], [RosterSpan; 2]);

pub struct CounterRosters<'a> {
    pub states: &'a mut [u32],
    pub leaders: &'a mut [u64],
    pub state_len: usize,
    pub leader_len: usize,
    pub required: RosterDemand,
}

fn array(memory: &impl Memory, object: usize, offset: usize) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("country roster header unreadable or invalid")
}

fn leader_arrays(
    memory: &impl Memory,
    base: usize,
    country: Country,
) -> Result<[Array; 2], &'static str> {
    let vtable = base
        .checked_add(ccountry_characters::VTABLE_RVA)
        .ok_or("country characters module address overflow")? as u64;
    let Some(object) = ccountry_characters::characters_object(memory, &country, vtable)? else {
        return Ok([Array::default(); 2]);
    };
    Ok([
        array(memory, object, LEADERS[0])?,
        array(memory, object, LEADERS[1])?,
    ])
}

fn add(required: &mut usize, count: u32) -> Result<(), &'static str> {
    *required = required
        .checked_add(count as usize)
        .ok_or("country roster demand overflow")?;
    Ok(())
}

/// Header sums only. The country census authenticates each country, and the
/// non-null character root must belong to it. No member pointers are read.
pub fn counter_roster_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<RosterDemand, &'static str> {
    let countries = Reader::new(memory, base, state)?;
    let mut demand = RosterDemand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        for offset in [OWNED_STATES, CONTROLLED_STATES, CORE_STATES] {
            add(
                &mut demand.states,
                array(memory, country.pointer, offset)?.count,
            )?;
        }
        for leaders in leader_arrays(memory, base, country)? {
            add(&mut demand.leaders, leaders.count)?;
        }
    }
    Ok(demand)
}

impl CounterRosters<'_> {
    pub(super) fn read(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: Country,
        states: [Array; 2],
        row_fits: bool,
    ) -> Result<Spans, &'static str> {
        let start = (self.state_len, self.leader_len);
        let result = self.read_inner(memory, base, country, states, row_fits);
        if result.is_err() {
            (self.state_len, self.leader_len) = start;
        }
        result
    }

    fn read_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: Country,
        states: [Array; 2],
        row_fits: bool,
    ) -> Result<Spans, &'static str> {
        let states = [
            states[0],
            states[1],
            array(memory, country.pointer, CORE_STATES)?,
        ];
        let leaders = leader_arrays(memory, base, country)?;
        let mut state_count = 0;
        let mut leader_count = 0;
        for source in states {
            add(&mut state_count, source.count)?;
            add(&mut self.required.states, source.count)?;
        }
        for source in leaders {
            add(&mut leader_count, source.count)?;
            add(&mut self.required.leaders, source.count)?;
        }
        if !row_fits {
            return Err("country roster row capacity exceeded");
        }
        if state_count > self.states.len().saturating_sub(self.state_len) {
            return Err("country state roster arena capacity exceeded");
        }
        if leader_count > self.leaders.len().saturating_sub(self.leader_len) {
            return Err("country leader roster arena capacity exceeded");
        }
        let mut state_spans = [RosterSpan::default(); 3];
        for (span, source) in state_spans.iter_mut().zip(states) {
            *span = RosterSpan {
                start: self.state_len,
                count: source.count as usize,
            };
            cstate::copy_ids(
                memory,
                source,
                &mut self.states[self.state_len..self.state_len + source.count as usize],
                "country state roster pointer unreadable",
                "country state roster id unreadable",
            )?;
            self.state_len += source.count as usize;
        }
        let mut leader_spans = [RosterSpan::default(); 2];
        for (span, source) in leader_spans.iter_mut().zip(leaders) {
            *span = RosterSpan {
                start: self.leader_len,
                count: source.count as usize,
            };
            cpdx_array::pointers(
                memory,
                source,
                "country leader roster pointer unreadable",
                |_, pointer| {
                    let handle = if pointer == 0 {
                        0
                    } else {
                        cunit_leader::handle(memory, base, pointer)?
                    };
                    self.leaders[self.leader_len] = handle;
                    self.leader_len += 1;
                    Ok(())
                },
            )?;
        }
        Ok((state_spans, leader_spans))
    }
}

#[cfg(test)]
mod tests;
