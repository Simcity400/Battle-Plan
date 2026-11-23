use super::{Sizing, demand_game_state, raise, table_demand};
use crate::extended;
use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::cgame_state;

pub struct Demand {
    countries: Result<usize, &'static str>,
    links: Result<usize, &'static str>,
    provinces: Result<usize, &'static str>,
    states: Result<usize, &'static str>,
    state_names: Option<Result<usize, &'static str>>,
    factions: Result<(usize, usize), &'static str>,
    faction_members: Option<Result<usize, &'static str>>,
    wars: Option<Result<usize, &'static str>>,
    groups: Result<usize, &'static str>,
}

impl Demand {
    /// # Safety
    /// Only the attaching runtime or its designated sizing pass may publish census demand from the
    /// session's authenticated image. Tests must own the live demand registries in isolation.
    pub unsafe fn observe(memory: &impl Memory, base: usize) -> Result<Option<Self>, &'static str> {
        // SAFETY: Session sizing or the isolated fixture owns the census and its observer demand.
        unsafe { Self::observe_periodic(memory, base, true) }
    }

    /// # Safety
    /// Only the attaching runtime or its designated sizing pass may publish census demand from the
    /// session's authenticated image. Tests must own the live demand registries in isolation.
    pub unsafe fn observe_periodic(
        memory: &impl Memory,
        base: usize,
        capacity_census: bool,
    ) -> Result<Option<Self>, &'static str> {
        let Some(state) = demand_game_state(memory, base)? else {
            return Ok(None);
        };
        let indexed = |offset, width| {
            state
                .checked_add(offset)
                .and_then(|address| Array::read(memory, address, width))
                .filter(|array| array.count != 0)
                .map(|array| array.count.saturating_sub(1) as usize)
                .ok_or("source demand indexed registry invalid")
        };
        let links = state
            .checked_add(cgame_state::COUNTRY_LINKS)
            .and_then(|address| Array::read(memory, address, 4))
            .map(|array| array.count as usize)
            .ok_or("source demand country links invalid");
        let factions = engine_native::cfaction::count(memory, base, state);
        let faction_rows =
            table_demand::rows(factions.outcome).map(|rows| (rows, factions.required_bytes));
        if let Ok(accounts) = engine_native::cfaction::system::accounts(memory, base, state) {
            // SAFETY: Sizing census records measured faction-account demand for the active session.
            unsafe {
                collector_factions::faction_companion_changes::demand_accounts(
                    accounts.count as usize * 2 + 1,
                );
            }
        }
        if let Ok((owners, _)) = faction_rows {
            collector_observers::country_input_changes::sharing_demand(owners);
            // SAFETY: Sizing census records measured faction-owner demand for the active session.
            unsafe {
                collector_factions::faction_goal_changes::demand(owners);
                collector_factions::faction_companion_changes::demand(owners);
            }
            collector_intel::variable_changes::Observer.demand_embedded(owners);
        }
        Ok(Some(Self {
            countries: indexed(cgame_state::COUNTRIES, 8),
            links,
            provinces: indexed(cgame_state::PROVINCES, 8),
            states: indexed(cgame_state::STATES, 8),
            state_names: capacity_census
                .then(|| engine_native::cstate::name_demand(memory, base, state)),
            factions: faction_rows,
            faction_members: capacity_census.then(|| {
                table_demand::rows(engine_native::cfaction::system::count_members(
                    memory, base, state,
                ))
            }),
            wars: capacity_census
                .then(|| engine_native::cwar_relation::row_demand(memory, base, state)),
            groups: engine_native::cprovince::conflict_group_demand(memory, base, state),
        }))
    }

    pub fn refused(&self) -> bool {
        [
            self.countries,
            self.links,
            self.provinces,
            self.states,
            self.groups,
        ]
        .iter()
        .any(Result::is_err)
            || self.factions.is_err()
            || [self.state_names, self.faction_members, self.wars]
                .into_iter()
                .flatten()
                .any(|demand| demand.is_err())
    }

    pub fn apply(&self, sizing: &mut Sizing) {
        self.apply_parts(&mut sizing.core, &mut sizing.extended);
    }

    pub fn apply_parts(&self, core: &mut [usize; 12], extended: &mut extended::Capacities) {
        for (index, demand) in [
            (0, self.countries),
            (1, self.provinces),
            (2, self.links),
            (3, self.states),
            (9, self.provinces),
            (10, self.groups),
        ] {
            if let Ok(required) = demand {
                raise(&mut core[index], required);
            }
        }
        for (index, demand) in [
            (5, self.wars),
            (7, self.faction_members),
            (11, self.state_names),
        ] {
            if let Some(Ok(required)) = demand {
                raise(&mut core[index], required);
            }
        }
        if let Ok((rows, bytes)) = self.factions {
            raise(&mut core[6], rows);
            raise(&mut core[8], bytes);
        }
        extended.raise_for_states(core[3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_source_sizing_skips_worker_walks_without_failures_or_lost_floors() {
        struct Registry;
        impl Memory for Registry {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                const STATE: usize = 0x1000;
                if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&(STATE as u64).to_le_bytes());
                    return true;
                }
                if address == STATE + cgame_state::COUNTRIES && output.len() == 16 {
                    output[..8].copy_from_slice(&0x3000u64.to_le_bytes());
                    output[8..12].copy_from_slice(&2u32.to_le_bytes());
                    output[12..].copy_from_slice(&2u32.to_le_bytes());
                    return true;
                }
                false
            }
        }
        let skipped =
        // SAFETY: Session sizing or the isolated fixture owns the census and its observer demand.
        unsafe { Demand::observe_periodic(&Registry, 0, false) }
            .unwrap()
            .unwrap();
        assert!(skipped.state_names.is_none());
        assert!(skipped.faction_members.is_none());
        assert!(skipped.wars.is_none());
        assert!(skipped.refused());
        let mut sizing = Sizing::for_slots(1);
        let previous = sizing.core;
        skipped.apply(&mut sizing);
        assert_eq!(sizing.core, previous);
        let attach =
        // SAFETY: Attach or the isolated fixture owns this authenticated census and its observer
        // demand.
        unsafe { Demand::observe(&Registry, 0) }.unwrap().unwrap();
        assert!(attach.state_names.unwrap().is_err());
        assert!(attach.faction_members.unwrap().is_err());
        assert!(attach.wars.unwrap().is_err());
    }
}
