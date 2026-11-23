use super::*;

#[test]
fn revision_and_shared_use_stamps_follow_explicit_invalidation() {
    let clock = GroupClock::default();
    let stamp = GroupStamp::new(7);
    let alias = stamp.clone();
    alias.set(9);
    assert_eq!(stamp.get(), 9);
    let before = clock.revision();
    clock.hint.store(true, Ordering::Release);
    clock.invalidate();
    assert_eq!(clock.revision(), before + 1);
    assert!(!clock.hint.load(Ordering::Acquire));
}

#[test]
fn a_racing_retirement_either_observes_the_ordered_touch_or_invalidates_it() {
    let clock = GroupClock::default();
    let constants = GroupStamp::new(0);
    let textures = GroupStamp::new(0);
    let gate = std::sync::Barrier::new(2);
    let (accepted, retired) = std::thread::scope(|scope| {
        let pruning = scope.spawn(|| {
            let mut retired = Vec::new();
            for _ in 0..4096 {
                gate.wait();
                clock.invalidate();
                retired.push(constants.get() + 1 < 3 || textures.get() + 1 < 3);
                gate.wait();
            }
            retired
        });
        let mut accepted = Vec::new();
        for _ in 0..4096 {
            constants.set(0);
            textures.set(0);
            let revision = clock.revision();
            gate.wait();
            accepted.push(clock.touch(revision, &constants, &textures, 3));
            gate.wait();
        }
        (accepted, pruning.join().unwrap())
    });
    for (accepted, retired) in accepted.into_iter().zip(retired) {
        assert!(
            !(accepted && retired),
            "a removed entry requires locked resolution"
        );
    }
}

#[test]
#[ignore = "requires a GPU"]
fn ordered_sessions_preserve_group_retirement_release_buffers_and_refusal_prefixes() {
    use crate::{Compiled, Shader, TextureKind};
    crate::tests::initialize_runtime_tables();
    let instance =
        crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let mut descriptor = wgpu::DeviceDescriptor::default();
    descriptor.required_limits.max_buffer_size = 4 << 20;
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    let shader = |wgsl: &str| Shader {
        wgsl: wgsl.into(),
        independent_draws: false,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    };
    let mut vertex = shader(
        "struct Block { value: vec4<f32> } @group(0) @binding(0) var<uniform> object: Block; @group(0) @binding(4) var<uniform> skin: Block; @vertex fn main() -> @builtin(position) vec4<f32> { return object.value + skin.value; }",
    );
    vertex.buffers.push(Buffer {
        binding: 0,
        size: 16,
        reads: std::iter::once(0..16).collect(),
    });
    vertex.buffers.push(Buffer {
        binding: 4, size: 16, reads: std::iter::once(0..16).collect(),
    });
    let mut pixel = shader(
        "@group(1) @binding(1) var image: texture_2d<f32>; @fragment fn main() -> @location(0) vec4<f32> { return textureLoad(image, vec2<i32>(0), 0); }",
    );
    pixel.textures.push((1, TextureKind::D2));
    let compiled = Compiled { vertex, pixel };
    let program = content_reader_gui::pdxshader::program::read_with(b"session.shader", &|path| {
        Some(if path == b"session.shader" {
            b"Effect Session { }".to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let state = State::authored(&program, &program.techniques[0]).unwrap();
    let mut pass = Pass::packed(&compiled, &state, wgpu::TextureFormat::Rgba8Unorm, 1);
    pass.vertices.clear();
    let pipeline = Pipeline::for_pass(
        &device,
        &compiled,
        &program,
        &program.techniques[0],
        &state,
        &pass,
    )
    .unwrap();
    let mut replacement = compiled.clone();
    replacement.vertex.wgsl = "struct Block { value: vec4<f32> } @group(0) @binding(4) var<uniform> skin: Block; @vertex fn main() -> @builtin(position) vec4<f32> { return skin.value; }".into();
    replacement.vertex.buffers.remove(0);
    let replacement = Pipeline::for_pass(
        &device, &replacement, &program, &program.techniques[0], &state, &pass,
    ).unwrap();
    let pipelines = [pipeline, replacement];
    let views: Vec<_> = (0..3)
        .map(|_| null_texture(&device, TextureKind::D2))
        .collect();
    let arenas = [Arena::new(&device), Arena::new(&device)];
    let mut held: [Option<BindingProgram>; 3] = [None, None, None];
    let mut storage = BindingSessionStorage::default();
    let mut selected = [usize::MAX; 3];
    let mut replacements = 0;
    let mut releases = 0;
    let mut growth = 0;
    for frame in 0_usize..24 {
        if frame == 9 {
            for arena in &arenas {
                arena.reset();
                arena.reset();
            }
        }
        let mut session = BindingSession::with_storage(&arenas[1], storage);
        for (draw, key) in [0, 1, 0, 2, 1, 2, 0].into_iter().enumerate() {
            let choice = (frame / 4 + key) % pipelines.len();
            let pipeline = &pipelines[choice];
            if selected[key] != choice {
                replacements += usize::from(held[key].is_some());
                selected[key] = choice;
                held[key] = None;
            }
            if draw == 3 && frame % 3 == 1 {
                pipeline.release_bind_groups();
                releases += 1;
            }
            if draw == 2 && frame % 6 == 2 {
                for value in [0x31, 0x52] {
                    let bytes = vec![value; 2 << 20];
                    for arena in &arenas {
                        arena.constants(&[(&bytes, 16)], &mut [0]).unwrap();
                    }
                }
                growth += 1;
            }
            let bytes: Vec<_> = [frame as u32, draw as u32, 0x80000000, 0x7fc01234]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            let skin = [bytes.as_slice(), bytes.as_slice()].concat();
            let data = |slot| match slot { 0 => Some(bytes.as_slice()), 4 => Some(skin.as_slice()), _ => None };
            let fresh = if held[key].is_none() {
                let (bindings, captured) = pipeline
                    .bind_program_with(&arenas[0], data, |_| Some(&views[key]))
                    .unwrap();
                held[key] = Some(captured);
                bindings
            } else {
                pipeline
                    .bind_with(&arenas[0], data, |_| Some(&views[key]))
                    .unwrap()
            };
            let captured = held[key].as_ref().unwrap();
            let actual = if draw % 2 == 0 {
                session.bind_ordered(key, captured, captured.constant_reads().iter().map(|spec| data(spec.binding)))
            } else { session.bind_with(key, captured, data) }.unwrap();
            assert_eq!(
                fresh.preparation_proof(),
                actual.preparation_proof(),
                "frame {frame}, draw {draw}"
            );
            assert!(fresh.same_resources(&actual));
            for missing in 0..captured.constant_reads().len() {
                let slot = captured.constant_reads()[missing].binding;
                let expected = captured.bind_with(&arenas[0], |binding| if binding == slot { None } else { data(binding) }).err();
                let before = arenas[1].proof_bytes_for_tests();
                assert_eq!(expected, session.bind_ordered(key, captured,
                    captured.constant_reads().iter().enumerate().map(|(index, spec)| if index == missing { None } else { data(spec.binding) })).err());
                assert_eq!(before, arenas[1].proof_bytes_for_tests(), "missing source must not stage a partial prefix");
                assert_eq!(expected, session.bind_ordered(key, captured,
                    captured.constant_reads().iter().take(missing).map(|spec| data(spec.binding))).err(), "short iterator, source {missing}");
                assert_eq!(before, arenas[1].proof_bytes_for_tests());
            }
            if draw == 4 && frame.is_multiple_of(5) {
                let oversized = vec![0x73; 4 << 20];
                let error = captured
                    .bind_with(&arenas[0], |_| Some(&oversized))
                    .err()
                    .unwrap();
                assert_eq!(
                    Some(error.clone()),
                    session.bind_ordered(key, captured, captured.constant_reads().iter().map(|_| Some(oversized.as_slice()))).err()
                );
                assert!(error.contains("buffer limit"));
                let retry = captured.bind_with(&arenas[0], data).unwrap();
                let actual = session.bind_ordered(key, captured, captured.constant_reads().iter().map(|spec| data(spec.binding))).unwrap();
                assert_eq!(retry.preparation_proof(), actual.preparation_proof());
                assert!(retry.same_resources(&actual));
            }
        }
        let expected = arenas[0].checkpoint();
        let actual = arenas[1].checkpoint();
        let trace = |writes: &super::super::FrozenBufferWrites| {
            writes
                .writes()
                .map(|(_, offset, bytes)| (offset, bytes.to_vec()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            trace(&expected),
            trace(&actual),
            "all staged bytes, frame {frame}"
        );
        expected.execute(&queue);
        actual.execute(&queue);
        storage = session.into_storage();
        assert!(
            storage
                .programs
                .iter()
                .all(|entry| entry.identity.is_none() && entry.buffers.is_empty())
        );
        for arena in &arenas {
            arena.reset();
        }
    }
    assert_eq!(releases, 8);
    assert_eq!(growth, 4);
    assert!(replacements >= 12, "same dense key must replace its effective program/recipe");
}
