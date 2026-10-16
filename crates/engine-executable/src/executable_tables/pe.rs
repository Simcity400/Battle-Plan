use iced_x86::{Decoder, DecoderOptions, Instruction};
use std::io;

pub const PREFERRED_IMAGE_BASE: u64 = 0x0000_0001_4000_0000;
const AMD64: u16 = 0x8664;
const PE32_PLUS: u16 = 0x020b;
const EXECUTABLE_SECTION: u32 = 0x2000_0000;
const EXCEPTION_DIRECTORY: usize = 3;
const SECURITY_DIRECTORY: usize = 4;

#[derive(Debug)]
struct Section {
    rva: u32,
    raw_size: u32,
    raw_offset: usize,
    virtual_end: u32,
    executable: bool,
}

/// An owned file image; RVA reads require complete file backing in one section.
#[derive(Debug)]
pub struct PeImage {
    bytes: Vec<u8>,
    sections: Vec<Section>,
    functions: Vec<(u32, u32)>,
    executable_ranges: Vec<(u32, u32)>,
    image_size: u32,
    pub base: u64,
}

impl PeImage {
    pub fn parse(bytes: Vec<u8>) -> io::Result<Self> {
        if file_bytes(&bytes, 0, 2)? != b"MZ" {
            return Err(invalid("PE DOS signature is missing"));
        }
        let pe = usize::try_from(file_u32(&bytes, 0x3c)?)
            .map_err(|_| invalid("PE header offset exceeds the file address space"))?;
        if pe < 0x40 || file_bytes(&bytes, pe, 4)? != b"PE\0\0" {
            return Err(invalid("PE signature is missing"));
        }
        let coff = add(pe, 4)?;
        file_bytes(&bytes, coff, 20)?;
        if file_u16(&bytes, coff)? != AMD64 {
            return Err(invalid("PE machine must be AMD64"));
        }
        let section_count = usize::from(file_u16(&bytes, add(coff, 2)?)?);
        if section_count == 0 {
            return Err(invalid("PE has no sections"));
        }
        let optional_size = usize::from(file_u16(&bytes, add(coff, 16)?)?);
        let optional = add(coff, 20)?;
        file_bytes(&bytes, optional, optional_size)?;
        if optional_size < 112 || file_u16(&bytes, optional)? != PE32_PLUS {
            return Err(invalid("PE optional header must be PE32+"));
        }
        let base = file_u64(&bytes, add(optional, 24)?)?;
        if base != PREFERRED_IMAGE_BASE {
            return Err(invalid("PE preferred image base is unsupported"));
        }
        let section_alignment = file_u32(&bytes, add(optional, 32)?)?;
        let file_alignment = file_u32(&bytes, add(optional, 36)?)?;
        if !section_alignment.is_power_of_two()
            || !file_alignment.is_power_of_two()
            || section_alignment < file_alignment
        {
            return Err(invalid("PE section and file alignments are invalid"));
        }
        let image_size = file_u32(&bytes, add(optional, 56)?)?;
        let headers_size = usize::try_from(file_u32(&bytes, add(optional, 60)?)?)
            .map_err(|_| invalid("PE header size exceeds the file address space"))?;
        if image_size == 0 || headers_size == 0 || headers_size > bytes.len() {
            return Err(invalid("PE image or header size is invalid"));
        }
        base.checked_add(u64::from(image_size))
            .ok_or_else(|| invalid("PE virtual address range overflows"))?;
        let directory_count = usize::try_from(file_u32(&bytes, add(optional, 108)?)?)
            .map_err(|_| invalid("PE directory count exceeds the file address space"))?;
        let directories_size = directory_count
            .checked_mul(8)
            .ok_or_else(|| invalid("PE directory size overflows"))?;
        if directories_size > optional_size - 112 {
            return Err(invalid("PE directories exceed the optional header"));
        }
        let section_table = add(optional, optional_size)?;
        let section_table_size = section_count
            .checked_mul(40)
            .ok_or_else(|| invalid("PE section table size overflows"))?;
        file_bytes(&bytes, section_table, section_table_size)?;
        if add(section_table, section_table_size)? > headers_size {
            return Err(invalid("PE sections exceed the declared headers"));
        }
        let mut sections = Vec::with_capacity(section_count);
        for index in 0..section_count {
            let entry = add(section_table, index * 40)?;
            let virtual_size = file_u32(&bytes, add(entry, 8)?)?;
            let rva = file_u32(&bytes, add(entry, 12)?)?;
            let raw_size = file_u32(&bytes, add(entry, 16)?)?;
            let raw_offset = usize::try_from(file_u32(&bytes, add(entry, 20)?)?)
                .map_err(|_| invalid("PE section file offset exceeds the file address space"))?;
            let virtual_end = rva
                .checked_add(virtual_size.max(raw_size))
                .ok_or_else(|| invalid("PE section RVA range overflows"))?;
            if virtual_end > image_size || u64::from(rva) < headers_size as u64 {
                return Err(invalid(
                    "PE section exceeds the image or overlaps its headers",
                ));
            }
            if raw_size != 0 {
                if raw_offset < headers_size {
                    return Err(invalid("PE section file data overlaps its headers"));
                }
                file_bytes(
                    &bytes,
                    raw_offset,
                    usize::try_from(raw_size)
                        .map_err(|_| invalid("PE section size exceeds the file address space"))?,
                )?;
            }
            sections.push(Section {
                rva,
                raw_size,
                raw_offset,
                virtual_end,
                executable: file_u32(&bytes, add(entry, 36)?)? & EXECUTABLE_SECTION != 0,
            });
        }
        sections.sort_unstable_by_key(|section| section.rva);
        for pair in sections.windows(2) {
            if pair[0].virtual_end > pair[1].rva {
                return Err(invalid("PE virtual sections overlap"));
            }
        }
        let mut raw_ranges: Vec<_> = sections
            .iter()
            .filter(|section| section.raw_size != 0)
            .map(|section| {
                (
                    section.raw_offset,
                    section.raw_offset + section.raw_size as usize,
                )
            })
            .collect();
        raw_ranges.sort_unstable();
        if raw_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(invalid("PE section file ranges overlap"));
        }
        let executable_ranges = sections
            .iter()
            .filter(|section| section.executable && section.raw_size != 0)
            .map(|section| (section.rva, section.rva + section.raw_size))
            .collect();
        let mut image = Self {
            bytes,
            sections,
            functions: Vec::new(),
            executable_ranges,
            image_size,
            base,
        };
        let mut exception = None;
        for index in 0..directory_count {
            let entry = add(add(optional, 112)?, index * 8)?;
            let rva = file_u32(&image.bytes, entry)?;
            let size = file_u32(&image.bytes, add(entry, 4)?)?;
            if rva == 0 && size == 0 {
                continue;
            }
            if rva == 0 || size == 0 {
                return Err(invalid("PE directory has an incomplete span"));
            }
            let size = usize::try_from(size)
                .map_err(|_| invalid("PE directory size exceeds the file address space"))?;
            if index == SECURITY_DIRECTORY {
                let offset = usize::try_from(rva)
                    .map_err(|_| invalid("PE certificate offset exceeds the file address space"))?;
                file_bytes(&image.bytes, offset, size)?;
            } else {
                image.bytes(rva, size)?;
            }
            if index == EXCEPTION_DIRECTORY {
                exception = Some((rva, size));
            }
        }
        let (rva, size) = exception.ok_or_else(|| invalid("PE exception directory is missing"))?;
        if size % 12 != 0 {
            return Err(invalid(
                "PE exception directory has a partial function entry",
            ));
        }
        let mut functions = Vec::with_capacity(size / 12);
        for offset in (0..size).step_by(12) {
            let entry = rva
                .checked_add(u32::try_from(offset).map_err(|_| {
                    invalid("PE function entry offset exceeds the RVA address space")
                })?)
                .ok_or_else(|| invalid("PE function entry RVA overflows"))?;
            let start = image.u32(entry)?;
            let end = image.u32(
                entry
                    .checked_add(4)
                    .ok_or_else(|| invalid("PE function entry RVA overflows"))?,
            )?;
            let unwind = image.u32(
                entry
                    .checked_add(8)
                    .ok_or_else(|| invalid("PE function entry RVA overflows"))?,
            )?;
            if start >= end || !image.is_executable_range(start, end) {
                return Err(invalid(
                    "PE function extent is invalid or lacks executable file backing",
                ));
            }
            image.bytes(unwind & !3, 4)?;
            if functions
                .last()
                .is_some_and(|&(_, previous_end)| previous_end > start)
            {
                return Err(invalid("PE function entries are unsorted or overlap"));
            }
            functions.push((start, end));
        }
        image.functions = functions;
        Ok(image)
    }

    pub fn bytes(&self, rva: u32, len: usize) -> io::Result<&[u8]> {
        let len_rva = u32::try_from(len)
            .map_err(|_| invalid("PE span length exceeds the RVA address space"))?;
        let end = rva
            .checked_add(len_rva)
            .ok_or_else(|| invalid("PE RVA span overflows"))?;
        for section in &self.sections {
            let section_end = section.rva + section.raw_size;
            if section.raw_size != 0 && rva >= section.rva && end <= section_end {
                let relative = usize::try_from(rva - section.rva).map_err(|_| {
                    invalid("PE relative file offset exceeds the file address space")
                })?;
                return file_bytes(&self.bytes, add(section.raw_offset, relative)?, len);
            }
        }
        Err(invalid(
            "PE RVA span lacks complete file backing in one section",
        ))
    }

    pub fn u8(&self, rva: u32) -> io::Result<u8> {
        Ok(self.bytes(rva, 1)?[0])
    }

    pub fn u16(&self, rva: u32) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(rva, 2)?.try_into().unwrap()))
    }

    pub fn u32(&self, rva: u32) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(rva, 4)?.try_into().unwrap()))
    }

    pub fn u64(&self, rva: u32) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(rva, 8)?.try_into().unwrap()))
    }

    pub fn string(&self, rva: u32) -> io::Result<&str> {
        let section = self
            .sections
            .iter()
            .find(|section| rva >= section.rva && rva < section.rva + section.raw_size)
            .ok_or_else(|| invalid("PE string starts outside file-backed section data"))?;
        let available = usize::try_from(section.rva + section.raw_size - rva)
            .map_err(|_| invalid("PE string span exceeds the file address space"))?;
        let bytes = self.bytes(rva, available)?;
        let end = bytes
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| invalid("PE string is not NUL-terminated within its section"))?;
        std::str::from_utf8(&bytes[..end]).map_err(|_| invalid("PE string contains invalid UTF-8"))
    }

    pub fn va_to_rva(&self, va: u64) -> io::Result<u32> {
        let relative = va
            .checked_sub(self.base)
            .ok_or_else(|| invalid("PE virtual address precedes the preferred image base"))?;
        let rva = u32::try_from(relative)
            .map_err(|_| invalid("PE virtual address exceeds the RVA address space"))?;
        if rva >= self.image_size {
            return Err(invalid("PE virtual address lies outside the image"));
        }
        Ok(rva)
    }

    /// Decodes file-backed code with preferred virtual addresses as instruction IPs.
    pub fn instructions(&self, start: u32, end: u32) -> io::Result<Vec<Instruction>> {
        if start > end || !self.is_executable_range(start, end) {
            return Err(invalid(
                "PE instruction range lacks executable file backing",
            ));
        }
        let bytes = self.bytes(
            start,
            usize::try_from(end - start)
                .map_err(|_| invalid("PE instruction span exceeds the file address space"))?,
        )?;
        let mut decoder = Decoder::with_ip(
            64,
            bytes,
            self.base + u64::from(start),
            DecoderOptions::NONE,
        );
        let mut instructions = Vec::new();
        while decoder.can_decode() {
            let instruction = decoder.decode();
            if instruction.is_invalid() {
                return Err(invalid(
                    "PE instruction range contains an invalid or truncated instruction",
                ));
            }
            instructions.push(instruction);
        }
        Ok(instructions)
    }

    pub fn function(&self, rva: u32) -> io::Result<(u32, u32)> {
        let index = self.functions.partition_point(|&(start, _)| start <= rva);
        if let Some(&extent) = index
            .checked_sub(1)
            .and_then(|index| self.functions.get(index))
            && rva < extent.1
        {
            return Ok(extent);
        }
        Err(invalid(
            "PE address has no exception-directory function extent",
        ))
    }

    pub fn functions(&self) -> &[(u32, u32)] {
        &self.functions
    }

    pub fn executable_ranges(&self) -> &[(u32, u32)] {
        &self.executable_ranges
    }

    fn is_executable_range(&self, start: u32, end: u32) -> bool {
        self.executable_ranges
            .iter()
            .any(|&(section_start, section_end)| {
                start >= section_start && start <= end && end <= section_end
            })
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn add(left: usize, right: usize) -> io::Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| invalid("PE file offset overflows"))
}

fn file_bytes(bytes: &[u8], offset: usize, len: usize) -> io::Result<&[u8]> {
    bytes
        .get(offset..add(offset, len)?)
        .ok_or_else(|| invalid("PE file span is truncated"))
}

fn file_u16(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        file_bytes(bytes, offset, 2)?.try_into().unwrap(),
    ))
}

fn file_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        file_bytes(bytes, offset, 4)?.try_into().unwrap(),
    ))
}

fn file_u64(bytes: &[u8], offset: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        file_bytes(bytes, offset, 8)?.try_into().unwrap(),
    ))
}
