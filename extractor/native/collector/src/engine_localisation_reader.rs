use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::c_void;
use theatre_selector_live_lane::engine_localisation_provenance::TargetImageMemory;

type Handle = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn ReadProcessMemory(
        process: Handle,
        base_address: *const c_void,
        buffer: *mut c_void,
        size: usize,
        bytes_read: *mut usize,
    ) -> i32;
}

const BLOCK_BITS: u32 = 16;
const BLOCK: u64 = 1 << BLOCK_BITS;

const MAX_CACHED_BLOCKS: usize = 1024;

pub struct InProcessEngineImage {
    blocks: RefCell<BTreeMap<u64, Option<Box<[u8]>>>>,
}

impl Default for InProcessEngineImage {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessEngineImage {
    pub fn new() -> Self {
        Self {
            blocks: RefCell::new(BTreeMap::new()),
        }
    }

    fn accepts(ok: i32, read: usize, requested: usize) -> bool {
        ok != 0 && read == requested
    }

    fn raw(address: u64, length: usize) -> Option<Vec<u8>> {
        let mut bytes = vec![0u8; length];
        let mut read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                address as *const c_void,
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut read,
            )
        };
        Self::accepts(ok, read, bytes.len()).then_some(bytes)
    }

    fn block(&self, index: u64) -> Option<Box<[u8]>> {
        if let Some(cached) = self.blocks.borrow().get(&index) {
            return cached.clone();
        }
        let address = index << BLOCK_BITS;
        let block = Self::raw(address, BLOCK as usize).map(Vec::into_boxed_slice);
        let mut cache = self.blocks.borrow_mut();
        if cache.len() < MAX_CACHED_BLOCKS {
            cache.insert(index, block.clone());
        }
        block
    }
}

impl TargetImageMemory for InProcessEngineImage {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        if length == 0 {
            return Err("engine localisation read has zero length".to_string());
        }
        let end = address
            .checked_add(length as u64)
            .ok_or("engine localisation read range overflow")?;
        let mut out = Vec::new();
        out.try_reserve_exact(length)
            .map_err(|error| format!("reserve engine localisation read: {error}"))?;
        let mut cursor = address;
        while cursor < end {
            let index = cursor >> BLOCK_BITS;
            let offset = (cursor - (index << BLOCK_BITS)) as usize;
            let take = ((BLOCK as usize) - offset).min((end - cursor) as usize);
            match self.block(index) {
                Some(block) => out.extend_from_slice(&block[offset..offset + take]),
                None => {
                    let exact = Self::raw(cursor, take).ok_or_else(|| {
                        format!("engine localisation address 0x{cursor:x} is not readable")
                    })?;
                    out.extend_from_slice(&exact);
                }
            }
            cursor += take as u64;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readable_range_in_this_process_comes_back_byte_for_byte() {
        let source: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
        let image = InProcessEngineImage::new();
        let read = image
            .read(source.as_ptr() as u64, source.len())
            .expect("our own buffer is readable");
        assert_eq!(read, source);
    }

    #[test]
    fn a_range_spanning_a_block_boundary_is_reassembled_in_order() {
        let source: Vec<u8> = (0..262_144u32).map(|value| (value % 251) as u8).collect();
        let image = InProcessEngineImage::new();
        let start = source.as_ptr() as u64 + 3;
        let read = image.read(start, source.len() - 3).expect("readable");
        assert_eq!(read.len(), source.len() - 3);
        assert_eq!(read, source[3..]);
    }

    #[test]
    fn an_unmapped_address_is_refused_rather_than_faulting() {
        let image = InProcessEngineImage::new();
        let error = image
            .read(0x10, 8)
            .expect_err("the null page must never be readable");
        assert!(error.contains("is not readable"), "got: {error}");
    }

    #[test]
    fn a_short_read_is_never_accepted_as_complete() {
        assert!(
            InProcessEngineImage::accepts(1, 64, 64),
            "a full read is complete"
        );
        assert!(
            !InProcessEngineImage::accepts(1, 63, 64),
            "a successful call that produced fewer bytes than asked for is not a complete read"
        );
        assert!(
            !InProcessEngineImage::accepts(0, 64, 64),
            "a failed call is never complete"
        );
    }

    #[test]
    fn a_read_straddling_the_end_of_a_mapping_never_returns_a_partial_count() {
        #[link(name = "kernel32")]
        extern "system" {
            fn VirtualAlloc(
                address: *mut c_void,
                size: usize,
                kind: u32,
                protect: u32,
            ) -> *mut c_void;
            fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        }
        const MEM_COMMIT_RESERVE: u32 = 0x1000 | 0x2000;
        const MEM_RELEASE: u32 = 0x8000;
        const PAGE_READWRITE: u32 = 0x04;

        let page = unsafe {
            VirtualAlloc(
                core::ptr::null_mut(),
                4096,
                MEM_COMMIT_RESERVE,
                PAGE_READWRITE,
            )
        };
        assert!(!page.is_null(), "one committed page");
        let mut bytes = vec![0u8; 8192];
        let mut read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                page as *const c_void,
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut read,
            )
        };
        unsafe { VirtualFree(page, 0, MEM_RELEASE) };
        assert!(
            ok == 0 || read == bytes.len(),
            "a partial count came back (ok={ok}, read={read}); the length clause is load-bearing"
        );
    }

    #[test]
    fn a_zero_length_read_is_refused() {
        let image = InProcessEngineImage::new();
        assert!(image.read(0x1000, 0).is_err());
    }

    struct Bump {
        bytes: Vec<u8>,
        at: usize,
        base: u64,
    }

    impl Bump {
        fn alloc(&mut self, len: usize) -> usize {
            let at = (self.at + 0x3f) & !0x3f;
            self.at = at + len;
            assert!(self.at < self.bytes.len(), "measurement buffer overflow");
            at
        }

        fn u64(&mut self, off: usize, value: u64) {
            self.bytes[off..off + 8].copy_from_slice(&value.to_le_bytes());
        }

        fn u32(&mut self, off: usize, value: u32) {
            self.bytes[off..off + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn addr(&mut self, off: usize, target: usize) {
            let value = self.base + target as u64;
            self.u64(off, value);
        }

        fn string(&mut self, off: usize, text: &str) {
            let heap = self.alloc(text.len() + 1);
            self.bytes[heap..heap + text.len()].copy_from_slice(text.as_bytes());
            self.addr(off, heap);
            self.u64(off + 0x10, text.len() as u64);
            self.u64(off + 0x18, text.len() as u64 | 0x40);
        }

        fn string_vector(&mut self, off: usize, items: &[String]) {
            let data = self.alloc(items.len().max(1) * 0x20);
            for (index, item) in items.iter().enumerate() {
                self.string(data + index * 0x20, item);
            }
            self.addr(off, data);
            self.u32(off + 0x08, items.len() as u32);
            self.u32(off + 0x0c, items.len() as u32);
        }

        fn c_string(&mut self, text: &str) -> usize {
            let at = self.alloc(text.len() + 1);
            self.bytes[at..at + text.len()].copy_from_slice(text.as_bytes());
            at
        }
    }

    #[test]
    #[ignore = "measurement harness, not a gate"]
    fn engine_modset_read_cost() {
        measure_engine_modset_read(0);
        measure_engine_modset_read(40);
    }

    fn measure_engine_modset_read(mod_count: u64) {
        use theatre_selector_live_lane::engine_modset_provenance as modset;

        const SPAN: usize = 96 << 20;
        let bytes = vec![0u8; SPAN];
        let origin = bytes.as_ptr() as u64;
        let mut image = Bump {
            bytes,
            at: modset::VFS_SEARCH_PATH_CACHE_RVA as usize + 0x1000,
            base: origin,
        };
        let image_base = image.base;

        let dlcs: Vec<String> = (0..24)
            .map(|index| format!("dlc/dlc{index:03}_name/dlc{index:03}.dlc"))
            .collect();
        let mods: Vec<String> = (0..mod_count)
            .map(|index| format!("mod/ugc_{}.mod", 2_000_000_000u64 + index))
            .collect();

        let manager = image.alloc(0xc8);
        image.addr(modset::DLC_MANAGER_RVA as usize, manager);
        image.string_vector(manager + 0x50, &dlcs[..4]);
        image.string_vector(manager + 0x68, &mods);

        for (field, entries, kind) in [(0xa0usize, &dlcs, 1u32), (0xb0usize, &mods, 2u32)] {
            let head = image.alloc(0x2e0);
            image.bytes[head + 0x19] = 1;
            let nodes: Vec<usize> = entries.iter().map(|_| image.alloc(0x2e0)).collect();
            if nodes.is_empty() {
                image.addr(head + 0x08, head);
                image.addr(manager + field, head);
                image.u64(manager + field + 8, 0);
                continue;
            }
            for (index, entry) in entries.iter().enumerate() {
                let node = nodes[index];
                image.bytes[node + 0x19] = 0;
                image.addr(node, head);
                image.addr(node + 0x10, *nodes.get(index + 1).unwrap_or(&head));
                image.string(node + 0x20, entry);
                let value = node + 0x40;
                image.string(value + 0x08, &format!("Name of {entry}"));
                image.string(value + 0x28, &format!("Logged {entry}"));
                image.string(value + 0x48, entry);
                image.string(value + 0x68, entry.trim_end_matches(".mod"));
                image.string(value + 0x88, "");
                image.string(value + 0x118, "");
                let deps: Vec<String> = (0..2)
                    .map(|k| format!("dependency_{k}_of_{entry}"))
                    .collect();
                let replace: Vec<String> = (0..3).map(|k| format!("replace/path_{k}")).collect();
                image.string_vector(value + 0xa8, &deps);
                image.string_vector(value + 0xc0, &replace);
                image.u32(value + 0x248, kind);
                image.bytes[value + 0x298] = 1;
            }
            image.addr(head + 0x08, nodes[0]);
            image.addr(manager + field, head);
            image.u64(manager + field + 8, entries.len() as u64);
        }

        let roots: Vec<String> = std::iter::once("d:/steam/the game".to_string())
            .chain(dlcs.iter().chain(mods.iter()).map(|entry| {
                format!(
                    "d:/steam/the game/{}",
                    entry.trim_end_matches(".mod")
                )
            }))
            .collect();
        let list = image.alloc((roots.len() + 1) * 8);
        for (index, root) in roots.iter().enumerate() {
            let text = image.c_string(root);
            image.addr(list + index * 8, text);
        }
        image.u64(list + roots.len() * 8, 0);
        image.addr(modset::VFS_SEARCH_PATH_CACHE_RVA as usize, list);

        let once = || {
            modset::read_engine_modset_provenance_once(&InProcessEngineImage::new(), image_base)
                .expect("measurement image parses")
        };
        let expected = 24 + mod_count as usize;
        let warm = once();
        assert_eq!(warm.discovered.len(), expected, "24 DLC plus the mods");
        assert_eq!(warm.requested_mods.len(), mod_count as usize);
        assert_eq!(
            warm.discovered.iter().filter(|entry| entry.mounted).count(),
            expected,
            "every entry has a matching search root"
        );

        const RUNS: u32 = 25;
        let start = std::time::Instant::now();
        for _ in 0..RUNS {
            let _ = once();
            let _ = once();
        }
        let each = start.elapsed() / RUNS;
        println!(
            "engine modset roster: {expected} entries ({mod_count} mods), {} search roots -- two-pass read {:?} ({:?} per pass)",
            roots.len(),
            each,
            each / 2
        );
    }
}
