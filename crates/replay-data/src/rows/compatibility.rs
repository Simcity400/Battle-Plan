pub use replay_rows::rows::compatibility::*;
use super::TableId;
#[cfg(test)]
use crate::FORMAT_VERSION;


#[cfg(test)]
pub const SAME_FRAME_ALIASES_FORMAT: u16 = 111;
#[cfg(test)]
pub const SAME_FRAME_OPERAND_ALIASES_FORMAT: u16 = 112;
#[cfg(test)]
pub const PRIVATE_DISPLAY_ALIASES_FORMAT: u16 = 115;
pub const STORED_ONCE_TABLES: [TableId; 5] = [
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::DivisionMovement,
    TableId::LandCombatDivisions,
    TableId::LandCombatDivisionModifiers,
];




#[cfg(test)]
pub const SUPPLY_FOOTER_FORMAT: u16 = 102;



























#[derive(Clone, Copy, Debug)]
pub struct RetiredField {
    pub table: TableId,
    pub index: usize,
    pub name: &'static str,
    pub retired: u16,
}

const fn retired(table: TableId, index: usize, name: &'static str) -> RetiredField {
    RetiredField {
        table,
        index,
        name,
        retired: RETIRED_INPUTS_FORMAT,
    }
}

pub static RETIRED_TABLES: [TableId; 19] = [
    TableId::NavalMineLevels,
    TableId::DivisionEtaInputs,
    TableId::AttritionModifierBreakdowns,
    TableId::ReservedNearestNavies,
    TableId::ReservedNearestBases,
    TableId::ReserveModifierBreakdowns,
    TableId::NavalModifierBreakdowns,
    TableId::OwnedWargoals,
    TableId::AirRegionHistory,
    TableId::AirRegionDamage,
    TableId::TransportNavies,
    TableId::TransportLedgers,
    TableId::TransportReceivers,
    TableId::TransportSearchRegions,
    TableId::TransportSpottingOwners,
    TableId::TransportSpottingRegions,
    TableId::TransportAirRoots,
    TableId::TransportAirRegions,
    TableId::TransportRiskShips,
];

pub static RETIRED_FIELDS: [RetiredField; 50] = [
    retired(TableId::Provinces, 8, "selected_areas"),
    retired(TableId::DivisionLandInputs, 1, "received_supply"),
    retired(TableId::DivisionLandInputs, 2, "maximum_grace"),
    retired(TableId::DivisionLandInputs, 3, "remaining_grace"),
    retired(TableId::CountryAppearance, 21, "transport_convoys"),
    retired(TableId::CountryCounterInputs, 13, "movement"),
    retired(TableId::RailwayGunCounterInputs, 29, "transport"),
    retired(TableId::AirBaseRegistrations, 3, "transport_planes"),
    retired(TableId::ReservedCountryNaming, 15, "movement"),
    retired(
        TableId::ReservedCountryNaming,
        21,
        "raw_country_observations",
    ),
    retired(
        TableId::ReservedCountryNaming,
        22,
        "transport_mission_inputs",
    ),
    retired(TableId::LandCombatModifiers, 1, "divisions"),
    retired(TableId::LandCombatDetails, 7, "sides.anti_air_attack"),
    retired(TableId::LandCombatDetails, 7, "sides.damage[2..6]"),
    retired(TableId::LandCombatDetails, 7, "sides.commander_experience"),
    retired(
        TableId::LandCombatDetails,
        7,
        "sides.commander_description_token",
    ),
    retired(TableId::LandCombatDetails, 7, "sides.timed_traits"),
    retired(TableId::LandCombatDetails, 7, "sides.learning_traits"),
    retired(TableId::LandCombatProgress, 0, "attacker_progress"),
    retired(TableId::LandCombatProgress, 1, "defender_progress"),
    retired(TableId::LandCombatProgress, 2, "direction_attacker"),
    retired(TableId::LandCombatProgress, 3, "direction_province"),
    retired(TableId::ShipModels, 7, "strike"),
    retired(TableId::CountryFuel, 2, "gain_raw"),
    retired(TableId::CountryFuel, 3, "cost_raw"),
    retired(TableId::CountryFuel, 4, "gain_from_states_raw"),
    retired(TableId::CountryFuel, 6, "lend_lease_consumption_raw"),
    retired(TableId::CountryFuel, 7, "gain_per_oil_raw"),
    retired(TableId::CountryFuel, 8, "history_index"),
    retired(TableId::CountryFuel, 9, "remaining_hours"),
    retired(TableId::CountryFuel, 13, "ring_requested_0_raw"),
    retired(TableId::CountryFuel, 14, "ring_requested_1_raw"),
    retired(TableId::CountryFuel, 15, "ring_requested_2_raw"),
    retired(TableId::CountryFuel, 20, "awards"),
    retired(TableId::Politics, 1, "election_frequency"),
    retired(TableId::Politics, 2, "elections_allowed"),
    retired(TableId::Capitulation, 2, "last_surrender_hour"),
    retired(TableId::Capitulation, 3, "faction_join_hour"),
    retired(TableId::Capitulation, 4, "legitimacy_raw"),
    retired(TableId::Capitulation, 5, "exile_army_leaders"),
    retired(TableId::ProductionLines, 8, "produced_raw"),
    retired(TableId::ProductionLineOutput, 4, "mio_credit"),
    retired(TableId::ProductionLineOutput, 10, "instant_raw"),
    retired(TableId::ProductionLineModifiers, 7, "conversion"),
    retired(TableId::InvasionPreparations, 9, "zero_war_present"),
    retired(TableId::InvasionPreparations, 10, "zero_war_retired"),
    retired(TableId::InvasionPreparations, 11, "zero_co_belligerent"),
    retired(TableId::InvasionPreparations, 12, "co_belligerents"),
    RetiredField {
        table: TableId::CountryMarket,
        index: 4,
        name: "auto_accept_market_access",
        retired: MARKET_AUTOMATION_FORMAT,
    },
    RetiredField {
        table: TableId::CountryMarket,
        index: 6,
        name: "auto_accept_purchase",
        retired: MARKET_AUTOMATION_FORMAT,
    },
];




#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    ReplayHeader,
    ReplayBlock,
    ReplayTrailer,
    ReplayFooter,
    SpoolEnvelope,
    Frame,
    Identity,
    SessionStart,
    BranchStart,
    Gap,
    RecoveryNote,
    Detach,
    ClockWitness,
    GapSummary,
    Partial,
    NavalCommanderInputs,
    LandBorderWarInputs,
    EventFeedEvent,
    EventLoss,
    PlayerCommands,
    AirCombatRecord,
}

impl RecordKind {
    pub const ALL: [Self; 21] = [
        Self::ReplayHeader,
        Self::ReplayBlock,
        Self::ReplayTrailer,
        Self::ReplayFooter,
        Self::SpoolEnvelope,
        Self::Frame,
        Self::Identity,
        Self::SessionStart,
        Self::BranchStart,
        Self::Gap,
        Self::RecoveryNote,
        Self::Detach,
        Self::ClockWitness,
        Self::GapSummary,
        Self::Partial,
        Self::NavalCommanderInputs,
        Self::LandBorderWarInputs,
        Self::EventFeedEvent,
        Self::EventLoss,
        Self::PlayerCommands,
        Self::AirCombatRecord,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Table(TableId),
    Record(RecordKind),
}

#[derive(Clone, Copy, Debug)]
pub struct LastChange {
    pub subject: Subject,
    pub introduced: u16,
    pub retired: Option<u16>,
    pub format: u16,
    pub commit: &'static str,
    pub kind: ChangeKind,
    pub appended: &'static [AppendedField],
    pub layout_since: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Unchanged,
    AppendedOnly,
    Layout,
    Meaning,
}


#[derive(Clone, Copy, Debug)]
pub struct TableGap {
    table: TableId,
    format: u16,
    cause: GapCause,
}

#[derive(Clone, Copy, Debug)]
enum GapCause {
    Introduced(u16),
    Retired(u16),
    Changed(u16),
}

impl std::fmt::Display for TableGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (table, format) = (self.table, self.format);
        match self.cause {
            GapCause::Introduced(introduced) => write!(
                f,
                "{table:?} not recorded at format {format} (introduced at format {introduced})"
            ),
            GapCause::Retired(retired) => write!(
                f,
                "{table:?} retired at format {retired}; this recording is format {format}"
            ),
            GapCause::Changed(changed) => write!(
                f,
                "{table:?} layout or meaning changed at format {changed}; this recording is format {format}"
            ),
        }
    }
}

pub static HISTORY: [LastChange; TableId::ALL.len() + RecordKind::ALL.len()] = {
    let baseline = LastChange {
        subject: Subject::Table(TableId::Clock),
        introduced: PROVEN_FROM,
        retired: None,
        format: PROVEN_FROM,
        commit: "0e632a9b16",
        kind: ChangeKind::Unchanged,
        appended: &[],
        layout_since: 0,
    };
    let mut history = [baseline; TableId::ALL.len() + RecordKind::ALL.len()];
    history[TableId::CountryWargoals.index()] = table(TableId::CountryWargoals, 104, "wargoals");
    history[TableId::CountryWargoals.index()].introduced = 104;
    history[TableId::WargoalTargets.index()] = table(TableId::WargoalTargets, 104, "wargoals");
    history[TableId::WargoalTargets.index()].introduced = 104;
    history[TableId::WargoalJustifications.index()] =
        table(TableId::WargoalJustifications, 104, "wargoals");
    history[TableId::WargoalJustifications.index()].introduced = 104;
    history[TableId::DoctrineCountries.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineCountries,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineFolders.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineFolders,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineTracks.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineTracks,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineLeaderMastery.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineLeaderMastery,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineCostReductions.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineCostReductions,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineMasteryGrants.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineMasteryGrants,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::DoctrineEquipmentBonuses.index()] = LastChange {
        introduced: DOCTRINES_FORMAT,
        ..table(
            TableId::DoctrineEquipmentBonuses,
            DOCTRINES_FORMAT,
            "pol/capture-doctrines",
        )
    };
    history[TableId::FactionAdvisorModifiers.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionAdvisorModifiers,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionMinimumUpgrades.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionMinimumUpgrades,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionDoctrine.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionDoctrine,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionProjection.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionProjection,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionResources.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionResources,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionResearch.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionResearch,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionAccounts.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionAccounts,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionVariables.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionVariables,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionRules.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionRules,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionPrograms.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionPrograms,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionUnlocks.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionUnlocks,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionManpower.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionManpower,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionGoalStatus.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionGoalStatus,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::FactionGoalEntries.index()] = LastChange {
        introduced: FACTION_STATE_FORMAT,
        ..table(
            TableId::FactionGoalEntries,
            FACTION_STATE_FORMAT,
            "pol/capture-factions",
        )
    };
    history[TableId::RocketProductionOutput.index()] = LastChange {
        introduced: WIDGET_CAPTURE_FORMAT,
        ..table(
            TableId::RocketProductionOutput,
            VARIANT_STAT_WITNESS_FORMAT,
            "variant stat lane witnesses",
        )
    };
    history[TableId::RaidSpeedInputs.index()] = table(
        TableId::RaidSpeedInputs,
        VARIANT_STAT_WITNESS_FORMAT,
        "variant stat lane witnesses",
    );
    history[TableId::ProductionLineOutput.index()] = appended_at(
        TableId::ProductionLineOutput,
        CONVERSION_PURPOSE_FORMAT,
        "conversion-purpose",
        &[
            AppendedField {
                index: 9,
                name: "spent_raw",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "instant_raw",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "purpose",
                introduced: CONVERSION_PURPOSE_FORMAT,
            },
        ],
    );
    let mut i = 0;
    while i < TableId::ALL.len() {
        history[i].subject = Subject::Table(TableId::ALL[i]);
        i += 1;
    }
    history[TableId::StateResistance.index()] = LastChange {
        kind: ChangeKind::Layout,
        ..appended_at(
            TableId::StateResistance,
            OCCUPATION_MAP_FORMAT,
            "occupation hover and map markers",
            &[
                AppendedField {
                    index: 6,
                    name: "hover",
                    introduced: OCCUPATION_HOVER_FORMAT,
                },
                AppendedField {
                    index: 7,
                    name: "modifier_sources",
                    introduced: OCCUPATION_HOVER_FORMAT,
                },
                AppendedField {
                    index: 8,
                    name: "map",
                    introduced: OCCUPATION_MAP_FORMAT,
                },
            ],
        )
    };
    history[TableId::ResearchTechnologies.index()] = table(
        TableId::ResearchTechnologies,
        POLITICAL_FORMAT,
        "technology-occurrences",
    );
    history[TableId::ResearchTechnologies.index()].appended = &[
        AppendedField {
            index: 5,
            name: "technology_handle",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 6,
            name: "applied_bonus_count",
            introduced: POLITICAL_FORMAT,
        },
    ];
    history[TableId::TechnologyDefinitions.index()] = table(
        TableId::TechnologyDefinitions,
        POLITICAL_FORMAT,
        "technology-occurrences",
    );
    history[TableId::ResearchSlots.index()] = appended_at(
        TableId::ResearchSlots,
        POLITICAL_FORMAT,
        "technology-occurrences",
        &[AppendedField {
            index: 5,
            name: "database_index",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::ResearchInputs.index()] = appended_at(
        TableId::ResearchInputs,
        POLITICAL_FORMAT,
        "technology-occurrences",
        &[AppendedField {
            index: 15,
            name: "database_index",
            introduced: POLITICAL_FORMAT,
        }],
    );
    let politics = [
        TableId::CountryPowerDaily,
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
        TableId::GovernmentChanges,
        TableId::PoliticsCaptureCoverage,
    ];
    let mut p = 0;
    while p < politics.len() {
        let id = politics[p];
        history[id.index()] = table(id, WIDGET_CAPTURE_FORMAT, "3b4f34f2ea");
        history[id.index()].introduced = WIDGET_CAPTURE_FORMAT;
        p += 1;
    }
    history[TableId::WarOwnedRelations.index()] = table(
        TableId::WarOwnedRelations,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarOwnedRelations.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::WarRelationGoals.index()] = table(
        TableId::WarRelationGoals,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarRelationGoals.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::WarOverviewCountries.index()] = table(
        TableId::WarOverviewCountries,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarOverviewCountries.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::WarNameRules.index()] = table(
        TableId::WarNameRules,
        WAR_OVERVIEW_FORMAT,
        "capture/w4-wars",
    );
    history[TableId::WarNameRules.index()].introduced = WAR_OVERVIEW_FORMAT;
    history[TableId::PeaceSettlements.index()] = table(
        TableId::PeaceSettlements,
        PEACE_SETTLEMENTS_FORMAT,
        "capture/w4-peace-deals",
    );
    history[TableId::PeaceSettlements.index()].introduced = PEACE_SETTLEMENTS_FORMAT;
    history[TableId::PeaceBids.index()] = table(
        TableId::PeaceBids,
        PEACE_SETTLEMENTS_FORMAT,
        "capture/w4-peace-deals",
    );
    history[TableId::PeaceBids.index()].introduced = PEACE_SETTLEMENTS_FORMAT;
    let mut r = 0;
    while r < RecordKind::ALL.len() {
        let kind = RecordKind::ALL[r];
        let (format, commit) = match kind {
            RecordKind::SessionStart => (24, "9ae9a5498c"),
            RecordKind::SpoolEnvelope => (33, "1b7474a555"),
            RecordKind::Frame => (91, "0e632a9b16"),
            RecordKind::ReplayBlock => (CHUNKED_FORMAT, "chunked-grouped-blocks"),
            RecordKind::GapSummary => (1, "7185d6e386"),
            RecordKind::Partial => (92, "11b905d2b2"),
            RecordKind::NavalCommanderInputs => (RECORDER_INPUTS_FORMAT, "d53fa4e05c"),
            RecordKind::LandBorderWarInputs => (RECORDER_INPUTS_FORMAT, "9d09404c0d"),
            RecordKind::EventFeedEvent => (SCRIPT_EVENT_FORMAT, "pol/capture-events-decisions"),
            RecordKind::EventLoss => (EVENT_LOSS_FORMAT, "record/reattach-loss"),
            RecordKind::PlayerCommands => (PLAYER_COMMANDS_FORMAT, "record/player-commands"),
            RecordKind::AirCombatRecord => (AIR_COMBAT_MENU_FORMAT, "air combat copied equipment"),
            _ => (1, "5d3b4dd945"),
        };
        history[i + r] = LastChange {
            subject: Subject::Record(kind),
            introduced: format,
            retired: None,
            format,
            commit,
            kind: if matches!(kind, RecordKind::Partial | RecordKind::ReplayBlock) {
                ChangeKind::Layout
            } else {
                ChangeKind::Unchanged
            },
            appended: &[],
            layout_since: 0,
        };
        r += 1;
    }
    history[TableId::NavalResultSources.index()] = table(
        TableId::NavalResultSources,
        WIDGET_CAPTURE_FORMAT,
        "widget-native-naval",
    );
    history[TableId::NavalResultSources.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::AirAces.index()] = appended_at(
        TableId::AirAces,
        GAME_SCORE_FORMAT,
        "17d59e11b4",
        &[AppendedField {
            index: 11,
            name: "presentation",
            introduced: GAME_SCORE_FORMAT,
        }],
    );
    history[TableId::CountryGameScoreInputs.index()] = table(
        TableId::CountryGameScoreInputs,
        GAME_SCORE_FORMAT,
        "49044c1c39",
    );
    history[TableId::CountryGameScoreInputs.index()].introduced = GAME_SCORE_FORMAT;
    history[TableId::AirWingGameScoreStats.index()] = table(
        TableId::AirWingGameScoreStats,
        GAME_SCORE_FORMAT,
        "49044c1c39",
    );
    history[TableId::AirWingGameScoreStats.index()].introduced = GAME_SCORE_FORMAT;
    history[TableId::GameScoreMultipliers.index()] = table(
        TableId::GameScoreMultipliers,
        GAME_SCORE_FORMAT,
        "49044c1c39",
    );
    history[TableId::GameScoreMultipliers.index()].introduced = GAME_SCORE_FORMAT;
    history[TableId::Countries.index()] = table(TableId::Countries, 93, "9fb09a565f");
    history[TableId::LandCombatDetails.index()] =
        table(TableId::LandCombatDetails, 93, "9fb09a565f");
    history[TableId::RaidAirScores.index()] = table(
        TableId::RaidAirScores,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy regional active-mission counters and aircraft spotting scale",
    );
    history[TableId::RadarCoverage.index()] = table(
        TableId::RadarCoverage,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy certified radar symbols",
    );
    history[TableId::SimulationFeatures.index()] = appended_at(
        TableId::SimulationFeatures,
        SCRIPTED_MAP_FORMAT,
        "pending political runtime inputs, pol/finish-eta movement defines, pol/mapmode-scripted state array extent",
        &[
            AppendedField {
                index: 2,
                name: "weather_suspend",
                introduced: 93,
            },
            AppendedField {
                index: 3,
                name: "instant_decisions",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "runtime_scalars",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "movement_defines",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "state_array_len",
                introduced: SCRIPTED_MAP_FORMAT,
            },
        ],
    );
    history[TableId::Provinces.index()] = appended_at(
        TableId::Provinces,
        ATTACK_INPUTS_FORMAT,
        "pending political runtime inputs, perf/attack-preview inputs, restore/move areas",
        &[
            AppendedField {
                index: 7,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "selected_areas",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "fort_bunker",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "controller_area",
                introduced: AREA_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "owner_area",
                introduced: AREA_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "area_node",
                introduced: AREA_FORMAT,
            },
        ],
    );
    history[TableId::CountryWars.index()] = appended_at(
        TableId::CountryWars,
        POLITICAL_FORMAT,
        "pending selected route inputs",
        &[AppendedField {
            index: 3,
            name: "ordered",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::WeatherRegions.index()] = appended_at(
        TableId::WeatherRegions,
        CAPTURE_ADDITIONS_FORMAT,
        "pending political runtime inputs, perf/attack-preview inputs, pol/finish-eta combat speed entries, air combat efficiency breakdown, capture/weather-sea-rail-bound",
        &[
            AppendedField {
                index: 7,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "combat",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "air_detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "map_modifiers_known",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "naval_detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "air_attack",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "air_defence",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "air_efficiency",
                introduced: AIR_COMBAT_MENU_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "custom_sea",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
        ],
    );
    history[TableId::WeatherProvinces.index()] = table(
        TableId::WeatherProvinces,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
    );
    history[TableId::StateFlags.index()] = table(TableId::StateFlags, 93, "9fb09a565f");
    history[TableId::RaidTooltipOperands.index()] =
        table(TableId::RaidTooltipOperands, 94, "ccf740b81d");
    history[TableId::DivisionMovement.index()] = table(
        TableId::DivisionMovement,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
    );
    history[TableId::CountryAppearance.index()] = table(
        TableId::CountryAppearance,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::IntelNetworks.index()] = table(
        TableId::IntelNetworks,
        INTEL_NETWORK_MAP_FORMAT,
        "pol/mapmode-operatives network map inputs",
    );
    history[TableId::States.index()] = appended_at(
        TableId::States,
        CAPTURE_ADDITIONS_FORMAT,
        "pending political runtime inputs, pol/mapmode-capture population, pol/mapmode-infrastructure category, pol/mapmode-scripted cached choices, nonpolitical map mode operands, capture additions",
        &[
            AppendedField {
                index: 6,
                name: "continent",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "railway_infrastructure",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "population",
                introduced: MAP_MODE_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "category",
                introduced: STATE_CATEGORY_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "manpower_locked",
                introduced: 104,
            },
            AppendedField {
                index: 11,
                name: "best_vp_index",
                introduced: SCRIPTED_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "single_controller",
                introduced: SCRIPTED_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "infrastructure",
                introduced: MAP_MODE_PARITY_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "marked_owners",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
        ],
    );
    history[TableId::Capitulation.index()] = appended_at(
        TableId::Capitulation,
        MAP_MODE_INPUTS_FORMAT,
        "pol/mapmode-capture owned war goals",
        &[AppendedField {
            index: 6,
            name: "war_goal_recipients",
            introduced: MAP_MODE_INPUTS_FORMAT,
        }],
    );
    history[TableId::StateCores.index()] = appended_at(
        TableId::StateCores,
        STRATEGIC_LOCATIONS_FORMAT,
        "pol/strategic-locations state and province strategic locations",
        &[
            AppendedField {
                index: 1,
                name: "claims",
                introduced: MAP_MODE_INPUTS_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "strategic_locations",
                introduced: STRATEGIC_LOCATIONS_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "province_strategic_locations",
                introduced: STRATEGIC_LOCATIONS_FORMAT,
            },
        ],
    );
    history[TableId::RailCooldowns.index()] = appended_at(
        TableId::RailCooldowns,
        CAPTURE_ADDITIONS_FORMAT,
        "pending political runtime inputs, capture/weather-sea-rail-bound",
        &[
            AppendedField {
                index: 1,
                name: "building",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "selection",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "selection_hour",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "building_bound",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
        ],
    );
    history[TableId::ArmyOrderPlans.index()] = appended_at(
        TableId::ArmyOrderPlans,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 20,
                name: "child_fronts",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "invasion_transports",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "execution",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::DivisionTemplates.index()] =
        table(TableId::DivisionTemplates, 97, "d313f6a6e3");
    history[TableId::Decisions.index()] = table(TableId::Decisions, 97, "d313f6a6e3");
    history[TableId::Countries.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::Countries.index()].appended = &[AppendedField {
        index: 9,
        name: "border_land_doctrine_level",
        introduced: 93,
    }];
    history[TableId::Divisions.index()] = table(
        TableId::Divisions,
        PRIVATE_COPY_ALIASES_FORMAT,
        "container-private copy aliases",
    );
    history[TableId::ProductionLines.index()] = table(
        TableId::ProductionLines,
        PRIVATE_COPY_ALIASES_FORMAT,
        "container-private copy aliases",
    );
    history[TableId::Politics.index()] = appended_at(
        TableId::Politics,
        104,
        "restore/economy has_elections",
        &[
            AppendedField {
                index: 6,
                name: "has_elections",
                introduced: HAS_ELECTIONS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "election_interval",
                introduced: 104,
            },
            AppendedField {
                index: 8,
                name: "next_election_cache",
                introduced: 104,
            },
        ],
    );
    history[TableId::LandCombatDivisions.index()] = appended_at(
        TableId::LandCombatDivisions,
        EXACT_COMBAT_FORMAT,
        "exact combat inputs",
        &[
            AppendedField {
                index: 30,
                name: "reinforcement_speed",
                introduced: RESERVE_SPEED_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "fuel_inputs[0]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 32,
                name: "fuel_inputs[1]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 33,
                name: "stat_values[0]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 34,
                name: "stat_values[1]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 35,
                name: "stat_values[2]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 36,
                name: "stat_values[3]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 37,
                name: "stat_values[4]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 38,
                name: "scales[0]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 39,
                name: "scales[1]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 40,
                name: "scales[2]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
            AppendedField {
                index: 41,
                name: "scales[3]",
                introduced: LAND_DIVISION_NUMBERS_FORMAT,
            },
        ],
    );
    history[TableId::LandCombatDivisions.index()].kind = ChangeKind::Layout;
    history[TableId::LandCombatDetails.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::LandCombatDetails.index()].appended = &[
        AppendedField {
            index: 9,
            name: "border_war",
            introduced: 93,
        },
        AppendedField {
            index: 10,
            name: "side_inputs",
            introduced: SIDE_INPUTS_FORMAT,
        },
    ];
    history[TableId::WeatherProvinces.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::WeatherProvinces.index()].appended = &[
        AppendedField {
            index: 4,
            name: "temperature",
            introduced: 93,
        },
        AppendedField {
            index: 5,
            name: "raw_mud",
            introduced: 93,
        },
        AppendedField {
            index: 6,
            name: "local_hour",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 7,
            name: "night_factor",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 8,
            name: "display",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 9,
            name: "clock_source",
            introduced: DERIVED_CLOCKS_FORMAT,
        },
        AppendedField {
            index: 10,
            name: "temperature_drift",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 11,
            name: "temperature_staged",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 12,
            name: "temperature_working",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 13,
            name: "temperature_pass",
            introduced: DERIVED_TEMPERATURE_FORMAT,
        },
        AppendedField {
            index: 14,
            name: "modifiers",
            introduced: MOVEMENT_TERMS_FORMAT,
        },
        AppendedField {
            index: 15,
            name: "daylight",
            introduced: AIR_MAP_FORMAT,
        },
        AppendedField {
            index: 16,
            name: "static_modifiers",
            introduced: CAPTURE_ADDITIONS_FORMAT,
        },
    ];
    history[TableId::WeatherProvinces.index()].format = CAPTURE_ADDITIONS_FORMAT;
    history[TableId::WeatherProvinces.index()].commit = "t3/province-modifiers-building-stats";
    history[TableId::StateFlags.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::StateFlags.index()].appended = &[
        AppendedField {
            index: 1,
            name: "border_conflict_active",
            introduced: 93,
        },
        AppendedField {
            index: 2,
            name: "border_conflict_countries",
            introduced: 93,
        },
    ];
    history[TableId::DivisionMovement.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionMovement.index()].appended = &[
        AppendedField {
            index: 14,
            name: "strategic_action",
            introduced: 96,
        },
        AppendedField {
            index: 15,
            name: "withdrawal",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 16,
            name: "movement_priority",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 17,
            name: "order_transport",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 18,
            name: "fresh",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 19,
            name: "combat_references",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 20,
            name: "selected",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 21,
            name: "transport",
            introduced: POLITICAL_FORMAT,
        },
        AppendedField {
            index: 22,
            name: "base_speed",
            introduced: BASE_SPEED_FORMAT,
        },
        AppendedField {
            index: 23,
            name: "fuel",
            introduced: MOVEMENT_TERMS_FORMAT,
        },
        AppendedField {
            index: 24,
            name: "transport_speed",
            introduced: EXPERIENCE_FORMAT,
        },
    ];
    history[TableId::RelationLinks.index()] = appended_at(
        TableId::RelationLinks,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[AppendedField {
            index: 1,
            name: "transport_subject",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::CountryAppearance.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::CountryAppearance.index()].format = POLITICAL_FORMAT;
    history[TableId::CountryAppearance.index()].commit = "pending political runtime inputs";
    history[TableId::CountryAppearance.index()].appended = &[
        AppendedField {
            index: 18,
            name: "subjects",
            introduced: 97,
        },
        AppendedField {
            index: 19,
            name: "original_tag",
            introduced: EFFECT_INPUTS_FORMAT,
        },
        AppendedField {
            index: 20,
            name: "original_tag_reference",
            introduced: EFFECT_INPUTS_FORMAT,
        },
        AppendedField {
            index: 21,
            name: "transport_convoys",
            introduced: POLITICAL_FORMAT,
        },
    ];
    history[TableId::DivisionTemplates.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::DivisionTemplates.index()].appended = &[AppendedField {
        index: 3,
        name: "name_key",
        introduced: 97,
    }];
    history[TableId::Decisions.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::Decisions.index()].appended = &[AppendedField {
        index: 7,
        name: "description_seed",
        introduced: 97,
    }];
    history[TableId::RaidTooltipOperands.index()].kind = ChangeKind::Meaning;
    history[TableId::AvailableAdvisors.index()] = table(
        TableId::AvailableAdvisors,
        RECORDER_INPUTS_FORMAT,
        "d53fa4e05c",
    );
    history[TableId::AvailableAdvisors.index()].introduced = RECORDER_INPUTS_FORMAT;
    history[TableId::OwnedWargoals.index()] =
        table(TableId::OwnedWargoals, RECORDER_INPUTS_FORMAT, "9d09404c0d");
    history[TableId::OwnedWargoals.index()].introduced = RECORDER_INPUTS_FORMAT;
    history[TableId::CountryExperienceGains.index()] = table(
        TableId::CountryExperienceGains,
        WIDGET_CAPTURE_FORMAT,
        "ecb7ef4c1a",
    );
    history[TableId::CountryExperienceGains.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::EventFeedHistory.index()] = table(
        TableId::EventFeedHistory,
        EVENT_FEED_OCCURRENCE_DELTAS_FORMAT,
        "event-feed-occurrence-deltas",
    );
    history[TableId::EventFeedHistory.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::AdvisorRoles.index()] = appended_at(
        TableId::AdvisorRoles,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[
            AppendedField {
                index: 4,
                name: "modifier_cache",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "tooltip_keys",
                introduced: EFFECT_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::UnitLeaders.index()] = appended_at(
        TableId::UnitLeaders,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[
            AppendedField {
                index: 20,
                name: "exile_country",
                introduced: 93,
            },
            AppendedField {
                index: 21,
                name: "codename",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "formation_reference",
                introduced: EFFECT_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::CharacterOperatives.index()] = appended(
        TableId::CharacterOperatives,
        "d53fa4e05c",
        &[
            AppendedField {
                index: 6,
                name: "codename",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "name",
                introduced: RECORDER_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::OperativeMemberships.index()] = appended_at(
        TableId::OperativeMemberships,
        INTEL_INPUTS_FORMAT,
        "intel-native-union",
        &[
            AppendedField {
                index: 8,
                name: "ordinal",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "target_country",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "operation",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "name",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "cooldown_kind",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "cooldown_start_hour",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "cooldown_end_hour",
                introduced: INTEL_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::Operations.index()] = table(
        TableId::Operations,
        OPERATION_PREPARATION_FORMAT,
        "pol/mapmode-operative-icons",
    );
    history[TableId::CountryTemplateState.index()] = table(
        TableId::CountryTemplateState,
        TEMPLATE_SETTINGS_FORMAT,
        "country template state",
    );
    history[TableId::CountryTemplateState.index()].introduced = TEMPLATE_SETTINGS_FORMAT;
    let intel = [TableId::AgencyUpgradeLevels, TableId::OperationCompletions];
    let mut i = 0;
    while i < intel.len() {
        let id = intel[i];
        history[id.index()] = table(id, INTEL_INPUTS_FORMAT, "intel-native-union");
        history[id.index()].introduced = INTEL_INPUTS_FORMAT;
        i += 1;
    }
    history[TableId::DivisionStatus.index()] = appended_at(
        TableId::DivisionStatus,
        MOVEMENT_OPERANDS_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 16,
                name: "border_combat_stats",
                introduced: 93,
            },
            AppendedField {
                index: 17,
                name: "naming",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "supply_grace_maximum",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "supply_grace_remaining",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "country_ordinal",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "invalid_orders_reason",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "supply",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 23,
                name: "attrition_inputs",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "dig_in_factor",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "name",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "camouflage",
                introduced: CAMOUFLAGE_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "attack_stats",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 29,
                name: "weather_factor",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 30,
                name: "name_template",
                introduced: NAME_TEMPLATE_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "movement_stats",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 32,
                name: "out_of_supply_days",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 33,
                name: "commander_modifiers",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 34,
                name: "supply_ratio_exact",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 35,
                name: "supply_grace_exact",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 36,
                name: "experience",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 37,
                name: "pending_template",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 38,
                name: "officer_experience",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 39,
                name: "career_counter",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 40,
                name: "panel_source",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 41,
                name: "combat_movement",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
            AppendedField {
                index: 42,
                name: "organisation_exact",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
            AppendedField {
                index: 43,
                name: "maximum_organisation_exact",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
            AppendedField {
                index: 44,
                name: "commander_width_factor",
                introduced: EXPERIENCE_FORMAT,
            },
        ],
    );
    history[TableId::OrdersGroupCounterInputs.index()] = appended_at(
        TableId::OrdersGroupCounterInputs,
        TEMPLATE_SETTINGS_FORMAT,
        "pending political runtime inputs, perf/attack-preview inputs",
        &[
            AppendedField {
                index: 1,
                name: "hq_distance",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "leader_enroute_days",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "border_war_presence",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "invasion_source_distance",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "commander_attack",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "commander_movement",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "hq_nearest_front",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "template_context",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
        ],
    );
    history[TableId::DivisionTemplateIcons.index()] = appended_at(
        TableId::DivisionTemplateIcons,
        104,
        "pending political runtime inputs, pol/finish-audio template grids, pol/capture-manpower exile predicate",
        &[
            AppendedField {
                index: 13,
                name: "formation_division_count",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "display_name",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "grids",
                introduced: TEMPLATE_GRIDS_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "manpower_exile",
                introduced: 104,
            },
            AppendedField {
                index: 17,
                name: "base_width",
                introduced: EXPERIENCE_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "layout",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "settings",
                introduced: TEMPLATE_SETTINGS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "speed_keys",
                introduced: MOVEMENT_OPERANDS_FORMAT,
            },
        ],
    );
    history[TableId::FleetCounterInputs.index()] = appended_at(
        TableId::FleetCounterInputs,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy fleet region membership",
        &[AppendedField {
            index: 6,
            name: "mission_regions",
            introduced: AIR_MAP_FORMAT,
        }],
    );
    history[TableId::CountryCounterInputs.index()] = appended_at(
        TableId::CountryCounterInputs,
        AIR_MAP_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 4,
                name: "owned_states",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "controlled_states",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "core_states",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "army_leaders",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "navy_leaders",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "mio_policies",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "neighbors",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "co_belligerents",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "zero_war",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "movement",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "zero_subject",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "pride_name",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "forward_visibility",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "directed_wars",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "attack_modifiers",
                introduced: ATTACK_INPUTS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "movement_modifiers",
                introduced: MOVEMENT_TERMS_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "recruitment_modifiers",
                introduced: 104,
            },
            AppendedField {
                index: 22,
                name: "sent_expeditionaries",
                introduced: 104,
            },
            AppendedField {
                index: 23,
                name: "air_modifiers",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "mines_effect_reduction",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "naval_detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "air_superiority_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "air_intercept_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "air_cas_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 29,
                name: "air_nav_efficiency",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 30,
                name: "air_attack_factor",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "air_defence_factor",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 32,
                name: "air_strategic_bomber_defence_factor",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 33,
                name: "air_combat_modifiers_known",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 34,
                name: "combat_width_factor",
                introduced: EXPERIENCE_FORMAT,
            },
        ],
    );
    history[TableId::OccupationSettings.index()] = appended_at(
        TableId::OccupationSettings,
        OCCUPATION_GROUP_LAWS_FORMAT,
        "occupation-group-law-union",
        &[
            AppendedField {
                index: 3,
                name: "occupied",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "group_laws",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "fallback_law",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
        ],
    );
    history[TableId::ScientistDaily.index()] = table(
        TableId::ScientistDaily,
        RESEARCH_PROJECT_DETAILS_FORMAT,
        "daily scientist observations",
    );
    history[TableId::ScientistDaily.index()].introduced = RESEARCH_PROJECT_DETAILS_FORMAT;
    history[TableId::Scientists.index()] = appended_at(
        TableId::Scientists,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[AppendedField {
            index: 4,
            name: "active",
            introduced: EFFECT_INPUTS_FORMAT,
        }],
    );
    history[TableId::CountryLeaderRoles.index()] = appended_at(
        TableId::CountryLeaderRoles,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
        &[AppendedField {
            index: 1,
            name: "current_ideology",
            introduced: EFFECT_INPUTS_FORMAT,
        }],
    );
    history[TableId::AirWings.index()] = appended_at(
        TableId::AirWings,
        AIR_MAP_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 15,
                name: "name",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "pool_subunit",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "raid_reference",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "strategic_bombing_raw",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "acquired_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 20,
                name: "exile_country",
                introduced: 104,
            },
            AppendedField {
                index: 21,
                name: "replacement_setting",
                introduced: DESIGN_STATE_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "map_flags",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 23,
                name: "fuel_base_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "fuel_current_type",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "fuel_effective_aircraft",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "fuel_context_mode",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "fuel_capacity_class",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "fuel_override_resolved",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::AceRangeInputs.index()] = appended_at(
        TableId::AceRangeInputs,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy normalization projection",
        &[
            AppendedField {
                index: 4,
                name: "normalization",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "fuel_modifier",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "combat_modifiers_known",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "speed_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "attack_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "defence_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "type_speed_modifier_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "type_combat_flags",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::VolunteerHosts.index()] = appended_at(
        TableId::VolunteerHosts,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy volunteer relation modifier",
        &[AppendedField {
            index: 2,
            name: "air_capacity_modifier",
            introduced: AIR_MAP_FORMAT,
        }],
    );
    history[TableId::RaidCountryInputs.index()] = appended_at(
        TableId::RaidCountryInputs,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy volunteer stockpile total",
        &[AppendedField {
            index: 4,
            name: "volunteer_stockpile_planes",
            introduced: AIR_MAP_FORMAT,
        }],
    );
    history[TableId::AirSortieModifiers.index()] = table(
        TableId::AirSortieModifiers,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy certified detection overrides",
    );
    history[TableId::AirMissionInputs.index()] = appended_at(
        TableId::AirMissionInputs,
        AIR_MAP_FORMAT,
        "pending political runtime inputs and pol/mapmode-air-navy mission operands",
        &[
            AppendedField {
                index: 6,
                name: "cas_ratio_raw",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "interception_alert_gate",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "current_type",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "effective_aircraft",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "normalization",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "detection_stat_13",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "detection_stat_14",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "patrol_disruption_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "patrol_attack_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "patrol_defence_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "patrol_speed_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "patrol_wing_attack_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "patrol_wing_defence_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "patrol_night_applied_raw",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::AirMissionInputs.index()].introduced = AIR_MAP_FORMAT;
    history[TableId::AirNavalPatrolHistories.index()] = appended_at(
        TableId::AirNavalPatrolHistories,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[AppendedField {
            index: 2,
            name: "scenario_channels",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::ShipModels.index()] = appended_at(
        TableId::ShipModels,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy repair eligibility",
        &[
            AppendedField {
                index: 6,
                name: "subunit_index",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "strike",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "hull_variant",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "has_criticals",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "air_capacity_kind",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "naval_power",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::TaskForceCounterInputs.index()] = appended_at(
        TableId::TaskForceCounterInputs,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 10,
                name: "name",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "supply",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "port_mission",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::RailwayGunCounterInputs.index()] = appended_at(
        TableId::RailwayGunCounterInputs,
        CAPTURE_ADDITIONS_FORMAT,
        "pending political runtime inputs, capture additions",
        &[
            AppendedField {
                index: 23,
                name: "name",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 24,
                name: "eta",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "supply",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "withdrawal",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 27,
                name: "fresh",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 28,
                name: "combat_references",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 29,
                name: "transport",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 30,
                name: "theatre",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
            AppendedField {
                index: 31,
                name: "queued_actions",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
        ],
    );
    history[TableId::TaskForceMapMotion.index()] = appended_at(
        TableId::TaskForceMapMotion,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 4,
                name: "movement_numerator",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "retreat",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "withdrawal",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "naval_transfer",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "combat_links",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "fresh",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::TimedIdeas.index()] =
        table(TableId::TimedIdeas, EFFECT_INPUTS_FORMAT, "ffd3e03cc6");
    history[TableId::TimedIdeas.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::DecisionRandomItems.index()] = table(
        TableId::DecisionRandomItems,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::DecisionRandomItems.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::CapturedOperativeReferences.index()] = table(
        TableId::CapturedOperativeReferences,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::CapturedOperativeReferences.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::ReservedCountryNaming.index()] = appended_at(
        TableId::ReservedCountryNaming,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 8,
                name: "original_tag",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "original_tag_reference",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "original_tag_buckets",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "country_array",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "owned_state_count",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "dynamic_tag_boundary",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "faction",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "movement",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "land_theatre_count",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "launch_sources",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "reverse_visibility",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "country_intel_sample",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "attack",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "raw_country_observations",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "transport_mission_inputs",
                introduced: POLITICAL_FORMAT,
            },
        ],
    );
    history[TableId::ReservedCountryNaming.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::AttritionModifierBreakdowns.index()] = table(
        TableId::AttritionModifierBreakdowns,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::AttritionModifierBreakdowns.index()].introduced = EFFECT_INPUTS_FORMAT;
    history[TableId::AirBases.index()] = appended_at(
        TableId::AirBases,
        POLITICAL_FORMAT,
        "pending political runtime inputs",
        &[AppendedField {
            index: 5,
            name: "carrier",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::AirBaseRegistrations.index()] = appended_at(
        TableId::AirBaseRegistrations,
        AIR_MAP_FORMAT,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 2,
                name: "carrier",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "transport_planes",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "fuel_demand_raw",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "fuel_allocated_raw",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::CountryProductionModifiers.index()] = table(
        TableId::CountryProductionModifiers,
        PRODUCTION_MODIFIERS_FORMAT,
        "production-modifiers",
    );
    history[TableId::CountryProductionModifiers.index()].introduced = PRODUCTION_MODIFIERS_FORMAT;
    history[TableId::ProductionLineModifiers.index()] = appended_at(
        TableId::ProductionLineModifiers,
        CONVERSION_PURPOSE_FORMAT,
        "conversion-purpose",
        &[AppendedField {
            index: 15,
            name: "converting",
            introduced: CONVERSION_PURPOSE_FORMAT,
        }],
    );
    history[TableId::ProductionLineModifiers.index()].introduced = PRODUCTION_MODIFIERS_FORMAT;
    history[TableId::CountryUnitBonuses.index()] = LastChange {
        introduced: UNIT_BONUSES_FORMAT,
        ..table(
            TableId::CountryUnitBonuses,
            UNIT_BONUSES_FORMAT,
            "feat/designer-stats",
        )
    };
    history[TableId::DivisionModelInputs.index()] = LastChange {
        introduced: DIVISION_MODEL_INPUTS_FORMAT,
        ..table(
            TableId::DivisionModelInputs,
            DIVISION_MODEL_INPUTS_FORMAT,
            "t3/map-model-witness",
        )
    };
    history[TableId::LandUnitOrder.index()] = LastChange {
        introduced: CAPTURE_ADDITIONS_FORMAT,
        ..table(
            TableId::LandUnitOrder,
            CAPTURE_ADDITIONS_FORMAT,
            "t3/measure-theatre-area-unit-order-cost",
        )
    };
    history[TableId::StateBuildingInputs.index()] = LastChange {
        introduced: CAPTURE_ADDITIONS_FORMAT,
        ..table(
            TableId::StateBuildingInputs,
            CAPTURE_ADDITIONS_FORMAT,
            "t3/province-modifiers-building-stats",
        )
    };
    let air_ledgers = [
        TableId::AirLossEvents,
        TableId::BuildingBombingDamage,
        TableId::AirDailyRoster,
    ];
    let mut ledger = 0;
    while ledger < air_ledgers.len() {
        let id = air_ledgers[ledger];
        history[id.index()] = LastChange {
            introduced: AIR_LEDGER_FORMAT,
            ..table(id, AIR_LEDGER_FORMAT, "7fa2b75e16")
        };
        ledger += 1;
    }
    history[TableId::AirRegionCombat.index()] = LastChange {
        introduced: AIR_REGION_COMBAT_FORMAT,
        ..table(
            TableId::AirRegionCombat,
            AIR_REGION_COMBAT_FORMAT,
            "restore/experience",
        )
    };
    history[TableId::NuclearStrikes.index()] = LastChange {
        introduced: NUCLEAR_STRIKES_FORMAT,
        ..table(
            TableId::NuclearStrikes,
            NUCLEAR_STRIKES_FORMAT,
            "record/nuclear-strikes",
        )
    };
    history[TableId::LandCombatDivisionModifiers.index()] = LastChange {
        introduced: LAND_MODIFIER_ROWS_FORMAT,
        ..table(
            TableId::LandCombatDivisionModifiers,
            EXACT_COMBAT_FORMAT,
            "exact combat inputs",
        )
    };
    history[TableId::FuelDemandParts.index()] = LastChange {
        introduced: FUEL_DEMAND_PARTS_FORMAT,
        ..table(
            TableId::FuelDemandParts,
            FUEL_DEMAND_PARTS_FORMAT,
            "perf/fuel-air-rows",
        )
    };
    history[TableId::MapEvents.index()] = LastChange {
        introduced: MAP_EVENTS_FORMAT,
        ..table(
            TableId::MapEvents,
            SCRIPTED_MAP_FORMAT,
            "pol/mapmode-scripted refresh family and interval health",
        )
    };
    history[TableId::OwnerTransportSpeeds.index()] = LastChange {
        introduced: OWNER_TRANSPORT_SPEEDS_FORMAT,
        ..table(
            TableId::OwnerTransportSpeeds,
            OWNER_TRANSPORT_SPEEDS_FORMAT,
            "pol/finish-smallcap",
        )
    };
    history[TableId::ReserveModifierTrees.index()] = LastChange {
        introduced: RESERVE_MODIFIER_TREES_FORMAT,
        ..table(
            TableId::ReserveModifierTrees,
            RESERVE_MODIFIER_TREES_FORMAT,
            "pol/finish-smallcap",
        )
    };
    history[TableId::BattleSounds.index()] = LastChange {
        introduced: BATTLE_SOUNDS_FORMAT,
        ..table(
            TableId::BattleSounds,
            BATTLE_SOUNDS_FORMAT,
            "pol/finish-audio",
        )
    };
    history[TableId::OperativeMissions.index()] = LastChange {
        introduced: OPERATIVE_MISSIONS_FORMAT,
        ..table(
            TableId::OperativeMissions,
            OPERATIVE_MISSIONS_FORMAT,
            "pol/mapmode-operative-icons",
        )
    };
    history[TableId::AgencyOperationBuffers.index()] = LastChange {
        introduced: AGENCY_OPERATION_BUFFERS_FORMAT,
        ..table(
            TableId::AgencyOperationBuffers,
            AGENCY_OPERATION_BUFFERS_FORMAT,
            "pol/mapmode-operative-icons",
        )
    };
    history[TableId::StrategicOperatives.index()] = LastChange {
        introduced: STRATEGIC_OPERATIVES_FORMAT,
        ..table(
            TableId::StrategicOperatives,
            STRATEGIC_OPERATIVES_FORMAT,
            "pol/mapmode-operative-icons",
        )
    };
    history[TableId::CombatLogGroups.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogGroups,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogMonths.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogMonths,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogManpower.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogManpower,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogTemplates.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogTemplates,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogReferences.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogReferences,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattles.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattles,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogEnemyEquipment.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogEnemyEquipment,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleSides.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleSides,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleEquipment.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleEquipment,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleModifiers.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleModifiers,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogBattleCountries.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogBattleCountries,
            COMBAT_LOG_FORMAT,
            "pol/capture-combat-log",
        )
    };
    history[TableId::CombatLogObservers.index()] = LastChange {
        introduced: COMBAT_LOG_FORMAT,
        ..table(
            TableId::CombatLogObservers,
            COMBAT_LOG_OBSERVER_KEYS_FORMAT,
            "combat-log-observer-keys",
        )
    };
    history[TableId::ResourceOrigins.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..appended_at(
            TableId::ResourceOrigins,
            CAPTURE_ADDITIONS_FORMAT,
            "capture/convoy-route-efficiency-cost",
            &[AppendedField {
                index: 8,
                name: "convoys_base",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            }],
        )
    };
    history[TableId::ResourceRights.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::ResourceRights,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::CountryOilFactors.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::CountryOilFactors,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::StateTemporaryResources.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::StateTemporaryResources,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::ConvoyClients.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..appended_at(
            TableId::ConvoyClients,
            CAPTURE_ADDITIONS_FORMAT,
            "capture/convoy-route-efficiency-cost",
            &[
                AppendedField {
                    index: 11,
                    name: "efficiency_a",
                    introduced: CAPTURE_ADDITIONS_FORMAT,
                },
                AppendedField {
                    index: 12,
                    name: "efficiency_b",
                    introduced: CAPTURE_ADDITIONS_FORMAT,
                },
                AppendedField {
                    index: 13,
                    name: "convoys_base",
                    introduced: CAPTURE_ADDITIONS_FORMAT,
                },
            ],
        )
    };
    history[TableId::StrategicRegionWeights.index()] = LastChange {
        introduced: CAPTURE_ADDITIONS_FORMAT,
        ..table(
            TableId::StrategicRegionWeights,
            CAPTURE_ADDITIONS_FORMAT,
            "capture/convoy-route-efficiency-cost",
        )
    };
    history[TableId::TradeRoutes.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::TradeRoutes,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::DivisionPanelStats.index()] = LastChange {
        introduced: EXPERIENCE_FORMAT,
        ..appended_at(
            TableId::DivisionPanelStats,
            PANEL_SUPPRESSION_FORMAT,
            "feat/designer-rows",
            &[AppendedField {
                index: 22,
                name: "suppression",
                introduced: PANEL_SUPPRESSION_FORMAT,
            }],
        )
    };
    history[TableId::DivisionPanelSources.index()] = LastChange {
        introduced: EXPERIENCE_FORMAT,
        ..table(
            TableId::DivisionPanelSources,
            EXPERIENCE_FORMAT,
            "pol/capture-experience",
        )
    };
    history[TableId::DivisionHistory.index()] = LastChange {
        introduced: EXPERIENCE_FORMAT,
        ..table(
            TableId::DivisionHistory,
            EXPERIENCE_FORMAT,
            "pol/capture-experience",
        )
    };
    history[TableId::RegionalConvoyEfficiency.index()] = LastChange {
        introduced: RESOURCES_MAP_FORMAT,
        ..table(
            TableId::RegionalConvoyEfficiency,
            RESOURCES_MAP_FORMAT,
            "pol/mapmode-resources-raids",
        )
    };
    history[TableId::MovementStateTerms.index()] = LastChange {
        introduced: MOVEMENT_STATE_TERMS_FORMAT,
        ..table(
            TableId::MovementStateTerms,
            MOVEMENT_STATE_TERMS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::CoreSupplyAccess.index()] = LastChange {
        introduced: CORE_SUPPLY_ACCESS_FORMAT,
        ..table(
            TableId::CoreSupplyAccess,
            CORE_SUPPLY_ACCESS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::CountryUnitSpeedEntries.index()] = LastChange {
        introduced: COUNTRY_UNIT_SPEED_ENTRIES_FORMAT,
        ..table(
            TableId::CountryUnitSpeedEntries,
            COUNTRY_UNIT_SPEED_ENTRIES_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::RiverRelations.index()] = LastChange {
        introduced: RIVER_RELATIONS_FORMAT,
        ..table(
            TableId::RiverRelations,
            RIVER_RELATIONS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::DivisionTransportSpeeds.index()] = LastChange {
        introduced: DIVISION_TRANSPORT_SPEEDS_FORMAT,
        ..table(
            TableId::DivisionTransportSpeeds,
            DIVISION_TRANSPORT_SPEEDS_FORMAT,
            "pol/eta-captures",
        )
    };
    history[TableId::CountryMarket.index()] = LastChange {
        introduced: 104,
        ..table(TableId::CountryMarket, 104, "pol/capture-market-lendlease")
    };
    declare_change(
        &mut history[TableId::CountryMarket.index()],
        LastChange {
            introduced: 104,
            ..appended_at(
                TableId::CountryMarket,
                MARKET_AUTOMATION_FORMAT,
                "port-1194/market-automation",
                &[],
            )
        },
    );
    history[TableId::MarketContractTerms.index()] = LastChange {
        introduced: 104,
        ..table(
            TableId::MarketContractTerms,
            104,
            "pol/capture-market-lendlease",
        )
    };
    history[TableId::LendLeaseAgreements.index()] = LastChange {
        introduced: 104,
        ..table(
            TableId::LendLeaseAgreements,
            104,
            "pol/capture-market-lendlease",
        )
    };
    history[TableId::MarketEvents.index()] = LastChange {
        introduced: 104,
        ..table(TableId::MarketEvents, 104, "pol/capture-market-lendlease")
    };
    let naval = [
        TableId::NavalRepairCountries,
        TableId::NavalRepairFleets,
        TableId::NavalRepairTaskForces,
        TableId::NavalTaskForceComposition,
        TableId::NavalRepairBases,
        TableId::NavalRepairQueue,
        TableId::NavalRefitShips,
        TableId::NavalRefitProgress,
        TableId::NavalRepairChildren,
    ];
    let mut index = 0;
    while index < naval.len() {
        let id = naval[index];
        history[id.index()] = LastChange {
            introduced: 104,
            ..table(id, 104, "pol/capture-naval-repair")
        };
        index += 1;
    }
    history[TableId::NavalMissionTaskForces.index()] = table(
        TableId::NavalMissionTaskForces,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy shared mission source membership",
    );
    history[TableId::NavalMissionCommanders.index()] = appended_at(
        TableId::NavalMissionCommanders,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy mission modifier sources",
        &[
            AppendedField {
                index: 4,
                name: "detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "night_spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "visibility",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 8,
                name: "speed",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::NavalMissionCountries.index()] = appended_at(
        TableId::NavalMissionCountries,
        AIR_MAP_FORMAT,
        "pol/mapmode-air-navy mission sources",
        &[
            AppendedField {
                index: 8,
                name: "detection",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 9,
                name: "spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 10,
                name: "night_spotting",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "visibility",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "escort_tree",
                introduced: AIR_MAP_FORMAT,
            },
            AppendedField {
                index: 13,
                name: "speed",
                introduced: AIR_MAP_FORMAT,
            },
        ],
    );
    history[TableId::AirGroundCrews.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::AirGroundCrews,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy certified Ground Crews",
        )
    };
    history[TableId::AirTransferInputs.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::AirTransferInputs,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy incoming Air transfers",
        )
    };
    history[TableId::NavalAccidentReports.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalAccidentReports,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy retained naval accidents",
        )
    };
    history[TableId::NavalRegionalMines.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalRegionalMines,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy complete regional mines",
        )
    };
    history[TableId::NavalDominanceTaskForceRanges.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalDominanceTaskForceRanges,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy demanded strike-force sibling reach",
        )
    };
    history[TableId::NavalPatrolMissionInputs.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::NavalPatrolMissionInputs,
            AIR_MAP_FORMAT,
            "pol/mapmode-air-navy patrol detection operands",
        )
    };
    history[TableId::VariantDefinitions.index()] = appended_at(
        TableId::VariantDefinitions,
        DESIGN_STATE_FORMAT,
        "pol/capture-designs",
        &[
            AppendedField {
                index: 14,
                name: "version",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "design",
                introduced: DESIGN_STATE_FORMAT,
            },
        ],
    );
    history[TableId::VariantChanges.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::VariantChanges,
            VARIANT_STAT_WITNESS_FORMAT,
            "variant stat lane witnesses",
        )
    };
    history[TableId::VariantBonusSnapshots.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::VariantBonusSnapshots,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::DesignCountryModifiers.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignCountryModifiers,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::DesignCountryTemplates.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignCountryTemplates,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };

    history[TableId::DesignOrganisationLiveness.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignOrganisationLiveness,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::DesignObserverContext.index()] = LastChange {
        introduced: DESIGN_STATE_FORMAT,
        ..table(
            TableId::DesignObserverContext,
            DESIGN_STATE_FORMAT,
            "pol/capture-designs",
        )
    };
    history[TableId::ScriptVariables.index()] = appended(
        TableId::ScriptVariables,
        "c8c5165b33",
        &[AppendedField {
            index: 1,
            name: "seed",
            introduced: RECORDER_INPUTS_FORMAT,
        }],
    );
    history[TableId::TrainInventory.index()] = appended(
        TableId::TrainInventory,
        "c8c5165b33",
        &[
            AppendedField {
                index: 3,
                name: "allocated_trucks",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 4,
                name: "required_trucks_raw",
                introduced: RECORDER_INPUTS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "required_trains",
                introduced: RECORDER_INPUTS_FORMAT,
            },
        ],
    );
    history[TableId::ALL.len() + RecordKind::NavalCommanderInputs as usize].introduced =
        PROVEN_FROM;
    history[TableId::ALL.len() + RecordKind::NavalCommanderInputs as usize].kind =
        ChangeKind::AppendedOnly;
    history[TableId::ALL.len() + RecordKind::NavalCommanderInputs as usize].appended =
        NestedRecord::NavalCommanderInputs.appended();
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].introduced = PROVEN_FROM;
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].kind =
        ChangeKind::AppendedOnly;
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].appended =
        NestedRecord::LandBorderWarInputs.appended();
    history[TableId::ALL.len() + RecordKind::EventFeedEvent as usize].introduced =
        WIDGET_CAPTURE_FORMAT;
    history[TableId::ALL.len() + RecordKind::EventFeedEvent as usize].kind = ChangeKind::Layout;
    history[TableId::CountryFuel.index()] = appended_at(
        TableId::CountryFuel,
        ECONOMY_INPUTS_FORMAT,
        "a72768d69e",
        &[
            AppendedField {
                index: 19,
                name: "country_instance",
                introduced: ECONOMY_INPUTS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "awards",
                introduced: ECONOMY_INPUTS_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "demand",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "award_day",
                introduced: FUEL_AWARD_DAY_FORMAT,
            },
        ],
    );
    history[TableId::LogisticsEquipment.index()] = appended_at(
        TableId::LogisticsEquipment,
        ECONOMY_INPUTS_FORMAT,
        "afa36ffdef",
        &[AppendedField {
            index: 3,
            name: "available_raw",
            introduced: ECONOMY_INPUTS_FORMAT,
        }],
    );
    history[TableId::NavalBases.index()] = appended_at(
        TableId::NavalBases,
        POLITICAL_FORMAT,
        "naval base serialized country tag witnesses (provisional)",
        &[AppendedField {
            index: 5,
            name: "country_tags",
            introduced: POLITICAL_FORMAT,
        }],
    );
    history[TableId::ConstructionLineOutput.index()] = appended_at(
        TableId::ConstructionLineOutput,
        CONSTRUCTION_OUTPUT_OWNER_FORMAT,
        "construction output owner and target",
        &[
            AppendedField {
                index: 13,
                name: "completion",
                introduced: CONSTRUCTION_COMPLETION_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "count_reset",
                introduced: POLITICAL_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "country",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 16,
                name: "building_type",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 17,
                name: "target_flags",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 18,
                name: "target_location",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "target_state",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "target_country",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 21,
                name: "route_first",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
            AppendedField {
                index: 22,
                name: "route_last",
                introduced: CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            },
        ],
    );
    history[TableId::StateGarrisons.index()] = appended_at(
        TableId::StateGarrisons,
        WIDGET_CAPTURE_FORMAT,
        "garrison-strength-operands",
        &[AppendedField {
            index: 5,
            name: "equipment",
            introduced: WIDGET_CAPTURE_FORMAT,
        }],
    );
    history[TableId::EquipmentDestruction.index()] = appended_at(
        TableId::EquipmentDestruction,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[
            AppendedField {
                index: 1,
                name: "coverage",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 2,
                name: "unavailable_reason",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    history[TableId::Fleets.index()] = appended_at(
        TableId::Fleets,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[
            AppendedField {
                index: 2,
                name: "name",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 3,
                name: "leader",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    history[TableId::OrdersGroups.index()] = appended_at(
        TableId::OrdersGroups,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[AppendedField {
            index: 9,
            name: "name",
            introduced: WIDGET_CAPTURE_FORMAT,
        }],
    );
    history[TableId::LandCombatProgress.index()] = appended_at(
        TableId::LandCombatProgress,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
        &[
            AppendedField {
                index: 4,
                name: "attacker_manpower_loss",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "defender_manpower_loss",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "attacker_equipment_losses",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "defender_equipment_losses",
                introduced: WIDGET_CAPTURE_FORMAT,
            },
        ],
    );
    history[TableId::DivisionEquipmentDeletions.index()] = table(
        TableId::DivisionEquipmentDeletions,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::DivisionEquipmentDeletions.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::DivisionLandInputs.index()] = table(
        TableId::DivisionLandInputs,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::DivisionLandInputs.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::DivisionLandInputs.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::LandCombatResults.index()] = table(
        TableId::LandCombatResults,
        COMBAT_LOG_FORMAT,
        "gap-combat-log",
    );
    history[TableId::LandCombatResults.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::DestroyedDivisions.index()] = table(
        TableId::DestroyedDivisions,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::DestroyedDivisions.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::LandEventCoverage.index()] = table(
        TableId::LandEventCoverage,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
    );
    history[TableId::LandEventCoverage.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::CountryCounterInputs.index()].format = EXPERIENCE_FORMAT;
    history[TableId::DivisionMovement.index()].format = EXPERIENCE_FORMAT;
    history[TableId::DivisionStatus.index()].format = EXPERIENCE_FORMAT;
    history[TableId::UnitLeaders.index()].format = EXPERIENCE_FORMAT;
    history[TableId::UnitLeaders.index()].kind = ChangeKind::Meaning;
    history[TableId::UnitLeaderTraitExperience.index()] = table(
        TableId::UnitLeaderTraitExperience,
        EXPERIENCE_FORMAT,
        "capture/experience",
    );
    let daily_progress = [
        TableId::UnitLeaders,
        TableId::UnitLeaderTraitExperience,
        TableId::CombatLogObservers,
        TableId::DivisionStatus,
    ];
    let mut daily = 0;
    while daily < daily_progress.len() {
        let entry = &mut history[daily_progress[daily].index()];
        entry.format = DAILY_PROGRESS_FORMAT;
        entry.kind = ChangeKind::Meaning;
        entry.commit = "capture/daily-progress";
        daily += 1;
    }
    history[TableId::FactionTheatres.index()] = appended_at(
        TableId::FactionTheatres,
        CAPTURE_ADDITIONS_FORMAT,
        "capture/faction-theatre-tooltip",
        &[
            AppendedField {
                index: 4,
                name: "name",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
            AppendedField {
                index: 5,
                name: "owner",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
            AppendedField {
                index: 6,
                name: "commander",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "countries",
                introduced: CAPTURE_ADDITIONS_FORMAT,
            },
        ],
    );
    history[TableId::IndustrialOrganisationPolicies.index()] = LastChange {
        introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        ..appended_at(
            TableId::IndustrialOrganisationPolicies,
            MIO_POLICY_COST_FORMAT,
            "feat/mio-policy-cost",
            &[AppendedField {
                index: 1,
                name: "cost_modifier_raw",
                introduced: MIO_POLICY_COST_FORMAT,
            }],
        )
    };
    history[TableId::IndustrialOrganisationDetails.index()] = LastChange {
        introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        ..table(
            TableId::IndustrialOrganisationDetails,
            INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            "pol/capture-mios",
        )
    };
    history[TableId::IndustrialOrganisationHistory.index()] = appended_at(
        TableId::IndustrialOrganisationHistory,
        INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        "pol/capture-mios",
        &[AppendedField {
            index: 1,
            name: "design_hour",
            introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
        }],
    );
    history[TableId::VariantStats.index()] = appended_at(
        TableId::VariantStats,
        VARIANT_STAT_WITNESS_FORMAT,
        "variant stat lane witnesses",
        &[
            AppendedField {
                index: 10,
                name: "organisation",
                introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            },
            AppendedField {
                index: 11,
                name: "organisation_trait_count",
                introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            },
            AppendedField {
                index: 12,
                name: "organisation_bonuses",
                introduced: INDUSTRIAL_ORGANISATION_DETAILS_FORMAT,
            },
        ],
    );
    history[TableId::VariantStats.index()].kind = ChangeKind::Layout;
    history[TableId::IntelNetworkSummaries.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelNetworkSummaries,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::PropagandaEfforts.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::PropagandaEfforts,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::IntelContributions.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelContributions,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::IntelNetworkAuxiliaries.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelNetworkAuxiliaries,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    history[TableId::IntelSources.index()] = LastChange {
        introduced: INTEL_CACHES_FORMAT,
        ..table(
            TableId::IntelSources,
            INTEL_CACHES_FORMAT,
            "pol/capture-intel",
        )
    };
    let mut participation = 0;
    while participation < 3 {
        let id = [
            TableId::WarParticipation,
            TableId::WarCapturedProvinces,
            TableId::WarLendLeaseHistories,
        ][participation];
        history[id.index()] = table(id, 104, "capture/war-participation");
        history[id.index()].introduced = 104;
        participation += 1;
    }
    apply_catalog_passes(&mut history);
    history[TableId::RaidAirScores.index()] = LastChange {
        introduced: history[TableId::RaidAirScores.index()].introduced,
        layout_since: AIR_MAP_FORMAT,
        ..appended_at(
            TableId::RaidAirScores,
            AIR_COMBAT_MENU_FORMAT,
            "air combat power contributions",
            &[AppendedField {
                index: 6,
                name: "menu",
                introduced: AIR_COMBAT_MENU_FORMAT,
            }],
        )
    };
    history[TableId::AirNavalPatrolHistories.index()] = LastChange {
        introduced: history[TableId::AirNavalPatrolHistories.index()].introduced,
        ..table(
            TableId::AirNavalPatrolHistories,
            AIR_HISTORY_SLOTS_FORMAT,
            "native air history slots",
        )
    };
    history[TableId::ALL.len() + RecordKind::AirCombatRecord as usize] = LastChange {
        subject: Subject::Record(RecordKind::AirCombatRecord),
        introduced: AIR_REGION_COMBAT_FORMAT,
        kind: ChangeKind::AppendedOnly,
        appended: NestedRecord::AirCombatRecord.appended(),
        ..history[TableId::ALL.len() + RecordKind::AirCombatRecord as usize]
    };
    history[TableId::AirMissionInputs.index()] = LastChange {
        introduced: AIR_MAP_FORMAT,
        ..table(
            TableId::AirMissionInputs,
            AIR_COMBAT_MENU_FORMAT,
            "air combat category flags",
        )
    };
    history[TableId::Cryptology.index()] = LastChange {
        introduced: history[TableId::Cryptology.index()].introduced,
        ..appended_at(
            TableId::Cryptology,
            AIR_COMBAT_MENU_FORMAT,
            "air combat cipher order",
            &[AppendedField {
                index: 4,
                name: "ordinal",
                introduced: AIR_COMBAT_MENU_FORMAT,
            }],
        )
    };
    history[TableId::NavalCombatShips.index()] = table(
        TableId::NavalCombatShips,
        EXACT_NAVAL_CONDITION_FORMAT,
        "exact naval condition operands",
    );
    history[TableId::StateResourceInputs.index()] = appended_at(
        TableId::StateResourceInputs,
        104,
        "pol/capture-manpower",
        &[
            AppendedField {
                index: 6,
                name: "local_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 7,
                name: "local_noncore_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 8,
                name: "recruitment_add",
                introduced: 104,
            },
            AppendedField {
                index: 9,
                name: "recruitment_factor",
                introduced: 104,
            },
        ],
    );
    history[TableId::Ships.index()] = appended_at(
        TableId::Ships,
        104,
        "pol/capture-manpower",
        &[
            AppendedField {
                index: 7,
                name: "acquired_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 8,
                name: "required_manpower",
                introduced: 104,
            },
        ],
    );
    declare_change(
        &mut history[TableId::Ships.index()],
        table(
            TableId::Ships,
            107,
            "live ship cache and saved naming operands",
        ),
    );
    history[TableId::CountryStabilityInputs.index()] = appended_at(
        TableId::CountryStabilityInputs,
        104,
        "country manpower and exile operands",
        &[
            AppendedField {
                index: 37,
                name: "recruitment_ratio",
                introduced: 104,
            },
            AppendedField {
                index: 38,
                name: "exile_host",
                introduced: 104,
            },
            AppendedField {
                index: 39,
                name: "exile_legitimacy",
                introduced: 104,
            },
            AppendedField {
                index: 40,
                name: "exile_leaders",
                introduced: 104,
            },
            AppendedField {
                index: 41,
                name: "exile_manpower",
                introduced: 104,
            },
            AppendedField {
                index: 42,
                name: "exile_manpower_cap",
                introduced: 104,
            },
        ],
    );

    const PARITY: u16 = MAP_MODE_PARITY_FORMAT;
    const fn parity(index: usize, name: &'static str) -> AppendedField {
        AppendedField {
            index,
            name,
            introduced: PARITY,
        }
    }
    const PARITY_FIELDS: [(TableId, &[AppendedField]); 6] = [
        (TableId::Players, &[parity(4, "ordinal")]),
        (
            TableId::OperativeMissions,
            &[
                parity(18, "map_selected"),
                parity(19, "map_warning"),
                parity(20, "active_effect"),
                parity(21, "pending_effect"),
            ],
        ),
        (TableId::AgencyOperationBuffers, &[parity(4, "offers")]),
        (TableId::RaidIntelInputs, &[parity(4, "defense_tree")]),
        (TableId::StrategicOperatives, &[parity(5, "mission_rosters")]),
        (TableId::AirGroundCrews, &[parity(1, "supply")]),
    ];
    let mut index = 0;
    while index < PARITY_FIELDS.len() {
        let (id, fields) = PARITY_FIELDS[index];
        history[id.index()] = LastChange {
            introduced: history[id.index()].introduced,
            ..appended_at(id, PARITY, "nonpolitical map mode operands", fields)
        };
        index += 1;
    }
    let mut copies = 0;
    while copies < STORED_ONCE_TABLES.len() {
        let id = STORED_ONCE_TABLES[copies];
        history[id.index()] = LastChange {
            introduced: history[id.index()].introduced,
            ..table(id, STORED_ONCE_FORMAT, "stored once")
        };
        copies += 1;
    }
    const CAPTURE: u16 = CAPTURE_ADDITIONS_FORMAT;
    const CAPTURE_FIELDS: [(TableId, u16, &[AppendedField]); 2] = [
        (
            TableId::Divisions,
            PRIVATE_COPY_ALIASES_FORMAT,
            &[AppendedField {
                index: 9,
                name: "theatre",
                introduced: CAPTURE,
            }],
        ),
        (
            TableId::DivisionMovement,
            STORED_ONCE_FORMAT,
            &[AppendedField {
                index: 22,
                name: "queued_actions",
                introduced: CAPTURE,
            }],
        ),
    ];
    let mut capture = 0;
    while capture < CAPTURE_FIELDS.len() {
        let (id, layout_since, fields) = CAPTURE_FIELDS[capture];
        history[id.index()] = LastChange {
            introduced: history[id.index()].introduced,
            layout_since,
            ..appended_at(id, CAPTURE, "capture additions", fields)
        };
        capture += 1;
    }
    history[TableId::PowerSpendDays.index()] = LastChange {
        introduced: POWER_LEDGER_DAY_LOG_FORMAT,
        ..table(
            TableId::PowerSpendDays,
            POWER_LEDGER_DAY_LOG_FORMAT,
            "power ledger day log",
        )
    };
    history[TableId::EquipmentLossCoverage.index()] = appended_at(
        TableId::EquipmentLossCoverage,
        EQUIPMENT_DESTRUCTION_STARTUP_FORMAT,
        "equipment losses startup coverage",
        &[AppendedField {
            index: 15,
            name: "drain_state",
            introduced: EQUIPMENT_DESTRUCTION_STARTUP_FORMAT,
        }],
    );
    history[TableId::LandCombatEdges.index()] = LastChange {
        introduced: LAND_COMBAT_EDGES_FORMAT,
        ..table(
            TableId::LandCombatEdges,
            LAND_COMBAT_EDGES_FORMAT,
            "land combat openings and division exits",
        )
    };
    history[TableId::CombatEngagementRefreshes.index()] = LastChange {
        introduced: ENGAGED_REFRESH_WITNESS_FORMAT,
        ..table(
            TableId::CombatEngagementRefreshes,
            ENGAGED_REFRESH_WITNESS_FORMAT,
            "combat engagement refresh witnesses",
        )
    };
    history[TableId::Raids.index()] = appended_at(
        TableId::Raids,
        RAID_COMPLETION_FIELDS_FORMAT,
        "raid completion date and country",
        &[
            AppendedField {
                index: 18,
                name: "end_hour",
                introduced: RAID_COMPLETION_FIELDS_FORMAT,
            },
            AppendedField {
                index: 19,
                name: "completion_country",
                introduced: RAID_COMPLETION_FIELDS_FORMAT,
            },
            AppendedField {
                index: 20,
                name: "completion_recorded",
                introduced: RAID_COMPLETION_FIELDS_FORMAT,
            },
        ],
    );
    history[TableId::RaidEnds.index()] = appended_at(
        TableId::RaidEnds,
        RAID_COMPLETION_FIELDS_FORMAT,
        "raid completion date and country",
        &[
            AppendedField {
                index: 24,
                name: "end_hour",
                introduced: RAID_COMPLETION_FIELDS_FORMAT,
            },
            AppendedField {
                index: 25,
                name: "completion_country",
                introduced: RAID_COMPLETION_FIELDS_FORMAT,
            },
            AppendedField {
                index: 26,
                name: "completion_country_slot",
                introduced: RAID_COMPLETION_FIELDS_FORMAT,
            },
        ],
    );
    let windows: [(TableId, &'static [AppendedField]); 2] = [
        (
            TableId::EventFeedHistory,
            &[AppendedField {
                index: 10,
                name: "windows",
                introduced: EVENT_WINDOWS_FORMAT,
            }],
        ),
        (
            TableId::FeedEvents,
            &[
                AppendedField {
                    index: 42,
                    name: "window",
                    introduced: EVENT_WINDOWS_FORMAT,
                },
                AppendedField {
                    index: 43,
                    name: "presentation",
                    introduced: EVENT_WINDOWS_FORMAT,
                },
            ],
        ),
    ];
    let mut appended = 0;
    while appended < windows.len() {
        let (id, fields) = windows[appended];
        let earlier = history[id.index()];
        history[id.index()] = LastChange {
            introduced: earlier.introduced,
            layout_since: if matches!(earlier.kind, ChangeKind::AppendedOnly) {
                earlier.layout_since
            } else {
                earlier.format
            },
            ..appended_at(id, EVENT_WINDOWS_FORMAT, "event window verdicts and presentations", fields)
        };
        appended += 1;
    }
    history
};

const fn apply_catalog_passes(
    history: &mut [LastChange; TableId::ALL.len() + RecordKind::ALL.len()],
) {
    let mut host = 0;
    while host < super::PACKET_HOUR_HOSTS.len() {
        let id = super::PACKET_HOUR_HOSTS[host];
        note_change(
            &mut history[id.index()],
            PACKET_HOUR_FORMAT,
            "perf/stamps packet hours",
        );
        host += 1;
    }
    note_change(
        &mut history[TableId::ReservedCountryNaming.index()],
        RESERVED_SPLIT_FORMAT,
        "perf/reserved-split",
    );
    let mut split = 0;
    while split < super::RESERVED_SPLIT_TABLES.len() {
        let id = super::RESERVED_SPLIT_TABLES[split];
        declare_change(
            &mut history[id.index()],
            LastChange {
                introduced: RESERVED_SPLIT_FORMAT,
                ..table(id, RESERVED_SPLIT_FORMAT, "perf/reserved-split")
            },
        );
        split += 1;
    }
    let mut nearest = 0;
    while nearest < 2 {
        let id = [
            TableId::ReservedNearestNavies,
            TableId::ReservedNearestBases,
        ][nearest];
        declare_change(
            &mut history[id.index()],
            LastChange {
                introduced: RESERVED_SPLIT_FORMAT,
                ..table(id, RESERVED_SPLIT_FORMAT, "perf/reserved-split")
            },
        );
        nearest += 1;
    }
    let mut naval = 0;
    while naval < 2 {
        let id = [
            TableId::NavalModifierTrees,
            TableId::NavalModifierTreeSamples,
        ][naval];
        declare_change(
            &mut history[id.index()],
            LastChange {
                introduced: NAVAL_TREES_FORMAT,
                ..table(id, NAVAL_TREES_FORMAT, "restore/naval")
            },
        );
        naval += 1;
    }
    declare_change(
        &mut history[TableId::DivisionRetreatBans.index()],
        LastChange {
            kind: ChangeKind::Meaning,
            ..table(
                TableId::DivisionRetreatBans,
                RETREAT_BANS_FORMAT,
                "cadence/raid",
            )
        },
    );
    declare_change(
        &mut history[TableId::StockpileArchetypes.index()],
        table(
            TableId::StockpileArchetypes,
            ESSENTIAL_STOCK_FORMAT,
            "cadence/raid",
        ),
    );
    declare_change(
        &mut history[TableId::RaidPathDistances.index()],
        table(
            TableId::RaidPathDistances,
            RAID_DISPATCHERS_FORMAT,
            "raid-dispatchers",
        ),
    );
    let mut r = 0;
    while r < RETIRED_TABLES.len() {
        history[RETIRED_TABLES[r].index()].retired = Some(RETIRED_INPUTS_FORMAT);
        r += 1;
    }
    let mut field = 0;
    while field < RETIRED_FIELDS.len() {
        let id = RETIRED_FIELDS[field].table;
        let entry = history[id.index()];
        let earlier_layout = !matches!(entry.kind, ChangeKind::AppendedOnly)
            && !matches!(entry.kind, ChangeKind::Unchanged)
            && entry.format > entry.introduced;
        if earlier_layout {
            field += 1;
            continue;
        }
        history[id.index()].kind = ChangeKind::AppendedOnly;
        note_change(
            &mut history[id.index()],
            RETIRED_INPUTS_FORMAT,
            "perf/eta-tooltips, perf/supply-dedupe, perf/retire-tooltips and perf/retire-previews retired inputs",
        );
        field += 1;
    }
    note_change(
        &mut history[TableId::DivisionStatus.index()],
        RETIRED_INPUTS_FORMAT,
        "perf/supply-dedupe, perf/retire-tooltips retired inputs, perf/division-status-2 camouflage, perf/attack-preview inputs, restore/supply footer, pol/oracle-base-diffs name template",
    );
}

const fn note_change(entry: &mut LastChange, format: u16, commit: &'static str) {
    if format >= entry.format {
        entry.format = format;
        entry.commit = commit;
    }
}

const fn declare_change(entry: &mut LastChange, declaration: LastChange) {
    if declaration.format >= entry.format {
        *entry = declaration;
    }
}

const fn appended(
    id: TableId,
    commit: &'static str,
    fields: &'static [AppendedField],
) -> LastChange {
    appended_at(id, RECORDER_INPUTS_FORMAT, commit, fields)
}

const fn appended_at(
    id: TableId,
    format: u16,
    commit: &'static str,
    fields: &'static [AppendedField],
) -> LastChange {
    let mut change = table(id, format, commit);
    change.kind = ChangeKind::AppendedOnly;
    change.appended = fields;
    change
}

const fn table(id: TableId, format: u16, commit: &'static str) -> LastChange {
    LastChange {
        subject: Subject::Table(id),
        introduced: PROVEN_FROM,
        retired: None,
        format,
        commit,
        kind: ChangeKind::Layout,
        appended: &[],
        layout_since: 0,
    }
}

pub struct EarlierLayout {
    pub table: TableId,
    pub until: u16,
    pub fields: usize,
}

pub static EARLIER_LAYOUTS: [EarlierLayout; 6] = [
    EarlierLayout {
        table: TableId::AirNavalPatrolHistories,
        until: AIR_HISTORY_SLOTS_FORMAT,
        fields: 3,
    },
    EarlierLayout {
        table: TableId::DivisionMovement,
        until: STORED_ONCE_FORMAT,
        fields: 25,
    },
    EarlierLayout {
        table: TableId::DivisionPanelStats,
        until: STORED_ONCE_FORMAT,
        fields: 23,
    },
    EarlierLayout {
        table: TableId::DivisionStatus,
        until: STORED_ONCE_FORMAT,
        fields: 45,
    },
    EarlierLayout {
        table: TableId::LandCombatDivisionModifiers,
        until: STORED_ONCE_FORMAT,
        fields: 2,
    },
    EarlierLayout {
        table: TableId::LandCombatDivisions,
        until: STORED_ONCE_FORMAT,
        fields: 42,
    },
];

impl LastChange {
    pub const fn present(&self, format: u16) -> bool {
        format >= self.introduced
            && match self.retired {
                Some(retired) => format < retired,
                None => true,
            }
    }

    pub fn table_gap(&self, table: TableId, format: u16) -> Option<String> {
        self.gap(table, format).map(|gap| gap.to_string())
    }

    pub fn gap(&self, table: TableId, format: u16) -> Option<TableGap> {
        let cause = if format < self.introduced {
            GapCause::Introduced(self.introduced)
        } else if let Some(retired) = self
            .retired
            .filter(|retired| *retired <= crate::FORMAT_VERSION)
        {
            GapCause::Retired(retired)
        } else if format < self.format && self.kind != ChangeKind::AppendedOnly {
            GapCause::Changed(self.format)
        } else if format < self.layout_since {
            GapCause::Changed(self.layout_since)
        } else {
            return None;
        };
        Some(TableGap {
            table,
            format,
            cause,
        })
    }

    pub fn fields(&self, table: TableId, format: u16) -> usize {
        self.appended
            .iter()
            .find(|field| format < field.introduced)
            .map_or(table.field_count(), |field| field.index)
    }

    pub fn framed_fields(&self, table: TableId, format: u16) -> usize {
        EARLIER_LAYOUTS
            .iter()
            .filter(|earlier| earlier.table == table && format < earlier.until)
            .min_by_key(|earlier| earlier.until)
            .map_or_else(|| self.fields(table, format), |earlier| earlier.fields)
    }
}

impl TableId {
    pub const fn last_change(self) -> &'static LastChange {
        &HISTORY[self.index()]
    }
}

impl RecordKind {
    pub const fn last_change(self) -> &'static LastChange {
        &HISTORY[TableId::ALL.len() + self as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_combat_menu_appends_leave_earlier_layouts_readable() {
        let before = EXACT_NAVAL_CONDITION_FORMAT;
        for (table, fields) in [
            (TableId::Cryptology, 4),
            (TableId::RaidAirScores, 6),
            (TableId::WeatherRegions, 14),
        ] {
            let history = table.last_change();
            assert_eq!(history.kind, ChangeKind::AppendedOnly, "{table:?}");
            assert!(history.gap(table, before).is_none(), "{table:?}");
            assert_eq!(history.fields(table, before), fields, "{table:?}");
        }
        let raid = TableId::RaidAirScores.last_change();
        assert!(raid.gap(TableId::RaidAirScores, AIR_MAP_FORMAT - 1).is_some());
        assert!(raid.gap(TableId::RaidAirScores, AIR_MAP_FORMAT).is_none());
        let combat = TableId::AirRegionCombat.last_change();
        assert!(combat.gap(TableId::AirRegionCombat, before).is_none());
        let record = RecordKind::AirCombatRecord.last_change();
        assert_eq!(record.kind, ChangeKind::AppendedOnly);
        assert_eq!(
            (record.appended[0].name, record.appended[0].introduced),
            ("AirCombatRecord.equipment", AIR_COMBAT_MENU_FORMAT)
        );
    }

    #[test]
    fn earlier_native_formats_frame_their_tables_as_written() {
        let fewer = |format: u16| -> Vec<(TableId, usize)> {
            let mut fewer = vec![
                (TableId::Divisions, 9),
                (TableId::RailwayGunCounterInputs, 30),
                (TableId::DivisionMovement, 25),
                (TableId::DivisionPanelStats, 23),
                (TableId::DivisionStatus, 45),
                (TableId::LandCombatDivisionModifiers, 2),
                (TableId::LandCombatDivisions, 42),
                (TableId::States, 13),
                (TableId::Players, 4),
                (TableId::RaidIntelInputs, 4),
                (TableId::OperativeMissions, 18),
                (TableId::StrategicOperatives, 5),
                (TableId::AgencyOperationBuffers, 4),
                (TableId::AirGroundCrews, 1),
            ];
            if format < AIR_COMBAT_MENU_FORMAT {
                fewer.extend([
                    (TableId::Cryptology, 4),
                    (TableId::WeatherRegions, 14),
                    (TableId::RaidAirScores, 6),
                ]);
            }
            if format < AIR_HISTORY_SLOTS_FORMAT {
                fewer.push((TableId::AirNavalPatrolHistories, 3));
            }
            if format < CAPTURE_ADDITIONS_FORMAT {
                fewer.extend([
                    (TableId::WeatherRegions, 15),
                    (TableId::RailCooldowns, 4),
                    (TableId::ConvoyClients, 11),
                    (TableId::ResourceOrigins, 8),
                    (TableId::FactionTheatres, 4),
                    (TableId::WeatherProvinces, 16),
                ]);
            }
            if format < EQUIPMENT_DESTRUCTION_STARTUP_FORMAT {
                fewer.push((TableId::EquipmentLossCoverage, 15));
            }
            if format < RAID_COMPLETION_FIELDS_FORMAT {
                fewer.extend([(TableId::Raids, 18), (TableId::RaidEnds, 24)]);
            }
            if format < EVENT_WINDOWS_FORMAT {
                fewer.extend([(TableId::EventFeedHistory, 10), (TableId::FeedEvents, 42)]);
            }
            if format < CONSTRUCTION_OUTPUT_OWNER_FORMAT {
                fewer.push((TableId::ConstructionLineOutput, 15));
            }
            fewer
        };
        for format in [
            MARKET_AUTOMATION_FORMAT,
            EXACT_COMBAT_FORMAT,
            PLAYER_COMMANDS_FORMAT,
            PROVINCE_WITNESS_FORMAT,
            CONSTRUCTION_OUTPUT_OWNER_FORMAT,
            EXACT_NAVAL_CONDITION_FORMAT,
            AIR_HISTORY_SLOTS_FORMAT,
        ] {
            let fewer = fewer(format);
            for table in TableId::ALL {
                let expected = fewer
                    .iter()
                    .find(|&&(held, _)| held == table)
                    .map_or(table.field_count(), |&(_, fields)| fields);
                assert_eq!(
                    table.last_change().framed_fields(table, format),
                    expected,
                    "{table:?} at {format}"
                );
            }
        }
    }

    #[test]
    fn map_mode_parity_fields_leave_earlier_columns_readable() {
        let states = TableId::States.last_change();
        assert!(states.gap(TableId::States, AIR_HISTORY_SLOTS_FORMAT).is_none());
        assert_eq!(states.fields(TableId::States, AIR_HISTORY_SLOTS_FORMAT), 13);
        assert_eq!(states.fields(TableId::States, MAP_MODE_PARITY_FORMAT), 14);
        for (table, fields) in [
            (TableId::Players, 4),
            (TableId::OperativeMissions, 18),
            (TableId::AgencyOperationBuffers, 4),
            (TableId::RaidIntelInputs, 4),
            (TableId::StrategicOperatives, 5),
            (TableId::AirGroundCrews, 1),
        ] {
            let history = table.last_change();
            assert_eq!(history.format, MAP_MODE_PARITY_FORMAT, "{table:?}");
            assert_eq!(history.kind, ChangeKind::AppendedOnly, "{table:?}");
            assert!(history.gap(table, AIR_HISTORY_SLOTS_FORMAT).is_none(), "{table:?}");
            assert_eq!(history.fields(table, AIR_HISTORY_SLOTS_FORMAT), fields);
            assert_eq!(history.fields(table, MAP_MODE_PARITY_FORMAT), table.field_count());
        }
    }

    #[test]
    fn older_retreat_bans_and_stock_are_refused() {
        for table in [TableId::DivisionRetreatBans, TableId::StockpileArchetypes] {
            assert!(
                table
                    .last_change()
                    .table_gap(table, POLITICAL_FORMAT)
                    .unwrap()
                    .contains(&format!("{table:?}"))
            );
            assert!(
                table
                    .last_change()
                    .table_gap(table, FORMAT_VERSION)
                    .is_none()
            );
        }
    }

    #[test]
    fn technology_key_changes_refuse_only_the_changed_contracts() {
        let historical = WIDGET_CAPTURE_FORMAT;
        for table in [
            TableId::ResearchTechnologies,
            TableId::TechnologyDefinitions,
        ] {
            assert!(
                table
                    .last_change()
                    .table_gap(table, historical)
                    .unwrap()
                    .contains(&format!("{table:?}"))
            );
            assert!(
                table
                    .last_change()
                    .table_gap(table, FORMAT_VERSION)
                    .is_none()
            );
        }
        for table in [
            TableId::ResearchSlots,
            TableId::ResearchInputs,
            TableId::Countries,
            TableId::Clock,
        ] {
            assert!(table.last_change().table_gap(table, historical).is_none());
        }
        assert_eq!(
            TableId::ResearchSlots
                .last_change()
                .fields(TableId::ResearchSlots, historical),
            5
        );
        assert_eq!(
            TableId::ResearchInputs
                .last_change()
                .fields(TableId::ResearchInputs, historical),
            15
        );
    }

    #[test]
    fn native_war_operands_are_named_gaps_before_their_introduction() {
        for table in [
            TableId::WarOwnedRelations,
            TableId::WarRelationGoals,
            TableId::WarOverviewCountries,
            TableId::WarNameRules,
        ] {
            let history = table.last_change();
            for format in PROVEN_FROM..WAR_OVERVIEW_FORMAT {
                assert!(!history.present(format));
                assert!(
                    history
                        .table_gap(table, format)
                        .unwrap()
                        .contains("not recorded")
                );
            }
            assert!(history.present(WAR_OVERVIEW_FORMAT));
            assert!(history.table_gap(table, WAR_OVERVIEW_FORMAT).is_none());
        }
    }

    #[test]
    fn peace_actions_are_named_gaps_before_their_introduction() {
        for table in [TableId::PeaceSettlements, TableId::PeaceBids] {
            let history = table.last_change();
            for format in PROVEN_FROM..PEACE_SETTLEMENTS_FORMAT {
                assert!(!history.present(format));
                assert!(
                    history
                        .table_gap(table, format)
                        .unwrap()
                        .contains("not recorded")
                );
            }
            assert!(history.present(PEACE_SETTLEMENTS_FORMAT));
            assert!(history.table_gap(table, PEACE_SETTLEMENTS_FORMAT).is_none());
        }
    }

    #[test]
    fn retired_inputs_refuse_by_name_in_every_recording() {
        for table in RETIRED_TABLES {
            let history = table.last_change();
            assert!(!history.present(FORMAT_VERSION), "{table:?}");
            assert!(
                history
                    .table_gap(table, FORMAT_VERSION)
                    .unwrap()
                    .contains("retired at format")
            );
        }
        for table in [
            TableId::DivisionEtaInputs,
            TableId::AttritionModifierBreakdowns,
            TableId::ReserveModifierBreakdowns,
            TableId::NavalModifierBreakdowns,
            TableId::OwnedWargoals,
            TableId::AirRegionHistory,
            TableId::AirRegionDamage,
        ] {
            let history = table.last_change();
            assert!(history.present(POLITICAL_FORMAT));
            assert!(
                history
                    .table_gap(table, POLITICAL_FORMAT)
                    .unwrap()
                    .contains(&format!(
                        "{table:?} retired at format {RETIRED_INPUTS_FORMAT}"
                    ))
            );
        }
        for field in RETIRED_FIELDS {
            let history = field.table.last_change();
            if field.table == TableId::ProductionLines {
                assert_eq!(history.kind, ChangeKind::Layout);
                assert_eq!(history.format, PRIVATE_COPY_ALIASES_FORMAT);
                assert!(history.table_gap(field.table, POLITICAL_FORMAT).is_some());
                assert!(history.table_gap(field.table, FORMAT_VERSION).is_none());
                continue;
            }
            assert_eq!(history.kind, ChangeKind::AppendedOnly, "{:?}", field.table);
            let retired = RETIRED_FIELDS
                .iter()
                .filter(|other| other.table == field.table)
                .map(|other| other.retired)
                .max()
                .unwrap_or(field.retired);
            let current = history
                .appended
                .iter()
                .map(|operand| operand.introduced)
                .max()
                .unwrap_or(retired)
                .max(retired);
            assert_eq!(history.format, current, "{:?}", field.table);
            assert!(
                history.table_gap(field.table, field.retired - 1).is_none(),
                "{:?}",
                field.table
            );
            assert!(history.table_gap(field.table, FORMAT_VERSION).is_none());
        }
    }

    #[test]
    fn combat_log_observer_numeric_lanes_become_daily_at_116() {
        let table = TableId::CombatLogObservers;
        let history = table.last_change();
        assert_eq!(history.introduced, COMBAT_LOG_FORMAT);
        assert_eq!(history.format, DAILY_PROGRESS_FORMAT);
        assert_eq!(history.kind, ChangeKind::Meaning);
        assert!(history.table_gap(table, 105).is_some());
        assert!(history.table_gap(table, DAILY_PROGRESS_FORMAT).is_none());
    }

    #[test]
    fn stored_once_relays_its_tables_and_refuses_older_recordings() {
        for table in STORED_ONCE_TABLES {
            let history = table.last_change();
            if table == TableId::DivisionMovement {
                assert_eq!(history.format, CAPTURE_ADDITIONS_FORMAT);
                assert_eq!(history.layout_since, STORED_ONCE_FORMAT);
            } else {
                assert_eq!(history.format, STORED_ONCE_FORMAT);
                assert_eq!(history.kind, ChangeKind::Layout);
            }
            assert!(history.table_gap(table, STORED_ONCE_FORMAT - 1).is_some());
            assert!(history.table_gap(table, STORED_ONCE_FORMAT).is_none());
            assert!(RETIRED_FIELDS.iter().all(|field| field.table != table));
        }
        assert_eq!(TableId::DivisionStatus.field_count(), 41);
        assert_eq!(TableId::DivisionMovement.field_count(), 23);
        assert_eq!(TableId::DivisionPanelStats.field_count(), 25);
        assert_eq!(TableId::LandCombatDivisionModifiers.field_count(), 2);
        assert_eq!(TableId::LandCombatDivisions.field_count(), 15);
    }

    #[test]
    fn event_feed_occurrence_deltas_refuse_every_earlier_batch_by_name() {
        let table = TableId::EventFeedHistory;
        let history = table.last_change();
        assert_eq!(history.format, EVENT_WINDOWS_FORMAT);
        assert_eq!(history.layout_since, EVENT_FEED_OCCURRENCE_DELTAS_FORMAT);
        assert_eq!(history.fields(table, EVENT_FEED_OCCURRENCE_DELTAS_FORMAT), 10);
        assert_eq!(history.fields(table, EVENT_WINDOWS_FORMAT), 11);
        for format in [
            EVENT_FEED_COLUMNS_FORMAT,
            EVENT_FEED_OWNER_REFUSALS_FORMAT,
            MIO_POLICY_COST_FORMAT,
        ] {
            assert!(history.table_gap(table, format).is_some());
        }
        assert!(
            history
                .table_gap(table, EVENT_FEED_OCCURRENCE_DELTAS_FORMAT)
                .is_none()
        );
    }

    #[test]
    fn resource_format_keeps_mains_table_and_field_introductions() {
        assert_eq!(RESOURCES_MAP_FORMAT, 103);
        assert_eq!(TableId::TradeRoutes.field_count(), 6);
        let route_history = TableId::TradeRoutes.last_change();
        assert_eq!((route_history.introduced, route_history.format), (103, 103));
        assert!(route_history.appended.is_empty());
        for format in [
            STATE_CATEGORY_FORMAT,
            STRATEGIC_LOCATIONS_FORMAT,
            BATTLE_SOUNDS_FORMAT,
            OPERATIVE_MISSIONS_FORMAT,
            STRATEGIC_OPERATIVES_FORMAT,
            AGENCY_OPERATION_BUFFERS_FORMAT,
            OPERATION_PREPARATION_FORMAT,
        ] {
            assert_eq!(format, 102);
        }
        assert!(TableId::BattleSounds.last_change().present(102));
        assert!(!TableId::ResourceOrigins.last_change().present(102));
        assert!(TableId::ResourceOrigins.last_change().present(103));
        assert_eq!(
            TableId::ResourceOrigins as u16,
            TableId::AgencyOperationBuffers as u16 + 1
        );
        assert_eq!(
            TableId::RegionalConvoyEfficiency as u16,
            TableId::ResourceOrigins as u16 + 6
        );
    }

    #[test]
    fn manpower_exile_field_has_its_current_format_catalog_entry() {
        let history = TableId::DivisionTemplateIcons.last_change();
        let field = history
            .appended
            .iter()
            .find(|field| field.index == 16)
            .unwrap();
        assert_eq!(field.name, "manpower_exile");
        assert_eq!(field.introduced, 104);
        assert_eq!(history.format, 104);
        assert_eq!(history.kind, ChangeKind::AppendedOnly);
    }

    #[test]
    fn later_appended_fields_survive_every_catalog_pass() {
        const LATER_FORMAT: u16 = FORMAT_VERSION + 1;
        for kind in [
            ChangeKind::AppendedOnly,
            ChangeKind::Layout,
            ChangeKind::Meaning,
        ] {
            let mut history = HISTORY;
            for table in TableId::ALL {
                history[table.index()] = LastChange {
                    kind,
                    ..appended_at(
                        table,
                        LATER_FORMAT,
                        "later declaration fixture",
                        &[AppendedField {
                            index: 0,
                            name: "later_operand",
                            introduced: LATER_FORMAT,
                        }],
                    )
                };
            }

            apply_catalog_passes(&mut history);

            for table in TableId::ALL {
                let entry = history[table.index()];
                assert_eq!(entry.format, LATER_FORMAT, "{table:?} {kind:?}");
                assert_eq!(entry.kind, kind, "{table:?}");
                assert_eq!(entry.introduced, PROVEN_FROM, "{table:?}");
                assert_eq!(entry.commit, "later declaration fixture", "{table:?}");
                assert_eq!(entry.appended.len(), 1, "{table:?}");
                assert_eq!(entry.appended[0].name, "later_operand", "{table:?}");
                assert_eq!(entry.appended[0].introduced, LATER_FORMAT, "{table:?}");
                assert_eq!(
                    entry.retired,
                    RETIRED_TABLES
                        .contains(&table)
                        .then_some(RETIRED_INPUTS_FORMAT),
                    "{table:?}"
                );
            }
        }
    }

    #[test]
    fn occupation_layout_survives_catalog_passes() {
        let table = TableId::StateResistance;
        let mut history = HISTORY;
        history[table.index()] = LastChange {
            kind: ChangeKind::Layout,
            ..appended_at(
                table,
                OCCUPATION_MAP_FORMAT,
                "occupation layout fixture",
                &[
                    AppendedField {
                        index: 6,
                        name: "hover",
                        introduced: 104,
                    },
                    AppendedField {
                        index: 7,
                        name: "modifier_sources",
                        introduced: 104,
                    },
                    AppendedField {
                        index: 8,
                        name: "map",
                        introduced: OCCUPATION_MAP_FORMAT,
                    },
                ],
            )
        };

        apply_catalog_passes(&mut history);

        let entry = history[table.index()];
        assert_eq!(entry.format, OCCUPATION_MAP_FORMAT);
        assert_eq!(entry.kind, ChangeKind::Layout);
        assert_eq!(entry.commit, "occupation layout fixture");
        assert_eq!(
            entry
                .appended
                .iter()
                .map(|field| (field.index, field.name, field.introduced))
                .collect::<Vec<_>>(),
            [
                (6, "hover", 104),
                (7, "modifier_sources", 104),
                (8, "map", 105)
            ]
        );
        assert!(entry.table_gap(table, OCCUPATION_MAP_FORMAT).is_none());
    }

    #[test]
    fn occupation_map_suffix_has_current_layout_history() {
        let history = TableId::StateResistance.last_change();
        assert_eq!(history.format, OCCUPATION_MAP_FORMAT);
        assert_eq!(history.kind, ChangeKind::Layout);
        assert_eq!(TableId::StateResistance.field_count(), 9);
        assert_eq!(
            history
                .appended
                .iter()
                .map(|field| (field.index, field.name, field.introduced))
                .collect::<Vec<_>>(),
            [
                (6, "hover", 104),
                (7, "modifier_sources", 104),
                (8, "map", 105)
            ]
        );
    }

    #[test]
    fn wargoal_inventory_catalog_entries_survive_every_pass() {
        for table in [
            TableId::CountryWargoals,
            TableId::WargoalTargets,
            TableId::WargoalJustifications,
        ] {
            let entry = table.last_change();
            assert_eq!(entry.introduced, 104);
            assert_eq!(entry.format, 104);
            assert_eq!(entry.kind, ChangeKind::Layout);
            assert!(entry.table_gap(table, 103).is_some());
            assert!(entry.table_gap(table, 104).is_none());
        }
    }

    #[test]
    fn saved_design_tables_begin_at_their_declared_introduction() {
        for table in [
            TableId::VariantChanges,
            TableId::VariantBonusSnapshots,
            TableId::DesignCountryModifiers,
            TableId::DesignCountryTemplates,
            TableId::DesignObserverContext,
            TableId::DesignOrganisationLiveness,
        ] {
            let entry = table.last_change();
            let expected_format = if matches!(table, TableId::VariantChanges) {
                VARIANT_STAT_WITNESS_FORMAT
            } else {
                DESIGN_STATE_FORMAT
            };
            assert_eq!(entry.introduced, DESIGN_STATE_FORMAT, "{table:?}");
            assert_eq!(entry.format, expected_format, "{table:?}");
            assert_eq!(entry.kind, ChangeKind::Layout, "{table:?}");
            for version in [PROVEN_FROM, 97, DESIGN_STATE_FORMAT - 1] {
                assert!(!entry.present(version), "{table:?} in format {version}");
                assert_eq!(
                    entry.table_gap(table, version).unwrap(),
                    format!(
                        "{table:?} not recorded at format {version} (introduced at format {DESIGN_STATE_FORMAT})"
                    ),
                );
            }
            assert!(entry.present(DESIGN_STATE_FORMAT), "{table:?}");
            if matches!(table, TableId::VariantChanges) {
                assert_eq!(
                    entry.table_gap(table, DESIGN_STATE_FORMAT).unwrap(),
                    format!(
                        "{table:?} layout or meaning changed at format {VARIANT_STAT_WITNESS_FORMAT}; this recording is format {DESIGN_STATE_FORMAT}"
                    ),
                );
            } else {
                assert!(entry.table_gap(table, DESIGN_STATE_FORMAT).is_none());
            }
            assert!(entry.present(FORMAT_VERSION), "{table:?}");
            assert!(entry.table_gap(table, FORMAT_VERSION).is_none());
        }
    }

    #[test]
    fn template_extensions_keep_their_format_after_packet_hour_changes() {
        for table in [
            TableId::DivisionTemplateIcons,
            TableId::OrdersGroupCounterInputs,
        ] {
            let entry = table.last_change();
            assert_eq!(entry.format, TEMPLATE_SETTINGS_FORMAT, "{table:?}");
            assert!(
                entry
                    .appended
                    .iter()
                    .all(|field| field.introduced <= entry.format)
            );
        }
    }

    #[test]
    fn scripted_map_catalog_records_cached_fields_and_expanded_event_layout() {
        for (table, latest, fields) in [
            (
                TableId::States,
                CAPTURE_ADDITIONS_FORMAT,
                &[(11, "best_vp_index"), (12, "single_controller")][..],
            ),
            (
                TableId::SimulationFeatures,
                SCRIPTED_MAP_FORMAT,
                &[(6, "state_array_len")][..],
            ),
        ] {
            let history = table.last_change();
            assert_eq!(history.format, latest);
            assert_eq!(history.kind, ChangeKind::AppendedOnly);
            for &(index, name) in fields {
                let field = history
                    .appended
                    .iter()
                    .find(|field| field.index == index)
                    .unwrap();
                assert_eq!((field.name, field.introduced), (name, SCRIPTED_MAP_FORMAT));
            }
        }
        let events = TableId::MapEvents.last_change();
        assert_eq!(events.introduced, MAP_EVENTS_FORMAT);
        assert_eq!(events.format, SCRIPTED_MAP_FORMAT);
        assert_eq!(events.kind, ChangeKind::Layout);
        assert_eq!(crate::rows::MapEventFamily::COUNT, 8);
    }

    #[test]
    fn recorder_format_changes_follow_main_in_their_original_order() {
        assert_eq!(EVENT_FEED_OWNER_REFUSALS_FORMAT, 109);
        assert_eq!(CONVERSION_PURPOSE_FORMAT, 110);
        assert_eq!(
            [
                SAME_FRAME_ALIASES_FORMAT,
                SAME_FRAME_OPERAND_ALIASES_FORMAT,
                PRIVATE_ALIASES_FORMAT,
                PRIVATE_COPY_ALIASES_FORMAT,
                PRIVATE_DISPLAY_ALIASES_FORMAT,
                DAILY_PROGRESS_FORMAT,
                UNIT_BONUSES_FORMAT,
                MIO_POLICY_COST_FORMAT,
                EVENT_FEED_OCCURRENCE_DELTAS_FORMAT,
                PANEL_SUPPRESSION_FORMAT,
                CAPTURE_MODE_FORMAT,
                DIVISION_MODEL_INPUTS_FORMAT,
                CHECKPOINT_FORMAT,
                CHUNKED_FORMAT,
                RAID_DISPATCHERS_FORMAT,
                MARKET_AUTOMATION_FORMAT,
                EXACT_COMBAT_FORMAT,
            ],
            [
                111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127
            ],
        );
        assert_eq!(PLAYER_COMMANDS_FORMAT, 131);
        assert_eq!(POWER_LEDGER_DAY_LOG_FORMAT, 143);
        assert_eq!(VARIANT_STAT_WITNESS_FORMAT, 144);
        assert_eq!(EQUIPMENT_DESTRUCTION_STARTUP_FORMAT, 145);
        assert_eq!(ENGAGED_REFRESH_WITNESS_FORMAT, 146);
        assert_eq!(RAID_COMPLETION_FIELDS_FORMAT, 147);
        assert_eq!(EVENT_WINDOWS_FORMAT, 148);
        assert_eq!(LAND_COMBAT_EDGES_FORMAT, 149);
        assert_eq!(FORMAT_VERSION, LAND_COMBAT_EDGES_FORMAT);
        let edges = TableId::LandCombatEdges.last_change();
        assert_eq!(edges.introduced, LAND_COMBAT_EDGES_FORMAT);
        assert!(edges.table_gap(TableId::LandCombatEdges, EVENT_WINDOWS_FORMAT).is_some());
        assert!(edges.table_gap(TableId::LandCombatEdges, LAND_COMBAT_EDGES_FORMAT).is_none());
        let refreshes = TableId::CombatEngagementRefreshes.last_change();
        assert_eq!(refreshes.introduced, ENGAGED_REFRESH_WITNESS_FORMAT);
        assert_eq!(refreshes.format, ENGAGED_REFRESH_WITNESS_FORMAT);
        assert_eq!(refreshes.kind, ChangeKind::Layout);
        assert!(refreshes.table_gap(TableId::CombatEngagementRefreshes, EQUIPMENT_DESTRUCTION_STARTUP_FORMAT).is_some());
        assert!(refreshes.table_gap(TableId::CombatEngagementRefreshes, ENGAGED_REFRESH_WITNESS_FORMAT).is_none());
        assert_eq!(TableId::PowerSpendTotals.field_count(), 32);
        let spend_days = TableId::PowerSpendDays.last_change();
        assert_eq!(spend_days.introduced, POWER_LEDGER_DAY_LOG_FORMAT);
        assert_eq!(spend_days.format, POWER_LEDGER_DAY_LOG_FORMAT);
        assert_eq!(spend_days.kind, ChangeKind::Layout);
        assert!(spend_days.table_gap(TableId::PowerSpendDays, CAPTURE_ADDITIONS_FORMAT).is_some());
        assert!(spend_days.table_gap(TableId::PowerSpendDays, POWER_LEDGER_DAY_LOG_FORMAT).is_none());
        let coverage = TableId::EquipmentLossCoverage.last_change();
        assert_eq!(coverage.format, EQUIPMENT_DESTRUCTION_STARTUP_FORMAT);
        assert_eq!(coverage.kind, ChangeKind::AppendedOnly);
        assert!(coverage.gap(TableId::EquipmentLossCoverage, VARIANT_STAT_WITNESS_FORMAT).is_none());
        assert_eq!(coverage.fields(TableId::EquipmentLossCoverage, VARIANT_STAT_WITNESS_FORMAT), 15);
        assert_eq!(coverage.fields(TableId::EquipmentLossCoverage, EQUIPMENT_DESTRUCTION_STARTUP_FORMAT), 16);
        let change = TableId::RaidPathDistances.last_change();
        assert_eq!(change.format, RAID_DISPATCHERS_FORMAT);
        assert_eq!(change.kind, ChangeKind::Layout);
        assert!(
            change
                .table_gap(TableId::RaidPathDistances, CHUNKED_FORMAT)
                .is_some()
        );
        let market = TableId::CountryMarket.last_change();
        assert_eq!(market.introduced, 104);
        assert_eq!(market.format, MARKET_AUTOMATION_FORMAT);
        assert_eq!(market.kind, ChangeKind::AppendedOnly);
        assert_eq!(TableId::CountryMarket.field_count(), 7);
        assert!(
            market
                .table_gap(TableId::CountryMarket, RAID_DISPATCHERS_FORMAT)
                .is_none()
        );
    }

    #[test]
    fn private_alias_columns_do_not_change_native_row_fields_or_prefixes() {
        for (table, fields) in [
            (TableId::CombatLogObservers, 11),
            (TableId::NavalCombatDetails, 7),
            (TableId::CountryFuel, 23),
        ] {
            assert_eq!(table.field_count(), fields);
            assert_eq!(
                table
                    .last_change()
                    .fields(table, PRIVATE_ALIASES_FORMAT - 1),
                fields
            );
            let daily = matches!(table, TableId::CombatLogObservers);
            assert_eq!(
                table
                    .last_change()
                    .table_gap(table, PRIVATE_ALIASES_FORMAT - 1)
                    .is_some(),
                daily,
            );
        }
    }

    #[test]
    fn copy_aliases_are_private_columns_not_native_fields() {
        for (table, fields) in [
            (TableId::Divisions, 10),
            (TableId::ProductionLines, 13),
        ] {
            assert_eq!(table.field_count(), fields);
            let format = PRIVATE_COPY_ALIASES_FORMAT;
            assert!(table.last_change().table_gap(table, format).is_none());
            assert!(table.last_change().table_gap(table, format - 1).is_some());
            assert_eq!(table.field_name(fields), None);
        }
    }

    #[test]
    fn capture_additions_leave_earlier_columns_readable() {
        let before = CAPTURE_ADDITIONS_FORMAT - 1;
        for (table, index, name, layout) in [
            (TableId::Divisions, 9, "theatre", PRIVATE_COPY_ALIASES_FORMAT),
            (TableId::DivisionMovement, 22, "queued_actions", STORED_ONCE_FORMAT),
            (TableId::RailwayGunCounterInputs, 30, "theatre", POLITICAL_FORMAT),
            (TableId::States, 14, "marked_owners", PROVEN_FROM),
        ] {
            let history = table.last_change();
            assert_eq!(history.format, CAPTURE_ADDITIONS_FORMAT, "{table:?}");
            assert_eq!(history.kind, ChangeKind::AppendedOnly, "{table:?}");
            assert_eq!(table.field_name(index), Some(name), "{table:?}");
            assert!(history.gap(table, before).is_none(), "{table:?}");
            assert_eq!(history.fields(table, before), index, "{table:?}");
            assert_eq!(history.fields(table, CAPTURE_ADDITIONS_FORMAT), table.field_count());
            assert!(history.gap(table, layout).is_none(), "{table:?}");
        }
        let guns = TableId::RailwayGunCounterInputs;
        assert_eq!(guns.field_name(31), Some("queued_actions"));
        assert_eq!(guns.field_count(), 32);
        for (table, layout) in [
            (TableId::Divisions, PRIVATE_COPY_ALIASES_FORMAT),
            (TableId::DivisionMovement, STORED_ONCE_FORMAT),
        ] {
            assert!(table.last_change().gap(table, layout - 1).is_some(), "{table:?}");
        }
    }

    #[test]
    fn compatibility_catalog_covers_every_table_and_record_once() {
        for table in TableId::ALL {
            let entry = table.last_change();
            assert_eq!(entry.subject, Subject::Table(table));
            assert!(!entry.commit.is_empty());
            assert!(entry.introduced <= entry.format);
            assert!(entry.format <= FORMAT_VERSION);
            assert!(
                entry
                    .retired
                    .is_none_or(|retired| retired > entry.introduced
                        || (retired == entry.introduced && retired == RESERVED_SPLIT_FORMAT))
            );
            if entry.kind == ChangeKind::AppendedOnly {
                if entry.appended.is_empty() {
                    assert!(
                        RETIRED_FIELDS.iter().any(|field| field.table == table),
                        "{table:?}"
                    );
                    continue;
                }
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(
                        field.introduced > PROVEN_FROM && field.introduced <= entry.format,
                        "{table:?}.{} introduced in {}, latest change {}",
                        field.name,
                        field.introduced,
                        entry.format
                    );
                }
                assert_eq!(first + entry.appended.len(), table.field_count());
            }
        }
        for kind in RecordKind::ALL {
            let entry = kind.last_change();
            assert_eq!(entry.subject, Subject::Record(kind));
            let fields = match kind {
                RecordKind::NavalCommanderInputs => {
                    Some(<super::super::NavalCommanderInputs as crate::value::Value>::STRUCT_FIELDS)
                }
                RecordKind::LandBorderWarInputs => {
                    Some(<super::super::LandBorderWarInputs as crate::value::Value>::STRUCT_FIELDS)
                }
                RecordKind::AirCombatRecord => {
                    Some(<super::super::AirCombatRecord as crate::value::Value>::STRUCT_FIELDS)
                }
                RecordKind::EventFeedEvent => {
                    assert_eq!(entry.kind, ChangeKind::Layout);
                    assert_eq!(entry.format, SCRIPT_EVENT_FORMAT);
                    assert!(entry.appended.is_empty());
                    None
                }
                _ => None,
            };
            if let Some(fields) = fields {
                assert!(
                    entry.introduced >= PROVEN_FROM
                        && entry.introduced < entry.appended[0].introduced
                );
                assert_eq!(entry.kind, ChangeKind::AppendedOnly);
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(field.name.contains('.'));
                    assert!(
                        field.introduced > PROVEN_FROM && field.introduced <= entry.format,
                        "{:?}.{} introduced {} after last change {}",
                        entry.subject,
                        field.name,
                        field.introduced,
                        entry.format
                    );
                }
                assert_eq!(first + entry.appended.len(), fields);
            }
        }
        let distinct: std::collections::BTreeSet<_> = HISTORY
            .iter()
            .map(|entry| format!("{:?}", entry.subject))
            .collect();
        assert_eq!(distinct.len(), HISTORY.len());
    }
}
