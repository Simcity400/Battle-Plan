//! Offline Windows H4CT listener and overlapped-I/O foundation.
//!
//! Nothing in this module injects a DLL or activates auto-attach. The type-state
//! API exists so future activation can require proof that a restrictive,
//! overlapped duplex listener was created before the one permitted injection.

use std::fmt;
use std::io;

use crate::control_transport::{
    claim_session_capability, ControlCancellation, ControlCapabilities, ControlDeadline,
    ControlError, ControlPeerIdentity, DeadlineControlChannel, DeadlineControlIo, EndpointRole,
    ValidatedSessionCapability,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestrictiveSecurityPolicy {
    /// LocalSystem remains off unless packaging later proves it is required.
    pub allow_local_system: bool,
}

impl Default for RestrictiveSecurityPolicy {
    fn default() -> Self {
        Self {
            allow_local_system: false,
        }
    }
}

#[derive(Debug)]
pub enum ControlPipeError {
    Io(io::ErrorKind),
    DeadlineExpired,
    Cancelled,
    InvalidPipeName,
    Peer(ControlError),
}

impl fmt::Display for ControlPipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "control pipe I/O failed: {kind:?}"),
            Self::DeadlineExpired => write!(f, "control pipe deadline expired"),
            Self::Cancelled => write!(f, "control pipe operation cancelled"),
            Self::InvalidPipeName => write!(f, "control pipe name is not a local H4CT pipe"),
            Self::Peer(error) => write!(f, "control pipe peer admission failed: {error}"),
        }
    }
}

impl std::error::Error for ControlPipeError {}

impl From<io::Error> for ControlPipeError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut => Self::DeadlineExpired,
            io::ErrorKind::Interrupted => Self::Cancelled,
            kind => Self::Io(kind),
        }
    }
}

/// Mockable Windows boundary. Production uses [`WindowsPipeBackend`]; tests can
/// inject listener, connect, identity, deadline, and cancellation faults.
pub trait ControlPipeBackend {
    type Listener;
    type Io: DeadlineControlIo;

    fn create_restrictive_listener(
        &mut self,
        pipe_name: &str,
        security: RestrictiveSecurityPolicy,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::Listener, ControlPipeError>;

    fn accept(
        &mut self,
        listener: Self::Listener,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::Io, ControlPipeError>;

    /// Must be backed by `GetNamedPipeClientProcessId` and a fresh process
    /// creation-FILETIME query in the production implementation.
    fn peer_identity(
        &mut self,
        io: &Self::Io,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<ControlPeerIdentity, ControlPipeError>;
}

/// Unforgeable outside this module. Future injection wiring should require a
/// borrow of this proof before invoking the existing one-shot injector.
pub struct ListenerReadyProof {
    _private: (),
}

/// Type-state representing a secured H4CT listener created before injection.
pub struct PreparedControlListener<B: ControlPipeBackend> {
    backend: B,
    listener: B::Listener,
    ready: ListenerReadyProof,
}

pub fn prepare_control_listener<B: ControlPipeBackend>(
    mut backend: B,
    pipe_name: &str,
    security: RestrictiveSecurityPolicy,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<PreparedControlListener<B>, ControlPipeError> {
    validate_local_pipe_name(pipe_name)?;
    preflight(deadline, cancellation)?;
    let listener =
        backend.create_restrictive_listener(pipe_name, security, deadline, cancellation)?;
    preflight(deadline, cancellation)?;
    Ok(PreparedControlListener {
        backend,
        listener,
        ready: ListenerReadyProof { _private: () },
    })
}

impl<B: ControlPipeBackend> PreparedControlListener<B> {
    pub fn listener_ready_proof(&self) -> &ListenerReadyProof {
        &self.ready
    }

    /// Accept one peer and return the deadline-bearing raw transport only after
    /// the restrictive listener has re-read and matched its PID + process
    /// creation FILETIME. This narrow seam is for fixed, feature-local typed
    /// handshakes (currently direct-capture READY/ACK); it does not admit the
    /// general H4CT message dispatcher or bypass capability admission.
    pub fn accept_and_identify(
        mut self,
        expected_peer: ControlPeerIdentity,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<B::Io, ControlPipeError> {
        preflight(deadline, cancellation)?;
        let io = self.backend.accept(self.listener, deadline, cancellation)?;
        preflight(deadline, cancellation)?;
        let observed_peer = self.backend.peer_identity(&io, deadline, cancellation)?;
        if expected_peer.pid == 0 || expected_peer.creation_filetime == 0 {
            return Err(ControlPipeError::Peer(ControlError::InvalidPeerIdentity));
        }
        if observed_peer.pid != expected_peer.pid {
            return Err(ControlPipeError::Peer(ControlError::PeerPidMismatch {
                expected: expected_peer.pid,
                actual: observed_peer.pid,
            }));
        }
        if observed_peer.creation_filetime != expected_peer.creation_filetime {
            return Err(ControlPipeError::Peer(
                ControlError::PeerCreationTimeMismatch {
                    expected: expected_peer.creation_filetime,
                    actual: observed_peer.creation_filetime,
                },
            ));
        }
        preflight(deadline, cancellation)?;
        Ok(io)
    }

    /// Consume the prepared listener, accept exactly one client, re-read its
    /// identity, and construct the only deadline-bearing channel form.
    pub fn accept_and_admit(
        mut self,
        session_id: [u8; 16],
        expected_peer: ControlPeerIdentity,
        required_capabilities: ControlCapabilities,
        observed_capabilities: ControlCapabilities,
        role: EndpointRole,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<DeadlineControlChannel<B::Io>, ControlPipeError> {
        preflight(deadline, cancellation)?;
        let io = self.backend.accept(self.listener, deadline, cancellation)?;
        preflight(deadline, cancellation)?;
        let observed_peer = self.backend.peer_identity(&io, deadline, cancellation)?;
        preflight(deadline, cancellation)?;
        let session = claim_session_capability(session_id).map_err(ControlPipeError::Peer)?;
        let admission = ValidatedSessionCapability::admit(
            session,
            expected_peer,
            observed_peer,
            required_capabilities,
            observed_capabilities,
        )
        .map_err(ControlPipeError::Peer)?;
        preflight(deadline, cancellation)?;
        Ok(DeadlineControlChannel::new(io, role, admission))
    }
}

fn validate_local_pipe_name(pipe_name: &str) -> Result<(), ControlPipeError> {
    let prefix = r"\\.\pipe\battle-plan_control_";
    if !pipe_name.starts_with(prefix)
        || pipe_name.len() <= prefix.len()
        || pipe_name.contains('\0')
        || pipe_name[prefix.len()..].contains(['\\', '/'])
    {
        return Err(ControlPipeError::InvalidPipeName);
    }
    Ok(())
}

fn preflight(
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<(), ControlPipeError> {
    if cancellation.is_cancelled() {
        Err(ControlPipeError::Cancelled)
    } else if deadline.is_expired() {
        Err(ControlPipeError::DeadlineExpired)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
mod win32 {
    use super::*;
    use std::ffi::c_void;
    use std::mem;
    use std::ptr;
    use std::time::Duration;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type LocalPtr = *mut c_void;

    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const PIPE_ACCESS_DUPLEX: Dword = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: Dword = 0x0008_0000;
    const FILE_FLAG_OVERLAPPED: Dword = 0x4000_0000;
    const PIPE_TYPE_BYTE: Dword = 0;
    const PIPE_READMODE_BYTE: Dword = 0;
    const PIPE_WAIT: Dword = 0;
    const PIPE_REJECT_REMOTE_CLIENTS: Dword = 0x0000_0008;
    const ERROR_IO_PENDING: Dword = 997;
    const ERROR_PIPE_CONNECTED: Dword = 535;
    const WAIT_OBJECT_0: Dword = 0;
    const WAIT_TIMEOUT: Dword = 258;
    const PROCESS_QUERY_LIMITED_INFORMATION: Dword = 0x1000;
    const SDDL_REVISION_1: Dword = 1;

    #[repr(C)]
    struct SecurityAttributes {
        length: Dword,
        security_descriptor: *mut c_void,
        inherit_handle: Bool,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Offsets {
        offset: Dword,
        offset_high: Dword,
    }

    #[repr(C)]
    union OffsetUnion {
        offsets: Offsets,
        pointer: *mut c_void,
    }

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: OffsetUnion,
        event: Handle,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: Dword,
            pipe_mode: Dword,
            max_instances: Dword,
            out_buffer_size: Dword,
            in_buffer_size: Dword,
            default_timeout: Dword,
            security: *mut SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut Overlapped) -> Bool;
        fn GetNamedPipeClientProcessId(pipe: Handle, pid: *mut Dword) -> Bool;
        fn CreateEventW(
            attributes: *mut c_void,
            manual_reset: Bool,
            initial_state: Bool,
            name: *const u16,
        ) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: Dword) -> Dword;
        fn GetOverlappedResult(
            handle: Handle,
            overlapped: *mut Overlapped,
            transferred: *mut Dword,
            wait: Bool,
        ) -> Bool;
        fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> Bool;
        fn ReadFile(
            handle: Handle,
            buffer: *mut c_void,
            len: Dword,
            transferred: *mut Dword,
            overlapped: *mut Overlapped,
        ) -> Bool;
        fn WriteFile(
            handle: Handle,
            buffer: *const c_void,
            len: Dword,
            transferred: *mut Dword,
            overlapped: *mut Overlapped,
        ) -> Bool;
        fn OpenProcess(access: Dword, inherit: Bool, pid: Dword) -> Handle;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> Bool;
        fn CloseHandle(handle: Handle) -> Bool;
        fn GetLastError() -> Dword;
        fn LocalFree(memory: LocalPtr) -> LocalPtr;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor: *const u16,
            revision: Dword,
            security_descriptor: *mut LocalPtr,
            size: *mut Dword,
        ) -> Bool;
    }

    struct OwnedHandle(Handle);

    impl OwnedHandle {
        fn new(raw: Handle) -> io::Result<Self> {
            if raw.is_null() || raw == INVALID_HANDLE_VALUE {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(raw))
            }
        }

        fn raw(&self) -> Handle {
            self.0
        }

        fn quarantine(&mut self, operation: Box<PendingOperation>) {
            self.0 = INVALID_HANDLE_VALUE;
            Box::leak(operation);
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    struct SecurityDescriptor(LocalPtr);

    impl SecurityDescriptor {
        fn restrictive(policy: RestrictiveSecurityPolicy) -> io::Result<Self> {
            // OW is the object's owner-rights SID. The protected DACL has no
            // inherited or Everyone ACE; LocalSystem is opt-in only.
            let sddl = if policy.allow_local_system {
                "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
            } else {
                "D:P(A;;GA;;;OW)"
            };
            let wide = wide(sddl);
            let mut descriptor = ptr::null_mut();
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(descriptor))
            }
        }

        fn attributes(&mut self) -> SecurityAttributes {
            SecurityAttributes {
                length: mem::size_of::<SecurityAttributes>() as Dword,
                security_descriptor: self.0,
                inherit_handle: 0,
            }
        }
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    struct PendingOperation {
        overlapped: Overlapped,
        event: OwnedHandle,
        buffer: Vec<u8>,
    }

    impl PendingOperation {
        fn new(buffer: Vec<u8>) -> io::Result<Box<Self>> {
            let event =
                OwnedHandle::new(unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) })?;
            let mut operation = Box::new(Self {
                overlapped: Overlapped {
                    internal: 0,
                    internal_high: 0,
                    offset: OffsetUnion {
                        pointer: ptr::null_mut(),
                    },
                    event: event.raw(),
                },
                event,
                buffer,
            });
            operation.overlapped.event = operation.event.raw();
            Ok(operation)
        }
    }

    pub struct WindowsControlListener {
        handle: OwnedHandle,
    }

    pub struct WindowsControlIo {
        handle: OwnedHandle,
    }

    // `WindowsControlIo` uniquely owns a kernel handle. Moving that ownership
    // to the recorder's control worker is safe: the handle is never aliased and
    // all I/O remains serialized by the owning worker.
    unsafe impl Send for WindowsControlIo {}

    #[derive(Default)]
    pub struct WindowsPipeBackend;

    impl ControlPipeBackend for WindowsPipeBackend {
        type Listener = WindowsControlListener;
        type Io = WindowsControlIo;

        fn create_restrictive_listener(
            &mut self,
            pipe_name: &str,
            security: RestrictiveSecurityPolicy,
            deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::Listener, ControlPipeError> {
            preflight(deadline, cancellation)?;
            let mut descriptor = SecurityDescriptor::restrictive(security)?;
            let mut attributes = descriptor.attributes();
            let name = wide(pipe_name);
            let handle = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    64 * 1024,
                    64 * 1024,
                    0,
                    &mut attributes,
                )
            };
            let handle = OwnedHandle::new(handle)?;
            preflight(deadline, cancellation)?;
            Ok(WindowsControlListener { handle })
        }

        fn accept(
            &mut self,
            listener: Self::Listener,
            deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::Io, ControlPipeError> {
            let WindowsControlListener { mut handle } = listener;
            let mut operation = PendingOperation::new(Vec::new())?;
            let connected = unsafe { ConnectNamedPipe(handle.raw(), &mut operation.overlapped) };
            if connected == 0 {
                let error = unsafe { GetLastError() };
                if error == ERROR_PIPE_CONNECTED {
                    return Ok(WindowsControlIo { handle });
                }
                if error != ERROR_IO_PENDING {
                    return Err(io::Error::from_raw_os_error(error as i32).into());
                }
                let _ = wait_pending(&mut handle, operation, deadline, cancellation)?;
            }
            preflight(deadline, cancellation)?;
            Ok(WindowsControlIo { handle })
        }

        fn peer_identity(
            &mut self,
            io: &Self::Io,
            deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<ControlPeerIdentity, ControlPipeError> {
            preflight(deadline, cancellation)?;
            let mut pid = 0;
            if unsafe { GetNamedPipeClientProcessId(io.handle.raw(), &mut pid) } == 0 {
                return Err(io::Error::last_os_error().into());
            }
            if pid == 0 {
                return Err(ControlPipeError::Peer(ControlError::InvalidPeerIdentity));
            }
            let process = OwnedHandle::new(unsafe {
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)
            })?;
            let mut creation = 0u64;
            let mut exit = creation;
            let mut kernel = creation;
            let mut user = creation;
            if unsafe {
                GetProcessTimes(
                    process.raw(),
                    &mut creation,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                )
            } == 0
            {
                return Err(io::Error::last_os_error().into());
            }
            preflight(deadline, cancellation)?;
            Ok(ControlPeerIdentity {
                pid,
                creation_filetime: creation,
            })
        }
    }

    impl DeadlineControlIo for WindowsControlIo {
        fn send_all_until(
            &mut self,
            mut bytes: &[u8],
            deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            while !bytes.is_empty() {
                let len = bytes.len().min(Dword::MAX as usize);
                let (transferred, _) = transfer(
                    &mut self.handle,
                    Some(&bytes[..len]),
                    0,
                    deadline,
                    cancellation,
                )?;
                if transferred == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "zero-byte H4CT write",
                    ));
                }
                bytes = &bytes[transferred..];
            }
            Ok(())
        }

        fn recv_exact_until(
            &mut self,
            mut bytes: &mut [u8],
            deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            while !bytes.is_empty() {
                let len = bytes.len().min(Dword::MAX as usize);
                let (transferred, buffer) =
                    transfer(&mut self.handle, None, len, deadline, cancellation)?;
                if transferred == 0 {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "H4CT EOF"));
                }
                bytes[..transferred].copy_from_slice(&buffer[..transferred]);
                bytes = &mut bytes[transferred..];
            }
            Ok(())
        }
    }

    fn transfer(
        handle: &mut OwnedHandle,
        write: Option<&[u8]>,
        read_len: usize,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<(usize, Vec<u8>)> {
        check_io_preflight(deadline, cancellation)?;
        let buffer = write.map_or_else(|| vec![0; read_len], |bytes| bytes.to_vec());
        let mut operation = PendingOperation::new(buffer)?;
        let mut transferred = 0;
        let submitted = unsafe {
            match write {
                Some(_) => WriteFile(
                    handle.raw(),
                    operation.buffer.as_ptr().cast(),
                    operation.buffer.len() as Dword,
                    &mut transferred,
                    &mut operation.overlapped,
                ),
                None => ReadFile(
                    handle.raw(),
                    operation.buffer.as_mut_ptr().cast(),
                    operation.buffer.len() as Dword,
                    &mut transferred,
                    &mut operation.overlapped,
                ),
            }
        };
        if submitted == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            let (completed, operation) = wait_pending(handle, operation, deadline, cancellation)?;
            return Ok((completed as usize, operation.buffer));
        }
        check_io_preflight(deadline, cancellation)?;
        Ok((transferred as usize, operation.buffer))
    }

    fn wait_pending(
        handle: &mut OwnedHandle,
        mut operation: Box<PendingOperation>,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<(Dword, Box<PendingOperation>)> {
        loop {
            if cancellation.is_cancelled() {
                return cancel_or_quarantine(handle, operation, io::ErrorKind::Interrupted);
            }
            let remaining = match deadline.remaining() {
                Some(remaining) if !remaining.is_zero() => remaining,
                _ => return cancel_or_quarantine(handle, operation, io::ErrorKind::TimedOut),
            };
            let wait_ms = duration_to_wait_ms(remaining.min(Duration::from_millis(20)));
            match unsafe { WaitForSingleObject(operation.event.raw(), wait_ms) } {
                WAIT_OBJECT_0 => {
                    let mut transferred = 0;
                    if unsafe {
                        GetOverlappedResult(
                            handle.raw(),
                            &mut operation.overlapped,
                            &mut transferred,
                            0,
                        )
                    } == 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    check_io_preflight(deadline, cancellation)?;
                    return Ok((transferred, operation));
                }
                WAIT_TIMEOUT => continue,
                _ => return Err(io::Error::last_os_error()),
            }
        }
    }

    fn cancel_or_quarantine<T>(
        handle: &mut OwnedHandle,
        mut operation: Box<PendingOperation>,
        kind: io::ErrorKind,
    ) -> io::Result<T> {
        unsafe {
            CancelIoEx(handle.raw(), &mut operation.overlapped);
        }
        if unsafe { WaitForSingleObject(operation.event.raw(), 0) } == WAIT_OBJECT_0 {
            let mut ignored = 0;
            unsafe {
                GetOverlappedResult(handle.raw(), &mut operation.overlapped, &mut ignored, 0);
            }
        } else {
            // The buffer, OVERLAPPED, event, and pipe handle must outlive an
            // unreaped operation. Transfer all of them to process-lifetime
            // quarantine rather than returning stack-backed memory to Rust.
            handle.quarantine(operation);
        }
        Err(io::Error::new(kind, "H4CT operation retired terminally"))
    }

    fn check_io_preflight(
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        if cancellation.is_cancelled() {
            Err(io::Error::new(io::ErrorKind::Interrupted, "H4CT cancelled"))
        } else if deadline.is_expired() {
            Err(io::Error::new(io::ErrorKind::TimedOut, "H4CT deadline"))
        } else {
            Ok(())
        }
    }

    fn duration_to_wait_ms(duration: Duration) -> Dword {
        duration
            .as_millis()
            .saturating_add(1)
            .min(Dword::MAX as u128) as Dword
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
pub use win32::{WindowsControlIo, WindowsControlListener, WindowsPipeBackend};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct MockIo {
        inbound: VecDeque<u8>,
        outbound: Vec<u8>,
    }

    impl DeadlineControlIo for MockIo {
        fn send_all_until(
            &mut self,
            bytes: &[u8],
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            self.outbound.extend_from_slice(bytes);
            Ok(())
        }

        fn recv_exact_until(
            &mut self,
            bytes: &mut [u8],
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> io::Result<()> {
            if cancellation.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            for byte in bytes {
                *byte = self
                    .inbound
                    .pop_front()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "mock EOF"))?;
            }
            Ok(())
        }
    }

    struct MockBackend {
        create_error: Option<ControlPipeError>,
        accept_error: Option<ControlPipeError>,
        identity_error: Option<ControlPipeError>,
        identity: ControlPeerIdentity,
        security_seen: Option<RestrictiveSecurityPolicy>,
        created: bool,
        accepted: bool,
        cancel_during_create: bool,
        cancel_during_accept: bool,
        cancel_during_identity: bool,
    }

    impl MockBackend {
        fn valid() -> Self {
            Self {
                create_error: None,
                accept_error: None,
                identity_error: None,
                identity: ControlPeerIdentity {
                    pid: 42,
                    creation_filetime: 9_001,
                },
                security_seen: None,
                created: false,
                accepted: false,
                cancel_during_create: false,
                cancel_during_accept: false,
                cancel_during_identity: false,
            }
        }
    }

    impl ControlPipeBackend for MockBackend {
        type Listener = ();
        type Io = MockIo;

        fn create_restrictive_listener(
            &mut self,
            _pipe_name: &str,
            security: RestrictiveSecurityPolicy,
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::Listener, ControlPipeError> {
            self.security_seen = Some(security);
            if self.cancel_during_create {
                cancellation.cancel();
            }
            if let Some(error) = self.create_error.take() {
                return Err(error);
            }
            self.created = true;
            Ok(())
        }

        fn accept(
            &mut self,
            _listener: Self::Listener,
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<Self::Io, ControlPipeError> {
            assert!(self.created, "accept before listener creation");
            if self.cancel_during_accept {
                cancellation.cancel();
            }
            if let Some(error) = self.accept_error.take() {
                return Err(error);
            }
            self.accepted = true;
            Ok(MockIo::default())
        }

        fn peer_identity(
            &mut self,
            _io: &Self::Io,
            _deadline: ControlDeadline,
            cancellation: &ControlCancellation,
        ) -> Result<ControlPeerIdentity, ControlPipeError> {
            assert!(self.accepted, "identity query before accept");
            if self.cancel_during_identity {
                cancellation.cancel();
            }
            if let Some(error) = self.identity_error.take() {
                return Err(error);
            }
            Ok(self.identity)
        }
    }

    fn deadline() -> ControlDeadline {
        ControlDeadline::after(Duration::from_secs(1)).unwrap()
    }

    fn session_id() -> [u8; 16] {
        static NEXT: AtomicU64 = AtomicU64::new(50_000);
        let mut id = [0x77; 16];
        id[8..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        id
    }

    #[test]
    fn listener_ready_proof_exists_only_after_restrictive_creation() {
        let cancellation = ControlCancellation::new();
        let prepared = prepare_control_listener(
            MockBackend::valid(),
            r"\\.\pipe\battle-plan_control_42_nonce",
            RestrictiveSecurityPolicy::default(),
            deadline(),
            &cancellation,
        )
        .unwrap();
        let _proof = prepared.listener_ready_proof();
        assert!(prepared.backend.created);
        assert_eq!(
            prepared.backend.security_seen,
            Some(RestrictiveSecurityPolicy {
                allow_local_system: false
            })
        );
    }

    #[test]
    fn bad_name_create_failure_cancel_and_deadline_publish_no_ready_state() {
        let cancellation = ControlCancellation::new();
        assert!(matches!(
            prepare_control_listener(
                MockBackend::valid(),
                r"\\server\pipe\remote",
                RestrictiveSecurityPolicy::default(),
                deadline(),
                &cancellation,
            ),
            Err(ControlPipeError::InvalidPipeName)
        ));

        let mut failed = MockBackend::valid();
        failed.create_error = Some(ControlPipeError::Io(io::ErrorKind::PermissionDenied));
        assert!(matches!(
            prepare_control_listener(
                failed,
                r"\\.\pipe\battle-plan_control_42_nonce",
                RestrictiveSecurityPolicy::default(),
                deadline(),
                &cancellation,
            ),
            Err(ControlPipeError::Io(io::ErrorKind::PermissionDenied))
        ));

        let cancelled = ControlCancellation::new();
        cancelled.cancel();
        assert!(matches!(
            prepare_control_listener(
                MockBackend::valid(),
                r"\\.\pipe\battle-plan_control_42_nonce",
                RestrictiveSecurityPolicy::default(),
                deadline(),
                &cancelled,
            ),
            Err(ControlPipeError::Cancelled)
        ));
        assert!(matches!(
            prepare_control_listener(
                MockBackend::valid(),
                r"\\.\pipe\battle-plan_control_42_nonce",
                RestrictiveSecurityPolicy::default(),
                ControlDeadline::at(std::time::Instant::now()),
                &cancellation,
            ),
            Err(ControlPipeError::DeadlineExpired)
        ));
    }

    #[test]
    fn admission_binds_pid_creation_time_and_capabilities() {
        let cancellation = ControlCancellation::new();
        let expected = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let heartbeat = ControlCapabilities::from_bits(ControlCapabilities::HEARTBEAT).unwrap();
        let prepared = prepare_control_listener(
            MockBackend::valid(),
            r"\\.\pipe\battle-plan_control_42_nonce",
            RestrictiveSecurityPolicy::default(),
            deadline(),
            &cancellation,
        )
        .unwrap();
        let channel = prepared
            .accept_and_admit(
                session_id(),
                expected,
                heartbeat,
                heartbeat,
                EndpointRole::Recorder,
                deadline(),
                &cancellation,
            )
            .unwrap();
        assert_eq!(channel.peer(), expected);
        assert_eq!(channel.capabilities(), heartbeat);
    }

    #[test]
    fn pid_filetime_capability_and_accept_faults_fail_closed() {
        let cancellation = ControlCancellation::new();
        let expected = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let heartbeat = ControlCapabilities::from_bits(ControlCapabilities::HEARTBEAT).unwrap();
        let none = ControlCapabilities::from_bits(0).unwrap();

        for observed in [
            ControlPeerIdentity {
                pid: 43,
                ..expected
            },
            ControlPeerIdentity {
                creation_filetime: 9_002,
                ..expected
            },
        ] {
            let mut backend = MockBackend::valid();
            backend.identity = observed;
            let prepared = prepare_control_listener(
                backend,
                r"\\.\pipe\battle-plan_control_42_nonce",
                RestrictiveSecurityPolicy::default(),
                deadline(),
                &cancellation,
            )
            .unwrap();
            assert!(matches!(
                prepared.accept_and_admit(
                    session_id(),
                    expected,
                    heartbeat,
                    heartbeat,
                    EndpointRole::Recorder,
                    deadline(),
                    &cancellation,
                ),
                Err(ControlPipeError::Peer(ControlError::PeerPidMismatch { .. }))
                    | Err(ControlPipeError::Peer(
                        ControlError::PeerCreationTimeMismatch { .. }
                    ))
            ));
        }

        let prepared = prepare_control_listener(
            MockBackend::valid(),
            r"\\.\pipe\battle-plan_control_42_nonce",
            RestrictiveSecurityPolicy::default(),
            deadline(),
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            prepared.accept_and_admit(
                session_id(),
                expected,
                heartbeat,
                none,
                EndpointRole::Recorder,
                deadline(),
                &cancellation,
            ),
            Err(ControlPipeError::Peer(
                ControlError::MissingCapabilities { .. }
            ))
        ));

        let mut backend = MockBackend::valid();
        backend.accept_error = Some(ControlPipeError::DeadlineExpired);
        let prepared = prepare_control_listener(
            backend,
            r"\\.\pipe\battle-plan_control_42_nonce",
            RestrictiveSecurityPolicy::default(),
            deadline(),
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            prepared.accept_and_admit(
                session_id(),
                expected,
                heartbeat,
                heartbeat,
                EndpointRole::Recorder,
                deadline(),
                &cancellation,
            ),
            Err(ControlPipeError::DeadlineExpired)
        ));
    }

    #[test]
    fn cancellation_races_after_create_accept_and_identity_publish_nothing() {
        let pipe = r"\\.\pipe\battle-plan_control_42_nonce";
        let expected = ControlPeerIdentity {
            pid: 42,
            creation_filetime: 9_001,
        };
        let heartbeat = ControlCapabilities::from_bits(ControlCapabilities::HEARTBEAT).unwrap();

        let mut create_backend = MockBackend::valid();
        create_backend.cancel_during_create = true;
        assert!(matches!(
            prepare_control_listener(
                create_backend,
                pipe,
                RestrictiveSecurityPolicy::default(),
                deadline(),
                &ControlCancellation::new(),
            ),
            Err(ControlPipeError::Cancelled)
        ));

        for cancel_accept in [true, false] {
            let mut backend = MockBackend::valid();
            backend.cancel_during_accept = cancel_accept;
            backend.cancel_during_identity = !cancel_accept;
            let cancellation = ControlCancellation::new();
            let prepared = prepare_control_listener(
                backend,
                pipe,
                RestrictiveSecurityPolicy::default(),
                deadline(),
                &cancellation,
            )
            .unwrap();
            assert!(matches!(
                prepared.accept_and_admit(
                    session_id(),
                    expected,
                    heartbeat,
                    heartbeat,
                    EndpointRole::Recorder,
                    deadline(),
                    &cancellation,
                ),
                Err(ControlPipeError::Cancelled)
            ));
        }
    }
}
