//! Synthetic held anchor reads and their byte bounds.
use super::*;
use crate::rows::{CountryRow, LexicalTokenRow};

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn held_anchor_reads_only_its_directory_and_selected_keyframe() {
    let campaign = b"synthetic-anchor-read";
    let resident = test_recording::build(campaign, &[1, 721, 1441], |_, tables| {
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=512)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("synthetic-country-{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        // Unrelated content makes whole-block anchor reads observably larger.
        tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
            (1..=512_u32)
                .map(|token| LexicalTokenRow {
                    token,
                    literal: (0..128_u32)
                        .map(|i| {
                            token
                                .wrapping_mul(2654435761)
                                .rotate_left(i % 32)
                                .wrapping_add(i) as u8
                        })
                        .collect(),
                })
                .collect(),
        ));
        Metrics::default()
    });
    assert_eq!(resident.index.len(), 3);
    let table = TableId::Countries;
    let chain = keyframe_chain(&resident, 2, table).unwrap();
    assert!(chain.len() > 1, "fixture must resolve an anchored keyframe");
    // Measure the whole-block alternative directly from recorded extents.
    // Unrelated sections in every link must make that alternative exceed
    // the selected anchor read, without repeating its directory sizing rule.
    let whole_bytes: u64 = chain[1..]
        .iter()
        .map(|&(position, _)| resident.index[position].length)
        .sum();
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "replay-anchor-read-{}.battleplan",
        std::process::id()
    )));
    std::fs::write(&fixture.0, resident.bytes()).unwrap();
    let held = Replay::open_path(&fixture.0).unwrap();
    let index = &resident.index[2];
    let block = resident.block_bytes(index).unwrap();
    let directory = Directory::parse(&block, index).unwrap();
    let from = From {
        index,
        position: Some(2),
        layout: directory.layout,
    };
    let expected = seed_table(&resident, &block, &directory, &from, table)
        .unwrap()
        .unwrap()
        .1;
    let (actual, read_bytes) = super::super::file_backed::measure_held_reads(|| {
        seed_table(&held, &block, &directory, &from, table)
            .unwrap()
            .unwrap()
            .1
    });
    assert_eq!(actual.rows, expected.rows);
    assert!(
        read_bytes < whole_bytes,
        "anchors requested {read_bytes} bytes; whole-block reads would request {whole_bytes}"
    );
}

#[test]
fn selected_keyframe_matches_first_frame_with_bounded_held_reads() {
    let campaign = b"synthetic-selected-keyframe";
    let resident = test_recording::build(campaign, &[1, 2, 721, 722], |hour, tables| {
        tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=512)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("synthetic-country-{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        // Large unrelated keyframes and later entries make whole-block reads
        // fail the bound even after paying for the selected table's anchor.
        tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
            (1..=256_u32)
                .map(|token| {
                    let mut state = (hour as u32).wrapping_mul(2654435761) ^ token;
                    LexicalTokenRow {
                        token,
                        literal: (0..512)
                            .map(|_| {
                                state ^= state << 13;
                                state ^= state >> 17;
                                state ^= state << 5;
                                state as u8
                            })
                            .collect(),
                    }
                })
                .collect(),
        ));
        let mut table_ns = [0; TABLE_COUNT];
        table_ns[TableId::Countries.index()] = 17;
        table_ns[TableId::Clock.index()] = 23;
        Metrics {
            total_ns: hour as u64 * 10,
            table_ns,
            pool_bytes: hour as u64 * 100,
            dropped_frames: hour as u64 / 2,
        }
    });
    assert_eq!(resident.index.len(), 2);
    assert!(
        keyframe_chain(&resident, 1, TableId::Countries)
            .unwrap()
            .len()
            > 1
    );
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "replay-selected-keyframe-{}.battleplan",
        std::process::id()
    )));
    std::fs::write(&fixture.0, resident.bytes()).unwrap();
    let held = Replay::open_path(&fixture.0).unwrap();
    let selected = [
        TableId::Countries,
        TableId::Clock,
        TableId::CountryCommandPower,
    ];
    for index in &resident.index {
        let expected = resident.read_block_selected(index, &selected).unwrap();
        assert_eq!(expected.len(), 2);
        assert_ne!(
            expected[0], expected[1],
            "second-frame mutation must fail equality"
        );
        assert!(matches!(
            expected[0].tables[TableId::CountryCommandPower.index()],
            TableUpdate::Unavailable { .. }
        ));
        assert_eq!(
            resident.read_keyframe_selected(index, &selected).unwrap(),
            expected[0]
        );
        let (actual, requested) = super::super::file_backed::measure_held_reads(|| {
            held.read_keyframe_selected(index, &selected).unwrap()
        });
        assert_eq!(actual, expected[0]);
        assert!(
            requested < index.length,
            "selected keyframe requested {requested} bytes from a {} byte block",
            index.length
        );
    }
    let index = &resident.index[1];
    let block = resident.block_bytes(index).unwrap();
    let directory = Directory::parse(&block, index).unwrap();
    let expected = resident.read_keyframe_selected(index, &selected).unwrap();
    for section in [
        keyframe_section(TableId::LexicalTokens),
        later_section(TableId::Clock),
    ] {
        let (offset, length, _) = directory.sections[section].unwrap();
        let mut bytes = resident.bytes().to_vec();
        bytes[index.offset as usize + offset + length - 1] ^= 1;
        let damaged = Replay::open(bytes).unwrap();
        assert_eq!(
            damaged.read_keyframe_selected(index, &selected).unwrap(),
            expected
        );
        assert!(damaged.read_block_selected(index, &selected).is_err());
    }
    let (offset, length, _) = directory.sections[keyframe_section(TableId::Clock)].unwrap();
    let mut bytes = resident.bytes().to_vec();
    bytes[index.offset as usize + offset + length - 1] ^= 1;
    let damaged = Replay::open(bytes).unwrap();
    assert!(damaged.read_keyframe_selected(index, &selected).is_err());
}
