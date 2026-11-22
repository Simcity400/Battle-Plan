
use eframe::wgpu;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU8, Ordering},
};

const SLOTS: usize = 8;
const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;
const STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

pub type FrameTime = (u64, Result<f64, String>);

static REQUESTED: OnceLock<bool> = OnceLock::new();

pub fn requested() -> bool {
    *REQUESTED
        .get_or_init(|| std::env::var_os("BATTLEPLAN_PERF_MAP_LOG").or_else(|| std::env::var_os("HOI4_PERF_MAP_LOG")).is_some_and(|path| !path.is_empty()))
}

pub fn request() {
    let _ = REQUESTED.set(true);
}

pub fn features() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
}

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
    label: Option<u64>,
    open: Option<usize>,
    next: usize,
    untimed: Vec<FrameTime>,
}

impl FrameTimer {
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

    pub fn label(&mut self, frame: u64) {
        self.settle("it was never rendered");
        self.label = Some(frame);
    }

    fn settle(&mut self, why: &str) {
        if let Some(index) = self.open.take() {
            self.untimed
                .push((self.slots[index].frame, Err(why.into())));
        }
        if let Some(frame) = self.label.take() {
            self.untimed.push((frame, Err(why.into())));
        }
    }

    pub fn abandon(&mut self) {
        self.settle("its render failed before its submission");
    }

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

    pub fn take(&mut self, device: &wgpu::Device) -> Vec<FrameTime> {
        let _ = device.poll(wgpu::PollType::Poll);
        self.collect(false)
    }

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
