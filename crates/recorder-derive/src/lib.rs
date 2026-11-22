
pub mod aid;
pub mod air_force;
pub mod air_ledger_totals;
pub mod air_missions;
pub mod air_owner_days;
pub mod air_region_activity;
pub mod air_region_days;
pub mod air_region_wings;
pub mod air_theatre_regions;
pub mod army;
pub mod casualties;
pub mod construction_projects;
pub mod construction_spend;
pub mod deployed;
pub mod division_equipment;
pub mod dockyard_repairs;
pub mod division_events;
pub mod drops;
pub mod equipment_losses;
pub mod experience;
pub mod faction_accounts;
pub mod faction_totals;
pub mod feed;
mod forces;
pub mod framework;
pub mod fuel_days;
pub mod fuel_demand;
pub mod game_score;
pub mod garrison_strength;
pub mod groups;
mod holders;
pub mod identity;
pub mod land_battles;
pub mod leader_battles;
pub mod leader_commands;
pub mod leader_traits;
pub mod looks;
pub mod naval_battles;
pub mod naval_control;
pub mod naval_losses;
pub mod navy;
pub mod nuke_totals;
pub mod peace;
pub mod occupation;
mod producers;
pub mod raid_ends;
pub mod production;
pub mod produced_values;
pub mod power_ledger;
pub mod research;
pub mod stability;
pub mod stability_parts;
pub mod supply_health;
pub mod stock;
pub mod threat_totals;
pub mod territory;
pub mod territory_regions;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod threats;
pub mod trade_flows;

pub use framework::{Definitions, Facts, Writers};

pub fn writers() -> Writers {
    Writers::new(all_writers())
}

pub fn writers_within(tables: usize) -> Writers {
    Writers::new(
        all_writers()
            .into_iter()
            .filter(|writer| writer.tables().iter().all(|output| output.table().index() < tables) && writer.reads().iter().all(|read| read.table.index() < tables))
            .collect(),
    )
}

fn all_writers() -> Vec<Box<dyn framework::Writer>> {
    vec![
        Box::new(identity::TagHolders::default()),
        Box::new(army::Army::default()),
        Box::new(air_force::AirForce::default()),
        Box::new(air_force::AirForceComposition::default()),
        Box::new(navy::Navy::default()),
        Box::new(deployed::Deployed::default()),
        Box::new(aid::Aid::default()),
        Box::new(supply_health::SupplyHealth::default()),
        Box::new(stability::Stability::default()),
        Box::new(stability_parts::StabilityParts::default()),
        Box::new(territory::Territory::default()),
        Box::new(territory_regions::TerritoryRegions::default()),
        Box::new(groups::Groups::default()),
        Box::new(looks::Looks::default()),
        Box::new(drops::Drops::default()),
        Box::new(equipment_losses::EquipmentLosses::default()),
        Box::new(experience::Experience::default()),
        Box::new(game_score::GameScore::default()),
        Box::new(threats::Threats::default()),
        Box::new(stock::Stockpile::default()),
        Box::new(division_equipment::DivisionEquipment::default()),
        Box::new(trade_flows::TradeFlows::default()),
        Box::new(fuel_days::FuelDays::default()),
        Box::new(fuel_demand::FuelDemand::default()),
        Box::new(construction_spend::ConstructionSpend::default()),
        Box::new(construction_projects::ConstructionProjects::default()),
        Box::new(production::Production::default()),
        Box::new(produced_values::ProducedValues::default()),
        Box::new(dockyard_repairs::DockyardRepairs::default()),
        Box::new(land_battles::LandBattles::default()),
        Box::new(naval_battles::NavalBattles::default()),
        Box::new(naval_losses::NavalLosses::default()),
        Box::new(division_events::DivisionLog::default()),
        Box::new(leader_battles::LeaderBattles::default()),
        Box::new(leader_traits::LeaderTraits),
        Box::new(leader_commands::LeaderCommands::default()),
        Box::new(research::Research::default()),
        Box::new(faction_accounts::FactionAccounts::default()),
        Box::new(peace::Peace::default()),
        Box::new(casualties::Casualties::default()),
        Box::new(occupation::Occupation::default()),
        Box::new(garrison_strength::GarrisonStrength::default()),
        Box::new(air_missions::AirMissions::default()),
        Box::new(air_region_wings::AirRegionWings::default()),
        Box::new(naval_control::NavalControl::default()),
        Box::new(nuke_totals::NukeTotals::default()),
        Box::new(raid_ends::RaidEnds::default()),
        Box::new(threat_totals::ThreatTotals::default()),
        Box::new(faction_totals::FactionTotals::default()),
        Box::new(air_region_activity::AirRegionActivity::default()),
        Box::new(air_theatre_regions::AirTheatreRegions::default()),
        Box::new(air_region_days::AirRegionDays::default()),
        Box::new(air_owner_days::AirOwnerDays::default()),
        Box::new(air_ledger_totals::AirLossTotals::default()),
        Box::new(air_ledger_totals::BombingTotals::default()),
        Box::new(power_ledger::PowerLedger::default()),
        Box::new(feed::Feed::default()),
    ]
}
