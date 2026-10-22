//! Synthetic size comparison and complete grouped/prepared reconstruction of interval batches.
use super::*;
use crate::{
    frame::FrameDecoder,
    rows::{Recorded, tables},
};
use replay_data::{event_feed_rows::*, value::Value};

fn late_war_row(hour: i32) -> EventFeedHistoryRow {
    use decisions::{DecisionChange, Operation};
    let mut row = EventFeedHistoryRow {
        generation: 41,
        read_hour: hour,
        coverage: EventFeedCoverage::Complete,
        counts: Some(EventFeedCounts {
            acquired: (hour as u64 + 1) * 400,
            settled: (hour as u64 + 1) * 400,
            ..Default::default()
        }),
        script_tokens: b"synthetic-decision".to_vec(),
        script_scopes: vec![EventFeedScope {
            country: 7,
            bindings: [0; 3],
            ..Default::default()
        }],
        ..Default::default()
    };
    // Eighty countries' pass boundaries and retained-operation edges, with sparse
    // choices/fires interleaved. No content or game installation is consulted.
    for index in 0..384_u32 {
        let owner = index / 4;
        let actor = EventFeedCountry {
            reference: owner,
            id: owner,
            instance: 1,
        };
        let (source, script) = match index % 16 {
            0 => (EventFeedSource::Annex, None),
            1 => (
                EventFeedSource::EventOption,
                Some(EventFeedScript {
                    token_len: 18,
                    scope_len: 1,
                    native_event: Some(EventFeedEventIdentity {
                        kind: 1,
                        id: index % 31,
                    }),
                    instance: Some(index + 1),
                    option: Some((index % 3) as i32 - 1),
                    ..Default::default()
                }),
            ),
            2 => (
                EventFeedSource::ScriptState,
                Some(EventFeedScript {
                    change: Some(EventFeedScriptChange::DelayedAdvance {
                        count: owner % 5,
                        finished: true,
                    }),
                    ..Default::default()
                }),
            ),
            _ => (
                EventFeedSource::ScriptState,
                Some(EventFeedScript {
                    token_len: 18,
                    change: Some(EventFeedScriptChange::Decision(DecisionChange {
                        operation: if index % 4 == 0 {
                            Operation::PassBegin
                        } else {
                            Operation::PassEnd
                        },
                        container: (owner % 6) as u8,
                        flags: 0,
                        ordinal: owner % 11,
                        political_power: None,
                    })),
                    ..Default::default()
                }),
            ),
        };
        row.events.push(EventFeedEvent {
            sequence: hour as u64 * 10_000 + u64::from(index) * 2 + 7,
            generation: row.generation,
            hour: hour - (index % 3) as i32,
            source,
            actor,
            target: EventFeedCountry::default(),
            cause: Recorded::Value(EventFeedCause::default()),
            script,
        });
    }
    // Each hour four owners rebuild their targeted-decision rosters, every
    // owner once a day: one decision for each other country, as captured.
    let decision = row.script_tokens.len() as u32;
    row.script_tokens
        .extend_from_slice(b"synthetic-targeted-decision");
    for owner in (0..4).map(|n| (hour.rem_euclid(24) + 24 * n) as u32) {
        for target in (0..96).filter(|target| *target != owner) {
            let country = |reference| EventFeedCountry {
                reference,
                id: reference,
                instance: 1,
            };
            row.events.push(EventFeedEvent {
                sequence: row.events.last().unwrap().sequence + 1,
                generation: row.generation,
                hour,
                source: EventFeedSource::ScriptState,
                actor: country(owner),
                target: country(target),
                cause: Recorded::Value(EventFeedCause::default()),
                script: Some(EventFeedScript {
                    token_start: decision,
                    token_len: 27,
                    country_target: Some(owner),
                    change: Some(EventFeedScriptChange::Decision(DecisionChange {
                        operation: Operation::Candidate,
                        container: 0,
                        flags: 0,
                        ordinal: target,
                        political_power: None,
                    })),
                    ..Default::default()
                }),
            });
        }
    }
    row.intern_tokens().unwrap();
    row
}

fn old_value(row: &EventFeedHistoryRow, bytes: &mut Vec<u8>) {
    // Measurement only: the former interleaved representation, not a supported
    // recording writer or a compatibility path.
    row.generation.encode(bytes).unwrap();
    row.read_hour.encode(bytes).unwrap();
    row.coverage.encode(bytes).unwrap();
    row.counts.encode(bytes).unwrap();
    row.outstanding.encode(bytes).unwrap();
    row.failure.encode(bytes).unwrap();
    row.script_refusals.encode(bytes).unwrap();
    row.events.encode(bytes).unwrap();
    row.script_tokens.encode(bytes).unwrap();
    row.script_scopes.encode(bytes).unwrap();
}

#[test]
fn event_feed_occurrences_synthetic_late_war_encoded_zstd_effect() {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let hours = 48;
    let mut alone_before = [0; 2];
    let mut alone_after = [0; 2];
    for hour in 70_000..70_000 + hours {
        let row = late_war_row(hour);
        let mut old = Vec::new();
        old_value(&row, &mut old);
        let mut new = Vec::new();
        row.encode(&mut new).unwrap();
        let mut input = Decoder::new(&new);
        assert_eq!(EventFeedHistoryRow::decode(&mut input).unwrap(), row);
        input.finish().unwrap();
        for (index, level) in [LEVEL, STORED_LEVEL].into_iter().enumerate() {
            alone_before[index] += compress_at(&old, level).unwrap().len();
            alone_after[index] += compress_at(&new, level).unwrap().len();
        }
        before.extend(old);
        after.extend(new);
    }
    println!(
        "event_feed synthetic: {hours} hours x 764 events; encoded {} -> {} B ({:.1} -> {:.1} B/h)",
        before.len(),
        after.len(),
        before.len() as f64 / f64::from(hours),
        after.len() as f64 / f64::from(hours)
    );
    for (index, level) in [LEVEL, STORED_LEVEL].into_iter().enumerate() {
        let old = compress_at(&before, level).unwrap().len();
        let new = compress_at(&after, level).unwrap().len();
        println!(
            "event_feed zstd {level}: concatenated {old} -> {new} B ({:.2} -> {:.2} B/h, {:.1}% smaller); independently compressed hours {:.2} -> {:.2} B/h",
            old as f64 / f64::from(hours),
            new as f64 / f64::from(hours),
            (1.0 - new as f64 / old as f64) * 100.0,
            alone_before[index] as f64 / f64::from(hours),
            alone_after[index] as f64 / f64::from(hours)
        );
        assert!(
            new < old,
            "synthetic occurrence deltas should compress smaller"
        );
    }
}

/// A branch's row: a world restart at hour 70_008 of the first branch and a
/// new world on the loaded branch, with empty and refused intervals between.
fn boundary_row(branch: u64, hour: i32) -> EventFeedHistoryRow {
    let mut row = late_war_row(hour);
    let generation = match (branch, hour) {
        (1, ..70_008) => 41,
        (1, _) => 42,
        _ => 43,
    };
    row.generation = generation;
    for event in &mut row.events {
        event.generation = generation;
        event.sequence += branch * 1_000_000_000;
    }
    if hour % 4 == 0 {
        row.events.clear();
        row.intern_tokens().unwrap();
    }
    if hour % 3 == 0 {
        row.coverage = EventFeedCoverage::Partial;
        row.failure = Some(b"synthetic native refusal".to_vec());
        row.script_refusals = DECISION_FAMILY_REFUSALS;
    }
    row
}

#[test]
fn event_feed_occurrences_grouped_seek_selected_and_prepared_are_value_identical_across_boundaries()
{
    let first: Vec<_> = (70_000..70_012).collect();
    let loaded: Vec<_> = (70_003..70_009).collect();
    // A forced keyframe inside the first branch; the loaded branch starts with one.
    let replay = test_recording::build_branches(
        b"synthetic-event-occurrences",
        &[(1, &first, &[70_006]), (2, &loaded, &[])],
        |branch, hour, tables| {
            tables[TableId::EventFeedHistory.index()] = TableView::Complete(
                TableRows::EventFeedHistory(vec![boundary_row(branch, hour)]),
            );
            let mut metrics = Metrics::default();
            metrics.table_ns[TableId::EventFeedHistory.index()] = 1;
            metrics
        },
    );
    let selected = [TableId::EventFeedHistory];
    let mut hours = 0;
    for block in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for frame in replay.read_block_selected(block, &selected).unwrap() {
            hours += 1;
            let decoded = decoder.apply(&frame).unwrap();
            assert_eq!(
                decoded.complete::<tables::EventFeedHistory>().unwrap(),
                &[boundary_row(frame.branch, frame.hour)]
            );
            let mut prepared = Vec::new();
            decoded.encode_checkpoint(&mut prepared).unwrap();
            let restored = DecodedFrame::decode_checkpoint(&mut Decoder::new(&prepared)).unwrap();
            assert_eq!(
                restored.complete::<tables::EventFeedHistory>().unwrap(),
                decoded.complete::<tables::EventFeedHistory>().unwrap()
            );
            let crate::container::SeekResult::Frame(seek) = replay
                .seek_selected(frame.branch, frame.hour, Some(&selected))
                .unwrap()
            else {
                panic!("known hour refused")
            };
            assert_eq!(
                seek.complete::<tables::EventFeedHistory>().unwrap(),
                decoded.complete::<tables::EventFeedHistory>().unwrap()
            );
        }
    }
    assert_eq!(hours, first.len() + loaded.len());
    let bytes: u64 = replay
        .index
        .iter()
        .flat_map(|block| replay.block_sections(block).unwrap().unwrap())
        .filter(|size| {
            matches!(
                size.section,
                Section::Keyframe(TableId::EventFeedHistory)
                    | Section::Later(TableId::EventFeedHistory)
            )
        })
        .map(|size| size.compressed)
        .sum();
    println!(
        "event_feed synthetic grouped sections: {bytes} B over {hours} hours ({:.1} B/h)",
        bytes as f64 / hours as f64
    );
}
