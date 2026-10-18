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
    state_names: Result<usize, &'static str>,
    factions: Result<(usize, usize), &'static str>,
    faction_members: Result<usize, &'static str>,
    wars: Result<usize, &'static str>,
    groups: Result<usize, &'static str>,
}

impl Demand {
    pub(crate) fn observe(memory: &impl Memory, base: usize) -> Result<Option<Self>, &'static str> {
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
        let factions = engine_native::cfaction::walk(memory, base, state, &mut [], &mut []);
        let faction_rows =
            table_demand::rows(factions.outcome).map(|rows| (rows, factions.required_bytes));
        Ok(Some(Self {
            countries: FAILURES[1].observe(indexed(cgame_state::COUNTRIES, 8)),
            links: FAILURES[2].observe(links),
            provinces: FAILURES[3].observe(indexed(cgame_state::PROVINCES, 8)),
            states: FAILURES[4].observe(indexed(cgame_state::STATES, 8)),
            state_names: FAILURES[5]
                .observe(engine_native::cstate::name_demand(memory, base, state)),
            factions: FAILURES[6].observe(faction_rows),
            faction_members: FAILURES[7].observe(table_demand::rows(
                engine_native::cfaction_system::walk_members(memory, base, state, &mut []),
            )),
            wars: FAILURES[8].observe(engine_native::cwar_relation::row_demand(
                memory, base, state,
            )),
            groups: FAILURES[9].observe(engine_native::cprovince::conflict_group_demand(
                memory, base, state,
            )),
        }))
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
            (5, self.wars),
            (7, self.faction_members),
            (9, self.provinces),
            (10, self.groups),
            (11, self.state_names),
        ] {
            if let Ok(required) = demand {
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
    fn live_source_demand_raises_five_thousand_country_and_mod_arenas() {
        let demand = Demand {
            countries: Ok(5_000),
            links: Ok(7_000),
            provinces: Ok(35_000),
            states: Ok(7_000),
            state_names: Ok(350_000),
            factions: Ok((512, 512 * 31)),
            faction_members: Ok(5_000),
            wars: Ok(25_000),
            groups: Ok(50_000),
        };
        let mut sizing = Sizing::for_slots(1);
        demand.apply(&mut sizing);
        for (index, required) in [
            (0, 5_000),
            (1, 35_000),
            (2, 7_000),
            (3, 7_000),
            (5, 25_000),
            (6, 512),
            (7, 5_000),
            (8, 512 * 31),
            (9, 35_000),
            (10, 50_000),
            (11, 350_000),
        ] {
            assert!(sizing.core[index] >= required);
        }
        let before = sizing;
        let failed = Demand {
            countries: Err("roster changed"),
            factions: Err("name unreadable"),
            ..demand
        };
        failed.apply(&mut sizing);
        assert_eq!(sizing.core, before.core);
    }
}
