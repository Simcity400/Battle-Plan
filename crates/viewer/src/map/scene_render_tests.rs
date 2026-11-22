#[path = "scene_render_sprites.rs"]
mod sprites;
use super::scene_failure_tests::Fixture;
use std::io;
use viewer_gpu::{camera::{Free, Lens, Pose}, gpu_contract::read_texture};
use viewer_world::{scene::{Request, Scene, SceneProof, passes}, scene_faults::Point};

fn error(result: io::Result<()>) -> Result<(), (io::ErrorKind, String)> {
    result.map_err(|error| (error.kind(), error.to_string()))
}

fn normalized(scene: &Scene) -> SceneProof {
    let mut proof = scene.frame_proof_for_tests();
    for (event, key) in proof
        .reports
        .sound_events
        .iter_mut()
        .zip(&proof.sound_owner_keys)
    {
        event.key = *key;
    }
    proof
}

fn equal_bytes(actual: &[u8], expected: &[u8], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}, byte length");
    let mismatch = actual
        .iter()
        .zip(expected)
        .enumerate()
        .find_map(|(offset, (a, b))| (a != b).then_some((offset, *a, *b)));
    assert_eq!(mismatch, None, "{label}, first byte mismatch");
}

fn compare(fixture: &Fixture, scenes: &[Scene; 2], label: &str) -> io::Result<()> {
    let mut expected = normalized(&scenes[0]);
    let expected_arena = std::mem::take(&mut expected.arena);
    for (index, scene) in scenes.iter().enumerate().skip(1) {
        let mut actual = normalized(scene);
        equal_bytes(
            &actual.arena,
            &expected_arena,
            &format!("{label}, path {index}, Arena"),
        );
        actual.arena.clear();
        assert_eq!(actual, expected, "{label}, path {index}");
    }
    let texture = |scene: &Scene, hdr| -> io::Result<Option<Vec<u8>>> {
        scene
            .targets()
            .map(|targets| {
                read_texture(
                    &fixture.state.device,
                    &fixture.state.queue,
                    if hdr {
                        targets.scene_texture()
                    } else {
                        targets.output()
                    },
                    if hdr { 8 } else { 4 },
                )
            })
            .transpose()
    };
    for hdr in [false, true] {
        let expected = texture(&scenes[0], hdr)?;
        for (index, scene) in scenes.iter().enumerate().skip(1) {
            let actual = texture(scene, hdr)?;
            assert_eq!(
                actual.is_some(),
                expected.is_some(),
                "{label}, path {index}, hdr {hdr}, target presence"
            );
            if let (Some(actual), Some(expected)) = (&actual, &expected) {
                equal_bytes(
                    actual,
                    expected,
                    &format!("{label}, path {index}, hdr {hdr}"),
                );
            }
        }
    }
    eprintln!("Scene immediate determinism: {label}");
    Ok(())
}

fn render(
    fixture: &Fixture,
    scenes: &mut [Scene; 2],
    sprites: &sprites::Sprites,
    request: Request<'_>,
) {
    let batch = sprites.batch((request.animation_time * 60.).round() as usize);
    let request = Request {
        overlay: viewer_world::scene::Overlay {
            sprites: Some(&batch),
            ..request.overlay
        },
        ..request
    };
    let (device, queue) = (&fixture.state.device, &fixture.state.queue);
    let reference = error(scenes[0].render(device, queue, request));
    let repeated = error(scenes[1].render(device, queue, request));
    assert_eq!(repeated, reference);
    for scene in scenes.iter() {
        if let Some(point) = scene.test_faults.fatal {
            assert!(reference.is_err(), "controlled fatal {point:?} must fail");
        } else {
            assert!(
                reference.is_ok(),
                "ordinary/isolated frame must complete: {reference:?}"
            );
        }
    }
    if reference.is_ok() {
        assert!(
            scenes[0]
                .shader_refusals()
                .keys()
                .all(|reason| !reason.contains("synthetic-scene-render")),
            "synthetic bake must draw rather than compare two refused paths"
        );
        let frame = (request.animation_time * 60.).round() as usize;
        if frame % 8 != 6 {
            let targets = scenes[0].targets().unwrap();
            let pixels = read_texture(device, queue, targets.output(), 4).unwrap();
            let at = (4 * targets.size()[0] as usize + 4) * 4;
            let expected = if (frame / 2).is_multiple_of(2) {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            };
            assert_eq!(
                &pixels[at..at + 4],
                &expected,
                "actual sampled bake at frame {frame}"
            );
        }
    }
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn scene_immediate_multiframe_bytes_state_and_pixels_are_deterministic() -> io::Result<()> {
    let fixture = Fixture::load("scene-immediate-multiframe")?;
    let sprites = sprites::Sprites::new();
    for samples in [1, 4] {
        let mut scenes = [
            fixture.scene_with_samples(samples)?,
            fixture.scene_with_samples(samples)?,
        ];
        for frame in 0..24 {
            let mut request = fixture.request(if frame < 12 { [256, 144] } else { [257, 145] });
            request.animation_time = frame as f32 / 60.;
            request.map_time = request.animation_time * 0.75;
            request.postfx = frame % 5 != 2;
            request.no_trees = frame % 7 == 4;
            request.no_water = frame % 9 == 5;
            if frame % 6 == 2 {
                request.camera.center[0] = 1.;
            }
            if frame % 6 == 3 {
                request.camera.center[0] = fixture.document.map.width as f32 - 1.;
            }
            if frame % 6 >= 4 {
                request.camera.height = if frame % 6 == 4 { 20. } else { 650. };
                request.camera.pose = Pose::Free(Free::new(
                    90_f32.to_radians(),
                    -5_f32.to_radians(),
                    Lens::MAP,
                ));
            }
            for scene in &mut scenes {
                scene.set_graphics(dashboard_core::protocol::MapGraphics {
                    samples,
                    full_resolution: frame % 8 < 4,
                    ..Default::default()
                });
            }
            render(&fixture, &mut scenes, &sprites, request);
            compare(
                &fixture,
                &scenes,
                &format!("samples {samples}, frame {frame}"),
            )?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn scene_immediate_later_fatal_refusals_and_recovery() -> io::Result<()> {
    let fixture = Fixture::load("scene-immediate-failures")?;
    let sprites = sprites::Sprites::new();
    for samples in [1, 4] {
        let mut scenes = [
            fixture.scene_with_samples(samples)?,
            fixture.scene_with_samples(samples)?,
        ];
        let base = fixture.request([256, 144]);
        render(&fixture, &mut scenes, &sprites, base);
        compare(&fixture, &scenes, "control")?;
        for (step, point) in [
            Point::Targets,
            Point::Shadow,
            Point::Trees,
            Point::Arrows,
            Point::Models,
            Point::Weather,
            Point::Post,
        ]
        .into_iter()
        .enumerate()
        {
            let request = Request {
                animation_time: (step + 1) as f32 / 30.,
                size: if point == Point::Targets {
                    [257, 145]
                } else {
                    base.size
                },
                ..base
            };
            for scene in &mut scenes {
                scene.test_faults.fatal = Some(point);
            }
            render(&fixture, &mut scenes, &sprites, request);
            assert!(
                scenes[0].test_faults.reached.borrow().contains(&point),
                "fixture must reach {point:?}"
            );
            compare(
                &fixture,
                &scenes,
                &format!("fatal {point:?}, samples {samples}"),
            )?;
            for scene in &mut scenes {
                scene.test_faults.fatal = None;
            }
            render(
                &fixture,
                &mut scenes,
                &sprites,
                Request {
                    animation_time: request.animation_time + 1. / 60.,
                    ..base
                },
            );
            compare(
                &fixture,
                &scenes,
                &format!("recover {point:?}, samples {samples}"),
            )?;
        }
        for pass in [
            passes::Pass::Underwater,
            passes::Pass::LandBorders,
            passes::Pass::CountryNames,
            passes::Pass::Sky,
        ] {
            for scene in &mut scenes {
                scene.test_faults.isolated = Some((pass, "controlled isolated refusal".into()));
            }
            render(&fixture, &mut scenes, &sprites, base);
            compare(
                &fixture,
                &scenes,
                &format!("isolated {pass:?}, samples {samples}"),
            )?;
            for scene in &mut scenes {
                scene.test_faults.isolated = None;
            }
            render(&fixture, &mut scenes, &sprites, base);
            compare(
                &fixture,
                &scenes,
                &format!("isolated recover {pass:?}, samples {samples}"),
            )?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn scene_province_frame_cut_spends_once_and_restores_an_unwalked_frame() -> io::Result<()> {
    use std::collections::{BTreeMap, BTreeSet};
    use viewer_gpu::province_schedule::Shared;
    use viewer_world::scene::SceneInput;

    let fixture = Fixture::load("scene-province-frame")?;
    let (device, queue) = (&fixture.state.device, &fixture.state.queue);
    for samples in [1, 4] {
        let mut scene = fixture.scene_with_samples(samples)?;
        let schedule = Shared::default();
        scene.apply(device, queue, SceneInput::ShareProvinceSchedule(&schedule))?;
        {
            let mut schedule = schedule.lock().unwrap();
            schedule.advance([1, 2, 101, 203], |id| id == 2, false);
            schedule.set_selected(BTreeSet::from([101]));
        }
        let expect = |pending, force| {
            let expected = (
                1u32,
                BTreeSet::from([1u32]),
                BTreeSet::from([2u32, 101]),
                BTreeSet::from([1u32]),
                pending,
                None::<u32>,
                BTreeSet::from([101u32]),
                BTreeMap::from([(101u32, force)]),
            );
            assert_eq!(
                format!("{:?}", schedule.lock().unwrap().presentation_state()),
                format!("{expected:?}"),
            );
        };
        expect(true, 2u8);
        let request = Request {
            animation_time: 0.,
            ..fixture.request([256, 144])
        };
        for _ in 0..2 {
            scene.apply(device, queue, SceneInput::CutProvinceFrame(0.))?;
            expect(false, 1);
        }
        let submissions = scene.test_faults.submissions.get();
        assert!(
            scene
                .render(
                    device,
                    queue,
                    Request {
                        size: [0, 0],
                        ..request
                    }
                )
                .is_err()
        );
        expect(true, 2);
        assert_eq!(scene.test_faults.submissions.get(), submissions);
        scene.apply(device, queue, SceneInput::CutProvinceFrame(0.))?;
        expect(false, 1);
        scene.render(device, queue, request)?;
        expect(false, 1);
        assert_eq!(scene.test_faults.submissions.get(), submissions + 1);
    }
    Ok(())
}
