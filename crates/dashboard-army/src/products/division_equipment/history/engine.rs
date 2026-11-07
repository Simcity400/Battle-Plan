//! Every country's replies at each hour of a branch, kept from the hour's changed rows
//! ([`Input`]). Each placed division's part of its replies ([`Part`]: its pool by type and
//! design, its comparison with its template's need, its refusals) is kept until one of its
//! own inputs changes; each reply is the sum of its divisions' parts, kept as counts and sums
//! ([`Sums`]) a part is added to and taken from; and only the values a changed part, read or
//! template reaches are given again ([`Output`]). An engine started at a restart cannot know
//! the reads made before it: until its own reads stand ([`Supplies::own`]) it keeps its rows
//! and answers nothing, and the engine of the segment before answers those hours.

use super::super::{Placed, UNLINKED_DIVISION};
use super::{CADENCE, KeptReason, NamingOperands};
use crate::products::{
    army_content::{self, DefinitionsOf, Holder, TemplateArtwork},
    division_needs,
};
use dashboard_base::products::equipment::{self, RawVariant};
use dashboard_series::hash::{FastMap, Words};
use dashboard_text::{
    catalogue::Catalogue, metadata::link_slot, subunit_requirements::Content,
    template_facts::TemplateNaming,
};
use dashboard_world::Division;
use replay_data::{
    codec::{Decoder, put_varint},
    rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LexicalTokenRow, TableId, TableRows,
        VariantDefinitionRow, tables,
    },
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_selected};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::BuildHasherDefault,
    io,
    sync::Arc,
};

type FastSet<K> = HashSet<K, BuildHasherDefault<Words>>;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Where [`Input::complete`] holds each table's completeness.
pub(crate) const DIVISIONS: usize = 0;
pub(crate) const MAXIMA: usize = 1;
pub(crate) const MANPOWER: usize = 2;
pub(crate) const NEW: usize = 3;
pub(crate) const DAILY: usize = 4;
pub(crate) const STOCK: usize = 5;
pub(crate) const DEFINITIONS: usize = 6;
/// The tables [`Input::complete`] answers for, in its order.
pub(crate) const COMPLETE: [TableId; 7] = [
    TableId::Divisions,
    TableId::DivisionPanelStats,
    TableId::DivisionLandInputs,
    TableId::NewDivisionEquipment,
    TableId::DivisionEquipment,
    TableId::Stockpiles,
    TableId::VariantDefinitions,
];

/// The rows a table gives an hour.
#[derive(replay_spool::Spooled)]
pub(crate) enum Rows<T> {
    /// What the hour before held.
    Same,
    /// Every row the table holds.
    Whole(Vec<T>),
    /// Rows set, and keys deleted.
    Changed(Vec<T>, Vec<u128>),
}

/// A pool or stockpile row as the walk gives it: its key (owner and ordinal), design and
/// signed amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Entry {
    pub(crate) key: u128,
    pub(crate) variant: u64,
    pub(crate) amount: i64,
}

/// What a lexical token change can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) enum Tokens {
    Same,
    /// Only new tokens: a spelling read before reads alike.
    Appended,
    Changed,
}

/// The countries an hour's replies read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Countries {
    /// Countries and CountryLinks both hold every row.
    pub(crate) complete: bool,
    /// By raw id.
    pub(crate) links: Vec<CountryLinkRow>,
    /// Each listed valid tag's holder; none where no link resolves one. In tag order.
    pub(crate) holders: Vec<(Arc<str>, Option<Holder>)>,
    /// Each canonical slot's first country by id; none while Countries is not whole.
    pub(crate) ids: Option<BTreeMap<u32, u32>>,
}

/// One hour as an engine reads it.
#[derive(replay_spool::Spooled)]
pub(crate) struct Input {
    pub(crate) hour: i32,
    /// The first hour of a walk, or one after an hour it did not walk: every table held this
    /// hour is given whole, with the countries and the small tables.
    pub(crate) whole: bool,
    pub(crate) gap: bool,
    /// Whether the hour reads the daily pools, the stockpile and the designs.
    pub(crate) reads: [bool; 3],
    /// Whether each of [`COMPLETE`] holds every row this hour.
    pub(crate) complete: [bool; 7],
    pub(crate) divisions: Rows<Division>,
    pub(crate) maxima: Rows<(u64, i64)>,
    pub(crate) manpower: Rows<(u64, i64)>,
    pub(crate) new: Rows<Entry>,
    pub(crate) daily: Rows<Entry>,
    pub(crate) stock: Rows<Entry>,
    /// The countries, when they may have changed.
    pub(crate) countries: Option<Arc<Countries>>,
    /// Templates whose rows changed; none when any may have.
    pub(crate) templates: Option<Vec<u64>>,
    pub(crate) tokens: Tokens,
    /// Local sub-units changed.
    pub(crate) subunits: bool,
    /// The templates' facts, when the countries' identity, links, tokens, local sub-units or
    /// templates may have changed them.
    pub(crate) context: Option<Context>,
    /// What an hour that reads the designs gives of them.
    pub(crate) designs: Option<Designs>,
    /// Designs whose definitions the hour changed; none when any may have.
    pub(crate) definitions: Option<Vec<u64>>,
}

/// A template's facts at the hour its tables were last given, before the engine numbers their
/// operands: its need, required manpower, naming and artwork, and whether its naming was
/// sampled for that hour alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(crate) struct TemplateFacts {
    pub(crate) need: Result<Vec<(String, i64)>, KeptReason>,
    pub(crate) required: Option<i64>,
    pub(crate) naming: Result<TemplateNaming, KeptReason>,
    pub(crate) artwork: Result<TemplateArtwork, KeptReason>,
    pub(crate) hourly: bool,
}

/// What the tables say of a template they hold no row of: each part's refusal where it stops
/// before the template's row is looked up; else the template's own, which names it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Absent {
    pub(crate) need: Option<KeptReason>,
    /// DivisionTemplates' refusal; else the template's dictionary is missing.
    pub(crate) naming: Option<KeptReason>,
    /// DivisionTemplateIcons' refusal; else its artwork operands are missing.
    pub(crate) artwork: Option<KeptReason>,
}

impl Absent {
    /// The facts of `handle`, a template no table holds, in its readers' own words.
    pub(crate) fn facts(&self, handle: u64) -> TemplateFacts {
        fn refused<T>(kept: &Option<KeptReason>, own: &dyn Fn() -> String) -> Result<T, KeptReason> {
            Err(kept.clone().unwrap_or_else(|| KeptReason::literal(own())))
        }
        TemplateFacts {
            need: refused(&self.need, &|| {
                format!("Template {handle} equipment composition was not recorded")
            }),
            required: None,
            naming: refused(&self.naming, &|| format!("template {handle} dictionary missing")),
            artwork: refused(&self.artwork, &|| {
                "Template artwork operands not recorded".to_owned()
            }),
            hourly: false,
        }
    }
}

/// The templates' facts given at an hour that may have changed them.
#[derive(replay_spool::Spooled)]
pub(crate) struct Context {
    /// Whether LexicalTokens, CountryLocalSubunits, DivisionTemplates and DivisionTemplateIcons
    /// each held every row.
    pub(crate) wholes: [bool; 4],
    /// `facts` holds every template the tables hold, in place of those held.
    pub(crate) whole: bool,
    pub(crate) facts: Vec<(u64, Arc<TemplateFacts>)>,
    /// Templates the tables no longer hold.
    pub(crate) gone: Vec<u64>,
    pub(crate) absent: Arc<Absent>,
}

/// What an hour that reads the designs gives of them.
#[derive(replay_spool::Spooled)]
pub(crate) struct Designs {
    pub(crate) definitions: DefinitionRows,
    /// The spellings of the equipment tokens the definitions name, by token; none where
    /// LexicalTokens did not hold every row.
    pub(crate) tokens: Option<Vec<LexicalTokenRow>>,
    /// The countries a lookup names designs for; none where Countries or CountryLinks did not
    /// hold every row.
    pub(crate) tags: Option<Arc<DesignTags>>,
}

/// The definitions a designs read gives, each in the fields a design's lookup reads.
#[derive(replay_spool::Spooled)]
pub(crate) enum DefinitionRows {
    /// Every definition, by design.
    Whole(Vec<Arc<VariantDefinitionRow>>),
    /// Those of the designs changed since the last designs read; none for one gone.
    Changed(Vec<(u64, Option<Arc<VariantDefinitionRow>>)>),
}

/// The countries a design's lookup names it for: their slots and tags, the links, and the
/// original tag a dynamic country is named after where CountryAppearance held every row.
#[derive(Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct DesignTags {
    pub(crate) countries: Vec<CountryRow>,
    pub(crate) links: Vec<CountryLinkRow>,
    pub(crate) appearance: Option<Vec<CountryAppearanceRow>>,
}

impl DesignTags {
    /// The tags a saved read's frame holds.
    fn saved(frame: &DecodedFrame) -> Option<Self> {
        Some(Self {
            countries: frame.complete::<tables::Countries>().ok()?.to_vec(),
            links: frame.complete::<tables::CountryLinks>().ok()?.to_vec(),
            appearance: frame
                .complete::<tables::CountryAppearance>()
                .ok()
                .map(<[_]>::to_vec),
        })
    }
}

/// An operand a published value or name reads, given once by the engine's id for it.
#[derive(Clone, Debug)]
#[derive(replay_spool::Spooled)]
pub(crate) enum Operand {
    /// An archetype's key; none for the unclassified.
    Kind(Option<Arc<str>>),
    Note(Arc<KeptReason>),
    Naming(Arc<Result<TemplateNaming, KeptReason>>),
    Artwork(Arc<Result<TemplateArtwork, KeptReason>>),
    Design(Arc<RawVariant>),
}

/// A type's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Measure {
    Held,
    Needed,
    Filled,
    Missing,
    Stock,
    Short,
    Unknown,
}

pub(crate) const MEASURES: [Measure; 7] = [
    Measure::Held,
    Measure::Needed,
    Measure::Filled,
    Measure::Missing,
    Measure::Stock,
    Measure::Short,
    Measure::Unknown,
];

/// A template's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Part {
    Divisions,
    Unknown,
    TypesShort,
    Manpower,
    Required,
    Hp,
    HpMax,
    Needed,
    Held,
    Naming,
    Artwork,
}

pub(crate) const PARTS: [Part; 11] = [
    Part::Divisions,
    Part::Unknown,
    Part::TypesShort,
    Part::Manpower,
    Part::Required,
    Part::Hp,
    Part::HpMax,
    Part::Needed,
    Part::Held,
    Part::Naming,
    Part::Artwork,
];

/// One value's place in a country's reply for one attribution; kinds and notes by operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Field {
    State,
    Divisions,
    Short,
    Unknown,
    Refused,
    Type(u32, Measure),
    /// A design's holding under its type.
    Variant(u32, u64),
    /// The naming operands of a design held under its type.
    Design(u32, u64),
    Template(u64, Part),
    Note(u32),
    StockDesign(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) enum Value {
    Number(i128),
    /// A naming, artwork or design operand.
    Operand(u32),
}

/// One value set, or ended, for a country by its index and an attribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) tag: u32,
    pub(crate) side: u8,
    pub(crate) field: Field,
    pub(crate) value: Option<Value>,
}

/// An answered hour's changes, encoded as they are pushed: a few bytes each, where a change held
/// as itself takes 64, so the hours a segment's walk projects wait for their emit in a fraction
/// of the memory. A reply's changes are pushed together: each names its country and attribution
/// only where they differ from the change before.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Changes {
    bytes: Vec<u8>,
    count: usize,
    last: Option<(u32, u8)>,
}

impl Changes {
    pub(crate) fn push(&mut self, change: Change) {
        let out = &mut self.bytes;
        let index = |found: Option<usize>| found.expect("a listed measure or part") as u8;
        let reply = (change.tag, change.side);
        let same = self.last == Some(reply);
        self.last = Some(reply);
        let field: u8 = match change.field {
            Field::State => 0,
            Field::Divisions => 1,
            Field::Short => 2,
            Field::Unknown => 3,
            Field::Refused => 4,
            Field::Type(..) => 5,
            Field::Variant(..) => 6,
            Field::Design(..) => 7,
            Field::Template(..) => 8,
            Field::Note(_) => 9,
            Field::StockDesign(_) => 10,
        };
        let value: u8 = match change.value {
            None => 0,
            Some(Value::Number(_)) => 1,
            Some(Value::Operand(_)) => 2,
        };
        out.push(field << 3 | u8::from(same) << 2 | value);
        if !same {
            put_varint(out, u128::from(change.tag));
            out.push(change.side);
        }
        match change.field {
            Field::State | Field::Divisions | Field::Short | Field::Unknown | Field::Refused => {}
            Field::Type(kind, measure) => {
                put_varint(out, u128::from(kind));
                out.push(index(MEASURES.iter().position(|&held| held == measure)));
            }
            Field::Variant(kind, handle) | Field::Design(kind, handle) => {
                put_varint(out, u128::from(kind));
                put_varint(out, u128::from(handle));
            }
            Field::Template(template, part) => {
                put_varint(out, u128::from(template));
                out.push(index(PARTS.iter().position(|&held| held == part)));
            }
            Field::Note(note) => put_varint(out, u128::from(note)),
            Field::StockDesign(handle) => put_varint(out, u128::from(handle)),
        }
        match change.value {
            None => {}
            Some(Value::Number(number)) => {
                put_varint(out, ((number << 1) ^ (number >> 127)) as u128);
            }
            Some(Value::Operand(operand)) => put_varint(out, u128::from(operand)),
        }
        self.count += 1;
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The changes in the order they were pushed.
    pub(crate) fn iter(&self) -> impl Iterator<Item = io::Result<Change>> + '_ {
        let mut input = Encoded {
            bytes: &self.bytes,
            reply: None,
        };
        (0..self.count).map(move |_| input.change())
    }

    /// Lets go of the room it grew into past its bytes.
    fn seal(mut self) -> Self {
        self.bytes.shrink_to_fit();
        self
    }
}

/// The bytes of [`Changes`] not yet read back.
struct Encoded<'a> {
    bytes: &'a [u8],
    /// The country and attribution of the change read last.
    reply: Option<(u32, u8)>,
}

impl Encoded<'_> {
    /// The change [`Changes::push`] encoded next.
    fn change(&mut self) -> io::Result<Change> {
        let header = self.byte()?;
        let (tag, side) = if header & 4 != 0 {
            self.reply.ok_or_else(broken)?
        } else {
            let reply = (self.narrow()?, self.byte()?);
            self.reply = Some(reply);
            reply
        };
        let field = match header >> 3 {
            0 => Field::State,
            1 => Field::Divisions,
            2 => Field::Short,
            3 => Field::Unknown,
            4 => Field::Refused,
            5 => {
                let kind = self.narrow()?;
                let measure = *MEASURES.get(usize::from(self.byte()?)).ok_or_else(broken)?;
                Field::Type(kind, measure)
            }
            6 => Field::Variant(self.narrow()?, self.wide()?),
            7 => Field::Design(self.narrow()?, self.wide()?),
            8 => {
                let template = self.wide()?;
                let part = *PARTS.get(usize::from(self.byte()?)).ok_or_else(broken)?;
                Field::Template(template, part)
            }
            9 => Field::Note(self.narrow()?),
            10 => Field::StockDesign(self.wide()?),
            _ => return Err(broken()),
        };
        let value = match header & 3 {
            0 => None,
            1 => {
                let folded = self.varint()?;
                Some(Value::Number(((folded >> 1) as i128) ^ -((folded & 1) as i128)))
            }
            2 => Some(Value::Operand(self.narrow()?)),
            _ => return Err(broken()),
        };
        Ok(Change {
            tag,
            side,
            field,
            value,
        })
    }

    #[inline]
    fn byte(&mut self) -> io::Result<u8> {
        let (&byte, rest) = self.bytes.split_first().ok_or_else(broken)?;
        self.bytes = rest;
        Ok(byte)
    }

    #[inline]
    fn wide(&mut self) -> io::Result<u64> {
        u64::try_from(self.varint()?).map_err(|_| broken())
    }

    #[inline]
    fn narrow(&mut self) -> io::Result<u32> {
        u32::try_from(self.varint()?).map_err(|_| broken())
    }

    /// A varint, its groups gathered in 64 bits while they fit.
    #[inline]
    fn varint(&mut self) -> io::Result<u128> {
        let mut value = 0_u64;
        for shift in (0..63).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(u128::from(value));
            }
        }
        let mut value = u128::from(value);
        for shift in (63..128).step_by(7) {
            let byte = self.byte()?;
            value |= u128::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
        }
        Err(broken())
    }
}

fn broken() -> io::Error {
    invalid("A division equipment change is damaged")
}

/// One answered hour.
#[derive(replay_spool::Spooled)]
pub(crate) struct Output {
    pub(crate) hour: i32,
    /// The pool and stockpile reads every reply reads; none while the replies are not known.
    pub(crate) known: Option<[Option<i32>; 2]>,
    /// Operands the engine named since its last output.
    pub(crate) operands: Vec<(u32, Operand)>,
    /// Countries the engine numbered since its last output, by index.
    pub(crate) tags: Vec<(u32, Arc<str>)>,
    /// Every listed country is given whole; any other is gone.
    pub(crate) whole: bool,
    /// The countries given whole, all of them with `whole`: their other values end.
    pub(crate) listed: Vec<u32>,
    pub(crate) changes: Changes,
}

/// One supplying read, by the hour of its read: unknown before a cold start's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Supply {
    pub(crate) read: Option<i32>,
    pub(crate) known: bool,
    /// Read before the engine started: what it holds is unknown.
    inherited: bool,
}

impl Supply {
    const FRESH: Self = Self {
        read: None,
        known: true,
        inherited: false,
    };
    const INHERITED: Self = Self {
        read: None,
        known: false,
        inherited: true,
    };

    /// A complete read starts a carry; an incomplete table or a day without a read ends it.
    /// An inherited read has ended for certain a day after the start: `cold`.
    fn step(&mut self, complete: bool, read: bool, hour: i32, resumed: Option<i32>, cold: i32) {
        if !complete {
            *self = Self::FRESH;
            return;
        }
        if read {
            *self = Self {
                read: Some(hour),
                known: true,
                inherited: false,
            };
            return;
        }
        if self.inherited {
            if i64::from(hour) - i64::from(cold) >= i64::from(CADENCE - 1) {
                *self = Self::FRESH;
            }
            return;
        }
        if self
            .read
            .is_some_and(|at| i64::from(hour) - i64::from(at) >= i64::from(CADENCE))
        {
            self.read = None;
        }
        if !self.known
            && resumed
                .is_some_and(|first| i64::from(hour) - i64::from(first) >= i64::from(CADENCE - 1))
        {
            self.known = true;
        }
    }
}

/// The pool, stockpile and designs reads, and the designs as of the stockpile's read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(crate) struct Supplies {
    pub(crate) pool: Supply,
    pub(crate) stock: Supply,
    pub(crate) designs: Supply,
    pub(crate) stock_designs: Supply,
    /// The first hour of a branch observed after hours it was not.
    resumed: Option<i32>,
    /// The hour an engine started at a restart.
    cold: i32,
}

impl Supplies {
    const FRESH: Self = Self {
        pool: Supply::FRESH,
        stock: Supply::FRESH,
        designs: Supply::FRESH,
        stock_designs: Supply::FRESH,
        resumed: None,
        cold: 0,
    };

    fn resumed(hour: i32) -> Self {
        let unknown = Supply {
            read: None,
            known: false,
            inherited: false,
        };
        Self {
            pool: unknown,
            stock: unknown,
            designs: unknown,
            stock_designs: unknown,
            resumed: Some(hour),
            cold: 0,
        }
    }

    fn inherited(hour: i32) -> Self {
        Self {
            pool: Supply::INHERITED,
            stock: Supply::INHERITED,
            designs: Supply::INHERITED,
            stock_designs: Supply::INHERITED,
            resumed: None,
            cold: hour,
        }
    }

    /// Whether every read the replies stand on is the engine's own.
    pub(crate) fn own(&self) -> bool {
        !self.pool.inherited
            && !self.stock.inherited
            && !self.designs.inherited
            && (self.stock.read.is_none() || !self.stock_designs.inherited)
    }

    fn known(&self) -> bool {
        self.pool.known
            && self.stock.known
            && self.designs.known
            && (self.stock.read.is_none() || self.stock_designs.known)
    }

    /// Step each read in the row path's order.
    fn step(&mut self, complete: [bool; 3], read: [bool; 3], hour: i32) {
        let (resumed, cold) = (self.resumed, self.cold);
        self.designs.step(complete[2], read[2], hour, resumed, cold);
        let before = self.stock;
        self.stock.step(complete[1], read[1], hour, resumed, cold);
        if (self.stock.read, self.stock.inherited) != (before.read, before.inherited) {
            self.stock_designs = self.designs;
        }
        self.pool.step(complete[0], read[0], hour, resumed, cold);
    }
}

/// A pool or stockpile row as kept: its ordinal, design and signed amount.
type Row = (u32, u64, i64);

/// A design's lookup in a designs read: the notes it made, its archetype's kind, and its
/// naming operands when its definition was read.
#[derive(Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct Look {
    notes: Vec<(u32, usize)>,
    kind: Option<u32>,
    raw: Option<u32>,
}

/// A stockpile read, its rows grouped by the links of its hour.
#[derive(replay_spool::Spooled)]
struct StockRead {
    /// Each slot's designs and amounts in design order, and how many rows no link places;
    /// none where the read's countries or links were not read whole.
    grouped: Option<(FastMap<u32, Vec<(u64, i64)>>, usize)>,
    ids: Option<BTreeMap<u32, u32>>,
}

/// What a reply reads of the national stockpile.
#[derive(Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct StockPart {
    /// Each classified kind's stock, by kind; none where it is withheld.
    totals: Option<Vec<(u32, i128)>>,
    notes: Vec<(u32, u32)>,
    /// Each design looked up, with its naming operands when named.
    designs: Vec<(u64, Option<u32>)>,
}

impl StockPart {
    fn refused(note: u32) -> Self {
        Self {
            totals: None,
            notes: vec![(note, 1)],
            designs: Vec::new(),
        }
    }
}

/// `list` by key, each key once with the sum of its values.
fn summed<K: Ord + Copy, V: Copy + std::ops::AddAssign>(list: &mut Vec<(K, V)>) {
    list.sort_unstable_by_key(|&(key, _)| key);
    list.dedup_by(|later, kept| {
        let same = later.0 == kept.0;
        if same {
            kept.1 += later.1;
        }
        same
    });
}

/// A template's need, required manpower and naming operands at the hour.
#[derive(replay_spool::Spooled)]
struct Template {
    /// Each kind's amount by kind, a kind needed twice at its first.
    need: Result<Arc<[(u32, i64)]>, u32>,
    required: Option<i64>,
    naming: u32,
    artwork: u32,
    /// Its naming was sampled for this hour only: it is named again each hour.
    hourly: bool,
}

/// A listed type: its holding, and its need where compared (else zero), from which its capped
/// holding and shortfall follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct TypeShare {
    kind: u32,
    held: i128,
    need: i64,
}

impl TypeShare {
    fn filled(&self) -> i128 {
        self.held.clamp(0, i128::from(self.need.max(0)))
    }

    fn shortfall(&self) -> i128 {
        (i128::from(self.need) - self.held).max(0)
    }
}

/// A division's part of its replies: its pool compared with its template's need, and its
/// refusals but those of its own numbers. Its lists are sorted, so two parts diff in a pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct Share {
    /// Each kind held or needed.
    types: Vec<TypeShare>,
    /// Each design's holding, by kind and design.
    variants: Vec<(u32, u64, i128)>,
    /// The need and capped holding compared; none while the comparison is unknown.
    compared: Option<(i128, i128)>,
    notes: Vec<u32>,
    required: Option<i64>,
}

impl Share {
    /// The part of a division with `pool` (none: unread) and `template`, made in this room.
    fn fill(
        &mut self,
        pool: Option<&[Row]>,
        looks: Option<&FastMap<u64, Arc<Look>>>,
        template: &Template,
        (fixed, unclassified): (&Fixed, u32),
    ) {
        let Self {
            types,
            variants,
            compared,
            notes,
            required,
        } = self;
        types.clear();
        variants.clear();
        notes.clear();
        if template.required.is_none() {
            notes.push(fixed.required);
        }
        if let Err(note) = &template.need {
            notes.push(*note);
        }
        let mut classified = true;
        match pool {
            None => notes.push(fixed.pool),
            Some(rows) => {
                for &(_, handle, amount) in rows {
                    let looked = looks.and_then(|looks| looks.get(&handle));
                    let kind = match looked.and_then(|look| look.kind) {
                        Some(kind) => kind,
                        None => {
                            notes.push(fixed.unclassified);
                            classified = false;
                            unclassified
                        }
                    };
                    variants.push((kind, handle, i128::from(amount)));
                }
            }
        }
        notes.sort_unstable();
        // A design held in two rows holds their sum; a design reads one kind.
        variants.sort_unstable_by_key(|&(kind, handle, _)| (kind, handle));
        variants.dedup_by(|later, kept| {
            let same = (later.0, later.1) == (kept.0, kept.1);
            if same {
                kept.2 += later.2;
            }
            same
        });
        let need = template.need.as_deref().unwrap_or(&[]);
        let known = template.need.is_ok() && pool.is_some() && classified;
        let (mut needed, mut filled) = (0, 0);
        let (mut holdings, mut needs) = (variants.iter().peekable(), need.iter().peekable());
        loop {
            let kind = match (holdings.peek(), needs.peek()) {
                (None, None) => break,
                (Some(held), None) => held.0,
                (None, Some(wanted)) => wanted.0,
                (Some(held), Some(wanted)) => held.0.min(wanted.0),
            };
            let mut in_pool = 0;
            while let Some(&(_, _, amount)) = holdings.next_if(|held| held.0 == kind) {
                in_pool += amount;
            }
            let amount = needs
                .next_if(|wanted| wanted.0 == kind)
                .map_or(0, |&(_, amount)| amount);
            let mut part = TypeShare {
                kind,
                held: in_pool,
                need: 0,
            };
            if known {
                part.need = amount;
                needed += i128::from(amount);
                filled += part.filled();
            }
            types.push(part);
        }
        *compared = known.then_some((needed, filled));
        *required = template.required;
    }

    /// Whether a compared type falls short.
    fn short(&self) -> bool {
        self.shorts().next().is_some()
    }

    /// The kinds it falls short of, where compared.
    fn shorts(&self) -> impl Iterator<Item = u32> + '_ {
        let known = self.compared.is_some();
        self.types
            .iter()
            .filter(move |part| known && part.shortfall() > 0)
            .map(|part| part.kind)
    }

    /// Its designs, sorted, into `into`.
    fn handles(&self, into: &mut Vec<u64>) {
        into.clear();
        into.extend(self.variants.iter().map(|&(_, handle, _)| handle));
        into.sort_unstable();
    }
}

/// A division's own numbers: hit points, and maximum hit points and manpower where read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct Numbers {
    hp: i64,
    maximum: Option<i64>,
    manpower: Option<i64>,
}

/// What a division added to its replies.
#[derive(replay_spool::Spooled)]
struct Added {
    placed: Placed,
    share: Share,
    numbers: Numbers,
}

#[derive(replay_spool::Spooled)]
struct DivisionState {
    facts: Division,
    added: Option<Added>,
}

/// A value a reply gave last; none until it gives one.
type Given = Option<Option<Value>>;

/// A reply's sum over its divisions, and the values its reply gave last of it.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
struct Sums {
    divisions: u32,
    short: u32,
    unknown: u32,
    types: BTreeMap<u32, TypeSum>,
    templates: BTreeMap<u64, TemplateSum>,
    notes: BTreeMap<u32, u32>,
    handles: BTreeMap<u64, u32>,
    /// Its divisions, those short and those unknown.
    given: [Given; 3],
}

#[derive(Default)]
#[derive(replay_spool::Spooled)]
struct TypeSum {
    /// Divisions listing the type.
    refs: u32,
    held: i128,
    variants: BTreeMap<u64, VariantSum>,
    /// Known divisions comparing the type, and their sums.
    known: u32,
    needed: i128,
    filled: i128,
    missing: i128,
    short: u32,
    /// In [`MEASURES`] order; the stock is each country's own.
    given: [Given; 7],
}

/// A design's holding under a type.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
struct VariantSum {
    /// Divisions holding it.
    divisions: u32,
    amount: i128,
    /// Its holding and naming operands.
    given: [Given; 2],
}

#[derive(Default)]
#[derive(replay_spool::Spooled)]
struct TemplateSum {
    divisions: u32,
    unknown: u32,
    /// Divisions lacking each, and the sum of the others.
    manpower: (u32, i128),
    required: (u32, i128),
    maximum: (u32, i128),
    hp: i128,
    known: u32,
    needed: i128,
    held: i128,
    short: BTreeMap<u32, u32>,
    /// In [`PARTS`] order.
    given: [Given; 11],
}

/// What an hour changed of a reply.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
struct Dirty {
    counts: bool,
    /// Every type's unknown count.
    unknown: bool,
    types: BTreeSet<u32>,
    variants: BTreeSet<(u32, u64)>,
    templates: BTreeSet<u64>,
    notes: bool,
}

/// What an hour's rows changed; a division may be listed more than once.
#[derive(Default)]
struct Touched {
    /// Divisions whose placement, a number's presence, or existence changed.
    structural: Vec<u64>,
    /// Divisions gone, with what they added.
    gone: Vec<(u64, Added)>,
    /// Divisions whose pool rows changed.
    pools: Vec<u64>,
    /// Divisions whose hit points, maximum or manpower changed, presence alike.
    numeric: Vec<u64>,
    countries: bool,
    /// A table the needs' sources or the templates' facts read became, or stopped being,
    /// whole.
    sources: bool,
    templates: bool,
}

/// The fixed refusals of a comparison and a stockpile part.
#[derive(replay_spool::Spooled)]
struct Fixed {
    manpower: u32,
    required: u32,
    maximum: u32,
    pool: u32,
    unclassified: u32,
    unlinked: u32,
    stock_unread: u32,
    stock_links: u32,
    stock_unlinked: u32,
    stock_unclassified: u32,
}

/// A designs read: its countries' tags, and whether it read its tokens whole, with the
/// spellings of the tokens its definitions name; its definitions are kept by design.
#[derive(replay_spool::Spooled)]
struct Read {
    tags: Option<Arc<DesignTags>>,
    spelled: bool,
    /// Sorted by token.
    tokens: Arc<[LexicalTokenRow]>,
}

impl Read {
    /// The read `designs` gives, its token rows those `history` names.
    fn of(designs: &Designs, history: &mut History) -> Self {
        Self {
            tags: designs.tags.clone(),
            spelled: designs.tokens.is_some(),
            tokens: history.spelled(designs.tokens.as_deref().unwrap_or(&[])),
        }
    }

    /// The read a save kept as `frame`, its token rows those `history` names.
    fn from_saved(frame: &DecodedFrame, history: &mut History) -> Self {
        let tokens = frame.complete::<tables::LexicalTokens>().ok();
        Self {
            tags: DesignTags::saved(frame).map(Arc::new),
            spelled: tokens.is_some(),
            tokens: history.spelled(tokens.unwrap_or(&[])),
        }
    }

    /// The spelling of `token`, where the read spelled the tokens.
    fn spelling(&self, token: u32) -> Option<&[u8]> {
        let at = self
            .tokens
            .binary_search_by_key(&token, |row| row.token)
            .ok()?;
        self.tokens.get(at).map(|row| row.literal.as_slice())
    }

    /// The frame a save keeps of it: its definitions as `history` keeps them at `hour`.
    fn saved(&self, history: &History, hour: i32) -> DecodedFrame {
        let unread = || -> SharedView {
            TableView::Unavailable {
                reason: "Not read whole at this designs read".into(),
            }
            .into()
        };
        let complete = |rows: TableRows| -> SharedView { TableView::Complete(rows).into() };
        let mut frame = DecodedFrame {
            branch: 0,
            hour,
            tables: std::array::from_fn(|_| not_selected()),
            metrics: Metrics::default(),
        };
        let tags = self.tags.as_deref();
        frame.tables[TableId::Countries.index()] = tags.map_or_else(unread, |tags| {
            complete(TableRows::Countries(tags.countries.clone()))
        });
        frame.tables[TableId::CountryLinks.index()] = tags.map_or_else(unread, |tags| {
            complete(TableRows::CountryLinks(tags.links.clone()))
        });
        frame.tables[TableId::CountryAppearance.index()] = tags
            .and_then(|tags| tags.appearance.as_ref())
            .map_or_else(unread, |rows| {
                complete(TableRows::CountryAppearance(rows.clone()))
            });
        frame.tables[TableId::VariantDefinitions.index()] =
            complete(TableRows::VariantDefinitions(history.rows_at(hour)));
        frame.tables[TableId::LexicalTokens.index()] = if self.spelled {
            complete(TableRows::LexicalTokens(self.tokens.to_vec()))
        } else {
            unread()
        };
        frame
    }
}

/// The fields of a definition a design's lookup reads.
#[derive(PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
struct Looked {
    equipment_token: u32,
    creator_link: u32,
    origin_link: u32,
    append_variant_name: u8,
    /// The display name's length, which `names` holds before the variant name.
    split: u32,
    names: Box<[u8]>,
}

impl Looked {
    fn display_name(&self) -> &[u8] {
        &self.names[..self.split as usize]
    }

    fn variant_name(&self) -> &[u8] {
        &self.names[self.split as usize..]
    }
}

/// A design's definition as kept: the fields a lookup reads where the row holds no other, else
/// the row whole.
#[derive(replay_spool::Spooled)]
enum Definition {
    Looked(Looked),
    Whole(Arc<VariantDefinitionRow>),
}

/// The fields of `row` a lookup reads.
fn fields(row: &VariantDefinitionRow) -> (u32, u32, u32, u8, &[u8], &[u8]) {
    (
        row.equipment_token,
        row.creator_link,
        row.origin_link,
        row.append_variant_name,
        &row.display_name,
        &row.variant_name,
    )
}

/// Whether every field of `row` a lookup does not read holds its default.
fn looked_only(row: &VariantDefinitionRow) -> bool {
    let VariantDefinitionRow {
        id: _,
        equipment_token: _,
        creator_link: _,
        origin_link: _,
        append_variant_name: _,
        obsolete,
        classifier_flags,
        mission_mask,
        display_name: _,
        variant_name: _,
        explicit_entity,
        archetype_token,
        icon_active,
        icon_kind,
        icon,
        version,
        design,
    } = row;
    let unset = VariantDefinitionRow::default();
    *obsolete == unset.obsolete
        && *classifier_flags == unset.classifier_flags
        && *mission_mask == unset.mission_mask
        && *explicit_entity == unset.explicit_entity
        && *archetype_token == unset.archetype_token
        && *icon_active == unset.icon_active
        && *icon_kind == unset.icon_kind
        && *icon == unset.icon
        && *version == unset.version
        && *design == unset.design
}

impl Definition {
    fn of(row: &Arc<VariantDefinitionRow>) -> Self {
        if !looked_only(row) {
            return Self::Whole(Arc::clone(row));
        }
        let mut names = Vec::with_capacity(row.display_name.len() + row.variant_name.len());
        names.extend_from_slice(&row.display_name);
        names.extend_from_slice(&row.variant_name);
        match u32::try_from(row.display_name.len()) {
            Ok(split) => Self::Looked(Looked {
                equipment_token: row.equipment_token,
                creator_link: row.creator_link,
                origin_link: row.origin_link,
                append_variant_name: row.append_variant_name,
                split,
                names: names.into_boxed_slice(),
            }),
            Err(_) => Self::Whole(Arc::clone(row)),
        }
    }

    fn token(&self) -> u32 {
        match self {
            Self::Looked(looked) => looked.equipment_token,
            Self::Whole(row) => row.equipment_token,
        }
    }

    /// The fields a lookup reads.
    fn looked(&self) -> (u32, u32, u32, u8, &[u8], &[u8]) {
        match self {
            Self::Looked(looked) => (
                looked.equipment_token,
                looked.creator_link,
                looked.origin_link,
                looked.append_variant_name,
                looked.display_name(),
                looked.variant_name(),
            ),
            Self::Whole(row) => fields(row),
        }
    }

    /// Whether it is `row`'s.
    fn is(&self, row: &VariantDefinitionRow) -> bool {
        match self {
            Self::Looked(_) => looked_only(row) && self.looked() == fields(row),
            Self::Whole(held) => **held == *row,
        }
    }

    /// Design `id`'s row.
    fn row(&self, id: u64) -> Cow<'_, VariantDefinitionRow> {
        match self {
            Self::Looked(looked) => Cow::Owned(VariantDefinitionRow {
                id,
                equipment_token: looked.equipment_token,
                creator_link: looked.creator_link,
                origin_link: looked.origin_link,
                append_variant_name: looked.append_variant_name,
                display_name: looked.display_name().to_vec(),
                variant_name: looked.variant_name().to_vec(),
                ..VariantDefinitionRow::default()
            }),
            Self::Whole(row) => Cow::Borrowed(row),
        }
    }
}

/// A design's versions, each with the hour of the read that first held it, none where the
/// read did not define it: most designs hold one definition through every read kept.
#[derive(replay_spool::Spooled)]
enum Versions {
    One(i32, Definition),
    Many(Vec<(i32, Option<Definition>)>),
}

impl Versions {
    fn latest(&self) -> Option<&Definition> {
        match self {
            Self::One(_, definition) => Some(definition),
            Self::Many(versions) => versions.last().and_then(|(_, row)| row.as_ref()),
        }
    }

    fn at(&self, hour: i32) -> Option<&Definition> {
        match self {
            Self::One(at, definition) => (*at <= hour).then_some(definition),
            Self::Many(versions) => {
                let after = versions.partition_point(|&(at, _)| at <= hour);
                versions.get(after.checked_sub(1)?)?.1.as_ref()
            }
        }
    }
}

/// Every design's definition at each designs read still looked up, by design.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
struct History {
    versions: FastMap<u64, Versions>,
    /// Designs holding more than one version.
    varied: Vec<u64>,
    /// The equipment tokens the latest definitions name, with how many name each.
    named: BTreeMap<u32, u32>,
    /// The token rows the last read spelled them with.
    spelled: Option<Arc<[LexicalTokenRow]>>,
    /// Whether the latest versions are the last designs frame's: a frame after one that was
    /// not read is compared whole.
    based: bool,
}

impl History {
    /// The definitions a designs read gives at `hour`: every design's, or those of the designs
    /// it changed since the read before, which this history must hold.
    fn read(&mut self, rows: &DefinitionRows, hour: i32) -> io::Result<()> {
        match rows {
            DefinitionRows::Changed(_) if !self.based => {
                return Err(invalid(
                    "A designs read gave the definitions it changed to an engine holding none",
                ));
            }
            DefinitionRows::Changed(changed) => {
                for (id, row) in changed {
                    self.set(*id, row.as_ref(), hour);
                }
            }
            DefinitionRows::Whole(rows) => {
                for row in rows {
                    self.set(row.id, Some(row), hour);
                }
                let defined = |id: u64| rows.binary_search_by_key(&id, |row| row.id).is_ok();
                let gone: Vec<u64> = self
                    .versions
                    .iter()
                    .filter(|&(&id, versions)| versions.latest().is_some() && !defined(id))
                    .map(|(&id, _)| id)
                    .collect();
                for id in gone {
                    self.set(id, None, hour);
                }
            }
        }
        self.based = true;
        Ok(())
    }

    /// Design `id`'s definition from `hour` on; a version only where it differs.
    fn set(&mut self, id: u64, row: Option<&Arc<VariantDefinitionRow>>, hour: i32) {
        let latest = self.versions.get(&id).and_then(Versions::latest);
        let same = match (latest, row) {
            (None, None) => true,
            (Some(latest), Some(row)) => latest.is(row),
            _ => false,
        };
        if same {
            return;
        }
        if let Some(token) = latest.map(Definition::token) {
            let named = self.named.entry(token).or_default();
            *named -= 1;
            if *named == 0 {
                self.named.remove(&token);
            }
        }
        if let Some(row) = row {
            *self.named.entry(row.equipment_token).or_default() += 1;
        }
        let row = row.map(Definition::of);
        let Some(versions) = self.versions.get_mut(&id) else {
            // No version yet and none now holds nothing: `same` returned above.
            if let Some(row) = row {
                self.versions.insert(id, Versions::One(hour, row));
            }
            return;
        };
        match versions {
            Versions::One(at, _) if *at == hour => match row {
                Some(row) => *versions = Versions::One(hour, row),
                // Undefined from every read it was defined at: as if never defined.
                None => {
                    self.versions.remove(&id);
                }
            },
            Versions::One(..) => {
                let Versions::One(at, held) =
                    std::mem::replace(versions, Versions::Many(Vec::new()))
                else {
                    unreachable!("matched as one version")
                };
                *versions = Versions::Many(vec![(at, Some(held)), (hour, row)]);
                self.varied.push(id);
            }
            Versions::Many(versions) => match versions.last_mut() {
                Some((at, held)) if *at == hour => *held = row,
                _ => {
                    if versions.len() == 1 {
                        self.varied.push(id);
                    }
                    versions.push((hour, row));
                }
            },
        }
    }

    /// Design `id`'s definition at the read at `hour`.
    fn at(&self, id: u64, hour: i32) -> Option<&Definition> {
        self.versions.get(&id)?.at(hour)
    }

    /// The rows of `tokens`, a read's, that the latest definitions name: the last read's rows
    /// again where they are alike.
    fn spelled(&mut self, tokens: &[LexicalTokenRow]) -> Arc<[LexicalTokenRow]> {
        let rows: Vec<LexicalTokenRow> = self
            .named
            .keys()
            .filter_map(|&token| {
                let at = tokens.binary_search_by_key(&token, |row| row.token).ok()?;
                tokens.get(at).cloned()
            })
            .collect();
        match &self.spelled {
            Some(held) if **held == *rows => Arc::clone(held),
            _ => {
                let rows: Arc<[LexicalTokenRow]> = rows.into();
                self.spelled = Some(Arc::clone(&rows));
                rows
            }
        }
    }

    /// Lets go of the versions no read at or after `oldest` holds; every one but the latest
    /// when no read is held.
    fn keep(&mut self, oldest: Option<i32>) {
        let versions = &mut self.versions;
        self.varied.retain(|id| {
            let Some(Versions::Many(held)) = versions.get_mut(id) else {
                return false;
            };
            let first = match oldest {
                Some(oldest) => held.partition_point(|&(at, _)| at <= oldest).max(1) - 1,
                None => held.len() - 1,
            };
            held.drain(..first);
            if held.len() > 1 {
                return true;
            }
            match held.pop() {
                Some((at, Some(definition))) => {
                    versions.insert(*id, Versions::One(at, definition));
                }
                _ => {
                    versions.remove(id);
                }
            }
            false
        });
    }

    /// Every definition at the read at `hour`, by design.
    fn rows_at(&self, hour: i32) -> Vec<VariantDefinitionRow> {
        let mut rows: Vec<VariantDefinitionRow> = self
            .versions
            .keys()
            .filter_map(|&id| self.at(id, hour).map(|row| row.row(id).into_owned()))
            .collect();
        rows.sort_unstable_by_key(|row| row.id);
        rows
    }
}

/// The definitions kept at one read.
struct AsOf<'a> {
    history: &'a History,
    hour: i32,
}

impl DefinitionsOf for AsOf<'_> {
    fn definition(&self, variant: u64) -> Option<Cow<'_, VariantDefinitionRow>> {
        self.history
            .at(variant, self.hour)
            .map(|definition| definition.row(variant))
    }
}

/// Whether two designs reads look a design up alike wherever its definition and token rows
/// are alike: each reads every table a lookup needs, with the same countries' slots and tags,
/// links and appearance.
fn designs_alike(a: &Read, b: &Read) -> bool {
    let read = a.spelled && b.spelled;
    let (Some(a), Some(b)) = (&a.tags, &b.tags) else {
        return false;
    };
    if Arc::ptr_eq(a, b) {
        return read;
    }
    let appearance = match (&a.appearance, &b.appearance) {
        (Some(a), Some(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| (a.country, &a.name_tag) == (b.country, &b.name_tag))
        }
        (None, None) => true,
        _ => false,
    };
    read && appearance
        && a.links == b.links
        && a.countries.len() == b.countries.len()
        && a.countries
            .iter()
            .zip(&b.countries)
            .all(|(a, b)| (a.canonical_slot, &a.tag) == (b.canonical_slot, &b.tag))
}

/// Adds `value` times `sign` to `total`.
fn add(total: &mut i128, value: i128, sign: i128) {
    *total += sign * value;
}

/// Counts one more (`sign` 1) or fewer (-1) in `count`.
fn count(count: &mut u32, sign: i128) {
    if sign > 0 {
        *count += 1;
    } else {
        *count -= 1;
    }
}

/// `held` taken by `sign` from or into a count of present things: whether it came or went.
fn counted<K: Ord + Copy>(map: &mut BTreeMap<K, u32>, key: K, sign: i128) {
    let held = map.entry(key).or_default();
    count(held, sign);
    if *held == 0 {
        map.remove(&key);
    }
}

/// One more where only `now` holds, one fewer where only `was` does: whether it changed.
fn recount(count: &mut u32, was: bool, now: bool) -> bool {
    match (was, now) {
        (false, true) => *count += 1,
        (true, false) => *count -= 1,
        _ => return false,
    }
    true
}

/// Each key of two lists sorted by it, once each, with its item in either.
fn merged<T, K: Ord>(
    a: &[T],
    b: &[T],
    key: impl Fn(&T) -> K,
    mut each: impl FnMut(K, Option<&T>, Option<&T>),
) {
    let (mut i, mut j) = (0, 0);
    loop {
        let order = match (a.get(i), b.get(j)) {
            (None, None) => return,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(x), Some(y)) => key(x).cmp(&key(y)),
        };
        match order {
            Ordering::Less => {
                each(key(&a[i]), Some(&a[i]), None);
                i += 1;
            }
            Ordering::Greater => {
                each(key(&b[j]), None, Some(&b[j]));
                j += 1;
            }
            Ordering::Equal => {
                each(key(&a[i]), Some(&a[i]), Some(&b[j]));
                i += 1;
                j += 1;
            }
        }
    }
}

/// The part of a division not counted anywhere.
static NO_SHARE: Share = Share {
    types: Vec::new(),
    variants: Vec::new(),
    compared: None,
    notes: Vec::new(),
    required: None,
};

/// One reply's sums moved from what a division added to what it adds, marking what differs.
fn shift_sums(
    sums: &mut Sums,
    dirty: &mut Dirty,
    template: u64,
    (was, now): (Option<(&Share, &Numbers)>, Option<(&Share, &Numbers)>),
    handles: &[Vec<u64>; 2],
    (manpower, maximum): (u32, u32),
) {
    let (old, new) = (
        was.map_or(&NO_SHARE, |(share, _)| share),
        now.map_or(&NO_SHARE, |(share, _)| share),
    );
    let unknown =
        |side: Option<(&Share, &Numbers)>| side.is_some_and(|(share, _)| share.compared.is_none());
    let counts = recount(&mut sums.divisions, was.is_some(), now.is_some())
        | recount(&mut sums.short, old.short(), new.short());
    if recount(&mut sums.unknown, unknown(was), unknown(now)) {
        dirty.counts = true;
        dirty.unknown = true;
    } else if counts {
        dirty.counts = true;
    }
    merged(
        &old.variants,
        &new.variants,
        |&(kind, handle, _)| (kind, handle),
        |(kind, handle), gone, come| {
            let (gone, come) = (gone.map(|held| held.2), come.map(|held| held.2));
            if gone == come {
                return;
            }
            let total = sums.types.entry(kind).or_default();
            let variant = total.variants.entry(handle).or_default();
            for (amount, sign) in [(gone, -1), (come, 1)] {
                if let Some(amount) = amount {
                    count(&mut variant.divisions, sign);
                    add(&mut variant.amount, amount, sign);
                }
            }
            if variant.divisions == 0 {
                total.variants.remove(&handle);
            }
            dirty.variants.insert((kind, handle));
        },
    );
    // A type's part: its holding, and its comparison where known.
    let part = |part: &TypeShare, known: bool| {
        let compared = known.then(|| (i128::from(part.need), part.filled(), part.shortfall()));
        (part.held, compared)
    };
    let known = (old.compared.is_some(), new.compared.is_some());
    merged(
        &old.types,
        &new.types,
        |listed| listed.kind,
        |kind, gone, come| {
            let gone = gone.map(|gone| part(gone, known.0));
            let come = come.map(|come| part(come, known.1));
            if gone == come {
                return;
            }
            let total = sums.types.entry(kind).or_default();
            for (side, sign) in [(gone, -1), (come, 1)] {
                let Some((held, compared)) = side else {
                    continue;
                };
                count(&mut total.refs, sign);
                add(&mut total.held, held, sign);
                if let Some((need, filled, shortfall)) = compared {
                    count(&mut total.known, sign);
                    add(&mut total.needed, need, sign);
                    add(&mut total.filled, filled, sign);
                    add(&mut total.missing, shortfall, sign);
                    if shortfall > 0 {
                        count(&mut total.short, sign);
                    }
                }
            }
            if total.refs == 0 {
                sums.types.remove(&kind);
            }
            dirty.types.insert(kind);
        },
    );
    let parts = |side: Option<(&Share, &Numbers)>| {
        side.map(|(share, numbers)| (share.compared, share.required, *numbers))
    };
    if parts(was) != parts(now) || !old.shorts().eq(new.shorts()) {
        let total = sums.templates.entry(template).or_default();
        for (side, sign) in [(was, -1), (now, 1)] {
            let Some((share, numbers)) = side else {
                continue;
            };
            count(&mut total.divisions, sign);
            if share.compared.is_none() {
                count(&mut total.unknown, sign);
            }
            for ((missing, sum), value) in [
                (&mut total.manpower, numbers.manpower),
                (&mut total.required, share.required),
                (&mut total.maximum, numbers.maximum),
            ] {
                match value {
                    Some(value) => add(sum, i128::from(value), sign),
                    None => count(missing, sign),
                }
            }
            add(&mut total.hp, i128::from(numbers.hp), sign);
            if let Some((needed, filled)) = share.compared {
                count(&mut total.known, sign);
                add(&mut total.needed, needed, sign);
                add(&mut total.held, filled, sign);
                for kind in share.shorts() {
                    counted(&mut total.short, kind, sign);
                }
            }
        }
        if total.divisions == 0 {
            sums.templates.remove(&template);
        }
        dirty.templates.insert(template);
    }
    let mut noted = false;
    if old.notes != new.notes {
        for (notes, sign) in [(&old.notes, -1), (&new.notes, 1)] {
            for &note in notes {
                counted(&mut sums.notes, note, sign);
            }
        }
        noted = true;
    }
    let lacks = |side: Option<(&Share, &Numbers)>| {
        side.map_or((false, false), |(_, numbers)| {
            (numbers.manpower.is_none(), numbers.maximum.is_none())
        })
    };
    let (gone, come) = (lacks(was), lacks(now));
    for (note, gone, come) in [(manpower, gone.0, come.0), (maximum, gone.1, come.1)] {
        if gone != come {
            counted(&mut sums.notes, note, if come { 1 } else { -1 });
            noted = true;
        }
    }
    merged(
        &handles[0],
        &handles[1],
        |&handle| handle,
        |handle, gone, come| {
            let sign = match (gone, come) {
                (Some(_), None) => -1,
                (None, Some(_)) => 1,
                _ => return,
            };
            let before = sums.handles.contains_key(&handle);
            counted(&mut sums.handles, handle, sign);
            if before != sums.handles.contains_key(&handle) {
                noted = true;
            }
        },
    );
    if noted {
        dirty.notes = true;
    }
}

/// One branch's replies, kept from hour to hour.
#[derive(replay_spool::Spooled)]
pub(crate) struct Engine {
    cat: Option<Arc<Catalogue>>,
    content: Result<Arc<Content>, String>,
    /// Operands by content, numbered by the engine.
    next: u32,
    kinds: HashMap<Option<Arc<str>>, u32>,
    notes: HashMap<KeptReason, u32>,
    namings: BTreeMap<Arc<Result<TemplateNaming, KeptReason>>, u32>,
    artworks: BTreeMap<Arc<Result<TemplateArtwork, KeptReason>>, u32>,
    designs: HashMap<NamingOperands, u32>,
    /// Operands numbered since the last output.
    fresh: Vec<(u32, Operand)>,
    unclassified: u32,
    warnings: Vec<u32>,
    fixed: Fixed,
    supplies: Supplies,
    last: Option<i32>,
    /// Whether the last hour's replies were given.
    open: bool,
    complete: [bool; 7],
    divisions: FastMap<u64, DivisionState>,
    maxima: FastMap<u64, i64>,
    manpower: FastMap<u64, i64>,
    new: FastMap<u64, Arc<[Row]>>,
    /// The daily pools as the table holds them, and as their last read did.
    daily: FastMap<u64, Arc<[Row]>>,
    pooled: FastMap<u64, Arc<[Row]>>,
    /// Divisions whose daily pool rows changed since the last read.
    daily_since: FastSet<u64>,
    /// The divisions the last pool read saw, sorted; none when Divisions was incomplete then.
    census: Option<Vec<u64>>,
    /// Stockpile rows by raw country.
    stock_rows: FastMap<u64, Arc<[Row]>>,
    stock: Option<StockRead>,
    countries: Arc<Countries>,
    /// Each template's facts as last given, by template, until the hour that gives them again,
    /// and what the tables say of one they hold no row of; none until first given.
    facts: FastMap<u64, Arc<TemplateFacts>>,
    absent: Option<Arc<Absent>>,
    /// Whether the needs' and templates' four tables were whole as last given.
    wholes: Option<[bool; 4]>,
    /// Each designs read still looked up, its definitions, and the designs looked up there.
    reads: BTreeMap<i32, Read>,
    history: History,
    looks: FastMap<Option<i32>, FastMap<u64, Arc<Look>>>,
    /// Pairs of reads that look a design up alike wherever its rows are alike.
    alike: Vec<((i32, i32), bool)>,
    templates: FastMap<u64, Template>,
    /// The templates named again each hour.
    hourly: BTreeSet<u64>,
    /// Handed on: it holds its reads and numbering alone, until an hour gives the rest whole.
    bare: bool,
    /// Countries numbered, by index; each listed one's holder, by index.
    tags: Vec<Arc<str>>,
    numbers: HashMap<Arc<str>, u32>,
    listed: BTreeMap<u32, Option<Holder>>,
    slots: FastMap<u32, Vec<u32>>,
    fresh_tags: Vec<(u32, Arc<str>)>,
    sums: FastMap<(u32, u8), Sums>,
    unlinked: [u32; 2],
    by_template: FastMap<u64, FastSet<u64>>,
    by_handle: FastMap<u64, FastSet<u64>>,
    stock_parts: FastMap<u32, StockPart>,
    /// Each country's notes and stockpile designs as last given, by attribution.
    given: FastMap<(u32, u8), BTreeMap<u32, u32>>,
    stock_given: FastMap<(u32, u8), Vec<(u64, u32)>>,
    dirty: FastMap<(u32, u8), Dirty>,
    stock_dirty: BTreeSet<u32>,
    notes_dirty: bool,
    /// The room the next division's part is made in.
    scratch: Share,
    /// The room a shift sorts the designs of the parts it moves between in.
    handles: [Vec<u64>; 2],
    /// A pool read with no rows.
    empty: Arc<[Row]>,
}

/// Whether two divisions are placed alike: their countries, template and reveal byte.
fn placed_alike(a: &Division, b: &Division) -> bool {
    (
        a.roster_owner,
        a.logical_owner,
        a.expeditionary_owner,
        a.template,
        a.reveal_skip,
    ) == (
        b.roster_owner,
        b.logical_owner,
        b.expeditionary_owner,
        b.template,
        b.reveal_skip,
    )
}

impl Engine {
    /// A branch with nothing observed: its first hour `resumed` when it is not the branch's,
    /// or started at a restart `cold`, which knows none of the reads before it.
    pub(crate) fn new(
        cat: Option<Arc<Catalogue>>,
        resumed: Option<i32>,
        cold: Option<i32>,
    ) -> Self {
        let content = division_needs::content(cat.as_ref());
        let mut engine = Self {
            cat,
            content,
            next: 0,
            kinds: HashMap::new(),
            notes: HashMap::new(),
            namings: BTreeMap::new(),
            artworks: BTreeMap::new(),
            designs: HashMap::new(),
            fresh: Vec::new(),
            unclassified: 0,
            warnings: Vec::new(),
            fixed: Fixed {
                manpower: 0,
                required: 0,
                maximum: 0,
                pool: 0,
                unclassified: 0,
                unlinked: 0,
                stock_unread: 0,
                stock_links: 0,
                stock_unlinked: 0,
                stock_unclassified: 0,
            },
            supplies: match (cold, resumed) {
                (Some(hour), _) => Supplies::inherited(hour),
                (None, Some(hour)) => Supplies::resumed(hour),
                (None, None) => Supplies::FRESH,
            },
            last: None,
            open: false,
            complete: [false; 7],
            divisions: FastMap::default(),
            maxima: FastMap::default(),
            manpower: FastMap::default(),
            new: FastMap::default(),
            daily: FastMap::default(),
            pooled: FastMap::default(),
            daily_since: FastSet::default(),
            census: None,
            stock_rows: FastMap::default(),
            stock: None,
            countries: Arc::default(),
            facts: FastMap::default(),
            absent: None,
            wholes: None,
            reads: BTreeMap::new(),
            history: History::default(),
            looks: FastMap::default(),
            alike: Vec::new(),
            templates: FastMap::default(),
            hourly: BTreeSet::new(),
            bare: false,
            tags: Vec::new(),
            numbers: HashMap::new(),
            listed: BTreeMap::new(),
            slots: FastMap::default(),
            fresh_tags: Vec::new(),
            sums: FastMap::default(),
            unlinked: [0; 2],
            by_template: FastMap::default(),
            by_handle: FastMap::default(),
            stock_parts: FastMap::default(),
            given: FastMap::default(),
            stock_given: FastMap::default(),
            dirty: FastMap::default(),
            stock_dirty: BTreeSet::new(),
            notes_dirty: false,
            scratch: Share::default(),
            handles: Default::default(),
            empty: Arc::from([]),
        };
        engine.unclassified = engine.kind(None);
        let warnings: Vec<String> = division_needs::warnings(&engine.content)
            .map(str::to_owned)
            .collect();
        engine.warnings = warnings
            .into_iter()
            .map(|text| engine.literal(&text))
            .collect();
        engine.fixed = Fixed {
            manpower: engine.literal("Division manpower was not recorded"),
            required: engine.literal("Template required manpower was not recorded"),
            maximum: engine.literal("Division maximum hit points were not recorded"),
            pool: engine
                .literal("Division equipment pool was not recorded within a day of this hour"),
            unclassified: engine.literal(
                "Division equipment design has no resolved archetype; holdings are unclassified",
            ),
            unlinked: engine.literal(UNLINKED_DIVISION),
            stock_unread: engine
                .literal("National equipment stockpile was not read within a day of this hour"),
            stock_links: engine
                .literal("National equipment stockpile or its country links were not recorded"),
            stock_unlinked: engine.literal("Stockpile country has no recorded link"),
            stock_unclassified: engine
                .literal("National stockpile design has no resolved archetype"),
        };
        engine
    }

    fn number(&mut self, operand: Operand) -> u32 {
        let id = self.next;
        self.next += 1;
        self.fresh.push((id, operand));
        id
    }

    fn kind(&mut self, key: Option<Arc<str>>) -> u32 {
        if let Some(&id) = self.kinds.get(&key) {
            return id;
        }
        let id = self.number(Operand::Kind(key.clone()));
        self.kinds.insert(key, id);
        id
    }

    fn note(&mut self, reason: KeptReason) -> u32 {
        if let Some(&id) = self.notes.get(&reason) {
            return id;
        }
        let id = self.number(Operand::Note(Arc::new(reason.clone())));
        self.notes.insert(reason, id);
        id
    }

    fn literal(&mut self, text: &str) -> u32 {
        self.note(KeptReason::literal(text))
    }

    fn naming(&mut self, naming: Result<TemplateNaming, KeptReason>) -> u32 {
        let naming = Arc::new(naming);
        if let Some(&id) = self.namings.get(&naming) {
            return id;
        }
        let id = self.number(Operand::Naming(Arc::clone(&naming)));
        self.namings.insert(naming, id);
        id
    }

    fn artwork(&mut self, artwork: Result<TemplateArtwork, KeptReason>) -> u32 {
        let artwork = Arc::new(artwork);
        if let Some(&id) = self.artworks.get(&artwork) {
            return id;
        }
        let id = self.number(Operand::Artwork(Arc::clone(&artwork)));
        self.artworks.insert(artwork, id);
        id
    }

    /// A design's number by its naming operands: the first read of them names it.
    fn design(&mut self, raw: Arc<RawVariant>) -> u32 {
        let operands = NamingOperands(raw);
        if let Some(&id) = self.designs.get(&operands) {
            return id;
        }
        let id = self.number(Operand::Design(Arc::clone(&operands.0)));
        self.designs.insert(operands, id);
        id
    }

    /// The index of country `tag`, numbered when new.
    fn tag(&mut self, tag: &Arc<str>) -> u32 {
        if let Some(&index) = self.numbers.get(tag) {
            return index;
        }
        let index = self.tags.len() as u32;
        self.tags.push(Arc::clone(tag));
        self.numbers.insert(Arc::clone(tag), index);
        self.fresh_tags.push((index, Arc::clone(tag)));
        index
    }

    /// Whether the engine answers its hours: its reads are its own.
    pub(crate) fn own(&self) -> bool {
        self.supplies.own()
    }

    /// Lets go of all but what the segment after cannot read itself: its reads, the pools as
    /// the table and their last read hold them, and its designs' lookups with the operands
    /// they name. Its next hour, the segment after's first, gives every other table whole and
    /// every reply whole.
    pub(crate) fn hand_on(&mut self) {
        self.bare = true;
        self.open = false;
        // Named again by content: the emit catalogues an operand once whatever its number.
        self.namings = BTreeMap::new();
        self.artworks = BTreeMap::new();
        self.divisions = FastMap::default();
        self.maxima = FastMap::default();
        self.manpower = FastMap::default();
        self.new = FastMap::default();
        self.stock_rows = FastMap::default();
        self.countries = Arc::default();
        self.facts = FastMap::default();
        self.absent = None;
        self.wholes = None;
        self.templates = FastMap::default();
        self.hourly = BTreeSet::new();
        self.listed = BTreeMap::new();
        self.slots = FastMap::default();
        self.sums = FastMap::default();
        self.unlinked = [0; 2];
        self.by_template = FastMap::default();
        self.by_handle = FastMap::default();
        self.stock_parts = FastMap::default();
        self.given = FastMap::default();
        self.stock_given = FastMap::default();
        self.dirty = FastMap::default();
        self.stock_dirty = BTreeSet::new();
        self.notes_dirty = false;
        self.scratch = Share::default();
    }

    /// One hour: its rows applied, its reads stepped, and its replies' changes; none while
    /// the reads it stands on are not its own.
    pub(crate) fn step(&mut self, input: &Input) -> io::Result<Option<Output>> {
        let _cpu = ::replay_frame::cpu::scope("division_equipment.engine", None);
        if self.bare {
            if !input.whole {
                return Err(io::Error::other(
                    "A division equipment engine handed on was not given its next hour whole",
                ));
            }
            self.bare = false;
        }
        let hour = input.hour;
        let follows = self.last.and_then(|last| last.checked_add(1)) == Some(hour);
        // The first hour reads what the branch started with; any other only the hour before.
        let continuous = self.last.is_none() || follows;
        let starts = !self.open || !follows;
        self.last = Some(hour);
        let mut touched = Touched::default();
        let complete_before = self.complete;
        self.complete = input.complete;
        self.apply(input, &mut touched)?;
        let output = self.observe(input, (continuous, starts), complete_before, touched);
        // Only the designs of the reads still looked up are kept.
        let wanted = [self.supplies.designs.read, self.supplies.stock_designs.read];
        self.reads.retain(|hour, _| wanted.contains(&Some(*hour)));
        self.history.keep(self.reads.keys().next().copied());
        self.looks
            .retain(|read, _| read.is_none() || wanted.contains(read));
        let reads = &self.reads;
        self.alike
            .retain(|((a, b), _)| reads.contains_key(a) && reads.contains_key(b));
        output
    }

    fn observe(
        &mut self,
        input: &Input,
        (continuous, starts): (bool, bool),
        complete_before: [bool; 7],
        mut touched: Touched,
    ) -> io::Result<Option<Output>> {
        let hour = input.hour;
        if input.gap {
            self.supplies = Supplies::FRESH;
            return Ok(Some(self.unknown(hour)));
        }
        if !continuous {
            self.supplies = Supplies::FRESH;
        }
        let before = self.supplies;
        self.supplies.step(
            [
                self.complete[DAILY],
                self.complete[STOCK],
                self.complete[DEFINITIONS],
            ],
            input.reads,
            hour,
        );
        if self.supplies.designs.read == Some(hour) {
            let designs = input
                .designs
                .as_ref()
                .ok_or_else(|| invalid("A designs read came without its tables"))?;
            let read = Read::of(designs, &mut self.history);
            self.reads.insert(hour, read);
        }
        let census_before = self.census.is_some();
        let read_now = self.supplies.pool.read == Some(hour);
        if read_now {
            for id in self.daily_since.drain() {
                match self.daily.get(&id) {
                    Some(rows) => self.pooled.insert(id, Arc::clone(rows)),
                    None => self.pooled.remove(&id),
                };
                touched.pools.push(id);
            }
            let mut ids = self.census.take().unwrap_or_default();
            self.census = self.complete[DIVISIONS].then(|| {
                ids.clear();
                ids.extend(self.divisions.keys().copied());
                ids.sort_unstable();
                ids
            });
        }
        if self.supplies.stock.read == Some(hour) {
            self.stock = Some(self.stock_read());
        } else if self.supplies.stock.read.is_none() {
            self.stock = None;
        }
        if !self.supplies.own() {
            // Nothing given yet: the hour its reads are its own gives every reply whole.
            self.open = false;
            return Ok(None);
        }
        let gated = self.countries.complete && self.complete[DIVISIONS];
        if !self.supplies.known() || !gated {
            return Ok(Some(self.unknown(hour)));
        }
        let output = if starts || touched.countries {
            self.rebuild(input)?
        } else {
            let every = before.pool.read.is_some() != self.supplies.pool.read.is_some()
                || (read_now && (!self.complete[NEW] || census_before != self.census.is_some()))
                || complete_before[NEW] != self.complete[NEW]
                || complete_before[MAXIMA] != self.complete[MAXIMA]
                || complete_before[MANPOWER] != self.complete[MANPOWER];
            self.update(input, touched, every, before)?
        };
        self.open = true;
        Ok(Some(output))
    }

    /// An hour whose replies are not known.
    fn unknown(&mut self, hour: i32) -> Output {
        self.open = false;
        Output {
            hour,
            known: None,
            operands: std::mem::take(&mut self.fresh),
            tags: std::mem::take(&mut self.fresh_tags),
            whole: false,
            listed: Vec::new(),
            changes: Changes::default(),
        }
    }

    /// The hour's rows into the tables the engine keeps.
    fn apply(&mut self, input: &Input, touched: &mut Touched) -> io::Result<()> {
        if let Some(countries) = &input.countries
            && **countries != *self.countries
        {
            touched.countries = true;
            self.countries = Arc::clone(countries);
        }
        if let Some(context) = &input.context {
            if let Some(held) = self.wholes.replace(context.wholes) {
                touched.sources = held[..2] != context.wholes[..2];
                touched.templates = held[2..] != context.wholes[2..];
            }
            if context.whole {
                self.facts.clear();
            }
            for handle in &context.gone {
                self.facts.remove(handle);
            }
            for (handle, facts) in &context.facts {
                self.facts.insert(*handle, Arc::clone(facts));
            }
            self.absent = Some(Arc::clone(&context.absent));
        }
        // Every designs read given moves the definitions on, read or not.
        if let Some(designs) = &input.designs {
            self.history.read(&designs.definitions, input.hour)?;
        }
        match &input.divisions {
            Rows::Same => {}
            Rows::Whole(rows) => {
                let ids: FastSet<u64> = rows.iter().map(|row| row.id).collect();
                let gone: Vec<u64> = self
                    .divisions
                    .keys()
                    .filter(|id| !ids.contains(id))
                    .copied()
                    .collect();
                for id in gone {
                    self.remove(id, touched);
                }
                for facts in rows {
                    self.set(*facts, touched);
                }
            }
            Rows::Changed(rows, gone) => {
                for &key in gone {
                    self.remove(key as u64, touched);
                }
                for facts in rows {
                    self.set(*facts, touched);
                }
            }
        }
        apply_values(&input.maxima, &mut self.maxima, touched);
        apply_values(&input.manpower, &mut self.manpower, touched);
        let mut changed = Vec::new();
        apply_rows(&input.new, &mut self.new, &mut changed);
        touched.pools.append(&mut changed);
        apply_rows(&input.daily, &mut self.daily, &mut changed);
        self.daily_since.extend(changed.drain(..));
        apply_rows(&input.stock, &mut self.stock_rows, &mut changed);
        Ok(())
    }

    fn set(&mut self, facts: Division, touched: &mut Touched) {
        match self.divisions.get_mut(&facts.id) {
            Some(division) if division.facts == facts => {}
            Some(division) => {
                let before = std::mem::replace(&mut division.facts, facts);
                if !placed_alike(&before, &facts) {
                    touched.structural.push(facts.id);
                } else if before.hit_points != facts.hit_points {
                    touched.numeric.push(facts.id);
                }
            }
            None => {
                self.divisions
                    .insert(facts.id, DivisionState { facts, added: None });
                touched.structural.push(facts.id);
            }
        }
    }

    fn remove(&mut self, id: u64, touched: &mut Touched) {
        if let Some(division) = self.divisions.remove(&id)
            && let Some(added) = division.added
        {
            touched.gone.push((id, added));
        }
    }

    /// A stockpile read of the hour's rows, grouped by its links.
    fn stock_read(&self) -> StockRead {
        let countries = &self.countries;
        let grouped = countries.complete.then(|| {
            let mut slots: FastMap<u32, Vec<(u64, i64)>> = FastMap::default();
            let mut unlinked = 0;
            for (&raw, rows) in &self.stock_rows {
                match link_slot(&countries.links, raw as u32) {
                    Some(slot) => slots
                        .entry(slot)
                        .or_default()
                        .extend(rows.iter().map(|row| (row.1, row.2))),
                    None => unlinked += rows.len(),
                }
            }
            for rows in slots.values_mut() {
                rows.sort_unstable();
            }
            (slots, unlinked)
        });
        StockRead {
            grouped,
            ids: countries.ids.clone(),
        }
    }

    /// Every reply computed again and given whole, as after an hour the replies were not
    /// known or the countries changed.
    fn rebuild(&mut self, input: &Input) -> io::Result<Output> {
        self.sums.clear();
        self.unlinked = [0; 2];
        self.by_template.clear();
        self.by_handle.clear();
        self.stock_parts.clear();
        self.given.clear();
        self.stock_given.clear();
        self.dirty.clear();
        self.stock_dirty.clear();
        self.notes_dirty = false;
        self.templates.clear();
        self.hourly.clear();
        self.listed.clear();
        self.slots.clear();
        let countries = Arc::clone(&self.countries);
        for (tag, holder) in &countries.holders {
            let index = self.tag(tag);
            self.listed.insert(index, *holder);
            if let Some(holder) = holder {
                self.slots.entry(holder.slot).or_default().push(index);
            }
        }
        // In id order, as an hour places them: a division numbers the operands it names first.
        let mut ids: Vec<u64> = self.divisions.keys().copied().collect();
        ids.sort_unstable();
        for division in self.divisions.values_mut() {
            division.added = None;
        }
        self.given()?;
        for &id in &ids {
            self.place(id)?;
        }
        let listed: Vec<(u32, Option<Holder>)> = self
            .listed
            .iter()
            .map(|(&tag, &holder)| (tag, holder))
            .collect();
        for &(tag, holder) in &listed {
            if let Some(holder) = holder {
                let part = self.stock_part(holder);
                self.stock_parts.insert(tag, part);
            }
        }
        self.dirty.clear();
        self.stock_dirty.clear();
        self.notes_dirty = false;
        let mut changes = Changes::default();
        for &(tag, _) in &listed {
            self.whole(tag, &mut changes);
        }
        Ok(Output {
            hour: input.hour,
            known: Some([self.supplies.pool.read, self.supplies.stock.read]),
            operands: std::mem::take(&mut self.fresh),
            tags: std::mem::take(&mut self.fresh_tags),
            whole: true,
            listed: listed.into_iter().map(|(tag, _)| tag).collect(),
            changes: changes.seal(),
        })
    }

    /// Whether the templates' facts were ever given.
    fn given(&self) -> io::Result<()> {
        match self.absent {
            Some(_) => Ok(()),
            None => Err(invalid("Division equipment's tables were never given")),
        }
    }

    /// Template `handle` as its facts last given name it, its operands numbered.
    fn template(&mut self, handle: u64) -> io::Result<Template> {
        let facts = match self.facts.get(&handle) {
            Some(facts) => Arc::clone(facts),
            None => {
                let absent = self
                    .absent
                    .as_deref()
                    .ok_or_else(|| invalid("Division equipment's tables were never given"))?;
                Arc::new(absent.facts(handle))
            }
        };
        let need = match &facts.need {
            Ok(need) => {
                let mut need: Vec<(u32, i64)> = need
                    .iter()
                    .map(|(key, amount)| (self.kind(Some(Arc::from(key.as_str()))), *amount))
                    .collect();
                // A kind needed twice is compared at its first amount.
                need.sort_by_key(|&(kind, _)| kind);
                need.dedup_by_key(|&mut (kind, _)| kind);
                Ok(need.into())
            }
            Err(why) => Err(self.note(why.clone())),
        };
        let naming = self.naming(facts.naming.clone());
        let artwork = self.artwork(facts.artwork.clone());
        Ok(Template {
            need,
            required: facts.required,
            naming,
            artwork,
            hourly: facts.hourly,
        })
    }

    fn keep_template(&mut self, handle: u64, template: Template) {
        if template.hourly {
            self.hourly.insert(handle);
        } else {
            self.hourly.remove(&handle);
        }
        self.templates.insert(handle, template);
    }

    /// An hour whose countries, links and local sub-units are the hour before's: only what
    /// its changes reach is computed again.
    fn update(
        &mut self,
        input: &Input,
        mut touched: Touched,
        every: bool,
        before: Supplies,
    ) -> io::Result<Output> {
        self.given()?;
        // Appended tokens spell only what was unspelled: only refused needs can change. Local
        // sub-units and tokens reach the needs alone. Asked in handle order: a template numbers
        // the operands it names first.
        let every_template = |templates: &FastMap<u64, Template>| {
            let mut asked: Vec<u64> = templates.keys().copied().collect();
            asked.sort_unstable();
            asked
        };
        let asked: Vec<u64> = match (&input.templates, input.tokens) {
            _ if input.subunits || touched.sources || touched.templates => {
                every_template(&self.templates)
            }
            (_, Tokens::Changed) | (None, _) => every_template(&self.templates),
            (Some(changed), tokens) => {
                let mut asked: BTreeSet<u64> = changed
                    .iter()
                    .copied()
                    .filter(|handle| self.templates.contains_key(handle))
                    .collect();
                asked.extend(self.hourly.iter().copied());
                if tokens == Tokens::Appended {
                    asked.extend(
                        self.templates
                            .iter()
                            .filter(|(_, template)| template.need.is_err())
                            .map(|(handle, _)| *handle),
                    );
                }
                asked.into_iter().collect()
            }
        };
        let mut relabelled = BTreeSet::new();
        for handle in asked {
            let now = self.template(handle)?;
            let Some(held) = self.templates.get(&handle) else {
                continue;
            };
            if held.need != now.need || held.required != now.required {
                touched
                    .structural
                    .extend(self.by_template.get(&handle).into_iter().flatten().copied());
            }
            if (held.naming, held.artwork) != (now.naming, now.artwork) {
                relabelled.insert(handle);
            }
            self.keep_template(handle, now);
        }
        for (id, added) in std::mem::take(&mut touched.gone) {
            self.take(id, &added);
        }
        // Placed in id order: a division numbers the operands it names first.
        let mut refresh = std::mem::take(&mut touched.structural);
        if every {
            refresh.clear();
            refresh.extend(self.divisions.keys().copied());
        } else {
            refresh.append(&mut touched.pools);
        }
        // A new designs read: each design held is looked up in it, and only those it looks up
        // otherwise reach their holders, a classification their divisions too.
        let (was, read) = (before.designs.read, self.supplies.designs.read);
        if was != read {
            let handles: Vec<u64> = self.by_handle.keys().copied().collect();
            self.look_up(read, was, input.definitions.as_deref(), &handles);
            for handle in handles {
                let (then, now) = (self.look(was, handle), self.look(read, handle));
                let (classified, named) = match (then, now) {
                    (Some(then), Some(now)) if Arc::ptr_eq(then, now) => continue,
                    (Some(then), Some(now)) => (
                        then.kind != now.kind || then.notes != now.notes,
                        then.raw != now.raw || then.notes != now.notes,
                    ),
                    _ => (true, true),
                };
                let users: Vec<u64> = self
                    .by_handle
                    .get(&handle)
                    .into_iter()
                    .flatten()
                    .copied()
                    .collect();
                if classified {
                    refresh.extend(users.iter().copied());
                }
                if named {
                    self.renamed(handle, &users);
                }
            }
        }
        refresh.sort_unstable();
        refresh.dedup();
        for &id in &refresh {
            self.place(id)?;
        }
        let mut numeric = std::mem::take(&mut touched.numeric);
        numeric.sort_unstable();
        numeric.dedup();
        for id in numeric {
            if refresh.binary_search(&id).is_err() {
                self.renumber(id)?;
            }
        }
        // A new stockpile read gives again only the countries whose stockpile part it changes.
        if (before.stock.read, before.stock_designs.read)
            != (self.supplies.stock.read, self.supplies.stock_designs.read)
        {
            let listed: Vec<(u32, Holder)> = self
                .listed
                .iter()
                .filter_map(|(&tag, holder)| Some((tag, (*holder)?)))
                .collect();
            for (tag, holder) in listed {
                let part = self.stock_part(holder);
                if self.stock_parts.get(&tag) != Some(&part) {
                    self.stock_parts.insert(tag, part);
                    self.stock_dirty.insert(tag);
                }
            }
        }
        for handle in relabelled {
            for id in self.by_template.get(&handle).into_iter().flatten() {
                let Some(added) = self
                    .divisions
                    .get(id)
                    .and_then(|state| state.added.as_ref())
                else {
                    continue;
                };
                for (side, slot) in added.placed.slots.into_iter().enumerate() {
                    if let Some(slot) = slot {
                        self.dirty
                            .entry((slot, side as u8))
                            .or_default()
                            .templates
                            .insert(handle);
                    }
                }
            }
        }
        Ok(self.changes(input.hour))
    }

    /// The replies holding `handle` give its design again, and their notes.
    fn renamed(&mut self, handle: u64, users: &[u64]) {
        for id in users {
            let Some(added) = self
                .divisions
                .get(id)
                .and_then(|state| state.added.as_ref())
            else {
                continue;
            };
            let kinds = added
                .share
                .variants
                .iter()
                .filter(|&&(_, held, _)| held == handle)
                .map(|&(kind, _, _)| kind);
            for (side, slot) in added.placed.slots.into_iter().enumerate() {
                let Some(slot) = slot else {
                    continue;
                };
                let dirty = self.dirty.entry((slot, side as u8)).or_default();
                dirty.notes = true;
                dirty
                    .variants
                    .extend(kinds.clone().map(|kind| (kind, handle)));
            }
        }
    }

    /// Division `id`'s part made again, and what differs from what it added changed.
    fn place(&mut self, id: u64) -> io::Result<()> {
        let Some(state) = self.divisions.get_mut(&id) else {
            return Ok(());
        };
        let facts = state.facts;
        let was = state.added.take();
        let Some(placed) = super::super::place(&facts.row(), &self.countries.links) else {
            if let Some(was) = was {
                self.take(id, &was);
            }
            return Ok(());
        };
        if !self.templates.contains_key(&placed.template) {
            let template = self.template(placed.template)?;
            self.keep_template(placed.template, template);
        }
        let pool = self.pool(id);
        let read = self.supplies.designs.read;
        let looked = self.looks.get(&read);
        if let Some(rows) = &pool
            && rows
                .iter()
                .any(|row| looked.is_none_or(|looks| !looks.contains_key(&row.1)))
        {
            let handles: Vec<u64> = rows.iter().map(|row| row.1).collect();
            self.look_up(read, read, None, &handles);
        }
        let mut share = std::mem::take(&mut self.scratch);
        share.fill(
            pool.as_deref(),
            self.looks.get(&read),
            &self.templates[&placed.template],
            (&self.fixed, self.unclassified),
        );
        let numbers = self.numbers_of(&facts);
        // A part is kept as a copy of the room it was made in, holding no room past its size.
        let added = match was {
            Some(mut was) if was.placed == placed => {
                if (&was.share, was.numbers) != (&share, numbers) {
                    self.shift(
                        id,
                        placed,
                        Some((&was.share, &was.numbers)),
                        Some((&share, &numbers)),
                    );
                    was.share = share.clone();
                    was.numbers = numbers;
                }
                was
            }
            was => {
                if let Some(was) = was {
                    self.take(id, &was);
                }
                let added = Added {
                    placed,
                    share: share.clone(),
                    numbers,
                };
                self.put(id, &added);
                added
            }
        };
        self.scratch = share;
        if let Some(state) = self.divisions.get_mut(&id) {
            state.added = Some(added);
        }
        Ok(())
    }

    /// A division whose numbers changed alone: their differences into its template's sums,
    /// or its part again where one appeared or went.
    fn renumber(&mut self, id: u64) -> io::Result<()> {
        let Some(state) = self.divisions.get(&id) else {
            return Ok(());
        };
        let numbers = self.numbers_of(&state.facts);
        let Some(added) = &state.added else {
            return Ok(());
        };
        let held = added.numbers;
        if held == numbers {
            return Ok(());
        }
        if held.maximum.is_some() != numbers.maximum.is_some()
            || held.manpower.is_some() != numbers.manpower.is_some()
        {
            return self.place(id);
        }
        let (placed, template) = (added.placed, added.placed.template);
        for (side, slot) in placed.slots.into_iter().enumerate() {
            let Some(slot) = slot else {
                continue;
            };
            let sums = self.sums.entry((slot, side as u8)).or_default();
            if let Some(total) = sums.templates.get_mut(&template) {
                total.hp += i128::from(numbers.hp) - i128::from(held.hp);
                total.maximum.1 += i128::from(numbers.maximum.unwrap_or(0))
                    - i128::from(held.maximum.unwrap_or(0));
                total.manpower.1 += i128::from(numbers.manpower.unwrap_or(0))
                    - i128::from(held.manpower.unwrap_or(0));
            }
            self.dirty
                .entry((slot, side as u8))
                .or_default()
                .templates
                .insert(template);
        }
        if let Some(added) = self
            .divisions
            .get_mut(&id)
            .and_then(|state| state.added.as_mut())
        {
            added.numbers = numbers;
        }
        Ok(())
    }

    fn numbers_of(&self, facts: &Division) -> Numbers {
        let id = facts.id;
        Numbers {
            hp: facts.hit_points,
            maximum: self.complete[MAXIMA]
                .then(|| self.maxima.get(&id).copied())
                .flatten(),
            manpower: self.complete[MANPOWER]
                .then(|| self.manpower.get(&id).copied())
                .flatten(),
        }
    }

    /// The division's pool as its replies read it, or none when it was not read.
    fn pool(&self, id: u64) -> Option<Arc<[Row]>> {
        if self.complete[NEW]
            && let Some(rows) = self.new.get(&id).filter(|rows| !rows.is_empty())
        {
            return Some(Arc::clone(rows));
        }
        let daily = self.supplies.pool.read.is_some();
        let existed = self
            .census
            .as_ref()
            .filter(|_| daily)
            .map(|ids| ids.binary_search(&id).is_ok());
        if daily {
            match self.pooled.get(&id).filter(|rows| !rows.is_empty()) {
                Some(rows) => return Some(Arc::clone(rows)),
                None if existed == Some(true) => return Some(Arc::clone(&self.empty)),
                None => {}
            }
        }
        if existed == Some(false) && daily && self.complete[NEW] {
            return Some(Arc::clone(&self.empty));
        }
        None
    }

    /// Puts division `id`'s part into its replies.
    fn put(&mut self, id: u64, added: &Added) {
        self.shift(id, added.placed, None, Some((&added.share, &added.numbers)));
    }

    /// Takes what a division added out of its replies; `id` names it while it is kept.
    fn take(&mut self, id: u64, added: &Added) {
        self.shift(id, added.placed, Some((&added.share, &added.numbers)), None);
    }

    /// What division `id` adds at `placed` moved from `was` to `now`, none for nothing.
    fn shift(
        &mut self,
        id: u64,
        placed: Placed,
        was: Option<(&Share, &Numbers)>,
        now: Option<(&Share, &Numbers)>,
    ) {
        let template = placed.template;
        match (was.is_some(), now.is_some()) {
            (false, true) => {
                self.by_template.entry(template).or_default().insert(id);
            }
            (true, false) => {
                if let Some(users) = self.by_template.get_mut(&template) {
                    users.remove(&id);
                    if users.is_empty() {
                        self.by_template.remove(&template);
                    }
                }
            }
            _ => {}
        }
        let mut handles = std::mem::take(&mut self.handles);
        was.map_or(&NO_SHARE, |(share, _)| share)
            .handles(&mut handles[0]);
        now.map_or(&NO_SHARE, |(share, _)| share)
            .handles(&mut handles[1]);
        merged(
            &handles[0],
            &handles[1],
            |&handle| handle,
            |handle, gone, come| match (gone, come) {
                (Some(_), None) => {
                    if let Some(users) = self.by_handle.get_mut(&handle) {
                        users.remove(&id);
                        if users.is_empty() {
                            self.by_handle.remove(&handle);
                        }
                    }
                }
                (None, Some(_)) => {
                    self.by_handle.entry(handle).or_default().insert(id);
                }
                _ => {}
            },
        );
        let fixed = (self.fixed.manpower, self.fixed.maximum);
        for (side, slot) in placed.slots.into_iter().enumerate() {
            let Some(slot) = slot else {
                if recount(&mut self.unlinked[side], was.is_some(), now.is_some()) {
                    self.notes_dirty = true;
                }
                continue;
            };
            let sums = self.sums.entry((slot, side as u8)).or_default();
            let dirty = self.dirty.entry((slot, side as u8)).or_default();
            shift_sums(sums, dirty, template, (was, now), &handles, fixed);
            if sums.divisions == 0 {
                self.sums.remove(&(slot, side as u8));
            }
        }
        self.handles = handles;
    }

    /// Whether reads `a` and `b` look a design up alike wherever its rows are alike.
    fn alike(&mut self, a: i32, b: i32) -> bool {
        let pair = (a.min(b), a.max(b));
        if let Some(&(_, alike)) = self.alike.iter().find(|(held, _)| *held == pair) {
            return alike;
        }
        let alike = match (self.reads.get(&a), self.reads.get(&b)) {
            (Some(a), Some(b)) => designs_alike(a, b),
            _ => false,
        };
        self.alike.push((pair, alike));
        alike
    }

    /// `handle`'s lookup in read `other`, when read `hour` looks it up alike: the same
    /// definition fields and token spelling a lookup reads.
    fn reused(&self, other: i32, hour: i32, handle: u64) -> Option<Arc<Look>> {
        let look = self.looks.get(&Some(other))?.get(&handle)?;
        let (was, now) = (self.reads.get(&other)?, self.reads.get(&hour)?);
        let alike = match (self.history.at(handle, other), self.history.at(handle, hour)) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                (std::ptr::eq(a, b) || a.looked() == b.looked())
                    && (Arc::ptr_eq(&was.tokens, &now.tokens)
                        || was.spelling(b.token()) == now.spelling(b.token()))
            }
            _ => false,
        };
        alike.then(|| Arc::clone(look))
    }

    /// Look up `handles` in the designs read at `read` (none: no read), each once a read. A
    /// design another read looked up alike is taken from it; one whose definition read `read`
    /// did not change (`definitions`) since `was`, alike with the same tokens, without
    /// comparing its rows.
    fn look_up(
        &mut self,
        read: Option<i32>,
        was: Option<i32>,
        definitions: Option<&[u64]>,
        handles: &[u64],
    ) {
        let held = self.looks.get(&read);
        let missing: BTreeSet<u64> = handles
            .iter()
            .copied()
            .filter(|handle| held.is_none_or(|held| !held.contains_key(handle)))
            .collect();
        if missing.is_empty() {
            return;
        }
        let mut found = Vec::with_capacity(missing.len());
        let mut fresh = Vec::new();
        // Designs whose definitions did not change keep their lookup between two alike reads
        // of the same tokens.
        let unchanged = match (read, was, definitions) {
            (Some(hour), Some(other), Some(definitions)) if other != hour => {
                // The tokens its definitions name, each read's.
                let same_tokens = match (self.reads.get(&other), self.reads.get(&hour)) {
                    (Some(was), Some(now)) => {
                        was.spelled == now.spelled
                            && (Arc::ptr_eq(&was.tokens, &now.tokens) || was.tokens == now.tokens)
                    }
                    _ => false,
                };
                (same_tokens && self.alike(other, hour)).then(|| {
                    let changed: FastSet<u64> = definitions.iter().copied().collect();
                    (other, changed)
                })
            }
            _ => None,
        };
        let others: Vec<i32> = match read {
            Some(hour) => {
                let others: Vec<i32> = self
                    .reads
                    .keys()
                    .copied()
                    .filter(|&other| other != hour)
                    .collect();
                others
                    .into_iter()
                    .filter(|&other| self.alike(other, hour))
                    .collect()
            }
            None => Vec::new(),
        };
        for handle in missing {
            if let Some((other, changed)) = &unchanged
                && !changed.contains(&handle)
                && let Some(look) = self
                    .looks
                    .get(&Some(*other))
                    .and_then(|looks| looks.get(&handle))
            {
                found.push((handle, Arc::clone(look)));
                continue;
            }
            let reused = read.and_then(|hour| {
                others
                    .iter()
                    .find_map(|&other| self.reused(other, hour, handle))
            });
            match reused {
                Some(look) => found.push((handle, look)),
                None => fresh.push(handle),
            }
        }
        if !fresh.is_empty() {
            let looked: Vec<(u64, super::super::Looked)> = {
                let kept = read.and_then(|hour| Some((hour, self.reads.get(&hour)?)));
                let as_of = kept.map(|(hour, _)| AsOf {
                    history: &self.history,
                    hour,
                });
                let parts = kept.zip(as_of.as_ref()).and_then(|((hour, read), as_of)| {
                    read.spelled.then_some(())?;
                    let tags = read.tags.as_deref()?;
                    Some((
                        hour,
                        equipment::Tags::new(
                            &tags.countries,
                            &tags.links,
                            tags.appearance.as_deref(),
                        ),
                        army_content::Definitions::Kept(as_of),
                        &read.tokens[..],
                    ))
                });
                let designs = army_content::Designs::of(parts, self.cat.as_ref());
                fresh
                    .into_iter()
                    .map(|handle| (handle, super::super::Looked::of(&designs, handle)))
                    .collect()
            };
            for (handle, looked) in looked {
                let notes = looked
                    .notes
                    .iter()
                    .map(|(note, times)| (self.literal(note), *times))
                    .collect();
                let (kind, raw) = match looked.operand {
                    Some(operand) => (
                        operand.archetype.map(|key| self.kind(Some(Arc::from(key)))),
                        Some(self.design(operand.raw)),
                    ),
                    None => (None, None),
                };
                found.push((handle, Arc::new(Look { notes, kind, raw })));
            }
        }
        self.looks.entry(read).or_default().extend(found);
    }

    fn look(&self, read: Option<i32>, handle: u64) -> Option<&Arc<Look>> {
        self.looks.get(&read)?.get(&handle)
    }

    /// `holder`'s stockpile part.
    fn stock_part(&mut self, holder: Holder) -> StockPart {
        let Some(stock) = self.stock.take() else {
            return StockPart::refused(self.fixed.stock_unread);
        };
        let part = self.stock_part_of(&stock, holder);
        self.stock = Some(stock);
        part
    }

    /// `holder`'s part of stockpile read `stock`.
    fn stock_part_of(&mut self, stock: &StockRead, holder: Holder) -> StockPart {
        let same = match &stock.ids {
            None => Err("its country identity was not recorded"),
            Some(ids) => match ids.get(&holder.slot) {
                None => Err("its country identity was not recorded"),
                Some(&id) if id == holder.id => Ok(()),
                Some(_) => Err("it belongs to another country identity"),
            },
        };
        if let Err(why) = same {
            let note = format!("National equipment stockpile is left out: {why} at its read");
            return StockPart::refused(self.literal(&note));
        }
        let Some((slots, unlinked)) = &stock.grouped else {
            return StockPart::refused(self.fixed.stock_links);
        };
        let rows = slots.get(&holder.slot).map_or(&[][..], Vec::as_slice);
        let read = self.supplies.stock_designs.read;
        let looked = self.looks.get(&read);
        if rows
            .iter()
            .any(|(handle, _)| looked.is_none_or(|looks| !looks.contains_key(handle)))
        {
            let handles: Vec<u64> = rows.iter().map(|&(handle, _)| handle).collect();
            self.look_up(read, read, None, &handles);
        }
        let looks = self.looks.get(&read);
        let mut notes = Vec::new();
        let mut complete = true;
        if *unlinked > 0 {
            notes.push((self.fixed.stock_unlinked, *unlinked as u32));
            complete = false;
        }
        let mut totals = Vec::new();
        let mut designs = Vec::new();
        // Each design's rows together: its lookup's notes count once.
        for rows in rows.chunk_by(|a, b| a.0 == b.0) {
            let handle = rows[0].0;
            let look = looks.and_then(|looks| looks.get(&handle));
            for &(note, times) in look.iter().flat_map(|look| &look.notes) {
                notes.push((note, times as u32));
            }
            designs.push((handle, look.and_then(|look| look.raw)));
            match look.and_then(|look| look.kind) {
                Some(kind) => {
                    let amount = rows.iter().map(|&(_, amount)| i128::from(amount)).sum();
                    totals.push((kind, amount));
                }
                None => {
                    notes.push((self.fixed.stock_unclassified, rows.len() as u32));
                    complete = false;
                }
            }
        }
        summed(&mut notes);
        summed(&mut totals);
        StockPart {
            totals: complete.then_some(totals),
            notes,
            designs,
        }
    }

    /// The hour's changes: each dirty reply's dirty values, and every value of the countries
    /// a stockpile read or the unlinked count reached.
    fn changes(&mut self, hour: i32) -> Output {
        let mut changes = Changes::default();
        let dirty = std::mem::take(&mut self.dirty);
        let stock_dirty = std::mem::take(&mut self.stock_dirty);
        let notes_all = std::mem::take(&mut self.notes_dirty);
        let mut touched: BTreeMap<(u32, u8), Dirty> = BTreeMap::new();
        for ((slot, side), reply) in dirty {
            match self.slots.get(&slot).map_or(&[][..], Vec::as_slice) {
                &[tag] => {
                    touched.insert((tag, side), reply);
                }
                tags => {
                    for &tag in tags {
                        let into = touched.entry((tag, side)).or_default();
                        into.counts |= reply.counts;
                        into.unknown |= reply.unknown;
                        into.types.extend(reply.types.iter().copied());
                        into.variants.extend(reply.variants.iter().copied());
                        into.templates.extend(reply.templates.iter().copied());
                        into.notes |= reply.notes;
                    }
                }
            }
        }
        for &tag in &stock_dirty {
            for side in 0..2 {
                touched.entry((tag, side)).or_default().notes = true;
            }
        }
        if notes_all {
            let tags: Vec<u32> = self.listed.keys().copied().collect();
            for tag in tags {
                for side in 0..2 {
                    touched.entry((tag, side)).or_default().notes = true;
                }
            }
        }
        for ((tag, side), dirty) in touched {
            let Some(Some(holder)) = self.listed.get(&tag).copied() else {
                continue;
            };
            self.reply(
                (tag, side, holder.slot),
                &dirty,
                (stock_dirty.contains(&tag), false),
                &mut changes,
            );
        }
        Output {
            hour,
            known: Some([self.supplies.pool.read, self.supplies.stock.read]),
            operands: std::mem::take(&mut self.fresh),
            tags: std::mem::take(&mut self.fresh_tags),
            whole: false,
            listed: Vec::new(),
            changes: changes.seal(),
        }
    }

    /// Every value of country `tag`, both attributions.
    fn whole(&mut self, tag: u32, changes: &mut Changes) {
        let holder = self.listed.get(&tag).copied().flatten();
        for side in 0..2 {
            let Some(holder) = holder else {
                changes.push(Change {
                    tag,
                    side,
                    field: Field::Refused,
                    value: Some(Value::Number(1)),
                });
                continue;
            };
            let (types, templates) = match self.sums.get(&(holder.slot, side)) {
                Some(sums) => (
                    sums.types.keys().copied().collect(),
                    sums.templates.keys().copied().collect(),
                ),
                None => (BTreeSet::new(), BTreeSet::new()),
            };
            let dirty = Dirty {
                counts: true,
                unknown: true,
                types,
                variants: BTreeSet::new(),
                templates,
                notes: true,
            };
            changes.push(Change {
                tag,
                side,
                field: Field::State,
                value: Some(Value::Number(1)),
            });
            self.reply((tag, side, holder.slot), &dirty, (true, true), changes);
        }
    }

    /// The dirty values of country `tag`'s reply for `side`, its holder at `slot`; every
    /// type's stock and the stockpile's designs too when `stock`, every design when `whole`.
    /// A slot one country holds keeps what it gave, and leaves out what is given unchanged.
    fn reply(
        &mut self,
        (tag, side, slot): (u32, u8, u32),
        dirty: &Dirty,
        (stock, whole): (bool, bool),
        changes: &mut Changes,
    ) {
        let read = self.supplies.designs.read;
        let kept = self.slots.get(&slot).is_some_and(|tags| tags.len() == 1);
        let mut empty = Sums::default();
        let sums = self.sums.get_mut(&(slot, side)).unwrap_or(&mut empty);
        let part = self.stock_parts.get(&tag);
        let mut give = |field: Field, value: Option<Value>, given: Option<&mut Given>| {
            if kept && let Some(given) = given {
                if *given == Some(value) {
                    return;
                }
                *given = Some(value);
            }
            changes.push(Change {
                tag,
                side,
                field,
                value,
            });
        };
        let number = |value: Option<i128>| value.map(Value::Number);
        if dirty.counts {
            let counts = [sums.divisions, sums.short, sums.unknown];
            for ((field, count), given) in [Field::Divisions, Field::Short, Field::Unknown]
                .into_iter()
                .zip(counts)
                .zip(&mut sums.given)
            {
                give(field, number(Some(i128::from(count))), Some(given));
            }
        }
        let every = dirty.unknown || stock;
        let mut types: BTreeSet<u32> = dirty.types.clone();
        if every {
            types.extend(sums.types.keys().copied());
        }
        // A design the hour changed is given in its type's place, which orders the keys it makes.
        types.extend(dirty.variants.iter().map(|&(kind, _)| kind));
        let (unclassified, unknown) = (self.unclassified, sums.unknown);
        for kind in types {
            let measured = dirty.types.contains(&kind) || (every && sums.types.contains_key(&kind));
            let Some(total) = sums.types.get_mut(&kind) else {
                for measure in MEASURES.into_iter().filter(|_| measured) {
                    give(Field::Type(kind, measure), None, None);
                }
                continue;
            };
            let known = total.known > 0;
            let stocked = part
                .and_then(|part| part.totals.as_ref())
                .filter(|_| kind != unclassified)
                .map(|totals| {
                    totals
                        .binary_search_by_key(&kind, |&(kind, _)| kind)
                        .map_or(0, |at| totals[at].1)
                });
            let values = [
                Some(total.held),
                known.then_some(total.needed),
                known.then_some(total.filled),
                known.then_some(total.missing),
                stocked,
                Some(i128::from(total.short)),
                Some(i128::from(unknown)),
            ];
            if measured {
                let values = MEASURES.into_iter().zip(values).zip(&mut total.given);
                for ((measure, value), given) in values {
                    let given = (measure != Measure::Stock).then_some(given);
                    give(Field::Type(kind, measure), number(value), given);
                }
            }
            if whole {
                for (&handle, variant) in &mut total.variants {
                    let value = number(Some(variant.amount));
                    give(
                        Field::Variant(kind, handle),
                        value,
                        Some(&mut variant.given[0]),
                    );
                }
            } else {
                for &(_, handle) in dirty.variants.range((kind, 0)..=(kind, u64::MAX)) {
                    if let Some(variant) = total.variants.get_mut(&handle) {
                        let value = number(Some(variant.amount));
                        give(
                            Field::Variant(kind, handle),
                            value,
                            Some(&mut variant.given[0]),
                        );
                    }
                }
            }
        }
        let looks = self.looks.get(&read);
        let design = |handle: u64| {
            read?;
            looks?.get(&handle)?.raw
        };
        for &(kind, handle) in &dirty.variants {
            let held = sums
                .types
                .get_mut(&kind)
                .and_then(|total| total.variants.get_mut(&handle));
            match held {
                Some(variant) => {
                    let [amount, named] = &mut variant.given;
                    let value = number(Some(variant.amount));
                    give(Field::Variant(kind, handle), value, Some(amount));
                    let value = design(handle).map(Value::Operand);
                    give(Field::Design(kind, handle), value, Some(named));
                }
                None => {
                    give(Field::Variant(kind, handle), None, None);
                    give(Field::Design(kind, handle), None, None);
                }
            }
        }
        // A type given whole gives its designs.
        for &kind in dirty.types.iter().filter(|_| whole) {
            if let Some(total) = sums.types.get_mut(&kind) {
                for (&handle, variant) in &mut total.variants {
                    let value = design(handle).map(Value::Operand);
                    give(
                        Field::Design(kind, handle),
                        value,
                        Some(&mut variant.given[1]),
                    );
                }
            }
        }
        for &handle in &dirty.templates {
            let Some(total) = sums.templates.get_mut(&handle) else {
                for part in PARTS {
                    give(Field::Template(handle, part), None, None);
                }
                continue;
            };
            let present = |value: (u32, i128)| (value.0 == 0).then_some(value.1);
            let known = total.known > 0;
            let template = self.templates.get(&handle);
            let values = [
                number(Some(i128::from(total.divisions))),
                number(Some(i128::from(total.unknown))),
                number(Some(total.short.len() as i128)),
                number(present(total.manpower)),
                number(present(total.required)),
                number(Some(total.hp)),
                number(present(total.maximum)),
                number(known.then_some(total.needed)),
                number(known.then_some(total.held)),
                template.map(|template| Value::Operand(template.naming)),
                template.map(|template| Value::Operand(template.artwork)),
            ];
            for ((part, value), given) in PARTS.into_iter().zip(values).zip(&mut total.given) {
                give(Field::Template(handle, part), value, Some(given));
            }
        }
        if dirty.notes {
            let mut notes: BTreeMap<u32, u32> = sums.notes.clone();
            if self.unlinked[side as usize] > 0 {
                *notes.entry(self.fixed.unlinked).or_default() += self.unlinked[side as usize];
            }
            for &warning in &self.warnings {
                *notes.entry(warning).or_default() += 1;
            }
            // A design's lookup notes count once in each reply that reads it.
            for &handle in sums.handles.keys() {
                for &(note, times) in looks
                    .and_then(|looks| looks.get(&handle))
                    .into_iter()
                    .flat_map(|look| &look.notes)
                {
                    *notes.entry(note).or_default() += times as u32;
                }
            }
            for &(note, times) in part.into_iter().flat_map(|part| &part.notes) {
                *notes.entry(note).or_default() += times;
            }
            let given = self.given.entry((tag, side)).or_default();
            for (&note, _) in given.iter().filter(|(note, _)| !notes.contains_key(note)) {
                changes.push(Change {
                    tag,
                    side,
                    field: Field::Note(note),
                    value: None,
                });
            }
            for (&note, &times) in &notes {
                if given.get(&note) != Some(&times) {
                    changes.push(Change {
                        tag,
                        side,
                        field: Field::Note(note),
                        value: Some(Value::Number(i128::from(times))),
                    });
                }
            }
            *given = notes;
        }
        if stock {
            let read = self.supplies.stock_designs.read;
            // In design order, as the part holds them.
            let designs: Vec<(u64, u32)> = part
                .into_iter()
                .flat_map(|part| &part.designs)
                .filter_map(|&(handle, raw)| Some((handle, read.and(raw)?)))
                .collect();
            let given = self.stock_given.remove(&(tag, side)).unwrap_or_default();
            for &(handle, raw) in &designs {
                if given.binary_search(&(handle, raw)).is_err() {
                    changes.push(Change {
                        tag,
                        side,
                        field: Field::StockDesign(handle),
                        value: Some(Value::Operand(raw)),
                    });
                }
            }
            // A design the stockpile no longer holds ends; the emit ends what it never held.
            for &(handle, _) in &given {
                if designs
                    .binary_search_by_key(&handle, |&(handle, _)| handle)
                    .is_err()
                {
                    changes.push(Change {
                        tag,
                        side,
                        field: Field::StockDesign(handle),
                        value: None,
                    });
                }
            }
            self.stock_given.insert((tag, side), designs);
        }
    }
}

fn damaged() -> io::Error {
    invalid("A saved division equipment engine is damaged")
}

fn put_count(out: &mut Vec<u8>, count: usize) -> io::Result<()> {
    let count = u32::try_from(count)
        .map_err(|_| invalid("A division equipment engine holds more than it can save"))?;
    out.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn put_hour(out: &mut Vec<u8>, hour: Option<i32>) {
    match hour {
        None => out.push(0),
        Some(hour) => {
            out.push(1);
            out.extend_from_slice(&hour.to_le_bytes());
        }
    }
}

fn hour_of(input: &mut Decoder<'_>) -> io::Result<Option<i32>> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(input.i32()?)),
        _ => Err(damaged()),
    }
}

fn put_rows(out: &mut Vec<u8>, map: &FastMap<u64, Arc<[Row]>>) -> io::Result<()> {
    put_count(out, map.len())?;
    for (&owner, rows) in map {
        out.extend_from_slice(&owner.to_le_bytes());
        put_count(out, rows.len())?;
        for &(ordinal, variant, amount) in rows.iter() {
            out.extend_from_slice(&ordinal.to_le_bytes());
            out.extend_from_slice(&variant.to_le_bytes());
            out.extend_from_slice(&amount.to_le_bytes());
        }
    }
    Ok(())
}

fn rows_of(input: &mut Decoder<'_>) -> io::Result<FastMap<u64, Arc<[Row]>>> {
    let owners = input.count(12)?;
    let mut map = FastMap::with_capacity_and_hasher(owners, Default::default());
    for _ in 0..owners {
        let owner = input.u64()?;
        let count = input.count(20)?;
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            rows.push((input.u32()?, input.u64()?, input.i64()?));
        }
        map.insert(owner, Arc::from(rows));
    }
    Ok(map)
}

fn put_values(out: &mut Vec<u8>, map: &FastMap<u64, i64>) -> io::Result<()> {
    put_count(out, map.len())?;
    for (&id, &value) in map {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

fn values_of(input: &mut Decoder<'_>) -> io::Result<FastMap<u64, i64>> {
    let count = input.count(16)?;
    let mut map = FastMap::with_capacity_and_hasher(count, Default::default());
    for _ in 0..count {
        map.insert(input.u64()?, input.i64()?);
    }
    Ok(map)
}

fn put_ids(out: &mut Vec<u8>, ids: &Option<BTreeMap<u32, u32>>) -> io::Result<()> {
    match ids {
        None => out.push(0),
        Some(ids) => {
            out.push(1);
            put_count(out, ids.len())?;
            for (&slot, &id) in ids {
                out.extend_from_slice(&slot.to_le_bytes());
                out.extend_from_slice(&id.to_le_bytes());
            }
        }
    }
    Ok(())
}

fn ids_of(input: &mut Decoder<'_>) -> io::Result<Option<BTreeMap<u32, u32>>> {
    Ok(match input.u8()? {
        0 => None,
        1 => {
            let count = input.count(8)?;
            let mut ids = BTreeMap::new();
            for _ in 0..count {
                ids.insert(input.u32()?, input.u32()?);
            }
            Some(ids)
        }
        _ => return Err(damaged()),
    })
}

impl Supply {
    fn save(&self, out: &mut Vec<u8>) {
        put_hour(out, self.read);
        out.push(u8::from(self.known) | u8::from(self.inherited) << 1);
    }

    fn load(input: &mut Decoder<'_>) -> io::Result<Self> {
        let read = hour_of(input)?;
        let flags = input.u8()?;
        if flags > 3 {
            return Err(damaged());
        }
        Ok(Self {
            read,
            known: flags & 1 != 0,
            inherited: flags & 2 != 0,
        })
    }
}

impl Engine {
    /// Writes what the engine stands on that no later hour gives again: its reads, the rows
    /// they and the hours since read, and the countries. The next hour it steps after
    /// [`Self::load`] gives every reply whole.
    pub(crate) fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let supplies = &self.supplies;
        for supply in [
            supplies.pool,
            supplies.stock,
            supplies.designs,
            supplies.stock_designs,
        ] {
            supply.save(out);
        }
        put_hour(out, supplies.resumed);
        out.extend_from_slice(&supplies.cold.to_le_bytes());
        put_hour(out, self.last);
        out.extend(self.complete.map(u8::from));
        put_count(out, self.divisions.len())?;
        for division in self.divisions.values() {
            let facts = division.facts;
            out.extend_from_slice(&facts.id.to_le_bytes());
            out.extend_from_slice(&facts.roster_owner.to_le_bytes());
            out.extend_from_slice(&facts.logical_owner.to_le_bytes());
            out.extend_from_slice(&facts.expeditionary_owner.to_le_bytes());
            out.extend_from_slice(&facts.template.to_le_bytes());
            out.extend_from_slice(&facts.hit_points.to_le_bytes());
            out.push(facts.reveal_skip);
        }
        put_values(out, &self.maxima)?;
        put_values(out, &self.manpower)?;
        for rows in [&self.new, &self.daily, &self.pooled, &self.stock_rows] {
            put_rows(out, rows)?;
        }
        put_count(out, self.daily_since.len())?;
        for id in &self.daily_since {
            out.extend_from_slice(&id.to_le_bytes());
        }
        match &self.census {
            None => out.push(0),
            Some(ids) => {
                out.push(1);
                put_count(out, ids.len())?;
                for id in ids {
                    out.extend_from_slice(&id.to_le_bytes());
                }
            }
        }
        match &self.stock {
            None => out.push(0),
            Some(stock) => {
                out.push(1);
                match &stock.grouped {
                    None => out.push(0),
                    Some((slots, unlinked)) => {
                        out.push(1);
                        put_count(out, slots.len())?;
                        for (&slot, rows) in slots {
                            out.extend_from_slice(&slot.to_le_bytes());
                            put_count(out, rows.len())?;
                            for &(variant, amount) in rows {
                                out.extend_from_slice(&variant.to_le_bytes());
                                out.extend_from_slice(&amount.to_le_bytes());
                            }
                        }
                        out.extend_from_slice(&(*unlinked as u64).to_le_bytes());
                    }
                }
                put_ids(out, &stock.ids)?;
            }
        }
        let countries = &self.countries;
        out.push(u8::from(countries.complete));
        put_count(out, countries.links.len())?;
        for link in &countries.links {
            out.extend_from_slice(&link.id.to_le_bytes());
            out.extend_from_slice(&link.canonical_slot.to_le_bytes());
        }
        put_count(out, countries.holders.len())?;
        for (tag, holder) in &countries.holders {
            put_count(out, tag.len())?;
            out.extend_from_slice(tag.as_bytes());
            match holder {
                None => out.push(0),
                Some(holder) => {
                    out.push(1);
                    out.extend_from_slice(&holder.slot.to_le_bytes());
                    out.extend_from_slice(&holder.id.to_le_bytes());
                }
            }
        }
        put_ids(out, &countries.ids)?;
        put_count(out, self.reads.len())?;
        for (&hour, read) in &self.reads {
            read.saved(&self.history, hour).encode_checkpoint(out)?;
        }
        out.push(u8::from(self.bare));
        // A lead gives the templates' facts only where they changed.
        match &self.absent {
            None => out.push(0),
            Some(absent) => {
                out.push(1);
                let mut facts: Vec<(u64, &TemplateFacts)> = self
                    .facts
                    .iter()
                    .map(|(&handle, facts)| (handle, &**facts))
                    .collect();
                facts.sort_unstable_by_key(|&(handle, _)| handle);
                let bytes = dashboard_base::prepared::codec::encode(&(&**absent, facts))?;
                put_count(out, bytes.len())?;
                out.extend_from_slice(&bytes);
            }
        }
        Ok(())
    }

    /// The engine [`Self::save`] wrote, reading content `cat`.
    pub(crate) fn load(cat: Option<Arc<Catalogue>>, bytes: &[u8]) -> io::Result<Self> {
        let mut engine = Self::new(cat, None, None);
        let mut input = Decoder::new(bytes);
        let mut supplies = [Supply::FRESH; 4];
        for supply in &mut supplies {
            *supply = Supply::load(&mut input)?;
        }
        let [pool, stock, designs, stock_designs] = supplies;
        engine.supplies = Supplies {
            pool,
            stock,
            designs,
            stock_designs,
            resumed: hour_of(&mut input)?,
            cold: input.i32()?,
        };
        engine.last = hour_of(&mut input)?;
        for complete in &mut engine.complete {
            *complete = input.bool()?;
        }
        let count = input.count(37)?;
        for _ in 0..count {
            let facts = Division {
                id: input.u64()?,
                roster_owner: input.u32()?,
                logical_owner: input.u32()?,
                expeditionary_owner: input.u32()?,
                template: input.u64()?,
                hit_points: input.i64()?,
                reveal_skip: input.u8()?,
            };
            engine
                .divisions
                .insert(facts.id, DivisionState { facts, added: None });
        }
        engine.maxima = values_of(&mut input)?;
        engine.manpower = values_of(&mut input)?;
        engine.new = rows_of(&mut input)?;
        engine.daily = rows_of(&mut input)?;
        engine.pooled = rows_of(&mut input)?;
        engine.stock_rows = rows_of(&mut input)?;
        let count = input.count(8)?;
        for _ in 0..count {
            engine.daily_since.insert(input.u64()?);
        }
        engine.census = match input.u8()? {
            0 => None,
            1 => {
                let count = input.count(8)?;
                let mut ids = Vec::with_capacity(count);
                for _ in 0..count {
                    ids.push(input.u64()?);
                }
                Some(ids)
            }
            _ => return Err(damaged()),
        };
        engine.stock = match input.u8()? {
            0 => None,
            1 => {
                let grouped = match input.u8()? {
                    0 => None,
                    1 => {
                        let count = input.count(8)?;
                        let mut slots =
                            FastMap::with_capacity_and_hasher(count, Default::default());
                        for _ in 0..count {
                            let slot = input.u32()?;
                            let rows = input.count(16)?;
                            let mut held = Vec::with_capacity(rows);
                            for _ in 0..rows {
                                held.push((input.u64()?, input.i64()?));
                            }
                            slots.insert(slot, held);
                        }
                        let unlinked = usize::try_from(input.u64()?).map_err(|_| damaged())?;
                        Some((slots, unlinked))
                    }
                    _ => return Err(damaged()),
                };
                Some(StockRead {
                    grouped,
                    ids: ids_of(&mut input)?,
                })
            }
            _ => return Err(damaged()),
        };
        let complete = input.bool()?;
        let count = input.count(8)?;
        let mut links = Vec::with_capacity(count);
        for _ in 0..count {
            links.push(CountryLinkRow {
                id: input.u32()?,
                canonical_slot: input.u32()?,
            });
        }
        let count = input.count(5)?;
        let mut holders = Vec::with_capacity(count);
        for _ in 0..count {
            let length = input.count(1)?;
            let tag = std::str::from_utf8(input.take(length)?).map_err(|_| damaged())?;
            let holder = match input.u8()? {
                0 => None,
                1 => Some(Holder {
                    slot: input.u32()?,
                    id: input.u32()?,
                }),
                _ => return Err(damaged()),
            };
            holders.push((Arc::<str>::from(tag), holder));
        }
        engine.countries = Arc::new(Countries {
            complete,
            links,
            holders,
            ids: ids_of(&mut input)?,
        });
        let count = input.count(1)?;
        for _ in 0..count {
            let frame = DecodedFrame::decode_checkpoint(&mut input)?;
            let rows = frame
                .complete::<tables::VariantDefinitions>()?
                .iter()
                .cloned()
                .map(Arc::new)
                .collect();
            engine
                .history
                .read(&DefinitionRows::Whole(rows), frame.hour)?;
            let read = Read::from_saved(&frame, &mut engine.history);
            engine.reads.insert(frame.hour, read);
        }
        // The next designs read follows one this engine never saw.
        engine.history.based = false;
        engine.bare = input.bool()?;
        match input.u8()? {
            0 => {}
            1 => {
                let length = input.count(1)?;
                let (absent, facts): (Absent, Vec<(u64, TemplateFacts)>) =
                    dashboard_base::prepared::codec::decode(input.take(length)?)?;
                engine.absent = Some(Arc::new(absent));
                engine.facts = facts
                    .into_iter()
                    .map(|(handle, facts)| (handle, Arc::new(facts)))
                    .collect();
            }
            _ => return Err(damaged()),
        }
        input.finish()?;
        engine.open = false;
        Ok(engine)
    }
}

/// One value table's rows applied, its changes noted as numeric, or as structural where a
/// value appears or goes.
fn apply_values(rows: &Rows<(u64, i64)>, values: &mut FastMap<u64, i64>, touched: &mut Touched) {
    let mut note = |id: u64, before: Option<i64>, now: Option<i64>| match (before, now) {
        (Some(before), Some(now)) => {
            if before != now {
                touched.numeric.push(id);
            }
        }
        (None, None) => {}
        _ => {
            touched.structural.push(id);
        }
    };
    match rows {
        Rows::Same => {}
        Rows::Whole(rows) => {
            let now: FastMap<u64, i64> = rows.iter().copied().collect();
            for (&id, &before) in values.iter() {
                if !now.contains_key(&id) {
                    note(id, Some(before), None);
                }
            }
            for (&id, &value) in &now {
                note(id, values.get(&id).copied(), Some(value));
            }
            *values = now;
        }
        Rows::Changed(rows, gone) => {
            for &key in gone {
                let id = key as u64;
                if let Some(before) = values.remove(&id) {
                    note(id, Some(before), None);
                }
            }
            for &(id, value) in rows {
                note(id, values.insert(id, value), Some(value));
            }
        }
    }
}

/// A pool or stockpile table's rows applied to `held`, its rows by owner (the key past its
/// ordinal) in ordinal order; the owners whose rows changed go to `changed`, once each.
fn apply_rows(rows: &Rows<Entry>, held: &mut FastMap<u64, Arc<[Row]>>, changed: &mut Vec<u64>) {
    let owner = |key: u128| (key >> 32) as u64;
    match rows {
        Rows::Same => {}
        Rows::Whole(rows) => {
            let mut sorted: Vec<(u64, Row)> = rows
                .iter()
                .map(|entry| {
                    (
                        owner(entry.key),
                        (entry.key as u32, entry.variant, entry.amount),
                    )
                })
                .collect();
            sorted.sort_unstable_by_key(|&(owner, row)| (owner, row.0));
            let mut next = FastMap::with_capacity_and_hasher(held.len(), Default::default());
            let mut now = Vec::new();
            for group in sorted.chunk_by(|a, b| a.0 == b.0) {
                let owner = group[0].0;
                now.clear();
                now.extend(group.iter().map(|&(_, row)| row));
                match held.remove(&owner) {
                    Some(was) if *was == *now => {
                        next.insert(owner, was);
                    }
                    _ => {
                        changed.push(owner);
                        next.insert(owner, Arc::from(&now[..]));
                    }
                }
            }
            changed.extend(held.keys().copied());
            *held = next;
        }
        Rows::Changed(rows, gone) => {
            // Each owner's edits by ordinal, in the order they apply: deletions, then rows set.
            let mut edits: Vec<(u64, u32, Option<(u64, i64)>)> =
                Vec::with_capacity(gone.len() + rows.len());
            edits.extend(gone.iter().map(|&key| (owner(key), key as u32, None)));
            edits.extend(rows.iter().map(|entry| {
                let set = (entry.variant, entry.amount);
                (owner(entry.key), entry.key as u32, Some(set))
            }));
            edits.sort_by_key(|&(owner, ordinal, _)| (owner, ordinal));
            let mut next = Vec::new();
            for group in edits.chunk_by(|a, b| a.0 == b.0) {
                let owner = group[0].0;
                next.clear();
                let same = {
                    let was: &[Row] = held.get(&owner).map_or(&[], |rows| rows);
                    let mut at = 0;
                    for keyed in group.chunk_by(|a, b| a.1 == b.1) {
                        let ordinal = keyed[0].1;
                        while let Some(&row) = was.get(at).filter(|row| row.0 < ordinal) {
                            next.push(row);
                            at += 1;
                        }
                        if was.get(at).is_some_and(|row| row.0 == ordinal) {
                            at += 1;
                        }
                        if let Some((_, _, Some((variant, amount)))) = keyed.last() {
                            next.push((ordinal, *variant, *amount));
                        }
                    }
                    next.extend_from_slice(&was[at..]);
                    *was == *next
                };
                if same {
                    continue;
                }
                changed.push(owner);
                if next.is_empty() {
                    held.remove(&owner);
                } else {
                    held.insert(owner, Arc::from(&next[..]));
                }
            }
        }
    }
}
