//! The first clocked hour checks the fixed layout; a consistent result is
//! kept for the attachment, an interrupted one is repeated next hour.
//! Refusals replace the affected tables after carry/slice merging, so no held
//! baseline reintroduces an unchecked value, and they never request a refresh.

use crate::status::{self, DebugValue, DisplayValue};
use engine::fixed_layout::Report;
use replay::{
    fixed_layout::{FAMILIES, Family},
    frame::{TABLE_COUNT, TableInput, TableView},
};
use serde::Serialize;
use std::{fmt, io};

/// A final report, or why this hour's check could not complete.
pub(crate) type Layout = Result<Report, &'static str>;

#[derive(Clone, Copy)]
enum Refusal {
    Failure(engine::fixed_layout::Failure),
    Unverified(&'static str),
}
impl fmt::Display for Refusal {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failure(failure) => failure.fmt(out),
            Self::Unverified(reason) => write!(out, "fixed layout unverified: {reason}"),
        }
    }
}
#[derive(Clone, Copy)]
struct FamilyReason {
    family: Family,
    refusal: Refusal,
}
impl fmt::Display for FamilyReason {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{} refused: {}", self.family.name, self.refusal)
    }
}
fn refusal(layout: &Layout, family: Family) -> Option<FamilyReason> {
    let refusal = match layout {
        Ok(report) => Refusal::Failure(report.failure(family.keys, family.tokens)?),
        Err(reason) => Refusal::Unverified(reason),
    };
    Some(FamilyReason { family, refusal })
}

pub(crate) fn apply(tables: &mut [TableInput; TABLE_COUNT], layout: &Layout) -> io::Result<()> {
    if layout.is_ok_and(|report| report.is_verified()) {
        return Ok(());
    }
    for table in replay::rows::TableId::ALL {
        let mut reason = String::new();
        for family in FAMILIES
            .iter()
            .copied()
            .filter(|family| family.tables.contains(&table))
        {
            if let Some(refusal) = refusal(layout, family) {
                crate::roster::add_reason(&mut reason, format_args!("{refusal}"))?;
            }
        }
        if !reason.is_empty() {
            tables[table.index()] = TableInput::Sampled(TableView::Unavailable { reason });
        }
    }
    Ok(())
}

pub(crate) fn status(report: Option<&Report>) -> impl Serialize + '_ {
    let Some(report) = report else {
        return status::Choice::First(status::status!({"state": "unverified"}));
    };
    status::Choice::Second(status::status!({
        "state": if report.is_verified() { "verified" } else { "refused" },
        "static_token_maximum": report.static_token_maximum,
        "modifier_keys_checked": engine::modifier_keys::NAMES.len(),
        "static_tokens_checked": engine::fixed_layout::TOKENS.len(),
        "layout_failures": status::sequence(report.failures().map(DisplayValue)),
        "refusals": status::sequence(FAMILIES.iter().copied().filter_map(move |family| {
            refusal(&Ok(*report), family).map(|reason| status::status!({
                "tables": status::sequence(family.tables.iter().map(DebugValue)),
                "reason": DisplayValue(reason),
            }))
        })),
    }))
}

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() -> io::Result<()> {
    let mut tables = replay::rows::TableId::ALL.map(|table| {
        TableInput::Sampled(TableView::Complete(replay::rows::TableRows::empty(table)))
    });
    apply(&mut tables, &Err("synthetic native layout changed"))
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
                    | TableId::CombatLogState
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
        apply(&mut input, &Ok(report)).unwrap();
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
        let status = serde_json::to_value(status(Some(&report))).unwrap();
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
            apply(&mut input, &Ok(report)).unwrap();
            assert_key_families_refused(&input, &format!("{name} ({index:#x})"));
            let status = serde_json::to_value(status(Some(&report))).unwrap();
            assert_eq!(status["layout_failures"].as_array().unwrap().len(), 1);
        }
    }

    #[test]
    fn valid_report_preserves_every_input_and_records_static_maximum() {
        let mut input = inputs();
        let original = input.clone();
        apply(&mut input, &Ok(valid())).unwrap();
        assert_eq!(input, original);
        let status = serde_json::to_value(status(Some(&valid()))).unwrap();
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
        apply(&mut input, &Ok(report)).unwrap();
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
        apply(&mut input, &Ok(report)).unwrap();
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
        apply(&mut input, &Ok(report)).unwrap();
        assert_eq!(input, original);
        let status = serde_json::to_value(status(Some(&report))).unwrap();
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
        apply(&mut input, &Err("lexer changed during lookup")).unwrap();
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
        assert_eq!(
            serde_json::to_value(status(None)).unwrap()["state"],
            "unverified"
        );
    }
}
