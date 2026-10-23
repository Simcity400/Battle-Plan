use super::super::pages::{
    BlockPages, BranchPages, FAMILY, HEADER_BYTES, Header, PageExtract, PageFold,
    STRIDES as PRODUCTION_STRIDES, chain_point, committed, progress_section, provisional_strides,
    section,
};
use super::super::{
    Builder, Current, Position, Progress, Reader, compress, reader, section as checkpoint_section,
};
use super::SELECTED;
use crate::prepared::{
    frame_changes::FrameChangeTracker, prepass::Extract, series::tests::Fixture,
};
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
use replay::{container::Replay, container::SeekResult, spool::Record};
use replay_data::{
    BranchId, Hour,
    rows::{CampaignRow, DivisionStatusRow, TableId, TableRows},
};
use replay_frame::{
    branches::Gap, frame::DecodedFrame, frame::FrameDecoder, frame::TableView,
    frame::endpoint_step::EndpointWriter,
};
use std::{
    cell::Cell,
    collections::BTreeMap,
    io::{self, Read},
    num::NonZeroU32,
    ops::ControlFlow,
    rc::Rc,
    sync::Arc,
};
use store::Store;

const STRIDES: &[u32] = &[3, 8];
const CHECKPOINTS: u32 = 25;

fn observed(branch: BranchId, hour: Hour) -> DecodedFrame {
    let mut frame = scale_frame(branch, hour, 3);
    frame.metrics.table_ns[TableId::DivisionStatus.index()] = 1;
    let status = |division: i32| DivisionStatusRow {
        division: division as u64,
        maximum_hit_points: i64::from(hour / 3) * 1_000 + i64::from(division),
        ..Default::default()
    };
    frame.tables[TableId::DivisionStatus.index()] = match hour.rem_euclid(17) {
        5 => TableView::Truncated {
            rows: TableRows::DivisionStatus(vec![status(1)]),
            missing: 7,
        },
        6 => TableView::Unavailable {
            reason: "status census failed".into(),
        },
        _ => TableView::Complete(TableRows::DivisionStatus(
            (1..=4)
                .filter(|division| (hour + division) % 5 != 0)
                .map(status)
                .collect(),
        )),
    }
    .into();
    frame
}

fn frames(
    branch: BranchId,
    hours: std::ops::RangeInclusive<Hour>,
    holes: &[Hour],
) -> Vec<DecodedFrame> {
    hours
        .filter(|hour| !holes.contains(hour))
        .map(|hour| observed(branch, hour))
        .collect()
}

fn recording() -> Replay {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames(1, 100..=130, &[111, 112]))],
    )
    .unwrap();
    let replay = first
        .append(&[continuation_records(&first, 1, frames(1, 131..=150, &[]))])
        .unwrap();
    let mut after_gap = continuation_records(&replay, 1, frames(1, 160..=190, &[]));
    after_gap.push(Record::Gap(Gap {
        branch: 1,
        first_hour: 151,
        last_hour: 159,
        reason: "flush failed".into(),
    }));
    let replay = replay.append(&[after_gap]).unwrap();
    let replay = replay
        .append(&[records(2, frames(2, 20..=45, &[]))])
        .unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 2, frames(2, 46..=60, &[50]))])
        .unwrap();
    assert!(replay.warnings.is_empty(), "{:?}", replay.warnings);
    replay
}

fn damage(replay: &Replay, block: usize) -> Replay {
    let index = &replay.index[block];
    let sections = replay.block_sections(index).unwrap().unwrap();
    let last = sections
        .iter()
        .rposition(|section| matches!(section.section, replay::container::Section::Later(_)))
        .unwrap();
    let after: u64 = sections[last + 1..]
        .iter()
        .map(|section| section.compressed)
        .sum();
    let mut bytes = replay.bytes().to_vec();
    bytes[(index.offset + index.length - after - 1) as usize] ^= 0x40;
    Replay::open(bytes).unwrap()
}

type Rewrite<'a> = dyn Fn(&Header, Vec<u8>) -> Option<Vec<u8>> + 'a;

struct Prepared {
    fixture: Fixture,
    map: Progress,
    pages: BTreeMap<BranchId, BranchPages>,
    refused: BTreeMap<BranchId, (Hour, String)>,
    replay: Arc<Replay>,
}

impl Prepared {
    fn new(map_source: &Replay, replay: Replay, accepted: Hour) -> Self {
        Self::rewritten(map_source, replay, accepted, &|_, page| Some(page))
    }

    fn rewritten(
        map_source: &Replay,
        replay: Replay,
        accepted: Hour,
        rewrite: &Rewrite<'_>,
    ) -> Self {
        let replay = Arc::new(replay);
        let fixture = Fixture::new();
        let mut builder = Builder::new(
            NonZeroU32::new(CHECKPOINTS).unwrap(),
            SELECTED,
            Progress::default(),
        )
        .unwrap();
        builder.set_ranges(&replay);
        let mut folds: BTreeMap<BranchId, PageFold> = BTreeMap::new();
        let mut refused = BTreeMap::new();
        let mut decoder = FrameDecoder::for_tables(SELECTED);
        let mut writer = fixture.writer();
        for (block, index) in map_source.index.iter().enumerate() {
            for (offset, frame) in map_source
                .read_block_selected(index, SELECTED)
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
            if index.last_hour <= accepted && !refused.contains_key(&index.branch) {
                let pages =
                    BlockPages::produce(&replay, block, SELECTED, STRIDES, || Ok(())).unwrap();
                let fold = folds.entry(index.branch).or_insert_with(|| {
                    PageFold::new(index.branch, SELECTED, STRIDES, None).unwrap()
                });
                if let Some(refusal) = fold.fold(block, pages).unwrap() {
                    refused.insert(index.branch, refusal);
                }
            }
        }
        let scratch = Fixture::new();
        let mut produced = scratch.writer();
        for fold in folds.values_mut() {
            fold.publish(&mut produced).unwrap();
        }
        produced.commit().unwrap();
        for (&branch, fold) in &folds {
            for chain in &fold.pages().chains {
                let name = section(branch, chain.stride);
                for range in 0..chain.ranges {
                    let mut page = Vec::new();
                    scratch
                        .store
                        .read(&scratch.key, &name, range)
                        .unwrap()
                        .unwrap()
                        .read_to_end(&mut page)
                        .unwrap();
                    let header = Header::read(page.as_slice()).unwrap();
                    let Some(page) = rewrite(&header, page) else {
                        break;
                    };
                    writer.write(&name, page.as_slice()).unwrap();
                }
            }
            let chains = serde_json::to_vec(fold.pages()).unwrap();
            writer
                .write_at(&progress_section(branch), 0, chains.as_slice())
                .unwrap();
            fold.save_states(&mut writer).unwrap();
        }
        builder.save_gaps(&mut writer, &replay.gaps).unwrap();
        writer.commit().unwrap();
        let map = builder.progress().clone();
        let pages = committed(&fixture.store, &fixture.key, &map).unwrap();
        assert_eq!(
            pages,
            folds
                .iter()
                .map(|(&branch, fold)| (branch, fold.pages().clone()))
                .collect::<Vec<_>>()
        );
        let pages = pages.into_iter().collect();
        Self {
            fixture,
            map,
            pages,
            refused,
            replay,
        }
    }

    /// Another handle on the prepared store.
    fn store(&self) -> Store {
        Store::local(
            self.fixture
                .directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap()
    }

    fn reader(&self, pages: bool) -> Reader {
        let reader = Reader::new(
            self.store(),
            self.fixture.key.clone(),
            self.map.clone(),
            Arc::clone(&self.replay),
            SELECTED,
        )
        .unwrap();
        if pages {
            reader.with_committed_pages()
        } else {
            reader
        }
    }

    /// Each chain's stride and last chain point on `branch`.
    fn lasts(&self, branch: BranchId) -> Vec<(u32, Option<Hour>)> {
        self.pages[&branch]
            .chains
            .iter()
            .map(|chain| (chain.stride, chain.last))
            .collect()
    }

    /// The start and end of each page of `branch`'s chain of `stride`.
    fn chain(&self, branch: BranchId, stride: u32) -> Vec<(Hour, Hour)> {
        let chain = self.pages[&branch]
            .chains
            .iter()
            .find(|chain| chain.stride == stride)
            .unwrap();
        (0..chain.ranges)
            .map(|range| {
                let page = self
                    .fixture
                    .store
                    .read(&self.fixture.key, &section(branch, stride), range)
                    .unwrap()
                    .unwrap();
                let header = Header::read(page).unwrap();
                (header.from, header.to)
            })
            .collect()
    }
}

fn bytes(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes).unwrap();
    bytes
}

fn canonical(current: &Current) -> Option<Vec<u8>> {
    match current {
        Current::Frame(frame) => Some(bytes(frame)),
        Current::Gap { .. } => None,
    }
}

fn native(replay: &Replay, branch: BranchId, hour: Hour) -> Current {
    match replay.seek_selected(branch, hour, Some(SELECTED)).unwrap() {
        SeekResult::Frame(frame) => Current::Frame(frame.into()),
        SeekResult::Gap {
            branch,
            hour,
            reason,
        } => Current::Gap {
            branch,
            hour,
            reason,
        },
    }
}

type Answer = Result<Option<Current>, String>;

fn same(
    paged: &mut Reader,
    plain: &mut Reader,
    branch: BranchId,
    hour: Hour,
    visit_from: Hour,
) -> (Answer, u64) {
    let (mut paged_visits, mut plain_visits) = (Vec::new(), Vec::new());
    let actual = paged
        .advance_paged(branch, hour, visit_from, |frame| {
            paged_visits.push((frame.hour, bytes(frame)))
        })
        .map_err(|error| error.to_string());
    let applied = paged.applied_pages;
    if let Ok(Some(actual)) = &actual {
        let replay = Replay::open(paged.replay_handle().bytes().to_vec()).unwrap();
        replay.cache_decoded_sections(false);
        let reference = native(&replay, branch, hour);
        assert_eq!(
            canonical(actual),
            canonical(&reference),
            "native endpoint {branch}/{hour}"
        );
        for &(at, ref actual) in &paged_visits {
            assert_eq!(
                Some(actual.clone()),
                canonical(&native(&replay, branch, at)),
                "native visitor {branch}/{at}"
            );
        }
    }
    let expected = plain
        .advance(branch, hour, |frame| {
            if frame.hour >= visit_from {
                plain_visits.push((frame.hour, bytes(frame)));
            }
        })
        .map_err(|error| error.to_string());
    assert_eq!((plain.applied_pages, plain.page_fallbacks), (0, 0));
    assert_eq!(actual, expected, "branch {branch} hour {hour}");
    if let (Ok(Some(actual)), Ok(Some(expected))) = (&actual, &expected) {
        assert_eq!(
            canonical(actual),
            canonical(expected),
            "canonical bytes at branch {branch} hour {hour}"
        );
    }
    assert_eq!(
        paged_visits, plain_visits,
        "visits at branch {branch} hour {hour}"
    );
    let (cursor, page_end, requested, _) = paged.continuation();
    if applied == 0 && matches!(actual, Ok(Some(_))) && !page_end {
        let (expected_cursor, _, expected_requested, _) = plain.continuation();
        assert_eq!(
            (cursor, requested),
            (expected_cursor, expected_requested),
            "continuation at branch {branch} hour {hour}"
        );
    }
    (actual, applied)
}

fn compare_every_hour(prepared: &Prepared, branch: BranchId, truth: bool) -> u64 {
    let (first, last) = prepared.reader(true).range(branch).unwrap();
    let mut applied = 0;
    for hour in first..=last {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        let (answer, pages) = same(&mut paged, &mut plain, branch, hour, hour);
        applied += pages;
        if truth && let Ok(Some(answer)) = &answer {
            let native = native(&prepared.replay, branch, hour);
            assert_eq!(answer, &native, "native hour {hour}");
            assert_eq!(
                canonical(answer),
                canonical(&native),
                "native bytes at {hour}"
            );
        }
    }
    for spacing in [1, 3, 7, 8, 16, 40] {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        for hour in (first..=last).filter(|hour| hour % spacing == 0) {
            applied += same(&mut paged, &mut plain, branch, hour, hour).1;
        }
    }
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    for hour in (first..=last).step_by(5) {
        applied += same(&mut paged, &mut plain, branch, hour, hour - 2).1;
    }
    for hour in (first..=last).rev().step_by(7) {
        applied += same(&mut paged, &mut plain, branch, hour, hour).1;
    }
    applied
}

#[test]
fn chain_points_are_the_applied_grid_hours_and_pages_cross_holes_blocks_and_gaps() {
    assert!(chain_point(104, 8) && chain_point(-8, 8) && chain_point(0, 3));
    assert!(!chain_point(105, 8) && !chain_point(-9, 8) && !chain_point(8, 0));
    assert!((-20..20).all(|hour| chain_point(hour, 1)));
    assert_eq!(provisional_strides(), [20, 40]);
    assert!(PageFold::new(1, SELECTED, &[], None).is_err());
    assert!(PageFold::new(1, SELECTED, &[0, 8], None).is_err());
    let fold = PageFold::new(1, SELECTED, &[8, 3, 8], None).unwrap();
    let strides: Vec<_> = fold
        .pages()
        .chains
        .iter()
        .map(|chain| chain.stride)
        .collect();
    assert_eq!(strides, [3, 8]);
    assert!(PageFold::new(1, SELECTED, &[4], Some(fold.pages().clone())).is_err());
    assert!(PageFold::new(1, &SELECTED[..2], &[3, 8], Some(fold.pages().clone())).is_err());

    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    assert_eq!(
        prepared.chain(1, 8),
        [
            (104, 120),
            (120, 128),
            (128, 136),
            (136, 144),
            (144, 160),
            (160, 168),
            (168, 176),
            (176, 184)
        ]
    );
    let thirds = prepared.chain(1, 3);
    for page in [(108, 114), (129, 132), (150, 162)] {
        assert!(thirds.contains(&page), "{page:?}");
    }
    assert_eq!(
        prepared.chain(2, 8),
        [(24, 32), (32, 40), (40, 48), (48, 56)]
    );
    // The hole's grid hour is a native step: no page ends there.
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 104, 104).0.unwrap();
    assert_eq!(same(&mut paged, &mut plain, 1, 112, 112).1, 0);
    for (from, to) in [(104, 120), (128, 136), (144, 160)] {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, from, from).0.unwrap();
        let (answer, pages) = same(&mut paged, &mut plain, 1, to, to);
        assert!(matches!(answer, Ok(Some(Current::Frame(_)))));
        assert_eq!((pages, paged.source_reads), (1, 0), "{from} to {to}");
    }
}

#[test]
fn pages_answer_every_hour_as_the_native_route_across_holes_gaps_keyframes_and_branches() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    assert!(prepared.refused.is_empty());
    for (branch, pages) in &prepared.pages {
        for chain in &pages.chains {
            assert!(chain.ranges > 0, "branch {branch} stride {}", chain.stride);
        }
    }
    let applied = compare_every_hour(&prepared, 1, true) + compare_every_hour(&prepared, 2, true);
    assert!(applied > 0, "pages were applied");
    // A playing walk on the coarsest grid reaches each target by one page.
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    let (reached, _) = same(&mut paged, &mut plain, 1, 160, 160);
    assert!(matches!(reached, Ok(Some(Current::Frame(_)))));
    for hour in [168, 176, 184] {
        let (_, pages) = same(&mut paged, &mut plain, 1, hour, hour);
        assert_eq!((pages, paged.source_reads), (1, 0), "hour {hour}");
    }
}

#[test]
fn a_refused_block_ends_its_chains_and_later_hours_answer_as_without_pages() {
    let clean = recording();
    let prepared = Prepared::new(&clean, damage(&clean, 1), Hour::MAX);
    let (hour, reason) = &prepared.refused[&1];
    assert_eq!(*hour, 131, "{reason}");
    // Each chain ends at its last chain point before the damaged block.
    assert_eq!(prepared.lasts(1), [(3, Some(129)), (8, Some(128))]);
    assert!(!prepared.refused.contains_key(&2));
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
    let (answer, pages) = same(&mut paged, &mut plain, 1, 131, 131);
    assert!(matches!(answer, Ok(Some(Current::Frame(_)))));
    assert_eq!(pages, 1);
    compare_every_hour(&prepared, 1, false);
    assert!(compare_every_hour(&prepared, 2, true) > 0);
}

#[test]
fn pages_stop_at_the_accepted_end_and_later_hours_stay_with_the_replay_reader() {
    let prepared = Prepared::new(&recording(), recording(), 175);
    // The block holding 160..190 is not wholly accepted: no page enters it.
    assert_eq!(prepared.lasts(1), [(3, Some(150)), (8, Some(144))]);
    compare_every_hour(&prepared, 1, true);
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    for hour in 176..=190 {
        assert_eq!(same(&mut paged, &mut plain, 1, hour, hour).0, Ok(None));
    }
}

fn fold_all(
    replay: &Replay,
    pages: impl Fn(usize) -> BlockPages,
) -> (BTreeMap<BranchId, BranchPages>, Fixture) {
    fold_all_at(replay, STRIDES, pages)
}

/// [`fold_all`] with chains of `strides`.
fn fold_all_at(
    replay: &Replay,
    strides: &[u32],
    pages: impl Fn(usize) -> BlockPages,
) -> (BTreeMap<BranchId, BranchPages>, Fixture) {
    let store = Fixture::new();
    let mut folds = BTreeMap::new();
    for block in 0..replay.index.len() {
        let branch = replay.index[block].branch;
        let fold = folds
            .entry(branch)
            .or_insert_with(|| PageFold::new(branch, SELECTED, strides, None).unwrap());
        assert_eq!(fold.fold(block, pages(block)).unwrap(), None);
    }
    let mut writer = store.writer();
    let mut chains = BTreeMap::new();
    for (branch, mut fold) in folds {
        fold.publish(&mut writer).unwrap();
        fold.save_states(&mut writer).unwrap();
        chains.insert(branch, fold.pages().clone());
    }
    writer.commit().unwrap();
    (chains, store)
}

fn produced(replay: &Replay, block: usize) -> BlockPages {
    produced_at(replay, block, STRIDES)
}

/// [`produced`] with chains of `strides`.
fn produced_at(replay: &Replay, block: usize, strides: &[u32]) -> BlockPages {
    BlockPages::produce(replay, block, SELECTED, strides, || Ok(())).unwrap()
}

fn extracted(replay: &Replay, block: usize) -> BlockPages {
    extracted_at(replay, block, STRIDES)
}

/// [`extracted`] with chains of `strides`, which the extract keeps.
fn extracted_at(replay: &Replay, block: usize, strides: &'static [u32]) -> BlockPages {
    let mut extract = PageExtract::new(SELECTED, strides);
    let mut tracker = FrameChangeTracker::new();
    replay
        .read_block_alone(
            &replay.index[block],
            SELECTED,
            |raw, frame| {
                let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                extract.observe(replay, frame, &changes)
            },
            || Ok(()),
        )
        .unwrap();
    *Box::new(extract)
        .finish()
        .unwrap()
        .downcast::<BlockPages>()
        .unwrap()
}

fn assert_native_chains(
    replay: &Replay,
    chains: &BTreeMap<BranchId, BranchPages>,
    fixture: &Fixture,
) {
    let reference = Replay::open(replay.bytes().to_vec()).unwrap();
    reference.cache_decoded_sections(false);
    let mut count = 0;
    for (&branch, pages) in chains {
        for chain in &pages.chains {
            let Some(first) = chain.first else {
                assert_eq!(chain.ranges, 0);
                continue;
            };
            let Current::Frame(seed) = native(&reference, branch, first) else {
                panic!("chain origin")
            };
            let mut frame = (*seed).clone();
            for range in 0..chain.ranges {
                let file = fixture
                    .store
                    .read(&fixture.key, &section(branch, chain.stride), range)
                    .unwrap()
                    .unwrap();
                let (header, step) = super::super::pages::read_page(file, SELECTED).unwrap();
                assert_eq!((frame.branch, frame.hour), (header.branch, header.from));
                frame = step.apply_owned(Some(frame)).unwrap();
                let expected = native(&reference, branch, header.to);
                assert_eq!(
                    Some(bytes(&frame)),
                    canonical(&expected),
                    "branch {branch} stride {} range {range}",
                    chain.stride
                );
                count += 1;
            }
            assert_eq!(chain.last, Some(frame.hour));
        }
    }
    assert!(count > 0, "committed page payloads checked natively");
}

#[test]
fn the_pre_pass_extract_writes_the_pages_of_the_block_read_alone() {
    let replay = recording();
    let (expected, from_blocks) = fold_all(&replay, |block| produced(&replay, block));
    let (chains, from_extracts) = fold_all(&replay, |block| extracted(&replay, block));
    assert_eq!(chains, expected);
    assert_eq!(from_extracts.payloads(), from_blocks.payloads());
    assert_native_chains(&replay, &chains, &from_extracts);
    assert_native_chains(&replay, &expected, &from_blocks);
    for index in &replay.index {
        let facts = crate::prepared::prepass::extract_alone(&replay, index, &FAMILY).unwrap();
        let pages = facts.downcast::<BlockPages>().unwrap();
        assert!(pages.refused().is_none());
        assert_eq!(pages.reached(), Some(index.last_hour));
    }
}

#[test]
fn the_pre_pass_extract_writes_the_pages_of_the_block_read_alone_at_the_production_strides() {
    let replay = recording();
    // The strides the family's own extract keeps.
    let strides: &'static [u32] = &PRODUCTION_STRIDES;
    let (expected, from_blocks) = fold_all_at(&replay, strides, |block| {
        produced_at(&replay, block, strides)
    });
    let (chains, from_extracts) = fold_all_at(&replay, strides, |block| {
        extracted_at(&replay, block, strides)
    });
    assert_eq!(chains, expected);
    assert_eq!(from_extracts.payloads(), from_blocks.payloads());
    assert_native_chains(&replay, &chains, &from_extracts);
    assert_native_chains(&replay, &expected, &from_blocks);
    let chain = |branch: BranchId, stride: u32| {
        expected[&branch]
            .chains
            .iter()
            .find(|chain| chain.stride == stride)
            .map(|chain| (chain.ranges, chain.first, chain.last))
    };
    assert!(
        chain(1, strides[0]).is_some_and(|(ranges, ..)| ranges > 0),
        "stride {} writes pages on this recording",
        strides[0]
    );
    assert_eq!(chain(1, strides[1]), Some((1, Some(120), Some(160))));
}

#[test]
fn a_restart_recovers_each_live_chains_last_state_and_writes_the_uninterrupted_pages() {
    let replay = recording();
    let (expected, whole) = fold_all(&replay, |block| produced(&replay, block));
    for cut in 1..replay.index.len() {
        let resumed = Fixture::new();
        let mut committed: BTreeMap<BranchId, BranchPages> = BTreeMap::new();
        for blocks in [0..cut, cut..replay.index.len()] {
            let mut folds = BTreeMap::new();
            for block in blocks {
                let branch = replay.index[block].branch;
                let fold = folds.entry(branch).or_insert_with(|| {
                    let saved = committed.get(&branch).map(|pages| {
                        serde_json::from_slice(&serde_json::to_vec(pages).unwrap()).unwrap()
                    });
                    let mut fold = PageFold::new(branch, SELECTED, STRIDES, saved).unwrap();
                    fold.resume(|section| resumed.store.read(&resumed.key, section, 0))
                        .unwrap();
                    fold
                });
                assert_eq!(fold.fold(block, produced(&replay, block)).unwrap(), None);
            }
            let mut writer = resumed.writer();
            for (branch, mut fold) in folds {
                fold.publish(&mut writer).unwrap();
                fold.save_states(&mut writer).unwrap();
                committed.insert(branch, fold.pages().clone());
            }
            writer.commit().unwrap();
        }
        assert_eq!(committed, expected, "cut before block {cut}");
        assert_native_chains(&replay, &committed, &resumed);
        assert_eq!(
            resumed.payloads(),
            whole.payloads(),
            "cut before block {cut}"
        );
    }
    let read = |section: &str| whole.store.read(&whole.key, section, 0);
    let mut moved = expected[&1].clone();
    moved.chains[1].last = moved.chains[1].last.map(|hour| hour - 8);
    let mut fold = PageFold::new(1, SELECTED, STRIDES, Some(moved)).unwrap();
    assert!(fold.resume(read).is_err());
    let mut fold = PageFold::new(1, SELECTED, STRIDES, Some(expected[&1].clone())).unwrap();
    assert!(fold.resume(|_| Ok(None)).is_err());
    let mut fold = PageFold::new(1, SELECTED, STRIDES, Some(expected[&1].clone())).unwrap();
    fold.resume(read).unwrap();
}

/// The reconstructed hour, from a reader without pages.
fn frame_at(prepared: &Prepared, branch: BranchId, hour: Hour) -> DecodedFrame {
    match prepared
        .reader(false)
        .advance(branch, hour, |_| {})
        .unwrap()
    {
        Some(Current::Frame(frame)) => (*frame).clone(),
        other => panic!("hour {hour}: {other:?}"),
    }
}

/// `page`'s header over another step.
fn rewrapped(page: &[u8], step: &[u8]) -> Vec<u8> {
    let mut out = page[..HEADER_BYTES].to_vec();
    compress(step, &mut out).unwrap();
    out
}

fn forged_campaign_step(clean: &Prepared) -> Vec<u8> {
    let (at, mut to) = (frame_at(clean, 1, 120), frame_at(clean, 1, 123));
    *to.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"another campaign".to_vec(),
        }]));
    let mut step = Vec::new();
    EndpointWriter::diff(SELECTED, &at, &to, &mut step).unwrap();
    step
}

fn continue_past_a_checkpoint(prepared: &Prepared) -> u64 {
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
    let (answer, pages) = same(&mut paged, &mut plain, 1, 140, 124);
    assert!(matches!(answer, Ok(Some(Current::Frame(_)))));
    // The continuation after the request, and its load policy, are intact.
    same(&mut paged, &mut plain, 1, 148, 146).0.unwrap();
    pages
}

#[test]
fn a_damaged_missing_mismatched_or_unappliable_page_leaves_the_native_continuation() {
    let clean = Prepared::new(&recording(), recording(), Hour::MAX);
    assert_eq!(
        continue_past_a_checkpoint(&clean),
        1,
        "page 120 to 123 is used"
    );
    let (at, to, later) = (
        frame_at(&clean, 1, 120),
        frame_at(&clean, 1, 123),
        frame_at(&clean, 1, 126),
    );
    let step = |tables: &[TableId], to: &DecodedFrame| {
        let mut step = Vec::new();
        EndpointWriter::diff(tables, &at, to, &mut step).unwrap();
        step
    };
    let without_countries: Vec<_> = SELECTED
        .iter()
        .copied()
        .filter(|&table| table != TableId::Countries)
        .collect();
    let with_states: Vec<_> = SELECTED.iter().copied().chain([TableId::States]).collect();
    enum Fault {
        Damaged,
        Missing,
        Step(Vec<u8>),
    }
    let faults = [
        ("damaged checksummed payload", Fault::Damaged),
        ("missing range", Fault::Missing),
        ("later endpoint", Fault::Step(step(SELECTED, &later))),
        (
            "omitted table",
            Fault::Step(step(without_countries.as_slice(), &to)),
        ),
        (
            "extra table",
            Fault::Step(step(with_states.as_slice(), &to)),
        ),
        ("unappliable", Fault::Step(forged_campaign_step(&clean))),
    ];
    for (name, fault) in &faults {
        let prepared =
            Prepared::rewritten(&recording(), recording(), Hour::MAX, &|header, mut page| {
                if (header.branch, header.stride, header.from, header.to) != (1, 3, 120, 123) {
                    return Some(page);
                }
                match fault {
                    Fault::Damaged => {
                        let last = page.len() - 1;
                        page[last] ^= 0x40;
                        Some(page)
                    }
                    Fault::Missing => None,
                    Fault::Step(step) => Some(rewrapped(&page, step)),
                }
            });
        assert_eq!(continue_past_a_checkpoint(&prepared), 0, "{name}");
    }
}

#[test]
fn a_page_refused_after_application_continues_natively_without_a_checkpoint() {
    let forged = forged_campaign_step(&Prepared::new(&recording(), recording(), Hour::MAX));
    let prepared = Prepared::rewritten(&recording(), recording(), Hour::MAX, &|header, page| {
        Some(
            if (header.branch, header.stride, header.from, header.to) == (1, 3, 120, 123) {
                rewrapped(&page, &forged)
            } else {
                page
            },
        )
    });
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
    // Checkpoint 100, from which both readers reached 120, is damaged since.
    let name = checkpoint_section(1);
    let mut checkpoint = Vec::new();
    prepared
        .fixture
        .store
        .read(&prepared.fixture.key, &name, 0)
        .unwrap()
        .unwrap()
        .read_to_end(&mut checkpoint)
        .unwrap();
    let last = checkpoint.len() - 1;
    checkpoint[last] ^= 0x40;
    let mut writer = prepared.fixture.writer();
    writer.write_at(&name, 0, checkpoint.as_slice()).unwrap();
    writer.commit().unwrap();
    let (answer, pages) = same(&mut paged, &mut plain, 1, 140, 124);
    assert!(matches!(answer, Ok(Some(Current::Frame(_)))));
    assert_eq!(
        (pages, paged.page_fallbacks, paged.checkpoint_reads),
        (0, 1, 0)
    );
}

#[test]
fn a_failing_native_tail_leaves_the_request_and_the_next_as_the_native_route_leaves_them() {
    let clean = recording();
    let prepared = Prepared::new(&clean, damage(&clean, 1), Hour::MAX);
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 126, 126).0.unwrap();
    // Page 126 to 129 applies; its native tail through 150 meets the damage.
    let (answer, pages) = same(&mut paged, &mut plain, 1, 150, 150);
    assert!(answer.is_err(), "{answer:?}");
    assert_eq!((pages, paged.page_fallbacks), (0, 1));
    assert_eq!(paged.continuation(), plain.continuation());
    let (answer, pages) = same(&mut paged, &mut plain, 1, 129, 128);
    assert!(matches!(answer, Ok(Some(Current::Frame(_)))));
    assert_eq!(pages, 0);
    assert_eq!(paged.continuation(), plain.continuation());
    assert_eq!(
        (
            paged.source_reads,
            paged.source_decoded_frames,
            paged.retained_source_frames
        ),
        (
            plain.source_reads,
            plain.source_decoded_frames,
            plain.retained_source_frames
        )
    );
}

#[test]
fn a_page_end_takes_its_frame_ordinal_from_the_source_and_another_block_is_refused() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    for (target, visit_from) in [(120, 120), (122, 121)] {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 104, 104).0.unwrap();
        assert_eq!(same(&mut paged, &mut plain, 1, target, visit_from).1, 1);
        let (cursor, page_end, ..) = paged.continuation();
        if target == visit_from {
            // A page's end holds its block alone.
            assert_eq!(
                (cursor.map(|cursor| cursor.block), page_end),
                (Some(0), true)
            );
        } else {
            assert_eq!((cursor, page_end), (plain.continuation().0, false));
        }
        // The next hour steps natively from there.
        assert_eq!(same(&mut paged, &mut plain, 1, target + 1, target + 1).1, 0);
        let (cursor, page_end, requested, _) = paged.continuation();
        let (expected, _, expected_requested, _) = plain.continuation();
        assert_eq!(
            (cursor, page_end, requested),
            (expected, false, expected_requested)
        );
        assert_eq!(
            cursor,
            Some(Position {
                block: 0,
                next_frame: (target + 1 - 100 - 2 + 1) as usize,
            })
        );
    }
    for block in [1u64, 3, 99] {
        let prepared =
            Prepared::rewritten(&recording(), recording(), Hour::MAX, &|header, mut page| {
                if (header.branch, header.stride, header.from, header.to) == (1, 8, 104, 120) {
                    page[28..36].copy_from_slice(&block.to_le_bytes());
                }
                Some(page)
            });
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 104, 104).0.unwrap();
        let (answer, pages) = same(&mut paged, &mut plain, 1, 120, 120);
        assert!(matches!(answer, Ok(Some(Current::Frame(_)))));
        assert_eq!((pages, paged.page_fallbacks), (0, 1), "block {block}");
    }
}

#[test]
fn a_clear_during_page_reads_leaves_the_request_to_the_native_route() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 160, 160).0.unwrap();
    let reads = Rc::new(Cell::new(0));
    let (seen, store) = (Rc::clone(&reads), prepared.store());
    reader::on_page_read(Some(Box::new(move || {
        seen.set(seen.get() + 1);
        store.clear().unwrap();
    })));
    let answer = paged
        .advance_paged(1, 168, 168, |_| panic!("a cleared request visits nothing"))
        .map_err(|error| error.to_string());
    reader::on_page_read(None);
    assert!(reads.get() > 0, "a page was read before the Clear");
    assert_eq!((answer, paged.applied_pages), (Ok(None), 0));
    let expected = plain
        .advance(1, 168, |_| panic!("a cleared request visits nothing"))
        .map_err(|error| error.to_string());
    assert_eq!(expected, Ok(None));
}

#[test]
fn damaged_gap_metadata_refuses_a_page_hit_as_it_refuses_the_native_route() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 160, 160).0.unwrap();
    // Gap metadata whose interval count differs from progress.
    let mut damaged = b"HOIGAP01".to_vec();
    damaged.extend_from_slice(&u64::MAX.to_le_bytes());
    let mut writer = prepared.fixture.writer();
    writer
        .write_at("map.gaps.1", 0, damaged.as_slice())
        .unwrap();
    writer.commit().unwrap();
    // The page 160 to 168 would reach the target alone.
    let (answer, pages) = same(&mut paged, &mut plain, 1, 168, 168);
    assert_eq!(
        (answer, pages),
        (Err("map gap page differs from progress".to_owned()), 0)
    );
    let before = paged.continuation();
    assert_eq!(
        paged
            .advance_pages(1, 168, 168, || true)
            .unwrap_err()
            .to_string(),
        "map gap page differs from progress",
    );
    assert_eq!((paged.applied_pages, paged.continuation()), (0, before));
}

thread_local! {
    /// Checked growths this thread admits before it denies one; zero denies none.
    static DENY_AFTER: Cell<u64> = const { Cell::new(0) };
    /// Whether a growth was denied since it was last cleared.
    static DENIED: Cell<bool> = const { Cell::new(false) };
    /// Whether only growth within a page route is counted.
    static PAGES_ONLY: Cell<bool> = const { Cell::new(true) };
    /// Whether every checked growth is denied.
    static DENY_ALL: Cell<bool> = const { Cell::new(false) };
}

fn denied() -> bool {
    if DENY_ALL.get() {
        return true;
    }
    if PAGES_ONLY.get() && !reader::in_page_work() {
        return false;
    }
    match DENY_AFTER.get() {
        0 => false,
        1 => {
            DENY_AFTER.set(0);
            DENIED.set(true);
            true
        }
        left => {
            DENY_AFTER.set(left - 1);
            false
        }
    }
}

fn denying<T>(run: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            engine_allocation::set_stop_hook(None);
            DENY_AFTER.set(0);
            DENY_ALL.set(false);
        }
    }
    engine_allocation::set_stop_hook(Some(denied));
    let _reset = Reset;
    run()
}

#[test]
fn page_allocation_failures_retry_natively_and_propagate_only_with_the_native_routes() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let mut denials = 0;
    for after in 1u64.. {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 160, 160).0.unwrap();
        DENY_AFTER.set(after);
        DENIED.set(false);
        let (answer, _) = denying(|| same(&mut paged, &mut plain, 1, 184, 178));
        assert!(
            matches!(answer, Ok(Some(Current::Frame(_)))),
            "denial {after}"
        );
        if !DENIED.get() {
            break;
        }
        denials += 1;
        same(&mut paged, &mut plain, 1, 190, 190).0.unwrap();
    }
    assert!(denials > 0, "page routes make checked growths");
    // With the native route denied too, its allocation failure propagates.
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 160, 160).0.unwrap();
    let kinds = [&mut paged, &mut plain].map(|held| {
        DENY_ALL.set(true);
        denying(|| {
            held.advance_paged(1, 184, 184, |_| {})
                .map(|_| ())
                .unwrap_err()
                .kind()
        })
    });
    assert_eq!(kinds, [io::ErrorKind::OutOfMemory; 2]);
}

#[test]
fn the_producer_returns_each_allocation_failure_as_its_error_never_as_a_refusal() {
    let replay = recording();
    let block = replay.index.len() - 1;
    let produce = || BlockPages::produce(&replay, block, SELECTED, STRIDES, || Ok(()));
    // Warm the decoded-section cache, so every run makes the same growths.
    assert!(produce().unwrap().refused().is_none());
    PAGES_ONLY.set(false);
    let mut after = 1;
    loop {
        DENY_AFTER.set(after);
        DENIED.set(false);
        let result = denying(produce);
        if !DENIED.get() {
            assert!(result.unwrap().refused().is_none());
            break;
        }
        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(io::ErrorKind::OutOfMemory),
            "growth {after}"
        );
        after += 1;
    }
    assert!(after > 1, "the producer makes checked growths");
}

#[test]
fn the_fold_returns_each_allocation_failure_as_its_error_never_as_a_refusal() {
    let replay = recording();
    // Branch 2's first block committed with its chains and states.
    let saved = Fixture::new();
    let mut fold = PageFold::new(2, SELECTED, STRIDES, None).unwrap();
    assert_eq!(fold.fold(3, produced(&replay, 3)).unwrap(), None);
    let mut writer = saved.writer();
    fold.publish(&mut writer).unwrap();
    fold.save_states(&mut writer).unwrap();
    writer.commit().unwrap();
    let committed = fold.pages().clone();
    let run = || -> io::Result<BranchPages> {
        let mut fold = PageFold::new(2, SELECTED, STRIDES, None)?;
        let first = BlockPages::produce(&replay, 3, SELECTED, STRIDES, || Ok(()))?;
        assert_eq!(fold.fold(3, first)?, None);
        let mut fold = PageFold::new(2, SELECTED, STRIDES, Some(committed.clone()))?;
        fold.resume(|section| saved.store.read(&saved.key, section, 0))?;
        let second = BlockPages::produce(&replay, 4, SELECTED, STRIDES, || Ok(()))?;
        assert_eq!(fold.fold(4, second)?, None);
        Ok(fold.pages().clone())
    };
    // Warm the decoded-section cache, so every run makes the same growths.
    let expected = run().unwrap();
    assert_eq!(
        expected
            .chains
            .iter()
            .map(|chain| (chain.stride, chain.ranges, chain.first, chain.last))
            .collect::<Vec<_>>(),
        [(3, 13, Some(21), Some(60)), (8, 4, Some(24), Some(56))],
    );
    PAGES_ONLY.set(false);
    let mut after = 1;
    loop {
        DENY_AFTER.set(after);
        DENIED.set(false);
        let result = denying(run);
        if !DENIED.get() {
            assert_eq!(result.unwrap(), expected);
            break;
        }
        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(io::ErrorKind::OutOfMemory),
            "growth {after}"
        );
        after += 1;
    }
    assert!(after > 1, "the fold makes checked growths");
}

#[test]
fn a_binding_denied_an_allocation_reads_without_pages_as_the_native_route() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    assert_eq!(prepared.pages.len(), 2, "two branches' chains bind");
    let reader = || {
        Reader::new(
            prepared.store(),
            prepared.fixture.key.clone(),
            prepared.map.clone(),
            Arc::clone(&prepared.replay),
            SELECTED,
        )
        .unwrap()
    };
    PAGES_ONLY.set(false);
    let mut denials = 0;
    // Each checked growth of binding the committed chains denied in turn.
    for after in 1u64.. {
        let unbound = reader();
        DENY_AFTER.set(after);
        DENIED.set(false);
        let mut paged = denying(|| unbound.with_committed_pages());
        let denied = DENIED.get();
        let mut plain = reader();
        same(&mut paged, &mut plain, 1, 160, 160).0.unwrap();
        let (answer, pages) = same(&mut paged, &mut plain, 1, 168, 168);
        assert!(
            matches!(answer, Ok(Some(Current::Frame(_)))),
            "denial {after}"
        );
        assert_eq!(pages, u64::from(!denied), "denial {after}");
        if !denied {
            break;
        }
        denials += 1;
    }
    assert!(denials > 0, "binding makes checked growths");
}

#[test]
fn pages_alone_reach_the_last_chain_point_before_a_target_and_the_native_route_goes_on() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    for (held, target, visit_from, reached) in [
        (120, 143, 143, Some(136)),
        (120, 140, 132, Some(128)),
        (128, 136, 136, Some(136)),
        (144, 168, 168, Some(168)),
        (104, 112, 112, None),
        (121, 140, 140, None),
    ] {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, held, held).0.unwrap();
        let before = paged.continuation();
        let ranges = reader::page_ranges_read();
        let label = format!("held {held}, target {target}, visits from {visit_from}");
        assert_eq!(
            paged.advance_pages(1, target, visit_from, || true).unwrap(),
            ControlFlow::Continue(reached),
            "{label}"
        );
        match reached {
            // The held state is the native route's at the chain point reached.
            Some(reached) => {
                assert!(paged.applied_pages > 0, "{label}");
                same(&mut paged, &mut plain, 1, reached, reached).0.unwrap();
            }
            None => assert_eq!(paged.continuation(), before, "{label}"),
        }
        if held == 121 {
            assert_eq!(reader::page_ranges_read(), ranges, "{label}");
        }
        // The next request goes on as the native route does.
        same(&mut paged, &mut plain, 1, target, visit_from)
            .0
            .unwrap();
    }
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 100, 100).0.unwrap();
    let ranges = reader::page_ranges_read();
    for hour in 101..=150 {
        assert_eq!(
            same(&mut paged, &mut plain, 1, hour, hour).1,
            0,
            "hour {hour}"
        );
    }
    assert_eq!(reader::page_ranges_read(), ranges);
}

#[test]
fn a_failing_page_alone_leaves_the_held_state_and_its_continuation() {
    let damaged = Prepared::rewritten(&recording(), recording(), Hour::MAX, &|header, mut page| {
        if (header.branch, header.stride, header.from, header.to) == (1, 8, 120, 128) {
            let last = page.len() - 1;
            page[last] ^= 0x40;
        }
        Some(page)
    });
    let (mut paged, mut plain) = (damaged.reader(true), damaged.reader(false));
    same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
    let before = paged.continuation();
    assert_eq!(
        paged.advance_pages(1, 143, 143, || true).unwrap(),
        ControlFlow::Continue(None)
    );
    assert_eq!((paged.page_fallbacks, paged.continuation()), (1, before));
    same(&mut paged, &mut plain, 1, 143, 143).0.unwrap();
    // Each checked growth of the pages alone denied in turn.
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let mut denials = 0;
    for after in 1u64.. {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
        let before = paged.continuation();
        DENY_AFTER.set(after);
        DENIED.set(false);
        let ControlFlow::Continue(reached) =
            denying(|| paged.advance_pages(1, 143, 143, || true)).unwrap()
        else {
            panic!("noncancelled request");
        };
        if !DENIED.get() {
            assert_eq!(reached, Some(136));
            break;
        }
        denials += 1;
        assert_eq!(
            (reached, paged.continuation()),
            (None, before),
            "denial {after}"
        );
        same(&mut paged, &mut plain, 1, 143, 143).0.unwrap();
    }
    assert!(denials > 0, "pages alone make checked growths");
}

#[test]
fn pages_apply_on_several_page_workers_as_native_reconstruction() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    for workers in [2, 4, 8] {
        let mut applied = 0;
        for branch in [1, 2] {
            let (first, last) = prepared.reader(true).range(branch).unwrap();
            for spacing in [3, 8, 16] {
                let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
                paged.set_page_workers(workers);
                for hour in (first..=last).filter(|hour| hour % spacing == 0) {
                    applied += same(&mut paged, &mut plain, branch, hour, hour).1;
                }
            }
        }
        assert!(applied > 0, "pages applied on {workers} workers");
    }
}

/// The range of `branch`'s chain of `stride` holding the page from `from`.
fn range_from(prepared: &Prepared, branch: BranchId, stride: u32, from: Hour) -> u64 {
    let chain = prepared.chain(branch, stride);
    chain.iter().position(|&(start, _)| start == from).unwrap() as u64
}

#[test]
fn the_page_after_a_reached_target_is_read_ahead_and_used_by_the_next_grid_request() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let next = range_from(&prepared, 1, 8, 128);
    assert_eq!(
        prepared.chain(1, 8)[next as usize - 1..next as usize + 3],
        [(120, 128), (128, 136), (136, 144), (144, 160)]
    );
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
    assert_eq!(paged.reading_ahead(), None, "a native answer reads no page");
    let used = reader::pages_read_ahead();
    // Pages that reach their target read their chain's next page ahead.
    assert_eq!(same(&mut paged, &mut plain, 1, 128, 128).1, 1);
    assert_eq!(paged.reading_ahead(), Some((1, 8, 128, next)));
    same(&mut paged, &mut plain, 1, 128, 128).0.unwrap();
    assert_eq!(paged.reading_ahead(), Some((1, 8, 128, next)));
    assert_eq!(reader::pages_read_ahead(), used);
    // The next grid request applies it and reads the page after it ahead.
    assert_eq!(same(&mut paged, &mut plain, 1, 136, 136).1, 1);
    assert_eq!(reader::pages_read_ahead(), used + 1);
    assert_eq!(paged.reading_ahead(), Some((1, 8, 136, next + 1)));
    // So do pages alone.
    assert_eq!(
        paged.advance_pages(1, 144, 144, || true).unwrap(),
        ControlFlow::Continue(Some(144))
    );
    assert_eq!(reader::pages_read_ahead(), used + 2);
    assert_eq!(paged.reading_ahead(), Some((1, 8, 144, next + 2)));
    same(&mut paged, &mut plain, 1, 144, 144).0.unwrap();
    // A native step drops it.
    assert_eq!(same(&mut paged, &mut plain, 1, 145, 145).1, 0);
    assert_eq!(paged.reading_ahead(), None);
    // So does a seek.
    same(&mut paged, &mut plain, 1, 160, 160).0.unwrap();
    assert_eq!(same(&mut paged, &mut plain, 1, 168, 168).1, 1);
    assert!(paged.reading_ahead().is_some());
    same(&mut paged, &mut plain, 1, 100, 100).0.unwrap();
    assert_eq!(paged.reading_ahead(), None);
    // A request that takes another chain from there reads its page in place.
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 104, 104).0.unwrap();
    assert_eq!(same(&mut paged, &mut plain, 1, 120, 120).1, 1);
    assert_eq!(paged.reading_ahead(), Some((1, 8, 120, next - 1)));
    let used = reader::pages_read_ahead();
    assert_eq!(same(&mut paged, &mut plain, 1, 123, 123).1, 1);
    assert_eq!(reader::pages_read_ahead(), used);
}

#[test]
fn pages_alone_continue_a_held_chain_point_however_far_and_name_where_a_native_walk_hands_over() {
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 1, 104, 104).0.unwrap();
    assert_eq!(
        paged.advance_pages(1, 168, 168, || true).unwrap(),
        ControlFlow::Continue(Some(168))
    );
    assert_eq!(paged.page_skip, None);
    same(&mut paged, &mut plain, 1, 168, 168).0.unwrap();
    // Held on another branch, no page applies.
    let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
    same(&mut paged, &mut plain, 2, 40, 40).0.unwrap();
    assert_eq!(
        paged.advance_pages(1, 168, 168, || true).unwrap(),
        ControlFlow::Continue(None)
    );
    assert_eq!(paged.page_skip, Some("no-held-state"));
    let reader = prepared.reader(true);
    assert_eq!(reader.page_entry(1, 121, 140), Some(128));
    assert_eq!(reader.page_entry(1, 128, 140), Some(128));
    assert_eq!(reader.page_entry(1, 133, 140), Some(135));
    assert_eq!(reader.page_entry(1, 137, 140), None);
    assert_eq!(reader.page_entry(2, 121, 140), None);
    assert_eq!(prepared.reader(false).page_entry(1, 121, 140), None);
}

#[test]
fn a_page_read_ahead_from_damaged_rewritten_or_cleared_bytes_is_never_applied() {
    let held_at_128 = |prepared: &Prepared| {
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
        assert_eq!(same(&mut paged, &mut plain, 1, 128, 128).1, 1);
        let (.., from, range) = paged.reading_ahead().unwrap();
        assert_eq!(from, 128);
        (paged, plain, range)
    };
    let used = reader::pages_read_ahead();
    let damaged = Prepared::rewritten(&recording(), recording(), Hour::MAX, &|header, mut page| {
        if (header.branch, header.stride, header.from, header.to) == (1, 8, 128, 136) {
            let last = page.len() - 1;
            page[last] ^= 0x40;
        }
        Some(page)
    });
    let (mut paged, mut plain, _) = held_at_128(&damaged);
    assert_eq!(same(&mut paged, &mut plain, 1, 136, 136).1, 0);
    assert_eq!(
        (paged.page_fallbacks, reader::pages_read_ahead()),
        (1, used)
    );
    let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
    let (mut paged, mut plain, range) = held_at_128(&prepared);
    let name = section(1, 8);
    let mut page = Vec::new();
    prepared
        .fixture
        .store
        .read(&prepared.fixture.key, &name, range)
        .unwrap()
        .unwrap()
        .read_to_end(&mut page)
        .unwrap();
    let last = page.len() - 1;
    page[last] ^= 0x40;
    let mut writer = prepared.fixture.writer();
    writer.write_at(&name, range, page.as_slice()).unwrap();
    writer.commit().unwrap();
    assert_eq!(same(&mut paged, &mut plain, 1, 136, 136).1, 0);
    assert_eq!(
        (paged.page_fallbacks, reader::pages_read_ahead()),
        (1, used)
    );
    // Cleared since: the request is left to the replay reader, as without one.
    let (mut paged, _plain, _) = held_at_128(&prepared);
    prepared.store().clear().unwrap();
    let answer = paged
        .advance_paged(1, 136, 136, |_| panic!("a cleared request visits nothing"))
        .map_err(|error| error.to_string());
    assert_eq!(answer, Ok(None));
    assert_eq!(
        (
            paged.reading_ahead(),
            paged.applied_pages,
            reader::pages_read_ahead()
        ),
        (None, 0, used)
    );
}

#[test]
fn map_capture_pages_keep_not_recorded_markers_model_witnesses_and_sample_facts() {
    use super::super::selected_tables;
    use replay_data::{
        rows::{DivisionCamouflage, DivisionModelInputRow},
        scope,
        scope::CaptureMode,
    };
    let selected = selected_tables();
    let observed = |hour| {
        let mut frame = scale_frame(1, hour, 1);
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
        frame.tables[TableId::DivisionModelInputs.index()] = if hour == 108 {
            TableView::Unavailable {
                reason: "model witness refused".into(),
            }
        } else {
            TableView::Complete(TableRows::DivisionModelInputs(vec![
                DivisionModelInputRow {
                    division: 1,
                    model_number: Some(hour % 3 - 1),
                    camouflage: Some(DivisionCamouflage {
                        category: (hour % 2 == 0).then_some(hour as u32),
                    }),
                    exercising: (hour != 104).then_some((hour % 2) as u8),
                },
            ]))
        }
        .into();
        frame.metrics.total_ns = hour as u64;
        frame.metrics.table_ns[TableId::DivisionModelInputs.index()] = u64::from(hour % 2 == 0);
        frame.metrics.pool_bytes = u64::MAX;
        frame.metrics.dropped_frames = 7;
        frame
    };
    let mut recorded = records(1, (100..=112).map(observed).collect());
    let Record::SessionStart(start) = &mut recorded[0] else {
        panic!("session start")
    };
    start.capture_mode = CaptureMode::Map;
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[recorded]).unwrap());
    assert_eq!(replay.capture_mode, CaptureMode::Map);
    let reference = Replay::open(replay.bytes().to_vec()).unwrap();
    reference.cache_decoded_sections(false);
    let fixture = Fixture::new();
    let mut builder =
        Builder::new(NonZeroU32::new(720).unwrap(), selected, Progress::default()).unwrap();
    builder.set_ranges(&replay);
    let mut writer = fixture.writer();
    let mut decoder = FrameDecoder::for_tables(selected);
    let mut fold = PageFold::new(1, selected, &[2, 4], None).unwrap();
    for (block, index) in replay.index.iter().enumerate() {
        for (offset, raw) in replay
            .read_block_selected(index, selected)
            .unwrap()
            .iter()
            .enumerate()
        {
            let decoded = decoder.advance(raw).unwrap();
            builder
                .accept(
                    &mut writer,
                    raw,
                    decoded,
                    Position {
                        block,
                        next_frame: offset + 1,
                    },
                )
                .unwrap();
        }
        let produced = BlockPages::produce(&replay, block, selected, &[2, 4], || Ok(())).unwrap();
        assert_eq!(fold.fold(block, produced).unwrap(), None);
    }
    fold.publish(&mut writer).unwrap();
    fold.save_states(&mut writer).unwrap();
    writer
        .write_at(
            &progress_section(1),
            0,
            serde_json::to_vec(fold.pages()).unwrap().as_slice(),
        )
        .unwrap();
    builder.save_gaps(&mut writer, &replay.gaps).unwrap();
    writer.commit().unwrap();
    let store = Store::local(
        fixture
            .directory
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
    .unwrap();
    let mut reader = Reader::new(
        store,
        fixture.key.clone(),
        builder.progress().clone(),
        Arc::clone(&replay),
        selected,
    )
    .unwrap()
    .with_committed_pages();
    for (hour, pages) in [(100, 0), (104, 1), (108, 1), (112, 1), (109, 0), (110, 0)] {
        let Current::Frame(actual) = reader
            .advance_paged(1, hour, hour, |_| panic!("no earlier visitors requested"))
            .unwrap()
            .unwrap()
        else {
            panic!("frame at {hour}")
        };
        let SeekResult::Frame(expected) = reference.seek_selected(1, hour, Some(selected)).unwrap()
        else {
            panic!("native frame at {hour}")
        };
        assert_eq!(bytes(&actual), bytes(&expected), "hour {hour}");
        assert_eq!(reader.applied_pages, pages, "hour {hour}");
        assert!(matches!(
            &*actual.tables[TableId::DivisionStatus.index()],
            TableView::NotRecorded {
                mode: CaptureMode::Map
            }
        ));
        assert_eq!(
            actual.tables[TableId::DivisionStatus.index()].sampled_at(),
            None
        );
        assert_eq!(
            &*actual.tables[TableId::DivisionModelInputs.index()],
            &*observed(hour).tables[TableId::DivisionModelInputs.index()],
        );
    }
}

#[test]
fn long_page_chains_keep_one_live_decoded_payload_and_continue_natively() {
    for pages in [32, 256] {
        let (first, end) = (104, 104 + 8 * pages);
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, frames(1, first..=end + 1, &[]))],
        )
        .unwrap();
        let prepared = Prepared::new(
            &replay,
            Replay::open(replay.bytes().to_vec()).unwrap(),
            Hour::MAX,
        );
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, first, first).0.unwrap();
        assert_eq!(same(&mut paged, &mut plain, 1, first + 8, first + 8).1, 1);
        assert!(
            paged.reading_ahead().is_some(),
            "the incoming read-ahead is also counted"
        );
        assert_eq!(
            paged.advance_pages(1, end, end, || true).unwrap(),
            ControlFlow::Continue(Some(end))
        );
        assert_eq!(paged.applied_pages, (pages - 1) as u64);
        assert_eq!(paged.reading_ahead(), None);
        assert_eq!(
            paged.page_payload_counts(),
            (0, 1),
            "chain of {pages} pages"
        );
        same(&mut paged, &mut plain, 1, end, end).0.unwrap();
        let (answer, applied) = same(&mut paged, &mut plain, 1, end + 1, end);
        answer.unwrap();
        assert_eq!(applied, 0);
        assert_eq!(
            paged.applied_frames, 1,
            "the native delta continues the paged decoder"
        );
    }
}

#[test]
fn cancelling_after_one_header_reads_no_later_range_or_native_fallback_and_publishes_nothing() {
    for target in [128, 143] {
        let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
        assert_eq!(paged.reading_ahead(), None);
        let before = paged.continuation();
        let ranges_before = reader::page_ranges_read();
        let wanted = Rc::new(Cell::new(true));
        let headers = Rc::new(Cell::new(0));
        reader::on_page_header_read(Some(Box::new({
            let (wanted, headers) = (Rc::clone(&wanted), Rc::clone(&headers));
            move || {
                headers.set(headers.get() + 1);
                wanted.set(false);
            }
        })));
        let result = paged.advance_pages(1, target, target, || wanted.get());
        reader::on_page_header_read(None);
        assert_eq!(result.unwrap(), ControlFlow::Break(()));
        assert_eq!(headers.get(), 1);
        assert_eq!(
            reader::page_ranges_read() - ranges_before,
            1,
            "no range after the first search header was opened"
        );
        assert_eq!(
            (
                paged.applied_pages,
                paged.page_bytes,
                paged.applied_frames,
                paged.page_fallbacks,
                paged.source_reads
            ),
            (0, 0, 0, 0, 0)
        );
        assert_eq!(
            paged.continuation(),
            before,
            "cursor, requested and retained source are not published"
        );
        assert_eq!(paged.held_hour(), Some(120));
        assert_eq!(paged.reading_ahead(), None);
        assert_eq!(paged.page_payload_counts(), (0, 0));
        let mut passed = Vec::new();
        let actual = paged
            .advance(1, 129, |frame| passed.push((frame.hour, bytes(frame))))
            .unwrap();
        let mut expected = Vec::new();
        let native = plain
            .advance(1, 129, |frame| expected.push((frame.hour, bytes(frame))))
            .unwrap();
        assert_eq!(
            actual.as_ref().and_then(canonical),
            native.as_ref().and_then(canonical)
        );
        assert_eq!(passed, expected);
        assert_eq!(
            passed.iter().map(|(hour, _)| *hour).collect::<Vec<_>>(),
            (121..129).collect::<Vec<_>>()
        );
        assert_eq!(
            paged.applied_frames, 9,
            "the next native request replays every unpublished hour"
        );
        same(&mut paged, &mut plain, 1, 130, 129).0.unwrap();
    }
}

#[test]
fn cancelling_after_one_page_reads_no_later_range_or_native_fallback_and_publishes_nothing() {
    for target in [128, 143] {
        let prepared = Prepared::new(&recording(), recording(), Hour::MAX);
        let (mut paged, mut plain) = (prepared.reader(true), prepared.reader(false));
        same(&mut paged, &mut plain, 1, 120, 120).0.unwrap();
        assert_eq!(paged.reading_ahead(), None);
        let before = paged.continuation();
        let wanted = Rc::new(Cell::new(true));
        let reads = Rc::new(Cell::new(0));
        let last_range = Rc::new(Cell::new(0));
        reader::on_page_read(Some(Box::new({
            let (wanted, reads, last_range) = (
                Rc::clone(&wanted),
                Rc::clone(&reads),
                Rc::clone(&last_range),
            );
            move || {
                reads.set(reads.get() + 1);
                last_range.set(reader::page_ranges_read());
                wanted.set(false);
            }
        })));
        let result = paged.advance_pages(1, target, target, || wanted.get());
        reader::on_page_read(None);
        assert_eq!(result.unwrap(), ControlFlow::Break(()));
        assert_eq!(
            reads.get(),
            1,
            "cancellation follows an actual decoded and applied page"
        );
        assert_eq!(
            reader::page_ranges_read(),
            last_range.get(),
            "no later page range was opened"
        );
        assert_eq!(
            (
                paged.applied_pages,
                paged.applied_frames,
                paged.page_fallbacks,
                paged.source_reads
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(
            paged.continuation(),
            before,
            "cursor, requested and retained source are not published"
        );
        assert_eq!(paged.held_hour(), Some(120));
        assert_eq!(paged.reading_ahead(), None);
        assert_eq!(paged.page_payload_counts(), (0, 1));
        let mut passed = Vec::new();
        let actual = paged
            .advance(1, 129, |frame| passed.push((frame.hour, bytes(frame))))
            .unwrap();
        let mut expected = Vec::new();
        let native = plain
            .advance(1, 129, |frame| expected.push((frame.hour, bytes(frame))))
            .unwrap();
        assert_eq!(
            actual.as_ref().and_then(canonical),
            native.as_ref().and_then(canonical)
        );
        assert_eq!(passed, expected);
        assert_eq!(
            passed.iter().map(|(hour, _)| *hour).collect::<Vec<_>>(),
            (121..129).collect::<Vec<_>>()
        );
        assert_eq!(
            paged.applied_frames, 9,
            "the next native request replays every unpublished hour"
        );
        same(&mut paged, &mut plain, 1, 130, 129).0.unwrap();
    }
}
