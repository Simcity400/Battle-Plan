//! Byte oracle: a small fallback-adapter check runs in the gate; large hardware sweeps are opt-in.
use super::*;
use gradient::{LayerKind, LayerPlane};
use std::sync::MutexGuard;
use std::{path::PathBuf, time::Instant};
use viewer_foundation::map::gpu_contract::device_descriptor;
use viewer_foundation::map::gpu_passes::PassTimer;
use viewer_foundation::map::test_support::serial;

#[test]
fn political_gpu_fallback_naval_attributes_change_shared_fill() {
    let (width, height) = (64, 32);
    let Some(harness) = Harness::fallback(width, height) else {
        return;
    };
    let ids = (0..width * height)
        .map(|i| {
            if i % width == 31 {
                0
            } else if i % width < 31 {
                1
            } else {
                2
            }
        })
        .collect::<Vec<_>>();
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let field = FieldDefines::TEST;
    let keys = [0, 1, 2];
    let base = Cache::default()
        .warm(
            &harness.device,
            &harness.queue,
            Inputs {
                map,
                strategic: &keys,
                field: &field,
            },
        )
        .unwrap();
    let tables = |value| {
        Arc::new(Regions {
            country_of: vec![0; 3],
            owner_of: Vec::new(),
            state_of: Vec::new(),
            country_colors: HashMap::new(),
            warnings: Vec::new(),
            strategic_attributes: [
                (
                    1,
                    gradient::Attributes {
                        control: value,
                        contested: 128,
                        gradient: None,
                    },
                ),
                (
                    2,
                    gradient::Attributes {
                        control: 128,
                        contested: 255,
                        gradient: Some(255),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        })
    };
    let mut a = View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let mut b = View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let one = tables(255);
    a.show(&harness.device, &harness.queue, &one).unwrap();
    b.show(&harness.device, &harness.queue, &tables(255))
        .unwrap();
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    let check = |view: &View, regions: &Regions| {
        let color = |_| field.navy_neutral;
        let attributes = |id| regions.strategic_attributes[&id];
        let mut layer = Layer::new(
            LayerKind::Fixed(field.strategic_thickness),
            &keys,
            &color,
            &field,
        );
        layer.region_opacity = 0;
        layer.no_region_color = field.navy_neutral;
        layer.region_attributes = Some(&attributes);
        let cpu = gradient::build_layer(&map, &layer).unwrap();
        let (dispatched, _, actual) = harness.view(view);
        compare(&cpu, &actual, "naval attributes");
        dispatched
    };
    assert!(check(&a, &one));
    assert!(!check(&b, &one));
    let two = tables(128);
    a.show(&harness.device, &harness.queue, &two).unwrap();
    assert!(!Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    assert!(check(&a, &two));
    assert!(!check(&b, &one));
}

#[test]
fn rounded_country_endpoint_bounds_the_search() {
    let field = FieldDefines {
        country_thickness: [-1_000_000.0, 253.9999],
        ..FieldDefines::TEST
    };
    let actual = gradient::country_thickness(u64::MAX, &field);
    assert!(actual > field.country_thickness[1]);
    assert_eq!(maximum_thickness(LayerKind::Country, &field), actual);
    assert_eq!(search_radius(actual, &field, 1024, 512), 4);
    assert_eq!(
        search_radius(field.country_thickness[1], &field, 1024, 512),
        3
    );
}

struct Harness {
    device: wgpu::Device,
    queue: wgpu::Queue,
    textures: [wgpu::Texture; 2],
    read: wgpu::Buffer,
    stride: u32,
    width: u32,
    rows: u32,
    timer: Option<PassTimer>,
    /// Held for the harness's life, and dropped after its device (last field).
    _serial: MutexGuard<'static, ()>,
}

impl Harness {
    fn new(width: u32, height: u32) -> Self {
        let serial = serial();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        println!("POLITICAL adapter {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&device_descriptor(&adapter))).unwrap();
        Self::with_device(serial, width, height, device, queue)
    }

    fn fallback(width: u32, height: u32) -> Option<Self> {
        // Taken first: a skip below drops its device before the guard.
        let serial = serial();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: true,
                ..Default::default()
            })) {
                Ok(adapter) => adapter,
                Err(error) => {
                    eprintln!("SKIP political fallback oracle: {error}");
                    return None;
                }
            };
        println!("POLITICAL fallback adapter {:?}", adapter.get_info());
        // A fallback adapter that cannot open the viewer's device, or lacks the compute the
        // kernel needs, is as absent as no adapter: the viewer would refuse it too.
        let (device, queue) =
            match pollster::block_on(adapter.request_device(&device_descriptor(&adapter))) {
                Ok(opened) => opened,
                Err(error) => {
                    eprintln!("SKIP political fallback oracle: {error}");
                    return None;
                }
            };
        if let Err(error) = check_device(&device) {
            eprintln!("SKIP political fallback oracle: {error}");
            return None;
        }
        Some(Self::with_device(serial, width, height, device, queue))
    }

    /// `serial` comes first: parameters drop in reverse order, so a panic here drops the device
    /// before the guard.
    fn with_device(
        serial: MutexGuard<'static, ()>,
        width: u32,
        height: u32,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Self {
        let width = width / 2;
        let rows = height / 2;
        let stride = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let textures = ["oracle channel 1", "oracle channel 2"].map(|label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height: rows,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("political oracle readback"),
            size: u64::from(stride) * u64::from(rows) * 2,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let timer = PassTimer::new(&device, &queue);
        Self {
            device,
            queue,
            textures,
            read,
            stride,
            width,
            rows,
            timer,
            _serial: serial,
        }
    }

    fn gpu(&mut self, gpu: &mut PoliticalGpu, layer: &Layer<'_>, frame: u64) -> (LayerPlane, f64) {
        let start = Instant::now();
        gpu.update(&self.device, &self.queue, layer).unwrap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Some(timer) = &mut self.timer {
            timer.label(frame);
            timer.mark(&mut encoder, "start");
        }
        gpu.encode(&mut encoder, [&self.textures[0], &self.textures[1]], 0);
        if let Some(timer) = &mut self.timer {
            timer.mark(&mut encoder, "political_compute");
            timer.finish(&mut encoder);
        }
        let cpu = start.elapsed().as_secs_f64() * 1000.0;
        let textures = self.textures.clone();
        let plane = self.read(encoder, [&textures[0], &textures[1]], 0);
        if let Some(timer) = &mut self.timer {
            timer.report(&self.device);
        }
        (plane, cpu)
    }

    /// Both channels of the layer starting at `origin_y`, after the encoder's own commands.
    fn read(
        &self,
        mut encoder: wgpu::CommandEncoder,
        textures: [&wgpu::Texture; 2],
        origin_y: u32,
    ) -> LayerPlane {
        for (i, texture) in textures.into_iter().enumerate() {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: origin_y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.read,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: i as u64 * u64::from(self.stride) * u64::from(self.rows),
                        bytes_per_row: Some(self.stride),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.rows,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit([encoder.finish()]);
        let slice = self.read.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = slice.get_mapped_range().unwrap();
        let mut channels = bytes
            .chunks_exact((self.stride * self.rows) as usize)
            .map(|plane| {
                plane
                    .chunks_exact(self.stride as usize)
                    .flat_map(|row| {
                        row[..self.width as usize * 4]
                            .chunks_exact(4)
                            .map(|texel| texel.try_into().unwrap())
                    })
                    .collect::<Vec<[u8; 4]>>()
            });
        let plane = LayerPlane {
            width: self.width,
            rows: self.rows,
            channel1: channels.next().unwrap(),
            channel2: channels.next().unwrap(),
        };
        drop(bytes);
        self.read.unmap();
        plane
    }

    /// Render one view's frame: whether it dispatched, then its country and strategic layers.
    fn view(&self, view: &View) -> (bool, LayerPlane, LayerPlane) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let dispatched = view.encode(&mut encoder);
        let [first, second] = view.textures().expect("shown").clone();
        let country = self.read(encoder, [&first, &second], first.height() / 2);
        let encoder = self.device.create_command_encoder(&Default::default());
        let strategic = self.read(encoder, [&first, &second], 0);
        (dispatched, country, strategic)
    }
}

fn compare(cpu: &LayerPlane, gpu: &LayerPlane, label: &str) {
    for (channel, (a, b)) in [&cpu.channel1, &cpu.channel2]
        .into_iter()
        .zip([&gpu.channel1, &gpu.channel2])
        .enumerate()
    {
        let count = a.iter().zip(b).filter(|(a, b)| a != b).count();
        let first = a.iter().zip(b).position(|(a, b)| a != b);
        assert_eq!(
            count,
            0,
            "{label} channel {channel}, first {:?}",
            first.map(|i| (i % cpu.width as usize, i / cpu.width as usize, a[i], b[i]))
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn political_gpu_fallback_exactness() {
    let (width, height) = (258, 130);
    let Some(mut harness) = Harness::fallback(width, height) else {
        return;
    };
    let ids = (0..width * height)
        .map(|i| {
            if i % width < 130 {
                70_000
            } else if i / width < 65 {
                1
            } else {
                2
            }
        })
        .collect::<Vec<_>>();
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let mut keys = vec![0; 70_001];
    keys[70_000] = 1;
    keys[1] = 2;
    let color = |key: u32| [(key * 51) as u8, (key * 83) as u8, (key * 11) as u8];
    for (thickness, center, reach) in [
        (5.0, 2.0, 30.0),
        (500.0, 2.0, 6000.0),
        (6000.0, -1.0, 7000.0),
        (0.0, 2.0, 30.0),
    ] {
        let field = FieldDefines {
            center,
            optimization_range: reach,
            ..FieldDefines::TEST
        };
        let kind = LayerKind::Fixed(thickness);
        let layer = Layer::new(kind, &keys, &color, &field);
        let mut gpu = PoliticalGpu::new(&harness.device, map, &layer).unwrap();
        let second = gpu.new_layer(&harness.device, &layer).unwrap();
        assert!(Arc::ptr_eq(&gpu.kernel, &second.kernel));
        drop(second);
        for hour in 0..4 {
            keys[1 + hour] = (hour as u32 + 2) % 5;
            let layer = Layer::new(kind, &keys, &color, &field);
            let cpu = gradient::build_layer(&map, &layer).unwrap();
            let (actual, _) = harness.gpu(&mut gpu, &layer, hour as u64);
            compare(&cpu, &actual, "fallback/mod defines");
            assert_eq!(gpu.alpha_keys.len(), 1, "one LUT per distinct thickness");
            let (actual, _) = harness.gpu(&mut gpu, &layer, hour as u64);
            compare(&cpu, &actual, "fallback/unchanged");
            assert_eq!(gpu.dispatched_cells, 0);
        }
    }

    let field = FieldDefines::TEST;
    let strategic_keys = keys.clone();
    let cache = Cache::default();
    let warm = || {
        cache
            .warm(
                &harness.device,
                &harness.queue,
                Inputs {
                    map,
                    strategic: &strategic_keys,
                    field: &field,
                },
            )
            .unwrap()
    };
    let base = warm();
    assert!(Arc::ptr_eq(&base, &warm()));
    let navy = |_| field.navy_neutral;
    let mut strategic_layer = Layer::new(
        LayerKind::Fixed(field.strategic_thickness),
        &strategic_keys,
        &navy,
        &field,
    );
    strategic_layer.region_opacity = 0;
    strategic_layer.no_region_color = field.navy_neutral;
    let strategic = gradient::build_layer(&map, &strategic_layer).unwrap();
    // Each call is a new table, as another view's builder would derive it.
    let hour = |key: u32| {
        let mut country_of = strategic_keys.clone();
        country_of[1] = key;
        Arc::new(super::super::political::Regions {
            country_colors: (0..6).map(|key| (key, color(key))).collect(),
            country_of,
            owner_of: Vec::new(),
            state_of: Vec::new(),
            warnings: Vec::new(),
            strategic_attributes: Default::default(),
        })
    };
    let check = |view: &View, regions: &Regions, dispatch: bool, label: &str| {
        let region_color = |key| regions.country_color(key);
        let layer = Layer::new(
            LayerKind::Country,
            &regions.country_of,
            &region_color,
            &field,
        );
        let cpu = gradient::build_layer(&map, &layer).unwrap();
        let (dispatched, country, first) = harness.view(view);
        assert_eq!(dispatched, dispatch, "{label}: dispatch");
        compare(&cpu, &country, label);
        compare(&strategic, &first, &format!("{label} strategic"));
    };
    let show = |view: &mut View, regions: &Arc<Regions>| {
        view.show(&harness.device, &harness.queue, regions).unwrap()
    };
    let new_view = || View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let (mut a, mut b) = (new_view(), new_view());
    // A view draws nothing of its own until its first fill.
    assert!(a.textures().is_none() && b.textures().is_none());
    let (one, two) = (hour(1), hour(2));
    assert!(show(&mut a, &one) && show(&mut b, &two));
    assert!(!Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    // A state costs its texture pair and the buffers it keeps; the dispatch scratch is shared.
    {
        let state = a.shown.as_ref().unwrap();
        let fill = state.lock();
        let gpu = &fill.gpu;
        assert!(Arc::ptr_eq(&gpu.scratch, &base.scratch));
        assert!(Arc::ptr_eq(
            &gpu.scratch,
            &b.shown.as_ref().unwrap().lock().gpu.scratch
        ));
        assert!(!Arc::ptr_eq(&base.scratch, &base.strategic.scratch));
        let cells = u64::from(gpu.params.columns) * u64::from(gpu.params.rows);
        let textures = 2 * u64::from(base.size[0]) * u64::from(base.size[1]) * 4;
        let expected = textures
            + cells * (36 + 4)
            + gpu.mapping.len() as u64 * 4
            + gpu.capacity as u64 * REGION_BYTES
            + gpu.alpha_buffer.size()
            + std::mem::size_of::<Params>() as u64
            + 12
            + fill.naval.own_bytes();
        drop(fill);
        assert_eq!(state.bytes(), expected);
        assert_eq!(a.state_bytes(), Some(expected));
        println!(
            "POLITICAL fallback state_bytes={expected} textures={textures} shared_scratch={}",
            base.scratch.bytes()
        );
    }
    check(&a, &one, true, "view a, first frame");
    check(&b, &two, true, "view b, first frame");
    assert!(!show(&mut a, &hour(1)) && !show(&mut b, &hour(2)));
    check(&a, &one, false, "view a, unchanged fill");
    check(&b, &two, false, "view b, unchanged fill");
    // The same fill from another view's tables (another perspective) shares a's state.
    let mut c = new_view();
    assert!(show(&mut c, &hour(1)));
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        c.shown.as_ref().unwrap()
    ));
    assert!(c.spare.is_none());
    check(&c, &one, false, "view c joins a");
    // c's unused spare waits in the pool: three states, the three views' spares.
    assert_eq!((base.live_states(), base.pooled()), (3, 1));
    // a leaves the shared state for the pooled one; c keeps the old fill.
    let three = hour(3);
    assert!(show(&mut a, &three));
    check(&a, &three, true, "view a splits");
    // No allocation: a new state would have left c's spare pooled, four live.
    assert_eq!((base.live_states(), base.pooled()), (3, 0));
    check(&c, &one, false, "view c keeps its fill");
    // b joins a; b's old state has no view left and is released.
    assert!(show(&mut b, &hour(3)));
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    check(&b, &three, false, "view b joins a");
    // b's old state (hour two) is pooled.
    assert_eq!((base.live_states(), base.pooled()), (3, 1));
    // A state only c holds follows c, incrementally, without growing the set.
    for key in [4, 5, 0] {
        let regions = hour(key);
        let before = Arc::as_ptr(c.shown.as_ref().unwrap());
        assert!(!show(&mut c, &regions));
        assert_eq!(Arc::as_ptr(c.shown.as_ref().unwrap()), before);
        check(&c, &regions, true, &format!("view c alone, key {key}"));
        assert_eq!((base.live_states(), base.pooled()), (3, 1));
    }
    // Back to hour two: c takes b's pooled state, which still holds that fill, and pools its own.
    let before = Arc::as_ptr(c.shown.as_ref().unwrap());
    assert!(show(&mut c, &hour(2)));
    assert_ne!(Arc::as_ptr(c.shown.as_ref().unwrap()), before);
    check(&c, &two, false, "view c finds a pooled fill");
    assert_eq!((base.live_states(), base.pooled()), (3, 1));
    drop((a, b, c));
    assert_eq!((base.live_states(), base.pooled()), (0, 0));

    // Thousands of fixed-thickness strategic regions still need only one LUT.
    let keys = (0..=5000).collect::<Vec<_>>();
    let ids = (0..width * height)
        .map(|i| 1 + i % 5000)
        .collect::<Vec<_>>();
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let layer = Layer::new(
        LayerKind::Fixed(field.strategic_thickness),
        &keys,
        &color,
        &field,
    );
    let mut gpu = PoliticalGpu::new(&harness.device, map, &layer).unwrap();
    let cpu = gradient::build_layer(&map, &layer).unwrap();
    let (actual, _) = harness.gpu(&mut gpu, &layer, 0);
    compare(&cpu, &actual, "5000 strategic regions");
    assert_eq!(gpu.alpha_keys.len(), 1);
    assert_eq!(gpu.alpha_buffer.size(), gpu.alpha_stride as u64);
}

#[test]
fn copies_reach_only_dirty_cells() {
    // Rows 0-1: columns 0-1 and 5 (merged down); row 2: column 5 (merged on); row 3: column 0
    // and columns 8-9, gapped, the last clipped to the texture width.
    let cells = [0, 1, 5, 10, 11, 15, 25, 38, 39, 30];
    let mut sorted = cells;
    sorted.sort_unstable();
    let rectangles = copy_rects(&sorted, 10, 76, 36);
    assert_eq!(
        rectangles,
        vec![
            [0, 0, 16, 16],
            [40, 0, 8, 24],
            [0, 24, 8, 8],
            [64, 24, 12, 8],
        ]
    );
}

fn grid_fixture(
    harness: &Harness,
    width: u32,
    height: u32,
    field: &FieldDefines,
) -> (Arc<Static>, Vec<u32>, Vec<u32>) {
    let columns = width / GRID_CELL;
    let ids = (0..width * height)
        .map(|i| 1 + i % width / GRID_CELL + i / width / GRID_CELL * columns)
        .collect::<Vec<_>>();
    let strategic = (0..=columns * (height / GRID_CELL))
        .map(|id| id % 3)
        .collect::<Vec<_>>();
    let base = Cache::default()
        .warm(
            &harness.device,
            &harness.queue,
            Inputs {
                map: ProvinceMap {
                    width,
                    height,
                    ids: &ids,
                },
                strategic: &strategic,
                field,
            },
        )
        .unwrap();
    (base, ids, strategic)
}

fn country_tables(country_of: Vec<u32>) -> Arc<Regions> {
    let color = |key: u32| [(key * 41) as u8, (key * 87) as u8, (key * 13) as u8];
    Arc::new(Regions {
        country_colors: (0..8).map(|key| (key, color(key))).collect(),
        country_of,
        owner_of: Vec::new(),
        state_of: Vec::new(),
        warnings: Vec::new(),
        strategic_attributes: Default::default(),
    })
}

/// Two states share one map's dispatch scratch, dispatching in one submission; the second
/// overwrote the whole output before the first's incremental update, whose rows have gaps.
#[test]
fn political_gpu_fallback_shared_scratch() {
    // Both countries stay above the area saturation threshold. Two interior
    // three-cell halos leave a column gap and an untouched bottom row.
    let (width, height) = (272, 128);
    let Some(harness) = Harness::fallback(width, height) else {
        return;
    };
    let field = FieldDefines::TEST;
    let (base, ids, _strategic) = grid_fixture(&harness, width, height, &field);
    let columns = width / GRID_CELL;
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let tables = country_tables;
    let halves = (0..=columns * (height / GRID_CELL))
        .map(|id| if id % columns < columns / 2 { 1 } else { 2 })
        .collect::<Vec<_>>();
    let mut changed = halves.clone();
    // Two provinces of one row, far apart: the dispatch's rows have a gap between them.
    changed[(1 + 3 * columns + 4) as usize] = 4;
    changed[(1 + 3 * columns + 12) as usize] = 5;
    let other = vec![3; halves.len()];
    let mut moved = other.clone();
    moved[(1 + 6 * columns + 8) as usize] = 6;
    let expect = |regions: &Regions| {
        let region_color = |key| regions.country_color(key);
        let layer = Layer::new(
            LayerKind::Country,
            &regions.country_of,
            &region_color,
            &field,
        );
        gradient::build_layer(&map, &layer).unwrap()
    };
    let new_view = || View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let (mut a, mut b) = (new_view(), new_view());
    let both = |a: &View, b: &View| {
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        assert!(a.encode(&mut encoder) && b.encode(&mut encoder));
        harness.queue.submit([encoder.finish()]);
    };
    for (first, second) in [
        (tables(halves), tables(other)),
        (tables(changed), tables(moved)),
    ] {
        a.show(&harness.device, &harness.queue, &first).unwrap();
        b.show(&harness.device, &harness.queue, &second).unwrap();
        both(&a, &b);
        for (view, regions, label) in [(&a, &first, "a"), (&b, &second, "b")] {
            let (dispatched, actual, _) = harness.view(view);
            assert!(!dispatched);
            compare(
                &expect(regions),
                &actual,
                &format!("shared scratch, view {label}"),
            );
        }
    }
    let fill = a.shown.as_ref().unwrap().lock();
    assert!(fill.gpu.dispatched_cells < fill.gpu.dirty.len());
    let rows = fill
        .gpu
        .copies
        .iter()
        .map(|rect| rect[1])
        .collect::<Vec<_>>();
    assert!(
        rows.iter()
            .any(|y| rows.iter().filter(|other| *other == y).count() > 1),
        "the incremental update must copy a row with a gap: {:?}",
        fill.gpu.copies
    );
}

/// Two maps stepping through hours together: after warm-up, no state is allocated, the pair
/// alternates between two states, and each update is incremental and exact.
#[test]
fn political_gpu_fallback_lockstep() {
    // 200 cells leave two changed seven-cell halos below half the map.
    let (width, height) = (400, 128);
    let Some(harness) = Harness::fallback(width, height) else {
        return;
    };
    let field = FieldDefines::TEST;
    let (base, ids, _strategic) = grid_fixture(&harness, width, height, &field);
    let columns = width / GRID_CELL;
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    // Each map's builder derives its own tables for the same hour.
    let tables = |country_of: &[u32]| country_tables(country_of.to_vec());
    let mut keys = (0..=columns * (height / GRID_CELL))
        .map(|id| if id % columns < columns / 2 { 1 } else { 2 })
        .collect::<Vec<_>>();
    let new_view = || View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let (mut a, mut b) = (new_view(), new_view());
    let cells = (columns * height / GRID_CELL) as usize;
    // The two warm-up spares are the only states ever: both stay alive and no third is live.
    let spares = [&a, &b].map(|view| Arc::downgrade(view.spare.as_ref().unwrap()));
    let mut used = [false; 2];
    // Two warm-up steps, then reuse each of the two states once.
    for step in 0..4 {
        let province = 1 + (step * 97) % (keys.len() - 1);
        keys[province] = (step % 5 + 3) as u32;
        let (first, second) = (tables(&keys), tables(&keys));
        a.show(&harness.device, &harness.queue, &first).unwrap();
        b.show(&harness.device, &harness.queue, &second).unwrap();
        let shown = a.shown.as_ref().unwrap();
        assert!(Arc::ptr_eq(shown, b.shown.as_ref().unwrap()));
        let index = spares
            .iter()
            .position(|spare| std::ptr::eq(spare.as_ptr(), Arc::as_ptr(shown)))
            .expect("a warm-up state");
        used[index] = true;
        let dispatched = shown.lock().gpu.dispatched_cells;
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        assert!(a.encode(&mut encoder) && !b.encode(&mut encoder));
        harness.queue.submit([encoder.finish()]);
        let region_color = |key| first.country_color(key);
        let layer = Layer::new(LayerKind::Country, &first.country_of, &region_color, &field);
        let cpu = gradient::build_layer(&map, &layer).unwrap();
        for (view, label) in [(&a, "a"), (&b, "b")] {
            let (again, actual, _) = harness.view(view);
            assert!(!again);
            compare(
                &cpu,
                &actual,
                &format!("lockstep step {step}, view {label}"),
            );
        }
        println!("POLITICAL lockstep step={step} dispatched={dispatched}/{cells}");
        assert!(
            spares.iter().all(|spare| spare.strong_count() != 0),
            "step {step}"
        );
        assert_eq!(base.live_states(), 2, "step {step}");
        if step >= 2 {
            // The reused state diffs from two hours back: two changed provinces' halos.
            assert!(dispatched < cells / 2, "step {step}: {dispatched}/{cells}");
        }
    }
    assert_eq!(used, [true, true]);
}

/// The two interleavings the pool must exclude, each in both orders, with the registry checked
/// to be held from another thread at the point where they used to interleave.
#[test]
fn political_gpu_fallback_pool_interleavings() {
    // One province suffices: these are registry interleavings, with no pixel oracle.
    let (width, height) = (16, 16);
    let Some(harness) = Harness::fallback(width, height) else {
        return;
    };
    let field = FieldDefines::TEST;
    let (base, _ids, strategic) = grid_fixture(&harness, width, height, &field);
    let hour = |key: u32| {
        let mut country_of = vec![1; strategic.len()];
        country_of[1] = key;
        country_tables(country_of)
    };
    let checks = || base.race_checks.load(std::sync::atomic::Ordering::Relaxed);
    let (device, queue) = (&harness.device, &harness.queue);
    let new_view = || View::new(Arc::clone(&base), device, queue).unwrap();
    let (mut x, mut y) = (new_view(), new_view());
    let (one, two) = (hour(1), hour(2));
    x.show(device, queue, &one).unwrap();
    y.show(device, queue, &two).unwrap();
    // x joins y; x's state, still holding hour one, is pooled.
    x.show(device, queue, &two).unwrap();
    assert_eq!(base.pooled(), 1);
    let shows = |state: &Arc<State>| state.lock().shown.as_ref().map(|shown| shown.country_of[1]);

    // (a) find, then recycle: find takes the pooled state out whole, still holding its fill.
    let before = checks();
    let found = base.find(&one).expect("the pooled fill");
    assert!(checks() > before);
    assert_eq!(shows(&found), Some(1));
    assert!(base.recycle().is_none());
    base.release(found);
    assert_eq!(base.pooled(), 1);
    // (a) recycle, then find: the recycled state is forgotten before any find can see it.
    let recycled = base.recycle().expect("the pooled state");
    assert_eq!(shows(&recycled), None);
    assert!(base.find(&one).is_none());

    // (b) find, then release: the found state has a holder, so it is not pooled.
    {
        let mut fill = recycled.lock();
        fill.gpu.update_regions(device, queue, &hour(3)).unwrap();
        fill.shown = Some(hour(3));
    }
    let before = checks();
    let found = base.find(&hour(3)).expect("the updated state");
    base.release(recycled);
    assert_eq!(base.pooled(), 0);
    // (b) release, then find: the state is pooled only while nobody holds it, and find takes
    // it back out.
    base.release(found);
    assert!(checks() > before);
    assert_eq!(base.pooled(), 1);
    let found = base.find(&hour(3)).expect("the pooled state");
    assert_eq!(base.pooled(), 0);
    assert!(base.recycle().is_none());
    drop(found);

    // A view's own state is claimed under the lock: no other view finds it mid-update.
    let own = x.shown.as_ref().unwrap();
    assert!(!base.claim(own), "x shares y's state");
    y.show(device, queue, &hour(4)).unwrap();
    let own = x.shown.as_ref().unwrap();
    assert!(base.claim(own));
    assert!(base.find(&two).is_none());
}

#[test]
#[ignore = "requires a compute adapter; runs the byte oracle through hardware"]
fn political_gpu_synthetic() {
    for (width, height, high_ids) in [(514, 258, false), (130, 70, true), (6, 2, false)] {
        let ids = (0..width * height)
            .map(|i| {
                let x = i % width;
                let y = i / width;
                let id = 1 + (x / 17 + y / 19 * 7) % 23;
                if high_ids && id == 23 { 70_000 } else { id }
            })
            .collect::<Vec<_>>();
        let map = ProvinceMap {
            width,
            height,
            ids: &ids,
        };
        let mut keys = vec![0; ids.iter().copied().max().unwrap() as usize + 1];
        for &id in &ids {
            keys[id as usize] = id % 7;
        }
        let mut colors = (0..9)
            .map(|i| [(i * 37) as u8, (i * 91) as u8, (i * 13) as u8])
            .collect::<Vec<_>>();
        let field = FieldDefines::TEST;
        let mut harness = Harness::new(width, height);
        for kind in [
            LayerKind::Country,
            LayerKind::Fixed(field.strategic_thickness),
        ] {
            let color = |key: u32| colors[key as usize];
            let layer = Layer::new(kind, &keys, &color, &field);
            let mut gpu = PoliticalGpu::new(&harness.device, map, &layer).unwrap();
            for hour in 0..80 {
                if hour == 25 {
                    keys.fill(0);
                } else if hour == 26 {
                    keys.fill(1);
                } else {
                    let id = ids[(hour * 293) % ids.len()] as usize;
                    keys[id] = (hour % 8) as u32;
                    if hour % 5 == 0 {
                        colors[hour % 8][hour % 3] = (hour * 97) as u8;
                    }
                }
                let color = |key: u32| colors[key as usize];
                let layer = Layer::new(kind, &keys, &color, &field);
                let cpu = gradient::build_layer(&map, &layer).unwrap();
                if hour % 17 == 0 {
                    gpu = PoliticalGpu::new(&harness.device, map, &layer).unwrap();
                }
                let (actual, _) = harness.gpu(&mut gpu, &layer, hour as u64);
                compare(
                    &cpu,
                    &actual,
                    &format!("{width}x{height} {kind:?} hour {hour}"),
                );
            }
        }
    }
}

#[test]
#[ignore = "requires a compute adapter; exercises sparse incremental dispatch and queued states"]
fn political_gpu_incremental() {
    let (width, height) = (1024, 512);
    let columns = width / GRID_CELL;
    let ids = (0..width * height)
        .map(|i| 1 + i % width / GRID_CELL + i / width / GRID_CELL * columns)
        .collect::<Vec<_>>();
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let mut keys = (0..=columns * (height / GRID_CELL))
        .map(|id| if id % columns < columns / 2 { 1 } else { 2 })
        .collect::<Vec<_>>();
    let field = FieldDefines::TEST;
    let color = |key: u32| [(key * 41) as u8, (key * 87) as u8, (key * 13) as u8];
    let layer = Layer::new(LayerKind::Country, &keys, &color, &field);
    let mut harness = Harness::new(width, height);
    let mut gpu = PoliticalGpu::new(&harness.device, map, &layer).unwrap();
    let mut incremental = 0;
    for hour in 0..60 {
        let id = 1 + (hour * 137) % (keys.len() - 1);
        keys[id] = (hour % 5) as u32;
        let layer = Layer::new(LayerKind::Country, &keys, &color, &field);
        if hour % 7 == 0 {
            gpu.update(&harness.device, &harness.queue, &layer).unwrap();
        }
        let cpu = gradient::build_layer(&map, &layer).unwrap();
        if hour == 30 {
            gpu = PoliticalGpu::new(&harness.device, map, &layer).unwrap();
        }
        let (actual, _) = harness.gpu(&mut gpu, &layer, hour as u64);
        compare(&cpu, &actual, &format!("incremental hour {hour}"));
        if gpu.dispatched_cells > 0 && gpu.dispatched_cells < gpu.dirty.len() {
            incremental += 1;
        }
        let (same, _) = harness.gpu(&mut gpu, &layer, hour as u64);
        assert_eq!(
            gpu.dispatched_cells, 0,
            "unchanged states must dispatch nothing"
        );
        compare(&cpu, &same, "unchanged");
    }
    assert!(incremental > 30, "only {incremental} sparse dispatches");
}

/// Uses the recorded content stack; no local colour or province catalog is embedded in fixtures.
#[test]
#[ignore = "set BATTLE_PLAN_POLITICAL_REPLAY, BATTLE_PLAN_POLITICAL_GAME_DIR and BATTLE_PLAN_POLITICAL_USER_DIR"]
fn political_gpu_recording() {
    let path =
        PathBuf::from(std::env::var_os("BATTLE_PLAN_POLITICAL_REPLAY").or_else(|| std::env::var_os("HOI4_POLITICAL_REPLAY")).expect("BATTLE_PLAN_POLITICAL_REPLAY"));
    let replay = replay::container::Replay::open(std::fs::read(&path).unwrap()).unwrap();
    let options = installed_content::ResolveOptions {
        game_dir: PathBuf::from(
            std::env::var_os("BATTLE_PLAN_POLITICAL_GAME_DIR").or_else(|| std::env::var_os("HOI4_POLITICAL_GAME_DIR")).expect("BATTLE_PLAN_POLITICAL_GAME_DIR"),
        ),
        user_dir: PathBuf::from(
            std::env::var_os("BATTLE_PLAN_POLITICAL_USER_DIR").or_else(|| std::env::var_os("HOI4_POLITICAL_USER_DIR")).expect("BATTLE_PLAN_POLITICAL_USER_DIR"),
        ),
        cache_dir: path.parent().unwrap().join("political-content-cache"),
    };
    let content = installed_content::resolve(&replay.identity, &options).unwrap();
    let map = installed_content::map::load(&content.layers).unwrap();
    let field = FieldDefines::load(&installed_content::defines::read(&content.layers).unwrap()).unwrap();
    let strategic = installed_content::regions::read(&content.layers).unwrap().regions;
    let bitmap = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let mut harness = Harness::new(map.width, map.height);
    let keys = super::super::political::strategic_regions_of(&map, &strategic).unwrap();
    let color = |_| field.navy_neutral;
    let mut layer = Layer::new(
        LayerKind::Fixed(field.strategic_thickness),
        &keys,
        &color,
        &field,
    );
    layer.no_region_color = field.navy_neutral;
    layer.region_opacity = 0;
    let mut static_gpu = PoliticalGpu::new(&harness.device, bitmap, &layer).unwrap();
    let strategic = gradient::build_layer(&bitmap, &layer).unwrap();
    let (actual, _) = harness.gpu(&mut static_gpu, &layer, 0);
    compare(&strategic, &actual, "strategic");
    let mut static_gpu = Some(static_gpu);
    let mut gpu = None;
    let mut total = 0;
    let step = std::env::var("BATTLE_PLAN_POLITICAL_STEP").or_else(|_| std::env::var("HOI4_POLITICAL_STEP")).map_or(1, |value| {
        value
            .parse::<usize>()
            .expect("positive BATTLE_PLAN_POLITICAL_STEP")
    });
    assert!(step > 0, "BATTLE_PLAN_POLITICAL_STEP must be positive");
    use replay::{
        frame::FrameDecoder,
        rows::{TableId, tables},
    };
    // Keep full wire validation, but materialize only the political operands and campaign identity.
    let selected = [
        TableId::Campaign,
        TableId::Provinces,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Subjects,
        TableId::Autonomy,
        TableId::CountryAppearance,
    ];
    let mut previous_branch = None;
    let mut ordinal = 0;
    for index in &replay.index {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for encoded in replay.read_block_selected(index, &selected).unwrap() {
            let frame = decoder.advance(&encoded).unwrap();
            let campaign = frame.complete::<tables::Campaign>().unwrap();
            assert_eq!(campaign.len(), 1);
            assert_eq!(campaign[0].id, replay.campaign_id);
            let branch = frame.branch;
            let hour = frame.hour;
            let reset = previous_branch != Some(branch);
            if !reset && ordinal % step != 0 {
                ordinal += 1;
                continue;
            }
            ordinal += 1;
            previous_branch = Some(branch);
            let start = Instant::now();
            let regions = super::super::political::regions(&map, frame).unwrap();
            let tables_ms = start.elapsed().as_secs_f64() * 1000.0;
            let color = |key| regions.country_color(key);
            let layer = Layer::new(LayerKind::Country, &regions.country_of, &color, &field);
            if total == 0 {
                let base = Cache::default()
                    .warm(
                        &harness.device,
                        &harness.queue,
                        Inputs {
                            map: bitmap,
                            strategic: &keys,
                            field: &field,
                        },
                    )
                    .unwrap();
                let mut view =
                    View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
                view.show(&harness.device, &harness.queue, &Arc::new(regions.clone()))
                    .unwrap();
                println!(
                    "POLITICAL state_bytes={} shared_scratch_bytes={}",
                    view.state_bytes().unwrap(),
                    base.scratch.bytes()
                );
            }
            let gpu = gpu.get_or_insert_with(|| {
                static_gpu
                    .take()
                    .unwrap()
                    .new_layer(&harness.device, &layer)
                    .unwrap()
            });
            let oracle = || {
                let start = Instant::now();
                let plane = gradient::build_layer(&bitmap, &layer).unwrap();
                let stacked = gradient::stack(&strategic, &plane).unwrap();
                std::hint::black_box(&stacked);
                (plane, start.elapsed().as_secs_f64() * 1000.0)
            };
            // Alternate execution order in the same process and on exactly the same inputs.
            let (cpu, actual) = if total % 2 == 0 {
                let cpu = oracle();
                (cpu, harness.gpu(gpu, &layer, total))
            } else {
                let actual = harness.gpu(gpu, &layer, total);
                (oracle(), actual)
            };
            compare(&cpu.0, &actual.0, &format!("branch {branch} hour {hour}"));
            println!(
                "POLITICAL hour={hour} branch={branch} old_field_stack_ms={:.6} tables_ms={tables_ms:.6} new_update_encode_ms={:.6} cells={} bytes_equal={}",
                cpu.1,
                actual.1,
                gpu.dispatched_cells,
                cpu.0.channel1.len() * 8
            );
            total += 1;
        }
    }
    println!(
        "POLITICAL verified_hours={total} recording={}",
        path.display()
    );
    assert!(total > 0);
}
