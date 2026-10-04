//! Recording self-check; saves contain no destruction event stream.
use super::{Report, complete, invalid};
use replay::{frame::DecodedFrame, rows::tables};
use std::{collections::BTreeSet, io};

pub fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::EquipmentDestruction>(frame)?;
    let links: BTreeSet<_> = complete::<tables::CountryLinks>(frame)?
        .iter()
        .map(|r| r.id)
        .collect();
    let variants: BTreeSet<_> = complete::<tables::VariantDefinitions>(frame)?
        .iter()
        .map(|r| r.id)
        .collect();
    for row in rows {
        if row.amount_raw <= 0 {
            return Err(invalid(
                "EquipmentDestruction contains a nonpositive removal",
            ));
        }
        if !links.contains(&row.country) {
            return Err(invalid(format!(
                "EquipmentDestruction owner {} has no CountryLink",
                row.country
            )));
        }
        if row.variant as u32 != engine::cequipment_variant::REFERENCE_TYPE {
            return Err(invalid(
                "EquipmentDestruction invalid design reference type",
            ));
        }
        if !variants.contains(&row.variant) {
            return Err(invalid(format!(
                "EquipmentDestruction design {:#x} has no VariantDefinition in this frame",
                row.variant
            )));
        }
    }
    report.compared("EquipmentDestruction", rows.len());
    report.limitation("recording self-check only (positive removals, codec-validated stable causes including unknown, country links, design reference type and this frame's VariantDefinitions). Saves serialize remaining pools (140FFDB00), not these loss events. Production, replenishment, upgrades and transfers make whole-pool or stockpile deltas an inexact loss oracle (140C7BFC0, 140C5E9C0, 140F4E400, 140FDFA80); no remaining-pool difference is waived or presented as an event comparison", "EquipmentDestruction.amount_raw");
    report.limitation("event amounts are bounded pre/post reads in each hooked native destruction writer. Synthetic fixtures verify per-design differences; live hook coverage, ownership and actual removals still require the coordinated capture. First/branch intervals are partial and unavailable. Stable causes group verified caller contexts; an unrecognized caller records explicit unknown and is counted in collector status. Unhooked writers are not covered: paradrop disruption (CArmy::ApplyParadropDisruption -> 140C7AE10) and the remaining equipment of divisions destroyed in combat (CLandCombatant::participant_lifecycle 141299850)", "EquipmentDestruction.cause");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            CountryLinkRow, EquipmentDestructionCause, EquipmentDestructionRow, TableId, TableRows,
            VariantDefinitionRow,
        },
    };
    #[test]
    fn recording_checks_do_not_claim_save_event_agreement() {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture absent".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 4999,
                ..Default::default()
            }]));
        let row = EquipmentDestructionRow {
            country: 4999,
            cause: EquipmentDestructionCause::Attrition,
            variant: 0xf00100000046,
            amount_raw: 7,
        };
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: row.variant,
                ..Default::default()
            }]));
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![row.clone()]));
        let mut report = Report::default();
        compare(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("Saves serialize remaining pools"))
        );
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![
                EquipmentDestructionRow {
                    cause: EquipmentDestructionCause::Unknown,
                    ..row.clone()
                },
            ]));
        compare(&frame, &mut Report::default()).unwrap();
        for bad in [
            EquipmentDestructionRow {
                country: 5000,
                ..row.clone()
            },
            EquipmentDestructionRow {
                variant: 0xf00200000046,
                ..row.clone()
            },
            EquipmentDestructionRow {
                variant: 0,
                ..row.clone()
            },
            EquipmentDestructionRow {
                amount_raw: 0,
                ..row
            },
        ] {
            *frame.tables[TableId::EquipmentDestruction.index()] =
                TableView::Complete(TableRows::EquipmentDestruction(vec![bad]));
            assert!(compare(&frame, &mut Report::default()).is_err());
        }
        *frame.tables[TableId::EquipmentDestruction.index()] = TableView::Unavailable {
            reason: "overflow".into(),
        };
        assert!(compare(&frame, &mut Report::default()).is_err());
    }
}
