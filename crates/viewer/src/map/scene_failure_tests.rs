//! Controlled faults in the production Scene, never a runtime fault switch.
//! Covers depth/stencil allocation and shadow clearing/blur.
//! These tests prove viewer refusal/recovery, not native exception policy or depth precision.
use super::*;
use std::{
    cell::{Cell, RefCell},
    fs,
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Point {
    Targets,
    Shadow,
    Trees,
    Arrows,
    Models,
    Weather,
    Post,
}

#[derive(Default)]
pub(crate) struct Faults {
    pub(crate) isolated: Option<(passes::Pass, String)>,
    pub(crate) fatal: Option<Point>,
    pub(crate) submissions: Cell<u64>,
    attempted: RefCell<Vec<passes::Pass>>,
    reached: RefCell<Vec<Point>>,
    sentinel: bool,
}

impl Faults {
    pub(super) fn begin_frame(&self) {
        self.attempted.borrow_mut().clear();
        self.reached.borrow_mut().clear();
    }

    pub(super) fn preflight(&self, pass: passes::Pass) -> io::Result<()> {
        self.attempted.borrow_mut().push(pass);
        if let Some((failed, reason)) = &self.isolated
            && *failed == pass
        {
            return Err(io::Error::other(reason.clone()));
        }
        Ok(())
    }

    pub(super) fn after(&self, point: Point) -> io::Result<()> {
        self.reached.borrow_mut().push(point);
        if self.fatal == Some(point) {
            return Err(io::Error::other(format!(
                "test Map {point:?}: controlled frame failure"
            )));
        }
        Ok(())
    }

    // An independent draw on Scene's actual encoder/output, after its real passes.
    // The full Scene assertions below also check pass placement and target history.
    pub(super) fn draw_sentinel(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
    ) {
        if !self.sentinel {
            return;
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Scene failure sentinel"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                @vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
                    let p = array<vec2f, 3>(vec2f(-1, -1), vec2f(3, -1), vec2f(-1, 3));
                    return vec4f(p[i], 0, 1);
                }
                @fragment fn pixel() -> @location(0) vec4f { return vec4f(0, 1, 0, 1); }
            "#
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Scene failure sentinel"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("pixel"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: targets.samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Scene failure sentinel"),
            color_attachments: &[Some(targets.output_attachment(wgpu::LoadOp::Load))],
            ..Default::default()
        });
        render.set_pipeline(&pipeline);
        render.set_scissor_rect(0, 0, 8, 8);
        render.draw(0..3, 0..1);
    }
}

/// Supply an external JSON via BATTLE_PLAN_SCENE_FAILURE_INPUT: replay, game, user, cache, output, branch,
/// hour and camera ([x,z,height]). Use a decided hour and an admitted-mesh camera below the shadow
/// cutoff. Each ignored check creates a new child directory under `output`.
#[derive(serde::Deserialize)]
pub(crate) struct Input {
    replay: PathBuf,
    game: PathBuf,
    user: PathBuf,
    cache: PathBuf,
    output: PathBuf,
    pub(crate) branch: replay::BranchId,
    pub(crate) hour: i32,
    camera: [f32; 3],
}

pub(crate) struct Fixture {
    pub(crate) document: crate::document::Document,
    pub(crate) state: eframe::egui_wgpu::RenderState,
    pub(crate) input: Input,
    pub(crate) output: PathBuf,
    names: Placements,
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    pub(crate) fn load(name: &str) -> io::Result<Self> {
        let path = std::env::var_os("BATTLE_PLAN_SCENE_FAILURE_INPUT").or_else(|| std::env::var_os("HOI4_SCENE_FAILURE_INPUT"))
            .ok_or_else(|| io::Error::other("BATTLE_PLAN_SCENE_FAILURE_INPUT is required"))?;
        let input: Input = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        fs::create_dir_all(&input.output)?;
        let output = input.output.join(name);
        fs::create_dir(&output)?;
        let serial = crate::map::political_gpu::tests::serial();
        let state = pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
            let adapter = crate::render::select_adapter(&available_adapters, None)?;
            eprintln!("Scene failure GPU: {:?}", adapter.get_info());
            let (device, queue) = adapter
                .request_device(&crate::render::device_descriptor(&adapter))
                .await
                .map_err(io::Error::other)?;
            let renderer =
                eframe::egui_wgpu::Renderer::new(&device, OUTPUT_FORMAT, Default::default());
            Ok::<_, io::Error>(eframe::egui_wgpu::RenderState {
                adapter,
                available_adapters,
                instance,
                device,
                queue,
                target_format: OUTPUT_FORMAT,
                renderer: Arc::new(eframe::egui::mutex::RwLock::new(renderer)),
                surface_config: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
            })
        })?;
        // Same covered-content check and recorded DLC/mod order as --dump. No override.
        let document = crate::document::Document::load(
            &input.replay,
            &installed_content::ResolveOptions {
                game_dir: input.game.clone(),
                user_dir: input.user.clone(),
                cache_dir: input.cache.clone(),
            },
            false,
            &crate::document::InitialHour {
                branch: Some(input.branch),
                hour: Some(input.hour),
                hour_fraction: 0.,
            },
            None,
            0,
        )?;
        document.icons.lazy.set_background(false);
        document.frame()?;
        let names = document.place_names()?;
        Ok(Self {
            document,
            state,
            input,
            output,
            names,
            _serial: serial,
        })
    }

    pub(crate) fn request(&self, size: [u32; 2]) -> Request<'_> {
        let [x, z, height] = self.input.camera;
        Request {
            size,
            camera: super::super::camera::Placement {
                center: [x, z],
                height,
            }
            .on(
                super::super::camera::MapSize::of(&self.document.map),
                self.document.map_defines,
            ),
            day_night: DayNight::at_hour(self.input.hour),
            animation_time: 0.,
            map_time: 0.,
            postfx: true,
            no_water: false,
            no_shadows: false,
            no_trees: false,
            overlay: Overlay {
                names: Some(&self.names),
                layer_alpha: 1.,
                ..Default::default()
            },
        }
    }

    pub(crate) fn scene(&self) -> io::Result<Scene> {
        let d = &self.document;
        let (device, queue) = (&self.state.device, &self.state.queue);
        let mut scene = crate::app::load_scene(
            device,
            queue,
            d,
            dashboard::protocol::MapGraphics {
                samples: 1,
                ..Default::default()
            },
        )?;
        let perspective = super::super::perspective::Choice::default();
        let political = super::super::political::build(&d.map, d.frame()?)?;
        scene.set_political(device, queue, &political.regions)?;
        scene.set_classes(
            device,
            &d.border_meshes,
            &political.regions,
            &d.demilitarized_zone(&political.regions)?,
        )?;
        scene.set_map_frame(device, queue, &d.frame_assets)?;
        let (buildings, _) = d.map_building_field(&perspective)?;
        scene.set_city_damage(&buildings, true);
        scene.set_map_buildings(device, queue, &d.map_buildings, buildings, None)?;
        let rails = d.rail_mesh()?;
        scene.set_railways(device, queue, &d.rail_assets, &rails)?;
        scene.set_bridges(device, queue, &d.bridges, d.bridges.field(&rails))?;
        let _ = scene.set_aircraft(d.aircraft.clone(), d.aircraft_plan(&perspective)?)?;
        let _ = scene.set_trains(
            d.trains.clone(),
            d.train_plan(&perspective)?,
            super::super::presentation::Continuity::Reset,
        )?;
        let (units, _) = d.unit_model_field(&perspective)?;
        scene.set_unit_models(device, queue, &d.unit_models, units, true)?;
        scene.set_orders(
            device,
            queue,
            &d.orders,
            d.order_field(&perspective)?.as_ref(),
        )?;
        scene.set_weather_effects(
            device,
            queue,
            d.weather_effects.clone(),
            super::super::weather_effects::field(d.frame()?)?,
        )?;
        let snow = d.snow_mud()?;
        scene.set_snow_mud(
            device,
            queue,
            &snow,
            super::super::presentation::Continuity::Reset,
        )?;
        let intel = d.intel_map(&perspective)?;
        scene.set_fog_of_war(queue, &intel.texels, intel.fog)?;
        scene.settle_models();
        Ok(scene)
    }

    fn render(&self, scene: &mut Scene, request: Request<'_>) -> io::Result<()> {
        scene.render(&self.state.device, &self.state.queue, request)
    }

    pub(crate) fn pixels(&self, scene: &Scene, name: &str) -> io::Result<Vec<u8>> {
        let path = self.output.join(format!("{name}.png"));
        crate::render::write_png(scene, &self.state.device, &self.state.queue, &path)?;
        let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(path)?));
        let mut reader = decoder.read_info().map_err(io::Error::other)?;
        let mut bytes = vec![
            0;
            reader
                .output_buffer_size()
                .ok_or_else(|| invalid("PNG too large"))?
        ];
        let info = reader.next_frame(&mut bytes).map_err(io::Error::other)?;
        bytes.truncate(info.buffer_size());
        Ok(bytes)
    }
}

const ISOLATED: [passes::Pass; 11] = [
    passes::Pass::Underwater,
    passes::Pass::Terrain,
    passes::Pass::LandBorders,
    passes::Pass::Rivers,
    passes::Pass::Railways,
    passes::Pass::Water,
    passes::Pass::SeaBorders,
    passes::Pass::Demilitarized,
    passes::Pass::Straits,
    passes::Pass::CountryNames,
    passes::Pass::Sky,
];

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLE_PLAN_SCENE_FAILURE_INPUT"]
fn production_scene_preflight_matrix() -> io::Result<()> {
    let fixture = Fixture::load("preflight")?;
    let mut scene = fixture.scene()?;
    let probe = Probe::new()?;
    let binding = probe.missing_binding(&fixture.state.device)?;
    let request = Request {
        postfx: false,
        no_shadows: true,
        ..fixture.request([256, 144])
    };
    fixture.render(&mut scene, request)?;
    assert!(
        scene.pass_diagnostics.is_empty(),
        "control: {:?}",
        scene.pass_warnings()
    );
    scene.test_faults.sentinel = true;
    fixture.render(&mut scene, request)?;
    let control = fixture.pixels(&scene, "control")?;
    for pass in ISOLATED {
        for (kind, reason) in [
            ("compiled", "test compiled program unavailable".to_string()),
            ("binding", binding.clone()),
        ] {
            scene.pass_diagnostics.clear();
            scene.test_faults.isolated = Some((pass, reason.clone()));
            let before = scene.test_faults.submissions.get();
            for frame in 0..2 {
                fixture.render(&mut scene, request)?;
                let mut expected = passes::Diagnostics::from([(pass, reason.clone())]);
                for dependent in match pass {
                    passes::Pass::Underwater => &[passes::Pass::Water][..],
                    passes::Pass::LandBorders => &[
                        passes::Pass::Rivers,
                        passes::Pass::Railways,
                        passes::Pass::Trees,
                        passes::Pass::Water,
                    ][..],
                    _ => &[],
                } {
                    expected.insert(
                        *dependent,
                        format!("skipped: output of {} is unavailable", pass.name()),
                    );
                    assert!(!scene.test_faults.attempted.borrow().contains(dependent));
                    if *dependent == passes::Pass::Trees {
                        assert!(!scene.test_faults.reached.borrow().contains(&Point::Trees));
                    }
                }
                assert_eq!(scene.pass_diagnostics, expected, "{pass:?}/{kind}/{frame}");
                let lines: Vec<_> = scene
                    .pass_warnings()
                    .into_iter()
                    .filter(|line| line.starts_with("Map shader "))
                    .collect();
                assert_eq!(lines, passes::lines(&expected).collect::<Vec<_>>());
                let pixels = fixture.pixels(&scene, &format!("{pass:?}-{kind}-{frame}"))?;
                assert_eq!(
                    &pixels[..4],
                    &[0, 255, 0, 255],
                    "independent output after {pass:?}"
                );
                for independent in ISOLATED.into_iter().filter(|p| !expected.contains_key(p)) {
                    assert!(
                        scene.test_faults.attempted.borrow().contains(&independent),
                        "independent {independent:?} not reached"
                    );
                }
            }
            assert_eq!(scene.test_faults.submissions.get(), before + 2);
            scene.test_faults.isolated = None;
            scene.pass_diagnostics.clear();
            fixture.render(&mut scene, request)?;
            assert!(scene.pass_diagnostics.is_empty());
            assert_eq!(
                fixture.pixels(&scene, &format!("{pass:?}-{kind}-recovered"))?,
                control
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLE_PLAN_SCENE_FAILURE_INPUT"]
fn production_scene_targets_and_post_recover() -> io::Result<()> {
    let fixture = Fixture::load("targets")?;
    let mut scene = fixture.scene()?;
    let mut control = fixture.scene()?;
    let mut generation = 0;
    for (step, (size, full, postfx, replaces)) in [
        ([256, 144], true, true, true),
        ([256, 144], true, true, false),
        ([2049, 1153], true, true, true),
        ([2049, 1153], false, true, true),
        ([2049, 1153], false, false, false),
        ([2049, 1153], false, true, false),
    ]
    .into_iter()
    .enumerate()
    {
        // Fresh target/post bindings, with the same CPU and committed shadow
        // history. Following the same resize bug in both scenes is no oracle.
        control.targets = None;
        control.post.invalidate();
        for current in [&mut scene, &mut control] {
            current.set_graphics(dashboard::protocol::MapGraphics {
                samples: 1,
                full_resolution: full,
                ..Default::default()
            });
            fixture.render(
                current,
                Request {
                    postfx,
                    ..fixture.request(size)
                },
            )?;
        }
        generation += u64::from(replaces);
        assert_eq!(scene.generation(), generation);
        let targets = scene.targets().unwrap();
        assert_eq!(targets.size(), size);
        assert_eq!(targets.world_size(), world_size(size, full));
        assert_eq!(
            [targets.scene.width(), targets.scene.height()],
            targets.world_size()
        );
        assert_eq!(
            [
                targets.depth_ms.texture().width(),
                targets.depth_ms.texture().height()
            ],
            targets.world_size()
        );
        assert_eq!(
            [
                targets.refraction_view.texture().width(),
                targets.refraction_view.texture().height()
            ],
            size
        );
        assert_eq!(
            fixture.pixels(&scene, &format!("step-{step}"))?,
            fixture.pixels(&control, &format!("control-{step}"))?
        );
        assert_eq!(scene.grading(), control.grading());
        assert_eq!(scene.shadows(), control.shadows());
    }
    let limit = fixture.state.device.limits().max_texture_dimension_2d;
    for size in [[0, 144], [256, 0], [limit + 1, 144], [1, 1], [3, 3]] {
        let submissions = scene.test_faults.submissions.get();
        assert!(
            fixture.render(&mut scene, fixture.request(size)).is_err(),
            "accepted {size:?}"
        );
        assert_eq!(scene.generation(), generation);
        assert_eq!(scene.test_faults.submissions.get(), submissions);
    }
    let mut invalid_camera = fixture.request([2049, 1153]);
    invalid_camera.camera.center[0] = f32::NAN;
    let submissions = scene.test_faults.submissions.get();
    assert!(fixture.render(&mut scene, invalid_camera).is_err());
    assert_eq!(scene.test_faults.submissions.get(), submissions);
    assert_eq!(scene.generation(), generation);
    let request = fixture.request([2049, 1153]);
    control.targets = None;
    control.post.invalidate();
    fixture.render(&mut scene, request)?;
    fixture.render(&mut control, request)?;
    assert_eq!(
        fixture.pixels(&scene, "recovered")?,
        fixture.pixels(&control, "recovered-control")?
    );
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLE_PLAN_SCENE_FAILURE_INPUT"]
fn production_scene_fatal_encoder_is_not_submitted() -> io::Result<()> {
    let fixture = Fixture::load("fatal")?;
    for point in [
        Point::Targets,
        Point::Shadow,
        Point::Trees,
        Point::Arrows,
        Point::Models,
        Point::Weather,
        Point::Post,
    ] {
        let mut scene = fixture.scene()?;
        let mut control = fixture.scene()?;
        let request = fixture.request([256, 144]);
        fixture.render(&mut scene, request)?;
        fixture.render(&mut control, request)?;
        assert!(
            scene.shadows(),
            "input camera must admit the resolved shadow chain"
        );
        if point == Point::Models {
            let drawn = scene.native_map_frame.as_ref().map_or(0, ModelPass::drawn)
                + scene.sprawl_drawn()
                + scene.map_buildings_drawn()
                + scene.unit_models_drawn()
                + scene.aircraft_drawn();
            assert!(drawn > 0, "input camera must admit a supported mesh actor");
        }
        let before = fixture.pixels(&scene, &format!("{point:?}-before"))?;
        let submissions = scene.test_faults.submissions.get();
        let generation = scene.generation();
        scene.test_faults.fatal = Some(point);
        let failed = Request {
            size: if point == Point::Targets {
                [257, 145]
            } else {
                request.size
            },
            ..request
        };
        let error = fixture.render(&mut scene, failed).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("test Map {point:?}: controlled frame failure")
        );
        assert!(scene.test_faults.reached.borrow().contains(&point));
        assert_eq!(
            scene.test_faults.submissions.get(),
            submissions,
            "submitted failed {point:?} encoder"
        );
        if point != Point::Targets {
            assert_eq!(
                fixture.pixels(&scene, &format!("{point:?}-unsubmitted"))?,
                before
            );
        }
        assert!(
            scene.pass_diagnostics.is_empty(),
            "fatal error became an isolated refusal"
        );
        // Post recovery must also work without a resize. The control keeps the
        // same committed shadow history but starts a fresh post cache.
        scene.test_faults.fatal = None;
        control.targets = None;
        control.post.invalidate();
        let recovered = Request {
            size: if point == Point::Targets {
                [258, 146]
            } else {
                request.size
            },
            ..request
        };
        fixture.render(&mut scene, recovered)?;
        fixture.render(&mut control, recovered)?;
        assert_eq!(scene.generation() > generation, point == Point::Targets);
        assert_eq!(
            fixture.pixels(&scene, &format!("{point:?}-recovered"))?,
            fixture.pixels(&control, &format!("{point:?}-control"))?
        );
        assert_eq!(scene.grading(), control.grading());
        assert_eq!(scene.shadows(), control.shadows());
        assert_eq!(scene.last_animation_time, control.last_animation_time);
    }
    Ok(())
}

/// Independently authored shader: declared unknown b2 tail, actual texture and vertex reads.
/// Both the Scene boundary faults and the retained-state test use its real ABI refusal.
pub(crate) struct Probe {
    effect: Arc<shader_pipeline::library::Effect>,
}

impl Probe {
    pub(crate) fn new() -> io::Result<Self> {
        let source = br#"
            VertexStruct I { float2 p : POSITION; };
            VertexStruct O { float4 p : SV_POSITION; };
            ConstantBuffer(2,0) { float4 colour; float4 unknown; };
            VertexShader = { MainCode V [[ O main(I i) { O o; o.p=float4(i.p,0,1); return o; } ]] }
            PixelShader = {
                Samplers = { T = { Index = 0 MagFilter = Point MinFilter = Point MipFilter = None AddressU = Clamp AddressV = Clamp } }
                MainCode P [[ float4 main(O o) : SV_TARGET { return colour * T_Texture.Sample(T_Sampler,float2(0.5,0.5)); } ]]
                MainCode U [[ float4 main(O o) : SV_TARGET { return unknown; } ]]
            }
            RasterizerState Visible { CullMode = none }
            Effect Known { VertexShader = V PixelShader = P RasterizerState = Visible }
            Effect Unknown { VertexShader = V PixelShader = U RasterizerState = Visible }
        "#;
        let program = installed_content::pdxshader::program::read_with(b"scene-test.shader", &|path| {
            Some(if path == b"scene-test.shader" {
                source.to_vec()
            } else {
                Vec::new()
            })
        })
        .map_err(io::Error::other)?;
        let compiler = shader_pipeline::Compiler::discover().map_err(io::Error::other)?;
        let compiled = program
            .techniques
            .iter()
            .map(|technique| compiler.compile(&program, technique, &[]).map(Arc::new))
            .collect();
        Ok(Self {
            effect: shader_pipeline::library::Effect::from_compiled(
                Arc::new(program),
                Vec::new(),
                compiled,
            ),
        })
    }

    pub(crate) fn template(
        &self,
        name: &str,
    ) -> io::Result<(
        shader_pipeline::library::Selection,
        shader_pipeline::gpu::Pass,
    )> {
        let selection = self.effect.select(name).map_err(io::Error::other)?.unwrap();
        let compiled = self
            .effect
            .compiled(selection.index)
            .map_err(io::Error::other)?;
        let state = shader_pipeline::gpu::State::authored(
            &self.effect.source,
            &self.effect.source.techniques[selection.index],
        )
        .map_err(io::Error::other)?;
        let mut pass = shader_pipeline::gpu::Pass::packed(&compiled, &state, OUTPUT_FORMAT, 1);
        let raster = installed_content::pdxshader::RasterizerState {
            cull_mode: 0,
            ..Default::default()
        };
        let depth = installed_content::pdxshader::DepthStencilState {
            depth_enable: false,
            depth_write_mask: 0,
            ..Default::default()
        };
        pass.depth_stencil =
            Some(shader_pipeline::gpu::depth_stencil(&depth, &raster, DEPTH_FORMAT).0);
        Ok((selection, pass))
    }

    pub(crate) fn state(
        &self,
        name: &str,
    ) -> io::Result<(
        super::super::shader_scene::State,
        shader_pipeline::gpu::Pass,
    )> {
        let (selection, pass) = self.template(name)?;
        let mut state = super::super::shader_scene::State::default();
        state
            .stream
            .bind_program(Some(selection), pass.vertices.clone());
        state
            .stream
            .bind_rasterizer(installed_content::pdxshader::RasterizerState {
                cull_mode: 0,
                ..Default::default()
            });
        state
            .stream
            .bind_depth_stencil(installed_content::pdxshader::DepthStencilState {
                depth_enable: false,
                depth_write_mask: 0,
                ..Default::default()
            });
        state.bind_vertex(
            0,
            8,
            Arc::from(bytemuck::cast_slice(&[[-1_f32, -1.], [3., -1.], [-1., 3.]])),
        );
        Ok((state, pass))
    }

    pub(crate) fn bind(
        &self,
        state: &mut super::super::shader_scene::State,
        texture: &wgpu::TextureView,
    ) -> io::Result<()> {
        use installed_content::pdxshader::program::Stage;
        use shader_pipeline::binding;
        let selection = self
            .effect
            .select("Known")
            .map_err(io::Error::other)?
            .unwrap();
        let authored = shader_pipeline::gpu::State::authored(
            &self.effect.source,
            &self.effect.source.techniques[selection.index],
        )
        .map_err(io::Error::other)?;
        let mut bytes = vec![0; 32];
        bytes[..16].copy_from_slice(bytemuck::cast_slice(&[1_f32; 4]));
        state.stream.bind_partial_constants(
            2,
            Stage::Pixel,
            Some(&bytes),
            std::slice::from_ref(&(0..16)),
        );
        state.stream.bind_texture(
            0,
            Stage::Pixel,
            Some(texture.clone()),
            authored.samplers.get(&binding(0, Stage::Pixel)),
        );
        Ok(())
    }

    pub(crate) fn pixel(
        &self,
        state: &mut super::super::shader_scene::State,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> io::Result<Vec<u8>> {
        let arena = shader_pipeline::gpu::Arena::new(device);
        let (_, template) = self.template("Known")?;
        let submission = state.prepare(&arena, "retained Scene fixture", &template, 3, 1)?;
        let targets = Targets::new(device, [8, 8], [8, 8], 1);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(
                    targets.output_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                )],
                depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Clear(1.))),
                ..Default::default()
            });
            render.set_pipeline(&submission.pass.pipeline.pipeline);
            submission.pass.bindings.bind(&mut render);
            for (slot, vertices) in submission.vertices.iter().enumerate() {
                render.set_vertex_buffer(slot as u32, vertices.slice());
            }
            render.draw(0..3, 0..1);
        }
        arena.flush(queue);
        queue.submit([encoder.finish()]);
        crate::render::read_test_texture(device, queue, targets.output(), 4)
    }

    fn missing_binding(&self, device: &wgpu::Device) -> io::Result<String> {
        let arena = shader_pipeline::gpu::Arena::new(device);
        let (mut state, pass) = self.state("Unknown")?;
        let error = state
            .prepare(&arena, "Scene binding fault", &pass, 3, 1)
            .err()
            .ok_or_else(|| io::Error::other("missing native b2 was accepted"))?;
        assert!(error.to_string().contains("constant buffer 2"), "{error}");
        Ok(error.to_string())
    }
}
