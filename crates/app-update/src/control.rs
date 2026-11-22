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
const CLOSING: char = 'Y';
const REFUSED: char = 'N';
const FREE: u8 = b'I';
const BUSY: u8 = b'B';

fn close_reply(closing: Result<(), String>) -> Vec<u8> {
    let reply = match closing {
        Ok(()) => CLOSING.to_string(),
        Err(reason) => format!("{REFUSED}{reason}"),
    };
    reply.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
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
    let name = pipe_name(role)?;
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
                        CLOSE => {
                            let answered = answer.close_for_update();
                            closing = answered.is_ok();
                            close_reply(answered)
                        }
                        QUERY => match answer.busy() {
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
