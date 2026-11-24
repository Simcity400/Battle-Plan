#[cfg(windows)]
mod windows {
    use std::{
        ffi::{OsStr, OsString},
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
            Diagnostics::{Debug::WriteProcessMemory, ToolHelp::*},
            LibraryLoader::*,
            Memory::*,
            Threading::*,
        },
    };

    struct Handle(HANDLE);
    impl Handle {
        fn new(handle: HANDLE) -> Result<Self, String> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                Err(io::Error::last_os_error().to_string())
            } else {
                Ok(Self(handle))
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
    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn same_file(path: &Path, expected: &Path) -> bool {
        path.canonicalize().is_ok_and(|path| {
            path.to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy())
        })
    }
    fn modules(pid: u32) -> Result<Vec<(usize, PathBuf)>, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let snapshot = loop {
            match Handle::new(unsafe {
                CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)
            }) {
                Ok(snapshot) => break snapshot,
                Err(_)
                    if unsafe { GetLastError() } == ERROR_BAD_LENGTH
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Err(error) => return Err(error),
            }
        };
        let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
        let mut found = unsafe { Module32FirstW(snapshot.0, &raw mut entry) };
        let mut result = Vec::new();
        while found != 0 {
            let length = entry
                .szExePath
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExePath.len());
            result.push((
                entry.modBaseAddr as usize,
                PathBuf::from(OsString::from_wide(&entry.szExePath[..length])),
            ));
            found = unsafe { Module32NextW(snapshot.0, &raw mut entry) };
        }
        if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(result)
    }
    fn ready(pid: u32) -> Result<(), String> {
        let name = wide(OsStr::new("Local\\hoi4_inputshim_v1"));
        let mapping = Handle::new(unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) })?;
        let view = unsafe { MapViewOfFile(mapping.0, FILE_MAP_READ, 0, 0, 4096) };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let fields = unsafe { std::slice::from_raw_parts(view.Value.cast::<u32>(), 21) };
        let result = if fields[0] == 0x4d485348 && fields[20] == pid {
            println!("attached pid={pid} installed_mask={:#x}", fields[19]);
            Ok(())
        } else {
            Err(format!(
                "Input shim did not initialize for PID {pid}; magic={:#x}, owner={}, error={}",
                fields[0], fields[20], fields[18]
            ))
        };
        unsafe {
            UnmapViewOfFile(view);
        }
        result
    }
    pub fn run() -> Result<(), String> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.len() != 3 || args[0] != "attach" {
            return Err("Usage: inputshim-inject attach <PID> <inputshim.dll>".into());
        }
        let pid: u32 = args[1]
            .to_str()
            .ok_or("Invalid PID")?
            .parse()
            .map_err(|_| "Invalid PID")?;
        let dll = Path::new(&args[2])
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let process = Handle::new(unsafe {
            OpenProcess(
                PROCESS_CREATE_THREAD
                    | PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_OPERATION
                    | PROCESS_VM_WRITE
                    | PROCESS_VM_READ
                    | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        })?;
        let remote_modules = modules(pid)?;
        if remote_modules.iter().any(|(_, path)| same_file(path, &dll)) {
            return ready(pid);
        }
        if remote_modules.iter().any(|(_, path)| {
            path.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("inputshim.dll"))
        }) {
            return Err("A different inputshim.dll is already loaded.".into());
        }
        let kernel = unsafe { GetModuleHandleW(wide(OsStr::new("kernel32.dll")).as_ptr()) };
        let loader = unsafe { GetProcAddress(kernel, c"LoadLibraryW".as_ptr().cast()) }
            .ok_or("LoadLibraryW export absent")?;
        let mut owner = ptr::null_mut();
        if unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                loader as *const u16,
                &raw mut owner,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut name = vec![0; 32768];
        let length =
            unsafe { GetModuleFileNameW(owner, name.as_mut_ptr(), name.len() as u32) } as usize;
        if length == 0 || length == name.len() {
            return Err("Loader owner path unavailable.".into());
        }
        let path = PathBuf::from(OsString::from_wide(&name[..length]));
        let filename = path.file_name().ok_or("Loader owner has no name")?;
        let remote_owner = remote_modules
            .iter()
            .find(|(_, path)| {
                path.file_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case(filename))
            })
            .ok_or("Loader owner absent in target")?;
        let address = remote_owner
            .0
            .checked_add(
                (loader as usize)
                    .checked_sub(owner as usize)
                    .ok_or("Loader offset underflow")?,
            )
            .ok_or("Loader address overflow")?;
        let path = wide(dll.as_os_str());
        let bytes = path.len() * 2;
        let remote = unsafe {
            VirtualAllocEx(
                process.0,
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
                WriteProcessMemory(process.0, remote, path.as_ptr().cast(), bytes, &raw mut written)
            } == 0
                || written != bytes
            {
                return Err(io::Error::last_os_error().to_string());
            }
            let entry = unsafe {
                std::mem::transmute::<usize, unsafe extern "system" fn(*mut std::ffi::c_void) -> u32>(
                    address,
                )
            };
            let thread = Handle::new(unsafe {
                CreateRemoteThread(
                    process.0,
                    ptr::null(),
                    0,
                    Some(entry),
                    remote,
                    0,
                    ptr::null_mut(),
                )
            })?;
            may_free = false;
            match unsafe { WaitForSingleObject(thread.0, 120000) } {
                WAIT_OBJECT_0 => may_free = true,
                WAIT_TIMEOUT => {
                    return Err(
                        "LoadLibraryW timed out; its argument is retained until process exit."
                            .into(),
                    );
                }
                _ => return Err(io::Error::last_os_error().to_string()),
            }
            if !modules(pid)?.iter().any(|(_, path)| same_file(path, &dll)) {
                return Err("Input shim DLL was not loaded.".into());
            }
            ready(pid)
        })();
        if may_free {
            unsafe {
                VirtualFreeEx(process.0, remote, 0, MEM_RELEASE);
            }
        }
        result
    }
}

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    match windows::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("Input shim injection requires Windows.");
        std::process::ExitCode::FAILURE
    }
}
