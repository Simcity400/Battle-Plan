
use crate::{LIFETIMES, LOGS, SERIES, summaries};
use replay_columnar::summary::{
    BASE_HOURS, Shape,
    cost::{self, Load},
};
use replay_data::rows::TableId;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct World {
    pub countries: u64,
}

pub const RECORDING: World = World { countries: 445 };
pub const LARGEST: World = World { countries: 1_500 };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    PerCountry(u64),
    Pairs,
    Fixed(u64),
}

impl Keys {
    pub fn of(self, world: World) -> u64 {
        match self {
            Self::PerCountry(each) => each * world.countries,
            Self::Pairs => world.countries * world.countries,
            Self::Fixed(count) => count,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cost {
    pub table: TableId,
    pub keys: Keys,
    pub changes: u64,
}

pub const SOMETIMES: u64 = 2;
pub const DAILY: u64 = 30;
pub const EVERY_BUCKET: u64 = 720 / BASE_HOURS.unsigned_abs() + 1;

const fn cost(table: TableId, keys: Keys, changes: u64) -> Cost {
    Cost { table, keys, changes }
}

pub static COSTS: &[Cost] = &[
    cost(TableId::Countries, Keys::PerCountry(1), DAILY),
    cost(TableId::CountryStability, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::StabilityParts, Keys::PerCountry(9), SOMETIMES),
    cost(TableId::ArmyTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryResources, Keys::PerCountry(4), SOMETIMES),
    cost(TableId::CountryDeployedManpower, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::SupplyHealth, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::AirForceTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::NavyTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryExperienceDaily, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryGameScores, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::FuelDays, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::FuelDemandTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::ProductionCountryTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::ProductionTypeTotals, Keys::PerCountry(3), SOMETIMES),
    cost(TableId::CasualtyTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::OccupierTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::TerritoryTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryFactories, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::Nukes, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::CountryNukeTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::WorldTension, Keys::Fixed(1), DAILY),
    cost(TableId::ThreatCountryAggregates, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::ThreatTotals, Keys::Fixed(1), DAILY),
    cost(TableId::FactionTotals, Keys::Fixed(32), SOMETIMES),
    cost(TableId::AirForceParts, Keys::PerCountry(3), SOMETIMES),
    cost(TableId::AirForceWingKinds, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::NavyClasses, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::MilitaryAid, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryExperience, Keys::PerCountry(1), DAILY),
    cost(TableId::Autonomy, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryFuel, Keys::PerCountry(1), DAILY),
    cost(TableId::CountryFuelConsumers, Keys::PerCountry(3), SOMETIMES),
    cost(TableId::NavalLossTotals, Keys::PerCountry(4), SOMETIMES),
    cost(TableId::MarketEvents, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::BalanceDrops, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::ThreatEvents, Keys::Fixed(64), SOMETIMES),
    cost(TableId::LandBattleEnds, Keys::Fixed(16), SOMETIMES),
    cost(TableId::DivisionEvents, Keys::PerCountry(3), SOMETIMES),
    cost(TableId::LeaderBattles, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::PeaceOutcomes, Keys::Fixed(64), SOMETIMES),
    cost(TableId::FeedEvents, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::AirLossEvents, Keys::PerCountry(4), SOMETIMES),
    cost(TableId::BuildingBombingDamage, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::LeaderTraitGains, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::ShipLosses, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::PeaceConferenceSpans, Keys::Fixed(1), SOMETIMES),
    cost(TableId::OperationCompletions, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::RaidEnds, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::PeaceSettlements, Keys::Fixed(1), SOMETIMES),
    cost(TableId::NuclearStrikes, Keys::Fixed(1), SOMETIMES),
    cost(TableId::LeaderCommands, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::LandCombatResults, Keys::Fixed(1), EVERY_BUCKET),
    cost(TableId::LandCombatResultObservers, Keys::Fixed(1), EVERY_BUCKET),
    cost(TableId::CombatLogBattleModifiers, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::TagHolders, Keys::PerCountry(1), 2),
    cost(TableId::GroupSpans, Keys::PerCountry(2), 2),
    cost(TableId::WarEvents, Keys::PerCountry(1), 2),
    cost(TableId::LookSpans, Keys::PerCountry(1), 2),
    cost(TableId::ArmyMix, Keys::PerCountry(5), 2),
    cost(TableId::ResearchCoverage, Keys::PerCountry(4), 2),
    cost(TableId::OperativeMemberships, Keys::PerCountry(4), 2),
    cost(TableId::Raids, Keys::PerCountry(1), 2),
    cost(TableId::CasualtyLifetimes, Keys::PerCountry(4), 2),
    cost(TableId::ConstructionProjects, Keys::PerCountry(6), 2),
    cost(TableId::UnitLeaders, Keys::PerCountry(4), 2),
    cost(TableId::Operations, Keys::PerCountry(1), 2),
    cost(TableId::NavalBattles, Keys::PerCountry(1), 2),
    cost(TableId::NavalResults, Keys::PerCountry(8), 2),
    cost(TableId::EquipmentLossTotals, Keys::PerCountry(62), SOMETIMES),
    cost(TableId::AirStatistics, Keys::PerCountry(8), 2),
    cost(TableId::PoliticalParties, Keys::PerCountry(4), DAILY),
    cost(TableId::ProductionTotals, Keys::PerCountry(8), 2),
    cost(TableId::DockyardRepairRates, Keys::PerCountry(3), 2),
    cost(TableId::RaidAirScores, Keys::PerCountry(6), DAILY),
    cost(TableId::ProducedValues, Keys::PerCountry(5), DAILY),
];

pub struct Price {
    pub table: TableId,
    pub shape: Shape,
    pub load: Load,
    pub held: u64,
    pub piece: u64,
}

pub fn prices(world: World) -> io::Result<Vec<Price>> {
    summaries()?
        .into_iter()
        .map(|(table, shape)| {
            let id = TableId::ALL[table];
            let stated = COSTS
                .iter()
                .find(|cost| cost.table == id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{id:?} is summarized and states no cost in dashboard_declare::budget::COSTS")))?;
            let load = Load { keys: stated.keys.of(world), changes: stated.changes };
            Ok(Price { table: id, held: cost::held(&shape, load), piece: cost::piece(&shape, load), shape, load })
        })
        .collect()
}

pub fn total(world: World) -> io::Result<u64> {
    let tables: Vec<(Shape, Load)> = prices(world)?.into_iter().map(|price| (price.shape, price.load)).collect();
    Ok(cost::peak(&tables))
}

pub fn declared(table: TableId) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = SERIES.iter().filter(|series| series.table == table).map(|series| series.kind).collect();
    names.extend(LOGS.iter().filter(|log| log.table == table).map(|log| log.name));
    if LIFETIMES.contains(&table) {
        names.push("lifetimes");
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_summarized_table_states_its_cost_and_every_cost_is_summarized() {
        for world in [RECORDING, LARGEST] {
            prices(world).unwrap();
        }
        for cost in COSTS {
            assert!(summaries().unwrap().iter().any(|(table, _)| *table == cost.table.index()), "{:?} states a cost and is not summarized", cost.table);
        }
    }
}