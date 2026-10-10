//! Proven field, wire and meaning history; no translation or old decoders.
use super::TableId;
use crate::FORMAT_VERSION;

/// Earlier history has not been audited for today's readers.
pub const PROVEN_FROM: u16 = 91;
/// Private format 95 was retired when #343 merged main's format 96; its
/// divergent table contracts were never published (PLAN format ledger).
pub const RETIRED_FORMAT: u16 = 95;
/// Provisional introduction format for the batched recorder inputs.
pub(crate) const RECORDER_INPUTS_FORMAT: u16 = 98;
/// Provisional introduction format for the second recorder-input batch.
pub(crate) const EFFECT_INPUTS_FORMAT: u16 = 99;
// Provisional until all widget capture families are integrated.
pub(crate) const WIDGET_CAPTURE_FORMAT: u16 = 100;
/// Provisional introduction format; central allocation is reconciled at merge.
pub(crate) const PRODUCTION_MODIFIERS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction format for applied fuel and logistics operands.
pub(crate) const ECONOMY_INPUTS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional main+1 construction outcome suffix; parent finalizes the shared landing.
pub(crate) const CONSTRUCTION_COMPLETION_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional append after the accepted published recorder-input prefix.
pub(crate) const OCCUPATION_GROUP_LAWS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction of ace presentation and live end-screen operands.
pub(crate) const GAME_SCORE_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional Intel suffix and observation format; renumber in the parent union.
pub(crate) const INTEL_INPUTS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction of the recorded native war-overview operands.
pub(crate) const WAR_OVERVIEW_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction of captured peace-settlement and resident bid rows.
pub(crate) const PEACE_SETTLEMENTS_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;
/// Provisional introduction for observed air loss and applied building damage events.
pub(crate) const AIR_LEDGER_FORMAT: u16 = WIDGET_CAPTURE_FORMAT;

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
}

impl RecordKind {
    pub const ALL: [Self; 17] = [
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
    /// Table lifetime in the timing/update stream. Baseline entries were
    /// already present at PROVEN_FROM; their earlier introduction is unaudited.
    pub introduced: u16,
    /// Retain retired TableIds and their field framing as wire tombstones.
    /// Never reuse an id or shift the remaining timing/update positions.
    pub retired: Option<u16>,
    /// For baseline entries: unchanged since this proven horizon, possibly older.
    pub format: u16,
    pub commit: &'static str,
    pub kind: ChangeKind,
    pub appended: &'static [AppendedField],
}

/// Audited changes retain enough framing to skip incompatible rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Unchanged,
    AppendedOnly,
    Layout,
    Meaning,
}

#[derive(Clone, Copy, Debug)]
pub struct AppendedField {
    pub index: usize,
    pub name: &'static str,
    pub introduced: u16,
}

/// One code-owned catalog for tables, nested values and framing/control kinds.
/// Evidence: row declarations and typed_codec diffs at each listed commit,
/// including the wider DivisionStatus mask (the recorded field count owns it).
/// The baseline is #302; #323/#325/#328/#341/#343 were audited against it. New
/// fields were appended; nested Opaque values keep their byte-length framing.
/// Nested suffixes use source format even when parent field counts are unchanged.
/// Record-kind dates 1/24/33 are inert below the proven horizon of 91.
/// Adding/retiring a table changes its lifetime, not the Frame framing date.
pub static HISTORY: [LastChange; TableId::ALL.len() + RecordKind::ALL.len()] = {
    let baseline = LastChange {
        subject: Subject::Table(TableId::Clock),
        introduced: PROVEN_FROM,
        retired: None,
        format: PROVEN_FROM,
        commit: "0e632a9b16",
        kind: ChangeKind::Unchanged,
        appended: &[],
    };
    let mut history = [baseline; TableId::ALL.len() + RecordKind::ALL.len()];
    history[TableId::RocketProductionOutput.index()] = table(
        TableId::RocketProductionOutput,
        WIDGET_CAPTURE_FORMAT,
        "wc/rocket",
    );
    history[TableId::RocketProductionOutput.index()].introduced = WIDGET_CAPTURE_FORMAT;
    history[TableId::ProductionLineOutput.index()] = appended_at(
        TableId::ProductionLineOutput,
        WIDGET_CAPTURE_FORMAT,
        "production-spending-inputs",
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
        ],
    );
    let mut i = 0;
    while i < TableId::ALL.len() {
        history[i].subject = Subject::Table(TableId::ALL[i]);
        i += 1;
    }
    history[TableId::ResearchTechnologies.index()] = table(
        TableId::ResearchTechnologies,
        FORMAT_VERSION,
        "technology-occurrences",
    );
    // The changed key has the same width; the old payload's five scalar
    // fields can be skipped by the compatibility reader without decoding rows.
    history[TableId::ResearchTechnologies.index()].appended = &[
        AppendedField {
            index: 5,
            name: "technology_handle",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 6,
            name: "applied_bonus_count",
            introduced: FORMAT_VERSION,
        },
    ];
    history[TableId::TechnologyDefinitions.index()] = table(
        TableId::TechnologyDefinitions,
        FORMAT_VERSION,
        "technology-occurrences",
    );
    history[TableId::ResearchSlots.index()] = appended_at(
        TableId::ResearchSlots,
        FORMAT_VERSION,
        "technology-occurrences",
        &[AppendedField {
            index: 5,
            name: "database_index",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::ResearchInputs.index()] = appended_at(
        TableId::ResearchInputs,
        FORMAT_VERSION,
        "technology-occurrences",
        &[AppendedField {
            index: 15,
            name: "database_index",
            introduced: FORMAT_VERSION,
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
            RecordKind::GapSummary => (1, "7185d6e386"),
            RecordKind::Partial => (92, "11b905d2b2"),
            RecordKind::NavalCommanderInputs => (RECORDER_INPUTS_FORMAT, "d53fa4e05c"),
            RecordKind::LandBorderWarInputs => (RECORDER_INPUTS_FORMAT, "9d09404c0d"),
            _ => (1, "5d3b4dd945"),
        };
        history[i + r] = LastChange {
            subject: Subject::Record(kind),
            introduced: format,
            retired: None,
            format,
            commit,
            kind: if matches!(kind, RecordKind::Partial) {
                ChangeKind::Layout
            } else {
                ChangeKind::Unchanged
            },
            appended: &[],
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
    history[TableId::SimulationFeatures.index()] = appended_at(
        TableId::SimulationFeatures,
        FORMAT_VERSION,
        "pending political runtime inputs",
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
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::Provinces.index()] = appended_at(
        TableId::Provinces,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 7,
                name: "attack",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 8,
                name: "selected_areas",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::CountryWars.index()] = appended_at(
        TableId::CountryWars,
        FORMAT_VERSION,
        "pending selected route inputs",
        &[AppendedField {
            index: 3,
            name: "ordered",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::WeatherRegions.index()] = appended_at(
        TableId::WeatherRegions,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[AppendedField {
            index: 7,
            name: "attack",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::WeatherProvinces.index()] = table(
        TableId::WeatherProvinces,
        FORMAT_VERSION,
        "pending political runtime inputs",
    );
    history[TableId::StateFlags.index()] = table(TableId::StateFlags, 93, "9fb09a565f");
    // WING_NAME extended the meaning of the existing packed entry family.
    history[TableId::RaidTooltipOperands.index()] =
        table(TableId::RaidTooltipOperands, 94, "ccf740b81d");
    history[TableId::DivisionMovement.index()] = table(
        TableId::DivisionMovement,
        FORMAT_VERSION,
        "pending political runtime inputs",
    );
    history[TableId::CountryAppearance.index()] = table(
        TableId::CountryAppearance,
        EFFECT_INPUTS_FORMAT,
        "ffd3e03cc6",
    );
    history[TableId::States.index()] = appended_at(
        TableId::States,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 6,
                name: "continent",
                introduced: OCCUPATION_GROUP_LAWS_FORMAT,
            },
            AppendedField {
                index: 7,
                name: "railway_infrastructure",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::RailCooldowns.index()] = appended_at(
        TableId::RailCooldowns,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 1,
                name: "building",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 2,
                name: "selection",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 3,
                name: "selection_hour",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::ArmyOrderPlans.index()] = appended_at(
        TableId::ArmyOrderPlans,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 20,
                name: "child_fronts",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 21,
                name: "invasion_transports",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 22,
                name: "execution",
                introduced: FORMAT_VERSION,
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
    history[TableId::LandCombatDetails.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::LandCombatDetails.index()].appended = &[AppendedField {
        index: 9,
        name: "border_war",
        introduced: 93,
    }];
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
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 7,
            name: "night_factor",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 8,
            name: "display",
            introduced: FORMAT_VERSION,
        },
    ];
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
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 16,
            name: "movement_priority",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 17,
            name: "order_transport",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 18,
            name: "fresh",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 19,
            name: "combat_references",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 20,
            name: "selected",
            introduced: FORMAT_VERSION,
        },
        AppendedField {
            index: 21,
            name: "transport",
            introduced: FORMAT_VERSION,
        },
    ];
    history[TableId::RelationLinks.index()] = appended_at(
        TableId::RelationLinks,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[AppendedField {
            index: 1,
            name: "transport_subject",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::CountryAppearance.index()].kind = ChangeKind::AppendedOnly;
    history[TableId::CountryAppearance.index()].format = FORMAT_VERSION;
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
            introduced: FORMAT_VERSION,
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
        WIDGET_CAPTURE_FORMAT,
        "wc/event-feed",
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
    history[TableId::Operations.index()] = appended_at(
        TableId::Operations,
        INTEL_INPUTS_FORMAT,
        "intel-native-union",
        &[
            AppendedField {
                index: 13,
                name: "bonus",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "detected",
                introduced: INTEL_INPUTS_FORMAT,
            },
            AppendedField {
                index: 15,
                name: "effect_description",
                introduced: INTEL_INPUTS_FORMAT,
            },
        ],
    );
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
        FORMAT_VERSION,
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
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::OrdersGroupCounterInputs.index()] = appended_at(
        TableId::OrdersGroupCounterInputs,
        FORMAT_VERSION,
        "pending political runtime inputs",
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
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::DivisionTemplateIcons.index()] = appended_at(
        TableId::DivisionTemplateIcons,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 13,
                name: "formation_division_count",
                introduced: EFFECT_INPUTS_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "display_name",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::CountryCounterInputs.index()] = appended_at(
        TableId::CountryCounterInputs,
        FORMAT_VERSION,
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
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 12,
                name: "zero_war",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 13,
                name: "movement",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 14,
                name: "zero_subject",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 15,
                name: "pride_name",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 16,
                name: "forward_visibility",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 17,
                name: "attack",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 18,
                name: "directed_wars",
                introduced: FORMAT_VERSION,
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
        FORMAT_VERSION,
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
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 17,
                name: "raid_reference",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 18,
                name: "strategic_bombing_raw",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::AirMissionInputs.index()] = appended_at(
        TableId::AirMissionInputs,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[AppendedField {
            index: 6,
            name: "cas_ratio_raw",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::AirNavalPatrolHistories.index()] = appended_at(
        TableId::AirNavalPatrolHistories,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[AppendedField {
            index: 2,
            name: "scenario_channels",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::ShipModels.index()] = appended_at(
        TableId::ShipModels,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 6,
                name: "subunit_index",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 7,
                name: "strike",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 8,
                name: "hull_variant",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::TaskForceCounterInputs.index()] = appended_at(
        TableId::TaskForceCounterInputs,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 10,
                name: "name",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 11,
                name: "supply",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 12,
                name: "port_mission",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::RailwayGunCounterInputs.index()] = appended_at(
        TableId::RailwayGunCounterInputs,
        FORMAT_VERSION,
        "pending political runtime inputs",
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
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 27,
                name: "fresh",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 28,
                name: "combat_references",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 29,
                name: "transport",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::TaskForceMapMotion.index()] = appended_at(
        TableId::TaskForceMapMotion,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 4,
                name: "movement_numerator",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 5,
                name: "retreat",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 6,
                name: "withdrawal",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 7,
                name: "naval_transfer",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 8,
                name: "combat_links",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 9,
                name: "fresh",
                introduced: FORMAT_VERSION,
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
        FORMAT_VERSION,
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
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 15,
                name: "movement",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 16,
                name: "land_theatre_count",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 17,
                name: "launch_sources",
                // The native-null naval discriminator preserves the original
                // unavailable/list bytes and this historical field boundary.
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 18,
                name: "reverse_visibility",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 19,
                name: "country_intel_sample",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 20,
                name: "attack",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 21,
                name: "raw_country_observations",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 22,
                name: "transport_mission_inputs",
                introduced: FORMAT_VERSION,
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
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[AppendedField {
            index: 5,
            name: "carrier",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::AirBaseRegistrations.index()] = appended_at(
        TableId::AirBaseRegistrations,
        FORMAT_VERSION,
        "pending political runtime inputs",
        &[
            AppendedField {
                index: 2,
                name: "carrier",
                introduced: FORMAT_VERSION,
            },
            AppendedField {
                index: 3,
                name: "transport_planes",
                introduced: FORMAT_VERSION,
            },
        ],
    );
    history[TableId::CountryProductionModifiers.index()] = table(
        TableId::CountryProductionModifiers,
        PRODUCTION_MODIFIERS_FORMAT,
        "production-modifiers",
    );
    history[TableId::CountryProductionModifiers.index()].introduced = PRODUCTION_MODIFIERS_FORMAT;
    history[TableId::ProductionLineModifiers.index()] = table(
        TableId::ProductionLineModifiers,
        PRODUCTION_MODIFIERS_FORMAT,
        "production-modifiers",
    );
    history[TableId::ProductionLineModifiers.index()].introduced = PRODUCTION_MODIFIERS_FORMAT;
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
    history[TableId::VariantDefinitions.index()] = appended(
        TableId::VariantDefinitions,
        "9d09404c0d",
        &[AppendedField {
            index: 14,
            name: "version",
            introduced: RECORDER_INPUTS_FORMAT,
        }],
    );
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
        &[AppendedField {
            index: 17,
            name: "NavalCommanderInputs.timed_traits",
            introduced: RECORDER_INPUTS_FORMAT,
        }];
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].introduced = PROVEN_FROM;
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].kind =
        ChangeKind::AppendedOnly;
    history[TableId::ALL.len() + RecordKind::LandBorderWarInputs as usize].appended =
        &[AppendedField {
            index: 2,
            name: "LandBorderWarInputs.combat_width",
            introduced: RECORDER_INPUTS_FORMAT,
        }];
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
        FORMAT_VERSION,
        "naval base serialized country tag witnesses (provisional)",
        &[AppendedField {
            index: 5,
            name: "country_tags",
            introduced: FORMAT_VERSION,
        }],
    );
    history[TableId::ConstructionLineOutput.index()] = appended_at(
        TableId::ConstructionLineOutput,
        FORMAT_VERSION,
        "construction completion and count-reset witnesses (provisional)",
        &[
            AppendedField {
                index: 13,
                name: "completion",
                introduced: CONSTRUCTION_COMPLETION_FORMAT,
            },
            AppendedField {
                index: 14,
                name: "count_reset",
                introduced: FORMAT_VERSION,
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
    // Signed net rows preserve every historical positive amount. Missing new
    // columns remain field gaps; correctly decoded old prefixes still open.
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
    history[TableId::LandCombatResults.index()] = table(
        TableId::LandCombatResults,
        WIDGET_CAPTURE_FORMAT,
        "91cbbe55",
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
    history
};

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
    }
}

impl LastChange {
    pub const fn present(&self, format: u16) -> bool {
        format >= self.introduced
            && match self.retired {
                Some(retired) => format < retired,
                None => true,
            }
    }

    pub(crate) fn table_gap(&self, table: TableId, format: u16) -> Option<String> {
        if format < self.introduced {
            Some(format!(
                "{table:?} not recorded at format {format} (introduced at format {})",
                self.introduced
            ))
        } else if let Some(retired) = self
            .retired
            .filter(|retired| *retired <= crate::FORMAT_VERSION)
        {
            Some(format!(
                "{table:?} retired at format {retired}; this recording is format {format}"
            ))
        } else if format < self.format && self.kind != ChangeKind::AppendedOnly {
            Some(format!(
                "{table:?} layout or meaning changed at format {}; this recording is format {format}",
                self.format
            ))
        } else {
            None
        }
    }

    pub(crate) fn fields(&self, table: TableId, format: u16) -> usize {
        self.appended
            .iter()
            .find(|field| format < field.introduced)
            .map_or(table.field_count(), |field| field.index)
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
                    .is_none_or(|retired| retired > entry.introduced)
            );
            if entry.kind == ChangeKind::AppendedOnly {
                assert!(!entry.appended.is_empty());
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(field.introduced > PROVEN_FROM && field.introduced <= entry.format);
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
                _ => None,
            };
            if let Some(fields) = fields {
                assert_eq!(entry.introduced, PROVEN_FROM);
                assert_eq!(entry.kind, ChangeKind::AppendedOnly);
                let first = entry.appended[0].index;
                for (offset, field) in entry.appended.iter().enumerate() {
                    assert_eq!(field.index, first + offset);
                    assert!(field.name.contains('.'));
                    assert!(field.introduced > PROVEN_FROM && field.introduced <= entry.format);
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
