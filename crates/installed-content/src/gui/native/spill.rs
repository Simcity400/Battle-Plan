//! In-object spills: the one place the game's unbounded component readers land.
//!
//! `read_axis_components@140ACEBE0` (each number through `read_float_token@1424B0320`) and
//! `CReader::ReadIntComponents@1402C7A50` (each `%i`) write component `i` at `start + 4 i`, with
//! no count bound, into whatever object (or stack frame) the caller hands them; a component the
//! scan stores nothing for is skipped but still counted. Where such a write lands is a fact of
//! the object's layout, so each receiving object describes its bytes as a [`Layout`]: the field a
//! 4-byte write lands on, whether anything reads that field before the game rewrites it, and,
//! only where the write corrupts something the game later reads in an undefined way or leaves
//! the allocation, the native access that fails. [`spill`] applies the components in order and
//! stops at the first failure.
//!
//! `heap_allocate@1421B4B50` is `HeapAlloc(heap, 0, size)` (`XOR EDX,EDX` at `14255FD3C`):
//! allocations are not zeroed, so a field no constructor writes holds no defined value.
//!
//! [`CxxString`] is the MSVC `std::string` such a write can land in: its 16-byte buffer (the
//! text inline while the capacity is at most 15, else the heap pointer), its size (`+0x10`) and
//! its capacity (`+0x18`), as `msvc_string::assign_bytes_candidate@140129790` and the
//! destructors use them.

/// An object's bytes as the component readers can reach them.
pub(super) trait Layout {
    /// The first offset past what the write may touch (the allocation's size, or the frame's
    /// live end), and what lies there.
    fn end(&self) -> (usize, String);
    /// One 4-byte write of `bits` at `offset` (below [`Layout::end`]): stored into the field it
    /// lands on, or nothing when no reader sees it before it is rewritten, or `Err` naming the
    /// native access that fails.
    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String>;
}

/// The components `writer` wrote from `start`, in order, into `object`: `Err` at the first that
/// fails, naming it.
pub(super) fn spill(
    object: &mut impl Layout,
    writer: &str,
    start: usize,
    components: impl IntoIterator<Item = (usize, u32)>,
) -> Result<(), String> {
    for (index, bits) in components {
        let offset = start + 4 * index;
        let (end, past) = object.end();
        let failure = if offset + 4 > end {
            Some(past)
        } else {
            object.write(offset, bits).err()
        };
        if let Some(failure) = failure {
            return Err(format!(
                "holds a component {}, which {writer} writes at +{offset:#X}: {failure}",
                index + 1
            ));
        }
    }
    Ok(())
}

/// The end of a heap object: past it, a heap overflow.
pub(super) fn heap_end(size: usize, allocated: &str) -> (usize, String) {
    (
        size,
        format!(
            "past the {size:#X} bytes {allocated} allocates (heap_allocate@1421B4B50): a heap \
             overflow"
        ),
    )
}

/// The capacity of a string's inline buffer (MSVC `_BUF_SIZE - 1`).
const INLINE: u64 = 15;

/// An MSVC `std::string` (0x20 bytes) while the component readers can write into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CxxString {
    /// `_Bx`: the text while the capacity is at most 15; else the heap pointer in its first
    /// eight bytes, which the replay does not hold.
    buffer: [u8; 16],
    size: u64,
    capacity: u64,
    /// The heap block's bytes while the capacity is above 15 (`capacity + 1` of them), `None`
    /// where nothing has written them (the allocation is not zeroed).
    heap: Vec<Option<u8>>,
}

impl Default for CxxString {
    /// The constructors' empty string: a zeroed buffer, size 0, capacity 15.
    fn default() -> Self {
        Self {
            buffer: [0; 16],
            size: 0,
            capacity: INLINE,
            heap: Vec::new(),
        }
    }
}

impl CxxString {
    /// `msvc_string::assign_bytes_candidate@140129790`: into the current buffer (the text and a
    /// NUL) when it fits the capacity, else into a new heap block of `max(len | 15,
    /// capacity * 3 / 2)` bytes and one, whose pointer the buffer then holds. Only a capacity a
    /// component wrote below 15 can make that new capacity 15, which reads the pointer as text.
    pub(super) fn assign(&mut self, text: &[u8]) -> Result<(), String> {
        let len = text.len() as u64;
        if len > self.capacity {
            let grown = self.capacity + self.capacity / 2;
            let capacity = (len | INLINE).max(grown);
            if capacity <= INLINE {
                return Err(format!(
                    "a {len}-byte text outgrows the capacity a component wrote ({}), so the \
                     string moves to a heap block whose pointer its buffer then holds, while its \
                     new capacity ({capacity}) reads that buffer as its text: a heap address the \
                     replay does not hold",
                    self.capacity
                ));
            }
            self.heap = vec![None; capacity as usize + 1];
            self.capacity = capacity;
        }
        let bytes = text.iter().copied().chain([0]);
        if self.capacity > INLINE {
            for (slot, byte) in self.heap.iter_mut().zip(bytes) {
                *slot = Some(byte);
            }
        } else {
            for (slot, byte) in self.buffer.iter_mut().zip(bytes) {
                *slot = byte;
            }
        }
        self.size = len;
        Ok(())
    }

    /// A 4-byte write at `offset` within the string.
    pub(super) fn write(&mut self, offset: usize, bits: u32) -> Result<(), String> {
        let bytes = bits.to_le_bytes();
        match offset {
            0 | 4 if self.capacity > INLINE => Err("the string's heap pointer, which its \
                 destructor frees and its readers read through: a heap address the replay does \
                 not hold"
                .into()),
            0 | 4 | 8 | 12 => {
                self.buffer[offset..offset + 4].copy_from_slice(&bytes);
                Ok(())
            }
            16 | 20 => {
                let mut size = self.size.to_le_bytes();
                size[offset - 16..offset - 12].copy_from_slice(&bytes);
                self.size = u64::from_le_bytes(size);
                Ok(())
            }
            _ => {
                let mut capacity = self.capacity.to_le_bytes();
                capacity[offset - 24..offset - 20].copy_from_slice(&bytes);
                let capacity = u64::from_le_bytes(capacity);
                if capacity == self.capacity {
                    Ok(())
                } else if self.capacity > INLINE || capacity > INLINE {
                    Err(format!(
                        "the string's capacity ({} to {capacity}), which decides whether its \
                         first eight bytes are its text or a heap pointer (a heap address the \
                         replay does not hold, or text read as one) and how its destructor frees \
                         the block",
                        self.capacity
                    ))
                } else {
                    // Still inline: only a later assignment's fit test sees it.
                    self.capacity = capacity;
                    Ok(())
                }
            }
        }
    }

    /// The text its readers see when it is the object's last field: [`CxxString::text_within`]
    /// with nothing after it.
    pub(super) fn text(&self) -> Result<Vec<u8>, String> {
        self.text_within(0x20)
    }

    /// The text its readers see, `room` bytes of the allocation lying from the string's start:
    /// its `size` bytes from its buffer or its block.
    ///
    /// Inline, a size past the 16-byte buffer reads on over the string's own size and capacity
    /// and then the object's following fields. Those readers take the text to its first NUL
    /// (the texture path and the click sound are C strings to the resolver and the sound
    /// player), and a size that stays inside a heap object is below 2^16, so its own third byte
    /// (index 18) is 0: what the readers see ends inside the string's own 0x20 bytes, and the
    /// bytes after it are not kept. A size past `room` reads past the allocation.
    pub(super) fn text_within(&self, room: usize) -> Result<Vec<u8>, String> {
        if self.capacity <= INLINE {
            let object: Vec<u8> = self
                .buffer
                .iter()
                .copied()
                .chain(self.size.to_le_bytes())
                .chain(self.capacity.to_le_bytes())
                .collect();
            return usize::try_from(self.size)
                .ok()
                .filter(|size| *size <= room)
                .map(|size| object[..size.min(object.len())].to_vec())
                .ok_or_else(|| {
                    format!(
                        "the string's size is {}, and its readers read that many bytes from its \
                         inline buffer, past the {room:#X} bytes the allocation holds from the \
                         string: a read past the heap block",
                        self.size
                    )
                });
        }
        usize::try_from(self.size)
            .ok()
            .and_then(|size| self.heap.get(..size))
            .and_then(|bytes| bytes.iter().copied().collect::<Option<Vec<u8>>>())
            .ok_or_else(|| {
                format!(
                    "the string's size is {}, and its readers read that many bytes from its \
                     heap block, past what was written into it (not zeroed) or past the block",
                    self.size
                )
            })
    }

    /// Whether the text is empty (`size == 0`), as a reader testing the size sees it.
    pub(super) fn is_empty(&self) -> bool {
        self.size == 0
    }
}

/// Where a [`Vector`] sits and who uses it, for the failures' reasons.
#[derive(Clone, Copy, Debug)]
pub(super) struct VectorSite {
    /// The vector, with its offset: `"the animation records' vector (+0x118)"`.
    pub what: &'static str,
    /// The native append (growth through the allocator, or a copy to `data + count × size`).
    pub append: &'static str,
    /// The readers of `count` elements from the data pointer.
    pub readers: &'static str,
    /// The destructor that frees the data through the allocator when it is not null.
    pub destructor: &'static str,
}

/// The engine's vector (`string_vector::initialize@14011DA30`: data `+0x0`, capacity `+0x8`,
/// count `+0xC`, allocator `+0x10`) as its appends and the component readers leave it. An append
/// (`string_vector::push_string@14011D230` and its copies): when the count equals the capacity,
/// allocator, the `count` old elements moved and the old block freed through it; else the
/// element copied to `data + count × size`. Then the count is raised.
#[derive(Clone, Debug)]
pub(super) struct Vector<T> {
    site: VectorSite,
    /// The heap block's elements, in order: `None` where a clear destroyed one. The data pointer
    /// is null while none has been appended.
    built: Vec<Option<T>>,
    /// The block's capacity in elements, as allocated.
    block: usize,
    /// The capacity and count as held.
    capacity: i32,
    count: i32,
    /// The key whose component wrote the allocator pointer, which holds the image's `14306D170`
    /// at a load address the replay does not hold (the image is `DYNAMIC_BASE`).
    allocator: Option<&'static str>,
}

impl<T: Clone> Vector<T> {
    pub(super) fn new(site: VectorSite) -> Self {
        Self {
            site,
            built: Vec::new(),
            block: 0,
            capacity: 0,
            count: 0,
            allocator: None,
        }
    }

    /// A component's 4-byte write at `offset` within the vector, by `writer`.
    pub(super) fn write(
        &mut self,
        offset: usize,
        bits: u32,
        writer: &'static str,
    ) -> Result<(), String> {
        match offset {
            0 | 4 => {
                if bits != 0 || !self.built.is_empty() {
                    return Err(format!(
                        "{}'s data pointer, which {} frees through the allocator when it is not \
                         null and {} read the elements through: a heap address the replay does \
                         not hold, or bits that are none",
                        self.site.what, self.site.destructor, self.site.readers
                    ));
                }
            }
            8 => self.capacity = bits as i32,
            12 => self.count = bits as i32,
            _ => self.allocator = Some(writer),
        }
        Ok(())
    }

    /// The native append of `element`.
    pub(super) fn append(&mut self, element: T) -> Result<(), String> {
        let count = self.count;
        if count == self.capacity {
            if let Some(key) = self.allocator {
                return Err(format!(
                    "is appended after {key} wrote {}'s allocator pointer, through which {} \
                     allocates the grown block",
                    self.site.what, self.site.append
                ));
            }
            let grown =
                crate::crt::truncate_f32(self.capacity as f32 * 1.5).max(count.wrapping_add(1));
            if grown <= 0 {
                return Err(format!(
                    "is appended with {}'s count and capacity at {count}, so {} allocates \
                     {grown} elements and copies through the null block",
                    self.site.what, self.site.append
                ));
            }
            let moved = usize::try_from(count).ok().filter(|count| {
                *count <= self.built.len() && self.built[..*count].iter().all(Option::is_some)
            });
            let Some(moved) = moved else {
                return Err(format!(
                    "is appended with {}'s count at {count}, so {} moves elements it never \
                     constructed or a clear destroyed",
                    self.site.what, self.site.append
                ));
            };
            self.built.truncate(moved);
            self.built.push(Some(element));
            self.block = grown as usize;
            self.capacity = grown;
        } else {
            let at = usize::try_from(count)
                .ok()
                .filter(|at| !self.built.is_empty() && *at < self.block && *at <= self.built.len());
            let Some(at) = at else {
                return Err(format!(
                    "is appended with {}'s count at {count} and capacity at {}, so {} copies \
                     it outside a block of {} elements (null while none is built) or past the \
                     elements built before it",
                    self.site.what, self.capacity, self.site.append, self.block
                ));
            };
            if at < self.built.len() {
                // Copied over the old element, which is not destroyed.
                self.built[at] = Some(element);
            } else {
                self.built.push(Some(element));
            }
        }
        self.count = count.wrapping_add(1);
        Ok(())
    }

    /// A clear that destroys the `count` elements from the data pointer, then sets the count to
    /// 0 (`string_vector::destroy_range@1401207D0`); the block stays.
    pub(super) fn clear(&mut self, clearer: &str) -> Result<(), String> {
        if let Ok(count) = usize::try_from(self.count)
            && count > 0
            && self
                .built
                .get(..count)
                .is_none_or(|elements| !elements.iter().all(Option::is_some))
        {
            return Err(format!(
                "{}'s count is {count} with {} built, and {clearer} destroys that many \
                 elements from its data pointer (null while none is built)",
                self.site.what,
                self.built.len()
            ));
        }
        let destroyed = usize::try_from(self.count).unwrap_or(0);
        for element in self.built.iter_mut().take(destroyed) {
            *element = None;
        }
        self.count = 0;
        Ok(())
    }

    /// The elements the readers see (`count` from the data pointer), or the failure the
    /// destructor or those readers meet.
    pub(super) fn visible(&self) -> Result<Vec<T>, String> {
        if let Some(key) = self.allocator
            && !self.built.is_empty()
        {
            return Err(format!(
                "{key} wrote {}'s allocator pointer, through which {} frees its block",
                self.site.what, self.site.destructor
            ));
        }
        match usize::try_from(self.count) {
            Err(_) | Ok(0) => Ok(Vec::new()),
            Ok(count) => self
                .built
                .get(..count)
                .and_then(|elements| elements.iter().cloned().collect::<Option<Vec<T>>>())
                .ok_or_else(|| {
                    format!(
                        "{}'s count is {count} with {} in its block, and {} read that many \
                         elements from its data pointer (null while none is built), past those \
                         constructed or over ones a clear destroyed",
                        self.site.what,
                        self.built.len(),
                        self.site.readers
                    )
                }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inline writes change the text as far as the size reaches; a size past the buffer and a
    /// capacity crossing 15 are the game's undefined reads; a long assignment moves to the heap,
    /// whose pointer a write corrupts.
    #[test]
    fn a_string_takes_writes_as_its_bytes_lie() {
        let mut string = CxxString::default();
        string.assign(b"abcdef").unwrap();
        string.write(0, 0x41).unwrap();
        assert_eq!(string.text().unwrap(), b"A\0\0\0ef");
        string.write(16, 3).unwrap();
        assert_eq!(string.text().unwrap(), b"A\0\0");
        // Past the buffer the size's own bytes are read, then past the string.
        string.write(16, 17).unwrap();
        assert_eq!(string.text().unwrap()[16], 17);
        string.write(16, 33).unwrap();
        assert!(string.text().is_err());
        // Inside a larger object the readers stop at the size's own zero byte.
        assert_eq!(string.text_within(0x70).unwrap().len(), 32);
        assert!(string.text_within(0x20).is_err());
        string.write(16, 2).unwrap();
        // A capacity written to 7: a nine-byte text grows it to max(9 | 15, 7 + 3) = 15, which
        // still reads the buffer (now the block's pointer) as the text.
        string.write(24, 7).unwrap();
        assert!(string.clone().assign(b"123456789").is_err());
        assert!(string.clone().assign(b"1234567").is_ok());
        let mut long = CxxString::default();
        long.assign(b"0123456789abcdefg").unwrap();
        assert_eq!(long.text().unwrap(), b"0123456789abcdefg");
        assert!(long.clone().write(0, 0).is_err());
        assert!(long.write(8, 5).is_ok());
        assert_eq!(long.text().unwrap(), b"0123456789abcdefg");
        long.write(16, 20).unwrap();
        assert!(long.text().is_err());
        assert!(CxxString::default().write(24, 16).is_err());
    }
}
