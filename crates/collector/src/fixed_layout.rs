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
        "state": if report.is_verified() { "verified" } else { "refused" },
        "static_token_maximum": report.static_token_maximum,
        "modifier_keys_checked": engine::modifier_keys::NAMES.len(),
        "static_tokens_checked": engine::fixed_layout::TOKENS.len(),
        "layout_failures": report.failures().map(|failure| failure.to_string()).collect::<Vec<_>>(),
        "refusals": refusals.into_iter().map(|(family, reason)| json!({
            "tables": family.tables.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
            "reason": reason,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{
        cpdx_lexer, cresearch_slot,
        fixed_layout::{GAP, TOKENS},
        modifier_keys,
    };
    use replay::{
        fixed_layout::{MOVEMENT, RESEARCH_IDLE},
        frame::{DecodedFrame, FrameDecoder, FrameEncoder, FrameInput, Metrics},
        rows::{TableId, TableRows},
    };

    fn valid() -> Report {
        Report {
            static_token_maximum: 0x6000,
            gap: Ok(()),
            modifiers: [Ok(()); modifier_keys::NAMES.len()],
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
                    | TableId::AirSortieModifierMetadata
                    | TableId::LocalPlayer
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

    fn mismatched(index: usize) -> Report {
        let mut report = valid();
        report.modifiers[index] = Err("definition token spells a different script name");
        report
    }

    /// Every fixed-key family's tables are refused naming `text`; no other
    /// table is touched.
    fn assert_key_families_refused(input: &[TableInput; TABLE_COUNT], text: &str) {
        let keyed: Vec<_> = FAMILIES
            .iter()
            .filter(|family| !family.keys.is_empty())
            .flat_map(|family| family.tables)
            .collect();
        let fresh = inputs();
        for table in TableId::ALL {
            if keyed.contains(&&table) {
                assert!(refused(input, table, text), "{table:?}");
            } else if untouched(&fresh, table) {
                assert!(untouched(input, table), "{table:?}");
            }
        }
    }

    #[test]
    fn a_used_key_mismatch_refuses_every_key_family_and_survives_encoding() {
        let report = mismatched(modifier_keys::ARMY_SPEED_FACTOR as usize);
        let mut input = inputs();
        apply(&mut input, &Ok(report));
        assert_key_families_refused(
            &input,
            "army_speed_factor (0x2b): definition token spells a different script name",
        );
        for table in [
            TableId::DivisionMovement,
            TableId::Countries,
            TableId::ModifierDefinitions,
            TableId::LexicalTokens,
            TableId::ResearchSlots,
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
        assert_eq!(status["layout_failures"].as_array().unwrap().len(), 1);
        let refusals = status["refusals"].as_array().unwrap();
        assert!(!refusals.is_empty());
        assert!(refusals.iter().all(|refusal| {
            refusal["reason"]
                .as_str()
                .unwrap()
                .contains("army_speed_factor")
        }));
    }

    /// A built-in no reader uses is part of the same index contract: after a
    /// patch moves or renames it, every fixed key is refused by its name.
    #[test]
    fn an_unused_builtin_mismatch_refuses_every_key_family_by_name() {
        for index in [0, GAP as usize - 1] {
            assert!(
                modifier_keys::ALL
                    .iter()
                    .all(|key| key.index as usize != index)
            );
            let name = modifier_keys::NAMES[index];
            let report = mismatched(index);
            let mut input = inputs();
            apply(&mut input, &Ok(report));
            assert_key_families_refused(&input, &format!("{name} ({index:#x})"));
            let status = status(Some(&report));
            assert_eq!(status["layout_failures"].as_array().unwrap().len(), 1);
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
        assert_key_families_refused(&input, "modifier definition gap (0x29a)");
        assert!(untouched(&input, TableId::ResearchSlots));
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
