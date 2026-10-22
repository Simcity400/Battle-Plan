use super::*;
use dashboard_base::test_fixtures::source;
use replay_data::{rows::SimulationFeatureRow, rows::TableRows};
use replay_frame::frame::TableView;

#[test]
fn saved_names_preserve_localised_empty_literal_and_unrenderable_names() {
    let localisation = dashboard_text::catalogue::tests::loaded(
        "l_english:\n mod_name:0 \"Localised name\"\n mod_empty:0 \"\"\n mod_refused:0 \"Name\u{14}?\"\n",
    );
    let known = saved_name(Some(&localisation), b"mod_name\0\xff");
    assert_eq!(known.name.as_deref(), Some("Localised name"));
    assert!(!known.unnamed && known.missing.is_none() && !known.literal_fallback);
    for bytes in [b"mod_empty".as_slice(), b"\0ignored"] {
        let empty = saved_name(Some(&localisation), bytes);
        assert!(empty.unnamed && empty.name.is_none() && empty.missing.is_none());
    }
    let literal = saved_name(None, b"Literal [unchanged]");
    assert_eq!(literal.name.as_deref(), Some("Literal [unchanged]"));
    assert!(literal.literal_fallback && !literal.unnamed);
    let key = saved_name(None, b"mod_name");
    assert_eq!(key.name.as_deref(), Some("mod_name"));
    for bytes in [b"mod_refused".as_slice(), &[0xff]] {
        let refused = saved_name(Some(&localisation), bytes);
        assert!(refused.name.is_none() && !refused.unnamed && refused.missing.is_some());
    }
}

#[test]
fn country_identity_lookup_uses_codec_order_and_refuses_reused_slots() {
    let countries: Vec<_> = (1..=24)
        .map(|id| CountryRow {
            id,
            canonical_slot: 25 - id,
            tag: format!("Opaque {id}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = countries
        .iter()
        .map(|row| CountryLinkRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
        })
        .collect();
    let indexed = country_rows(&countries, &links).unwrap();
    for raw in [1, 11, 24] {
        let identity = identity(raw, &links, &indexed).unwrap();
        assert_eq!(country_at(identity, &countries, &links).unwrap().id, raw);
        assert_eq!(
            tag_at(Some(identity), &countries, &links),
            tag_of(Some(identity), &indexed)
        );
    }
    let frozen = CountryIdentity { id: 24, slot: 1 };
    let mut changed = countries;
    changed.last_mut().unwrap().canonical_slot = 3;
    assert!(country_at(frozen, &changed, &links).is_none());
    assert!(tag_at(Some(frozen), &changed, &links).is_none());
    assert!(country_rows(&changed, &links).is_none());
}

#[test]
fn partial_country_identities_refuse_every_holder_of_an_ambiguous_slot() {
    let countries: Vec<_> = [(7, 1), (17, 1), (27, 1), (37, 2)]
        .into_iter()
        .map(|(id, canonical_slot)| CountryRow {
            id,
            canonical_slot,
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = countries
        .iter()
        .map(|row| CountryLinkRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
        })
        .collect();
    let (rows, unresolved) = country_rows_partial(&countries, &links);
    assert_eq!(unresolved, vec![7, 17, 27]);
    assert!(identity(7, &links, &rows).is_none());
    assert_eq!(rows.len(), 1);
    assert_eq!(
        identity(37, &links, &rows),
        Some(CountryIdentity { id: 37, slot: 2 })
    );
    assert!(country_rows(&countries, &links).is_none());
}

#[test]
fn duplicate_memberships_canonicalise_aliases_but_refuse_conflicting_values() {
    let countries = [CountryRow {
        id: 7,
        canonical_slot: 1,
        tag: b"Opaque".to_vec(),
        ..Default::default()
    }];
    let links = [
        CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        },
        CountryLinkRow {
            id: 107,
            canonical_slot: 1,
        },
    ];
    let countries = country_rows(&countries, &links).unwrap();
    let row = OperativeMembershipRow {
        id: 1,
        owner_country: 7,
        target_country: Some(7).into(),
        capturer_country: 7,
        roster_country: 7,
        assignment: 3,
        action_variant: Some(1),
        ..Default::default()
    };
    let mut seen = BTreeMap::new();
    assert_eq!(admit(&mut seen, &row, &links, &countries), Ok(true));
    let alias = OperativeMembershipRow {
        owner_country: 107,
        target_country: Some(107).into(),
        capturer_country: 107,
        roster_country: 107,
        ..row.clone()
    };
    assert_eq!(admit(&mut seen, &alias, &links, &countries), Ok(false));
    let conflicting = OperativeMembershipRow {
        assignment: 5,
        ..alias
    };
    assert_eq!(
        admit(&mut seen, &conflicting, &links, &countries),
        Err("Conflicting operative memberships".into())
    );
    assert_eq!(admit(&mut seen, &row, &links, &countries), Ok(false));
}

#[test]
fn agency_feature_and_intel_reads_require_their_recorded_own_inputs() {
    let mut frame = source(1, 0, None);
    assert_eq!(agency_feature(&frame), None);
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    assert_eq!(agency_feature(&frame), Some(false));
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 1 << 5,
            ..Default::default()
        }]));
    assert_eq!(agency_feature(&frame), Some(true));
    frame.metrics.table_ns[TableId::WingCumulative.index()] = 1;
    assert!(!membership_read(&frame));
    frame.metrics.table_ns[TableId::OperativeMemberships.index()] = 1;
    assert!(membership_read(&frame));
    // Operations is hourly: only its own read in this frame counts, not a
    // sibling table's.
    assert!(!operations_read(&frame));
    frame.metrics.table_ns[TableId::OperationAssignments.index()] = 1;
    assert!(!operations_read(&frame));
    frame.metrics.table_ns[TableId::Operations.index()] = 1;
    assert!(operations_read(&frame));
}

#[test]
fn operative_admission_uses_native_capture_and_mission_variants() {
    assert!(captured(0));
    for assignment in 1..=5 {
        assert!(!captured(assignment));
        assert!(supported_kind_assignment(0, assignment).is_ok());
    }
    assert!(
        supported_kind_assignment(4, 3)
            .unwrap_err()
            .contains("roster 4")
    );
    assert!(
        supported_kind_assignment(0, 6)
            .unwrap_err()
            .contains("assignment 6")
    );
    assert!(!captured(6));
    assert_eq!(mission_variant(None), Ok(false));
    assert_eq!(mission_variant(Some(0)), Ok(false));
    for variant in 1..=8 {
        assert_eq!(mission_variant(Some(variant)), Ok(true));
    }
    assert_eq!(
        mission_variant(Some(9)),
        Err("Unsupported operative mission variant 9".into())
    );
}

fn check_equal_country_link_reads(count: u32) {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut input = source(1, 100, None);
    *input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("realm {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    let mut countries = CountryInputs::default();
    let mut tracker = FrameChangeTracker::new();
    countries.update(&input, Some(&tracker.observe(&input, format)));
    let before = countries.link_comparisons;
    input.hour += 1;
    input.metrics.table_ns[TableId::Countries.index()] = 1;
    input.metrics.table_ns[TableId::CountryLinks.index()] = 1;
    countries.update(&input, Some(&tracker.observe(&input, format)));
    assert!(!countries.changed);
    assert_eq!(
        countries.link_comparisons, before,
        "an equal acquisition retains the join"
    );
    assert_eq!(countries.labels.len(), count as usize);
}

#[test]
fn equal_country_link_reads_do_not_compare_every_link() {
    check_equal_country_link_reads(64);
}
