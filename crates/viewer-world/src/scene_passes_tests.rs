use super::*;
use eframe::wgpu::{self, util::DeviceExt};
use std::sync::Arc;
#[cfg(test)]
use viewer_runtime::test_support::TEST_HANG_GUARD;
use viewer_gpu::test_support::gpu as tests;
use viewer_gpu::textures::data_texture;

pub(super) const ALL_PASSES: [Pass; 13] = [
    Pass::Underwater,
    Pass::Terrain,
    Pass::LandBorders,
    Pass::Rivers,
    Pass::Railways,
    Pass::Trees,
    Pass::Water,
    Pass::SeaBorders,
    Pass::Demilitarized,
    Pass::Straits,
    Pass::UnknownSnowMud,
    Pass::CountryNames,
    Pass::Sky,
];

fn dependents(failed: Pass) -> &'static [Pass] {
    match failed {
        Pass::LandBorders => &[Pass::Rivers, Pass::Railways, Pass::Trees, Pass::Water],
        Pass::Underwater => &[Pass::Water],
        _ => &[],
    }
}

#[test]
fn typed_dependency_closure() {
    assert_eq!(
        ALL_PASSES.map(Pass::name),
        [
            "underwater",
            "terrain",
            "land borders",
            "rivers",
            "railways",
            "trees",
            "water",
            "sea borders",
            "demilitarized",
            "straits",
            "unknown snow and mud",
            "country names",
            "sky",
        ]
    );
    for failed in ALL_PASSES {
        let mut diagnostics = Diagnostics::new();
        let mut frame = Frame::new(&mut diagnostics);
        frame.run::<()>(failed, || Err(io::Error::other("fault")));
        for pass in ALL_PASSES {
            let ready = pass != failed && !dependents(failed).contains(&pass);
            assert_eq!(frame.ready(pass), ready, "{failed:?} -> {pass:?}");
            assert_eq!(
                frame.run(pass, || {
                    assert!(ready, "withheld {pass:?} executed after {failed:?}");
                    Ok(7)
                }),
                ready.then_some(7),
            );
        }
        let mut expected = Diagnostics::from([(failed, "fault".into())]);
        for &pass in dependents(failed) {
            expected.insert(
                pass,
                format!("skipped: output of {} is unavailable", failed.name()),
            );
        }
        assert_eq!(diagnostics, expected);
        let expected: Vec<_> = expected
            .iter()
            .map(|(pass, reason)| format!("Map shader {}: 1 refused pass: {reason}", pass.name()))
            .collect();
        assert_eq!(lines(&diagnostics).collect::<Vec<_>>(), expected);
        let mut frame = Frame::new(&mut diagnostics);
        assert!(
            frame
                .run(failed, || Err::<(), _>(io::Error::other("later fault")))
                .is_none()
        );
        assert_eq!(lines(&diagnostics).collect::<Vec<_>>(), expected);
        assert_eq!(Frame::new(&mut diagnostics).run(failed, || Ok(9)), Some(9));
    }
}

#[test]
fn a_refused_hatch_is_named_once_and_withholds_no_other_pass() {
    let mut diagnostics = Diagnostics::new();
    let mut frame = Frame::new(&mut diagnostics);
    let reason = "unknown snow and mud hatch pipeline: unsupported target";
    assert!(
        frame
            .run::<()>(Pass::UnknownSnowMud, || Err(io::Error::other(reason)))
            .is_none()
    );
    assert!(
        frame
            .run(Pass::UnknownSnowMud, || -> io::Result<()> {
                panic!("refused hatch retried")
            })
            .is_none()
    );
    for pass in ALL_PASSES
        .into_iter()
        .filter(|&pass| pass != Pass::UnknownSnowMud)
    {
        assert!(frame.ready(pass), "{pass:?}");
    }
    assert_eq!(
        lines(&diagnostics).collect::<Vec<_>>(),
        [format!(
            "Map shader unknown snow and mud: 1 refused pass: {reason}"
        )]
    );
}

#[test]
fn multiple_producers_name_the_first_unavailable_dependency() {
    let mut diagnostics = Diagnostics::new();
    let mut frame = Frame::new(&mut diagnostics);
    for pass in [Pass::LandBorders, Pass::Underwater] {
        frame.run::<()>(pass, || Err(io::Error::other(pass.name())));
    }
    assert!(!frame.ready(Pass::Water));
    assert_eq!(
        diagnostics[&Pass::Water],
        "skipped: output of underwater is unavailable"
    );
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn world_rolls_back_all_retained_bindings_and_keeps_successful_binds() -> io::Result<()> {
    use super::super::super::scene_faults::Probe;
    use content_reader_gui::pdxshader::program::Stage;
    let (_serial, device, queue) = tests::device();
    let probe = Probe::new()?;
    let texture = |colour| {
        data_texture(
            &device,
            &queue,
            "retained fixture",
            [1, 1],
            wgpu::TextureFormat::Rgba8Unorm,
            colour,
        )
    };
    let green = texture(&[0, 255, 0, 255])?;
    let red = texture(&[255, 0, 0, 255])?;
    for pass in ALL_PASSES {
        let (mut state, _) = probe.state("Known")?;
        probe.bind(&mut state, &green.view)?;
        let retained = RefCell::new(state);
        let mut diagnostics = Diagnostics::new();
        let mut frame = Frame::new(&mut diagnostics);
        assert!(
            frame
                .world::<()>(pass, &retained, || {
                    let mut state = retained.borrow_mut();
                    probe.bind(&mut state, &red.view)?;
                    state.stream.bind_constants(2, Stage::Pixel, Some(&[0; 32]));
                    state.stream.bind_blend(None, wgpu::ColorWrites::empty());
                    state
                        .stream
                        .bind_rasterizer(content_reader_gui::pdxshader::RasterizerState::default());
                    state
                        .stream
                        .bind_depth_stencil(content_reader_gui::pdxshader::DepthStencilState {
                            depth_function: 0,
                            ..Default::default()
                        });
                    state.bind_vertex(0, 64, Arc::from(&[0_u8; 64][..]));
                    state.stream.forget_program();
                    Err(io::Error::other("changed retained state before refusal"))
                })
                .is_none()
        );
        assert_eq!(
            &probe.pixel(&mut retained.borrow_mut(), &device, &queue)?[..4],
            &[0, 255, 0, 255],
            "rollback {pass:?}"
        );
        let mut tail = viewer_scene_base::shader_scene::State::default();
        tail.inherit(&retained.borrow());
        let (selection, template) = probe.template("Unknown")?;
        tail.stream
            .bind_program(Some(selection), template.vertices.clone());
        let arena = shader_pipeline::gpu::Arena::new(&device);
        let error = tail
            .prepare(&arena, "restored unknown tail", &template, 3, 1)
            .err()
            .expect("rollback must retain the unknown tail");
        assert!(
            error.to_string().contains("unavailable bytes 16..32"),
            "{error}"
        );
        assert!(
            frame
                .world::<()>(pass, &retained, || panic!("failed pass retried"))
                .is_none()
        );
        for &dependent in dependents(pass) {
            assert!(
                frame
                    .world::<()>(dependent, &retained, || panic!("dependent pass ran"))
                    .is_none()
            );
        }
        assert_eq!(
            &probe.pixel(&mut retained.borrow_mut(), &device, &queue)?[..4],
            &[0, 255, 0, 255]
        );
        assert_eq!(
            Frame::new(&mut diagnostics).world(pass, &retained, || {
                probe.bind(&mut retained.borrow_mut(), &red.view)?;
                Ok(7)
            }),
            Some(7)
        );
        assert_eq!(
            &probe.pixel(&mut retained.borrow_mut(), &device, &queue)?[..4],
            &[255, 0, 0, 255],
            "successful binds {pass:?}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires the pinned shader compiler and a GPU"]
fn shader_initialization_refuses_only_actually_read_unknown_bytes() -> io::Result<()> {
    use super::super::super::scene_faults::Probe;
    use content_reader_gui::pdxshader::program::Stage;
    use shader_pipeline::{binding, gpu};
    let (_serial, device, queue) = tests::device();
    let probe = Probe::new()?;
    let arena = gpu::Arena::new(&device);
    let texture = data_texture(
        &device,
        &queue,
        "known-byte fixture",
        [1, 1],
        wgpu::TextureFormat::Rgba8Unorm,
        &[255; 4],
    )?;
    for (name, accepted) in [("Known", true), ("Unknown", false)] {
        let (selection, template) = probe.template(name)?;
        let authored = gpu::State::authored(
            &selection.effect.source,
            &selection.effect.source.techniques[selection.index],
        )
        .map_err(io::Error::other)?;
        let pipeline = gpu::Cache::default()
            .get_pass(&device, &selection, &authored, &template)
            .map_err(io::Error::other)?;
        let checked = pipeline.check_initialized(
            binding(2, Stage::Pixel),
            32,
            std::slice::from_ref(&(0..16)),
        );
        assert_eq!(checked.is_ok(), accepted, "{name}: {checked:?}");
        assert!(
            pipeline
                .check_initialized(binding(2, Stage::Pixel), 16, std::slice::from_ref(&(0..16)))
                .is_ok(),
            "D3D read outside the allocation is zero"
        );
        let (mut state, template) = probe.state(name)?;
        probe.bind(&mut state, &texture.view)?;
        let prepared = state.prepare(&arena, name, &template, 3, 1);
        assert_eq!(prepared.is_ok(), accepted, "{name}");
        if !accepted {
            let error = prepared.err().unwrap().to_string();
            assert!(
                error.contains(name) && error.contains("unavailable bytes 16..32"),
                "{error}"
            );
        }
    }
    Ok(())
}

#[test]
fn preflight_fault_skips_water_and_independent_pass_renders() {
    let instance = shader_pipeline::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let descriptor = wgpu::TextureDescriptor {
        label: Some("independent output"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let output = device.create_texture(&descriptor);
    let view = output.create_view(&Default::default());
    let source = device.create_texture_with_data(
        &queue,
        &wgpu::TextureDescriptor {
            label: Some("fixture source"),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ..descriptor.clone()
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[0, 255, 0, 255],
    );
    let source = source.create_view(&Default::default());
    let blitter = wgpu::util::TextureBlitter::new(&device, output.format());
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut diagnostics = Diagnostics::new();
    let mut frame = Frame::new(&mut diagnostics);
    let failed: Result<(), String> = Err("forced underwater preflight fault".into());
    assert!(frame.run(Pass::Underwater, || compiled(&failed)).is_none());
    assert!(
        frame
            .run::<()>(Pass::Underwater, || panic!("failed pass retried"))
            .is_none()
    );
    assert!(
        frame
            .run::<()>(Pass::Water, || panic!("sampled unavailable refraction"))
            .is_none()
    );
    assert!(
        frame
            .run(Pass::CountryNames, || {
                blitter.copy(&device, &mut encoder, &source, &view);
                Ok(())
            })
            .is_some()
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fixture readback"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        output.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(TEST_HANG_GUARD),
        })
        .unwrap();
    receive.recv_timeout(TEST_HANG_GUARD).unwrap().unwrap();
    assert_eq!(
        &buffer.slice(..).get_mapped_range().unwrap()[..4],
        &[0, 255, 0, 255]
    );
    Frame::new(&mut diagnostics).run(Pass::Underwater, || compiled(&failed));
    let warnings: Vec<_> = lines(&diagnostics).collect();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("Map shader underwater: 1 refused pass: forced underwater"))
    );
    assert!(warnings.iter().any(|line| {
        line.contains("Map shader water: 1 refused pass: skipped: output of underwater")
    }));
}
