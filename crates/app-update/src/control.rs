//! The pipe each app answers on, so the other app and the installer can ask whether it may be
//! replaced now and ask it to close for an update. `installer-hooks.nsh` speaks the same bytes.
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

/// Close for an update: answered `CLOSING` (and the app closes), `PREPARER` (a headless preparer
/// closes, which the installer starts again) or `REFUSED`.
pub const CLOSE: u8 = b'C';
/// May this app be replaced now: answered `FREE`, or `BUSY` followed by the reason in UTF-8.
pub const QUERY: u8 = b'Q';
const CLOSING: u8 = b'Y';
const PREPARER: u8 = b'P';
const REFUSED: u8 = b'N';
const FREE: u8 = b'I';
const BUSY: u8 = b'B';

/// What a request asks the app that answers it.
pub trait Answer: Send + Sync + 'static {
    /// Whether this app will close for an update.
    fn close_for_update(&self) -> Result<(), String>;
    /// Close, once the asker has read the answer: an app gone before it answered would leave
    /// the installer unsure what to start again.
    fn closed_for_update(&self) {}
    fn install_blocked(&self) -> Option<String>;
    /// A headless preparer, which the installer starts again after it.
    fn preparer(&self) -> bool {
        false
    }
}

/// `\\.\pipe\HOI4Replay.<session>.<role>`: one name per Windows session, as the installer builds it.
fn pipe_name(role: Role) -> io::Result<Vec<u16>> {
    let mut session = 0_u32;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(r"\\.\pipe\HOI4Replay.{session}.{}", role.pipe())
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

/// Answer requests on this app's pipe for as long as it runs.
pub fn serve(role: Role, answer: Arc<dyn Answer>) -> io::Result<()> {
    let name = pipe_name(role)?;
    // Unlimited instances: two copies of Battle Plan each answer, so each can be closed.
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
    // A handle is an address; the thread takes it as a number.
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
                let reply = if unsafe {
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
                        CLOSE => vec![match answer.close_for_update() {
                            Ok(()) => {
                                closing = true;
                                if answer.preparer() { PREPARER } else { CLOSING }
                            }
                            Err(_) => REFUSED,
                        }],
                        QUERY => match answer.install_blocked() {
                            None => vec![FREE],
                            Some(reason) => [&[BUSY], reason.as_bytes()].concat(),
                        },
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                if !reply.is_empty() {
                    let mut written = 0_u32;
                    unsafe {
                        WriteFile(
                            pipe,
                            reply.as_ptr(),
                            reply.len() as u32,
                            &mut written,
                            std::ptr::null_mut(),
                        );
                        // Returns once the asker has read the reply.
                        FlushFileBuffers(pipe);
                    }
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

/// Why `role` cannot be replaced now; `None` when it is free or not running.
pub fn install_blocked(role: Role) -> Option<String> {
    let name = match pipe_name(role) {
        Ok(name) => name,
        Err(error) => {
            return Some(format!(
                "{} could not be asked whether it is busy: {error}",
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
        // No pipe: the app is not running.
        if error == ERROR_FILE_NOT_FOUND {
            return None;
        }
        return Some(format!(
            "{} did not answer whether it is busy: {}",
            role.app(),
            io::Error::from_raw_os_error(error as i32)
        ));
    }
    match reply[..read as usize].split_first() {
        Some((&FREE, _)) => None,
        Some((&BUSY, reason)) => Some(String::from_utf8_lossy(reason).into_owned()),
        _ => Some(format!(
            "{} gave an answer this version does not understand.",
            role.app()
        )),
    }
}
