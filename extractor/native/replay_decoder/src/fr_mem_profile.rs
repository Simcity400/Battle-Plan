//! Lane FR resident-memory instrument: attribute post-capture finalize's
//! RESIDENT BYTES to named owners, the way `cz_finalize_profile` attributes its
//! wall time to phases.
//!
//! The recorder already published a process peak
//! (`host_finalize.peak_private_bytes`) and one prose claim about the capture
//! path's bound. A peak says how much and never what, so this instrument runs
//! the real [`crate::build_certified_replay_artifacts`] -- same decoder, same
//! encoder, same verification, no substitutions -- while:
//!
//!   * an allocation-tracking allocator maintains the live heap
//!     (`recorder_host/mem-alloc-trace`), which no forgotten charge can
//!     under-report — though it covers only what passes through Rust's global
//!     allocator, so a native library's own arena (libzstd's compression
//!     context above all) is absent from it and present only in the OS's
//!     private-bytes column; and
//!   * `recorder_host::mem_account`'s always-on component counters record what
//!     each named owner claims to hold.
//!
//! Sampling both on the same cadence is what makes the table checkable: the gap
//! between the live heap and the sum of the components is exactly the part of
//! the footprint nobody has named yet, and it is written to every row rather
//! than summarized away.
//!
//! Read-only with respect to its input stream; every artifact lands in the
//! caller's own empty spill directory. Never on by default, never packaged.
//!
//! Wall time measured under this instrument is NOT comparable to production:
//! the tracking allocator adds an atomic RMW to every allocation. Timing
//! comparisons belong to `cc_cache_probe`, which runs the same call on the
//! production allocator.

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use recorder_host::mem_account::{self, Component};

use crate::{OpenError, StreamIdentityMode};

/// Sampling cadence. Fast enough to catch a phase that lasts a few seconds
/// (the end-of-finalize packing spike is the one that matters), slow enough
/// that the sampler's own cost is irrelevant next to a multi-minute fold.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// Samples retained. Reserved up front, once, so the sampler's own storage is a
/// CONSTANT inside the tracked live heap instead of a `Vec` that reallocates
/// ten times a second for the length of the run — growth that would have landed
/// in the very residual this instrument asks the reader to interpret. At the
/// 100 ms cadence this covers 3.6 hours; past that, sampling stops rather than
/// grows, and `samples_truncated` says so.
const MAX_SAMPLES: usize = 128 * 1024;

struct Sample {
    elapsed_ms: u128,
    /// `None` when the OS call failed. Kept optional all the way to the CSV,
    /// where it is written as an empty field: a zero here would read as a
    /// process that momentarily held no memory.
    private_bytes: Option<u64>,
    working_set_bytes: Option<u64>,
    live_heap_bytes: u64,
    components: [u64; mem_account::COMPONENT_COUNT],
}

/// Run the certified artifact build under memory sampling and write the
/// per-sample CSV plus a summary to `report_dir`.
///
/// `report_dir` must already exist; `spill_dir` must be the empty directory the
/// artifact build requires.
#[allow(clippy::too_many_arguments)]
pub fn profile_finalize_memory(
    stream: &Path,
    spill_dir: &Path,
    report_dir: &Path,
    game_id: &str,
    identity_mode: StreamIdentityMode,
    source_len: u64,
    source_sha256: &str,
    checkpoint_interval_hours: u32,
) -> Result<String, OpenError> {
    let started = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let samples = Arc::new(std::sync::Mutex::new(Vec::<Sample>::with_capacity(
        MAX_SAMPLES,
    )));
    let truncated = Arc::new(AtomicBool::new(false));
    // Published so the reader can subtract it: the reservation is real memory
    // this instrument added to the process, and a residual that quietly
    // included it would overstate the recorder.
    let sampler_reserved_bytes = (MAX_SAMPLES * std::mem::size_of::<Sample>()) as u64;

    let sampler = {
        let stop = Arc::clone(&stop);
        let samples = Arc::clone(&samples);
        let truncated = Arc::clone(&truncated);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let current = recorder_host::proc_mem::current_memory();
                let mut components = [0u64; mem_account::COMPONENT_COUNT];
                for (slot, component) in components.iter_mut().zip(Component::ALL.iter()) {
                    *slot = mem_account::held(*component);
                }
                let sample = Sample {
                    elapsed_ms: started.elapsed().as_millis(),
                    private_bytes: current.map(|m| m.private_bytes),
                    working_set_bytes: current.map(|m| m.working_set_bytes),
                    live_heap_bytes: mem_account::live_heap_bytes().unwrap_or(0),
                    components,
                };
                if let Ok(mut samples) = samples.lock() {
                    // Never grow: a reallocation here would be the instrument
                    // moving the number it is reporting.
                    if samples.len() == samples.capacity() {
                        truncated.store(true, Ordering::Relaxed);
                    } else {
                        samples.push(sample);
                    }
                }
                std::thread::sleep(SAMPLE_INTERVAL);
            }
        })
    };

    mem_account::mark("fr_profile.before_build");
    let build = crate::build_certified_replay_artifacts(
        stream,
        spill_dir,
        game_id.to_string(),
        identity_mode,
        source_len,
        source_sha256,
        checkpoint_interval_hours,
    );
    mem_account::mark("fr_profile.after_build");
    let wall = started.elapsed();

    stop.store(true, Ordering::Relaxed);
    let _ = sampler.join();

    let artifacts = build?;

    let samples = samples
        .lock()
        .map(|mut s| std::mem::take(&mut *s))
        .unwrap_or_default();

    // --- CSV: one row per sample, every number the summary is derived from ---
    let csv_path = report_dir.join("finalize_mem_samples.csv");
    let mut csv = std::fs::File::create(&csv_path)?;
    let mut header = String::from("elapsed_ms,private_bytes,working_set_bytes,live_heap_bytes");
    for component in Component::ALL {
        header.push(',');
        header.push_str(component.key());
    }
    // Signed, for the same reason the production report is: saturating a
    // negative residual to zero would hide the one arithmetic signal that can
    // catch an over-charge. Read it with `overaccounted_sample_note`: a sample
    // is not an atomic snapshot, so one negative row can be a torn read, while
    // many of them cannot.
    header.push_str(",named_total_bytes,unnamed_heap_bytes\n");
    csv.write_all(header.as_bytes())?;
    let mut overaccounted_samples = 0u64;
    for sample in &samples {
        let named: u64 = sample.components.iter().fold(0, |a, b| a.saturating_add(*b));
        let unnamed = sample.live_heap_bytes as i128 - named as i128;
        if unnamed < 0 {
            overaccounted_samples += 1;
        }
        let field = |value: Option<u64>| match value {
            Some(bytes) => bytes.to_string(),
            None => String::new(),
        };
        let mut row = format!(
            "{},{},{},{}",
            sample.elapsed_ms,
            field(sample.private_bytes),
            field(sample.working_set_bytes),
            sample.live_heap_bytes
        );
        for bytes in sample.components {
            row.push_str(&format!(",{bytes}"));
        }
        row.push_str(&format!(",{named},{unnamed}\n"));
        csv.write_all(row.as_bytes())?;
    }
    csv.flush()?;

    // --- summary: peaks, and the residual the component table does not name ---
    let sampled_peak_private = samples.iter().filter_map(|s| s.private_bytes).max();
    let sampled_peak_working_set = samples.iter().filter_map(|s| s.working_set_bytes).max();
    let unreadable_os_samples = samples
        .iter()
        .filter(|s| s.private_bytes.is_none())
        .count() as u64;
    let sampled_peak_live_heap = samples.iter().map(|s| s.live_heap_bytes).max().unwrap_or(0);
    // The exact peaks for the same quantities. A 100 ms sampler misses any
    // spike shorter than its interval, so the sampled maxima are NAMED as
    // sampled and the authoritative numbers sit beside them: Windows keeps
    // process-lifetime peaks itself, and the tracking allocator keeps an exact
    // live-heap peak taken on the allocation that set it.
    let os_peak = recorder_host::proc_mem::peak_memory();
    let exact_peak_live_heap = mem_account::peak_live_heap_bytes();
    // The unnamed residual is taken at the sample where the LIVE HEAP peaked,
    // not as a max over samples: maxing each term independently would report a
    // residual that no single instant ever exhibited.
    let peak_row = samples
        .iter()
        .max_by_key(|s| s.live_heap_bytes)
        .map(|s| {
            let named: u64 = s.components.iter().fold(0, |a, b| a.saturating_add(*b));
            serde_json::json!({
                "elapsed_ms": s.elapsed_ms as u64,
                "live_heap_bytes": s.live_heap_bytes,
                "named_total_bytes": named,
                // Signed: negative means the components claimed more than the
                // heap held at that instant, which is an accounting defect and
                // must not be rounded away to zero.
                "unnamed_heap_bytes": s.live_heap_bytes as i128 - named as i128,
            })
        })
        .unwrap_or(serde_json::Value::Null);

    let mut components = serde_json::Map::new();
    for (index, component) in Component::ALL.iter().enumerate() {
        let observed_peak = samples
            .iter()
            .map(|s| s.components[index])
            .max()
            .unwrap_or(0);
        components.insert(
            component.key().to_string(),
            serde_json::json!({
                "sampled_peak_bytes": observed_peak,
                "registry_peak_bytes": mem_account::peak(*component),
                "scaling": component.scaling().key(),
                "accounting_underflow": mem_account::underflowed(*component),
            }),
        );
    }

    let marks: Vec<serde_json::Value> = mem_account::marks()
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "label": entry.label,
                "private_bytes": entry.private_bytes,
                "live_heap_bytes": entry.live_heap_bytes,
            })
        })
        .collect();

    let report = serde_json::json!({
        "schema": "hoi4-fr-finalize-memory-profile-v1",
        "stream": stream.display().to_string(),
        "stream_decoded_bytes": source_len,
        "hour_count": artifacts.hour_count,
        "checkpoint_interval_hours": artifacts.checkpoint_interval_hours,
        "checkpoint_cache_len": artifacts.checkpoint_cache_len,
        "hour_index_len": artifacts.hour_index_len,
        "legacy_json_bytes_avoided": artifacts.legacy_json_bytes_avoided,
        "wall_s": wall.as_secs_f64(),
        "wall_s_note": "measured under an allocation-tracking allocator; NOT comparable to a \
                        production finalize time",
        "sample_count": samples.len(),
        "sample_interval_ms": SAMPLE_INTERVAL.as_millis() as u64,
        "sampled_peak_private_bytes": sampled_peak_private,
        "sampled_peak_working_set_bytes": sampled_peak_working_set,
        "sampled_peak_live_heap_bytes": sampled_peak_live_heap,
        "os_peak_private_bytes": os_peak.map(|peak| peak.peak_private_bytes),
        "os_peak_working_set_bytes": os_peak.map(|peak| peak.peak_working_set_bytes),
        "exact_peak_live_heap_bytes": exact_peak_live_heap,
        "peak_semantics": "sampled_* are maxima over 100 ms samples and can miss a shorter \
                           spike; os_peak_* and exact_peak_live_heap_bytes are exact, and are \
                           process-lifetime, so they also cover this instrument's own startup",
        "overaccounted_sample_count": overaccounted_samples,
        "overaccounted_sample_note": "samples where the named components claimed more than the \
                                      tracked live heap held. A sample is NOT an atomic \
                                      snapshot: the component counters and the live-heap counter \
                                      are read one after another while two threads allocate, so \
                                      an isolated negative row can be a torn read. The signal is \
                                      a count that is a large fraction of sample_count, or one \
                                      that persists across runs.",
        "unreadable_os_sample_count": unreadable_os_samples,
        "samples_truncated": truncated.load(Ordering::Relaxed),
        "sampler_reserved_bytes": sampler_reserved_bytes,
        "sampler_reserved_note": "the sampler's own fixed buffer, counted inside every \
                                  live_heap_bytes reading; subtract it to price the recorder \
                                  alone",
        "components_not_exercised": ["spool_writer"],
        "components_not_exercised_note": "this instrument runs finalize only; no capture spool \
                                          is opened, so spool_writer is structurally absent \
                                          here rather than measured at zero",
        "at_live_heap_peak": peak_row,
        "components": components,
        "marks": marks,
        "samples_csv": csv_path.display().to_string(),
    });
    let report = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    let report_path = report_dir.join("finalize_mem_report.json");
    std::fs::write(&report_path, report.as_bytes())?;
    Ok(report)
}
