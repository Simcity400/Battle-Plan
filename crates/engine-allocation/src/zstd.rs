
use crate::labels::{self, Kind, Owner};
use std::{
    alloc::{Layout, alloc, dealloc},
    ffi::{CStr, c_void},
    io,
    mem::MaybeUninit,
    ptr::NonNull,
    sync::OnceLock,
};
use zstd_sys::{
    ZSTD_CCtx, ZSTD_DCtx, ZSTD_ErrorCode, ZSTD_ResetDirective, ZSTD_cParameter, ZSTD_customMem,
    ZSTD_inBuffer, ZSTD_outBuffer,
};

pub struct Holder {
    name: &'static str,
    owner: OnceLock<Owner>,
}

impl Holder {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            owner: OnceLock::new(),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    fn owner(&self) -> Owner {
        *self.owner.get_or_init(|| labels::owner(self.name))
    }

    fn memory(&'static self) -> ZSTD_customMem {
        ZSTD_customMem {
            customAlloc: Some(allocate),
            customFree: Some(free),
            opaque: std::ptr::from_ref(self).cast_mut().cast(),
        }
    }
}

const HEADER: usize = 16;

unsafe extern "C" fn allocate(opaque: *mut c_void, size: usize) -> *mut c_void {
    // SAFETY: `Holder::memory` passes a `&'static Holder`.
    let holder = unsafe { &*opaque.cast::<Holder>() };
    let _memory = labels::enter(holder.owner(), Kind::Zstd);
    let Some(layout) = size
        .checked_add(HEADER)
        .and_then(|total| Layout::from_size_align(total, HEADER).ok())
    else {
        return std::ptr::null_mut();
    };
    if crate::allocation_attempt().is_err() {
        return std::ptr::null_mut();
    }
    unsafe {
        let base = alloc(layout);
        if base.is_null() {
            return std::ptr::null_mut();
        }
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "alloc returned base for a layout aligned to HEADER, which is 16"
        )]
        base.cast::<usize>().write(layout.size());
        base.add(HEADER).cast()
    }
}

unsafe extern "C" fn free(_: *mut c_void, address: *mut c_void) {
    if address.is_null() {
        return;
    }
    // SAFETY: `allocate` made the block, its size written ahead of it.
    unsafe {
        let base = address.cast::<u8>().sub(HEADER);
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "allocate returned address as a 16-aligned base plus HEADER, so base is 16-aligned"
        )]
        let size = base.cast::<usize>().read();
        dealloc(base, Layout::from_size_align_unchecked(size, HEADER));
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Error(usize);

impl Error {
    pub fn name(self) -> &'static str {
        unsafe { CStr::from_ptr(zstd_sys::ZSTD_getErrorName(self.0)) }
            .to_str()
            .unwrap_or("zstd error")
    }

    fn memory(self) -> bool {
        let code = unsafe { zstd_sys::ZSTD_getErrorCode(self.0) };
        code == ZSTD_ErrorCode::ZSTD_error_memory_allocation
    }

    pub fn invalid(self) -> io::Error {
        self.refused(|refusal| refusal)
    }

    pub fn refused(self, wrap: impl FnOnce(io::Error) -> io::Error) -> io::Error {
        if self.memory() {
            crate::oom()
        } else {
            wrap(io::Error::new(io::ErrorKind::InvalidData, self.name()))
        }
    }
}

impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        if error.memory() {
            crate::oom()
        } else {
            io::Error::other(error.name())
        }
    }
}

fn checked(code: usize) -> Result<usize, Error> {
    if unsafe { zstd_sys::ZSTD_isError(code) } != 0 {
        Err(Error(code))
    } else {
        Ok(code)
    }
}

pub fn bound(raw: usize) -> usize {
    unsafe { zstd_sys::ZSTD_compressBound(raw) }
}

pub fn content_size(frame: &[u8]) -> Option<u64> {
    let size = unsafe { zstd_sys::ZSTD_getFrameContentSize(frame.as_ptr().cast(), frame.len()) };
    (size < u64::MAX - 1).then_some(size)
}

pub fn frame_size(stored: &[u8]) -> Option<usize> {
    checked(unsafe { zstd_sys::ZSTD_findFrameCompressedSize(stored.as_ptr().cast(), stored.len()) }).ok()
}

pub struct Compressor(NonNull<ZSTD_CCtx>);

// SAFETY: a zstd context may move between threads; it is used by one at a time.
unsafe impl Send for Compressor {}

impl Compressor {
    pub fn new(holder: &'static Holder, level: i32) -> io::Result<Self> {
        // SAFETY: the holder's functions allocate and free through the global allocator.
        let context = unsafe { zstd_sys::ZSTD_createCCtx_advanced(holder.memory()) };
        let mut context = Self(NonNull::new(context).ok_or_else(crate::oom)?);
        context.set(ZSTD_cParameter::ZSTD_c_compressionLevel, level)?;
        Ok(context)
    }

    pub fn set(&mut self, parameter: ZSTD_cParameter, value: i32) -> io::Result<()> {
        checked(unsafe { zstd_sys::ZSTD_CCtx_setParameter(self.0.as_ptr(), parameter, value) })?;
        Ok(())
    }

    pub fn level(&mut self, level: i32) -> io::Result<()> {
        self.set(ZSTD_cParameter::ZSTD_c_compressionLevel, level)
    }

    pub fn window_log(&mut self, log: u32) -> io::Result<()> {
        self.set(ZSTD_cParameter::ZSTD_c_windowLog, log as i32)
    }

    pub fn cap_window(&mut self, level: i32, log: u32) -> io::Result<()> {
        let params = unsafe { zstd_sys::ZSTD_getCParams(level, 1 << log, 0) };
        self.set(ZSTD_cParameter::ZSTD_c_windowLog, log as i32)?;
        self.set(ZSTD_cParameter::ZSTD_c_chainLog, params.chainLog as i32)?;
        self.set(ZSTD_cParameter::ZSTD_c_hashLog, params.hashLog as i32)
    }

    pub fn compress(&mut self, dst: &mut [u8], src: &[u8]) -> Result<usize, Error> {
        checked(unsafe {
            zstd_sys::ZSTD_compress2(
                self.0.as_ptr(),
                dst.as_mut_ptr().cast(),
                dst.len(),
                src.as_ptr().cast(),
                src.len(),
            )
        })
    }

    pub fn compress_append(&mut self, src: &[u8], out: &mut Vec<u8>) -> Result<usize, Error> {
        let at = out.len();
        unsafe {
            let written = checked(zstd_sys::ZSTD_compress2(
                self.0.as_ptr(),
                out.as_mut_ptr().add(at).cast(),
                out.capacity() - at,
                src.as_ptr().cast(),
                src.len(),
            ))?;
            out.set_len(at + written);
            Ok(written)
        }
    }

    pub fn compress_append_prefixed(&mut self, prefix: &[u8], src: &[u8], out: &mut Vec<u8>) -> Result<usize, Error> {
        // SAFETY: a live context; zstd keeps a reference to the prefix for the next frame only,
        // which ends before this call returns.
        checked(unsafe { zstd_sys::ZSTD_CCtx_refPrefix(self.0.as_ptr(), prefix.as_ptr().cast(), prefix.len()) })?;
        self.compress_append(src, out)
    }

    pub fn bytes(&self) -> usize {
        unsafe { zstd_sys::ZSTD_sizeof_CCtx(self.0.as_ptr()) }
    }
}

impl Drop for Compressor {
    fn drop(&mut self) {
        // SAFETY: the context is this one's alone.
        unsafe { zstd_sys::ZSTD_freeCCtx(self.0.as_ptr()) };
    }
}

pub struct Decompressor(NonNull<ZSTD_DCtx>);

// SAFETY: a zstd context may move between threads; it is used by one at a time.
unsafe impl Send for Decompressor {}

impl Decompressor {
    pub fn new(holder: &'static Holder) -> io::Result<Self> {
        // SAFETY: the holder's functions allocate and free through the global allocator.
        let context = unsafe { zstd_sys::ZSTD_createDCtx_advanced(holder.memory()) };
        Ok(Self(NonNull::new(context).ok_or_else(crate::oom)?))
    }

    pub fn decompress(&mut self, dst: &mut [u8], src: &[u8]) -> Result<usize, Error> {
        checked(unsafe {
            zstd_sys::ZSTD_decompressDCtx(
                self.0.as_ptr(),
                dst.as_mut_ptr().cast(),
                dst.len(),
                src.as_ptr().cast(),
                src.len(),
            )
        })
    }

    pub fn decompress_into(&mut self, dst: &mut [MaybeUninit<u8>], src: &[u8]) -> Result<usize, Error> {
        checked(unsafe {
            zstd_sys::ZSTD_decompressDCtx(
                self.0.as_ptr(),
                dst.as_mut_ptr().cast(),
                dst.len(),
                src.as_ptr().cast(),
                src.len(),
            )
        })
    }

    pub fn decompress_append(&mut self, src: &[u8], out: &mut Vec<u8>) -> Result<usize, Error> {
        let at = out.len();
        unsafe {
            let written = checked(zstd_sys::ZSTD_decompressDCtx(
                self.0.as_ptr(),
                out.as_mut_ptr().add(at).cast(),
                out.capacity() - at,
                src.as_ptr().cast(),
                src.len(),
            ))?;
            out.set_len(at + written);
            Ok(written)
        }
    }

    pub fn decompress_append_prefixed(
        &mut self,
        prefix: &[u8],
        src: &[u8],
        out: &mut Vec<u8>,
    ) -> Result<usize, Error> {
        // SAFETY: a live context; zstd keeps a reference to the prefix, dropped below.
        checked(unsafe {
            zstd_sys::ZSTD_DCtx_refPrefix(self.0.as_ptr(), prefix.as_ptr().cast(), prefix.len())
        })?;
        let written = self.decompress_append(src, out);
        let cleared =
            checked(unsafe { zstd_sys::ZSTD_DCtx_refPrefix(self.0.as_ptr(), std::ptr::null(), 0) });
        let written = written?;
        cleared?;
        Ok(written)
    }

    pub fn reset(&mut self) -> Result<(), Error> {
        checked(unsafe {
            zstd_sys::ZSTD_DCtx_reset(
                self.0.as_ptr(),
                ZSTD_ResetDirective::ZSTD_reset_session_only,
            )
        })?;
        Ok(())
    }

    pub fn stream(
        &mut self,
        out: &mut Vec<u8>,
        src: &[u8],
        read: &mut usize,
    ) -> Result<bool, Error> {
        let mut output = ZSTD_outBuffer {
            dst: out.as_mut_ptr().cast(),
            size: out.capacity(),
            pos: out.len(),
        };
        let mut input = ZSTD_inBuffer {
            src: src.as_ptr().cast(),
            size: src.len(),
            pos: (*read).min(src.len()),
        };
        let ended = checked(unsafe {
            zstd_sys::ZSTD_decompressStream(self.0.as_ptr(), &raw mut output, &raw mut input)
        })? == 0;
        unsafe { out.set_len(output.pos) };
        *read = input.pos;
        Ok(ended)
    }

    pub fn size(&self) -> usize {
        unsafe { zstd_sys::ZSTD_sizeof_DCtx(self.0.as_ptr()) }
    }
}

impl Drop for Decompressor {
    fn drop(&mut self) {
        // SAFETY: the context is this one's alone.
        unsafe { zstd_sys::ZSTD_freeDCtx(self.0.as_ptr()) };
    }
}
