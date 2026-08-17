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

/// Lane MV: the differential a builder rewrite has to pass.
///
/// A faster builder is only a faster builder if the frame it produces is the
/// same frame. This walks EVERY hour of the recording, builds the typed frame
/// the shipped way, and prints a SHA-256 per hour over the `Debug` rendering of
/// the views a rewrite can reach plus the frame's coverage warnings. `Debug` on
/// these views is a total, ordered rendering of their contents (every container
/// on the path is a `BTreeMap`/`Vec`, and `Leaf` distinguishes its three
/// states), so two runs that agree digest-for-digest built the same views, in
/// the same order, with the same warnings.
///
/// It is run BEFORE a rewrite and AFTER it, and the two files are compared. It
/// asserts no budget and is `#[ignore]`d, for the same reason as the profile
/// above: the real captures are far too large to commit.
///
/// kills: any change to a typed builder that alters a value, an ordering, a
/// `Leaf` state, or a coverage warning on a real recording — the post-rewrite
/// digest for that hour stops matching the pre-rewrite one.
#[test]
#[ignore = "measurement: needs MN_FIXTURE pointing at a real index/cache pair"]
fn digest_the_typed_frame_every_hour() {
    use sha2::{Digest, Sha256};

    let dir = std::env::var("MN_FIXTURE").expect("MN_FIXTURE names a directory");
    let index_bytes = std::fs::read(format!("{dir}/hour_index.h4ix")).expect("read index");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).expect("parse index");
    let source_len = index.source_len();
    let source_sha = index.source_sha256().to_string();
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let file = std::fs::File::open(format!("{dir}/checkpoints.h4cp")).expect("open cache");
    let cache =
        snapshot::CertifiedReplayCache::open_file(&index_bytes, file, source_len, &source_sha)
            .expect("admit cache");

    println!(
        "{{\"artifact\":\"{}\",\"hours\":{}}}",
        dir.replace('\\', "/"),
        hours.len()
    );
    for hour in &hours {
        let (world, _) = cache.reconstruct_value(*hour).expect("reconstruct the hour");
        let body = snapshot::Snapshot::from_world_value(world).expect("canonical body");
        let raw = RawFrame {
            hour_index: *hour,
            date: body.date.clone(),
            save_version: body.save_version.clone(),
            schema_version: body.schema_version.clone(),
            rest: body.rest,
        };
        let _ = frame_profile::take();
        let frame = WorldFrame::profile_from_raw(raw, None);
        // Diplomacy is rendered FIELD BY FIELD, and its pair matrix as an
        // explicit dense enumeration, so that the digest is a statement about
        // the pairs a consumer can observe rather than about the container
        // they happen to be stored in. A rewrite that changes the storage and
        // keeps every observable pair identical must keep this digest
        // identical.
        let dip = frame.diplomacy();
        let mut pair_text = String::new();
        let mut at_war_from_matrix: Vec<(&str, &str)> = Vec::new();
        for ((a, b), pair) in dip.pairs() {
            pair_text.push_str(&format!("{a}|{b}={pair:?}\n"));
            if matches!(pair.at_war, replay_core::world::Leaf::Measured(true)) {
                at_war_from_matrix.push((a, b));
            }
            // The enumeration and the single-pair lookup are two different
            // routes to the same answer, and a consumer uses the second one.
            //
            // kills: an existence rule in the single-pair route that differs
            // from the enumeration's -- e.g. one that answers "this hour says
            // nothing" for a pair only a complete witness covers.
            assert_eq!(
                dip.pair(a, b).as_ref(),
                Some(&pair),
                "hour {hour}: pair({a}, {b}) disagrees with the enumeration"
            );
        }
        // The war overview does NOT walk the matrix -- it asks
        // `at_war_pairs()`. The digest above cannot see that query, so the two
        // are required to agree, on every hour of a real recording.
        //
        // kills: an `at_war_pairs()` that answers from the wrong witness
        // universe, drops a source-measured war the complete witness does not
        // cover, or keeps one it does.
        assert_eq!(
            dip.at_war_pairs(),
            at_war_from_matrix,
            "hour {hour}: at_war_pairs() disagrees with the pair matrix"
        );
        let mut parts: Vec<(&str, String)> = vec![
            ("dip.pairs", pair_text),
            ("dip.status", format!("{:?}", dip.status)),
            ("dip.countries", format!("{:?}", dip.countries)),
            ("dip.war_relations", format!("{:?}", dip.war_relations)),
            ("dip.wars", format!("{:?}", dip.wars)),
            ("dip.faction_members", format!("{:?}", dip.faction_members)),
            ("dip.relation_flags", format!("{:?}", dip.relation_flags)),
            (
                "dip.rest",
                format!(
                    "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
                    dip.operation_instances,
                    dip.operation_token_memberships,
                    dip.intel_levels,
                    dip.air_disclosure,
                    dip.intel_network_graphs,
                    dip.subject_autonomy_current,
                    dip.agency_upgrade_controller_refresh,
                    dip.cipher_controller_refresh,
                    dip.intelligence_controller_companion,
                    dip.domain53_snapshot_payload_sha256,
                ),
            ),
            ("division_units", format!("{:?}", frame.division_units())),
            ("buildings", format!("{:?}", frame.buildings_by_state())),
            ("trade", format!("{:?}", frame.trade_by_country())),
            ("snow_mud", format!("{:?}", frame.snow_mud())),
            ("scripted_gui", format!("{:?}", frame.scripted_gui())),
            ("warnings", format!("{:?}", frame.coverage_warnings())),
        ];
        parts.sort_by(|a, b| a.0.cmp(b.0));
        let mut whole = Sha256::new();
        for (name, rendered) in &parts {
            let mut per = Sha256::new();
            per.update(name.as_bytes());
            per.update([0u8]);
            per.update(rendered.as_bytes());
            let digest = per.finalize();
            whole.update(digest);
        }
        println!(
            "{{\"hour\":{hour},\"digest\":\"{:x}\",\"warnings\":{}}}",
            whole.finalize(),
            frame.coverage_warnings().len()
        );
    }
}
