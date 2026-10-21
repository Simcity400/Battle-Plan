use super::*;

type RecentState = (usize, usize, usize);
type WriteTrace = Vec<(usize, u64, Vec<u8>)>;

#[test]
fn checkpoint_suffix_bytes_are_owned_aligned_and_unchanged_across_frames() {
    let mut saved = Vec::new();
    for frame in 0..128_u8 {
        let mut staged = Vec::new();
        let mut flushed = 0;
        let mut expected_offset = 0;
        for length in 1..=9 {
            let bytes = (0..length)
                .map(|index| frame.wrapping_add(index as u8))
                .collect::<Vec<_>>();
            staged.extend_from_slice(&bytes);
            let mut expected = bytes;
            while expected.len() % 4 != 0 {
                expected.push(0);
            }
            let frozen = checkpoint_bytes(&mut staged, &mut flushed).unwrap();
            assert_eq!(frozen, (expected_offset, expected.clone()));
            expected_offset += expected.len() as u64;
            assert_eq!(flushed, expected_offset as usize);
            assert!(checkpoint_bytes(&mut staged, &mut flushed).is_none());
            saved.push((frozen, expected));
        }
        staged.fill(255);
        staged.clear();
        assert!(
            saved
                .iter()
                .all(|((_, actual), expected)| actual == expected)
        );
    }
    assert!(checkpoint_bytes(&mut Vec::new(), &mut 0).is_none());
}

#[derive(Debug, PartialEq, Eq)]
struct State {
    current: usize,
    frame: u64,
    chunks: Vec<(u64, Vec<u8>, usize)>,
    recent: Vec<RecentState>,
    last: Vec<Option<RecentState>>,
    vertices: Vec<RecentState>,
    writes: WriteTrace,
    refusals: std::collections::BTreeSet<String>,
}

fn destination(inner: &Inner, caller: &wgpu::Buffer, buffer: &wgpu::Buffer) -> usize {
    if buffer == caller {
        0
    } else {
        1 + inner
            .chunks
            .iter()
            .position(|chunk| &chunk.buffer == buffer)
            .expect("checkpoint must retain the original destination")
    }
}

fn state(arena: &Arena, caller: &wgpu::Buffer) -> State {
    let inner = arena.inner.borrow();
    let recent = |r: &Recent| (r.offset, r.length, r.reserved);
    State {
        current: inner.current,
        frame: inner.frame,
        chunks: inner
            .chunks
            .iter()
            .map(|chunk| (chunk.buffer.size(), chunk.staged.clone(), chunk.flushed))
            .collect(),
        recent: inner.recent.iter().map(recent).collect(),
        last: inner
            .last
            .iter()
            .map(|entry| entry.as_ref().map(recent))
            .collect(),
        vertices: inner.vertices.iter().map(recent).collect(),
        writes: inner
            .writes
            .iter()
            .map(|(buffer, offset, bytes)| {
                (destination(&inner, caller, buffer), *offset, bytes.clone())
            })
            .collect(),
        refusals: inner.refusals.clone(),
    }
}

/// Independent oracle for the unchanged flush, not a call through checkpoint_bytes.
fn pending(arena: &Arena, caller: &wgpu::Buffer) -> WriteTrace {
    let inner = arena.inner.borrow();
    let mut writes = inner
        .writes
        .iter()
        .map(|(buffer, offset, bytes)| {
            (destination(&inner, caller, buffer), *offset, bytes.clone())
        })
        .collect::<Vec<_>>();
    for (index, chunk) in inner.chunks.iter().enumerate() {
        if chunk.flushed < chunk.staged.len() {
            let mut bytes = chunk.staged[chunk.flushed..].to_vec();
            while bytes.len() % 4 != 0 {
                bytes.push(0);
            }
            writes.push((index + 1, chunk.flushed as u64, bytes));
        }
    }
    writes
}

fn frozen_trace(
    arena: &Arena,
    caller: &wgpu::Buffer,
    frozen: &super::super::FrozenBufferWrites,
) -> WriteTrace {
    let inner = arena.inner.borrow();
    frozen
        .writes()
        .map(|(buffer, offset, bytes)| {
            (destination(&inner, caller, buffer), offset, bytes.to_vec())
        })
        .collect()
}

fn same_slice(arenas: &[Arena; 2], callers: &[wgpu::Buffer; 2], slices: &[Slice; 2]) {
    let positions = std::array::from_fn::<_, 2, _>(|index| {
        let slice = &slices[index];
        (
            destination(
                &arenas[index].inner.borrow(),
                &callers[index],
                &slice.buffer,
            ),
            slice.offset,
            slice.size,
        )
    });
    assert_eq!(positions[0], positions[1]);
}

#[test]
#[ignore = "requires a GPU"]
fn checkpoints_match_flush_bytes_offsets_refusals_prefix_and_carry_over_multiple_frames() {
    let _serial = crate::tests::GPU_TESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let mut descriptor = wgpu::DeviceDescriptor::default();
    descriptor.required_limits.max_buffer_size = CHUNK;
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    let arenas = [Arena::new(&device), Arena::new(&device)];
    let callers = std::array::from_fn::<_, 2, _>(|_| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("checkpoint caller destination"),
            size: 64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    });
    let mut image = vec![0; 64];
    let mut captures = Vec::new();
    // Keep a spare empty chunk after reset, so oversized refusals also exercise that path.
    for arena in &arenas {
        arena.vertices(&vec![3; CHUNK as usize - 4]).unwrap();
        arena.vertices(&[4; 8]).unwrap();
    }
    arenas[0].flush(&queue);
    arenas[1].checkpoint().execute(&queue);
    queue.submit([]);
    for frame in 0..24_u8 {
        // Every previous checkpoint's dependent copy has already been submitted. No reset
        // occurs while the consumer still owns an unsubmitted use of arena destinations.
        for arena in &arenas {
            arena.reset();
        }
        assert_eq!(
            state(&arenas[0], &callers[0]),
            state(&arenas[1], &callers[1])
        );
        let bits = [
            0_u32,
            0x8000_0000,
            0x7fc0_0001 + u32::from(frame),
            0xffc0_4321,
        ];
        let constants = bytemuck::cast_slice::<_, u8>(&bits);
        let mut offsets = [[u32::MAX; 2]; 2];
        let constant_buffers = std::array::from_fn::<_, 2, _>(|index| {
            arenas[index]
                .constants(&[(constants, 32), (&[frame; 5], 12)], &mut offsets[index])
                .unwrap()
        });
        assert_eq!(offsets[0], offsets[1]);
        for index in 0..2 {
            assert_eq!(
                destination(
                    &arenas[index].inner.borrow(),
                    &callers[index],
                    &constant_buffers[index]
                ),
                1
            );
            arenas[index].write(&callers[index], 0, &[frame; 8]);
            arenas[index].write(&callers[index], 4, &[frame.wrapping_add(1); 4]);
        }
        let geometry = std::array::from_fn(|index| arenas[index].vertices(&[frame; 7]).unwrap());
        same_slice(&arenas, &callers, &geometry);
        for boundary in 0..3 {
            let expected = pending(&arenas[0], &callers[0]);
            assert_eq!(expected, pending(&arenas[1], &callers[1]));
            let frozen = arenas[1].checkpoint();
            assert_eq!(frozen.frame(), arenas[0].frame());
            assert_eq!(frozen_trace(&arenas[1], &callers[1], &frozen), expected);
            arenas[0].flush(&queue);
            // Mutating producer scratch after sealing cannot change this boundary's bytes.
            let appended = std::array::from_fn(|index| {
                arenas[index]
                    .vertices(&[frame, boundary, 19, 23, 29])
                    .unwrap()
            });
            same_slice(&arenas, &callers, &appended);
            assert_eq!(frozen_trace(&arenas[1], &callers[1], &frozen), expected);
            frozen.execute(&queue);
            for (destination, offset, bytes) in &expected {
                if *destination == 0 {
                    image[*offset as usize..*offset as usize + bytes.len()].copy_from_slice(bytes);
                }
            }
            assert_eq!(
                state(&arenas[0], &callers[0]),
                state(&arenas[1], &callers[1])
            );
            let reads = std::array::from_fn::<_, 2, _>(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("checkpoint caller readback"),
                    size: 64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            for index in 0..2 {
                encoder.copy_buffer_to_buffer(&callers[index], 0, &reads[index], 0, 64);
            }
            queue.submit([encoder.finish()]);
            captures.push((reads, image.clone()));
            if boundary == 0 {
                // A reached prefix survives a later failed main draw. Refusal must leave
                // staged offsets/dedup intact, and retry must not invent an earlier write.
                for constant_upload in [false, true] {
                    let errors = std::array::from_fn::<_, 2, _>(|index| {
                        let before = state(&arenas[index], &callers[index]);
                        let error = if constant_upload {
                            let mut refused_offsets = [u32::MAX];
                            let result = arenas[index]
                                .constants(&[(constants, CHUNK + 1)], &mut refused_offsets);
                            assert_eq!(refused_offsets, [u32::MAX]);
                            result.unwrap_err()
                        } else {
                            arenas[index]
                                .vertices(&vec![8; CHUNK as usize + 1])
                                .unwrap_err()
                        };
                        assert_eq!(state(&arenas[index], &callers[index]), before);
                        arenas[index].refuse(&error);
                        error
                    });
                    assert_eq!(errors[0], errors[1]);
                }
                let retried =
                    std::array::from_fn(|index| arenas[index].vertices(&[frame; 7]).unwrap());
                same_slice(&arenas, &callers, &retried);
                assert_eq!(retried[0].offset, geometry[0].offset);
                for (index, arena) in arenas.iter().enumerate() {
                    arena.write(&callers[index], 12, &[frame.wrapping_add(2); 4]);
                }
            }
        }
        if frame % 4 == 0 {
            for arena in &arenas {
                arena.vertices(&vec![frame; CHUNK as usize - 4]).unwrap();
                arena.vertices(&[frame; 8]).unwrap();
            }
        }
        // This failed/unsubmitted suffix must NOT execute. Chunk suffixes reset as today,
        // whereas the caller-buffer write carries into the next frame's first checkpoint.
        for (index, arena) in arenas.iter().enumerate() {
            arena.write(&callers[index], 24, &[frame.wrapping_add(3); 4]);
        }
        assert_eq!(
            state(&arenas[0], &callers[0]),
            state(&arenas[1], &callers[1])
        );
    }
    for (reads, _) in &captures {
        for read in reads {
            read.slice(..)
                .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        }
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    for (index, (reads, expected)) in captures.iter().enumerate() {
        for read in reads {
            let bytes = read.slice(..).get_mapped_range().unwrap();
            assert_eq!(&bytes[..], expected, "checkpoint {index}");
            drop(bytes);
            read.unmap();
        }
    }
}

#[test]
#[ignore = "requires a GPU"]
fn reached_checkpoint_executes_after_preparation_error_without_flushing_unreached_suffix() {
    let _serial = crate::tests::GPU_TESTS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (device, queue) = crate::tests::device();
    let arena = Arena::new(&device);
    let caller = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("failed attempt caller destination"),
        size: 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    arena.write(&caller, 0, &[1, 2, 3, 4]);
    arena.vertices(&[5, 6, 7]).unwrap();
    let reached = arena.checkpoint();
    // Preparation fails AFTER sealing the reached boundary. The owner must still replay
    // it, but must not accidentally seal later writes from the abandoned main segment.
    arena.write(&caller, 4, &[9, 10, 11, 12]);
    let main = Err::<(), _>("synthetic later preparation failure");
    assert_eq!(main, Err("synthetic later preparation failure"));
    reached.execute(&queue);
    let readback = |expected: &[u8]| {
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("failed attempt prefix readback"),
            size: 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&caller, 0, &read, 0, 8);
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = read.slice(..).get_mapped_range().unwrap();
        assert_eq!(&bytes[..], expected);
        drop(bytes);
        read.unmap();
    };
    readback(&[1, 2, 3, 4, 0, 0, 0, 0]);
    // Only after the dependent commands submit may preparation reset the arena.
    arena.reset();
    let carried = arena.checkpoint();
    assert_eq!(carried.writes().count(), 1);
    assert_eq!(carried.writes().next().unwrap().2, &[9, 10, 11, 12]);
    carried.execute(&queue);
    readback(&[1, 2, 3, 4, 9, 10, 11, 12]);
    assert_eq!(arena.checkpoint().writes().count(), 0);
}
