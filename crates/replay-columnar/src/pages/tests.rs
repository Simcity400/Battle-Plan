use super::*;
use crate::unit::{PartEntry, restore::{Chunk, RestoreEntry}};

fn frame(offset: u64, streams: Vec<u32>) -> FrameEntry {
    FrameEntry { streams, offset, stored: 17, raw: 40, digest: [offset as u8; 16], chained: offset % 2 == 0, small: offset % 3 == 0 }
}

fn summary(run: u32, table: u32, first: u32) -> Extent {
    Extent { kind: Kind::Summary, run, stream: table, first, hours: UNIT_ORDINALS, offset: 900 + u64::from(table), stored: 30, raw: 30, digest: [table as u8; 16] }
}

fn entry(run: u32, table: u32, block: u32) -> Entry {
    let log = table == 3;
    let restore = RestoreEntry { whole: table % 2 == 0, streams: vec![(0, vec![Chunk { first: 0, frame: frame(500, vec![]), prefix: (table % 2 == 1).then(|| frame(400, vec![])) }])], log };
    let ordinal = |rows: u64| if log { (1 << 32) + 1_000 + rows } else { 0 };
    Entry {
        unit: TableEntry {
            parts: vec![
                PartEntry { first: 0, frames: vec![frame(100 + u64::from(table), vec![4, 5]), frame(200, vec![6])], ordinal: ordinal(0), log, ..PartEntry::default() },
                PartEntry { first: 300, frames: vec![frame(250, vec![4])], ordinal: ordinal(70), log, ..PartEntry::default() },
            ],
            status: vec![1, 0, 1, 1, 1],
            first: (block == 0).then(|| restore.clone()),
            restore: (table % 3 == 0).then_some(restore),
            packed: table == 2,
            log,
            count: ordinal(0),
        },
        summary: (table != 1).then(|| summary(run, table, block * UNIT_ORDINALS)),
    }
}

fn head(hours: u16) -> BlockHead {
    let strings = if hours % 2 == 0 { vec![StringChunk { first: 77, frame: frame(12, vec![]) }, StringChunk { first: 4_077, frame: frame(14, vec![]) }] } else { Vec::new() };
    BlockHead { hours, clock: frame(10, vec![]), strings, first_string: 77, chain: [3; 32] }
}

#[test]
fn a_block_page_round_trips_and_chains() {
    let block = Block { run: 2, block: 5, head: head(720), tables: (0..4).map(|table| entry(2, table, 5)).collect() };
    let bytes = block.write().unwrap();
    let extent = Extent { kind: Kind::BlockPage, run: 2, stream: 0, first: 5 * UNIT_ORDINALS, hours: 720, offset: 12, stored: 1, raw: bytes.len() as u64, digest: [0; 16] };
    let read = Block::read(&bytes, &extent, 4).unwrap();
    assert_eq!(read, block);
    let chained = Block { head: BlockHead { chain: Block::chain_after(&[9; 32], &block.body().unwrap()), ..block.head.clone() }, ..block.clone() };
    assert!(chained.chains_from(&[9; 32]).unwrap());
    assert!(!chained.chains_from(&[8; 32]).unwrap());
    assert!(Block::read(&bytes, &extent, 5).is_err());
    let elsewhere = Extent { first: 6 * UNIT_ORDINALS, ..extent };
    assert!(Block::read(&bytes, &elsewhere, 4).is_err());
    assert_eq!(read.unit_page().tables[3], block.tables[3].unit);
}

#[test]
fn a_span_row_and_directory_round_trip() {
    let row = SpanRow { blocks: (0..SPAN_BLOCKS).map(|block| entry(1, 3, block)).collect(), summaries: vec![(1, summary(1, 3, 0)), (2, summary(1, 3, 0))] };
    let mut bytes = Vec::new();
    row.write(&mut bytes).unwrap();
    assert_eq!(SpanRow::read(&bytes, 1, 3, SPAN_BLOCKS).unwrap(), row);
    assert!(SpanRow::read(&bytes, 1, 4, SPAN_BLOCKS).is_err());
    let directory = SpanDirectory {
        run: 1,
        first_block: 16,
        blocks: (0..SPAN_BLOCKS).map(|_| head(720)).collect(),
        chain: [4; 32],
        rows: (0..3).map(|table| RowPlace { table, offset: u64::from(table) * 50, length: 50, digest: [table as u8; 16] }).collect(),
    };
    let bytes = directory.write();
    let extent = Extent { kind: Kind::SpanDirectory, run: 1, stream: 0, first: 16 * UNIT_ORDINALS, hours: 16 * UNIT_ORDINALS, offset: 12, stored: 1, raw: 1, digest: [0; 16] };
    assert_eq!(SpanDirectory::read(&bytes, &extent, 3).unwrap(), directory);
    assert!(SpanDirectory::read(&bytes, &extent, 4).is_err());
}

#[test]
fn a_run_directory_round_trips_and_refuses_what_does_not_cover_it() {
    let page = |block: u32, hours: u32| Extent { kind: Kind::BlockPage, run: 0, stream: 0, first: block * UNIT_ORDINALS, hours, offset: 40, stored: 9, raw: 9, digest: [1; 16] };
    let span = SpanPages {
        directory: Extent { kind: Kind::SpanDirectory, run: 0, stream: 0, first: 0, hours: 16 * UNIT_ORDINALS, offset: 50, stored: 9, raw: 9, digest: [2; 16] },
        index: Extent { kind: Kind::SpanIndex, run: 0, stream: 0, first: 0, hours: 16 * UNIT_ORDINALS, offset: 60, stored: 9, raw: 9, digest: [3; 16] },
    };
    let directory = RunDirectory {
        branch: 7,
        epoch: 1,
        first_hour: -40,
        hours: 17 * UNIT_ORDINALS + 5,
        chain: [5; 32],
        spans: vec![span],
        open: vec![page(16, UNIT_ORDINALS), page(17, 5)],
        tail: vec![(3, summary(0, 3, 16 * UNIT_ORDINALS))],
    };
    let bytes = directory.write(0).unwrap();
    let extent = Extent { kind: Kind::RunDirectory, run: 0, stream: 0, first: 0, hours: 0, offset: 70, stored: 9, raw: bytes.len() as u64, digest: [0; 16] };
    assert_eq!(RunDirectory::read(&bytes, &extent, 0).unwrap(), directory);
    let short = RunDirectory { open: vec![page(16, UNIT_ORDINALS)], ..directory };
    assert!(short.write(0).is_err());
    assert!(RunDirectory::read(&bytes, &Extent { run: 1, ..extent }, 1).is_err());
    let late = |first_hour: i32| RunDirectory { branch: 7, epoch: 1, first_hour, hours: 5, chain: [5; 32], spans: Vec::new(), open: vec![page(0, 5)], tail: Vec::new() };
    let fits = late(i32::MAX - UNIT_ORDINALS as i32);
    assert_eq!(RunDirectory::read(&fits.write(0).unwrap(), &extent, 0).unwrap(), fits, "a run whose blocks end on the last hour that fits is read");
    for first_hour in [i32::MAX - 10, i32::MIN] {
        let past = late(first_hour);
        assert!(past.write(0).is_err(), "{first_hour}: a run past the hour range is never written");
        let error = RunDirectory::read(&past.encode().unwrap(), &extent, 0).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{first_hour}: a run past the hour range is read as damaged: {error}");
    }
}
