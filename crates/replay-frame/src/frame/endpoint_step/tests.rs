use super::*;
use crate::rows::{
    CampaignRow, ClockRow, CountryRow, DivisionHistoryRow, DivisionStatusRow, EventFeedHistoryRow,
    Recorded,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SELECTED: &[TableId] = &[
    TableId::Clock,
    TableId::Campaign,
    TableId::Countries,
    TableId::DivisionStatus,
    TableId::DivisionHistory,
    TableId::EventFeedHistory,
];
fn canonical(frame: &DecodedFrame) -> Vec<u8> {
    let mut out = Vec::new();
    frame.encode_checkpoint(&mut out).unwrap();
    out
}
fn source(tick: i32) -> DecodedFrame {
    let hour = 300 + tick;
    let mut frame = DecodedFrame {
        branch: 19,
        hour,
        metrics: Metrics {
            total_ns: tick as u64,
            pool_bytes: u64::MAX - 1,
            dropped_frames: 7,
            ..Default::default()
        },
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not acquired".into(),
            }
            .into()
        }),
    };
    frame.tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour,
        ..Default::default()
    }]))
    .into();
    frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: vec![0, 255] }])).into();
    let first = CountryRow {
        id: 1,
        canonical_slot: 4,
        tag: vec![0, 255, tick as u8],
        command_power_raw: i64::MIN.wrapping_add(i64::from(tick)),
        border_land_doctrine_level: Recorded::Value((tick % 2 != 0).then_some(tick)),
        ..Default::default()
    };
    let second = CountryRow {
        id: u32::MAX,
        canonical_slot: 8,
        tag: b"same after reinsert".to_vec(),
        ..Default::default()
    };
    let rows = TableRows::Countries(if tick % 12 == 1 {
        vec![first]
    } else {
        vec![first, second]
    });
    frame.tables[TableId::Countries.index()] = match tick % 12 {
        7 => TableView::Truncated {
            rows,
            missing: 50_001,
        },
        8 => TableView::Unavailable {
            reason: "fixture refused census".into(),
        },
        _ => TableView::Complete(rows),
    }
    .into();
    frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: u64::MAX,
            maximum_hit_points: i64::MAX,
            name: Recorded::Value(Some(vec![0, 255, tick as u8])),
            supply: Recorded::Value(None),
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::DivisionHistory.index()] = TableView::Complete(
        TableRows::DivisionHistory(vec![Arc::new(DivisionHistoryRow {
            division: u64::MAX,
            list: DivisionHistoryRow::AWARDS,
            ordinal: u32::MAX,
            available: 1,
            name: vec![255, 0, tick as u8],
            provinces: vec![7, 3, 7],
            medal_count: tick as u8,
            ..Default::default()
        })]),
    )
    .into();
    frame.tables[TableId::EventFeedHistory.index()] =
        TableView::Complete(TableRows::EventFeedHistory(vec![EventFeedHistoryRow {
            coverage: crate::event_feed_rows::EventFeedCoverage::Partial,
            read_hour: hour,
            ..Default::default()
        }]))
        .into();
    if tick % 3 != 0 {
        frame.metrics.table_ns[TableId::Countries.index()] = super::super::READ_TIME_NOT_RECORDED;
    }
    frame
}
fn wire(frame: &Frame) -> Frame {
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let frame = Frame::decode(&mut input).unwrap();
    input.finish().unwrap();
    frame
}
fn emit(
    writer: &mut EndpointWriter,
    frame: &DecodedFrame,
    previous: Option<&DecodedFrame>,
) -> (DecodedFrame, Vec<MembershipBoundary>) {
    let mut bytes = Vec::new();
    let stats = writer.finish(frame, &mut bytes).unwrap();
    assert_eq!(stats.bytes, bytes.len());
    let step = EndpointStep::decode(&bytes).unwrap();
    let boundaries = step.boundaries().cloned().collect();
    let result = step.apply(previous).unwrap();
    assert_eq!(canonical(&result), canonical(frame));
    (result, boundaries)
}

#[test]
fn endpoint_native_k24_every_tail_roots_and_held_outputs_match_canonical_bytes() {
    for tail in 0..24 {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(SELECTED);
        let mut native = EndpointWriter::for_tables(SELECTED);
        let mut coarse = EndpointWriter::for_tables(SELECTED);
        let mut before_native = None;
        let mut before_coarse = None;
        let mut interval = Vec::new();
        let mut held = Vec::new();
        for tick in 0..=48 + tail {
            let mut raw = encoder.encode(source(tick), tick % 24 == 0).unwrap();
            // A decoded table refusal, not a writer-selected replacement error.
            if tick == 35 {
                raw.tables[TableId::DivisionStatus.index()] =
                    TableUpdate::Unchanged { baseline: -1 };
            }
            let raw = wire(&raw);
            let frame = decoder.advance(&raw).unwrap();
            native.observe(&raw, frame).unwrap();
            coarse.observe(&raw, frame).unwrap();
            let (next, boundaries) = emit(&mut native, frame, before_native.as_ref());
            interval.extend(boundaries);
            before_native = Some(next);
            if tick % 24 == 0 || tick == 48 + tail {
                let (next, mut boundaries) = emit(&mut coarse, frame, before_coarse.as_ref());
                interval.sort_by_key(|boundary| (boundary.table.index(), boundary.hour));
                boundaries.sort_by_key(|boundary| (boundary.table.index(), boundary.hour));
                assert_eq!(boundaries, interval, "tail {tail}, tick {tick}");
                interval.clear();
                let mut root = Vec::new();
                EndpointWriter::root(frame, SELECTED, &mut root).unwrap();
                let restored = EndpointStep::decode(&root).unwrap().apply(None).unwrap();
                assert_eq!(canonical(&restored), canonical(&next));
                held.push((next.clone(), canonical(&next)));
                before_coarse = Some(next);
            }
            for (old, bytes) in &held {
                assert_eq!(&canonical(old), bytes, "held frame changed");
            }
        }
    }
}

#[test]
fn endpoint_membership_keeps_delete_reinsert_and_never_calls_partial_absence_deletion() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    let mut writer = EndpointWriter::for_tables(SELECTED);
    let mut seed = None;
    for tick in 0..=9 {
        let raw = wire(&encoder.encode(source(tick), tick == 0).unwrap());
        let frame = decoder.advance(&raw).unwrap();
        writer.observe(&raw, frame).unwrap();
        if tick == 0 {
            seed = Some(emit(&mut writer, frame, None).0);
        }
        if tick == 9 {
            let (_, boundaries) = emit(&mut writer, frame, seed.as_ref());
            let countries: Vec<_> = boundaries
                .iter()
                .filter(|b| b.table == TableId::Countries)
                .collect();
            let removed = countries.iter().find(|b| b.hour == 301).unwrap();
            assert_eq!(removed.deleted, [u128::from(u32::MAX)]);
            let inserted = countries.iter().find(|b| b.hour == 302).unwrap();
            assert_eq!(inserted.inserted, [u128::from(u32::MAX)]);
            let partial = countries.iter().find(|b| b.hour == 307).unwrap();
            assert_eq!(partial.kind, BoundaryKind::Partial);
            assert!(partial.deleted.is_empty());
            assert_eq!(
                countries.iter().find(|b| b.hour == 308).unwrap().kind,
                BoundaryKind::Unavailable
            );
            assert_eq!(
                countries.iter().find(|b| b.hour == 309).unwrap().kind,
                BoundaryKind::CompleteReset
            );
        }
    }
}

#[test]
fn endpoint_reader_refuses_every_truncated_payload_and_wrong_baselines_without_mutation() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    let raw = wire(&encoder.encode(source(0), true).unwrap());
    let first = decoder.apply(&raw).unwrap();
    let mut writer = EndpointWriter::for_tables(SELECTED);
    writer.observe(&raw, &first).unwrap();
    let mut root = Vec::new();
    writer.finish(&first, &mut root).unwrap();
    for cut in 0..root.len() {
        assert!(EndpointStep::decode(&root[..cut]).is_err(), "cut {cut}");
    }
    let mut trailing = root.clone();
    trailing.push(0);
    assert_eq!(
        EndpointStep::decode(&trailing).unwrap_err().to_string(),
        "trailing bytes"
    );
    let raw = wire(&encoder.encode(source(1), false).unwrap());
    let next = decoder.apply(&raw).unwrap();
    writer.observe(&raw, &next).unwrap();
    let mut bytes = Vec::new();
    writer.finish(&next, &mut bytes).unwrap();
    let old_bytes = canonical(&first);
    for wrong in [None, Some(&next)] {
        assert_eq!(
            EndpointStep::decode(&bytes)
                .unwrap()
                .apply(wrong)
                .unwrap_err()
                .to_string(),
            "endpoint baseline differs"
        );
    }
    let mut wrong = first.clone();
    wrong.branch += 1;
    assert!(
        EndpointStep::decode(&bytes)
            .unwrap()
            .apply(Some(&wrong))
            .is_err()
    );
    assert_eq!(canonical(&first), old_bytes);
    let mut bad = EndpointStep::decode(&bytes).unwrap();
    let at = bad
        .sections
        .iter()
        .position(|s| s.table == TableId::Countries)
        .unwrap();
    let Operation::Patch { deleted, .. } = &mut bad.sections[at].operation else {
        panic!("expected patch");
    };
    *deleted = vec![1]; // key 1 is also patched; previous tables have already applied.
    assert_eq!(
        bad.apply(Some(&first)).unwrap_err().to_string(),
        "endpoint changed and deleted keys overlap"
    );
    assert_eq!(canonical(&first), old_bytes);
}

#[test]
fn endpoint_writer_refuses_gap_branch_and_wrong_hour_and_encoding_is_atomic() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    let raw = wire(&encoder.encode(source(0), true).unwrap());
    let first = decoder.apply(&raw).unwrap();
    let mut writer = EndpointWriter::for_tables(SELECTED);
    writer.observe(&raw, &first).unwrap();
    let mut bad = first.clone();
    bad.tables[TableId::Countries.index()] = bad.tables[TableId::Countries.index()]
        .clone()
        .with_sampled_at(Some(999));
    let mut output = vec![7, 8, 9];
    assert!(writer.finish(&bad, &mut output).is_err());
    assert_eq!(output, [7, 8, 9]);
    writer.finish(&first, &mut output).unwrap();
    let mut other = raw.clone();
    other.branch += 1;
    let mut other_frame = first.clone();
    other_frame.branch += 1;
    assert_eq!(
        writer
            .observe(&other, &other_frame)
            .unwrap_err()
            .to_string(),
        "endpoint observation crosses a branch or native gap"
    );
    other.branch = raw.branch;
    other.hour += 2;
    other_frame.branch = first.branch;
    other_frame.hour += 2;
    assert!(writer.observe(&other, &other_frame).is_err());
    let raw = wire(&encoder.encode(source(1), false).unwrap());
    let next = decoder.apply(&raw).unwrap();
    assert!(writer.observe(&raw, &first).is_err());
    writer.observe(&raw, &next).unwrap();
    emit(&mut writer, &next, Some(&first));
}

#[test]
fn endpoint_root_preserves_absence_sample_hours_and_owned_buffer_lifetime() {
    let mut frame = source(0);
    for table in TableId::ALL {
        if !SELECTED.contains(&table) {
            frame.tables[table.index()] = not_selected();
        }
    }
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    rows[0].border_land_doctrine_level = Recorded::NotRecorded;
    let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *frame.tables[TableId::DivisionStatus.index()]
    else {
        panic!()
    };
    rows[0].supply = Recorded::NotRecorded;
    frame.tables[TableId::Countries.index()] = frame.tables[TableId::Countries.index()]
        .clone()
        .with_sampled_at(Some(273));
    let expected = canonical(&frame);
    let mut bytes = Vec::new();
    EndpointWriter::root(&frame, SELECTED, &mut bytes).unwrap();
    let step = EndpointStep::decode(&bytes).unwrap();
    drop((bytes, frame));
    let restored = step.apply(None).unwrap();
    assert_eq!(
        restored.tables[TableId::Countries.index()].sampled_at(),
        Some(273)
    );
    assert_eq!(canonical(&restored), expected);
}

/// Ticks 0..=30 on the wire and as reconstructed, with keyframes at 0 and 24.
fn window() -> (Vec<Frame>, Vec<DecodedFrame>) {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    let raws: Vec<_> = (0..=30)
        .map(|tick| wire(&encoder.encode(source(tick), tick % 24 == 0).unwrap()))
        .collect();
    let frames = raws.iter().map(|raw| decoder.apply(raw).unwrap()).collect();
    (raws, frames)
}

#[test]
fn endpoint_resume_writes_the_observing_writers_next_step_byte_for_byte() {
    // Every resume hour, across keyframes, truncation, refusal and reinsertion.
    let (raws, frames) = window();
    let last = frames.last().unwrap();
    for resume in 0..frames.len() - 1 {
        let mut observed = EndpointWriter::for_tables(SELECTED);
        for (raw, frame) in raws.iter().zip(&frames).take(resume + 1) {
            observed.observe(raw, frame).unwrap();
        }
        observed.finish(&frames[resume], &mut Vec::new()).unwrap();
        let mut resumed = EndpointWriter::resume(SELECTED, &frames[resume]).unwrap();
        for (raw, frame) in raws.iter().zip(&frames).skip(resume + 1) {
            observed.observe(raw, frame).unwrap();
            resumed.observe(raw, frame).unwrap();
        }
        let (mut expected, mut actual) = (Vec::new(), Vec::new());
        observed.finish(last, &mut expected).unwrap();
        resumed.finish(last, &mut actual).unwrap();
        assert_eq!(actual, expected, "resumed at {}", frames[resume].hour);
        let step = EndpointStep::decode(&actual).unwrap();
        let reached = step.apply(Some(&frames[resume])).unwrap();
        assert_eq!(canonical(&reached), canonical(last));
    }
    // A resumed writer has nothing pending at its own hour.
    let mut resumed = EndpointWriter::resume(SELECTED, &frames[3]).unwrap();
    assert!(resumed.finish(&frames[3], &mut Vec::new()).is_err());
}

#[test]
fn endpoint_owned_apply_equals_flat_apply_and_patches_only_unshared_rows_in_place() {
    let (raws, frames) = window();
    let mut writer = EndpointWriter::for_tables(SELECTED);
    let (mut flat, mut owned): (Option<DecodedFrame>, Option<DecodedFrame>) = (None, None);
    for (tick, (raw, frame)) in raws.iter().zip(&frames).enumerate() {
        writer.observe(raw, frame).unwrap();
        if tick % 4 != 0 && tick != frames.len() - 1 {
            continue;
        }
        let mut bytes = Vec::new();
        writer.finish(frame, &mut bytes).unwrap();
        let next_flat = EndpointStep::decode(&bytes)
            .unwrap()
            .apply(flat.as_ref())
            .unwrap();
        // Every other step, an exported copy shares the baseline's tables.
        let exported = owned.clone().filter(|_| tick % 8 == 0);
        let exported_bytes = exported.as_ref().map(canonical);
        let next_owned = EndpointStep::decode(&bytes)
            .unwrap()
            .apply_owned(owned.take())
            .unwrap();
        assert_eq!(canonical(&next_owned), canonical(&next_flat));
        assert_eq!(canonical(&next_owned), canonical(frame));
        assert_eq!(
            exported.as_ref().map(canonical),
            exported_bytes,
            "a shared baseline is never patched"
        );
        flat = Some(next_flat);
        owned = Some(next_owned);
    }
    let second_tag = |frame: &DecodedFrame| match &*frame.tables[TableId::Countries.index()] {
        TableView::Complete(TableRows::Countries(rows)) => rows[1].tag.as_ptr(),
        _ => panic!("complete countries"),
    };
    for shared in [false, true] {
        let mut writer = EndpointWriter::for_tables(SELECTED);
        writer.observe(&raws[2], &frames[2]).unwrap();
        let mut seed = Vec::new();
        writer.finish(&frames[2], &mut seed).unwrap();
        let base = EndpointStep::decode(&seed)
            .unwrap()
            .apply_owned(None)
            .unwrap();
        writer.observe(&raws[3], &frames[3]).unwrap();
        let mut bytes = Vec::new();
        writer.finish(&frames[3], &mut bytes).unwrap();
        let step = EndpointStep::decode(&bytes).unwrap();
        assert!(
            step.sections
                .iter()
                .any(|section| section.table == TableId::Countries
                    && matches!(section.operation, Operation::Patch { .. }))
        );
        let before = second_tag(&base);
        let held = shared.then(|| base.clone());
        let next = step.apply_owned(Some(base)).unwrap();
        assert_eq!(canonical(&next), canonical(&frames[3]));
        assert_eq!(second_tag(&next) == before, !shared, "shared {shared}");
        if let Some(held) = held {
            assert_eq!(second_tag(&held), before);
        }
    }
    // A wrong baseline is refused before any table is taken.
    let mut writer = EndpointWriter::for_tables(SELECTED);
    writer.observe(&raws[0], &frames[0]).unwrap();
    let mut seed = Vec::new();
    writer.finish(&frames[0], &mut seed).unwrap();
    assert_eq!(
        EndpointStep::decode(&seed)
            .unwrap()
            .apply_owned(Some(frames[0].clone()))
            .unwrap_err()
            .to_string(),
        "endpoint baseline differs"
    );
}

#[test]
fn endpoint_owned_sparse_patch_keeps_its_table_buffer_and_membership_changes_merge() {
    let (raws, frames) = window();
    let buffer = |frame: &DecodedFrame, table: TableId| match &*frame.tables[table.index()] {
        TableView::Complete(TableRows::DivisionStatus(rows)) => rows.as_ptr() as usize,
        TableView::Complete(TableRows::Countries(rows)) => rows.as_ptr() as usize,
        _ => panic!("complete {table:?}"),
    };
    // A seed at `from`, then one step to the next tick, both as page bytes.
    let steps = |from: usize| {
        let mut writer = EndpointWriter::for_tables(SELECTED);
        writer.observe(&raws[from], &frames[from]).unwrap();
        let mut seed = Vec::new();
        writer.finish(&frames[from], &mut seed).unwrap();
        writer.observe(&raws[from + 1], &frames[from + 1]).unwrap();
        let mut step = Vec::new();
        writer.finish(&frames[from + 1], &mut step).unwrap();
        (seed, step)
    };
    // Ticks 2 to 3 change fields only; 12 to 13 also deletes a country.
    for (from, merged) in [(2, false), (12, true)] {
        let (seed, step) = steps(from);
        let base = EndpointStep::decode(&seed)
            .unwrap()
            .apply_owned(None)
            .unwrap();
        let (status, countries) = (
            buffer(&base, TableId::DivisionStatus),
            buffer(&base, TableId::Countries),
        );
        let flat_growths = {
            let step = EndpointStep::decode(&step).unwrap();
            let before = engine_allocation::growths();
            let flat = step.apply(Some(&base)).unwrap();
            let growths = engine_allocation::growths() - before;
            assert_eq!(canonical(&flat), canonical(&frames[from + 1]));
            growths
        };
        let step = EndpointStep::decode(&step).unwrap();
        let before = engine_allocation::growths();
        let next = step.apply_owned(Some(base)).unwrap();
        let owned_growths = engine_allocation::growths() - before;
        assert_eq!(canonical(&next), canonical(&frames[from + 1]));
        assert_eq!(buffer(&next, TableId::DivisionStatus), status);
        assert_eq!(buffer(&next, TableId::Countries) == countries, !merged);
        assert!(
            owned_growths < flat_growths,
            "owned {owned_growths}, flat {flat_growths}"
        );
    }
    // A shared baseline is copied, never written through.
    let (seed, step) = steps(2);
    let base = EndpointStep::decode(&seed)
        .unwrap()
        .apply_owned(None)
        .unwrap();
    let held = base.clone();
    let (held_bytes, held_status) = (canonical(&held), buffer(&held, TableId::DivisionStatus));
    let next = EndpointStep::decode(&step)
        .unwrap()
        .apply_owned(Some(base))
        .unwrap();
    assert_eq!(canonical(&next), canonical(&frames[3]));
    assert_ne!(buffer(&next, TableId::DivisionStatus), held_status);
    assert_eq!(buffer(&held, TableId::DivisionStatus), held_status);
    assert_eq!(canonical(&held), held_bytes);
}

/// Steps of disjoint table selections between the same two points merge into the step of
/// their union, byte for byte; steps of other points, or of one table twice, do not.
#[test]
fn steps_of_disjoint_selections_merge_into_the_step_of_their_union() {
    let (_, frames) = window();
    let groups: [&[TableId]; 3] = [
        &[TableId::Clock, TableId::DivisionStatus],
        &[TableId::Campaign, TableId::EventFeedHistory],
        &[TableId::Countries, TableId::DivisionHistory],
    ];
    for (from, to) in [(0, 1), (3, 17), (24, 27)] {
        let mut whole = Vec::new();
        EndpointWriter::diff(SELECTED, &frames[from], &frames[to], &mut whole).unwrap();
        let parts: Vec<Vec<u8>> = groups
            .iter()
            .map(|group| {
                let mut part = Vec::new();
                EndpointWriter::diff(group, &frames[from], &frames[to], &mut part).unwrap();
                part
            })
            .collect();
        let mut merged = Vec::new();
        merge(
            &parts.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            &mut merged,
        )
        .unwrap();
        assert_eq!(merged, whole, "{from} to {to}");
    }
    let mut one = Vec::new();
    EndpointWriter::diff(groups[0], &frames[0], &frames[1], &mut one).unwrap();
    let mut later = Vec::new();
    EndpointWriter::diff(groups[1], &frames[0], &frames[2], &mut later).unwrap();
    assert!(
        merge(&[&one, &later], &mut Vec::new()).is_err(),
        "another point"
    );
    assert!(
        merge(&[&one, &one], &mut Vec::new()).is_err(),
        "a table twice"
    );
}

#[test]
fn endpoint_diff_between_any_two_hours_reaches_the_later_hour_and_patches_across_keyframes() {
    let (_, frames) = window();
    for (at, previous) in frames.iter().enumerate() {
        for frame in &frames[at + 1..] {
            let mut bytes = vec![7];
            let stats = EndpointWriter::diff(SELECTED, previous, frame, &mut bytes).unwrap();
            assert_eq!(stats.bytes, bytes.len() - 1);
            let step = EndpointStep::decode(&bytes[1..]).unwrap();
            assert_eq!(step.boundaries().count(), 0);
            let flat = step.apply(Some(previous)).unwrap();
            assert_eq!(
                canonical(&flat),
                canonical(frame),
                "{} to {}",
                previous.hour,
                frame.hour
            );
            let owned = EndpointStep::decode(&bytes[1..])
                .unwrap()
                .apply_owned(Some(previous.clone()))
                .unwrap();
            assert_eq!(canonical(&owned), canonical(frame));
        }
    }
    let (before, after) = (&frames[23], &frames[25]);
    let mut bytes = Vec::new();
    EndpointWriter::diff(SELECTED, before, after, &mut bytes).unwrap();
    let step = EndpointStep::decode(&bytes).unwrap();
    let operation = |table: TableId| {
        &step
            .sections
            .iter()
            .find(|section| section.table == table)
            .unwrap()
            .operation
    };
    for table in [TableId::Countries, TableId::DivisionStatus, TableId::Clock] {
        assert!(
            matches!(operation(table), Operation::Patch { .. }),
            "{table:?}"
        );
    }
    assert!(matches!(
        operation(TableId::Campaign),
        Operation::Complete(_)
    ));
    // The same hour, an earlier hour and another branch are refused.
    for (from, to) in [(after, after), (after, before)] {
        let mut out = vec![1];
        assert!(EndpointWriter::diff(SELECTED, from, to, &mut out).is_err());
        assert_eq!(out, [1]);
    }
    let mut other = after.clone();
    other.branch += 1;
    assert!(EndpointWriter::diff(SELECTED, before, &other, &mut Vec::new()).is_err());
}

#[test]
fn endpoint_decoder_advance_applies_owned_steps_and_the_native_decoder_continues() {
    let (raws, frames) = window();
    // A seed at tick 0, then steps every five ticks through 25.
    let mut writer = EndpointWriter::for_tables(SELECTED);
    let mut steps = Vec::new();
    for (tick, (raw, frame)) in raws.iter().zip(&frames).enumerate().take(26) {
        writer.observe(raw, frame).unwrap();
        if tick % 5 == 0 {
            let mut bytes = Vec::new();
            writer.finish(frame, &mut bytes).unwrap();
            steps.push((tick, bytes));
        }
    }
    let mut decoder = FrameDecoder::for_tables(SELECTED);
    let mut previous: Option<usize> = None;
    for (tick, bytes) in &steps {
        // A clone shares the held state; advancing one never writes the other's.
        let held = decoder.clone();
        let reached = decoder
            .advance_endpoint(EndpointStep::decode(bytes).unwrap())
            .unwrap();
        assert_eq!(canonical(reached), canonical(&frames[*tick]));
        assert_eq!(
            held.current().map(canonical),
            previous.map(|previous| canonical(&frames[previous])),
            "the clone at tick {tick}"
        );
        // The native decoder continues from the endpoint as from its own hour.
        let mut native = decoder.clone();
        for (raw, frame) in raws.iter().zip(&frames).skip(tick + 1).take(3) {
            assert_eq!(
                canonical(native.advance(raw).unwrap()),
                canonical(frame),
                "native {} after endpoint {tick}",
                frame.hour
            );
        }
        previous = Some(*tick);
    }
    // A step for other tables is refused with the state kept.
    let mut other = Vec::new();
    EndpointWriter::diff(&SELECTED[..4], &frames[25], &frames[27], &mut other).unwrap();
    assert_eq!(
        decoder
            .advance_endpoint(EndpointStep::decode(&other).unwrap())
            .unwrap_err()
            .to_string(),
        "endpoint tables differ from the decoder's"
    );
    assert_eq!(
        decoder.current().map(canonical),
        Some(canonical(&frames[25]))
    );
    // A step from another baseline is refused before it consumes the state.
    let mut later = Vec::new();
    EndpointWriter::diff(SELECTED, &frames[26], &frames[27], &mut later).unwrap();
    assert_eq!(
        decoder
            .advance_endpoint(EndpointStep::decode(&later).unwrap())
            .unwrap_err()
            .to_string(),
        "endpoint baseline differs"
    );
    assert_eq!(
        decoder.current().map(canonical),
        Some(canonical(&frames[25]))
    );
    assert_eq!(
        canonical(decoder.advance(&raws[26]).unwrap()),
        canonical(&frames[26])
    );
}

fn strided(raws: &[Frame], frames: &[DecodedFrame], stride: usize) -> Vec<(usize, Vec<u8>)> {
    let mut writer = EndpointWriter::for_tables(SELECTED);
    let mut steps = Vec::new();
    for (tick, (raw, frame)) in raws.iter().zip(frames).enumerate() {
        writer.observe(raw, frame).unwrap();
        if tick % stride == 0 {
            let mut bytes = Vec::new();
            writer.finish(frame, &mut bytes).unwrap();
            steps.push((tick, bytes));
        }
    }
    steps
}

/// A seed at tick `from` and the step on to tick `to`, as page bytes.
fn seed_and_step(
    raws: &[Frame],
    frames: &[DecodedFrame],
    from: usize,
    to: usize,
) -> (Vec<u8>, Vec<u8>) {
    let mut writer = EndpointWriter::for_tables(SELECTED);
    for (raw, frame) in raws.iter().zip(frames).take(from + 1) {
        writer.observe(raw, frame).unwrap();
    }
    let mut seed = Vec::new();
    writer.finish(&frames[from], &mut seed).unwrap();
    let mut step = Vec::new();
    EndpointWriter::diff(SELECTED, &frames[from], &frames[to], &mut step).unwrap();
    (seed, step)
}

#[test]
fn endpoint_parallel_apply_equals_native_reconstruction_at_every_worker_count() {
    let (raws, frames) = window();
    for stride in [1, 4, 7] {
        let steps = strided(&raws, &frames, stride);
        for workers in [1, 2, 4, 8] {
            let mut parallel: Option<DecodedFrame> = None;
            for (tick, bytes) in &steps {
                // Every other step, an exported copy shares the baseline's tables.
                let exported = parallel.clone().filter(|_| tick % 2 == 0);
                let exported_bytes = exported.as_ref().map(canonical);
                let started = AtomicUsize::new(0);
                let step = EndpointStep::decode(bytes).unwrap();
                let sections = step.sections.len();
                let next = step
                    .apply_owned_parallel(parallel.take(), workers, &|| {
                        started.fetch_add(1, Ordering::Relaxed);
                    })
                    .unwrap();
                let expected = &frames[*tick];
                let at = format!("stride {stride}, {workers} workers, tick {tick}");
                assert_eq!(canonical(&next), canonical(&frames[*tick]), "{at}");
                assert_eq!(next.metrics, expected.metrics, "{at}");
                for table in TableId::ALL {
                    assert_eq!(
                        next.tables[table.index()].sampled_at(),
                        expected.tables[table.index()].sampled_at(),
                        "{at}: {table:?}"
                    );
                }
                // Each thread beside the caller ran the worker hook first.
                assert_eq!(started.into_inner(), workers.min(sections) - 1, "{at}");
                assert_eq!(
                    exported.as_ref().map(canonical),
                    exported_bytes,
                    "a shared baseline is never patched"
                );
                parallel = Some(next);
            }
        }
    }
    // Steps between two hours, across keyframes, truncation and refusal.
    for (at, previous) in frames.iter().enumerate().step_by(3) {
        for frame in frames[at + 1..].iter().step_by(4) {
            let mut bytes = Vec::new();
            EndpointWriter::diff(SELECTED, previous, frame, &mut bytes).unwrap();
            for workers in [2, 4, 8] {
                let reached = EndpointStep::decode(&bytes)
                    .unwrap()
                    .apply_owned_parallel(Some(previous.clone()), workers, &|| {})
                    .unwrap();
                assert_eq!(
                    canonical(&reached),
                    canonical(frame),
                    "{} to {} on {workers} workers",
                    previous.hour,
                    frame.hour
                );
            }
        }
    }
}

#[test]
fn endpoint_parallel_apply_patches_unshared_tables_in_place_and_copies_shared_ones() {
    let (raws, frames) = window();
    let buffer = |frame: &DecodedFrame| match &*frame.tables[TableId::DivisionStatus.index()] {
        TableView::Complete(TableRows::DivisionStatus(rows)) => rows.as_ptr() as usize,
        _ => panic!("complete division status"),
    };
    // Ticks 2 to 3 change fields only.
    let (seed, step) = seed_and_step(&raws, &frames, 2, 3);
    for workers in [2, 4, 8] {
        for shared in [false, true] {
            let base = EndpointStep::decode(&seed)
                .unwrap()
                .apply_owned(None)
                .unwrap();
            let before = buffer(&base);
            let held = shared.then(|| base.clone());
            let held_bytes = held.as_ref().map(canonical);
            let next = EndpointStep::decode(&step)
                .unwrap()
                .apply_owned_parallel(Some(base), workers, &|| {})
                .unwrap();
            assert_eq!(canonical(&next), canonical(&frames[3]));
            assert_eq!(
                buffer(&next) == before,
                !shared,
                "{workers} workers, shared {shared}"
            );
            assert_eq!(held.as_ref().map(canonical), held_bytes);
            if let Some(held) = held {
                assert_eq!(buffer(&held), before);
            }
        }
    }
}

#[test]
fn endpoint_parallel_apply_fails_with_the_lowest_failing_tables_error() {
    let (raws, frames) = window();
    let (seed, step) = seed_and_step(&raws, &frames, 2, 3);
    let broken = |countries: bool, status: bool| {
        let mut base = EndpointStep::decode(&seed)
            .unwrap()
            .apply_owned(None)
            .unwrap();
        if countries {
            base.tables[TableId::Countries.index()] = TableView::Unavailable {
                reason: "fixture refused census".into(),
            }
            .into();
        }
        if status {
            base.tables[TableId::DivisionStatus.index()] =
                TableView::Complete(TableRows::DivisionStatus(Vec::new())).into();
        }
        base
    };
    let error = |base: DecodedFrame, workers: usize| {
        EndpointStep::decode(&step)
            .unwrap()
            .apply_owned_parallel(Some(base), workers, &|| {})
            .unwrap_err()
            .to_string()
    };
    let countries = error(broken(true, false), 1);
    let status = error(broken(false, true), 1);
    assert_eq!(countries, "endpoint patch needs complete baseline");
    assert_eq!(status, "endpoint patch has no row baseline");
    assert_eq!(error(broken(true, true), 1), countries);
    // Whatever the schedule, the error is the lower table's.
    for workers in [2, 4, 8] {
        for _ in 0..16 {
            assert_eq!(error(broken(false, true), workers), status);
            assert_eq!(error(broken(true, true), workers), countries);
        }
    }
}

// Checked growths counted on every thread; the one `DENY_AT` names is denied.
static GROWTH_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static DENY_AT: AtomicU64 = AtomicU64::new(u64::MAX);
static DENIED: AtomicBool = AtomicBool::new(false);
fn deny_one_growth() -> bool {
    let attempt = GROWTH_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    let deny = attempt == DENY_AT.load(Ordering::SeqCst);
    if deny {
        DENIED.store(true, Ordering::SeqCst);
    }
    deny
}

#[test]
fn endpoint_parallel_apply_returns_an_allocation_error_whichever_thread_is_denied() {
    let (raws, frames) = window();
    engine_allocation::set_stop_hook(Some(deny_one_growth));
    // Field changes, a deleted country, and a step across the keyframe.
    for (from, to) in [(2, 3), (12, 13), (23, 25)] {
        let (seed, step) = seed_and_step(&raws, &frames, from, to);
        for (workers, shared) in [(2, false), (4, false), (4, true), (8, true)] {
            let mut denials = 0;
            for at in 0.. {
                assert!(at < 100_000, "denial never ends");
                let base = EndpointStep::decode(&seed)
                    .unwrap()
                    .apply_owned(None)
                    .unwrap();
                let held = shared.then(|| base.clone());
                let step = EndpointStep::decode(&step).unwrap();
                GROWTH_ATTEMPTS.store(0, Ordering::SeqCst);
                DENIED.store(false, Ordering::SeqCst);
                DENY_AT.store(at, Ordering::SeqCst);
                let result = step.apply_owned_parallel(Some(base), workers, &|| {});
                DENY_AT.store(u64::MAX, Ordering::SeqCst);
                let case = format!("{from} to {to} on {workers} workers, growth {at}");
                if !DENIED.load(Ordering::SeqCst) {
                    assert_eq!(
                        canonical(&result.unwrap()),
                        canonical(&frames[to]),
                        "{case}"
                    );
                    break;
                }
                assert_eq!(
                    result.unwrap_err().kind(),
                    std::io::ErrorKind::OutOfMemory,
                    "{case}"
                );
                if let Some(held) = held {
                    assert_eq!(canonical(&held), canonical(&frames[from]), "{case}");
                }
                denials += 1;
            }
            assert!(denials > 0, "{from} to {to} on {workers} workers grows");
        }
    }
    engine_allocation::set_stop_hook(None);
}

#[test]
fn endpoint_decoder_parallel_advance_reaches_each_step_and_the_native_decoder_continues() {
    let (raws, frames) = window();
    let steps = strided(&raws, &frames, 5);
    for workers in [2, 4, 8] {
        let mut decoder = FrameDecoder::for_tables(SELECTED);
        for (tick, bytes) in steps.iter().take(6) {
            // The held state stays shared, as playback keeps it.
            let _held = decoder.clone();
            let reached = decoder
                .advance_endpoint_parallel(EndpointStep::decode(bytes).unwrap(), workers, &|| {})
                .unwrap();
            assert_eq!(canonical(reached), canonical(&frames[*tick]));
            let mut native = decoder.clone();
            for (raw, frame) in raws.iter().zip(&frames).skip(tick + 1).take(3) {
                assert_eq!(
                    canonical(native.advance(raw).unwrap()),
                    canonical(frame),
                    "native {} after endpoint {tick} on {workers} workers",
                    frame.hour
                );
            }
        }
    }
}

#[test]
fn endpoint_carry_matches_native_identity_across_equal_snapshots_and_empty_deltas() {
    let mut encoder = FrameEncoder::new();
    let mut native = FrameDecoder::for_tables(SELECTED);
    let countries = source(0).tables[TableId::Countries.index()].clone();
    let mut raws = Vec::new();
    let mut frames = Vec::new();
    for tick in 0..6 {
        let mut input = source(tick);
        input.tables[TableId::Countries.index()] = countries.clone();
        let mut raw = encoder.encode(input, tick == 0 || tick == 4).unwrap();
        if tick == 2 {
            raw.tables[TableId::Countries.index()] = TableUpdate::Delta {
                baseline: 301,
                changed: Vec::new(),
                deleted: Vec::new(),
            };
        }
        let raw = wire(&raw);
        frames.push(native.apply(&raw).unwrap());
        raws.push(raw);
    }
    let shares = [false, true, false, true, false, true];
    for tick in 1..frames.len() {
        assert_eq!(
            SharedView::ptr_eq(
                &frames[tick - 1].tables[TableId::Countries.index()],
                &frames[tick].tables[TableId::Countries.index()],
            ),
            shares[tick],
            "native tick {tick}",
        );
    }
    for observed in [false, true] {
        for workers in [1, 4] {
            let mut writer = EndpointWriter::for_tables(SELECTED);
            let mut reached: Option<DecodedFrame> = None;
            for (tick, frame) in frames.iter().enumerate() {
                let mut bytes = Vec::new();
                if observed {
                    writer.observe(&raws[tick], frame).unwrap();
                    writer.finish(frame, &mut bytes).unwrap();
                } else if tick == 0 {
                    EndpointWriter::root(frame, SELECTED, &mut bytes).unwrap();
                } else {
                    EndpointWriter::diff(SELECTED, &frames[tick - 1], frame, &mut bytes).unwrap();
                }
                let held = reached.clone();
                let next = EndpointStep::decode(&bytes)
                    .unwrap()
                    .apply_owned_parallel(reached.take(), workers, &|| {})
                    .unwrap();
                assert_eq!(canonical(&next), canonical(frame), "tick {tick}");
                if let Some(held) = held {
                    assert_eq!(canonical(&held), canonical(&frames[tick - 1]));
                    assert_eq!(
                        SharedView::ptr_eq(
                            &held.tables[TableId::Countries.index()],
                            &next.tables[TableId::Countries.index()],
                        ),
                        shares[tick],
                        "observed {observed}, {workers} workers, tick {tick}",
                    );
                }
                reached = Some(next);
            }
        }
    }
}

#[test]
fn endpoint_not_recorded_modes_survive_roots_steps_and_native_continuation() {
    const CAPTURED: &[TableId] = &[
        TableId::Clock,
        TableId::Campaign,
        TableId::DivisionStatus,
        TableId::DivisionModelInputs,
    ];
    let mut encoder = FrameEncoder::new();
    let mut native = FrameDecoder::for_tables(CAPTURED);
    let mut writer = EndpointWriter::for_tables(CAPTURED);
    let mut reached = FrameDecoder::for_tables(CAPTURED);
    let mut frames = Vec::new();
    let mut raws = Vec::new();
    for tick in 0..5 {
        let mode = if tick < 2 {
            CaptureMode::Map
        } else {
            CaptureMode::Full
        };
        let mut input = source(tick);
        input.tables[TableId::DivisionModelInputs.index()] =
            TableView::Complete(TableRows::DivisionModelInputs(vec![
                crate::rows::DivisionModelInputRow {
                    division: 7,
                    model_number: Some(tick),
                    ..Default::default()
                },
            ]))
            .into();
        for table in TableId::ALL {
            if table.last_change().present(FORMAT_VERSION) && !crate::scope::records(mode, table) {
                input.tables[table.index()] = TableView::NotRecorded { mode }.into();
                input.metrics.table_ns[table.index()] = 0;
            }
        }
        let raw = wire(&encoder.encode(input, tick == 0 || tick == 2).unwrap());
        let frame = native.apply(&raw).unwrap();
        writer.observe(&raw, &frame).unwrap();
        let mut bytes = Vec::new();
        writer.finish(&frame, &mut bytes).unwrap();
        assert_eq!(
            canonical(
                reached
                    .advance_endpoint(EndpointStep::decode(&bytes).unwrap())
                    .unwrap()
            ),
            canonical(&frame),
        );
        let mut root = Vec::new();
        EndpointWriter::root(&frame, CAPTURED, &mut root).unwrap();
        assert_eq!(
            canonical(
                &EndpointStep::decode(&root)
                    .unwrap()
                    .apply_owned(None)
                    .unwrap()
            ),
            canonical(&frame)
        );
        let omitted = if mode == CaptureMode::Map {
            TableId::DivisionStatus
        } else {
            TableId::DivisionModelInputs
        };
        assert!(
            matches!(&*frame.tables[omitted.index()], TableView::NotRecorded { mode: actual } if *actual == mode)
        );
        assert_eq!(frame.tables[omitted.index()].sampled_at(), None);
        frames.push(frame);
        raws.push(raw);
    }
    for tick in 0..frames.len() - 1 {
        let mut root = Vec::new();
        EndpointWriter::root(&frames[tick], CAPTURED, &mut root).unwrap();
        let mut continued = FrameDecoder::for_tables(CAPTURED);
        continued
            .advance_endpoint(EndpointStep::decode(&root).unwrap())
            .unwrap();
        assert_eq!(
            canonical(continued.advance(&raws[tick + 1]).unwrap()),
            canonical(&frames[tick + 1])
        );
    }
}

#[test]
fn endpoint_transient_membership_keeps_boundaries_without_deleting_an_absent_baseline_key() {
    let selected = &[TableId::Countries];
    for baseline_has_key in [false, true] {
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(selected);
        let mut writer = EndpointWriter::for_tables(selected);
        let mut baseline = None;
        let mut final_frame = None;
        for tick in 0..=3 {
            let mut frame = source(tick);
            let rows = TableRows::Countries(
                vec![CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    command_power_raw: i64::from(tick),
                    ..Default::default()
                }]
                .into_iter()
                .chain(
                    (if baseline_has_key {
                        tick == 0 || tick == 2
                    } else {
                        tick == 1
                    })
                    .then_some(CountryRow {
                        id: 2,
                        canonical_slot: 2,
                        ..Default::default()
                    }),
                )
                .collect(),
            );
            frame.tables[TableId::Countries.index()] = TableView::Complete(rows).into();
            let raw = wire(&encoder.encode(frame, tick == 0).unwrap());
            let frame = decoder.advance(&raw).unwrap();
            writer.observe(&raw, frame).unwrap();
            if tick == 0 {
                baseline = Some(emit(&mut writer, frame, None).0);
            }
            if tick == 3 {
                final_frame = Some(frame.clone());
            }
        }
        let final_frame = final_frame.unwrap();
        let mut encoded = Vec::new();
        writer.finish(&final_frame, &mut encoded).unwrap();
        let step = EndpointStep::decode(&encoded).unwrap();
        let section = &step.sections[0];
        let Operation::Patch { deleted, rows } = &section.operation else {
            panic!("the surviving row is patched")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            deleted.as_slice(),
            if baseline_has_key { &[2][..] } else { &[][..] }
        );
        let expected = if baseline_has_key {
            vec![
                MembershipBoundary {
                    table: TableId::Countries,
                    hour: 301,
                    kind: BoundaryKind::Delta,
                    inserted: vec![],
                    deleted: vec![2],
                },
                MembershipBoundary {
                    table: TableId::Countries,
                    hour: 302,
                    kind: BoundaryKind::Delta,
                    inserted: vec![2],
                    deleted: vec![],
                },
                MembershipBoundary {
                    table: TableId::Countries,
                    hour: 303,
                    kind: BoundaryKind::Delta,
                    inserted: vec![],
                    deleted: vec![2],
                },
            ]
        } else {
            vec![
                MembershipBoundary {
                    table: TableId::Countries,
                    hour: 301,
                    kind: BoundaryKind::Delta,
                    inserted: vec![2],
                    deleted: vec![],
                },
                MembershipBoundary {
                    table: TableId::Countries,
                    hour: 302,
                    kind: BoundaryKind::Delta,
                    inserted: vec![],
                    deleted: vec![2],
                },
            ]
        };
        assert_eq!(section.boundaries, expected);
        assert_eq!(
            canonical(&step.apply(baseline.as_ref()).unwrap()),
            canonical(&final_frame)
        );
    }
}

#[test]
fn endpoint_changed_facts_are_predecessor_bound_and_cancel_before_consuming_baselines() {
    let selected = &[TableId::Countries];
    let mut encoder = FrameEncoder::new();
    let mut source_decoder = FrameDecoder::for_tables(selected);
    let mut writer = EndpointWriter::for_tables(selected);
    let mut target = FrameDecoder::for_tables(selected).with_change_facts();
    let raw = encoder.encode(source(0), true).unwrap();
    let first = source_decoder.advance(&raw).unwrap();
    writer.observe(&raw, first).unwrap();
    let mut bytes = Vec::new();
    writer.finish(first, &mut bytes).unwrap();
    target
        .advance_endpoint(EndpointStep::decode(&bytes).unwrap())
        .unwrap();
    let previous = target.current().unwrap().tables[TableId::Countries.index()].identity();
    let raw = encoder.encode(source(1), false).unwrap();
    let next = source_decoder.advance(&raw).unwrap();
    writer.observe(&raw, next).unwrap();
    bytes.clear();
    writer.finish(next, &mut bytes).unwrap();
    assert_eq!(
        target
            .advance_endpoint_parallel_cancellable(
                EndpointStep::decode(&bytes).unwrap(),
                1,
                &|| {},
                &|| true,
            )
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(target.current().unwrap().hour, first_hour());
    let current = target
        .advance_endpoint(EndpointStep::decode(&bytes).unwrap())
        .unwrap();
    assert_eq!(canonical(current), canonical(next));
    let facts = current.tables[TableId::Countries.index()]
        .changes_since(&previous)
        .unwrap();
    assert_eq!(facts.deleted(), &[u128::from(u32::MAX)]);
    assert_eq!(
        facts
            .changed()
            .iter()
            .map(|row| row.key)
            .collect::<Vec<_>>(),
        [1]
    );
    assert!(facts.row(1).unwrap().fields != 0);
    fn first_hour() -> i32 {
        300
    }
}
