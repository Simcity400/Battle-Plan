//! Production Win32 H4CR/H4CT paired-server primitive.
//!
//! The positive ready proof in this module is created only after both named
//! pipes exist and both overlapped `ConnectNamedPipe` operations are armed.
//! Injection is guarded by [`AutoAttachDuplexProductPermit`], whose product
//! constructor remains disabled. Nothing in this module discovers a process,
//! injects a DLL, or selects itself from the live recorder entry point.
//!
//! `CreateNamedPipeW`, security construction, connect completion, and peer
//! validation remain inside bounded owned workers. Late results are dropped on
//! those workers and retire or quarantine every unpublished handle.

use std::ffi::c_void;
use std::fmt;
use std::io;
use std::mem;
use std::ptr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use crate::control_transport::{
    claim_session_capability, ControlCancellation, ControlCapabilities, ControlDeadline,
    ControlError, ControlPeerIdentity, DeadlineControlChannel, DeadlineControlInputReadiness,
    DeadlineControlIo, EndpointRole, ValidatedSessionCapability,
};
use crate::windows_control_pipe::RestrictiveSecurityPolicy;

use super::duplex_worker::{
    AutoAttachDuplexProductPermit, DuplexAttemptAuthority, DuplexWorkerError, DuplexWorkerSession,
    RoutedHeartbeatCycle, RoutedRolloverHeartbeatCycle,
};
use super::identity::ProcessKey;
use super::sources::WorkerSpec;
use crate::duplex_session::identity_control::{
    IdentityDurableCandidateSink, RecorderIdentityControlDispatcher,
};
use crate::duplex_session::rollover_control::{
    RecorderRolloverCoordinator, RecorderRolloverStore,
};

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;
type LocalPtr = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const PIPE_ACCESS_INBOUND: Dword = 0x0000_0001;
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
const SYNCHRONIZE: Dword = 0x0010_0000;
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
    fn PeekNamedPipe(
        pipe: Handle,
        buffer: *mut c_void,
        buffer_len: Dword,
        bytes_read: *mut Dword,
        total_bytes_available: *mut Dword,
        bytes_left_in_message: *mut Dword,
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

#[derive(Debug)]
pub enum WindowsCombinedBackendError {
    Authority(DuplexWorkerError),
    Io(io::ErrorKind),
    InvalidPipeName(&'static str),
    DeadlineExpired,
    Cancelled,
    TargetExited,
    PeerMismatch {
        pipe: &'static str,
        expected: ControlPeerIdentity,
        observed: ControlPeerIdentity,
    },
    Control(ControlError),
    Injection(String),
    SetupWorker(String),
    SetupWorkerPanicked,
}

impl fmt::Display for WindowsCombinedBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authority(error) => write!(f, "combined authority failed: {error}"),
            Self::Io(kind) => write!(f, "combined pipe I/O failed: {kind:?}"),
            Self::InvalidPipeName(pipe) => write!(f, "invalid local {pipe} pipe name"),
            Self::DeadlineExpired => write!(f, "combined operation deadline expired"),
            Self::Cancelled => write!(f, "combined operation cancelled"),
            Self::TargetExited => write!(f, "retained target process exited"),
            Self::PeerMismatch {
                pipe,
                expected,
                observed,
            } => write!(
                f,
                "{pipe} peer {observed:?} did not match retained peer {expected:?}"
            ),
            Self::Control(error) => write!(f, "control admission failed: {error}"),
            Self::Injection(error) => write!(f, "single injection failed: {error}"),
            Self::SetupWorker(error) => write!(f, "paired setup worker failed: {error}"),
            Self::SetupWorkerPanicked => write!(f, "paired setup worker panicked"),
        }
    }
}

impl std::error::Error for WindowsCombinedBackendError {}

impl From<io::Error> for WindowsCombinedBackendError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut => Self::DeadlineExpired,
            io::ErrorKind::Interrupted => Self::Cancelled,
            kind => Self::Io(kind),
        }
    }
}

impl From<ControlError> for WindowsCombinedBackendError {
    fn from(error: ControlError) -> Self {
        Self::Control(error)
    }
}

struct OwnedHandle(Handle);

// SAFETY: ownership of the raw kernel handle is unique, moving the wrapper
// between threads does not access the handle concurrently, and Drop closes it
// exactly once unless terminal quarantine deliberately takes ownership.
unsafe impl Send for OwnedHandle {}

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
        let sddl = if policy.allow_local_system {
            "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
        } else {
            "D:P(A;;GA;;;OW)"
        };
        let wide = wide(sddl);
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
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

// SAFETY: the OVERLAPPED, event, and buffer are heap-stable and move only as
// one owned box. No reference into them is published across threads.
unsafe impl Send for PendingOperation {}

impl PendingOperation {
    fn new(buffer: Vec<u8>) -> io::Result<Box<Self>> {
        let event = OwnedHandle::new(unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) })?;
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

struct QuarantinedOperation {
    _pipe: OwnedHandle,
    _operation: Box<PendingOperation>,
}

struct ArmedPipe {
    handle: Option<OwnedHandle>,
    pending: Option<Box<PendingOperation>>,
}

impl ArmedPipe {
    fn finish(
        mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
        target: Handle,
    ) -> Result<OwnedHandle, WindowsCombinedBackendError> {
        let handle = self.handle.take().expect("armed pipe owns handle");
        if let Some(operation) = self.pending.take() {
            return finish_pending_connect(handle, operation, deadline, cancellation, target);
        }
        preflight(deadline, cancellation)?;
        Ok(handle)
    }
}

impl Drop for ArmedPipe {
    fn drop(&mut self) {
        let (Some(handle), Some(operation)) = (self.handle.take(), self.pending.take()) else {
            return;
        };
        retire_or_quarantine(handle, operation);
    }
}

/// Opaque proof that both one-instance server pipes exist and both overlapped
/// connect operations were submitted before the injection boundary.
pub struct WindowsCombinedListenersReadyProof {
    _private: (),
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

impl WindowsCombinedListenersReadyProof {
    pub fn startup_deadline(&self) -> ControlDeadline {
        self.startup_deadline
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }
}

struct BoundTargetProcess {
    handle: OwnedHandle,
}

impl BoundTargetProcess {
    fn open(key: ProcessKey) -> Result<Self, WindowsCombinedBackendError> {
        let handle = open_process_identity(key.pid)?;
        let observed = process_creation_filetime(&handle)?;
        if observed != key.creation_filetime.0 {
            return Err(WindowsCombinedBackendError::PeerMismatch {
                pipe: "target",
                expected: ControlPeerIdentity {
                    pid: key.pid,
                    creation_filetime: key.creation_filetime.0,
                },
                observed: ControlPeerIdentity {
                    pid: key.pid,
                    creation_filetime: observed,
                },
            });
        }
        Ok(Self { handle })
    }

    fn ensure_live(
        &self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), WindowsCombinedBackendError> {
        preflight(deadline, cancellation)?;
        match unsafe { WaitForSingleObject(self.handle.raw(), 0) } {
            WAIT_TIMEOUT => Ok(()),
            WAIT_OBJECT_0 => {
                cancellation.cancel();
                Err(WindowsCombinedBackendError::TargetExited)
            }
            _ => {
                cancellation.cancel();
                Err(io::Error::last_os_error().into())
            }
        }
    }
}

/// Both connects are armed. The authority, target handle, deadline and shared
/// cancellation cannot be split or replaced before the one permitted injection.
pub struct PreparedWindowsServerPair {
    authority: DuplexAttemptAuthority,
    data: ArmedPipe,
    control: ArmedPipe,
    target: BoundTargetProcess,
    deadline: ControlDeadline,
    cancellation: ControlCancellation,
    ready: WindowsCombinedListenersReadyProof,
    setup_worker: Option<SetupWorkerKeepalive>,
}

struct SetupWorkerKeepalive {
    release: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Drop for SetupWorkerKeepalive {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Create the H4CR inbound and H4CT duplex server handles and arm both accepts.
/// This function creates no relative timeout; every step consumes `deadline`.
fn prepare_server_pair(
    spec: &WorkerSpec,
    security: RestrictiveSecurityPolicy,
    deadline: ControlDeadline,
    cancellation: ControlCancellation,
) -> Result<PreparedWindowsServerPair, WindowsCombinedBackendError> {
    let authority = DuplexAttemptAuthority::from_worker_spec(spec)
        .map_err(WindowsCombinedBackendError::Authority)?;
    preflight(deadline, &cancellation)?;
    let target = BoundTargetProcess::open(authority.process_key())?;
    target.ensure_live(deadline, &cancellation)?;

    let data_handle = create_pipe(
        authority.data_pipe_name(),
        "H4CR",
        PIPE_ACCESS_INBOUND,
        security,
        deadline,
        &cancellation,
    )?;
    let data = arm_connect(data_handle, deadline, &cancellation)?;
    target.ensure_live(deadline, &cancellation)?;

    let control_handle = create_pipe(
        authority.control_pipe_name(),
        "H4CT",
        PIPE_ACCESS_DUPLEX,
        security,
        deadline,
        &cancellation,
    )?;
    let control = arm_connect(control_handle, deadline, &cancellation)?;
    target.ensure_live(deadline, &cancellation)?;

    Ok(PreparedWindowsServerPair {
        authority,
        data,
        control,
        target,
        deadline,
        cancellation: cancellation.clone(),
        ready: WindowsCombinedListenersReadyProof {
            _private: (),
            startup_deadline: deadline,
            cancellation,
        },
        setup_worker: None,
    })
}

/// Bounded production preparation seam. The worker owns the immutable spec,
/// security allocation, both candidate handles, and both armed connect
/// operations. If the caller's unchanged startup deadline/cancellation wins,
/// dropping the receiver leaves the worker as terminal quarantine; its late
/// result is dropped there, cancelling/reaping or quarantining both connects.
pub fn prepare_server_pair_bounded(
    spec: &WorkerSpec,
    security: RestrictiveSecurityPolicy,
    deadline: ControlDeadline,
    cancellation: ControlCancellation,
) -> Result<PreparedWindowsServerPair, WindowsCombinedBackendError> {
    preflight(deadline, &cancellation)?;
    let spec = spec.clone();
    let worker_cancellation = cancellation.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("h4cr-h4ct-pair-setup".to_string())
        .spawn(move || {
            let result = prepare_server_pair(&spec, security, deadline, worker_cancellation);
            if sender.send(result).is_ok() {
                // Overlapped ConnectNamedPipe operations are owned by the
                // initiating thread on Windows. Exiting this thread before the
                // caller reaps them aborts both operations with
                // ERROR_OPERATION_ABORTED.
                let _ = release_receiver.recv();
            }
        })
        .map_err(|error| WindowsCombinedBackendError::SetupWorker(error.to_string()))?;
    loop {
        preflight(deadline, &cancellation)?;
        let remaining = deadline
            .remaining()
            .ok_or(WindowsCombinedBackendError::DeadlineExpired)?;
        match receiver.recv_timeout(remaining.min(Duration::from_millis(10))) {
            Ok(Ok(mut prepared)) => {
                prepared.setup_worker = Some(SetupWorkerKeepalive {
                    release: Some(release_sender),
                    worker: Some(worker),
                });
                return Ok(prepared);
            }
            Ok(Err(error)) => {
                let _ = release_sender.send(());
                worker
                    .join()
                    .map_err(|_| WindowsCombinedBackendError::SetupWorkerPanicked)?;
                return Err(error);
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                let _ = release_sender.send(());
                return match worker.join() {
                    Ok(()) => Err(WindowsCombinedBackendError::SetupWorker(
                        "paired setup worker disconnected".to_string(),
                    )),
                    Err(_) => Err(WindowsCombinedBackendError::SetupWorkerPanicked),
                };
            }
        }
    }
}

impl PreparedWindowsServerPair {
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn listeners_ready_proof(&self) -> &WindowsCombinedListenersReadyProof {
        &self.ready
    }

    /// The permit is unmintable whenever a product prerequisite is false. No
    /// pipe event or received control message can call this closure.
    pub fn commit_single_injection_for_product<F>(
        self,
        _permit: AutoAttachDuplexProductPermit,
        inject_once: F,
    ) -> Result<WindowsInjectionCommitted, WindowsCombinedBackendError>
    where
        F: FnOnce(
            &WindowsCombinedListenersReadyProof,
            &DuplexAttemptAuthority,
        ) -> Result<(), String>,
    {
        self.target.ensure_live(self.deadline, &self.cancellation)?;
        if let Err(error) = inject_once(&self.ready, &self.authority) {
            self.cancellation.cancel();
            return Err(WindowsCombinedBackendError::Injection(error));
        }
        self.target.ensure_live(self.deadline, &self.cancellation)?;
        Ok(WindowsInjectionCommitted {
            authority: self.authority,
            data: self.data,
            control: self.control,
            target: self.target,
            deadline: self.deadline,
            cancellation: self.cancellation,
            setup_worker: self.setup_worker,
        })
    }
}

pub struct WindowsInjectionCommitted {
    authority: DuplexAttemptAuthority,
    data: ArmedPipe,
    control: ArmedPipe,
    target: BoundTargetProcess,
    deadline: ControlDeadline,
    cancellation: ControlCancellation,
    setup_worker: Option<SetupWorkerKeepalive>,
}

impl WindowsInjectionCommitted {
    /// Wait for both already-armed connects, validate both client PID/creation
    /// pairs, then publish both endpoints atomically.
    fn accept_and_bind(mut self) -> Result<ConnectedWindowsPipePair, WindowsCombinedBackendError> {
        self.target.ensure_live(self.deadline, &self.cancellation)?;
        let data_handle =
            self.data
                .finish(self.deadline, &self.cancellation, self.target.handle.raw())?;
        self.target.ensure_live(self.deadline, &self.cancellation)?;
        let control_handle =
            self.control
                .finish(self.deadline, &self.cancellation, self.target.handle.raw())?;
        // Both initiating-thread-owned connect operations are now reaped, so
        // the setup thread may exit without aborting live pipe I/O.
        drop(self.setup_worker.take());
        self.target.ensure_live(self.deadline, &self.cancellation)?;

        let expected = self.authority.expected_peer();
        let data_process = validate_pipe_peer(
            "H4CR",
            &data_handle,
            expected,
            self.deadline,
            &self.cancellation,
        )?;
        preflight(self.deadline, &self.cancellation)?;
        let control_process = validate_pipe_peer(
            "H4CT",
            &control_handle,
            expected,
            self.deadline,
            &self.cancellation,
        )?;
        self.target.ensure_live(self.deadline, &self.cancellation)?;

        let none = ControlCapabilities::from_bits(0)?;
        let session = claim_session_capability(self.authority.session_id())?;
        let admission = ValidatedSessionCapability::admit(session, expected, expected, none, none)?;
        let control_io =
            WindowsControlIo::new(control_handle, control_process, self.cancellation.clone());
        let control = DeadlineControlChannel::new(control_io, EndpointRole::Recorder, admission);
        Ok(ConnectedWindowsPipePair {
            authority: self.authority,
            data: WindowsDataIo::new(data_handle, data_process, self.cancellation.clone()),
            control,
            target: self.target,
            startup_deadline: self.deadline,
            cancellation: self.cancellation,
        })
    }

    /// Bound connect completion and peer PID/creation revalidation on one
    /// owned worker. A late pair can never publish after caller expiry: the
    /// dropped receiver makes the worker drop and terminally retire it.
    pub fn accept_and_bind_bounded(
        self,
    ) -> Result<ConnectedWindowsPipePair, WindowsCombinedBackendError> {
        let deadline = self.deadline;
        let cancellation = self.cancellation.clone();
        preflight(deadline, &cancellation)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("h4cr-h4ct-pair-accept".to_string())
            .spawn(move || {
                let result = self.accept_and_bind();
                let _ = sender.send(result);
            })
            .map_err(|error| WindowsCombinedBackendError::SetupWorker(error.to_string()))?;
        loop {
            preflight(deadline, &cancellation)?;
            let remaining = deadline
                .remaining()
                .ok_or(WindowsCombinedBackendError::DeadlineExpired)?;
            match receiver.recv_timeout(remaining.min(Duration::from_millis(10))) {
                Ok(result) => {
                    worker
                        .join()
                        .map_err(|_| WindowsCombinedBackendError::SetupWorkerPanicked)?;
                    return result;
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return match worker.join() {
                        Ok(()) => Err(WindowsCombinedBackendError::SetupWorker(
                            "paired accept worker disconnected".to_string(),
                        )),
                        Err(_) => Err(WindowsCombinedBackendError::SetupWorkerPanicked),
                    };
                }
            }
        }
    }
}

/// Atomically published pair. H4CR and H4CT retain the same cancellation and
/// exact target identity. The central receive method is the only recommended
/// control receive seam; it lets one router dispatch CIF1 versus heartbeat.
pub struct ConnectedWindowsPipePair {
    authority: DuplexAttemptAuthority,
    data: WindowsDataIo,
    control: DeadlineControlChannel<WindowsControlIo>,
    target: BoundTargetProcess,
    startup_deadline: ControlDeadline,
    cancellation: ControlCancellation,
}

impl ConnectedWindowsPipePair {
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }

    pub fn check_expected_target_live(&self) -> Result<(), WindowsCombinedBackendError> {
        if self.cancellation.is_cancelled() {
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        match unsafe { WaitForSingleObject(self.target.handle.raw(), 0) } {
            WAIT_TIMEOUT => Ok(()),
            // Target exit ends control liveness but must not cancel H4CR: the
            // data half still owns draining every already-written complete
            // record as the valid crash prefix.
            WAIT_OBJECT_0 => Err(WindowsCombinedBackendError::TargetExited),
            _ => {
                self.cancellation.cancel();
                Err(io::Error::last_os_error().into())
            }
        }
    }

    /// Consume the atomically published pair into the only public runner form.
    /// Raw H4CT receive and raw H4CR handle access stay private; the runner
    /// exposes the central `DuplexWorkerSession` router and cancellation-aware
    /// exact data reads only.
    pub fn into_runner(self) -> WindowsCombinedRunner {
        let expectation = self.authority.collector_hello_expectation().clone();
        let control = DuplexWorkerSession::new(
            self.control,
            expectation,
            self.startup_deadline,
            self.cancellation.clone(),
        );
        WindowsCombinedRunner {
            authority: self.authority,
            data: self.data,
            control: Some(control),
            target: self.target,
            cancellation: self.cancellation,
            next_heartbeat_due: std::time::Instant::now(),
        }
    }
}

/// Paired production runner. All control
/// receives remain inside `DuplexWorkerSession`; all H4CR reads retain the same
/// target handle and shared cancellation token.
pub struct WindowsCombinedRunner {
    authority: DuplexAttemptAuthority,
    data: WindowsDataIo,
    control: Option<DuplexWorkerSession<WindowsControlIo>>,
    target: BoundTargetProcess,
    cancellation: ControlCancellation,
    next_heartbeat_due: std::time::Instant,
}

impl WindowsCombinedRunner {
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn control_session_mut(&mut self) -> Option<&mut DuplexWorkerSession<WindowsControlIo>> {
        self.control.as_mut()
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }

    /// Consume the authenticated pair into independently ownable H4CR and
    /// H4CT drivers. Both retain the same attempt authority and shared
    /// cancellation; no raw pipe/process handle is exposed. This is the
    /// product composition boundary for a blocking data-drain worker plus a
    /// recurring-heartbeat/cooperative-stop control worker.
    pub fn into_concurrent_parts(
        self,
    ) -> Result<(WindowsCombinedDataReader, WindowsCombinedControlDriver), WindowsCombinedBackendError>
    {
        let Self {
            authority,
            data,
            control,
            target,
            cancellation,
            next_heartbeat_due,
        } = self;
        let control = control.ok_or_else(|| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                from: super::duplex_worker::DuplexWorkerState::Finalizing,
                action: "split runner after control close",
            })
        })?;
        Ok((
            WindowsCombinedDataReader {
                authority: authority.clone(),
                data,
                cancellation: cancellation.clone(),
            },
            WindowsCombinedControlDriver {
                authority,
                control: Some(control),
                target,
                cancellation,
                next_heartbeat_due,
            },
        ))
    }

    pub fn read_data_exact_until(
        &mut self,
        bytes: &mut [u8],
        deadline: ControlDeadline,
    ) -> Result<(), WindowsCombinedBackendError> {
        self.ensure_target_live()?;
        self.data
            .recv_exact_until(bytes, deadline)
            .map_err(WindowsCombinedBackendError::from)?;
        self.ensure_target_live()
    }

    pub fn check_recurring_liveness(&mut self) -> Result<(), WindowsCombinedBackendError> {
        self.ensure_target_live()?;
        self.control
            .as_mut()
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                    from: super::duplex_worker::DuplexWorkerState::Finalizing,
                    action: "check liveness after control close",
                })
            })?
            .check_liveness()
            .map_err(WindowsCombinedBackendError::Authority)
    }

    /// Recurring-heartbeat cadence for the paired product runner. The
    /// owner calls this from its dedicated control worker; a due operation is
    /// synchronous but bounded by one unchanged five-second deadline. It uses a
    /// fresh CSPRNG challenge and routes asynchronous CIF1 through the same H4CT
    /// receive loop before accepting the matching proof.
    pub fn drive_due_heartbeat<S: IdentityDurableCandidateSink>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<S>,
    ) -> Result<Option<RoutedHeartbeatCycle>, WindowsCombinedBackendError> {
        self.ensure_target_live()?;
        let now = std::time::Instant::now();
        if now < self.next_heartbeat_due {
            return Ok(None);
        }
        let challenge = super::staging::generate_secret_128().map_err(|error| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(error))
        })?;
        let deadline =
            ControlDeadline::after(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
                .map_err(|error| {
                    WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                        error.to_string(),
                    ))
                })?;
        let outcome = self
            .control
            .as_mut()
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                    from: super::duplex_worker::DuplexWorkerState::Finalizing,
                    action: "heartbeat after control close",
                })
            })?
            .heartbeat_cycle_with_identity(identity, challenge, deadline)
            .map_err(WindowsCombinedBackendError::Authority)?;
        self.ensure_target_live()?;
        self.next_heartbeat_due = now
            .checked_add(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                    "heartbeat schedule overflow".to_string(),
                ))
            })?;
        Ok(Some(outcome))
    }

    /// Rollover-aware heartbeat cadence. This is the production router for a
    /// rollover-capable session: a PausedForControl proof is followed by one
    /// CRP1 durable transaction under the unchanged cycle deadline.
    pub fn drive_due_heartbeat_with_rollover<IS, RS>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<IS>,
        rollover: &mut RecorderRolloverCoordinator,
        store: &mut RS,
    ) -> Result<Option<RoutedRolloverHeartbeatCycle>, WindowsCombinedBackendError>
    where
        IS: IdentityDurableCandidateSink,
        RS: RecorderRolloverStore,
    {
        self.ensure_target_live()?;
        let now = std::time::Instant::now();
        if now < self.next_heartbeat_due {
            return Ok(None);
        }
        let challenge = super::staging::generate_secret_128().map_err(|error| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(error))
        })?;
        let deadline =
            ControlDeadline::after(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
                .map_err(|error| {
                    WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                        error.to_string(),
                    ))
                })?;
        let outcome = self
            .control
            .as_mut()
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                    from: super::duplex_worker::DuplexWorkerState::Finalizing,
                    action: "rollover heartbeat after control close",
                })
            })?
            .heartbeat_cycle_with_identity_and_rollover(
                identity,
                rollover,
                store,
                challenge,
                deadline,
            )
            .map_err(WindowsCombinedBackendError::Authority)?;
        self.ensure_target_live()?;
        self.next_heartbeat_due = now
            .checked_add(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                    "heartbeat schedule overflow".to_string(),
                ))
            })?;
        Ok(Some(outcome))
    }

    /// Transfer explicit-stop ownership from H4CT to the H4CR drain/finalizer.
    /// Dropping the control session closes the server handle, which is the v1
    /// collector's authenticated graceful-stop receipt.  It intentionally does
    /// not cancel the shared data transport: the caller must continue reading
    /// H4CR to a clean record boundary/EOF, fsync, fold, pack, and verify under
    /// the outer bounded graceful-stop deadline.
    pub fn close_control_for_graceful_data_drain(
        &mut self,
    ) -> Result<ControlDeadline, WindowsCombinedBackendError> {
        let stop_deadline = ControlDeadline::after(Duration::from_millis(
            super::policy::GRACEFUL_STOP_DEADLINE_MS,
        ))
        .map_err(|error| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(error.to_string()))
        })?;
        let mut control = self.control.take().ok_or_else(|| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                from: super::duplex_worker::DuplexWorkerState::Finalizing,
                action: "close control twice",
            })
        })?;
        control.request_graceful_stop_at(std::time::Instant::now());
        control
            .begin_finalizing()
            .map_err(WindowsCombinedBackendError::Authority)?;
        drop(control);
        Ok(stop_deadline)
    }

    fn ensure_target_live(&self) -> Result<(), WindowsCombinedBackendError> {
        if self.cancellation.is_cancelled() {
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        match unsafe { WaitForSingleObject(self.target.handle.raw(), 0) } {
            WAIT_TIMEOUT => Ok(()),
            WAIT_OBJECT_0 => {
                self.cancellation.cancel();
                Err(WindowsCombinedBackendError::TargetExited)
            }
            _ => {
                self.cancellation.cancel();
                Err(io::Error::last_os_error().into())
            }
        }
    }
}

/// H4CR half of an authenticated combined pair. The underlying overlapped
/// transport retains its independently opened exact peer-process handle, so a
/// split does not downgrade PID/creation-FILETIME authority.
pub struct WindowsCombinedDataReader {
    authority: DuplexAttemptAuthority,
    data: WindowsDataIo,
    cancellation: ControlCancellation,
}

impl WindowsCombinedDataReader {
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }

    /// Copy, but do not consume, the currently queued H4CR prefix and report
    /// the total queued byte count. This is used only by the pre-CID1 router so
    /// a partial data record cannot starve a complete CIF1 on H4CT.
    pub fn peek_available_prefix(
        &self,
        prefix: &mut [u8],
    ) -> Result<(usize, usize), WindowsCombinedBackendError> {
        if self.cancellation.is_cancelled() {
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        self.data
            .peek_available_prefix(prefix)
            .map_err(WindowsCombinedBackendError::from)
    }

    pub fn read_exact_until(
        &mut self,
        bytes: &mut [u8],
        deadline: ControlDeadline,
    ) -> Result<(), WindowsCombinedBackendError> {
        if self.cancellation.is_cancelled() {
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        self.data
            .recv_exact_until(bytes, deadline)
            .map_err(WindowsCombinedBackendError::from)
    }

    /// Process-lifetime H4CR read. It has no synthetic duration deadline;
    /// cooperative stop closes H4CT and lets the collector close H4CR, while
    /// the shared cancellation remains the bounded hard-failure escape hatch.
    pub fn read_exact(&mut self, bytes: &mut [u8]) -> Result<(), WindowsCombinedBackendError> {
        if self.cancellation.is_cancelled() {
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        self.data
            .recv_exact_cancellable(bytes)
            .map_err(WindowsCombinedBackendError::from)
    }
}

/// H4CT half of an authenticated combined pair. It exclusively owns all
/// control receives, the retained target handle, heartbeat cadence, and the
/// cooperative close that transfers ownership to H4CR drain/finalization.
pub struct WindowsCombinedControlDriver {
    authority: DuplexAttemptAuthority,
    control: Option<DuplexWorkerSession<WindowsControlIo>>,
    target: BoundTargetProcess,
    cancellation: ControlCancellation,
    next_heartbeat_due: std::time::Instant,
}

impl WindowsCombinedControlDriver {
    pub fn authority(&self) -> &DuplexAttemptAuthority {
        &self.authority
    }

    pub fn cancellation(&self) -> &ControlCancellation {
        &self.cancellation
    }

    pub fn control_session_mut(&mut self) -> Option<&mut DuplexWorkerSession<WindowsControlIo>> {
        self.control.as_mut()
    }

    pub fn check_recurring_liveness(&mut self) -> Result<(), WindowsCombinedBackendError> {
        self.ensure_target_live()?;
        self.control
            .as_mut()
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                    from: super::duplex_worker::DuplexWorkerState::Finalizing,
                    action: "check liveness after control close",
                })
            })?
            .check_liveness()
            .map_err(WindowsCombinedBackendError::Authority)
    }

    pub fn drive_due_heartbeat<S: IdentityDurableCandidateSink>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<S>,
    ) -> Result<Option<RoutedHeartbeatCycle>, WindowsCombinedBackendError> {
        self.ensure_target_live()?;
        let now = std::time::Instant::now();
        if now < self.next_heartbeat_due {
            return Ok(None);
        }
        let challenge = super::staging::generate_secret_128().map_err(|error| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(error))
        })?;
        let deadline =
            ControlDeadline::after(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
                .map_err(|error| {
                    WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                        error.to_string(),
                    ))
                })?;
        let outcome = self
            .control
            .as_mut()
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                    from: super::duplex_worker::DuplexWorkerState::Finalizing,
                    action: "heartbeat after control close",
                })
            })?
            .heartbeat_cycle_with_identity(identity, challenge, deadline)
            .map_err(WindowsCombinedBackendError::Authority)?;
        self.ensure_target_live()?;
        self.next_heartbeat_due = now
            .checked_add(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                    "heartbeat schedule overflow".to_string(),
                ))
            })?;
        Ok(Some(outcome))
    }

    /// Rollover-aware variant used by the split production control driver.
    pub fn drive_due_heartbeat_with_rollover<IS, RS>(
        &mut self,
        identity: &mut RecorderIdentityControlDispatcher<IS>,
        rollover: &mut RecorderRolloverCoordinator,
        store: &mut RS,
    ) -> Result<Option<RoutedRolloverHeartbeatCycle>, WindowsCombinedBackendError>
    where
        IS: IdentityDurableCandidateSink,
        RS: RecorderRolloverStore,
    {
        self.ensure_target_live()?;
        let now = std::time::Instant::now();
        if now < self.next_heartbeat_due {
            return Ok(None);
        }
        let challenge = super::staging::generate_secret_128().map_err(|error| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(error))
        })?;
        let deadline =
            ControlDeadline::after(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
                .map_err(|error| {
                    WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                        error.to_string(),
                    ))
                })?;
        let outcome = self
            .control
            .as_mut()
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                    from: super::duplex_worker::DuplexWorkerState::Finalizing,
                    action: "rollover heartbeat after control close",
                })
            })?
            .heartbeat_cycle_with_identity_and_rollover(
                identity,
                rollover,
                store,
                challenge,
                deadline,
            )
            .map_err(WindowsCombinedBackendError::Authority)?;
        self.ensure_target_live()?;
        self.next_heartbeat_due = now
            .checked_add(Duration::from_millis(super::policy::HEARTBEAT_INTERVAL_MS))
            .ok_or_else(|| {
                WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(
                    "heartbeat schedule overflow".to_string(),
                ))
            })?;
        Ok(Some(outcome))
    }

    pub fn close_for_graceful_data_drain(
        &mut self,
    ) -> Result<ControlDeadline, WindowsCombinedBackendError> {
        let stop_deadline = ControlDeadline::after(Duration::from_millis(
            super::policy::GRACEFUL_STOP_DEADLINE_MS,
        ))
        .map_err(|error| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::Transport(error.to_string()))
        })?;
        let mut control = self.control.take().ok_or_else(|| {
            WindowsCombinedBackendError::Authority(DuplexWorkerError::IllegalTransition {
                from: super::duplex_worker::DuplexWorkerState::Finalizing,
                action: "close control twice",
            })
        })?;
        control.request_graceful_stop_at(std::time::Instant::now());
        control
            .begin_finalizing()
            .map_err(WindowsCombinedBackendError::Authority)?;
        drop(control);
        Ok(stop_deadline)
    }

    fn ensure_target_live(&self) -> Result<(), WindowsCombinedBackendError> {
        if self.cancellation.is_cancelled() {
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        match unsafe { WaitForSingleObject(self.target.handle.raw(), 0) } {
            WAIT_TIMEOUT => Ok(()),
            // Do not cancel the data half: it owns valid-prefix drain after a
            // target exit and will classify EOF only at a record boundary.
            WAIT_OBJECT_0 => Err(WindowsCombinedBackendError::TargetExited),
            _ => {
                self.cancellation.cancel();
                Err(io::Error::last_os_error().into())
            }
        }
    }
}

pub struct WindowsDataIo {
    pipe: WindowsOverlappedPipe,
}

impl WindowsDataIo {
    fn new(handle: OwnedHandle, peer: OwnedHandle, cancellation: ControlCancellation) -> Self {
        Self {
            pipe: WindowsOverlappedPipe::new(handle, peer, cancellation),
        }
    }

    pub fn recv_exact_until(
        &mut self,
        bytes: &mut [u8],
        deadline: ControlDeadline,
    ) -> io::Result<()> {
        self.pipe.recv_exact(bytes, deadline, None)
    }

    fn peek_available_prefix(&self, prefix: &mut [u8]) -> io::Result<(usize, usize)> {
        self.pipe.peek_available_prefix(prefix)
    }


    pub fn recv_exact_cancellable(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.pipe.recv_exact_cancellable(bytes)
    }
}

pub struct WindowsControlIo {
    pipe: WindowsOverlappedPipe,
}

impl WindowsControlIo {
    fn new(handle: OwnedHandle, peer: OwnedHandle, cancellation: ControlCancellation) -> Self {
        Self {
            pipe: WindowsOverlappedPipe::new(handle, peer, cancellation),
        }
    }
}

impl DeadlineControlInputReadiness for WindowsControlIo {
    fn input_bytes_available(&self) -> io::Result<usize> {
        self.pipe.peek_available_prefix(&mut []).map(|(_, total)| total)
    }
}

impl DeadlineControlIo for WindowsControlIo {
    fn send_all_until(
        &mut self,
        bytes: &[u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        self.pipe.send_all(bytes, deadline, Some(cancellation))
    }

    fn recv_exact_until(
        &mut self,
        bytes: &mut [u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> io::Result<()> {
        self.pipe.recv_exact(bytes, deadline, Some(cancellation))
    }
}

/// Largest length the process-lifetime H4CR read submits to a single
/// `ReadFile`, and therefore the size of the one reusable scratch buffer.
///
/// A byte-mode named pipe returns at most what its kernel buffer holds, so
/// asking for more than [`H4CR_PIPE_BUFFER_BYTES`] in one call never moves
/// more bytes; it only decides how much memory the reader has to own. Keeping
/// the two equal makes the common case a single `ReadFile` per record while
/// the scratch stays at one megabyte no matter how large a record gets.
const H4CR_READ_CHUNK_BYTES: usize = H4CR_PIPE_BUFFER_BYTES;

struct WindowsOverlappedPipe {
    handle: Option<OwnedHandle>,
    peer_process: OwnedHandle,
    shared_cancellation: ControlCancellation,
    /// Reusable landing buffer for the process-lifetime H4CR read.
    ///
    /// Every `ReadFile` used to be handed a freshly allocated, freshly ZEROED
    /// `Vec` sized to the whole remaining record. Because the pipe returns at
    /// most its kernel buffer per call, a record larger than that buffer was
    /// read in several passes and each pass re-zeroed the entire remaining
    /// length -- the zeroing was quadratic in record size. On the t15 capture
    /// (2.81 GiB of payload, 4,865 records over 64 KiB, largest 8.96 MiB)
    /// that came to 78.86 GiB of memset for 2.81 GiB actually moved. The
    /// collector blocks inside `WriteFileSync` for exactly as long as this
    /// side takes to drain, so the amplification was charged straight to the
    /// capture's per-frame budget.
    ///
    /// Reuse is sound: the buffer is only ever taken back from an operation
    /// that COMPLETED, which means `GetOverlappedResult` has returned and the
    /// kernel no longer owns the memory. An operation that was cancelled or
    /// failed is quarantined together with its pipe handle by
    /// [`retire_or_quarantine`] and its buffer never comes back here.
    read_scratch: Option<Vec<u8>>,
}

impl WindowsOverlappedPipe {
    fn new(
        handle: OwnedHandle,
        peer_process: OwnedHandle,
        shared_cancellation: ControlCancellation,
    ) -> Self {
        Self {
            handle: Some(handle),
            peer_process,
            shared_cancellation,
            read_scratch: None,
        }
    }

    fn peek_available_prefix(&self, prefix: &mut [u8]) -> io::Result<(usize, usize)> {
        if self.shared_cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "combined pipe startup probe cancelled",
            ));
        }
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe handle closed"))?;
        let prefix_len = Dword::try_from(prefix.len()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "pipe peek prefix is too large")
        })?;
        let mut copied = 0u32;
        let mut available = 0u32;
        let buffer = if prefix.is_empty() {
            ptr::null_mut()
        } else {
            prefix.as_mut_ptr().cast::<c_void>()
        };
        if unsafe {
            PeekNamedPipe(
                handle.raw(),
                buffer,
                prefix_len,
                &mut copied,
                &mut available,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if self.shared_cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "combined pipe startup probe cancelled after completion",
            ));
        }
        Ok((copied as usize, available as usize))
    }

    fn recv_exact(
        &mut self,
        mut bytes: &mut [u8],
        deadline: ControlDeadline,
        caller_cancellation: Option<&ControlCancellation>,
    ) -> io::Result<()> {
        while !bytes.is_empty() {
            let len = bytes.len().min(Dword::MAX as usize);
            let (transferred, buffer) =
                self.transfer(None, len, deadline, caller_cancellation)?;
            if transferred == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "pipe EOF"));
            }
            bytes[..transferred].copy_from_slice(&buffer[..transferred]);
            bytes = &mut bytes[transferred..];
        }
        Ok(())
    }

    fn recv_exact_cancellable(&mut self, mut bytes: &mut [u8]) -> io::Result<()> {
        let mut read_any = false;
        while !bytes.is_empty() {
            let transferred = match self.transfer_cancellable(bytes) {
                Ok(completed) => completed,
                Err(error)
                    if read_any
                        && matches!(
                            error.kind(),
                            io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof
                        ) =>
                {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "pipe EOF after a partial exact read",
                    ));
                }
                Err(error) => return Err(error),
            };
            if transferred == 0 {
                return Err(io::Error::new(
                    if read_any {
                        io::ErrorKind::UnexpectedEof
                    } else {
                        io::ErrorKind::BrokenPipe
                    },
                    if read_any {
                        "pipe EOF after a partial exact read"
                    } else {
                        "pipe EOF before exact read"
                    },
                ));
            }
            read_any = true;
            bytes = &mut bytes[transferred..];
        }
        Ok(())
    }

    /// Read once into the front of `dest`, returning how many bytes landed.
    ///
    /// The submitted length is `min(dest.len(), H4CR_READ_CHUNK_BYTES)` and
    /// never the scratch buffer's own length. That distinction is load
    /// bearing: `ReadFile` on a byte-mode pipe hands back as much as it was
    /// asked for and as much as is buffered, so submitting the full scratch
    /// length would pull bytes belonging to the NEXT record out of the pipe
    /// and then drop them on the floor when only `dest.len()` were copied
    /// out, desynchronising the record stream.
    fn transfer_cancellable(&mut self, dest: &mut [u8]) -> io::Result<usize> {
        if self.shared_cancellation.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "pipe cancelled"));
        }
        let submit_len = dest.len().min(H4CR_READ_CHUNK_BYTES);
        let scratch = self
            .read_scratch
            .take()
            .unwrap_or_else(|| vec![0; H4CR_READ_CHUNK_BYTES]);
        let mut operation = PendingOperation::new(scratch)?;
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe is terminal"))?;
        let mut transferred = 0;
        if unsafe {
            ReadFile(
                handle.raw(),
                operation.buffer.as_mut_ptr().cast(),
                submit_len as Dword,
                &mut transferred,
                &mut operation.overlapped,
            )
        } == 0
        {
            let error = unsafe { GetLastError() };
            if error != ERROR_IO_PENDING {
                self.handle.take();
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            let (completed, returned_operation) = wait_pending_transfer_cancellable(
                &mut self.handle,
                operation,
                &self.shared_cancellation,
            )?;
            transferred = completed;
            operation = returned_operation;
        }
        if self.shared_cancellation.is_cancelled() {
            self.handle.take();
            return Err(io::Error::new(io::ErrorKind::Interrupted, "pipe cancelled"));
        }
        // The operation completed, so the kernel has released the buffer and
        // it can be handed back for the next read. Copy out only the bytes the
        // pipe actually delivered -- the rest of the scratch is whatever the
        // previous record left there.
        let transferred = (transferred as usize).min(submit_len);
        dest[..transferred].copy_from_slice(&operation.buffer[..transferred]);
        self.read_scratch = Some(operation.buffer);
        Ok(transferred)
    }

    fn send_all(
        &mut self,
        mut bytes: &[u8],
        deadline: ControlDeadline,
        caller_cancellation: Option<&ControlCancellation>,
    ) -> io::Result<()> {
        while !bytes.is_empty() {
            let len = bytes.len().min(Dword::MAX as usize);
            let (transferred, _) =
                self.transfer(Some(&bytes[..len]), 0, deadline, caller_cancellation)?;
            if transferred == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "zero-byte pipe write",
                ));
            }
            bytes = &bytes[transferred..];
        }
        Ok(())
    }

    fn transfer(
        &mut self,
        write: Option<&[u8]>,
        read_len: usize,
        deadline: ControlDeadline,
        caller_cancellation: Option<&ControlCancellation>,
    ) -> io::Result<(usize, Vec<u8>)> {
        io_preflight(deadline, &self.shared_cancellation, caller_cancellation)?;
        let buffer = write.map_or_else(|| vec![0; read_len], |bytes| bytes.to_vec());
        let mut operation = PendingOperation::new(buffer)?;
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "pipe is terminal"))?;
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
                self.handle.take();
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            let (completed, returned_operation) = wait_pending_transfer(
                &mut self.handle,
                operation,
                deadline,
                &self.shared_cancellation,
                caller_cancellation,
                self.peer_process.raw(),
            )?;
            transferred = completed;
            operation = returned_operation;
        }
        if let Err(error) = io_preflight(deadline, &self.shared_cancellation, caller_cancellation) {
            self.handle.take();
            return Err(error);
        }
        Ok((transferred as usize, operation.buffer))
    }
}

fn wait_pending_transfer_cancellable(
    handle: &mut Option<OwnedHandle>,
    mut operation: Box<PendingOperation>,
    cancellation: &ControlCancellation,
) -> io::Result<(Dword, Box<PendingOperation>)> {
    loop {
        if cancellation.is_cancelled() {
            retire_slot(handle, operation);
            return Err(io::Error::new(io::ErrorKind::Interrupted, "pipe cancelled"));
        }
        // Completion wins over a simultaneous process-exit notification so
        // already-written H4CR bytes remain drainable as a valid prefix.
        if unsafe { WaitForSingleObject(operation.event.raw(), 0) } == WAIT_OBJECT_0 {
            let raw = handle.as_ref().expect("live pipe owns handle").raw();
            let mut transferred = 0;
            if unsafe {
                GetOverlappedResult(raw, &mut operation.overlapped, &mut transferred, 0)
            } == 0
            {
                let error = io::Error::last_os_error();
                handle.take();
                return Err(error);
            }
            return Ok((transferred, operation));
        }
        // Do not terminate a data read merely because the retained peer
        // process signalled exit. The named pipe remains the authority for
        // buffered bytes followed by EOF, which is how a valid crash prefix is
        // drained without accepting a different peer.
        match unsafe { WaitForSingleObject(operation.event.raw(), 20) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => continue,
            _ => {
                let error = io::Error::last_os_error();
                retire_slot(handle, operation);
                return Err(error);
            }
        }
    }
}

/// Kernel buffer the H4CR data pipe is created with.
///
/// The collector publishes a whole frame in one `WriteFileSync` and blocks
/// until this side has drained it, so the buffer sets how many
/// producer-blocks/consumer-drains a frame costs. t15 frames average 7.46 MiB
/// with single records up to 8.96 MiB; against the previous 64 KiB that was
/// well over a hundred handoffs per frame. One megabyte of nonpaged pool per
/// pipe is a rounding error next to the multi-gigabyte stream it carries.
///
/// This is a transport throughput constant only. It does not change which
/// records are produced, their contents, their order, or their framing.
const H4CR_PIPE_BUFFER_BYTES: usize = 1024 * 1024;

/// Kernel buffer for the H4CT control pipe, which carries small bounded
/// control frames rather than bulk records. Unchanged from the original
/// sizing; called out separately so raising the data pipe cannot silently
/// drag the control pipe along with it.
const H4CT_PIPE_BUFFER_BYTES: usize = 64 * 1024;

fn create_pipe(
    name: &str,
    kind: &'static str,
    access: Dword,
    security: RestrictiveSecurityPolicy,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<OwnedHandle, WindowsCombinedBackendError> {
    let (prefix, buffer_bytes) = match kind {
        "H4CR" => (
            r"\\.\pipe\battle-plan_collector_",
            H4CR_PIPE_BUFFER_BYTES,
        ),
        "H4CT" => (r"\\.\pipe\battle-plan_control_", H4CT_PIPE_BUFFER_BYTES),
        _ => return Err(WindowsCombinedBackendError::InvalidPipeName(kind)),
    };
    if !name.starts_with(prefix)
        || name.len() <= prefix.len()
        || name.contains('\0')
        || name[prefix.len()..].contains(['\\', '/'])
    {
        return Err(WindowsCombinedBackendError::InvalidPipeName(kind));
    }
    preflight(deadline, cancellation)?;
    let mut descriptor = SecurityDescriptor::restrictive(security)?;
    let mut attributes = descriptor.attributes();
    let wide_name = wide(name);
    let handle = unsafe {
        CreateNamedPipeW(
            wide_name.as_ptr(),
            access | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            buffer_bytes as Dword,
            buffer_bytes as Dword,
            0,
            &mut attributes,
        )
    };
    let handle = OwnedHandle::new(handle)?;
    preflight(deadline, cancellation)?;
    Ok(handle)
}

fn arm_connect(
    handle: OwnedHandle,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<ArmedPipe, WindowsCombinedBackendError> {
    preflight(deadline, cancellation)?;
    let mut operation = PendingOperation::new(Vec::new())?;
    let connected = unsafe { ConnectNamedPipe(handle.raw(), &mut operation.overlapped) };
    if connected != 0 {
        preflight(deadline, cancellation)?;
        return Ok(ArmedPipe {
            handle: Some(handle),
            pending: None,
        });
    }
    match unsafe { GetLastError() } {
        ERROR_PIPE_CONNECTED => {
            preflight(deadline, cancellation)?;
            Ok(ArmedPipe {
                handle: Some(handle),
                pending: None,
            })
        }
        ERROR_IO_PENDING => Ok(ArmedPipe {
            handle: Some(handle),
            pending: Some(operation),
        }),
        error => Err(io::Error::from_raw_os_error(error as i32).into()),
    }
}

fn finish_pending_connect(
    handle: OwnedHandle,
    mut operation: Box<PendingOperation>,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
    target: Handle,
) -> Result<OwnedHandle, WindowsCombinedBackendError> {
    loop {
        if cancellation.is_cancelled() {
            retire_or_quarantine(handle, operation);
            return Err(WindowsCombinedBackendError::Cancelled);
        }
        if deadline.is_expired() {
            retire_or_quarantine(handle, operation);
            return Err(WindowsCombinedBackendError::DeadlineExpired);
        }
        match unsafe { WaitForSingleObject(target, 0) } {
            WAIT_TIMEOUT => {}
            WAIT_OBJECT_0 => {
                cancellation.cancel();
                retire_or_quarantine(handle, operation);
                return Err(WindowsCombinedBackendError::TargetExited);
            }
            _ => {
                cancellation.cancel();
                retire_or_quarantine(handle, operation);
                return Err(io::Error::last_os_error().into());
            }
        }
        let remaining = match deadline.remaining() {
            Some(remaining) if !remaining.is_zero() => remaining,
            _ => {
                retire_or_quarantine(handle, operation);
                return Err(WindowsCombinedBackendError::DeadlineExpired);
            }
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
                    return Err(io::Error::last_os_error().into());
                }
                preflight(deadline, cancellation)?;
                return Ok(handle);
            }
            WAIT_TIMEOUT => {}
            _ => {
                let error = io::Error::last_os_error();
                retire_or_quarantine(handle, operation);
                return Err(error.into());
            }
        }
    }
}

fn wait_pending_transfer(
    handle: &mut Option<OwnedHandle>,
    mut operation: Box<PendingOperation>,
    deadline: ControlDeadline,
    shared_cancellation: &ControlCancellation,
    caller_cancellation: Option<&ControlCancellation>,
    target: Handle,
) -> io::Result<(Dword, Box<PendingOperation>)> {
    loop {
        if let Err(error) = io_preflight(deadline, shared_cancellation, caller_cancellation) {
            retire_slot(handle, operation);
            return Err(error);
        }
        match unsafe { WaitForSingleObject(target, 0) } {
            WAIT_TIMEOUT => {}
            WAIT_OBJECT_0 => {
                shared_cancellation.cancel();
                retire_slot(handle, operation);
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "target exited"));
            }
            _ => {
                shared_cancellation.cancel();
                retire_slot(handle, operation);
                return Err(io::Error::last_os_error());
            }
        }
        let remaining = match deadline.remaining() {
            Some(remaining) if !remaining.is_zero() => remaining,
            _ => {
                retire_slot(handle, operation);
                return Err(io::Error::new(io::ErrorKind::TimedOut, "pipe deadline"));
            }
        };
        let wait_ms = duration_to_wait_ms(remaining.min(Duration::from_millis(20)));
        match unsafe { WaitForSingleObject(operation.event.raw(), wait_ms) } {
            WAIT_OBJECT_0 => {
                let raw = handle.as_ref().expect("live pipe owns handle").raw();
                let mut transferred = 0;
                if unsafe {
                    GetOverlappedResult(raw, &mut operation.overlapped, &mut transferred, 0)
                } == 0
                {
                    let error = io::Error::last_os_error();
                    handle.take();
                    return Err(error);
                }
                if let Err(error) = io_preflight(deadline, shared_cancellation, caller_cancellation)
                {
                    handle.take();
                    return Err(error);
                }
                return Ok((transferred, operation));
            }
            WAIT_TIMEOUT => {}
            _ => {
                let error = io::Error::last_os_error();
                retire_slot(handle, operation);
                return Err(error);
            }
        }
    }
}

fn retire_slot(handle: &mut Option<OwnedHandle>, operation: Box<PendingOperation>) {
    if let Some(handle) = handle.take() {
        retire_or_quarantine(handle, operation);
    }
}

fn retire_or_quarantine(handle: OwnedHandle, mut operation: Box<PendingOperation>) {
    unsafe {
        CancelIoEx(handle.raw(), &mut operation.overlapped);
    }
    if unsafe { WaitForSingleObject(operation.event.raw(), 0) } == WAIT_OBJECT_0 {
        let mut ignored = 0;
        unsafe {
            GetOverlappedResult(handle.raw(), &mut operation.overlapped, &mut ignored, 0);
        }
    } else {
        Box::leak(Box::new(QuarantinedOperation {
            _pipe: handle,
            _operation: operation,
        }));
    }
}

fn validate_pipe_peer(
    kind: &'static str,
    pipe: &OwnedHandle,
    expected: ControlPeerIdentity,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<OwnedHandle, WindowsCombinedBackendError> {
    preflight(deadline, cancellation)?;
    let mut pid = 0;
    if unsafe { GetNamedPipeClientProcessId(pipe.raw(), &mut pid) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let process = open_process_identity(pid)?;
    preflight(deadline, cancellation)?;
    let observed = ControlPeerIdentity {
        pid,
        creation_filetime: process_creation_filetime(&process)?,
    };
    if observed != expected {
        return Err(WindowsCombinedBackendError::PeerMismatch {
            pipe: kind,
            expected,
            observed,
        });
    }
    preflight(deadline, cancellation)?;
    Ok(process)
}

fn open_process_identity(pid: u32) -> Result<OwnedHandle, WindowsCombinedBackendError> {
    if pid == 0 {
        return Err(WindowsCombinedBackendError::Control(
            ControlError::InvalidPeerIdentity,
        ));
    }
    Ok(OwnedHandle::new(unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid)
    })?)
}

fn process_creation_filetime(process: &OwnedHandle) -> io::Result<u64> {
    let (mut creation, mut exit, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
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
        Err(io::Error::last_os_error())
    } else {
        Ok(creation)
    }
}

fn preflight(
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> Result<(), WindowsCombinedBackendError> {
    if cancellation.is_cancelled() {
        Err(WindowsCombinedBackendError::Cancelled)
    } else if deadline.is_expired() {
        Err(WindowsCombinedBackendError::DeadlineExpired)
    } else {
        Ok(())
    }
}

fn io_preflight(
    deadline: ControlDeadline,
    shared_cancellation: &ControlCancellation,
    caller_cancellation: Option<&ControlCancellation>,
) -> io::Result<()> {
    if shared_cancellation.is_cancelled()
        || caller_cancellation.is_some_and(ControlCancellation::is_cancelled)
    {
        Err(io::Error::new(io::ErrorKind::Interrupted, "pipe cancelled"))
    } else if deadline.is_expired() {
        Err(io::Error::new(io::ErrorKind::TimedOut, "pipe deadline"))
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
