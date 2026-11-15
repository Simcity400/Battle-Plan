//! Each window map frame's GPU time, for the perf map log (`BATTLE_PLAN_PERF_MAP_LOG`).
//!
//! Timestamps bracket the commands of the frame's one submission: a start buffer submitted first
//! and an end written last in the frame's own encoder. The queue's own uploads run ahead of the
//! start, outside the time. The readback maps asynchronously and is collected frames later, so
//! no frame waits for its own.

use eframe::wgpu;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU8, Ordering},
};

/// Frames whose timestamps may be on their way back at once.
const SLOTS: usize = 8;
const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;
/// Each slot's place in the resolve buffer, as query resolves must be aligned.
const STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

/// A frame's GPU milliseconds, or why it has none.
pub type FrameTime = (u64, Result<f64, String>);

static REQUESTED: OnceLock<bool> = OnceLock::new();

/// Whether this process writes the perf map log, whose `map` records carry the GPU time.
pub fn requested() -> bool {
    // The variable the viewer's perf log opens its file from, unless its host named the file.
    *REQUESTED
        .get_or_init(|| std::env::var_os("BATTLE_PLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")).is_some_and(|path| !path.is_empty()))
}

/// The host writes the perf map log: its maps' devices time their frames.
pub fn request() {
    let _ = REQUESTED.set(true);
}

/// The device features frame timing needs.
pub fn features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
}

/// Whether `device` can time frames, or why it cannot.
pub fn support(device: &wgpu::Device) -> Result<(), String> {
    let missing = features() - device.features();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("the map device lacks {missing:?}"))
    }
}

struct Slot {
    read: wgpu::Buffer,
    state: Arc<AtomicU8>,
    frame: u64,
}

pub struct FrameTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    period: f64,
    /// The caller's number for the next frame ([`FrameTimer::label`]).
    label: Option<u64>,
    /// The slot of the frame begun and not yet submitted.
    open: Option<usize>,
    next: usize,
    /// Frames that will get no time, reported with the next results.
    untimed: Vec<FrameTime>,
}

impl FrameTimer {
    /// A timer when the perf log is written and `device` can write timestamps.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !requested() || support(device).is_err() {
            return None;
        }
        let slots = (0..SLOTS)
            .map(|_| Slot {
                read: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gpu frame time readback"),
                    size: 16,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
                frame: 0,
            })
            .collect();
        Some(Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu frame time"),
                ty: wgpu::QueryType::Timestamp,
                count: 2 * SLOTS as u32,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu frame time resolve"),
                size: STRIDE * SLOTS as u64,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            slots,
            period: f64::from(queue.get_timestamp_period()),
            label: None,
            open: None,
            next: 0,
            untimed: Vec::new(),
        })
    }

    /// Numbers the next frame [`FrameTimer::begin`] starts.
    pub fn label(&mut self, frame: u64) {
        self.settle("it was never rendered");
        self.label = Some(frame);
    }

    /// Reports the labelled frame that will get no time, begun or not, as `why`.
    fn settle(&mut self, why: &str) {
        if let Some(index) = self.open.take() {
            self.untimed
                .push((self.slots[index].frame, Err(why.into())));
        }
        if let Some(frame) = self.label.take() {
            self.untimed.push((frame, Err(why.into())));
        }
    }

    /// The labelled frame's render failed: it is reported without a time.
    pub fn abandon(&mut self) {
        self.settle("its render failed before its submission");
    }

    /// The buffer that starts timing the labelled frame, submitted ahead of its others. None
    /// for an unlabelled frame, or while the slot it would take still waits for its readback.
    pub fn begin(&mut self, device: &wgpu::Device) -> Option<wgpu::CommandBuffer> {
        if let Some(index) = self.open.take() {
            self.untimed.push((
                self.slots[index].frame,
                Err("its render failed before its submission".into()),
            ));
        }
        let frame = self.label.take()?;
        let index = self.next;
        if self.slots[index].state.load(Ordering::Acquire) != IDLE {
            self.untimed.push((
                frame,
                Err(format!("all {SLOTS} readback slots were still in use")),
            ));
            return None;
        }
        self.next = (index + 1) % SLOTS;
        self.slots[index].frame = frame;
        self.open = Some(index);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu frame time start"),
        });
        encoder.write_timestamp(&self.set, 2 * index as u32);
        Some(encoder.finish())
    }

    /// Ends the begun frame's timing in its last encoder, before that encoder finishes.
    pub fn end(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(index) = self.open else {
            return;
        };
        let first = 2 * index as u32;
        encoder.write_timestamp(&self.set, first + 1);
        encoder.resolve_query_set(
            &self.set,
            first..first + 2,
            &self.resolve,
            STRIDE * index as u64,
        );
        encoder.copy_buffer_to_buffer(
            &self.resolve,
            STRIDE * index as u64,
            &self.slots[index].read,
            0,
            16,
        );
    }

    /// The begun frame was submitted: its timestamps map back when the GPU is done.
    pub fn submitted(&mut self) {
        let Some(index) = self.open.take() else {
            return;
        };
        let state = Arc::clone(&self.slots[index].state);
        state.store(MAPPING, Ordering::Release);
        self.slots[index]
            .read
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { READY } else { FAILED },
                    Ordering::Release,
                );
            });
    }

    /// Each labelled frame settled since the last call, oldest first: its GPU milliseconds, or
    /// why it has none. Never waits for the GPU.
    pub fn take(&mut self, device: &wgpu::Device) -> Vec<FrameTime> {
        let _ = device.poll(wgpu::PollType::Poll);
        self.collect(false)
    }

    /// Every labelled frame not yet reported, as its scene closes: those whose timestamps came
    /// back with their time, the rest named. Never waits for the GPU.
    pub fn retire(&mut self) -> Vec<FrameTime> {
        self.settle("its scene closed before it was submitted");
        self.collect(true)
    }

    fn collect(&mut self, closing: bool) -> Vec<FrameTime> {
        let mut done = std::mem::take(&mut self.untimed);
        for offset in 0..SLOTS {
            let slot = &self.slots[(self.next + offset) % SLOTS];
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let ticks = slot.read.slice(..).get_mapped_range().ok().map(|view| {
                        let tick = |at: usize| {
                            u64::from_le_bytes(view[at..at + 8].try_into().expect("eight bytes"))
                        };
                        [tick(0), tick(8)]
                    });
                    slot.read.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                    done.push((
                        slot.frame,
                        match ticks {
                            Some([start, end]) if end >= start => {
                                Ok((end - start) as f64 * self.period / 1e6)
                            }
                            Some(_) => Err("the GPU wrote its end before its start".into()),
                            None => Err("its timestamps could not be read".into()),
                        },
                    ));
                }
                FAILED => {
                    // A failed map still holds the buffer's mapping until it is unmapped.
                    slot.read.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                    done.push((slot.frame, Err("its timestamp readback failed".into())));
                }
                MAPPING if closing => {
                    slot.state.store(IDLE, Ordering::Release);
                    done.push((
                        slot.frame,
                        Err("its scene closed before its timestamps came back".into()),
                    ));
                }
                _ => {}
            }
        }
        done
    }
}
