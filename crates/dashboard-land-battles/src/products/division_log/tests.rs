use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::DestroyedDivisionsRow, rows::DivisionStatusRow,
    rows::LandEventCoverageRow, rows::NameOperands, rows::Recorded, rows::TableRows,
};
use replay_frame::frame::TableView;

#[path = "tests/speed.rs"]
mod speed;

fn frame(hour: i32, divisions: u32) -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(0));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=5000)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        (1..=divisions)
            .map(|id| DivisionRow {
                id: u64::from(id) << 32 | 4713,
                roster_owner: id,
                logical_owner: 5000,
                expeditionary_owner: u32::MAX,
                province: id,
                template: u64::from(id) << 32 | 4714,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            (1..=divisions)
                .map(|id| DivisionStatusRow {
                    division: u64::from(id) << 32 | 4713,
                    naming: Recorded::Value(Some(NameOperands {
                        custom: format!("Unit {id}").into_bytes(),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(
            (1..=divisions)
                .map(|id| DivisionTemplateRow {
                    template: u64::from(id) << 32 | 4714,
                    owner: id,
                    name: format!("Template {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] =
        TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![]));
    *frame.tables[TableId::DestroyedDivisions.index()] =
        TableView::Complete(TableRows::DestroyedDivisions(vec![]));
    *frame.tables[TableId::LandEventCoverage.index()] =
        TableView::Complete(TableRows::LandEventCoverage(vec![LandEventCoverageRow {
            from_hour: hour - 1,
            to_hour: hour,
            complete: true,
            lost_events: 73,
            ..Default::default()
        }]));
    frame
}
fn witness(frame: &mut DecodedFrame, disposition: Disposition, cause: Option<Cause>) {
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] =
        TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![
            DivisionEquipmentDeletionRow {
                division_handle: 1 << 32 | 4713,
                country: 1,
                expeditionary_owner: u32::MAX,
                disposition,
                cause,
            },
        ]));
}
fn destroyed(frame: &mut DecodedFrame) {
    *frame.tables[TableId::DestroyedDivisions.index()] =
        TableView::Complete(TableRows::DestroyedDivisions(vec![DestroyedDivisionsRow {
            id: 1,
            division: 1 << 32 | 4713,
            owner: 1,
            killer_country: 2,
            killer_division: Some(2 << 32 | 4713),
            province: 17,
            hour: frame.hour,
            ..Default::default()
        }]));
}

#[test]
fn initial_census_is_not_creation_and_every_new_country_is_retained() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 1));
    assert!(scan.events.is_empty());
    scan.observe(&frame(101, 5000));
    assert_eq!(scan.events.len(), 4999);
    let last = &scan.events[4998];
    assert_eq!(last.row.tag.as_deref(), Some("mod country 5000"));
    assert_eq!(last.row.after_hour, Some(100));
    assert!(last.row.by);
    assert_eq!(
        last.presentation
            .as_ref()
            .unwrap()
            .template
            .as_ref()
            .unwrap()
            .literal,
        b"Template 5000"
    );
}

#[test]
fn disappearance_never_infers_a_loss_and_reappearance_is_not_creation() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 1));
    scan.observe(&frame(101, 0));
    scan.observe(&frame(102, 1));
    assert!(scan.events.is_empty());
    assert!(
        scan.warnings
            .keys()
            .any(|(_, reason)| reason.contains("no loss inferred"))
    );
}

#[test]
fn witnessed_cause_controls_loss_and_exact_killer_requires_coverage() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 1));
    let mut end = frame(101, 0);
    witness(&mut end, Disposition::Loss, Some(Cause::DivisionDestroyed));
    destroyed(&mut end);
    scan.observe(&end);
    let event = &scan.events[0];
    assert_eq!(event.row.kind, DivisionLogKind::Lost);
    assert!(!event.row.by);
    assert_eq!(event.row.killer_tag.as_deref(), Some("mod country 2"));
    assert_eq!(event.row.province, Some(17));
    assert_eq!(event.row.presentation_hour, Some(100));
    // A cumulative old loss count does not override this certified interval.
    assert_eq!(event.row.hour, 101);
    let mut unknown = Scan::default();
    unknown.observe(&frame(100, 1));
    *end.tables[TableId::LandEventCoverage.index()] = TableView::Unavailable {
        reason: "missed drain".into(),
    };
    unknown.observe(&end);
    assert!(unknown.events[0].row.by);
    assert_eq!(unknown.events[0].row.killer_tag, None);
}

#[test]
fn disbanding_cannot_borrow_killer_and_transfer_witnesses_are_deduplicated() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 1));
    let mut end = frame(101, 0);
    witness(&mut end, Disposition::Loss, Some(Cause::DivisionDisbanded));
    destroyed(&mut end);
    scan.observe(&end);
    assert_eq!(scan.events[0].row.status, "division_disbanded");
    assert!(scan.events[0].row.killer_tag.is_none());
    let mut transfers = Scan::default();
    transfers.observe(&frame(100, 1));
    witness(&mut end, Disposition::VolunteerTransferred, None);
    transfers.observe(&end);
    end.hour = 104;
    transfers.observe(&end);
    assert_eq!(transfers.events.len(), 1);
    assert_eq!(transfers.events[0].row.kind, DivisionLogKind::Transfer);
    transfers.observe(&frame(105, 1));
    assert_eq!(transfers.events.len(), 1);
}

#[test]
fn missing_census_and_country_reads_keep_dates_and_identity_unknown() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 1));
    let mut missing = frame(101, 0);
    *missing.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    scan.observe(&missing);
    let mut end = frame(104, 0);
    witness(&mut end, Disposition::Loss, Some(Cause::DivisionDestroyed));
    destroyed(&mut end);
    *end.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "identity failed".into(),
    };
    scan.observe(&end);
    let row = &scan.events[0].row;
    assert!(row.by);
    assert!(row.tag.is_none());
    assert!(row.killer_tag.is_none());
    assert_eq!(row.presentation_hour, Some(100));
    scan.observe(&frame(105, 2));
    assert_eq!(scan.events[1].row.after_hour, Some(104));
}

#[test]
fn ambiguous_death_handles_do_not_choose_a_killer() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 1));
    let mut end = frame(101, 0);
    witness(&mut end, Disposition::Loss, Some(Cause::DivisionDestroyed));
    destroyed(&mut end);
    let rows = end.complete::<tables::DestroyedDivisions>().unwrap();
    let mut twice = rows.to_vec();
    twice.push(DestroyedDivisionsRow {
        id: 2,
        killer_country: 3,
        ..twice[0].clone()
    });
    *end.tables[TableId::DestroyedDivisions.index()] =
        TableView::Complete(TableRows::DestroyedDivisions(twice));
    scan.observe(&end);
    assert!(scan.events[0].row.by);
    assert!(scan.events[0].row.killer_tag.is_none());
}

#[test]
fn icon_uses_native_nonnegative_index_and_falls_back_only_when_absent() {
    assert_eq!(
        icon_key(1, 0, b"mod_unit", |key| Ok(key == "GFX_div_templ_0_large")).unwrap(),
        "GFX_div_templ_0_large"
    );
    assert_eq!(
        icon_key(1, 65536, b"mod_unit", |key| Ok(
            key == "GFX_div_templ_65536_large"
        ))
        .unwrap(),
        "GFX_div_templ_65536_large"
    );
    assert_eq!(
        icon_key(1, 0, b"mod_unit", |key| Ok(
            key == "GFX_unit_mod_unit_icon_medium"
        ))
        .unwrap(),
        "GFX_unit_mod_unit_icon_medium"
    );
    assert!(
        icon_key(1, 0, b"mod_unit", |_| Err("refused custom sprite".into()))
            .unwrap_err()
            .contains("refused custom")
    );
    // The native priority validity gate precedes even a present custom sprite.
    assert!(
        icon_key(0, 0, b"", |_| panic!(
            "invalid priority must not look up artwork"
        ))
        .unwrap_err()
        .contains("null or invalid")
    );
    assert_eq!(
        icon_key(1, 0, b"", |_| Ok(true)).unwrap(),
        "GFX_div_templ_0_large"
    );
}

#[test]
fn filtered_totals_include_unloaded_history_but_never_future_reads() {
    let mut scan = Scan::default();
    scan.observe(&frame(100, 0));
    scan.observe(&frame(101, 5000));
    let mut end = frame(102, 5000);
    let divisions = end
        .complete::<tables::Divisions>()
        .unwrap()
        .iter()
        .filter(|row| row.id != ((1 << 32) | 4713))
        .cloned()
        .collect();
    *end.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
    witness(&mut end, Disposition::Loss, Some(Cause::DivisionDisbanded));
    scan.observe(&end);
    let mut params = DivisionLogTotalsParams {
        branch_id: "1".into(),
        first_hour: 100,
        last_hour: 101,
        countries: LandBattleCountryFilter {
            tags: None,
            membership: None,
            unobserved: vec![],
        },
        templates: None,
    };
    let cancelled = AtomicBool::new(false);
    let index = totals_index(&scan, &params, "all".into(), &cancelled).unwrap();
    assert_eq!(
        index.counts[index.reads.partition_point(|hour| *hour <= 101)],
        [5000, 0, 0]
    );
    assert_eq!(
        index.counts[index.reads.partition_point(|hour| *hour <= 102)],
        [5000, 1, 0]
    );
    params.countries.tags = Some(vec!["mod country 5000".into()]);
    let index = totals_index(&scan, &params, "country".into(), &cancelled).unwrap();
    assert_eq!(index.counts.last(), Some(&[1, 0, 0]));
    params.countries.tags = None;
    params.templates = Some(vec![]);
    let index = totals_index(
        &scan,
        &params,
        "unavailable saved template".into(),
        &cancelled,
    )
    .unwrap();
    assert_eq!(index.counts.last(), Some(&[0, 0, 0]));
}

#[test]
fn observed_cached_names_survive_generator_failure_and_do_not_load_legacy_content() {
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[]);
    let content = Arc::new(catalogue);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "division-names".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(
            Replay::from_records(
                dashboard_base::test_fixtures::CAMPAIGN.into(),
                &[dashboard_base::test_fixtures::records(
                    1,
                    vec![frame(100, 0)],
                )],
            )
            .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut supplied = frame(101, 1);
    for (cache, naming, expected) in [
        (
            Recorded::Value(Some(b"Retained native cache".to_vec())),
            Recorded::Value(None),
            Some("Retained native cache"),
        ),
        (
            Recorded::Value(Some(Vec::new())),
            Recorded::Value(None),
            Some(""),
        ),
        (
            Recorded::Value(None),
            Recorded::Value(Some(NameOperands {
                custom: b"Wrong substitute".to_vec(),
                ..Default::default()
            })),
            None,
        ),
        (
            Recorded::NotRecorded,
            Recorded::Value(Some(NameOperands {
                custom: b"Historical literal".to_vec(),
                ..Default::default()
            })),
            Some("Historical literal"),
        ),
    ] {
        *supplied.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: 1 << 32 | 4713,
                name: cache,
                naming,
                ..Default::default()
            }]));
        let mut presentation = Presentations::new(&supplied, None)
            .get(1 << 32 | 4713, None)
            .unwrap();
        // Name-only request: unrelated template and image loaders stay out of the seam.
        presentation.template = None;
        presentation.icon = None;
        let scan = Scan::default();
        let event = Event {
            row: scan.row(
                1 << 32 | 4713,
                101,
                DivisionLogKind::Created,
                "first_observed",
                1,
                Some(&presentation),
            ),
            presentation: Some(presentation),
        };
        let mut warnings = BTreeMap::new();
        let row = named(&source, &event, Some(&content), &mut warnings, true);
        assert_eq!(row.name.as_deref(), expected);
        if expected.is_none() {
            assert!(
                row.name_missing
                    .as_deref()
                    .unwrap()
                    .contains("DivisionStatus.name unavailable")
            );
            assert_eq!(warnings.values().sum::<usize>(), 1);
        } else {
            assert!(row.name_missing.is_none());
            assert!(warnings.is_empty());
        }
        assert!(source.state::<Cache>().lock().unwrap().names.is_none());
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_division_log_matches_oracle_after_commits_resume_append_and_country_scale() {
    use dashboard_base::prepared::series::tests::Fixture;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut death = frame(103, 4999);
    witness(
        &mut death,
        Disposition::Loss,
        Some(Cause::DivisionDestroyed),
    );
    destroyed(&mut death);
    if let TableView::Complete(TableRows::DestroyedDivisions(rows)) =
        &mut *death.tables[TableId::DestroyedDivisions.index()]
    {
        rows[0].hour = 102;
    }
    let mut unknown = frame(104, 5000);
    unknown.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic missing actors".into(),
    }
    .into();
    witness(&mut unknown, Disposition::Merged, None);
    let inputs = [
        frame(100, 1),
        frame(101, 4999),
        frame(102, 4999),
        death,
        unknown,
    ];
    let mut replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![inputs[0].clone()],
        )],
    )
    .unwrap();
    for input in &inputs[1..] {
        replay = replay
            .append(&[dashboard_base::test_fixtures::continuation_records(
                &replay,
                1,
                vec![input.clone()],
            )])
            .unwrap();
    }
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-divisions".into(),
            revision: "1".into(),
        },
        "prepared-divisions.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut prepared = Scan::new(source.replay_ref().format);
    for (index, input) in inputs.iter().enumerate() {
        prepared.configure(source.replay_ref().format);
        prepared.prepare(input).unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&source, &context, &mut writer, 1, index, &flag)
            .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        prepared.resume(input);
    }
    let oracle = scan(source.replay_ref(), 1, &flag).unwrap();
    for index in 0..super::windows(source.replay_ref(), 1).len() {
        let params = DivisionLogParams {
            branch_id: "1".into(),
            window: index,
        };
        let actual = prepared::query(
            &source,
            "division-log",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(
            actual,
            serde_json::to_value(oracle.reply(&source, &params, &context, &flag).unwrap()).unwrap()
        );
    }
    let params = BranchParams {
        branch_id: "1".into(),
    };
    let actual = prepared::query(
        &source,
        "division-log-manifest",
        serde_json::to_value(&params).unwrap(),
        &fixture.store,
        &fixture.key,
        &flag,
    )
    .unwrap();
    assert_eq!(
        actual,
        serde_json::to_value(oracle.manifest(&source, &params, &context, &flag).unwrap()).unwrap()
    );
    let all = LandBattleCountryFilter {
        tags: None,
        membership: None,
        unobserved: vec![],
    };
    let filters = [
        all.clone(),
        LandBattleCountryFilter {
            tags: Some(vec![
                "mod country 1".into(),
                "mod country 1".into(),
                "mod country 4999".into(),
            ]),
            ..all.clone()
        },
        LandBattleCountryFilter {
            tags: None,
            membership: Some(vec![
                dashboard_base::products::country_groups::MembershipRun {
                    tag: "mod country 1".into(),
                    first_hour: 101,
                    last_hour: 104,
                },
                dashboard_base::products::country_groups::MembershipRun {
                    tag: "mod country 1".into(),
                    first_hour: 102,
                    last_hour: 103,
                },
            ]),
            unobserved: vec![dashboard_base::products::country_groups::HourSpan {
                first_hour: 102,
                last_hour: 102,
            }],
        },
        LandBattleCountryFilter {
            unobserved: vec![dashboard_base::products::country_groups::HourSpan {
                first_hour: 103,
                last_hour: 104,
            }],
            ..all
        },
    ];
    for countries in filters {
        for templates in [
            None,
            Some(vec![]),
            Some(vec![(1_u64 << 32 | 4714).to_string()]),
        ] {
            for first_hour in 100..=104 {
                for last_hour in first_hour..=104 {
                    let params = DivisionLogTotalsParams {
                        branch_id: "1".into(),
                        first_hour,
                        last_hour,
                        countries: countries.clone(),
                        templates: templates.clone(),
                    };
                    let index = totals_index(&oracle, &params, "oracle".into(), &flag).unwrap();
                    let expected = totals_reply(
                        index.counts[index.reads.partition_point(|read| *read <= last_hour)],
                    )
                    .unwrap();
                    let actual = prepared::query(
                        &source,
                        "division-log-totals",
                        serde_json::to_value(&params).unwrap(),
                        &fixture.store,
                        &fixture.key,
                        &flag,
                    )
                    .unwrap();
                    assert_eq!(
                        actual,
                        serde_json::to_value(expected).unwrap(),
                        "range {first_hour}..{last_hour}"
                    );
                }
            }
        }
    }
}

#[test]
fn prepared_division_resume_keeps_last_complete_presentation_and_witness_signature() {
    let mut prepared = Scan::default();
    let mut oracle = Scan::default();
    let first = frame(100, 1);
    let mut unavailable = frame(101, 0);
    *unavailable.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "census failed".into(),
    };
    let mut removed = frame(102, 0);
    witness(&mut removed, Disposition::VolunteerTransferred, None);
    for input in [&first, &unavailable, &removed] {
        prepared.observe(input);
        oracle.observe(input);
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        prepared.resume(input);
    }
    let mut repeated = removed.clone();
    repeated.hour = 103;
    prepared.observe(&repeated);
    oracle.observe(&repeated);
    assert_eq!(prepared.events.len(), 1);
    assert_eq!(prepared.events[0].row.presentation_hour, Some(100));
    assert_eq!(
        serde_json::to_value(&prepared.events).unwrap(),
        serde_json::to_value(&oracle.events).unwrap()
    );
}
