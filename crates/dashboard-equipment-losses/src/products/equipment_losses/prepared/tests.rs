use super::columnar_parity::Driver;
use super::*;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_core::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag};
use replay_data::{
    rows::EquipmentDestructionRow, rows::SunkShipEntry, rows::SunkShipsRow, rows::TableRows,
};
use replay_frame::frame::TableView;

#[test]
fn prepared_loss_contract_names_land_coverage_separately_from_the_prefix() {
    let declaration = <PreparedLossesReply as ts_rs::TS>::decl();
    assert!(declaration.contains("landCoverage:"));
    assert!(!declaration.contains("LossesReply &"));
}

/// The published key of `country`'s leaf.
fn leaf_key(prepared: &Prepared, country: Identity, leaf: LeafKey) -> u64 {
    let country = &prepared.countries[&country];
    country.names[country.leaves[&leaf] as usize]
}

/// Content whose Combat Log retention deletes no entry that can still change.
fn retained() -> Content<'static> {
    Content {
        retained_months: Some(12),
        ..Content::default()
    }
}

fn empty_drain(frame: &mut DecodedFrame) {
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(Vec::new()));
}

#[test]
fn lend_lease_deletion_ends_only_current_owners_exchanges_through_a_resume() -> io::Result<()> {
    use dashboard_base::prepared::observer_tests::assert_reply_parity;
    use replay_data::rows::LendLeaseEquipmentRow;
    for history in [2_usize, 10] {
        let mut seed = frame(10, history as u32, 0, false);
        empty_drain(&mut seed);
        *seed.tables[TableId::LendLeaseEquipment.index()] =
            TableView::Complete(TableRows::LendLeaseEquipment(
                (0..history as u32)
                    .map(|index| LendLeaseEquipmentRow {
                        giver: 100_000 + index,
                        receiver: 100_000,
                        exchange: index + 1,
                        kind: 1,
                        variant: 17,
                        amount_raw: 123_456,
                    })
                    .collect(),
            ));
        seed.metrics.table_ns[TableId::LendLeaseEquipment.index()] = 1;
        let mut first = seed.clone();
        first.hour = 11;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *first.tables[TableId::Countries.index()]
        {
            rows.truncate(1);
        }
        if let TableView::Complete(TableRows::LendLeaseEquipment(rows)) =
            &mut *first.tables[TableId::LendLeaseEquipment.index()]
        {
            rows.truncate(1);
        }
        let mut warm = first.clone();
        warm.hour = 12;
        let mut deleted = warm.clone();
        deleted.hour = 13;
        *deleted.tables[TableId::LendLeaseEquipment.index()] =
            TableView::Complete(TableRows::LendLeaseEquipment(Vec::new()));
        let frames = [seed, first, warm, deleted];
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())])?;
        let flag = AtomicBool::new(false);
        let mut results = Vec::new();
        for resume in [false, true] {
            let mut prepared = Prepared::new(&replay, 1, 10);
            let mut driver = Driver::new(&replay, 10)?;
            let mut fixture = Fixture::new();
            for (ordinal, chunk) in [&frames[..2], &frames[2..3], &frames[3..]]
                .into_iter()
                .enumerate()
            {
                for frame in chunk {
                    driver.observe(&mut prepared, frame)?;
                }
                let mut writer = if ordinal == 2 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                prepared.flush(&mut writer, 1, &flag)?;
                writer.commit()?;
                if resume && ordinal == 0 {
                    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
                    driver.restart();
                }
            }
            assert_eq!(prepared.countries.len(), history);
            assert_eq!(prepared.countries[&(0, 100_000)].scanned.ended[0].from, 13);
            let mut replies = Vec::new();
            for index in [0, history as u32 - 1] {
                for land_source in [LandSource::Destruction, LandSource::CombatLog] {
                    let selection = LossesParams {
                        branch_id: "1".into(),
                        tag: String::from_utf8(scale_tag(index)).unwrap(),
                        land_source,
                        country: Some(LossCountry {
                            slot: index,
                            country_id: 100_000 + index,
                            origin_hour: 10,
                        }),
                    };
                    let expected = scan(
                        &replay,
                        1,
                        &selection.tag,
                        land_source,
                        selection.country,
                        &flag,
                    )?;
                    let actual = read_scanned(
                        &fixture.store,
                        &fixture.key,
                        1,
                        (index, 100_000 + index),
                        (10.0, 13.0, 10),
                        &[10, 11, 12, 13],
                        replay.format,
                        &flag,
                    )?
                    .unwrap();
                    let actual = reply(&selection, &actual, retained(), Vec::new());
                    assert_reply_parity(
                        &actual,
                        &reply(&selection, &expected, retained(), Vec::new()),
                    )?;
                    replies.push(serde_json::to_value(actual)?);
                }
            }
            results.push(replies);
        }
        assert_reply_parity(&results[0], &results[1])?;
    }
    Ok(())
}

#[test]
fn deferred_loss_values_and_exchange_read_dates_match_oracle_through_commits_resume_append_and_gap()
-> io::Result<()> {
    use dashboard_base::prepared::observer_tests::assert_reply_parity;
    use replay_data::rows::LendLeaseEquipmentRow;
    let mut first = frame(10, 2, 100_001, false);
    *first.tables[TableId::LendLeaseEquipment.index()] =
        TableView::Complete(TableRows::LendLeaseEquipment(vec![LendLeaseEquipmentRow {
            giver: 100_000,
            exchange: 1,
            kind: 1,
            variant: 17,
            amount_raw: 123_456,
            receiver: 100_001,
        }]));
    first.metrics.table_ns[TableId::LendLeaseEquipment.index()] = 1;
    let mut frames = Vec::new();
    for hour in 10..=34 {
        if hour == 30 {
            continue;
        }
        let mut next = first.clone();
        next.hour = hour;
        if hour > 10 {
            empty_drain(&mut next);
        }
        if hour >= 22 {
            *next.tables[TableId::LendLeaseEquipment.index()] =
                TableView::Complete(TableRows::LendLeaseEquipment(Vec::new()));
        }
        frames.push(next);
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())])?;
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10)?;
    let mut fixture = Fixture::new();
    for (ordinal, chunk) in frames.chunks(8).enumerate() {
        for frame in chunk {
            driver.observe(&mut prepared, frame)?;
        }
        let mut writer = if ordinal > 1 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
        driver.restart();
    }
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    for land_source in [LandSource::Destruction, LandSource::CombatLog] {
        let selection = LossesParams {
            branch_id: "1".into(),
            tag: tag.clone(),
            land_source,
            country: Some(LossCountry {
                slot: 0,
                country_id: 100_000,
                origin_hour: 10,
            }),
        };
        let reference = scan(&replay, 1, &tag, land_source, selection.country, &flag)?;
        let actual = read_scanned(
            &fixture.store,
            &fixture.key,
            1,
            (0, 100_000),
            (10.0, 34.0, 100),
            &(10..=34).collect::<Vec<_>>(),
            replay.format,
            &flag,
        )?
        .unwrap();
        assert_eq!(
            actual.ended[0].from, 22,
            "equal supplying reads move the exchange ending date"
        );
        assert_reply_parity(
            &reply(&selection, &actual, retained(), Vec::new()),
            &reply(&selection, &reference, retained(), Vec::new()),
        )?;
        let leaf = leaf_key(
            &prepared,
            (0, 100_000),
            (
                LossSource::Destruction(EquipmentDestructionCause::LandCombat),
                Some(17),
            ),
        );
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            "equipment-loss-values-1".into(),
            &flag,
        )?;
        assert_eq!(reader.value_at(leaf, 29)?.unwrap().value, 100_001);
        assert!(reader.value_at(leaf, 30)?.is_none());
        assert_eq!(reader.value_at(leaf, 34)?.unwrap().value, 100_001);
    }
    Ok(())
}

#[test]
fn changed_carried_variant_operands_wait_for_the_first_real_read_including_after_resume()
-> io::Result<()> {
    let hour = (10..100)
        .find(|hour| !replay_data::cadence::is_refresh(TableId::VariantDefinitions, hour + 1))
        .unwrap();
    let first = dashboard_core::test_fixtures::named_frame(hour, Some(b"mod old name"));
    let mut carried = first.clone();
    carried.hour += 1;
    carried.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *carried.tables[TableId::VariantDefinitions.index()]
    {
        rows[0].display_name = b"mod new name".to_vec();
    }
    let mut read = carried.clone();
    read.hour += 1;
    read.metrics.table_ns[TableId::VariantModules.index()] = 1;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![first.clone(), carried.clone(), read.clone()],
        )],
    )?;
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, hour);
    let mut driver = Driver::new(&replay, hour)?;
    let mut oracle = VariantCatalogue::default();
    for frame in [&first, &carried] {
        driver.observe(&mut prepared, frame)?;
        oracle.observe(replay.format, frame);
    }
    assert_eq!(
        prepared.variants.current(20).unwrap().display_name,
        "mod old name"
    );
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag)?;
    writer.commit()?;
    let mut resumed: Prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
    let mut going = Driver::new(&replay, read.hour)?;
    driver.observe(&mut prepared, &read)?;
    going.observe(&mut resumed, &read)?;
    oracle.observe(replay.format, &read);
    assert_eq!(
        prepared.variants.current(20).unwrap().display_name,
        "mod new name"
    );
    dashboard_base::prepared::observer_tests::assert_reply_parity(&prepared.variants, &oracle)?;
    dashboard_base::prepared::observer_tests::assert_reply_parity(&resumed.variants, &oracle)?;
    Ok(())
}

#[test]
fn cancelling_log_deltas_keep_the_supplying_boundary_then_carry_through_publication()
-> io::Result<()> {
    use replay_data::{rows::CombatLogEquipmentRow, rows::SliceReadRow};
    let mut first = frame(10, 1, 100_001, false);
    empty_drain(&mut first);
    *first.tables[TableId::CombatLogEquipment.index()] =
        TableView::Complete(TableRows::CombatLogEquipment(
            [(1, 100_000), (2, 200_000)]
                .into_iter()
                .map(|(group, amount_raw)| CombatLogEquipmentRow {
                    country: 100_000,
                    group,
                    variant: 17,
                    amount_raw,
                    ..Default::default()
                })
                .collect(),
        ));
    *first.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::CombatLogEquipment.index() as u32,
            owner: 100_000,
            sample_hour: 10,
        }]));
    let mut changed = first.clone();
    changed.hour = 11;
    if let TableView::Complete(TableRows::CombatLogEquipment(rows)) =
        &mut *changed.tables[TableId::CombatLogEquipment.index()]
    {
        rows[0].amount_raw = 150_000;
        rows[1].amount_raw = 150_000;
    }
    if let TableView::Complete(TableRows::SliceReads(rows)) =
        &mut *changed.tables[TableId::SliceReads.index()]
    {
        rows[0].sample_hour = 11;
    }
    let mut carried = changed.clone();
    carried.hour = 12;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![first.clone(), changed.clone(), carried.clone()],
        )],
    )?;
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10)?;
    driver.observe(&mut prepared, &first)?;
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag)?;
    writer.commit()?;
    driver.observe(&mut prepared, &changed)?;
    driver.observe(&mut prepared, &carried)?;
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag)?;
    writer.commit()?;
    let actor = leaf_key(&prepared, (0, 100_000), (LossSource::CombatLog(0), Some(17)));
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "equipment-loss-values-1".into(),
        &flag,
    )?;
    let samples = points(&sampled_events(&mut reader, actor, 10.0, 12.0, 10, &[10])?);
    assert_eq!(
        samples.last().unwrap().hour,
        11,
        "a cancelling supplying read remains visible without a reached read"
    );
    assert_eq!(reader.value_at(actor, 12)?.unwrap().value, 300_000);
    Ok(())
}

#[test]
fn a_known_holder_with_an_unresolved_new_link_keeps_unknown_ownership_not_absence() -> io::Result<()>
{
    let first = frame(10, 1, 100_001, false);
    let mut next = first.clone();
    next.hour = 11;
    *next.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(Vec::new()));
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first.clone(), next.clone()])],
    )?;
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10)?;
    driver.observe(&mut prepared, &first)?;
    driver.observe(&mut prepared, &next)?;
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag)?;
    writer.commit()?;
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let selection = LossesParams {
        branch_id: "1".into(),
        tag: tag.clone(),
        land_source: LandSource::Destruction,
        country: Some(LossCountry {
            slot: 0,
            country_id: 100_000,
            origin_hour: 10,
        }),
    };
    let reference = scan(
        &replay,
        1,
        &tag,
        selection.land_source,
        selection.country,
        &flag,
    )?;
    let actual = read_scanned(
        &fixture.store,
        &fixture.key,
        1,
        (0, 100_000),
        (10.0, 11.0, 2),
        &[10, 11],
        replay.format,
        &flag,
    )?
    .unwrap();
    assert_eq!(
        actual.destruction_unknown[0].reason,
        UnknownReason::DestructionUnresolved
    );
    dashboard_base::prepared::observer_tests::assert_reply_parity(
        &reply(&selection, &actual, retained(), Vec::new()),
        &reply(&selection, &reference, retained(), Vec::new()),
    )?;
    Ok(())
}

fn frame(hour: i32, count: u32, amount: i64, ships: bool) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(
            (0..count)
                .map(|index| EquipmentDestructionRow {
                    country: 100_000 + index,
                    cause: EquipmentDestructionCause::LandCombat,
                    variant: 17_u64 << 32 | u64::from(VARIANT_TYPE),
                    amount_raw: amount,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: ships
                .then(|| SunkShipEntry {
                    ship_name: b"mod ship".to_vec(),
                    victim_country: 100_000,
                    sink_hour: 10,
                    victim_equipment_variant: 17_u64 << 32 | u64::from(VARIANT_TYPE),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
        }]));
    frame
}

#[test]
fn prepared_loss_points_match_reference_after_commits_resume_append_and_backdated_ship() {
    let mut frames = vec![
        frame(10, 2, 100_001, false),
        frame(11, 2, -50_000, false),
        frame(12, 2, 25_000, true),
        frame(14, 2, 75_000, true),
        frame(15, 2, -25_000, true),
        frame(16, 2, 50_000, true),
    ];
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frames[3].tables[TableId::Countries.index()]
    {
        countries[0].tag = b"mod renamed holder".to_vec();
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10).unwrap();
    for (ordinal, chunk) in frames.chunks(2).enumerate() {
        for frame in chunk {
            driver.observe(&mut prepared, frame).unwrap();
        }
        prepared.finish_block(chunk.last().unwrap().hour);
        let mut writer = if ordinal == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        assert!(
            prepared
                .countries
                .values()
                .all(|country| country.scanned.events.is_empty())
        );
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        driver.restart();
    }
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    for land_source in [LandSource::Destruction, LandSource::CombatLog] {
        let selection = LossesParams {
            branch_id: "1".into(),
            tag: tag.clone(),
            land_source,
            country: Some(LossCountry {
                slot: 0,
                country_id: 100_000,
                origin_hour: 10,
            }),
        };
        let reference = scan(&replay, 1, &tag, land_source, selection.country, &flag).unwrap();
        let reads: Vec<_> = (9..=16).collect();
        let actual = read_scanned(
            &fixture.store,
            &fixture.key,
            1,
            (0, 100_000),
            (10.0, 16.0, 20),
            &reads,
            replay.format,
            &flag,
        )
        .unwrap()
        .unwrap();
        let expected = reply(&selection, &reference, retained(), Vec::new());
        let actual = reply(&selection, &actual, retained(), Vec::new());
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{land_source:?}"
        );
    }
    let scanned = read_scanned(
        &fixture.store,
        &fixture.key,
        1,
        (0, 100_000),
        (10.0, 10.0, 1),
        &[10],
        replay.format,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        points(&scanned.events[&(LossSource::SunkShips, Some(17))])[0],
        LossPoint {
            hour: 10,
            total_raw: AMOUNT_SCALE.to_string()
        }
    );
}

/// Before the family's coverage completes, a request answers its prepared
/// prefix: each reached hour after the prefix, and the viewport, end at the
/// prefix's last hour, and the prefix's pages answer that request as the
/// complete pages do. Once complete, the request is answered as asked.
#[test]
fn a_request_past_the_prepared_prefix_answers_at_its_last_hour_as_complete_pages_do() {
    use dashboard_base::{
        prepared::coverage::{Coverage, SCHEMA},
        products::country_groups::HourSpan,
    };
    let frames = vec![
        frame(10, 2, 100_001, false),
        frame(11, 2, -50_000, false),
        frame(12, 2, 25_000, false),
        frame(14, 2, 75_000, false),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let flag = AtomicBool::new(false);
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let selection = LossesParams {
        branch_id: "1".into(),
        tag,
        land_source: LandSource::Destruction,
        country: Some(LossCountry {
            slot: 0,
            country_id: 100_000,
            origin_hour: 10,
        }),
    };
    let asked = Params {
        selection: selection.clone(),
        from: Some(10.0),
        to: Some(14.0),
        width: Some(20),
        reads: vec![10, 12, 14],
        head_hour: 14,
        detail_hour: 12,
    };
    let coverage = |last_hour, complete| Coverage {
        schema: SCHEMA,
        first_hour: 10,
        last_hour,
        complete,
    };
    let mut params = Params {
        selection: selection.clone(),
        reads: asked.reads.clone(),
        ..asked
    };
    let prefix = Prefix {
        coverage: coverage(11, false),
        pending: vec![HourSpan {
            first_hour: 12,
            last_hour: 14,
        }],
    };
    let (from, to) = through_prefix(&mut params, (10.0, 14.0), &prefix);
    assert_eq!((from, to), (10.0, 11.0));
    assert_eq!(params.reads, [10, 11]);
    assert_eq!((params.head_hour, params.detail_hour), (11, 11));
    let read = |fixture: &Fixture| {
        let scanned = read_scanned(
            &fixture.store,
            &fixture.key,
            1,
            (0, 100_000),
            (from, to, 20),
            &params.reads,
            replay.format,
            &flag,
        )
        .unwrap()
        .unwrap();
        serde_json::to_value(reply(&selection, &scanned, retained(), Vec::new())).unwrap()
    };
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10).unwrap();
    let mut cold = None;
    for chunk in frames.chunks(2) {
        for frame in chunk {
            driver.observe(&mut prepared, frame).unwrap();
        }
        prepared.finish_block(chunk.last().unwrap().hour);
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        cold.get_or_insert_with(|| read(&fixture));
    }
    assert_eq!(cold.unwrap(), read(&fixture));
    let mut whole = Params {
        selection,
        reads: asked.reads.clone(),
        ..asked
    };
    let complete = Prefix {
        coverage: coverage(14, true),
        pending: Vec::new(),
    };
    assert_eq!(
        through_prefix(&mut whole, (10.0, 14.0), &complete),
        (10.0, 14.0)
    );
    assert_eq!(whole.reads, [10, 12, 14]);
    assert_eq!((whole.head_hour, whole.detail_hour), (14, 12));
}

#[test]
fn only_loss_designs_publish_naming_keys_and_names_resume_exactly() {
    use dashboard_base::products::equipment::{RawVariant, VariantVersion};
    let frames = vec![frame(10, 2, 100_000, false), frame(11, 2, 100_000, false)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10).unwrap();
    for frame in &frames {
        driver.observe(&mut prepared, frame).unwrap();
    }
    for variant in [17_u32, 18] {
        let handle = u64::from(variant) << 32 | u64::from(VARIANT_TYPE);
        prepared.variants.versions.insert(
            handle,
            vec![VariantVersion {
                first_hour: 10,
                raw: Some(RawVariant {
                    id: handle,
                    first_hour: 10,
                    display_name: format!("mod design {variant}"),
                    refusals: vec![("design".into(), "synthetic unread modules".into())],
                    ..Default::default()
                }),
            }],
        );
    }
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    assert_eq!(
        prepared.keys.len(),
        5,
        "two holders, two loss leaves and one loss design"
    );
    let handle = 17_u64 << 32 | u64::from(VARIANT_TYPE);
    let scanned = read_scanned(
        &fixture.store,
        &fixture.key,
        1,
        (0, 100_000),
        (10.0, 11.0, 2),
        &[10, 11],
        replay.format,
        &flag,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        scanned.variants.versions[&handle],
        prepared.variants.versions[&handle]
    );
    assert!(!prepared.keys.contains_key(
        &serde_json::to_string(&("variant", 18_u64 << 32 | u64::from(VARIANT_TYPE))).unwrap()
    ));
}

#[test]
fn sampled_losses_keep_the_first_native_date_when_later_extrema_hide_it() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (hour, value) in [(10, 1), (11, 100), (12, -100), (13, 200)] {
        values.push(0, Run::new(hour, hour, value, hour == 10));
        values
            .event(
                0,
                hour,
                &LossPoint {
                    hour,
                    total_raw: value.to_string(),
                },
            )
            .unwrap();
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "loss", 5, &flag).unwrap();
    writer.commit().unwrap();
    let mut reader = Reader::new(&fixture.store, &fixture.key, "loss".into(), &flag).unwrap();
    let sampled = points(&sampled_events(&mut reader, 0, 12.0, 13.0, 1, &[9, 13]).unwrap());
    assert_eq!(
        sampled.first().unwrap(),
        &LossPoint {
            hour: 10,
            total_raw: "1".into()
        }
    );
    assert_eq!(sampled.last().unwrap().total_raw, "200");
}

#[test]
fn equal_signed_total_keeps_its_latest_loss_date_across_a_gap() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut values = Accumulator::default();
    for (first, last) in [(10, 11), (14, 15)] {
        values.push(0, Run::new(first, last, 100_001, true));
        if first == 10 {
            values
                .event(
                    0,
                    first,
                    &LossPoint {
                        hour: first,
                        total_raw: "100001".into(),
                    },
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        values.flush(&mut writer, "loss", 5, &flag).unwrap();
        writer.commit().unwrap();
    }
    let mut reader = Reader::new(&fixture.store, &fixture.key, "loss".into(), &flag).unwrap();
    assert_eq!(
        points(&sampled_events(&mut reader, 0, 14.0, 15.0, 1, &[15]).unwrap()),
        vec![LossPoint {
            hour: 10,
            total_raw: "100001".into()
        }]
    );
}

#[test]
fn retained_holders_are_retagged_while_identity_tables_are_incomplete_and_at_their_recovery()
-> io::Result<()> {
    use dashboard_base::prepared::observer_tests::assert_reply_parity;
    // Holder 1 leaves the census at hour 12. CountryLinks is unread at hour 14,
    // the first hour after a commit, and read again at hour 15, the first hour
    // after the next commit. Every hour has a nonempty destruction drain.
    let frames: Vec<_> = (10..=16)
        .map(|hour| {
            let mut next = frame(hour, if hour < 12 { 2 } else { 1 }, 100_001, false);
            if hour == 14 {
                *next.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                    reason: "fixture links unread".into(),
                };
            }
            next
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())])?;
    let flag = AtomicBool::new(false);
    let blocks = [&frames[..4], &frames[4..5], &frames[5..]];
    // The resumed run publishes the oracle's unresolved hour, and its end at the recovery
    // hour, for the departed and the current holder.
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1, 10);
    let mut driver = Driver::new(&replay, 10)?;
    for block in blocks {
        for frame in block {
            driver.observe(&mut prepared, frame)?;
        }
        prepared.finish_block(block.last().unwrap().hour);
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
        driver.restart();
    }
    for index in [0, 1] {
        for land_source in [LandSource::Destruction, LandSource::CombatLog] {
            let selection = LossesParams {
                branch_id: "1".into(),
                tag: String::from_utf8(scale_tag(index)).unwrap(),
                land_source,
                country: Some(LossCountry {
                    slot: index,
                    country_id: 100_000 + index,
                    origin_hour: 10,
                }),
            };
            let expected = scan(
                &replay,
                1,
                &selection.tag,
                land_source,
                selection.country,
                &flag,
            )?;
            let actual = read_scanned(
                &fixture.store,
                &fixture.key,
                1,
                (index, 100_000 + index),
                (10.0, 16.0, 10),
                &(10..=16).collect::<Vec<_>>(),
                replay.format,
                &flag,
            )?
            .unwrap();
            assert_reply_parity(
                &reply(&selection, &actual, retained(), Vec::new()),
                &reply(&selection, &expected, retained(), Vec::new()),
            )?;
        }
    }
    Ok(())
}

#[test]
fn country_rename_between_variant_reads_survives_balance_only_deltas() -> io::Result<()> {
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let first = dashboard_core::test_fixtures::slot_hour(TableId::VariantDefinitions);
    let last = first + replay_data::cadence::STAGGER_PERIOD;
    let mut input = dashboard_core::test_fixtures::named_frame(first, Some(b"mod design"));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"AAA".to_vec();
    }
    // The creator is not dynamic, so its own tag names the design.
    *input.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 9001,
                ..Default::default()
            },
        ]));
    let replay = dashboard_core::test_fixtures::replay(vec![input.clone()]);
    let mut prepared = Prepared::new(&replay, 1, first);
    let mut driver = Driver::new(&replay, first)?;
    let mut oracle = VariantCatalogue::default();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in first..=last {
        input.hour = hour;
        input.metrics.table_ns.fill(0);
        if hour == first || hour == last {
            input.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
        }
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *input.tables[TableId::Countries.index()]
        {
            if hour == first + 1 {
                rows[0].tag = b"BBB".to_vec();
            }
            rows[0].political_power_raw = Some(i64::from(hour));
        }
        let wire = Arc::new(encoder.encode(input.clone(), hour == first)?);
        let frame = decoder.advance(&wire)?;
        driver.observe(&mut prepared, frame)?;
        oracle.observe(replay.format, frame);
        assert_eq!(
            serde_json::to_vec(&prepared.variants)?,
            serde_json::to_vec(&oracle)?,
            "variant versions at {hour}"
        );
        assert_eq!(
            prepared.variants.current(20).unwrap().name_tag.as_deref(),
            Some(if hour < last { "AAA" } else { "BBB" })
        );
    }
    Ok(())
}
