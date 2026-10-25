use super::family_lanes::payloads;
use super::*;
use dashboard_base::prepared_map::{self, Reader, pages};
use std::{collections::BTreeMap, ops::RangeInclusive, time::Duration};

fn hours(branch: u64, hours: RangeInclusive<i32>) -> Vec<DecodedFrame> {
    hours
        .filter(|&hour| hour != 40)
        .map(|hour| frame(branch, hour, 2))
        .collect()
}

/// Branch 1's first block: hours 1 to 50.
fn first_block() -> Replay {
    Replay::from_records(CAMPAIGN.into(), &[records(1, hours(1, 1..=50))]).unwrap()
}

/// `first` grown by branch 1's second block and a block of branch 2.
fn grown(first: &Replay) -> Replay {
    let replay = first
        .append(&[continuation_records(first, 1, hours(1, 51..=130))])
        .unwrap();
    replay.append(&[records(2, hours(2, 25..=95))]).unwrap()
}

/// Prepare `source` with map pages the one family prepared on its own.
fn prepare(source: &Source, flag: &AtomicBool) {
    independent::with_test_families(&[&independent::map_pages::MAP_PAGES], || {
        run(source, None, flag, |_| {}).unwrap();
    });
}

fn published(source: &Source) -> BTreeMap<(String, u64), Vec<u8>> {
    payloads(source, None)
        .into_iter()
        .filter(|((name, _), _)| name.starts_with("map-pages."))
        .collect()
}

fn assert_native_pages(source: &Source, replay: &Replay) {
    let reference = Replay::open(replay.bytes().to_vec()).unwrap();
    reference.cache_decoded_sections(false);
    let selected = prepared_map::selected_tables();
    let mut checked = 0;
    for ((name, _), bytes) in published(source) {
        if name.starts_with("map-pages.progress.") || name.starts_with("map-pages.state.") {
            continue;
        }
        let (header, step) = pages::read_page(bytes.as_slice(), selected).unwrap();
        let replay::container::SeekResult::Frame(before) = reference
            .seek_selected(header.branch, header.from, Some(selected))
            .unwrap()
        else {
            panic!("native page baseline")
        };
        let actual = step.apply_owned(Some(*before)).unwrap();
        let replay::container::SeekResult::Frame(expected) = reference
            .seek_selected(header.branch, header.to, Some(selected))
            .unwrap()
        else {
            panic!("native page end")
        };
        let mut actual_bytes = Vec::new();
        let mut expected_bytes = Vec::new();
        actual.encode_checkpoint(&mut actual_bytes).unwrap();
        expected.encode_checkpoint(&mut expected_bytes).unwrap();
        assert_eq!(
            actual_bytes, expected_bytes,
            "{name} {} to {}",
            header.from, header.to
        );
        checked += 1;
    }
    assert!(checked > 0, "committed page payloads checked natively");
}

#[test]
fn map_pages_commit_with_their_blocks_resume_and_bind_a_reader_answering_as_the_native_route() {
    let replay = grown(&first_block());
    let flag = AtomicBool::new(false);
    let mut published_runs = Vec::new();
    for resumed in [false, true] {
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source(if resumed {
            "map-pages-resumed"
        } else {
            "map-pages"
        });
        if resumed {
            independent::with_test_families(&[&independent::map_pages::MAP_PAGES], || {
                super::super::RELOAD_FLUSHED.with(|reload| reload.set(true));
                while unprepared_blocks(&source, None).unwrap() != Some(0) {
                    run_measured(&source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
                }
                super::super::RELOAD_FLUSHED.with(|reload| reload.set(false));
            });
        }
        prepare(&source, &flag);
        let (store, key, progress) = map_checkpoint_binding(&source, None).unwrap().unwrap();
        let committed = pages::committed(&store, &key, &progress).unwrap();
        let branches: Vec<_> = committed.iter().map(|&(branch, _)| branch).collect();
        assert_eq!(branches, [1, 2]);
        for (_, chains) in &committed {
            let strides: Vec<_> = chains.chains.iter().map(|chain| chain.stride).collect();
            assert_eq!(strides, pages::provisional_strides());
        }
        let reader = |paged: bool| {
            let reader = Reader::new(
                Arc::clone(&store),
                key.clone(),
                progress.clone(),
                source.replay(),
                prepared_map::selected_tables(),
            )
            .unwrap();
            if paged {
                reader.with_pages(committed.clone()).unwrap()
            } else {
                reader
            }
        };
        let mut applied = 0;
        for (&branch, &(first, last)) in &progress.ranges {
            for spacing in [1, 20] {
                let (mut paged, mut plain) = (reader(true), reader(false));
                for hour in (first..=last).filter(|hour| hour % spacing == 0) {
                    let mut visits = Vec::new();
                    let answer = paged
                        .advance_paged(branch, hour, hour, |frame| visits.push(frame.clone()))
                        .map_err(|error| error.to_string());
                    applied += paged.applied_pages;
                    let native = plain
                        .advance(branch, hour, |_| {})
                        .map_err(|error| error.to_string());
                    assert_eq!(answer, native, "branch {branch} hour {hour}");
                    if let Ok(Some(actual)) = &answer {
                        let replay = Replay::open(replay.bytes().to_vec()).unwrap();
                        replay.cache_decoded_sections(false);
                        let expected = replay
                            .seek_selected(branch, hour, Some(prepared_map::selected_tables()))
                            .unwrap();
                        match (actual, expected) {
                            (
                                prepared_map::Current::Frame(actual),
                                replay::container::SeekResult::Frame(expected),
                            ) => {
                                let mut actual_bytes = Vec::new();
                                let mut expected_bytes = Vec::new();
                                actual.encode_checkpoint(&mut actual_bytes).unwrap();
                                expected.encode_checkpoint(&mut expected_bytes).unwrap();
                                assert_eq!(
                                    actual_bytes, expected_bytes,
                                    "native endpoint {branch}/{hour}"
                                );
                            }
                            (
                                prepared_map::Current::Gap { reason, .. },
                                replay::container::SeekResult::Gap {
                                    reason: expected, ..
                                },
                            ) => assert_eq!(reason, &expected),
                            pair => panic!("native endpoint {branch}/{hour}: {pair:?}"),
                        }
                    }
                    assert!(visits.is_empty(), "branch {branch} hour {hour}");
                }
            }
        }
        assert!(applied > 0, "pages were applied");
        assert_native_pages(&source, &replay);
        let pages = published(&source);
        assert!(!pages.is_empty());
        published_runs.push(pages);
    }
    assert_eq!(
        published_runs[0], published_runs[1],
        "a resumed preparation publishes the pages of one run"
    );
}

#[test]
fn a_grown_recording_continues_from_saved_chain_states_without_reading_accepted_blocks() {
    let first = first_block();
    let whole = grown(&first);
    let flag = AtomicBool::new(false);
    let mut reference = Fixture::new(&whole);
    let source = reference.source("map-pages-whole");
    prepare(&source, &flag);
    assert_native_pages(&source, &whole);
    let expected = published(&source);
    drop(source);
    let mut fixture = Fixture::new(&first);
    let source = fixture.source("map-pages-first");
    prepare(&source, &flag);
    let (store, key, progress) = map_checkpoint_binding(&source, None).unwrap().unwrap();
    let committed = pages::committed(&store, &key, &progress).unwrap();
    assert_eq!(committed[0].1.chains[0].last, Some(20));
    drop(source);
    fixture.replace(&whole);
    let source = fixture.source("map-pages-grown");
    source.replay_ref().log_section_decodes();
    independent::with_test_families(&[&independent::map_pages::MAP_PAGES], || {
        let accepted = run(&source, None, &flag, |_| {}).unwrap();
        assert_eq!(accepted.new_blocks, whole.index.len() - first.index.len());
    });
    let read_blocks: std::collections::BTreeSet<_> = source
        .replay_ref()
        .wire_decodes()
        .iter()
        .map(|read| read.block)
        .collect();
    assert_eq!(
        read_blocks,
        (first.index.len()..whole.index.len()).collect()
    );
    assert_native_pages(&source, &whole);
    assert_eq!(
        published(&source),
        expected,
        "the tail continues the chains from their saved states"
    );
}
