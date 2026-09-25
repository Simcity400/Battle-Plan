//! Per-pass GPU timestamps of one frame, a profiling aid under
//! `BATTLE_PLAN_VIEWER_GPU_PASSES`.
//!
//! [`crate::render::device_descriptor`] asks for the timestamp features only
//! when the variable is set, so an ordinary run never creates a query set and
//! never waits for one. The scene writes a timestamp after each group of
//! passes; reading them back waits for the frame's submission, so a timed run
//! measures the GPU's own per-pass cost, not the pipelined frame rate. The
//! backend may reorder commands between timestamps, so these are attributions,
//! not exact boundaries.

use std::sync::OnceLock;

use eframe::wgpu;

/// Timestamps one frame may write.
const MAX: u32 = 48;

/// Whether per-pass GPU timing is requested.
pub fn requested() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BATTLE_PLAN_VIEWER_GPU_PASSES").or_else(|| std::env::var_os("HOI4_VIEWER_GPU_PASSES")).is_some())
}

/// The device features per-pass timing needs.
pub fn features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
}

pub struct PassTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    labels: Vec<&'static str>,
    period: f32,
    /// The caller's number for the frame being timed ([`PassTimer::label`]).
    frame: u64,
}

impl PassTimer {
    /// A timer when timing is requested and the device can write timestamps.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !requested() {
            return None;
        }
        if !device.features().contains(features()) {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                super::counter_move::profile_print(
                    "PROFILE gpu_passes unavailable: the adapter lacks TIMESTAMP_QUERY_INSIDE_ENCODERS"
                        .into(),
                );
            });
            return None;
        }
        let bytes = u64::from(MAX) * 8;
        Some(Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu pass timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: MAX,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu pass resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu pass readback"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            labels: Vec::new(),
            period: queue.get_timestamp_period(),
            frame: 0,
        })
    }

    /// Numbers the next report, so its line pairs with the caller's own
    /// per-frame line (the window's `PROFILE frame`, a dump's `BENCH frame`).
    pub fn label(&mut self, frame: u64) {
        self.frame = frame;
    }

    /// Marks the end of the passes named `label` (the first mark is the start).
    pub fn mark(&mut self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        let index = self.labels.len() as u32;
        if index < MAX {
            encoder.write_timestamp(&self.set, index);
            self.labels.push(label);
        }
    }

    /// Resolves the frame's timestamps into the readback buffer.
    pub fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let count = self.labels.len() as u32;
        if count < 2 {
            return;
        }
        encoder.resolve_query_set(&self.set, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, u64::from(count) * 8);
    }

    /// Waits for the submitted frame and prints each group's milliseconds.
    pub fn report(&mut self, device: &wgpu::Device) {
        let labels = std::mem::take(&mut self.labels);
        if labels.len() < 2 {
            return;
        }
        let bytes = labels.len() as u64 * 8;
        let slice = self.read.slice(0..bytes);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        if device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .is_err()
        {
            self.read.unmap();
            return;
        }
        let ticks: Vec<u64> = {
            let Ok(data) = slice.get_mapped_range() else {
                self.read.unmap();
                return;
            };
            data.chunks_exact(8)
                .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("eight bytes")))
                .collect()
        };
        self.read.unmap();
        let (total, parts) = pass_times(&labels, &ticks, self.period);
        let parts = parts
            .iter()
            .map(|(label, ms)| format!("{label}={ms:.3}"))
            .collect::<Vec<_>>()
            .join(" ");
        super::counter_move::profile_print(format!(
            "PROFILE gpu_passes frame {} total {total:.3} {parts}",
            self.frame
        ));
        self.frame += 1;
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
    let parts = labels
        .iter()
        .skip(1)
        .zip(ticks.windows(2))
        .map(|(label, pair)| (*label, ms(pair[0], pair[1])))
        .collect();
    (total, parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_group_ends_at_its_own_mark_and_never_wraps() {
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
