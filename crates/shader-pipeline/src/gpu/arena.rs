//! One frame's uploads. Constant blocks and repacked vertex streams are staged on the CPU,
//! bound at offsets into a few persistent buffers and written once per buffer before the
//! frame is submitted, so a draw creates neither a buffer nor a bind group.

use std::cell::RefCell;

/// Buffer size unless a single draw's uploads need more.
const CHUNK: u64 = 4 << 20;
/// Recent constant blocks compared before staging another copy: consecutive draws share the
/// scene and light-grid blocks, and each stage often receives the same bytes.
const RECENT: usize = 8;

pub struct Arena {
    device: wgpu::Device,
    alignment: u64,
    inner: RefCell<Inner>,
    /// Native packed geometry is copied into staging before the next draw reuses this storage.
    packed: RefCell<Vec<f32>>,
}

#[derive(Default)]
struct Inner {
    chunks: Vec<Chunk>,
    current: usize,
    frame: u64,
    recent: Vec<Recent>,
    /// The preceding block at each reflected position, still checked byte-for-byte.
    last: [Option<Recent>; super::CONSTANT_BINDINGS],
    /// Packed quads commonly share geometry while their constant blocks place them.
    vertices: Vec<Recent>,
    /// Writes to caller-owned buffers, kept until a flush, even across an unsubmitted frame.
    writes: Vec<(wgpu::Buffer, u64, Vec<u8>)>,
    /// Every draw refused through this arena ([`Arena::refuse`]).
    refusals: std::collections::BTreeSet<String>,
}

struct Chunk {
    buffer: wgpu::Buffer,
    staged: Vec<u8>,
    /// Staged bytes already written by a flush.
    flushed: usize,
}

#[derive(Clone, Copy)]
struct Recent {
    offset: usize,
    length: usize,
    reserved: usize,
}

/// A range of an arena buffer, bound by this frame's commands.
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
    /// Records that a draw was refused for `reason`, a stable name of what was refused and
    /// why: a map compares the refusals it makes with and without visual mods by these names.
    pub fn refuse(&self, reason: &str) {
        let mut inner = self.inner.borrow_mut();
        if !inner.refusals.contains(reason) {
            inner.refusals.insert(reason.to_owned());
        }
    }

    /// Every draw refused through this arena, by name ([`Self::refuse`]).
    pub fn refusals(&self) -> std::collections::BTreeSet<String> {
        self.inner.borrow().refusals.clone()
    }

    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            alignment: u64::from(device.limits().min_uniform_buffer_offset_alignment),
            inner: RefCell::default(),
            packed: RefCell::default(),
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

    /// Start a frame. Earlier uploads must no longer be bound by unsubmitted commands.
    pub fn reset(&self) {
        let mut inner = self.inner.borrow_mut();
        // Keep the buffers the last frame used and one spare; bind groups on the others
        // expire from pipeline caches with them.
        let keep = (inner.current + 2).min(inner.chunks.len());
        inner.chunks.truncate(keep);
        for chunk in &mut inner.chunks {
            chunk.staged.clear();
            chunk.flushed = 0;
        }
        inner.current = 0;
        inner.frame += 1;
        inner.recent.clear();
        inner.last.fill(None);
        inner.vertices.clear();
    }

    /// Frames started so far, which identifies the frame being encoded.
    pub fn frame(&self) -> u64 {
        self.inner.borrow().frame
    }

    /// Write the uploads staged since the last flush, before submitting the commands that bind
    /// them. Earlier bindings stay valid until `reset`.
    pub fn flush(&self, queue: &wgpu::Queue) {
        let mut inner = self.inner.borrow_mut();
        for (buffer, offset, bytes) in inner.writes.drain(..) {
            queue.write_buffer(&buffer, offset, &bytes);
        }
        for chunk in &mut inner.chunks {
            if chunk.staged.len() == chunk.flushed {
                continue;
            }
            chunk.staged.resize(align(chunk.staged.len(), 4), 0);
            queue.write_buffer(
                &chunk.buffer,
                chunk.flushed as u64,
                &chunk.staged[chunk.flushed..],
            );
            chunk.flushed = chunk.staged.len();
        }
    }

    /// One draw's constant blocks in a single buffer, each zero-filled to its declared size
    /// (D3D11 reads past a bound buffer's bytes as zero). Writes each block's offset.
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
        if inner.room(&self.device, total + alignment)? {
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

    /// Write `bytes` into a caller-owned buffer (`COPY_DST`, four-byte aligned) at the next flush,
    /// before the commands that read them.
    pub fn write(&self, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        self.inner
            .borrow_mut()
            .writes
            .push((buffer.clone(), offset, bytes.to_vec()));
    }

    /// A vertex stream. An empty stream binds four zero bytes, which no attribute reads.
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
        if inner.room(&self.device, size + 4)? {
            inner.recent.clear();
            inner.last.fill(None);
            inner.vertices.clear();
        }
        let chunk = &mut inner.chunks[inner.current];
        let offset = align(chunk.staged.len(), 4);
        chunk.staged.resize(offset, 0);
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
    /// Make the current buffer able to take `size` more bytes; true if it changed.
    fn room(&mut self, device: &wgpu::Device, size: usize) -> Result<bool, String> {
        let mut changed = false;
        loop {
            let replace = match self.chunks.get(self.current) {
                Some(chunk) if (chunk.staged.len() + size) as u64 <= chunk.buffer.size() => {
                    return Ok(changed);
                }
                Some(chunk) if !chunk.staged.is_empty() || self.current + 1 < self.chunks.len() => {
                    self.current += 1;
                    changed = true;
                    continue;
                }
                // An unused last buffer too small for this upload is replaced.
                found => found.is_some(),
            };
            let bytes = CHUNK.max(size as u64);
            if bytes > device.limits().max_buffer_size {
                return Err(format!(
                    "a draw's {bytes} upload bytes exceed the device's {}-byte buffer limit",
                    device.limits().max_buffer_size
                ));
            }
            let chunk = Chunk {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("native upload arena"),
                    size: bytes,
                    usage: wgpu::BufferUsages::UNIFORM
                        | wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                staged: Vec::with_capacity(bytes as usize),
                flushed: 0,
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

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Reuse needs the recent block's bytes and reserved zeros to cover the request.
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
        // The same native scratch allocation changes between draws.
        bytes[8] = 2;
        arena.constants(&[(&bytes, 16)], &mut offsets).unwrap();
        assert_ne!(offsets[0], original);
        let mutated = offsets[0];
        arena.constants(&[(&bytes, 64)], &mut offsets).unwrap();
        assert_ne!(offsets[0], mutated);
        // Neither a vertex upload that changes chunks nor a reset retains old offsets.
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
        // Equal float values are insufficient: signed zero and NaN payload bytes survive.
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
}
