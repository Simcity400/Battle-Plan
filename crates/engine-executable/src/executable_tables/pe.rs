use iced_x86::{Decoder, DecoderOptions, Instruction, OpKind};
use std::{io, sync::OnceLock, thread};

pub const PREFERRED_IMAGE_BASE: u64 = 0x0000_0001_4000_0000;
const AMD64: u16 = 0x8664;
const PE32_PLUS: u16 = 0x020b;
const EXECUTABLE_SECTION: u32 = 0x2000_0000;
const WRITABLE_SECTION: u32 = 0x8000_0000;
const EXCEPTION_DIRECTORY: usize = 3;
const SECURITY_DIRECTORY: usize = 4;

#[derive(Debug)]
struct Section {
    rva: u32,
    raw_size: u32,
    raw_offset: usize,
    virtual_end: u32,
    executable: bool,
    writable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RttiClass {
    pub locator: u32,
    pub offset: u32,
    pub descriptor: u32,
    pub vtable: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RipLea {
    pub instruction: u32,
    pub target: u32,
    pub register: usize,
}

#[derive(Debug)]
pub struct PeImage {
    bytes: Vec<u8>,
    sections: Vec<Section>,
    functions: Vec<(u32, u32)>,
    executable_ranges: Vec<(u32, u32)>,
    image_size: u32,
    leas: OnceLock<Vec<RipLea>>,
    classes: OnceLock<Vec<RttiClass>>,
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
                writable: file_u32(&bytes, add(entry, 36)?)? & WRITABLE_SECTION != 0,
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
            leas: OnceLock::new(),
            classes: OnceLock::new(),
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

    pub fn data_sections(&self) -> impl Iterator<Item = (u32, &[u8])> {
        self.sections
            .iter()
            .filter(|section| !section.executable)
            .map(|section| {
                (
                    section.rva,
                    &self.bytes[section.raw_offset..section.raw_offset + section.raw_size as usize],
                )
            })
    }

    pub fn read_only_data(&self) -> impl Iterator<Item = &[u8]> {
        self.sections
            .iter()
            .filter(|section| !section.executable && !section.writable)
            .map(|section| {
                &self.bytes[section.raw_offset..section.raw_offset + section.raw_size as usize]
            })
    }

    pub fn is_read_only(&self, rva: u32, len: usize) -> bool {
        let Some(end) = u32::try_from(len).ok().and_then(|len| rva.checked_add(len)) else {
            return false;
        };
        self.sections.iter().any(|section| {
            !section.executable
                && !section.writable
                && rva >= section.rva
                && end <= section.rva + section.raw_size
        })
    }

    pub fn is_writable_virtual(&self, rva: u32, len: usize) -> bool {
        let Some(end) = u32::try_from(len).ok().and_then(|len| rva.checked_add(len)) else {
            return false;
        };
        self.sections.iter().any(|section| {
            !section.executable
                && section.writable
                && rva >= section.rva
                && end <= section.virtual_end
        })
    }

    pub fn find_hashed_strings(&self, anchors: &[Hashed]) -> Vec<Vec<u32>> {
        let mut found = vec![Vec::new(); anchors.len()];
        for (rva, data) in self.data_sections() {
            let mut start = 0;
            for end in memchr::memchr_iter(0, data).chain(std::iter::once(data.len())) {
                let text = &data[start..end];
                for (anchor, found) in anchors.iter().zip(&mut found) {
                    if text.len() == anchor.length && fnv1a(text) == anchor.fnv {
                        found.push(rva + start as u32);
                    }
                }
                start = end + 1;
            }
        }
        found
    }

    pub fn classes_named(&self, anchor: Hashed) -> impl Iterator<Item = RttiClass> + '_ {
        self.classes().iter().copied().filter(move |class| {
            self.type_name(class.descriptor)
                .is_ok_and(|name| anchor.matches(name.as_bytes()))
        })
    }

    pub fn references_to(&self, target: u32) -> Vec<u32> {
        self.references_to_each(&[target]).remove(0)
    }

    pub fn references_to_each(&self, targets: &[u32]) -> Vec<Vec<u32>> {
        let mut found = vec![Vec::new(); targets.len()];
        let Some(landings) = Landings::new(targets) else {
            return found;
        };
        let threads = std::thread::available_parallelism().map_or(1, |threads| threads.get());
        std::thread::scope(|scope| {
            let mut scans = Vec::new();
            for section in self.sections.iter().filter(|section| section.executable) {
                let code =
                    &self.bytes[section.raw_offset..section.raw_offset + section.raw_size as usize];
                let chunk = code.len().div_ceil(threads).max(1);
                for start in (0..code.len()).step_by(chunk) {
                    let landings = &landings;
                    scans.push(scope.spawn(move || {
                        self.scan_references(
                            section,
                            code,
                            start..(start + chunk).min(code.len()),
                            landings,
                        )
                    }));
                }
            }
            for scan in scans {
                let hits = scan
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                for (index, rva) in hits {
                    found[index].push(rva);
                }
            }
        });
        for list in &mut found {
            list.sort_unstable();
            list.dedup();
        }
        found
    }

    fn scan_references(
        &self,
        section: &Section,
        code: &[u8],
        positions: std::ops::Range<usize>,
        landings: &Landings,
    ) -> Vec<(usize, u32)> {
        let mut hits = Vec::new();
        let origin = i64::from(section.rva) + 4;
        let end = (positions.end + 3).min(code.len());
        for (at, window) in (positions.start..).zip(code[positions.start..end].windows(4)) {
            let displacement = i32::from_le_bytes(window.try_into().unwrap());
            let lands = origin + at as i64 + i64::from(displacement);
            for index in landings.at(lands) {
                let target = landings.targets[index];
                let end = at + 4 + (i64::from(target) - lands) as usize;
                if let Some(start) = self.reference_start(section, code, at, end, target) {
                    hits.push((index, section.rva + start as u32));
                }
            }
        }
        hits
    }

    fn reference_start(
        &self,
        section: &Section,
        code: &[u8],
        at: usize,
        end: usize,
        target: u32,
    ) -> Option<usize> {
        let target_va = self.base + u64::from(target);
        for length in (5..=15).rev() {
            let Some(start) = end.checked_sub(length).filter(|start| *start <= at) else {
                continue;
            };
            let Some(bytes) = code.get(start..end) else {
                continue;
            };
            let mut decoder = Decoder::with_ip(
                64,
                bytes,
                self.base + u64::from(section.rva) + start as u64,
                DecoderOptions::NONE,
            );
            let instruction = decoder.decode();
            if instruction.is_invalid() || instruction.len() != length {
                continue;
            }
            let lands = (instruction.is_ip_rel_memory_operand()
                && instruction.ip_rel_memory_address() == target_va)
                || (matches!(instruction.op0_kind(), OpKind::NearBranch64)
                    && instruction.near_branch_target() == target_va);
            if lands {
                return Some(start);
            }
        }
        None
    }

    pub fn rip_leas(&self) -> &[RipLea] {
        self.leas.get_or_init(|| {
            let threads = thread::available_parallelism().map_or(1, |count| count.get().min(8));
            let mut found = Vec::new();
            for section in self.sections.iter().filter(|section| section.executable) {
                let code =
                    &self.bytes[section.raw_offset..section.raw_offset + section.raw_size as usize];
                let chunk = code.len().div_ceil(threads).max(1);
                thread::scope(|scope| {
                    let parts: Vec<_> = (0..code.len())
                        .step_by(chunk)
                        .map(|start| {
                            scope.spawn(move || {
                                rip_leas(code, section.rva, start, (start + chunk).min(code.len()))
                            })
                        })
                        .collect();
                    for part in parts {
                        found.extend(
                            part.join()
                                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                        );
                    }
                });
            }
            found
        })
    }

    pub fn text_at(&self, rva: u32, len: usize) -> Option<&[u8]> {
        let section = self.sections.iter().find(|section| {
            !section.executable
                && !section.writable
                && rva >= section.rva
                && rva < section.rva + section.raw_size
        })?;
        let data = &self.bytes[section.raw_offset..section.raw_offset + section.raw_size as usize];
        let at = (rva - section.rva) as usize;
        (at.checked_sub(1).is_none_or(|before| data[before] == 0) && data.get(at + len) == Some(&0))
            .then(|| &data[at..at + len])
            .filter(|text| !text.contains(&0))
    }

    pub fn classes(&self) -> &[RttiClass] {
        self.classes.get_or_init(|| {
            let read_only = || {
                self.sections
                    .iter()
                    .filter(|section| !section.executable && !section.writable)
                    .map(|section| {
                        (
                            section.rva,
                            &self.bytes
                                [section.raw_offset..section.raw_offset + section.raw_size as usize],
                        )
                    })
            };
            let mut locators = Vec::new();
            for (rva, data) in read_only() {
                for at in (0..data.len().saturating_sub(23)).step_by(4) {
                    let word = |offset: usize| {
                        u32::from_le_bytes(data[at + offset..at + offset + 4].try_into().unwrap())
                    };
                    if word(0) == 1 && word(20) == rva + at as u32 {
                        locators.push((rva + at as u32, word(4), word(12)));
                    }
                }
            }
            let (Some(&(low, ..)), Some(&(high, ..))) = (locators.first(), locators.last()) else {
                return Vec::new();
            };
            let (low, high) = (self.base + u64::from(low), self.base + u64::from(high));
            let mut classes = Vec::new();
            for (rva, data) in read_only() {
                for at in (0..data.len().saturating_sub(15)).step_by(8) {
                    let pointer = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
                    if pointer < low || pointer > high {
                        continue;
                    }
                    let locator = (pointer - self.base) as u32;
                    let method = u64::from_le_bytes(data[at + 8..at + 16].try_into().unwrap());
                    if let Ok(index) = locators.binary_search_by_key(&locator, |&(at, ..)| at)
                        && self.is_code_pointer(method)
                    {
                        let (_, offset, descriptor) = locators[index];
                        classes.push(RttiClass {
                            locator,
                            offset,
                            descriptor,
                            vtable: rva + at as u32 + 8,
                        });
                    }
                }
            }
            classes.sort_unstable_by_key(|class| (class.locator, class.vtable));
            classes
        })
    }

    pub fn locator_bases(&self, locator: u32) -> io::Result<Vec<(u32, u32)>> {
        let hierarchy = self.u32(rtti(locator, 16)?)?;
        let count = self.u32(rtti(hierarchy, 8)?)?;
        let bases = self.u32(rtti(hierarchy, 12)?)?;
        self.bytes(bases, count as usize * 4)?;
        (0..count)
            .map(|index| {
                let descriptor = self.u32(bases + index * 4)?;
                Ok((self.u32(descriptor)?, self.u32(rtti(descriptor, 8)?)?))
            })
            .collect()
    }

    pub fn type_name(&self, descriptor: u32) -> io::Result<&str> {
        self.string(rtti(descriptor, 16)?)
    }

    pub fn referencing_functions(
        &self,
        references: impl IntoIterator<Item = u32>,
    ) -> io::Result<Vec<(u32, u32)>> {
        let mut held = Vec::new();
        for reference in references {
            held.push((self.function(reference)?, reference));
        }
        held.sort_unstable();
        held.dedup();
        let mut extents: Vec<(u32, u32)> = Vec::new();
        for group in held.chunk_by(|left, right| left.0 == right.0) {
            let (start, end) = group[0].0;
            let starts: Vec<_> = self
                .instructions(start, end)?
                .iter()
                .map(|instruction| (instruction.ip() - self.base) as u32)
                .collect();
            if group
                .iter()
                .any(|&(_, reference)| starts.binary_search(&reference).is_err())
            {
                return Err(invalid("PE reference does not start an instruction"));
            }
            extents.push((start, end));
        }
        Ok(extents)
    }

    pub fn is_code_pointer(&self, va: u64) -> bool {
        self.va_to_rva(va).is_ok_and(|rva| {
            self.executable_ranges
                .iter()
                .any(|&(start, end)| (start..end).contains(&rva))
        })
    }

    pub fn function_instructions(&self, rva: u32) -> io::Result<Vec<Instruction>> {
        let (start, end) = self.function(rva)?;
        self.instructions(start, end)
    }

    fn is_executable_range(&self, start: u32, end: u32) -> bool {
        self.executable_ranges
            .iter()
            .any(|&(section_start, section_end)| {
                start >= section_start && start <= end && end <= section_end
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hashed {
    pub length: usize,
    pub fnv: u64,
}

impl Hashed {
    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() == self.length && fnv1a(bytes) == self.fnv
    }

    pub fn prefixes(&self, bytes: &[u8]) -> bool {
        bytes.len() >= self.length && fnv1a(&bytes[..self.length]) == self.fnv
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub what: &'static str,
    pub text: Hashed,
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn rip_leas(code: &[u8], rva: u32, start: usize, end: usize) -> Vec<RipLea> {
    let mut found = Vec::new();
    for at in memchr::memchr_iter(0x8d, &code[start..end]).map(|at| start + at) {
        if at == 0
            || at + 6 > code.len()
            || code[at - 1] & 0xfb != 0x48
            || code[at + 1] & 0xc7 != 0x05
        {
            continue;
        }
        let displacement = i32::from_le_bytes(code[at + 2..at + 6].try_into().unwrap());
        let Ok(target) = u32::try_from(i64::from(rva) + at as i64 + 6 + i64::from(displacement))
        else {
            continue;
        };
        found.push(RipLea {
            instruction: rva + at as u32 - 1,
            target,
            register: usize::from((code[at - 1] & 4) << 1 | (code[at + 1] >> 3) & 7),
        });
    }
    found
}

fn rtti(rva: u32, offset: u32) -> io::Result<u32> {
    rva.checked_add(offset)
        .ok_or_else(|| invalid("RTTI address overflows"))
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

struct Landings<'a> {
    targets: &'a [u32],
    sorted: Vec<(i64, usize)>,
    low: i64,
    span: u64,
    buckets: Vec<u64>,
}

impl<'a> Landings<'a> {
    const TAILS: [i64; 4] = [0, 1, 2, 4];
    const BUCKET_BITS: u32 = 4;

    fn new(targets: &'a [u32]) -> Option<Self> {
        let mut sorted: Vec<(i64, usize)> = targets
            .iter()
            .enumerate()
            .flat_map(|(index, &target)| Self::TAILS.map(|tail| (i64::from(target) - tail, index)))
            .collect();
        sorted.sort_unstable();
        let low = sorted.first()?.0;
        let span = (sorted.last()?.0 - low) as u64;
        let mut buckets = vec![0; ((span >> Self::BUCKET_BITS) / 64 + 1) as usize];
        for &(landing, _) in &sorted {
            let bucket = ((landing - low) as u64 >> Self::BUCKET_BITS) as usize;
            buckets[bucket / 64] |= 1 << (bucket % 64);
        }
        Some(Self {
            targets,
            sorted,
            low,
            span,
            buckets,
        })
    }

    fn at(&self, lands: i64) -> impl Iterator<Item = usize> + '_ {
        let offset = lands.wrapping_sub(self.low) as u64;
        let bucket = (offset >> Self::BUCKET_BITS) as usize;
        let possible = offset <= self.span && self.buckets[bucket / 64] & (1 << (bucket % 64)) != 0;
        let first = if possible {
            self.sorted.partition_point(|&(landing, _)| landing < lands)
        } else {
            self.sorted.len()
        };
        self.sorted[first..]
            .iter()
            .take_while(move |&&(landing, _)| landing == lands)
            .map(|&(_, index)| index)
    }
}
