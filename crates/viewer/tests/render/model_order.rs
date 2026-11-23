use content_reader_mesh::mesh;
use content_reader_layers::test_support::Stack;
use content_reader_image::native::Decode;
use std::sync::Arc;
use viewer_audio::{Content, Emitter, Engine, Frame, Group, Mix, Samples};
use viewer_gpu::camera::{Camera, MapDefines, MapSize, Pose};
use viewer_models::{entity_animation, ModelInstance, ModelLibrary, ModelPass, sound_events};
use viewer_weather_effects::entity_effects::Lights;
use viewer_scene_base::pointlights::Tiles;

#[test]
fn bake_permutations_preserve_event_light_and_audio_admission_and_random_streams() {
    let stack = Stack::new("model_order_audio");
    let base = stack.base();
    let entities = r#"
        entity = { name = zeta state = { name = idle event = { light = lamp sound = { soundeffect = limited } } } }
        entity = { name = alpha state = { name = idle event = { light = lamp sound = { soundeffect = limited } } } }
        entity = { name = middle state = { name = idle event = { light = lamp sound = { soundeffect = limited } } } }
        light = { name = lamp radius = 1 intensity = 1 color = { r = 1 g = 0.5 b = 0.2 } }
    "#;
    stack.write(&base, "gfx/entities/synthetic.asset", entities);
    stack.write(
        &base,
        "sound/synthetic.asset",
        r#"
        sound = { name = tone file = "tone.wav" }
        sound = { name = other file = "other.wav" }
        soundeffect = { name = limited sounds = { sound = tone sound = other }
            max_audible = 2 max_audible_behaviour = fail
            volume = 0.25 volume_random_offset = { 0 0.5 }
            delay_random_offset = { 0 0.02 } playbackrate_random_offset = { -25 25 } }
    "#,
    );
    let layers = stack.layers(&[]);
    let descriptors = mesh::parse_descriptors(entities.as_bytes(), "synthetic.asset").unwrap();
    let catalog = content_reader_sound::sound::Catalog::read(&layers).unwrap();
    assert!(catalog.notes.is_empty(), "{:?}", catalog.notes.lines());
    let content = Arc::new(Content::new(catalog, None));
    let effect = &content.catalog.effects[content.catalog.effect("limited").unwrap()];
    assert_eq!(effect.volume, 0.25);
    assert_eq!(
        (
            effect.playback_rate_offset.first,
            effect.playback_rate_offset.second
        ),
        (-0.25, 0.25)
    );
    let trace = [
        (
            [1_448_581_189u32, 612_099_500, 43_820_730, 1_778_769_266],
            [0x3bbacc46, 0x3f429ca7, 0x3f2a05de],
            "other",
        ),
        (
            [1_205_405_764u32, 366_710_553, 823_883_870, 947_171_856],
            [0x3b5fd286, 0x3f711b78, 0x3ef0e964],
            "other",
        ),
        (
            [88_954_627u32, 1_193_973_479, 305_671_545, 1_387_064_587],
            [0x3c362f97, 0x3f52382e, 0x3f12aced],
            "tone",
        ),
    ];
    for (words, expected, sound) in trace {
        let fraction = |word: u32| word as f32 / 2_147_483_648.;
        let values = [
            fraction(words[1]) * 0.02,
            1. + (fraction(words[2]) * 0.5 - 0.25),
            0.25 + fraction(words[3]) * 0.5,
        ];
        assert_eq!(values.map(f32::to_bits), expected);
        assert_eq!(
            if words[0] < 1_073_741_824 {
                "tone"
            } else {
                "other"
            },
            sound
        );
    }
    let lights = Lights::load(&layers, &mut Vec::new()).unwrap();
    let camera = Camera {
        center: [-100.; 2],
        height: 300.,
        map: MapSize {
            width: 512,
            height: 512,
        },
        defines: MapDefines::test(),
        pose: Pose::Map,
    };
    let names = ["zeta", "alpha", "middle"];
    let mut reference = None;
    for order in [
        names,
        ["middle", "alpha", "zeta"],
        ["alpha", "zeta", "middle"],
    ] {
        for label in ["general", "unit models"] {
            let shards = std::thread::scope(|scope| {
                let jobs: Vec<_> = order
                    .into_iter()
                    .map(|name| {
                        let (layers, descriptors) = (&layers, &descriptors);
                        scope.spawn(move || {
                            ModelLibrary::load(
                                layers,
                                Decode::default(),
                                descriptors,
                                &[name.into()],
                                &mut Vec::new(),
                            )
                            .unwrap()
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .map(|job| job.join().unwrap())
                    .collect::<Vec<_>>()
            });
            let mut library = ModelLibrary::default();
            for shard in shards {
                library.merge(shard);
            }
            let mut pass = ModelPass::new(&library, label);
            let placements = (0..86)
                .map(|at| {
                    let position = if at < 6 {
                        [4. + at as f32 * 0.1, 0., 4.]
                    } else {
                        let n = at - 6;
                        [28. + (n % 15) as f32 * 24., 0., 28. + (n / 15) as f32 * 24.]
                    };
                    (
                        library.slot(names[at % 3]).unwrap(),
                        ModelInstance {
                            position,
                            scale: 1.,
                            animation: entity_animation::AMBIENT_IDLE,
                            animation_seed: 80 + at as u32,
                            animation_identity: 100 + at as u32,
                            ..Default::default()
                        },
                    )
                })
                .collect();
            pass.update_placed(placements, 256.);
            let mut engine = Engine::new(7);
            engine.set_content(Some(Arc::clone(&content)));
            engine.log = Some(Vec::new());
            for (sound, definition) in content.catalog.sounds.iter().enumerate() {
                let base = if definition.name == "tone" {
                    2_000
                } else {
                    8_000
                };
                engine.provide(
                    sound,
                    Some(Arc::new(Samples {
                        data: (0..4_096)
                            .flat_map(|frame| {
                                [
                                    base + (frame % 257) as i16 * 37,
                                    base / 2 + (frame % 251) as i16 * 19,
                                ]
                            })
                            .collect(),
                    })),
                );
            }
            let mix = Mix {
                master: 1.,
                dev_master: 1.,
                units: 1.,
                ..Default::default()
            };
            let mut frames = Vec::new();
            for (frame, (time, elapsed)) in
                [(1., 0.), (1.003, 0.003), (1.004, 0.001), (1.006, 0.002)]
                    .into_iter()
                    .enumerate()
            {
                pass.animate(time, None, &[0.]);
                assert_eq!(pass.events.len(), 86);
                for event in &mut pass.events {
                    event.key.pass = 0;
                }
                let mut sounds = Vec::new();
                sound_events::collect(&pass.events, &mut Default::default(), &mut sounds, |_| {
                    false
                });
                assert_eq!(sounds.len(), 86);
                let mut tiles = Tiles::default();
                tiles.rebuild(camera, &lights.evaluate(&pass.events));
                assert_eq!(tiles.count, 64, "the game global limit is unchanged");
                assert_eq!(
                    &tiles.indices[..4],
                    &[0, 1, 2, 3],
                    "only the first four lights win a cell"
                );
                for (at, event) in pass.events.iter().enumerate() {
                    assert_eq!(event.seed, 80 + at as u32);
                    assert_eq!(
                        event.key.model,
                        library.meshes[library.slot(names[at % 3]).unwrap()].asset
                    );
                }
                let emitters = sounds
                    .iter()
                    .map(|sound| Emitter {
                        key: sound.key,
                        effect: Arc::clone(&sound.effect),
                        position: sound.position,
                        age: sound.age,
                        group: Group::Units,
                        restart: false,
                    })
                    .collect();
                engine.frame(&Frame {
                    emitters,
                    elapsed,
                    mix,
                    ..Default::default()
                });
                assert_eq!(engine.census().2, 86, "one audio instance per source event");
                assert_eq!(engine.census().3, 2, "max_audible is unchanged");
                let mut pcm = vec![0; 16];
                engine.render(&mut pcm);
                let expected: &[i16] = match frame {
                    0 | 1 => &[0; 16],
                    2 => &[
                        3764, 1882, 3780, 1890, 3796, 1898, 3813, 1907, 3829, 1915, 3846, 1923,
                        3862, 1932, 3878, 1940,
                    ],
                    3 => &[
                        9198, 4600, 9233, 4617, 9268, 4636, 9302, 4654, 9338, 4672, 9372, 4690,
                        9408, 4708, 9443, 4726,
                    ],
                    _ => unreachable!(),
                };
                assert_eq!(
                    pcm, expected,
                    "fixed delay/volume/rate trace, frame {frame}"
                );
                if frame >= 2 {
                    assert!(pcm.iter().any(|&sample| sample != 0));
                }
                frames.push((
                    pass.events
                        .iter()
                        .map(|event| (event.key.clone(), event.seed))
                        .collect::<Vec<_>>(),
                    sounds,
                    tiles.indices,
                    tiles.data.map(|row| row.map(f32::to_bits)),
                    pcm,
                ));
            }
            assert_eq!(engine.log.as_ref().unwrap().len(), 2);
            engine.frame(&Frame {
                mix,
                ..Default::default()
            });
            assert_eq!(engine.census().2, 0);
            let mut tail = vec![0; 16_384];
            engine.render(&mut tail);
            assert!(tail.iter().any(|&sample| sample != 0));
            assert_eq!(
                engine.census().3,
                0,
                "one-shots have finished before the probe"
            );
            let mut probe_frames = Vec::new();
            for (frame, elapsed) in [0., 0.010, 0.002].into_iter().enumerate() {
                engine.frame(&Frame {
                    emitters: vec![Emitter {
                        key: u64::MAX,
                        effect: Arc::from("limited"),
                        position: [401., 0., 401.],
                        age: 0.,
                        group: Group::Units,
                        restart: false,
                    }],
                    elapsed,
                    mix,
                    ..Default::default()
                });
                assert_eq!((engine.census().2, engine.census().3), (1, 1));
                let mut pcm = vec![0; 16];
                engine.render(&mut pcm);
                let expected: &[i16] = if frame < 2 {
                    &[0; 16]
                } else {
                    &[
                        1145, 572, 1163, 581, 1180, 590, 1198, 599, 1215, 608, 1232, 617, 1250,
                        626, 1267, 635,
                    ]
                };
                assert_eq!(
                    pcm, expected,
                    "fixed post-rejection random trace, probe frame {frame}"
                );
                if frame == 2 {
                    assert!(pcm.iter().any(|&sample| sample != 0));
                }
                probe_frames.push(pcm);
            }
            let starts = engine.log.unwrap();
            assert_eq!(starts.len(), 3);
            assert_eq!(
                starts
                    .iter()
                    .map(|start| start.sound.as_str())
                    .collect::<Vec<_>>(),
                ["other", "other", "tone"]
            );
            assert!(
                starts
                    .iter()
                    .all(|start| start.effect == "limited" && start.gain == 1.)
            );
            assert_eq!((starts[0].at, starts[1].at), (0., 0.));
            assert!(starts[2].at > 0.);
            assert_eq!(
                starts
                    .iter()
                    .map(|start| start.position)
                    .collect::<Vec<_>>(),
                vec![[4., 0., 4.], [4.1, 0., 4.], [401., 0., 401.]]
            );
            let observed = (frames, tail, probe_frames, starts);
            if let Some(expected) = &reference {
                assert_eq!(&observed, expected);
            } else {
                reference = Some(observed);
            }
        }
    }
}
