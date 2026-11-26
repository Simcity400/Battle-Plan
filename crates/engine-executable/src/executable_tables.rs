
pub mod auxiliary;
pub mod commands;
pub mod cursors;
pub mod effect_identity;
pub mod folding;
pub mod keywords;
pub mod numeric;
pub mod pe;
pub mod recorded;
pub mod registries;
#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;
#[cfg(test)]
mod synthetic_image;
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

pub type Tables = &'static ExecutableTables;

static REGISTRY: Mutex<Vec<Tables>> = Mutex::new(Vec::new());
static PROCESS: OnceLock<Tables> = OnceLock::new();
static TEST_INITIALIZATION: OnceLock<Result<Tables, String>> = OnceLock::new();

pub fn current() -> Option<Tables> {
    job_priority::context()
        .and_then(|context| context.downcast_ref::<ExecutableTables>())
        .or_else(|| PROCESS.get().copied())
}

pub fn scope<T>(tables: Option<Tables>, work: impl FnOnce() -> T) -> T {
    job_priority::with_context(tables.map(|tables| tables as job_priority::Context), work)
}

pub fn enter(tables: Option<Tables>) {
    job_priority::set_context(tables.map(|tables| tables as job_priority::Context));
}

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
                let made = Box::new((self.initialize)());
                let mut views = self.views.lock().unwrap_or_else(|error| error.into_inner());
                match views.iter().find(|(tables, _)| *tables == key) {
                    Some((_, view)) => *view,
                    None => {
                        let made: &'static Verified<T> = Box::leak(made);
                        views.push((key, made));
                        made
                    }
                }
            }
        };
        view.as_ref()
    }
}

impl<T: Send + Sync + 'static> std::ops::Deref for RuntimeTable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        required(self.get())
    }
}

pub fn required<'a, T>(table: Result<&'a T, &Withheld>) -> &'a T {
    table.unwrap_or_else(|withheld| panic!("{withheld}"))
}

pub fn auxiliary_part<T>(
    select: impl FnOnce(&'static auxiliary::AuxiliaryTables) -> &'static Verified<T>,
) -> Result<&'static T, &'static Withheld> {
    select(get().auxiliary.as_ref()?).as_ref()
}

pub fn view<T: 'static, U: ?Sized + 'static>(
    table: impl FnOnce(&'static ExecutableTables) -> &'static Verified<T>,
    field: impl FnOnce(&'static T) -> &'static U,
) -> Verified<&'static U> {
    table(get()).as_ref().map(field).map_err(Clone::clone)
}

pub struct ExecutableTables {
    identity: ExecutableIdentity,
    pub version: Verified<String>,
    pub keywords: Verified<keywords::KeywordTables>,
    pub registries: Verified<registries::RegistryTables>,
    pub auxiliary: Verified<auxiliary::AuxiliaryTables>,
    pub variables: Verified<variables::VariableTables>,
    pub numeric: Verified<numeric::NumericTables>,
    pub commands: Verified<commands::CommandTables>,
    pub folding: Verified<folding::FoldingTable>,
    pub cursors: Verified<cursors::CursorTable>,
}

impl ExecutableTables {
    pub fn identity(&self) -> &ExecutableIdentity {
        &self.identity
    }

    pub fn recognised(&self) -> bool {
        self.identity.recognised()
    }

    pub fn build_warning(&self) -> Option<String> {
        self.identity.warning(self.version.as_deref().ok())
    }

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
            self.cursors.as_ref().err(),
        ]
        .into_iter()
        .flatten()
        .chain(
            self.auxiliary
                .iter()
                .flat_map(auxiliary::AuxiliaryTables::withheld),
        )
    }

    pub fn content_digests(&self) -> [(&'static str, Verified<[u8; 32]>); 8] {
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
            (CURSORS, digest(&self.cursors, cursors::CursorTable::content_digest)),
        ]
    }

    pub fn warnings(&self) -> Vec<String> {
        self.build_warning()
            .into_iter()
            .chain(self.withheld().map(ToString::to_string))
            .collect()
    }
}

pub fn initialize(game_dir: &Path) -> io::Result<Tables> {
    initialize_from(&game_dir.join("hoi4.exe"), game_dir)
}

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
const CURSORS: &str = "map cursor registry";

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
            cursors: Err(withheld.requires(CURSORS)),
        };
    };
    let cursor_identity = identity.clone();
    std::thread::scope(|scope| {
        let variables =
            scope.spawn(|| table(VARIABLES, || variables::VariableTables::decode(game_dir)));
        let commands = scope.spawn(|| table(COMMANDS, || commands::CommandTables::decode(image)));
        let numeric = scope.spawn(|| table(NUMERIC, || numeric::NumericTables::decode(image)));
        let folding = scope.spawn(|| table(FOLDING, || folding::FoldingTable::decode(image)));
        let cursors = scope.spawn(move || {
            table(CURSORS, || {
                cursors::CursorTable::decode(image, &cursor_identity)
            })
        });
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
            cursors: joined(CURSORS, cursors),
        }
    })
}

pub fn registry_identities_source(bytes: Vec<u8>) -> io::Result<String> {
    let image = PeImage::parse(bytes)?;
    let keywords = keywords::KeywordTables::decode(&image)?;
    registries::pinned_source(&image, &keywords)
}

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

pub fn get() -> Tables {
    current().unwrap_or_else(|| {
        debug_assert!(false, "no recording's tables are in scope for this work");
        unscoped()
    })
}

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
            cursors: Err(withheld(CURSORS)),
        }
    })
}

pub fn is_initialized() -> bool {
    current().is_some()
}

pub fn recorded_facts() -> Result<Vec<u8>, String> {
    current()
        .map(recorded::encode)
        .ok_or_else(|| "the game's executable tables were not loaded".to_owned())
}

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

pub fn folding() -> io::Result<&'static folding::FoldingTable> {
    get().folding.as_ref().map_err(io::Error::from)
}

pub fn cursors() -> io::Result<&'static cursors::CursorTable> {
    get().cursors.as_ref().map_err(io::Error::from)
}

pub fn initialize_for_tests() -> io::Result<Tables> {
    let game_dir = std::env::var_os("BATTLEPLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "BATTLEPLAN_GAME_DIR is unset; executable metadata tests require an explicit supported installation",
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
    fn concurrent_misses_keep_one_view_and_dispose_of_the_other() {
        static ENTERED: std::sync::Barrier = std::sync::Barrier::new(2);
        static DROPPED: AtomicUsize = AtomicUsize::new(0);
        struct Counted(usize);
        impl Drop for Counted {
            fn drop(&mut self) {
                DROPPED.fetch_add(1, Ordering::SeqCst);
            }
        }
        static TABLE: RuntimeTable<Counted> = RuntimeTable::new(|| {
            ENTERED.wait();
            Ok(Counted(7))
        });
        let views: Vec<usize> = std::thread::scope(|threads| {
            let callers: Vec<_> = (0..2)
                .map(|_| {
                    threads.spawn(|| {
                        let mut kept = 0;
                        in_scope(|| kept = std::ptr::from_ref(&*TABLE) as usize);
                        kept
                    })
                })
                .collect();
            callers.into_iter().map(|caller| caller.join().unwrap()).collect()
        });
        assert_eq!(views[0], views[1]);
        assert_eq!(DROPPED.load(Ordering::SeqCst), 1);
        in_scope(|| assert_eq!(TABLE.0, 7));
        assert_eq!(DROPPED.load(Ordering::SeqCst), 1);
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
