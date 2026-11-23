use crate::games::{GameProcess, Thread, classify};
use std::{
    ffi::{OsStr, OsString, c_void},
    io,
    mem::size_of,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
    time::{Duration, Instant},
};
use windows_sys::{
    Wdk::System::{
        SystemInformation::{NtQuerySystemInformation, SystemProcessInformation},
        Threading::{NtQueryInformationProcess, ProcessBasicInformation},
    },
    Win32::{
        Foundation::*,
        System::{
            Diagnostics::{
                Debug::{ReadProcessMemory, WriteProcessMemory},
                ToolHelp::*,
            },
            LibraryLoader::*,
            Memory::*,
            Threading::*,
            Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime},
            WindowsProgramming::{SYSTEM_PROCESS_INFORMATION, SYSTEM_THREAD_INFORMATION},
        },
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
        self.wait(0)
    }
    pub fn wait(&self, milliseconds: u32) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(self.0, milliseconds) } {
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

pub fn game_processes() -> Result<Vec<GameProcess>, String> {
    game_processes_named("hoi4.exe")
}

pub fn game_present(pid: u32, start_time: u64) -> Result<bool, String> {
    Ok(game_processes()?
        .iter()
        .any(|game| game.pid == pid && game.start_time == start_time))
}

pub fn game_processes_named(name: &str) -> Result<Vec<GameProcess>, String> {
    let mut found = Vec::new();
    visit_named_processes(name, |game, _| found.push(game))?;
    Ok(found)
}

fn visit_named_processes(
    name: &str,
    mut on_match: impl FnMut(GameProcess, &[Thread]),
) -> Result<(), String> {
    let listing = system_processes()?;
    let bytes_len = listing.len() * size_of::<u64>();
    let base = listing.as_ptr().cast::<u8>();
    let mut offset = 0usize;
    loop {
        let Some(header_end) = offset.checked_add(size_of::<SYSTEM_PROCESS_INFORMATION>()) else {
            break;
        };
        if header_end > bytes_len {
            break;
        }
        // SAFETY: offset < header_end <= bytes_len, so base.add(offset) stays within listing.
        let header_ptr = unsafe { base.add(offset) }.cast::<SYSTEM_PROCESS_INFORMATION>();
        if !header_ptr.is_aligned() {
            break;
        }
        // SAFETY: header_ptr is aligned and offset..header_end lies within the OS-owned listing.
        let entry = unsafe { &*header_ptr };
        let image = &entry.ImageName;
        if !image.Buffer.is_null()
            && image.Buffer.is_aligned()
            // SAFETY: NtQuerySystemInformation populated ImageName.Buffer with ImageName.Length bytes of UTF-16 inside listing.
            && OsString::from_wide(unsafe {
                std::slice::from_raw_parts(image.Buffer, usize::from(image.Length) / 2)
            })
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            let count = entry.NumberOfThreads as usize;
            let raw_threads = if count == 0 {
                &[]
            } else {
                let Some(threads_bytes) = count.checked_mul(size_of::<SYSTEM_THREAD_INFORMATION>())
                else {
                    break;
                };
                let Some(threads_end) = header_end.checked_add(threads_bytes) else {
                    break;
                };
                if threads_end > bytes_len {
                    break;
                }
                // SAFETY: header_end <= threads_end <= bytes_len, derived directly from base so
                // provenance spans the full listing buffer rather than narrowing through entry.
                let threads_ptr =
                    unsafe { base.add(header_end) }.cast::<SYSTEM_THREAD_INFORMATION>();
                if !threads_ptr.is_aligned() {
                    break;
                }
                // SAFETY: threads_ptr is aligned, disjoint from entry, and header_end..threads_end lies within listing.
                unsafe { std::slice::from_raw_parts(threads_ptr, count) }
            };
            let threads: Vec<Thread> = raw_threads
                .iter()
                .map(|thread| Thread {
                    state: thread.ThreadState,
                    wait_reason: thread.WaitReason,
                })
                .collect();
            let pid = entry.UniqueProcessId as usize as u32;
            let (deleting, signalled) = exit_facts(pid);
            let game = GameProcess {
                pid,
                start_time: u64::from_le_bytes(entry.Reserved1[24..32].try_into().unwrap()),
                state: classify(&threads, deleting, signalled),
            };
            on_match(game, &threads);
        }
        if entry.NextEntryOffset == 0 {
            break;
        }
        let Some(next) = offset.checked_add(entry.NextEntryOffset as usize) else {
            break;
        };
        offset = next;
    }
    Ok(())
}

fn system_processes() -> Result<Vec<u64>, String> {
    let mut needed = 0u32;
    let mut buffer: Vec<u64> = Vec::new();
    loop {
        let bytes = (buffer.len() * size_of::<u64>()) as u32;
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut needed,
            )
        };
        match status {
            0 if !buffer.is_empty() => return Ok(buffer),
            0 | STATUS_INFO_LENGTH_MISMATCH => {
                buffer = vec![0; (needed as usize).div_ceil(size_of::<u64>()).max(1)];
            }
            status => return Err(format!("listing processes failed with status {status:#x}")),
        }
    }
}

fn exit_facts(pid: u32) -> (bool, bool) {
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    let Ok(handle) = Handle::new(raw) else {
        let gone = unsafe { GetLastError() } == ERROR_INVALID_PARAMETER;
        return (gone, gone);
    };
    #[repr(C)]
    struct ExtendedBasicInformation {
        size: usize,
        basic: [usize; 6],
        flags: u32,
    }
    const PROCESS_DELETING: u32 = 1 << 2;
    let mut information = ExtendedBasicInformation {
        size: size_of::<ExtendedBasicInformation>(),
        basic: [0; 6],
        flags: 0,
    };
    let deleting = unsafe {
        NtQueryInformationProcess(
            handle.0,
            ProcessBasicInformation,
            ptr::from_mut(&mut information).cast(),
            size_of::<ExtendedBasicInformation>() as u32,
            ptr::null_mut(),
        )
    } == 0
        && information.flags & PROCESS_DELETING != 0;
    (deleting, handle.signaled().unwrap_or(false))
}

pub fn local_clock(filetime: u64) -> Option<(u16, u16)> {
    let universal = FILETIME {
        dwLowDateTime: filetime as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    (unsafe { FileTimeToSystemTime(&universal, &mut utc) } != 0
        && unsafe { SystemTimeToTzSpecificLocalTime(ptr::null(), &utc, &mut local) } != 0)
        .then_some((local.wHour, local.wMinute))
}

pub struct Module {
    pub base: usize,
    pub path: PathBuf,
}
const MODULE_SNAPSHOT_WAIT: Duration = Duration::from_millis(100);
const MODULE_SNAPSHOT_RETRY: Duration = Duration::from_millis(1);

fn module_snapshot<T>(
    deadline: Instant,
    mut create: impl FnMut() -> io::Result<T>,
    mut now: impl FnMut() -> Instant,
    mut sleep: impl FnMut(Duration),
) -> io::Result<T> {
    loop {
        if now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "module snapshot remained unstable",
            ));
        }
        match create() {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) if error.raw_os_error() == Some(ERROR_BAD_LENGTH as i32) => {}
            Err(error) => return Err(error),
        }
        let remaining = deadline.saturating_duration_since(now());
        if !remaining.is_zero() {
            sleep(remaining.min(MODULE_SNAPSHOT_RETRY));
        }
    }
}

pub fn modules(pid: u32) -> Result<Vec<Module>, String> {
    modules_until(pid, Instant::now() + MODULE_SNAPSHOT_WAIT)
}

pub fn modules_until(pid: u32, deadline: Instant) -> Result<Vec<Module>, String> {
    let snapshot = module_snapshot(
        deadline.min(Instant::now() + MODULE_SNAPSHOT_WAIT),
        || {
            let raw =
                unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
            if raw == INVALID_HANDLE_VALUE {
                Err(io::Error::last_os_error())
            } else {
                Ok(Handle(raw))
            }
        },
        Instant::now,
        std::thread::sleep,
    )
    .map_err(|error| error.to_string())?;
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

    pub fn open_selected(game: &GameProcess, inject: bool) -> Result<Option<Self>, String> {
        match Self::open(game.pid, inject) {
            Ok(process) => Ok((process.start_time == game.start_time).then_some(process)),
            Err(_) if !game_present(game.pid, game.start_time)? => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn has_module(&self, path: &Path) -> Result<bool, String> {
        self.has_module_until(path, Instant::now() + MODULE_SNAPSHOT_WAIT)
    }

    pub fn has_module_until(&self, path: &Path, deadline: Instant) -> Result<bool, String> {
        let expected = path.canonicalize().map_err(|e| e.to_string())?;
        Ok(modules_until(self.pid, deadline)?.iter().any(|m| {
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

    fn prefetch(&self, _address: usize, _len: usize) {}
}

pub struct Event {
    pub handle: Handle,
}
// SAFETY: an event handle may be signalled, reset and waited on from any thread.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
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

pub struct Claim {
    _held: Handle,
}

impl Claim {
    pub fn take(name: &str) -> Result<Option<Self>, String> {
        let name = wide(OsStr::new(name));
        unsafe { SetLastError(0) };
        let raw = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        let error = unsafe { GetLastError() };
        if raw.is_null() {
            return if error == ERROR_ACCESS_DENIED {
                Ok(None)
            } else {
                Err(io::Error::from_raw_os_error(error as i32).to_string())
            };
        }
        let handle = Handle(raw);
        Ok((error != ERROR_ALREADY_EXISTS).then_some(Self { _held: handle }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn module_snapshot_churn_expires_without_another_attempt() {
        for budget in [MODULE_SNAPSHOT_WAIT, Duration::from_micros(2500)] {
            let started = Instant::now();
            let elapsed = Cell::new(Duration::ZERO);
            let attempts = Cell::new(0);
            let result = module_snapshot::<()>(
                started + budget,
                || {
                    assert!(elapsed.get() < budget);
                    attempts.set(attempts.get() + 1);
                    Err(io::Error::from_raw_os_error(ERROR_BAD_LENGTH as i32))
                },
                || started + elapsed.get(),
                |delay| {
                    assert!(!delay.is_zero() && delay <= MODULE_SNAPSHOT_RETRY);
                    elapsed.set(elapsed.get() + delay);
                },
            );
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert_eq!(elapsed.get(), budget);
            assert_eq!(attempts.get(), budget.as_micros().div_ceil(1000) as u32);
        }
    }

    #[test]
    fn module_snapshot_retries_only_bad_length_and_can_recover() {
        let started = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let attempts = Cell::new(0);
        let snapshot = module_snapshot(
            started + MODULE_SNAPSHOT_WAIT,
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() < 3 {
                    Err(io::Error::from_raw_os_error(ERROR_BAD_LENGTH as i32))
                } else {
                    Ok(42)
                }
            },
            || started + elapsed.get(),
            |delay| elapsed.set(elapsed.get() + delay),
        )
        .unwrap();
        assert_eq!(snapshot, 42);
        assert_eq!(attempts.get(), 3);
        assert_eq!(elapsed.get(), MODULE_SNAPSHOT_RETRY * 2);

        let error = module_snapshot::<()>(
            started + MODULE_SNAPSHOT_WAIT,
            || Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32)),
            || started,
            |_| panic!("permanent snapshot errors must not retry"),
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
    }

    #[test]
    fn expired_module_snapshot_deadline_does_not_start_an_observation() {
        let deadline = Instant::now();
        let error = module_snapshot::<()>(
            deadline,
            || panic!("expired snapshot must not start"),
            || deadline,
            |_| panic!("expired snapshot must not wait"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn a_claimed_name_is_refused_until_its_holder_lets_it_go() {
        let name = format!(
            "Local\\BattlePlan.ClaimTest.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let first = Claim::take(&name).unwrap().expect("a fresh name");
        assert!(Claim::take(&name).unwrap().is_none());
        drop(first);
        let again = Claim::take(&name).unwrap();
        assert!(again.is_some());
    }

    #[test]
    fn a_selected_process_opens_only_while_it_has_the_selected_start_time() {
        let pid = std::process::id();
        let start_time = Process::open(pid, false).unwrap().start_time;
        let selected = GameProcess {
            pid,
            start_time,
            state: crate::games::ProcessState::Running,
        };
        let opened = Process::open_selected(&selected, false).unwrap();
        assert_eq!(opened.map(|process| process.start_time), Some(start_time));
        for start_time in [start_time - 1, start_time + 1] {
            let reused = GameProcess {
                start_time,
                ..selected
            };
            assert!(Process::open_selected(&reused, false).unwrap().is_none());
        }
    }

    const READY_CHILD: &str = "BATTLEPLAN_TEST_READY_CHILD";
    const HELD_CHILD: &str = "BATTLEPLAN_TEST_HELD_CHILD";

    #[test]
    fn a_held_child_waits_for_its_release() {
        if let Some(name) = std::env::var_os(HELD_CHILD) {
            let release = Event::open(&name.to_string_lossy()).unwrap();
            if let Some(ready_name) = std::env::var_os(READY_CHILD) {
                Event::open(&ready_name.to_string_lossy())
                    .unwrap()
                    .set()
                    .unwrap();
            }
            assert!(release.handle.wait(INFINITE).unwrap());
        }
    }

    fn game_processes_named_with_threads(
        name: &str,
    ) -> Result<Vec<(GameProcess, Vec<Thread>)>, String> {
        let mut found = Vec::new();
        visit_named_processes(name, |game, threads| {
            found.push((game, threads.to_vec()));
        })?;
        Ok(found)
    }

    struct TestChild {
        child: Option<std::process::Child>,
        exited: bool,
        copy: PathBuf,
    }

    impl TestChild {
        fn child_mut(&mut self) -> &mut std::process::Child {
            self.child.as_mut().expect("child handle retained")
        }
    }

    impl Drop for TestChild {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                if !self.exited {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                drop(child);
            }
            let _ = std::fs::remove_file(&self.copy);
        }
    }

    #[test]
    fn a_live_process_is_running_until_it_ends() {
        use crate::games::ProcessState;
        use std::os::windows::io::AsRawHandle;

        let unique = format!(
            "battleplan-state-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let image = format!("{unique}.exe");
        let copy = std::env::temp_dir().join(&image);
        std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
        let ready_name = format!("Local\\{unique}.ready");
        let release_name = format!("Local\\{unique}.release");
        let ready = Event::new(&ready_name).unwrap();
        let release = Event::new(&release_name).unwrap();
        let child = std::process::Command::new(&copy)
            .args([
                "--exact",
                "windows::tests::a_held_child_waits_for_its_release",
                "--test-threads=1",
            ])
            .env(READY_CHILD, &ready_name)
            .env(HELD_CHILD, &release_name)
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut guard = TestChild {
            child: Some(child),
            exited: false,
            copy,
        };

        let handles = [ready.handle.0, guard.child_mut().as_raw_handle() as HANDLE];
        // SAFETY: Both handles are live kernel synchronization handles owned by this test.
        let waited = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        assert_eq!(
            waited,
            WAIT_OBJECT_0,
            "held child pid {pid} did not signal ready; wait={waited}, status={:?}",
            guard.child_mut().try_wait()
        );

        let observe = || -> Vec<(GameProcess, Vec<Thread>)> {
            game_processes_named_with_threads(&image)
                .unwrap()
                .into_iter()
                .filter(|(game, _)| game.pid == pid)
                .collect()
        };

        let running = observe();
        let running_states: Vec<ProcessState> =
            running.iter().map(|(game, _)| game.state).collect();
        assert_eq!(
            running_states,
            [ProcessState::Running],
            "expected Running for pid {pid}; observed={running:?}"
        );
        let public_running: Vec<ProcessState> = game_processes_named(&image)
            .unwrap()
            .into_iter()
            .filter(|game| game.pid == pid)
            .map(|game| game.state)
            .collect();
        assert_eq!(public_running, [ProcessState::Running]);

        release.set().unwrap();
        let status = guard.child_mut().wait().unwrap();
        guard.exited = true;
        assert!(status.success());

        let ended = observe();
        let ended_states: Vec<ProcessState> = ended.iter().map(|(game, _)| game.state).collect();
        assert!(
            matches!(ended_states.as_slice(), [] | [ProcessState::Exiting]),
            "expected Exiting or absent after exit for pid {pid}; observed={ended:?}"
        );
        drop(guard);
    }

    #[test]
    fn slow_bad_length_observation_cannot_restart_the_snapshot_budget() {
        let started = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let error = module_snapshot::<()>(
            started + MODULE_SNAPSHOT_WAIT,
            || {
                assert_eq!(elapsed.get(), Duration::ZERO);
                elapsed.set(MODULE_SNAPSHOT_WAIT);
                Err(io::Error::from_raw_os_error(ERROR_BAD_LENGTH as i32))
            },
            || started + elapsed.get(),
            |_| panic!("an exhausted observation must not wait or retry"),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }
}
