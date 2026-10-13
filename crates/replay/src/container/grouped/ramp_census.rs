//! Ramped layout bench (ignored test): a recording's tables written again as
//! keyed and as ramped blocks with the current writer, checked to decode
//! identically, with each one's write time, its read time (every block of
//! every table, the fastest of five interleaved passes) and its bytes per
//! hour, all and by table.
//!
//! `RAMP_CENSUS=<recording> cargo test --release -p replay --lib
//! ramp_census -- --ignored --nocapture`
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
    println!(
        "{layout:?} written in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    out
}

fn table_bytes(replay: &Replay) -> BTreeMap<TableId, u64> {
    let mut bytes = BTreeMap::new();
    for block in &replay.index {
        for section in replay.block_sections(block).unwrap().unwrap() {
            if let Section::Keyframe(table) | Section::Later(table) = section.section {
                *bytes.entry(table).or_default() += section.compressed;
            }
        }
    }
    bytes
}

#[test]
#[ignore]
fn ramp_census() {
    let path = std::env::var("RAMP_CENSUS").expect("RAMP_CENSUS recording");
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
    let keyed = written(&replay, &selected, grouped::Layout::Keyed);
    let ramped = written(&replay, &selected, grouped::Layout::Ramped);
    let mut hours = 0_u64;
    for (block, (k, r)) in keyed.index.iter().zip(&ramped.index).enumerate() {
        let k = keyed.read_block_selected(k, &selected).unwrap();
        let r = ramped.read_block_selected(r, &selected).unwrap();
        assert_eq!(k.len(), r.len(), "block {block}");
        for (k, r) in k.iter().zip(&r) {
            assert_eq!(k.tables, r.tables, "block {block} hour {}", k.hour);
        }
        hours += k.len() as u64;
    }
    println!("every hour decodes identically");
    // Interleaved, the fastest of five each.
    let mut fastest = [f64::MAX; 2];
    for _ in 0..5 {
        for (at, layout) in [&keyed, &ramped].into_iter().enumerate() {
            let started = std::time::Instant::now();
            for block in &layout.index {
                layout.read_block_selected(block, &selected).unwrap();
            }
            fastest[at] = fastest[at].min(started.elapsed().as_secs_f64());
        }
    }
    println!(
        "read keyed {:.2} s ramped {:.2} s (fastest of 5, interleaved)",
        fastest[0], fastest[1]
    );
    let (k, r) = (table_bytes(&keyed), table_bytes(&ramped));
    let per_hour = |bytes: u64| bytes as f64 / hours as f64;
    println!(
        "hours={hours} tables keyed={:.1} ramped={:.1} B/h",
        per_hour(k.values().sum()),
        per_hour(r.values().sum())
    );
    let mut rows: Vec<_> = k
        .iter()
        .map(|(table, bytes)| (*table, *bytes, r[table]))
        .collect();
    rows.sort_by_key(|(_, keyed, ramped)| *ramped as i64 - *keyed as i64);
    for (table, keyed, ramped) in rows.iter().take(25).chain(rows.iter().rev().take(10)) {
        println!(
            "{table:?} keyed={:.1} ramped={:.1} change={:+.1}",
            per_hour(*keyed),
            per_hour(*ramped),
            per_hour(*ramped) - per_hour(*keyed)
        );
    }
}
