
use crate::status::{self, DebugValue, DisplayValue};
use engine_fixed_layout::fixed_layout::Report;
use replay_data::rows::TABLE_COUNT;
use replay_derive::{fixed_layout::FAMILIES, fixed_layout::Family};
use replay_frame::{frame::TableInput, frame::TableView};
use serde::Serialize;
use std::{fmt, io};

pub type Layout = Result<Report, &'static str>;

pub fn task_force_eta_guard(layout: Option<&Layout>) -> Result<(), &'static str> {
    let Some(Ok(report)) = layout else {
        return Err("fresh task-force modifier layout unverified");
    };
    let family = replay_derive::fixed_layout::TASK_FORCE_COUNTER_ETA;
    if report.failure(family.keys, family.tokens).is_some() {
        Err("fresh task-force modifier layout mismatch")
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Refusal {
    Failure(engine_fixed_layout::fixed_layout::Failure),
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

pub fn apply(tables: &mut [TableInput; TABLE_COUNT], layout: &Layout) -> io::Result<()> {
    if layout.is_ok_and(|report| report.is_verified()) {
        return Ok(());
    }
    for table in replay_data::rows::TableId::ALL {
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

pub fn status(report: Option<&Report>) -> impl Serialize + '_ {
    let Some(report) = report else {
        return status::Choice::First(status::status!({"state": "unverified"}));
    };
    status::Choice::Second(status::status!({
        "state": if report.is_verified() { "verified" } else { "refused" },
        "static_token_maximum": report.static_token_maximum,
        "modifier_keys_checked": report.modifiers.len(),
        "static_tokens_checked": engine_fixed_layout::fixed_layout::TOKEN_IDS.len(),
        "layout_failures": status::sequence(report.failures().map(DisplayValue)),
        "refusals": status::sequence(FAMILIES.iter().copied().filter_map(move |family| {
            refusal(&Ok(*report), family).map(|reason| status::status!({
                "tables": status::sequence(family.tables.iter().map(DebugValue)),
                "reason": DisplayValue(reason),
            }))
        })),
    }))
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_failure_exercise() -> io::Result<()> {
    let mut tables = replay_data::rows::TableId::ALL.map(|table| {
        TableInput::Sampled(TableView::Complete(replay_data::rows::TableRows::empty(
            table,
        )))
    });
    apply(&mut tables, &Err("synthetic native layout changed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_executable::modifier_keys;
    use engine_fixed_layout::{fixed_layout::GAP, fixed_layout::TOKEN_IDS};
    use engine_native_memory::cpdx_lexer;
    use engine_research::cresearch_slot;
    use replay_data::{rows::TableId, rows::TableRows};
    use replay_derive::fixed_layout::RESEARCH_IDLE;
    use replay_frame::{
        frame::DecodedFrame, frame::FrameDecoder, frame::FrameEncoder, frame::FrameInput,
        frame::Metrics,
    };

    fn enter_layout() {
        use engine_executable::executable_tables::{self, Tables};
        static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
        let tables =
            *TABLES.get_or_init(|| synthetic_game::Executable::fixed_layout(TOKEN_IDS).build());
        executable_tables::enter(Some(tables));
    }

    fn valid() -> Report {
        enter_layout();
        Report {
            static_token_maximum: 0x6000,
            gap: Ok(()),
            modifiers: [Ok(()); GAP as usize],
            tokens: [Ok(()); TOKEN_IDS.len()],
        }
    }

    fn inputs() -> [TableInput; TABLE_COUNT] {
        TableId::ALL.map(|table| {
            if table.key_bits() == 0 && table != TableId::ReservedCountryNaming {
                TableInput::Sampled(TableView::Unavailable {
                    reason: "outside fixed-layout fixture".into(),
                })
            } else if table == TableId::ReservedCountryNaming {
                TableInput::SharedComplete(std::sync::Arc::new(TableRows::ReservedCountryNaming(
                    vec![replay_data::rows::ReservedCountryNamingRow::default()],
                )))
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

    #[test]
    fn task_force_eta_guard_reuses_layout_without_withholding_motion_prefix() {
        assert_eq!(
            task_force_eta_guard(None),
            Err("fresh task-force modifier layout unverified")
        );
        assert_eq!(
            task_force_eta_guard(Some(&Err("fixture interrupted lexer"))),
            Err("fresh task-force modifier layout unverified"),
        );
        assert_eq!(task_force_eta_guard(Some(&Ok(valid()))), Ok(()));
        for &key in replay_derive::fixed_layout::TASK_FORCE_COUNTER_ETA.keys {
            let layout = Ok(mismatched(key as usize));
            assert_eq!(
                task_force_eta_guard(Some(&layout)),
                Err("fresh task-force modifier layout mismatch")
            );
            let mut input = inputs();
            apply(&mut input, &layout).unwrap();
            assert!(untouched(&input, TableId::TaskForceMapMotion));
        }
    }

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
    fn strategic_action_key_mismatch_keeps_current_movement_available() {
        let mut input = inputs();
        apply(
            &mut input,
            &Ok(mismatched(
                modifier_keys::DISABLE_STRATEGIC_REDEPLOYMENT as usize,
            )),
        )
        .unwrap();
        assert!(untouched(&input, TableId::DivisionMovement));
        assert!(untouched(&input, TableId::Divisions));
        assert!(
            replay_derive::fixed_layout::STRATEGIC_ACTION
                .tables
                .is_empty()
        );
    }

    #[test]
    fn fixed_layout_inputs_cover_every_singleton_table() {
        let input = inputs();
        for table in TableId::ALL
            .into_iter()
            .filter(|table| table.key_bits() == 0)
        {
            match &input[table.index()] {
                TableInput::Sampled(TableView::Unavailable { .. }) => {}
                TableInput::SharedComplete(rows) => assert_eq!(rows.len(), 1, "{table:?}"),
                _ => panic!("{table:?} is neither refused nor one row"),
            }
        }
    }

    #[test]
    fn a_used_key_mismatch_refuses_every_key_family_and_survives_encoding() {
        let index = modifier_keys::ARMY_SPEED_FACTOR;
        let name = format!("synthetic_modifier_{index}");
        let report = mismatched(index as usize);
        let mut input = inputs();
        apply(&mut input, &Ok(report)).unwrap();
        assert_key_families_refused(
            &input,
            &format!("{name} ({index:#x}): definition token spells a different script name"),
        );
        for table in [
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
        let family = FAMILIES
            .iter()
            .find(|family| !family.keys.is_empty() && !family.tables.is_empty())
            .unwrap();
        assert!(matches!(
            &*frame.tables[family.tables[0].index()],
            TableView::Unavailable { reason } if reason.contains(&format!("{} refused: {name}", family.name))
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
                .contains(&name)
        }));
    }

    #[test]
    fn an_unused_builtin_mismatch_refuses_every_key_family_by_name() {
        enter_layout();
        for index in [0, GAP as usize - 1] {
            assert!(
                modifier_keys::ALL_INDICES
                    .iter()
                    .all(|&key| key as usize != index)
            );
            let name = modifier_keys::names().unwrap()[index];
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
        assert_key_families_refused(&input, &format!("modifier definition gap ({GAP:#x})"));
        assert!(untouched(&input, TableId::ResearchSlots));
    }

    #[test]
    fn token_mismatch_refuses_its_consumers_by_name() {
        let idle = cresearch_slot::IDLE_HANDLE;
        let mut report = valid();
        report.tokens[engine_fixed_layout::fixed_layout::tokens()
            .unwrap()
            .position(|token| token.id == idle)
            .unwrap()] = Err("token spells a different name");
        let mut input = inputs();
        apply(&mut input, &Ok(report)).unwrap();
        for &table in RESEARCH_IDLE.tables {
            assert!(refused(
                &input,
                table,
                &format!("research slots refused: synthetic_token_{idle} ({idle:#x})")
            ));
        }
        assert!(untouched(&input, TableId::LexicalTokens));

        let fixed15 = cpdx_lexer::FIXED15_TOKEN;
        let mut report = valid();
        report.tokens[engine_fixed_layout::fixed_layout::tokens()
            .unwrap()
            .position(|token| token.id == fixed15)
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
                .contains(&format!("synthetic_token_{fixed15} ({fixed15:#x})"))
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

    #[test]
    fn retired_transport_families_never_appear_in_status_while_shared_keys_refuse_consumers() {
        for index in [
            modifier_keys::SPOTTING_CHANCE_AGAINST,
            modifier_keys::AIR_ATTACK_FACTOR,
        ] {
            let name = format!("synthetic_modifier_{index}");
            let report = mismatched(index as usize);
            let status = serde_json::to_value(status(Some(&report))).unwrap();
            assert_eq!(status["state"], "refused");
            let refusals = status["refusals"].as_array().unwrap();
            assert!(refusals.iter().all(|entry| {
                let reason = entry["reason"].as_str().unwrap();
                !reason.contains("selected transport risk")
                    && !reason.contains("selected transport air normalization")
            }));
            let mut input = inputs();
            apply(&mut input, &Ok(report)).unwrap();
            assert!(refused(&input, TableId::NavalMissionTaskForces, &name));
            assert!(refused(&input, TableId::CountryCounterInputs, &name));
        }
    }
}
