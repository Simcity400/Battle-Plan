use std::ffi::c_void;
use std::fs::File;
use std::ops::Deref;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

type Handle = *mut c_void;

const PAGE_READONLY: u32 = 0x02;
const FILE_MAP_READ: u32 = 0x0004;

extern "system" {
    fn CreateFileMappingW(
        file: Handle,
        attributes: *mut c_void,
        protect: u32,
        max_size_high: u32,
        max_size_low: u32,
        name: *const u16,
    ) -> Handle;
    fn MapViewOfFile(
        mapping: Handle,
        desired_access: u32,
        offset_high: u32,
        offset_low: u32,
        bytes_to_map: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(base: *const c_void) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

/// mapping handle are held for the view's lifetime and released on drop.
pub struct Mmap {
    _file: File,
    mapping: Handle,
    ptr: *const u8,
    len: usize,
}

// The mapping is read-only and immutable for the guard's lifetime.
unsafe impl Send for Mmap {}
unsafe impl Sync for Mmap {}

impl Mmap {
    pub fn open(path: &Path) -> std::io::Result<Mmap> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len == 0 {
            return Ok(Mmap {
                _file: file,
                mapping: std::ptr::null_mut(),
                ptr: std::ptr::null(),
                len: 0,
            });
        }
        let len_usize = usize::try_from(len).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "stream larger than address space",
            )
        })?;
        let handle = file.as_raw_handle() as Handle;
        let mapping = unsafe {
            CreateFileMappingW(
                handle,
                std::ptr::null_mut(),
                PAGE_READONLY,
                0,
                0,
                std::ptr::null(),
            )
        };
        if mapping.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0) };
        if view.is_null() {
            let err = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(mapping);
            }
            return Err(err);
        }
        Ok(Mmap {
            _file: file,
            mapping,
            ptr: view as *const u8,
            len: len_usize,
        })
    }
}

impl Deref for Mmap {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: `ptr` maps `len` valid, initialized, read-only bytes for the
        // lifetime of `self` (the mapping + file handle are held until drop).
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                UnmapViewOfFile(self.ptr as *const c_void);
            }
            if !self.mapping.is_null() {
                CloseHandle(self.mapping);
            }
        }
    }
}
