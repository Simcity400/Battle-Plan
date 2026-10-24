use super::*;
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::Recorded,
    rows::ProvinceRow, rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn frame(owner: u32) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "synthetic recording gap".into(),
    });
    tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 7,
            canonical_slot: 3,
            tag: b"current/mod".to_vec(),
            ..Default::default()
        }]));
    tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 3,
        }]));
    tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 7,
            original_tag_reference: Recorded::Value(Some(1)),
            original_tag: Recorded::Value(Some(b"original/mod".to_vec())),
            ..Default::default()
        }]));
    tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            state: 2,
            controller: 11,
            ..Default::default()
        }]));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![StateRow {
        id: 2,
        owner,
        controller: 11,
        ..Default::default()
    }]));
    DecodedFrame {
        branch: 1,
        hour: 3,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

#[test]
fn empty_qualified_victory_point_suppresses_bare_retry_and_uses_state() {
    let mut keys = Vec::new();
    let name = location(
        &frame(11),
        &RaidScope {
            province: 1,
            ..Default::default()
        },
        |key| {
            keys.push(key.to_owned());
            Ok(Some(
                if key == "original/mod_VICTORY_POINTS_1" {
                    ""
                } else {
                    "wrong bare name"
                }
                .into(),
            ))
        },
        |id| {
            assert_eq!(id, 2);
            Ok("recorded state".into())
        },
    );
    assert_eq!(name.as_deref(), Ok("recorded state"));
    assert_eq!(keys, ["original/mod_VICTORY_POINTS_1"]);
}

#[test]
fn unresolved_and_reserved_owners_refuse_before_bare_content_lookup() {
    for (owner, reason) in [(99, "owner 99 is unresolved"), (0, "ReservedCountryNaming")] {
        assert!(
            victory_point(&frame(owner), 1, |_| panic!(
                "unknown owner must not choose a content key"
            ))
            .unwrap_err()
            .contains(reason)
        );
    }
    let mut missing = frame(11);
    *missing.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "original-tag recording gap".into(),
    };
    assert!(
        victory_point(&missing, 1, |_| panic!(
            "missing recording is not absent content"
        ))
        .unwrap_err()
        .contains("original-tag recording gap")
    );
    let mut no_state = frame(11);
    *no_state.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![]));
    assert!(
        victory_point(&no_state, 1, |_| panic!(
            "missing state is not absent content"
        ))
        .unwrap_err()
        .contains("raid target state 2 is absent")
    );
    let mut ambiguous = frame(11);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *ambiguous.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 8,
            canonical_slot: 3,
            ..Default::default()
        });
    }
    assert!(
        victory_point(&ambiguous, 1, |_| panic!(
            "ambiguous owner cannot select a tag"
        ))
        .unwrap_err()
        .contains("ambiguous countries")
    );
}

#[test]
fn empty_state_scope_and_building_location_keep_absence() {
    for building_location in [0, u32::MAX] {
        let scope = RaidScope {
            building_type: 4,
            building_location,
            ..Default::default()
        };
        assert_eq!(state(&frame(11), &scope), Ok(None));
        assert_eq!(
            location(&frame(11), &scope, |_| panic!(), |_| panic!()).as_deref(),
            Ok("no_target_name")
        );
    }
    let scope = RaidScope {
        building_location: 1,
        ..Default::default()
    };
    assert_eq!(
        state(&frame(11), &scope),
        Ok(None),
        "an absent building cannot supply a location"
    );
}

#[test]
fn original_tag_selection_keeps_original_current_empty_and_invalid_inputs_distinct() {
    let mut country = CountryRow {
        tag: b"current/mod".to_vec(),
        ..Default::default()
    };
    let mut appearance = CountryAppearanceRow {
        original_tag_reference: Recorded::Value(Some(1)),
        original_tag: Recorded::Value(Some(b"original/mod".to_vec())),
        ..Default::default()
    };
    let tag = crate::country_names::original_tag;
    assert_eq!(tag(&country, &appearance), Ok("original/mod"));
    appearance.original_tag = Recorded::Value(Some(Vec::new()));
    assert_eq!(tag(&country, &appearance), Ok(""));
    appearance.original_tag_reference = Recorded::Value(Some(0));
    assert_eq!(tag(&country, &appearance), Ok("current/mod"));
    country.tag.clear();
    assert_eq!(tag(&country, &appearance), Ok(""));
    appearance.original_tag_reference = Recorded::Value(Some(1));
    appearance.original_tag = Recorded::Value(Some(vec![0xff]));
    assert_eq!(
        tag(&country, &appearance),
        Err("country original-or-own tag is not UTF-8".into())
    );

    let mut observed = frame(11);
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *observed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(0));
        rows[0].original_tag = Recorded::Value(Some(Vec::new()));
    }
    assert_eq!(
        victory_point(&observed, 1, |key| {
            assert_eq!(key, "current/mod_VICTORY_POINTS_1");
            Ok(Some("current target".into()))
        }),
        Ok(Some("current target".into()))
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *observed.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag_reference = Recorded::Value(Some(1));
        rows[0].original_tag = Recorded::Value(Some(Vec::new()));
    }
    assert_eq!(
        victory_point(&observed, 1, |key| {
            assert_eq!(key, "_VICTORY_POINTS_1");
            Ok(Some("empty exact prefix".into()))
        }),
        Ok(Some("empty exact prefix".into()))
    );
}

#[test]
fn a_known_zero_state_owner_uses_the_reserved_exact_getter_without_positive_tables() {
    let mut observed = frame(0);
    *observed.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "unrelated positive countries".into(),
    };
    *observed.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "unrelated positive links".into(),
    };
    *observed.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "unrelated positive appearance".into(),
    };
    *observed.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                original_tag_reference: Recorded::Value(Some(1)),
                original_tag: Recorded::Value(Some(b"RESERVED_ORIGINAL".to_vec())),
                ..Default::default()
            },
        ]));
    assert_eq!(
        victory_point(&observed, 1, |key| {
            assert_eq!(key, "RESERVED_ORIGINAL_VICTORY_POINTS_1");
            Ok(Some("reserved target".into()))
        }),
        Ok(Some("reserved target".into()))
    );
}

#[test]
fn recorded_null_province_state_is_absent_without_inventing_an_owner_tag() {
    let mut observed = frame(11);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *observed.tables[TableId::Provinces.index()]
    {
        rows[0].state = 0;
    }
    for scope in [
        RaidScope {
            province: 1,
            ..Default::default()
        },
        RaidScope {
            leader_province: 1,
            ..Default::default()
        },
        RaidScope {
            building_type: 4,
            building_flags: 1,
            building_location: 1,
            ..Default::default()
        },
    ] {
        assert_eq!(state(&observed, &scope), Ok(None));
    }
    assert!(
        victory_point(&observed, 1, |_| panic!(
            "reserved owner's tag is unrecorded"
        ))
        .unwrap_err()
        .contains("ReservedCountryNaming")
    );
}

#[test]
fn resolved_captive_country_keeps_raw_bits_and_zero_suppresses_fallback() {
    for (captor, raw) in [(0, None), (-1, Some(u32::MAX))] {
        assert_eq!(
            country(
                &frame(11),
                &RaidScope {
                    leader_country: Some(captor),
                    leader_province: 1,
                    ..Default::default()
                }
            ),
            Ok(raw)
        );
    }
}
