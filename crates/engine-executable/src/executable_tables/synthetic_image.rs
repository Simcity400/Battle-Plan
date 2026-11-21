use super::pe::{PREFERRED_IMAGE_BASE, PeImage};

pub(crate) const TEXT: u32 = 0x1000;
pub(crate) const RDATA: u32 = 0x3000;
const TEXT_SIZE: u32 = 0x2000;
const RDATA_SIZE: u32 = 0x2000;
const FUNCTION_TABLE: u32 = RDATA + 0x1800;
const UNWIND: u32 = RDATA + RDATA_SIZE - 4;
const HEADERS: usize = 0x400;

pub(crate) struct SyntheticImage {
    text: Vec<u8>,
    rdata: Vec<u8>,
    functions: Vec<(u32, u32)>,
}

pub(crate) struct Code {
    at: u32,
    bytes: Vec<u8>,
}

impl Code {
    pub(crate) fn at(at: u32) -> Self {
        Self {
            at,
            bytes: Vec::new(),
        }
    }

    pub(crate) fn here(&self) -> u32 {
        self.at + self.bytes.len() as u32
    }

    pub(crate) fn raw(mut self, bytes: &[u8]) -> Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    pub(crate) fn rip(mut self, opcode: &[u8], target: u32) -> Self {
        let next = self.here() + opcode.len() as u32 + 4;
        let displacement = i32::try_from(i64::from(target) - i64::from(next))
            .expect("synthetic targets lie within a 32-bit displacement");
        self.bytes.extend_from_slice(opcode);
        self.bytes.extend_from_slice(&displacement.to_le_bytes());
        self
    }

    pub(crate) fn call(self, target: u32) -> Self {
        self.rip(&[0xe8], target)
    }

    pub(crate) fn ret(self) -> Self {
        self.raw(&[0xc3])
    }
}

impl SyntheticImage {
    pub(crate) fn new() -> Self {
        Self {
            text: vec![0xcc; TEXT_SIZE as usize],
            rdata: vec![0; RDATA_SIZE as usize],
            functions: Vec::new(),
        }
    }

    pub(crate) fn va(rva: u32) -> u64 {
        PREFERRED_IMAGE_BASE + u64::from(rva)
    }

    pub(crate) fn put(&mut self, rva: u32, bytes: &[u8]) -> &mut Self {
        let (section, start) = if (TEXT..TEXT + TEXT_SIZE).contains(&rva) {
            (&mut self.text, TEXT)
        } else {
            assert!(
                (RDATA..FUNCTION_TABLE).contains(&rva),
                "synthetic data at {rva:#x} lies outside the image's sections"
            );
            (&mut self.rdata, RDATA)
        };
        let at = (rva - start) as usize;
        section[at..at + bytes.len()].copy_from_slice(bytes);
        self
    }

    pub(crate) fn pointer(&mut self, rva: u32, target: u32) -> &mut Self {
        self.put(rva, &Self::va(target).to_le_bytes())
    }

    pub(crate) fn text(&mut self, rva: u32, text: &str) -> &mut Self {
        self.put(rva, &[text.as_bytes(), &[0]].concat())
    }

    pub(crate) fn code(&mut self, code: Code) -> &mut Self {
        self.put(code.at, &code.bytes)
    }

    pub(crate) fn function(&mut self, code: Code) -> &mut Self {
        let (start, end) = (code.at, code.here());
        self.put(start, &code.bytes);
        let at = self.functions.partition_point(|&(kept, _)| kept < start);
        self.functions.insert(at, (start, end));
        self
    }

    pub(crate) fn image(&self) -> PeImage {
        assert!(!self.functions.is_empty(), "a synthetic image declares its functions");
        let mut rdata = self.rdata.clone();
        for (index, &(start, end)) in self.functions.iter().enumerate() {
            let at = (FUNCTION_TABLE - RDATA) as usize + index * 12;
            for (offset, value) in [start, end, UNWIND].into_iter().enumerate() {
                rdata[at + offset * 4..at + offset * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        let mut bytes = vec![0; HEADERS];
        let put16 = |bytes: &mut Vec<u8>, at: usize, value: u16| {
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes())
        };
        let put32 = |bytes: &mut Vec<u8>, at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        bytes[..2].copy_from_slice(b"MZ");
        put32(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        put16(&mut bytes, 0x84, 0x8664);
        put16(&mut bytes, 0x86, 2);
        put16(&mut bytes, 0x94, 0xf0);
        let optional = 0x98;
        put16(&mut bytes, optional, 0x20b);
        bytes[optional + 24..optional + 32].copy_from_slice(&PREFERRED_IMAGE_BASE.to_le_bytes());
        put32(&mut bytes, optional + 32, 0x1000);
        put32(&mut bytes, optional + 36, 0x200);
        put32(&mut bytes, optional + 56, RDATA + RDATA_SIZE);
        put32(&mut bytes, optional + 60, HEADERS as u32);
        put32(&mut bytes, optional + 108, 16);
        put32(&mut bytes, optional + 112 + 3 * 8, FUNCTION_TABLE);
        put32(
            &mut bytes,
            optional + 112 + 3 * 8 + 4,
            (self.functions.len() * 12) as u32,
        );
        let sections = optional + 0xf0;
        for (index, (rva, size, flags)) in [
            (TEXT, TEXT_SIZE, 0x6000_0020),
            (RDATA, RDATA_SIZE, 0x4000_0040),
        ]
        .into_iter()
        .enumerate()
        {
            let entry = sections + index * 40;
            put32(&mut bytes, entry + 8, size);
            put32(&mut bytes, entry + 12, rva);
            put32(&mut bytes, entry + 16, size);
            put32(&mut bytes, entry + 20, (HEADERS + index * TEXT_SIZE as usize) as u32);
            put32(&mut bytes, entry + 36, flags);
        }
        bytes.extend_from_slice(&self.text);
        bytes.extend_from_slice(&rdata);
        PeImage::parse(bytes).expect("the synthetic image is a valid PE image")
    }
}
