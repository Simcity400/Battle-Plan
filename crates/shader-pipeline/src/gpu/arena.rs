
use std::cell::RefCell;

const CHUNK: u64 = 4 << 20;
const RECENT: usize = 8;
const BELT_CHUNK: u64 = 1 << 20;

pub struct Arena {
    device: wgpu::Device,
    alignment: u64,
    storage_limit: u64,
    inner: RefCell<Inner>,
    packed: RefCell<Vec<f32>>,
    belt: RefCell<Option<wgpu::util::StagingBelt>>,
}

#[derive(Default)]
struct Inner {
    chunks: Vec<Chunk>,
    current: usize,
    frame: u64,
    waiting_for_build: bool,
    recent: Vec<Recent>,
    last: [Option<Recent>; super::CONSTANT_BINDINGS],
    vertices: Vec<Recent>,
    writes: Vec<(wgpu::Buffer, u64, Vec<u8>)>,
    refusals: std::collections::BTreeSet<String>,
    written: u64,
}

struct Chunk {
    buffer: wgpu::Buffer,
    staged: Vec<u8>,
    flushed: usize,
    held: Vec<u8>,
}

use super::unheld;

#[derive(Clone, Copy)]
struct Recent {
    offset: usize,
    length: usize,
    reserved: usize,
}

#[derive(Clone, Debug)]
pub struct Slice {
    pub buffer: wgpu::Buffer,
    pub offset: u64,
    pub size: u64,
}

impl Slice {
    pub fn slice(&self) -> wgpu::BufferSlice<'_> {
        self.buffer.slice(self.offset..self.offset + self.size)
    }
}

impl Arena {
    pub fn wait_for_build(&self) {
        self.inner.borrow_mut().waiting_for_build = true;
    }

    pub fn waiting_for_build(&self) -> bool {
        self.inner.borrow().waiting_for_build
    }

    pub fn refuse(&self, reason: &str) {
        let mut inner = self.inner.borrow_mut();
        if !inner.refusals.contains(reason) {
            inner.refusals.insert(reason.to_owned());
        }
    }

    pub fn refusals(&self) -> std::collections::BTreeSet<String> {
        self.inner.borrow().refusals.clone()
    }

    pub fn written_bytes(&self) -> u64 {
        self.inner.borrow().written
    }

    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            alignment: u64::from(device.limits().min_uniform_buffer_offset_alignment),
            storage_limit: u64::from(device.limits().max_storage_buffer_binding_size),
            inner: RefCell::default(),
            packed: RefCell::default(),
            belt: RefCell::default(),
        }
    }

    pub(crate) fn packed_vertices(
        &self,
        packing: &crate::vertices::Packing,
        vertices: &crate::vertices::Vertices<'_>,
    ) -> Result<Slice, String> {
        let mut packed = self.packed.borrow_mut();
        packing.pack_into(vertices, &mut packed)?;
        self.vertices(bytemuck::cast_slice(&packed))
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn reset(&self) {
        let mut inner = self.inner.borrow_mut();
        let keep = (inner.current + 2).min(inner.chunks.len());
        inner.chunks.truncate(keep);
        for chunk in &mut inner.chunks {
            chunk.staged.clear();
            chunk.flushed = 0;
        }
        inner.current = 0;
        inner.frame += 1;
        inner.waiting_for_build = false;
        inner.recent.clear();
        inner.last.fill(None);
        inner.vertices.clear();
    }

    pub fn frame(&self) -> u64 {
        self.inner.borrow().frame
    }

    pub fn flush(&self, queue: &wgpu::Queue) {
        self.flush_with(|buffer, offset, bytes| {
            crate::gpu::stats::write_buffer(queue, buffer, offset, bytes);
        });
    }

    pub fn flush_into(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut belt = self.belt.borrow_mut();
        let belt = belt.get_or_insert_with(|| wgpu::util::StagingBelt::new(self.device.clone(), BELT_CHUNK));
        self.flush_with(|buffer, offset, bytes| {
            for (at, piece) in (offset..).step_by(BELT_CHUNK as usize).zip(bytes.chunks(BELT_CHUNK as usize)) {
                crate::gpu::stats::belt_write(belt, encoder, buffer, at, piece);
            }
        });
        belt.finish_and_recall_on_submit(encoder);
    }

    fn flush_with(&self, mut write: impl FnMut(&wgpu::Buffer, u64, &[u8])) {
        let mut inner = self.inner.borrow_mut();
        let mut written = 0;
        for (buffer, offset, bytes) in std::mem::take(&mut inner.writes) {
            written += bytes.len() as u64;
            write(&buffer, offset, &bytes);
        }
        for chunk in &mut inner.chunks {
            if chunk.staged.len() == chunk.flushed {
                continue;
            }
            chunk.staged.resize(align(chunk.staged.len(), 4), 0);
            let (from, to) = (chunk.flushed, chunk.staged.len());
            chunk.flushed = to;
            let known = chunk.held.len().clamp(from, to);
            for span in unheld(from, &chunk.staged[from..known], &chunk.held[from..known]) {
                written += (span.end - span.start) as u64;
                write(&chunk.buffer, span.start as u64, &chunk.staged[span.clone()]);
                chunk.held[span.clone()].copy_from_slice(&chunk.staged[span]);
            }
            if to > known {
                written += (to - known) as u64;
                write(&chunk.buffer, known as u64, &chunk.staged[known..to]);
                chunk.held.resize(to, 0);
                chunk.held[known..to].copy_from_slice(&chunk.staged[known..to]);
            }
        }
        inner.written += written;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn checkpoint(&self) -> super::FrozenBufferWrites {
        let mut inner = self.inner.borrow_mut();
        let mut writes = Vec::with_capacity(inner.writes.len() + inner.chunks.len());
        writes.append(&mut inner.writes);
        for chunk in &mut inner.chunks {
            if let Some((offset, bytes)) = checkpoint_bytes(&mut chunk.staged, &mut chunk.flushed) {
                chunk.held.truncate(offset as usize);
                writes.push((chunk.buffer.clone(), offset, bytes));
            }
        }
        super::FrozenBufferWrites::new(inner.frame, writes)
    }

    pub(crate) fn constants(
        &self,
        blocks: &[(&[u8], u64)],
        offsets: &mut [u32],
    ) -> Result<wgpu::Buffer, String> {
        let alignment = self.alignment as usize;
        let reserved = |bytes: &[u8], size: u64| bytes.len().max(size as usize);
        let total = blocks
            .iter()
            .map(|(bytes, size)| align(reserved(bytes, *size), alignment))
            .sum::<usize>();
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        if inner.room(&self.device, total + alignment, 1, u64::MAX)? {
            inner.recent.clear();
            inner.last.fill(None);
            inner.vertices.clear();
        }
        let chunk = &mut inner.chunks[inner.current];
        for (index, (&(bytes, size), slot)) in blocks.iter().zip(offsets).enumerate() {
            let reserved = reserved(bytes, size);
            let matches = |recent: &Recent| {
                recent.length == bytes.len()
                    && recent.reserved >= reserved
                    && chunk.staged[recent.offset..recent.offset + recent.length] == *bytes
            };
            let found = inner.last[index]
                .filter(matches)
                .or_else(|| inner.recent.iter().find(|recent| matches(recent)).copied());
            let recent = match found {
                Some(recent) => recent,
                None => {
                    let offset = align(chunk.staged.len(), alignment);
                    chunk.staged.resize(offset, 0);
                    super::stats::staged(bytes.len());
                    chunk.staged.extend_from_slice(bytes);
                    chunk.staged.resize(offset + reserved, 0);
                    if inner.recent.len() == RECENT {
                        inner.recent.remove(0);
                    }
                    let recent = Recent {
                        offset,
                        length: bytes.len(),
                        reserved,
                    };
                    inner.recent.push(recent);
                    recent
                }
            };
            inner.last[index] = Some(recent);
            *slot = u32::try_from(recent.offset).map_err(|_| "constant offset exceeds u32")?;
        }
        Ok(chunk.buffer.clone())
    }

    pub fn write(&self, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        super::stats::staged(bytes.len());
        self.inner
            .borrow_mut()
            .writes
            .push((buffer.clone(), offset, bytes.to_vec()));
    }

    pub(crate) fn storage(&self, bytes: &[u8], stride: usize) -> Result<Slice, String> {
        let alignment = stride;
        let size = align(bytes.len().max(4), 4);
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        if inner.room(&self.device, size, alignment, self.storage_limit)? {
            inner.recent.clear();
            inner.last.fill(None);
            inner.vertices.clear();
        }
        let chunk = &mut inner.chunks[inner.current];
        let offset = align(chunk.staged.len(), alignment);
        chunk.staged.resize(offset, 0);
        super::stats::staged(bytes.len());
        chunk.staged.extend_from_slice(bytes);
        chunk.staged.resize(offset + size, 0);
        Ok(Slice {
            buffer: chunk.buffer.clone(),
            offset: offset as u64,
            size: size as u64,
        })
    }

    pub fn vertices(&self, bytes: &[u8]) -> Result<Slice, String> {
        let size = align(bytes.len().max(4), 4);
        let mut inner = self.inner.borrow_mut();
        let inner = &mut *inner;
        if let Some(chunk) = inner.chunks.get(inner.current)
            && let Some(recent) = inner.vertices.iter().find(|recent| {
                recent.length == bytes.len()
                    && chunk.staged[recent.offset..recent.offset + recent.length] == *bytes
            })
        {
            return Ok(Slice {
                buffer: chunk.buffer.clone(),
                offset: recent.offset as u64,
                size: size as u64,
            });
        }
        if inner.room(&self.device, size, 4, u64::MAX)? {
            inner.recent.clear();
            inner.last.fill(None);
            inner.vertices.clear();
        }
        let chunk = &mut inner.chunks[inner.current];
        let offset = align(chunk.staged.len(), 4);
        chunk.staged.resize(offset, 0);
        super::stats::staged(bytes.len());
        chunk.staged.extend_from_slice(bytes);
        chunk.staged.resize(offset + size, 0);
        if inner.vertices.len() == RECENT {
            inner.vertices.remove(0);
        }
        inner.vertices.push(Recent {
            offset,
            length: bytes.len(),
            reserved: size,
        });
        Ok(Slice {
            buffer: chunk.buffer.clone(),
            offset: offset as u64,
            size: size as u64,
        })
    }
}

impl Inner {
    fn room(
        &mut self,
        device: &wgpu::Device,
        size: usize,
        alignment: usize,
        limit: u64,
    ) -> Result<bool, String> {
        let original = self.current;
        let mut changed = false;
        loop {
            let replace = match self.chunks.get(self.current) {
                Some(chunk)
                    if (align(chunk.staged.len(), alignment) + size) as u64
                        <= chunk.buffer.size().min(limit) =>
                {
                    return Ok(changed);
                }
                Some(chunk) if !chunk.staged.is_empty() || self.current + 1 < self.chunks.len() => {
                    self.current += 1;
                    changed = true;
                    continue;
                }
                found => found.is_some(),
            };
            let bytes = CHUNK.min(limit).max(size as u64);
            if bytes > device.limits().max_buffer_size.min(limit) {
                self.current = original;
                return Err(format!(
                    "a draw's {bytes} upload bytes exceed the device's {}-byte buffer limit",
                    device.limits().max_buffer_size.min(limit)
                ));
            }
            let chunk = Chunk {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("native upload arena"),
                    size: bytes,
                    usage: wgpu::BufferUsages::UNIFORM
                        | wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                staged: Vec::with_capacity(bytes as usize),
                flushed: 0,
                held: Vec::new(),
            };
            if replace {
                self.chunks[self.current] = chunk;
            } else {
                self.chunks.push(chunk);
            }
            changed = true;
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn checkpoint_bytes(staged: &mut Vec<u8>, flushed: &mut usize) -> Option<(u64, Vec<u8>)> {
    if staged.len() == *flushed {
        return None;
    }
    staged.resize(align(staged.len(), 4), 0);
    let offset = *flushed;
    let bytes = staged[offset..].to_vec();
    *flushed = staged.len();
    Some((offset as u64, bytes))
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GPU"]
    fn belt_writes_land_before_the_commands_after_them_frame_after_frame() {
        let (device, queue) = crate::tests::device();
        let arena = Arena::new(&device);
        let buffer = |usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 2 * BELT_CHUNK,
                usage,
                mapped_at_creation: false,
            })
        };
        let target = buffer(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let read = buffer(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
        let mut expected = vec![0_u8; 2 * BELT_CHUNK as usize];
        let large = (0..5).map(|step| (8, BELT_CHUNK as usize + 8 + 64 * step));
        for (frame, (offset, length)) in [(0, 64), (16, 32)].into_iter().chain(large).enumerate() {
            arena.reset();
            let bytes: Vec<u8> = (0..length).map(|at| (at * 7 + frame * 31 + at / BELT_CHUNK as usize * 101) as u8).collect();
            arena.write(&target, offset as u64, &bytes);
            expected[offset..offset + length].copy_from_slice(&bytes);
            let mut uploads = device.create_command_encoder(&Default::default());
            arena.flush_into(&mut uploads);
            let mut after = device.create_command_encoder(&Default::default());
            after.copy_buffer_to_buffer(&target, 0, &read, 0, 2 * BELT_CHUNK);
            queue.submit([uploads.finish(), after.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
            device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
            let held = read.slice(..).get_mapped_range().unwrap();
            assert!(held[..] == expected[..], "frame {frame}");
            drop(held);
            read.unmap();
        }
        assert_eq!(
            arena.written_bytes(),
            64 + 32 + (0..5).map(|step| BELT_CHUNK + 8 + 64 * step).sum::<u64>(),
            "the belt writes what a queue flush writes"
        );
        let mut belt = arena.belt.borrow_mut();
        let belt = belt.as_mut().expect("the belt was made");
        let four = wgpu::BufferSize::new(4).expect("nonzero");
        let _ = belt.allocate(four, four);
        let described = format!("{belt:?}");
        let held: u64 = ["active_chunks: ", "closed_chunks: ", "free_chunks: "]
            .iter()
            .map(|field| {
                let count = &described[described.find(field).expect("the belt names its chunks") + field.len()..];
                count[..count.find([',', ' ']).unwrap_or(count.len())].parse::<u64>().expect("a count")
            })
            .sum();
        assert_eq!(held, 2, "growing writes reuse the belt's buffers: {described}");
    }

    #[test]
    #[ignore = "requires a GPU"]
    fn stages_aligned_zero_filled_blocks_reuses_recent_bytes_and_resets() {
        let (device, _) = crate::tests::device();
        let arena = Arena::new(&device);
        let alignment = device.limits().min_uniform_buffer_offset_alignment;
        let mut offsets = [0; 3];
        let first = arena
            .constants(&[(&[1; 8], 16), (&[1; 8], 16), (&[2; 8], 64)], &mut offsets)
            .unwrap();
        assert_eq!(offsets, [0, 0, alignment]);
        {
            let staged = &arena.inner.borrow().chunks[0].staged;
            assert_eq!(staged[..16], [[1; 8], [0; 8]].concat());
            assert_eq!(
                staged[alignment as usize..][..64],
                [[2; 8].as_slice(), &[0; 56]].concat()
            );
        }
        arena
            .constants(&[(&[1; 8], 32)], &mut offsets[..1])
            .unwrap();
        assert_eq!(offsets[0], (alignment + 64).div_ceil(alignment) * alignment);
        let vertices = arena.vertices(&[7; 6]).unwrap();
        assert_eq!((vertices.offset % 4, vertices.size), (0, 8));
        assert_eq!(arena.vertices(&[]).unwrap().size, 4);
        let frame = arena.frame();
        arena.reset();
        assert_eq!(arena.frame(), frame + 1);
        let again = arena.constants(&[(&[3; 4], 4)], &mut offsets[..1]).unwrap();
        assert_eq!((offsets[0], again), (0, first));
    }
    #[test]
    #[ignore = "requires a GPU"]
    fn reused_constants_stay_recent_among_changing_object_blocks() {
        let (device, _) = crate::tests::device();
        let arena = Arena::new(&device);
        let scene = [1; 32];
        let mut offsets = [0; 2];
        for value in 2..32 {
            arena
                .constants(&[(&scene, 32), (&[value; 16], 16)], &mut offsets)
                .unwrap();
            assert_eq!(
                offsets[0], 0,
                "a used scene block must not be evicted by object blocks"
            );
        }
    }

    #[test]
    #[ignore = "requires a GPU"]
    fn preceding_block_cache_checks_mutated_bytes_sizes_and_chunk_changes() {
        let (device, _) = crate::tests::device();
        let arena = Arena::new(&device);
        let mut bytes = [1; 16];
        let mut offsets = [0];
        let first = arena.constants(&[(&bytes, 16)], &mut offsets).unwrap();
        let original = offsets[0];
        arena.constants(&[(&bytes, 16)], &mut offsets).unwrap();
        assert_eq!(offsets[0], original);
        bytes[8] = 2;
        arena.constants(&[(&bytes, 16)], &mut offsets).unwrap();
        assert_ne!(offsets[0], original);
        let mutated = offsets[0];
        arena.constants(&[(&bytes, 64)], &mut offsets).unwrap();
        assert_ne!(offsets[0], mutated);
        arena.vertices(&vec![9; CHUNK as usize + 1]).unwrap();
        let next = arena.constants(&[(&bytes, 64)], &mut offsets).unwrap();
        assert_ne!(next, first);
        arena.reset();
        arena.constants(&[(&bytes, 64)], &mut offsets).unwrap();
        assert_eq!(offsets[0], 0);
    }

    #[test]
    #[ignore = "requires a GPU"]
    fn identical_vertices_reuse_bytes_until_a_chunk_change_or_reset() {
        let (device, queue) = crate::tests::device();
        let arena = Arena::new(&device);
        let bytes = [1, 2, 3, 4, 5, 6];
        let first = arena.vertices(&bytes).unwrap();
        let check = |slice: Slice| {
            assert_eq!(slice.buffer, first.buffer);
            assert_eq!(slice.offset, first.offset);
            assert_eq!(slice.size, first.size);
        };
        check(arena.vertices(&bytes).unwrap());
        assert_eq!(arena.inner.borrow().chunks[0].staged.len(), 8);
        let mut offsets = [0];
        arena.constants(&[(&[7; 4], 4)], &mut offsets).unwrap();
        arena.flush(&queue);
        check(arena.vertices(&bytes).unwrap());
        assert_ne!(
            arena.vertices(&[1, 2, 3, 4, 5, 7]).unwrap().offset,
            first.offset
        );
        let zero = arena.vertices(&0_f32.to_ne_bytes()).unwrap();
        assert_ne!(
            zero.offset,
            arena.vertices(&(-0_f32).to_ne_bytes()).unwrap().offset
        );
        let nan = arena
            .vertices(&f32::from_bits(0x7fc0_0001).to_ne_bytes())
            .unwrap();
        assert_ne!(
            nan.offset,
            arena
                .vertices(&f32::from_bits(0x7fc0_0002).to_ne_bytes())
                .unwrap()
                .offset
        );
        arena.vertices(&vec![9; CHUNK as usize + 1]).unwrap();
        assert_ne!(arena.vertices(&bytes).unwrap().buffer, first.buffer);
        arena.reset();
        let changed = arena.vertices(&[8; 6]).unwrap();
        assert_eq!(changed.offset, 0);
        let again = arena.vertices(&bytes).unwrap();
        assert_ne!(again.offset, changed.offset);
        let inner = arena.inner.borrow();
        assert_eq!(
            &inner.chunks[0].staged[again.offset as usize..][..6],
            &bytes
        );
    }

    #[test]
    #[ignore = "requires a GPU"]
    fn refused_upload_preserves_current_chunk_and_recent_offsets_for_same_frame_retry() {
        let _serial = crate::tests::GPU_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let instance =
            crate::gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let mut descriptor = wgpu::DeviceDescriptor::default();
        descriptor.required_limits.max_buffer_size = CHUNK;
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
        for spare in [true, false] {
            for constants_upload in [false, true] {
                let arena = Arena::new(&device);
                if spare {
                    arena.vertices(&vec![3; CHUNK as usize - 4]).unwrap();
                    arena.vertices(&[4; 8]).unwrap();
                    assert_eq!(arena.inner.borrow().chunks.len(), 2);
                    arena.reset();
                }
                let bytes = [7; 16];
                let vertices = [9; 12];
                let mut offsets = [0];
                let buffer = arena.constants(&[(&bytes, 32)], &mut offsets).unwrap();
                let original_offsets = offsets;
                let geometry = arena.vertices(&vertices).unwrap();
                arena.flush(&queue);
                let snapshot = || {
                    let inner = arena.inner.borrow();
                    let recent = |r: Recent| (r.offset, r.length, r.reserved);
                    (
                        inner.current,
                        inner
                            .chunks
                            .iter()
                            .map(|chunk| {
                                (chunk.buffer.clone(), chunk.staged.clone(), chunk.flushed)
                            })
                            .collect::<Vec<_>>(),
                        inner.recent.iter().copied().map(recent).collect::<Vec<_>>(),
                        inner.last.map(|r| r.map(recent)),
                        inner
                            .vertices
                            .iter()
                            .copied()
                            .map(recent)
                            .collect::<Vec<_>>(),
                    )
                };
                let before = snapshot();
                let error = if constants_upload {
                    let mut refused_offsets = [u32::MAX];
                    let error = arena
                        .constants(&[(&bytes, CHUNK + 1)], &mut refused_offsets)
                        .unwrap_err();
                    assert_eq!(refused_offsets, [u32::MAX]);
                    error
                } else {
                    arena.vertices(&vec![8; CHUNK as usize + 1]).unwrap_err()
                };
                assert!(error.contains("buffer limit"), "{error}");
                let retry = arena.constants(&[(&bytes, 32)], &mut offsets).unwrap();
                assert_eq!(retry, buffer);
                assert_eq!(offsets, original_offsets);
                let retry = arena.vertices(&vertices).unwrap();
                assert_eq!(retry.buffer, geometry.buffer);
                assert_eq!(retry.offset, geometry.offset);
                assert_eq!(retry.size, geometry.size);
                assert_eq!(
                    snapshot(),
                    before,
                    "spare={spare}, constants={constants_upload}"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "arena/checkpoint_tests.rs"]
mod checkpoint_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "arena/proof.rs"]
mod proof;
