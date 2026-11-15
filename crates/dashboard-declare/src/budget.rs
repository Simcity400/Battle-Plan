//! What every declaration's summaries cost the recorder at its heaviest hour, by the summary
//! builder's model ([`summary::cost`]), against what the recorder's memory gate gives them: a
//! declaration states the keys its table holds and how often its cells move, and its price is
//! known the moment it is declared.

use crate::{LIFETIMES, LOGS, SERIES, summaries};
use replay_columnar::summary::{
    BASE_HOURS, Shape,
    cost::{self, Load},
};
use replay_data::rows::TableId;
use std::io;

/// A world a price is taken in, by its live countries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct World {
    pub countries: u64,
}

/// The 13-year recording's heaviest block (block 123): 439 live countries and a few more rows.
pub const RECORDING: World = World { countries: 445 };
/// The world the recorder is designed for.
pub const LARGEST: World = World { countries: 1_500 };

/// How many keys a summarized table holds in a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// So many per live country.
    PerCountry(u64),
    /// One per ordered pair of live countries.
    Pairs,
    /// So many in any world.
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

/// A summarized table's heaviest block: its keys, and per key and summarized column the changes
/// the block makes (a lifetimes table: comings and goings; a log: buckets holding a row).
#[derive(Clone, Copy, Debug)]
pub struct Cost {
    pub table: TableId,
    pub keys: Keys,
    pub changes: u64,
}

/// Changes of a cell in a block of 720 hours, for a table that does not move every day. The
/// recorder's level-0 pieces measured 0.6 MB raw a block with the first wave and 1.8 MB with both:
/// past Countries' daily political and command power, about one changed bucket a cell; this is
/// twice that.
pub const SOMETIMES: u64 = 2;
/// Changes of a cell in a block that moves every game day.
pub const DAILY: u64 = 30;
/// A log's buckets in a block when a row lands in every one.
pub const EVERY_BUCKET: u64 = 720 / BASE_HOURS.unsigned_abs() + 1;

const fn cost(table: TableId, keys: Keys, changes: u64) -> Cost {
    Cost { table, keys, changes }
}

/// Every summarized table's heaviest block. Keys per country are the 13-year recording's live
/// rows at block 123 over its 439 countries where the recorder's log measured them, rounded up;
/// the rest are stated.
pub static COSTS: &[Cost] = &[
    // Political and command power move every day for every country.
    cost(TableId::Countries, Keys::PerCountry(1), DAILY),
    cost(TableId::CountryStability, Keys::PerCountry(1), SOMETIMES),
    // 3,618 rows measured.
    cost(TableId::StabilityParts, Keys::PerCountry(9), SOMETIMES),
    cost(TableId::ArmyTotals, Keys::PerCountry(1), SOMETIMES),
    // A row per country and resource it holds: the first wave's pieces imply at most three a
    // country on average; four.
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
    // 1,062 rows measured.
    cost(TableId::ProductionTypeTotals, Keys::PerCountry(3), SOMETIMES),
    // A row per holder that fought or occupied: at most one per country.
    cost(TableId::CasualtyTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::OccupierTotals, Keys::PerCountry(1), SOMETIMES),
    // A row for every live country and the world's.
    cost(TableId::TerritoryTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::CountryFactories, Keys::PerCountry(1), SOMETIMES),
    // Stated: a row per country and nuclear type held, two types.
    cost(TableId::Nukes, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::CountryNukeTotals, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::WorldTension, Keys::Fixed(1), DAILY),
    cost(TableId::ThreatCountryAggregates, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::ThreatTotals, Keys::Fixed(1), DAILY),
    // A row per faction with a member: stated, a few dozen at most in any campaign seen.
    cost(TableId::FactionTotals, Keys::Fixed(32), SOMETIMES),
    // 1,132, 672, 256 and 28 rows measured.
    cost(TableId::AirForceParts, Keys::PerCountry(3), SOMETIMES),
    cost(TableId::AirForceWingKinds, Keys::PerCountry(2), SOMETIMES),
    cost(TableId::NavyClasses, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::MilitaryAid, Keys::PerCountry(1), SOMETIMES),
    // Banked experience moves every day.
    cost(TableId::CountryExperience, Keys::PerCountry(1), DAILY),
    // A row per subject: at most one per country.
    cost(TableId::Autonomy, Keys::PerCountry(1), SOMETIMES),
    // A tank a country, read and moving every day.
    cost(TableId::CountryFuel, Keys::PerCountry(1), DAILY),
    // Stated: a row per country and fuel consumer, three consumers.
    cost(TableId::CountryFuelConsumers, Keys::PerCountry(3), SOMETIMES),
    // Stated: a row per holder that lost or sank ships, by attribution and kind: four.
    cost(TableId::NavalLossTotals, Keys::PerCountry(4), SOMETIMES),
    // Logs: their facet values' combinations in a block, stated.
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
    // Counted by no facet: one key.
    cost(TableId::PeaceConferenceSpans, Keys::Fixed(1), SOMETIMES),
    cost(TableId::OperationCompletions, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::RaidEnds, Keys::PerCountry(1), SOMETIMES),
    cost(TableId::PeaceSettlements, Keys::Fixed(1), SOMETIMES),
    cost(TableId::NuclearStrikes, Keys::Fixed(1), SOMETIMES),
    // Stated: the leaders whose command ended in a block, at most one a country.
    cost(TableId::LeaderCommands, Keys::PerCountry(1), SOMETIMES),
    // A row per finalized land combat: 28,220 over blocks 72 to 110, one an hour on average.
    cost(TableId::LandCombatResults, Keys::Fixed(1), EVERY_BUCKET),
    // A key per battle record: 11,510 records over blocks 72 to 110 (295 a block); three times that.
    cost(TableId::CombatLogBattleModifiers, Keys::PerCountry(2), SOMETIMES),
    // Lifetimes: rows measured (GroupSpans 514, ArmyMix 2,029, ResearchCoverage 1,580,
    // CasualtyLifetimes 1,420), each coming or going a few times a block.
    cost(TableId::TagHolders, Keys::PerCountry(1), 2),
    cost(TableId::GroupSpans, Keys::PerCountry(2), 2),
    cost(TableId::WarEvents, Keys::PerCountry(1), 2),
    cost(TableId::LookSpans, Keys::PerCountry(1), 2),
    cost(TableId::ArmyMix, Keys::PerCountry(5), 2),
    cost(TableId::ResearchCoverage, Keys::PerCountry(4), 2),
    cost(TableId::OperativeMemberships, Keys::PerCountry(4), 2),
    cost(TableId::Raids, Keys::PerCountry(1), 2),
    cost(TableId::CasualtyLifetimes, Keys::PerCountry(4), 2),
    // Stated: the construction lines a block holds, live or ended in it, six a country.
    cost(TableId::ConstructionProjects, Keys::PerCountry(6), 2),
    // 1,567 rows at the campaign's end, none deleted over its last 74 blocks: block 123 held fewer.
    cost(TableId::UnitLeaders, Keys::PerCountry(4), 2),
    // At most 11 live at once and 86 begun over the last 35 blocks measured.
    cost(TableId::Operations, Keys::PerCountry(1), 2),
    // A row per live naval combat: 2,120 combats begun over blocks 72 to 110 (54 a block), at most
    // 6 live at once.
    cost(TableId::NavalBattles, Keys::PerCountry(1), 2),
    // Stated: none recorded in the 13-year recording. A result per naval combat, which the game
    // keeps for years: 54 combats a block is about 650 a year; eight a country holds five years.
    cost(TableId::NavalResults, Keys::PerCountry(8), 2),
    // 27,000 rows measured over 439 countries. Most leaves are still; a fighting country's drain
    // leaves move hourly.
    cost(TableId::EquipmentLossTotals, Keys::PerCountry(62), SOMETIMES),
    // Stated: the regions a country fought an air war over, eight a country; a row comes once and
    // stays while its year window holds a value.
    cost(TableId::AirStatistics, Keys::PerCountry(8), 2),
    // Stated: a party a vanilla ideology group, its popularity drifting daily.
    cost(TableId::PoliticalParties, Keys::PerCountry(4), DAILY),
    // Stated: production spending's keys, so a producer's read passes over the blocks it had none
    // in (its country totals are priced as a series above). A row per design and purpose it applied
    // input to in a block, eight a country; a total, an unattributed rest and the repaired designs,
    // three a country.
    cost(TableId::ProductionTotals, Keys::PerCountry(8), 2),
    cost(TableId::DockyardRepairRates, Keys::PerCountry(3), 2),
    // Stated: a row per owner with an air object and region it has an entry in, about six a live
    // country; a region's scores move at its daily phase slot.
    cost(TableId::RaidAirScores, Keys::PerCountry(6), DAILY),
    // Stated: a producer's every type, rest and archetypes, ProductionTypeTotals' three a country
    // and two more; its totals move with production's daily reads.
    cost(TableId::ProducedValues, Keys::PerCountry(5), DAILY),
];

/// One summarized table's price in a world.
pub struct Price {
    pub table: TableId,
    pub shape: Shape,
    pub load: Load,
    /// What its builder holds at the block's end.
    pub held: u64,
    /// Its block's piece, raw.
    pub piece: u64,
}

/// Every summarized table's price in `world`; an error names a table no cost states.
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

/// What every declaration's summaries hold at the heaviest moment of a block in `world`: one
/// closing a span, its tables finished and folded in [`summaries`]' order, the recorder's.
pub fn total(world: World) -> io::Result<u64> {
    let tables: Vec<(Shape, Load)> = prices(world)?.into_iter().map(|price| (price.shape, price.load)).collect();
    Ok(cost::peak(&tables))
}

/// The tables declared, for a reader of [`prices`] that wants each declaration's own name.
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

    /// Every summarized table states its cost in both worlds, and every stated cost is a summarized
    /// table's: the model the memory gate is read against covers what the recorder summarizes.
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