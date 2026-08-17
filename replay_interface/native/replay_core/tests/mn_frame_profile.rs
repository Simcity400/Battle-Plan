//! Lane MN batch 2: price the TYPED frame, per builder, on a real recording.
//!
//! WHY. Batch 1 removed the JSON round trip from the draw path and measured
//! what was left AT THE READER'S DOOR: 264 ms of a 13.9 MB t53 hour, all of it
//! building the world as a `serde_json::Value` tree. The frame the viewer
//! actually draws is built ABOVE that door, by
//! `WorldFrame::from_raw_with_design_catalog`, which runs its whole typed-view
//! battery for every hour — and nothing had ever priced it (Codex design
//! review, batch 2, finding 4, BLOCKING). The owner's bar is that an hour cost
//! O(changed paths). Which builders to make incremental first is a measurement,
//! and this is it.
//!
//! It is NOT a gate: it asserts nothing about a budget, it is `#[ignore]`d, it
//! only runs under the `mn_frame_profile` feature, and it reads an index/cache
//! pair the caller points it at, because the real captures are far too large to
//! commit. Run it as:
//!
//! ```text
//! MN_FIXTURE=<dir with hour_index.h4ix + checkpoints.h4cp> \
//!   cargo test -p replay_core --features mn_frame_profile \
//!   --test mn_frame_profile -- --ignored --nocapture
//! ```
#![cfg(feature = "mn_frame_profile")]

use std::collections::BTreeMap;
use std::time::Instant;

use replay_core::world::{frame_profile, WorldFrame};
use replay_core::RawFrame;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p / 100.0) * ((sorted.len() - 1) as f64);
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
    }
}

#[test]
#[ignore = "measurement: needs MN_FIXTURE pointing at a real index/cache pair"]
fn price_the_typed_frame_per_builder() {
    let dir = std::env::var("MN_FIXTURE").expect("MN_FIXTURE names a directory");
    let samples: usize = std::env::var("MN_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10);

    let index_bytes = std::fs::read(format!("{dir}/hour_index.h4ix")).expect("read index");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).expect("parse index");
    let source_len = index.source_len();
    let source_sha = index.source_sha256().to_string();
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let file = std::fs::File::open(format!("{dir}/checkpoints.h4cp")).expect("open cache");
    let cache = snapshot::CertifiedReplayCache::open_file(
        &index_bytes,
        file,
        source_len,
        &source_sha,
    )
    .expect("admit cache");

    // The last hours of the recording, where the world is largest, one forward
    // delta apiece — the shape ordinary playback has.
    let start = hours.len().saturating_sub(samples + 1);
    let mut world_ms: Vec<f64> = Vec::new();
    let mut frame_ms: Vec<f64> = Vec::new();
    let mut per_builder: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    let mut domains = 0usize;

    for position in (start + 1)..hours.len() {
        cache
            .reconstruct_value(hours[position - 1])
            .expect("prime the cursor");
        let t0 = Instant::now();
        let (world, _) = cache
            .reconstruct_value(hours[position])
            .expect("reconstruct the hour");
        let body = snapshot::Snapshot::from_world_value(world).expect("canonical body");
        world_ms.push(t0.elapsed().as_secs_f64() * 1e3);
        domains = body.rest.len();

        let raw = RawFrame {
            hour_index: hours[position],
            date: body.date.clone(),
            save_version: body.save_version.clone(),
            schema_version: body.schema_version.clone(),
            rest: body.rest,
        };
        let _ = frame_profile::take();
        let t1 = Instant::now();
        let frame = WorldFrame::profile_from_raw(raw, None);
        frame_ms.push(t1.elapsed().as_secs_f64() * 1e3);
        std::hint::black_box(&frame);
        for (name, span) in frame_profile::take() {
            per_builder.entry(name).or_default().push(span);
        }
    }

    let mut report = |label: &str, mut samples: Vec<f64>| {
        samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
        println!(
            "{{\"metric\":\"{label}\",\"n\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"max_ms\":{:.3}}}",
            samples.len(),
            percentile(&samples, 50.0),
            percentile(&samples, 95.0),
            samples.last().copied().unwrap_or(f64::NAN),
        );
    };
    println!("{{\"artifact\":\"{}\",\"hours\":{},\"domains\":{domains}}}", dir.replace('\\', "/"), hours.len());
    report("reader_world_tree", world_ms.clone());
    report("typed_frame_total", frame_ms.clone());

    // Builders, dearest first: this is the ordering the incremental work
    // follows, and the only reason this test exists.
    let mut ranked: Vec<(&'static str, f64, f64)> = per_builder
        .into_iter()
        .map(|(name, mut spans)| {
            spans.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
            let p50 = percentile(&spans, 50.0);
            let p95 = percentile(&spans, 95.0);
            (name, p50, p95)
        })
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).expect("no NaN timings"));
    let total: f64 = ranked.iter().map(|(_, p50, _)| *p50).sum();
    for (name, p50, p95) in &ranked {
        println!(
            "{{\"builder\":\"{name}\",\"p50_ms\":{p50:.3},\"p95_ms\":{p95:.3},\"share_pct\":{:.1}}}",
            if total > 0.0 { 100.0 * p50 / total } else { 0.0 }
        );
    }
    println!("{{\"builders\":{},\"builder_p50_sum_ms\":{total:.3}}}", ranked.len());
}
