//! Immutable executable metadata loaded from a verified installation.

pub mod auxiliary;
pub mod commands;
pub mod keywords;
pub mod numeric;
pub mod pe;
pub mod registries;
pub mod variables;

pub use pe::PeImage;
pub use registries::TriggerReader;

use crate::build::{ExecutableIdentity, PINNED_BUILD};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub static EXECUTABLE_TABLES: OnceLock<Arc<ExecutableTables>> = OnceLock::new();
static INITIALIZATION: Mutex<()> = Mutex::new(());
static TEST_INITIALIZATION: OnceLock<io::Result<Arc<ExecutableTables>>> = OnceLock::new();

/// A shared metadata view that can be retried after an uninitialized access.
pub struct RuntimeTable<T> {
    value: OnceLock<T>,
    initialize: fn() -> T,
}

impl<T> RuntimeTable<T> {
    pub const fn new(initialize: fn() -> T) -> Self {
        Self {
            value: OnceLock::new(),
            initialize,
        }
    }
}

impl<T> std::ops::Deref for RuntimeTable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.get_or_init(self.initialize)
    }
}

pub struct ExecutableTables {
    identity: ExecutableIdentity,
    pub keywords: keywords::KeywordTables,
    pub registries: registries::RegistryTables,
    pub auxiliary: auxiliary::AuxiliaryTables,
    pub variables: variables::VariableTables,
    pub numeric: numeric::NumericTables,
    pub commands: commands::CommandTables,
}

impl ExecutableTables {
    pub fn identity(&self) -> &ExecutableIdentity {
        &self.identity
    }
}

/// Verifies the selected installation before publishing its immutable metadata.
pub fn initialize(game_dir: &Path) -> io::Result<Arc<ExecutableTables>> {
    initialize_from(&game_dir.join("hoi4.exe"), game_dir)
}

/// Uses the directory containing the selected executable for shipped documentation.
pub fn initialize_executable(exe_path: &Path) -> io::Result<Arc<ExecutableTables>> {
    let game_dir = exe_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "executable path has no directory",
        )
    })?;
    initialize_from(exe_path, game_dir)
}

fn initialize_from(exe_path: &Path, game_dir: &Path) -> io::Result<Arc<ExecutableTables>> {
    let mut source = File::open(exe_path)?;
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes)?;
    let identity = verify_executable(&bytes)?;
    // The guard only serializes publication; failed decoding leaves no shared partial tables.
    let _guard = INITIALIZATION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(tables) = EXECUTABLE_TABLES.get() {
        variables::VariableTables::verify(game_dir)?;
        return Ok(Arc::clone(tables));
    }
    let image = PeImage::parse(bytes)?;
    let keywords = keywords::KeywordTables::decode(&image)?;
    let registries = registries::RegistryTables::decode(&image, &keywords)?;
    let auxiliary = auxiliary::AuxiliaryTables::decode(&image, &keywords)?;
    let variables = variables::VariableTables::decode(game_dir)?;
    let numeric = numeric::NumericTables::decode(&image, &keywords)?;
    let commands = commands::CommandTables::decode(&image)?;
    let tables = Arc::new(ExecutableTables {
        identity,
        keywords,
        registries,
        auxiliary,
        variables,
        numeric,
        commands,
    });
    EXECUTABLE_TABLES
        .set(Arc::clone(&tables))
        .map_err(|_| io::Error::other("executable metadata was published during initialization"))?;
    Ok(tables)
}

pub fn get() -> &'static ExecutableTables {
    EXECUTABLE_TABLES
        .get()
        .expect("executable metadata is uninitialized; call engine::executable_tables::initialize with the selected game directory before using installation-dependent features")
        .as_ref()
}

pub fn is_initialized() -> bool {
    EXECUTABLE_TABLES.get().is_some()
}

/// Explicit installation opt-in for dependent crate tests.
pub fn initialize_for_tests() -> io::Result<Arc<ExecutableTables>> {
    let game_dir = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "BATTLE_PLAN_GAME_DIR is unset; executable metadata tests require an explicit supported installation",
        )
    })?;
    if let Some(tables) = EXECUTABLE_TABLES.get() {
        return Ok(Arc::clone(tables));
    }
    TEST_INITIALIZATION
        .get_or_init(|| initialize(Path::new(&game_dir)))
        .as_ref()
        .map(Arc::clone)
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

fn verify_executable(bytes: &[u8]) -> io::Result<ExecutableIdentity> {
    let identity = identity_of(bytes)?;
    if !PINNED_BUILD.matches(&identity) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            UnsupportedExecutable {
                expected_size: PINNED_BUILD.exe_size,
                actual_size: identity.size,
                expected_sha256: PINNED_BUILD.exe_sha256,
                actual_sha256: identity.sha256,
            },
        ));
    }
    Ok(identity)
}

fn identity_of(bytes: &[u8]) -> io::Result<ExecutableIdentity> {
    let size = u64::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "executable size exceeds u64"))?;
    Ok(ExecutableIdentity {
        size,
        sha256: Sha256::digest(bytes).into(),
    })
}

#[derive(Debug)]
pub struct UnsupportedExecutable {
    pub expected_size: u64,
    pub actual_size: u64,
    pub expected_sha256: [u8; 32],
    pub actual_sha256: [u8; 32],
}

impl fmt::Display for UnsupportedExecutable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported executable: expected {} bytes with SHA-256 {}, received {} bytes with SHA-256 {}",
            self.expected_size,
            hex_digest(&self.expected_sha256),
            self.actual_size,
            hex_digest(&self.actual_sha256),
        )
    }
}

impl std::error::Error for UnsupportedExecutable {}

/// Runtime names remain valid for the process lifetime of the published tables.
pub fn leak_string(value: impl Into<String>) -> &'static str {
    Box::leak(value.into().into_boxed_str())
}

#[derive(Debug)]
pub struct TableDigestMismatch {
    pub table: String,
    pub bytes: usize,
    pub expected_sha256: [u8; 32],
    pub actual_sha256: [u8; 32],
}

impl fmt::Display for TableDigestMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} metadata digest mismatch for {} canonical bytes: expected SHA-256 {}, received {}",
            self.table,
            self.bytes,
            hex_digest(&self.expected_sha256),
            hex_digest(&self.actual_sha256),
        )
    }
}

impl std::error::Error for TableDigestMismatch {}

pub fn verify_digest(name: &str, canonical_bytes: &[u8], expected_hex: &str) -> io::Result<()> {
    let expected_sha256 = parse_digest(expected_hex).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} expected metadata digest is not a SHA-256 hexadecimal string"),
        )
    })?;
    let actual_sha256: [u8; 32] = Sha256::digest(canonical_bytes).into();
    if actual_sha256 != expected_sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            TableDigestMismatch {
                table: name.to_owned(),
                bytes: canonical_bytes.len(),
                expected_sha256,
                actual_sha256,
            },
        ));
    }
    Ok(())
}

fn parse_digest(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut result = [0; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        result[index] = hex_digit(pair[0])?.checked_mul(16)? + hex_digit(pair[1])?;
    }
    Some(result)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_digest(bytes: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(64);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing a digest to a String cannot fail");
    }
    text
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod runtime_table_tests {
    use super::RuntimeTable;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn failed_view_initialization_can_be_retried_and_then_is_shared() {
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        let table = RuntimeTable::new(|| {
            let attempt = ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            assert_ne!(attempt, 0, "first metadata view initialization fails");
            7usize
        });
        assert!(std::panic::catch_unwind(|| *table).is_err());
        assert_eq!(*table, 7);
        assert_eq!(*table, 7);
        assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 2);
    }
}
