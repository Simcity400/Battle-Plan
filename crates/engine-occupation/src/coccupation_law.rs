
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029b_c290);
pub const NAME: usize = 0x10;

pub const DATABASE_RVA: Rva = Rva(0x033d_d3e0);
pub const DATABASE_FALLBACK: usize = 0x48;

use engine_native_memory::{Memory, msvc_string, read, read_slice};

pub(crate) fn fallback(memory: &impl Memory, base: usize) -> Result<Literal, &'static str> {
    let database = engine_sites::at(base, DATABASE_RVA)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("occupation-law database unavailable")?;
    let law = database
        .checked_add(DATABASE_FALLBACK)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("occupation-law database fallback unavailable")?;
    literal(memory, base, law)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Literal {
    pub address: usize,
    pub len: usize,
}

pub(crate) fn literal(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Literal, &'static str> {
    let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(vtable as u64) {
        return Err("occupation law vtable mismatch");
    }
    let address = pointer
        .checked_add(NAME)
        .ok_or("occupation law address overflow")?;
    let header =
        read::<[u8; 32]>(memory, address).ok_or("occupation law string header unreadable")?;
    let len = usize::try_from(u64::from_le_bytes(header[16..24].try_into().unwrap()))
        .map_err(|_| "occupation law string length overflow")?;
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("occupation law string header invalid");
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "occupation law string pointer overflow")?
    };
    if read::<u8>(
        memory,
        source
            .checked_add(len)
            .ok_or("occupation law string address overflow")?,
    ) != Some(0)
    {
        return Err("occupation law string terminator unreadable or invalid");
    }
    Ok(Literal {
        address: source,
        len,
    })
}

pub(crate) struct Names<'a> {
    bytes: engine_native_memory::walk::Buffer<'a, u8>,
    pub len: usize,
    pub required: usize,
}

impl<'a> Names<'a> {
    pub fn new_buffer(bytes: engine_native_memory::walk::Buffer<'a, u8>) -> Self {
        Self {
            bytes,
            len: 0,
            required: 0,
        }
    }

    pub fn copy(
        &mut self,
        memory: &impl Memory,
        value: Literal,
        row_fits: bool,
    ) -> Result<Option<usize>, &'static str> {
        self.required = self
            .required
            .checked_add(value.len)
            .ok_or("occupation law arena size overflow")?;
        self.bytes.ensure(self.required)?;
        if !row_fits || value.len > self.bytes.len() - self.len {
            return Ok(None);
        }
        let start = self.len;
        if !read_slice(
            memory,
            value.address,
            &mut self.bytes[start..start + value.len],
        ) {
            return Err("occupation law token bytes unreadable");
        }
        self.len += value.len;
        Ok(Some(start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn literal_law_bytes_are_preserved_and_capacity_omits_whole_tokens() {
        let mut bytes = [0; 0x60];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[NAME..NAME + 4].copy_from_slice(&[b'l', 0, 255, b'w']);
        bytes[NAME + 16..NAME + 24].copy_from_slice(&4_u64.to_le_bytes());
        bytes[NAME + 24..NAME + 32].copy_from_slice(&15_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let token = literal(&memory, 0, 0x1000).unwrap();
        let mut arena = Vec::new();
        let mut names = Names::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut arena));
        assert_eq!(names.copy(&memory, token, true), Ok(Some(0)));
        assert_eq!((names.len, names.required), (4, 4));
        assert_eq!(arena[..4], [b'l', 0, 255, b'w']);
        bytes[NAME + 4] = 1;
        assert!(
            literal(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            )
            .is_err()
        );
    }
}
