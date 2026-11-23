use crate::{Host, Role};
use std::{
    io,
    sync::{Arc, Condvar, Mutex, OnceLock, PoisonError},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_CONNECTED,
        ERROR_PIPE_NOT_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    },
    Storage::FileSystem::{FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile},
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            CallNamedPipeW, ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
            PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
            PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{CreateEventW, GetCurrentProcessId, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects},
    },
};

pub const CLOSE: u8 = b'C';
pub const QUERY: u8 = b'Q';
const CLOSING: u8 = b'Y';
const REFUSED: u8 = b'N';
const FREE: u8 = b'I';
const BUSY: u8 = b'B';
const WRITE_LIMIT: u32 = 2_000;
const READ_BACK_LIMIT: u32 = 5_000;

fn close_reply(closing: Result<(), String>) -> (Vec<u8>, Option<Vec<u8>>) {
    match closing {
        Ok(()) => (vec![CLOSING], None),
        Err(reason) => (
            vec![REFUSED],
            Some(reason.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        ),
    }
}

fn pipe_name(role: Role) -> io::Result<Vec<u16>> {
    let mut session = 0_u32;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(r"\\.\pipe\BattlePlan.{session}.{}", role.pipe())
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

#[derive(Default)]
struct Serving {
    stopping: bool,
    finished: bool,
}

pub(crate) struct Server {
    stop: HANDLE,
    serving: Mutex<Serving>,
    finished: Condvar,
}

// SAFETY: the stop event is a kernel handle, usable from any thread, and lives as long as the
// server (it is never closed).
unsafe impl Send for Server {}
// SAFETY: as above; every other field is synchronised.
unsafe impl Sync for Server {}

static SERVED: OnceLock<Arc<Server>> = OnceLock::new();

impl Server {
    fn lock(&self) -> std::sync::MutexGuard<'_, Serving> {
        self.serving.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn stop(&self) {
        self.lock().stopping = true;
        // SAFETY: the event lives as long as the server.
        unsafe { SetEvent(self.stop) };
        let serving = self.lock();
        drop(self.finished.wait_while(serving, |serving| !serving.finished).unwrap_or_else(PoisonError::into_inner));
    }

    fn accept(&self) -> bool {
        !self.lock().stopping
    }

    fn finish(&self) {
        self.lock().finished = true;
        self.finished.notify_all();
    }
}

pub fn stop() {
    if let Some(server) = SERVED.get() {
        server.stop();
    }
}

enum Ended {
    Stopped,
    TimedOut,
    Failed(u32),
}

struct Pipe {
    handle: HANDLE,
    io: HANDLE,
}

impl Pipe {
    fn wait(&self, overlapped: &mut OVERLAPPED, started: bool, stop: Option<HANDLE>, limit: u32) -> Result<u32, Ended> {
        let mut moved = 0_u32;
        // SAFETY: `overlapped` and the buffer it was started with outlive this call, which returns
        // only once the operation has completed, by success, failure or an observed cancellation.
        unsafe {
            if started {
                return if GetOverlappedResult(self.handle, overlapped, &mut moved, 0) != 0 { Ok(moved) } else { Err(Ended::Failed(GetLastError())) };
            }
            let error = GetLastError();
            if error == ERROR_PIPE_CONNECTED {
                return Ok(0);
            }
            if error != ERROR_IO_PENDING {
                return Err(Ended::Failed(error));
            }
            let waits = [self.io, stop.unwrap_or(self.io)];
            let count = if stop.is_some() { 2 } else { 1 };
            let waited = WaitForMultipleObjects(count, waits.as_ptr(), 0, limit);
            if waited == WAIT_OBJECT_0 {
                return if GetOverlappedResult(self.handle, overlapped, &mut moved, 0) != 0 { Ok(moved) } else { Err(Ended::Failed(GetLastError())) };
            }
            CancelIoEx(self.handle, overlapped);
            GetOverlappedResult(self.handle, overlapped, &mut moved, 1);
            Err(if count == 2 && waited == WAIT_OBJECT_0 + 1 { Ended::Stopped } else { Ended::TimedOut })
        }
    }

    fn fresh(&self) -> OVERLAPPED {
        // SAFETY: a zeroed OVERLAPPED is valid, and the event is ours to reset.
        unsafe {
            ResetEvent(self.io);
            OVERLAPPED { hEvent: self.io, ..std::mem::zeroed() }
        }
    }

    fn connect(&self, stop: HANDLE) -> Result<(), Ended> {
        let mut overlapped = self.fresh();
        // SAFETY: the overlapped structure outlives the wait that completes it.
        let started = unsafe { ConnectNamedPipe(self.handle, &mut overlapped) } != 0;
        self.wait(&mut overlapped, started, Some(stop), INFINITE).map(drop)
    }

    fn read(&self, buffer: &mut [u8], stop: Option<HANDLE>, limit: u32) -> Result<u32, Ended> {
        let mut overlapped = self.fresh();
        // SAFETY: the buffer and the overlapped structure outlive the wait that completes them.
        let started = unsafe { ReadFile(self.handle, buffer.as_mut_ptr(), buffer.len() as u32, std::ptr::null_mut(), &mut overlapped) } != 0;
        self.wait(&mut overlapped, started, stop, limit)
    }

    fn write(&self, message: &[u8]) -> Result<u32, Ended> {
        let mut overlapped = self.fresh();
        // SAFETY: the message and the overlapped structure outlive the wait that completes them.
        let started = unsafe { WriteFile(self.handle, message.as_ptr(), message.len() as u32, std::ptr::null_mut(), &mut overlapped) } != 0;
        self.wait(&mut overlapped, started, None, WRITE_LIMIT)
    }

    fn answer(&self, reply: &[u8], reason: Option<&[u8]>) {
        let written = self.write(reply).and_then(|_| reason.map_or(Ok(0), |reason| self.write(reason)));
        let delivered = written.and_then(|_| match self.read(&mut [0_u8; 1], None, READ_BACK_LIMIT) {
            Err(Ended::Failed(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED)) | Ok(_) => Ok(0),
            other => other,
        });
        match delivered {
            Ok(_) => {}
            Err(Ended::Failed(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED)) => {
                eprintln!("The app control pipe’s client left before its answer was read, so whether it heard it is unknown.");
            }
            Err(Ended::TimedOut | Ended::Stopped) => {
                eprintln!("The app control pipe’s client didn’t read its answer in time, so whether it heard it is unknown.");
            }
            Err(Ended::Failed(error)) => {
                eprintln!("The app control pipe’s answer failed, so whether its client heard it is unknown: {}", io::Error::from_raw_os_error(error as i32));
            }
        }
    }
}

pub(crate) fn serve(role: Role, host: Arc<dyn Host>) -> io::Result<()> {
    serve_on(&pipe_name(role)?, host, |server| drop(SERVED.set(Arc::clone(server)))).map(drop)
}

fn serve_on(name: &[u16], host: Arc<dyn Host>, publish: impl FnOnce(&Arc<Server>)) -> io::Result<Arc<Server>> {
    // SAFETY: plain handle creation; every handle is closed by the server thread, or here if it
    // never starts.
    let (handle, io, stop) = unsafe {
        let handle = CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            512,
            16,
            0,
            std::ptr::null(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        (handle, CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()), CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()))
    };
    if io.is_null() || stop.is_null() {
        let error = io::Error::last_os_error();
        // SAFETY: closing the handles made above.
        unsafe {
            CloseHandle(handle);
            CloseHandle(io);
        }
        return Err(error);
    }
    let server = Arc::new(Server { stop, serving: Mutex::default(), finished: Condvar::new() });
    publish(&server);
    let serving = Arc::clone(&server);
    let (raw, event) = (handle as usize, io as usize);
    std::thread::Builder::new()
        .name("update control".into())
        .spawn(move || {
            let pipe = Pipe { handle: raw as HANDLE, io: event as HANDLE };
            serve_until_stopped(&pipe, &serving, &*host);
            // SAFETY: no I/O is outstanding on either handle once the loop has returned.
            unsafe {
                CloseHandle(pipe.handle);
                CloseHandle(pipe.io);
            }
            serving.finish();
        })
        .map_err(|error| {
            // SAFETY: the thread never started, so nothing else holds the handles.
            unsafe {
                CloseHandle(handle);
                CloseHandle(io);
            }
            server.finish();
            error
        })?;
    Ok(server)
}

fn serve_until_stopped(pipe: &Pipe, server: &Server, host: &dyn Host) {
    loop {
        match pipe.connect(server.stop) {
            Ok(()) => {}
            Err(Ended::Stopped) => return,
            Err(Ended::TimedOut) => continue,
            Err(Ended::Failed(ERROR_NO_DATA)) => {
                // SAFETY: no I/O is outstanding on the handle here.
                unsafe { DisconnectNamedPipe(pipe.handle) };
                continue;
            }
            Err(Ended::Failed(error)) => {
                eprintln!("The update control pipe failed: {}", io::Error::from_raw_os_error(error as i32));
                return;
            }
        }
        let mut request = [0_u8; 16];
        let read = pipe.read(&mut request, Some(server.stop), INFINITE);
        let stopped = matches!(read, Err(Ended::Stopped));
        if let Ok(1) = read
            && server.accept()
        {
            let (reply, reason) = match request[0] {
                CLOSE => close_reply(host.close_for_update()),
                QUERY => match host.busy() {
                    None => (vec![FREE], None),
                    Some(reason) => ([&[BUSY], reason.as_bytes()].concat(), None),
                },
                _ => (Vec::new(), None),
            };
            if !reply.is_empty() {
                pipe.answer(&reply, reason.as_deref());
            }
        }
        // SAFETY: no I/O is outstanding on the handle here.
        unsafe { DisconnectNamedPipe(pipe.handle) };
        if stopped || !server.accept() {
            return;
        }
    }
}
pub(crate) fn install_blocked(role: Role) -> Option<String> {
    let name = match pipe_name(role) {
        Ok(name) => name,
        Err(error) => {
            return Some(format!(
                "{} isn’t responding, so the update can’t start yet. Try again in a moment: {error}",
                role.app()
            ));
        }
    };
    let mut reply = [0_u8; 512];
    let mut read = 0_u32;
    let request = QUERY;
    let answered = unsafe {
        CallNamedPipeW(
            name.as_ptr(),
            (&request as *const u8).cast(),
            1,
            reply.as_mut_ptr().cast(),
            reply.len() as u32,
            &mut read,
            2000,
        )
    } != 0;
    if !answered {
        let error = unsafe { GetLastError() };
        if error == ERROR_FILE_NOT_FOUND {
            return None;
        }
        return Some(format!(
            "{} isn’t responding, so the update can’t start yet. Try again in a moment: {}",
            role.app(),
            io::Error::from_raw_os_error(error as i32)
        ));
    }
    match reply[..read as usize].split_first() {
        Some((&FREE, _)) => None,
        Some((&BUSY, reason)) => Some(String::from_utf8_lossy(reason).into_owned()),
        _ => Some(format!(
            "{} gave an answer this version doesn’t understand.",
            role.app()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{CLOSE, serve_on};
    use std::{ffi::c_void, sync::{Arc, Mutex, mpsc}};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
        },
        System::Pipes::{
            CallNamedPipeW, ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
            NMPWAIT_WAIT_FOREVER, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE, PIPE_WAIT,
            SetNamedPipeHandleState, TransactNamedPipe, WaitNamedPipeW,
        },
    };

    const REFUSAL: &str = "BattlePlan has unsaved changes. Save or discard them in BattlePlan, then restart to update.";

    struct App(Result<(), String>);

    impl crate::Host for App {
        fn install(&self, _: crate::Launch) -> Result<(), String> {
            unreachable!("no install in these tests")
        }

        fn close_for_update(&self) -> Result<(), String> {
            self.0.clone()
        }

        fn write_preference(&self, _: &std::path::Path, _: &[u8]) -> std::io::Result<Option<String>> {
            unreachable!("no preference in these tests")
        }
    }

    struct Paused {
        asked: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl crate::Host for Paused {
        fn install(&self, _: crate::Launch) -> Result<(), String> {
            unreachable!("no install in these tests")
        }

        fn close_for_update(&self) -> Result<(), String> {
            self.asked.lock().unwrap().send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(())
        }

        fn write_preference(&self, _: &std::path::Path, _: &[u8]) -> std::io::Result<Option<String>> {
            unreachable!("no preference in these tests")
        }
    }

    fn pipe(test: &str) -> Vec<u16> {
        format!(r"\\.\pipe\BattlePlan.test.{}.{test}", std::process::id())
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }

    fn older_setup(name: &[u16]) -> Option<u8> {
        let mut reply = [0_u8; 4];
        let mut read = 0_u32;
        let request = CLOSE;
        let answered = unsafe {
            CallNamedPipeW(
                name.as_ptr(),
                (&request as *const u8).cast(),
                1,
                reply.as_mut_ptr().cast(),
                4,
                &mut read,
                NMPWAIT_WAIT_FOREVER,
            )
        } != 0;
        (answered && read == 1).then_some(reply[0])
    }

    fn setup(name: &[u16]) -> (u8, Option<String>) {
        unsafe { WaitNamedPipeW(name.as_ptr(), NMPWAIT_WAIT_FOREVER) };
        let pipe = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(pipe, INVALID_HANDLE_VALUE, "setup reaches the app’s pipe");
        let mode = PIPE_READMODE_MESSAGE;
        unsafe { SetNamedPipeHandleState(pipe, &mode, std::ptr::null(), std::ptr::null()) };
        let request = CLOSE;
        let mut status = [0_u8; 4];
        let mut read = 0_u32;
        let answered = unsafe {
            TransactNamedPipe(
                pipe,
                (&request as *const u8).cast::<c_void>(),
                1,
                status.as_mut_ptr().cast(),
                4,
                &mut read,
                std::ptr::null_mut(),
            )
        } != 0;
        assert!(answered && read == 1, "the first answer is one byte");
        let mut reason = [0_u16; 1023];
        let mut read = 0_u32;
        let explained = status[0] == b'N'
            && unsafe {
                ReadFile(
                    pipe,
                    reason.as_mut_ptr().cast(),
                    2046,
                    &mut read,
                    std::ptr::null_mut(),
                )
            } != 0;
        unsafe { CloseHandle(pipe) };
        (status[0], explained.then(|| String::from_utf16_lossy(&reason[..read as usize / 2])))
    }

    fn older_app(name: &[u16], answer: u8) {
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                1,
                512,
                16,
                0,
                std::ptr::null(),
            )
        };
        assert_ne!(pipe, INVALID_HANDLE_VALUE);
        let pipe = pipe as usize;
        std::thread::spawn(move || {
            let pipe = pipe as *mut c_void;
            let mut request = [0_u8; 16];
            let (mut read, mut written) = (0_u32, 0_u32);
            unsafe {
                ConnectNamedPipe(pipe, std::ptr::null_mut());
                ReadFile(pipe, request.as_mut_ptr(), 16, &mut read, std::ptr::null_mut());
                WriteFile(pipe, &answer, 1, &mut written, std::ptr::null_mut());
                FlushFileBuffers(pipe);
                DisconnectNamedPipe(pipe);
                CloseHandle(pipe);
            }
        });
    }

    #[test]
    fn an_older_setup_reads_the_one_byte_answer_and_this_setup_also_reads_the_refusal() {
        let name = pipe("refusing");
        serve_on(&name, Arc::new(App(Err(REFUSAL.into()))), |_| {}).unwrap();
        assert_eq!(older_setup(&name), Some(b'N'), "an older setup’s four-byte read takes the refusal");
        assert_eq!(setup(&name), (b'N', Some(REFUSAL.to_owned())), "this setup reads the reason after it");
        let name = pipe("closing");
        serve_on(&name, Arc::new(App(Ok(()))), |_| {}).unwrap();
        assert_eq!(older_setup(&name), Some(b'Y'));
        assert_eq!(setup(&name), (b'Y', None));
    }

    #[test]
    fn an_idle_apps_first_close_is_answered_even_when_stopping_begins_before_the_reply_is_written() {
        let name = pipe("stopping");
        let (asked, decided) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let server = serve_on(&name, Arc::new(Paused { asked: Mutex::new(asked), release: Mutex::new(released) }), |_| {}).unwrap();
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| setup(&name));
            decided.recv().unwrap();
            let stopping = scope.spawn(|| server.stop());
            while server.accept() {
                std::thread::yield_now();
            }
            assert!(!stopping.is_finished(), "stop waits for the transaction it can no longer refuse");
            release.send(()).unwrap();
            assert_eq!(reader.join().unwrap(), (b'Y', None), "setup reads the Y its close was owed");
            stopping.join().unwrap();
        });
        assert_eq!(older_setup(&name), None, "nothing is accepted once the pipe has stopped");
    }

    #[test]
    fn an_idle_pipe_stops_at_once_and_a_client_that_never_reads_doesnt_hold_it() {
        let name = pipe("idle");
        let server = serve_on(&name, Arc::new(App(Ok(()))), |_| {}).unwrap();
        server.stop();
        assert_eq!(older_setup(&name), None);
    }

    #[test]
    fn this_setup_reads_an_older_apps_one_byte_refusal_and_names_its_own_reason() {
        let name = pipe("older-app");
        older_app(&name, b'N');
        assert_eq!(setup(&name), (b'N', None), "the older app sends no reason, and setup isn’t left waiting for one");
    }
}
