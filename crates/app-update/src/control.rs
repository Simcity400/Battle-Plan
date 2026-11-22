use crate::Role;
use std::{ffi::c_void, io, sync::Arc};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile},
    System::{
        Pipes::{
            CallNamedPipeW, ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
            PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
            PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::GetCurrentProcessId,
    },
};

pub const CLOSE: u8 = b'C';
pub const QUERY: u8 = b'Q';
const CLOSING: u8 = b'Y';
const REFUSED: u8 = b'N';
const FREE: u8 = b'I';
const BUSY: u8 = b'B';

fn close_reply(closing: Result<(), String>) -> (Vec<u8>, Option<Vec<u8>>) {
    match closing {
        Ok(()) => (vec![CLOSING], None),
        Err(reason) => (
            vec![REFUSED],
            Some(reason.encode_utf16().flat_map(u16::to_le_bytes).collect()),
        ),
    }
}

fn write(pipe: *mut c_void, message: &[u8]) {
    let mut written = 0_u32;
    unsafe {
        WriteFile(
            pipe,
            message.as_ptr(),
            message.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
    }
}

pub trait Answer: Send + Sync + 'static {
    fn close_for_update(&self) -> Result<(), String>;
    fn closed_for_update(&self) {}
    fn busy(&self) -> Option<String>;
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

pub fn serve(role: Role, answer: Arc<dyn Answer>) -> io::Result<()> {
    serve_on(&pipe_name(role)?, answer)
}

fn serve_on(name: &[u16], answer: Arc<dyn Answer>) -> io::Result<()> {
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            512,
            16,
            0,
            std::ptr::null(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let raw = pipe as usize;
    std::thread::Builder::new()
        .name("update control".into())
        .spawn(move || {
            let pipe = raw as *mut c_void;
            loop {
                let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0
                    || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                if !connected {
                    eprintln!(
                        "The update control pipe failed: {}",
                        io::Error::last_os_error()
                    );
                    unsafe { CloseHandle(pipe) };
                    return;
                }
                let mut request = [0_u8; 16];
                let mut read = 0_u32;
                let mut closing = false;
                let (reply, reason) = if unsafe {
                    ReadFile(
                        pipe,
                        request.as_mut_ptr(),
                        request.len() as u32,
                        &mut read,
                        std::ptr::null_mut(),
                    )
                } != 0
                    && read == 1
                {
                    match request[0] {
                        CLOSE => {
                            let answered = answer.close_for_update();
                            closing = answered.is_ok();
                            close_reply(answered)
                        }
                        QUERY => match answer.busy() {
                            None => (vec![FREE], None),
                            Some(reason) => ([&[BUSY], reason.as_bytes()].concat(), None),
                        },
                        _ => (Vec::new(), None),
                    }
                } else {
                    (Vec::new(), None)
                };
                if !reply.is_empty() {
                    write(pipe, &reply);
                    if let Some(reason) = &reason {
                        write(pipe, reason);
                    }
                    unsafe { FlushFileBuffers(pipe) };
                    if closing {
                        answer.closed_for_update();
                    }
                }
                unsafe { DisconnectNamedPipe(pipe) };
            }
        })
        .map(|_| ())
        .map_err(|error| {
            unsafe { CloseHandle(pipe) };
            error
        })
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
    use super::{Answer, CLOSE, serve_on};
    use std::{ffi::c_void, sync::Arc};
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

    impl Answer for App {
        fn close_for_update(&self) -> Result<(), String> {
            self.0.clone()
        }

        fn busy(&self) -> Option<String> {
            None
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
        serve_on(&name, Arc::new(App(Err(REFUSAL.into())))).unwrap();
        assert_eq!(older_setup(&name), Some(b'N'), "an older setup’s four-byte read takes the refusal");
        assert_eq!(setup(&name), (b'N', Some(REFUSAL.to_owned())), "this setup reads the reason after it");
        let name = pipe("closing");
        serve_on(&name, Arc::new(App(Ok(())))).unwrap();
        assert_eq!(older_setup(&name), Some(b'Y'));
        assert_eq!(setup(&name), (b'Y', None));
    }

    #[test]
    fn this_setup_reads_an_older_apps_one_byte_refusal_and_names_its_own_reason() {
        let name = pipe("older-app");
        older_app(&name, b'N');
        assert_eq!(setup(&name), (b'N', None), "the older app sends no reason, and setup isn’t left waiting for one");
    }
}
