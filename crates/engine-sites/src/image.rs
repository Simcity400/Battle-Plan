use std::borrow::Cow;

use crate::{Verified, Withheld};

pub struct Image<'a> {
    sections: Vec<Section<'a>>,
    live: Option<&'a dyn Fn(usize, &mut [u8]) -> bool>,
    pointer_base: u64,
    pub(crate) base: usize,
}

struct Section<'a> {
    name: [u8; 8],
    rva: u32,
    size: u32,
    bytes: &'a [u8],
}

pub struct OwnedImage {
    sections: Vec<([u8; 8], u32, Vec<u8>)>,
    pointer_base: u64,
}

const PAGE: usize = 0x1000;

fn zeroed(len: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).ok()?;
    bytes.resize(len, 0);
    Some(bytes)
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

impl<'a> Image<'a> {
    pub fn live(base: usize, copy: &'a dyn Fn(usize, &mut [u8]) -> bool) -> Verified<Self> {
        let read = |offset: usize, len: usize| {
            let mut bytes = zeroed(len)?;
            copy(base.checked_add(offset)?, &mut bytes).then_some(bytes)
        };
        let header = header(read)?;
        let sections = header
            .sections
            .iter()
            .map(|&(name, rva, size, _, _)| Section {
                name,
                rva,
                size,
                bytes: &[],
            })
            .collect();
        Ok(Self {
            sections,
            live: Some(copy),
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
                    size: bytes.len() as u32,
                    bytes,
                })
                .collect(),
            live: None,
            pointer_base: self.pointer_base,
            base,
        }
    }
}

impl<'a> Image<'a> {
    fn containing(&self, rva: u32) -> Option<&Section<'a>> {
        self.sections.iter().find(|s| rva >= s.rva && rva - s.rva < s.size)
    }

    fn read(&self, section: &Section<'a>, start: usize, len: usize) -> Option<Cow<'a, [u8]>> {
        let end = start.checked_add(len)?;
        if end > section.size as usize {
            return None;
        }
        let Some(copy) = self.live else {
            return section.bytes.get(start..end).map(Cow::Borrowed);
        };
        let mut bytes = zeroed(len)?;
        let address = self.base.checked_add(section.rva as usize)?.checked_add(start)?;
        copy(address, &mut bytes).then_some(Cow::Owned(bytes))
    }

    pub(crate) fn section(&self, name: &[u8]) -> Option<(u32, Cow<'a, [u8]>)> {
        let s = self
            .sections
            .iter()
            .find(|s| s.name.split(|&b| b == 0).next() == Some(name))?;
        Some((s.rva, self.read(s, 0, s.size as usize)?))
    }

    pub(crate) fn section_of(&self, rva: u32) -> Option<&[u8]> {
        self.containing(rva).map(|s| s.name.split(|&b| b == 0).next().unwrap_or(&[]))
    }

    pub(crate) fn bytes(&self, rva: u32, len: usize) -> Option<Cow<'a, [u8]>> {
        let s = self.containing(rva)?;
        self.read(s, (rva - s.rva) as usize, len)
    }

    pub(crate) fn u32(&self, rva: u32) -> Option<u32> {
        self.bytes(rva, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i32(&self, rva: u32) -> Option<i32> {
        self.u32(rva).map(|v| v as i32)
    }

    pub(crate) fn pointer(&self, rva: u32) -> Option<u32> {
        let b = self.bytes(rva, 8)?;
        let value = u64::from_le_bytes(b[..].try_into().ok()?);
        u32::try_from(value.checked_sub(self.pointer_base)?).ok()
    }

    pub(crate) fn pointer_base(&self) -> u64 {
        self.pointer_base
    }

    pub(crate) fn cstring(&self, rva: u32) -> Option<Cow<'a, [u8]>> {
        let s = self.containing(rva)?;
        let start = (rva - s.rva) as usize;
        let limit = (s.size as usize - start).min(1024);
        if self.live.is_none() {
            let tail = s.bytes.get(start..start + limit)?;
            let end = tail.iter().position(|&b| b == 0)?;
            return Some(Cow::Borrowed(&tail[..end]));
        }
        let mut name = Vec::new();
        while name.len() < limit {
            let at = start + name.len();
            let address = self.base.checked_add(s.rva as usize)?.checked_add(at)?;
            let chunk = (PAGE - address % PAGE).min(limit - name.len());
            let bytes = self.read(s, at, chunk)?;
            if let Some(end) = bytes.iter().position(|&b| b == 0) {
                name.extend_from_slice(&bytes[..end]);
                return Some(Cow::Owned(name));
            }
            name.extend_from_slice(&bytes);
        }
        None
    }
}
