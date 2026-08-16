//! Single shipped recording-open policy.
//!
//! Every product surface enters through the explicit bounded decoder
//! capability. Snapshot-native recordings pass through without invoking it;
//! compact-only containers are authenticated into the product-owned spool and
//! folded by the extractor's one production `WorldFold`. There is deliberately
//! no ordinary-open or raw-stream fallback in this module.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use replay_core::{
    CertifiedPrefoldSpoolDecoder, CertifiedPrefoldSpoolRequest, ContentManifest,
    PackagedDecisionCatalog, PackagedG2EngineCapture, PackagedStaticArtifact,
    PortableDesignCatalog, PrefoldDecodeError, PrefoldSourceReceipt, PrefoldStreamIdentityMode,
    PrefoldedSpool, PrefoldedSpoolEntry, RawFrame, RecordedCampaignIdentity,
    RecorderProductAdmission, ReplaySource, SourceError,
};

#[cfg(all(windows, not(test)))]
#[path = "windows_spool_storage.rs"]
mod windows_spool_storage;

static PRODUCTION_DECODER: ProductionPrefoldWorker = ProductionPrefoldWorker;

const PREFOLD_WORKER_DECODER_ID: &str = crate::prefold_worker::DECODER_ID;
const PREFOLD_WORKER_WALL_LIMIT: Duration = Duration::from_secs(15 * 60);
const PREFOLD_WORKER_STDOUT_LIMIT: usize = 32 * 1024 * 1024;
const PREFOLD_WORKER_STDERR_LIMIT: usize = 1024 * 1024;

struct BoundedWorkerOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn read_child_pipe_bounded<R: Read>(
    mut reader: R,
    maximum_bytes: usize,
    overflow: Arc<AtomicBool>,
) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::with_capacity(maximum_bytes.min(64 * 1024));
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().checked_add(read).is_none_or(|len| len > maximum_bytes) {
            overflow.store(true, Ordering::Release);
            return Ok(output);
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

fn collect_prefold_worker_output(
    mut child: Child,
) -> Result<BoundedWorkerOutput, PrefoldDecodeError> {
    let stdout = child.stdout.take().ok_or_else(|| {
        PrefoldDecodeError::new("production prefold worker stdout was not piped")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        PrefoldDecodeError::new("production prefold worker stderr was not piped")
    })?;
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_overflow = Arc::clone(&overflow);
    let stderr_overflow = Arc::clone(&overflow);
    let stdout_reader = std::thread::spawn(move || {
        read_child_pipe_bounded(stdout, PREFOLD_WORKER_STDOUT_LIMIT, stdout_overflow)
    });
    let stderr_reader = std::thread::spawn(move || {
        read_child_pipe_bounded(stderr, PREFOLD_WORKER_STDERR_LIMIT, stderr_overflow)
    });

    let deadline = Instant::now() + PREFOLD_WORKER_WALL_LIMIT;
    let (status, termination_error) = loop {
        if overflow.load(Ordering::Acquire) {
            let _ = child.kill();
            break (
                child.wait().ok(),
                Some("production prefold worker exceeded its bounded stdout/stderr capture"),
            );
        }
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), None),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                break (
                    child.wait().ok(),
                    Some("production prefold worker exceeded the 15 minute wall-clock limit"),
                );
            }
            Err(_) => {
                let _ = child.kill();
                break (
                    child.wait().ok(),
                    Some("production prefold worker status polling failed"),
                );
            }
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| PrefoldDecodeError::new("production prefold stdout reader panicked"))?
        .map_err(|error| {
            PrefoldDecodeError::new(format!("read production prefold stdout: {error}"))
        })?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| PrefoldDecodeError::new("production prefold stderr reader panicked"))?
        .map_err(|error| {
            PrefoldDecodeError::new(format!("read production prefold stderr: {error}"))
        })?;
    if let Some(error) = termination_error {
        return Err(PrefoldDecodeError::new(error));
    }
    Ok(BoundedWorkerOutput {
        status: status.ok_or_else(|| {
            PrefoldDecodeError::new("production prefold worker exited without a status")
        })?,
        stdout,
        stderr,
    })
}

#[cfg(windows)]
mod prefold_worker_job {
    use std::ffi::c_void;
    use std::mem;
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use std::process::Child;

    type Handle = *mut c_void;
    const EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const LIMIT_PROCESS_TIME: u32 = 0x0000_0002;
    const LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
    const LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    const LIMIT_KILL_ON_CLOSE: u32 = 0x0000_2000;
    const PROCESS_MEMORY_LIMIT: usize = 2 * 1024 * 1024 * 1024;
    const PROCESS_CPU_LIMIT_100NS: i64 = 15 * 60 * 10_000_000;

    #[repr(C)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct ExtendedLimitInformation {
        basic_limit_information: BasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            information_class: i32,
            information: *const c_void,
            information_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    pub(super) struct PrefoldWorkerJob(Handle);

    impl PrefoldWorkerJob {
        pub(super) fn create() -> Result<Self, String> {
            let handle = unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) };
            if handle.is_null() {
                return Err(format!("create prefold resource job: {}", std::io::Error::last_os_error()));
            }
            let mut information: ExtendedLimitInformation = unsafe { mem::zeroed() };
            information.basic_limit_information.per_process_user_time_limit =
                PROCESS_CPU_LIMIT_100NS;
            information.basic_limit_information.active_process_limit = 1;
            information.basic_limit_information.limit_flags = LIMIT_PROCESS_TIME
                | LIMIT_ACTIVE_PROCESS
                | LIMIT_PROCESS_MEMORY
                | LIMIT_KILL_ON_CLOSE;
            information.process_memory_limit = PROCESS_MEMORY_LIMIT;
            if unsafe {
                SetInformationJobObject(
                    handle,
                    EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&information as *const ExtendedLimitInformation).cast(),
                    mem::size_of::<ExtendedLimitInformation>() as u32,
                )
            } == 0
            {
                let error = std::io::Error::last_os_error();
                unsafe { CloseHandle(handle) };
                return Err(format!("configure prefold resource job: {error}"));
            }
            Ok(Self(handle))
        }

        pub(super) fn assign(&self, child: &Child) -> Result<(), String> {
            if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle().cast()) } == 0 {
                return Err(format!("assign prefold worker resource job: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        }
    }

    impl Drop for PrefoldWorkerJob {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn prefold_identity_mode_label(mode: PrefoldStreamIdentityMode) -> &'static str {
    match mode {
        PrefoldStreamIdentityMode::Legacy => "legacy",
        PrefoldStreamIdentityMode::IdentityV1 => "identity-v1",
    }
}

/// Process boundary for the extractor-owned production WorldFold. The worker
/// is the exact recorder sibling already present in the offline two-executable
/// package; this preserves fault isolation without linking extractor crates or
/// adding a third shipped executable.
struct ProductionPrefoldWorker;

impl CertifiedPrefoldSpoolDecoder for ProductionPrefoldWorker {
    fn decoder_id(&self) -> &'static str {
        PREFOLD_WORKER_DECODER_ID
    }

    fn decode_certified_spool(
        &self,
        request: CertifiedPrefoldSpoolRequest,
    ) -> Result<PrefoldedSpool, PrefoldDecodeError> {
        let identity_mode = request.identity_mode();
        let (game_id, raw_path, snapshot_dir, source_len, source_sha256) = request.into_parts();
        let worker = AuthenticatedSelfExecutable::open_prefold_worker()?;
        let source = AuthenticatedPrefoldSource::open(&raw_path, source_len, &source_sha256)?;
        #[cfg(windows)]
        let resource_job = prefold_worker_job::PrefoldWorkerJob::create()
            .map_err(PrefoldDecodeError::new)?;
        let mut child = Command::new(worker.path())
            .arg(crate::prefold_worker::INTERNAL_VERB)
            .arg("--game-id")
            .arg(&game_id)
            .arg("--source")
            .arg(&raw_path)
            .arg("--snapshot-dir")
            .arg(&snapshot_dir)
            .arg("--source-len")
            .arg(source_len.to_string())
            .arg("--source-sha256")
            .arg(&source_sha256)
            .arg("--identity-mode")
            .arg(prefold_identity_mode_label(identity_mode))
            .arg("--worker-executable-sha256")
            .arg(worker.sha256())
            // Stdin is a duplicate of the already authenticated retained file
            // object. The child never reopens `--source`; the path remains in
            // the portable request solely for diagnostics and contract
            // compatibility.
            .stdin(source.child_stdio()?)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                PrefoldDecodeError::new(format!(
                    "launch authenticated recorder prefold child {}: {error}",
                    worker.path().display()
                ))
            })?;
        #[cfg(windows)]
        if let Err(error) = resource_job.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PrefoldDecodeError::new(error));
        }
        let output = collect_prefold_worker_output(child)?;
        worker.verify_unchanged()?;
        if !output.status.success() {
            return Err(PrefoldDecodeError::new(format!(
                "production prefold worker exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|error| {
            PrefoldDecodeError::new(format!(
                "production prefold worker returned malformed JSON: {error}"
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            PrefoldDecodeError::new("production prefold worker receipt is not an object")
        })?;
        if !worker_receipt_identity_matches(object, &game_id, identity_mode, worker.sha256()) {
            return Err(PrefoldDecodeError::new(
                "production prefold worker receipt identity mismatch",
            ));
        }
        let schema_version = object.get("schema_version").cloned().ok_or_else(|| {
            PrefoldDecodeError::new("production prefold worker omitted schema_version")
        })?;
        let rows = object
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                PrefoldDecodeError::new("production prefold worker omitted entries array")
            })?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let row = row.as_object().ok_or_else(|| {
                PrefoldDecodeError::new("production prefold worker entry is not an object")
            })?;
            let hour_index = row
                .get("hour_index")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted hour_index"))?;
            let date = row
                .get("date")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted date"))?;
            let file_name = row
                .get("file_name")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted file_name"))?;
            let byte_len = row
                .get("byte_len")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted byte_len"))?;
            let sha256 = row
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| PrefoldDecodeError::new("worker entry omitted sha256"))?;
            entries.push(PrefoldedSpoolEntry::try_new(
                hour_index, date, file_name, byte_len, sha256,
            )?);
        }
        PrefoldedSpool::try_new(game_id, schema_version, entries)
    }
}

/// Retained compact-source capability used across the self-child boundary.
///
/// On Windows the open denies both write and delete sharing and requests the
/// final component itself rather than following a reparse point. Length and
/// SHA-256 are checked through this same handle before it is duplicated into
/// child stdin. Holding `file` until the child exits keeps the authenticated
/// object immutable for the complete fold.
struct AuthenticatedPrefoldSource {
    file: File,
}

impl AuthenticatedPrefoldSource {
    fn open(
        path: &Path,
        expected_len: u64,
        expected_sha256: &str,
    ) -> Result<Self, PrefoldDecodeError> {
        let mut file = open_validated_handle_with(
            path,
            "compact prefold source",
            OpenedObjectKind::Image,
            open_locked_prefold_source,
        )?;
        let observed_len = file
            .metadata()
            .map_err(|error| {
                PrefoldDecodeError::new(format!(
                    "inspect compact prefold source {}: {error}",
                    path.display()
                ))
            })?
            .len();
        if observed_len != expected_len {
            return Err(PrefoldDecodeError::new(format!(
                "compact prefold source length {observed_len} != authenticated length {expected_len}: {}",
                path.display()
            )));
        }
        let observed_sha256 = hash_open_file(&mut file, path, "compact prefold source")?;
        if observed_sha256 != expected_sha256 {
            return Err(PrefoldDecodeError::new(format!(
                "compact prefold source SHA-256 {observed_sha256} != authenticated SHA-256 {expected_sha256}: {}",
                path.display()
            )));
        }
        Ok(Self { file })
    }

    fn child_stdio(&self) -> Result<Stdio, PrefoldDecodeError> {
        self.file.try_clone().map(Stdio::from).map_err(|error| {
            PrefoldDecodeError::new(format!(
                "duplicate authenticated compact source for prefold child: {error}"
            ))
        })
    }

    #[cfg(test)]
    fn read_clone(&self) -> Result<Vec<u8>, PrefoldDecodeError> {
        let mut file = self.file.try_clone().map_err(|error| {
            PrefoldDecodeError::new(format!("duplicate compact-source test handle: {error}"))
        })?;
        file.seek(SeekFrom::Start(0)).map_err(|error| {
            PrefoldDecodeError::new(format!("rewind compact-source test handle: {error}"))
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|error| {
            PrefoldDecodeError::new(format!("read compact-source test handle: {error}"))
        })?;
        Ok(bytes)
    }
}

fn worker_receipt_identity_matches(
    object: &serde_json::Map<String, serde_json::Value>,
    game_id: &str,
    identity_mode: PrefoldStreamIdentityMode,
    executable_sha256: &str,
) -> bool {
    object.get("schema").and_then(serde_json::Value::as_str)
        == Some(crate::prefold_worker::RECEIPT_SCHEMA)
        && object.get("decoder_id").and_then(serde_json::Value::as_str)
            == Some(PREFOLD_WORKER_DECODER_ID)
        && object
            .get("worker_executable_sha256")
            .and_then(serde_json::Value::as_str)
            == Some(executable_sha256)
        && object.get("game_id").and_then(serde_json::Value::as_str) == Some(game_id)
        && object
            .get("identity_mode")
            .and_then(serde_json::Value::as_str)
            == Some(prefold_identity_mode_label(identity_mode))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExecutableFileIdentity {
    volume: u64,
    file: u64,
}

struct AuthenticatedSelfExecutable {
    path: PathBuf,
    sha256: String,
    identity: ExecutableFileIdentity,
    // Holding this file with no write/delete sharing prevents replacement of
    // the image between validation and CreateProcess on shipped Windows.
    _image: File,
    // Retain every directory from the volume root through the image parent
    // without delete sharing.  Protecting only the immediate parent leaves a
    // higher ancestor available for rename/junction rebinding before
    // CreateProcess resolves the path again.
    _ancestors: Vec<File>,
}

/// Child-side retained self-image capability. Keeping this value alive holds
/// the authenticated executable and all of its pathname ancestors through the
/// complete fold instead of returning a hash after dropping those guards.
pub(crate) struct AuthenticatedCurrentExecutable(AuthenticatedSelfExecutable);

impl AuthenticatedCurrentExecutable {
    pub(crate) fn open() -> Result<Self, PrefoldDecodeError> {
        AuthenticatedSelfExecutable::open().map(Self)
    }

    pub(crate) fn sha256(&self) -> &str {
        self.0.sha256()
    }

    pub(crate) fn verify_unchanged(&self) -> Result<(), PrefoldDecodeError> {
        self.0.verify_unchanged()
    }
}

impl AuthenticatedSelfExecutable {
    fn open() -> Result<Self, PrefoldDecodeError> {
        let path = std::env::current_exe().map_err(|error| {
            PrefoldDecodeError::new(format!("resolve replay_app executable path: {error}"))
        })?;
        Self::open_path(path, "replay_app executable")
    }

    fn open_prefold_worker() -> Result<Self, PrefoldDecodeError> {
        let replay_path = std::env::current_exe().map_err(|error| {
            PrefoldDecodeError::new(format!("resolve replay_app executable path: {error}"))
        })?;
        let parent = replay_path.parent().ok_or_else(|| {
            PrefoldDecodeError::new("replay_app executable has no parent directory")
        })?;
        let path = parent.join(crate::prefold_worker::WORKER_EXE_NAME);
        Self::open_path(path, "recorder worker executable")
    }

    fn open_path(path: PathBuf, label: &str) -> Result<Self, PrefoldDecodeError> {
        if !path.is_absolute() {
            return Err(PrefoldDecodeError::new(
                format!("{label} path is not absolute"),
            ));
        }
        let parent = path.parent().ok_or_else(|| {
            PrefoldDecodeError::new(format!("{label} has no parent directory"))
        })?;
        let ancestors = retain_executable_ancestors(parent)?;
        let mut image = open_validated_image(&path, label)?;
        let identity = executable_identity(&image, &path)?;
        let sha256 = hash_open_image(&mut image, &path)?;
        Ok(Self {
            path,
            sha256,
            identity,
            _image: image,
            _ancestors: ancestors,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn sha256(&self) -> &str {
        &self.sha256
    }

    fn verify_unchanged(&self) -> Result<(), PrefoldDecodeError> {
        let mut reopened = open_validated_image(&self.path, "post-launch recorder worker executable")?;
        let identity = executable_identity(&reopened, &self.path)?;
        let sha256 = hash_open_image(&mut reopened, &self.path)?;
        if identity != self.identity || sha256 != self.sha256 {
            return Err(PrefoldDecodeError::new(
                "recorder prefold child executable changed across process execution",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum OpenedObjectKind {
    Image,
    Directory,
}

fn retain_executable_ancestors(parent: &Path) -> Result<Vec<File>, PrefoldDecodeError> {
    let mut paths = parent
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty())
        .collect::<Vec<_>>();
    paths.reverse();

    let mut handles = Vec::with_capacity(paths.len());
    for path in paths {
        handles.push(open_validated_handle_with(
            path,
            "recorder worker executable ancestor",
            OpenedObjectKind::Directory,
            open_locked_directory,
        )?);
    }
    Ok(handles)
}

fn open_validated_image(path: &Path, label: &str) -> Result<File, PrefoldDecodeError> {
    open_validated_handle_with(path, label, OpenedObjectKind::Image, open_locked_image)
}

fn open_validated_handle_with(
    path: &Path,
    label: &str,
    expected: OpenedObjectKind,
    opener: impl FnOnce(&Path) -> Result<File, PrefoldDecodeError>,
) -> Result<File, PrefoldDecodeError> {
    // This pathname check gives a useful early diagnostic, but is deliberately
    // not the security decision: the path can change before the open.
    reject_alias(path, label)?;
    let file = opener(path)?;
    validate_opened_handle(&file, path, label, expected)?;
    Ok(file)
}

fn validate_opened_handle(
    file: &File,
    path: &Path,
    label: &str,
    expected: OpenedObjectKind,
) -> Result<(), PrefoldDecodeError> {
    // File::metadata queries the opened object (fstat/GetFileInformationByHandle)
    // rather than resolving the pathname a second time.
    let metadata = file.metadata().map_err(|error| {
        PrefoldDecodeError::new(format!(
            "inspect opened {label} handle {}: {error}",
            path.display()
        ))
    })?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(PrefoldDecodeError::new(format!(
                "opened {label} handle is a reparse point: {}",
                path.display()
            )));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(PrefoldDecodeError::new(format!(
            "opened {label} handle is a symlink: {}",
            path.display()
        )));
    }
    let type_matches = match expected {
        OpenedObjectKind::Image => metadata.is_file(),
        OpenedObjectKind::Directory => metadata.is_dir(),
    };
    if !type_matches {
        let expected = match expected {
            OpenedObjectKind::Image => "regular file",
            OpenedObjectKind::Directory => "directory",
        };
        return Err(PrefoldDecodeError::new(format!(
            "opened {label} handle is not a {expected}: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_alias(path: &Path, label: &str) -> Result<(), PrefoldDecodeError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        PrefoldDecodeError::new(format!("inspect {label} {}: {error}", path.display()))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(PrefoldDecodeError::new(format!(
            "{label} is a symlink/reparse alias: {}",
            path.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(PrefoldDecodeError::new(format!(
                "{label} is a reparse alias: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_locked_image(path: &Path) -> Result<File, PrefoldDecodeError> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path).map_err(|error| {
        PrefoldDecodeError::new(format!(
            "retain recorder worker executable without write/delete sharing {}: {error}",
            path.display()
        ))
    })
}

#[cfg(not(windows))]
fn open_locked_image(path: &Path) -> Result<File, PrefoldDecodeError> {
    OpenOptions::new().read(true).open(path).map_err(|error| {
        PrefoldDecodeError::new(format!(
            "open replay_app executable {}: {error}",
            path.display()
        ))
    })
}

#[cfg(windows)]
fn open_locked_prefold_source(path: &Path) -> Result<File, PrefoldDecodeError> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let mut options = OpenOptions::new();
    options
        .read(true)
        // No FILE_SHARE_WRITE and no FILE_SHARE_DELETE: after admission the
        // exact object cannot be modified or path-substituted until the fold
        // child and its receipt are complete.
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    options.open(path).map_err(|error| {
        PrefoldDecodeError::new(format!(
            "retain compact prefold source without write/delete sharing {}: {error}",
            path.display()
        ))
    })
}

#[cfg(not(windows))]
fn open_locked_prefold_source(path: &Path) -> Result<File, PrefoldDecodeError> {
    OpenOptions::new().read(true).open(path).map_err(|error| {
        PrefoldDecodeError::new(format!(
            "open compact prefold source {}: {error}",
            path.display()
        ))
    })
}

#[cfg(windows)]
fn open_locked_directory(path: &Path) -> Result<File, PrefoldDecodeError> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;

    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const INVALID_HANDLE_VALUE: *mut c_void = -1isize as *mut c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut c_void,
        ) -> *mut c_void;
    }

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(PrefoldDecodeError::new(format!(
            "retain replay_app directory without delete sharing {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(not(windows))]
fn open_locked_directory(path: &Path) -> Result<File, PrefoldDecodeError> {
    File::open(path).map_err(|error| {
        PrefoldDecodeError::new(format!(
            "open replay_app directory {}: {error}",
            path.display()
        ))
    })
}

fn hash_open_image(file: &mut File, path: &Path) -> Result<String, PrefoldDecodeError> {
    hash_open_file(file, path, "replay_app executable")
}

fn hash_open_file(file: &mut File, path: &Path, label: &str) -> Result<String, PrefoldDecodeError> {
    use sha2::{Digest, Sha256};

    file.seek(SeekFrom::Start(0)).map_err(|error| {
        PrefoldDecodeError::new(format!("seek {label} {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            PrefoldDecodeError::new(format!("hash {label} {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        PrefoldDecodeError::new(format!("rewind {label} {}: {error}", path.display()))
    })?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(windows)]
fn executable_identity(
    file: &File,
    path: &Path,
) -> Result<ExecutableFileIdentity, PrefoldDecodeError> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        _number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            handle: *mut c_void,
            info: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut info = MaybeUninit::<ByHandleFileInformation>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
        return Err(PrefoldDecodeError::new(format!(
            "query replay_app executable identity {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    let info = unsafe { info.assume_init() };
    // A second hard-link name does not weaken the retained image capability:
    // Windows applies this handle's denied write/delete sharing to the file
    // object, not merely to the pathname used to open it. Cargo legitimately
    // hard-links integration-test binaries, so link count is not identity.
    // The volume/file index and whole-file hash below remain the admission and
    // post-execution identity join.
    Ok(ExecutableFileIdentity {
        volume: u64::from(info.volume_serial_number),
        file: (u64::from(info.file_index_high) << 32) | u64::from(info.file_index_low),
    })
}

#[cfg(unix)]
fn executable_identity(
    file: &File,
    path: &Path,
) -> Result<ExecutableFileIdentity, PrefoldDecodeError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|error| {
        PrefoldDecodeError::new(format!(
            "query replay_app executable identity {}: {error}",
            path.display()
        ))
    })?;
    // Hard-link count is not file identity. Keep the opened descriptor alive
    // and join the device/inode plus whole-file hash before and after execution.
    Ok(ExecutableFileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn executable_identity(
    file: &File,
    path: &Path,
) -> Result<ExecutableFileIdentity, PrefoldDecodeError> {
    let metadata = file.metadata().map_err(|error| {
        PrefoldDecodeError::new(format!(
            "query replay_app executable identity {}: {error}",
            path.display()
        ))
    })?;
    Ok(ExecutableFileIdentity {
        volume: 0,
        file: metadata.len(),
    })
}

/// Open one recording under the shipped activation policy.
pub(crate) fn open_product_recording(path: &Path) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_product_recording_segment(path, None)
}

/// Open either one ordinary portable recording or one explicitly selected
/// member of an ordered CID1 rollover package set.
pub(crate) fn open_product_recording_segment(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    Ok(open_product_recording_segment_impl(path, logical_stream_id, false)?.source)
}

pub(crate) struct ProductRecordingOpen {
    pub source: Box<dyn ReplaySource>,
    /// Whole-file SHA-256 from the exact container handle moved into `source`.
    /// Directory stores have no single-file authority and return `None`.
    pub authority_sha256: Option<String>,
}

pub(crate) fn open_product_recording_segment_with_authority(
    path: &Path,
    logical_stream_id: Option<u64>,
) -> Result<ProductRecordingOpen, SourceError> {
    open_product_recording_segment_impl(path, logical_stream_id, true)
}

fn open_product_recording_segment_impl(
    path: &Path,
    logical_stream_id: Option<u64>,
    require_whole_file_receipt: bool,
) -> Result<ProductRecordingOpen, SourceError> {
    // Acquire before any is_file/is_dir probe. Windows rejects UNC lexically,
    // opens every component no-follow, and denies delete sharing for the
    // complete source lifetime.
    let recording_guard = ProductRecordingPathGuard::acquire(path)?;
    // Resolve the stable machine-local root before authenticating a package-set
    // member, so the normal container open follows the retained-handle join
    // immediately with no intervening filesystem setup.
    let root = product_spool_root()?;
    let is_rollover = path.file_name().and_then(|value| value.to_str())
        == Some(replay_core::ROLLOVER_PACKAGE_SET_FILE)
        || (path.is_dir() && path.join(replay_core::ROLLOVER_PACKAGE_SET_FILE).is_file());
    let (source, authority_sha256) = if is_rollover {
        let selected = replay_core::authenticate_rollover_segment(path, logical_stream_id)
            .map_err(map_rollover_authentication_error)?;
        let sha256 = selected.package_sha256().to_string();
        let source = replay_core::open_container_with_spooled_decoder(
            selected.into_container(),
            root.path(),
            &PRODUCTION_DECODER,
        )?;
        (source, Some(sha256))
    } else {
        if let Some(selected) = logical_stream_id {
            return Err(SourceError::Unsupported(format!(
                "--logical-stream {selected} requires a {} descriptor or a directory containing one",
                replay_core::ROLLOVER_PACKAGE_SET_FILE
            )));
        }
        if path.is_file() {
            let (container, sha256) = if require_whole_file_receipt {
                let (container, _bytes, sha256) =
                    snapshot::Container::open_strict_with_file_receipt(
                        path,
                        &snapshot::ExpectedIdentity::default(),
                    )
                    .map_err(map_retained_container_error)?;
                (container, Some(sha256))
            } else {
                let container =
                    snapshot::Container::open_strict(path, &snapshot::ExpectedIdentity::default())
                        .map_err(map_retained_container_error)?;
                (container, None)
            };
            let source = replay_core::open_container_with_spooled_decoder(
                container,
                root.path(),
                &PRODUCTION_DECODER,
            )?;
            (source, sha256)
        } else {
            (
                replay_core::open_recording_with_spooled_decoder(
                    path,
                    root.path(),
                    &PRODUCTION_DECODER,
                )?,
                None,
            )
        }
    };
    Ok(ProductRecordingOpen {
        source: Box::new(ProductReplaySource {
            inner: source,
            _spool_root: root,
            _recording_guard: recording_guard,
        }),
        authority_sha256,
    })
}

/// Preserve the typed rejection class of a rollover-member authentication
/// failure at the product boundary. A tampered or substituted member is an
/// INTEGRITY failure ("recording failed payload integrity validation"), a
/// malformed set is container/schema, and only a genuine capability refusal
/// (missing `--logical-stream`, recovery-only set, direct recording, or a
/// package-set schema this build does not know) reports as "not playable by
/// this build" — symmetric with the single-file branch's
/// `map_retained_container_error`.
fn map_rollover_authentication_error(
    error: replay_core::RolloverAuthenticationError,
) -> SourceError {
    let message = format!("authenticate rollover package-set member: {error}");
    match error {
        replay_core::RolloverAuthenticationError::Integrity(_) => SourceError::Integrity(message),
        replay_core::RolloverAuthenticationError::ModsetMismatch(_) => {
            SourceError::ModsetMismatch(message)
        }
        replay_core::RolloverAuthenticationError::Structure(_) => SourceError::Open(message),
        replay_core::RolloverAuthenticationError::Unsupported(_) => {
            SourceError::Unsupported(message)
        }
    }
}

fn map_retained_container_error(error: snapshot::ContainerError) -> SourceError {
    let message = format!("open retained replay container: {error}");
    // The retained-handle product path opens the snapshot container directly,
    // bypassing replay_core's Store adapter. Preserve BOTH typed signals the
    // adapter preserves — modset mismatch and payload integrity — so this route
    // classifies identically to the ordinary one.
    match error {
        snapshot::ContainerError::ModsetMismatch { .. } => SourceError::ModsetMismatch(message),
        snapshot::ContainerError::PayloadIntegrity(_) => SourceError::Integrity(message),
        snapshot::ContainerError::Zip(
            snapshot::zip::ZipError::PayloadIntegrity(_)
            | snapshot::zip::ZipError::SizeMismatch { .. }
            | snapshot::zip::ZipError::Decompress(_),
        ) => SourceError::Integrity(message),
        _ => SourceError::Open(message),
    }
}

/// Test-only seam: identical to the product open except the spool root and
/// certified decoder are supplied by the test fixture. `#[cfg(test)]` keeps it
/// out of the product binary — the module doc's "no ordinary-open or
/// raw-stream fallback" claim stays enforced by the compiler, not by
/// convention.
#[cfg(test)]
fn open_product_recording_at(
    path: &Path,
    spool_root: ProductSpoolRoot,
    decoder: &dyn CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let recording_guard = ProductRecordingPathGuard::acquire(path)?;
    let source =
        replay_core::open_recording_with_spooled_decoder(path, spool_root.path(), decoder)?;
    Ok(Box::new(ProductReplaySource {
        inner: source,
        _spool_root: spool_root,
        _recording_guard: recording_guard,
    }))
}

struct ProductRecordingPathGuard {
    #[cfg(windows)]
    _handles: Vec<File>,
}

fn map_recording_path_error(context: String, error: std::io::Error) -> SourceError {
    let message = format!("{context}: {error}");
    if error.kind() == std::io::ErrorKind::NotFound {
        SourceError::NotFound(message)
    } else {
        SourceError::Open(message)
    }
}

impl ProductRecordingPathGuard {
    fn acquire(path: &Path) -> Result<Self, SourceError> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            use std::path::{Component, Prefix};

            const FILE_SHARE_READ: u32 = 0x0000_0001;
            const FILE_SHARE_WRITE: u32 = 0x0000_0002;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

            let unresolved = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|error| SourceError::Open(format!("resolve replay path: {error}")))?
                    .join(path)
            };
            let mut components = unresolved.components();
            let prefix = match components.next() {
                Some(Component::Prefix(prefix))
                    if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
                {
                    prefix.as_os_str().to_os_string()
                }
                _ => {
                    return Err(SourceError::Open(
                        "replay input must be on a local drive; UNC, device, and network paths are refused"
                            .to_string(),
                    ))
                }
            };
            if !matches!(components.next(), Some(Component::RootDir)) {
                return Err(SourceError::Open(
                    "replay input path is not a canonical rooted local-drive path".to_string(),
                ));
            }
            let mut normalized = PathBuf::from(&prefix);
            normalized.push("\\");
            let mut depth = 0usize;
            for component in components {
                match component {
                    Component::Normal(name) => {
                        normalized.push(name);
                        depth += 1;
                    }
                    Component::CurDir => {}
                    Component::ParentDir if depth > 0 => {
                        normalized.pop();
                        depth -= 1;
                    }
                    Component::ParentDir => {
                        return Err(SourceError::Open(
                            "replay input path escapes its local-drive root".to_string(),
                        ));
                    }
                    Component::Prefix(_) | Component::RootDir => {
                        return Err(SourceError::Open(
                            "replay input path contains an invalid rooted component".to_string(),
                        ));
                    }
                }
            }
            let mut normalized_components = normalized.components();
            normalized_components.next();
            normalized_components.next();
            let normals = normalized_components.collect::<Vec<_>>();
            if normals.is_empty() {
                return Err(SourceError::Open("replay input cannot be a drive root".to_string()));
            }
            let mut cursor = PathBuf::from(prefix);
            cursor.push("\\");
            let mut handles = Vec::with_capacity(normals.len());
            for (index, component) in normals.iter().enumerate() {
                cursor.push(component.as_os_str());
                let file = OpenOptions::new()
                    .read(true)
                    // Ancestor directories remain rename/rebind-proof because
                    // delete sharing is omitted, while write sharing permits
                    // unrelated child creation (including a LocalAppData
                    // spool beneath a shared ancestor). The final recording
                    // object remains immutable with read sharing only.
                    .share_mode(if index + 1 == normals.len() {
                        FILE_SHARE_READ
                    } else {
                        FILE_SHARE_READ | FILE_SHARE_WRITE
                    })
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(&cursor)
                    .map_err(|error| {
                        map_recording_path_error(
                            format!("open replay input component {} no-follow", cursor.display()),
                            error,
                        )
                    })?;
                let metadata = file.metadata().map_err(|error| {
                    map_recording_path_error(
                        format!("inspect replay input component {}", cursor.display()),
                        error,
                    )
                })?;
                let is_final = index + 1 == normals.len();
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                    || (!is_final && !metadata.is_dir())
                    || (is_final && !metadata.is_dir() && !metadata.is_file())
                {
                    return Err(SourceError::Open(format!(
                        "replay input contains a reparse point or unexpected object type: {}",
                        cursor.display()
                    )));
                }
                handles.push(file);
            }
            return Ok(Self { _handles: handles });
        }
        #[cfg(not(windows))]
        {
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                map_recording_path_error(
                    format!("inspect replay input {}", path.display()),
                    error,
                )
            })?;
            if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
                return Err(SourceError::Open(format!(
                    "replay input is a symlink or unsupported object type: {}",
                    path.display()
                )));
            }
            Ok(Self {})
        }
    }
}

/// Resolve the machine-local parent for transient replay data. Nothing inside
/// a portable recording selects or influences this path.
fn product_spool_root() -> Result<ProductSpoolRoot, SourceError> {
    #[cfg(test)]
    let base = std::env::temp_dir();
    #[cfg(all(windows, not(test)))]
    {
        let (canonical, guards) = windows_spool_storage::prepare_product_spool()
            .map_err(|error| SourceError::Open(format!("prepare private product replay spool: {error}")))?;
        return Ok(ProductSpoolRoot {
            canonical,
            _guards: guards,
        });
    }
    #[cfg(all(not(windows), not(test)))]
    let base = std::env::temp_dir();

    #[cfg(any(test, not(windows)))]
    let product = ensure_owned_child(&base, "battle-plan")?;
    #[cfg(any(test, not(windows)))]
    let spool = ensure_owned_child(&product, "replay_spool")?;
    #[cfg(any(test, not(windows)))]
    Ok(ProductSpoolRoot { canonical: spool })
}

fn ensure_owned_child(base: &Path, name: &str) -> Result<PathBuf, SourceError> {
    let base_meta = fs::symlink_metadata(base).map_err(|error| {
        SourceError::Open(format!(
            "inspect product spool parent {}: {error}",
            base.display()
        ))
    })?;
    if !base_meta.file_type().is_dir() || base_meta.file_type().is_symlink() {
        return Err(SourceError::Open(format!(
            "product spool parent is not a regular non-symlink directory: {}",
            base.display()
        )));
    }
    let canonical_base = fs::canonicalize(base).map_err(|error| {
        SourceError::Open(format!(
            "canonicalize product spool parent {}: {error}",
            base.display()
        ))
    })?;
    let child = canonical_base.join(name);
    match fs::create_dir(&child) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(SourceError::Open(format!(
                "create product spool component {}: {error}",
                child.display()
            )))
        }
    }
    let child_meta = fs::symlink_metadata(&child).map_err(|error| {
        SourceError::Open(format!(
            "inspect product spool component {}: {error}",
            child.display()
        ))
    })?;
    if !child_meta.file_type().is_dir() || child_meta.file_type().is_symlink() {
        return Err(SourceError::Open(format!(
            "product spool component is not a regular non-symlink directory: {}",
            child.display()
        )));
    }
    let canonical_child = fs::canonicalize(&child).map_err(|error| {
        SourceError::Open(format!(
            "canonicalize product spool component {}: {error}",
            child.display()
        ))
    })?;
    if canonical_child.parent() != Some(canonical_base.as_path()) {
        return Err(SourceError::Open(format!(
            "product spool component escaped its canonical parent: {}",
            child.display()
        )));
    }
    Ok(canonical_child)
}

/// The stable product-owned parent. Per-recording children and their recursive
/// cleanup remain owned by replay_core's identity-marked spool guard. The
/// parent deliberately persists: deleting a shared empty parent can race a
/// concurrent source between its root check and private-child creation.
struct ProductSpoolRoot {
    canonical: PathBuf,
    #[cfg(all(windows, not(test)))]
    _guards: Vec<File>,
}

impl ProductSpoolRoot {
    #[cfg(test)]
    fn ensure(path: PathBuf) -> Result<Self, SourceError> {
        fs::create_dir_all(&path).map_err(|error| {
            SourceError::Open(format!(
                "create product replay spool {}: {error}",
                path.display()
            ))
        })?;
        let meta = fs::symlink_metadata(&path).map_err(|error| {
            SourceError::Open(format!(
                "inspect product replay spool {}: {error}",
                path.display()
            ))
        })?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(SourceError::Open(format!(
                "product replay spool is not a regular non-symlink directory: {}",
                path.display()
            )));
        }
        let canonical = fs::canonicalize(&path).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize product replay spool {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { canonical })
    }

    fn path(&self) -> &Path {
        &self.canonical
    }
}

/// Retains the canonical product-root identity alongside the replay source.
/// The inner replay_core owner deletes only its own authenticated child.
struct ProductReplaySource {
    inner: Box<dyn ReplaySource>,
    _spool_root: ProductSpoolRoot,
    _recording_guard: ProductRecordingPathGuard,
}

impl ReplaySource for ProductReplaySource {
    fn game_id(&self) -> &str {
        self.inner.game_id()
    }
    fn hours(&self) -> &[i64] {
        self.inner.hours()
    }
    fn warnings(&self) -> &[String] {
        self.inner.warnings()
    }
    fn declared_coverage(&self) -> Option<&replay_core::DeclaredCoverage> {
        // The shipped product path wraps the admitted source; every accessor
        // here must forward, or evidence admitted at ingest silently becomes
        // the trait default on exactly the route players use.
        self.inner.declared_coverage()
    }
    fn content_manifest(&self) -> Option<&ContentManifest> {
        self.inner.content_manifest()
    }
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.inner.campaign_identity()
    }
    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.inner.product_admission()
    }
    /// Found by the derived accessor list below the moment it replaced the
    /// hand-maintained array: this one had never been forwarded, so the G2
    /// engine capture admitted at ingest read as absent on the product route
    /// while `replay_dump` saw it.
    fn g2_engine_capture(&self) -> Option<&PackagedG2EngineCapture> {
        self.inner.g2_engine_capture()
    }
    fn design_catalog(&self) -> Option<&Arc<PortableDesignCatalog>> {
        self.inner.design_catalog()
    }
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.inner.decision_catalog()
    }
    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.inner.static_artifact(name)
    }
    fn prefold_receipt(&self) -> Option<&PrefoldSourceReceipt> {
        self.inner.prefold_receipt()
    }
    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        self.inner.date_at(hour_index)
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        self.inner.frame(hour_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        CertifiedPrefoldSpoolRequest, PrefoldDecodeError, PrefoldedSpool, PrefoldedSpoolEntry,
    };
    use snapshot::{pack, PackEntry, PackIdentity};

    /// R7 F1 regression: each typed rollover authentication class keeps its
    /// honest `SourceError` class at the product boundary. Only a genuine
    /// selection/usage refusal may reach the "recording not playable by this
    /// build" headline; a tampered member is an integrity failure.
    #[test]
    fn rollover_authentication_classes_map_to_their_source_error_classes() {
        use replay_core::RolloverAuthenticationError as E;
        assert!(matches!(
            map_rollover_authentication_error(E::Integrity("member sha differs".into())),
            SourceError::Integrity(message) if message.contains("member sha differs")
        ));
        assert!(matches!(
            map_rollover_authentication_error(E::ModsetMismatch("modset disagrees".into())),
            SourceError::ModsetMismatch(_)
        ));
        assert!(matches!(
            map_rollover_authentication_error(E::Structure("descriptor malformed".into())),
            SourceError::Open(_)
        ));
        assert!(matches!(
            map_rollover_authentication_error(E::Unsupported("select one explicitly".into())),
            SourceError::Unsupported(_)
        ));
    }

    fn temp_root(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "replay_app_product_spool_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// The shipped product source is a WRAPPER: every `ReplaySource` accessor
    /// with a default body silently becomes that default if the wrapper forgets
    /// to forward it, and the default answers "there is nothing here". A
    /// forgotten forward is therefore evidence loss on exactly the route
    /// players use, with no compile error and no failing behaviour test.
    ///
    /// THE ACCESSOR LIST IS DERIVED FROM THE TRAIT, not written here. It used
    /// to be a hand-maintained array, which is the same defect one level up:
    /// adding `product_admission` to the trait left the wrapper dropping it AND
    /// this test passing, because nobody edited the array (Codex round 4,
    /// HIGH). Reading `replay_core`'s trait means a new accessor fails this
    /// test the moment it exists.
    ///
    /// kills: adding any accessor to `ReplaySource` without forwarding it here.
    #[test]
    fn the_product_source_wrapper_forwards_every_replay_source_accessor() {
        // Normalize line endings first: include_str! sees each file exactly as
        // checked out, and a CRLF checkout would otherwise never match "\n}\n".
        let extract = |text: &str, header: &str| -> String {
            let start = text.find(header).unwrap_or_else(|| {
                panic!("{header:?} must exist");
            });
            let block = &text[start..];
            let end = block.find("\n}\n").expect("block must close") + 1;
            block[..end].to_string()
        };

        let trait_source =
            include_str!("../../replay_core/src/source.rs").replace("\r\n", "\n");
        let trait_block = extract(&trait_source, "pub trait ReplaySource: Send + Sync {");
        let accessors = trait_block
            .lines()
            .filter_map(|line| line.trim().strip_prefix("fn "))
            .filter_map(|rest| rest.split('(').next())
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert!(
            accessors.len() >= 12,
            "the trait scrape found only {accessors:?}; the header or layout moved and this test \
             would silently check nothing"
        );

        let source = include_str!("recording_open.rs").replace("\r\n", "\n");
        let block = extract(&source, "impl ReplaySource for ProductReplaySource {");
        for accessor in &accessors {
            assert!(
                block.contains(&format!("fn {accessor}(")),
                "ProductReplaySource must forward {accessor}; the trait default would drop it"
            );
            assert!(
                block.contains(&format!("self.inner.{accessor}(")),
                "ProductReplaySource::{accessor} must delegate to the admitted source"
            );
        }
    }

    #[test]
    fn worker_identity_mode_wire_labels_are_closed_and_stable() {
        assert_eq!(
            prefold_identity_mode_label(PrefoldStreamIdentityMode::Legacy),
            "legacy"
        );
        assert_eq!(
            prefold_identity_mode_label(PrefoldStreamIdentityMode::IdentityV1),
            "identity-v1"
        );
    }

    #[test]
    fn worker_pipe_capture_is_bounded_at_the_exact_byte_ceiling() {
        let overflow = Arc::new(AtomicBool::new(false));
        let accepted = read_child_pipe_bounded(
            std::io::Cursor::new(vec![b'a'; 16]),
            16,
            Arc::clone(&overflow),
        )
        .unwrap();
        assert_eq!(accepted.len(), 16);
        assert!(!overflow.load(Ordering::Acquire));

        let rejected = read_child_pipe_bounded(
            std::io::Cursor::new(vec![b'b'; 17]),
            16,
            Arc::clone(&overflow),
        )
        .unwrap();
        assert!(rejected.len() <= 16);
        assert!(overflow.load(Ordering::Acquire));
        assert_eq!(PREFOLD_WORKER_WALL_LIMIT, Duration::from_secs(15 * 60));
        assert!(PREFOLD_WORKER_STDERR_LIMIT < PREFOLD_WORKER_STDOUT_LIMIT);
    }

    #[cfg(windows)]
    #[test]
    fn product_recording_guard_rejects_unc_before_filesystem_probes() {
        let error = match ProductRecordingPathGuard::acquire(Path::new(
            r"\\server\share\recording.battleplan",
        )) {
            Err(error) => error,
            Ok(_) => panic!("UNC playback must fail lexically without network access"),
        };
        assert!(error.to_string().contains("UNC"));
    }

    #[test]
    fn worker_receipt_is_bound_to_authenticated_executable_not_decoder_id_alone() {
        let expected_hash = "11".repeat(32);
        let mut receipt = serde_json::json!({
            "schema": crate::prefold_worker::RECEIPT_SCHEMA,
            "decoder_id": PREFOLD_WORKER_DECODER_ID,
            "worker_executable_sha256": expected_hash,
            "game_id": "g",
            "identity_mode": "identity-v1",
        });
        assert!(worker_receipt_identity_matches(
            receipt.as_object().unwrap(),
            "g",
            PrefoldStreamIdentityMode::IdentityV1,
            &"11".repeat(32),
        ));
        receipt["worker_executable_sha256"] = serde_json::json!("22".repeat(32));
        assert!(!worker_receipt_identity_matches(
            receipt.as_object().unwrap(),
            "g",
            PrefoldStreamIdentityMode::IdentityV1,
            &"11".repeat(32),
        ));
    }

    #[test]
    fn opened_handle_type_rejects_swap_between_path_check_and_open() {
        let root = temp_root("self_image_check_open_swap");
        let candidate = root.join("replay_app.fixture");
        let displaced = root.join("replay_app.displaced");
        fs::write(&candidate, b"original image").unwrap();

        let error = open_validated_handle_with(
            &candidate,
            "fixture executable",
            OpenedObjectKind::Image,
            |path| {
                // The production helper has already completed its pathname
                // precheck when this hook runs. Replace the checked file with
                // a directory before the handle is opened.
                fs::rename(path, &displaced).map_err(|error| {
                    PrefoldDecodeError::new(format!("displace fixture image: {error}"))
                })?;
                fs::create_dir(path).map_err(|error| {
                    PrefoldDecodeError::new(format!("replace fixture image: {error}"))
                })?;
                open_locked_directory(path)
            },
        )
        .expect_err("opened-handle type validation must reject the replacement directory");
        assert!(
            error.to_string().contains("is not a regular file"),
            "unexpected opened-handle rejection: {error}"
        );

        fs::remove_dir(&candidate).unwrap();
        fs::remove_file(&displaced).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn authenticated_prefold_source_is_not_rebound_by_path_substitution() {
        let root = temp_root("prefold_source_substitution");
        let source_path = root.join("source.h4cr");
        let displaced = root.join("admitted.h4cr");
        let original = b"H4CRauthenticated-source-object";
        let replacement = b"H4CRreplacement-path-object";
        fs::write(&source_path, original).unwrap();
        let expected_sha256 = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(original));
        let source =
            AuthenticatedPrefoldSource::open(&source_path, original.len() as u64, &expected_sha256)
                .unwrap();

        let rename = fs::rename(&source_path, &displaced);
        #[cfg(windows)]
        assert!(
            rename.is_err(),
            "deny-delete sharing must block source pathname substitution"
        );
        #[cfg(not(windows))]
        {
            rename.unwrap();
            fs::write(&source_path, replacement).unwrap();
        }

        assert_eq!(
            source.read_clone().unwrap(),
            original,
            "the child-bound duplicate must still read the admitted file object"
        );
        drop(source);

        #[cfg(windows)]
        fs::remove_file(&source_path).unwrap();
        #[cfg(not(windows))]
        {
            assert_eq!(portable_test_fs_retry::read(&source_path).unwrap(), replacement);
            fs::remove_file(&source_path).unwrap();
            fs::remove_file(&displaced).unwrap();
        }
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn product_parent_is_stable_and_never_recursively_cleaned() {
        let parent = temp_root("parent_cleanup");
        let root_path = parent.join("spool");
        let root = ProductSpoolRoot::ensure(root_path.clone()).unwrap();
        fs::write(root_path.join("unexpected-residue"), b"preserve").unwrap();
        drop(root);
        assert!(root_path.exists(), "stable product root must be preserved");
        fs::remove_file(root_path.join("unexpected-residue")).unwrap();
        fs::remove_dir(&root_path).unwrap();
        fs::remove_dir(&parent).unwrap();
    }

    struct FixtureSpoolDecoder;

    struct CampaignIdentitySource {
        identity: RecordedCampaignIdentity,
    }

    impl ReplaySource for CampaignIdentitySource {
        fn game_id(&self) -> &str {
            "campaign-identity"
        }

        fn hours(&self) -> &[i64] {
            &[]
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
            Some(&self.identity)
        }

        fn frame(&self, _hour_index: i64) -> Result<RawFrame, SourceError> {
            Err(SourceError::Open("fixture has no frames".into()))
        }
    }

    #[test]
    fn product_source_preserves_validated_campaign_metadata() {
        let parent = temp_root("campaign_identity_forwarding");
        let spool_path = parent.join("spool");
        let source = ProductReplaySource {
            inner: Box::new(CampaignIdentitySource {
                identity: RecordedCampaignIdentity {
                    state: "unavailable".into(),
                    campaign_id: None,
                    game_unique_id: None,
                    game_unique_seed: None,
                    source: None,
                    source_build_profile: Some("hoi4-a729".into()),
                    observed_game_hour_raw: None,
                    stream_header_sha256: None,
                    resolution_note: Some("fixture unavailable".into()),
                    witnesses: replay_core::RecordedCampaignIdentityWitnesses {
                        start_date_raw: Some(1),
                        session: Some(2),
                        scenario: None,
                        initial_human_country_tags: None,
                        modset_identity: None,
                    },
                },
            }),
            _spool_root: ProductSpoolRoot::ensure(spool_path.clone()).unwrap(),
            _recording_guard: ProductRecordingPathGuard::acquire(&spool_path).unwrap(),
        };
        let identity = source
            .campaign_identity()
            .expect("product wrapper must delegate campaign metadata");
        assert_eq!(
            identity.resolution_note.as_deref(),
            Some("fixture unavailable")
        );
        assert_eq!(identity.witnesses.session, Some(2));
        drop(source);
        fs::remove_dir(&spool_path).unwrap();
        fs::remove_dir(&parent).unwrap();
    }

    struct AdmissionSource {
        admission: RecorderProductAdmission,
    }

    impl ReplaySource for AdmissionSource {
        fn game_id(&self) -> &str {
            "admission"
        }

        fn hours(&self) -> &[i64] {
            &[]
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn product_admission(&self) -> Option<&RecorderProductAdmission> {
            Some(&self.admission)
        }

        fn frame(&self, _hour_index: i64) -> Result<RawFrame, SourceError> {
            Err(SourceError::Open("fixture has no frames".into()))
        }
    }

    /// BEHAVIOURAL sentinel for the late-attach admission on the product route.
    ///
    /// The forwarding test above is lexical, so a body that calls
    /// `self.inner.product_admission()` and then returns `None` would satisfy
    /// both of its substring searches while dropping the evidence (Codex round
    /// 5, LOW). This one asks the wrapper for the value and compares it.
    ///
    /// kills: `ProductReplaySource::product_admission` returning the trait
    /// default, or delegating and discarding.
    #[test]
    fn product_source_preserves_the_late_attach_admission() {
        let parent = temp_root("product_admission_forwarding");
        let spool_path = parent.join("spool");
        let admission = RecorderProductAdmission {
            authority: replay_core::ProductionProfileAuthority::Disabled,
            guarantees_not_held: vec![
                replay_core::ProductionProfileGuarantee::ProductionProfileEnabled,
            ],
            profile_domains_not_scheduled: vec!["decision_panel_live".into()],
            missing_required_static_artifacts: vec!["focus_defs_static".into()],
            static_artifacts_not_content_bound: vec!["localisation_join".into()],
        };
        let source = ProductReplaySource {
            inner: Box::new(AdmissionSource {
                admission: admission.clone(),
            }),
            _spool_root: ProductSpoolRoot::ensure(spool_path.clone()).unwrap(),
            _recording_guard: ProductRecordingPathGuard::acquire(&spool_path).unwrap(),
        };
        assert_eq!(
            source.product_admission(),
            Some(&admission),
            "the product route must carry the named shortfall, not the trait default"
        );
        drop(source);
        fs::remove_dir(&spool_path).unwrap();
        fs::remove_dir(&parent).unwrap();
    }

    impl CertifiedPrefoldSpoolDecoder for FixtureSpoolDecoder {
        fn decoder_id(&self) -> &'static str {
            "test.replay-app.bounded-spool"
        }

        fn decode_certified_spool(
            &self,
            request: CertifiedPrefoldSpoolRequest,
        ) -> Result<PrefoldedSpool, PrefoldDecodeError> {
            let (game_id, raw_path, snapshot_dir, source_len, source_sha256) = request.into_parts();
            let raw =
                portable_test_fs_retry::read(raw_path).map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            if raw != b"H4CRfixture-stream"
                || raw.len() as u64 != source_len
                || snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&raw))
                    != source_sha256
            {
                return Err(PrefoldDecodeError::new("fixture source receipt mismatch"));
            }
            let bytes = serde_json::to_vec(&serde_json::json!({
                "game_id": game_id,
                "schema_version": "1",
                "hour_index": 7,
                "date": "1936.1.1.7"
            }))
            .unwrap();
            let file_name = "000000000007.snapshot.json";
            fs::write(snapshot_dir.join(file_name), &bytes)
                .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            let entry = PrefoldedSpoolEntry::try_new(
                7,
                "1936.1.1.7",
                file_name,
                bytes.len() as u64,
                snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes)),
            )?;
            PrefoldedSpool::try_new(
                game_id,
                serde_json::Value::String("1".to_string()),
                vec![entry],
            )
        }
    }

    #[test]
    fn shipped_policy_activates_only_the_explicit_bounded_capability() {
        let parent = temp_root("bounded_activation");
        let game = "bounded-product-game";
        let container_path = parent.join("compact.battleplan");
        let entries = vec![
            PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&serde_json::json!({
                    "game_id": game,
                    "schema_version": "1",
                    "snapshots": []
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/{game}/collector_raw/collector_stream.bin"),
                bytes: b"H4CRfixture-stream".to_vec(),
                role: "collector_compact_stream",
                hour_index: None,
            },
        ];
        let identity = PackIdentity {
            schema_version: Some("1".to_string()),
            ..Default::default()
        };
        fs::write(
            &container_path,
            pack(game, &entries, &identity, true).unwrap(),
        )
        .unwrap();
        // R18-5: ordinary open refuses the compact-stream-only container with
        // the typed subclass (compact evidence present in the manifest role).
        assert!(matches!(
            portable_test_fs_retry::with_tree_read_guard(&container_path, || {
                replay_core::open_recording(&container_path)
            })
            .expect("hold the container open across the scanner's window"),
            Err(SourceError::UnsupportedCompactStreamOnly(_))
        ));

        let spool_path = parent.join("product-spool");
        let root = ProductSpoolRoot::ensure(spool_path.clone()).unwrap();
        let source = open_product_recording_at(&container_path, root, &FixtureSpoolDecoder)
            .expect("explicit bounded product activation");
        assert_eq!(source.hours(), &[7]);
        let receipt = source.prefold_receipt().expect("source receipt exposed");
        assert_eq!(receipt.decoder_id(), "test.replay-app.bounded-spool");
        assert_eq!(receipt.source_len(), 18);
        assert_eq!(source.frame(7).unwrap().date.as_deref(), Some("1936.1.1.7"));
        drop(source);
        assert!(spool_path.is_dir(), "stable product spool parent persists");
        assert_eq!(
            fs::read_dir(&spool_path).unwrap().count(),
            0,
            "per-source authenticated child must be cleaned"
        );
        fs::remove_dir(&spool_path).unwrap();
        fs::remove_dir_all(parent).unwrap();
    }
}
