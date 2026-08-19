//! them may pay coherence latency to reclaim ownership.

use super::*;

pub(super) const HO_MARKER_WATCH_ENV: &str = "BATTLE_PLAN_HO_MARKER_WATCH";
pub(super) const HO_MARKER_WATCH_SECONDS_ENV: &str = "BATTLE_PLAN_HO_MARKER_WATCH_SECONDS";

const DEFAULT_WINDOW_SECONDS: u64 = 60;
const EPOCH_MS: u64 = 200;
const CYCLE_EPOCHS: u64 = 5;
const LOW_EPOCH_PHASE: u64 = 4;
const LOW_SAMPLE_SLEEP_US: u64 = 500;
const MAX_EVENTS: usize = 400_000;
const MAX_EPOCHS: usize = 8_192;
const SINGLETON_RECHECK_SAMPLES: u64 = 4_096;

fn epoch_is_spin(index: u64) -> bool {
    index % CYCLE_EPOCHS != LOW_EPOCH_PHASE
}

fn window_seconds_from(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (1..=600).contains(value))
        .unwrap_or(DEFAULT_WINDOW_SECONDS)
}

#[derive(Clone, Copy)]
struct Transition {
    seq: u64,
    epoch: u64,
    lo_t_ns: u64,
    prev_t_ns: u64,
    t_ns: u64,
    prev_marker: u8,
    marker: u8,
    prev_date: i32,
    date: i32,
}

#[derive(Clone, Copy)]
struct EpochStat {
    index: u64,
    spin: bool,
    t_start_ns: u64,
    t_end_ns: u64,
    samples: u64,
    min_gap_ns: u64,
    max_gap_ns: u64,
    sum_gap_ns: u64,
    date_first: i32,
    date_last: i32,
    date_seen: bool,
}

impl EpochStat {
    fn new(index: u64, spin: bool, t_start_ns: u64) -> Self {
        Self {
            index,
            spin,
            t_start_ns,
            t_end_ns: t_start_ns,
            samples: 0,
            min_gap_ns: u64::MAX,
            max_gap_ns: 0,
            sum_gap_ns: 0,
            date_first: 0,
            date_last: 0,
            date_seen: false,
        }
    }
}

struct Config {
    path: String,
    window: Duration,
    module_base: usize,
    gs_ptr: usize,
    singleton_slot: usize,
    date_addr: usize,
    marker_addr: usize,
    window_seconds: u64,
}

pub(super) struct Guard {
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) fn start(module_base: usize, gs_ptr: usize) -> Option<Guard> {
    let raw_path = std::env::var(HO_MARKER_WATCH_ENV).ok()?;
    let path = raw_path.trim().to_string();
    if path.is_empty() {
        return None;
    }
    let window_seconds =
        window_seconds_from(std::env::var(HO_MARKER_WATCH_SECONDS_ENV).ok().as_deref());
    let singleton_slot = module_base
        .checked_add(compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)?;
    let observed = read_u64_stable(singleton_slot)? as usize;
    if observed == 0 || observed != gs_ptr || !is_heap_pointer_aligned(observed, 8) {
        return None;
    }
    let date_addr = gs_ptr.checked_add(GS_CURRENT_DATE_OFFSET)?;
    let marker_addr = gs_ptr.checked_add(GS_TICK_BUSY_MARKER_OFFSET)?;
    read_scalar::<4>(date_addr)?;
    read_scalar::<1>(marker_addr)?;
    let cfg = Config {
        path,
        window: Duration::from_secs(window_seconds),
        module_base,
        gs_ptr,
        singleton_slot,
        date_addr,
        marker_addr,
        window_seconds,
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread_shutdown = Arc::clone(&shutdown);
    let handle = thread::Builder::new()
        .name("ho-marker-watch".into())
        .spawn(move || run(cfg, thread_shutdown))
        .ok()?;
    Some(Guard {
        shutdown,
        handle: Some(handle),
    })
}

fn run(cfg: Config, shutdown: Arc<AtomicBool>) {
    let origin = Instant::now();
    let mut events: Vec<Transition> = Vec::with_capacity(MAX_EVENTS);
    let mut epochs: Vec<EpochStat> = Vec::with_capacity(MAX_EPOCHS);
    let mut seq: u64 = 0;
    let mut truncated_events = false;
    let mut abort: Option<&'static str> = None;

    let marker_ptr = cfg.marker_addr as *const u8;
    let date_ptr = cfg.date_addr as *const i32;
    let slot_ptr = cfg.singleton_slot as *const u64;

    let mut prev_marker = unsafe { core::ptr::read_volatile(marker_ptr) };
    let mut prev_date = unsafe { core::ptr::read_volatile(date_ptr) };
    let mut prev_t_ns = duration_nanos_u64(origin.elapsed());
    let mut lo_t_ns = 0u64;
    let seed_marker = prev_marker;
    let seed_date = prev_date;

    let mut epoch_index: u64 = 0;
    'window: while !shutdown.load(Ordering::Acquire) {
        let epoch_start = origin.elapsed();
        if epoch_start >= cfg.window {
            break;
        }
        if epochs.len() >= MAX_EPOCHS {
            abort = Some("epoch capacity exhausted");
            break;
        }
        let spin = epoch_is_spin(epoch_index);
        let epoch_end = epoch_start + Duration::from_millis(EPOCH_MS);
        let mut stat = EpochStat::new(epoch_index, spin, duration_nanos_u64(epoch_start));
        let mut since_recheck: u64 = 0;
        loop {
            let marker = unsafe { core::ptr::read_volatile(marker_ptr) };
            let date = unsafe { core::ptr::read_volatile(date_ptr) };
            let now = origin.elapsed();
            let t_ns = duration_nanos_u64(now);

            stat.samples = stat.samples.saturating_add(1);
            let gap = t_ns.saturating_sub(prev_t_ns);
            stat.min_gap_ns = stat.min_gap_ns.min(gap);
            stat.max_gap_ns = stat.max_gap_ns.max(gap);
            stat.sum_gap_ns = stat.sum_gap_ns.saturating_add(gap);
            if !stat.date_seen {
                stat.date_first = date;
                stat.date_seen = true;
            }
            stat.date_last = date;
            stat.t_end_ns = t_ns;

            if marker != prev_marker || date != prev_date {
                if events.len() < MAX_EVENTS {
                    events.push(Transition {
                        seq,
                        epoch: epoch_index,
                        lo_t_ns,
                        prev_t_ns,
                        t_ns,
                        prev_marker,
                        marker,
                        prev_date,
                        date,
                    });
                } else {
                    truncated_events = true;
                }
                seq = seq.saturating_add(1);
            }
            prev_marker = marker;
            prev_date = date;
            lo_t_ns = prev_t_ns;
            prev_t_ns = t_ns;

            since_recheck = since_recheck.saturating_add(1);
            if since_recheck >= SINGLETON_RECHECK_SAMPLES {
                since_recheck = 0;
                let slot = unsafe { core::ptr::read_volatile(slot_ptr) } as usize;
                if slot != cfg.gs_ptr {
                    abort = Some("CGameState singleton was replaced during the window");
                    epochs.push(stat);
                    break 'window;
                }
            }

            if shutdown.load(Ordering::Acquire) {
                epochs.push(stat);
                break 'window;
            }
            if now >= epoch_end || now >= cfg.window {
                break;
            }
            if spin {
                core::hint::spin_loop();
            } else {
                thread::sleep(Duration::from_micros(LOW_SAMPLE_SLEEP_US));
            }
        }
        epochs.push(stat);
        epoch_index = epoch_index.saturating_add(1);
    }

    let total_ns = duration_nanos_u64(origin.elapsed());
    let manifest = render_manifest(
        &cfg,
        &events,
        &epochs,
        seq,
        truncated_events,
        abort,
        seed_marker,
        seed_date,
        total_ns,
    );
    if let Ok(mut file) = std::fs::File::create(&cfg.path) {
        let _ = file.write_all(manifest.as_bytes());
        let _ = file.flush();
    }
}

fn duration_nanos_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[allow(clippy::too_many_arguments)]
fn render_manifest(
    cfg: &Config,
    events: &[Transition],
    epochs: &[EpochStat],
    seq: u64,
    truncated_events: bool,
    abort: Option<&'static str>,
    seed_marker: u8,
    seed_date: i32,
    total_ns: u64,
) -> String {
    let mut out = String::with_capacity(96 * (events.len() + epochs.len()) + 1024);
    out.push_str(&format!(
        "{{\"schema\":\"battle-plan.ho_marker_watch.v1\",\"kind\":\"header\",\
         \"collector_profile\":\"{profile}\",\
         \"module_base\":\"0x{base:X}\",\"cgamestate\":\"0x{gs:X}\",\
         \"singleton_slot\":\"0x{slot:X}\",\
         \"singleton_rva\":\"0x{rva:X}\",\
         \"date_addr\":\"0x{date:X}\",\"date_offset\":\"0x{date_off:X}\",\
         \"marker_addr\":\"0x{marker:X}\",\"marker_offset\":\"0x{marker_off:X}\",\
         \"window_seconds\":{window},\"epoch_ms\":{epoch_ms},\
         \"cycle_epochs\":{cycle},\"low_epoch_phase\":{low_phase},\
         \"low_sample_sleep_us\":{low_sleep},\
         \"max_events\":{max_events},\"singleton_recheck_samples\":{recheck},\
         \"seed_marker\":{seed_marker},\"seed_date\":{seed_date}}}\n",
        profile = super::super::COLLECTOR_PROFILE_ID,
        base = cfg.module_base,
        gs = cfg.gs_ptr,
        slot = cfg.singleton_slot,
        rva = compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
        date = cfg.date_addr,
        date_off = GS_CURRENT_DATE_OFFSET,
        marker = cfg.marker_addr,
        marker_off = GS_TICK_BUSY_MARKER_OFFSET,
        window = cfg.window_seconds,
        epoch_ms = EPOCH_MS,
        cycle = CYCLE_EPOCHS,
        low_phase = LOW_EPOCH_PHASE,
        low_sleep = LOW_SAMPLE_SLEEP_US,
        max_events = MAX_EVENTS,
        recheck = SINGLETON_RECHECK_SAMPLES,
    ));
    for epoch in epochs {
        out.push_str(&format!(
            "{{\"kind\":\"epoch\",\"index\":{index},\"arm\":\"{arm}\",\
             \"t_start_ns\":{start},\"t_end_ns\":{end},\"samples\":{samples},\
             \"min_gap_ns\":{min_gap},\"max_gap_ns\":{max_gap},\"sum_gap_ns\":{sum_gap},\
             \"date_first\":{date_first},\"date_last\":{date_last}}}\n",
            index = epoch.index,
            arm = if epoch.spin { "spin" } else { "low" },
            start = epoch.t_start_ns,
            end = epoch.t_end_ns,
            samples = epoch.samples,
            min_gap = if epoch.min_gap_ns == u64::MAX {
                0
            } else {
                epoch.min_gap_ns
            },
            max_gap = epoch.max_gap_ns,
            sum_gap = epoch.sum_gap_ns,
            date_first = epoch.date_first,
            date_last = epoch.date_last,
        ));
    }
    for event in events {
        out.push_str(&format!(
            "{{\"kind\":\"event\",\"seq\":{seq},\"epoch\":{epoch},\
             \"lo_t_ns\":{lo_t},\"prev_t_ns\":{prev_t},\"t_ns\":{t},\
             \"prev_marker\":{prev_marker},\"marker\":{marker},\
             \"prev_date\":{prev_date},\"date\":{date}}}\n",
            seq = event.seq,
            epoch = event.epoch,
            lo_t = event.lo_t_ns,
            prev_t = event.prev_t_ns,
            t = event.t_ns,
            prev_marker = event.prev_marker,
            marker = event.marker,
            prev_date = event.prev_date,
            date = event.date,
        ));
    }
    out.push_str(&format!(
        "{{\"kind\":\"summary\",\"total_ns\":{total_ns},\"epochs\":{epochs_len},\
         \"transitions_observed\":{seq},\"transitions_recorded\":{recorded},\
         \"events_truncated\":{truncated},\"abort\":{abort}}}\n",
        total_ns = total_ns,
        epochs_len = epochs.len(),
        seq = seq,
        recorded = events.len(),
        truncated = truncated_events,
        abort = match abort {
            Some(reason) => format!("\"{reason}\""),
            None => "null".to_string(),
        },
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fifth_epoch_is_the_low_load_control_arm() {
        let arms: Vec<bool> = (0..12).map(epoch_is_spin).collect();
        assert_eq!(
            arms,
            vec![true, true, true, true, false, true, true, true, true, false, true, true]
        );
        let spin = arms.iter().filter(|is_spin| **is_spin).count();
        assert_eq!(spin, 10, "80 % duty cycle");
    }

    #[test]
    fn window_seconds_is_bounded_and_defaults_to_sixty() {
        assert_eq!(window_seconds_from(None), 60);
        assert_eq!(window_seconds_from(Some("")), 60);
        assert_eq!(window_seconds_from(Some("not-a-number")), 60);
        assert_eq!(window_seconds_from(Some("0")), 60);
        assert_eq!(window_seconds_from(Some("601")), 60);
        assert_eq!(window_seconds_from(Some(" 90 ")), 90);
        assert_eq!(window_seconds_from(Some("600")), 600);
    }

    #[test]
    fn the_two_watched_fields_are_the_validated_addresses() {
        assert_eq!(GS_TICK_BUSY_MARKER_OFFSET, 0x4C0);
        assert_eq!(GS_CURRENT_DATE_OFFSET, 0x468);
    }

    #[test]
    fn the_rigorous_lower_bound_lags_the_point_estimate_by_one_sample() {
        let mut lo = 10u64;
        let mut prev = 20u64;
        let mut seen = Vec::new();
        for stamp in [30u64, 40, 50] {
            seen.push((lo, prev, stamp));
            lo = prev;
            prev = stamp;
        }
        assert_eq!(seen, vec![(10, 20, 30), (20, 30, 40), (30, 40, 50)]);
        for (lo_t, prev_t, t) in seen {
            assert!(
                lo_t < prev_t,
                "the rigorous bound must precede the estimate"
            );
            assert!(prev_t < t);
        }
    }
}
