
use eframe::wgpu;
use std::io;
use viewer_gpu::gpu_contract::Targets;

pub(crate) const COUNTS: usize = 11 + TILES;

pub(crate) const TILES: usize = 16 * 8;

pub(crate) const PARTS: [&str; 2] = ["world", "screen"];

const SEEN: u32 = 64;

pub(crate) struct Compare {
    kept: [wgpu::Texture; 2],
    pipeline: wgpu::ComputePipeline,
    counts: wgpu::Buffer,
    groups: [wgpu::BindGroup; 2],
    stride: u64,
    slots: usize,
    last: Option<u64>,
    sizes: [[u32; 2]; 2],
}

impl Compare {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &Targets,
        slots: usize,
    ) -> io::Result<Self> {
        shader_pipeline::gpu::stats::build();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("picture count"),
            source: wgpu::ShaderSource::Wgsl(include_str!("screen_count.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("picture count"),
            entries: &[
                texel_entry(0),
                texel_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: true,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("picture count"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("picture count"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("compare"),
            compilation_options: Default::default(),
            cache: None,
        });
        let alignment = u64::from(device.limits().min_storage_buffer_offset_alignment).max(4);
        let stride = (COUNTS as u64 * 4).next_multiple_of(alignment);
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture counts"),
            size: stride * (slots * PARTS.len()) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pictures = [targets.scene_texture(), targets.output()];
        let kept = [
            kept_like(device, pictures[0], "picture count: the world as it was"),
            kept_like(device, pictures[1], "picture count: the output as it was"),
        ];
        let groups = [
            group(device, &layout, pictures[0], &kept[0], &counts, stride),
            group(device, &layout, pictures[1], &kept[1], &counts, stride),
        ];
        let compare = Self {
            kept,
            pipeline,
            counts,
            groups,
            stride,
            slots,
            last: None,
            sizes: [targets.world_size(), targets.size()],
        };
        for slot in 0..slots {
            compare.start(queue, slot);
        }
        Ok(compare)
    }

    fn start(&self, queue: &wgpu::Queue, slot: usize) {
        let mut start = vec![0u8; COUNTS * 4];
        start[7 * 4..9 * 4].fill(0xff);
        for part in 0..PARTS.len() {
            queue.write_buffer(&self.counts, self.at(slot, part), &start);
        }
    }

    fn at(&self, slot: usize, part: usize) -> u64 {
        (slot * PARTS.len() + part) as u64 * self.stride
    }

    pub(crate) fn sizes(&self) -> [[u32; 2]; 2] {
        self.sizes
    }

    pub(crate) fn take(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &Targets,
        index: u64,
        slot: usize,
    ) -> bool {
        assert!(slot < self.slots, "a slot of this comparison");
        if self.sizes != [targets.world_size(), targets.size()] {
            self.last = None;
            return false;
        }
        let against = self.last == index.checked_sub(1);
        let pictures = [targets.scene_texture(), targets.output()];
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("picture count"),
        });
        for (part, picture) in pictures.iter().enumerate() {
            if against {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("picture count"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                let offset = u32::try_from(self.at(slot, part)).expect("the counts' own offset");
                pass.set_bind_group(0, &self.groups[part], &[offset]);
                let [width, height] = self.sizes[part];
                pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
            }
            encoder.copy_texture_to_texture(
                picture.as_image_copy(),
                self.kept[part].as_image_copy(),
                picture.size(),
            );
        }
        queue.submit([encoder.finish()]);
        self.last = Some(index);
        against
    }

    pub(crate) fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slots: usize,
    ) -> io::Result<Vec<[u32; COUNTS]>> {
        let bytes = self.stride * (slots * PARTS.len()) as u64;
        if bytes == 0 {
            return Ok(Vec::new());
        }
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("picture counts readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("picture counts readback"),
        });
        encoder.copy_buffer_to_buffer(&self.counts, 0, &read, 0, bytes);
        let submission = queue.submit([encoder.finish()]);
        let slice = read.slice(..);
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .map_err(io::Error::other)?;
        receive
            .recv_timeout(std::time::Duration::from_secs(60))
            .map_err(io::Error::other)?
            .map_err(io::Error::other)?;
        let mapped = slice.get_mapped_range().map_err(io::Error::other)?;
        let mut taken = Vec::with_capacity(slots * PARTS.len());
        for slot in 0..slots * PARTS.len() {
            let at = slot * self.stride as usize;
            let mut values = [0u32; COUNTS];
            for (value, bytes) in values
                .iter_mut()
                .zip(mapped[at..at + COUNTS * 4].chunks_exact(4))
            {
                *value = u32::from_le_bytes(bytes.try_into().expect("four bytes"));
            }
            taken.push(values);
        }
        drop(mapped);
        read.unmap();
        Ok(taken)
    }
}

fn texel_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn kept_like(device: &wgpu::Device, like: &wgpu::Texture, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: like.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: like.format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    now: &wgpu::Texture,
    was: &wgpu::Texture,
    counts: &wgpu::Buffer,
    stride: u64,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("picture count"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&now.create_view(&Default::default())),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&was.create_view(&Default::default())),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: counts,
                    offset: 0,
                    size: Some(std::num::NonZeroU64::new(stride).expect("a positive stride")),
                }),
            },
        ],
    })
}

pub(crate) mod readout {
    use super::{Compare, SEEN};
    use eframe::wgpu;
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    pub(crate) fn on() -> bool {
        static ON: std::sync::LazyLock<bool> =
            std::sync::LazyLock::new(|| std::env::var_os("BATTLEPLAN_VIEWER_SHOW_PICTURES").or_else(|| std::env::var_os("HOI4_VIEWER_SHOW_PICTURES")).is_some());
        *ON
    }

    static PRESENTED: AtomicU64 = AtomicU64::new(0);

    pub(crate) fn presented() {
        if on() {
            PRESENTED.fetch_add(1, Ordering::Relaxed);
        }
    }

    trait Moment: Copy + PartialOrd {
        fn earlier(self, by: Duration) -> Option<Self>;
    }

    impl Moment for Instant {
        fn earlier(self, by: Duration) -> Option<Self> {
            self.checked_sub(by)
        }
    }

    #[derive(Clone, Copy)]
    struct Frame<T = Instant> {
        at: T,
        differed: bool,
        seen: bool,
        presented: u64,
    }

    struct Readout {
        compare: Compare,
        read: Option<(wgpu::Buffer, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
        frames: Vec<Frame>,
        index: u64,
        waiting: bool,
    }

    thread_local! {
        static READOUT: RefCell<Option<Readout>> = const { RefCell::new(None) };
    }

    pub(crate) fn count(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &viewer_world::scene::Scene,
    ) {
        if !on() {
            return;
        }
        let Some(targets) = scene.targets() else {
            return;
        };
        READOUT.with_borrow_mut(|held| {
            let readout = match held {
                Some(readout) => readout,
                none => match Compare::new(device, queue, targets, 1) {
                    Ok(compare) => none.insert(Readout {
                        compare,
                        read: None,
                        frames: Vec::new(),
                        index: 0,
                        waiting: false,
                    }),
                    Err(_) => return,
                },
            };
            readout.collect(device);
            let index = readout.index;
            readout.index += 1;
            if readout.waiting {
                return;
            }
            if readout.compare.take(device, queue, targets, index, 0) {
                readout.ask(device, queue);
            }
        });
    }

    impl Readout {
        fn ask(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
            let bytes = u64::from(super::COUNTS as u32) * 4;
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("picture count readout"),
                size: bytes * super::PARTS.len() as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("picture count readout"),
            });
            for part in 0..super::PARTS.len() {
                encoder.copy_buffer_to_buffer(
                    &self.compare.counts,
                    self.compare.at(0, part),
                    &read,
                    bytes * part as u64,
                    bytes,
                );
            }
            queue.submit([encoder.finish()]);
            let arrived = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let fired = std::sync::Arc::clone(&arrived);
            read.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                fired.store(result.is_ok(), std::sync::atomic::Ordering::Release);
            });
            self.read = Some((read, arrived));
            self.waiting = true;
        }

        fn collect(&mut self, device: &wgpu::Device) {
            let Some((read, arrived)) = &self.read else {
                return;
            };
            let _ = device.poll(wgpu::PollType::Poll);
            if !arrived.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let screen = {
                let Ok(mapped) = read.slice(..).get_mapped_range() else {
                    self.read = None;
                    self.waiting = false;
                    return;
                };
                let at = super::COUNTS * 4;
                let value = |index: usize| {
                    u32::from_le_bytes(
                        mapped[at + index * 4..at + index * 4 + 4]
                            .try_into()
                            .expect("four bytes"),
                    )
                };
                [value(0), value(2)]
            };
            let (read, _) = self.read.take().expect("held above");
            read.unmap();
            drop(read);
            self.frames.push(Frame {
                at: Instant::now(),
                differed: screen[0] > 0,
                seen: screen[1] > SEEN,
                presented: PRESENTED.load(Ordering::Relaxed),
            });
            trim(&mut self.frames, Instant::now());
            self.waiting = false;
            self.compare.last = None;
        }
    }

    fn trim<T: Moment>(frames: &mut Vec<Frame<T>>, now: T) {
        if let Some(second) = now.earlier(Duration::from_secs(1)) {
            frames.retain(|kept| kept.at >= second);
        }
    }

    pub(crate) fn lines() -> Vec<String> {
        if !on() {
            return Vec::new();
        }
        READOUT.with_borrow(|held| {
            let Some(readout) = held else {
                return Vec::new();
            };
            let (first, last) = match (readout.frames.first(), readout.frames.last()) {
                (Some(first), Some(last)) if readout.frames.len() > 1 => (first, last),
                _ => return Vec::new(),
            };
            let seconds = last.at.duration_since(first.at).as_secs_f64().max(1e-9);
            let compared = (readout.frames.len() - 1) as f64;
            let presents = last.presented.saturating_sub(first.presented) as f64 / seconds;
            let differed = readout.frames.iter().skip(1).filter(|frame| frame.differed).count() as f64;
            let seen = readout.frames.iter().skip(1).filter(|frame| frame.seen).count() as f64;
            vec![
                format!("Presented a second: {presents:.0}"),
                format!(
                    "A new picture: {:.0}% of frames compared, a change a watcher could see {:.0}%",
                    100.0 * differed / compared,
                    100.0 * seen / compared
                ),
            ]
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        impl Moment for Duration {
            fn earlier(self, by: Duration) -> Option<Self> {
                self.checked_sub(by)
            }
        }

        #[test]
        fn frames_from_the_clock_origin_keep_the_last_second() {
            let mut frames = Vec::new();
            let mut kept = |at: u64, now: u64| {
                frames.push(Frame {
                    at: Duration::from_millis(at),
                    differed: false,
                    seen: false,
                    presented: at,
                });
                trim(&mut frames, Duration::from_millis(now));
                frames.iter().map(|frame| frame.presented).collect::<Vec<_>>()
            };
            assert_eq!(kept(0, 0), [0]);
            assert_eq!(kept(500, 500), [0, 500]);
            assert_eq!(kept(1_000, 1_000), [0, 500, 1_000]);
            assert_eq!(kept(1_500, 1_501), [1_000, 1_500]);
            assert_eq!(kept(2_500, 2_501), [2_500]);
        }
    }
}
