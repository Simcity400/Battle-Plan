use dashboard_base::products::{Registered, at_hour};

pub const AT_HOUR: &[at_hour::Entry] = &[
    dashboard_land::registrations::ARMY_TEMPLATEROSTER,
    dashboard_air::registrations::AIR_STATISTICS_AIRSTATISTICSAT,
    dashboard_politics::registrations::POLITICS_STATE_POLITICSSTATE,
    dashboard_politics::registrations::POLITICS_ELECTIONS_NEXTELECTIONS,
    dashboard_politics::registrations::COUNTRY_PEOPLE_COUNTRYPEOPLE,
    dashboard_world::registrations::TERRITORY_TERRITORYSHARE,
    dashboard_world::registrations::TERRITORY_TERRITORYBREAKDOWN,
    dashboard_wars::registrations::FACTIONS_FACTIONROSTERS,
    dashboard_world::registrations::SCALARS_GAMESCORE,
    dashboard_world::registrations::SCALARS_NUCLEARARSENAL,
    dashboard_world::registrations::SCALARS_WORLDTENSION,
    dashboard_world::registrations::SCALARS_FACTORYPARTS,
    dashboard_land::registrations::EXPERIENCE_COUNTRYEXPERIENCE,
    dashboard_land::registrations::TRAINING_QUEUE_TRAININGQUEUE,
    dashboard_wars::registrations::DIPLOMACY_DIPLOMATICRELATIONS,
    dashboard_wars::registrations::CASUALTY_MATRIX_CASUALTYMATRIX,
    dashboard_wars::registrations::WARGOALS_WARGOALS,
    dashboard_intelligence::registrations::INTELLIGENCE_DECRYPTION_DECRYPTION,
    dashboard_intelligence::registrations::INTELLIGENCE_OPERATIVE_MATRIX_OPERATIVEMATRIX,
    dashboard_intelligence::registrations::INTELLIGENCE_AGENCIES_AGENCYCOMPARISON,
    dashboard_intelligence::registrations::INTELLIGENCE_ROSTER_OPERATIVESROSTER,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEOPERATIONS,
    dashboard_wars::registrations::OCCUPATION_RESERVE_OCCUPATIONRESERVE,
    dashboard_intelligence::registrations::INTEL_COVERAGE_INTELNETWORKCOVERAGE,
    dashboard_economy::registrations::ECONOMY_FUEL_BRANCHES_FUELBRANCHES,
    dashboard_economy::registrations::ECONOMY_RESOURCES_RESOURCEBALANCE,
    dashboard_economy::registrations::ECONOMY_LEND_LEASE_BALANCE_LENDLEASEBALANCE,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINESAT,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLICENCES,
    dashboard_production::registrations::PRODUCTION_INDUSTRIALORGANISATIONS,
    dashboard_production::registrations::PRODUCTION_EQUIPMENTDESIGNS,
    dashboard_land::registrations::DOCTRINE_MASTERY_DOCTRINEMASTERY,
    dashboard_land::registrations::DOCTRINE_MASTERY_REWARDS_DOCTRINEREWARDS,
    dashboard_naval::registrations::MINEFIELDS_MINEFIELDS,
    dashboard_naval::registrations::MINEFIELDS_MINEFIELDREPORTS,
    dashboard_naval::registrations::NAVAL_YARDS_NAVALREPAIRS,
    dashboard_naval::registrations::NAVAL_YARDS_NAVALREFITS,
    dashboard_land::registrations::LEADERS_UNITLEADERS,
    dashboard_land::registrations::LEADERS_CAREER_LEADERHOUR,
    dashboard_naval::registrations::NAVAL_MANAGEMENT_NAVALMANAGEMENT,
    dashboard_air::registrations::AIR_ACES_AIRACESAT,
    dashboard_air::registrations::REGIONAL_WEATHER_WEATHERPROVINCES,
    dashboard_politics::registrations::FOCUS_FOCUSUNAVAILABLEAT,
    dashboard_politics::registrations::STABILITY_COUNTRYSTABILITYBREAKDOWN,
    dashboard_politics::registrations::STABILITY_STABILITYPARTNAMES,
    dashboard_land::registrations::DIVISIONS_TEMPLATES_DIVISIONTEMPLATES,
    dashboard_land::registrations::DIVISIONS_CAREERS_DIVISIONCAREERS,
    dashboard_land::registrations::DIVISIONS_CAREERS_DIVISIONRECORD,
    dashboard_land::registrations::DIVISIONS_EQUIPMENT_DIVISIONEQUIPMENT,
    dashboard_research::registrations::RESEARCH_SLOTS_RESEARCHSLOTS,
    dashboard_air::registrations::AIR_MISSIONS_AIRMISSIONSAT,
    dashboard_air::registrations::AIR_MISSIONS_AIRMISSIONWINGSAT,
    dashboard_wars::registrations::OCCUPATION_STATES_OCCUPIEDSTATES,
    dashboard_wars::registrations::OCCUPATION_DETAIL_OCCUPATIONDETAIL,
    dashboard_economy::registrations::ECONOMY_MARKET_MARKETLENDLEASE,
    dashboard_economy::registrations::ECONOMY_FUEL_COUNTRYFUEL,
    dashboard_production::registrations::STOCKPILE_EQUIPMENTSTOCKPILEAT,
    dashboard_production::registrations::STOCKPILE_STOCKPILEARTWORK,
    dashboard_wars::registrations::FACTION_STATE_DETAILS_FACTIONDETAILS,
    dashboard_wars::registrations::FACTION_STATE_COUNTRY_GOALS_COUNTRYFACTIONGOALS,
    dashboard_wars::registrations::WARS_CONTRIBUTIONS_WARCONTRIBUTIONS,
    dashboard_research::registrations::SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTDETAILSAT,
    dashboard_research::registrations::SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTEFFECTSAT,
    dashboard_research::registrations::SPECIAL_PROJECTS_DETAILS_SPECIALPROJECTITERATIONAT,
    dashboard_research::registrations::TECHNOLOGY_EFFECTS_TECHNOLOGYEFFECTSAT,
    dashboard_research::registrations::ENTITY_STATS_ENTITYSTATSAT,
    dashboard_air::registrations::AIR_SUPERIORITY_AIRSUPERIORITYAT,
    dashboard_production::registrations::EQUIPMENT_LOSSES_EQUIPMENTLOSSHEAD,
    dashboard_land::registrations::ARMY_LOSSES_ARMYLOSSES,
    dashboard_politics::registrations::POWER_LEDGER_DAY_POWERDAY,
    dashboard_wars::registrations::FACTION_STATE_REWARDS_FACTIONGOALREWARDS,
];

const OWN: &[Registered] = &[
    dashboard_players::registrations::CAMPAIGN_CLOCK_CAMPAIGNCLOCK,
    dashboard_players::registrations::PLAYER_APM_PLAYERAPM,
    dashboard_politics::registrations::NATIONAL_FOCUS_NATIONALFOCUS,
    dashboard_politics::registrations::FOCUS_ARTWORK_FOCUSARTWORK,
    dashboard_politics::registrations::TECHNOLOGY_ARTWORK_TECHNOLOGYARTWORK,
    dashboard_world::registrations::COUNTRY_FLAGS_COUNTRYFLAGS,
    dashboard_land::registrations::DOCTRINES_DOCTRINES,
    dashboard_air::registrations::AIRCRAFT_CATALOGUE_AIRCRAFTCATALOGUE,
    dashboard_world::registrations::ARTWORK_SPRITEARTWORK,
    dashboard_land::registrations::ARMY_ARMYSIZE,
    dashboard_world::registrations::IDENTITIES_COUNTRYIDENTITIES,
    dashboard_world::registrations::STYLES_COUNTRYFACTS,
    dashboard_world::registrations::STYLES_COUNTRYSTYLES,
    dashboard_world::registrations::GROUPS_COUNTRYGROUPS,
    dashboard_world::registrations::LAND_OWNERS_LANDOWNERSPRODUCT,
    dashboard_politics::registrations::LOGS_LOGPAGE,
    dashboard_politics::registrations::LOGS_LOGTAIL,
    dashboard_politics::registrations::LOGS_LOGCOUNTS,
    dashboard_air::registrations::AIR_LEDGER_MILITARYAIRLEDGER,
    dashboard_politics::registrations::POLITICS_HISTORY_POLITICSSTATEHISTORY,
    dashboard_politics::registrations::POLITICS_CIVIL_WARS_CIVILWARS,
    dashboard_wars::registrations::FACTIONS_FACTIONLINES,
    dashboard_world::registrations::SCALARS_THREATEVENTS,
    dashboard_world::registrations::SCALARS_FACTORYGROUPS,
    dashboard_land::registrations::FORCES_AIRFORCESIZE,
    dashboard_land::registrations::FORCES_NAVYSIZE,
    dashboard_wars::registrations::VOLUNTEERS_VOLUNTEERSEXPEDITIONARIES,
    dashboard_world::registrations::SUBJECTS_SUBJECTS,
    dashboard_wars::registrations::DIPLOMACY_DIPLOMATICPACTS,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGELOG,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEOPERATION,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEENDED,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGETYPES,
    dashboard_intelligence::registrations::RAIDS_RAIDLOG,
    dashboard_economy::registrations::ECONOMY_RESOURCES_RESOURCESIZES,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINES,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINEDETAIL,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINEBREAKDOWN,
    dashboard_world::registrations::PLACE_NAMES_PLACENAMES,
    dashboard_world::registrations::SEA_REGIONS_STRATEGICREGIONS,
    dashboard_naval::registrations::MINEFIELDS_MINEFIELDHISTORY,
    dashboard_naval::registrations::NAVAL_SUPREMACY_NAVALSUPREMACY,
    dashboard_land::registrations::LEADERS_LEADERPORTRAITS,
    dashboard_land::registrations::LEADERS_CAREER_LEADERCAREER,
    dashboard_land::registrations::LEADERS_CAREER_LEADERCHOICES,
    dashboard_air::registrations::REGIONAL_WEATHER_REGIONALWEATHER,
    dashboard_politics::registrations::FOCUS_FOCUSHOURPRODUCT,
    dashboard_politics::registrations::FOCUS_FOCUSORDERPAGEPRODUCT,
    dashboard_politics::registrations::FOCUS_FOCUSCOMPLETIONSPRODUCT,
    dashboard_politics::registrations::FOCUS_CARD_FOCUSCARD,
    dashboard_politics::registrations::IDEAS_IDEAHISTORY,
    dashboard_politics::registrations::PARTY_HISTORY_PARTYHISTORY,
    dashboard_wars::registrations::CASUALTIES_CASUALTYPAIRS,
    dashboard_wars::registrations::CASUALTIES_CASUALTYBREAKDOWN,
    dashboard_wars::registrations::CASUALTIES_CASUALTYLABELS,
    dashboard_wars::registrations::PEACE_PEACEDEALS,
    dashboard_wars::registrations::TERRITORY_TRANSFERS_TERRITORYTRANSFERS,
    dashboard_economy::registrations::CONVOYS_CONVOYHEALTH,
    dashboard_economy::registrations::TRADE_FLOWS_TRADEFLOWS,
    dashboard_production::registrations::PRODUCTION_SPENDING_PRODUCTIONSPENDING,
    dashboard_wars::registrations::OCCUPATION_RESERVE_RESERVEHISTORY,
    dashboard_production::registrations::CONSTRUCTION_LOG_CONSTRUCTIONLOGMANIFEST,
    dashboard_production::registrations::CONSTRUCTION_LOG_CONSTRUCTIONLOG,
    dashboard_production::registrations::CONSTRUCTION_SPENDING_CONSTRUCTIONSPENDING,
    dashboard_land::registrations::DIVISION_LOG_DIVISIONLOG,
    dashboard_land::registrations::DIVISION_LOG_DIVISIONLOGMANIFEST,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLESMANIFEST,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLES,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLESRANKING,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLECOURSES,
    dashboard_land::registrations::LAND_TERRAIN_LANDBATTLETERRAINS,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLESMANIFEST,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLES,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLESRANKING,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLEDETAILS,
    dashboard_naval::registrations::NAVAL_SUNK_SHIPSSUNKMANIFEST,
    dashboard_naval::registrations::NAVAL_SUNK_SHIPSSUNK,
    dashboard_naval::registrations::NAVAL_LOSSES_NAVALLOSSES,
    dashboard_research::registrations::RESEARCH_RESEARCH,
    dashboard_research::registrations::RESEARCH_BREAKDOWN_RESEARCHBREAKDOWN,
    dashboard_politics::registrations::FEED_EVENTFEED,
    dashboard_politics::registrations::FEED_EVENTCHOICES,
    dashboard_politics::registrations::FEED_DECISIONS,
    dashboard_politics::registrations::FEED_EVENTFEEDSPAN,
    dashboard_economy::registrations::ECONOMY_MARKET_LEDGER_MARKETLEDGER,
    dashboard_economy::registrations::ECONOMY_MARKET_LEDGER_MARKETLEDGERTAIL,
    dashboard_economy::registrations::ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDS,
    dashboard_economy::registrations::ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDHISTORY,
    dashboard_economy::registrations::ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMAND,
    dashboard_economy::registrations::ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMANDHISTORY,
    dashboard_production::registrations::STOCKPILE_EQUIPMENTSTOCKPILECATALOGUE,
    dashboard_production::registrations::STOCKPILE_EQUIPMENTSTOCKPILECHART,
    dashboard_wars::registrations::FACTION_STATE_HISTORY_FACTIONSTATE,
    dashboard_wars::registrations::WARS_WARS,
    dashboard_wars::registrations::WARS_WARLOSSES,
    dashboard_research::registrations::SPECIAL_PROJECTS_SPECIALPROJECTS,
    dashboard_air::registrations::AIR_SUPERIORITY_AIRSUPERIORITYCHOICES,
    dashboard_land::registrations::COMBAT_LOG_COMBATLOG,
    dashboard_production::registrations::EQUIPMENT_LOSSES_EQUIPMENTLOSSES,
    dashboard_air::registrations::AIR_STATISTICS_AIRSTATISTICSREGIONS,
    dashboard_wars::registrations::SCOREBOARD_SCOREBOARDCOLUMNS,
    dashboard_wars::registrations::SCOREBOARD_SCOREBOARD,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCED,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDTYPES,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDCOUNTRY,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDAT,
    dashboard_politics::registrations::POWER_LEDGER_POWERSPENDING,
    dashboard_politics::registrations::POWER_LEDGER_BREAKDOWN_POWERSPENDINGBREAKDOWN,
    dashboard_politics::registrations::STABILITY_READS_STABILITYREADS,
    dashboard_air::registrations::AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE,
    dashboard_air::registrations::AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYRANKING,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGETYPETOKENS,
];

const ALL: [Registered; OWN.len() + AT_HOUR.len()] = at_hour::joined(OWN, AT_HOUR);
pub static PRODUCTS: &[Registered] = &ALL;

const MOVED_OWN: &[&str] = &[
    dashboard_land::registrations::ARMY_ARMYSIZE.key,
    dashboard_world::registrations::IDENTITIES_COUNTRYIDENTITIES.key,
    dashboard_world::registrations::STYLES_COUNTRYFACTS.key,
    dashboard_world::registrations::STYLES_COUNTRYSTYLES.key,
    dashboard_world::registrations::GROUPS_COUNTRYGROUPS.key,
    dashboard_world::registrations::LAND_OWNERS_LANDOWNERSPRODUCT.key,
    dashboard_politics::registrations::LOGS_LOGPAGE.key,
    dashboard_politics::registrations::LOGS_LOGTAIL.key,
    dashboard_politics::registrations::LOGS_LOGCOUNTS.key,
    dashboard_air::registrations::AIR_LEDGER_MILITARYAIRLEDGER.key,
    dashboard_politics::registrations::POLITICS_HISTORY_POLITICSSTATEHISTORY.key,
    dashboard_politics::registrations::POLITICS_CIVIL_WARS_CIVILWARS.key,
    dashboard_wars::registrations::FACTIONS_FACTIONLINES.key,
    dashboard_world::registrations::SCALARS_THREATEVENTS.key,
    dashboard_world::registrations::SCALARS_FACTORYGROUPS.key,
    dashboard_land::registrations::FORCES_AIRFORCESIZE.key,
    dashboard_land::registrations::FORCES_NAVYSIZE.key,
    dashboard_wars::registrations::VOLUNTEERS_VOLUNTEERSEXPEDITIONARIES.key,
    dashboard_world::registrations::SUBJECTS_SUBJECTS.key,
    dashboard_wars::registrations::DIPLOMACY_DIPLOMATICPACTS.key,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGELOG.key,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEOPERATION.key,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGEENDED.key,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGETYPES.key,
    dashboard_intelligence::registrations::RAIDS_RAIDLOG.key,
    dashboard_economy::registrations::ECONOMY_RESOURCES_RESOURCESIZES.key,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINES.key,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINEDETAIL.key,
    dashboard_production::registrations::PRODUCTION_PRODUCTIONLINEBREAKDOWN.key,
    dashboard_world::registrations::PLACE_NAMES_PLACENAMES.key,
    dashboard_world::registrations::SEA_REGIONS_STRATEGICREGIONS.key,
    dashboard_naval::registrations::MINEFIELDS_MINEFIELDHISTORY.key,
    dashboard_naval::registrations::NAVAL_SUPREMACY_NAVALSUPREMACY.key,
    dashboard_land::registrations::LEADERS_LEADERPORTRAITS.key,
    dashboard_land::registrations::LEADERS_CAREER_LEADERCAREER.key,
    dashboard_land::registrations::LEADERS_CAREER_LEADERCHOICES.key,
    dashboard_air::registrations::REGIONAL_WEATHER_REGIONALWEATHER.key,
    dashboard_politics::registrations::NATIONAL_FOCUS_NATIONALFOCUS.key,
    dashboard_politics::registrations::FOCUS_FOCUSHOURPRODUCT.key,
    dashboard_politics::registrations::FOCUS_FOCUSORDERPAGEPRODUCT.key,
    dashboard_politics::registrations::FOCUS_FOCUSCOMPLETIONSPRODUCT.key,
    dashboard_politics::registrations::FOCUS_CARD_FOCUSCARD.key,
    dashboard_politics::registrations::IDEAS_IDEAHISTORY.key,
    dashboard_politics::registrations::PARTY_HISTORY_PARTYHISTORY.key,
    dashboard_wars::registrations::CASUALTIES_CASUALTYPAIRS.key,
    dashboard_wars::registrations::CASUALTIES_CASUALTYBREAKDOWN.key,
    dashboard_wars::registrations::CASUALTIES_CASUALTYLABELS.key,
    dashboard_wars::registrations::PEACE_PEACEDEALS.key,
    dashboard_wars::registrations::TERRITORY_TRANSFERS_TERRITORYTRANSFERS.key,
    dashboard_economy::registrations::CONVOYS_CONVOYHEALTH.key,
    dashboard_economy::registrations::TRADE_FLOWS_TRADEFLOWS.key,
    dashboard_production::registrations::PRODUCTION_SPENDING_PRODUCTIONSPENDING.key,
    dashboard_wars::registrations::OCCUPATION_RESERVE_RESERVEHISTORY.key,
    dashboard_production::registrations::CONSTRUCTION_LOG_CONSTRUCTIONLOGMANIFEST.key,
    dashboard_production::registrations::CONSTRUCTION_LOG_CONSTRUCTIONLOG.key,
    dashboard_production::registrations::CONSTRUCTION_SPENDING_CONSTRUCTIONSPENDING.key,
    dashboard_land::registrations::DIVISION_LOG_DIVISIONLOG.key,
    dashboard_land::registrations::DIVISION_LOG_DIVISIONLOGMANIFEST.key,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLESMANIFEST.key,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLES.key,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLESRANKING.key,
    dashboard_land::registrations::LAND_BATTLES_LANDBATTLECOURSES.key,
    dashboard_land::registrations::LAND_TERRAIN_LANDBATTLETERRAINS.key,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLESMANIFEST.key,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLES.key,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLESRANKING.key,
    dashboard_naval::registrations::NAVAL_BATTLES_NAVALBATTLEDETAILS.key,
    dashboard_naval::registrations::NAVAL_SUNK_SHIPSSUNKMANIFEST.key,
    dashboard_naval::registrations::NAVAL_SUNK_SHIPSSUNK.key,
    dashboard_naval::registrations::NAVAL_LOSSES_NAVALLOSSES.key,
    dashboard_research::registrations::RESEARCH_RESEARCH.key,
    dashboard_research::registrations::RESEARCH_BREAKDOWN_RESEARCHBREAKDOWN.key,
    dashboard_politics::registrations::FEED_EVENTFEED.key,
    dashboard_politics::registrations::FEED_EVENTCHOICES.key,
    dashboard_politics::registrations::FEED_DECISIONS.key,
    dashboard_politics::registrations::FEED_EVENTFEEDSPAN.key,
    dashboard_economy::registrations::ECONOMY_MARKET_LEDGER_MARKETLEDGER.key,
    dashboard_economy::registrations::ECONOMY_MARKET_LEDGER_MARKETLEDGERTAIL.key,
    dashboard_economy::registrations::ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDS.key,
    dashboard_economy::registrations::ECONOMY_FUEL_AWARDS_COUNTRYFUELAWARDHISTORY.key,
    dashboard_economy::registrations::ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMAND.key,
    dashboard_economy::registrations::ECONOMY_FUEL_DEMAND_COUNTRYFUELDEMANDHISTORY.key,
    dashboard_production::registrations::STOCKPILE_EQUIPMENTSTOCKPILECATALOGUE.key,
    dashboard_production::registrations::STOCKPILE_EQUIPMENTSTOCKPILECHART.key,
    dashboard_wars::registrations::FACTION_STATE_HISTORY_FACTIONSTATE.key,
    dashboard_wars::registrations::WARS_WARS.key,
    dashboard_wars::registrations::WARS_WARLOSSES.key,
    dashboard_research::registrations::SPECIAL_PROJECTS_SPECIALPROJECTS.key,
    dashboard_air::registrations::AIR_SUPERIORITY_AIRSUPERIORITYCHOICES.key,
    dashboard_land::registrations::COMBAT_LOG_COMBATLOG.key,
    dashboard_production::registrations::EQUIPMENT_LOSSES_EQUIPMENTLOSSES.key,
    dashboard_air::registrations::AIR_STATISTICS_AIRSTATISTICSREGIONS.key,
    dashboard_wars::registrations::SCOREBOARD_SCOREBOARDCOLUMNS.key,
    dashboard_wars::registrations::SCOREBOARD_SCOREBOARD.key,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCED.key,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDTYPES.key,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDCOUNTRY.key,
    dashboard_production::registrations::EQUIPMENT_PRODUCED_EQUIPMENTPRODUCEDAT.key,
    dashboard_politics::registrations::POWER_LEDGER_POWERSPENDING.key,
    dashboard_politics::registrations::POWER_LEDGER_BREAKDOWN_POWERSPENDINGBREAKDOWN.key,
    dashboard_politics::registrations::STABILITY_READS_STABILITYREADS.key,
    dashboard_air::registrations::AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYTIMELINE.key,
    dashboard_air::registrations::AIR_SUPERIORITY_TIMELINE_AIRSUPERIORITYRANKING.key,
    dashboard_intelligence::registrations::INTELLIGENCE_ESPIONAGE_ESPIONAGETYPETOKENS.key,
];

const MOVED_ALL: [&str; MOVED_OWN.len() + AT_HOUR.len()] = at_hour::keys(MOVED_OWN, AT_HOUR);
pub const MOVED: &[&str] = &MOVED_ALL;

pub fn find(key: &str) -> Option<&'static Registered> {
    PRODUCTS.iter().find(|product| product.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::{CountryParams, validate_tag};

    #[test]
    fn exported_product_type_names_do_not_hide_different_shapes() {
        let mut declarations = std::collections::BTreeMap::new();
        for product in PRODUCTS {
            for declaration in product.types.expect("every exported product registers its types")() {
                let name = declaration.strip_prefix("type ").unwrap().split_whitespace().next().unwrap().to_owned();
                if let Some(previous) = declarations.insert(name.clone(), declaration.clone()) {
                    assert_eq!(previous, declaration, "different exported types named {name}");
                }
            }
        }
    }

    #[test]
    fn tag_validation_preserves_long_recorded_lookup_keys() {
        for tag in ["x".repeat(129), "å›½".repeat(129)] {
            assert_eq!(validate_tag(&tag), Ok(()));
            assert_eq!(
                CountryParams {
                    branch_id: "1".into(),
                    country: tag.into(),
                }
                .validate(),
                Ok(())
            );
        }
        for tag in ["", "\0", "tag\0suffix"] {
            assert_eq!(validate_tag(tag), Err("Invalid country tag"));
        }
    }

    #[test]
    fn browser_interactive_products_match_service_admission() {
        let contract = crate::typescript_contract();
        let declaration = contract
            .lines()
            .find(|line| line.starts_with("export const INTERACTIVE_PRODUCTS"))
            .expect("browser admission needs the service's product lanes");
        let replays = contract
            .lines()
            .find(|line| line.starts_with("export const REPLAY_PRODUCTS"))
            .expect("browser admission needs the service's replaying pages");
        for product in PRODUCTS {
            assert_eq!(
                declaration.contains(&format!("{:?}", product.key)),
                product.interactive,
                "{} admission differs between browser and service",
                product.key
            );
            assert_eq!(replays.contains(&format!("{:?}", product.key)), product.replays, "{} replays differently in browser and service", product.key);
        }
    }

    #[test]
    fn product_keys_are_unique() {
        let mut keys: Vec<_> = PRODUCTS.iter().map(|product| product.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRODUCTS.len());
        if let Some(path) = std::env::var_os("BATTLEPLAN_TYPESCRIPT_OUTPUT").or_else(|| std::env::var_os("HOI4_TYPESCRIPT_OUTPUT")) {
            std::fs::write(path, crate::typescript_contract()).unwrap();
        }
    }

}
