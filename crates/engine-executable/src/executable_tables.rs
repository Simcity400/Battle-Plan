//! Immutable executable metadata, each table located by its content and verified on its own.

pub mod auxiliary;
pub mod commands;
pub mod folding;
pub mod keywords;
pub mod numeric;
pub mod pe;
pub mod recorded;
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
use std::sync::{Mutex, OnceLock};

/// One build's tables. Each set is kept for the process's lifetime, so a view of it is
/// `'static`; a process holds a set per build it has met, never more.
pub type Tables = &'static ExecutableTables;

/// Every set of tables this process has decoded or been given, one per build.
static REGISTRY: Mutex<Vec<Tables>> = Mutex::new(Vec::new());
/// The live installed game's tables, for a process that works on it alone (the collector, the
/// recorder and its tools): read where no recording's tables are in scope.
static PROCESS: OnceLock<Tables> = OnceLock::new();
static TEST_INITIALIZATION: OnceLock<Result<Tables, String>> = OnceLock::new();

/// The tables in scope on this thread: the recording's whose work this is ([`scope`]), or the
/// installed game's in a process that works on it alone.
pub fn current() -> Option<Tables> {
    job_priority::context()
        .and_then(|context| context.downcast_ref::<ExecutableTables>())
        .or_else(|| PROCESS.get().copied())
}

/// Runs `work` with `tables` in scope on this thread and every thread it starts with
/// `job_priority`'s `JobPriority` or `Caller`; `None` leaves no recording's tables in scope.
pub fn scope<T>(tables: Option<Tables>, work: impl FnOnce() -> T) -> T {
    job_priority::with_context(tables.map(|tables| tables as job_priority::Context), work)
}

/// Puts `tables` in scope on the calling thread from now on, for a tool that works on one
/// recording alone.
pub fn enter(tables: Option<Tables>) {
    job_priority::set_context(tables.map(|tables| tables as job_priority::Context));
}

/// A view of each build's tables, made once per build that is in scope when it is read.
pub struct RuntimeTable<T: 'static> {
    views: Mutex<Vec<(usize, &'static Verified<T>)>>,
    initialize: fn() -> Verified<T>,
}

impl<T: Send + Sync + 'static> RuntimeTable<T> {
    pub const fn new(initialize: fn() -> Verified<T>) -> Self {
        Self {
            views: Mutex::new(Vec::new()),
            initialize,
        }
    }

    pub fn get(&self) -> Result<&T, &Withheld> {
        let key = std::ptr::from_ref(get()) as usize;
        let found = self
            .views
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find(|(tables, _)| *tables == key)
            .map(|(_, view)| *view);
        let view = match found {
            Some(view) => view,
            None => {
                // Made unlocked: a view may read others.
                let made: &'static Verified<T> = Box::leak(Box::new((self.initialize)()));
                let mut views = self.views.lock().unwrap_or_else(|error| error.into_inner());
                match views.iter().find(|(tables, _)| *tables == key) {
                    Some((_, view)) => *view,
                    None => {
                        views.push((key, made));
                        made
                    }
                }
            }
        };
        view.as_ref()
    }
}

/// Reaching a withheld view stops its caller with the withheld reason, which the
/// content and product boundaries report by name.
impl<T: Send + Sync + 'static> std::ops::Deref for RuntimeTable<T> {
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
    pub folding: Verified<folding::FoldingTable>,
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
            self.folding.as_ref().err(),
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
    pub fn content_digests(&self) -> [(&'static str, Verified<[u8; 32]>); 7] {
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
            (FOLDING, digest(&self.folding, folding::FoldingTable::content_digest)),
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

/// Reads the installation a process works on alone (the collector, the recorder and its tools)
/// and makes its tables the process's ([`current`]); each table that fails is withheld by name.
/// Only an unreadable executable is an error. Playback never calls it: a recording's tables are
/// its own ([`initialize_recorded`]).
pub fn initialize(game_dir: &Path) -> io::Result<Tables> {
    initialize_from(&game_dir.join("hoi4.exe"), game_dir)
}

/// Uses the directory containing the selected executable for shipped documentation.
pub fn initialize_executable(exe_path: &Path) -> io::Result<Tables> {
    let game_dir = exe_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "executable path has no directory",
        )
    })?;
    initialize_from(exe_path, game_dir)
}

fn initialize_from(exe_path: &Path, game_dir: &Path) -> io::Result<Tables> {
    let bytes = fs::read(exe_path)?;
    let identity = ExecutableIdentity::of(&bytes);
    if let Some(tables) = PROCESS.get() {
        return if tables.identity == identity {
            Ok(tables)
        } else {
            Err(io::Error::other(
                "this process works on another build of the game; start it again for this one",
            ))
        };
    }
    let tables = register(decode(identity, bytes, game_dir));
    Ok(*PROCESS.get_or_init(|| tables))
}

/// `tables` kept for the process, or the same build's set kept before.
fn register(tables: ExecutableTables) -> Tables {
    let mut registry = REGISTRY.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(kept) = registry.iter().find(|kept| {
        kept.identity == tables.identity && kept.content_digests() == tables.content_digests()
    }) {
        return kept;
    }
    let kept: Tables = Box::leak(Box::new(tables));
    registry.push(kept);
    kept
}

const KEYWORDS: &str = "keyword table";
const REGISTRIES: &str = "trigger and effect registries";
const AUXILIARY: &str = "equipment, strategy and rule tables";
const VARIABLES: &str = "built-in variable names";
const NUMERIC: &str = "numeric property table";
const COMMANDS: &str = "player command table";
const FOLDING: &str = "diacritic folding table";

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
            folding: Err(withheld.requires(FOLDING)),
        };
    };
    std::thread::scope(|scope| {
        let variables =
            scope.spawn(|| table(VARIABLES, || variables::VariableTables::decode(game_dir)));
        let commands = scope.spawn(|| table(COMMANDS, || commands::CommandTables::decode(image)));
        let numeric = scope.spawn(|| table(NUMERIC, || numeric::NumericTables::decode(image)));
        let folding = scope.spawn(|| table(FOLDING, || folding::FoldingTable::decode(image)));
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
            folding: joined(FOLDING, folding),
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

/// The tables in scope ([`current`]). With none in scope every table is withheld for that reason,
/// so the work reaching them is refused by name as for a recording that carries none.
pub fn get() -> Tables {
    current().unwrap_or_else(|| {
        debug_assert!(false, "no recording's tables are in scope for this work");
        unscoped()
    })
}

/// Every table withheld because no recording's tables are in scope.
fn unscoped() -> Tables {
    static UNSCOPED: OnceLock<ExecutableTables> = OnceLock::new();
    UNSCOPED.get_or_init(|| {
        let withheld = |name: &'static str| {
            Withheld::new(name, "no recording's tables are in scope for this work")
        };
        ExecutableTables {
            identity: ExecutableIdentity::of(&[]),
            version: Err(withheld("executable version")),
            keywords: Err(withheld(KEYWORDS)),
            registries: Err(withheld(REGISTRIES)),
            auxiliary: Err(withheld(AUXILIARY)),
            variables: Err(withheld(VARIABLES)),
            numeric: Err(withheld(NUMERIC)),
            commands: Err(withheld(COMMANDS)),
            folding: Err(withheld(FOLDING)),
        }
    })
}

pub fn is_initialized() -> bool {
    current().is_some()
}

/// The tables in scope as a recording carries them, so its playback reads no installed
/// executable ([`recorded`]).
pub fn recorded_facts() -> Result<Vec<u8>, String> {
    current()
        .map(recorded::encode)
        .ok_or_else(|| "the game's executable tables were not loaded".to_owned())
}

/// [`recorded_facts`] of the executable at `path` and its installation's documentation, which
/// must be the executable whose SHA-256 is `recorded`; decoded apart from every kept set.
pub fn recorded_facts_from(path: &Path, recorded: [u8; 32]) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let identity = ExecutableIdentity::of(&bytes);
    if identity.sha256 != recorded {
        return Err(format!(
            "{} is no longer the recorded executable",
            path.display()
        ));
    }
    let game_dir = path.parent().unwrap_or_else(|| Path::new("."));
    Ok(recorded::encode(&decode(identity, bytes, game_dir)))
}

/// The installed game's tables, for reading what that game reads now (the user's own
/// `settings.txt`), never a recording: kept with every other build's, the executable read again
/// only once its size or last write changes, and never made the process's.
pub fn installed(game_dir: &Path) -> io::Result<Tables> {
    type Stamp = (std::path::PathBuf, u64, Option<std::time::SystemTime>);
    static READ: Mutex<Vec<(Stamp, Tables)>> = Mutex::new(Vec::new());
    let exe_path = game_dir.join("hoi4.exe");
    let metadata = fs::metadata(&exe_path)?;
    let stamp = (exe_path, metadata.len(), metadata.modified().ok());
    let mut read = READ.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, tables)) = read.iter().find(|(kept, _)| *kept == stamp) {
        return Ok(tables);
    }
    let bytes = fs::read(&stamp.0)?;
    let tables = register(decode(ExecutableIdentity::of(&bytes), bytes, game_dir));
    read.retain(|(kept, _)| kept.0 != stamp.0);
    read.push((stamp, tables));
    Ok(tables)
}

/// `work` with the tables in scope, or with none the installed game's at `game_dir`
/// ([`installed`]): what that game reads now, read outside any recording's work.
pub fn or_installed<T>(game_dir: &Path, work: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    if current().is_some() {
        return work();
    }
    let tables = installed(game_dir).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{}: {error}", game_dir.join("hoi4.exe").display()),
        )
    })?;
    scope(Some(tables), work)
}

/// The tables a recording carries, kept with every other build's: work on that recording reads
/// them in its [`scope`].
pub fn initialize_recorded(bytes: &[u8]) -> io::Result<Tables> {
    static DECODED: Mutex<Vec<([u8; 32], Tables)>> = Mutex::new(Vec::new());
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut decoded = DECODED.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, tables)) = decoded.iter().find(|(kept, _)| *kept == digest) {
        return Ok(tables);
    }
    let tables = register(recorded::decode(bytes)?);
    decoded.push((digest, tables));
    Ok(tables)
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

/// The find box's code point to ASCII character folding, or why it is withheld.
pub fn folding() -> io::Result<&'static folding::FoldingTable> {
    get().folding.as_ref().map_err(io::Error::from)
}

/// Explicit installation opt-in for dependent crate tests, which need every table.
pub fn initialize_for_tests() -> io::Result<Tables> {
    let game_dir = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "BATTLE_PLAN_GAME_DIR is unset; executable metadata tests require an explicit supported installation",
        )
    })?;
    TEST_INITIALIZATION
        .get_or_init(|| {
            let tables = initialize(Path::new(&game_dir)).map_err(|error| error.to_string())?;
            if let Some(withheld) = tables.withheld().next() {
                return Err(withheld.to_string());
            }
            Ok(tables)
        })
        .clone()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
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

    // A view is kept per recording's tables, so each test works inside one scope.
    fn in_scope(work: impl FnOnce()) {
        super::scope(Some(super::unscoped()), work);
    }

    #[test]
    fn failed_view_initialization_can_be_retried_and_then_is_shared() {
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
        in_scope(|| {
            let table = RuntimeTable::new(|| {
                let attempt = ATTEMPTS.fetch_add(1, Ordering::Relaxed);
                assert_ne!(attempt, 0, "first metadata view initialization fails");
                Ok(7usize)
            });
            assert!(std::panic::catch_unwind(|| *table).is_err());
            assert_eq!(*table, 7);
            assert_eq!(*table, 7);
            assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 2);
        });
    }

    #[test]
    fn a_withheld_view_is_kept_and_stops_its_caller_with_the_reason() {
        in_scope(|| {
            let table: RuntimeTable<usize> =
                RuntimeTable::new(|| Err(super::Withheld::new("synthetic table", "absent")));
            assert_eq!(table.get().unwrap_err().reason, "absent");
            let stopped = std::panic::catch_unwind(|| *table).unwrap_err();
            assert_eq!(
                stopped.downcast_ref::<String>().unwrap(),
                "synthetic table withheld: absent"
            );
        });
    }
}
