use super::*;

#[test]
fn prefix_walk_keeps_independent_native_values_and_decodes_the_accepted_block_once() {
    let replay = status_replay(&(100..820).collect::<Vec<_>>());
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 720, SELECTED);
    let oracle = Replay::open(replay.bytes().to_vec()).unwrap();
    oracle.cache_decoded_sections(false);
    let frames = oracle
        .read_block_selected(&oracle.index[0], SELECTED)
        .unwrap();
    for cache in [false, true] {
        for stride in [1, 24, 168] {
            let source = Arc::new(Replay::open(replay.bytes().to_vec()).unwrap());
            source.cache_decoded_sections(cache);
            let mut map = fixture.reader(progress.clone(), source, SELECTED);
            let mut decoder = FrameDecoder::for_tables(SELECTED);
            let expected = decoder.advance(&frames[0]).unwrap();
            let dashboard_base::prepared_map::Current::Frame(initial) =
                map.seek(1, 100).unwrap().unwrap()
            else {
                panic!("recorded seed became a gap")
            };
            equal_frames(&initial, expected, SELECTED);
            let mut from = 100;
            let mut offset = 1;
            let (mut decoded, mut reads) = (0, 0);
            while from < 819 {
                let target = (from + stride).min(819);
                let mut passed = Vec::new();
                assert!(
                    map.advance_without_output(1, target, |frame| {
                        let mut bytes = Vec::new();
                        frame.encode_checkpoint(&mut bytes).unwrap();
                        passed.push((frame.hour, bytes));
                    })
                    .unwrap()
                );
                reads += map.source_reads;
                decoded += map.source_decoded_frames;
                assert_eq!(map.applied_frames, (target - from) as u64);
                let mut expected_passed = Vec::new();
                while offset < frames.len() && frames[offset].hour <= target {
                    let frame = decoder.advance(&frames[offset]).unwrap();
                    if frame.hour < target {
                        let mut bytes = Vec::new();
                        frame.encode_checkpoint(&mut bytes).unwrap();
                        expected_passed.push((frame.hour, bytes));
                    }
                    offset += 1;
                }
                assert_eq!(passed, expected_passed);
                assert_eq!(passed.len(), (target - from - 1) as usize);
                let dashboard_base::prepared_map::Current::Frame(actual) = map
                    .advance(1, target, |_| panic!("duplicate visitor"))
                    .unwrap()
                    .unwrap()
                else {
                    panic!("recorded endpoint became a gap")
                };
                equal_frames(&actual, decoder.current().unwrap(), SELECTED);
                from = target;
            }
            assert_eq!((decoded, reads, map.retained_source_frames), (720, 1, 720));
        }
    }
}

#[test]
fn prefix_admission_failure_reaches_playback_fallback_without_repeating_visitors() {
    let replay = Arc::new(status_replay(&(100..160).collect::<Vec<_>>()));
    let fixture = Fixture::new(&replay);
    let progress = prepare(&fixture, &replay, 720, SELECTED);
    let mut unreadable = Replay::open(replay.bytes().to_vec()).unwrap();
    unreadable.index[0].offset = u64::MAX;
    let map = fixture.reader(progress, Arc::new(unreadable), SELECTED);
    let mut playback = crate::playback::Playback::unloaded(Arc::clone(&replay))
        .unwrap()
        .with_checkpoints(map);
    playback.seek(1, 100).unwrap();
    let mut passed = Vec::new();
    playback
        .seek_with(1, 124, |frame| passed.push(frame.clone()))
        .unwrap();
    assert_eq!(
        passed.iter().map(|frame| frame.hour).collect::<Vec<_>>(),
        (100..124).collect::<Vec<_>>()
    );
    let oracle = Replay::open(replay.bytes().to_vec()).unwrap();
    oracle.cache_decoded_sections(false);
    let mut decoder = FrameDecoder::new();
    let frames = oracle.read_block(&oracle.index[0]).unwrap();
    for (actual, raw) in passed.iter().zip(&frames) {
        assert_eq!(actual, decoder.advance(raw).unwrap());
    }
    let expected = decoder.advance(&frames[24]).unwrap();
    assert_eq!(playback.frame(), Some(expected));
}
