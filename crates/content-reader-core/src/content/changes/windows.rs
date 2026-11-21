use super::State;
use std::{
    cell::UnsafeCell,
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, atomic::Ordering},
    thread::JoinHandle,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_IO_PENDING, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
        FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_CREATION, FILE_NOTIFY_CHANGE_DIR_NAME,
        FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SECURITY,
        FILE_NOTIFY_CHANGE_SIZE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, ReadDirectoryChangesW,
    },
    System::IO::{
        CancelIoEx, CreateIoCompletionPort, GetQueuedCompletionStatus,
        OVERLAPPED, PostQueuedCompletionStatus,
    },
};

struct Port(usize);

impl Drop for Port {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0 as _);
        }
    }
}

struct Request {
    file: File,
    buffer: Box<UnsafeCell<[u32; 16 * 1024]>>,
    overlapped: Box<UnsafeCell<OVERLAPPED>>,
    pending: bool,
}

unsafe impl Send for Request {}

impl Request {
    fn arm(&mut self, subtree: bool) -> io::Result<()> {
        unsafe {
            *self.overlapped.get() = std::mem::zeroed();
        }
        let ok = unsafe {
            ReadDirectoryChangesW(
                self.file.as_raw_handle(),
                self.buffer.get().cast(),
                std::mem::size_of::<[u32; 16 * 1024]>() as u32,
                i32::from(subtree),
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_ATTRIBUTES
                    | FILE_NOTIFY_CHANGE_SIZE
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SECURITY
                    | FILE_NOTIFY_CHANGE_CREATION,
                std::ptr::null_mut(),
                self.overlapped.get(),
                None,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
        }
        self.pending = true;
        Ok(())
    }

}

struct Subscription {
    request: Mutex<Request>,
    completed: Condvar,
    subtree: bool,
}

pub(super) struct Owner {
    port: Arc<Port>,
    state: Arc<State>,
    roots: BTreeMap<(PathBuf, bool), Box<Subscription>>,
    thread: Option<JoinHandle<()>>,
}

impl Owner {
    pub(super) fn new(state: Arc<State>) -> io::Result<Self> {
        let handle =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, std::ptr::null_mut(), 0, 1) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let port = Arc::new(Port(handle as usize));
        let worker_port = Arc::clone(&port);
        let worker_state = Arc::clone(&state);
        let thread = std::thread::Builder::new()
            .name("content changes".into())
            .spawn(move || {
                run(&worker_port, &worker_state);
            })?;
        Ok(Self {
            port,
            state,
            roots: BTreeMap::new(),
            thread: Some(thread),
        })
    }

    pub(super) fn watch(&mut self, path: &Path, subtree: bool) -> io::Result<()> {
        if self.roots.keys().any(|(root, recursive)| {
            (*recursive && path.starts_with(root)) || (root == path && (!subtree || *recursive))
        }) {
            return Ok(());
        }
        self.state.changed();
        let file = OpenOptions::new()
            .read(true)
            .access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED)
            .open(path)?;
        let subscription = Box::new(Subscription {
            request: Mutex::new(Request {
                file,
                buffer: Box::new(UnsafeCell::new([0; 16 * 1024])),
                overlapped: Box::new(UnsafeCell::new(unsafe { std::mem::zeroed() })),
                pending: false,
            }),
            completed: Condvar::new(),
            subtree,
        });
        let key = (&*subscription as *const Subscription) as usize;
        let associated = unsafe {
            CreateIoCompletionPort(
                subscription
                    .request
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .file
                    .as_raw_handle(),
                self.port.0 as _,
                key,
                0,
            )
        };
        if associated.is_null() {
            return Err(io::Error::last_os_error());
        }
        let subscription = self
            .roots
            .entry((path.to_owned(), subtree))
            .or_insert(subscription);
        subscription
            .request
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .arm(subtree)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.state.stopping.store(true, Ordering::Release);
        for subscription in self.roots.values() {
            let request = subscription.request.lock().unwrap_or_else(|error| error.into_inner());
            if request.pending {
                unsafe { CancelIoEx(request.file.as_raw_handle(), request.overlapped.get()); }
            }
        }
        for subscription in self.roots.values() {
            let mut request = subscription.request.lock().unwrap_or_else(|error| error.into_inner());
            while request.pending {
                request = subscription.completed.wait(request).unwrap_or_else(|error| error.into_inner());
            }
        }
        unsafe {
            PostQueuedCompletionStatus(self.port.0 as _, 0, 0, std::ptr::null());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(port: &Port, state: &State) {
    loop {
        let mut bytes = 0;
        let mut key = 0;
        let mut overlapped = std::ptr::null_mut();
        let ok = unsafe {
            GetQueuedCompletionStatus(port.0 as _, &mut bytes, &mut key, &mut overlapped, u32::MAX)
        };
        if key == 0 {
            if !state.stopping.load(Ordering::Acquire) {
                state.failed();
            }
            return;
        }
        let subscription = unsafe { &*(key as *const Subscription) };
        let mut request = subscription
            .request
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        request.pending = false;
        subscription.completed.notify_all();
        if state.stopping.load(Ordering::Acquire) {
            continue;
        }
        if ok == 0 || bytes == 0 {
            state.failed();
            continue;
        }
        state.changed();
        if request.arm(subscription.subtree).is_err() {
            state.failed();
        }
    }
}
