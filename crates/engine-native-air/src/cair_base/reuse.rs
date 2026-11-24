
use super::{
    COUNTRY_CONTAINERS, DenseEntry, Reader, RegistrationBase, RegistrationBuffers, RegistrationRow,
    RegistrationState, Resolver, SlotMode, dense_plan,
};
use engine_native_memory::Memory;
use engine_native::ccountry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkedBase<'a> {
    pub slots: u32,
    pub sought_plan: bool,
    pub plan: Option<&'a [DenseEntry]>,
    pub rows: &'a [RegistrationRow],
    pub wings_start: usize,
    pub wings: &'a [u64],
    pub pools: usize,
    pub required_wings: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct HeldBase<'a> {
    pub output: WalkedBase<'a>,
    pub countries: &'a [u32],
}

pub trait RegistrationReuse {
    fn previous(&self, handle: u64, pointer: usize) -> Option<HeldBase<'_>>;
    fn trusted_carried(&self, _handle: u64, _pointer: usize) -> bool {
        false
    }
    fn reused(&mut self, handle: u64, pointer: usize);
    fn begin(&mut self, handle: u64, pointer: usize);
    fn container(&mut self, country: u32);
    fn walked(&mut self, output: WalkedBase<'_>);
}

impl<M: Memory> Reader<'_, M> {
    #[expect(
        clippy::too_many_arguments,
        reason = "reuse updates the same resolver, buffers and state as a fresh registration"
    )]
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
        let trusted = reuse.trusted_carried(handle, pointer);
        let Some(held) = reuse.previous(handle, pointer) else {
            return false;
        };
        let output = held.output;
        let sought = slots.dense;
        let (indexed_count, is_dense) = if trusted {
            if sought != output.sought_plan {
                return false;
            }
            (output.slots as usize, output.plan.is_some())
        } else {
            let Ok(containers) = self.array(pointer, COUNTRY_CONTAINERS) else {
                return false;
            };
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
            (containers.count as usize, plan.is_some())
        };
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
        state.required_indexed = state.required_indexed.max(indexed_count);
        state.dense_bases += usize::from(is_dense);
        state.reused_bases += 1;
        reuse.reused(handle, pointer);
        true
    }
}
