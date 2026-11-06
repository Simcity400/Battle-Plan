//! Which build-specific pieces may touch the game. An unrecognised build is a warning, not a
//! refusal: each piece is verified or withheld by name before it reads or patches anything.

use engine_executable::{
    availability::{Verified, Withheld},
    executable_tables,
};
use serde::{Serialize, Serializer, ser::SerializeMap, ser::SerializeSeq};
use std::{
    io,
    sync::{Mutex, OnceLock, PoisonError},
};

struct Build {
    recognised: bool,
    warning: Option<String>,
}

static BUILD: OnceLock<Build> = OnceLock::new();
static WITHHELD: Mutex<Vec<Withheld>> = Mutex::new(Vec::new());

/// Records the running build before any piece is queried; the first record stands.
pub fn record(recognised: bool, warning: Option<String>) {
    let _ = BUILD.set(Build {
        recognised,
        warning,
    });
}

/// A build-specific piece: its game sites verify themselves where it hooks or reads them, and
/// its struct layouts through the located executable.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub name: &'static str,
}

impl Piece {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    /// Whether this piece may touch the game on the running build: one load once recorded.
    pub fn verified(self) -> Verified<()> {
        match BUILD.get() {
            Some(build) if build.recognised => Ok(()),
            // On any other build the located image must witness every struct layout.
            Some(_) => match engine_sites::located() {
                Some(located) => located.layouts().map_err(|withheld| withheld.requires(self.name)),
                None => Err(Withheld::new(
                    self.name,
                    "the game executable has not been located",
                )),
            },
            None => Err(Withheld::new(
                self.name,
                "the running game build has not been identified",
            )),
        }
    }
}

/// Lists a piece the caller keeps out of the game, for status.
pub fn report(withheld: &Withheld) -> io::Result<()> {
    let mut list = WITHHELD.lock().unwrap_or_else(PoisonError::into_inner);
    if list.iter().any(|known| known.part == withheld.part) {
        return Ok(());
    }
    engine_allocation::reserve(&mut list, 1)?;
    list.push(withheld.clone());
    Ok(())
}

/// `{"recognised", "warning", "withheld": [{"part", "reason"}]}`: executable tables, game sites,
/// then pieces.
pub fn status() -> impl Serialize {
    struct Status;
    struct Withholding;
    struct Entry<'a>(&'a Withheld);
    impl Serialize for Entry<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("part", &self.0.part)?;
            map.serialize_entry("reason", &self.0.reason)?;
            map.end()
        }
    }
    impl Serialize for Withholding {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(None)?;
            if let Some(tables) = executable_tables::current() {
                for withheld in tables.withheld() {
                    sequence.serialize_element(&Entry(withheld))?;
                }
            }
            if let Some(located) = engine_sites::located() {
                for (_, withheld) in located.withheld() {
                    sequence.serialize_element(&Entry(&withheld))?;
                }
            }
            for withheld in WITHHELD
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
            {
                sequence.serialize_element(&Entry(withheld))?;
            }
            sequence.end()
        }
    }
    impl Serialize for Status {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let build = BUILD.get();
            let mut map = serializer.serialize_map(Some(3))?;
            map.serialize_entry("recognised", &build.map(|build| build.recognised))?;
            map.serialize_entry("warning", &build.and_then(|build| build.warning.as_deref()))?;
            map.serialize_entry("withheld", &Withholding)?;
            map.end()
        }
    }
    Status
}
