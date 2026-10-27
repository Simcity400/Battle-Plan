//! A state base's registration output appended from an earlier walk of the
//! same bytes ([`RegistrationReuse`]).
//!
//! The walk's output for one state base is a function of the base's subtree
//! (its slot vector and dense list, its containers, their pools and the pools'
//! wings), of the image's vtables, and of what the country tables say of each
//! accepted container: the dense plan, a reserved slot's unlinked reference
//! and each other slot's resolved country. A caller that knows this walk's
//! memory holds a subtree exactly as an earlier recorded walk read it hands
//! that walk's output back; the walk then takes the plan again, checks every
//! container's country again, and appends the output only when they agree and
//! every row fits. Anything else walks the base, so its rows, demand and
//! failure are always those of the walk.

use super::{
    COUNTRY_CONTAINERS, DenseEntry, Reader, RegistrationBase, RegistrationBuffers, RegistrationRow,
    RegistrationState, Resolver, SlotMode, dense_plan,
};
use crate::{Memory, ccountry};

/// One state base's output as a complete walk of it produced it, every row
/// published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkedBase<'a> {
    /// The base's `COUNTRY_CONTAINERS` slot count.
    pub slots: u32,
    /// The walk sought the base's dense plan ([`SlotMode`]).
    pub sought_plan: bool,
    /// The plan it took, or `None` for the whole scan.
    pub plan: Option<&'a [DenseEntry]>,
    /// Its rows in walk order. Each `wings_start` counts from `wings_start`
    /// below, the base's first wing in the walk's arena.
    pub rows: &'a [RegistrationRow],
    pub wings_start: usize,
    pub wings: &'a [u64],
    /// Its pool and wing demand.
    pub pools: usize,
    pub required_wings: usize,
}

/// A recorded output a later walk may append.
#[derive(Clone, Copy, Debug)]
pub struct HeldBase<'a> {
    pub output: WalkedBase<'a>,
    /// Each row's container country reference, in row order.
    pub countries: &'a [u32],
}

/// Records each state base a walk completes and offers recorded outputs back.
pub trait RegistrationReuse {
    /// The output recorded for the state base `handle` at `pointer` when this
    /// walk's memory holds its subtree byte for byte as the recorded walk read
    /// it, with `wings_start` zero.
    fn previous(&self, handle: u64, pointer: usize) -> Option<HeldBase<'_>>;
    /// The walk appended `previous(handle, pointer)`.
    fn reused(&mut self, handle: u64, pointer: usize);
    /// The walk reads the state base afresh.
    fn begin(&mut self, handle: u64, pointer: usize);
    /// Each container the base begun accepted, in row order.
    fn container(&mut self, country: u32);
    /// The base begun completed with every row published.
    fn walked(&mut self, output: WalkedBase<'_>);
}

impl<M: Memory> Reader<'_, M> {
    /// Appends the recorded output of a state base when its plan, every
    /// container's country and the arenas agree; returns whether it did.
    /// Nothing but the resolver's cache and plan scratch changes otherwise.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reuse_registration(
        &self,
        observed: RegistrationBase,
        countries: &ccountry::Reader<'_, M>,
        resolved: &mut Resolver<'_>,
        buffers: &mut RegistrationBuffers<'_>,
        state: &mut RegistrationState,
        slots: SlotMode,
        reuse: &mut (dyn RegistrationReuse + '_),
    ) -> bool {
        let RegistrationBase { pointer, handle } = observed;
        let Some(held) = reuse.previous(handle, pointer) else {
            return false;
        };
        let output = held.output;
        let Ok(containers) = self.array(pointer, COUNTRY_CONTAINERS) else {
            return false;
        };
        let sought = slots.dense;
        if containers.count != output.slots || sought != output.sought_plan {
            return false;
        }
        let plan = if sought {
            dense_plan(
                self.memory,
                self.base,
                pointer,
                containers,
                countries.arrays(),
                &mut state.scratch.plan,
            )
            .ok()
        } else {
            None
        };
        if plan.map(|len| &state.scratch.plan[..len]) != output.plan {
            return false;
        }
        let (Some(rows_end), Some(wings_end), Some(pools), Some(required_wings)) = (
            state.len.checked_add(output.rows.len()),
            state.wings_len.checked_add(output.wings.len()),
            state.required_pools.checked_add(output.pools),
            state.required_wings.checked_add(output.required_wings),
        ) else {
            return false;
        };
        if rows_end > buffers.registrations.len()
            || wings_end > buffers.wings.len()
            || held.countries.len() != output.rows.len()
        {
            return false;
        }
        // The walk's own acceptance of each container, against this hour's
        // country tables.
        let accepted = held
            .countries
            .iter()
            .zip(output.rows)
            .all(|(&country, row)| {
                if row.slot == 0 {
                    country == 0 || matches!(countries.link(country), Ok(None))
                } else {
                    resolved
                        .resolve(countries, country)
                        .is_ok_and(|resolved| resolved.canonical_slot == row.slot)
                }
            });
        if !accepted {
            return false;
        }
        let wings_start = state.wings_len;
        for (destination, row) in buffers.registrations[state.len..rows_end]
            .iter_mut()
            .zip(output.rows)
        {
            *destination = RegistrationRow {
                wings_start: row.wings_start + wings_start,
                ..*row
            };
        }
        buffers.wings[wings_start..wings_end].copy_from_slice(output.wings);
        state.len = rows_end;
        state.wings_len = wings_end;
        state.required_pools = pools;
        state.required_wings = required_wings;
        state.required_indexed = state.required_indexed.max(containers.count as usize);
        state.dense_bases += usize::from(plan.is_some());
        state.reused_bases += 1;
        reuse.reused(handle, pointer);
        true
    }
}
