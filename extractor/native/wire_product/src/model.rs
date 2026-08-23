use compact_contract::stream_validator::StreamIdentityMode;
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

pub const ROOT: &str = "rebuilt_wire";
pub const BUNDLE_PATH: &str = "rebuilt_wire/bundle.bin";
pub const BRANCHES_PATH: &str = "rebuilt_wire/branches.bin";
pub const SCHEMAS_PATH: &str = "rebuilt_wire/schemas.bin";
pub const SYMBOLS_PATH: &str = "rebuilt_wire/symbols.json";
pub const PROVINCES_PATH: &str = "rebuilt_wire/provinces.bin";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ArtifactRole {
    BundleMetadata = 0,
    BranchTable = 1,
    SchemaRegistry = 2,
    SymbolManifest = 3,
    StateBlock = 4,
    GapRecord = 5,
    ProvinceCatalog = 6,
}

impl ArtifactRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BundleMetadata => "rebuilt_wire_bundle_metadata",
            Self::BranchTable => "rebuilt_wire_branch_table",
            Self::SchemaRegistry => "rebuilt_wire_schema_registry",
            Self::SymbolManifest => "rebuilt_wire_symbol_manifest",
            Self::StateBlock => "rebuilt_wire_state_block",
            Self::GapRecord => "rebuilt_wire_gap_record",
            Self::ProvinceCatalog => "rebuilt_wire_province_catalog",
        }
    }

    pub fn from_str(value: &str) -> Result<Self, ProductError> {
        match value {
            "rebuilt_wire_bundle_metadata" => Ok(Self::BundleMetadata),
            "rebuilt_wire_branch_table" => Ok(Self::BranchTable),
            "rebuilt_wire_schema_registry" => Ok(Self::SchemaRegistry),
            "rebuilt_wire_symbol_manifest" => Ok(Self::SymbolManifest),
            "rebuilt_wire_state_block" => Ok(Self::StateBlock),
            "rebuilt_wire_gap_record" => Ok(Self::GapRecord),
            "rebuilt_wire_province_catalog" => Ok(Self::ProvinceCatalog),
            _ => Err(ProductError::new(format!(
                "unknown rebuilt-wire artifact role {value:?}"
            ))),
        }
    }

    pub(crate) fn from_code(code: u8) -> Result<Self, ProductError> {
        match code {
            0 => Ok(Self::BundleMetadata),
            1 => Ok(Self::BranchTable),
            2 => Ok(Self::SchemaRegistry),
            3 => Ok(Self::SymbolManifest),
            4 => Ok(Self::StateBlock),
            5 => Ok(Self::GapRecord),
            6 => Ok(Self::ProvinceCatalog),
            _ => Err(ProductError::new(format!(
                "unassigned rebuilt-wire artifact role code {code}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleEntry {
    pub path: String,
    pub role: ArtifactRole,
    pub bytes: Vec<u8>,
    pub len: u64,
    pub blake3: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleEntryDescriptor {
    pub path: String,
    pub role: ArtifactRole,
    pub len: u64,
    pub blake3: [u8; 32],
}

impl From<&BundleEntry> for BundleEntryDescriptor {
    fn from(entry: &BundleEntry) -> Self {
        Self {
            path: entry.path.clone(),
            role: entry.role,
            len: entry.len,
            blake3: entry.blake3,
        }
    }
}

impl From<&MaterializedEntry> for BundleEntryDescriptor {
    fn from(entry: &MaterializedEntry) -> Self {
        Self {
            path: entry.path.clone(),
            role: entry.role,
            len: entry.len,
            blake3: entry.blake3,
        }
    }
}

impl BundleEntry {
    pub fn from_bytes(
        path: impl Into<String>,
        role: ArtifactRole,
        bytes: Vec<u8>,
    ) -> Result<Self, ProductError> {
        Self::new(path, role, bytes)
    }

    pub fn new(
        path: impl Into<String>,
        role: ArtifactRole,
        bytes: Vec<u8>,
    ) -> Result<Self, ProductError> {
        let path = path.into();
        validate_entry_path(&path)?;
        let len = u64::try_from(bytes.len())
            .map_err(|_| ProductError::new("artifact length does not fit u64"))?;
        let blake3 = *blake3::hash(&bytes).as_bytes();
        Ok(Self {
            path,
            role,
            bytes,
            len,
            blake3,
        })
    }

    pub fn validate_declared_integrity(&self) -> Result<(), ProductError> {
        validate_entry_path(&self.path)?;
        let actual_len = u64::try_from(self.bytes.len())
            .map_err(|_| ProductError::new("artifact length does not fit u64"))?;
        if self.len != actual_len {
            return Err(ProductError::new(format!(
                "{} declared length {} does not match {} bytes",
                self.path, self.len, actual_len
            )));
        }
        let actual = *blake3::hash(&self.bytes).as_bytes();
        if self.blake3 != actual {
            return Err(ProductError::new(format!(
                "{} declared BLAKE3 does not match its bytes",
                self.path
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PreparedBundle {
    pub(crate) root: PathBuf,
    pub(crate) entries: Vec<MaterializedEntry>,
    seen_paths: BTreeSet<String>,
}

impl PreparedBundle {
    pub fn from_entries(entries: Vec<BundleEntry>) -> Result<Self, ProductError> {
        crate::verify_entries(&entries)?;
        let mut bundle = Self::create_spool(std::env::temp_dir().as_path())?;
        for entry in entries {
            bundle.push_bytes(entry.path, entry.role, &entry.bytes)?;
        }
        crate::verify_prepared_bundle(&bundle)?;
        Ok(bundle)
    }

    pub fn entries(&self) -> &[MaterializedEntry] {
        &self.entries
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn create_spool(parent: &Path) -> Result<Self, ProductError> {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductError::new(format!("create rebuilt-wire spool parent: {error}"))
        })?;
        Ok(Self {
            root: create_unique_materialization_root(parent)?,
            entries: Vec::new(),
            seen_paths: BTreeSet::new(),
        })
    }

    pub(crate) fn push_bytes(
        &mut self,
        path: impl Into<String>,
        role: ArtifactRole,
        bytes: &[u8],
    ) -> Result<(), ProductError> {
        let path = path.into();
        validate_entry_path(&path)?;
        validate_spool_admission(
            self.entries.len(),
            role,
            self.seen_paths.contains(&path),
            &path,
        )?;
        let file_path = self
            .root
            .join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ProductError::new(format!("create {}: {error}", parent.display()))
            })?;
        }
        std::fs::write(&file_path, bytes).map_err(|error| {
            ProductError::new(format!("write {}: {error}", file_path.display()))
        })?;
        if !self.seen_paths.insert(path.clone()) {
            return Err(ProductError::new(
                "rebuilt-wire path index changed during artifact admission",
            ));
        }
        self.entries.push(MaterializedEntry {
            path,
            role,
            file_path,
            len: u64::try_from(bytes.len())
                .map_err(|_| ProductError::new("artifact length does not fit u64"))?,
            blake3: *blake3::hash(bytes).as_bytes(),
        });
        Ok(())
    }

    pub fn materialize(&self, parent: &Path) -> Result<MaterializedBundle, ProductError> {
        crate::verify_prepared_bundle(self)?;
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductError::new(format!(
                "create materialization parent {}: {error}",
                parent.display()
            ))
        })?;
        let root = create_unique_materialization_root(parent)?;
        let mut materialized = MaterializedBundle {
            root,
            entries: Vec::with_capacity(self.entries.len()),
        };
        for entry in &self.entries {
            let file_path = materialized
                .root
                .join(entry.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            if file_path.exists() {
                return Err(ProductError::new(format!(
                    "refusing to replace existing materialized artifact {}",
                    file_path.display()
                )));
            }
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    ProductError::new(format!("create {}: {error}", parent.display()))
                })?;
            }
            std::fs::copy(&entry.file_path, &file_path).map_err(|error| {
                ProductError::new(format!(
                    "copy {} to {}: {error}",
                    entry.file_path.display(),
                    file_path.display()
                ))
            })?;
            materialized.entries.push(MaterializedEntry {
                path: entry.path.clone(),
                role: entry.role,
                file_path,
                len: entry.len,
                blake3: entry.blake3,
            });
        }
        Ok(materialized)
    }
}

fn validate_spool_admission(
    entry_count: usize,
    role: ArtifactRole,
    duplicate: bool,
    path: &str,
) -> Result<(), ProductError> {
    let limit_reached = if role == ArtifactRole::BundleMetadata {
        entry_count > crate::codec::MAX_ARTIFACTS
    } else {
        entry_count >= crate::codec::MAX_ARTIFACTS
    };
    if limit_reached {
        return Err(ProductError::new(format!(
            "rebuilt-wire spool exceeds the {} artifact production bound",
            crate::codec::MAX_ARTIFACTS
        )));
    }
    if duplicate {
        return Err(ProductError::new(format!(
            "rebuilt-wire spool repeats artifact {path}"
        )));
    }
    Ok(())
}

impl Drop for PreparedBundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedEntry {
    pub path: String,
    pub role: ArtifactRole,
    pub file_path: PathBuf,
    pub len: u64,
    pub blake3: [u8; 32],
}

#[derive(Debug)]
pub struct MaterializedBundle {
    pub root: PathBuf,
    pub entries: Vec<MaterializedEntry>,
}

impl MaterializedBundle {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for MaterializedBundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BranchParent {
    pub branch: u32,
    pub through_tick: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchSpec {
    pub branch: u32,
    pub parent: Option<BranchParent>,
    pub first_tick: u64,
    pub session_fingerprint: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BundleBuildConfig {
    pub block_hours: u64,
    pub require_complete_hub_simulation: bool,
}

impl Default for BundleBuildConfig {
    fn default() -> Self {
        Self {
            block_hours: 48,
            require_complete_hub_simulation: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum D81LeadingGapKind {
    CaptureFailed,
    Coalesced,
    ExplicitGap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D81LeadingGap {
    branch: u32,
    tick_start: u64,
    tick_end: u64,
    kind: D81LeadingGapKind,
}

impl D81LeadingGap {
    pub fn new(
        branch: u32,
        tick_start: u64,
        tick_end: u64,
        kind: D81LeadingGapKind,
    ) -> Result<Self, ProductError> {
        if tick_start >= tick_end {
            return Err(ProductError::new(
                "declared d81 leading gap is empty or reversed",
            ));
        }
        Ok(Self {
            branch,
            tick_start,
            tick_end,
            kind,
        })
    }

    pub const fn branch(&self) -> u32 {
        self.branch
    }

    pub const fn tick_start(&self) -> u64 {
        self.tick_start
    }

    pub const fn tick_end(&self) -> u64 {
        self.tick_end
    }

    pub const fn kind(&self) -> D81LeadingGapKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug)]
pub enum H4crSource<'a> {
    Bytes(&'a [u8]),
    File(&'a Path),
    RetainedFile(&'a std::fs::File),
    BytesSelection(&'a [u8], H4crSelection),
    FileSelection(&'a Path, H4crSelection),
    RetainedFileSelection(&'a std::fs::File, H4crSelection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct H4crSelection {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub start_byte: u64,
    pub end_byte: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct H4crSession<'a> {
    pub branch: u32,
    pub identity_mode: StreamIdentityMode,
    pub source: H4crSource<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D81SessionInspection {
    pub first_tick: u64,
    pub first_d81_tick: Option<u64>,
    pub last_tick: u64,
    pub decoded_sha256: [u8; 32],
    pub data_records: u64,
    pub publication_authoritative_records: u64,
    pub failure_records: u64,
    /// DATA records whose `PSD1` envelope refused reconstruction. Each such
    /// hour publishes as a status-bound gap rather than aborting the build;
    /// this count is the named exclusion a reader checks the product against.
    pub unreconstructed_records: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBranch {
    pub branch: u32,
    pub first_tick: u64,
    pub last_tick: u64,
    pub state_observations: u64,
    pub gap_hours: u64,
    pub blocks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBundle {
    pub ticks_per_hour_log2: u8,
    pub content_manifest_sha256: [u8; 32],
    pub definition_csv_sha256: [u8; 32],
    pub symbol_count: u64,
    pub schema_count: u64,
    pub branches: Vec<VerifiedBranch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductError {
    message: String,
}

impl ProductError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProductError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProductError {}

pub(crate) fn validate_entry_path(path: &str) -> Result<(), ProductError> {
    if path.is_empty()
        || !path.starts_with(&format!("{ROOT}/"))
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
    {
        return Err(ProductError::new(format!(
            "unsafe or noncanonical rebuilt-wire artifact path {path:?}"
        )));
    }
    Ok(())
}

fn create_unique_materialization_root(parent: &Path) -> Result<PathBuf, ProductError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    for _ in 0..1024 {
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".rebuilt_wire_product-{pid}-{ordinal}"));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ProductError::new(format!(
                    "create materialization root {}: {error}",
                    candidate.display()
                )))
            }
        }
    }
    Err(ProductError::new(
        "could not allocate a unique rebuilt-wire materialization root",
    ))
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn spool_artifact_bound_reserves_exactly_one_bundle_metadata_entry() {
        assert!(validate_spool_admission(
            crate::codec::MAX_ARTIFACTS - 1,
            ArtifactRole::StateBlock,
            false,
            "rebuilt_wire/blocks/last.bin",
        )
        .is_ok());
        assert!(validate_spool_admission(
            crate::codec::MAX_ARTIFACTS,
            ArtifactRole::StateBlock,
            false,
            "rebuilt_wire/blocks/overflow.bin",
        )
        .is_err());
        assert!(validate_spool_admission(
            crate::codec::MAX_ARTIFACTS,
            ArtifactRole::BundleMetadata,
            false,
            BUNDLE_PATH,
        )
        .is_ok());
        assert!(validate_spool_admission(
            crate::codec::MAX_ARTIFACTS + 1,
            ArtifactRole::BundleMetadata,
            false,
            BUNDLE_PATH,
        )
        .is_err());
    }

    #[test]
    fn spool_duplicate_path_is_rejected_by_the_index() {
        assert!(validate_spool_admission(
            1,
            ArtifactRole::StateBlock,
            true,
            "rebuilt_wire/blocks/duplicate.bin",
        )
        .unwrap_err()
        .to_string()
        .contains("repeats artifact"));
    }
}
