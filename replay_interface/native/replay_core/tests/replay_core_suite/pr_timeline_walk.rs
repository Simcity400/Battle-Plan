//! Lane PR: the landing-gate cost model, measured. `uz_rc_vanilla8_operands`
//! opens the sealed rc_vanilla8 container and asks `WorldTimeline::frame` for
//! every hour; this walks the same path and prints what each hour cost and what
//! the process peaked at, so the gate's wall clock is attributed rather than
//! guessed. Measurement only: `PR_CAPTURE` names the container.

use std::time::Instant;

use replay_core::{open_recording, WorldTimeline};

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

fn peak_working_set_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(
                process: isize,
                counters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }
        let mut counters = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            ..Default::default()
        };
        // SAFETY: a pseudo-handle to this process and a correctly sized struct.
        let ok = unsafe {
            K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb)
        };
        (ok != 0).then_some(counters.peak_working_set_size as u64)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[test]
#[ignore = "measurement: needs PR_CAPTURE pointing at a sealed .battleplan"]
fn walk_every_hour_of_the_capture_the_way_the_gate_does() {
    let path = std::env::var("PR_CAPTURE").expect("PR_CAPTURE names a sealed container");
    let open_started = Instant::now();
    let timeline = WorldTimeline::open(open_recording(std::path::Path::new(&path)).expect("open"))
        .expect("timeline");
    let open_ms = open_started.elapsed().as_secs_f64() * 1e3;
    let hours = timeline.hours().to_vec();
    let walk_started = Instant::now();
    let mut per_hour_ms: Vec<f64> = Vec::with_capacity(hours.len());
    for hour in &hours {
        let started = Instant::now();
        let frame = timeline.frame(*hour).expect("frame");
        std::hint::black_box(&frame);
        per_hour_ms.push(started.elapsed().as_secs_f64() * 1e3);
    }
    let walk_ms = walk_started.elapsed().as_secs_f64() * 1e3;
    let mut sorted = per_hour_ms.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let p = |q: f64| sorted[((q / 100.0) * (sorted.len() - 1) as f64).round() as usize];
    println!(
        "{{\"tool\":\"pr_timeline_walk\",\"capture\":\"{}\",\"hours\":{},\"open_ms\":{open_ms:.1},\"walk_ms\":{walk_ms:.1},\"per_hour_ms\":{{\"p50\":{:.1},\"p95\":{:.1},\"max\":{:.1},\"first\":{:.1},\"last\":{:.1}}},\"frame_cache_entries\":{},\"peak_working_set_bytes\":{}}}",
        path.replace('\\', "/"),
        hours.len(),
        p(50.0),
        p(95.0),
        sorted.last().copied().unwrap_or(f64::NAN),
        per_hour_ms.first().copied().unwrap_or(f64::NAN),
        per_hour_ms.last().copied().unwrap_or(f64::NAN),
        timeline.frame_cache_entries(),
        peak_working_set_bytes().map(|b| b.to_string()).unwrap_or_else(|| "null".into()),
    );
}
