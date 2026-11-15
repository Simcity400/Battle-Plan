//! Per-pass GPU timestamps of a frame: a profiling aid under `BATTLE_PLAN_VIEWER_GPU_PASSES`, which
//! prints each frame's after waiting for it, and the view record's `gpu_passes` while a measurement
//! keeps that record ([`crate::view_record`]), which never waits: each frame's timestamps map back
//! on their own and are read frames later.
//!
//! [`crate::map::gpu_contract::device_descriptor`] asks for the timestamp features only in those
//! two cases, so an ordinary run never creates a query set. The scene writes a timestamp after
//! each group of passes. The backend may reorder commands between timestamps, so these are
//! attributions, not exact boundaries.

use crate::profile_writer::profile_print;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU8, Ordering},
};

use eframe::wgpu;

/// Timestamps one frame may write.
// Three wraps revisit every map-body pass and both mesh queues.
const MAX: u32 = 128;
/// Frames whose timestamps may be on their way back at once.
const SLOTS: usize = 4;
const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

/// Whether per-pass GPU timing is printed (the profiling aid).
pub fn requested() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_GPU_PASSES").or_else(|| std::env::var_os("HOI4_VIEWER_GPU_PASSES")).is_some())
}

/// Whether per-pass GPU timing goes to the view record.
pub fn recorded() -> bool {
    crate::view_record::on()
}

/// The device features per-pass timing needs.
pub fn features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
}

/// The view record's section for the per-pass GPU times, in microseconds.
pub const SECTION: &str = "gpu_passes";

struct Slot {
    read: wgpu::Buffer,
    labels: Vec<&'static str>,
    state: Arc<AtomicU8>,
    frame: u64,
}

pub struct PassTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    current: usize,
    /// This frame's marks, and whether its slot was free to take them.
    labels: Vec<&'static str>,
    active: bool,
    period: f32,
    /// The caller's number for the frame being timed ([`PassTimer::label`]).
    frame: u64,
    record: bool,
}

impl PassTimer {
    /// A timer when timing is printed or recorded and the device can write timestamps. A
    /// recording without the features says so in each frame's record ([`Self::unavailable`]).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let record = recorded();
        if !requested() && !record {
            return None;
        }
        if !device.features().contains(features()) {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                profile_print(
                    "PROFILE gpu_passes unavailable: the adapter lacks TIMESTAMP_QUERY_INSIDE_ENCODERS"
                        .into(),
                );
            });
            return None;
        }
        let bytes = u64::from(MAX) * 8;
        let slots = (0..SLOTS)
            .map(|_| Slot {
                read: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu pass readback"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                labels: Vec::new(),
                state: Arc::new(AtomicU8::new(IDLE)),
                frame: 0,
            })
            .collect();
        Some(Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu pass timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX * SLOTS as u32,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu pass resolve"),
                size: bytes * SLOTS as u64,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            slots,
            current: 0,
            labels: Vec::new(),
            active: false,
            period: queue.get_timestamp_period(),
            frame: 0,
            record,
        })
    }

    /// A frame recorded on a device without timestamps: its record names the GPU times as not
    /// recorded, never as zero.
    pub fn unavailable() {
        crate::view_record::set(SECTION, "not_recorded", 1);
    }

    /// Numbers the next report, so its line pairs with the caller's own
    /// per-frame line (the window's `PROFILE frame`, a dump's `BENCH frame`).
    pub fn label(&mut self, frame: u64) {
        self.frame = frame;
    }

    /// Marks the end of the passes named `label` (the first mark is the start). A frame whose
    /// slot still waits for an earlier frame's readback is not timed.
    pub fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        if self.labels.is_empty() {
            self.active = self.slots[self.current].state.load(Ordering::Acquire) == IDLE;
        }
        let index = self.labels.len() as u32;
        if self.active && index < MAX {
            encoder.write_timestamp(&self.set, self.current as u32 * MAX + index);
        }
        self.labels.push(label);
    }

    /// Forgets the marks of a frame whose encoder was dropped: they were never submitted.
    pub fn abandon(&mut self) {
        self.labels.clear();
    }

    /// Resolves the frame's timestamps into its slot's readback buffer.
    pub fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let count = (self.labels.len() as u32).min(MAX);
        if !self.active || count < 2 {
            return;
        }
        let base = self.current as u32 * MAX;
        let offset = u64::from(base) * 8;
        encoder.resolve_query_set(&self.set, base..base + count, &self.resolve, offset);
        encoder.copy_buffer_to_buffer(&self.resolve, offset, &self.slots[self.current].read, 0, u64::from(count) * 8);
    }

    /// After the frame's submission: printed, it waits for the frame and prints each group's
    /// milliseconds; recorded, its readback starts and every frame whose timestamps came back is
    /// written to the record, never waiting.
    pub fn report(&mut self, device: &wgpu::Device) {
        let mut labels = std::mem::take(&mut self.labels);
        labels.truncate(MAX as usize);
        if self.active && labels.len() >= 2 {
            let slot = &mut self.slots[self.current];
            slot.labels = labels;
            slot.frame = self.frame;
            let state = Arc::clone(&slot.state);
            state.store(MAPPING, Ordering::Release);
            slot.read.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                state.store(if result.is_ok() { READY } else { FAILED }, Ordering::Release);
            });
            self.current = (self.current + 1) % SLOTS;
        } else if self.record && !self.active {
            crate::view_record::add(SECTION, "frames_not_timed", 1);
        }
        self.frame += 1;
        let poll = if self.record {
            wgpu::PollType::Poll
        } else {
            wgpu::PollType::Wait { submission_index: None, timeout: None }
        };
        let _ = device.poll(poll);
        self.collect();
    }

    fn collect(&mut self) {
        for offset in 0..SLOTS {
            let slot = &mut self.slots[(self.current + offset) % SLOTS];
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let ticks: Option<Vec<u64>> = slot
                        .read
                        .slice(0..slot.labels.len() as u64 * 8)
                        .get_mapped_range()
                        .ok()
                        .map(|data| {
                            data.chunks_exact(8)
                                .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("eight bytes")))
                                .collect()
                        });
                    slot.read.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                    let labels = std::mem::take(&mut slot.labels);
                    let Some(ticks) = ticks else {
                        continue;
                    };
                    let (total, parts) = pass_times(&labels, &ticks, self.period);
                    if self.record {
                        use crate::view_record::set;
                        let us = |ms: f64| (ms * 1000.0).round() as u64;
                        set(SECTION, "total", us(total));
                        set(SECTION, "frames_ago", self.frame.saturating_sub(slot.frame));
                        for (label, ms) in parts {
                            set(SECTION, label, us(ms));
                        }
                    } else {
                        let parts = parts
                            .iter()
                            .map(|(label, ms)| format!("{label}={ms:.3}"))
                            .collect::<Vec<_>>()
                            .join(" ");
                        profile_print(format!("PROFILE gpu_passes frame {} total {total:.3} {parts}", slot.frame));
                    }
                }
                FAILED => {
                    slot.read.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                    slot.labels.clear();
                    if self.record {
                        crate::view_record::add(SECTION, "readbacks_failed", 1);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Each group's milliseconds between its mark and the one before, and the whole
/// frame's: `labels[i]` ends at `ticks[i]`, the first label being the start.
/// A backend that reorders commands can report a later timestamp earlier; that
/// group reads 0 rather than wrapping.
fn pass_times(
    labels: &[&'static str],
    ticks: &[u64],
    period: f32,
) -> (f64, Vec<(&'static str, f64)>) {
    let ms = |from: u64, to: u64| to.saturating_sub(from) as f64 * f64::from(period) / 1e6;
    let total = match (ticks.first(), ticks.last()) {
        (Some(first), Some(last)) => ms(*first, *last),
        _ => 0.0,
    };
    // A native world-wrap traversal revisits the same pass labels. Report each
    // group once, summing its copies in first-submission order.
    let mut parts: Vec<(&str, f64)> = Vec::new();
    for (label, pair) in labels.iter().skip(1).zip(ticks.windows(2)) {
        let elapsed = ms(pair[0], pair[1]);
        if let Some((_, value)) = parts.iter_mut().find(|(name, _)| name == label) {
            *value += elapsed;
        } else {
            parts.push((label, elapsed));
        }
    }
    (total, parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_copies_sum_without_losing_later_passes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (total, parts) = pass_times(
            &["start", "terrain", "water", "terrain", "water", "post"],
            &[0, 1_000, 3_000, 7_000, 15_000, 31_000],
            1_000.,
        );
        assert_eq!(total, 31.);
        assert_eq!(parts, vec![("terrain", 5.), ("water", 10.), ("post", 16.)]);
    }

    #[test]
    fn each_group_ends_at_its_own_mark_and_never_wraps() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let labels = ["start", "shadow", "terrain", "post"];
        // A 10 ns period: 1,000 ticks are 0.01 ms.
        let (total, parts) = pass_times(&labels, &[1_000, 101_000, 91_000, 301_000], 10.0);
        assert_eq!(parts[0], ("shadow", 1.0));
        // Reordered by the backend: an earlier tick reads 0, not u64 wrap.
        assert_eq!(parts[1], ("terrain", 0.0));
        assert_eq!(parts[2], ("post", 2.1));
        assert!((total - 3.0).abs() < 1e-9);
        assert_eq!(pass_times(&[], &[], 1.0), (0.0, Vec::new()));
    }
}
