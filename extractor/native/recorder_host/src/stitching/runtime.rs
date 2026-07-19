//! Product-owned pre-fold and append-order transactions.
//!
//! This module closes two formerly caller-by-convention seams without selecting
//! the dormant auto-attach path:
//!
//! * [`DurableAppendLedger`] allocates one campaign-local [`AppendTicket`] under
//!   an exclusive cross-process lock and atomically commits it before returning;
//! * [`SegmentRuntime`] stages one complete logical frame, calls
//!   [`SegmentCutter::route_checked`], seals the old fold before constructing a
//!   fresh fold, and is the production call site for [`publish_product_stitch`].
//!
//! A failure poisons the runtime.  There is no retry on partly-mutated fold
//! state, no guessed ticket, and no publication while a fold remains open.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::formation::{
    FormationInputError, FrameFacts, FrameRouting, SealedSegment, SegmentCutter,
};
use super::product::{publish_product_stitch, ProductStitchOutcome};
use super::publish::StitchedPayloadSource;
use super::{AppendTicket, SegmentInput, StitchRequiredInputs};

const LEDGER_MAGIC: &[u8; 8] = b"AOLT1\0\0\0";
const MAX_TICKET_FIELD_BYTES: usize = 4096;
const MIN_RECORD_BYTES: usize = 8 + 4 + 4 + 4 + 32 + 32;

/// Campaign-local durable ticket allocator.
///
/// The caller supplies a store root, not a ledger path.  The actual filenames
/// are derived from SHA-256(campaign_key), so opaque identity bytes never become
/// path components.  A stale `.lock` after process death is intentionally a
/// loud fail-closed condition; deleting it requires an operator to first prove
/// that no allocator still owns the campaign.
#[derive(Debug, Clone)]
pub struct DurableAppendLedger {
    store_root: PathBuf,
}

impl DurableAppendLedger {
    pub fn new(store_root: PathBuf) -> Self {
        Self { store_root }
    }

    pub fn allocate(
        &self,
        campaign_key: &[u8],
        session_nonce: &str,
        process_start_identity: &str,
    ) -> Result<AppendTicket, String> {
        validate_ticket_field("campaign_key", campaign_key)?;
        validate_text_field("session_nonce", session_nonce)?;
        validate_text_field("process_start_identity", process_start_identity)?;
        fs::create_dir_all(&self.store_root)
            .map_err(|error| format!("create append-ledger store: {error}"))?;

        let stem = lower_hex(&Sha256::digest(campaign_key));
        let ledger_path = self.store_root.join(format!("{stem}.append-ledger-v1"));
        let lock_path = self
            .store_root
            .join(format!("{stem}.append-ledger-v1.lock"));
        let _lock = LedgerLock::acquire(lock_path)?;

        let mut tickets = read_ledger(&ledger_path, campaign_key)?;
        if let Some(existing) = tickets.iter().find(|ticket| {
            ticket.session_nonce == session_nonce
                && ticket.process_start_identity == process_start_identity
        }) {
            return Ok(existing.clone());
        }
        if tickets
            .iter()
            .any(|ticket| ticket.session_nonce == session_nonce)
        {
            return Err(
                "append-ticket session nonce collides with a different process identity"
                    .to_string(),
            );
        }
        let play_order = match tickets.last() {
            Some(last) => last
                .play_order
                .checked_add(1)
                .ok_or_else(|| "append-ticket play_order exhausted".to_string())?,
            None => 0,
        };
        let ticket = AppendTicket {
            campaign_key: campaign_key.to_vec(),
            play_order,
            session_nonce: session_nonce.to_owned(),
            process_start_identity: process_start_identity.to_owned(),
        };
        tickets.push(ticket.clone());
        write_ledger_atomically(&ledger_path, &tickets)?;
        Ok(ticket)
    }
}

struct LedgerLock {
    path: PathBuf,
    _file: File,
}

impl LedgerLock {
    fn acquire(path: PathBuf) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "acquire campaign append-ledger lock {}: {error}",
                    path.display()
                )
            })?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn validate_ticket_field(name: &str, value: &[u8]) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_TICKET_FIELD_BYTES {
        return Err(format!("append-ticket {name} has invalid length"));
    }
    Ok(())
}

fn validate_text_field(name: &str, value: &str) -> Result<(), String> {
    validate_ticket_field(name, value.as_bytes())?;
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(format!("append-ticket {name} has invalid text"));
    }
    Ok(())
}

fn read_ledger(path: &Path, campaign_key: &[u8]) -> Result<Vec<AppendTicket>, String> {
    let mut bytes = Vec::new();
    match File::open(path) {
        Ok(mut file) => file
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read append ledger {}: {error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("open append ledger {}: {error}", path.display())),
    };
    if bytes.len() < LEDGER_MAGIC.len() || &bytes[..LEDGER_MAGIC.len()] != LEDGER_MAGIC {
        return Err("append ledger has invalid/truncated magic".to_string());
    }
    let mut cursor = LEDGER_MAGIC.len();
    let mut tickets = Vec::new();
    let mut previous_digest = [0u8; 32];
    while cursor < bytes.len() {
        let record_len = take_u32(&bytes, &mut cursor)? as usize;
        if record_len < MIN_RECORD_BYTES
            || record_len > 3 * MAX_TICKET_FIELD_BYTES + MIN_RECORD_BYTES
            || cursor
                .checked_add(record_len)
                .is_none_or(|end| end > bytes.len())
        {
            return Err("append ledger record length is invalid".to_string());
        }
        let record_start = cursor;
        let record_end = cursor + record_len;
        let digest_start = record_end - 32;
        let play_order = take_u64_bounded(&bytes, &mut cursor, digest_start)?;
        let campaign_len = take_u32_bounded(&bytes, &mut cursor, digest_start)? as usize;
        let nonce_len = take_u32_bounded(&bytes, &mut cursor, digest_start)? as usize;
        let process_len = take_u32_bounded(&bytes, &mut cursor, digest_start)? as usize;
        for (name, len) in [
            ("campaign_key", campaign_len),
            ("session_nonce", nonce_len),
            ("process_start_identity", process_len),
        ] {
            if len == 0 || len > MAX_TICKET_FIELD_BYTES {
                return Err(format!("append ledger {name} length is invalid"));
            }
        }
        let recorded_previous = take_array32(&bytes, &mut cursor, digest_start)?;
        if recorded_previous != previous_digest {
            return Err("append ledger hash chain is broken".to_string());
        }
        let campaign = take_vec(&bytes, &mut cursor, digest_start, campaign_len)?;
        let nonce = take_string(&bytes, &mut cursor, digest_start, nonce_len)?;
        let process = take_string(&bytes, &mut cursor, digest_start, process_len)?;
        if cursor != digest_start {
            return Err("append ledger record contains trailing fields".to_string());
        }
        let recorded_digest: [u8; 32] = bytes[digest_start..record_end]
            .try_into()
            .expect("record bounds checked");
        let actual_digest: [u8; 32] = Sha256::digest(&bytes[record_start..digest_start]).into();
        if recorded_digest != actual_digest {
            return Err("append ledger record digest mismatch".to_string());
        }
        if campaign != campaign_key {
            return Err("append ledger campaign key disagrees with its path authority".to_string());
        }
        if play_order != tickets.len() as u64 {
            return Err("append ledger play_order is not dense from zero".to_string());
        }
        validate_text_field("session_nonce", &nonce)?;
        validate_text_field("process_start_identity", &process)?;
        tickets.push(AppendTicket {
            campaign_key: campaign,
            play_order,
            session_nonce: nonce,
            process_start_identity: process,
        });
        previous_digest = recorded_digest;
        cursor = record_end;
    }
    Ok(tickets)
}

fn write_ledger_atomically(path: &Path, tickets: &[AppendTicket]) -> Result<(), String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(LEDGER_MAGIC);
    let mut previous_digest = [0u8; 32];
    for (expected_order, ticket) in tickets.iter().enumerate() {
        if ticket.play_order != expected_order as u64 {
            return Err("refusing to write nondense append ledger".to_string());
        }
        validate_ticket_field("campaign_key", &ticket.campaign_key)?;
        validate_text_field("session_nonce", &ticket.session_nonce)?;
        validate_text_field("process_start_identity", &ticket.process_start_identity)?;
        let mut body = Vec::new();
        body.extend_from_slice(&ticket.play_order.to_le_bytes());
        body.extend_from_slice(&(ticket.campaign_key.len() as u32).to_le_bytes());
        body.extend_from_slice(&(ticket.session_nonce.len() as u32).to_le_bytes());
        body.extend_from_slice(&(ticket.process_start_identity.len() as u32).to_le_bytes());
        body.extend_from_slice(&previous_digest);
        body.extend_from_slice(&ticket.campaign_key);
        body.extend_from_slice(ticket.session_nonce.as_bytes());
        body.extend_from_slice(ticket.process_start_identity.as_bytes());
        let digest: [u8; 32] = Sha256::digest(&body).into();
        let record_len = body
            .len()
            .checked_add(digest.len())
            .and_then(|len| u32::try_from(len).ok())
            .ok_or_else(|| "append ledger record is too large".to_string())?;
        bytes.extend_from_slice(&record_len.to_le_bytes());
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(&digest);
        previous_digest = digest;
    }

    let temp = path.with_extension(format!(
        "append-ledger-v1.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp);
        return Err(format!("write durable append-ledger temp: {error}"));
    }
    if let Err(error) = atomic_replace(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(format!("atomically commit append ledger: {error}"));
    }
    #[cfg(not(windows))]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("fsync append-ledger directory: {error}"))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temp, destination)
}

#[cfg(windows)]
fn atomic_replace(temp: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "append-ledger path contains NUL",
            ));
        }
        value.push(0);
        Ok(value)
    };
    let existing = wide(temp)?;
    let replacement = wide(destination)?;
    let ok = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, String> {
    take_u32_bounded(bytes, cursor, bytes.len())
}

fn take_u32_bounded(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32, String> {
    let slice = take_slice(bytes, cursor, end, 4)?;
    Ok(u32::from_le_bytes(slice.try_into().expect("fixed slice")))
}

fn take_u64_bounded(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u64, String> {
    let slice = take_slice(bytes, cursor, end, 8)?;
    Ok(u64::from_le_bytes(slice.try_into().expect("fixed slice")))
}

fn take_array32(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<[u8; 32], String> {
    Ok(take_slice(bytes, cursor, end, 32)?
        .try_into()
        .expect("fixed slice"))
}

fn take_vec(bytes: &[u8], cursor: &mut usize, end: usize, len: usize) -> Result<Vec<u8>, String> {
    Ok(take_slice(bytes, cursor, end, len)?.to_vec())
}

fn take_string(bytes: &[u8], cursor: &mut usize, end: usize, len: usize) -> Result<String, String> {
    String::from_utf8(take_vec(bytes, cursor, end, len)?)
        .map_err(|_| "append ledger text is not UTF-8".to_string())
}

fn take_slice<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    end: usize,
    len: usize,
) -> Result<&'a [u8], String> {
    let next = cursor
        .checked_add(len)
        .ok_or_else(|| "append ledger offset overflow".to_string())?;
    if next > end || next > bytes.len() {
        return Err("append ledger is truncated".to_string());
    }
    let value = &bytes[*cursor..next];
    *cursor = next;
    Ok(value)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// One complete staged frame.  `payload` is opaque to the router and is moved
/// exactly once into either a fold or failed-evidence retention.
#[derive(Debug)]
pub struct StagedFrame<P> {
    pub facts: FrameFacts,
    pub payload: P,
}

/// Fresh per-segment fold.  Consuming `self` at seal makes accidental reuse
/// across a cut impossible at this boundary.
pub trait SegmentFold<P> {
    fn apply_complete(&mut self, frame: StagedFrame<P>) -> Result<(), String>;
    fn retain_failed(&mut self, frame: StagedFrame<P>, reason: &str) -> Result<(), String>;
    fn seal(self, extent: SealedSegment) -> Result<SegmentInput, String>;
}

/// Owns fresh fold construction and raw failed-frame retention when no segment
/// is open (for example during reload uncertainty).
pub trait SegmentFoldFactory<P> {
    type Fold: SegmentFold<P>;
    fn open_fresh(
        &mut self,
        ticket: &AppendTicket,
        ordinal: u64,
        first: &FrameFacts,
    ) -> Result<Self::Fold, String>;
    fn retain_unassigned_failed(
        &mut self,
        ticket: &AppendTicket,
        frame: StagedFrame<P>,
        reason: &str,
    ) -> Result<(), String>;
}

#[derive(Debug)]
pub enum SegmentRuntimeError {
    Formation(FormationInputError),
    Fold(String),
    Poisoned,
    OrdinalMismatch { expected: u64, actual: u64 },
    OpenFoldMissing,
}

impl std::fmt::Display for SegmentRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Formation(error) => write!(f, "staged frame rejected: {error}"),
            Self::Fold(error) => write!(f, "segment fold transaction failed: {error}"),
            Self::Poisoned => write!(f, "segment runtime is poisoned"),
            Self::OrdinalMismatch { expected, actual } => {
                write!(
                    f,
                    "open fold ordinal {actual} does not match route {expected}"
                )
            }
            Self::OpenFoldMissing => write!(f, "route selected a segment with no open fold"),
        }
    }
}

impl std::error::Error for SegmentRuntimeError {}

/// Ordered pre-fold runtime.  It is intentionally independent of the live pipe
/// reader so offline and future activated drivers use the same cut transaction.
pub struct SegmentRuntime<P, F: SegmentFoldFactory<P>> {
    ticket: AppendTicket,
    cutter: SegmentCutter,
    factory: F,
    open: Option<(u64, F::Fold)>,
    sealed: Vec<SegmentInput>,
    poisoned: bool,
    _payload: std::marker::PhantomData<P>,
}

impl<P, F: SegmentFoldFactory<P>> SegmentRuntime<P, F> {
    pub fn new(ticket: AppendTicket, factory: F) -> Self {
        Self {
            ticket,
            cutter: SegmentCutter::new(),
            factory,
            open: None,
            sealed: Vec::new(),
            poisoned: false,
            _payload: std::marker::PhantomData,
        }
    }

    pub fn stage(&mut self, frame: StagedFrame<P>) -> Result<(), SegmentRuntimeError> {
        if self.poisoned {
            return Err(SegmentRuntimeError::Poisoned);
        }
        let routing = self
            .cutter
            .route_checked(&frame.facts)
            .map_err(SegmentRuntimeError::Formation)?;
        let result = self.apply_route(routing, frame);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn apply_route(
        &mut self,
        routing: FrameRouting,
        frame: StagedFrame<P>,
    ) -> Result<(), SegmentRuntimeError> {
        match routing {
            FrameRouting::Cut {
                sealed,
                new_ordinal,
                ..
            } => {
                if let Some(extent) = sealed {
                    self.seal_exact(extent)?;
                } else if self.open.is_some() {
                    return Err(SegmentRuntimeError::Fold(
                        "cut omitted extent while a fold was open".to_string(),
                    ));
                }
                let mut fold = self
                    .factory
                    .open_fresh(&self.ticket, new_ordinal, &frame.facts)
                    .map_err(SegmentRuntimeError::Fold)?;
                if frame.facts.hour_index.is_some() {
                    fold.apply_complete(frame)
                        .map_err(SegmentRuntimeError::Fold)?;
                } else {
                    fold.retain_failed(frame, "incomplete logical frame")
                        .map_err(SegmentRuntimeError::Fold)?;
                }
                self.open = Some((new_ordinal, fold));
            }
            FrameRouting::Extend { ordinal } => {
                let (actual, fold) = self
                    .open
                    .as_mut()
                    .ok_or(SegmentRuntimeError::OpenFoldMissing)?;
                if *actual != ordinal {
                    return Err(SegmentRuntimeError::OrdinalMismatch {
                        expected: ordinal,
                        actual: *actual,
                    });
                }
                if frame.facts.hour_index.is_some() {
                    fold.apply_complete(frame)
                        .map_err(SegmentRuntimeError::Fold)?;
                } else {
                    fold.retain_failed(frame, "incomplete logical frame")
                        .map_err(SegmentRuntimeError::Fold)?;
                }
            }
            FrameRouting::UncertainSeal { sealed } => {
                if let Some(extent) = sealed {
                    self.seal_exact(extent)?;
                }
                self.factory
                    .retain_unassigned_failed(&self.ticket, frame, "reload uncertainty")
                    .map_err(SegmentRuntimeError::Fold)?;
            }
            FrameRouting::HeldUncertain => self
                .factory
                .retain_unassigned_failed(&self.ticket, frame, "reload uncertainty hold")
                .map_err(SegmentRuntimeError::Fold)?,
        }
        Ok(())
    }

    fn seal_exact(&mut self, extent: SealedSegment) -> Result<(), SegmentRuntimeError> {
        let (ordinal, fold) = self
            .open
            .take()
            .ok_or(SegmentRuntimeError::OpenFoldMissing)?;
        if ordinal != extent.segment_ordinal {
            return Err(SegmentRuntimeError::OrdinalMismatch {
                expected: extent.segment_ordinal,
                actual: ordinal,
            });
        }
        let input = fold.seal(extent).map_err(SegmentRuntimeError::Fold)?;
        self.sealed.push(input);
        Ok(())
    }

    /// Seal the tail and return immutable stitch inputs.  The runtime is
    /// consumed, so no frame can race publication after this point.
    pub fn finish(mut self) -> Result<Vec<SegmentInput>, SegmentRuntimeError> {
        if self.poisoned {
            return Err(SegmentRuntimeError::Poisoned);
        }
        if let Some(extent) = self.cutter.finish() {
            self.seal_exact(extent)?;
        } else if self.open.is_some() {
            return Err(SegmentRuntimeError::OpenFoldMissing);
        }
        Ok(self.sealed)
    }

    /// The sole live-driver composition call from staged folds to the atomic
    /// stitched publisher.
    pub fn finish_and_publish<S: StitchedPayloadSource>(
        self,
        game_id: &str,
        required_inputs: StitchRequiredInputs,
        source: &S,
        destination: &Path,
    ) -> Result<ProductStitchOutcome, String> {
        let segments = self.finish().map_err(|error| error.to_string())?;
        publish_product_stitch(game_id, required_inputs, segments, source, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct LoggingFold {
        ordinal: u64,
        log: Rc<RefCell<Vec<String>>>,
    }

    impl SegmentFold<()> for LoggingFold {
        fn apply_complete(&mut self, frame: StagedFrame<()>) -> Result<(), String> {
            self.log.borrow_mut().push(format!(
                "apply:{}:{:?}",
                self.ordinal, frame.facts.hour_index
            ));
            Ok(())
        }

        fn retain_failed(&mut self, _frame: StagedFrame<()>, reason: &str) -> Result<(), String> {
            self.log
                .borrow_mut()
                .push(format!("failed:{}:{reason}", self.ordinal));
            Ok(())
        }

        fn seal(self, extent: SealedSegment) -> Result<SegmentInput, String> {
            self.log
                .borrow_mut()
                .push(format!("seal:{}", extent.segment_ordinal));
            Err("fixture stops after proving seal order".to_string())
        }
    }

    struct LoggingFactory(Rc<RefCell<Vec<String>>>);

    impl SegmentFoldFactory<()> for LoggingFactory {
        type Fold = LoggingFold;

        fn open_fresh(
            &mut self,
            _ticket: &AppendTicket,
            ordinal: u64,
            _first: &FrameFacts,
        ) -> Result<Self::Fold, String> {
            self.0.borrow_mut().push(format!("open:{ordinal}"));
            Ok(LoggingFold {
                ordinal,
                log: Rc::clone(&self.0),
            })
        }

        fn retain_unassigned_failed(
            &mut self,
            _ticket: &AppendTicket,
            _frame: StagedFrame<()>,
            reason: &str,
        ) -> Result<(), String> {
            self.0.borrow_mut().push(format!("unassigned:{reason}"));
            Ok(())
        }
    }

    fn frame(hour: i64, sequence: u64, byte: u64) -> StagedFrame<()> {
        StagedFrame {
            facts: FrameFacts {
                session_identity: "session".to_string(),
                epoch_token: "epoch".to_string(),
                hour_index: Some(hour),
                reload_uncertain: false,
                first_record_sequence: sequence,
                first_byte_offset: byte,
                last_record_sequence: sequence + 1,
                end_byte_offset: byte + 100,
            },
            payload: (),
        }
    }

    fn ticket() -> AppendTicket {
        AppendTicket {
            campaign_key: b"campaign".to_vec(),
            play_order: 0,
            session_nonce: "session".to_string(),
            process_start_identity: "process".to_string(),
        }
    }

    #[test]
    fn rollback_seals_before_any_fresh_fold_and_failure_poisons() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut runtime = SegmentRuntime::new(ticket(), LoggingFactory(Rc::clone(&log)));
        runtime.stage(frame(10, 0, 0)).unwrap();
        assert!(runtime.stage(frame(9, 2, 100)).is_err());
        assert_eq!(
            &*log.borrow(),
            &vec![
                "open:0".to_string(),
                "apply:0:Some(10)".to_string(),
                "seal:0".to_string(),
            ]
        );
        assert!(matches!(
            runtime.stage(frame(11, 4, 200)),
            Err(SegmentRuntimeError::Poisoned)
        ));
    }

    #[test]
    fn append_ledger_is_dense_idempotent_and_campaign_local() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-append-ledger-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ledger = DurableAppendLedger::new(root.clone());
        let first = ledger
            .allocate(b"campaign-a", "session-a", "proc-a")
            .unwrap();
        assert_eq!(first.play_order, 0);
        assert_eq!(
            ledger
                .allocate(b"campaign-a", "session-a", "proc-a")
                .unwrap(),
            first
        );
        let second = ledger
            .allocate(b"campaign-a", "session-b", "proc-b")
            .unwrap();
        assert_eq!(second.play_order, 1);
        let other = ledger
            .allocate(b"campaign-b", "session-a", "proc-a")
            .unwrap();
        assert_eq!(other.play_order, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn append_ledger_rejects_truncation_and_identity_collision() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-append-ledger-corrupt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ledger = DurableAppendLedger::new(root.clone());
        ledger.allocate(b"campaign", "session", "proc").unwrap();
        assert!(ledger
            .allocate(b"campaign", "session", "different")
            .is_err());
        assert_eq!(
            ledger
                .allocate(b"campaign", "different-session", "proc")
                .unwrap()
                .play_order,
            1
        );
        let stem = lower_hex(&Sha256::digest(b"campaign"));
        let path = root.join(format!("{stem}.append-ledger-v1"));
        let mut bytes = fs::read(&path).unwrap();
        bytes.pop();
        fs::write(&path, bytes).unwrap();
        assert!(ledger.allocate(b"campaign", "next", "next-proc").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
