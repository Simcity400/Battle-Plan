//! The first clocked hour checks the fixed layout; a consistent result is
//! kept for the attachment, an interrupted one is repeated next hour.
//! Refusals replace the affected tables after carry/slice merging, so no held
//! baseline reintroduces an unchecked value, and they never request a refresh.

use engine::fixed_layout::Report;
use replay::{
    fixed_layout::{FAMILIES, Family},
    frame::{TABLE_COUNT, TableInput, TableView},
};
use serde_json::{Value, json};

/// A final report, or why this hour's check could not complete.
pub(crate) type Layout = Result<Report, &'static str>;

fn refusals(layout: &Layout) -> impl Iterator<Item = (Family, String)> + '_ {
    FAMILIES.iter().filter_map(move |&family| {
        let reason = match layout {
            Ok(report) => report.failure(family.keys, family.tokens)?.to_string(),
            Err(reason) => format!("fixed layout unverified: {reason}"),
        };
        Some((family, format!("{} refused: {reason}", family.name)))
    })
}

pub(crate) fn apply(tables: &mut [TableInput; TABLE_COUNT], layout: &Layout) {
    if layout.is_ok_and(|report| report.is_verified()) {
        return;
    }
    let mut reasons = std::collections::BTreeMap::<_, Vec<String>>::new();
    for (family, reason) in refusals(layout) {
        for &table in family.tables {
            reasons.entry(table).or_default().push(reason.clone());
        }
    }
    for (table, reasons) in reasons {
        tables[table.index()] = TableInput::Sampled(TableView::Unavailable {
            reason: reasons.join("; "),
        });
    }
}

pub(crate) fn status(report: Option<&Report>) -> Value {
    let Some(report) = report else {
        return json!({"state": "unverified"});
    };
    let refusals: Vec<_> = refusals(&Ok(*report)).collect();
    json!({
        "state": if refusals.is_empty() { "verified" } else { "refused" },
        "static_token_maximum": report.static_token_maximum,
        "modifier_keys_checked": engine::modifier_keys::ALL.len(),
        "static_tokens_checked": engine::fixed_layout::TOKENS.len(),
        "refusals": refusals.into_iter().map(|(family, reason)| json!({
            "tables": family.tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
            "reason": reason,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{cpdx_lexer, cresearch_slot, fixed_layout::TOKENS, modifier_keys};
    use replay::{
        fixed_layout::{MOVEMENT, RESEARCH_IDLE},
        frame::{DecodedFrame, FrameDecoder, FrameEncoder, FrameInput, Metrics},
        rows::{TableId, TableRows},
    };

    fn valid() -> Report {
        Report {
            static_token_maximum: 0x6000,
            gap: Ok(()),
            modifiers: [Ok(()); modifier_keys::ALL.len()],
            tokens: [Ok(()); TOKENS.len()],
        }
    }

    fn inputs() -> [TableInput; TABLE_COUNT] {
        TableId::ALL.map(|table| {
            if matches!(
                table,
                TableId::Clock
                    | TableId::Campaign
                    | TableId::SunkShips
                    | TableId::WorldTension
                    | TableId::SimulationFeatures
            ) {
                TableInput::Sampled(TableView::Unavailable {
                    reason: "fixture has no clock or campaign".into(),
                })
            } else {
                TableInput::SharedComplete(std::sync::Arc::new(TableRows::empty(table)))
            }
        })
    }

    fn refused(input: &[TableInput; TABLE_COUNT], table: TableId, text: &str) -> bool {
        matches!(&input[table.index()], TableInput::Sampled(TableView::Unavailable { reason }) if reason.contains(text))
    }

    fn untouched(input: &[TableInput; TABLE_COUNT], table: TableId) -> bool {
        matches!(input[table.index()], TableInput::SharedComplete(_))
    }

    #[test]
    fn mismatch_refuses_only_dependent_tables_and_survives_encoding() {
        let mut report = valid();
        let position = modifier_keys::ALL
            .iter()
            .position(|key| key.index == modifier_keys::ARMY_SPEED_FACTOR)
            .unwrap();
        report.modifiers[position] = Err("definition token spells a different script name");
        let mut input = inputs();
        apply(&mut input, &Ok(report));
        for table in [TableId::DivisionEtaInputs, TableId::LandCombatDivisions] {
            assert!(refused(
                &input,
                table,
                "army_speed_factor (0x2b): definition token spells a different script name"
            ));
        }
        for table in [
            TableId::DivisionMovement,
            TableId::Countries,
            TableId::ModifierDefinitions,
            TableId::LexicalTokens,
            TableId::ResearchModifiers,
        ] {
            assert!(untouched(&input, table), "{table:?}");
        }
        let frame = FrameInput {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: input,
        };
        let encoded = FrameEncoder::new().encode_input(frame, true).unwrap();
        let frame: DecodedFrame = FrameDecoder::new().apply(&encoded).unwrap();
        assert!(matches!(
            &*frame.tables[TableId::DivisionEtaInputs.index()],
            TableView::Unavailable { reason } if reason.contains(&format!("{} refused: army_speed_factor", MOVEMENT.name))
        ));
        let status = status(Some(&report));
        assert_eq!(status["state"], "refused");
        let refusals = status["refusals"].as_array().unwrap();
        assert!(!refusals.is_empty());
        assert!(refusals.iter().all(|refusal| {
            refusal["reason"]
                .as_str()
                .unwrap()
                .contains("army_speed_factor")
        }));
    }

    #[test]
    fn a_retreat_ban_key_mismatch_withholds_only_the_retreat_bans() {
        for key in [
            modifier_keys::CANNOT_RETREAT_WHILE_DEFENDING,
            modifier_keys::CANNOT_RETREAT_WHILE_ATTACKING,
        ] {
            let mut report = valid();
            let position = modifier_keys::ALL
                .iter()
                .position(|expected| expected.index == key)
                .unwrap();
            report.modifiers[position] = Err("definition token spells a different script name");
            let mut input = inputs();
            apply(&mut input, &Ok(report));
            let name = modifier_keys::ALL[position].name;
            assert!(refused(
                &input,
                TableId::DivisionRetreatBans,
                &format!("commander retreat bans refused: {name} ({key:#x})")
            ));
            for table in [
                TableId::DivisionStatus,
                TableId::DivisionMovement,
                TableId::LandCombats,
            ] {
                assert!(untouched(&input, table), "{table:?}");
            }
        }
    }

    /// A command power key or an ace country modifier withholds only
    /// CountryCommandPower; a launch key only the launch inputs and ace
    /// ranges; the ace speed key only RaidSpeedInputs.
    #[test]
    fn launch_command_power_and_raid_speed_keys_withhold_their_own_tables() {
        let refuse = |key: u32| {
            let mut report = valid();
            let position = modifier_keys::ALL
                .iter()
                .position(|expected| expected.index == key)
                .unwrap();
            report.modifiers[position] = Err("definition token spells a different script name");
            let mut input = inputs();
            apply(&mut input, &Ok(report));
            input
        };
        for (key, family) in [
            (modifier_keys::MAX_COMMAND_POWER, "command power refused"),
            (
                modifier_keys::AIR_ACE_BONUSES_FACTOR,
                "country ace modifiers refused",
            ),
            (
                modifier_keys::ACE_EFFECTIVENESS_FACTOR,
                "country ace modifiers refused",
            ),
        ] {
            let power = refuse(key);
            assert!(
                refused(&power, TableId::CountryCommandPower, family),
                "{key:#x}"
            );
            for table in [
                TableId::RaidCountryInputs,
                TableId::AceRangeInputs,
                TableId::RaidSpeedInputs,
            ] {
                assert!(untouched(&power, table), "{key:#x} {table:?}");
            }
        }
        let launch = refuse(modifier_keys::NAVAL_COMMANDO_RAID_DISTANCE);
        for table in [TableId::RaidCountryInputs, TableId::AceRangeInputs] {
            assert!(refused(&launch, table, "raid launch refused"), "{table:?}");
        }
        for table in [TableId::CountryCommandPower, TableId::RaidSpeedInputs] {
            assert!(untouched(&launch, table), "{table:?}");
        }
        let speed = refuse(modifier_keys::AIR_MAXIMUM_SPEED_FACTOR);
        assert!(refused(
            &speed,
            TableId::RaidSpeedInputs,
            "raid unit speed refused"
        ));
        for table in [
            TableId::CountryCommandPower,
            TableId::RaidCountryInputs,
            TableId::AceRangeInputs,
        ] {
            assert!(untouched(&speed, table), "{table:?}");
        }
    }

    /// Each invasion preparation key withholds only InvasionPreparations.
    #[test]
    fn invasion_preparation_keys_withhold_only_their_table() {
        for key in [
            modifier_keys::INVASION_PREPARATION,
            modifier_keys::NAVAL_INVASION_PREP_DAYS,
            modifier_keys::INVASION_PREPARATION_AGAINST,
        ] {
            let mut report = valid();
            let position = modifier_keys::ALL
                .iter()
                .position(|expected| expected.index == key)
                .unwrap();
            report.modifiers[position] = Err("definition token spells a different script name");
            let mut input = inputs();
            apply(&mut input, &Ok(report));
            let name = modifier_keys::ALL[position].name;
            assert!(refused(
                &input,
                TableId::InvasionPreparations,
                &format!("invasion preparation refused: {name} ({key:#x})")
            ));
            for table in TableId::ALL
                .into_iter()
                .filter(|&table| table != TableId::InvasionPreparations)
                .filter(|&table| untouched(&inputs(), table))
            {
                assert!(untouched(&input, table), "{table:?}");
            }
        }
    }

    #[test]
    fn valid_report_preserves_every_input_and_records_static_maximum() {
        let mut input = inputs();
        let original = input.clone();
        apply(&mut input, &Ok(valid()));
        assert_eq!(input, original);
        let status = status(Some(&valid()));
        assert_eq!(status["state"], "verified");
        assert_eq!(status["static_token_maximum"], 0x6000);
        assert_eq!(status["refusals"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn gap_refuses_modifier_families_but_not_token_families() {
        let report = Report {
            gap: Err("row is not the default-constructed gap"),
            ..valid()
        };
        let mut input = inputs();
        apply(&mut input, &Ok(report));
        for family in FAMILIES.iter().filter(|family| !family.keys.is_empty()) {
            for &table in family.tables {
                assert!(
                    refused(&input, table, "modifier definition gap (0x29a)"),
                    "{table:?}"
                );
            }
        }
        assert!(untouched(&input, TableId::ResearchSlots));
        assert!(untouched(&input, TableId::Countries));
    }

    #[test]
    fn token_mismatch_refuses_its_consumers_by_name() {
        let mut report = valid();
        report.tokens[TOKENS
            .iter()
            .position(|token| token.id == cresearch_slot::IDLE_HANDLE)
            .unwrap()] = Err("token spells a different name");
        let mut input = inputs();
        apply(&mut input, &Ok(report));
        for &table in RESEARCH_IDLE.tables {
            assert!(refused(
                &input,
                table,
                "research slots refused: empty (0x2a4e)"
            ));
        }
        assert!(untouched(&input, TableId::LexicalTokens));

        // The save oracle alone consumes the fixed15 token: named, nothing refused.
        let mut report = valid();
        report.tokens[TOKENS
            .iter()
            .position(|token| token.id == cpdx_lexer::FIXED15_TOKEN)
            .unwrap()] = Err("exceeds the static token maximum");
        let mut input = inputs();
        let original = input.clone();
        apply(&mut input, &Ok(report));
        assert_eq!(input, original);
        let status = status(Some(&report));
        assert_eq!(status["state"], "refused");
        assert!(
            status["refusals"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("long_float (0x167)")
        );
    }

    #[test]
    fn unverified_hour_refuses_every_consumer() {
        let mut input = inputs();
        apply(&mut input, &Err("lexer changed during lookup"));
        for family in FAMILIES {
            for &table in family.tables {
                assert!(refused(
                    &input,
                    table,
                    "fixed layout unverified: lexer changed during lookup"
                ));
            }
        }
        assert!(untouched(&input, TableId::Countries));
        assert_eq!(status(None)["state"], "unverified");
    }
}
