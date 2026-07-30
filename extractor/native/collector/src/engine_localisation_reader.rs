//! In-process reader for the engine's resolved localisation.
//!
//! The parser lives in [`theatre_selector_live_lane::engine_localisation_provenance`];
//! this supplies its memory. Two things make that non-trivial in the collector.
//!
//! # Why not a raw pointer dereference
//!
//! The collector is injected into the game. A raw dereference of a pointer read
//! out of engine memory would fault the *player's game* if any offset were
//! wrong or any structure were mid-construction — the failure mode this
//! project's fail-closed rule exists to avoid. `ReadProcessMemory` against our
//! own process returns an error instead of raising an access violation, so a
//! bad address becomes a refused identity rather than a crash. That is worth
//! more than the syscall it costs.
//!
//! # Why block caching
//!
//! The a729 table observed live holds 129,097 entries whose key and value text
//! lives in a 13.5 MB arena. Read naively that is a quarter of a million
//! syscalls. Access is near-sequential — each entry's key and value are adjacent
//! in the arena, and entries are walked in order — so serving reads out of
//! 64 KiB blocks collapses it to a few hundred syscalls without needing to know
//! the structure's shape in advance.

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

/// Upper bound on cached bytes. The observed arena is 13.5 MB and the entry
/// array 3.1 MB; this leaves room without becoming unbounded on a corrupt read.
const MAX_CACHED_BLOCKS: usize = 1024;

/// Reads this process's own address space, safely.
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

    /// Whether a `ReadProcessMemory` outcome may be trusted as a complete read.
    ///
    /// Split out from [`Self::raw`] so the short-read clause is reachable from a
    /// test: this platform's `ReadProcessMemory` fails a straddling read
    /// outright rather than returning a partial count, so the clause cannot be
    /// provoked through a real call.
    fn accepts(ok: i32, read: usize, requested: usize) -> bool {
        ok != 0 && read == requested
    }

    /// Read exactly `length` bytes at `address`, or `None`.
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

    /// Read one aligned block, or `None` when it is not fully readable.
    ///
    /// A partially readable block is treated as unreadable rather than
    /// zero-filled: a short read means the range crosses into unmapped memory,
    /// and silently substituting zeroes would turn that into plausible data.
    /// Callers fall back to an exact-range read, because a block straddling the
    /// end of a mapping is unreadable as a block while the bytes actually asked
    /// for are perfectly readable.
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
                // The whole block is not readable, but the span actually asked
                // for may still be. Only a failure of the exact range is a real
                // refusal.
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
        // 256 KiB guarantees the buffer crosses at least three 64 KiB blocks,
        // so the reassembly path is exercised rather than a single-block hit.
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
        assert!(InProcessEngineImage::accepts(1, 64, 64), "a full read is complete");
        assert!(
            !InProcessEngineImage::accepts(1, 63, 64),
            "a successful call that produced fewer bytes than asked for is not a complete read"
        );
        assert!(!InProcessEngineImage::accepts(0, 64, 64), "a failed call is never complete");
    }

    /// Probes whether `ReadProcessMemory` can return a *partial* count, which is
    /// what the length clause in [`InProcessEngineImage::accepts`] exists to
    /// reject. One page is committed and two pages are requested.
    #[test]
    fn a_read_straddling_the_end_of_a_mapping_never_returns_a_partial_count() {
        #[link(name = "kernel32")]
        extern "system" {
            fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32)
                -> *mut c_void;
            fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        }
        const MEM_COMMIT_RESERVE: u32 = 0x1000 | 0x2000;
        const MEM_RELEASE: u32 = 0x8000;
        const PAGE_READWRITE: u32 = 0x04;

        let page = unsafe {
            VirtualAlloc(core::ptr::null_mut(), 4096, MEM_COMMIT_RESERVE, PAGE_READWRITE)
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
        // Either outcome keeps the reader correct; the assertion records which
        // one this platform actually produces, so the length clause is not
        // mistaken for dead code if the behaviour ever changes.
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
}
