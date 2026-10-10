use super::*;
use replay::rows::{Recorded, ReservedCountryNamingRow, TableId, TableRows};

fn frame(faction: Recorded<Option<u64>>) -> DecodedFrame {
    let mut frame = super::tests::positive_frame();
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                faction,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 0,
                faction: 5,
                ordinal: 0,
            },
        ]));
    frame
}

#[test]
fn raw_zero_first_member_clears_only_the_event_selector() {
    let frame = frame(Recorded::Value(Some(5)));
    let scopes = RecordedScopes::read(&frame, Arc::default());
    assert_eq!(scopes.slot(0), None);
    assert_eq!(scopes.faction_leader_raw(1), Ok(Some(0)));
    assert_eq!(scopes.raw_slot(0), Ok(0));
    // Scalar consumers retain their separate unsupported raw-zero disposition.
    assert!(
        scopes
            .faction_leader(1)
            .unwrap_err()
            .contains("CountryLinks")
    );
    for context in [
        Context::country(1),
        Context::default(),
        Context {
            this: Some(Target::State(64)),
            ..Context::default()
        },
        Context {
            this: Some(Target::Character(7)),
            ..Context::default()
        },
    ] {
        assert_eq!(
            scopes.resolve_text("faction_leader", false, &context),
            Ok(Target::State(crate::triggers::NULL_STATE))
        );
    }
    assert_eq!(
        scopes.resolve_text("faction_leader.PREV", false, &Context::country(1)),
        Ok(Target::Country(1))
    );
}

#[test]
fn typed_reserved_faction_keeps_null_and_faults_distinct() {
    for (field, expected) in [
        (Recorded::Value(Some(0)), Ok(None)),
        (Recorded::Value(Some(5)), Ok(Some(5))),
    ] {
        let mut frame = frame(field);
        *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
            reason: "unrelated member census".into(),
        };
        assert_eq!(
            RecordedScopes::read(&frame, Arc::default()).faction(0),
            expected
        );
    }
    for field in [Recorded::Value(None), Recorded::NotRecorded] {
        let frame = frame(field);
        let scopes = RecordedScopes::read(&frame, Arc::default());
        assert!(
            scopes
                .faction(0)
                .unwrap_err()
                .contains("ReservedCountryNaming.faction")
        );
        // A positive member's faction does not demand the reserved getter.
        assert_eq!(scopes.faction(1), Ok(Some(5)));
    }
    for table in [
        TableView::Unavailable {
            reason: "fixture fault".into(),
        },
        TableView::Truncated {
            rows: TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                faction: Recorded::Value(Some(5)),
                ..Default::default()
            }]),
            missing: 1,
        },
        TableView::Complete(TableRows::ReservedCountryNaming(Vec::new())),
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow::default(),
            ReservedCountryNamingRow::default(),
        ])),
    ] {
        let mut frame = frame(Recorded::Value(Some(5)));
        *frame.tables[TableId::ReservedCountryNaming.index()] = table;
        assert!(
            RecordedScopes::read(&frame, Arc::default())
                .faction(0)
                .unwrap_err()
                .contains("ReservedCountryNaming")
        );
    }
}

#[test]
fn literal_zero_members_do_not_poison_positive_absence_but_unlinked_positive_members_do() {
    let mut frame = frame(Recorded::Value(Some(5)));
    let scopes = RecordedScopes::read(&frame, Arc::default());
    assert_eq!(scopes.faction(2), Ok(None));
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    {
        rows.push(FactionMemberRow {
            country: 999,
            faction: 7,
            ordinal: 0,
        });
    }
    let scopes = RecordedScopes::read(&frame, Arc::default());
    assert!(scopes.faction(2).unwrap_err().contains("999"));
    assert_eq!(scopes.faction_leader_raw(1), Ok(Some(0)));
    *frame.tables[TableId::FactionMembers.index()] = TableView::Truncated {
        rows: TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 0,
                faction: 5,
                ordinal: 0,
            },
        ]),
        missing: 3,
    };
    let scopes = RecordedScopes::read(&frame, Arc::default());
    assert_eq!(scopes.faction_leader_raw(1), Ok(Some(0)));
    assert!(scopes.faction(2).unwrap_err().contains("truncated"));
    assert!(
        scopes
            .faction_first_raw(7)
            .unwrap_err()
            .contains("truncated")
    );
}

#[test]
fn absent_faction_stops_prev_while_positive_leader_alias_keeps_its_country_carrier() {
    let mut frame = frame(Recorded::Value(Some(0)));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    let scopes = RecordedScopes::read(&frame, Arc::default());
    assert!(matches!(
        scopes.resolve_text("faction_leader.PREV", false, &Context::country(1)),
        Err(Unresolved::Nothing(_))
    ));
    // Synthetic resolver-law control: positive first word maps canonical0.
    let mut frame = super::tests::frame();
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 200,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 0,
            },
        ]));
    let scopes = RecordedScopes::read(&frame, Arc::default());
    assert_eq!(
        scopes.resolve_text("faction_leader", false, &Context::country(1)),
        Ok(Target::Country(0))
    );
}
