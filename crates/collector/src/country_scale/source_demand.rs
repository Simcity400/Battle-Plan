//! Advisory source storage from native headers before attach and on demand cadence.
use super::{DemandFailures, Sizing, demand_game_state, raise, table_demand};
use crate::extended;
use engine_native::{Memory, cgame_state, cpdx_array::Array};

const NAMES: [&str; 10] = [
    "world",
    "countries",
    "country_links",
    "provinces",
    "states",
    "state_names",
    "factions",
    "faction_members",
    "war_sources",
    "border_conflict_groups",
];
static FAILURES: [DemandFailures; 10] = [const { DemandFailures::new() }; 10];

pub(crate) fn diagnostics() -> impl serde::Serialize {
    crate::status::map(
        NAMES
            .into_iter()
            .zip(FAILURES.iter().map(DemandFailures::diagnostics)),
    )
}

pub(crate) struct Demand {
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
    pub(crate) fn observe(memory: &impl Memory, base: usize) -> Result<Option<Self>, &'static str> {
        Self::observe_periodic(memory, base, true)
    }

    pub(crate) fn observe_periodic(
        memory: &impl Memory,
        base: usize,
        capacity_census: bool,
    ) -> Result<Option<Self>, &'static str> {
        let Some(state) = FAILURES[0].observe(demand_game_state(memory, base))? else {
            return Ok(None);
        };
        // World reset and province initialization
        // size the indexed live registries; slot zero supplies no source row.
        let indexed = |offset, width| {
            state
                .checked_add(offset)
                .and_then(|address| Array::read(memory, address, width))
                .filter(|array| array.count != 0)
                .map(|array| array.count.saturating_sub(1) as usize)
                .ok_or("source demand indexed registry invalid")
        };
        // CountryLinks preserves its zero entry as an independent replay row.
        let links = state
            .checked_add(cgame_state::COUNTRY_LINKS)
            .and_then(|address| Array::read(memory, address, 4))
            .map(|array| array.count as usize)
            .ok_or("source demand country links invalid");
        // Faction names share the observer-owner census. Indexed registry
        // floors and conflict groups still feed fixed consumers; keep these
        // reads on the fast cadence, unlike the independent worker-only walks.
        let factions = engine_native::cfaction::walk(memory, base, state, &mut [], &mut []);
        let faction_rows =
            table_demand::rows(factions.outcome).map(|rows| (rows, factions.required_bytes));
        if let Ok(accounts) = engine_native::cfaction_system::accounts(memory, base, state) {
            crate::faction_companion_changes::demand_accounts(accounts.count as usize * 2 + 1);
        }
        if let Ok((owners, _)) = faction_rows {
            crate::country_input_changes::sharing_demand(owners);
            crate::faction_goal_changes::demand(owners);
            crate::faction_companion_changes::demand(owners);
            crate::variable_changes::Observer.demand_embedded(owners);
        }
        Ok(Some(Self {
            countries: FAILURES[1].observe(indexed(cgame_state::COUNTRIES, 8)),
            links: FAILURES[2].observe(links),
            provinces: FAILURES[3].observe(indexed(cgame_state::PROVINCES, 8)),
            states: FAILURES[4].observe(indexed(cgame_state::STATES, 8)),
            state_names: capacity_census.then(|| {
                FAILURES[5].observe(engine_native::cstate::name_demand(memory, base, state))
            }),
            factions: FAILURES[6].observe(faction_rows),
            faction_members: capacity_census.then(|| {
                FAILURES[7].observe(table_demand::rows(
                    engine_native::cfaction_system::walk_members(memory, base, state, &mut []),
                ))
            }),
            wars: capacity_census.then(|| {
                FAILURES[8].observe(engine_native::cwar_relation::row_demand(
                    memory, base, state,
                ))
            }),
            groups: FAILURES[9].observe(engine_native::cprovince::conflict_group_demand(
                memory, base, state,
            )),
        }))
    }

    pub(crate) fn refused(&self) -> bool {
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

    pub(crate) fn apply(&self, sizing: &mut Sizing) {
        self.apply_parts(&mut sizing.core, &mut sizing.extended);
    }

    pub(crate) fn apply_parts(&self, core: &mut [usize; 12], extended: &mut extended::Capacities) {
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
        // These independent worker walks grow before publication. A skipped
        // hint is not a failed observation and preserves its previous floor.
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
        let failures = || {
            [5, 7, 8].map(|index| {
                serde_json::to_value(FAILURES[index].diagnostics()).unwrap()["failed_observations"]
                    .as_u64()
                    .unwrap()
            })
        };
        let before = failures();
        let skipped = Demand::observe_periodic(&Registry, 0, false)
            .unwrap()
            .unwrap();
        assert!(skipped.state_names.is_none());
        assert!(skipped.faction_members.is_none());
        assert!(skipped.wars.is_none());
        assert_eq!(failures(), before);
        // Unreadable prompt/fixed inputs still request the original retry rate.
        assert!(skipped.refused());
        let mut sizing = Sizing::for_slots(1);
        let previous = sizing.core;
        skipped.apply(&mut sizing);
        assert_eq!(sizing.core, previous);
        let attach = Demand::observe(&Registry, 0).unwrap().unwrap();
        assert!(attach.state_names.unwrap().is_err());
        assert!(attach.faction_members.unwrap().is_err());
        assert!(attach.wars.unwrap().is_err());
        assert_eq!(failures(), before.map(|count| count + 1));
    }
}
