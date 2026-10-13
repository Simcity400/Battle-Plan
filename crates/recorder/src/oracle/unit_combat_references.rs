//! Internal consistency checks only; no independent text-save counterpart.
use super::*;
use replay::rows::{Recorded, UnitCombatReferences};

pub(super) fn compare(frame: &DecodedFrame, report: &mut Report) {
    report.attempt("DivisionMovement combat-reference consistency", |report| {
        for row in frame.complete::<tables::DivisionMovement>()? {
            check_packet(
                frame,
                report,
                &row.combat_references,
                row.combat_count,
                "DivisionMovement.combat_references",
                row.division,
            )?;
        }
        Ok(())
    });
    report.attempt(
        "RailwayGunCounterInputs combat-reference consistency",
        |report| {
            for row in frame.complete::<tables::RailwayGunCounterInputs>()? {
                check_packet(
                    frame,
                    report,
                    &row.combat_references,
                    row.combat_count,
                    "RailwayGunCounterInputs.combat_references",
                    row.gun,
                )?;
            }
            Ok(())
        },
    );
}
fn check_packet(
    frame: &DecodedFrame,
    report: &mut Report,
    field: &Recorded<Option<UnitCombatReferences>>,
    count: u32,
    name: &str,
    unit: u64,
) -> io::Result<()> {
    let label = format!("{name} {unit:#x}");
    let Some(packet) = report.recorded(field, name, &label) else {
        return Ok(());
    };
    let Some(packet) = packet else {
        report.difference(
            &label,
            "unavailable",
            "required same-hour complete native unit vector",
        );
        return Ok(());
    };
    if !packet.valid_at(frame.hour, count) {
        report.difference(
            &label,
            "stale, malformed or different count",
            "current-hour exact count and supported authenticated kinds",
        );
        return Ok(());
    }
    for (ordinal, entry) in packet.entries.iter().enumerate() {
        let present = match entry.kind {
            0 | 1 => frame
                .complete::<tables::LandCombats>()?
                .binary_search_by_key(&entry.combat, |row| row.id)
                .is_ok(),
            2 => frame
                .complete::<tables::NavalCombats>()?
                .binary_search_by_key(&entry.combat, |row| row.id)
                .is_ok(),
            _ => false,
        };
        let entry_label = format!("{label}[{ordinal}] {:#x}", entry.combat);
        if !present {
            report.difference(&entry_label, "absent from recorded class census", "present");
            continue;
        }
        if entry.kind <= 1 {
            let rows = frame.complete::<tables::LandCombatDetails>()?;
            let Ok(at) = rows.binary_search_by_key(&entry.combat, |row| row.combat) else {
                report.difference(
                    &entry_label,
                    "absent from recorded land class operands",
                    "present",
                );
                continue;
            };
            if let Some(border) = report.recorded(
                &rows[at].border_war,
                "LandCombatDetails.border_war",
                &entry_label,
            ) {
                let actual_kind = u8::from(border.is_some());
                if entry.kind != actual_kind {
                    report.difference(format!("{entry_label}.kind"), entry.kind, actual_kind);
                }
            }
        }
    }
    report.limitation("the unit save writer does not save its combat vector. Order, repeats, kind and handle association are checked only within the recording; saved combat-side participants are a separate comparison and cannot certify this unit-owned vector. A matched-hour independent native witness remains required. The railway gun's saved combat list is a different vector",&label);
    // Deliberately no report.compared: consistency is not a saved-field match.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{LandCombatDetailsRow, LandCombatRow, UnitCombatReference};
    fn packet() -> UnitCombatReferences {
        UnitCombatReferences {
            sample_hour: replay::rows::PacketHour::Host,
            entries: vec![
                UnitCombatReference {
                    combat: u64::MAX,
                    kind: 0,
                },
                UnitCombatReference {
                    combat: u64::MAX,
                    kind: 0,
                },
            ],
        }
    }
    #[test]
    fn historical_absence_refusal_and_unsaved_known_empty_are_separate() {
        let (frame, _, _, _) = super::super::tests::fixture();
        for (field, count, differences) in [
            (Recorded::NotRecorded, 0, 0),
            (Recorded::Value(None), 0, 1),
            (
                Recorded::Value(Some(UnitCombatReferences {
                    sample_hour: replay::rows::PacketHour::Host,
                    entries: Vec::new(),
                })),
                0,
                0,
            ),
        ] {
            let mut report = Report::default();
            check_packet(
                &frame,
                &mut report,
                &field,
                count,
                "DivisionMovement.combat_references",
                77,
            )
            .unwrap();
            assert_eq!(report.differences.len(), differences);
            assert!(report.compared_tables.is_empty());
            if matches!(field, Recorded::Value(Some(_))) {
                assert!(
                    report
                        .limitations
                        .keys()
                        .any(|reason| reason.contains("independent native witness"))
                );
            }
        }
    }
    #[test]
    fn stale_count_and_missing_combat_remain_differences_never_save_acceptance() {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                id: u64::MAX,
                ..Default::default()
            }]));
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                combat: u64::MAX,
                border_war: None.into(),
                ..Default::default()
            }]));
        for (packet, count, differences) in [(packet(), 2, 0), (packet(), 1, 1)] {
            let mut report = Report::default();
            check_packet(
                &frame,
                &mut report,
                &Some(packet).into(),
                count,
                "DivisionMovement.combat_references",
                77,
            )
            .unwrap();
            assert_eq!(report.differences.len(), differences);
            assert!(report.compared_tables.is_empty());
        }
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(Vec::new()));
        let mut report = Report::default();
        check_packet(
            &frame,
            &mut report,
            &Some(packet()).into(),
            2,
            "DivisionMovement.combat_references",
            77,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 2);
    }
    #[test]
    fn wrong_supported_kind_is_a_difference_and_missing_kind_stays_unverified() {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        *frame.tables[TableId::LandCombats.index()] =
            TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
                id: u64::MAX,
                ..Default::default()
            }]));
        for border in [None, Some(replay::rows::LandBorderWarInputs::default())] {
            *frame.tables[TableId::LandCombatDetails.index()] =
                TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                    combat: u64::MAX,
                    border_war: border.clone().into(),
                    ..Default::default()
                }]));
            let actual = u8::from(border.is_some());
            let packet = UnitCombatReferences {
                sample_hour: replay::rows::PacketHour::Host,
                entries: vec![UnitCombatReference {
                    combat: u64::MAX,
                    kind: 1 - actual,
                }],
            };
            let mut report = Report::default();
            check_packet(
                &frame,
                &mut report,
                &Some(packet).into(),
                1,
                "DivisionMovement.combat_references",
                77,
            )
            .unwrap();
            assert_eq!(report.differences.len(), 1);
            assert!(report.compared_tables.is_empty());
        }
        *frame.tables[TableId::LandCombatDetails.index()] =
            TableView::Complete(TableRows::LandCombatDetails(vec![LandCombatDetailsRow {
                combat: u64::MAX,
                border_war: Recorded::NotRecorded,
                ..Default::default()
            }]));
        let mut report = Report::default();
        check_packet(
            &frame,
            &mut report,
            &Some(packet()).into(),
            2,
            "DivisionMovement.combat_references",
            77,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("not recorded"))
        );
        assert!(report.compared_tables.is_empty());
    }
}
