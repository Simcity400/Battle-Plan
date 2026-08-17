//! Lane MN: price EVERY stage the viewer pays to put one game hour on screen,
//! on a real recording's index/cache pair, using THE SHIPPED code — and prove
//! the cheap route serves the same world as the expensive one it replaces.
//!
//! Lane MC measured the seek and found the floor: asking for the hour the
//! cursor already stands on still cost 179 ms on capture t53, all of it in
//! `to_json_bytes` plus the parity SHA-256. That was measured at the CACHE
//! boundary. The product paid more above it — a strict JSON parse of those same
//! bytes and a `serde(flatten)` deserialize into `Snapshot` — and the owner's
//! ruling ("that doesn't seem like a very performant way of doing things") is
//! about the whole round trip, not one stage of it.
//!
//! `cost <dir> [samples]` prices, on the same hours:
//!   OLD  reconstruct_json (walk + serialize + parity digest)
//!        + parse_json_strict + serde_json::from_value::<Snapshot>
//!   NEW  reconstruct_value (walk + direct tree conversion)
//!        + Snapshot::from_world_value
//! for a forward step and for the hour the cursor already stands on.
//!
//! `equal <dir> [stride]` is the DIFFERENTIAL: it walks the recording through
//! both routes and requires the same typed body for every hour, in five request
//! patterns (forward, same-hour, backward, scattered/cross-window, and a
//! resumed-after-abandon walk). The old route is the parity-checked one, so an
//! hour that agrees with it agrees with the digest `replay_decoder` sealed.
//!
//! WHY AN INSTRUMENT AND NOT A TEST. A committed test asserts a budget on a
//! committed fixture; this prices and proves on the real captures, whose
//! artifacts are far too large to commit.
//!
//! WHY IT LIVES ON THE ACQUISITION SIDE. Same reason as `mc_scrub_bench`: it
//! names the two files it is pointed at, and the replay side's
//! `production_sources_have_no_descendant_pathname_read` audit scans every
//! module under that crate's `src/`.
//!
//! Never on by default, never packaged, writes only to stdout.
use std::time::Instant;

use snapshot::CertifiedReplayCache;

fn main() {
    if let Err(error) = run() {
        eprintln!("mn_frame_cost: {error}");
        std::process::exit(1);
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
        "{{\"metric\":\"{label}\",\"n\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"min_ms\":{:.3},\"max_ms\":{:.3}{extra}}}",
        samples.len(),
        percentile(&samples, 50.0),
        percentile(&samples, 95.0),
        samples.first().copied().unwrap_or(f64::NAN),
        samples.last().copied().unwrap_or(f64::NAN),
    );
}

/// Open the pair the way the product opens it: the cache stays on disk.
fn open(dir: &str) -> Result<(CertifiedReplayCache, Vec<i64>, f64, u64), String> {
    let index_bytes = std::fs::read(format!("{dir}/hour_index.h4ix"))
        .map_err(|e| format!("read index: {e}"))?;
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes)
        .map_err(|e| format!("parse index: {e}"))?;
    let source_len = index.source_len();
    let source_sha = index.source_sha256().to_string();
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let cache_len = std::fs::metadata(format!("{dir}/checkpoints.h4cp"))
        .map_err(|e| format!("stat cache: {e}"))?
        .len();
    let file = std::fs::File::open(format!("{dir}/checkpoints.h4cp"))
        .map_err(|e| format!("open cache: {e}"))?;
    let started = Instant::now();
    let cache = CertifiedReplayCache::open_file(&index_bytes, file, source_len, &source_sha)
        .map_err(|e| format!("open cache: {e}"))?;
    let open_ms = started.elapsed().as_secs_f64() * 1e3;
    Ok((cache, hours, open_ms, cache_len))
}

/// The typed body the OLD route produced: bytes, hashed, parsed, flattened.
fn old_route(
    cache: &CertifiedReplayCache,
    hour: i64,
) -> Result<(snapshot::Snapshot, usize), String> {
    let (json, _) = cache
        .reconstruct_json(hour)
        .map_err(|e| format!("hour {hour} old route: {e}"))?;
    let len = json.len();
    let value = snapshot::parse_json_strict(&json).map_err(|e| format!("parse: {e}"))?;
    let body: snapshot::Snapshot =
        serde_json::from_value(value).map_err(|e| format!("flatten: {e}"))?;
    Ok((body, len))
}

/// The typed body the NEW route produces: the reader's own tree, converted.
fn new_route(cache: &CertifiedReplayCache, hour: i64) -> Result<snapshot::Snapshot, String> {
    let (world, _) = cache
        .reconstruct_value(hour)
        .map_err(|e| format!("hour {hour} new route: {e}"))?;
    snapshot::Snapshot::from_world_value(world).map_err(|e| format!("body: {e}"))
}

fn same_body(left: &snapshot::Snapshot, right: &snapshot::Snapshot, hour: i64) -> Result<(), String> {
    if left.game_id != right.game_id
        || left.date != right.date
        || left.hour_index != right.hour_index
        || left.save_version != right.save_version
        || left.schema_version != right.schema_version
    {
        return Err(format!("hour {hour}: header fields differ between routes"));
    }
    if left.rest != right.rest {
        for (key, value) in &left.rest {
            match right.rest.get(key) {
                Some(other) if other == value => {}
                Some(_) => return Err(format!("hour {hour}: domain {key} differs")),
                None => return Err(format!("hour {hour}: domain {key} missing from new route")),
            }
        }
        for key in right.rest.keys() {
            if !left.rest.contains_key(key) {
                return Err(format!("hour {hour}: domain {key} only in new route"));
            }
        }
        return Err(format!("hour {hour}: bodies differ but no domain does"));
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or("usage: mn_frame_cost <cost|equal> <dir> [n]")?;
    let dir = args.next().ok_or("usage: mn_frame_cost <cost|equal> <dir> [n]")?;
    let n: usize = args
        .next()
        .map(|value| value.parse().map_err(|_| "n must be a number"))
        .transpose()?
        .unwrap_or(8);
    let (cache, hours, open_ms, cache_len) = open(&dir)?;
    println!(
        "{{\"artifact\":\"{}\",\"hours\":{},\"cache_bytes\":{cache_len},\"open_admission_ms\":{:.1}}}",
        dir.replace('\\', "/"),
        hours.len(),
        open_ms
    );
    match mode.as_str() {
        "cost" => cost(&cache, &hours, n),
        "domains" => domains(&cache, &hours, n),
        "seek" => seek(&dir, &cache, &hours, n),
        "equal" => equal(&dir, &cache, &hours, n),
        other => Err(format!("unknown mode {other}")),
    }
}

fn cost(cache: &CertifiedReplayCache, hours: &[i64], samples: usize) -> Result<(), String> {
    // Forward play over the last hours of the recording, where the world is
    // largest. Each route is timed on the same hour, one forward delta apiece.
    let start = hours.len().saturating_sub(samples + 1);
    let mut old_ms = Vec::new();
    let mut new_ms = Vec::new();
    let mut bytes = 0usize;
    for position in (start + 1)..hours.len() {
        // Prime each route's cursor on the PRECEDING INDEXED hour — not on
        // `hour - 1`, which a recording with a coverage gap does not carry —
        // so the timed call is one delta. The two routes share one reader, so
        // each is re-primed before its own measurement.
        let hour = &hours[position];
        let previous = hours[position - 1];
        old_route(cache, previous)?;
        let t0 = Instant::now();
        let (old, len) = old_route(cache, *hour)?;
        old_ms.push(t0.elapsed().as_secs_f64() * 1e3);
        bytes = len;

        new_route(cache, previous)?;
        let t1 = Instant::now();
        let new = new_route(cache, *hour)?;
        new_ms.push(t1.elapsed().as_secs_f64() * 1e3);
        same_body(&old, &new, *hour)?;
    }
    let extra = format!(",\"world_mb\":{:.2}", bytes as f64 / 1e6);
    stats("forward_step_old_bytes_hash_parse_flatten", old_ms, &extra);
    stats("forward_step_new_direct_tree", new_ms, &extra);

    // The floor: the hour the cursor already stands on.
    let held = *hours.last().expect("a recording has hours");
    let mut old_same = Vec::new();
    let mut new_same = Vec::new();
    old_route(cache, held)?;
    for _ in 0..samples {
        let t0 = Instant::now();
        old_route(cache, held)?;
        old_same.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    new_route(cache, held)?;
    for _ in 0..samples {
        let t0 = Instant::now();
        new_route(cache, held)?;
        new_same.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    stats("same_hour_old_bytes_hash_parse_flatten", old_same, &extra);
    stats("same_hour_new_direct_tree", new_same, &extra);
    Ok(())
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

/// RANDOM SEEKS -- what dropping the seek bar somewhere costs, on both routes.
///
/// A random target is usually backward or into another checkpoint window, which
/// is exactly where a cursor cannot be resumed, so this prices the walk plus the
/// per-hour work rather than the per-hour work alone. Each route gets its own
/// reader so neither warms the other's cursor, and both are asked for the SAME
/// sequence of hours.
fn seek(dir: &str, cache: &CertifiedReplayCache, hours: &[i64], samples: usize) -> Result<(), String> {
    let (old_cache, _, _, _) = open(dir)?;
    let mut rng = Lcg(0x5eed_1234_9abc_def0);
    let targets: Vec<i64> = (0..samples)
        .map(|_| hours[(rng.next() as usize) % hours.len()])
        .collect();
    let mut old_ms = Vec::new();
    let mut new_ms = Vec::new();
    for hour in &targets {
        let t0 = Instant::now();
        old_route(&old_cache, *hour)?;
        old_ms.push(t0.elapsed().as_secs_f64() * 1e3);
        let t1 = Instant::now();
        new_route(cache, *hour)?;
        new_ms.push(t1.elapsed().as_secs_f64() * 1e3);
    }
    stats("random_seek_old_bytes_hash_parse_flatten", old_ms, "");
    stats("random_seek_new_direct_tree", new_ms, "");
    Ok(())
}

/// HOW MUCH OF A WORLD ACTUALLY CHANGES IN AN HOUR, by top-level domain.
///
/// This prices the lever the remaining floor needs. After the JSON round trip
/// is gone, what a drawn hour still costs is BUILDING the whole world as a
/// `serde_json::Value` tree — about 210 ms of a 13.9 MB t53 hour, all of it
/// allocation. The way out is to stop rebuilding what did not change: carry the
/// previous hour's converted domain across and re-convert only the domains this
/// hour's delta touched. Whether that is worth its complexity depends entirely
/// on how much of the world those domains are, which is a MEASUREMENT and is
/// this.
///
/// It compares consecutive hours domain by domain, so it reports the true
/// minimum (what actually differs), which is a lower bound on what a
/// patch-driven implementation would re-convert.
fn domains(cache: &CertifiedReplayCache, hours: &[i64], samples: usize) -> Result<(), String> {
    let start = hours.len().saturating_sub(samples + 1);
    let mut shares = Vec::new();
    let mut counts = Vec::new();
    let mut previous: Option<(i64, snapshot::Snapshot)> = None;
    for hour in &hours[start..] {
        let body = new_route(cache, *hour)?;
        if let Some((was, before)) = previous.take() {
            let mut changed_bytes = 0usize;
            let mut total_bytes = 0usize;
            let mut changed = Vec::new();
            for (key, value) in &body.rest {
                let size = serde_json::to_vec(value).map_err(|e| e.to_string())?.len();
                total_bytes += size;
                if before.rest.get(key) != Some(value) {
                    changed_bytes += size;
                    changed.push(key.clone());
                }
            }
            for key in before.rest.keys() {
                if !body.rest.contains_key(key) {
                    changed.push(key.clone());
                }
            }
            let share = if total_bytes == 0 {
                0.0
            } else {
                100.0 * changed_bytes as f64 / total_bytes as f64
            };
            println!(
                "{{\"hour\":{hour},\"from_hour\":{was},\"domains\":{},\"changed_domains\":{},\"world_bytes\":{total_bytes},\"changed_bytes\":{changed_bytes},\"changed_share_pct\":{share:.1},\"names\":{}}}",
                body.rest.len(),
                changed.len(),
                serde_json::to_string(&changed).map_err(|e| e.to_string())?
            );
            shares.push(share);
            counts.push(changed.len() as f64);
        }
        previous = Some((*hour, body));
    }
    stats("consecutive_hour_changed_share_pct", shares, "");
    stats("consecutive_hour_changed_domain_count", counts, "");
    Ok(())
}

/// The differential. Two readers over the same artifact so each route keeps its
/// own cursor, five request patterns, every hour compared.
fn equal(
    dir: &str,
    cache: &CertifiedReplayCache,
    hours: &[i64],
    stride: usize,
) -> Result<(), String> {
    let (old_cache, _, _, _) = open(dir)?;
    let mut checked = 0usize;

    // Forward and same-hour are EVERY hour: they are the orders playback and a
    // drag actually produce, and they are cheap because both routes' cursors
    // resume. Backward and scattered are sampled every `back_step` hours -- a
    // backward jump rebuilds from the checkpoint on BOTH routes (that is
    // residual R-B, not a property of this change), so walking all of them
    // costs hours of wall clock to re-prove a resume condition that the
    // sampled set already exercises at every window position.
    let back_step = 4usize;
    let forward: Vec<i64> = hours.to_vec();
    let backward: Vec<i64> = hours
        .iter()
        .rev()
        .step_by(back_step)
        .copied()
        .collect();
    let stride = stride.max(1);
    let scattered: Vec<i64> = (0..hours.len())
        .step_by(back_step)
        .map(|step| hours[(step * stride) % hours.len()])
        .collect();
    let mut same_hour: Vec<i64> = Vec::new();
    for hour in hours.iter().take(8) {
        same_hour.push(*hour);
        same_hour.push(*hour);
        same_hour.push(*hour);
    }

    for (label, order) in [
        ("forward", &forward),
        ("same_hour", &same_hour),
        ("backward", &backward),
        ("scattered", &scattered),
    ] {
        for hour in order.iter() {
            let (old, _) = old_route(&old_cache, *hour)?;
            let new = new_route(cache, *hour)?;
            same_body(&old, &new, *hour)?;
            checked += 1;
        }
        println!("{{\"pattern\":\"{label}\",\"hours_agreed\":{}}}", order.len());
    }

    // The fifth pattern: a walk the stop switch abandons part-way, then the
    // same hour asked for again. The resumed walk must still serve the world
    // the parity-checked route serves.
    let deepest = hours
        .iter()
        .copied()
        .max_by_key(|hour| {
            cache
                .index()
                .entry(*hour)
                .map(|entry| entry.delta_steps())
                .unwrap_or(0)
        })
        .expect("a recording has hours");
    cache.forget_scrub_cursor();
    let allowed = std::cell::Cell::new(3u32);
    let outcome = cache
        .reconstruct_value_interruptible(deepest, &|| {
            let go = allowed.get() > 0;
            allowed.set(allowed.get().saturating_sub(1));
            go
        })
        .map_err(|e| format!("abandoned walk: {e}"))?;
    let abandoned_at = match outcome {
        snapshot::FrameOutcome::Abandoned { reached_hour, .. } => reached_hour,
        snapshot::FrameOutcome::Reconstructed { .. } => deepest,
    };
    let resumed = new_route(cache, deepest)?;
    let (want, _) = old_route(&old_cache, deepest)?;
    same_body(&want, &resumed, deepest)?;
    checked += 1;
    println!(
        "{{\"pattern\":\"resumed_after_abandon\",\"deepest_hour\":{deepest},\"abandoned_at\":{abandoned_at},\"hours_agreed\":1}}"
    );
    println!("{{\"differential\":\"PASS\",\"comparisons\":{checked}}}");
    Ok(())
}
