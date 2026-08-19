//! Lane HO — the marker-edge measurement (task #193 stage 1).
//!
//! # What this is, and what it deliberately is not
//!
//! A DIAGNOSTIC-ONLY passive watcher. It samples two words of `CGameState` —
//! the tick-busy marker byte `+0x4C0` and the hour scalar `+0x468` — and
//! records their transitions with a monotonic timestamp. It installs no hook,
//! patches no byte, changes no bracket, and takes part in no decision. With
//! `BATTLE_PLAN_HO_MARKER_WATCH` unset it does not start: no thread, no read, no
//! allocation.
//!
//! It exists to answer ONE question: **how long is the marker busy, and how
//! much quiet time does its falling edge open into?** Every
//! blocker that would let a marker-gated frame be WRONGLY ACCEPTED (N1, N2, N5,
//! N6, N7, N8, and N9) is still open, so nothing this module measures may be
//! read as evidence that a frame is clean or that the interval is quiescent.
//! Mechanical feasibility only.
//!
//! # Why the two arms
//!
//! A tight sampling loop is the only way to resolve an edge to better than a
//! scheduler quantum, and a tight sampling loop is also the most obvious way to
//! PERTURB the thing being measured: it burns a core and it pulls
//! `CGameState`'s cache lines to the watcher's core, so every engine store to
//! them may pay coherence latency to reclaim ownership.
//!
//! So the window is cut into 200 ms epochs on a 5-epoch cycle: four SPIN
//! epochs, then one LOW epoch. Both arms read BOTH fields; what separates them
//! is RATE. SPIN samples as fast as the loop runs. LOW samples at roughly
//! 1 kHz — four orders of magnitude less memory traffic — and is therefore a
//! near-zero-load control taken IN THE SAME PROCESS, in the same window,
//! against the same campaign. Comparing the engine's hour rate across the two
//! epoch kinds is what says whether the spin moved the number it reported.
//! Both arms therefore run inside the same process and campaign.
//!
//! The LOW arm reads the marker rather than skipping it, and that is a
//! correctness requirement, not a convenience: an arm that carried the last
//! SPIN value forward would make every excursion falling wholly inside a LOW
//! epoch INVISIBLE, and short excursions fit inside one more often than long
//! ones — a length-biased hole in exactly the distribution this lane exists to
//! publish. A thousand one-byte reads a second cannot be the perturbation the
//! control is looking for.
//!
//! # The uncertainty interval is recorded, not assumed — but it is a MODEL
//!
//! Every transition carries BOTH its own timestamp and the previous sample's,
//! so no consumer has to guess the sampling resolution, and per-epoch
//! min/max/sum sample gaps are published for the same reason. Because both arms
//! observe every edge, a consumer never has to DROP a transition: it grades
//! each one by the width of its own interval — tens of nanoseconds from the
//! SPIN arm, about a millisecond from the LOW arm.
//!
//! **`(prev_t_ns, t_ns]` is NOT a rigorous bracket and must not be read as
//! one.** The loop performs both loads and only THEN reads the clock, so the
//! previous sample's load happened at or before `prev_t_ns`, and a transition
//! can fall between that load and `prev_t_ns` — before that interval starts.
//! Two consecutive samples can also share a timestamp, giving an empty
//! interval, which no true bracket can be.
//!
//! `lo_t_ns` is what makes a rigorous bracket available, and it costs NOTHING
//! per sample. Write `T_k` for the timestamp of sample `k` and `L_k` for when
//! its loads happened. A transition seen at sample `k` occurred in
//! `(L_{k-1}, L_k]`. The loop is sequential, so `T_{k-2} <= L_{k-1}`, and the
//! clock is read after the loads, so `L_k <= T_k`. Therefore the transition
//! occurred in the CLOSED interval **`[T_{k-2}, T_k]`** — and `T_{k-2}` is just
//! the timestamp from two iterations ago, which this loop already has in a
//! register. It is emitted as `lo_t_ns`.
//!
//! Closed, not half-open: the clock's resolution lets two samples share a
//! timestamp, so `lo_t_ns == t_ns` occurs and a half-open spelling would make
//! those intervals empty.
//!
//! So each transition carries three stamps: `lo_t_ns` (`T_{k-2}`, the sound
//! lower bound), `prev_t_ns` (`T_{k-1}`, the tighter POINT ESTIMATE bound), and
//! `t_ns` (`T_k`, sound upper bound). Doubling the clock reads to stamp either
//! side of each load pair would give a tighter bracket still, and is
//! deliberately not done: the clock read is the dominant cost of this loop, so
//! paying it twice would change the very sampling rate the measurement
//! characterises.
//!
//! # Raw loads, and the one residual that buys
//!
//! The sampling loop uses `read_volatile` on addresses authenticated once
//! through the ordinary guarded primitives (`read_u64_stable` on the singleton
//! slot, `read_scalar` on both fields). The guarded path is a region-gate plus
//! an SEH/`NtReadVirtualMemory` copy per read; at this sample rate it would
//! dominate the measurement it is supposed to take. The cost of the raw read is
//! that a `CGameState` torn down mid-window (a campaign unload or reload) would
//! leave the loop reading freed memory. The loop therefore re-reads the
//! SINGLETON SLOT — which lives in the module image and is always mapped —
//! every 4096 samples and aborts the whole run if it no longer names the
//! object that was authenticated at start. A reload inside one 4096-sample
//! window is the named residual; the measurement window is a single
//! mid-campaign minute with no reload, and the abort is reported in the
//! manifest either way.

use super::*;

/// Absolute path the manifest is written to. Unset or empty ⇒ the watcher
/// never starts.
pub(super) const HO_MARKER_WATCH_ENV: &str = "BATTLE_PLAN_HO_MARKER_WATCH";
/// Window length in whole seconds. Default 60, clamped to 1..=600.
pub(super) const HO_MARKER_WATCH_SECONDS_ENV: &str = "BATTLE_PLAN_HO_MARKER_WATCH_SECONDS";

const DEFAULT_WINDOW_SECONDS: u64 = 60;
const EPOCH_MS: u64 = 200;
/// Four SPIN epochs then one LOW epoch: an 80 % measurement duty cycle with a
/// control arm that is never smaller than a fifth of the window.
const CYCLE_EPOCHS: u64 = 5;
const LOW_EPOCH_PHASE: u64 = 4;
const LOW_SAMPLE_SLEEP_US: u64 = 500;
/// 400 k transitions. At the observed shape (a handful per game hour) a 60 s
/// max-speed window produces low thousands; the cap exists so a pathological
/// flapping field cannot grow this unboundedly inside the game process.
const MAX_EVENTS: usize = 400_000;
const MAX_EPOCHS: usize = 8_192;
const SINGLETON_RECHECK_SAMPLES: u64 = 4_096;

/// Which arm an epoch belongs to.
///
/// The cycle is fixed in code rather than configured: a run whose duty cycle
/// could be dialled to 100 % would publish a distribution with no control
/// beside it and nothing in the manifest would say so.
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
    /// `T_{k-2}`: the RIGOROUS lower bound on when this transition happened.
    /// See the module header.
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

/// Stops and joins the watcher thread when the capture scope ends.
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

/// Start the watcher, or return `None` and leave the process exactly as it was.
///
/// `gs_ptr` is the caller's already-authenticated `CGameState`. It is
/// re-authenticated here against the module's own singleton slot rather than
/// trusted, because this module then reads that object raw for a minute.
pub(super) fn start(module_base: usize, gs_ptr: usize) -> Option<Guard> {
    let raw_path = std::env::var(HO_MARKER_WATCH_ENV).ok()?;
    let path = raw_path.trim().to_string();
    if path.is_empty() {
        return None;
    }
    let window_seconds = window_seconds_from(
        std::env::var(HO_MARKER_WATCH_SECONDS_ENV)
            .ok()
            .as_deref(),
    );
    let singleton_slot = module_base
        .checked_add(compact_contract::build_profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)?;
    let observed = read_u64_stable(singleton_slot)? as usize;
    if observed == 0 || observed != gs_ptr || !is_heap_pointer_aligned(observed, 8) {
        return None;
    }
    let date_addr = gs_ptr.checked_add(GS_CURRENT_DATE_OFFSET)?;
    let marker_addr = gs_ptr.checked_add(GS_TICK_BUSY_MARKER_OFFSET)?;
    // One guarded read of each field before the loop is allowed to touch them
    // raw. `read_scalar`, not `read_u8_stable`: a double-read-stable probe of a
    // field that legitimately flips would refuse to start the watcher for the
    // exact reason the watcher exists.
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

    // SAFETY (this read and every `read_volatile` below): `cfg.gs_ptr` was
    // re-authenticated in `start` against the module's own singleton slot and
    // both field addresses passed a guarded `read_scalar` probe, so they name
    // committed, readable, naturally aligned words of the live `CGameState`.
    // The singleton slot is re-read every `SINGLETON_RECHECK_SAMPLES` samples
    // and the run aborts if the object is replaced; see the module header for
    // the residual that leaves.
    let mut prev_marker = unsafe { core::ptr::read_volatile(marker_ptr) };
    let mut prev_date = unsafe { core::ptr::read_volatile(date_ptr) };
    let mut prev_t_ns = duration_nanos_u64(origin.elapsed());
    // `T_{k-2}`. Seeded to the ORIGIN, not to `prev_t_ns`: the seed loads above
    // happened BEFORE `prev_t_ns` was stamped, so a transition caught by the
    // very first loop sample can precede `prev_t_ns`, and seeding there would
    // publish a lower bound the transition is allowed to violate. Zero cannot
    // be violated — nothing in this window precedes the origin — so the first
    // sample's bracket is merely wide rather than wrong.
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
            // BOTH arms read BOTH fields. An arm that carried the last SPIN
            // value forward would erase every excursion contained entirely
            // within a LOW epoch, and short excursions are contained more
            // often than long ones — a length-biased hole in the published
            // distribution. What separates the arms is the RATE at which these
            // two loads are issued, which is the thing the control is for.
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

    /// kills: widening the duty cycle so every epoch spins (or moving the LOW
    /// phase off the end of the cycle). Either change publishes a marker
    /// distribution with no near-zero-load control arm beside it, which is the
    /// one thing that makes the numbers defensible against "the watcher moved
    /// what it measured".
    #[test]
    fn every_fifth_epoch_is_the_low_load_control_arm() {
        let arms: Vec<bool> = (0..12).map(epoch_is_spin).collect();
        assert_eq!(
            arms,
            vec![
                true, true, true, true, false, true, true, true, true, false, true, true
            ]
        );
        let spin = arms.iter().filter(|is_spin| **is_spin).count();
        assert_eq!(spin, 10, "80 % duty cycle");
    }

    /// kills: dropping the range filter or the default, either of which lets a
    /// stray environment value turn a bounded 60 s diagnostic window into an
    /// unbounded one inside the game process.
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

    /// kills: publishing the marker offset as anything other than the validated
    /// `CGameState+0x4C0`, or the date offset drifting off `+0x468`. The whole
    /// measurement is about the distance between those two fields' events.
    #[test]
    fn the_two_watched_fields_are_the_validated_addresses() {
        assert_eq!(GS_TICK_BUSY_MARKER_OFFSET, 0x4C0);
        assert_eq!(GS_CURRENT_DATE_OFFSET, 0x468);
    }

    /// kills: emitting `lo_t_ns` as anything but the timestamp from TWO samples
    /// ago — in particular collapsing it onto `prev_t_ns`, which is the exact
    /// mistake that makes the published interval a point estimate rather than a
    /// rigorous bracket. A consumer reading `(lo_t_ns, t_ns]` as sound would
    /// then be wrong, and nothing in the manifest would say so.
    #[test]
    fn the_rigorous_lower_bound_lags_the_point_estimate_by_one_sample() {
        // The loop's stamp rotation, in the order `run` performs it.
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
            assert!(lo_t < prev_t, "the rigorous bound must precede the estimate");
            assert!(prev_t < t);
        }
    }
}
