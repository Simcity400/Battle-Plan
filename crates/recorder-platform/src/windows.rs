use std::{
    ffi::{OsStr, OsString, c_void},
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{
        Diagnostics::{
            Debug::{ReadProcessMemory, WriteProcessMemory},
            ToolHelp::*,
        },
        LibraryLoader::*,
        Memory::*,
        Threading::*,
    },
};

pub struct Handle(pub HANDLE);
impl Handle {
    pub fn new(raw: HANDLE) -> Result<Self, String> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(Self(raw))
        }
    }
    pub fn signaled(&self) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(self.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error().to_string()),
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
fn text(value: &[u16]) -> OsString {
    OsString::from_wide(&value[..value.iter().position(|&v| v == 0).unwrap_or(value.len())])
}

pub fn games() -> Result<Vec<u32>, String> {
    let snapshot = Handle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?;
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    let mut pids = Vec::new();
    while found != 0 {
        if text(&entry.szExeFile)
            .to_string_lossy()
            .eq_ignore_ascii_case("hoi4.exe")
        {
            pids.push(entry.th32ProcessID);
        }
        found = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(pids)
}

pub struct Module {
    pub base: usize,
    pub path: PathBuf,
}
pub fn modules(pid: u32) -> Result<Vec<Module>, String> {
    let snapshot = loop {
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
        if raw != INVALID_HANDLE_VALUE {
            break Handle(raw);
        }
        if unsafe { GetLastError() } != ERROR_BAD_LENGTH {
            return Err(io::Error::last_os_error().to_string());
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let mut found = unsafe { Module32FirstW(snapshot.0, &mut entry) };
    let mut result = Vec::new();
    while found != 0 {
        result.push(Module {
            base: entry.modBaseAddr as usize,
            path: PathBuf::from(text(&entry.szExePath)),
        });
        found = unsafe { Module32NextW(snapshot.0, &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(result)
}

pub struct Process {
    pub handle: Handle,
    pub pid: u32,
    pub path: PathBuf,
    pub start_time: u64,
}
impl Process {
    pub fn open(pid: u32, inject: bool) -> Result<Self, String> {
        let rights = PROCESS_QUERY_LIMITED_INFORMATION
            | PROCESS_VM_READ
            | PROCESS_SYNCHRONIZE
            | if inject {
                PROCESS_CREATE_THREAD
                    | PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_OPERATION
                    | PROCESS_VM_WRITE
            } else {
                0
            };
        let handle = Handle::new(unsafe { OpenProcess(rights, 0, pid) })?;
        let mut buffer = vec![0; 32768];
        let mut len = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut len) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let path = PathBuf::from(OsString::from_wide(&buffer[..len as usize]));
        let mut times = [FILETIME::default(); 4];
        if unsafe {
            GetProcessTimes(
                handle.0,
                &mut times[0],
                &mut times[1],
                &mut times[2],
                &mut times[3],
            )
        } == 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(Self {
            handle,
            pid,
            path,
            start_time: (u64::from(times[0].dwHighDateTime) << 32)
                | u64::from(times[0].dwLowDateTime),
        })
    }

    pub fn has_module(&self, path: &Path) -> Result<bool, String> {
        let expected = path.canonicalize().map_err(|e| e.to_string())?;
        Ok(modules(self.pid)?.iter().any(|m| {
            m.path.canonicalize().is_ok_and(|p| {
                p.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy())
            })
        }))
    }

    pub fn inject(&self, dll: &Path, cancelled: impl Fn() -> bool) -> Result<(), String> {
        let dll = dll.canonicalize().map_err(|e| e.to_string())?;
        let path = wide(dll.as_os_str());
        let bytes = path.len() * 2;
        let remote = unsafe {
            VirtualAllocEx(
                self.handle.0,
                ptr::null(),
                bytes,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut may_free = true;
        let result = (|| {
            let mut written = 0;
            if unsafe {
                WriteProcessMemory(
                    self.handle.0,
                    remote,
                    path.as_ptr().cast(),
                    bytes,
                    &mut written,
                )
            } == 0
                || written != bytes
            {
                return Err(io::Error::last_os_error().to_string());
            }
            let kernel = unsafe { GetModuleHandleW(wide(OsStr::new("kernel32.dll")).as_ptr()) };
            let loader = unsafe { GetProcAddress(kernel, c"LoadLibraryW".as_ptr().cast()) }
                .ok_or("LoadLibraryW export absent")?;
            // Resolve the module actually owning the export (forwarders can land in KernelBase).
            let mut owner = ptr::null_mut();
            if unsafe {
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                        | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                    loader as *const u16,
                    &mut owner,
                )
            } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            let mut name = vec![0; 32768];
            let len = unsafe { GetModuleFileNameW(owner, name.as_mut_ptr(), name.len() as u32) };
            if len == 0 || len as usize == name.len() {
                return Err("loader owner path unavailable".into());
            }
            let owner_path = PathBuf::from(OsString::from_wide(&name[..len as usize]));
            let owner_name = owner_path
                .file_name()
                .ok_or("loader owner has no name")?
                .to_string_lossy();
            let remote_owner = modules(self.pid)?
                .into_iter()
                .find(|m| {
                    m.path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&owner_name))
                })
                .ok_or("loader owner is not mapped in game")?;
            let remote_loader = remote_owner
                .base
                .checked_add(
                    (loader as usize)
                        .checked_sub(owner as usize)
                        .ok_or("loader RVA underflow")?,
                )
                .ok_or("remote loader overflow")?;
            let entry: unsafe extern "system" fn(*mut c_void) -> u32 =
                unsafe { std::mem::transmute(remote_loader) };
            let thread = Handle::new(unsafe {
                CreateRemoteThread(
                    self.handle.0,
                    ptr::null(),
                    0,
                    Some(entry),
                    remote,
                    0,
                    ptr::null_mut(),
                )
            })?;
            may_free = false;
            let deadline = Instant::now() + Duration::from_secs(120);
            while !thread.signaled()? {
                if cancelled() {
                    return Err("injection cancelled; delayed collector will see the cancellation marker; remote argument retained until process exit".into());
                }
                if Instant::now() >= deadline {
                    return Err("LoadLibraryW timed out; its remote argument remains allocated until process exit".into());
                }
                if self.handle.signaled()? {
                    return Err("game exited during injection".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            may_free = true;
            if !self.has_module(&dll)? {
                return Err("LoadLibraryW returned but collector module is absent".into());
            }
            Ok(())
        })();
        if may_free {
            unsafe {
                VirtualFreeEx(self.handle.0, remote, 0, MEM_RELEASE);
            }
        }
        result
    }
}

/// Read-only access to another process's memory for one-shot oracles. Each
/// copy is one `ReadProcessMemory`; a partial or failed read is a failure.
pub struct ProcessMemory<'a>(pub &'a Handle);

impl engine_native::Memory for ProcessMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let mut read = 0;
        (unsafe {
            ReadProcessMemory(
                self.0.0,
                address as *const c_void,
                destination.as_mut_ptr().cast(),
                destination.len(),
                &mut read,
            )
        }) != 0
            && read == destination.len()
    }

    /// The addresses belong to another process; a local cache hint is useless.
    fn prefetch(&self, _address: usize, _len: usize) {}
}

pub struct Event {
    pub handle: Handle,
}
impl Event {
    pub fn open(name: &str) -> Result<Self, String> {
        Ok(Self {
            handle: Handle::new(unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
                    0,
                    wide(OsStr::new(name)).as_ptr(),
                )
            })?,
        })
    }
    pub fn new(name: &str) -> Result<Self, String> {
        Ok(Self {
            handle: Handle::new(unsafe {
                CreateEventW(ptr::null(), 1, 0, wide(OsStr::new(name)).as_ptr())
            })?,
        })
    }
    pub fn set(&self) -> Result<(), String> {
        if unsafe { SetEvent(self.handle.0) } == 0 {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
}
