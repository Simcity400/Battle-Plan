//! What production reads of one hour, from a row frame or a columnar walk alike
//! ([`super::ProductionAccumulator::ingest_hour`]).

use crate::products::equipment::{
    RawVariant,
    production::{CountryHolders, Rows},
};
use replay_data::rows::{ProductionLineOutputRow, ProductionLineRow, RocketProductionOutputRow};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

/// The tables production reads whose hour holds every row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Complete {
    pub countries: bool,
    pub links: bool,
    pub output: bool,
    pub lines: bool,
    pub rocket: bool,
    pub definitions: bool,
    pub lexical: bool,
}

/// Which of production's reads an hour dates: the line output's, Rocket output's and the
/// variant census's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    pub output: bool,
    pub rocket: bool,
    pub variants: bool,
}

impl Reads {
    pub(crate) fn any(self) -> bool {
        self.output || self.rocket || self.variants
    }
}

/// What an hour changed of the rows production reads, beside the countries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Changed {
    /// A table production reads but Campaign, Countries and CountryLinks.
    pub others: bool,
    pub output: bool,
    pub rocket: bool,
}

/// The countries of each ProductionLines row.
#[derive(Clone, Copy)]
pub(crate) enum Lines<'a> {
    Rows(&'a [ProductionLineRow]),
    /// Each country some row names, once.
    Countries(&'a [u32]),
}

impl<'a> Lines<'a> {
    pub(crate) fn countries(self) -> impl Iterator<Item = u32> + 'a {
        let (rows, countries) = match self {
            Self::Rows(rows) => (rows, &[][..]),
            Self::Countries(countries) => (&[][..], countries),
        };
        rows.iter()
            .map(|line| line.country)
            .chain(countries.iter().copied())
    }
}

/// The variant reads production names designs by.
pub(crate) enum Variants<'a> {
    /// A row frame, which the accumulator's own reads build from.
    Frame(&'a DecodedFrame),
    /// What a columnar walk read since the accumulator last took them.
    Read(&'a mut Pending),
}

/// The definitions complete variant reads built since the accumulator last took them, and
/// every definition live at the last.
#[derive(Default)]
pub(crate) struct Pending {
    pub built: BTreeMap<u64, RawVariant>,
    pub present: HashSet<u64>,
}

impl Pending {
    /// Keeps a complete read's operands: `built` read anew, `dropped` gone, and nothing but
    /// `built` live where it read `whole`.
    pub(crate) fn keep(&mut self, whole: bool, built: Vec<(u64, RawVariant)>, dropped: &[u64]) {
        if whole {
            self.present.clear();
            self.built.clear();
        }
        for id in dropped {
            self.present.remove(id);
            self.built.remove(id);
        }
        for (id, raw) in built {
            self.present.insert(id);
            self.built.insert(id, raw);
        }
    }
}

/// One hour as production reads it.
pub(crate) struct Hour<'a> {
    pub hour: i32,
    /// The countries where they changed since the hour before: none while Countries is not
    /// complete.
    pub countries: Option<Option<Arc<CountryHolders>>>,
    pub changed: Changed,
    /// The reads that gate observation, and those the hour's metrics date: the same but in
    /// tests that observe every hour as changed.
    pub dated: Reads,
    pub reads: Reads,
    pub complete: Complete,
    /// VariantChanges holds an accepted design edit.
    pub design_changed: bool,
    /// What names designs moved: a name read anew or dropped, or a naming table or its shape.
    pub naming_moved: bool,
    /// Read only while complete.
    pub output: Rows<'a, ProductionLineOutputRow>,
    /// Why the line output is unavailable, where it is.
    pub output_reason: Option<&'a str>,
    pub lines: Lines<'a>,
    pub rocket: Rows<'a, RocketProductionOutputRow>,
    pub variants: Variants<'a>,
}
