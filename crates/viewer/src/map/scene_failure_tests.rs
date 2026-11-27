use eframe::wgpu;
use std::io;
use std::sync::MutexGuard;
use std::{fs, path::PathBuf, sync::Arc};
use viewer_runtime::errors::invalid;
use viewer_gpu::camera::MapSize;
use viewer_gpu::camera::Placement;
use viewer_gpu::frame::DayNight;
use viewer_gpu::gpu_contract::OUTPUT_FORMAT;
use viewer_gpu::gpu_contract::map_device_descriptor;
use viewer_gpu::gpu_contract::select_adapter;
use viewer_runtime::name_draw::Placements;
use viewer_foundation::perspective::Choice;
use viewer_runtime::presentation::Continuity;
#[cfg(test)]
use viewer_gpu::test_support::serial as test_support_serial;
use viewer_models::ModelPass;
use viewer_world::scene::passes;
use viewer_world::scene::*;
use viewer_world::scene_faults::{Point, Probe};

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    replay: PathBuf,
    game: PathBuf,
    user: PathBuf,
    cache: PathBuf,
    output: PathBuf,
    pub(crate) branch: replay_data::BranchId,
    pub(crate) hour: i32,
    camera: [f32; 3],
}

pub(crate) struct Fixture {
    pub(crate) document: crate::document::Document,
    pub(crate) state: eframe::egui_wgpu::RenderState,
    pub(crate) input: Input,
    pub(crate) output: PathBuf,
    names: Placements,
    _serial: MutexGuard<'static, ()>,
}

impl Fixture {
    pub(crate) fn load(name: &str) -> io::Result<Self> {
        let path = std::env::var_os("BATTLEPLAN_SCENE_FAILURE_INPUT").or_else(|| std::env::var_os("HOI4_SCENE_FAILURE_INPUT"))
            .ok_or_else(|| io::Error::other("BATTLEPLAN_SCENE_FAILURE_INPUT is required"))?;
        let input: Input = serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)?;
        fs::create_dir_all(&input.output)?;
        let output = input.output.join(name);
        fs::create_dir(&output)?;
        let serial = test_support_serial();
        let state = pollster::block_on(async {
            let instance =
                shader_pipeline::gpu::instance();
            let available_adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
            let adapter = select_adapter(&available_adapters, None)?;
            eprintln!("Scene failure GPU: {:?}", adapter.get_info());
            let (device, queue) = adapter
                .request_device(&map_device_descriptor(&adapter))
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
        let document = crate::document::Document::load(
            &input.replay,
            &content_reader_layers::ResolveOptions {
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
        let names = document.place_names(&Default::default())?;
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
            camera: Placement {
                center: [x, z],
                height,
                pose: viewer_gpu::camera::Pose::Map,
            }
            .on(MapSize::of(&self.document.map), self.document.map_defines),
            day_night: DayNight::at_hour(self.input.hour),
            animation_time: 0.,
            map_time: 0.,
            transition_time: None,
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
        self.scene_with_samples(1)
    }

    pub(crate) fn scene_with_samples(&self, samples: u32) -> io::Result<Scene> {
        let d = &self.document;
        let (device, queue) = (&self.state.device, &self.state.queue);
        let mut scene = crate::app::load_scene(
            device,
            queue,
            d,
            dashboard_core::protocol::MapGraphics {
                samples,
                ..Default::default()
            },
            &Default::default(),
        )?;
        scene.build_names_in_upload(true);
        let perspective = Choice::default();
        let political = viewer_map_modes::political::build(&d.map, d.frame()?)?;
        scene.set_political(device, queue, &political.regions)?;
        scene.set_classes(
            device,
            &d.border_meshes,
            &political.regions,
            &d.demilitarized_zone(&political.regions)?,
        )?;
        scene.set_map_frame(&d.frame_assets)?;
        let (buildings, _) = d.map_building_field(&perspective)?;
        scene.set_city_damage(&buildings, true);
        scene.set_map_buildings(&d.map_buildings, buildings, None)?;
        let rails = d.rail_mesh()?;
        scene.set_railways(device, queue, &d.rail_assets, &rails)?;
        scene.set_bridges(&d.bridges, d.bridges.field(&rails))?;
        let _ = scene.set_aircraft(
            d.aircraft.clone(),
            d.aircraft_plan(&perspective)?,
            Continuity::Reset,
        )?;
        let _ = scene.set_trains(
            d.trains.clone(),
            d.train_plan(&perspective)?,
            Continuity::Reset,
        )?;
        let (units, _) = d.unit_model_field(&perspective)?;
        scene.set_unit_models(device, &d.unit_models, units, true)?;
        scene.set_orders(
            device,
            queue,
            &d.orders,
            d.order_field(&perspective)?.as_ref().clone(),
        )?;
        scene.set_weather_effects(
            device,
            queue,
            d.weather_effects.clone(),
            d.hour()?.weather_field()?,
        )?;
        let snow = Arc::new(d.snow_mud()?);
        scene.set_snow_mud(device, queue, &snow, Continuity::Reset)?;
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
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
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
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn production_scene_targets_and_post_recover() -> io::Result<()> {
    let fixture = Fixture::load("targets")?;
    let mut scene = fixture.scene()?;
    let mut control = fixture.scene()?;
    let mut generation = 0;
    for (step, (size, full, postfx, replaces, sea_bed)) in [
        ([256, 144], true, true, true, [128, 72]),
        ([256, 144], true, true, false, [128, 72]),
        ([2049, 1153], true, true, true, [1025, 577]),
        ([2049, 1153], false, true, true, [1025, 577]),
        ([2049, 1153], false, false, false, [1025, 577]),
        ([2049, 1153], false, true, false, [1025, 577]),
    ]
    .into_iter()
    .enumerate()
    {
        control.targets = None;
        control.post.invalidate();
        for current in [&mut scene, &mut control] {
            current.set_graphics(dashboard_core::protocol::MapGraphics {
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
            [
                targets.scene_texture().width(),
                targets.scene_texture().height()
            ],
            targets.world_size()
        );
        assert_eq!(
            [
                targets
                    .depth_attachment(wgpu::LoadOp::Load)
                    .view
                    .texture()
                    .width(),
                targets
                    .depth_attachment(wgpu::LoadOp::Load)
                    .view
                    .texture()
                    .height()
            ],
            targets.world_size()
        );
        assert_eq!(
            [
                targets.refraction_view.texture().width(),
                targets.refraction_view.texture().height()
            ],
            sea_bed,
            "the water samples the half-size sea bed"
        );
        assert_eq!(
            scene.shadow_screen(),
            Some(viewer_scene_base::shadow::Screen::Half),
            "the shipped shadow readers sample the half-size screen buffer"
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
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
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

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn near_names_fade_in_as_the_country_names_fade_out() -> io::Result<()> {
    use viewer_runtime::name_draw::next_generation;
    let fixture = Fixture::load("near-names")?;
    let mut scene = fixture.scene()?;
    let names = Placements {
        country: Vec::new(),
        warnings: Vec::new(),
        generation: next_generation(),
        near: Some(Arc::new(Placements {
            generation: next_generation(),
            ..fixture.names.clone()
        })),
    };
    assert!(!fixture.names.country.is_empty());
    let mut drawn = |layer_alpha| -> io::Result<u32> {
        let mut request = fixture.request([256, 144]);
        request.overlay = Overlay {
            names: Some(&names),
            layer_alpha,
            ..request.overlay
        };
        fixture.render(&mut scene, request)?;
        Ok(scene.overlay_counts().0)
    };
    assert!(drawn(0.)? > 0, "the near names are not drawn close in");
    assert_eq!(drawn(1.)?, 0, "the near names stay drawn far out");
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn installed_names_handed_nothing_for_frames_are_drawn_when_handed_again() -> io::Result<()> {
    use viewer_runtime::name_draw::next_generation;
    let fixture = Fixture::load("names-handed-nothing")?;
    let mut scene = fixture.scene()?;
    scene.build_names_in_upload(false);
    let names = Placements { generation: next_generation(), ..fixture.names.clone() };
    assert!(!names.country.is_empty());
    scene.set_hour_layers(&fixture.state.device, &fixture.state.queue, HourLayers { names: Some(&names), ..Default::default() })?;
    let nothing = Placements::default();
    let mut draw = |handed: &Placements| -> io::Result<()> {
        let mut request = fixture.request([256, 144]);
        request.overlay = Overlay { names: Some(handed), ..request.overlay };
        fixture.render(&mut scene, request)
    };
    for frame in 0..3 {
        draw(&nothing).map_err(|error| io::Error::other(format!("frame {frame} handed no names: {error}")))?;
    }
    draw(&names).map_err(|error| io::Error::other(format!("the installed names handed again: {error}")))?;
    assert!(scene.holds_country_names(&names), "the installed names are still prepared");
    Ok(())
}
