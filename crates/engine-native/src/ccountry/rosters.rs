//! Stored hourly country rosters; never sort, filter or refresh them.

use super::{CONTROLLED_STATES, CORE_STATES, Country, OWNED_STATES, PRIDE_OF_THE_FLEET, Reader};
use crate::{
    Memory, ccountry_characters, cpdx_array, cpdx_array::Array, cproduction_status, cstate,
    cunit_leader, read, walk::Buffer,
};

/// Controller changes and flagged derived-state updates rebuild raw neighbor
/// references. No hourly refresh is performed.
pub const NEIGHBORS: usize = 0x1088;

/// The MIO policy roster's refusal for a null `CCountry+0xF68`
/// (`policies_header`): the native state of a country without production
/// status, not a read fault.
pub const POLICY_STATUS_NULL: &str = "MIO policy production status is null";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EffectRosterDemand {
    pub neighbors: usize,
    pub policies: usize,
}

pub struct EffectRosters<'a> {
    pub neighbors: Buffer<'a, u32>,
    pub policies: Buffer<'a, u32>,
    pub neighbor_len: usize,
    pub policy_len: usize,
    pub required: EffectRosterDemand,
}

pub fn counter_effect_roster_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<EffectRosterDemand, &'static str> {
    let countries = Reader::new(memory, base, state)?;
    let mut demand = EffectRosterDemand::default();
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let neighbors = country
            .pointer
            .checked_add(NEIGHBORS)
            .and_then(|at| Array::read(memory, at, 4))
            .ok_or("country neighbor roster unreadable or invalid")?;
        add(&mut demand.neighbors, neighbors.count)?;
        if let Some(policies) = cproduction_status::policies_header(memory, base, country)? {
            add(&mut demand.policies, policies.count)?;
        }
    }
    Ok(demand)
}

impl EffectRosters<'_> {
    pub(super) fn read_neighbors(
        &mut self,
        memory: &impl Memory,
        country: Country,
        fits: bool,
    ) -> Result<RosterSpan, &'static str> {
        let source = country
            .pointer
            .checked_add(NEIGHBORS)
            .and_then(|at| Array::read(memory, at, 4))
            .ok_or("country neighbor roster unreadable or invalid")?;
        add(&mut self.required.neighbors, source.count)?;
        let count = source.count as usize;
        // Neighbor refresh retains every stored occurrence.
        self.neighbors.ensure(
            self.neighbor_len
                .checked_add(count)
                .ok_or("country roster demand overflow")?,
        )?;
        if !fits || count > self.neighbors.len().saturating_sub(self.neighbor_len) {
            return Err("country neighbor roster arena capacity exceeded");
        }
        let span = RosterSpan {
            start: self.neighbor_len,
            count,
        };
        cpdx_array::copy_u32s(
            memory,
            source,
            &mut self.neighbors[span.start..span.start + count],
            "country neighbor roster entries unreadable",
        )?;
        self.neighbor_len += count;
        Ok(span)
    }

    pub(super) fn read_policies(
        &mut self,
        memory: &impl Memory,
        base: usize,
        country: Country,
        fits: bool,
    ) -> Result<RosterSpan, &'static str> {
        let Some(source) = cproduction_status::policies_header(memory, base, country)? else {
            return Err(POLICY_STATUS_NULL);
        };
        add(&mut self.required.policies, source.count)?;
        let count = source.count as usize;
        self.policies.ensure(
            self.policy_len
                .checked_add(count)
                .ok_or("country roster demand overflow")?,
        )?;
        if !fits || count > self.policies.len().saturating_sub(self.policy_len) {
            return Err("MIO policy roster arena capacity exceeded");
        }
        let span = RosterSpan {
            start: self.policy_len,
            count,
        };
        cproduction_status::read_policies(
            memory,
            base,
            source,
            &mut self.policies[span.start..span.start + count],
        )?;
        self.policy_len += count;
        Ok(span)
    }
}

/// Army and navy unit-leader pointer arrays maintained by role insertion
/// and removal.
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
    pub states: Buffer<'a, u32>,
    pub leaders: Buffer<'a, u64>,
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
    counter_roster_demand_inner(memory, base, state, None)
}

/// Reuse authenticated roster controls for an independent sparse ship-name
/// sizing hint. After each country's headers observe its full pride reference.
/// Observer faults leave roster demand independent; the original API adds
/// no pride reads.
pub fn counter_roster_demand_with_pride_observer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    observer: &mut dyn FnMut(Result<u64, &'static str>),
) -> Result<RosterDemand, &'static str> {
    counter_roster_demand_inner(memory, base, state, Some(observer))
}

fn counter_roster_demand_inner(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut observer: Option<&mut dyn FnMut(Result<u64, &'static str>)>,
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
        if let Some(observer) = observer.as_mut() {
            observer(
                country
                    .pointer
                    .checked_add(PRIDE_OF_THE_FLEET)
                    .and_then(|address| read::<u64>(memory, address))
                    .ok_or("country pride-name demand reference unreadable"),
            );
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
        // Leader and state headers bound complete original rosters; worker growth
        // preserves their occurrence order.
        self.states.ensure(
            self.state_len
                .checked_add(state_count)
                .ok_or("country roster demand overflow")?,
        )?;
        self.leaders.ensure(
            self.leader_len
                .checked_add(leader_count)
                .ok_or("country roster demand overflow")?,
        )?;
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
            // Each page's leaders in one guarded batch; a null slot is zero.
            cpdx_array::pointer_pages(
                memory,
                source,
                "country leader roster pointer unreadable",
                |_, page| {
                    let start = self.leader_len;
                    cunit_leader::handles(
                        memory,
                        base,
                        page,
                        &mut self.leaders[start..start + page.len()],
                    )?;
                    self.leader_len += page.len();
                    Ok(())
                },
            )?;
        }
        Ok((state_spans, leader_spans))
    }
}

/// One country's roster entries from an earlier complete read, in its row's
/// span order: what a caller carrying the country
/// ([`super::walk_counter_inputs_carried_growing`]) places in this read's
/// arenas instead of reading them.
#[derive(Clone, Copy, Debug)]
pub struct CarriedRosters<'a> {
    pub states: [&'a [u32]; 3],
    pub leaders: [&'a [u64]; 2],
    pub neighbors: &'a [u32],
    /// `None`: the country had no production status ([`POLICY_STATUS_NULL`]).
    pub policies: Option<&'a [u32]>,
}

/// Where [`CarriedRosters::append`] placed the entries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarriedSpans {
    pub states: [RosterSpan; 3],
    pub leaders: [RosterSpan; 2],
    pub neighbors: RosterSpan,
    pub policies: Option<RosterSpan>,
}

/// Room for `count` entries at `len`; the new length.
fn room<T: Default>(buffer: &mut Buffer<'_, T>, len: usize, count: usize) -> Option<usize> {
    let end = len.checked_add(count)?;
    (buffer.ensure(end).ok()? && end <= buffer.len()).then_some(end)
}

fn place<T: Copy>(buffer: &mut Buffer<'_, T>, len: &mut usize, entries: &[T]) -> RosterSpan {
    let span = RosterSpan {
        start: *len,
        count: entries.len(),
    };
    buffer[span.start..span.start + span.count].copy_from_slice(entries);
    *len += span.count;
    span
}

impl CarriedRosters<'_> {
    /// Appends the entries at the arenas' current lengths and raises each
    /// demand by their counts, as a fresh read of the same complete rosters
    /// would (its spans follow the same order). `None`, with no length or
    /// demand moved, when an arena cannot hold them; the caller then reads
    /// the country.
    pub fn append(
        &self,
        rosters: &mut CounterRosters<'_>,
        effects: &mut EffectRosters<'_>,
    ) -> Option<CarriedSpans> {
        let states = self
            .states
            .iter()
            .map(|entries| entries.len())
            .sum::<usize>();
        let leaders = self
            .leaders
            .iter()
            .map(|entries| entries.len())
            .sum::<usize>();
        let policies = self.policies.map_or(0, <[u32]>::len);
        room(&mut rosters.states, rosters.state_len, states)?;
        room(&mut rosters.leaders, rosters.leader_len, leaders)?;
        room(
            &mut effects.neighbors,
            effects.neighbor_len,
            self.neighbors.len(),
        )?;
        room(&mut effects.policies, effects.policy_len, policies)?;
        let demand = RosterDemand {
            states: rosters.required.states.checked_add(states)?,
            leaders: rosters.required.leaders.checked_add(leaders)?,
        };
        let effect_demand = EffectRosterDemand {
            neighbors: effects
                .required
                .neighbors
                .checked_add(self.neighbors.len())?,
            policies: effects.required.policies.checked_add(policies)?,
        };
        rosters.required = demand;
        effects.required = effect_demand;
        let mut spans = CarriedSpans::default();
        for (span, entries) in spans.states.iter_mut().zip(self.states) {
            *span = place(&mut rosters.states, &mut rosters.state_len, entries);
        }
        for (span, entries) in spans.leaders.iter_mut().zip(self.leaders) {
            *span = place(&mut rosters.leaders, &mut rosters.leader_len, entries);
        }
        spans.neighbors = place(
            &mut effects.neighbors,
            &mut effects.neighbor_len,
            self.neighbors,
        );
        spans.policies = self
            .policies
            .map(|entries| place(&mut effects.policies, &mut effects.policy_len, entries));
        Some(spans)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pride_demand_tests {
    use super::*;
    use crate::ccountry::synthetic::World;
    use std::cell::RefCell;

    struct Tracked<'a> {
        memory: crate::memory::SliceMemory<'a>,
        denied: Option<usize>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Tracked<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            if self
                .denied
                .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
            {
                return false;
            }
            self.memory.copy(address, destination)
        }
    }
    fn tracked(world: &World, denied: Option<usize>) -> Tracked<'_> {
        Tracked {
            memory: world.memory(),
            denied,
            reads: RefCell::default(),
        }
    }

    #[test]
    fn original_roster_hint_has_identical_read_order_without_pride_observer() {
        let mut world = World::new(0x4000_0000, 3, 0x1100);
        let pride: Vec<_> = (1..=3)
            .map(|slot| world.object(slot) + PRIDE_OF_THE_FLEET)
            .collect();
        for (address, handle) in pride
            .iter()
            .zip([0, (7_u64 << 32) | 51, (9_u64 << 32) | 51])
        {
            world.u64(*address, handle);
        }
        let old = tracked(&world, Some(pride[1]));
        assert_eq!(
            counter_roster_demand(&old, world.module, world.game_state()),
            Ok(RosterDemand::default())
        );
        assert!(
            old.reads
                .borrow()
                .iter()
                .all(|(address, _)| !pride.contains(address))
        );
        let observed = tracked(&world, None);
        let mut handles = Vec::new();
        assert_eq!(
            counter_roster_demand_with_pride_observer(
                &observed,
                world.module,
                world.game_state(),
                &mut |handle| handles.push(handle)
            ),
            Ok(RosterDemand::default())
        );
        assert_eq!(
            handles,
            [Ok(0), Ok((7_u64 << 32) | 51), Ok((9_u64 << 32) | 51)]
        );
        let old_reads = old.reads.into_inner();
        let observed_reads: Vec<_> = observed
            .reads
            .into_inner()
            .into_iter()
            .filter(|(address, _)| !pride.contains(address))
            .collect();
        assert_eq!(observed_reads, old_reads);
    }

    #[test]
    fn independent_pride_read_failure_keeps_roster_sums_and_later_observations() {
        let mut world = World::new(0x4000_0000, 3, 0x1100);
        let states = world.alloc(8);
        world.array(world.object(1) + OWNED_STATES, states, 1);
        world.u64(world.object(3) + PRIDE_OF_THE_FLEET, (11_u64 << 32) | 51);
        let memory = tracked(&world, Some(world.object(2) + PRIDE_OF_THE_FLEET));
        let mut handles = Vec::new();
        assert_eq!(
            counter_roster_demand_with_pride_observer(
                &memory,
                world.module,
                world.game_state(),
                &mut |handle| handles.push(handle)
            ),
            Ok(RosterDemand {
                states: 1,
                leaders: 0
            })
        );
        assert_eq!(
            handles,
            [
                Ok(0),
                Err("country pride-name demand reference unreadable"),
                Ok((11_u64 << 32) | 51)
            ]
        );
        // An unauthenticated country is never offered to the observer.
        world.u32(world.object(2) + crate::ccountry::ID, 99);
        let memory = tracked(&world, None);
        handles.clear();
        assert!(
            counter_roster_demand_with_pride_observer(
                &memory,
                world.module,
                world.game_state(),
                &mut |handle| handles.push(handle)
            )
            .is_err()
        );
        assert_eq!(handles, [Ok(0)]);
    }
}
