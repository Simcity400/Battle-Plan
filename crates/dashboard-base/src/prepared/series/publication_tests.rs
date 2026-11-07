use super::*;
use crate::prepared::pages::Pages;

const COUNTRIES: u64 = 5_000;

#[derive(Default)]
struct PagesInMemory(Vec<Vec<u8>>);
impl Pages for PagesInMemory {
    fn write_at(&mut self, _: &str, range: u64, bytes: &[u8]) -> io::Result<()> {
        self.0[range as usize] = bytes.to_vec();
        Ok(())
    }
    fn write(&mut self, _: &str, bytes: &[u8]) -> io::Result<u64> {
        let range = self.0.len() as u64;
        self.0.push(bytes.to_vec());
        Ok(range)
    }
    fn read_at(&mut self, _: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.get(range as usize).cloned())
    }
}
fn flush(values: &mut Accumulator, pages: &mut PagesInMemory, reference: bool, decimals: u8) {
    WHOLE_REGISTRATION.set(reference);
    WHOLE_SUMMARY_MAPS.set(reference);
    WHOLE_PAYLOADS.set(reference);
    values
        .flush(pages, "values", decimals, &AtomicBool::new(false))
        .unwrap();
    WHOLE_REGISTRATION.set(false);
    WHOLE_SUMMARY_MAPS.set(false);
    WHOLE_PAYLOADS.set(false);
}
fn saved(values: &Accumulator, reference: bool) -> Vec<u8> {
    saved::WHOLE_SAVED_KEYS.set(reference);
    let bytes = serde_json::to_vec(values).unwrap();
    saved::WHOLE_SAVED_KEYS.set(false);
    bytes
}

#[test]
fn flush_keeps_key_states_and_merges_ordered_runs_without_trees() {
    let mut values = [Accumulator::default(), Accumulator::default()];
    let mut pages = [PagesInMemory::default(), PagesInMemory::default()];
    for (batch, (first, last)) in [(-19, 17), (18, 39), (43, 80)].into_iter().enumerate() {
        for (index, values) in values.iter_mut().enumerate() {
            values.through(first);
            for key in 0..COUNTRIES {
                for hour in first..=last {
                    values.push_boundary(
                        key,
                        Run::new(
                            hour,
                            hour,
                            i128::from((hour / 3 + key as i32) % 7),
                            hour == first,
                        ),
                        hour % 9 == 0,
                    );
                }
            }
            // Event rows share dates and cross every alignment, including negatives.
            for hour in first..=last {
                values.event(COUNTRIES, hour, &(hour, batch)).unwrap();
            }
            let maps = SUMMARY_MAP_WORK.get();
            let states = STATE_MAP_WORK.get();
            flush(values, &mut pages[index], index == 1, 5);
            assert_eq!(
                STATE_MAP_WORK.get() - states,
                if index == 1 {
                    COUNTRIES as usize + 1
                } else {
                    0
                }
            );
            if index == 0 {
                assert_eq!(SUMMARY_MAP_WORK.get(), maps);
            } else {
                assert!(SUMMARY_MAP_WORK.get() - maps >= COUNTRIES as usize * LEVELS.len());
            }
        }
        assert_eq!(pages[0].0, pages[1].0);
        assert_eq!(saved(&values[0], false), saved(&values[1], true));
        if batch == 1 {
            for values in &mut values {
                *values = serde_json::from_slice(&saved(values, false)).unwrap();
            }
        }
    }
}

#[test]
fn idle_flushes_reuse_only_equal_payloads_and_keep_boundaries_resume_and_append_bytes() {
    let mut values = [Accumulator::default(), Accumulator::default()];
    let mut pages = [PagesInMemory::default(), PagesInMemory::default()];
    let mut reused = 0;
    for (batch, (first, last)) in [
        (-7, -3),
        (-2, 0),
        (1, 2),
        (3, 4),
        (5, 6),
        (7, 15),
        (16, 17),
        (18, 19),
        (20, 21),
        (22, 255),
        (256, 257),
        (258, 259),
        (260, 4095),
        (4096, 4097),
        (4098, 4099),
        (4100, 65535),
        (65536, 65537),
    ]
    .into_iter()
    .enumerate()
    {
        for (index, values) in values.iter_mut().enumerate() {
            values.through(first);
            if batch == 0 || batch == 7 {
                for key in 0..COUNTRIES {
                    values.push(key, Run::new(first, last, i128::from(key % 7), true));
                }
            }
            // A changed key, a supplying boundary, an unavailable value and a gap
            // sit beside the untouched keys. No authenticated carry is implied.
            values.observe_boundary(
                5_001,
                first,
                (batch != 4).then_some(123),
                batch == 0,
                batch == 8,
            );
            values.through(last);
            let before = idle::IDLE_REUSES.get();
            flush(
                values,
                &mut pages[index],
                index == 1,
                if batch < 11 { 5 } else { 0 },
            );
            if index == 0 {
                reused += idle::IDLE_REUSES.get() - before;
            }
        }
        assert_eq!(pages[0].0, pages[1].0, "batch {batch}");
        assert_eq!(
            saved(&values[0], false),
            saved(&values[1], true),
            "batch {batch}"
        );
        if batch == 5 {
            for values in &mut values {
                *values = serde_json::from_slice(&saved(values, false)).unwrap();
            }
        }
    }
    assert!(
        reused >= 7,
        "quiet payloads must survive repeated flushes: {reused}"
    );
    assert_eq!(values[0].idle.counts().0, 0);
    assert!(values[0].idle.counts().1 <= 8);
}

#[test]
fn unchanged_continuation_keys_are_encoded_once_and_public_mutations_invalidate_them() {
    let mut values = Accumulator::default();
    for key in 0..COUNTRIES {
        values.event(key, -3, &key).unwrap();
    }
    let mut pages = PagesInMemory::default();
    flush(&mut values, &mut pages, false, 0);
    let before = saved::SAVED_KEY_WORK.get();
    assert_eq!(saved(&values, false), saved(&values, true));
    assert_eq!(saved::SAVED_KEY_WORK.get() - before, COUNTRIES as usize);
    let json: serde_json::Value = serde_json::from_slice(&saved(&values, false)).unwrap();
    assert_eq!(
        json.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["active", "frontier", "previous", "states"]
    );
    for hour in 0..4 {
        values.through(hour);
        values.event(7, hour, &hour).unwrap();
        flush(&mut values, &mut pages, false, 0);
        let before = saved::SAVED_KEY_WORK.get();
        assert_eq!(saved(&values, false), saved(&values, true));
        assert_eq!(saved::SAVED_KEY_WORK.get() - before, 1);
    }
    // Equality checks the stored operands, including floating-point sign bits.
    let state = values.states.get_mut(&7).unwrap();
    state.last = Some(Run::new(0, 3, 5, false));
    state.partial.set(
        0,
        Some(Partial::Span {
            aligned: 0,
            first: 0,
            last: 3,
            value: 5,
            first_break: false,
        }),
    );
    assert_eq!(saved(&values, false), saved(&values, true));
    let state = values.states.get_mut(&7).unwrap();
    let mut bucket = constant_bucket(0, 0, 3, false, 0);
    bucket.first_value = -0.0;
    state.partial.set(0, Some(Partial::Bucket { aligned: 0, bucket }));
    let negative_zero = saved(&values, false);
    let state = values.states.get_mut(&7).unwrap();
    let Some(Partial::Bucket { aligned, mut bucket }) = state.partial.level(0) else {
        unreachable!()
    };
    bucket.first_value = 0.0;
    state.partial.set(0, Some(Partial::Bucket { aligned, bucket }));
    let before = saved::SAVED_KEY_WORK.get();
    let positive_zero = saved(&values, false);
    assert_ne!(negative_zero, positive_zero);
    assert_eq!(saved::SAVED_KEY_WORK.get() - before, 1);
    assert_eq!(positive_zero, saved(&values, true));
    let mut restored: Accumulator = serde_json::from_slice(&saved(&values, false)).unwrap();
    let before = saved::SAVED_KEY_WORK.get();
    assert_eq!(saved(&restored, false), saved(&values, true));
    assert_eq!(saved::SAVED_KEY_WORK.get() - before, COUNTRIES as usize);
    restored.event(7, 8, &"append").unwrap();
    flush(&mut restored, &mut pages, false, 0);
    assert_eq!(saved(&restored, false), saved(&restored, true));
}

#[test]
fn backdated_or_overlapping_runs_keep_the_ordered_map_and_exact_bytes() {
    for runs in [
        vec![Run::new(-5, 8, 19, true), Run::new(3, 10, 7, false)],
        vec![Run::new(8, 12, 19, true), Run::new(-3, 2, 7, true)],
    ] {
        let mut pages = [PagesInMemory::default(), PagesInMemory::default()];
        for (index, pages) in pages.iter_mut().enumerate() {
            let mut values = Accumulator::default();
            values.through(-5);
            for &run in &runs {
                values.push(0, run);
            }
            values.through(12);
            let before = SUMMARY_MAP_WORK.get();
            flush(&mut values, pages, index == 1, 5);
            assert_eq!(SUMMARY_MAP_WORK.get() - before, LEVELS.len());
        }
        assert_eq!(pages[0].0, pages[1].0);
    }
}

#[test]
fn nonconstant_partial_buckets_keep_exact_totals_through_restore_and_append() {
    for (runs, total) in [
        (
            [Run::new(8, 12, 19, true), Run::new(0, 2, 7, true)],
            116_u128,
        ),
        (
            [
                Run::new(0, 0, 1, true),
                Run::new(10, 10, i128::MAX / 2, true),
            ],
            (i128::MAX / 2) as u128 + 1,
        ),
    ] {
        // The first merge has reversed endpoints; the second has a valid
        // total even though filling its gaps with the final value would
        // overflow. Neither merged bucket is a constant span.
        let publish = |whole| {
            WHOLE_BUCKETS.set(whole);
            let mut values = Accumulator::default();
            let mut pages = PagesInMemory::default();
            let last = runs.iter().map(|run| run.last_hour).max().unwrap();
            values.through(0);
            for run in runs {
                values.push(0, run);
            }
            values.through(last);
            flush(&mut values, &mut pages, false, 0);
            for partial in values.state(0).unwrap().partial.get() {
                assert_eq!(partial.unwrap().bucket(0).absolute_total, total);
            }
            let continuation = saved(&values, false);
            let mut restored: Accumulator = serde_json::from_slice(&continuation).unwrap();
            assert_eq!(saved(&restored, false), continuation);
            let mut restored_pages = PagesInMemory(pages.0.clone());
            for (values, pages) in [
                (&mut values, &mut pages),
                (&mut restored, &mut restored_pages),
            ] {
                values.push(0, Run::new(last + 1, last + 1, 3, false));
                flush(values, pages, false, 0);
                for partial in values.state(0).unwrap().partial.get() {
                    assert_eq!(partial.unwrap().bucket(0).absolute_total, total + 3);
                }
            }
            assert_eq!(pages.0, restored_pages.0);
            assert_eq!(saved(&values, false), saved(&restored, false));
            WHOLE_BUCKETS.set(false);
            pages.0
        };
        assert_eq!(publish(false), publish(true));
    }
}

#[test]
fn cached_continuation_has_the_same_stream_compression_bytes() {
    let mut values = Accumulator::default();
    for key in 0..COUNTRIES {
        values.push(key, Run::new(-7, 19, i128::from(key % 11) * -10_001, true));
    }
    let mut pages = PagesInMemory::default();
    flush(&mut values, &mut pages, false, 5);
    let compressed = |reference| {
        saved::WHOLE_SAVED_KEYS.set(reference);
        let mut writer = io::BufWriter::with_capacity(
            1 << 20,
            zstd::stream::Encoder::new(Vec::new(), 1).unwrap(),
        );
        serde_json::to_writer(&mut writer, &values).unwrap();
        let bytes = writer
            .into_inner()
            .map_err(io::IntoInnerError::into_error)
            .unwrap()
            .finish()
            .unwrap();
        saved::WHOLE_SAVED_KEYS.set(false);
        bytes
    };
    assert_eq!(compressed(false), compressed(true));
    assert_eq!(compressed(false), compressed(true));
}

#[test]
fn active_continuation_keys_stream_without_retaining_duplicate_states() {
    let mut values = Accumulator::default();
    let mut pages = PagesInMemory::default();
    values.advance(0).unwrap();
    for key in 0..COUNTRIES {
        values.observe(key, 0, Some(i128::from(key % 7)), true);
    }
    for (first, last) in [(1, 7), (8, 16)] {
        values.advance_span(first, last).unwrap();
        flush(&mut values, &mut pages, false, 5);
        let before = saved::SAVED_KEY_WORK.get();
        assert_eq!(saved(&values, false), saved(&values, true));
        assert_eq!(saved::SAVED_KEY_WORK.get(), before);
    }
}

#[test]
fn cancelled_flush_restores_its_checkpoint_before_append_without_reusing_rejected_work() {
    let mut values = [Accumulator::default(), Accumulator::default()];
    let mut pages = [PagesInMemory::default(), PagesInMemory::default()];
    for (index, values) in values.iter_mut().enumerate() {
        values.through(0);
        for key in 0..COUNTRIES {
            values.push(key, Run::new(0, 3, i128::from(key % 7), true));
        }
        values.through(3);
        flush(values, &mut pages[index], index == 1, 5);
    }
    let checkpoint = saved(&values[0], false);
    values[0].event(7, 4, &"cancelled").unwrap();
    assert_eq!(
        values[0]
            .flush(&mut pages[0], "values", 5, &AtomicBool::new(true))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(pages[0].0.len(), 1);
    values[0] = serde_json::from_slice(&checkpoint).unwrap();
    for (index, values) in values.iter_mut().enumerate() {
        values.through(4);
        values.event(7, 5, &"append").unwrap();
        values.through(6);
        flush(values, &mut pages[index], index == 1, 5);
    }
    assert_eq!(pages[0].0, pages[1].0);
    assert_eq!(saved(&values[0], false), saved(&values[1], true));
}
