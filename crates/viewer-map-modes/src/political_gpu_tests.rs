use super::*;
use gradient::{LayerKind, LayerPlane};
use std::sync::MutexGuard;
use std::time::Instant;
use viewer_gpu::gpu_contract::device_descriptor;
use viewer_gpu::gpu_passes::PassTimer;
use viewer_gpu::test_support::serial;

#[path = "political_gpu/membership_gpu_tests.rs"]
mod membership_gpu_tests;

fn local_strategic_styles_split_shared_views_and_rejoin_after_clearing(fallback: &Fallback) {
    let harness = fallback.harness(64, 32);
    let ids: Vec<_> = (0..64 * 32)
        .map(|at| if at % 64 < 32 { 1 } else { 2 })
        .collect();
    let keys = [0, 1, 2];
    let base = Cache::default()
        .warm(
            &harness.device,
            &harness.queue,
            Inputs {
                map: ProvinceMap {
                    width: 64,
                    height: 32,
                    ids: &ids,
                },
                strategic: &keys,
                field: &FieldDefines::TEST,
            },
        )
        .unwrap();
    let regions = Arc::new(Regions {
        country_of: vec![0; 3].into(),
        owner_of: Vec::new().into(),
        state_of: Vec::new().into(),
        country_colors: Default::default(),
        warnings: Vec::new(),
        strategic_attributes: BTreeMap::from([
            (
                1,
                map_display_data::landscape::Attributes {
                    control: 27,
                    contested: 171,
                    gradient: None,
                },
            ),
            (
                2,
                map_display_data::landscape::Attributes {
                    control: 91,
                    contested: 63,
                    gradient: None,
                },
            ),
        ]),
        mode: None,
    });
    let mut a = View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let mut b = View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    a.show(&harness.device, &harness.queue, &regions).unwrap();
    b.show(&harness.device, &harness.queue, &regions).unwrap();
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    let selected = BTreeMap::from([(
        1,
        Style {
            color: [17, 231, 43, 127],
            pulse: false,
            fill: 1.0,
        },
    )]);
    let expected = |styles: &BTreeMap<u32, Style>| {
        let field = FieldDefines::TEST;
        let color = |key| {
            styles.get(&key).map_or(field.navy_neutral, |style| {
                [style.color[0], style.color[1], style.color[2]]
            })
        };
        let style = |key| {
            styles
                .get(&key)
                .map_or(gradient::RegionStyle::plain(0), |style| {
                    gradient::RegionStyle {
                        alpha: style.color[3],
                        pulse: style.pulse,
                        fill: style.fill,
                        thickness: None,
                    }
                })
        };
        let mut layer = Layer::new(
            LayerKind::Fixed(field.strategic_thickness),
            &keys,
            &color,
            &field,
        );
        let attributes = |key| {
            regions
                .strategic_attributes
                .get(&key)
                .copied()
                .unwrap_or_default()
        };
        layer.region_opacity = 0;
        layer.no_region_color = field.navy_neutral;
        layer.region_style = Some(&style);
        layer.region_attributes = Some(&attributes);
        gradient::build_layer(
            &ProvinceMap {
                width: 64,
                height: 32,
                ids: &ids,
            },
            &layer,
        )
        .unwrap()
    };
    assert!(
        a.set_strategic_styles(&harness.device, &harness.queue, &selected)
            .unwrap()
    );
    assert!(!Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap().lock().shown.as_ref().unwrap(),
        &regions
    ));
    let (_, _, a_pixels) = harness.view(&a);
    let (_, _, b_pixels) = harness.view(&b);
    compare(
        &expected(&selected),
        &a_pixels,
        "local selected strategic style",
    );
    compare(
        &expected(&BTreeMap::new()),
        &b_pixels,
        "other view remains unselected",
    );
    assert_ne!(a_pixels.channel1, b_pixels.channel1);
    assert_ne!(a_pixels.channel2, b_pixels.channel2);
    assert!(
        a_pixels
            .channel2
            .iter()
            .zip(&b_pixels.channel2)
            .all(|(a, b)| [a[0], a[2], a[3]] == [b[0], b[2], b[3]])
    );
    let (_, _, unchanged) = harness.view(&b);
    compare(
        &b_pixels,
        &unchanged,
        "second view keeps both texture channels",
    );
    assert!(
        !a.set_strategic_styles(&harness.device, &harness.queue, &selected)
            .unwrap()
    );
    assert!(
        b.set_strategic_styles(&harness.device, &harness.queue, &selected)
            .unwrap()
    );
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    let mut c = View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    assert!(
        !c.set_strategic_styles(&harness.device, &harness.queue, &selected)
            .unwrap()
    );
    c.show(&harness.device, &harness.queue, &regions).unwrap();
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        c.shown.as_ref().unwrap()
    ));
    a.set_strategic_styles(&harness.device, &harness.queue, &BTreeMap::new())
        .unwrap();
    b.set_strategic_styles(&harness.device, &harness.queue, &BTreeMap::new())
        .unwrap();
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    let (_, _, restored) = harness.view(&a);
    compare(
        &b_pixels,
        &restored,
        "clearing selection restores both channels",
    );
    let (_, _, still_selected) = harness.view(&c);
    compare(
        &a_pixels,
        &still_selected,
        "cleared views leave selected peer untouched",
    );
}

fn province_colors_keep_dirty_tiles_until_submission_and_clear_old_selection(fallback: &Fallback) {
    use std::collections::BTreeMap;
    let harness = fallback.harness(128, 64);
    for second in [2, 65_537] {
        let ids: Vec<_> = (0..64 * 32)
            .map(|at| if at % 32 < 16 { 1 } else { second })
            .collect();
        let keys = vec![0; second as usize + 1];
        let cache = Cache::default();
        let source = cache
            .warm(
                &harness.device,
                &harness.queue,
                Inputs {
                    map: ProvinceMap {
                        width: 64,
                        height: 32,
                        ids: &ids,
                    },
                    strategic: &keys,
                    field: &FieldDefines::TEST,
                },
            )
            .unwrap();
        let mut pass = ProvinceColors::new(&harness.device, &source).unwrap();
        let red = [255, 0, 0, 127];
        let blue = [0, 0, 255, 255];
        let green = [0, 255, 0, 64];
        pass.update(&harness.queue, &BTreeMap::from([(1, red), (second, blue)]))
            .unwrap();
        let mut aborted = harness.device.create_command_encoder(&Default::default());
        assert!(pass.encode(&mut aborted));
        drop(aborted);
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        assert!(pass.encode(&mut encoder));
        let got = harness.read(encoder, [&pass.bound.texture; 2], 0);
        pass.submitted();
        for (at, color) in got.channel1.iter().enumerate() {
            assert_eq!(*color, if at % 32 < 16 { red } else { blue });
        }
        let mut idle = harness.device.create_command_encoder(&Default::default());
        assert!(
            !pass.encode(&mut idle),
            "submitted stripes are no longer pending"
        );
        pass.update(&harness.queue, &BTreeMap::from([(second, green)]))
            .unwrap();
        pass.update(&harness.queue, &BTreeMap::from([(1, red), (second, green)]))
            .unwrap();
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        pass.encode(&mut encoder);
        let got = harness.read(encoder, [&pass.bound.texture; 2], 0);
        pass.submitted();
        for (at, color) in got.channel1.iter().enumerate() {
            assert_eq!(*color, if at % 32 < 16 { red } else { green });
        }
        pass.update(&harness.queue, &BTreeMap::new()).unwrap();
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        pass.encode(&mut encoder);
        let got = harness.read(encoder, [&pass.bound.texture; 2], 0);
        pass.submitted();
        assert!(got.channel1.iter().all(|color| *color == [0; 4]));
    }
}

fn political_gpu_fallback_naval_attributes_change_shared_fill(fallback: &Fallback) {
    let (width, height) = (64, 32);
    let harness = fallback.harness(width, height);
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
            country_of: vec![0; 3].into(),
            owner_of: Vec::new().into(),
            state_of: Vec::new().into(),
            country_colors: Default::default(),
            warnings: Vec::new(),
            strategic_attributes: [
                (
                    1,
                    map_display_data::landscape::Attributes {
                        control: value,
                        contested: 128,
                        gradient: None,
                    },
                ),
                (
                    2,
                    map_display_data::landscape::Attributes {
                        control: 128,
                        contested: 255,
                        gradient: Some(255),
                    },
                ),
            ]
            .into_iter()
            .collect(),
            mode: None,
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
    _serial: Option<MutexGuard<'static, ()>>,
}

struct Fallback {
    device: wgpu::Device,
    queue: wgpu::Queue,
    _serial: MutexGuard<'static, ()>,
}

impl Fallback {
    fn open() -> Option<Self> {
        let serial = serial();
        let instance = shader_pipeline::gpu::instance();
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
        Some(Self {
            device,
            queue,
            _serial: serial,
        })
    }

    fn harness(&self, width: u32, height: u32) -> Harness {
        Harness::with_device(None, width, height, self.device.clone(), self.queue.clone())
    }
}

#[test]
fn political_gpu_fallback_oracles() {
    let Some(fallback) = Fallback::open() else {
        return;
    };
    local_strategic_styles_split_shared_views_and_rejoin_after_clearing(&fallback);
    province_colors_keep_dirty_tiles_until_submission_and_clear_old_selection(&fallback);
    political_gpu_fallback_naval_attributes_change_shared_fill(&fallback);
    political_gpu_fallback_exactness(&fallback);
    political_gpu_fallback_shared_scratch(&fallback);
    political_gpu_fallback_lockstep(&fallback);
    political_gpu_fallback_pool_interleavings(&fallback);
    political_gpu_fallback_named_changes_equal_the_whole_update(&fallback);
    membership_gpu_tests::retained_membership_keeps_exact_bytes_areas_and_refusals_across_hours(
        &fallback,
    );
}

impl Harness {
    fn new(width: u32, height: u32) -> Self {
        let serial = serial();
        let instance = shader_pipeline::gpu::instance();
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .unwrap();
        println!("POLITICAL adapter {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&device_descriptor(&adapter))).unwrap();
        Self::with_device(Some(serial), width, height, device, queue)
    }

    fn with_device(
        serial: Option<MutexGuard<'static, ()>>,
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
        gpu.recorded(true);
        if let Some(timer) = &mut self.timer {
            timer.report(&self.device);
        }
        (plane, cpu)
    }

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

    fn view(&self, view: &View) -> (bool, LayerPlane, LayerPlane) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let encoded = view.encode(&mut encoder);
        let dispatched = encoded.is_some();
        let [first, second] = view.textures().expect("shown").clone();
        let country = self.read(encoder, [&first, &second], first.height() / 2);
        if let Some(encoded) = encoded {
            encoded.submitted();
        }
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

fn political_gpu_fallback_exactness(fallback: &Fallback) {
    let (width, height) = (258, 130);
    let mut harness = fallback.harness(width, height);
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
    let hour = |key: u32| {
        let mut country_of = strategic_keys.clone();
        country_of[1] = key;
        Arc::new(map_display_data::landscape::Regions {
            country_colors: (0..6)
                .try_fold(
                    Default::default(),
                    |tree: map_display_data::value_index::ValueIndex<_, _>, key| {
                        tree.inserted(key, color(key))
                    },
                )
                .unwrap(),
            country_of: country_of.into(),
            owner_of: Vec::new().into(),
            state_of: Vec::new().into(),
            warnings: Vec::new(),
            strategic_attributes: Default::default(),
            mode: None,
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
    assert!(a.textures().is_none() && b.textures().is_none());
    let (one, two) = (hour(1), hour(2));
    assert!(show(&mut a, &one) && show(&mut b, &two));
    assert!(!Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
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
            + fill.naval.own_bytes()
            + fill.mode.own_bytes();
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
    let mut c = new_view();
    assert!(show(&mut c, &hour(1)));
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        c.shown.as_ref().unwrap()
    ));
    assert!(c.spare.is_none());
    check(&c, &one, false, "view c joins a");
    assert_eq!((base.live_states(), base.pooled()), (3, 1));
    let three = hour(3);
    assert!(show(&mut a, &three));
    check(&a, &three, true, "view a splits");
    assert_eq!((base.live_states(), base.pooled()), (3, 0));
    check(&c, &one, false, "view c keeps its fill");
    assert!(show(&mut b, &hour(3)));
    assert!(Arc::ptr_eq(
        a.shown.as_ref().unwrap(),
        b.shown.as_ref().unwrap()
    ));
    check(&b, &three, false, "view b joins a");
    assert_eq!((base.live_states(), base.pooled()), (3, 1));
    for key in [4, 5, 0] {
        let regions = hour(key);
        let before = Arc::as_ptr(c.shown.as_ref().unwrap());
        assert!(!show(&mut c, &regions));
        assert_eq!(Arc::as_ptr(c.shown.as_ref().unwrap()), before);
        check(&c, &regions, true, &format!("view c alone, key {key}"));
        assert_eq!((base.live_states(), base.pooled()), (3, 1));
    }
    let before = Arc::as_ptr(c.shown.as_ref().unwrap());
    assert!(show(&mut c, &hour(2)));
    assert_ne!(Arc::as_ptr(c.shown.as_ref().unwrap()), before);
    check(&c, &two, false, "view c finds a pooled fill");
    assert_eq!((base.live_states(), base.pooled()), (3, 1));
    drop((a, b, c));
    assert_eq!((base.live_states(), base.pooled()), (0, 0));

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
        country_colors: (0..8)
            .try_fold(
                Default::default(),
                |tree: map_display_data::value_index::ValueIndex<_, _>, key| {
                    tree.inserted(key, color(key))
                },
            )
            .unwrap(),
        country_of: country_of.into(),
        owner_of: Vec::new().into(),
        state_of: Vec::new().into(),
        warnings: Vec::new(),
        strategic_attributes: Default::default(),
        mode: None,
    })
}

fn political_gpu_fallback_shared_scratch(fallback: &Fallback) {
    let (width, height) = (272, 128);
    let harness = fallback.harness(width, height);
    let field = FieldDefines::TEST;
    let (base, ids, strategic) = grid_fixture(&harness, width, height, &field);
    let columns = width / GRID_CELL;
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let navy = |_| field.navy_neutral;
    let mut naval_layer = Layer::new(LayerKind::Fixed(field.strategic_thickness), &strategic, &navy, &field);
    naval_layer.region_opacity = 0;
    naval_layer.no_region_color = field.navy_neutral;
    let naval = gradient::build_layer(&map, &naval_layer).unwrap();
    let tables = country_tables;
    let halves = (0..=columns * (height / GRID_CELL))
        .map(|id| if id % columns < columns / 2 { 1 } else { 2 })
        .collect::<Vec<_>>();
    let mut changed = halves.clone();
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
        let (first, second) = (a.encode(&mut encoder), b.encode(&mut encoder));
        assert!(first.is_some() && second.is_some());
        harness.queue.submit([encoder.finish()]);
        first.into_iter().chain(second).for_each(Encoded::submitted);
    };
    for (round, (first, second)) in [
        (tables(halves), tables(other)),
        (tables(changed), tables(moved)),
    ]
    .into_iter()
    .enumerate()
    {
        a.show(&harness.device, &harness.queue, &first).unwrap();
        b.show(&harness.device, &harness.queue, &second).unwrap();
        if round == 0 {
            for view in [&a, &b] {
                assert_eq!(view.shown.as_ref().unwrap().lock().naval.dispatched_cells, 0);
            }
        }
        both(&a, &b);
        for (view, regions, label) in [(&a, &first, "a"), (&b, &second, "b")] {
            let (dispatched, actual, sea) = harness.view(view);
            assert!(!dispatched);
            compare(
                &expect(regions),
                &actual,
                &format!("shared scratch, view {label}"),
            );
            compare(&naval, &sea, &format!("shared scratch, view {label}'s sea"));
        }
    }
    let air = Draw { channels: [Source::Strategic, Source::Empty], strategic: None, country: None, layer: None, second: None, political: [0.0; 2] };
    let in_air = Arc::new(Regions { mode: Some(Arc::new(air)), ..(*tables(vec![3; (columns * (height / GRID_CELL) + 1) as usize])).clone() });
    let mut c = new_view();
    c.show(&harness.device, &harness.queue, &in_air).unwrap();
    assert_eq!(c.shown.as_ref().unwrap().lock().naval.dispatched_cells, 0, "the sea stood in its half: nothing to redraw");
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

fn political_gpu_fallback_lockstep(fallback: &Fallback) {
    let (width, height) = (400, 128);
    let harness = fallback.harness(width, height);
    let field = FieldDefines::TEST;
    let (base, ids, _strategic) = grid_fixture(&harness, width, height, &field);
    let columns = width / GRID_CELL;
    let map = ProvinceMap {
        width,
        height,
        ids: &ids,
    };
    let tables = |country_of: &[u32]| country_tables(country_of.to_vec());
    let mut keys = (0..=columns * (height / GRID_CELL))
        .map(|id| if id % columns < columns / 2 { 1 } else { 2 })
        .collect::<Vec<_>>();
    let new_view = || View::new(Arc::clone(&base), &harness.device, &harness.queue).unwrap();
    let (mut a, mut b) = (new_view(), new_view());
    let cells = (columns * height / GRID_CELL) as usize;
    let spares = [&a, &b].map(|view| Arc::downgrade(view.spare.as_ref().unwrap()));
    let mut used = [false; 2];
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
        let mut aborted = harness.device.create_command_encoder(&Default::default());
        drop(a.encode(&mut aborted).expect("pending"));
        drop(aborted);
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        let recorded = a.encode(&mut encoder).expect("still pending");
        assert!(b.encode(&mut encoder).is_none());
        harness.queue.submit([encoder.finish()]);
        recorded.submitted();
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
            assert!(dispatched < cells / 2, "step {step}: {dispatched}/{cells}");
        }
    }
    assert_eq!(used, [true, true]);
}

fn political_gpu_fallback_pool_interleavings(fallback: &Fallback) {
    let (width, height) = (16, 16);
    let harness = fallback.harness(width, height);
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
    x.show(device, queue, &two).unwrap();
    assert_eq!(base.pooled(), 1);
    let shows = |state: &Arc<State>| state.lock().shown.as_ref().map(|shown| shown.country_of[1]);

    let before = checks();
    let found = base.find(&one, &BTreeMap::new()).expect("the pooled fill");
    assert!(checks() > before);
    assert_eq!(shows(&found), Some(1));
    assert!(base.recycle().is_none());
    base.release(found);
    assert_eq!(base.pooled(), 1);
    let recycled = base.recycle().expect("the pooled state");
    assert_eq!(shows(&recycled), None);
    assert!(base.find(&one, &BTreeMap::new()).is_none());

    {
        let mut fill = recycled.lock();
        fill.gpu.update_regions(device, queue, &hour(3)).unwrap();
        fill.shown = Some(hour(3));
    }
    let before = checks();
    let found = base
        .find(&hour(3), &BTreeMap::new())
        .expect("the updated state");
    base.release(recycled);
    assert_eq!(base.pooled(), 0);
    base.release(found);
    assert!(checks() > before);
    assert_eq!(base.pooled(), 1);
    let found = base
        .find(&hour(3), &BTreeMap::new())
        .expect("the pooled state");
    assert_eq!(base.pooled(), 0);
    assert!(base.recycle().is_none());
    drop(found);

    let own = x.shown.as_ref().unwrap();
    assert!(!base.claim(own), "x shares y's state");
    y.show(device, queue, &hour(4)).unwrap();
    let own = x.shown.as_ref().unwrap();
    assert!(base.claim(own));
    assert!(base.find(&two, &BTreeMap::new()).is_none());
}

#[test]
fn political_fill_shader_validates() {
    viewer_scene_base::shader_validation::validate(
        "political_fill",
        include_str!("political_fill.wgsl"),
    );
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

#[test]
fn a_second_mode_layer_takes_the_country_layers_place() {
    assert_eq!(
        holders([Source::Mode, Source::Second]),
        [Held::Mode, Held::Country]
    );
    assert_eq!(
        holders([Source::Mode, Source::Mode]),
        [Held::Mode, Held::Mode]
    );
    assert_eq!(holders(POLITICAL), [Held::Naval, Held::Country]);
}

#[test]
fn political_gpu_decoded_frames() {
    use super::super::political::{regions, strategic_regions_of, test_support as fixtures};
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{FrameDecoder, FrameEncoder, TableView};

    let map = fixtures::map();
    let bitmap = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let field = FieldDefines::TEST;
    let keys = strategic_regions_of(&map, &fixtures::strategic_regions()).unwrap();
    let color = |_| field.navy_neutral;
    let mut layer = Layer::new(LayerKind::Fixed(field.strategic_thickness), &keys, &color, &field);
    layer.no_region_color = field.navy_neutral;
    layer.region_opacity = 0;
    let mut harness = Harness::new(map.width, map.height);
    let mut gpu = PoliticalGpu::new(&harness.device, bitmap, &layer).unwrap();
    let strategic = gradient::build_layer(&bitmap, &layer).unwrap();
    let (actual, _) = harness.gpu(&mut gpu, &layer, 0);
    compare(&strategic, &actual, "strategic");

    let mut source = fixtures::occupied();
    source.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut gpu = Some(gpu);
    let mut country = None;
    for (step, (controller, country_key)) in [(200, 2), (100, 1), (200, 2)].into_iter().enumerate() {
        source.hour = 42 + step as i32;
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *source.tables[TableId::Provinces.index()]
        else {
            unreachable!();
        };
        rows[2].controller = controller;
        let encoded = encoder.encode(source.clone(), step == 0).unwrap();
        let frame = decoder.advance(&encoded).unwrap();
        let regions = regions(&map, frame).unwrap();
        assert_eq!(regions.country_of[3], country_key, "recorded hour {}", frame.hour);
        let color = |key| regions.country_color(key);
        let layer = Layer::new(LayerKind::Country, &regions.country_of, &color, &field);
        let country = country.get_or_insert_with(|| {
            gpu.take().unwrap().new_layer(&harness.device, &layer).unwrap()
        });
        let expected = gradient::build_layer(&bitmap, &layer).unwrap();
        let (actual, _) = harness.gpu(country, &layer, step as u64 + 1);
        compare(&expected, &actual, &format!("recorded hour {}", frame.hour));
    }
}

fn political_gpu_fallback_named_changes_equal_the_whole_update(fallback: &Fallback) {
    let (width, height) = (272, 128);
    let harness = fallback.harness(width, height);
    let field = FieldDefines::TEST;
    let columns = width / GRID_CELL;
    let ids = (0..width * height)
        .map(|i| 1 + i % width / GRID_CELL + i / width / GRID_CELL * columns)
        .collect::<Vec<_>>();
    let map = ProvinceMap { width, height, ids: &ids };
    let mut keys = (0..=columns * (height / GRID_CELL))
        .map(|id| match id % columns {
            0 => 0,
            x if x < columns / 2 => 1,
            x if x < columns - 3 => 2,
            x => 3 + x % 2,
        })
        .collect::<Vec<_>>();
    let mut colors = (0..12u32).map(|key| [(key * 41) as u8, (key * 87 + 5) as u8, (key * 13 + 9) as u8]).collect::<Vec<_>>();
    let mut gpu = {
        let color = |key: u32| colors[key as usize];
        PoliticalGpu::new(&harness.device, map, &Layer::new(LayerKind::Country, &keys, &color, &field)).unwrap()
    };
    let draw = |gpu: &mut PoliticalGpu, submitted: bool| {
        let mut encoder = harness.device.create_command_encoder(&Default::default());
        gpu.encode(&mut encoder, [&harness.textures[0], &harness.textures[1]], 0);
        if !submitted {
            drop(encoder);
            gpu.recorded(false);
            return None;
        }
        let plane = harness.read(encoder, [&harness.textures[0], &harness.textures[1]], 0);
        gpu.recorded(true);
        Some(plane)
    };
    let row = columns as usize;
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let (mut partial, mut provinces, mut recolored) = (0, Vec::new(), Vec::new());
    for frame in 0..40 {
        let held = keys.clone();
        match frame {
            12 | 20 => {
                let (from, to, past) = if frame == 12 { (1, 2, 3) } else { (2, 1, row / 2 + 2) };
                for id in (1..keys.len()).filter(|&id| held[id] == from && (id - 1) % row > past) {
                    keys[id] = to;
                }
            }
            _ => {
                for _ in 0..1 + next(5) {
                    let id = 1 + next(keys.len() - 1);
                    keys[id] = next(12) as u32;
                }
            }
        }
        if frame % 4 == 3 {
            let key = 1 + next(11);
            colors[key][frame % 3] = colors[key][frame % 3].wrapping_add(97);
            recolored.push(key as u32);
        }
        provinces.extend((0..keys.len()).filter(|&id| keys[id] != held[id]));
        {
            let color = |key: u32| colors[key as usize];
            let layer = Layer::new(LayerKind::Country, &keys, &color, &field);
            gpu.update_changed(&harness.device, &harness.queue, &layer, &provinces, &recolored).unwrap();
        }
        provinces.clear();
        recolored.clear();
        if frame % 9 == 4 {
            continue;
        }
        if frame % 11 == 6 {
            draw(&mut gpu, false);
            continue;
        }
        if gpu.dispatched_cells > 0 && gpu.dispatched_cells < gpu.dirty.len() {
            partial += 1;
        }
        let actual = draw(&mut gpu, true).unwrap();
        let color = |key: u32| colors[key as usize];
        let cpu = gradient::build_layer(&map, &Layer::new(LayerKind::Country, &keys, &color, &field)).unwrap();
        compare(&cpu, &actual, &format!("named changes, frame {frame}"));
    }
    println!("POLITICAL named changes: {partial} of 32 drawn frames partial");
    assert!(partial > 20, "only {partial} partial dispatches");
}
