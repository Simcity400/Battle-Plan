//! Portable ownership boundary for compact-stream decoders.
//!
//! The portable format crate owns the request/result contract, but deliberately
//! does not contain an H4CR parser or a world fold. A producer implementation
//! must consume the complete verified raw stream, certify physical EOF and the
//! closed-hour boundary, and only then return [`crate::PrefoldedReplay`]. This
//! keeps the one production fold in its owning distribution while allowing a
//! standalone replay consumer to depend on this neutral contract rather than
//! an extractor library.

use crate::PrefoldedReplay;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Manifest role of the canonical compact record stream in a replay container.
pub const COMPACT_STREAM_ROLE: &str = "collector_compact_stream";

/// Framing mode selected from authenticated recorder metadata before a
/// complete compact stream is handed to a production decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefoldStreamIdentityMode {
    Legacy,
    IdentityV1,
}

/// Owned input to one complete certified pre-fold operation.
///
/// Ownership is intentional: a container reader can transfer its verified
/// entry bytes without an additional full-stream copy, and no borrowed prefix
/// can escape while certification is still in progress.
#[derive(Debug)]
pub struct CertifiedPrefoldRequest {
    game_id: String,
    raw_h4cr: Vec<u8>,
    source_sha256: String,
    identity_mode: PrefoldStreamIdentityMode,
}

impl CertifiedPrefoldRequest {
    /// Construct a request over physically raw H4CR bytes.
    ///
    /// ZIP/zstd decoding belongs to the authenticated container/file boundary;
    /// this seam receives the resulting raw record stream only.
    pub fn try_new(
        game_id: impl Into<String>,
        raw_h4cr: Vec<u8>,
    ) -> Result<Self, PrefoldDecodeError> {
        Self::try_new_with_identity_mode(game_id, raw_h4cr, PrefoldStreamIdentityMode::Legacy)
    }

    pub fn try_new_with_identity_mode(
        game_id: impl Into<String>,
        raw_h4cr: Vec<u8>,
        identity_mode: PrefoldStreamIdentityMode,
    ) -> Result<Self, PrefoldDecodeError> {
        let game_id = game_id.into();
        if game_id.trim().is_empty() {
            return Err(PrefoldDecodeError::new("prefold request game_id is empty"));
        }
        if !raw_h4cr.starts_with(b"H4CR") {
            return Err(PrefoldDecodeError::new(
                "prefold request is not a physically raw H4CR stream",
            ));
        }
        let source_sha256 = crate::sha256::to_hex(&crate::sha256::Sha256::digest(&raw_h4cr));
        Ok(Self {
            game_id,
            raw_h4cr,
            source_sha256,
            identity_mode,
        })
    }

    pub fn game_id(&self) -> &str {
        &self.game_id
    }

    pub fn raw_h4cr(&self) -> &[u8] {
        &self.raw_h4cr
    }

    pub fn source_len(&self) -> u64 {
        self.raw_h4cr.len() as u64
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn identity_mode(&self) -> PrefoldStreamIdentityMode {
        self.identity_mode
    }

    pub fn into_parts(self) -> (String, Vec<u8>) {
        (self.game_id, self.raw_h4cr)
    }
}

/// A decoder that exposes no folded prefix and returns only a fully certified,
/// immutable replay.
///
/// Implementations MUST drive their canonical production fold through physical
/// EOF and reject malformed framing, payload, sequence, and hour boundaries.
/// Returning a successfully decoded prefix is a contract violation. The
/// portable result type independently rechecks game/schema/hour identity.
pub trait CertifiedPrefoldDecoder: Send + Sync {
    /// Stable diagnostic identity for errors and release manifests.
    fn decoder_id(&self) -> &'static str;

    /// Consume and certify the entire request before returning any hours.
    fn decode_certified(
        &self,
        request: CertifiedPrefoldRequest,
    ) -> Result<PrefoldedReplay, PrefoldDecodeError>;
}

/// Invoke a decoder and mechanically bind its result back to the requested
/// game identity. Consumers should use this function instead of calling the
/// trait method directly.
pub fn decode_certified_prefolded(
    decoder: &dyn CertifiedPrefoldDecoder,
    request: CertifiedPrefoldRequest,
) -> Result<CertifiedPrefoldedReplay, PrefoldDecodeError> {
    let expected_game_id = request.game_id().to_string();
    let source_len = request.source_len();
    let source_sha256 = request.source_sha256().to_string();
    let identity_mode = request.identity_mode();
    let replay = decoder.decode_certified(request)?;
    if replay.index().game_id() != expected_game_id {
        return Err(PrefoldDecodeError::new(format!(
            "decoder {:?} returned game_id {:?} for request {:?}",
            decoder.decoder_id(),
            replay.index().game_id(),
            expected_game_id
        )));
    }
    Ok(CertifiedPrefoldedReplay {
        replay,
        receipt: PrefoldSourceReceipt {
            decoder_id: decoder.decoder_id().to_string(),
            source_len,
            source_sha256,
            identity_mode,
        },
    })
}

/// Immutable source binding minted by the portable invocation wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefoldSourceReceipt {
    decoder_id: String,
    source_len: u64,
    source_sha256: String,
    identity_mode: PrefoldStreamIdentityMode,
}

impl PrefoldSourceReceipt {
    pub fn decoder_id(&self) -> &str {
        &self.decoder_id
    }

    pub fn source_len(&self) -> u64 {
        self.source_len
    }

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn identity_mode(&self) -> PrefoldStreamIdentityMode {
        self.identity_mode
    }
}

/// A pre-folded replay bound to the exact verified stream bytes and decoder
/// capability that produced it.
#[derive(Debug, Clone)]
pub struct CertifiedPrefoldedReplay {
    replay: PrefoldedReplay,
    receipt: PrefoldSourceReceipt,
}

impl CertifiedPrefoldedReplay {
    pub fn replay(&self) -> &PrefoldedReplay {
        &self.replay
    }

    pub fn receipt(&self) -> &PrefoldSourceReceipt {
        &self.receipt
    }

    pub fn into_replay(self) -> PrefoldedReplay {
        self.replay
    }
}

impl std::ops::Deref for CertifiedPrefoldedReplay {
    type Target = PrefoldedReplay;

    fn deref(&self) -> &Self::Target {
        &self.replay
    }
}

/// Owned request for the bounded, disk-backed variant of the portable decoder
/// seam. `raw_h4cr_path` names a private, already authenticated spool file and
/// `snapshot_dir` is an empty private directory owned by the caller.
#[derive(Debug)]
pub struct CertifiedPrefoldSpoolRequest {
    game_id: String,
    raw_h4cr_path: PathBuf,
    snapshot_dir: PathBuf,
    source_len: u64,
    source_sha256: String,
    identity_mode: PrefoldStreamIdentityMode,
}

impl CertifiedPrefoldSpoolRequest {
    pub fn try_new(
        game_id: impl Into<String>,
        raw_h4cr_path: impl Into<PathBuf>,
        snapshot_dir: impl Into<PathBuf>,
        source_len: u64,
        source_sha256: impl Into<String>,
    ) -> Result<Self, PrefoldDecodeError> {
        Self::try_new_with_identity_mode(
            game_id,
            raw_h4cr_path,
            snapshot_dir,
            source_len,
            source_sha256,
            PrefoldStreamIdentityMode::Legacy,
        )
    }

    pub fn try_new_with_identity_mode(
        game_id: impl Into<String>,
        raw_h4cr_path: impl Into<PathBuf>,
        snapshot_dir: impl Into<PathBuf>,
        source_len: u64,
        source_sha256: impl Into<String>,
        identity_mode: PrefoldStreamIdentityMode,
    ) -> Result<Self, PrefoldDecodeError> {
        let game_id = game_id.into();
        let raw_h4cr_path = raw_h4cr_path.into();
        let snapshot_dir = snapshot_dir.into();
        let source_sha256 = source_sha256.into();
        if game_id.trim().is_empty() {
            return Err(PrefoldDecodeError::new("spool prefold request game_id is empty"));
        }
        if source_len < 4 {
            return Err(PrefoldDecodeError::new(
                "spool prefold request source is shorter than H4CR magic",
            ));
        }
        if !is_lower_sha256(&source_sha256) {
            return Err(PrefoldDecodeError::new(
                "spool prefold request source_sha256 is not lowercase SHA-256 hex",
            ));
        }
        if raw_h4cr_path.as_os_str().is_empty() || snapshot_dir.as_os_str().is_empty() {
            return Err(PrefoldDecodeError::new(
                "spool prefold request paths must be nonempty",
            ));
        }
        if raw_h4cr_path == snapshot_dir {
            return Err(PrefoldDecodeError::new(
                "spool prefold source path and snapshot directory are identical",
            ));
        }
        Ok(Self {
            game_id,
            raw_h4cr_path,
            snapshot_dir,
            source_len,
            source_sha256,
            identity_mode,
        })
    }

    pub fn game_id(&self) -> &str {
        &self.game_id
    }
    pub fn raw_h4cr_path(&self) -> &Path {
        &self.raw_h4cr_path
    }
    pub fn snapshot_dir(&self) -> &Path {
        &self.snapshot_dir
    }
    pub fn source_len(&self) -> u64 {
        self.source_len
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn identity_mode(&self) -> PrefoldStreamIdentityMode {
        self.identity_mode
    }
    pub fn into_parts(self) -> (String, PathBuf, PathBuf, u64, String) {
        (
            self.game_id,
            self.raw_h4cr_path,
            self.snapshot_dir,
            self.source_len,
            self.source_sha256,
        )
    }
}

/// One immutable file in a certified disk-backed pre-fold result. Only a
/// single normal path component is admitted, preventing a decoder from
/// escaping the caller-owned snapshot directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefoldedSpoolEntry {
    hour_index: i64,
    date: String,
    file_name: String,
    byte_len: u64,
    sha256: String,
}

impl PrefoldedSpoolEntry {
    pub fn try_new(
        hour_index: i64,
        date: impl Into<String>,
        file_name: impl Into<String>,
        byte_len: u64,
        sha256: impl Into<String>,
    ) -> Result<Self, PrefoldDecodeError> {
        let date = date.into();
        let file_name = file_name.into();
        let sha256 = sha256.into();
        let mut components = Path::new(&file_name).components();
        let is_leaf = matches!(components.next(), Some(Component::Normal(_)))
            && components.next().is_none();
        if !is_leaf
            || file_name.contains('/')
            || file_name.contains('\\')
            || crate::zip::validate_path(&file_name).is_err()
        {
            return Err(PrefoldDecodeError::new(format!(
                "spooled hour {hour_index} has unsafe file name {file_name:?}"
            )));
        }
        if date.trim().is_empty() || byte_len == 0 || !is_lower_sha256(&sha256) {
            return Err(PrefoldDecodeError::new(format!(
                "spooled hour {hour_index} has invalid date/length/SHA metadata"
            )));
        }
        Ok(Self {
            hour_index,
            date,
            file_name,
            byte_len,
            sha256,
        })
    }

    pub fn hour_index(&self) -> i64 { self.hour_index }
    pub fn date(&self) -> &str { &self.date }
    pub fn file_name(&self) -> &str { &self.file_name }
    pub fn byte_len(&self) -> u64 { self.byte_len }
    pub fn sha256(&self) -> &str { &self.sha256 }
}

/// Portable manifest for lazily loaded, disk-backed canonical snapshots.
#[derive(Debug, Clone)]
pub struct PrefoldedSpool {
    game_id: String,
    schema_version: Value,
    entries: Vec<PrefoldedSpoolEntry>,
    hours: Vec<i64>,
}

impl PrefoldedSpool {
    pub fn try_new(
        game_id: impl Into<String>,
        schema_version: Value,
        entries: Vec<PrefoldedSpoolEntry>,
    ) -> Result<Self, PrefoldDecodeError> {
        let game_id = game_id.into();
        if game_id.trim().is_empty() || entries.is_empty() {
            return Err(PrefoldDecodeError::new(
                "spooled prefold requires a game id and at least one hour",
            ));
        }
        let mut previous = None;
        let mut hours = Vec::with_capacity(entries.len());
        let mut file_names = std::collections::BTreeSet::new();
        for entry in &entries {
            if previous.is_some_and(|hour| entry.hour_index <= hour) {
                return Err(PrefoldDecodeError::new(format!(
                    "spooled prefold hours are not strictly increasing at {}",
                    entry.hour_index
                )));
            }
            previous = Some(entry.hour_index);
            if !file_names.insert(entry.file_name.clone()) {
                return Err(PrefoldDecodeError::new(format!(
                    "spooled prefold repeats file name {:?}",
                    entry.file_name
                )));
            }
            hours.push(entry.hour_index);
        }
        Ok(Self { game_id, schema_version, entries, hours })
    }

    pub fn game_id(&self) -> &str { &self.game_id }
    pub fn schema_version(&self) -> &Value { &self.schema_version }
    pub fn entries(&self) -> &[PrefoldedSpoolEntry] { &self.entries }
    pub fn hours(&self) -> &[i64] { &self.hours }
    pub fn entry(&self, hour_index: i64) -> Option<&PrefoldedSpoolEntry> {
        self.hours.binary_search(&hour_index).ok().map(|index| &self.entries[index])
    }
}

/// Bounded decoder capability. Implementations must authenticate the complete
/// source file against the request receipt, drive the one production fold to a
/// certified EOF, and spill exactly one immutable snapshot per closed hour.
pub trait CertifiedPrefoldSpoolDecoder: Send + Sync {
    fn decoder_id(&self) -> &'static str;
    fn decode_certified_spool(
        &self,
        request: CertifiedPrefoldSpoolRequest,
    ) -> Result<PrefoldedSpool, PrefoldDecodeError>;
}

#[derive(Debug)]
pub struct CertifiedPrefoldedSpool {
    spool: PrefoldedSpool,
    snapshot_dir: PathBuf,
    receipt: PrefoldSourceReceipt,
}

impl CertifiedPrefoldedSpool {
    pub fn spool(&self) -> &PrefoldedSpool { &self.spool }
    pub fn snapshot_dir(&self) -> &Path { &self.snapshot_dir }
    pub fn receipt(&self) -> &PrefoldSourceReceipt { &self.receipt }
    pub fn into_parts(self) -> (PrefoldedSpool, PathBuf, PrefoldSourceReceipt) {
        (self.spool, self.snapshot_dir, self.receipt)
    }
}

pub fn decode_certified_prefolded_spool(
    decoder: &dyn CertifiedPrefoldSpoolDecoder,
    request: CertifiedPrefoldSpoolRequest,
) -> Result<CertifiedPrefoldedSpool, PrefoldDecodeError> {
    let expected_game_id = request.game_id().to_string();
    let snapshot_dir = request.snapshot_dir().to_path_buf();
    let source_len = request.source_len();
    let source_sha256 = request.source_sha256().to_string();
    let identity_mode = request.identity_mode();
    let spool = decoder.decode_certified_spool(request)?;
    if spool.game_id() != expected_game_id {
        return Err(PrefoldDecodeError::new(format!(
            "decoder {:?} returned game_id {:?} for spool request {:?}",
            decoder.decoder_id(), spool.game_id(), expected_game_id
        )));
    }
    Ok(CertifiedPrefoldedSpool {
        spool,
        snapshot_dir,
        receipt: PrefoldSourceReceipt {
            decoder_id: decoder.decoder_id().to_string(),
            source_len,
            source_sha256,
            identity_mode,
        },
    })
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefoldDecodeError {
    message: String,
}

impl PrefoldDecodeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for PrefoldDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PrefoldDecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    struct WrongGameDecoder;

    struct IdentityV1Decoder;

    impl CertifiedPrefoldDecoder for WrongGameDecoder {
        fn decoder_id(&self) -> &'static str {
            "test.wrong-game"
        }

        fn decode_certified(
            &self,
            _request: CertifiedPrefoldRequest,
        ) -> Result<PrefoldedReplay, PrefoldDecodeError> {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "game_id": "other",
                "schema_version": "1",
                "hour_index": 1
            }))
            .unwrap();
            let entry = crate::PrefoldedHourEntry::from_json(1, &bytes)
                .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            PrefoldedReplay::try_new(
                "other",
                serde_json::Value::String("1".into()),
                vec![entry],
            )
            .map_err(|error| PrefoldDecodeError::new(error.to_string()))
        }
    }

    impl CertifiedPrefoldDecoder for IdentityV1Decoder {
        fn decoder_id(&self) -> &'static str {
            "test.identity-v1"
        }

        fn decode_certified(
            &self,
            request: CertifiedPrefoldRequest,
        ) -> Result<PrefoldedReplay, PrefoldDecodeError> {
            assert_eq!(request.identity_mode(), PrefoldStreamIdentityMode::IdentityV1);
            let (game_id, _) = request.into_parts();
            let bytes = serde_json::to_vec(&serde_json::json!({
                "game_id": game_id.clone(),
                "schema_version": "1",
                "hour_index": 1
            }))
            .unwrap();
            let entry = crate::PrefoldedHourEntry::from_json(1, &bytes)
                .map_err(|error| PrefoldDecodeError::new(error.to_string()))?;
            PrefoldedReplay::try_new(
                game_id,
                serde_json::Value::String("1".into()),
                vec![entry],
            )
            .map_err(|error| PrefoldDecodeError::new(error.to_string()))
        }
    }

    #[test]
    fn request_requires_game_identity_and_raw_h4cr() {
        assert!(CertifiedPrefoldRequest::try_new("", b"H4CR".to_vec()).is_err());
        assert!(CertifiedPrefoldRequest::try_new("g", b"PK\x03\x04".to_vec()).is_err());
        let request = CertifiedPrefoldRequest::try_new("g", b"H4CRbytes".to_vec()).unwrap();
        assert_eq!(request.game_id(), "g");
        assert_eq!(request.raw_h4cr(), b"H4CRbytes");
        assert_eq!(request.source_len(), 9);
        assert_eq!(request.source_sha256().len(), 64);
        assert_eq!(request.identity_mode(), PrefoldStreamIdentityMode::Legacy);
    }

    #[test]
    fn explicit_identity_mode_survives_decoder_and_receipt_boundary() {
        let request = CertifiedPrefoldRequest::try_new_with_identity_mode(
            "identity-game",
            b"H4CRbytes".to_vec(),
            PrefoldStreamIdentityMode::IdentityV1,
        )
        .unwrap();
        let certified = decode_certified_prefolded(&IdentityV1Decoder, request).unwrap();
        assert_eq!(
            certified.receipt().identity_mode(),
            PrefoldStreamIdentityMode::IdentityV1
        );
    }

    #[test]
    fn decode_wrapper_rebinds_result_to_requested_game() {
        let request = CertifiedPrefoldRequest::try_new("wanted", b"H4CRbytes".to_vec()).unwrap();
        let error = decode_certified_prefolded(&WrongGameDecoder, request).unwrap_err();
        assert!(error.message().contains("returned game_id"));
        assert!(error.message().contains("wanted"));
    }

    #[test]
    fn spool_manifest_rejects_path_escape_duplicate_files_and_hour_order() {
        let make = |hour, file: &str| {
            PrefoldedSpoolEntry::try_new(
                hour,
                "1936.1.1.12",
                file,
                10,
                "a".repeat(64),
            )
        };
        for unsafe_name in ["../x.json", "a/b.json", "a\\b.json", "CON"] {
            assert!(make(1, unsafe_name).is_err(), "accepted {unsafe_name:?}");
        }
        assert!(PrefoldedSpool::try_new(
            "g",
            Value::String("1".into()),
            vec![make(1, "one.json").unwrap(), make(2, "one.json").unwrap()],
        )
        .is_err());
        assert!(PrefoldedSpool::try_new(
            "g",
            Value::String("1".into()),
            vec![make(2, "two.json").unwrap(), make(1, "one.json").unwrap()],
        )
        .is_err());
    }
}
