use crate::{Verified, Withheld};

pub struct Image<'a> {
    sections: Vec<Section<'a>>,
    pointer_base: u64,
    pub(crate) base: usize,
}

struct Section<'a> {
    name: [u8; 8],
    rva: u32,
    bytes: &'a [u8],
}

pub struct OwnedImage {
    sections: Vec<([u8; 8], u32, Vec<u8>)>,
    pointer_base: u64,
}

fn malformed() -> Withheld {
    Withheld::new("executable image", "malformed PE headers")
}

struct Header {
    sections: Vec<([u8; 8], u32, u32, u32, u32)>,
    image_base: u64,
}

fn header(read: impl Fn(usize, usize) -> Option<Vec<u8>>) -> Verified<Header> {
    let u16_at = |offset: usize| read(offset, 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |offset: usize| read(offset, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let pe = u32_at(0x3c).ok_or_else(malformed)? as usize;
    if u32_at(pe) != Some(0x4550) || u16_at(pe + 24) != Some(0x20b) {
        return Err(malformed());
    }
    let count = usize::from(u16_at(pe + 6).ok_or_else(malformed)?);
    let optional = usize::from(u16_at(pe + 20).ok_or_else(malformed)?);
    let image_base = read(pe + 24 + 24, 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
        .ok_or_else(malformed)?;
    let table = pe + 24 + optional;
    let mut sections = Vec::with_capacity(count);
    for index in 0..count {
        let row = read(table + 40 * index, 40).ok_or_else(malformed)?;
        let word = |at: usize| u32::from_le_bytes([row[at], row[at + 1], row[at + 2], row[at + 3]]);
        let mut name = [0; 8];
        name.copy_from_slice(&row[..8]);
        sections.push((name, word(12), word(8), word(20), word(16)));
    }
    Ok(Header {
        sections,
        image_base,
    })
}

impl Image<'static> {
    /// The image of the module loaded at `base` in this process.
    ///
    /// # Safety
    /// `base` must be the base of a loaded PE32+ module that stays loaded for the program's life, and
    /// whose section bytes are readable and unchanged while this image borrows them (#1628).
    pub unsafe fn module(base: usize) -> Verified<Self> {
        let read = |offset: usize, len: usize| {
            // SAFETY: the headers of a loaded module are mapped and readable.
            Some(unsafe { std::slice::from_raw_parts((base + offset) as *const u8, len) }.to_vec())
        };
        let header = header(read)?;
        let sections = header
            .sections
            .iter()
            .map(|&(name, rva, virtual_size, _, _)| Section {
                name,
                rva,
                // SAFETY: a loaded module maps every section at its RVA for its virtual size.
                bytes: unsafe {
                    std::slice::from_raw_parts((base + rva as usize) as *const u8, virtual_size as usize)
                },
            })
            .collect();
        Ok(Self {
            sections,
            pointer_base: base as u64,
            base,
        })
    }
}

impl OwnedImage {
    pub fn new(file: &[u8]) -> Verified<Self> {
        let read = |offset: usize, len: usize| file.get(offset..offset.checked_add(len)?).map(<[u8]>::to_vec);
        let header = header(read)?;
        let mut sections = Vec::with_capacity(header.sections.len());
        for &(name, rva, virtual_size, raw_offset, raw_size) in &header.sections {
            let mut bytes = vec![0; virtual_size as usize];
            let copied = raw_size.min(virtual_size) as usize;
            let raw = file
                .get(raw_offset as usize..raw_offset as usize + copied)
                .ok_or_else(malformed)?;
            bytes[..copied].copy_from_slice(raw);
            sections.push((name, rva, bytes));
        }
        Ok(Self {
            sections,
            pointer_base: header.image_base,
        })
    }

    pub fn image(&self, base: usize) -> Image<'_> {
        Image {
            sections: self
                .sections
                .iter()
                .map(|(name, rva, bytes)| Section {
                    name: *name,
                    rva: *rva,
                    bytes,
                })
                .collect(),
            pointer_base: self.pointer_base,
            base,
        }
    }
}

impl<'a> Image<'a> {
    pub(crate) fn section(&self, name: &[u8]) -> Option<(u32, &'a [u8])> {
        self.sections
            .iter()
            .find(|s| s.name.split(|&b| b == 0).next() == Some(name))
            .map(|s| (s.rva, s.bytes))
    }

    pub(crate) fn section_of(&self, rva: u32) -> Option<&[u8]> {
        self.sections
            .iter()
            .find(|s| rva >= s.rva && ((rva - s.rva) as usize) < s.bytes.len())
            .map(|s| s.name.split(|&b| b == 0).next().unwrap_or(&[]))
    }

    pub(crate) fn bytes(&self, rva: u32, len: usize) -> Option<&'a [u8]> {
        let s = self
            .sections
            .iter()
            .find(|s| rva >= s.rva && ((rva - s.rva) as usize) < s.bytes.len())?;
        let start = (rva - s.rva) as usize;
        s.bytes.get(start..start.checked_add(len)?)
    }

    pub(crate) fn u32(&self, rva: u32) -> Option<u32> {
        self.bytes(rva, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i32(&self, rva: u32) -> Option<i32> {
        self.u32(rva).map(|v| v as i32)
    }

    pub(crate) fn pointer(&self, rva: u32) -> Option<u32> {
        let b = self.bytes(rva, 8)?;
        let value = u64::from_le_bytes(b.try_into().ok()?);
        u32::try_from(value.checked_sub(self.pointer_base)?).ok()
    }

    pub(crate) fn pointer_base(&self) -> u64 {
        self.pointer_base
    }

    pub(crate) fn cstring(&self, rva: u32) -> Option<&'a [u8]> {
        let s = self
            .sections
            .iter()
            .find(|s| rva >= s.rva && ((rva - s.rva) as usize) < s.bytes.len())?;
        let tail = &s.bytes[(rva - s.rva) as usize..];
        let end = tail.iter().take(1024).position(|&b| b == 0)?;
        Some(&tail[..end])
    }
}
