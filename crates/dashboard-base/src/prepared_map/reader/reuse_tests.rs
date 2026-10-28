use super::*;
use crate::prepared::series::tests::Fixture;
use crate::prepared_map::Builder;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame};
use replay_data::rows::{DivisionStatusRow, Recorded, TableRows};
use replay_frame::frame::{TableUpdate, recorded::update_bytes};
use std::{collections::BTreeMap, num::NonZeroU32};

const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Clock,
    TableId::Countries,
    TableId::DivisionStatus,
];

fn observed(hour: Hour, countries: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, countries);
    frame.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![replay_data::rows::ClockRow {
            hour,
            ..Default::default()
        }]))
        .into();
    frame.metrics.table_ns[TableId::Clock.index()] = 1;
    frame.metrics.total_ns = hour as u64 * 7;
    frame.metrics.table_ns[TableId::DivisionStatus.index()] = u64::from(hour % 3 != 0);
    let rows = TableRows::DivisionStatus(vec![DivisionStatusRow {
        division: 1,
        maximum_hit_points: i64::from(hour) * 1000,
        name: Recorded::Value(Some(vec![0, 255, hour as u8])),
        ..Default::default()
    }]);
    frame.tables[TableId::DivisionStatus.index()] = match hour % 37 {
        3 => TableView::Unavailable {
            reason: "synthetic status refusal".into(),
        },
        4 => TableView::Truncated { rows, missing: 7 },
        5 => TableView::Complete(TableRows::DivisionStatus(Vec::new())),
        _ => TableView::Complete(rows),
    }
    .into();
    frame
}

fn recording(hours: impl Iterator<Item = Hour>) -> Replay {
    Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, hours.map(|hour| observed(hour, 2)).collect())],
    )
    .unwrap()
}

fn prepared(replay: &Replay, accepted: Hour, selected: &[TableId]) -> (Fixture, Progress) {
    let fixture = Fixture::new();
    let mut builder =
        Builder::new(NonZeroU32::new(720).unwrap(), selected, Progress::default()).unwrap();
    builder.set_ranges(replay);
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut writer = fixture.writer();
    for (block, index) in replay.index.iter().enumerate() {
        for (offset, frame) in replay
            .read_block_selected(index, selected)
            .unwrap()
            .iter()
            .enumerate()
        {
            if frame.hour > accepted {
                break;
            }
            let decoded = decoder.advance(frame).unwrap();
            builder
                .accept(
                    &mut writer,
                    frame,
                    decoded,
                    Position {
                        block,
                        next_frame: offset + 1,
                    },
                )
                .unwrap();
        }
    }
    builder.save_gaps(&mut writer, &replay.gaps).unwrap();
    writer.commit().unwrap();
    (fixture, builder.progress().clone())
}

fn reader(
    fixture: &Fixture,
    progress: &Progress,
    replay: &Replay,
    selected: &[TableId],
    cache: bool,
) -> Reader {
    let replay = Arc::new(Replay::open(replay.bytes().to_vec()).unwrap());
    replay.cache_decoded_sections(cache);
    let mut reader = Reader::new(
        Store::local(
            fixture
                .directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap(),
        fixture.key.clone(),
        progress.clone(),
        replay,
        selected,
    )
    .unwrap();
    reader.workers = 1;
    reader
}

fn canonical(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes).unwrap();
    bytes
}

fn reference(replay: &Replay, selected: &[TableId]) -> BTreeMap<(BranchId, Hour), Vec<u8>> {
    let replay = Replay::open(replay.bytes().to_vec()).unwrap();
    replay.cache_decoded_sections(false);
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut expected = BTreeMap::new();
    for index in &replay.index {
        for frame in replay.read_block_selected(index, selected).unwrap() {
            let decoded = decoder.advance(&frame).unwrap();
            expected.insert((decoded.branch, decoded.hour), canonical(decoded));
        }
    }
    expected
}

fn endpoint(actual: &Current, expected: &BTreeMap<(BranchId, Hour), Vec<u8>>, hour: Hour) {
    let Current::Frame(frame) = actual else {
        panic!("expected recorded endpoint {hour}")
    };
    assert_eq!((frame.branch, frame.hour), (1, hour));
    assert_eq!(canonical(frame), expected[&(1, hour)]);
}

fn visitors(
    actual: &[(Hour, Vec<u8>)],
    expected: &BTreeMap<(BranchId, Hour), Vec<u8>>,
    hours: impl IntoIterator<Item = Hour>,
) {
    let expected: Vec<_> = hours
        .into_iter()
        .map(|hour| (hour, expected[&(1, hour)].clone()))
        .collect();
    assert_eq!(actual, expected);
}

fn charge(map: &Reader) -> usize {
    let frames = &map.block.as_ref().unwrap().frames;
    size_of::<Vec<Frame>>()
        + 2 * size_of::<usize>()
        + frames.capacity() * size_of::<Frame>()
        + frames
            .iter()
            .flat_map(|f| &f.tables)
            .map(update_bytes)
            .sum::<usize>()
}

fn refusal(error: io::Error, message: &str) {
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), message);
}

#[test]
fn accepted_reuse_dense_walk_decodes_one_block_and_keeps_exact_visitors_and_cursors() {
    let replay = recording(100..820);
    assert_eq!(replay.index.len(), 1);
    let expected = reference(&replay, TABLES);
    let (fixture, progress) = prepared(&replay, 819, TABLES);
    for cache in [false, true] {
        for stride in [24, 168] {
            let mut map = reader(&fixture, &progress, &replay, TABLES, cache);
            let held = map.seek(1, 100).unwrap().unwrap();
            endpoint(&held, &expected, 100);
            assert_eq!(map.source_reads, 0);
            let mut from = 100;
            let (mut frames, mut reads) = (0, 0);
            let mut wire = Vec::new();
            while from < 819 {
                let target = (from + stride).min(819);
                let mut passed = Vec::new();
                let (result, work) = replay::container::test_work::table_decodes(|| {
                    map.advance_without_output(1, target, |f| passed.push((f.hour, canonical(f))))
                });
                assert!(result.unwrap());
                wire.extend(work);
                frames += map.source_decoded_frames;
                reads += map.source_reads;
                assert_eq!(map.applied_frames, (target - from) as u64);
                visitors(&passed, &expected, (from + 1)..target);
                assert_eq!(
                    map.cursor,
                    Some(Position {
                        block: 0,
                        next_frame: (target - 100 + 1) as usize,
                    })
                );
                endpoint(
                    &map.advance(1, target, |_| panic!("duplicate visitor"))
                        .unwrap()
                        .unwrap(),
                    &expected,
                    target,
                );
                assert_eq!((map.applied_frames, map.source_reads), (0, 0));
                from = target;
            }
            assert_eq!((frames, reads, map.retained_source_frames), (720, 1, 720));
            assert_eq!(
                wire.iter()
                    .filter(|(_, _, table, _)| *table == TableId::Clock)
                    .map(|(_, _, _, hours)| *hours)
                    .collect::<Vec<_>>(),
                [720]
            );
            endpoint(&held, &expected, 100);
        }
    }
}

fn aliased(hour: Hour) -> DecodedFrame {
    use replay_data::rows::{
        DivisionRow, ProductionLineModifiersRow, ProductionLineRow, ProvinceUnitOrderRow,
    };
    let mut frame = observed(hour, 2);
    let organisation = 123_456 + i64::from(hour) * 10_000;
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *frame.tables[TableId::DivisionStatus.index()]
    {
        for row in rows {
            row.organisation_exact = Some(organisation).into();
        }
    }
    let modifiers = ProductionLineModifiersRow {
        line: 9,
        country: 100_000,
        active_factories: 2,
        factory_efficiencies: vec![100_001 + i64::from(hour), i64::MAX],
        ..Default::default()
    };
    let mut line = ProductionLineRow {
        handle: 9,
        country: 100_000,
        active_factories: 2,
        max_factories: 3,
        ..Default::default()
    };
    line.efficiency_raw = if (hour - 300) % 12 == 3 {
        i64::MIN
    } else {
        line.factory_efficiency(&modifiers).unwrap()
    };
    for rows in [
        TableRows::Divisions(vec![DivisionRow {
            id: 1,
            province: hour as u32,
            organisation: DivisionRow::ORGANISATION_STEP.round(organisation),
            ..Default::default()
        }]),
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: hour as u32,
            mixed: vec![1],
            ..Default::default()
        }]),
        TableRows::ProductionLines(vec![line]),
        TableRows::ProductionLineModifiers(vec![modifiers]),
    ] {
        let table = rows.table();
        frame.tables[table.index()] = match (hour - 300) % 12 {
            4 => TableView::Unavailable {
                reason: "synthetic alias source refusal".into(),
            },
            5 => TableView::Complete(TableRows::empty(table)),
            _ => TableView::Complete(rows),
        }
        .into();
        frame.metrics.table_ns[table.index()] = 1;
    }
    replay_frame::frame::retire_absent(frame)
}

#[test]
fn accepted_reuse_aliases_keyframes_churn_and_tail_keep_canonical_bytes() {
    let mut input = records(1, vec![aliased(300)]);
    input.truncate(2);
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    for hour in 300..352 {
        input.push(replay::spool::Record::Frame(
            encoder
                .encode(aliased(hour), (hour - 300) % 24 == 0)
                .unwrap(),
        ));
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    let expected = reference(&replay, &TableId::ALL);
    let (fixture, progress) = prepared(&replay, 351, &TableId::ALL);
    for cache in [false, true] {
        let mut map = reader(&fixture, &progress, &replay, &TableId::ALL, cache);
        endpoint(&map.seek(1, 300).unwrap().unwrap(), &expected, 300);
        let mut held = Vec::new();
        let mut previous = 300;
        for target in [308, 316, 324, 332, 340, 348, 351] {
            let mut passed = Vec::new();
            let actual = map
                .advance(1, target, |f| passed.push((f.hour, canonical(f))))
                .unwrap()
                .unwrap();
            visitors(&passed, &expected, (previous + 1)..target);
            endpoint(&actual, &expected, target);
            let Current::Frame(frame) = actual else {
                panic!()
            };
            held.push((Arc::clone(&frame), canonical(&frame)));
            previous = target;
        }
        for (frame, bytes) in held {
            assert_eq!(canonical(&frame), bytes);
        }
    }
}

#[test]
fn accepted_reuse_sparse_gaps_resets_and_cold_prefixes_have_fixed_cursor_policy() {
    let hours = (100..180).filter(|h| ![102, 124, 125, 150].contains(h));
    let replay = recording(hours);
    let expected = reference(&replay, TABLES);
    let (fixture, progress) = prepared(&replay, 179, TABLES);
    for cache in [false, true] {
        let mut map = reader(&fixture, &progress, &replay, TABLES, cache);
        for (target, applied, passed_hours, next_frame, source_frames) in [
            (110, 9, vec![101, 103, 104, 105, 106, 107, 108, 109], 10, 10),
            (110, 0, vec![], 10, 0),
            (112, 2, vec![111], 12, 76),
            (124, 11, (113..124).collect(), 23, 0),
            (126, 1, vec![], 24, 0),
            (150, 23, (127..150).collect(), 47, 0),
            (160, 10, (151..160).collect(), 57, 0),
        ] {
            let mut passed = Vec::new();
            let actual = map
                .advance(1, target, |f| passed.push((f.hour, canonical(f))))
                .unwrap()
                .unwrap();
            visitors(&passed, &expected, passed_hours);
            if [124, 150].contains(&target) {
                assert_eq!(
                    actual,
                    Current::Gap {
                        branch: 1,
                        hour: target,
                        reason: "hour was not recorded".into()
                    }
                );
            } else {
                endpoint(&actual, &expected, target);
            }
            assert_eq!(map.applied_frames, applied);
            assert_eq!(map.source_decoded_frames, source_frames);
            assert_eq!(
                map.cursor,
                Some(Position {
                    block: 0,
                    next_frame
                })
            );
        }
        map.block = None;
        endpoint(
            &map.advance(1, 107, |_| {}).unwrap().unwrap(),
            &expected,
            107,
        );
        assert_eq!((map.source_reads, map.source_decoded_frames), (1, 7));
        assert_eq!(map.block.as_ref().unwrap().through, 107);
        assert!(!map.advance_without_output(2, 160, |_| panic!()).unwrap());
        assert!(!map.advance_without_output(1, 180, |_| panic!()).unwrap());
    }
    fixture.store.clear().unwrap();
    let mut map = reader(&fixture, &progress, &replay, TABLES, false);
    assert!(!map.advance_without_output(1, 174, |_| panic!()).unwrap());
}

#[test]
fn accepted_reuse_tail_bounds_memory_and_releases_replaced_block() {
    let replay = recording(100..950);
    assert_eq!(replay.index.len(), 2);
    let expected = reference(&replay, TABLES);
    let (fixture, progress) = prepared(&replay, 900, TABLES);
    let mut map = reader(&fixture, &progress, &replay, TABLES, true);
    drop(map.seek(1, 100).unwrap());
    let held = map.advance(1, 124, |_| {}).unwrap().unwrap();
    let weak = Arc::downgrade(&map.block.as_ref().unwrap().frames);
    assert!(charge(&map) >= 720 * size_of::<Frame>());
    let mut passed = Vec::new();
    endpoint(
        &map.advance(1, 832, |f| passed.push((f.hour, canonical(f))))
            .unwrap()
            .unwrap(),
        &expected,
        832,
    );
    visitors(&passed, &expected, 125..832);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 1,
            next_frame: 13
        })
    );
    assert_eq!(map.retained_source_frames, 81);
    assert!(
        weak.upgrade().is_none(),
        "held outputs must not pin raw source blocks"
    );
    assert!(map.replay_handle().decoded_section_cache_bytes() > 0);
    let source_charge = charge(&map);
    map.replay_handle().cache_decoded_sections(false);
    assert_eq!(map.replay_handle().decoded_section_cache_bytes(), 0);
    assert_eq!(charge(&map), source_charge);
    endpoint(&held, &expected, 124);
    assert!(!map.advance_without_output(1, 901, |_| panic!()).unwrap());
}

#[test]
fn accepted_reuse_future_campaign_failure_is_deferred_to_original_apply_hour() {
    let good = recording(100..250);
    let expected = reference(&good, TABLES);
    let (fixture, progress) = prepared(&good, 249, TABLES);
    let mut raw = good
        .read_block_selected(&good.index[0], &TableId::ALL)
        .unwrap();
    raw[100].tables[TableId::Campaign.index()] =
        TableUpdate::Snapshot(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
            id: b"wrong campaign".to_vec(),
        }]));
    let bad = replay::container::test_work::with_block_frames(&good, &raw).unwrap();
    for future_wire_error in [false, true] {
        let bad = if future_wire_error {
            replay::container::test_work::with_trailing_table_control(&bad, TableId::Clock).unwrap()
        } else {
            Replay::open(bad.bytes().to_vec()).unwrap()
        };
        for cache in [false, true] {
            let mut map = reader(&fixture, &progress, &bad, TABLES, cache);
            endpoint(&map.seek(1, 100).unwrap().unwrap(), &expected, 100);
            endpoint(
                &map.advance(1, 124, |_| {}).unwrap().unwrap(),
                &expected,
                124,
            );
            assert_eq!(
                map.retained_source_frames,
                if future_wire_error { 25 } else { 150 }
            );
            let mut passed = Vec::new();
            refusal(
                map.advance(1, 220, |f| passed.push((f.hour, canonical(f))))
                    .unwrap_err(),
                "frame campaign identity differs from replay header",
            );
            visitors(&passed, &expected, 125..200);
            assert_eq!(map.applied_frames, 75);
            assert_eq!(map.requested, None);
            assert_eq!(map.cursor, None);
            assert!(map.decoder.current().is_none());
            assert!(!map.page_cursor);
        }
    }
}

#[test]
fn accepted_reuse_sizes_large_rows_and_does_not_reload_an_admitted_prefix() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            (100..110).map(|hour| observed(hour, 5_000)).collect(),
        )],
    )
    .unwrap();
    let (fixture, progress) = prepared(&replay, 109, TABLES);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false);
    drop(map.seek(1, 101).unwrap());
    let prefix_charge = charge(&map);
    assert_eq!(map.retained_source_frames, 2);
    drop(map.advance(1, 103, |_| {}).unwrap());
    assert!(charge(&map) > prefix_charge);
    assert_eq!(map.retained_source_frames, 10);
    drop(map.advance(1, 105, |_| {}).unwrap());
    assert_eq!((map.source_reads, map.source_decoded_frames), (0, 0));
}

#[test]
fn accepted_reuse_future_wire_error_retries_exact_prefix_before_any_visitor() {
    let good = recording(100..250);
    let expected = reference(&good, TABLES);
    let (fixture, progress) = prepared(&good, 249, TABLES);
    let bad =
        replay::container::test_work::with_trailing_table_control(&good, TableId::Clock).unwrap();
    for cache in [false, true] {
        for warm in [false, true] {
            let mut map = reader(&fixture, &progress, &bad, TABLES, cache);
            if warm {
                map.replay
                    .read_block_selected_through(&map.replay.index[0], Some(TABLES), 148, None)
                    .unwrap();
            }
            endpoint(&map.seek(1, 100).unwrap().unwrap(), &expected, 100);
            let mut previous = 100;
            for target in [124, 148] {
                let mut passed = Vec::new();
                let actual = map
                    .advance(1, target, |f| passed.push((f.hour, canonical(f))))
                    .unwrap()
                    .unwrap();
                endpoint(&actual, &expected, target);
                visitors(&passed, &expected, (previous + 1)..target);
                assert_eq!(map.retained_source_frames, (target - 100 + 1) as usize);
                previous = target;
            }
            refusal(
                map.advance(1, 149, |_| panic!("admission before visitors"))
                    .unwrap_err(),
                "grouped section has unread column bytes",
            );
            assert_eq!(map.requested, Some((1, 148)));
            assert_eq!(
                map.cursor,
                Some(Position {
                    block: 0,
                    next_frame: 49
                })
            );
            assert_eq!(map.applied_frames, 0);
            endpoint(&map.seek(1, 149).unwrap().unwrap(), &expected, 149);
            refusal(
                map.advance(1, 249, |_| panic!("admission before visitors"))
                    .unwrap_err(),
                "grouped section has unread column bytes",
            );
            assert_eq!(
                map.cursor,
                Some(Position {
                    block: 0,
                    next_frame: 50
                })
            );
            assert_eq!(map.requested, Some((1, 149)));
            assert_eq!(map.applied_frames, 0);
        }
    }
}

#[test]
fn accepted_reuse_never_reads_unaccepted_suffix_or_future_block_and_rejects_bad_cursor() {
    let good = recording(100..950);
    let (fixture, progress) = prepared(&good, 124, TABLES);
    let mut map = reader(&fixture, &progress, &good, TABLES, false);
    Arc::get_mut(&mut map.replay).unwrap().index[1].offset = u64::MAX;
    drop(map.seek(1, 100).unwrap());
    drop(map.advance(1, 124, |_| {}).unwrap());
    assert_eq!((map.source_reads, map.source_decoded_frames), (1, 25));
    assert_eq!(map.block.as_ref().unwrap().through, 124);
    assert!(
        !map.advance_without_output(1, 125, |_| panic!("unaccepted visitor"))
            .unwrap()
    );

    let suffix = recording(100..250);
    let (fixture, progress) = prepared(&suffix, 124, TABLES);
    let bad =
        replay::container::test_work::with_trailing_table_control(&suffix, TableId::Clock).unwrap();
    let mut map = reader(&fixture, &progress, &bad, TABLES, true);
    drop(map.seek(1, 100).unwrap());
    drop(map.advance(1, 124, |_| {}).unwrap());
    assert_eq!(map.retained_source_frames, 25);

    let (fixture, progress) = prepared(&good, 949, TABLES);
    let mut map = reader(&fixture, &progress, &good, TABLES, false);
    drop(map.seek(1, 100).unwrap());
    map.cursor.as_mut().unwrap().next_frame = 2;
    refusal(
        map.advance(1, 124, |_| panic!("bad cursor exposed history"))
            .unwrap_err(),
        "map source cursor differs from decoded baseline",
    );
    assert_eq!(map.applied_frames, 0);
    assert_eq!(map.requested, Some((1, 100)));
}

#[test]
fn admitted_source_is_loaded_without_applying_or_advancing_the_published_cursor() {
    let replay = recording(100..160);
    let expected = reference(&replay, TABLES);
    let (fixture, progress) = prepared(&replay, 159, TABLES);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false);
    drop(map.seek(1, 100).unwrap());
    let cursor = map.cursor.unwrap();
    let admitted = map
        .admit(
            1,
            124,
            Some(100),
            cursor,
            false,
            Load::AcceptedBlockOrPrefix,
        )
        .unwrap();
    assert_eq!(map.decoder.current().unwrap().hour, 100);
    assert_eq!(map.cursor, Some(cursor));
    assert_eq!(map.requested, Some((1, 100)));
    assert_eq!(map.applied_frames, 0);
    assert_eq!(admitted.frames.len(), 1);
    assert_eq!(admitted.frames[0].1, 1..25);
    assert_eq!(
        admitted.cursor,
        Position {
            block: 0,
            next_frame: 25
        }
    );
    let mut passed = Vec::new();
    map.apply_admitted(
        124,
        admitted,
        |f| passed.push((f.hour, canonical(f))),
        &|| false,
    )
    .unwrap();
    visitors(&passed, &expected, 101..124);
    assert_eq!(
        canonical(map.decoder.current().unwrap()),
        expected[&(1, 124)]
    );
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 25
        })
    );
}

#[test]
fn accepted_reuse_clear_after_wider_read_preserves_the_last_successful_continuation() {
    let replay = recording(100..950);
    let expected = reference(&replay, TABLES);
    for cache in [false, true] {
        let (fixture, progress) = prepared(&replay, 949, TABLES);
        let mut map = reader(&fixture, &progress, &replay, TABLES, cache);
        drop(map.seek(1, 100).unwrap());
        endpoint(
            &map.advance(1, 124, |_| {}).unwrap().unwrap(),
            &expected,
            124,
        );
        assert_eq!(map.load(1, 832), Load::AcceptedBlockOrPrefix);
        assert_eq!(map.block.as_ref().unwrap().through, 819);
        let cursor = map.cursor;
        let requested = map.requested;
        let baseline = canonical(map.decoder.current().unwrap());
        assert_eq!(
            cursor,
            Some(Position {
                block: 0,
                next_frame: 25
            })
        );
        assert_eq!(requested, Some((1, 124)));
        assert_eq!(baseline, expected[&(1, 124)]);
        let store = Arc::clone(&map.store);
        let key = map.key.clone();
        map.source_read_hook = Some(Box::new(move |through, result| {
            assert_eq!(through, 949);
            let frames = result.as_ref().unwrap();
            assert_eq!(frames.len(), 130);
            assert_eq!((frames[0].hour, frames.last().unwrap().hour), (820, 949));
            assert!(store.is_reusable(&key).unwrap());
            store.clear().unwrap();
        }));
        let mut passed = Vec::new();
        refusal(
            map.advance(1, 832, |frame| passed.push(frame.hour))
                .unwrap_err(),
            "map store was cleared during source read",
        );
        assert!(!fixture.store.is_reusable(&fixture.key).unwrap());
        assert!(passed.is_empty());
        assert_eq!(map.applied_frames, 0);
        assert_eq!((map.source_reads, map.source_decoded_frames), (1, 130));
        assert_eq!(map.block.as_ref().unwrap().through, 949);
        assert_eq!(map.cursor, cursor);
        assert_eq!(map.requested, requested);
        assert_eq!(canonical(map.decoder.current().unwrap()), baseline);
    }
}

#[test]
fn accepted_reuse_clear_after_prefix_retry_preserves_the_last_successful_continuation() {
    let good = recording(100..250);
    let expected = reference(&good, TABLES);
    let bad =
        replay::container::test_work::with_trailing_table_control(&good, TableId::Clock).unwrap();
    for cache in [false, true] {
        let (fixture, progress) = prepared(&good, 249, TABLES);
        let mut map = reader(&fixture, &progress, &bad, TABLES, cache);
        drop(map.seek(1, 100).unwrap());
        endpoint(
            &map.advance(1, 124, |_| {}).unwrap().unwrap(),
            &expected,
            124,
        );
        assert_eq!(map.load(1, 148), Load::AcceptedBlockOrPrefix);
        assert_eq!(map.block.as_ref().unwrap().through, 124);
        let cursor = map.cursor;
        let requested = map.requested;
        let baseline = canonical(map.decoder.current().unwrap());
        assert_eq!(
            cursor,
            Some(Position {
                block: 0,
                next_frame: 25
            })
        );
        assert_eq!(requested, Some((1, 124)));
        assert_eq!(baseline, expected[&(1, 124)]);
        let store = Arc::clone(&map.store);
        let key = map.key.clone();
        let mut attempts = [249, 148].into_iter();
        map.source_read_hook = Some(Box::new(move |through, result| {
            assert_eq!(through, attempts.next().expect("unexpected source read"));
            assert!(store.is_reusable(&key).unwrap());
            if through == 249 {
                let error = result.as_ref().unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert_eq!(error.to_string(), "grouped section has unread column bytes");
            } else {
                let frames = result.as_ref().unwrap();
                assert_eq!(frames.len(), 49);
                assert_eq!(frames.last().unwrap().hour, 148);
                assert!(attempts.next().is_none());
                store.clear().unwrap();
            }
        }));
        let mut passed = Vec::new();
        refusal(
            map.advance(1, 148, |frame| passed.push(frame.hour))
                .unwrap_err(),
            "map store was cleared during source read",
        );
        assert!(!fixture.store.is_reusable(&fixture.key).unwrap());
        assert!(passed.is_empty());
        assert_eq!(map.applied_frames, 0);
        assert_eq!((map.source_reads, map.source_decoded_frames), (1, 49));
        assert_eq!(map.block.as_ref().unwrap().through, 148);
        assert_eq!(map.cursor, cursor);
        assert_eq!(map.requested, requested);
        assert_eq!(canonical(map.decoder.current().unwrap()), baseline);
    }
}

#[test]
fn accepted_reuse_preserves_map_capture_markers_without_inventing_unavailable_reads() {
    use replay_data::scope::{self, CaptureMode};
    let frames = (100..130)
        .map(|hour| {
            let mut frame = observed(hour, 2);
            for table in TableId::ALL {
                if table.last_change().present(replay_data::FORMAT_VERSION)
                    && !scope::records(CaptureMode::Map, table)
                {
                    frame.tables[table.index()] = TableView::NotRecorded {
                        mode: CaptureMode::Map,
                    }
                    .into();
                }
            }
            frame
        })
        .collect();
    let mut input = records(1, frames);
    let replay::spool::Record::SessionStart(session) = &mut input[0] else {
        panic!()
    };
    session.capture_mode = CaptureMode::Map;
    let replay = Replay::from_records(CAMPAIGN.into(), &[input]).unwrap();
    assert!(!scope::records(CaptureMode::Map, TableId::DivisionStatus));
    let expected = reference(&replay, TABLES);
    let (fixture, progress) = prepared(&replay, 129, TABLES);
    for cache in [false, true] {
        let mut map = reader(&fixture, &progress, &replay, TABLES, cache);
        drop(map.seek(1, 100).unwrap());
        for target in [104, 108, 129] {
            let actual = map.advance(1, target, |_| {}).unwrap().unwrap();
            endpoint(&actual, &expected, target);
            let Current::Frame(frame) = actual else {
                panic!()
            };
            assert_eq!(
                &*frame.tables[TableId::DivisionStatus.index()],
                &TableView::NotRecorded {
                    mode: CaptureMode::Map
                }
            );
        }
    }
}

#[test]
fn accepted_reuse_resets_to_requested_prefix_at_stride_and_branch_boundaries() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(1, (100..950).map(|hour| observed(hour, 2)).collect()),
            records(
                2,
                (100..180)
                    .map(|hour| {
                        let mut frame = observed(hour, 2);
                        frame.branch = 2;
                        frame
                    })
                    .collect(),
            ),
        ],
    )
    .unwrap();
    assert_eq!(replay.index.len(), 3);
    let expected = reference(&replay, TABLES);
    let (fixture, progress) = prepared(&replay, 949, TABLES);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false);
    drop(map.seek(1, 100).unwrap());
    drop(map.advance(1, 124, |_| {}).unwrap());
    assert_eq!(map.retained_source_frames, 720);
    let actual = map.advance(1, 900, |_| {}).unwrap().unwrap();
    endpoint(&actual, &expected, 900);
    assert!(map.checkpoint_reads > 0);
    assert_eq!((map.source_reads, map.source_decoded_frames), (1, 81));
    assert_eq!(map.block.as_ref().unwrap().through, 900);
    let Current::Frame(actual) = map.advance(2, 114, |_| {}).unwrap().unwrap() else {
        panic!()
    };
    assert_eq!(canonical(&actual), expected[&(2, 114)]);
    assert_eq!((map.source_reads, map.source_decoded_frames), (1, 15));
    assert_eq!(map.block.as_ref().unwrap().through, 114);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 2,
            next_frame: 15
        })
    );
    drop(map.advance(2, 116, |_| {}).unwrap());
    assert_eq!((map.source_reads, map.source_decoded_frames), (1, 80));
}

#[test]
fn accepted_reuse_refuses_a_later_block_before_applying_any_admitted_prefix() {
    let replay = recording(100..950);
    let (fixture, progress) = prepared(&replay, 949, TABLES);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false);
    Arc::get_mut(&mut map.replay).unwrap().index[1].offset = u64::MAX;
    drop(map.seek(1, 100).unwrap());
    drop(map.advance(1, 124, |_| {}).unwrap());
    refusal(
        map.advance(1, 832, |_| panic!("failed admission exposed a visitor"))
            .unwrap_err(),
        "block offset overflow",
    );
    assert_eq!(map.decoder.current().unwrap().hour, 124);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 25
        })
    );
    assert_eq!(map.requested, Some((1, 124)));
    assert_eq!(map.applied_frames, 0);
}

#[test]
fn cancelled_backward_checkpoint_restore_keeps_the_forward_decoder_cursor_pair() {
    let replay = recording(80..141);
    let (fixture, progress) = prepared(&replay, 140, TABLES);
    let budget = engine_allocation::credit::MemoryBudget::new(1 << 20);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false).with_change_facts(budget);
    drop(map.seek(1, 120).unwrap());
    assert_eq!(map.decoder.current().unwrap().hour, 120);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 41
        })
    );
    let checks = std::cell::Cell::new(0);
    let error = map
        .read_shared_cancellable(
            1,
            85,
            |_| panic!("cancelled restore published"),
            false,
            true,
            &|| {
                checks.set(checks.get() + 1);
                checks.get() == 2
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(checks.get(), 2);
    assert_eq!(map.decoder.current().unwrap().hour, 120);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 41
        })
    );
    assert!(!map.page_cursor);
    assert_eq!(map.requested, Some((1, 120)));
    let Current::Frame(frame) = map
        .advance(1, 121, |_| panic!("adjacent request has no passed hour"))
        .unwrap()
        .unwrap()
    else {
        panic!("prepared forward frame");
    };
    assert_eq!(frame.hour, 121);
    let TableView::Complete(TableRows::Clock(rows)) = &*frame.tables[TableId::Clock.index()] else {
        panic!("complete clock");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hour, 121);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 42
        })
    );
    assert_eq!(map.applied_frames, 1);
}

#[test]
fn intermediate_campaign_failure_after_backward_resume_invalidates_the_decoder_cursor_pair() {
    let replay = recording(80..141);
    let (fixture, progress) = prepared(&replay, 140, TABLES);
    let budget = engine_allocation::credit::MemoryBudget::new(1 << 20);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false).with_change_facts(budget);
    drop(map.seek(1, 120).unwrap());
    let frames = Arc::make_mut(&mut map.block.as_mut().unwrap().frames);
    assert_eq!(frames[3].hour, 83);
    frames[3].tables[TableId::Campaign.index()] =
        TableUpdate::Snapshot(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
            id: b"wrong campaign at intermediate hour 83".to_vec(),
        }]));
    let mut visited = Vec::new();
    let error = map
        .read_shared_cancellable(
            1,
            85,
            |frame| visited.push(frame.hour),
            false,
            true,
            &|| false,
        )
        .unwrap_err();
    refusal(error, "frame campaign identity differs from replay header");
    assert_eq!(visited, [81, 82]);
    assert!(map.decoder.current().is_none());
    assert_eq!(map.cursor, None);
    assert!(!map.page_cursor);
    assert_eq!(map.requested, None);
    assert_eq!(map.applied_frames, 2);
    // Retry from the unchanged source, not the injected corrupt retained block.
    map.block = None;
    map.retained_source_frames = 0;
    let Current::Frame(frame) = map.advance(1, 121, |_| {}).unwrap().unwrap() else {
        panic!("prepared forward frame after reseek");
    };
    assert_eq!(frame.hour, 121);
    let TableView::Complete(TableRows::Clock(rows)) = &*frame.tables[TableId::Clock.index()] else {
        panic!("complete clock");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hour, 121);
    assert_eq!(map.checkpoint_reads, 1);
    assert_eq!(map.applied_frames, 41);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 42,
        })
    );
}

#[test]
fn retained_seek_visitors_refuse_credit_without_losing_the_committed_prepared_cursor() {
    let selected = &[TableId::Campaign, TableId::Countries];
    let frames = (0..=3)
        .map(|hour| {
            let mut frame = scale_frame(1, hour, 40);
            frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
                (1..=40)
                    .map(|id| replay_data::rows::CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("{id:03}").into_bytes(),
                        command_power_raw: if hour == 2 && id != 1 {
                            1
                        } else {
                            i64::from(hour)
                        },
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let (fixture, progress) = prepared(&replay, 3, selected);
    let budget = engine_allocation::credit::MemoryBudget::new(4608);
    let mut map =
        reader(&fixture, &progress, &replay, selected, false).with_change_facts(budget.clone());
    drop(map.seek(1, 0).unwrap());
    let initial = map.decoder.current().unwrap().tables[TableId::Countries.index()].identity();
    let mut passed = Vec::new();
    let checks = std::cell::Cell::new(0);
    let error = map
        .read_shared_cancellable(
            1,
            3,
            |frame| passed.push(Arc::clone(frame)),
            true,
            true,
            &|| {
                checks.set(checks.get() + 1);
                assert!(
                    checks.get() <= 7,
                    "admission must not wait on retained seek visitors"
                );
                false
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    assert!(FrameDecoder::change_fact_admission_refused(&error));
    assert_eq!(
        error.to_string(),
        "display change-fact working set remains pinned; common paging is required"
    );
    assert_eq!(checks.get(), 7);
    assert_eq!(
        passed.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        passed[0].tables[TableId::Countries.index()]
            .changes_since(&initial)
            .unwrap()
            .changed()
            .len(),
        40
    );
    assert_eq!(
        passed[1].tables[TableId::Countries.index()]
            .changes_since(&passed[0].tables[TableId::Countries.index()].identity())
            .unwrap()
            .changed()
            .len(),
        1
    );
    assert_eq!(map.decoder.current().unwrap().hour, 2);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 3
        })
    );
    assert_eq!(map.applied_frames, 2);
    assert!(!map.page_cursor);
    drop(passed);
    drop(initial);
    let mut visited = Vec::new();
    let Current::Frame(frame) = map
        .read_shared_cancellable(1, 3, |frame| visited.push(frame.hour), true, true, &|| {
            false
        })
        .unwrap()
        .unwrap()
    else {
        panic!("prepared retry frame");
    };
    assert_eq!(frame.hour, 3);
    assert!(visited.is_empty());
    assert_eq!(map.checkpoint_reads, 0);
    assert_eq!(map.applied_frames, 1);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 4
        })
    );
    let rows = frame
        .complete::<replay_data::rows::tables::Countries>()
        .unwrap();
    assert_eq!(rows.len(), 40);
    assert_eq!((rows[0].id, rows[39].id), (1, 40));
    assert!(rows.iter().all(|row| row.command_power_raw == 3));
    assert!(budget.peak() <= 4608);
    drop(frame);
    drop(map);
    assert_eq!(budget.used(), 0);
}

#[test]
fn mid_walk_cancellation_keeps_hour_122_and_retry_visits_only_123_and_124() {
    let replay = recording(80..141);
    let (fixture, progress) = prepared(&replay, 140, TABLES);
    let budget = engine_allocation::credit::MemoryBudget::new(1 << 20);
    let mut map = reader(&fixture, &progress, &replay, TABLES, false).with_change_facts(budget);
    drop(map.seek(1, 120).unwrap());
    let cancelled = std::cell::Cell::new(false);
    let mut visited = Vec::new();
    let error = map
        .read_shared_cancellable(
            1,
            125,
            |frame| {
                visited.push(frame.hour);
                if frame.hour == 122 {
                    cancelled.set(true);
                }
            },
            true,
            true,
            &|| cancelled.get(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(visited, [121, 122]);
    assert_eq!(map.decoder.current().unwrap().hour, 122);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 43
        })
    );
    assert!(!map.page_cursor);
    assert_eq!(map.requested, Some((1, 120)));
    assert_eq!(map.applied_frames, 2);
    visited.clear();
    let Current::Frame(frame) = map
        .read_shared_cancellable(
            1,
            125,
            |frame| visited.push(frame.hour),
            true,
            true,
            &|| false,
        )
        .unwrap()
        .unwrap()
    else {
        panic!("prepared retry frame");
    };
    assert_eq!(frame.hour, 125);
    assert_eq!(visited, [123, 124]);
    assert_eq!(map.decoder.current().unwrap().hour, 125);
    assert_eq!(
        map.cursor,
        Some(Position {
            block: 0,
            next_frame: 46
        })
    );
    assert_eq!(map.checkpoint_reads, 0);
    assert_eq!(map.applied_frames, 3);
    assert_eq!(map.requested, Some((1, 125)));
    let rows = frame
        .complete::<replay_data::rows::tables::Clock>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hour, 125);
}
