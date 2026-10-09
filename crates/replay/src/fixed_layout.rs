//! Consumers of the fixed literals in `engine::modifier_keys` and
//! `engine::fixed_layout::TOKENS`. Recording refuses a family's tables when
//! the built-in modifier layout or one of its tokens fails; playback checks
//! its own literals against the recording. Dynamic modifier keys are not
//! constrained.

use crate::{
    frame::DecodedFrame,
    rows::{TableId, tables},
};
use engine::{cbuilding_database, cpdx_lexer, cresearch_slot, modifier_keys as keys};

#[derive(Clone, Copy, Debug)]
pub struct Family {
    pub name: &'static str,
    /// Built-in modifier indices.
    pub keys: &'static [u32],
    /// Static lexer token ids.
    pub tokens: &'static [u32],
    /// Recorded tables the collector computes with them; empty for field-local
    /// capture guards or when only playback or the save oracle uses them.
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
    tables: &[TableId::DivisionEtaInputs],
};

/// Current-hour class-0 counter action operands, `140C79B70`.
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
    tables: &[TableId::DivisionEtaInputs],
};

/// `141927730` reads the current variant's stat 0x3E through this token.
/// Its guard withholds only a railway gun's ETA suffix, preserving the row.
pub const RAILWAY_SPEED: Family = Family {
    name: "railway gun speed",
    keys: &[],
    tokens: &[engine::railway_gun_speed::MAXIMUM_SPEED_TOKEN],
    tables: &[],
};

pub const LAND_COMBAT_DIVISIONS: Family = Family {
    name: "land combat divisions",
    keys: &[
        keys::ARMY_SPEED_FACTOR,
        keys::ARMY_ATTACK_SPEED_FACTOR,
        keys::ARMY_SPEED_FACTOR_FOR_CONTROLLER,
        keys::ENEMY_ARMY_SPEED_FACTOR,
        keys::ARMY_RETREAT_SPEED_FACTOR,
        keys::OUT_OF_SUPPLY_FACTOR,
        keys::SPECIAL_FORCES_OUT_OF_SUPPLY_FACTOR,
        keys::NAVAL_SPEED_FACTOR,
        keys::NAVAL_MINES_EFFECT_REDUCTION,
        keys::LAND_REINFORCE_RATE,
        keys::COMBAT_WIDTH_FACTOR,
        keys::INITIATIVE_FACTOR,
    ],
    tokens: &[],
    tables: &[
        TableId::LandCombatDivisions,
        TableId::ReserveModifierBreakdowns,
    ],
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
    keys: &engine::carmy_attrition::KEYS,
    tokens: &[],
    tables: &[
        TableId::DivisionStatus,
        TableId::AttritionModifierBreakdowns,
    ],
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

/// 14160A7B0/141609460's side trees and 14195C7D0/141963700's
/// ship/carrier trees, selected by the hourly reader's native hover keys.
pub const NAVAL_BREAKDOWNS: Family = Family {
    name: "naval modifier breakdowns",
    keys: &engine::naval_ship_modifier_breakdown::KEYS,
    tokens: &[],
    tables: &[TableId::NavalModifierBreakdowns],
};

/// The combat window's positioning recomputation: surface detection and the
/// carrier sortie keys, read through the ship modifier port of 140C29020,
/// which also reads the weather and critical keys.
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

/// The combat window's anti-air recomputation, through the same ship
/// modifier port.
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

/// The raid launch gate's daily range getters: `GetModifiedMapRange@140F49440`
/// (key 5 on the country and, in AceRangeInputs, on the ace and its type) and
/// `GetRaidUnitRangeLimit@141453E10`.
pub const RAID_LAUNCH: Family = Family {
    name: "raid launch",
    keys: &[keys::AIR_RANGE_FACTOR, keys::NAVAL_COMMANDO_RAID_DISTANCE],
    tokens: &[],
    tables: &[TableId::RaidCountryInputs, TableId::AceRangeInputs],
};

/// `CCountry::CanAffordCommandPower@1406F0380`'s capacity pair.
pub const COMMAND_POWER: Family = Family {
    name: "command power",
    keys: &[keys::MAX_COMMAND_POWER, keys::MAX_COMMAND_POWER_MULT],
    tokens: &[],
    tables: &[TableId::CountryCommandPower],
};

/// The ace country modifiers every raid reads live, recorded hourly per
/// country beside the command power pair: 0x25a on an ace's owner
/// (`CAce::scaled_modifier@14060DDF0`) and 0x72 on a wing's country (the ace
/// terms of `GetModifiedMapRange@140F49440` and
/// `CAirWing::GetAceModifiedMaxSpeed@140F4B5E0`).
pub const COUNTRY_ACE_MODIFIERS: Family = Family {
    name: "country ace modifiers",
    keys: &[keys::AIR_ACE_BONUSES_FACTOR, keys::ACE_EFFECTIVENESS_FACTOR],
    tokens: &[],
    tables: &[TableId::CountryCommandPower],
};

/// `CalculateRaidUnitSpeed@141453880`'s ace speed terms: key 2 on the ace and
/// its type. Its country terms are [`COUNTRY_ACE_MODIFIERS`]'.
pub const RAID_UNIT_SPEED: Family = Family {
    name: "raid unit speed",
    keys: &[keys::AIR_MAXIMUM_SPEED_FACTOR],
    tokens: &[],
    tables: &[TableId::RaidSpeedInputs],
};

/// `COrderInstance::GetRequiredInvasionPreparationHours@141016930`'s naval
/// invasion terms: 0x1a on the country, the relation and the leader, 0x184
/// on the country, 0x1ce on the relation.
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

/// The naval mission icon's active counts and efficiencies (140FA2AC0,
/// 140F99450, 140F98DE0): combined task-force keys, mission-owner escort,
/// roster-owner weather and expeditionary/roster-owner underway range.
pub const NAVAL_MISSIONS: Family = Family {
    name: "naval mission active counts and efficiencies",
    keys: &engine::naval_mission_inputs::COUNTRY_KEYS,
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

/// The save oracle compares recorded tokens against fixed15 save text.
pub const ORACLE_FIXED15: Family = Family {
    name: "save oracle fixed15 tokens",
    keys: &[],
    tokens: &[cpdx_lexer::FIXED15_TOKEN],
    tables: &[],
};

/// Success operands read by 14157C670, 140F49710, 140F4D130.
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
        engine::raid_tooltip_operands::NUCLEAR_BOMB_TOKEN,
        engine::raid_tooltip_operands::THERMONUCLEAR_BOMB_TOKEN,
    ],
    tables: &[TableId::RaidTooltipOperands],
};

/// `1411DED90/140C07140` counterintelligence and `1413E3350` cryptology.
pub const RAID_INTEL: Family = Family {
    name: "raid target intel",
    keys: &[keys::INTELLIGENCE_AGENCY_DEFENSE, keys::CRYPTO_STRENGTH],
    tokens: &[],
    tables: &[TableId::RaidIntelInputs],
};

/// State-resource save comparison selects the engine's infrastructure token.
pub const ORACLE_INFRASTRUCTURE: Family = Family {
    name: "save oracle infrastructure",
    keys: &[],
    tokens: &[cbuilding_database::INFRASTRUCTURE_TOKEN],
    tables: &[],
};

pub const PRODUCTION_MODIFIERS: Family = Family {
    name: "production modifier hovers",
    keys: &engine::production_modifiers::KEYS,
    tokens: &[],
    tables: &[
        TableId::CountryProductionModifiers,
        TableId::ProductionLineModifiers,
    ],
};

pub const FAMILIES: &[Family] = &[
    PRODUCTION_MODIFIERS,
    MOVEMENT,
    STRATEGIC_ACTION,
    NAVAL_TRANSFER,
    RAILWAY_SPEED,
    LAND_COMBAT_DIVISIONS,
    DIVISION_STATUS,
    ATTRITION,
    RETREAT_BANS,
    SHIP_ORGANISATION,
    NAVAL_WEAPONS,
    NAVAL_SUMMARY,
    NAVAL_AIR,
    NAVAL_BREAKDOWNS,
    NAVAL_POSITIONING,
    NAVAL_ANTI_AIR,
    RESEARCH,
    COUNTRY_VISIBILITY,
    STATE_VISIBILITY,
    POWER_BALANCE,
    STATE_RESOURCES,
    RAID_LAUNCH,
    COMMAND_POWER,
    COUNTRY_ACE_MODIFIERS,
    RAID_UNIT_SPEED,
    RAID_INTEL,
    RAID_SUCCESS,
    INVASION_PREPARATION,
    NAVAL_MISSIONS,
    RESEARCH_IDLE,
    ORACLE_FIXED15,
    ORACLE_INFRASTRUCTURE,
];

/// Playback authenticates its own modifier literals, even in a recording made
/// before the recorder checked them. A partial or absent table is not evidence.
pub fn validate_recording(frame: &DecodedFrame, family: Family) -> Result<(), String> {
    let validate = || -> Result<(), String> {
        let definitions = frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|e| e.to_string())?;
        let tokens = frame
            .complete::<tables::LexicalTokens>()
            .map_err(|e| e.to_string())?;
        let gap = engine::fixed_layout::GAP;
        if definitions
            .binary_search_by_key(&gap, |row| row.index)
            .is_ok()
        {
            return Err(format!("modifier definition gap ({gap:#x}) is registered"));
        }
        for key in keys::ALL
            .iter()
            .filter(|key| family.keys.contains(&key.index))
        {
            let failure = |reason| format!("{} ({:#x}): {reason}", key.name, key.index);
            let row = definitions
                .binary_search_by_key(&key.index, |row| row.index)
                .map(|at| &definitions[at])
                .map_err(|_| failure("definition row missing"))?;
            let token = tokens
                .binary_search_by_key(&row.token, |row| row.token)
                .map(|at| &tokens[at])
                .map_err(|_| failure("definition token missing from LexicalTokens"))?;
            if token.literal != key.name.as_bytes() {
                return Err(failure("definition token spells a different script name"));
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

    fn fixture() -> DecodedFrame {
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
                keys::ALL
                    .iter()
                    .map(|key| ModifierDefinitionRow {
                        index: key.index,
                        token: key.index + 1,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                keys::ALL
                    .iter()
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
        for key in keys::ALL {
            assert!(
                FAMILIES
                    .iter()
                    .any(|family| family.keys.contains(&key.index)),
                "uncovered key {}",
                key.name
            );
        }
        for token in engine::fixed_layout::TOKENS {
            assert!(
                FAMILIES
                    .iter()
                    .any(|family| family.tokens.contains(&token.id)),
                "uncovered token {}",
                token.name
            );
        }
        for family in FAMILIES {
            for index in family.keys {
                assert!(keys::ALL.iter().any(|key| key.index == *index));
            }
            for id in family.tokens {
                assert!(
                    engine::fixed_layout::TOKENS
                        .iter()
                        .any(|token| token.id == *id)
                );
            }
        }
    }

    #[test]
    fn mismatch_fixture_refuses_each_viewer_literal_by_name() {
        for family in [
            PRODUCTION_MODIFIERS,
            NAVAL_WEAPONS,
            SHIP_ORGANISATION,
            NAVAL_POSITIONING,
            NAVAL_ANTI_AIR,
            NAVAL_BREAKDOWNS,
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
                assert!(
                    refusal.contains(
                        keys::ALL
                            .iter()
                            .find(|expected| expected.index == key)
                            .unwrap()
                            .name
                    )
                );
                assert!(refusal.contains("different script name"));
            }
        }
    }

    #[test]
    fn naval_breakdown_family_covers_all_capture_keys_and_its_table() {
        assert_eq!(
            NAVAL_BREAKDOWNS.keys,
            engine::naval_ship_modifier_breakdown::KEYS
        );
        assert_eq!(NAVAL_BREAKDOWNS.tables, [TableId::NavalModifierBreakdowns]);
        for key in NAVAL_BREAKDOWNS.keys {
            assert!(keys::ALL.iter().any(|expected| expected.index == *key));
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
            index: engine::fixed_layout::GAP + 1,
            token: 999,
            ..Default::default()
        });
        assert_eq!(validate_recording(&frame, NAVAL_WEAPONS), Ok(()));
        let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
            &mut *frame.tables[TableId::ModifierDefinitions.index()]
        else {
            panic!()
        };
        definitions.last_mut().unwrap().index = engine::fixed_layout::GAP;
        assert!(
            validate_recording(&frame, NAVAL_WEAPONS)
                .unwrap_err()
                .contains("gap (0x29a) is registered")
        );
    }
}
