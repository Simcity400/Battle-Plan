//! Immutable executable metadata, each table located by its content and verified on its own.

pub mod auxiliary;
pub mod commands;
pub mod keywords;
pub mod numeric;
pub mod pe;
pub mod registries;
pub mod variables;

pub use pe::PeImage;
pub use registries::TriggerReader;

use crate::availability::{Verified, Withheld};
use crate::build::{self, ExecutableIdentity};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

pub static EXECUTABLE_TABLES: OnceLock<Arc<ExecutableTables>> = OnceLock::new();
static INITIALIZATION: Mutex<()> = Mutex::new(());
static TEST_INITIALIZATION: OnceLock<io::Result<Arc<ExecutableTables>>> = OnceLock::new();

/// A shared metadata view that can be retried after an uninitialized access.
pub struct RuntimeTable<T> {
    value: OnceLock<Verified<T>>,
    initialize: fn() -> Verified<T>,
}

impl<T> RuntimeTable<T> {
    pub const fn new(initialize: fn() -> Verified<T>) -> Self {
        Self {
            value: OnceLock::new(),
            initialize,
        }
    }

    pub fn get(&self) -> Result<&T, &Withheld> {
        self.value.get_or_init(self.initialize).as_ref()
    }
}

/// Reaching a withheld view stops its caller with the withheld reason, which the
/// content and product boundaries report by name.
impl<T> std::ops::Deref for RuntimeTable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        required(self.get())
    }
}

/// A table its caller cannot go on without; a withheld one stops the caller with its reason.
pub fn required<'a, T>(table: Result<&'a T, &Withheld>) -> &'a T {
    table.unwrap_or_else(|withheld| panic!("{withheld}"))
}

/// One part of the published auxiliary tables, or why it or the tables are withheld.
pub fn auxiliary_part<T>(
    select: impl FnOnce(&'static auxiliary::AuxiliaryTables) -> &'static Verified<T>,
) -> Result<&'static T, &'static Withheld> {
    select(get().auxiliary.as_ref()?).as_ref()
}

/// One field of a published table, as a view's initializer.
pub fn view<T: 'static, U: ?Sized + 'static>(
    table: impl FnOnce(&'static ExecutableTables) -> &'static Verified<T>,
    field: impl FnOnce(&'static T) -> &'static U,
) -> Verified<&'static U> {
    table(get()).as_ref().map(field).map_err(Clone::clone)
}

pub struct ExecutableTables {
    identity: ExecutableIdentity,
    /// The executable's own version text.
    pub version: Verified<String>,
    pub keywords: Verified<keywords::KeywordTables>,
    pub registries: Verified<registries::RegistryTables>,
    pub auxiliary: Verified<auxiliary::AuxiliaryTables>,
    pub variables: Verified<variables::VariableTables>,
    pub numeric: Verified<numeric::NumericTables>,
    pub commands: Verified<commands::CommandTables>,
}

impl ExecutableTables {
    pub fn identity(&self) -> &ExecutableIdentity {
        &self.identity
    }

    pub fn recognised(&self) -> bool {
        self.identity.recognised()
    }

    /// Plain-language warning when this release does not recognise the build.
    pub fn build_warning(&self) -> Option<String> {
        self.identity.warning(self.version.as_deref().ok())
    }

    /// Every table, or part of one, that failed its own check.
    pub fn withheld(&self) -> impl Iterator<Item = &Withheld> {
        [
            self.version.as_ref().err(),
            self.keywords.as_ref().err(),
            self.registries.as_ref().err(),
            self.auxiliary.as_ref().err(),
            self.variables.as_ref().err(),
            self.numeric.as_ref().err(),
            self.commands.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .chain(
            self.auxiliary
                .iter()
                .flat_map(auxiliary::AuxiliaryTables::withheld),
        )
    }

    /// Each table's build-independent content digest, or why it is withheld.
    pub fn content_digests(&self) -> [(&'static str, Verified<[u8; 32]>); 6] {
        fn digest<T>(table: &Verified<T>, of: fn(&T) -> [u8; 32]) -> Verified<[u8; 32]> {
            table.as_ref().map(of).map_err(Clone::clone)
        }
        [
            (KEYWORDS, digest(&self.keywords, keywords::KeywordTables::content_digest)),
            (REGISTRIES, digest(&self.registries, registries::RegistryTables::content_digest)),
            (AUXILIARY, digest(&self.auxiliary, auxiliary::AuxiliaryTables::content_digest)),
            (VARIABLES, digest(&self.variables, variables::VariableTables::content_digest)),
            (NUMERIC, digest(&self.numeric, numeric::NumericTables::content_digest)),
            (COMMANDS, digest(&self.commands, commands::CommandTables::content_digest)),
        ]
    }

    /// The build warning followed by each withheld table, for display.
    pub fn warnings(&self) -> Vec<String> {
        self.build_warning()
            .into_iter()
            .chain(self.withheld().map(ToString::to_string))
            .collect()
    }
}

/// Reads the selected installation and publishes each table that verifies itself.
/// Only an unreadable executable is an error; a table that fails is withheld by name.
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
    let bytes = fs::read(exe_path)?;
    let identity = ExecutableIdentity::of(&bytes);
    // The guard only serializes publication.
    let _guard = INITIALIZATION
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(tables) = EXECUTABLE_TABLES.get() {
        if tables.identity != identity {
            return Err(io::Error::other(
                "the selected game is a different build from the one already loaded; restart to use it",
            ));
        }
        return Ok(Arc::clone(tables));
    }
    let tables = Arc::new(decode(identity, bytes, game_dir));
    EXECUTABLE_TABLES
        .set(Arc::clone(&tables))
        .map_err(|_| io::Error::other("executable metadata was published during initialization"))?;
    Ok(tables)
}

const KEYWORDS: &str = "keyword table";
const REGISTRIES: &str = "trigger and effect registries";
const AUXILIARY: &str = "equipment, strategy and rule tables";
const VARIABLES: &str = "built-in variable names";
const NUMERIC: &str = "numeric property table";
const COMMANDS: &str = "player command table";

/// Decodes every table from one executable; tables without a shared input run in parallel.
pub fn decode(identity: ExecutableIdentity, bytes: Vec<u8>, game_dir: &Path) -> ExecutableTables {
    let image = PeImage::parse(bytes).map_err(|error| Withheld::new("executable image", error));
    let Ok(image) = &image else {
        let withheld = image.as_ref().unwrap_err();
        return ExecutableTables {
            identity,
            version: Err(withheld.requires("game version text")),
            keywords: Err(withheld.requires(KEYWORDS)),
            registries: Err(withheld.requires(REGISTRIES)),
            auxiliary: Err(withheld.requires(AUXILIARY)),
            variables: table(VARIABLES, || variables::VariableTables::decode(game_dir)),
            numeric: Err(withheld.requires(NUMERIC)),
            commands: Err(withheld.requires(COMMANDS)),
        };
    };
    std::thread::scope(|scope| {
        let variables =
            scope.spawn(|| table(VARIABLES, || variables::VariableTables::decode(game_dir)));
        let commands = scope.spawn(|| table(COMMANDS, || commands::CommandTables::decode(image)));
        let numeric = scope.spawn(|| table(NUMERIC, || numeric::NumericTables::decode(image)));
        let version = build::version_banner(&image.read_only_data().collect::<Vec<_>>());
        let keywords = table(KEYWORDS, || keywords::KeywordTables::decode(image));
        let (registries, auxiliary) = match &keywords {
            Err(withheld) => (
                Err(withheld.requires(REGISTRIES)),
                Err(withheld.requires(AUXILIARY)),
            ),
            Ok(keywords) => std::thread::scope(|inner| {
                let auxiliary = inner.spawn(|| {
                    table(AUXILIARY, || {
                        auxiliary::AuxiliaryTables::decode(image, keywords)
                    })
                });
                // Consumers match recognised-build identities and token ids, so every identity
                // must have its counterpart and the keyword ids must be the pinned ones.
                let registries = table(REGISTRIES, || {
                    keywords.fixed_ids()?;
                    let registries = registries::RegistryTables::decode(image, keywords)?;
                    if !registries.identities_matched() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "{} readers or classes have no counterpart in the recognised build",
                                registries.unmatched_identities()
                            ),
                        ));
                    }
                    Ok(registries)
                });
                (registries, joined(AUXILIARY, auxiliary))
            }),
        };
        ExecutableTables {
            identity,
            version,
            keywords,
            registries,
            auxiliary,
            variables: joined(VARIABLES, variables),
            numeric: joined(NUMERIC, numeric),
            commands: joined(COMMANDS, commands),
        }
    })
}

/// The source of the registries' recognised-build identity table, from the recognised build.
pub fn registry_identities_source(bytes: Vec<u8>) -> io::Result<String> {
    let image = PeImage::parse(bytes)?;
    let keywords = keywords::KeywordTables::decode(&image)?;
    registries::pinned_source(&image, &keywords)
}

/// A decoder's result; a panic on an unexpected image withholds its table like any failure.
fn table<T>(name: &'static str, decode: impl FnOnce() -> io::Result<T>) -> Verified<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode)) {
        Ok(result) => result.map_err(|error| Withheld::new(name, error)),
        Err(_) => Err(stopped(name)),
    }
}

fn joined<T>(
    name: &'static str,
    handle: std::thread::ScopedJoinHandle<'_, Verified<T>>,
) -> Verified<T> {
    handle.join().unwrap_or_else(|_| Err(stopped(name)))
}

fn stopped(name: &'static str) -> Withheld {
    Withheld::new(name, "its decoder stopped on an unexpected image")
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

/// The published keyword table, or why it is withheld.
pub fn keywords() -> io::Result<&'static keywords::KeywordTables> {
    get().keywords.as_ref().map_err(io::Error::from)
}

pub fn registries() -> io::Result<&'static registries::RegistryTables> {
    get().registries.as_ref().map_err(io::Error::from)
}

pub fn auxiliary() -> io::Result<&'static auxiliary::AuxiliaryTables> {
    get().auxiliary.as_ref().map_err(io::Error::from)
}

pub fn variables() -> io::Result<&'static variables::VariableTables> {
    get().variables.as_ref().map_err(io::Error::from)
}

pub fn numeric() -> io::Result<&'static numeric::NumericTables> {
    get().numeric.as_ref().map_err(io::Error::from)
}

pub fn commands() -> io::Result<&'static commands::CommandTables> {
    get().commands.as_ref().map_err(io::Error::from)
}

/// Explicit installation opt-in for dependent crate tests, which need every table.
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
        .get_or_init(|| {
            let tables = initialize(Path::new(&game_dir))?;
            if let Some(withheld) = tables.withheld().next() {
                return Err(withheld.into());
            }
            Ok(tables)
        })
        .as_ref()
        .map(Arc::clone)
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

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
            Ok(7usize)
        });
        assert!(std::panic::catch_unwind(|| *table).is_err());
        assert_eq!(*table, 7);
        assert_eq!(*table, 7);
        assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_withheld_view_is_kept_and_stops_its_caller_with_the_reason() {
        let table: RuntimeTable<usize> =
            RuntimeTable::new(|| Err(super::Withheld::new("synthetic table", "absent")));
        assert_eq!(table.get().unwrap_err().reason, "absent");
        let stopped = std::panic::catch_unwind(|| *table).unwrap_err();
        assert_eq!(
            stopped.downcast_ref::<String>().unwrap(),
            "synthetic table withheld: absent"
        );
    }
}
