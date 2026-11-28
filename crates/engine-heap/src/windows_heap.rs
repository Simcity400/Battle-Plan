use std::alloc::Layout;
use std::mem::size_of;
use std::num::NonZeroUsize;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    HEAP_SUMMARY, HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapFree, HeapReAlloc,
};

const ALIGNMENT: usize = 2 * size_of::<usize>();

#[derive(Clone, Copy)]
pub(super) struct PrivateHeap(NonZeroUsize);

static HEAP: OnceLock<Option<PrivateHeap>> = OnceLock::new();
type HeapSummary = unsafe extern "system" fn(HANDLE, u32, *mut HEAP_SUMMARY) -> i32;
static HEAP_SUMMARY_FN: OnceLock<Option<HeapSummary>> = OnceLock::new();

pub(super) fn for_layout(layout: Layout) -> Option<PrivateHeap> {
    if layout.align() > ALIGNMENT || super::pages::own(layout.size(), layout.align()) {
        return None;
    }
    *HEAP.get_or_init(|| {
        // SAFETY: flags zero creates a serialized heap with no fixed maximum size.
        // The process keeps the handle for its lifetime.
        NonZeroUsize::new(unsafe { HeapCreate(0, 0, 0) } as usize).map(PrivateHeap)
    })
}

impl PrivateHeap {
    fn handle(self) -> HANDLE {
        self.0.get() as HANDLE
    }

    pub(super) unsafe fn alloc(self, size: usize, zero: bool) -> *mut u8 {
        let flags = if zero { HEAP_ZERO_MEMORY } else { 0 };
        // SAFETY: handle belongs to the process's live private heap. HeapAlloc accepts any byte count.
        unsafe { HeapAlloc(self.handle(), flags, size).cast() }
    }

    pub(super) unsafe fn free(self, pointer: *mut u8) {
        // SAFETY: the caller supplied a live block from this heap.
        unsafe { HeapFree(self.handle(), 0, pointer.cast()) };
    }

    pub(super) unsafe fn resize(self, pointer: *mut u8, size: usize) -> *mut u8 {
        // SAFETY: the caller supplied a live block from this heap. On failure HeapReAlloc keeps it live.
        unsafe { HeapReAlloc(self.handle(), 0, pointer.cast(), size).cast() }
    }
}

pub(super) fn committed() -> Option<usize> {
    let Some(Some(heap)) = HEAP.get() else {
        return Some(0);
    };
    let mut summary = HEAP_SUMMARY {
        cb: size_of::<HEAP_SUMMARY>() as u32,
        ..HEAP_SUMMARY::default()
    };
    let summary_fn = (*HEAP_SUMMARY_FN.get_or_init(|| {
        // SAFETY: both lookup names are static and nul-terminated. Kernel32 stays loaded.
        let address = unsafe {
            let kernel = GetModuleHandleW(windows_sys::w!("kernel32.dll"));
            GetProcAddress(kernel, c"HeapSummary".as_ptr().cast())
        }?;
        // SAFETY: the system export uses the HeapSummary signature when present.
        Some(unsafe { std::mem::transmute::<_, HeapSummary>(address) })
    }))?;
    // SAFETY: the handle is live and summary is writable with the size HeapSummary requires.
    let read = unsafe { summary_fn(heap.handle(), 0, &raw mut summary) };
    (read != 0).then_some(summary.cbCommitted)
}
