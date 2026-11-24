
use std::{ffi::c_void, os::windows::ffi::OsStrExt as _, path::Path, ptr};
use windows_sys::{
    Win32::{
        Foundation::{FreeLibrary, HMODULE},
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::LibraryLoader::{GetProcAddress, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW},
    },
    core::{GUID, HRESULT},
};

const CLSID_DXC_COMPILER: GUID = GUID::from_u128(0x73e22d93_e6ce_47f3_b5bf_f0664f39c1b0);
const IID_COMPILER3: GUID = GUID::from_u128(0x228b4687_5a6a_4730_900c_9702b2203f54);
const IID_RESULT: GUID = GUID::from_u128(0x58346cda_dde7_4497_9461_6f87af5e0659);
const CP_UTF8: u32 = 65001;

fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain([0]).collect()
}

pub(crate) fn product_version(path: &Path) -> Result<String, String> {
    const MISSING: &str = "dxcompiler.dll carries no product version";
    let file = wide(path);
    unsafe {
        let size = GetFileVersionInfoSizeW(file.as_ptr(), ptr::null_mut());
        if size == 0 {
            return Err(MISSING.into());
        }
        let mut block = vec![0u8; size as usize];
        if GetFileVersionInfoW(file.as_ptr(), 0, size, block.as_mut_ptr().cast()) == 0 {
            return Err(MISSING.into());
        }
        let (mut found, mut length) = (ptr::null_mut(), 0u32);
        if VerQueryValueW(block.as_ptr().cast(), wide("\\VarFileInfo\\Translation").as_ptr(), &raw mut found, &raw mut length) == 0 || length < 4 {
            return Err(MISSING.into());
        }
        let language = *found.cast::<u16>();
        let page = *found.cast::<u16>().add(1);
        let key = wide(format!("\\StringFileInfo\\{language:04x}{page:04x}\\ProductVersion"));
        if VerQueryValueW(block.as_ptr().cast(), key.as_ptr(), &raw mut found, &raw mut length) == 0 || length == 0 {
            return Err(MISSING.into());
        }
        let text = std::slice::from_raw_parts(found.cast::<u16>(), length as usize);
        Ok(String::from_utf16_lossy(text).trim_end_matches('\0').to_owned())
    }
}

type CreateInstance =
    unsafe extern "system" fn(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;

#[repr(C)]
struct UnknownVtbl {
    _query_interface:
        unsafe extern "system" fn(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT,
    _add_ref: unsafe extern "system" fn(this: *mut c_void) -> u32,
    release: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

#[repr(C)]
struct Buffer {
    ptr: *const c_void,
    size: usize,
    encoding: u32,
}

#[repr(C)]
struct Compiler3Vtbl {
    unknown: UnknownVtbl,
    compile: unsafe extern "system" fn(
        this: *mut c_void,
        source: *const Buffer,
        arguments: *const *const u16,
        count: u32,
        include: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT,
}

#[repr(C)]
struct ResultVtbl {
    unknown: UnknownVtbl,
    status: unsafe extern "system" fn(this: *mut c_void, status: *mut HRESULT) -> HRESULT,
    result: unsafe extern "system" fn(this: *mut c_void, blob: *mut *mut c_void) -> HRESULT,
    errors: unsafe extern "system" fn(this: *mut c_void, blob: *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct BlobVtbl {
    unknown: UnknownVtbl,
    pointer: unsafe extern "system" fn(this: *mut c_void) -> *mut c_void,
    size: unsafe extern "system" fn(this: *mut c_void) -> usize,
}

struct Com<V>(ptr::NonNull<*const V>);

impl<V> Com<V> {
    /// # Safety
    /// `raw` is null or an interface pointer of `V`'s layout, whose reference this takes.
    unsafe fn from_raw(raw: *mut c_void) -> Option<Self> {
        ptr::NonNull::new(raw.cast()).map(Self)
    }

    fn this(&self) -> *mut c_void {
        self.0.as_ptr().cast()
    }

    fn vtbl(&self) -> &V {
        unsafe { &**self.0.as_ptr() }
    }

}

impl<V> Drop for Com<V> {
    fn drop(&mut self) {
        // SAFETY: every vtable here begins with IUnknown's; this holds one reference.
        unsafe {
            let unknown = &*(*self.0.as_ptr()).cast::<UnknownVtbl>();
            (unknown.release)(self.this());
        }
    }
}

fn blob_bytes(blob: &Com<BlobVtbl>) -> Vec<u8> {
    // SAFETY: the blob owns `size` bytes at `pointer` while it lives.
    unsafe {
        let pointer = (blob.vtbl().pointer)(blob.this()).cast::<u8>();
        let size = (blob.vtbl().size)(blob.this());
        if pointer.is_null() || size == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(pointer, size).to_vec()
        }
    }
}

pub(crate) struct Library {
    create: CreateInstance,
}

impl Library {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let file = wide(path);
        // SAFETY: a NUL-terminated path; the module is never freed once it serves.
        let module: HMODULE = unsafe { LoadLibraryExW(file.as_ptr(), ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
        if module.is_null() {
            return Err(format!("dxcompiler.dll did not load: {}", std::io::Error::last_os_error()));
        }
        match unsafe { GetProcAddress(module, c"DxcCreateInstance".as_ptr().cast()) } {
            Some(create) => Ok(Self {
                create: unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateInstance>(create) },
            }),
            None => {
                unsafe { FreeLibrary(module) };
                Err("dxcompiler.dll has no DxcCreateInstance".into())
            }
        }
    }

    fn compiler(&self) -> Result<Com<Compiler3Vtbl>, String> {
        let mut out = ptr::null_mut();
        let hr = unsafe { (self.create)(&CLSID_DXC_COMPILER, &IID_COMPILER3, &raw mut out) };
        if hr < 0 {
            return Err(format!("DXC made no compiler (HRESULT {hr:#010x})"));
        }
        unsafe { Com::from_raw(out) }.ok_or_else(|| "DXC made no compiler".to_owned())
    }

    pub(crate) fn compile(&self, source: &str, arguments: &[String]) -> Result<Vec<u8>, String> {
        let compiler = self.compiler()?;
        let wide: Vec<Vec<u16>> = arguments.iter().map(|argument| argument.encode_utf16().chain([0]).collect()).collect();
        let pointers: Vec<*const u16> = wide.iter().map(|argument| argument.as_ptr()).collect();
        let buffer = Buffer { ptr: source.as_ptr().cast(), size: source.len(), encoding: CP_UTF8 };
        let mut out = ptr::null_mut();
        let hr = unsafe {
            (compiler.vtbl().compile)(
                compiler.this(),
                &raw const buffer,
                pointers.as_ptr(),
                pointers.len() as u32,
                ptr::null_mut(),
                &IID_RESULT,
                &raw mut out,
            )
        };
        let result = (hr >= 0).then(|| unsafe { Com::<ResultVtbl>::from_raw(out) }).flatten();
        let Some(result) = result else {
            return Err(format!("DXC did not compile (HRESULT {hr:#010x})"));
        };
        let mut status = 0;
        unsafe {
            if (result.vtbl().status)(result.this(), &raw mut status) < 0 {
                return Err("DXC gave no status".into());
            }
            if status < 0 {
                let mut errors = ptr::null_mut();
                let text = match ((result.vtbl().errors)(result.this(), &raw mut errors) >= 0)
                    .then(|| Com::<BlobVtbl>::from_raw(errors))
                    .flatten()
                {
                    Some(blob) => String::from_utf8_lossy(&blob_bytes(&blob)).trim_end_matches('\0').to_owned(),
                    None => String::new(),
                };
                return Err(if text.is_empty() { format!("HRESULT {status:#010x}") } else { text });
            }
            let mut object = ptr::null_mut();
            if (result.vtbl().result)(result.this(), &raw mut object) < 0 {
                return Err("DXC gave no object".into());
            }
            let blob = Com::<BlobVtbl>::from_raw(object).ok_or("DXC gave no object")?;
            Ok(blob_bytes(&blob))
        }
    }
}
