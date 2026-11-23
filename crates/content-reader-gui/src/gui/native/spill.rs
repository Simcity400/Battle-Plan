
pub(super) trait Layout {
    fn end(&self) -> (usize, String);
    fn write(&mut self, offset: usize, bits: u32) -> Result<(), String>;
}

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
                "holds a component {}, which {writer} writes: {failure}",
                index + 1
            ));
        }
    }
    Ok(())
}

pub(super) fn heap_end(size: usize, allocated: &str) -> (usize, String) {
    (
        size,
        format!("past the {size:#X} bytes {allocated} allocates: a heap overflow"),
    )
}

const INLINE: u64 = 15;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CxxString {
    buffer: [u8; 16],
    size: u64,
    capacity: u64,
    heap: Vec<Option<u8>>,
}

impl Default for CxxString {
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
                    self.capacity = capacity;
                    Ok(())
                }
            }
        }
    }

    pub(super) fn text(&self) -> Result<Vec<u8>, String> {
        self.text_within(0x20)
    }

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

    pub(super) fn is_empty(&self) -> bool {
        self.size == 0
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct VectorSite {
    pub what: &'static str,
    pub append: &'static str,
    pub readers: &'static str,
    pub destructor: &'static str,
}

#[derive(Clone, Debug)]
pub(super) struct Vector<T> {
    site: VectorSite,
    built: Vec<Option<T>>,
    block: usize,
    capacity: i32,
    count: i32,
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
                content_reader_parser::crt::truncate_f32(self.capacity as f32 * 1.5).max(count.wrapping_add(1));
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
                self.built[at] = Some(element);
            } else {
                self.built.push(Some(element));
            }
        }
        self.count = count.wrapping_add(1);
        Ok(())
    }

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

    #[test]
    fn a_string_takes_writes_as_its_bytes_lie() {
        content_reader_script::test_game::enter();
        let mut string = CxxString::default();
        string.assign(b"abcdef").unwrap();
        string.write(0, 0x41).unwrap();
        assert_eq!(string.text().unwrap(), b"A\0\0\0ef");
        string.write(16, 3).unwrap();
        assert_eq!(string.text().unwrap(), b"A\0\0");
        string.write(16, 17).unwrap();
        assert_eq!(string.text().unwrap()[16], 17);
        string.write(16, 33).unwrap();
        assert!(string.text().is_err());
        assert_eq!(string.text_within(0x70).unwrap().len(), 32);
        assert!(string.text_within(0x20).is_err());
        string.write(16, 2).unwrap();
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
