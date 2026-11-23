use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
            TH32CS_SNAPMODULE,
        },
        LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            GetModuleHandleExW, GetModuleHandleW,
        },
    },
};

pub(super) const RECORD_SIZE: usize = 32;
pub(super) const PREFIX_SIZE: usize = 88;
pub(super) const FOOTER_SIZE: usize = 24;
pub(super) const MAGIC: &[u8; 8] = b"HOI4WSMP";
pub(super) const FOOTER_MAGIC: &[u8; 8] = b"WSMPEND!";

pub(super) struct Header {
    pub interval_us: u32,
    pub(crate) qpc_hz: u64,
    pub(crate) tsc_hz: u64,
    pub(crate) collector_base: u64,
    pub image_size: u32,
    pub(crate) timestamp: u32,
    pub(crate) guid: [u8; 16],
    pub(crate) age: u32,
    pub(crate) pdb_path: String,
    pub(crate) game_base: u64,
}
impl Header {
    pub(super) fn current(interval_us: u32, qpc_hz: u64) -> Result<Self, String> {
        let mut module = ptr::null_mut();
        // SAFETY: the address of this module's static names the collector image, which stays
        // loaded while its own code runs here, so the unchanged reference count is enough.
        if unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                MAGIC.as_ptr().cast(),
                &mut module,
            )
        } == 0
        {
            return Err("worker_sampler cannot locate collector image".into());
        }
        let base = module as usize;
        // SAFETY: every read below copies a bounded range of the collector image into an owned
        // buffer, and a range that cannot be read withholds the header.
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let copy = |offset: usize, len: usize| {
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(len).ok()?;
            bytes.resize(len, 0);
            engine_native::read_slice(&memory, base.checked_add(offset)?, &mut bytes)
                .then_some(bytes)
        };
        let dos = copy(0, 64).ok_or("worker_sampler collector DOS header unreadable")?;
        if &dos[..2] != b"MZ" {
            return Err("worker_sampler collector DOS header invalid".into());
        }
        let pe = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as usize;
        if !(64..=1024 * 1024).contains(&pe) {
            return Err("worker_sampler collector PE offset invalid".into());
        }
        let nt = copy(pe, 84).ok_or("worker_sampler collector PE64 header unreadable")?;
        if &nt[..4] != b"PE\0\0" || u16::from_le_bytes(nt[24..26].try_into().unwrap()) != 0x20b {
            return Err("worker_sampler collector PE64 header invalid".into());
        }
        let image_size = u32::from_le_bytes(nt[80..84].try_into().unwrap());
        let timestamp = u32::from_le_bytes(nt[8..12].try_into().unwrap());
        let within = |start: usize, len: usize| {
            start
                .checked_add(len)
                .filter(|&end| end <= image_size as usize)
                .and_then(|_| copy(start, len))
        };
        let directories = within(pe + 24 + 108, 4)
            .map(|count| u32::from_le_bytes(count[..].try_into().unwrap()))
            .ok_or("worker_sampler collector data directories unreadable")?;
        let optional = u16::from_le_bytes(nt[20..22].try_into().unwrap()) as usize;
        if directories < 7 || optional < 112 + 7 * 8 {
            return Err("worker_sampler collector debug directory missing".into());
        }
        let directory = within(pe + 24 + 112 + 6 * 8, 8)
            .ok_or("worker_sampler collector debug directory missing")?;
        let rva = u32::from_le_bytes(directory[..4].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(directory[4..8].try_into().unwrap()) as usize;
        if rva
            .checked_add(size)
            .is_none_or(|end| end > image_size as usize)
        {
            return Err("worker_sampler collector debug directory out of bounds".into());
        }
        let mut identity = None;
        for index in 0..size / 28 {
            let entry = within(rva + 28 * index, 28)
                .ok_or("worker_sampler collector debug directory unreadable")?;
            if u32::from_le_bytes(entry[12..16].try_into().unwrap()) != 2 {
                continue;
            }
            let len = u32::from_le_bytes(entry[16..20].try_into().unwrap()) as usize;
            let start = u32::from_le_bytes(entry[20..24].try_into().unwrap()) as usize;
            let Some(codeview) = within(start, len) else {
                continue;
            };
            if codeview.len() < 25 || &codeview[..4] != b"RSDS" {
                continue;
            }
            let guid = codeview[4..20].try_into().unwrap();
            let age = u32::from_le_bytes(codeview[20..24].try_into().unwrap());
            let path = &codeview[24..];
            let end = path.iter().position(|b| *b == 0).unwrap_or(path.len());
            let path = std::str::from_utf8(&path[..end])
                .map_err(|_| "worker_sampler collector PDB path is not UTF-8")?
                .to_owned();
            identity = Some((guid, age, path));
            break;
        }
        let (guid, age, pdb_path) =
            identity.ok_or("worker_sampler collector RSDS PDB identity missing")?;
        Ok(Self {
            interval_us,
            qpc_hz,
            tsc_hz: collector_timing::hook_timing::tsc_hz().unwrap_or(0),
            collector_base: base as u64,
            image_size,
            timestamp,
            guid,
            age,
            pdb_path,
            game_base: unsafe { GetModuleHandleW(ptr::null()) } as u64,
        })
    }
    fn encode(&self) -> io::Result<Vec<u8>> {
        let path_len = u32::try_from(self.pdb_path.len()).map_err(io::Error::other)?;
        let header_len = (PREFIX_SIZE as u32)
            .checked_add(path_len)
            .ok_or_else(|| io::Error::other("header length overflow"))?;
        let mut bytes = Vec::with_capacity(header_len as usize);
        bytes.extend_from_slice(MAGIC);
        for value in [1, header_len, RECORD_SIZE as u32, self.interval_us] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [self.qpc_hz, self.tsc_hz, self.collector_base] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [self.image_size, self.timestamp] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.guid);
        for value in [self.age, path_len] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.game_base.to_le_bytes());
        bytes.extend_from_slice(self.pdb_path.as_bytes());
        Ok(bytes)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Record {
    pub qpc: u64,
    pub cycles: u64,
    pub slot: u16,
    pub role: u8,
    pub worker: u8,
    pub job: u16,
    pub(crate) pc: u64,
}
impl Record {
    fn encode(self) -> [u8; RECORD_SIZE] {
        let mut bytes = [0; RECORD_SIZE];
        bytes[..8].copy_from_slice(&self.qpc.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.cycles.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.pc.to_le_bytes());
        bytes[24..26].copy_from_slice(&self.slot.to_le_bytes());
        bytes[26..28].copy_from_slice(&self.job.to_le_bytes());
        bytes[28] = self.role;
        bytes[29] = self.worker;
        bytes
    }
}
pub(super) struct Chunk {
    records: Vec<[u8; RECORD_SIZE]>,
    used: usize,
}
impl Chunk {
    pub(super) fn new(capacity: usize) -> io::Result<Self> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(capacity)
            .map_err(io::Error::other)?;
        records.resize(capacity, [0; RECORD_SIZE]);
        Ok(Self { records, used: 0 })
    }
    pub(super) fn append(&mut self, record: &Record) -> bool {
        if let Some(slot) = self.records.get_mut(self.used) {
            *slot = record.encode();
            self.used += 1;
            true
        } else {
            false
        }
    }
    pub(super) fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.records.as_ptr().cast(), self.used * RECORD_SIZE) }
    }
    pub(super) fn len(&self) -> usize {
        self.used
    }
    pub(super) fn full(&self) -> bool {
        self.used == self.records.len()
    }
    pub(super) fn clear(&mut self) {
        self.used = 0;
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ThreadInfo {
    pub slot: u16,
    pub thread_id: u32,
    pub role: String,
    pub(crate) worker_index: Option<u8>,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Module {
    pub base: u64,
    pub size: u32,
    pub path: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Drops {
    pub total: u64,
    pub capture: u64,
    pub write: u64,
    pub(crate) writer_backpressure: u64,
    pub(crate) buffer_full: u64,
    pub(crate) game_exit_race: u64,
}
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Trailer {
    pub(crate) record_count: u64,
    pub(crate) dropped_counts: Drops,
    pub threads: Vec<ThreadInfo>,
    pub(crate) job_roles: Vec<String>,
    pub modules: Vec<Module>,
}
pub(super) fn modules() -> Vec<Module> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut modules = Vec::new();
    let mut entry = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut next = unsafe { Module32FirstW(snapshot, &mut entry) };
    while next != 0 {
        let len = entry
            .szExePath
            .iter()
            .position(|v| *v == 0)
            .unwrap_or(entry.szExePath.len());
        modules.push(Module {
            base: entry.modBaseAddr as u64,
            size: entry.modBaseSize,
            path: String::from_utf16_lossy(&entry.szExePath[..len]),
        });
        next = unsafe { Module32NextW(snapshot, &mut entry) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    modules
}
pub(super) struct Writer {
    file: BufWriter<File>,
    bytes: u64,
}
impl Writer {
    pub(super) fn create(path: &Path, header: &Header) -> io::Result<Self> {
        let bytes = header.encode()?;
        let mut writer = Self {
            file: BufWriter::with_capacity(256 * 1024, File::create(path)?),
            bytes: 0,
        };
        writer.write(&bytes)?;
        writer.file.flush()?;
        Ok(writer)
    }
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.bytes += bytes.len() as u64;
        Ok(())
    }
    pub(super) fn records(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.write(bytes)?;
        self.file.flush()
    }
    pub(super) fn finish(&mut self, trailer: &Trailer) -> io::Result<()> {
        let offset = self.bytes;
        let trailer = serde_json::to_vec(trailer).map_err(io::Error::other)?;
        self.write(&trailer)?;
        let mut footer = [0; FOOTER_SIZE];
        footer[..8].copy_from_slice(FOOTER_MAGIC);
        footer[8..16].copy_from_slice(&offset.to_le_bytes());
        footer[16..].copy_from_slice(&(trailer.len() as u64).to_le_bytes());
        self.write(&footer)?;
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_sampler_chunk_handoff_and_drop_count() {
        let mut chunk = Chunk::new(2).unwrap();
        let record = Record {
            qpc: 12,
            cycles: 34,
            pc: 0x1234,
            ..Record::default()
        };
        assert!(chunk.append(&record));
        assert!(chunk.append(&record));
        assert!(chunk.full());
        assert!(!chunk.append(&record));
        let allocation = chunk.bytes().as_ptr();
        assert_eq!(chunk.bytes().len(), 64);
        chunk.clear();
        assert_eq!(chunk.len(), 0);
        assert!(chunk.append(&record));
        assert_eq!(chunk.bytes().as_ptr(), allocation);
    }
    #[test]
    fn worker_sampler_format_round_trip_and_truncated_prefix() {
        let directory = std::env::temp_dir().join(format!(
            "hoi4-worker-sampler-{}-{}",
            std::process::id(),
            unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("synthetic.bin");
        let header = Header {
            interval_us: 250,
            qpc_hz: 10_000_000,
            tsc_hz: 3_000_000_000,
            collector_base: 0x180000000,
            image_size: 12345,
            timestamp: 777,
            guid: [4; 16],
            age: 2,
            pdb_path: "collector.pdb".into(),
            game_base: 0x140000000,
        };
        let mut writer = Writer::create(&path, &header).unwrap();
        let record = Record {
            qpc: 1,
            cycles: 2,
            slot: 3,
            role: 7,
            worker: 4,
            job: 5,
            pc: 0x180000111,
        };
        writer.records(&record.encode()).unwrap();
        let trailer = Trailer {
            record_count: 1,
            dropped_counts: Drops {
                total: 3,
                capture: 1,
                write: 0,
                writer_backpressure: 2,
                buffer_full: 0,
                game_exit_race: 0,
            },
            threads: vec![ThreadInfo {
                slot: 3,
                thread_id: 101,
                role: "acquisition".into(),
                worker_index: Some(4),
            }],
            job_roles: vec!["characters".into()],
            modules: vec![],
        };
        writer.finish(&trailer).unwrap();
        drop(writer);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], MAGIC);
        let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let u64_at = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        assert_eq!(u32_at(8), 1);
        assert_eq!(u32_at(16), RECORD_SIZE as u32);
        assert_eq!(u64_at(24), header.qpc_hz);
        assert_eq!(&bytes[56..72], &header.guid);
        let start = u32_at(12) as usize;
        let encoded = &bytes[start..start + RECORD_SIZE];
        let decoded = Record {
            qpc: u64::from_le_bytes(encoded[..8].try_into().unwrap()),
            cycles: u64::from_le_bytes(encoded[8..16].try_into().unwrap()),
            pc: u64::from_le_bytes(encoded[16..24].try_into().unwrap()),
            slot: u16::from_le_bytes(encoded[24..26].try_into().unwrap()),
            job: u16::from_le_bytes(encoded[26..28].try_into().unwrap()),
            role: encoded[28],
            worker: encoded[29],
        };
        assert_eq!(decoded, record);
        let end = bytes.len() - FOOTER_SIZE;
        assert_eq!(&bytes[end..end + 8], FOOTER_MAGIC);
        let offset = u64_at(end + 8) as usize;
        let length = u64_at(end + 16) as usize;
        assert_eq!(offset, start + RECORD_SIZE);
        assert_eq!(offset + length, end);
        let parsed: Trailer = serde_json::from_slice(&bytes[offset..end]).unwrap();
        assert_eq!(parsed.record_count, 1);
        assert_eq!(parsed.threads, trailer.threads);
        std::fs::write(&path, &bytes[..start + RECORD_SIZE + 17]).unwrap();
        let truncated = std::fs::read(&path).unwrap();
        assert_eq!((truncated.len() - start) / RECORD_SIZE, 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
