//! Modelled layout bench (ignored test): a recording's tables written again
//! as ramped and as modelled blocks with the current writer, checked to
//! decode identically, with each one's write time, its read time per block
//! (every table, the fastest of five interleaved passes) and its bytes per
//! frame, all and by table, with each table's keys and masks columns.
//!
//! `MODEL_CENSUS=<recording> cargo test --release -p replay --lib
//! model_census -- --ignored --nocapture`
use crate::{
    container::{Replay, Section, grouped},
    rows::TableId,
    spool::Record,
};
use std::collections::BTreeMap;

fn written(replay: &Replay, selected: &[TableId], layout: grouped::Layout) -> Replay {
    let mut segments: Vec<Vec<Record>> = Vec::new();
    let (mut session, mut branch) = (None, None);
    for block in &replay.index {
        let start = replay
            .branches
            .iter()
            .find(|start| start.id == block.branch)
            .unwrap();
        if session != Some(start.session) {
            let started = replay
                .sessions
                .iter()
                .find(|candidate| candidate.id == start.session)
                .unwrap();
            segments.push(vec![Record::SessionStart(started.clone())]);
            session = Some(start.session);
            branch = None;
        }
        let records = segments.last_mut().unwrap();
        if branch != Some(block.branch) {
            records.push(Record::BranchStart(start.clone()));
            branch = Some(block.branch);
        }
        for frame in replay.read_block_selected(block, selected).unwrap() {
            records.push(Record::Frame(frame));
        }
    }
    grouped::WRITTEN_LAYOUT.set(layout);
    let started = std::time::Instant::now();
    let out = Replay::from_records(replay.campaign_id.clone(), &segments).unwrap();
    grouped::WRITTEN_LAYOUT.set(grouped::Layout::Advanced);
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
fn model_census() {
    let path = std::env::var("MODEL_CENSUS").expect("MODEL_CENSUS recording");
    let replay = Replay::open(std::fs::read(path).unwrap()).unwrap();
    let selected: Vec<TableId> = replay
        .block_sections(&replay.index[0])
        .unwrap()
        .unwrap()
        .into_iter()
        .filter_map(|section| match section.section {
            Section::Keyframe(table) => Some(table),
            _ => None,
        })
        .collect();
    let ramped = written(&replay, &selected, grouped::Layout::Ramped);
    let modelled = written(&replay, &selected, grouped::Layout::Modelled);
    let mut frames = 0_u64;
    let mut sizes = Vec::new();
    for (block, (r, m)) in ramped.index.iter().zip(&modelled.index).enumerate() {
        let r = ramped.read_block_selected(r, &selected).unwrap();
        let m = modelled.read_block_selected(m, &selected).unwrap();
        assert_eq!(r, m, "block {block}");
        frames += r.len() as u64;
        sizes.push(r.len());
    }
    assert_eq!(ramped.index.len(), modelled.index.len());
    println!("every hour decodes identically");
    // Each block's read time, the fastest of five interleaved passes.
    let blocks = ramped.index.len();
    let mut fastest = vec![[f64::MAX; 2]; blocks];
    for _ in 0..5 {
        for (block, times) in fastest.iter_mut().enumerate() {
            for (at, layout) in [&ramped, &modelled].into_iter().enumerate() {
                let started = std::time::Instant::now();
                layout
                    .read_block_selected(&layout.index[block], &selected)
                    .unwrap();
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
        "read per block: ramped {:.1} ms modelled {:.1} ms (all {blocks} blocks); \
         ramped {:.1} ms modelled {:.1} ms ({} blocks of 720 frames); fastest of 5, interleaved",
        mean(0, &every),
        mean(1, &every),
        mean(0, &full),
        mean(1, &full),
        full.len()
    );
    let (r, m) = (table_bytes(&ramped), table_bytes(&modelled));
    let per_frame = |bytes: u64| bytes as f64 / frames as f64;
    let total = |bytes: &BTreeMap<TableId, (u64, u64)>, at: usize| {
        bytes
            .values()
            .map(|pair| if at == 0 { pair.0 } else { pair.1 })
            .sum::<u64>()
    };
    let file = |replay: &Replay| replay.bytes().len() as u64;
    println!(
        "frames={frames} file ramped={:.1} modelled={:.1} B/frame; tables ramped={:.1} \
         modelled={:.1}; keys and masks ramped={:.1} modelled={:.1}",
        per_frame(file(&ramped)),
        per_frame(file(&modelled)),
        per_frame(total(&r, 0)),
        per_frame(total(&m, 0)),
        per_frame(total(&r, 1)),
        per_frame(total(&m, 1))
    );
    let mut rows: Vec<_> = r
        .iter()
        .map(|(table, ramped)| (*table, *ramped, m.get(table).copied().unwrap_or_default()))
        .collect();
    rows.sort_by_key(|(_, ramped, modelled)| modelled.0 as i64 - ramped.0 as i64);
    for (table, ramped, modelled) in rows.iter().take(40).chain(rows.iter().rev().take(10)) {
        println!(
            "{table:?} ramped={:.1} modelled={:.1} change={:+.1} keys+masks ramped={:.1} modelled={:.1}",
            per_frame(ramped.0),
            per_frame(modelled.0),
            per_frame(modelled.0) - per_frame(ramped.0),
            per_frame(ramped.1),
            per_frame(modelled.1)
        );
    }
}
