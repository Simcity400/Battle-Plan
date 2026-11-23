
use engine_sites::Rva;
use crate::{Memory, read, read_slice};

pub const DATA: usize = 0;
pub const LENGTH: usize = 16;
pub const CAPACITY: usize = 24;
pub const INLINE_CAPACITY: u64 = 15;
pub const STRIDE: usize = 32;
pub const ALIGNMENT: usize = 8;
pub const CONSTRUCT_BYTES_CANDIDATE_RVA: Rva = Rva(0x0011_f030);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text<const N: usize> {
    pub bytes: [u8; N],
    pub len: usize,
}

impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> Text<N> {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(self.as_bytes()).ok()
    }
}

pub fn read_text<const N: usize>(memory: &impl Memory, address: usize) -> Option<Text<N>> {
    let mut value = Text::default();
    read_text_inner::<N, false>(memory, address, &mut value)?;
    Some(value)
}

pub fn read_text_prefetched<const N: usize>(
    memory: &impl Memory,
    address: usize,
) -> Option<Text<N>> {
    let mut value = Text::default();
    read_text_prefetched_into(memory, address, &mut value)?;
    Some(value)
}

pub fn read_text_prefetched_into<const N: usize>(
    memory: &impl Memory,
    address: usize,
    value: &mut Text<N>,
) -> Option<()> {
    read_text_inner::<N, true>(memory, address, value)
}

#[inline]
fn read_text_inner<const N: usize, const PREFETCHED: bool>(
    memory: &impl Memory,
    address: usize,
    value: &mut Text<N>,
) -> Option<()> {
    value.len = 0;
    let length_address = address.checked_add(LENGTH)?;
    let capacity_address = address.checked_add(CAPACITY);
    let (len, capacity) = if PREFETCHED
        && length_address.checked_add(8).is_some()
        && let Some(capacity_address) = capacity_address.filter(|at| at.checked_add(8).is_some())
    {
        let mut length = [0; 8];
        let mut capacity = [0; 8];
        if memory.copy_scattered_prefetched(
            [length_address, capacity_address],
            &mut [&mut length[..], &mut capacity[..]],
        ) != 2
        {
            return None;
        }
        (u64::from_le_bytes(length), u64::from_le_bytes(capacity))
    } else {
        (
            read::<u64>(memory, length_address)?,
            read::<u64>(memory, capacity_address?)?,
        )
    };
    read_text_with_layout_into(memory, address, len, capacity, value)
}

#[inline]
pub fn read_text_with_layout_into<const N: usize>(
    memory: &impl Memory,
    address: usize,
    len: u64,
    capacity: u64,
    value: &mut Text<N>,
) -> Option<()> {
    value.len = 0;
    if len > capacity || len > N as u64 || capacity < INLINE_CAPACITY {
        return None;
    }
    let source = if capacity == INLINE_CAPACITY {
        address
    } else {
        usize::try_from(read::<u64>(memory, address)?).ok()?
    };
    let len = len as usize;
    if read::<u8>(memory, source.checked_add(len)?)? != 0 {
        return None;
    }
    if !read_slice(memory, source, &mut value.bytes[..len]) {
        return None;
    }
    value.len = len;
    Some(())
}

#[derive(Clone, Copy)]
pub struct Descriptor {
    header: [u8; STRIDE],
    source: usize,
    pub len: usize,
    inline: bool,
    observation: Option<usize>,
}

pub fn copy_growing(
    memory: &impl Memory,
    source: usize,
    len: usize,
    output: &mut Vec<u8>,
    start: usize,
    unreadable: &'static str,
    allocation: &'static str,
) -> Result<(), &'static str> {
    copy_with_policy::<true>(memory, source, len, output, start, unreadable, allocation)
}

pub fn copy_with_policy<const TERMINAL: bool>(
    memory: &impl Memory,
    source: usize,
    len: usize,
    output: &mut Vec<u8>,
    start: usize,
    unreadable: &'static str,
    allocation: &'static str,
) -> Result<(), &'static str> {
    if start > output.len() || start.checked_add(len).is_none() || source.checked_add(len).is_none()
    {
        return Err(unreadable);
    }
    let mut scratch = [std::mem::MaybeUninit::<u8>::uninit(); GROWING_CHUNK];
    let mut copied = 0;
    while copied < len {
        let count = (len - copied).min(scratch.len());
        let Some(chunk) =
            crate::memory::read_slice_uninit(memory, source + copied, &mut scratch[..count])
        else {
            return Err(unreadable);
        };
        let end = start + copied + count;
        grow_to::<TERMINAL>(output, end, allocation)?;
        output[start + copied..end].copy_from_slice(chunk);
        copied += count;
    }
    Ok(())
}

const GROWING_CHUNK: usize = 256;

fn grow_to<const TERMINAL: bool>(
    output: &mut Vec<u8>,
    end: usize,
    allocation: &'static str,
) -> Result<(), &'static str> {
    if end > output.len() {
        let additional = end - output.len();
        if TERMINAL {
            engine_allocation::reserve(output, additional).map_err(|_| allocation)?;
        } else {
            output.try_reserve(additional).map_err(|_| allocation)?;
        }
        output.resize(end, 0);
    }
    Ok(())
}

pub fn place(
    bytes: &[u8],
    output: &mut Vec<u8>,
    start: usize,
    unreadable: &'static str,
    allocation: &'static str,
) -> Result<(), &'static str> {
    if start > output.len() || start.checked_add(bytes.len()).is_none() {
        return Err(unreadable);
    }
    for (index, chunk) in bytes.chunks(GROWING_CHUNK).enumerate() {
        let at = start + index * GROWING_CHUNK;
        let end = at + chunk.len();
        grow_to::<true>(output, end, allocation)?;
        output[at..end].copy_from_slice(chunk);
    }
    Ok(())
}

pub fn append(
    memory: &impl Memory,
    address: usize,
    arena: &mut Vec<u8>,
    used: &mut usize,
) -> Result<(usize, usize), &'static str> {
    let header = read::<[u8; STRIDE]>(memory, address).ok_or("string header unreadable")?;
    append_with_header(memory, &header, arena, used)
}

const JOINED_PAYLOAD: usize = 256;

pub fn append_with_header(
    memory: &impl Memory,
    header: &[u8; STRIDE],
    arena: &mut Vec<u8>,
    used: &mut usize,
) -> Result<(usize, usize), &'static str> {
    let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let length = word(LENGTH);
    let capacity = word(CAPACITY);
    if length > capacity || capacity < INLINE_CAPACITY {
        return Err("string header invalid");
    }
    let len = usize::try_from(length).map_err(|_| "string length overflow")?;
    let start = *used;
    let end = start
        .checked_add(len)
        .ok_or("string arena length overflow")?;
    if start > arena.len() {
        return Err("string arena start invalid");
    }
    let grow = |arena: &mut Vec<u8>, end: usize| {
        if end > arena.len() {
            let additional = end - arena.len();
            engine_allocation::reserve(arena, additional)
                .map_err(|_| "string arena allocation failed")?;
            arena.resize(end, 0);
        }
        Ok::<(), &'static str>(())
    };
    if capacity == INLINE_CAPACITY {
        if header.get(len) != Some(&0) {
            return Err("string terminator unreadable or invalid");
        }
        grow(arena, end)?;
        arena[start..end].copy_from_slice(&header[..len]);
    } else {
        let source = usize::try_from(word(DATA))
            .ok()
            .filter(|p| *p != 0)
            .ok_or("string heap pointer invalid")?;
        let mut scratch = [std::mem::MaybeUninit::<u8>::uninit(); JOINED_PAYLOAD];
        if let Some(joined) = (len < JOINED_PAYLOAD)
            .then(|| crate::memory::read_slice_uninit(memory, source, &mut scratch[..len + 1]))
            .flatten()
        {
            if joined[len] != 0 {
                return Err("string terminator unreadable or invalid");
            }
            grow(arena, end)?;
            arena[start..end].copy_from_slice(&joined[..len]);
        } else {
            if source.checked_add(len).and_then(|p| read::<u8>(memory, p)) != Some(0) {
                return Err("string terminator unreadable or invalid");
            }
            copy_growing(
                memory,
                source,
                len,
                arena,
                start,
                "string bytes unreadable",
                "string arena allocation failed",
            )?;
        }
    }
    *used = end;
    Ok((start, len))
}

pub fn heap_span(header: &[u8; STRIDE]) -> Option<(usize, usize)> {
    let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
    let (length, capacity) = (word(LENGTH), word(CAPACITY));
    if capacity <= INLINE_CAPACITY || length > capacity {
        return None;
    }
    let source = usize::try_from(word(DATA)).ok().filter(|p| *p != 0)?;
    Some((source, usize::try_from(length).ok()?.checked_add(1)?))
}

impl Descriptor {
    pub fn copy_words(
        &self,
        memory: &impl Memory,
        mut put: impl FnMut(u64),
    ) -> Result<(), &'static str> {
        for at in (0..self.len).step_by(8) {
            let count = (self.len - at).min(8);
            let mut word = [0; 8];
            if self.inline {
                word[..count].copy_from_slice(&self.header[at..at + count]);
            } else if !memory.copy(
                self.source
                    .checked_add(at)
                    .ok_or("string offset overflow")?,
                &mut word[..count],
            ) {
                return Err("string bytes unreadable");
            }
            put(u64::from_le_bytes(word));
        }
        Ok(())
    }
    pub fn read(memory: &impl Memory, address: usize) -> Result<Self, &'static str> {
        let header = read::<[u8; STRIDE]>(memory, address).ok_or("string header unreadable")?;
        Self::from_header(memory, address, header)
    }

    pub fn from_header(
        memory: &impl Memory,
        address: usize,
        header: [u8; STRIDE],
    ) -> Result<Self, &'static str> {
        let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
        let length = word(LENGTH);
        let capacity = word(CAPACITY);
        if length > capacity || capacity < INLINE_CAPACITY {
            return Err("string header invalid");
        }
        let len = usize::try_from(length).map_err(|_| "string length overflow")?;
        let inline = capacity == INLINE_CAPACITY;
        let source = if inline {
            address
        } else {
            usize::try_from(word(DATA))
                .ok()
                .filter(|p| *p != 0)
                .ok_or("string heap pointer invalid")?
        };
        let terminated = if inline {
            header.get(len) == Some(&0)
        } else {
            source.checked_add(len).and_then(|p| read::<u8>(memory, p)) == Some(0)
        };
        if !terminated {
            return Err("string terminator unreadable or invalid");
        }
        Ok(Self {
            header,
            source,
            len,
            inline,
            observation: None,
        })
    }

    pub fn heap_payload(&self) -> Option<(usize, usize)> {
        if self.inline {
            return None;
        }
        self.source.checked_add(self.len)?;
        Some((self.source, self.len))
    }

    pub fn copy_chunk(
        &self,
        memory: &impl Memory,
        start: usize,
        output: &mut [u8],
    ) -> Result<(), &'static str> {
        let end = start
            .checked_add(output.len())
            .filter(|end| *end <= self.len)
            .ok_or("string chunk exceeds payload")?;
        if self.inline {
            output.copy_from_slice(&self.header[start..end]);
        } else if !read_slice(
            memory,
            self.source
                .checked_add(start)
                .ok_or("string offset overflow")?,
            output,
        ) {
            return Err("string bytes unreadable");
        }
        Ok(())
    }

    pub fn copy(&self, memory: &impl Memory, output: &mut [u8]) -> Result<(), &'static str> {
        let output = output
            .get_mut(..self.len)
            .ok_or("string output too short")?;
        if self.inline {
            output.copy_from_slice(&self.header[..self.len]);
        } else if !read_slice(memory, self.source, output) {
            return Err("string bytes unreadable");
        }
        Ok(())
    }

    pub fn copy_buffer(
        &self,
        memory: &impl Memory,
        output: &mut crate::walk::Buffer<'_, u8>,
        start: usize,
    ) -> Result<(), &'static str> {
        let growing = match output {
            crate::walk::Buffer::Count | crate::walk::Buffer::Exact(_) => None,
            crate::walk::Buffer::Growing(values) => Some((values, 0)),
            crate::walk::Buffer::GrowingFrom(values, offset) => Some((values, *offset)),
        };
        if let Some((values, offset)) = growing {
            let start = offset
                .checked_add(start)
                .ok_or("string arena length overflow")?;
            let end = start
                .checked_add(self.len)
                .ok_or("string arena length overflow")?;
            if self.inline {
                let len = values.len().max(end);
                engine_allocation::resize_default(values, len)
                    .map_err(|_| "string arena allocation failed")?;
            } else if end > values.len() {
                return copy_growing(
                    memory,
                    self.source,
                    self.len,
                    values,
                    start,
                    "string bytes unreadable",
                    "string arena allocation failed",
                );
            }
        }
        self.copy(
            memory,
            output
                .get_mut(start..)
                .ok_or("string arena start invalid")?,
        )
    }
}

pub fn read_text_observed<const N: usize, const PREFETCHED: bool>(
    memory: &impl Memory,
    address: usize,
    raw: i32,
    source_kind: crate::raw_country_observations::Source,
    journal: &mut crate::raw_country_observations::Journal,
) -> Option<Text<N>> {
    use crate::raw_country_observations::{Attempt, TextAttempt};
    let index = journal.begin_text(raw, source_kind, address);
    let mut attempt = TextAttempt {
        address,
        ..Default::default()
    };
    let mut value = Text::<N>::default();
    let result = (|| {
        let length_address = address
            .checked_add(LENGTH)
            .ok_or("string length address overflow")?;
        let capacity_address = address.checked_add(CAPACITY);
        let (length, capacity) = if PREFETCHED
            && length_address.checked_add(8).is_some()
            && let Some(capacity_address) =
                capacity_address.filter(|at| at.checked_add(8).is_some())
        {
            let mut length = [0; 8];
            let mut capacity = [0; 8];
            let completed = memory.copy_scattered_prefetched(
                [length_address, capacity_address],
                &mut [&mut length[..], &mut capacity[..]],
            );
            attempt.length = Attempt::Completed(if completed >= 1 {
                Ok(u64::from_le_bytes(length))
            } else {
                Err("string length unreadable")
            });
            if completed >= 1 {
                attempt.capacity = Attempt::Completed(if completed == 2 {
                    Ok(u64::from_le_bytes(capacity))
                } else {
                    Err("string capacity unreadable")
                });
            }
            if completed != 2 {
                return Err("string layout unreadable");
            }
            (u64::from_le_bytes(length), u64::from_le_bytes(capacity))
        } else {
            let length = read::<u64>(memory, length_address).ok_or("string length unreadable");
            attempt.length = Attempt::Completed(length);
            let length = length?;
            let capacity = read::<u64>(
                memory,
                capacity_address.ok_or("string capacity address overflow")?,
            )
            .ok_or("string capacity unreadable");
            attempt.capacity = Attempt::Completed(capacity);
            (length, capacity?)
        };
        if length > capacity || capacity < INLINE_CAPACITY {
            return Err("string layout invalid");
        }
        if length > N as u64 {
            attempt.policy_limited = true;
            return Ok(false);
        }
        let source = if capacity == INLINE_CAPACITY {
            Ok(address)
        } else {
            read::<u64>(memory, address)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("string pointer unreadable")
        };
        attempt.source = Attempt::Completed(source);
        let source = source?;
        let len = length as usize;
        let terminator = read::<u8>(
            memory,
            source
                .checked_add(len)
                .ok_or("string terminator address overflow")?,
        )
        .ok_or("string terminator unreadable");
        attempt.terminator = Attempt::Completed(terminator);
        if terminator? != 0 {
            return Err("string terminator invalid");
        }
        if !read_slice(memory, source, &mut value.bytes[..len]) {
            return Err("string payload unreadable");
        }
        value.len = len;
        Ok(true)
    })();
    attempt.failure = result.as_ref().err().copied();
    if let Err(reason) = result {
        attempt.payload = Attempt::Completed(Err(reason));
    }
    journal.finish_text(
        index,
        attempt,
        matches!(result, Ok(true)).then(|| value.as_bytes()),
    );
    matches!(result, Ok(true)).then_some(value)
}

impl Descriptor {
    pub fn read_observed(
        memory: &impl Memory,
        address: usize,
        raw: i32,
        source_kind: crate::raw_country_observations::Source,
        journal: &mut crate::raw_country_observations::Journal,
    ) -> Result<Self, &'static str> {
        use crate::raw_country_observations::{Attempt, TextAttempt};
        let index = journal.begin_text(raw, source_kind, address);
        let mut attempt = TextAttempt {
            address,
            ..Default::default()
        };
        let result = (|| {
            let header = read::<[u8; STRIDE]>(memory, address).ok_or("string header unreadable");
            attempt.header = Attempt::Completed(header);
            let header = header?;
            let word = |at| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
            let length = word(LENGTH);
            let capacity = word(CAPACITY);
            attempt.length = Attempt::Completed(Ok(length));
            attempt.capacity = Attempt::Completed(Ok(capacity));
            if length > capacity || capacity < INLINE_CAPACITY {
                return Err("string header invalid");
            }
            let len = usize::try_from(length).map_err(|_| "string length overflow")?;
            let inline = capacity == INLINE_CAPACITY;
            let source = if inline {
                Ok(address)
            } else {
                usize::try_from(word(DATA))
                    .ok()
                    .filter(|value| *value != 0)
                    .ok_or("string heap pointer invalid")
            };
            attempt.source = Attempt::Completed(source);
            let source = source?;
            let terminator = if inline {
                header
                    .get(len)
                    .copied()
                    .ok_or("string terminator unreadable or invalid")
            } else {
                source
                    .checked_add(len)
                    .and_then(|at| read::<u8>(memory, at))
                    .ok_or("string terminator unreadable or invalid")
            };
            attempt.terminator = Attempt::Completed(terminator);
            if terminator? != 0 {
                return Err("string terminator unreadable or invalid");
            }
            Ok(Self {
                header,
                source,
                len,
                inline,
                observation: index,
            })
        })();
        attempt.failure = result.as_ref().err().copied();
        journal.finish_text(index, attempt, None);
        result
    }
    pub fn copy_observed(
        &self,
        memory: &impl Memory,
        output: &mut [u8],
        journal: &mut crate::raw_country_observations::Journal,
    ) -> Result<(), &'static str> {
        let result = self.copy(memory, output);
        if let Some(mut attempt) = journal.text_attempt(self.observation) {
            if let Err(reason) = result {
                attempt.failure = Some(reason);
                attempt.payload = crate::raw_country_observations::Attempt::Completed(Err(reason));
            }
            journal.finish_text(
                self.observation,
                attempt,
                result.is_ok().then(|| &output[..self.len]),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    use std::cell::{Cell, RefCell};

    #[test]
    fn growing_payload_allocates_only_for_successfully_acquired_chunks() {
        struct Bounded {
            readable: usize,
            reads: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Bounded {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                output.fill(0x5a);
                address >= 0x1000 && address - 0x1000 + output.len() <= self.readable
            }
        }
        for readable in [0, 256, 1025] {
            let memory = Bounded {
                readable,
                reads: RefCell::new(Vec::new()),
            };
            let mut output = vec![0xa5; 3];
            let capacity = output.capacity();
            let length = if readable == 1025 { 1025 } else { 32_768 };
            let result = copy_growing(
                &memory,
                0x1000,
                length,
                &mut output,
                3,
                "unreadable",
                "allocation",
            );
            assert_eq!(
                result,
                if readable == 1025 {
                    Ok(())
                } else {
                    Err("unreadable")
                }
            );
            assert_eq!(output.len(), 3 + readable);
            assert_eq!(&output[..3], &[0xa5; 3]);
            assert!(output[3..].iter().all(|&byte| byte == 0x5a));
            if readable == 0 {
                assert_eq!(output.capacity(), capacity);
            }
            let reads = memory.reads.borrow();
            for (index, &(address, width)) in reads.iter().enumerate() {
                assert_eq!(address, 0x1000 + index * 256);
                assert_eq!(
                    width,
                    if readable == 1025 && index == 4 {
                        1
                    } else {
                        256
                    }
                );
            }
        }
    }

    struct TracedMemory<'a> {
        source: SliceMemory<'a>,
        fields: RefCell<Vec<(usize, usize)>>,
        guards: Cell<usize>,
        prefetched: Cell<usize>,
        fault: Option<(usize, usize)>,
    }

    impl<'a> TracedMemory<'a> {
        fn new(base: usize, bytes: &'a [u8], fault: Option<(usize, usize)>) -> Self {
            Self {
                source: SliceMemory { base, bytes },
                fields: RefCell::new(Vec::new()),
                guards: Cell::new(0),
                prefetched: Cell::new(0),
                fault,
            }
        }

        fn field(&self, address: usize, output: &mut [u8]) -> bool {
            let ordinal = self.fields.borrow().len();
            self.fields.borrow_mut().push((address, output.len()));
            if let Some((failed, prefix)) = self.fault
                && ordinal == failed
            {
                self.source.copy(address, &mut output[..prefix]);
                return false;
            }
            self.source.copy(address, output)
        }
    }

    impl Memory for TracedMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.guards.set(self.guards.get() + 1);
            self.field(address, output)
        }

        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            outputs: &mut [&mut [u8]; N],
        ) -> usize {
            self.prefetched.set(self.prefetched.get() + 1);
            self.guards.set(self.guards.get() + 1);
            for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
                if output.is_empty() {
                    continue;
                }
                if address == 0
                    || address.checked_add(output.len()).is_none()
                    || !self.field(address, output)
                {
                    return index;
                }
            }
            N
        }
    }

    #[test]
    fn inline_and_heap_strings_are_exact_and_never_silently_shortened() {
        let mut bytes = [0_u8; 96];
        bytes[..3].copy_from_slice(b"GER");
        bytes[16..24].copy_from_slice(&3_u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&15_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&0x1040_u64.to_le_bytes());
        bytes[48..56].copy_from_slice(&4_u64.to_le_bytes());
        bytes[56..64].copy_from_slice(&31_u64.to_le_bytes());
        bytes[64..68].copy_from_slice(&[65, 0, 255, 66]);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            read_text::<3>(&memory, 0x1000).unwrap().as_str(),
            Some("GER")
        );
        assert!(read_text::<2>(&memory, 0x1000).is_none());
        assert_eq!(
            read_text::<8>(&memory, 0x1020).unwrap().as_bytes(),
            &[65, 0, 255, 66]
        );
        let traced = TracedMemory::new(0x1000, &bytes, None);
        let inline = Descriptor::read(&traced, 0x1000).unwrap();
        let heap = Descriptor::read(&traced, 0x1020).unwrap();
        assert_eq!(inline.heap_payload(), None);
        assert_eq!(heap.heap_payload(), Some((0x1040, 4)));
        assert_eq!(
            *traced.fields.borrow(),
            [(0x1000, STRIDE), (0x1020, STRIDE), (0x1044, 1)]
        );
        let mut output = [0xa5; 5];
        heap.copy(&traced, &mut output).unwrap();
        assert_eq!(output, [65, 0, 255, 66, 0xa5]);
        assert_eq!(traced.fields.borrow().last(), Some(&(0x1040, 4)));
    }

    #[test]
    fn opt_in_text_scatter_preserves_scalar_fields_every_fault_and_string_gates() {
        for (len, capacity) in [
            (0, 15),
            (3, 15),
            (15, 15),
            (0, 31),
            (3, 31),
            (17, 31),
            (256, 511),
            (16, 15),
            (32, 31),
            (257, 511),
            (0, 14),
        ] {
            for terminated in [true, false] {
                let mut bytes = vec![0; 1024];
                let payload = if capacity == INLINE_CAPACITY { 0 } else { 256 };
                for (index, byte) in bytes[payload..payload + len].iter_mut().enumerate() {
                    *byte = [0xff, 0, b'a'][index % 3];
                }
                bytes[payload + len] = u8::from(!terminated);
                if capacity != INLINE_CAPACITY {
                    bytes[..8].copy_from_slice(&0x1100_u64.to_le_bytes());
                }
                bytes[LENGTH..LENGTH + 8].copy_from_slice(&(len as u64).to_le_bytes());
                bytes[CAPACITY..CAPACITY + 8].copy_from_slice(&capacity.to_le_bytes());
                let scalar = TracedMemory::new(0x1000, &bytes, None);
                let prefetched = TracedMemory::new(0x1000, &bytes, None);
                let in_place = TracedMemory::new(0x1000, &bytes, None);
                let mut reused = Text {
                    bytes: [0xa5; 256],
                    len: 256,
                };
                let expected = read_text::<256>(&scalar, 0x1000);
                let actual = read_text_prefetched::<256>(&prefetched, 0x1000);
                let filled = read_text_prefetched_into(&in_place, 0x1000, &mut reused);
                assert_eq!(actual, expected);
                assert_eq!(filled.is_some(), actual.is_some());
                assert_eq!(
                    actual.is_some(),
                    len as u64 <= capacity
                        && len <= 256
                        && capacity >= INLINE_CAPACITY
                        && terminated
                );
                let fields = scalar.fields.borrow().clone();
                assert_eq!(*prefetched.fields.borrow(), fields);
                assert_eq!(*in_place.fields.borrow(), fields);
                assert_eq!(in_place.guards.get(), prefetched.guards.get());
                if let Some(value) = actual {
                    assert_eq!(reused.as_bytes(), value.as_bytes());
                    assert!(reused.bytes[reused.len..].iter().all(|byte| *byte == 0xa5));
                } else {
                    assert_eq!(reused.len, 0);
                }
                assert_eq!(scalar.prefetched.get(), 0, "old API stays scalar");
                assert_eq!(prefetched.prefetched.get(), 1);
                assert_eq!(scalar.guards.get(), prefetched.guards.get() + 1);
                assert_eq!(&fields[..2], &[(0x1010, 8), (0x1018, 8)]);
                if let Some(value) = actual {
                    assert_eq!(value.as_bytes(), &bytes[payload..payload + len]);
                    assert_eq!(
                        fields.len(),
                        3 + usize::from(capacity != INLINE_CAPACITY) + usize::from(len != 0)
                    );
                }
                for (ordinal, &(_, width)) in fields.iter().enumerate() {
                    for prefix in [0, width - 1] {
                        let scalar = TracedMemory::new(0x1000, &bytes, Some((ordinal, prefix)));
                        let prefetched = TracedMemory::new(0x1000, &bytes, Some((ordinal, prefix)));
                        let in_place = TracedMemory::new(0x1000, &bytes, Some((ordinal, prefix)));
                        let mut reused = Text {
                            bytes: [0xa5; 256],
                            len: 256,
                        };
                        assert!(read_text::<256>(&scalar, 0x1000).is_none());
                        assert!(read_text_prefetched::<256>(&prefetched, 0x1000).is_none());
                        assert!(
                            read_text_prefetched_into(&in_place, 0x1000, &mut reused).is_none()
                        );
                        assert_eq!(reused.len, 0);
                        assert_eq!(*scalar.fields.borrow(), fields[..=ordinal]);
                        assert_eq!(*prefetched.fields.borrow(), fields[..=ordinal]);
                        assert_eq!(*in_place.fields.borrow(), fields[..=ordinal]);
                        assert_eq!(in_place.guards.get(), prefetched.guards.get());
                        assert_eq!(scalar.prefetched.get(), 0);
                        assert_eq!(prefetched.prefetched.get(), 1);
                        assert_eq!(
                            scalar.guards.get(),
                            prefetched.guards.get() + usize::from(ordinal != 0)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn opt_in_text_scatter_keeps_address_overflow_prefixes_and_empty_heap_pointer_reads() {
        let mut bytes = [0; 64];
        bytes[CAPACITY..CAPACITY + 8].copy_from_slice(&INLINE_CAPACITY.to_le_bytes());
        for (address, observations, prefetched_calls) in [
            (0, 2, 1),
            (usize::MAX, 0, 0),
            (usize::MAX - 15, 0, 0),
            (usize::MAX - 16, 0, 0),
            (usize::MAX - 23, 0, 0),
            (usize::MAX - 24, 1, 0),
            (usize::MAX - 31, 1, 0),
            (usize::MAX - 32, 3, 1),
        ] {
            for fault in [None, Some((0, 0))] {
                let scalar = TracedMemory::new(address, &bytes, fault);
                let prefetched = TracedMemory::new(address, &bytes, fault);
                assert_eq!(
                    read_text_prefetched::<256>(&prefetched, address),
                    read_text::<256>(&scalar, address)
                );
                assert_eq!(*prefetched.fields.borrow(), *scalar.fields.borrow());
                assert_eq!(scalar.prefetched.get(), 0);
                assert_eq!(prefetched.prefetched.get(), prefetched_calls);
                if fault.is_none() {
                    assert_eq!(scalar.fields.borrow().len(), observations);
                    assert_eq!(
                        scalar.guards.get(),
                        prefetched.guards.get() + prefetched_calls
                    );
                }
            }
        }
        for pointer in [0, 0x1020, usize::MAX] {
            for len in [0_u64, 3] {
                bytes[..8].copy_from_slice(&(pointer as u64).to_le_bytes());
                bytes[LENGTH..LENGTH + 8].copy_from_slice(&len.to_le_bytes());
                bytes[CAPACITY..CAPACITY + 8].copy_from_slice(&31_u64.to_le_bytes());
                let scalar = TracedMemory::new(0x1000, &bytes, None);
                let prefetched = TracedMemory::new(0x1000, &bytes, None);
                assert_eq!(
                    read_text_prefetched::<256>(&prefetched, 0x1000),
                    read_text::<256>(&scalar, 0x1000)
                );
                assert_eq!(*prefetched.fields.borrow(), *scalar.fields.borrow());
                assert_eq!(
                    &scalar.fields.borrow()[..3],
                    &[(0x1010, 8), (0x1018, 8), (0x1000, 8)]
                );
                assert_eq!(scalar.prefetched.get(), 0);
                assert_eq!(prefetched.prefetched.get(), 1);
                assert_eq!(scalar.guards.get(), prefetched.guards.get() + 1);
            }
        }
    }
}
