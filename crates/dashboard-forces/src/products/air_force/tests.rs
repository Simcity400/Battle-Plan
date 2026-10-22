use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag};
use dashboard_core::transport;
use replay::container::Replay;
use replay_data::{rows::AirWingRow, rows::TableId, rows::TableRows, rows::WingEquipmentRow};
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn frame(
    hour: i32,
    wings: &[(u64, u32, i32, i32)],
    equipment: &[(u64, u64, i64)],
    countries: u32,
) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, countries);
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(
        wings
            .iter()
            .map(|&(id, owner, country_override, planes)| AirWingRow {
                id,
                owner,
                country_override,
                planes,
                ..Default::default()
            })
            .collect(),
    ));
    let mut ordinals = BTreeMap::new();
    *frame.tables[TableId::WingEquipment.index()] = TableView::Complete(TableRows::WingEquipment(
        equipment
            .iter()
            .map(|&(wing, variant, amount_raw)| {
                let ordinal = ordinals.entry(wing).or_insert(0);
                let row = WingEquipmentRow {
                    wing,
                    ordinal: *ordinal,
                    variant,
                    amount_raw,
                };
                *ordinal += 1;
                row
            })
            .collect(),
    ));
    frame
}

fn scan_frames(frames: Vec<DecodedFrame>) -> ScannedAir {
    scan::scan(
        &Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap(),
        1,
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn recorded_source(frames: Vec<DecodedFrame>) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "air-fixture".into(),
            revision: "1".into(),
        },
        "synthetic-air.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn tag(index: u32) -> String {
    String::from_utf8(scale_tag(index)).unwrap()
}
fn params(measure: AirForceMeasure, ownership: AirForceOwnership) -> AirForceParams {
    AirForceParams {
        branch_id: "1".into(),
        tags: vec![tag(0), tag(1)],
        top: None,
        measure,
        ownership,
    }
}
fn at(scanned: &ScannedAir, index: u32, hour: i32, ownership: AirForceOwnership) -> Option<i64> {
    scanned.series[&tag(index)]
        .iter()
        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
        .and_then(|run| run.counts(ownership))
        .map(|counts| counts.planes)
}

#[test]
fn air_force_native_plane_bank_and_signed_effective_override() {
    let scanned = scan_frames(vec![frame(
        22,
        &[(1, 100_000, 100_001, 10), (2, 100_000, -1, 4)],
        &[(1, 71, 600_000), (1, 72, 400_000), (2, 73, 400_000)],
        2,
    )]);
    assert_eq!(at(&scanned, 0, 22, AirForceOwnership::Roster), Some(14));
    assert_eq!(at(&scanned, 1, 22, AirForceOwnership::Roster), Some(0));
    assert_eq!(at(&scanned, 0, 22, AirForceOwnership::Effective), Some(4));
    assert_eq!(at(&scanned, 1, 22, AirForceOwnership::Effective), Some(10));
    let reply = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.parts.len(), 3);
    assert!(reply.parts.iter().all(|part| part.unresolved));
    let values = &reply.series[0].runs[0].values;
    assert_eq!(values.iter().flatten().sum::<i64>(), 14);
    assert!(reply.parts.iter().any(|part| part.key == ":variant/71"));
    let wings = reply::reply(
        &scanned,
        &params(AirForceMeasure::Wings, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(wings.series[0].runs[0].total, Some(2));
    assert_eq!(
        wings.series[0].runs[0].values.iter().flatten().sum::<i64>(),
        2
    );
    assert!(wings.parts.iter().any(|part| {
        part.name
            .as_deref()
            .is_some_and(|name| name.starts_with("Mixed types:"))
    }));
}

#[test]
fn air_force_complete_zero_unread_unknown_gaps_and_absent_owner() {
    let mut missing = frame(23, &[], &[], 2);
    *missing.tables[TableId::AirWings.index()] = TableView::Unavailable {
        reason: "synthetic unread".into(),
    };
    let scanned = scan_frames(vec![
        frame(22, &[], &[], 2),
        missing,
        frame(25, &[(1, 999_999, 0, 5)], &[(1, 71, 500_000)], 2),
    ]);
    assert_eq!(at(&scanned, 0, 22, AirForceOwnership::Roster), Some(0));
    assert_eq!(at(&scanned, 0, 23, AirForceOwnership::Roster), None);
    assert_eq!(at(&scanned, 0, 24, AirForceOwnership::Roster), None);
    assert_eq!(at(&scanned, 1, 25, AirForceOwnership::Roster), None);
    assert_eq!(scanned.segments[&tag(0)].len(), 2);
    let mut absent = frame(22, &[(1, 100_002, 0, 5)], &[(1, 71, 500_000)], 2);
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *absent.tables[TableId::CountryLinks.index()]
    {
        links.push(replay_data::rows::CountryLinkRow {
            id: 100_002,
            canonical_slot: 2,
        });
    }
    assert_eq!(
        at(&scan_frames(vec![absent]), 0, 22, AirForceOwnership::Roster),
        None
    );
}

#[test]
fn air_force_missing_or_fractional_bank_keeps_total_and_unknown_variant_parts() {
    let mut missing = frame(22, &[(1, 100_000, 0, 9)], &[], 2);
    *missing.tables[TableId::WingEquipment.index()] = TableView::Unavailable {
        reason: "synthetic missing bank".into(),
    };
    let scanned = scan_frames(vec![
        missing,
        frame(
            23,
            &[(1, 100_000, 0, 9)],
            &[(1, 71, 450_000), (1, 72, 450_000)],
            2,
        ),
    ]);
    assert_eq!(at(&scanned, 0, 22, AirForceOwnership::Roster), Some(9));
    let reply = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    let unknown = reply
        .parts
        .iter()
        .position(|part| part.key == ":variant/71")
        .unwrap();
    assert_eq!(reply.series[0].runs[1].total, Some(9));
    assert_eq!(reply.series[0].runs[1].values[unknown], None);
    let remainder = reply
        .parts
        .iter()
        .position(|part| part.key == UNIDENTIFIED)
        .unwrap();
    assert_eq!(reply.series[0].runs[1].values[remainder], Some(9));
}

#[test]
fn air_force_unchanged_tables_obey_variant_read_dates_and_gaps() {
    use replay_data::{rows::LexicalTokenRow, rows::VariantDefinitionRow};
    let mut before = frame(19, &[(1, 100_000, 0, 8)], &[(1, 71, 800_000)], 2);
    *before.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"observed_air".to_vec(),
        }]));
    *before.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 71,
            equipment_token: 1,
            creator_link: 100_000,
            ..Default::default()
        }]));
    assert!(!dashboard_text::metadata::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &before.metrics,
        TableId::VariantDefinitions,
        19
    ));
    let mut read = before.clone();
    read.hour = 20;
    read.metrics.table_ns[TableId::VariantModules.index()] = 1;
    let mut held = before.clone();
    held.hour = 21;
    let mut after_gap = before.clone();
    after_gap.hour = 23;
    assert!(!dashboard_text::metadata::slot_read(
        replay_data::format::FormatVersion::CURRENT,
        &after_gap.metrics,
        TableId::VariantDefinitions,
        23
    ));
    let scanned = scan_frames(vec![before, read, held, after_gap]);
    let runs = &scanned.series[&tag(0)];
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs[0].roster.as_ref().unwrap().planes_by_token["71/"],
        Some(8)
    );
    assert_eq!(runs[1].first_hour, 20);
    assert_eq!(runs[1].last_hour, 21);
    assert_eq!(
        runs[1].roster.as_ref().unwrap().planes_by_token["71/observed_air"],
        Some(8)
    );
    assert_eq!(
        runs[2].roster.as_ref().unwrap().planes_by_token["71/"],
        Some(8)
    );
    assert!(runs[2].starts_segment);
    assert_eq!(scanned.segments[&tag(0)].len(), 2);
    let reply = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    let known = reply
        .parts
        .iter()
        .position(|part| part.key == ":variant/71/observed_air")
        .unwrap();
    assert_eq!(reply.series[0].runs[0].values[known], Some(0));
    assert_eq!(reply.series[0].runs[1].values[known], Some(8));
}

#[test]
fn air_force_ranking_clips_unknown_and_can_cancel() {
    let scanned = scan_frames(vec![
        frame(22, &[(1, 100_000, 0, 10)], &[(1, 71, 1_000_000)], 2),
        frame(23, &[(1, 100_001, 0, 20)], &[(1, 71, 2_000_000)], 2),
    ]);
    let mut params = params(AirForceMeasure::Planes, AirForceOwnership::Roster);
    params.tags.clear();
    params.top = Some(1);
    let reply = reply::reply(
        &scanned,
        &params,
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.series[0].runs[0].last_hour, 22);
    assert_eq!(reply.series[1].runs[0].first_hour, 23);
    assert!(
        reply::reply(
            &scanned,
            &params,
            None,
            |_| None,
            vec![],
            &AtomicBool::new(true)
        )
        .is_err()
    );
    params.top = Some(0);
    assert!(AirForceSize::validate(&params).is_err());
}

#[test]
fn air_force_manual_aliases_and_top_replies_keep_native_holder_history() {
    let frames = (22..=24)
        .map(|hour| {
            let mut frame = frame(hour, &[(1, 100_000, 0, 10)], &[], 2);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                if hour > 22 {
                    rows[0].tag = b"renamed air holder".to_vec();
                }
                if hour == 24 {
                    rows[1].tag = scale_tag(0);
                }
            }
            frame
        })
        .collect();
    let source = recorded_source(frames);
    let mut request = params(AirForceMeasure::Planes, AirForceOwnership::Roster);
    request.tags = vec![tag(0)];
    for top in [false, true] {
        if top {
            request.tags.clear();
            request.top = Some(1);
        }
        let output = AirForceSize::load(
            &source,
            &request,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        let old = output
            .series
            .iter()
            .find(|series| series.tag == tag(0))
            .unwrap();
        let renamed = output
            .series
            .iter()
            .find(|series| series.tag == "renamed air holder")
            .unwrap();
        assert_eq!(old.identities[0].last_hour, 22);
        assert_eq!(old.identities[1].country_id, 100_001);
        assert_eq!(old.identities[1].first_hour, 24);
        assert_eq!(renamed.identities[0].country_id, 100_000);
        assert_eq!(renamed.identities[0].slot, 0);
        assert_eq!(renamed.identities[0].last_hour, 24);
        assert_eq!(renamed.runs.last().unwrap().total, Some(10));
    }
}

#[test]
fn air_force_unsupported_country_tags_are_named_and_counted() {
    let mut unread = frame(22, &[], &[], 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *unread.tables[TableId::Countries.index()]
    {
        rows[0].tag = vec![255];
    }
    let source = recorded_source(vec![unread]);
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let reply = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("Country tag bytes [255]: 1 observations"))
    );
    assert_eq!(reply.series[0].tag, tag(1));
    assert_eq!(reply.series[0].runs[0].total, Some(0));
    let error = visit_plane_runs(
        &source,
        1,
        &[tag(1)],
        &AtomicBool::new(false),
        |_, _, _, _, _| panic!("undisplayable native tags must refuse before visiting"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("[255]: 1"));
}

#[test]
fn air_force_reply_budget_uses_actual_json_bytes_and_never_truncates() {
    let mut scanned = ScannedAir::default();
    let counts = Counts {
        planes: 1,
        planes_by_token: [(format!("1/{}", "x".repeat(MAX_REPLY_BYTES / 6)), Some(1))].into(),
        ..Default::default()
    };
    scanned.series.insert(
        tag(0),
        vec![RawRun {
            first_hour: 0,
            last_hour: 0,
            starts_segment: true,
            roster: Some(Arc::new(counts)),
            effective: None,
        }],
    );
    let output = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.parts.len(), 1);
    assert_eq!(output.parts[0].name.as_deref(), Some("Variant 1"));
    assert_eq!(output.series[0].runs[0].total, Some(1));
    assert_eq!(output.series[0].runs[0].values, vec![Some(1)]);
    assert!(transport::json_len(&output, MAX_REPLY_BYTES).is_some());
    let oversized = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        None,
        |_| None,
        vec!["x".repeat(MAX_REPLY_BYTES)],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(transport::json_len(&oversized, MAX_REPLY_BYTES).is_none());
}

#[test]
fn air_force_run_visitor_reuses_scan_and_keeps_zero_unknown_gap_and_replacement() {
    let mut unread = frame(23, &[], &[], 2);
    *unread.tables[TableId::AirWings.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let mut replacement = frame(26, &[(1, 200_000, 0, 7)], &[(1, 71, 700_000)], 2);
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *replacement.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries[0].id = 200_000;
    countries.sort_by_key(|row| row.id);
    let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *replacement.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    links[0].id = 200_000;
    links.sort_by_key(|row| row.id);
    let source = recorded_source(vec![
        frame(22, &[(1, 100_000, 100_001, 7)], &[(1, 71, 700_000)], 2),
        unread,
        frame(25, &[(1, 100_000, 0, 7)], &[(1, 71, 700_000)], 2),
        replacement,
    ]);
    let output = AirForceSize::load(
        &source,
        &params(AirForceMeasure::Wings, AirForceOwnership::Effective),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.series[0].runs[0].total, Some(0));
    assert_eq!(output.series[1].runs[0].total, Some(1));
    let tags = [tag(0), tag(1), "unobserved".into()];
    let mut seen = BTreeMap::<String, Vec<_>>::new();
    visit_plane_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(false),
        |tag, first, last, starts, value| {
            seen.entry(tag.to_owned())
                .or_default()
                .push((first, last, starts, value));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        seen[&tag(0)],
        [
            (22, 22, true, Some(7)),
            (23, 23, true, None),
            (25, 25, true, Some(7)),
            (26, 26, true, Some(7)),
        ]
    );
    assert_eq!(
        seen[&tag(1)],
        [
            (22, 22, true, Some(0)),
            (23, 23, true, None),
            (25, 26, true, Some(0)),
        ]
    );
    assert!(!seen.contains_key("unobserved"));
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 1);

    assert!(
        visit_plane_runs(
            &source,
            99,
            &tags,
            &AtomicBool::new(false),
            |_, _, _, _, _| { panic!("unknown branch must refuse before visiting") }
        )
        .is_err()
    );
    let error = visit_plane_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(true),
        |_, _, _, _, _| panic!("cancelled visitor must not call back"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(source.state::<Scans>().lock().unwrap().runs, 1);

    let cancelled = AtomicBool::new(false);
    let mut callbacks = 0;
    let error = visit_plane_runs(&source, 1, &tags, &cancelled, |_, _, _, _, _| {
        callbacks += 1;
        cancelled.store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(callbacks, 1);
    let error = visit_plane_runs(
        &source,
        1,
        &tags,
        &AtomicBool::new(false),
        |_, _, _, _, _| Err(io::Error::other("synthetic callback refusal")),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "synthetic callback refusal");
    let mut unresolved = frame(22, &[], &[], 2);
    *unresolved.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "unread country links".into(),
    };
    let cold = recorded_source(vec![unresolved]);
    let error = visit_plane_runs(&cold, 1, &tags, &AtomicBool::new(true), |_, _, _, _, _| {
        panic!("cancelled visitor must not call back")
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(cold.state::<Scans>().lock().unwrap().runs, 0);
    visit_plane_runs(&cold, 1, &tags, &AtomicBool::new(false), |_, _, _, _, _| {
        panic!("unavailable country links must not prove a zero or known count")
    })
    .unwrap();
    assert_eq!(cold.state::<Scans>().lock().unwrap().runs, 1);
}

#[test]
fn air_force_recorded_variants_use_mod_archetypes_and_keep_unresolved_designs() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::{rows::LexicalTokenRow, rows::VariantDefinitionRow};
    let root = std::env::temp_dir().join(format!("hoi4_air_force_mod_{}", uuid::Uuid::new_v4()));
    let base = root.join("base");
    let addon = root.join("mod");
    for layer in [&base, &addon] {
        std::fs::create_dir_all(layer.join("common/units/equipment")).unwrap();
    }
    std::fs::write(
        base.join("common/units/equipment/00_fixture.txt"),
        "equipments={removed_air={is_archetype=yes type=fighter}}",
    )
    .unwrap();
    std::fs::write(addon.join("common/units/equipment/00_fixture.txt"),
        "equipments={mod_air={is_archetype=yes type=fighter} mod_frame={archetype=mod_air parent=mod_air} mod_bomber={is_archetype=yes type=tactical_bomber}}" ).unwrap();
    let layers = installed_content_core::ResolvedLayers::from_folders(&[
        (&base, &[]),
        (&addon, &["common/units/equipment"]),
    ])
    .unwrap();
    let units = installed_content_core::equipment::Database::load_complete(&layers).unwrap();
    assert!(units.named("removed_air").is_none());
    let mut known = frame(
        22,
        &[(1, 100_000, 0, 10)],
        &[(1, 71, 400_000), (1, 72, 300_000), (1, 73, 300_000)],
        2,
    );
    *known.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"mod_frame".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"absent_mod_air".to_vec(),
            },
        ]));
    *known.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            VariantDefinitionRow {
                id: 71,
                equipment_token: 1,
                creator_link: 100_000,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 72,
                equipment_token: 2,
                creator_link: 100_000,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: 73,
                equipment_token: 2,
                creator_link: 100_000,
                ..Default::default()
            },
        ]));
    known.metrics.table_ns[TableId::VariantModules.index()] = 1;
    let scanned = scan_frames(vec![known]);
    let output = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Roster),
        Some(&units),
        |key| (key == "mod_air").then(|| "Modded planes".into()),
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.parts.len(), 3);
    let resolved = output
        .parts
        .iter()
        .position(|part| part.key == "mod_air")
        .unwrap();
    assert_eq!(
        output.parts[resolved].name.as_deref(),
        Some("Modded planes")
    );
    assert_eq!(output.series[0].runs[0].values[resolved], Some(4));
    assert!(
        output
            .parts
            .iter()
            .any(|part| part.key == ":variant/72/absent_mod_air")
    );
    assert!(
        output
            .parts
            .iter()
            .any(|part| part.key == ":variant/73/absent_mod_air")
    );
    let mixed_counts = Counts {
        wings: 1,
        wings_by_tokens: [(vec!["1/mod_frame".into(), "2/mod_bomber".into()], 1)].into(),
        ..Default::default()
    };
    let mut mixed = ScannedAir::default();
    mixed.series.insert(
        tag(0),
        vec![RawRun {
            first_hour: 22,
            last_hour: 22,
            starts_segment: true,
            roster: Some(Arc::new(mixed_counts)),
            effective: None,
        }],
    );
    let wings = reply::reply(
        &mixed,
        &params(AirForceMeasure::Wings, AirForceOwnership::Roster),
        Some(&units),
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(wings.parts.len(), 1);
    assert!(!wings.parts[0].unresolved);
    assert!(
        !wings.parts[0]
            .key
            .starts_with(dashboard_base::products::equipment::UNRESOLVED_MARK)
    );
    assert_eq!(
        wings.parts[0].name.as_deref(),
        Some("Mixed types: Equipment type name unavailable / Equipment type name unavailable")
    );
    drop(layers);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn air_force_wing_projection_merges_invisible_attrition_but_keeps_gaps() {
    let scanned = scan_frames(vec![
        frame(22, &[(1, 100_000, 0, 100)], &[(1, 71, 10_000_000)], 2),
        frame(23, &[(1, 100_000, 0, 90)], &[(1, 71, 9_000_000)], 2),
        frame(25, &[(1, 100_000, 0, 80)], &[(1, 71, 8_000_000)], 2),
    ]);
    let output = reply::reply(
        &scanned,
        &params(AirForceMeasure::Wings, AirForceOwnership::Roster),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(output.series[0].runs.len(), 2);
    assert_eq!(
        (
            output.series[0].runs[0].first_hour,
            output.series[0].runs[0].last_hour
        ),
        (22, 23)
    );
    assert!(output.series[0].runs[1].starts_segment);
}

#[test]
fn air_force_owner_diagnostics_follow_setting_and_count_changed_hours() {
    let scanned = scan_frames(vec![frame(
        22,
        &[(1, 999, 100_000, 2), (2, 999, 100_000, 3)],
        &[],
        2,
    )]);
    assert_eq!(scanned.unreadable_owners, [1, 0]);
    let output = reply::reply(
        &scanned,
        &params(AirForceMeasure::Planes, AirForceOwnership::Effective),
        None,
        |_| None,
        vec![],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        !output
            .warnings
            .iter()
            .any(|warning| warning.contains("unresolved ownership"))
    );
}

#[test]
fn air_force_cache_retains_two_scanned_branches_and_rebuilds_evicted_history() {
    let branches: Vec<_> = (1..=3)
        .map(|branch| {
            let mut frame = frame(22, &[], &[], 2);
            frame.branch = branch;
            records(branch, vec![frame])
        })
        .collect();
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "air-cache".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &branches).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    for branch in [1, 2, 3, 2] {
        let mut values = Vec::new();
        visit_plane_runs(
            &source,
            branch,
            &[tag(0)],
            &AtomicBool::new(false),
            |_, first, last, _, value| {
                assert_eq!((first, last), (22, 22));
                values.push(value);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(values, [Some(0)]);
    }
    let state = source.state::<Scans>();
    assert_eq!(state.lock().unwrap().runs, 3);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|(branch, _)| *branch)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    visit_plane_runs(
        &source,
        1,
        &[tag(0)],
        &AtomicBool::new(false),
        |_, _, _, _, _| Ok(()),
    )
    .unwrap();
    assert_eq!(state.lock().unwrap().runs, 4);
}

#[test]
fn air_force_sparse_updates_match_full_censuses_and_share_unchanged_counts() {
    let mut wings: Vec<_> = (1..=8)
        .map(|id| (id, 100_000 + u32::from(id > 4), 0, 100))
        .collect();
    let mut equipment: Vec<_> = wings
        .iter()
        .map(|&(id, _, _, planes)| (id, 71, i64::from(planes) * 100_000))
        .collect();
    let mut frames = vec![frame(22, &wings, &equipment, 2)];
    wings[0].3 = 103;
    equipment[0].2 = 10_300_000;
    // Unknown split evidence is reversible even while another wing names the same design.
    equipment[1].2 = 99_999;
    frames.push(frame(23, &wings, &equipment, 2));
    equipment[1].2 = 10_000_000;
    frames.push(frame(24, &wings, &equipment, 2));
    wings[2].2 = 100_001;
    wings.retain(|wing| wing.0 != 4);
    equipment.retain(|entry| entry.0 != 4);
    wings.push((200, 100_000, 0, 50));
    equipment.push((200, 72, 5_000_000));
    frames.push(frame(25, &wings, &equipment, 2));
    let scanned = scan_frames(frames.clone());
    assert_eq!(scanned.reconciled_wings, 13);
    for observed in frames {
        let hour = observed.hour;
        let rebuilt = scan_frames(vec![observed]);
        for country in 0..2 {
            for owner in [AirForceOwnership::Roster, AirForceOwnership::Effective] {
                let runs = &scanned.series[&tag(country)];
                let run = runs
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                    .unwrap();
                assert_eq!(
                    run.counts(owner),
                    rebuilt.series[&tag(country)][0].counts(owner)
                );
            }
        }
    }
    let untouched = &scanned.series[&tag(1)];
    assert_eq!((untouched[0].first_hour, untouched[0].last_hour), (22, 24));
    assert!(Arc::ptr_eq(
        untouched[0].roster.as_ref().unwrap(),
        untouched[1].roster.as_ref().unwrap()
    ));
}

#[test]
fn air_force_country_read_failure_rebuilds_units_changed_during_the_missing_hour() {
    let before = frame(22, &[(1, 100_000, 0, 10)], &[(1, 71, 1_000_000)], 2);
    let mut missing = frame(23, &[(1, 100_000, 0, 11)], &[(1, 71, 1_100_000)], 2);
    *missing.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "unread country census".into(),
    };
    let after = frame(24, &[(1, 100_000, 0, 11)], &[(1, 71, 1_100_000)], 2);
    let scanned = scan_frames(vec![before, missing, after]);
    assert_eq!(at(&scanned, 0, 22, AirForceOwnership::Roster), Some(10));
    assert_eq!(at(&scanned, 0, 23, AirForceOwnership::Roster), None);
    assert_eq!(at(&scanned, 0, 24, AirForceOwnership::Roster), Some(11));
    assert!(scanned.series[&tag(0)].last().unwrap().starts_segment);
}

#[test]
fn air_force_sparse_histories_visit_only_affected_countries() {
    let wings = [(1, 100_000, 100_000, 100)];
    let equipment = [(1, 71, 10_000_000)];
    let changed = [(1, 100_000, 100_000, 101)];
    let bank = [(1, 71, 10_100_000)];
    let frames = vec![
        frame(22, &wings, &equipment, 5000),
        frame(23, &wings, &equipment, 5000),
        frame(24, &changed, &bank, 5000),
        frame(25, &changed, &bank, 5000),
        frame(26, &[(1, 100_001, 100_000, 101)], &bank, 5000),
    ];
    let scanned = scan_frames(frames);
    // Initial catalogue, one reinforcement owner and two transfer owners.
    assert_eq!(scanned.projected_countries, 5000 + 1 + 2);
    assert_eq!(at(&scanned, 0, 23, AirForceOwnership::Roster), Some(100));
    assert_eq!(at(&scanned, 0, 25, AirForceOwnership::Roster), Some(101));
    assert_eq!(at(&scanned, 0, 26, AirForceOwnership::Roster), Some(0));
    assert_eq!(at(&scanned, 1, 26, AirForceOwnership::Roster), Some(101));
    assert_eq!(at(&scanned, 0, 26, AirForceOwnership::Effective), Some(101));
    for country in 2..5000 {
        let runs = &scanned.series[&tag(country)];
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].first_hour, runs[0].last_hour), (22, 26));
        assert_eq!(
            scanned.segments[&tag(country)],
            vec![HourSpan {
                first_hour: 22,
                last_hour: 26,
            }],
        );
    }
}

#[test]
fn air_force_deferred_histories_stop_at_absence_unknown_reads_and_gaps() {
    let known = |hour| frame(hour, &[(1, 100_000, 0, 7)], &[(1, 71, 700_000)], 2);
    let mut unknown = known(28);
    *unknown.tables[TableId::AirWings.index()] = TableView::Unavailable {
        reason: "unread wings".into(),
    };
    let frames = vec![
        known(22),
        known(23),
        frame(24, &[(1, 100_000, 0, 8)], &[(1, 71, 800_000)], 2),
        frame(25, &[], &[], 1), // country 1 disappears; country 0 loses its last wing
        known(26),
        known(27),
        unknown,
        known(29),
        known(31),
        known(32),
    ];
    let scanned = scan_frames(frames.clone());
    for observed in frames {
        let hour = observed.hour;
        let rebuilt = scan_frames(vec![observed]);
        for country in 0..2 {
            for owner in [AirForceOwnership::Roster, AirForceOwnership::Effective] {
                let expected = rebuilt
                    .series
                    .get(&tag(country))
                    .and_then(|runs| runs.first())
                    .and_then(|run| run.counts(owner));
                let actual = scanned.series[&tag(country)]
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                    .and_then(|run| run.counts(owner));
                assert_eq!(actual, expected, "country={country} hour={hour}");
            }
        }
    }
    assert_eq!(at(&scanned, 1, 25, AirForceOwnership::Roster), None);
    assert_eq!(at(&scanned, 0, 30, AirForceOwnership::Roster), None);
    for hour in [29, 31] {
        assert!(
            scanned.series[&tag(0)]
                .iter()
                .find(|run| run.first_hour == hour)
                .unwrap()
                .starts_segment
        );
    }
}

#[test]
fn air_force_full_snapshot_marks_owners_that_lost_their_last_wing() {
    let before = frame(22, &[(1, 100_000, 0, 7)], &[(1, 71, 700_000)], 2);
    let mut encoded = records(1, vec![before]);
    // A continuous full acquisition has no deleted-row keys to dirty this owner.
    encoded.push(replay::spool::Record::Frame(
        replay_frame::frame::FrameEncoder::new()
            .encode(frame(23, &[], &[], 2), true)
            .unwrap(),
    ));
    let scanned = scan::scan(
        &Replay::from_records(CAMPAIGN.into(), &[encoded]).unwrap(),
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    for owner in [AirForceOwnership::Roster, AirForceOwnership::Effective] {
        assert_eq!(at(&scanned, 0, 22, owner), Some(7));
        assert_eq!(at(&scanned, 0, 23, owner), Some(0));
    }
}

#[test]
fn phase_b_air_database_and_spelling_refusals_resolve_once_per_binding() {
    for removed in [false, true] {
        let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
            "common/units/equipment/air.txt",
            "equipments={mod_air={is_archetype=yes}}",
        )]);
        let raw = frame(100, &[(1, 100_000, 0, 7)], &[(1, 1, 700_000)], 1);
        let source = recorded_source(vec![raw.clone()]);
        let catalogue = Arc::new(catalogue);
        if removed {
            std::fs::remove_file(folder.0.join("common/units/equipment/air.txt")).unwrap();
        }
        let configured = dashboard_base::products::equipment::units(&source, &catalogue);
        assert_eq!(configured.is_ok(), !removed);
        let binding = part_content(&source, Some(&catalogue));
        let mut scanned = scan_frames(vec![raw]);
        // The native run keeps the recorded spelling, including its exact refusal.
        for runs in scanned.series.values_mut() {
            for run in runs {
                let counts = Arc::make_mut(run.roster.as_mut().unwrap());
                counts.planes_by_token = BTreeMap::from([("1/mod_air".into(), Some(7))]);
                counts.wings_by_tokens = BTreeMap::from([(vec!["1/mod_air".into()], 1)]);
            }
        }
        let params = params(AirForceMeasure::Planes, AirForceOwnership::Roster);
        let expected = reply::reply_cached(
            &scanned,
            &params,
            configured.as_ref().ok().map(Arc::as_ref),
            |_| None,
            Vec::new(),
            &AtomicBool::new(false),
            Some(&binding),
        )
        .unwrap();
        for _ in 0..10 {
            let units = dashboard_base::products::equipment::units(&source, &catalogue);
            assert_eq!(units.is_ok(), configured.is_ok());
            let actual = reply::reply_cached(
                &scanned,
                &params,
                units.as_ref().ok().map(Arc::as_ref),
                |_| None,
                Vec::new(),
                &AtomicBool::new(false),
                Some(&binding),
            )
            .unwrap();
            dashboard_base::prepared::observer_tests::assert_reply_parity(&actual, &expected)
                .unwrap();
        }
        assert_eq!(
            dashboard_base::products::equipment::unit_loads(&catalogue),
            1
        );
        assert_eq!(binding.tokens.loads(&"1/mod_air".to_owned()), 1);
        assert_eq!(binding.signatures.loads(&vec!["1/mod_air".into()]), 1);
    }
}

#[test]
fn restored_air_forgets_variant_names_that_stop_being_known_after_a_commit() {
    use replay_data::{rows::LexicalTokenRow, rows::VariantDefinitionRow};
    // Hour 22 is the variant table's daily read; its names hold through the
    // commit after hour 25. From hour 27 the variant table is unavailable, so
    // the names stop being known before any further read.
    let frames: Vec<_> = (22..30)
        .map(|hour| {
            let mut next = frame(hour, &[(1, 100_000, 0, 8)], &[(1, 71, 800_000)], 2);
            *next.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 1,
                    literal: b"observed_air".to_vec(),
                }]));
            *next.tables[TableId::VariantDefinitions.index()] = if hour < 27 {
                TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                    id: 71,
                    equipment_token: 1,
                    creator_link: 100_000,
                    ..Default::default()
                }]))
            } else {
                TableView::Unavailable {
                    reason: "synthetic incomplete variant read".into(),
                }
            };
            next
        })
        .collect();
    let source = recorded_source(frames.clone());
    let replay = source.replay_ref();
    for frame in &frames {
        assert_eq!(
            dashboard_text::metadata::slot_read(
                replay.format,
                &frame.metrics,
                TableId::VariantDefinitions,
                frame.hour
            ),
            frame.hour == 22
        );
    }
    // Observed without a restore, the names end at hour 27.
    let scanned = scan_frames(frames.clone());
    let last = scanned.series[&tag(0)].last().unwrap();
    assert_eq!(last.first_hour, 27);
    let counts = last.roster.as_ref().unwrap();
    assert_eq!(counts.planes_by_token["71/"], Some(8));
    assert!(!counts.planes_by_token.contains_key("71/observed_air"));
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let (first, second) = frames.split_at(4);
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        replay.format,
        &[first, second],
        prepared::Prepared::default,
        |observer, frame, changes, _| {
            // Before a block, preparation configures both the continued and
            // the restored observer and resumes them from the committed frame.
            if frame.hour == second[0].hour {
                observer.configure(&source, &context);
                observer
                    .resume(first.last().unwrap(), replay, &flag)
                    .unwrap();
            }
            observer
                .observe(frame, replay, &source, &context, &flag, changes)
                .unwrap();
        },
        |observer, writer, _| {
            observer.flush(writer, 1, &flag).unwrap();
            writer
                .write_at(
                    "air-continuation-1",
                    0,
                    serde_json::to_vec(&*observer).unwrap().as_slice(),
                )
                .unwrap();
        },
        |fixture| {
            serde_json::from_reader(std::io::BufReader::new(
                fixture
                    .store
                    .read(&fixture.key, "air-continuation-1", 0)
                    .unwrap()
                    .unwrap(),
            ))
            .unwrap()
        },
    );
}
