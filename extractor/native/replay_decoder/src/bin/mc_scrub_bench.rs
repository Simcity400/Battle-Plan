//! Lane MC: measure what a timeline SCRUB costs on a real recording's
//! checkpoint cache, at whatever checkpoint interval the artifact was built
//! with, using THE SHIPPED READER.
//!
//! The owner's UX ruling is "a smooth timeline you can scrub like a video", and
//! the size lanes want the checkpoint interval moved from 168 to 744 game
//! hours. Those two meet here: a seek costs one checkpoint decode plus every
//! delta between that checkpoint and the wanted hour, so the interval is
//! exactly the depth of the deepest seek. This instrument prices that depth on
//! real hours instead of on a model of them.
//!
//! WHY AN INSTRUMENT AND NOT A TEST. A committed test asserts a budget on a
//! committed fixture; this prices the real captures, whose artifacts are far
//! too large to commit. It reads an index/cache pair a caller points it at and
//! writes nothing.
//!
//! Modes:
//!   depth  <dir>            per-depth cost: seek to the hour at delta distance
//!                           0,1,2,4,8,... from its checkpoint. Isolates the
//!                           fixed cost (checkpoint decode + serialize + parity
//!                           digest) from the per-delta cost.
//!   random <dir> <n> <seed> n pseudo-random seeks, cold and cursored, p50/p95.
//!   noise  <dir> <n>        the measurement noise floor: the same seek, to the
//!                           same hour, n times.
//!
//! WHY IT LIVES ON THE ACQUISITION SIDE. It names the two files it is pointed
//! at, and the replay side's `production_sources_have_no_descendant_pathname_read`
//! audit scans every module under that crate's `src/` -- a measurement bin is
//! not a reason to teach a production capability gate two new classified read
//! sites. It sits beside the other checkpoint-cache instruments
//! (`ha_entry_confinement`, `hr_truncated_prefix`) instead, and drives exactly
//! the same shipped reader.
//!
//! Never on by default, never packaged, writes only to stdout.
use std::time::Instant;

use snapshot::CertifiedReplayCache;

fn main() {
    if let Err(error) = run() {
        eprintln!("mc_scrub_bench: {error}");
        std::process::exit(1);
    }
}

/// Deterministic 64-bit LCG. A bench that picked hours from a system RNG could
/// not be re-run against its own numbers.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
}

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

fn stats(label: &str, mut samples: Vec<f64>, extra: &str) {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    println!(
        "{{\"metric\":\"{label}\",\"n\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"min_ms\":{:.3},\"max_ms\":{:.3},\"mean_ms\":{:.3}{extra}}}",
        samples.len(),
        percentile(&samples, 50.0),
        percentile(&samples, 95.0),
        samples.first().copied().unwrap_or(f64::NAN),
        samples.last().copied().unwrap_or(f64::NAN),
        samples.iter().sum::<f64>() / samples.len() as f64,
    );
}

/// Open the pair the way the product opens it: the cache stays on disk
/// (`open_file`), which is the only shape that works for a campaign-sized
/// artifact and the one the container readers use.
///
/// The identity arguments are the ones this artifact was built with. These
/// fixtures come from `ll_revive_pricing`, which seals with a zero source
/// digest and the last hour's H4CR end offset as the source length; a caller
/// pointing this at a shipped recording passes that recording's own pair.
fn open(dir: &str) -> Result<(CertifiedReplayCache, Vec<i64>, Vec<u32>, u32), String> {
    let index_bytes = std::fs::read(format!("{dir}/hour_index.h4ix"))
        .map_err(|e| format!("read index: {e}"))?;
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes)
        .map_err(|e| format!("parse index: {e}"))?;
    let source_len = index.source_len();
    let source_sha = index.source_sha256().to_string();
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let depths: Vec<u32> = index.hours().iter().map(|h| h.delta_steps()).collect();
    let interval = index.checkpoint_interval_hours();
    let file = std::fs::File::open(format!("{dir}/checkpoints.h4cp"))
        .map_err(|e| format!("open cache: {e}"))?;
    let cache = CertifiedReplayCache::open_file(&index_bytes, file, source_len, &source_sha)
        .map_err(|e| format!("open cache: {e}"))?;
    Ok((cache, hours, depths, interval))
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("usage: mc_scrub_bench <mode> <dir> ...")?;
    let dir = args.next().ok_or("usage: mc_scrub_bench <mode> <dir> ...")?;
    let (cache, hours, depths, interval) = open(&dir)?;
    println!(
        "{{\"artifact\":\"{}\",\"hours\":{},\"checkpoint_interval\":{interval},\"max_depth\":{}}}",
        dir.replace('\\', "/"),
        hours.len(),
        depths.iter().copied().max().unwrap_or(0),
    );

    match mode.as_str() {
        "depth" => {
            // One seek per delta distance, ascending. Each is measured with the
            // cursor DISABLED, so every sample is the cold cost the product
            // pays today for a jump into an unvisited window.
            let mut targets: Vec<usize> = Vec::new();
            let mut step = 0usize;
            while step < depths.len() {
                if depths.iter().any(|d| *d as usize == step) {
                    targets.push(step);
                }
                step = if step == 0 { 1 } else { step * 2 };
            }
            if let Some(deepest) = depths.iter().copied().max() {
                if !targets.contains(&(deepest as usize)) {
                    targets.push(deepest as usize);
                }
            }
            for want in targets {
                let Some(position) = depths.iter().position(|d| *d as usize == want) else {
                    continue;
                };
                let hour = hours[position];
                cache.forget_scrub_cursor();
                let start = Instant::now();
                let (json, receipt) = cache
                    .reconstruct_json(hour)
                    .map_err(|e| format!("hour {hour}: {e}"))?;
                let ms = start.elapsed().as_secs_f64() * 1000.0;
                println!(
                    "{{\"metric\":\"depth\",\"delta_steps\":{},\"hour\":{hour},\"cold_ms\":{ms:.3},\"world_json_bytes\":{}}}",
                    receipt.applied_deltas,
                    json.len(),
                );
            }
        }
        "random" => {
            let n: usize = args
                .next()
                .unwrap_or_else(|| "40".into())
                .parse()
                .map_err(|e| format!("n: {e}"))?;
            let seed: u64 = args
                .next()
                .unwrap_or_else(|| "20260817".into())
                .parse()
                .map_err(|e| format!("seed: {e}"))?;
            let mut rng = Lcg(seed);
            let picks: Vec<i64> = (0..n)
                .map(|_| hours[(rng.next() as usize) % hours.len()])
                .collect();

            let mut cold = Vec::with_capacity(n);
            for hour in &picks {
                cache.forget_scrub_cursor();
                let start = Instant::now();
                cache
                    .reconstruct_json(*hour)
                    .map_err(|e| format!("hour {hour}: {e}"))?;
                cold.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            stats("random_seek_cold", cold, "");

            // Same picks, cursor left alive between them: this is the shape a
            // real scrub has, where consecutive requests are usually close.
            cache.forget_scrub_cursor();
            let mut warm = Vec::with_capacity(n);
            for hour in &picks {
                let start = Instant::now();
                cache
                    .reconstruct_json(*hour)
                    .map_err(|e| format!("hour {hour}: {e}"))?;
                warm.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            stats("random_seek_cursored", warm, "");

            // A forward play run: every hour in order, which is what the
            // transport does at any speed step.
            cache.forget_scrub_cursor();
            let mut play = Vec::with_capacity(hours.len());
            for hour in &hours {
                let start = Instant::now();
                cache
                    .reconstruct_json(*hour)
                    .map_err(|e| format!("hour {hour}: {e}"))?;
                play.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            stats("forward_step_cursored", play, "");
        }
        "tail" => {
            // THE FLOOR UNDER EVERY SEEK STRATEGY. Ask for the hour the cursor
            // is ALREADY standing on: zero checkpoint decodes, zero delta
            // applies. What is left is what `reconstruct_json` does after the
            // walk and cannot skip -- identity validation, spelling the world
            // back out as canonical JSON, and hashing that JSON against the
            // parity digest the recording published. No cursor, ladder or
            // prefetch can make this smaller; only not doing it can.
            let n: usize = args
                .next()
                .unwrap_or_else(|| "10".into())
                .parse()
                .map_err(|e| format!("n: {e}"))?;
            // The widest hour in the recording, so the floor is reported where
            // it bites hardest rather than at an attach-ramp hour.
            let deepest = depths
                .iter()
                .enumerate()
                .max_by_key(|(_, depth)| **depth)
                .map(|(position, _)| position)
                .unwrap_or(0);
            let hour = hours[deepest];
            let (json, receipt) = cache
                .reconstruct_json(hour)
                .map_err(|e| format!("hour {hour}: {e}"))?;
            let bytes = json.len();
            drop(json);
            let mut tail = Vec::with_capacity(n);
            for _ in 0..n {
                let start = Instant::now();
                let (_, again) = cache
                    .reconstruct_json(hour)
                    .map_err(|e| format!("hour {hour}: {e}"))?;
                tail.push(start.elapsed().as_secs_f64() * 1000.0);
                if again.walked_deltas != 0 {
                    return Err(format!(
                        "hour {hour} walked {} deltas when the cursor was already on it",
                        again.walked_deltas
                    ));
                }
            }
            stats(
                "serialize_and_parity_tail_only",
                tail,
                &format!(
                    ",\"world_json_bytes\":{bytes},\"index_declared_deltas\":{}",
                    receipt.applied_deltas
                ),
            );
        }
        "noise" => {
            let n: usize = args
                .next()
                .unwrap_or_else(|| "20".into())
                .parse()
                .map_err(|e| format!("n: {e}"))?;
            let hour = hours[hours.len() / 2];
            let mut cold = Vec::with_capacity(n);
            for _ in 0..n {
                cache.forget_scrub_cursor();
                let start = Instant::now();
                cache
                    .reconstruct_json(hour)
                    .map_err(|e| format!("hour {hour}: {e}"))?;
                cold.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            stats("noise_floor_same_hour_cold", cold, "");
        }
        other => return Err(format!("unknown mode {other:?}")),
    }
    Ok(())
}
