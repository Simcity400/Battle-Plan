use super::*;

const TABLES: usize = 3;

fn extent(kind: Kind, run: u32, stream: u32, first: u32, hours: u32) -> Extent {
    Extent {
        kind,
        run,
        stream,
        first,
        hours,
        offset: 100 + u64::from(stream),
        stored: 10,
        raw: 24,
        digest: [stream as u8; 16],
    }
}

fn bits(ones: &[u32]) -> Bits {
    let mut words = vec![0_u64; TABLES.div_ceil(64)];
    for &bit in ones {
        words[(bit / 64) as usize] |= 1 << (bit % 64);
    }
    Bits::from_words(words).unwrap()
}

fn page(first: u32, hours: u32, restart: bool) -> IndexPage {
    IndexPage {
        run: 1,
        first,
        hours,
        first_hour: -5 + first as i32,
        last_hour: -5 + (first + hours - 1) as i32,
        records: extent(Kind::Records, 1, 0, first, hours),
        tables: vec![
            extent(Kind::Block, 1, 0, first, hours),
            extent(Kind::Block, 1, 2, first, hours),
        ],
        chained: bits(if first == 0 { &[] } else { &[2] }),
        restart: restart.then(|| RestartDirectory {
            after: first + hours - 1,
            hour: 7,
            ordinal: 2,
            chunk: extent(Kind::DictionaryChunk, 1, 0, first + hours - 1, 0),
            ids: (3, 9),
            tables: (0..TABLES as u32)
                .map(|table| RestartTable {
                    table,
                    extent: extent(Kind::RestartTable, 1, table, first + hours - 1, 0),
                    base: table != 1,
                    chunks: if table == 1 { vec![0, 2] } else { Vec::new() },
                })
                .collect(),
        }),
        sectioned: bits(&[1]),
        read: bits(&[0, 1]),
        status_changed: bits(&[2]),
        chain: [7; 32],
    }
}

fn reads(page: &IndexPage) -> io::Result<IndexPage> {
    let envelope = extent(Kind::IndexPage, 1, 0, page.first, page.hours);
    IndexPage::read(&page.write()?, &envelope, TABLES)
}

#[test]
fn a_page_reads_back_what_was_written_and_belongs_to_its_envelope() {
    for restart in [false, true] {
        let written = page(UNIT_ORDINALS, UNIT_ORDINALS, restart);
        let bytes = written.write().unwrap();
        let envelope = extent(Kind::IndexPage, 1, 0, UNIT_ORDINALS, UNIT_ORDINALS);
        assert_eq!(IndexPage::read(&bytes, &envelope, TABLES).unwrap(), written);
        let elsewhere = extent(Kind::IndexPage, 1, 0, 0, UNIT_ORDINALS);
        assert!(IndexPage::read(&bytes, &elsewhere, TABLES).is_err());
        assert!(IndexPage::read(&bytes, &envelope, TABLES + 64).is_err());
        assert_eq!(written.table(2), Some(&written.tables[1]));
        assert_eq!(written.table(1), None);
    }
}

#[test]
fn pages_refuse_blocks_and_restarts_off_their_block() {
    let mut off_grid = page(0, UNIT_ORDINALS, false);
    off_grid.tables[1].first = UNIT_ORDINALS - 1;
    assert!(reads(&off_grid).is_err());
    let mut unordered = page(0, UNIT_ORDINALS, false);
    unordered.tables.swap(0, 1);
    assert!(reads(&unordered).is_err());
    let mut short_records = page(0, UNIT_ORDINALS, false);
    short_records.records.hours -= 1;
    assert!(reads(&short_records).is_err());
    let mut past_schema = page(0, UNIT_ORDINALS, false);
    past_schema.tables[1].stream = TABLES as u32;
    assert!(reads(&past_schema).is_err());
    // A chained table holds a block, and a run's first page chains none.
    let mut unheld = page(UNIT_ORDINALS, UNIT_ORDINALS, false);
    unheld.chained = bits(&[1]);
    assert!(reads(&unheld).is_err());
    let mut first = page(0, UNIT_ORDINALS, false);
    first.chained = bits(&[2]);
    assert!(reads(&first).is_err());
    let mut missing = page(0, UNIT_ORDINALS, true);
    missing.restart.as_mut().unwrap().tables.pop();
    assert!(reads(&missing).is_err());
    let mut early = page(0, UNIT_ORDINALS, true);
    early.restart.as_mut().unwrap().after -= 1;
    assert!(reads(&early).is_err());
    // A table's chunks are earlier restarts', ascending.
    let mut later_chunk = page(0, UNIT_ORDINALS, true);
    later_chunk.restart.as_mut().unwrap().tables[1].chunks = vec![0, 3];
    assert!(reads(&later_chunk).is_err());
    let mut unordered_chunks = page(0, UNIT_ORDINALS, true);
    unordered_chunks.restart.as_mut().unwrap().tables[1].chunks = vec![2, 0];
    assert!(reads(&unordered_chunks).is_err());
    // A restart stored whole holds no change from the one before.
    let mut keyframe = page(0, UNIT_ORDINALS, true);
    keyframe.restart.as_mut().unwrap().ordinal = KEYFRAME_RESTARTS;
    assert!(reads(&keyframe).is_err());
}

fn directory(pages: Vec<Extent>, ordinals: u32) -> RunDirectory {
    RunDirectory {
        branch: 9,
        epoch: epoch::LOAD,
        first_hour: 100,
        ordinals,
        chain: [5; 32],
        pages,
    }
}

#[test]
fn a_run_directory_reads_back_and_its_pages_cover_the_run_with_one_short_tail() {
    let pages = vec![
        extent(Kind::IndexPage, 2, 0, 0, UNIT_ORDINALS),
        extent(Kind::IndexPage, 2, 0, UNIT_ORDINALS, 5),
    ];
    let written = directory(pages.clone(), UNIT_ORDINALS + 5);
    let envelope = extent(Kind::RunDirectory, 2, 0, 0, 0);
    let bytes = written.write(2).unwrap();
    assert_eq!(RunDirectory::read(&bytes, &envelope, 2).unwrap(), written);
    assert!(RunDirectory::read(&bytes, &envelope, 3).is_err());
    // A short page before the last, a missing first page, and pages short of the ordinals.
    let short_first = vec![
        extent(Kind::IndexPage, 2, 0, 0, 5),
        extent(Kind::IndexPage, 2, 0, 5, UNIT_ORDINALS),
    ];
    assert!(directory(short_first, UNIT_ORDINALS + 5).write(2).is_err());
    assert!(
        directory(pages[1..].to_vec(), UNIT_ORDINALS + 5)
            .write(2)
            .is_err()
    );
    assert!(directory(pages, UNIT_ORDINALS + 6).write(2).is_err());
}

fn root() -> Root {
    Root {
        identity: Identity {
            schema: [1; 32],
            campaign: b"campaign".to_vec(),
            content: b"content".to_vec(),
            capture_mode: 2,
        },
        revision: 4,
        runs: vec![
            extent(Kind::RunDirectory, 0, 0, 0, 0),
            extent(Kind::RunDirectory, 1, 0, 0, 0),
        ],
        controls: vec![extent(Kind::Controls, 0, 0, 0, 0); 2],
        facts: vec![extent(Kind::Facts, 0, 0, 0, 0)],
    }
}

#[test]
fn a_root_lists_its_run_directories_in_order_and_its_controls() {
    let written = root();
    assert_eq!(Root::read(&written.write().unwrap()).unwrap(), written);
    let mut swapped = root();
    swapped.runs.swap(0, 1);
    assert!(swapped.write().is_err());
    let mut misplaced = root();
    misplaced.controls[1].kind = Kind::RunDirectory;
    assert!(misplaced.write().is_err());
}

#[test]
fn an_extent_is_read_only_inside_the_extents_and_with_its_digest() {
    let mut file = MAGIC.to_vec();
    file.extend_from_slice(&VERSION.to_le_bytes());
    let stored = b"stored bytes".to_vec();
    let offset = file.len() as u64;
    file.extend_from_slice(&stored);
    let mut envelope = extent(Kind::Block, 0, 1, 0, 1);
    envelope.offset = offset;
    envelope.stored = stored.len() as u64;
    envelope.digest = digest(&stored);
    file.extend_from_slice(&footer(0, 0, &[0; 16]));
    assert_eq!(envelope.stored_in(&file).unwrap(), stored.as_slice());
    let mut tampered = file.clone();
    tampered[offset as usize] ^= 1;
    assert!(envelope.stored_in(&tampered).is_err());
    let mut past = envelope;
    past.stored += 1;
    assert!(past.stored_in(&file).is_err());
    let mut header = envelope;
    header.offset = 4;
    assert!(header.stored_in(&file).is_err());
}

/// A file with roots `roots` appended in turn, each followed by its footer.
fn revisions(roots: &[&[u8]]) -> Vec<u8> {
    let mut file = MAGIC.to_vec();
    file.extend_from_slice(&VERSION.to_le_bytes());
    for root in roots {
        let offset = file.len() as u64;
        file.extend_from_slice(root);
        file.extend_from_slice(&footer(offset, root.len() as u64, &digest(root)));
    }
    file
}

#[test]
fn readers_take_the_last_footer_whose_root_verifies() {
    let file = revisions(&[b"first root", b"second root"]);
    let (root, end) = last_root(&file).unwrap();
    assert_eq!(end, file.len());
    assert_eq!(root.stored_in(&file[..end]).unwrap(), b"second root");
    // A torn tail after the last footer is ignored.
    let mut torn = file.clone();
    torn.extend_from_slice(b"half an extent and H4COLE");
    assert_eq!(last_root(&torn).unwrap(), (root, end));
    // A damaged last root falls back to the revision before it.
    let mut damaged = file.clone();
    let second = damaged.len() - FOOTER_BYTES - 1;
    damaged[second] ^= 1;
    let (earlier, earlier_end) = last_root(&damaged).unwrap();
    assert_eq!(
        earlier.stored_in(&damaged[..earlier_end]).unwrap(),
        b"first root"
    );
    // A footer whose root does not sit right before it commits nothing.
    let mut detached = file.clone();
    detached.extend_from_slice(b"padding");
    detached.extend_from_slice(&footer(HEADER_BYTES as u64, 10, &digest(b"first root")));
    assert_eq!(last_root(&detached).unwrap(), (root, end));
    let mut older = file;
    older[8..12].copy_from_slice(&2_u32.to_le_bytes());
    assert!(last_root(&older).is_err());
    assert!(last_root(&revisions(&[])).is_err());
}

#[test]
fn recovering_a_footer_hashes_at_most_twice_the_file() {
    // Every footer of the tail claims a root reaching back to the header.
    let mut file = revisions(&[b"first root"]);
    for _ in 0..64 {
        file.extend_from_slice(&[0; 1000]);
        let stored = (file.len() - HEADER_BYTES) as u64;
        file.extend_from_slice(&footer(HEADER_BYTES as u64, stored, &[0; 16]));
    }
    assert!(last_root(&file).is_err());
}

#[test]
fn the_chain_follows_every_block_extent_of_a_run() {
    let start = [0; 32];
    let (records, tables) = (
        extent(Kind::Records, 0, 0, 0, 1),
        vec![extent(Kind::Block, 0, 1, 0, 1), extent(Kind::Block, 0, 2, 0, 1)],
    );
    let block = chain_block(&start, &records, &tables);
    assert_eq!(block, chain_block(&start, &records, &tables));
    assert_ne!(block, chain_block(&block, &records, &tables));
    assert_ne!(block, chain_block(&start, &records, &tables[..1]));
    let mut changed = tables.clone();
    changed[1].digest[0] ^= 1;
    assert_ne!(block, chain_block(&start, &records, &changed));
    let mut moved = tables;
    moved[1].stream = 3;
    assert_ne!(block, chain_block(&start, &records, &moved));
}
