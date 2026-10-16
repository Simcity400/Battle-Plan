//! Layout bench (ignored test): a recording's tables written again
//! as H4G9 (advanced) and H4GC (chosen) blocks, checked to
//! decode identically, with each one's write time, its read time per block
//! (every table, the fastest of five interleaved passes) and its bytes per
//! frame, all and by table, with each table's keys and masks columns and
//! how many later sections chose each row coding. Re-encoding holds one
//! block's frames at a time; comparisons read one block from each recording.
//!
//! `LAYOUT_CENSUS=<recording> cargo test --release -p replay --lib
//! layout_census -- --ignored --nocapture`
use crate::{
    container::{Replay, Section, grouped},
    rows::TableId,
};
use std::collections::BTreeMap;

fn written(replay: &Replay, layout: grouped::Layout) -> Replay {
    grouped::WRITTEN_LAYOUT.set(layout);
    let started = std::time::Instant::now();
    let out = replay.reencoded(|_, _, _| Ok(()));
    grouped::WRITTEN_LAYOUT.set(grouped::Layout::Chosen);
    let out = out.unwrap();
    println!(
        "{layout:?} written in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    out
}

/// By table: its keyframe and later sections' bytes, and its later
/// sections' keys and masks columns, each compressed alone.
fn table_bytes(replay: &Replay) -> BTreeMap<TableId, (u64, u64)> {
    let mut bytes: BTreeMap<TableId, (u64, u64)> = BTreeMap::new();
    for block in &replay.index {
        for sizes in replay.block_columns(block).unwrap().unwrap() {
            let (Section::Keyframe(table) | Section::Later(table)) = sizes.section else {
                continue;
            };
            let entry = bytes.entry(table).or_default();
            entry.0 += sizes.compressed;
            if let Section::Later(_) = sizes.section {
                entry.1 += sizes.columns[1].alone + sizes.columns[2].alone;
            }
        }
    }
    bytes
}

#[test]
#[ignore]
fn layout_census() {
    let path = std::env::var("LAYOUT_CENSUS").expect("LAYOUT_CENSUS recording");
    let replay = Replay::open(std::fs::read(path).unwrap()).unwrap();
    let advanced = written(&replay, grouped::Layout::Advanced);
    let chosen = written(&replay, grouped::Layout::Chosen);
    drop(replay);
    let mut frames = 0_u64;
    let mut sizes = Vec::new();
    assert_eq!(advanced.index.len(), chosen.index.len());
    for (block, (r, m)) in advanced.index.iter().zip(&chosen.index).enumerate() {
        assert_eq!(
            (r.branch, r.first_hour, r.last_hour),
            (m.branch, m.first_hour, m.last_hour)
        );
        let r = advanced.read_block(r).unwrap();
        let m = chosen.read_block(m).unwrap();
        assert_eq!(r, m, "block {block}");
        frames += r.len() as u64;
        sizes.push(r.len());
    }
    println!("every hour decodes identically");
    // Each block's read time, the fastest of five interleaved passes.
    let blocks = advanced.index.len();
    let mut fastest = vec![[f64::MAX; 2]; blocks];
    for _ in 0..5 {
        for (block, times) in fastest.iter_mut().enumerate() {
            for (at, layout) in [&advanced, &chosen].into_iter().enumerate() {
                let started = std::time::Instant::now();
                layout.read_block(&layout.index[block]).unwrap();
                times[at] = times[at].min(started.elapsed().as_secs_f64());
            }
        }
    }
    let full: Vec<usize> = (0..blocks).filter(|block| sizes[*block] == 720).collect();
    let mean = |at: usize, blocks: &[usize]| {
        blocks.iter().map(|block| fastest[*block][at]).sum::<f64>() * 1_000.0
            / blocks.len().max(1) as f64
    };
    let every: Vec<usize> = (0..blocks).collect();
    println!(
        "read per block: H4G9 {:.1} ms H4GC {:.1} ms (all {blocks} blocks); \
         H4G9 {:.1} ms H4GC {:.1} ms ({} blocks of 720 frames); fastest of 5, interleaved",
        mean(0, &every),
        mean(1, &every),
        mean(0, &full),
        mean(1, &full),
        full.len()
    );
    let (r, m) = (table_bytes(&advanced), table_bytes(&chosen));
    let codings = row_codings(&chosen);
    let per_frame = |bytes: u64| bytes as f64 / frames as f64;
    let total = |bytes: &BTreeMap<TableId, (u64, u64)>, at: usize| {
        bytes
            .values()
            .map(|pair| if at == 0 { pair.0 } else { pair.1 })
            .sum::<u64>()
    };
    let file = |replay: &Replay| replay.bytes().len() as u64;
    println!(
        "frames={frames} file H4G9={:.1} H4GC={:.1} B/frame; tables H4G9={:.1} \
         H4GC={:.1}; keys and masks H4G9={:.1} H4GC={:.1}",
        per_frame(file(&advanced)),
        per_frame(file(&chosen)),
        per_frame(total(&r, 0)),
        per_frame(total(&m, 0)),
        per_frame(total(&r, 1)),
        per_frame(total(&m, 1))
    );
    let mut rows: Vec<_> = r
        .iter()
        .map(|(table, advanced)| (*table, *advanced, m.get(table).copied().unwrap_or_default()))
        .collect();
    // TableId order prints every table, including those with no later section.
    for (table, advanced, chosen) in &rows {
        let [modelled, listed] = codings.get(table).copied().unwrap_or_default();
        println!(
            "{table:?} H4G9={:.1} H4GC={:.1} change={:+.1} keys+masks H4G9={:.1} H4GC={:.1}; H4GC blocks modelled={modelled} listed={listed}",
            per_frame(advanced.0),
            per_frame(chosen.0),
            per_frame(chosen.0) - per_frame(advanced.0),
            per_frame(advanced.1),
            per_frame(chosen.1)
        );
    }
    rows.sort_by_key(|(_, advanced, chosen)| chosen.0 as i64 - advanced.0 as i64);
    println!("biggest decreases, then increases (B/frame):");
    for (table, advanced, chosen) in rows.iter().take(40).chain(rows.iter().rev().take(10)) {
        println!(
            "{table:?} change={:+.1}",
            per_frame(chosen.0) - per_frame(advanced.0)
        );
    }
}

/// Per table, later sections whose coding byte names modelled or listed rows.
fn row_codings(replay: &Replay) -> BTreeMap<TableId, [u64; 2]> {
    let mut counts: BTreeMap<TableId, [u64; 2]> = BTreeMap::new();
    for block in &replay.index {
        let bytes = replay.block_bytes(block).unwrap();
        let directory = grouped::Directory::parse(&bytes, block).unwrap();
        assert_eq!(directory.layout, grouped::Layout::Chosen);
        for table in TableId::ALL {
            let Some(raw) = directory
                .raw(&bytes, grouped::later_section(table))
                .unwrap()
            else {
                continue;
            };
            let mut input = grouped::ColumnReader::open(&raw, table.field_count()).unwrap();
            let modelled = input.modelled_rows(table, directory.layout).unwrap();
            counts.entry(table).or_default()[usize::from(!modelled)] += 1;
        }
    }
    counts
}
