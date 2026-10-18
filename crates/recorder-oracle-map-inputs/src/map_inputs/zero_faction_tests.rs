use super::*;
use replay_data::{rows::Recorded, rows::ReservedCountryNamingRow};

#[test]
fn reserved_faction_oracle_preserves_null_full_handle_refusal_and_history() {
    let (mut frame, _, _, _) = super::super::fixtures::fixture();
    for value in [
        Recorded::Value(Some(0)),
        Recorded::Value(Some((u64::from(u32::MAX) << 32) | 0x128)),
        Recorded::Value(None),
        Recorded::NotRecorded,
    ] {
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    faction: value,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        reserved_naming(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 0);
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("global faction membership") && reason.contains("matched VM witness")
        }));
        assert_eq!(
            report
                .limitations
                .contains_key("ReservedCountryNaming.faction not recorded"),
            matches!(value, Recorded::NotRecorded)
        );
        assert_eq!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("reserved country faction identity unreadable")),
            matches!(value, Recorded::Value(None))
        );
    }
}
