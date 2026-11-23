
use crate::{
    frame::DecodedSource,
    rows::{TableId, tables},
};
use engine_executable::modifier_keys as keys;
use engine_native_parts::cbuilding_database;
use engine_native_memory::cpdx_lexer;
use engine_research::cresearch_slot;

#[derive(Clone, Copy, Debug)]
pub struct Family {
    pub name: &'static str,
    pub keys: &'static [u32],
    pub tokens: &'static [u32],
    pub tables: &'static [TableId],
}

pub const MOVEMENT: Family = Family {
    name: "division movement",
    keys: &[
        keys::ARMY_SPEED_FACTOR,
        keys::ARMY_ARMOR_SPEED_FACTOR,
        keys::ARMY_SPEED_FACTOR_FOR_CONTROLLER,
        keys::ENEMY_ARMY_SPEED_FACTOR,
        keys::ARMY_ATTACK_SPEED_FACTOR,
        keys::DISABLE_STRATEGIC_REDEPLOYMENT,
        keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER,
        keys::ARMY_RETREAT_SPEED_FACTOR,
        keys::RIVER_CROSSING_FACTOR,
        keys::RIVER_CROSSING_FACTOR_AGAINST,
        keys::OUT_OF_SUPPLY_FACTOR,
        keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
    ],
    tokens: &[],
    tables: &[],
};

pub const STRATEGIC_ACTION: Family = Family {
    name: "division strategic action",
    keys: &[
        keys::DISABLE_STRATEGIC_REDEPLOYMENT,
        keys::DISABLE_STRATEGIC_REDEPLOYMENT_FOR_CONTROLLER,
    ],
    tokens: &[],
    tables: &[],
};

pub const NAVAL_TRANSFER: Family = Family {
    name: "naval transfer speed",
    keys: &[keys::NAVAL_SPEED_FACTOR, keys::NAVAL_MINES_EFFECT_REDUCTION],
    tokens: &[],
    tables: &[],
};

pub const TASK_FORCE_COUNTER_ETA: Family = Family {
    name: "fresh task-force counter ETA",
    keys: &[
        keys::NAVAL_SPEED_FACTOR,
        keys::NAVAL_RANGE,
        keys::NAVAL_RANGE_FACTOR,
        keys::NAVY_WEATHER_PENALTY,
        keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR,
    ],
    tokens: &[],
    tables: &[],
};

pub const AIR_MAP: Family = Family {
    name: "Air map detection and base modifiers",
    keys: &[
        keys::REGION_AIR_DETECTION,
        keys::AIR_INTERCEPT_DETECTION,
        keys::AIR_SUPERIORITY_DETECTION,
        keys::AIR_HOME_DEFENCE_FACTOR,
        keys::AIR_POWER_PROJECTION_FACTOR,
        keys::AIR_VOLUNTEER_CAPACITY,
        keys::AIR_STRATEGIC_BOMBER_NIGHT_PENALTY,
        keys::AIR_NIGHT_PENALTY,
        keys::AIR_WEATHER_PENALTY,
        keys::AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR,
        keys::AIR_FUEL_CONSUMPTION_FACTOR,
    ],
    tokens: &[],
    tables: &[],
};

pub const RAILWAY_SPEED: Family = Family {
    name: "railway gun speed",
    keys: &[],
    tokens: &[engine_railways::railway_gun_speed::MAXIMUM_SPEED_TOKEN],
    tables: &[],
};

pub const LAND_COMBAT_DIVISIONS: Family = Family {
    name: "land combat divisions",
    keys: &[
        keys::LAND_REINFORCE_RATE,
        keys::COMBAT_WIDTH_FACTOR,
        keys::INITIATIVE_FACTOR,
    ],
    tokens: &[],
    tables: &[TableId::LandCombatDivisions],
};

pub const DIVISION_STATUS: Family = Family {
    name: "division status",
    keys: &[
        keys::UNIT_UPKEEP_ATTRITION_FACTOR,
        keys::COMBAT_ENTRENCHMENT,
    ],
    tokens: &[],
    tables: &[TableId::DivisionStatus],
};

pub const ATTRITION: Family = Family {
    name: "fresh army attrition",
    keys: &engine_land_inputs::carmy_attrition::KEYS,
    tokens: &[],
    tables: &[],
};

pub const SELECTED_ATTACK_WEATHER_ATTACK: Family = Family {
    name: "selected attack weather attack",
    keys: &[keys::ARMY_ATTACK_FACTOR],
    tokens: &[],
    tables: &[],
};

pub const SELECTED_ATTACK_WEATHER_BREAKTHROUGH: Family = Family {
    name: "selected attack weather breakthrough",
    keys: &[keys::BREAKTHROUGH_FACTOR],
    tokens: &[],
    tables: &[],
};

pub const SELECTED_ATTACK_NIGHT: Family = Family {
    name: "selected attack night",
    keys: &[keys::LAND_NIGHT_ATTACK],
    tokens: &[],
    tables: &[],
};

pub const SELECTED_ATTACK_FORT: Family = Family {
    name: "selected attack fort",
    keys: &[keys::LAND_BUNKER_EFFECTIVENESS_FACTOR],
    tokens: &[],
    tables: &[],
};

pub const RETREAT_BANS: Family = Family {
    name: "commander retreat bans",
    keys: &[
        keys::CANNOT_RETREAT_WHILE_DEFENDING,
        keys::CANNOT_RETREAT_WHILE_ATTACKING,
    ],
    tokens: &[],
    tables: &[TableId::DivisionRetreatBans],
};

pub const SHIP_ORGANISATION: Family = Family {
    name: "ship organisation",
    keys: &[
        keys::NAVY_ORG,
        keys::NAVY_ORG_FACTOR,
        keys::MAX_ORGANISATION,
        keys::NAVAL_CRITICAL_EFFECT_FACTOR,
        keys::NAVY_WEATHER_PENALTY,
    ],
    tokens: &[],
    tables: &[TableId::ShipModels],
};

pub const NAVAL_WEAPONS: Family = Family {
    name: "naval weapon modifiers",
    keys: &[
        keys::NAVY_WEATHER_PENALTY,
        keys::NAVAL_CRITICAL_EFFECT_FACTOR,
        keys::SUBMARINE_ATTACK,
        keys::NAVY_SUBMARINE_ATTACK_FACTOR,
        keys::NAVY_SUBMARINE_DEFENCE_FACTOR,
        keys::NAVY_CAPITAL_SHIP_ATTACK_FACTOR,
        keys::NAVY_CAPITAL_SHIP_DEFENCE_FACTOR,
        keys::NAVY_SCREEN_ATTACK_FACTOR,
        keys::NAVY_SCREEN_DEFENCE_FACTOR,
        keys::NAVY_CAPITAL_SHIP_ATTACK_FACTOR_AGAINST,
        keys::NAVY_CAPITAL_SHIP_DEFENCE_FACTOR_AGAINST,
        keys::NAVY_SCREEN_ATTACK_FACTOR_AGAINST,
        keys::NAVY_SCREEN_DEFENCE_FACTOR_AGAINST,
        keys::NAVY_SUBMARINE_ATTACK_FACTOR_AGAINST,
        keys::NAVY_SUBMARINE_DEFENCE_FACTOR_AGAINST,
        keys::NAVAL_NIGHT_ATTACK,
        keys::NAVAL_DAMAGE_FACTOR,
        keys::NAVAL_DEFENSE_FACTOR,
        keys::NAVAL_HAS_POTF_IN_COMBAT_ATTACK,
        keys::NAVAL_HAS_POTF_IN_COMBAT_DEFENSE,
    ],
    tokens: &[],
    tables: &[],
};

pub const NAVAL_SUMMARY: Family = Family {
    name: "naval combat summary",
    keys: &[
        keys::NAVAL_ENEMY_POSITIONING_IN_INITIAL_ATTACK,
        keys::SCREENING_EFFICIENCY,
        keys::POSITIONING,
        keys::SCREENING_WITHOUT_SCREENS,
        keys::NAVAL_ENEMY_FLEET_SIZE_RATIO_PENALTY_FACTOR,
    ],
    tokens: &[],
    tables: &[TableId::NavalCombatDetails],
};

pub const NAVAL_AIR: Family = Family {
    name: "naval air efficiency",
    keys: &[
        keys::CARRIER_TRAFFIC,
        keys::CARRIER_NIGHT_TRAFFIC,
        keys::SORTIE_EFFICIENCY,
        keys::FIGHTER_SORTIE_EFFICIENCY,
    ],
    tokens: &[],
    tables: &[TableId::NavalCombatDetails],
};

pub const NAVAL_TREES: Family = Family {
    name: "naval modifier trees",
    keys: &engine_naval_history::naval_modifier_trees::KEYS,
    tokens: &[],
    tables: &[
        TableId::NavalModifierTrees,
        TableId::NavalModifierTreeSamples,
    ],
};

pub const NAVAL_POSITIONING: Family = Family {
    name: "naval positioning modifiers",
    keys: &[
        keys::NAVAL_DETECTION,
        keys::SORTIE_EFFICIENCY,
        keys::FIGHTER_SORTIE_EFFICIENCY,
        keys::NAVY_WEATHER_PENALTY,
        keys::NAVAL_CRITICAL_EFFECT_FACTOR,
    ],
    tokens: &[],
    tables: &[],
};

pub const NAVAL_ANTI_AIR: Family = Family {
    name: "naval anti-air modifiers",
    keys: &[
        keys::NAVY_ANTI_AIR_ATTACK,
        keys::NAVY_ANTI_AIR_ATTACK_FACTOR,
        keys::NAVY_WEATHER_PENALTY,
        keys::NAVAL_CRITICAL_EFFECT_FACTOR,
    ],
    tokens: &[],
    tables: &[],
};

pub const RESEARCH: Family = Family {
    name: "research modifiers",
    keys: &[
        keys::RESEARCH_SPEED_FACTOR,
        keys::RESEARCH_SHARING_PER_COUNTRY_BONUS_FACTOR,
        keys::SCIENTIST_RESEARCH_BONUS_FACTOR,
    ],
    tokens: &[],
    tables: &[TableId::ResearchModifiers],
};

pub const COUNTRY_VISIBILITY: Family = Family {
    name: "country visibility",
    keys: &[keys::INTEL_FROM_COMBAT_FACTOR],
    tokens: &[],
    tables: &[TableId::CountryVisibility],
};

pub const STATE_VISIBILITY: Family = Family {
    name: "state visibility",
    keys: &[keys::LOCAL_INTEL_TO_ENEMIES],
    tokens: &[],
    tables: &[TableId::StateVisibility],
};

pub const POWER_BALANCE: Family = Family {
    name: "power balance modifiers",
    keys: &[keys::POWER_BALANCE_DAILY, keys::POWER_BALANCE_WEEKLY],
    tokens: &[],
    tables: &[
        TableId::PowerBalanceMembers,
        TableId::PowerBalanceModifierBreakdowns,
    ],
};

pub const STATE_RESOURCES: Family = Family {
    name: "state resources",
    keys: &[
        keys::STATE_RESOURCES_FACTOR,
        keys::LOCAL_RESOURCE_GAIN_EFFICIENCY_PER_INFRASTRUCTURE,
    ],
    tokens: &[],
    tables: &[TableId::StateResourceInputs],
};

pub const COUNTRY_RECRUITMENT: Family = Family {
    name: "country recruitment",
    keys: &engine_native::ccountry::RECRUITMENT_MODIFIER_KEYS,
    tokens: &[],
    tables: &[TableId::CountryCounterInputs],
};

pub const COUNTRY_AIR_COMBAT: Family = Family {
    name: "country air combat modifiers",
    keys: &engine_native::ccountry::AIR_COMBAT_MODIFIER_KEYS,
    tokens: &[],
    tables: &[TableId::CountryCounterInputs],
};

pub const STATE_RECRUITMENT: Family = Family {
    name: "state recruitment",
    keys: &[
        keys::LOCAL_MANPOWER,
        keys::LOCAL_NON_CORE_MANPOWER,
        keys::RECRUITABLE_POPULATION,
        keys::RECRUITABLE_POPULATION_FACTOR,
    ],
    tokens: &[],
    tables: &[TableId::StateResourceInputs],
};

pub const RAID_LAUNCH: Family = Family {
    name: "raid launch",
    keys: &[keys::AIR_RANGE_FACTOR, keys::NAVAL_COMMANDO_RAID_DISTANCE],
    tokens: &[],
    tables: &[TableId::RaidCountryInputs, TableId::AceRangeInputs],
};

pub const COMMAND_POWER: Family = Family {
    name: "command power",
    keys: &[keys::MAX_COMMAND_POWER, keys::MAX_COMMAND_POWER_MULT],
    tokens: &[],
    tables: &[TableId::CountryCommandPower],
};

pub const COUNTRY_ACE_MODIFIERS: Family = Family {
    name: "country ace modifiers",
    keys: &[keys::AIR_ACE_BONUSES_FACTOR, keys::ACE_EFFECTIVENESS_FACTOR],
    tokens: &[],
    tables: &[TableId::CountryCommandPower],
};

pub const RAID_UNIT_SPEED: Family = Family {
    name: "raid unit speed",
    keys: &[keys::AIR_MAXIMUM_SPEED_FACTOR],
    tokens: &[],
    tables: &[TableId::RaidSpeedInputs],
};

pub const INVASION_PREPARATION: Family = Family {
    name: "invasion preparation",
    keys: &[
        keys::INVASION_PREPARATION,
        keys::NAVAL_INVASION_PREP_DAYS,
        keys::INVASION_PREPARATION_AGAINST,
    ],
    tokens: &[],
    tables: &[TableId::InvasionPreparations],
};

pub const PARADROP_PREPARATION: Family = Family {
    name: "paradrop preparation",
    keys: &[keys::AIR_INVASION_PREPARATION, keys::AIR_INVASION_PREP_DAYS],
    tokens: &[],
    tables: &[],
};

pub const PARADROP_TRANSPORT: Family = Family {
    name: "paradrop transport weight",
    keys: &[keys::PARATROOPER_WEIGHT_FACTOR],
    tokens: &[],
    tables: &[],
};

const NAVAL_MISSION_KEYS: [u32; engine_native_naval::naval_mission_inputs::COUNTRY_KEYS.len() + 1] = {
    let mut out = [0; engine_native_naval::naval_mission_inputs::COUNTRY_KEYS.len() + 1];
    let mut index = 0;
    while index < engine_native_naval::naval_mission_inputs::COUNTRY_KEYS.len() {
        out[index] = engine_native_naval::naval_mission_inputs::COUNTRY_KEYS[index];
        index += 1;
    }
    out[index] = keys::SPOTTING_CHANCE_AGAINST;
    out
};

pub const NAVAL_MISSIONS: Family = Family {
    name: "naval mission active counts and efficiencies",
    keys: &NAVAL_MISSION_KEYS,
    tokens: &[],
    tables: &[
        TableId::NavalMissionTaskForces,
        TableId::NavalMissionLedgers,
        TableId::NavalMissionCountries,
        TableId::NavalMissionCommanders,
    ],
};

pub const RESEARCH_IDLE: Family = Family {
    name: "research slots",
    keys: &[],
    tokens: &[cresearch_slot::IDLE_HANDLE],
    tables: &[
        TableId::ResearchSlots,
        TableId::ResearchInputs,
        TableId::ResearchAppliedBonuses,
        TableId::ResearchModifiers,
    ],
};

pub const ORACLE_FIXED15: Family = Family {
    name: "save oracle fixed15 tokens",
    keys: &[],
    tokens: &[cpdx_lexer::FIXED15_TOKEN],
    tables: &[],
};

pub const RAID_SUCCESS: Family = Family {
    name: "raid success operands",
    keys: &[
        keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR,
        keys::AIR_DEFENCE_FACTOR,
        keys::AIR_AGILITY_FACTOR,
        keys::AIR_HOME_DEFENCE_FACTOR,
        keys::AIR_POWER_PROJECTION_FACTOR,
        keys::AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR,
        keys::AIR_STRATEGIC_BOMBER_NIGHT_PENALTY,
        keys::AIR_NIGHT_PENALTY,
        keys::AIR_WEATHER_PENALTY,
        keys::AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR,
        keys::AIR_ACE_BONUSES_FACTOR,
        keys::ACE_EFFECTIVENESS_FACTOR,
    ],
    tokens: &[
        engine_native_air::raid_tooltip_operands::NUCLEAR_BOMB_TOKEN,
        engine_native_air::raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN,
    ],
    tables: &[TableId::RaidTooltipOperands],
};

pub const RAID_INTEL: Family = Family {
    name: "raid target intel",
    keys: &[keys::INTELLIGENCE_AGENCY_DEFENSE, keys::CRYPTO_STRENGTH],
    tokens: &[],
    tables: &[TableId::RaidIntelInputs],
};

pub const ORACLE_INFRASTRUCTURE: Family = Family {
    name: "save oracle infrastructure",
    keys: &[],
    tokens: &[cbuilding_database::INFRASTRUCTURE_TOKEN],
    tables: &[],
};

pub const PRODUCTION_MODIFIERS: Family = Family {
    name: "production modifier hovers",
    keys: &engine_production::production_modifiers::KEYS,
    tokens: &[],
    tables: &[
        TableId::CountryProductionModifiers,
        TableId::ProductionLineModifiers,
    ],
};

pub const DESIGN_ORGANISATION_CHANGE: Family = Family {
    name: "design organisation change cost",
    keys: &[keys::MILITARY_INDUSTRIAL_ORGANIZATION_DESIGN_TEAM_CHANGE_COST],
    tokens: &[],
    tables: &[TableId::DesignCountryModifiers],
};

pub const ORGANISATION_POLICY_COST: Family = Family {
    name: "organisation policy cost",
    keys: &[keys::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST],
    tokens: &[],
    tables: &[TableId::IndustrialOrganisationPolicies],
};

pub const FAMILIES: &[Family] = &[
    PRODUCTION_MODIFIERS,
    DESIGN_ORGANISATION_CHANGE,
    ORGANISATION_POLICY_COST,
    MOVEMENT,
    STRATEGIC_ACTION,
    NAVAL_TRANSFER,
    TASK_FORCE_COUNTER_ETA,
    AIR_MAP,
    RAILWAY_SPEED,
    LAND_COMBAT_DIVISIONS,
    DIVISION_STATUS,
    ATTRITION,
    SELECTED_ATTACK_WEATHER_ATTACK,
    SELECTED_ATTACK_WEATHER_BREAKTHROUGH,
    SELECTED_ATTACK_NIGHT,
    SELECTED_ATTACK_FORT,
    RETREAT_BANS,
    SHIP_ORGANISATION,
    NAVAL_WEAPONS,
    NAVAL_SUMMARY,
    NAVAL_AIR,
    NAVAL_TREES,
    NAVAL_POSITIONING,
    NAVAL_ANTI_AIR,
    RESEARCH,
    COUNTRY_VISIBILITY,
    STATE_VISIBILITY,
    POWER_BALANCE,
    STATE_RESOURCES,
    COUNTRY_RECRUITMENT,
    COUNTRY_AIR_COMBAT,
    STATE_RECRUITMENT,
    RAID_LAUNCH,
    COMMAND_POWER,
    COUNTRY_ACE_MODIFIERS,
    RAID_UNIT_SPEED,
    RAID_INTEL,
    RAID_SUCCESS,
    INVASION_PREPARATION,
    PARADROP_PREPARATION,
    PARADROP_TRANSPORT,
    NAVAL_MISSIONS,
    RESEARCH_IDLE,
    ORACLE_FIXED15,
    ORACLE_INFRASTRUCTURE,
];

pub fn validate_recording(frame: &dyn DecodedSource, family: Family) -> Result<(), String> {
    let definitions = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string());
    let tokens = frame.lexical().map_err(|e| e.to_string());
    let whole = match (&definitions, &tokens) {
        (Err(error), _) | (_, Err(error)) => Err(error.clone()),
        _ => Ok(()),
    };
    validate_definitions(
        family,
        whole,
        |index| {
            let definitions = definitions.as_ref().ok()?;
            let at = definitions.binary_search_by_key(&index, |row| row.index).ok()?;
            Some(definitions[at].token)
        },
        |token, name| {
            let tokens = tokens.as_ref().ok()?;
            let at = tokens.binary_search_by_key(&token, |row| row.token).ok()?;
            Some(tokens[at].literal == name)
        },
    )
}

pub fn validate_definitions(
    family: Family,
    whole: Result<(), String>,
    token: impl Fn(u32) -> Option<u32>,
    spells: impl Fn(u32, &[u8]) -> Option<bool>,
) -> Result<(), String> {
    let validate = || -> Result<(), String> {
        if !engine_executable::executable_tables::is_initialized() {
            return Err("executable metadata unavailable; choose the game folder".into());
        }
        whole?;
        let gap = engine_fixed_layout::fixed_layout::GAP;
        if token(gap).is_some() {
            return Err(format!("modifier definition gap ({gap:#x}) is registered"));
        }
        for key in keys::all()
            .map_err(|e| e.to_string())?
            .filter(|key| family.keys.contains(&key.index))
        {
            let failure = |reason| format!("{} ({:#x}): {reason}", key.name, key.index);
            let token = token(key.index).ok_or_else(|| failure("definition row missing"))?;
            match spells(token, key.name.as_bytes()) {
                None => return Err(failure("definition token missing from LexicalTokens")),
                Some(false) => return Err(failure("definition token spells a different script name")),
                Some(true) => {}
            }
        }
        Ok(())
    };
    validate().map_err(|reason| format!("{} refused: {reason}", family.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        frame::{FrameDecoder, FrameEncoder, Metrics, TableView},
        rows::{LexicalTokenRow, ModifierDefinitionRow, TableRows},
    };

    use crate::frame::DecodedFrame;

    fn fixture() -> DecodedFrame {
        synthetic_game::enter_fixed_modifiers();
        let mut keys = keys::all().unwrap().collect::<Vec<_>>();
        keys.sort_unstable_by_key(|key| key.index);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture has no sample".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(
                keys.iter()
                    .map(|key| ModifierDefinitionRow {
                        index: key.index,
                        token: key.index + 1,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                keys.iter()
                    .map(|key| LexicalTokenRow {
                        token: key.index + 1,
                        literal: key.name.as_bytes().to_vec(),
                    })
                    .collect(),
            ));
        frame
    }

    #[test]
    fn strategic_action_fixed_layout_checks_only_its_two_native_keys() {
        assert!(STRATEGIC_ACTION.tables.is_empty());
        assert_eq!(STRATEGIC_ACTION.keys.len(), 2);
        let mut frame = fixture();
        let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            panic!("definitions");
        };
        rows.iter_mut()
            .find(|row| row.index == keys::ARMY_SPEED_FACTOR)
            .unwrap()
            .token = 0;
        assert!(validate_recording(&frame, MOVEMENT).is_err());
        assert_eq!(validate_recording(&frame, STRATEGIC_ACTION), Ok(()));
        for key in STRATEGIC_ACTION.keys {
            let mut malformed = fixture();
            let TableView::Complete(TableRows::ModifierDefinitions(rows)) =
                &mut *malformed.tables[TableId::ModifierDefinitions.index()]
            else {
                panic!("definitions");
            };
            rows.iter_mut().find(|row| row.index == *key).unwrap().token = 0;
            assert!(
                validate_recording(&malformed, STRATEGIC_ACTION)
                    .unwrap_err()
                    .contains("division strategic action refused")
            );
        }
    }

    #[test]
    fn every_family_contract_survives_recording_round_trip() {
        let frame = fixture();
        let encoded = FrameEncoder::new().encode(frame, true).unwrap();
        let decoded = FrameDecoder::new().apply(&encoded).unwrap();
        for &family in FAMILIES {
            assert_eq!(
                validate_recording(&decoded, family),
                Ok(()),
                "{}",
                family.name
            );
        }
    }

    #[test]
    fn families_cover_exactly_the_checked_literals() {
        for key in keys::ALL_INDICES {
            assert!(
                FAMILIES.iter().any(|family| family.keys.contains(key)),
                "uncovered key {key:#x}"
            );
        }
        for token in engine_fixed_layout::fixed_layout::TOKEN_IDS {
            assert!(
                FAMILIES.iter().any(|family| family.tokens.contains(token)),
                "uncovered token {token:#x}"
            );
        }
        for family in FAMILIES {
            for index in family.keys {
                assert!(keys::ALL_INDICES.contains(index));
            }
            for id in family.tokens {
                assert!(engine_fixed_layout::fixed_layout::TOKEN_IDS.contains(id));
            }
        }
    }

    #[test]
    fn mismatch_fixture_refuses_each_viewer_literal_by_name() {
        for family in [
            TASK_FORCE_COUNTER_ETA,
            PRODUCTION_MODIFIERS,
            NAVAL_WEAPONS,
            SHIP_ORGANISATION,
            NAVAL_POSITIONING,
            NAVAL_ANTI_AIR,
            NAVAL_TREES,
            PARADROP_PREPARATION,
            PARADROP_TRANSPORT,
            COUNTRY_RECRUITMENT,
            STATE_RECRUITMENT,
        ] {
            for &key in family.keys {
                let mut frame = fixture();
                let TableView::Complete(TableRows::LexicalTokens(tokens)) =
                    &mut *frame.tables[TableId::LexicalTokens.index()]
                else {
                    panic!()
                };
                tokens
                    .iter_mut()
                    .find(|token| token.token == key + 1)
                    .unwrap()
                    .literal = b"renumbered_by_patch".to_vec();
                let refusal = validate_recording(&frame, family).unwrap_err();
                assert!(refusal.contains(family.name));
                assert!(refusal.contains(keys::fixed_name(key).unwrap()));
                assert!(refusal.contains("different script name"));
            }
        }
    }

    #[test]
    fn naval_tree_family_covers_all_capture_keys_and_its_tables() {
        assert_eq!(
            NAVAL_TREES.keys,
            engine_naval_history::naval_modifier_trees::KEYS
        );
        assert_eq!(
            NAVAL_TREES.tables,
            [
                TableId::NavalModifierTrees,
                TableId::NavalModifierTreeSamples
            ]
        );
        for key in NAVAL_TREES.keys {
            assert!(keys::ALL_INDICES.contains(key));
        }
    }

    #[test]
    fn recruitment_families_cover_all_eight_keys_and_their_capture_tables() {
        assert_eq!(
            COUNTRY_RECRUITMENT.keys,
            [
                keys::MOBILIZATION_SPEED,
                keys::CONSCRIPTION,
                keys::CONSCRIPTION_FACTOR,
                keys::NON_CORE_MANPOWER,
            ]
        );
        assert_eq!(COUNTRY_RECRUITMENT.tables, [TableId::CountryCounterInputs]);
        assert_eq!(
            STATE_RECRUITMENT.keys,
            [
                keys::LOCAL_MANPOWER,
                keys::LOCAL_NON_CORE_MANPOWER,
                keys::RECRUITABLE_POPULATION,
                keys::RECRUITABLE_POPULATION_FACTOR,
            ]
        );
        assert_eq!(STATE_RECRUITMENT.tables, [TableId::StateResourceInputs]);
        for family in [COUNTRY_RECRUITMENT, STATE_RECRUITMENT] {
            assert!(FAMILIES.iter().any(|entry| entry.name == family.name));
        }
    }

    #[test]
    fn absent_or_partial_tables_and_missing_definition_or_token_refuse() {
        for table in [TableId::ModifierDefinitions, TableId::LexicalTokens] {
            for view in [
                TableView::Unavailable {
                    reason: "not in recording".into(),
                },
                TableView::Truncated {
                    rows: TableRows::empty(table),
                    missing: 1,
                },
                TableView::Complete(TableRows::empty(table)),
            ] {
                let mut frame = fixture();
                *frame.tables[table.index()] = view;
                assert!(
                    validate_recording(&frame, NAVAL_WEAPONS)
                        .unwrap_err()
                        .contains(NAVAL_WEAPONS.name)
                );
            }
        }
    }

    #[test]
    fn registered_gap_refuses_but_dynamic_definitions_are_allowed() {
        let mut frame = fixture();
        let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            panic!()
        };
        definitions.push(ModifierDefinitionRow {
            index: engine_fixed_layout::fixed_layout::GAP + 1,
            token: 999,
            ..Default::default()
        });
        assert_eq!(validate_recording(&frame, NAVAL_WEAPONS), Ok(()));
        let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            panic!()
        };
        definitions.last_mut().unwrap().index = engine_fixed_layout::fixed_layout::GAP;
        assert!(
            validate_recording(&frame, NAVAL_WEAPONS)
                .unwrap_err()
                .contains("gap (0x29a) is registered")
        );
    }

    #[test]
    fn retired_transport_families_are_absent_while_shared_keys_stay_validated() {
        for retired in [
            "selected transport risk",
            "selected transport air normalization",
        ] {
            assert!(
                FAMILIES.iter().all(|family| family.name != retired),
                "retired family still active: {retired}"
            );
        }

        let mut spotting_mismatch = fixture();
        let TableView::Complete(TableRows::LexicalTokens(tokens)) =
            &mut *spotting_mismatch.tables[TableId::LexicalTokens.index()]
        else {
            panic!()
        };
        tokens
            .iter_mut()
            .find(|token| token.token == keys::SPOTTING_CHANCE_AGAINST + 1)
            .unwrap()
            .literal = b"renumbered_by_patch".to_vec();
        let refusal = validate_recording(&spotting_mismatch, NAVAL_MISSIONS).unwrap_err();
        assert!(refusal.contains(NAVAL_MISSIONS.name));
        assert!(refusal.contains(keys::fixed_name(keys::SPOTTING_CHANCE_AGAINST).unwrap()));

        for &key in &engine_native::ccountry::AIR_COMBAT_MODIFIER_KEYS {
            let mut air_mismatch = fixture();
            let TableView::Complete(TableRows::LexicalTokens(tokens)) =
                &mut *air_mismatch.tables[TableId::LexicalTokens.index()]
            else {
                panic!()
            };
            tokens
                .iter_mut()
                .find(|token| token.token == key + 1)
                .unwrap()
                .literal = b"renumbered_by_patch".to_vec();
            let owner = FAMILIES
                .iter()
                .copied()
                .find(|family| {
                    family.tables.contains(&TableId::CountryCounterInputs)
                        && family.keys.contains(&key)
                })
                .expect("air combat modifier key must belong to an active CountryCounterInputs family");
            let refusal = validate_recording(&air_mismatch, owner).unwrap_err();
            assert!(refusal.contains(owner.name));
            assert!(refusal.contains(keys::fixed_name(key).unwrap()));
        }
    }
}
