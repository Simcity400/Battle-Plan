//! Which build-specific pieces may touch the game. An unrecognised build is a warning, not a
//! refusal: each piece is verified or withheld by name before it reads or patches anything.

use engine_executable::{
    availability::{Verified, Withheld},
    build::PINNED_BUILD,
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

/// A build-specific piece, and what it relies on that its own checks do not cover.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub name: &'static str,
    pub relies_on: &'static str,
}

impl Piece {
    pub const fn new(name: &'static str, relies_on: &'static str) -> Self {
        Self { name, relies_on }
    }

    /// Whether this piece may touch the game on the running build: one load once recorded.
    pub fn verified(self) -> Verified<()> {
        match BUILD.get() {
            Some(build) if build.recognised => Ok(()),
            // Per-site checks feed in here; until then only the identity proves what a piece relies on.
            Some(_) => Err(Withheld::new(
                self.name,
                format_args!(
                    "the game build is unrecognised, and this relies on the {} of the recognised build (game {})",
                    self.relies_on, PINNED_BUILD.version
                ),
            )),
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

/// `{"recognised", "warning", "withheld": [{"part", "reason"}]}`: executable tables, then pieces.
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
