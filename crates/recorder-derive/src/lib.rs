use recorder_derive_air::{air_force, air_ledger_totals, air_missions, air_owner_days, air_region_activity, air_region_days, air_region_wings, air_theatre_regions, nuke_totals, raid_ends};
use recorder_derive_economy::{aid, construction_projects, construction_spend, fuel_days, fuel_demand, produced_values, production, supply_health, trade_flows};
use recorder_derive_equipment::{division_equipment, equipment_losses, stock};
use recorder_derive_land::{army, casualties, deployed, division_events, division_kills, experience, garrison_strength, land_battles, leader_battles, leader_commands, leader_traits};
use recorder_derive_naval::{dockyard_repairs, naval_battles, naval_control, naval_losses, navy};
use recorder_derive_politics::{faction_accounts, faction_totals, game_score, peace, power_ledger, research, stability, stability_parts, threat_totals, threats};
use recorder_derive_world::{drops, feed, groups, identity, looks, occupation, territory, territory_regions};
use recorder_derive_framework::framework::{Columns, Writer, Writers};

#[cfg(test)]
mod tests;

pub fn writers() -> Writers {
    Writers::new(writer_list())
}

pub fn writers_within(columns: &[usize], current: &[usize]) -> Writers {
    Writers::new(
        writer_list()
            .into_iter()
            .filter(|writer| {
                writer.tables().iter().all(|output| {
                    let index = output.table().index();
                    columns.get(index).is_some_and(|&held| held == current[index])
                }) && writer.reads().iter().all(|read| {
                    let index = read.table.index();
                    let Some(&held) = columns.get(index) else {
                        return false;
                    };
                    match read.columns {
                        Columns::Every => true,
                        Columns::These(needed) => needed.iter().all(|&col| col < held),
                    }
                })
            })
            .collect(),
    )
}

fn writer_list() -> Vec<Box<dyn Writer>> {
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
        Box::new(division_kills::DivisionKills),
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
