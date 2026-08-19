use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use super::identity_control::{
    encode_identity_failure, IdentityDurableCandidateSink, IdentityFailureV1,
    ValidatedFirstCompactRecord,
};
use crate::control_transport::{ControlCancellation, ControlDeadline};

pub const FIRST_RECORD_JOURNAL_DIRECTORY: &str = "first-records";
pub const IDENTITY_FAILURE_JOURNAL_DIRECTORY: &str = "identity-failures";
pub const IDENTITY_FAILURE_GAP_LEDGER_DIRECTORY: &str = "identity-failure-gap-ledger";

static CANDIDATE_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct FilesystemIdentityDurableSink {
    root: PathBuf,
}

impl FilesystemIdentityDurableSink {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn create_candidate(
        &self,
        directory: &str,
        semantic_name: &str,
        extension: &str,
        bytes: &[u8],
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<FilesystemDurableCandidate, String> {
        preflight(deadline, cancellation)?;
        let directory = self.root.join(directory);
        let semantic_name = semantic_name.to_string();
        let extension = extension.to_string();
        let expected_bytes = bytes.to_vec();
        let worker_cancellation = cancellation.clone();
        let state = run_bounded_worker(
            "identity-journal-create",
            deadline,
            cancellation,
            move || {
                preflight(deadline, &worker_cancellation)?;
                fs::create_dir_all(&directory).map_err(|error| {
                    format!(
                        "create identity journal directory {}: {error}",
                        directory.display()
                    )
                })?;
                preflight(deadline, &worker_cancellation)?;
                let mut state = loop {
                    let nonce = CANDIDATE_NONCE.fetch_add(1, Ordering::Relaxed);
                    let file_name = format!(
                        "{semantic_name}-pid{}-n{nonce}.{extension}",
                        std::process::id()
                    );
                    let path = directory.join(file_name);
                    match open_unique_candidate(&path) {
                        Ok(file) => break CandidateState::new(path, file, expected_bytes.clone()),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            preflight(deadline, &worker_cancellation)?;
                        }
                        Err(error) => {
                            return Err(format!(
                                "create unique identity journal candidate {}: {error}",
                                path.display()
                            ));
                        }
                    }
                };
                if let Err(error) = write_all_checked(
                    state.file_mut()?,
                    &expected_bytes,
                    deadline,
                    &worker_cancellation,
                ) {
                    return Err(format!(
                        "write identity journal candidate {}: {error}",
                        state.path.display()
                    ));
                }
                preflight(deadline, &worker_cancellation)?;
                Ok(state)
            },
        )?;
        preflight(deadline, cancellation)?;
        Ok(FilesystemDurableCandidate::new(state))
    }

    fn flush_candidate(
        candidate: &mut FilesystemDurableCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        preflight(deadline, cancellation)?;
        let mut state = candidate.take_state()?;
        let worker_cancellation = cancellation.clone();
        state = run_bounded_worker(
            "identity-journal-flush",
            deadline,
            cancellation,
            move || {
                preflight(deadline, &worker_cancellation)?;
                state.file_mut()?.flush().map_err(|error| {
                    format!(
                        "flush identity journal candidate {}: {error}",
                        state.path.display()
                    )
                })?;
                preflight(deadline, &worker_cancellation)?;
                Ok(state)
            },
        )?;
        candidate.restore_state(state);
        preflight(deadline, cancellation)
    }

    fn sync_candidate(
        candidate: &mut FilesystemDurableCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        preflight(deadline, cancellation)?;
        let mut state = candidate.take_state()?;
        let worker_cancellation = cancellation.clone();
        state = run_bounded_worker("identity-journal-sync", deadline, cancellation, move || {
            preflight(deadline, &worker_cancellation)?;
            state.file_mut()?.sync_all().map_err(|error| {
                format!(
                    "sync identity journal candidate {}: {error}",
                    state.path.display()
                )
            })?;
            preflight(deadline, &worker_cancellation)?;
            state.revalidate_exact_bytes(deadline, &worker_cancellation)?;
            preflight(deadline, &worker_cancellation)?;
            sync_candidate_directory(&state.path).map_err(|error| {
                format!(
                    "sync identity journal directory for {}: {error}",
                    state.path.display()
                )
            })?;
            preflight(deadline, &worker_cancellation)?;
            Ok(state)
        })?;
        candidate.restore_state(state);
        preflight(deadline, cancellation)?;
        candidate.mark_durable();
        Ok(())
    }
}

#[derive(Debug)]
pub struct FilesystemDurableCandidate {
    state: Option<CandidateState>,
    last_path: PathBuf,
}

#[derive(Debug)]
struct CandidateState {
    path: PathBuf,
    file: Option<File>,
    expected_bytes: Vec<u8>,
    durable: bool,
}

impl FilesystemDurableCandidate {
    fn new(state: CandidateState) -> Self {
        let last_path = state.path.clone();
        Self {
            state: Some(state),
            last_path,
        }
    }

    pub fn path(&self) -> &Path {
        self.state
            .as_ref()
            .map(|state| state.path.as_path())
            .unwrap_or(&self.last_path)
    }

    fn take_state(&mut self) -> Result<CandidateState, String> {
        self.state.take().ok_or_else(|| {
            "identity journal candidate is owned by a quarantined worker".to_string()
        })
    }

    fn restore_state(&mut self, state: CandidateState) {
        self.last_path = state.path.clone();
        self.state = Some(state);
    }

    fn mark_durable(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.durable = true;
        }
    }
}

impl CandidateState {
    fn new(path: PathBuf, file: File, expected_bytes: Vec<u8>) -> Self {
        Self {
            path,
            file: Some(file),
            expected_bytes,
            durable: false,
        }
    }

    fn revalidate_exact_bytes(
        &mut self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        preflight(deadline, cancellation)?;
        let path = self.path.clone();
        let expected = self.expected_bytes.clone();
        self.file_mut()?.seek(SeekFrom::Start(0)).map_err(|error| {
            format!(
                "seek identity journal candidate {} for readback: {error}",
                path.display()
            )
        })?;
        preflight(deadline, cancellation)?;
        let mut actual = vec![0u8; expected.len()];
        read_exact_checked(self.file_mut()?, &mut actual, deadline, cancellation).map_err(
            |error| {
                format!(
                    "read back identity journal candidate {}: {error}",
                    path.display()
                )
            },
        )?;
        let mut extra = [0u8; 1];
        let extra_len = read_checked(self.file_mut()?, &mut extra, deadline, cancellation)
            .map_err(|error| {
                format!(
                    "check identity journal candidate {} trailing bytes: {error}",
                    path.display()
                )
            })?;
        if extra_len != 0 || actual != expected {
            return Err(format!(
                "identity journal candidate {} failed exact-byte readback",
                path.display()
            ));
        }
        Ok(())
    }

    fn file_mut(&mut self) -> Result<&mut File, String> {
        self.file
            .as_mut()
            .ok_or_else(|| "identity journal candidate file is closed".to_string())
    }
}

impl Drop for CandidateState {
    fn drop(&mut self) {
        if self.durable {
            return;
        }
        let _ = self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

impl IdentityDurableCandidateSink for FilesystemIdentityDurableSink {
    type FirstRecordCandidate = FilesystemDurableCandidate;
    type IdentityFailureCandidate = FilesystemDurableCandidate;
    type IdentityFailureGapLedgerCandidate = FilesystemDurableCandidate;

    fn write_first_record_candidate(
        &mut self,
        first_record: &ValidatedFirstCompactRecord,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::FirstRecordCandidate, String> {
        let acknowledgement = first_record.acknowledgement();
        self.create_candidate(
            FIRST_RECORD_JOURNAL_DIRECTORY,
            &format!(
                "stream{}-seq{}",
                acknowledgement.logical_stream_id, acknowledgement.h4cr_sequence
            ),
            "h4cr",
            first_record.complete_record(),
            deadline,
            cancellation,
        )
    }

    fn flush_first_record_candidate(
        &mut self,
        candidate: &mut Self::FirstRecordCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        Self::flush_candidate(candidate, deadline, cancellation)
    }

    fn sync_first_record_candidate(
        &mut self,
        candidate: &mut Self::FirstRecordCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        Self::sync_candidate(candidate, deadline, cancellation)
    }

    fn write_identity_failure_candidate(
        &mut self,
        failure: &IdentityFailureV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::IdentityFailureCandidate, String> {
        preflight(deadline, cancellation)?;
        let bytes = encode_identity_failure(failure)
            .map_err(|error| format!("encode identity failure journal entry: {error}"))?;
        preflight(deadline, cancellation)?;
        let logical_id = failure
            .current_logical_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unbound".to_string());
        self.create_candidate(
            IDENTITY_FAILURE_JOURNAL_DIRECTORY,
            &format!("attempt{}-stream{logical_id}", failure.attempt_id),
            "cif1",
            &bytes,
            deadline,
            cancellation,
        )
    }

    fn flush_identity_failure_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        Self::flush_candidate(candidate, deadline, cancellation)
    }

    fn sync_identity_failure_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        Self::sync_candidate(candidate, deadline, cancellation)
    }

    fn write_identity_failure_gap_ledger_candidate(
        &mut self,
        failure: &IdentityFailureV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Self::IdentityFailureGapLedgerCandidate, String> {
        preflight(deadline, cancellation)?;
        let bytes = encode_identity_failure(failure)
            .map_err(|error| format!("encode identity failure gap-ledger entry: {error}"))?;
        preflight(deadline, cancellation)?;
        let logical_id = failure
            .current_logical_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unbound".to_string());
        self.create_candidate(
            IDENTITY_FAILURE_GAP_LEDGER_DIRECTORY,
            &format!("attempt{}-stream{logical_id}-gap", failure.attempt_id),
            "cif1",
            &bytes,
            deadline,
            cancellation,
        )
    }

    fn flush_identity_failure_gap_ledger_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureGapLedgerCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        Self::flush_candidate(candidate, deadline, cancellation)
    }

    fn sync_identity_failure_gap_ledger_candidate(
        &mut self,
        candidate: &mut Self::IdentityFailureGapLedgerCandidate,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<(), String> {
        Self::sync_candidate(candidate, deadline, cancellation)
    }
}

fn preflight(deadline: ControlDeadline, cancellation: &ControlCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("identity journal filesystem operation cancelled".to_string())
    } else if deadline.is_expired() {
        Err("identity journal filesystem operation deadline expired".to_string())
    } else {
        Ok(())
    }
}

fn run_bounded_worker<T, F>(
    worker_name: &'static str,
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    preflight(deadline, cancellation)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name(worker_name.to_string())
        .spawn(move || {
            let result = operation();
            let _ = sender.send(result);
        })
        .map_err(|error| format!("spawn {worker_name} worker: {error}"))?;

    loop {
        preflight(deadline, cancellation)?;
        let remaining = deadline
            .remaining()
            .ok_or_else(|| format!("{worker_name} exceeded identity journal operation deadline"))?;
        let wait = remaining.min(Duration::from_millis(10));
        match receiver.recv_timeout(wait) {
            Ok(result) => {
                worker
                    .join()
                    .map_err(|_| format!("{worker_name} worker panicked"))?;
                return result;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return worker
                    .join()
                    .map(|_| Err(format!("{worker_name} worker disconnected")))
                    .unwrap_or_else(|_| Err(format!("{worker_name} worker panicked")));
            }
        }
    }
}

fn open_unique_candidate(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
        options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    }
    options.open(path)
}

#[cfg(not(windows))]
fn sync_candidate_directory(candidate_path: &Path) -> io::Result<()> {
    let parent = candidate_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "candidate has no parent"))?;
    File::open(parent)?.sync_all()
}

#[cfg(windows)]
fn sync_candidate_directory(candidate_path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    type Handle = *mut std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *mut std::ffi::c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: Handle,
        ) -> Handle;
        fn FlushFileBuffers(file: Handle) -> i32;
        fn CloseHandle(object: Handle) -> i32;
    }

    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

    let parent = candidate_path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "candidate has no parent"))?;
    let mut wide: Vec<u16> = parent
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_mut_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let flushed = unsafe { FlushFileBuffers(handle) };
    let flush_error = (flushed == 0).then(io::Error::last_os_error);
    let closed = unsafe { CloseHandle(handle) };
    if let Some(error) = flush_error {
        Err(error)
    } else if closed == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn write_all_checked(
    writer: &mut File,
    mut bytes: &[u8],
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> io::Result<()> {
    while !bytes.is_empty() {
        preflight(deadline, cancellation).map_err(|error| {
            io::Error::new(
                if cancellation.is_cancelled() {
                    io::ErrorKind::Interrupted
                } else {
                    io::ErrorKind::TimedOut
                },
                error,
            )
        })?;
        match writer.write(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    preflight(deadline, cancellation).map_err(|error| {
        io::Error::new(
            if cancellation.is_cancelled() {
                io::ErrorKind::Interrupted
            } else {
                io::ErrorKind::TimedOut
            },
            error,
        )
    })
}

fn read_exact_checked(
    reader: &mut File,
    mut bytes: &mut [u8],
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let read = read_checked(reader, bytes, deadline, cancellation)?;
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        bytes = &mut bytes[read..];
    }
    Ok(())
}

fn read_checked(
    reader: &mut File,
    bytes: &mut [u8],
    deadline: ControlDeadline,
    cancellation: &ControlCancellation,
) -> io::Result<usize> {
    loop {
        preflight(deadline, cancellation).map_err(preflight_io_error)?;
        match reader.read(bytes) {
            Ok(read) => {
                preflight(deadline, cancellation).map_err(preflight_io_error)?;
                return Ok(read);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn preflight_io_error(error: String) -> io::Error {
    let kind = if error.contains("cancelled") {
        io::ErrorKind::Interrupted
    } else {
        io::ErrorKind::TimedOut
    };
    io::Error::new(kind, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use super::super::identity_control::{
        decode_identity_failure, IdentityControlJournal, IdentityFailureReason,
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = CANDIDATE_NONCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "battle-plan-recorder-identity-journal-test-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn deadline() -> ControlDeadline {
        ControlDeadline::after(Duration::from_secs(10)).unwrap()
    }

    fn failure(attempt_id: u64) -> IdentityFailureV1 {
        IdentityFailureV1 {
            reason: IdentityFailureReason::UnreadableRoot,
            current_logical_id: None,
            attempt_id,
            attempted_hour: None,
            uuid_raw: None,
            seed_raw: None,
            profile_raw: None,
            first_read_complete: false,
            second_read_complete: false,
            reads_equal: false,
            last_admitted_sequence: None,
            current_cid1_hash: None,
        }
    }

    fn valid_legacy_first_record() -> (ValidatedFirstCompactRecord, Vec<u8>) {
        let hour = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;
        let payload = [0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        let mut bytes = vec![0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        bytes[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        bytes[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        bytes[6..8]
            .copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        bytes[8..16].copy_from_slice(&1u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&hour.to_le_bytes());
        bytes[20..22].copy_from_slice(&10u16.to_le_bytes());
        bytes[22..24]
            .copy_from_slice(&compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF.to_le_bytes());
        bytes[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        (
            ValidatedFirstCompactRecord::legacy(0, &bytes).unwrap(),
            bytes,
        )
    }

    #[test]
    fn journal_receipt_follows_exact_durable_first_record_bytes() {
        let directory = TestDirectory::new();
        let (first_record, exact) = valid_legacy_first_record();
        let mut journal =
            IdentityControlJournal::new(FilesystemIdentityDurableSink::new(directory.0.clone()));
        let receipt = journal
            .durable_first_record_admission(&first_record, deadline(), &ControlCancellation::new())
            .unwrap();
        assert_eq!(receipt.acknowledgement(), first_record.acknowledgement());

        let entries = fs::read_dir(directory.0.join(FIRST_RECORD_JOURNAL_DIRECTORY))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(fs::read(entries[0].path()).unwrap(), exact);
    }

    #[test]
    fn journal_receipt_follows_exact_durable_failure_bytes() {
        let directory = TestDirectory::new();
        let value = failure(7);
        let mut journal =
            IdentityControlJournal::new(FilesystemIdentityDurableSink::new(directory.0.clone()));
        let receipt = journal
            .durable_identity_failure(&value, deadline(), &ControlCancellation::new())
            .unwrap();
        assert_eq!(receipt.attempt_id(), 7);
        assert_eq!(receipt.logical_stream_id(), None);
        assert_eq!(receipt.gap_ledger_receipt().attempt_id(), 7);

        let entries = fs::read_dir(directory.0.join(IDENTITY_FAILURE_JOURNAL_DIRECTORY))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let exact = fs::read(entries[0].path()).unwrap();
        assert_eq!(exact, encode_identity_failure(&value).unwrap());
        assert_eq!(decode_identity_failure(&exact).unwrap(), value);

        let gap_entries = fs::read_dir(directory.0.join(IDENTITY_FAILURE_GAP_LEDGER_DIRECTORY))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(gap_entries.len(), 1);
        let gap_exact = fs::read(gap_entries[0].path()).unwrap();
        assert_eq!(gap_exact, exact);
        assert_eq!(decode_identity_failure(&gap_exact).unwrap(), value);
    }

    #[test]
    fn incomplete_candidate_cleanup_preserves_last_durable_entry() {
        let directory = TestDirectory::new();
        let mut sink = FilesystemIdentityDurableSink::new(directory.0.clone());
        let cancellation = ControlCancellation::new();

        let mut durable = sink
            .write_identity_failure_candidate(&failure(10), deadline(), &cancellation)
            .unwrap();
        sink.flush_identity_failure_candidate(&mut durable, deadline(), &cancellation)
            .unwrap();
        sink.sync_identity_failure_candidate(&mut durable, deadline(), &cancellation)
            .unwrap();
        let durable_path = durable.path().to_path_buf();
        drop(durable);
        assert!(durable_path.is_file());

        let incomplete = sink
            .write_identity_failure_candidate(&failure(11), deadline(), &cancellation)
            .unwrap();
        let incomplete_path = incomplete.path().to_path_buf();
        assert!(incomplete_path.is_file());
        drop(incomplete);
        assert!(!incomplete_path.exists());
        assert!(durable_path.is_file());
    }

    #[test]
    fn cancelled_or_expired_operations_create_no_candidate() {
        let directory = TestDirectory::new();
        let mut sink = FilesystemIdentityDurableSink::new(directory.0.clone());
        let cancellation = ControlCancellation::new();
        cancellation.cancel();
        assert!(sink
            .write_identity_failure_candidate(&failure(20), deadline(), &cancellation)
            .is_err());
        assert!(sink
            .write_identity_failure_candidate(
                &failure(21),
                ControlDeadline::at(std::time::Instant::now()),
                &ControlCancellation::new(),
            )
            .is_err());
        assert!(!directory
            .0
            .join(IDENTITY_FAILURE_JOURNAL_DIRECTORY)
            .exists());
    }

    #[test]
    fn timed_out_worker_eventually_cleans_its_owned_result() {
        struct CleanupProbe(Arc<std::sync::atomic::AtomicBool>);

        impl Drop for CleanupProbe {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let cleaned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cleaned = cleaned.clone();
        let deadline = ControlDeadline::after(Duration::from_millis(1)).unwrap();
        let result = run_bounded_worker(
            "identity-journal-cleanup-test",
            deadline,
            &ControlCancellation::new(),
            move || {
                thread::sleep(Duration::from_millis(25));
                Ok(CleanupProbe(worker_cleaned))
            },
        );
        assert!(result.is_err());
        let wait_until = std::time::Instant::now() + Duration::from_secs(1);
        while !cleaned.load(Ordering::Acquire) && std::time::Instant::now() < wait_until {
            thread::yield_now();
        }
        assert!(cleaned.load(Ordering::Acquire));
    }
}
