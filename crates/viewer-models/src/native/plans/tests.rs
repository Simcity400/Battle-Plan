use crate::native::plans::*;

#[test]
fn immutable_sources_match_copying_scratch_and_pending_through_refusals_and_fatal_prefixes() {
    let mut random = 0x8f7a211acb122099_u64;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random as u32
    };
    let mut bank = Scratch::default();
    for frame in 0_usize..512 {
        let mut reference = ObjectConstants::default();
        for bytes in reference.bytes.chunks_exact_mut(4) {
            bytes.copy_from_slice(&next().to_le_bytes());
        }
        if !frame.is_multiple_of(3) {
            reference.pixel = Some([frame as u8; 208]);
        }
        let mut actual = ObjectConstants {
            bytes: reference.bytes,
            pixel: reference.pixel,
            retained: Default::default(),
        };
        bank.begin(&actual);
        let mut expected_slots: [Option<Vec<u8>>; CONSTANT_BINDINGS] = std::array::from_fn(|_| None);
        let mut expected_frame = None;
        let mut expected_grid = false;
        let mut masked = PendingSources::default();
        let frames = [vec![frame as u8; 416], vec![(frame + 1) as u8; 416]];
        let views = frames.each_ref().map(Vec::as_slice);
        let grid = [frame as u8; 64];
        let poses: Vec<Option<ActorPoses>> = (0..3)
            .map(|index| {
                Some(Arc::from([mesh_pose::Actor {
                    world: Ok(mesh_pose::IDENTITY),
                    pose: Ok(mesh_pose::Pose {
                        bones: vec![
                            [[f32::from_bits([0x80000000, 0x7fc01234, 0xffc05678][index]); 4];
                                4];
                            index + 1
                        ],
                        joints: Vec::new(),
                    }),
                }]))
            })
            .collect();
        for descriptor in 0..64 {
            let before = (reference.bytes, reference.pixel);
            let checkpoint = bank.checkpoint();
            let world = std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(next())));
            let userdata = (!next().is_multiple_of(3))
                .then(|| std::array::from_fn(|_| f32::from_bits(next())));
            reference.bytes[..64].copy_from_slice(bytemuck::bytes_of(&world));
            if let Some(data) = userdata {
                reference.bytes[64..96].copy_from_slice(bytemuck::bytes_of(&data));
                reference.pixel = Some(reference.bytes);
            }
            bank.update(&world, userdata);
            assert_eq!(bank.vertex(), reference.bytes);
            assert_eq!(
                bank.pixel(),
                reference.pixel.as_ref().map(|bytes| bytes.as_slice())
            );
            let outcome = next() % 11;
            if outcome < 3 {
                (reference.bytes, reference.pixel) = before;
                bank.restore(checkpoint);
            } else if outcome == 3 && descriptor > 32 {
                bank.sync(&mut actual);
                assert_eq!(actual.bytes, reference.bytes);
                assert_eq!(actual.pixel, reference.pixel);
                break;
            } else {
                let view = descriptor % 2;
                let instance = descriptor % poses.len();
                let mut buffers = [None; CONSTANT_BINDINGS];
                buffers[0] = Some(views[view]);
                buffers[1] = Some(views[view]);
                buffers[2] = Some(bank.vertex());
                buffers[3] = bank.pixel();
                if next().is_multiple_of(2) {
                    buffers[4] = Some(bytemuck::cast_slice(
                        &poses[instance].as_ref().unwrap()[0]
                            .pose
                            .as_ref()
                            .unwrap()
                            .bones,
                    ));
                }
                buffers[5] = Some(&grid);
                for (slot, bytes) in buffers.iter().enumerate() {
                    if (slot < 2 && expected_frame == Some(view)) || (slot == 5 && expected_grid) {
                        continue;
                    }
                    if let Some(bytes) = bytes { expected_slots[slot] = Some(bytes.to_vec()); }
                }
                expected_frame = Some(view);
                expected_grid |= buffers[5].is_some();
                let mask = buffers.iter().enumerate().fold(0, |mask, (slot, bytes)| mask | if bytes.is_some() { 1 << slot } else { 0 });
                masked.constants_mask(mask, view, &bank, instance, 0);
                for (slot, expected) in expected_slots.iter().enumerate() {
                    let expected = expected.as_deref();
                    let observed = masked.sources[slot].map(|source| match source {
                        Source::Frame(view) => views[view],
                        Source::Grid => &grid,
                        Source::Object(source) => bank.bytes(source),
                        Source::Skin(instance, node) => bytemuck::cast_slice(
                            &poses[instance].as_ref().unwrap()[node]
                                .pose
                                .as_ref()
                                .unwrap()
                                .bones,
                        ),
                    });
                    assert_eq!(
                        expected, observed,
                        "frame {frame}, descriptor {descriptor}, slot {slot}"
                    );
                }
                assert_eq!((expected_frame, expected_grid), (masked.state.frame, masked.state.grid));
            }
            bank.sync(&mut actual);
            assert_eq!(actual.bytes, reference.bytes);
            assert_eq!(actual.pixel, reference.pixel);
        }
    }
}

#[test]
fn pending_sources_keep_immutable_views_and_last_bound_snapshots() {
    let mut scratch = Scratch::default();
    scratch.begin(&ObjectConstants::default());
    scratch.update(&[[4.; 4]; 4], Some([5.; 8]));
    let mut pending = PendingSources::default();
    pending.constants_mask(0b11_1111, 0, &scratch, 2, 3);
    let Some(Source::Object(first_vertex)) = pending.sources[2] else { panic!("VS origin"); };
    let Some(Source::Object(first_pixel)) = pending.sources[3] else { panic!("PS origin"); };
    assert_eq!(first_vertex, first_pixel);
    assert!(matches!(pending.sources[4], Some(Source::Skin(2, 3))));
    assert!(matches!(pending.sources[5], Some(Source::Grid)));
    scratch.update(&[[6.; 4]; 4], None);
    pending.constants_mask(0b00_0111, 0, &scratch, 8, 9);
    assert!(matches!(pending.sources[0], Some(Source::Frame(0))));
    assert!(matches!(pending.sources[1], Some(Source::Frame(0))));
    let Some(Source::Object(vertex)) = pending.sources[2] else { panic!("next VS origin"); };
    let Some(Source::Object(pixel)) = pending.sources[3] else { panic!("held PS origin"); };
    assert_ne!(vertex, first_vertex);
    assert_eq!(pixel, first_pixel);
    assert_eq!(&scratch.bytes(vertex)[..64], bytemuck::bytes_of(&[[6_f32; 4]; 4]));
    assert_eq!(&scratch.bytes(pixel)[..64], bytemuck::bytes_of(&[[4_f32; 4]; 4]));
    assert_eq!(&scratch.bytes(pixel)[64..96], bytemuck::bytes_of(&[5_f32; 8]));
    assert!(matches!(pending.sources[4], Some(Source::Skin(2, 3))));
    pending.constants_mask(0b00_0111, 1, &scratch, 8, 9);
    assert!(matches!(pending.sources[0], Some(Source::Frame(1))));
    assert!(matches!(pending.sources[1], Some(Source::Frame(1))));
    assert!(matches!(pending.sources[5], Some(Source::Grid)));
    assert_eq!((pending.state.frame, pending.state.grid), (Some(1), true));
}
