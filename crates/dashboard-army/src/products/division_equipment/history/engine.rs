//! Every country's replies at each hour of a branch, kept from the hour's changed rows
//! ([`Input`]). Each placed division's part of its replies ([`Part`]: its pool by type and
//! design, its comparison with its template's need, its refusals) is kept until one of its
//! own inputs changes; each reply is the sum of its divisions' parts, kept as counts and sums
//! ([`Sums`]) a part is added to and taken from; and only the values a changed part, read or
//! template reaches are given again ([`Output`]). An engine started at a restart cannot know
//! the reads made before it: until its own reads stand ([`Supplies::own`]) it keeps its rows
//! and answers nothing, and the engine of the segment before answers those hours.

use super::super::{Placed, UNLINKED_DIVISION, find};
use super::{CADENCE, KeptReason, NamingOperands, kept_artwork, kept_naming};
use crate::products::{
    army_content::{self, Holder, TemplateArtwork},
    division_needs,
};
use dashboard_base::products::equipment::RawVariant;
use dashboard_text::{
    catalogue::Catalogue,
    metadata::link_slot,
    subunit_requirements::{Content, SourceIndex, Sources},
    template_facts::TemplateNaming,
};
use dashboard_world::Division;
use replay::columnar_frames::view_heap_bytes;
use replay_data::{
    codec::{Decoder, put_varint},
    rows::{CountryLinkRow, TableId, VariantDefinitionRow, tables},
};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

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
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::NewDivisionEquipment,
    TableId::DivisionEquipment,
    TableId::Stockpiles,
    TableId::VariantDefinitions,
];

/// The rows a table gives an hour.
pub(crate) enum Rows<T> {
    /// What the hour before held.
    Same,
    /// Every row the table holds.
    Whole(Vec<T>),
    /// Rows set, and keys deleted.
    Changed(Vec<T>, Vec<u128>),
}

impl<T> Rows<T> {
    fn heap(&self) -> usize {
        match self {
            Self::Same => 0,
            Self::Whole(rows) => rows.capacity() * size_of::<T>(),
            Self::Changed(rows, gone) => {
                rows.capacity() * size_of::<T>() + gone.capacity() * size_of::<u128>()
            }
        }
    }
}

/// A pool or stockpile row as the walk gives it: its key (owner and ordinal), design and
/// signed amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) key: u128,
    pub(crate) variant: u64,
    pub(crate) amount: i64,
}

/// What a lexical token change can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tokens {
    Same,
    /// Only new tokens: a spelling read before reads alike.
    Appended,
    Changed,
}

/// The countries an hour's replies read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

impl Countries {
    fn heap(&self) -> usize {
        self.links.capacity() * size_of::<CountryLinkRow>()
            + self
                .holders
                .iter()
                .map(|(tag, _)| size_of::<(Arc<str>, Option<Holder>)>() + 16 + tag.len())
                .sum::<usize>()
            + self.ids.as_ref().map_or(0, |ids| ids.len() * 32)
    }
}

/// One hour as an engine reads it.
pub(crate) struct Input {
    pub(crate) hour: i32,
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
    /// The countries' identity, links, tokens, local sub-units and templates, when one
    /// changed.
    pub(crate) frame: Option<DecodedFrame>,
    /// [`super::DESIGNS`] at an hour that reads them.
    pub(crate) designs: Option<DecodedFrame>,
    /// Designs whose definitions the hour changed; none when any may have.
    pub(crate) definitions: Option<Vec<u64>>,
}

impl Input {
    /// The bytes it owns, its frames' views too: a lead outlives the walk that read it.
    pub(crate) fn heap(&self) -> usize {
        self.divisions.heap()
            + self.maxima.heap()
            + self.manpower.heap()
            + self.new.heap()
            + self.daily.heap()
            + self.stock.heap()
            + self
                .countries
                .as_ref()
                .map_or(0, |countries| countries.heap())
            + self
                .templates
                .as_ref()
                .map_or(0, |keys| keys.capacity() * 8)
            + self
                .definitions
                .as_ref()
                .map_or(0, |keys| keys.capacity() * 8)
            + views(self.frame.iter().chain(&self.designs))
    }
}

/// An operand a published value or name reads, given once by the engine's id for it.
#[derive(Clone, Debug)]
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
/// of the memory. Handles are kept whole, which the emit reads back without decoding.
#[derive(Default)]
pub(crate) struct Changes {
    bytes: Vec<u8>,
    count: usize,
}

impl Changes {
    pub(crate) fn push(&mut self, change: Change) {
        let out = &mut self.bytes;
        let index = |found: Option<usize>| found.expect("a listed measure or part") as u8;
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
        out.push(field << 2 | value);
        put_varint(out, u128::from(change.tag));
        out.push(change.side);
        match change.field {
            Field::State | Field::Divisions | Field::Short | Field::Unknown | Field::Refused => {}
            Field::Type(kind, measure) => {
                put_varint(out, u128::from(kind));
                out.push(index(MEASURES.iter().position(|&held| held == measure)));
            }
            Field::Variant(kind, handle) | Field::Design(kind, handle) => {
                put_varint(out, u128::from(kind));
                out.extend_from_slice(&handle.to_le_bytes());
            }
            Field::Template(template, part) => {
                out.extend_from_slice(&template.to_le_bytes());
                out.push(index(PARTS.iter().position(|&held| held == part)));
            }
            Field::Note(note) => put_varint(out, u128::from(note)),
            Field::StockDesign(handle) => out.extend_from_slice(&handle.to_le_bytes()),
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
        let mut input = Encoded(&self.bytes);
        (0..self.count).map(move |_| input.change())
    }

    /// Lets go of the room it grew into past its bytes.
    fn seal(mut self) -> Self {
        self.bytes.shrink_to_fit();
        self
    }

    fn heap(&self) -> usize {
        self.bytes.capacity()
    }
}

/// The bytes of [`Changes`] not yet read back.
struct Encoded<'a>(&'a [u8]);

impl Encoded<'_> {
    /// The change [`Changes::push`] encoded next.
    fn change(&mut self) -> io::Result<Change> {
        let header = self.byte()?;
        let tag = self.narrow()?;
        let side = self.byte()?;
        let field = match header >> 2 {
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
            6 => Field::Variant(self.narrow()?, self.whole()?),
            7 => Field::Design(self.narrow()?, self.whole()?),
            8 => {
                let template = self.whole()?;
                let part = *PARTS.get(usize::from(self.byte()?)).ok_or_else(broken)?;
                Field::Template(template, part)
            }
            9 => Field::Note(self.narrow()?),
            10 => Field::StockDesign(self.whole()?),
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
        let (&byte, rest) = self.0.split_first().ok_or_else(broken)?;
        self.0 = rest;
        Ok(byte)
    }

    /// A handle, kept whole.
    #[inline]
    fn whole(&mut self) -> io::Result<u64> {
        let (bytes, rest) = self.0.split_first_chunk::<8>().ok_or_else(broken)?;
        self.0 = rest;
        Ok(u64::from_le_bytes(*bytes))
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

impl Output {
    /// The bytes it owns; its operands are the engine's too.
    pub(crate) fn heap(&self) -> usize {
        self.operands.capacity() * size_of::<(u32, Operand)>()
            + self.tags.capacity() * size_of::<(u32, Arc<str>)>()
            + self.tags.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            + self.listed.capacity() * 4
            + self.changes.heap()
    }
}

/// One supplying read, by the hour of its read: unknown before a cold start's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
struct Look {
    notes: Vec<(u32, usize)>,
    kind: Option<u32>,
    raw: Option<u32>,
}

/// A stockpile read, its rows grouped by the links of its hour.
struct StockRead {
    /// Each slot's designs and amounts, and how many rows no link places; none where the
    /// read's countries or links were not read whole.
    grouped: Option<(HashMap<u32, Vec<(u64, i64)>>, usize)>,
    ids: Option<BTreeMap<u32, u32>>,
}

/// What a reply reads of the national stockpile.
#[derive(Debug, Default, PartialEq, Eq)]
struct StockPart {
    /// Each classified kind's stock; none where it is withheld.
    totals: Option<BTreeMap<u32, i128>>,
    notes: Vec<(u32, u32)>,
    /// Each design looked up, with its naming operands when named.
    designs: Vec<(u64, Option<u32>)>,
}

/// A template's need, required manpower and naming operands at the hour.
struct Template {
    need: Result<Arc<[(u32, i64)]>, u32>,
    required: Option<i64>,
    naming: u32,
    artwork: u32,
    /// Its naming was sampled for this hour only: it is named again each hour.
    hourly: bool,
}

/// One type of a division's pool: its holding and each design's.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Holding {
    kind: u32,
    total: i128,
    variants: Vec<(u64, i128)>,
}

/// Each type's need, capped holding and shortfall.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Compared {
    types: Vec<(u32, i128, i128, i128)>,
    needed: i128,
    filled: i128,
    /// The kinds it is short of.
    short: Vec<u32>,
}

/// A division's part of its replies: its pool compared with its template's need, and its
/// refusals but those of its own numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Share {
    holdings: Vec<Holding>,
    compared: Option<Compared>,
    /// A known need's kinds, listed for a division whose comparison is unknown.
    listed: Vec<u32>,
    /// Every kind it lists a type for, once.
    kinds: Vec<u32>,
    notes: Vec<u32>,
    handles: Vec<u64>,
    required: Option<i64>,
}

/// A division's own numbers: hit points, and maximum hit points and manpower where read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Numbers {
    hp: i64,
    maximum: Option<i64>,
    manpower: Option<i64>,
}

/// What a division added to its replies.
struct Added {
    placed: Placed,
    share: Share,
    numbers: Numbers,
}

struct DivisionState {
    facts: Division,
    added: Option<Added>,
}

/// A reply's sum over its divisions.
#[derive(Default)]
struct Sums {
    divisions: u32,
    short: u32,
    unknown: u32,
    types: BTreeMap<u32, TypeSum>,
    templates: BTreeMap<u64, TemplateSum>,
    notes: BTreeMap<u32, u32>,
    handles: BTreeMap<u64, u32>,
}

#[derive(Default)]
struct TypeSum {
    /// Divisions listing the type.
    refs: u32,
    held: i128,
    variants: BTreeMap<u64, (u32, i128)>,
    /// Known divisions comparing the type, and their sums.
    known: u32,
    needed: i128,
    filled: i128,
    missing: i128,
    short: u32,
}

#[derive(Default)]
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
}

/// What an hour changed of a reply.
#[derive(Default)]
struct Dirty {
    counts: bool,
    /// Every type's unknown count.
    unknown: bool,
    types: BTreeSet<u32>,
    variants: BTreeSet<(u32, u64)>,
    templates: BTreeSet<u64>,
    notes: bool,
}

/// What an hour's rows changed.
#[derive(Default)]
struct Touched {
    /// Divisions whose placement, a number's presence, or existence changed.
    structural: BTreeSet<u64>,
    /// Divisions gone, with what they added.
    gone: Vec<(u64, Added)>,
    /// Divisions whose pool rows changed.
    pools: BTreeSet<u64>,
    /// Divisions whose hit points, maximum or manpower changed, presence alike.
    numeric: BTreeSet<u64>,
    countries: bool,
    /// A table the needs' sources or the templates' facts read became, or stopped being,
    /// whole.
    sources: bool,
    templates: bool,
}

/// The fixed refusals of a comparison and a stockpile part.
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

/// A read's definition of design `handle`.
fn definition(frame: &DecodedFrame, handle: u64) -> Option<&VariantDefinitionRow> {
    let rows = frame.complete::<tables::VariantDefinitions>().ok()?;
    let at = rows.binary_search_by_key(&handle, |row| row.id).ok()?;
    rows.get(at)
}

/// A read's spelling of `token`.
fn spelling(frame: &DecodedFrame, token: u32) -> Option<&[u8]> {
    let rows = frame.complete::<tables::LexicalTokens>().ok()?;
    let at = rows.binary_search_by_key(&token, |row| row.token).ok()?;
    rows.get(at).map(|row| row.literal.as_slice())
}

/// Whether two views hold the same rows or refusal.
fn same(a: &SharedView, b: &SharedView) -> bool {
    SharedView::ptr_eq(a, b) || **a == **b
}

/// Whether two designs reads look a design up alike wherever its definition and token rows
/// are alike: each reads every table a lookup needs, with the same countries' slots and tags,
/// links and appearance.
fn designs_alike(a: &DecodedFrame, b: &DecodedFrame) -> bool {
    let read = |frame: &DecodedFrame| {
        frame.complete::<tables::VariantDefinitions>().is_ok()
            && frame.complete::<tables::LexicalTokens>().is_ok()
    };
    let (Ok(countries), Ok(others)) = (
        a.complete::<tables::Countries>(),
        b.complete::<tables::Countries>(),
    ) else {
        return false;
    };
    let (Ok(links), Ok(other_links)) = (
        a.complete::<tables::CountryLinks>(),
        b.complete::<tables::CountryLinks>(),
    ) else {
        return false;
    };
    let appearance = match (
        a.complete::<tables::CountryAppearance>(),
        b.complete::<tables::CountryAppearance>(),
    ) {
        (Ok(a), Ok(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(a, b)| (a.country, &a.name_tag) == (b.country, &b.name_tag))
        }
        (Err(_), Err(_)) => true,
        _ => false,
    };
    read(a)
        && read(b)
        && appearance
        && links == other_links
        && countries.len() == others.len()
        && countries
            .iter()
            .zip(others)
            .all(|(a, b)| (a.canonical_slot, &a.tag) == (b.canonical_slot, &b.tag))
}

/// Needs whose hour's sources are read only at a template's first need, and then from the
/// links and local sub-units kept since their tables last changed.
struct LazyNeeds<'f> {
    content: Result<Arc<Content>, String>,
    frame: &'f DecodedFrame,
    index: Option<Arc<SourceIndex>>,
    sources: OnceCell<Result<Sources<'f>, String>>,
}

impl<'f> LazyNeeds<'f> {
    fn sources(&self) -> &Result<Sources<'f>, String> {
        self.sources.get_or_init(|| match &self.index {
            Some(index) => division_needs::reused_sources(self.frame, Arc::clone(index)),
            None => division_needs::sources(self.frame),
        })
    }

    /// A template's need, or its refusal as kept: refused at its composition read, after its
    /// content and sources were read, that read's refusal, which names the hour.
    fn template(&self, handle: u64) -> Result<Vec<(String, i64)>, KeptReason> {
        let frame = self.frame;
        let need = division_needs::template(&self.content, || self.sources(), frame, handle);
        need.map_err(|why| {
            if self.content.is_ok()
                && self.sources().is_ok()
                && let Err(error) = frame.complete::<tables::DivisionTemplateIcons>()
            {
                let kept = KeptReason::unreadable(
                    division_needs::COMPOSITION_UNAVAILABLE,
                    frame,
                    TableId::DivisionTemplateIcons,
                    &error,
                );
                if kept.at(frame.hour) == why {
                    return kept;
                }
            }
            KeptReason::literal(why)
        })
    }

    /// The sources it read, to keep for the hours after while their tables last.
    fn index(&self) -> Option<Arc<SourceIndex>> {
        match self.sources.get() {
            Some(Ok(sources)) => Some(Arc::clone(sources.index())),
            _ => None,
        }
    }
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

/// One branch's replies, kept from hour to hour.
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
    divisions: HashMap<u64, DivisionState>,
    maxima: HashMap<u64, i64>,
    manpower: HashMap<u64, i64>,
    new: HashMap<u64, Arc<[Row]>>,
    /// The daily pools as the table holds them, and as their last read did.
    daily: HashMap<u64, Arc<[Row]>>,
    pooled: HashMap<u64, Arc<[Row]>>,
    /// Divisions whose daily pool rows changed since the last read.
    daily_since: HashSet<u64>,
    /// The divisions the last pool read saw, sorted; none when Divisions was incomplete then.
    census: Option<Vec<u64>>,
    /// Stockpile rows by raw country.
    stock_rows: HashMap<u64, Arc<[Row]>>,
    stock: Option<StockRead>,
    countries: Arc<Countries>,
    /// The small tables as last given.
    frame: Option<DecodedFrame>,
    /// The links, local sub-units and tokens the needs resolve through, while they last.
    sources: Option<Arc<SourceIndex>>,
    /// The designs' tables at each read still looked up, and the designs looked up there.
    reads: BTreeMap<i32, DecodedFrame>,
    looks: HashMap<Option<i32>, HashMap<u64, Arc<Look>>>,
    /// Pairs of reads that look a design up alike wherever its rows are alike.
    alike: Vec<((i32, i32), bool)>,
    templates: HashMap<u64, Template>,
    /// Countries numbered, by index; each listed one's holder, by index.
    tags: Vec<Arc<str>>,
    numbers: HashMap<Arc<str>, u32>,
    listed: BTreeMap<u32, Option<Holder>>,
    slots: HashMap<u32, Vec<u32>>,
    fresh_tags: Vec<(u32, Arc<str>)>,
    sums: HashMap<(u32, u8), Sums>,
    unlinked: [u32; 2],
    by_template: HashMap<u64, HashSet<u64>>,
    by_handle: HashMap<u64, HashSet<u64>>,
    stock_parts: HashMap<u32, StockPart>,
    /// Each country's notes and stockpile designs as last given, by attribution.
    given: HashMap<(u32, u8), BTreeMap<u32, u32>>,
    stock_given: HashMap<(u32, u8), Vec<u64>>,
    dirty: HashMap<(u32, u8), Dirty>,
    stock_dirty: BTreeSet<u32>,
    notes_dirty: bool,
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
            divisions: HashMap::new(),
            maxima: HashMap::new(),
            manpower: HashMap::new(),
            new: HashMap::new(),
            daily: HashMap::new(),
            pooled: HashMap::new(),
            daily_since: HashSet::new(),
            census: None,
            stock_rows: HashMap::new(),
            stock: None,
            countries: Arc::default(),
            frame: None,
            sources: None,
            reads: BTreeMap::new(),
            looks: HashMap::new(),
            alike: Vec::new(),
            templates: HashMap::new(),
            tags: Vec::new(),
            numbers: HashMap::new(),
            listed: BTreeMap::new(),
            slots: HashMap::new(),
            fresh_tags: Vec::new(),
            sums: HashMap::new(),
            unlinked: [0; 2],
            by_template: HashMap::new(),
            by_handle: HashMap::new(),
            stock_parts: HashMap::new(),
            given: HashMap::new(),
            stock_given: HashMap::new(),
            dirty: HashMap::new(),
            stock_dirty: BTreeSet::new(),
            notes_dirty: false,
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

    /// One hour: its rows applied, its reads stepped, and its replies' changes; none while
    /// the reads it stands on are not its own.
    pub(crate) fn step(&mut self, input: &Input) -> io::Result<Option<Output>> {
        let _cpu = ::replay_frame::cpu::scope("division_equipment.engine", None);
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
                .clone()
                .ok_or_else(|| invalid("A designs read came without its tables"))?;
            self.reads.insert(hour, designs);
        }
        let census_before = self.census.is_some();
        let read_now = self.supplies.pool.read == Some(hour);
        if read_now {
            self.pooled = self.daily.clone();
            touched.pools.extend(self.daily_since.drain());
            self.census = self.complete[DIVISIONS].then(|| {
                let mut ids: Vec<u64> = self.divisions.keys().copied().collect();
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
        if let Some(frame) = &input.frame {
            if let Some(held) = &self.frame {
                let whole = |frame: &DecodedFrame, table: TableId| {
                    matches!(&*frame.tables[table.index()], TableView::Complete(_))
                };
                let flipped = |table| whole(held, table) != whole(frame, table);
                touched.sources =
                    flipped(TableId::LexicalTokens) || flipped(TableId::CountryLocalSubunits);
                touched.templates =
                    flipped(TableId::DivisionTemplates) || flipped(TableId::DivisionTemplateIcons);
            }
            self.frame = Some(frame.clone());
        }
        // The needs' sources are read from the links, local sub-units and tokens.
        if touched.countries || touched.sources || input.subunits || input.tokens != Tokens::Same {
            self.sources = None;
        }
        match &input.divisions {
            Rows::Same => {}
            Rows::Whole(rows) => {
                let ids: HashSet<u64> = rows.iter().map(|row| row.id).collect();
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
        let mut changed = HashSet::new();
        apply_rows(&input.new, &mut self.new, &mut changed);
        touched.pools.extend(changed.drain());
        apply_rows(&input.daily, &mut self.daily, &mut changed);
        self.daily_since.extend(changed.drain());
        apply_rows(&input.stock, &mut self.stock_rows, &mut changed);
        Ok(())
    }

    fn set(&mut self, facts: Division, touched: &mut Touched) {
        match self.divisions.get_mut(&facts.id) {
            Some(division) if division.facts == facts => {}
            Some(division) => {
                let before = std::mem::replace(&mut division.facts, facts);
                if !placed_alike(&before, &facts) {
                    touched.structural.insert(facts.id);
                } else if before.hit_points != facts.hit_points {
                    touched.numeric.insert(facts.id);
                }
            }
            None => {
                self.divisions
                    .insert(facts.id, DivisionState { facts, added: None });
                touched.structural.insert(facts.id);
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
            let mut slots: HashMap<u32, Vec<(u64, i64)>> = HashMap::new();
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
        let ids: Vec<u64> = self.divisions.keys().copied().collect();
        for &id in &ids {
            if let Some(division) = self.divisions.get_mut(&id) {
                division.added = None;
            }
        }
        let frame = self.frame()?;
        let needs = self.needs(&frame);
        for &id in &ids {
            self.place(id, &frame, &needs)?;
        }
        self.keep_sources(&needs);
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

    /// The small tables as last given.
    fn frame(&self) -> io::Result<DecodedFrame> {
        self.frame
            .clone()
            .ok_or_else(|| invalid("Division equipment's tables were never given"))
    }

    /// The needs of the hour's templates, their sources read once and kept while they last.
    fn needs<'f>(&self, frame: &'f DecodedFrame) -> LazyNeeds<'f> {
        LazyNeeds {
            content: self.content.clone(),
            frame,
            index: self.sources.clone(),
            sources: OnceCell::new(),
        }
    }

    fn keep_sources(&mut self, needs: &LazyNeeds<'_>) {
        if self.sources.is_none() {
            self.sources = needs.index();
        }
    }

    fn template(&mut self, needs: &LazyNeeds<'_>, frame: &DecodedFrame, handle: u64) -> Template {
        let need = match needs.template(handle) {
            Ok(need) => Ok(need
                .into_iter()
                .map(|(key, amount)| (self.kind(Some(Arc::from(key))), amount))
                .collect()),
            Err(why) => Err(self.note(why)),
        };
        let required = find(
            frame.complete::<tables::DivisionTemplateIcons>().ok(),
            handle,
            |row| row.template,
        )
        .and_then(|icon| {
            icon.settings
                .require("template settings")
                .ok()?
                .as_ref()
                .map(|settings| i64::from(settings.total_manpower))
        });
        let naming = self.naming(kept_naming(frame, handle));
        let artwork = self.artwork(kept_artwork(frame, handle));
        Template {
            need,
            required,
            naming,
            artwork,
            hourly: army_content::template_naming_hourly(frame, handle),
        }
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
        let frame = self.frame()?;
        let needs = self.needs(&frame);
        // Appended tokens spell only what was unspelled: only refused needs can change. Local
        // sub-units and tokens reach the needs alone.
        let asked: Vec<u64> = match (&input.templates, input.tokens) {
            _ if input.subunits || touched.sources || touched.templates => {
                self.templates.keys().copied().collect()
            }
            (_, Tokens::Changed) | (None, _) => self.templates.keys().copied().collect(),
            (Some(changed), tokens) => {
                let mut asked: BTreeSet<u64> = changed
                    .iter()
                    .copied()
                    .filter(|handle| self.templates.contains_key(handle))
                    .collect();
                asked.extend(
                    self.templates
                        .iter()
                        .filter(|(_, template)| {
                            template.hourly
                                || (tokens == Tokens::Appended && template.need.is_err())
                        })
                        .map(|(handle, _)| *handle),
                );
                asked.into_iter().collect()
            }
        };
        let mut relabelled = BTreeSet::new();
        for handle in asked {
            let now = self.template(&needs, &frame, handle);
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
            self.templates.insert(handle, now);
        }
        for (id, added) in std::mem::take(&mut touched.gone) {
            self.take(id, &added);
        }
        let mut refresh: BTreeSet<u64> = touched.structural;
        refresh.extend(touched.pools);
        if every {
            refresh.extend(self.divisions.keys().copied());
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
        for &id in &refresh {
            self.place(id, &frame, &needs)?;
        }
        self.keep_sources(&needs);
        for id in touched.numeric {
            if !refresh.contains(&id) {
                self.renumber(id, &frame, &needs)?;
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
            let kinds: Vec<u32> = added
                .share
                .holdings
                .iter()
                .filter(|holding| holding.variants.iter().any(|&(held, _)| held == handle))
                .map(|holding| holding.kind)
                .collect();
            for (side, slot) in added.placed.slots.into_iter().enumerate() {
                let Some(slot) = slot else {
                    continue;
                };
                let dirty = self.dirty.entry((slot, side as u8)).or_default();
                dirty.notes = true;
                dirty
                    .variants
                    .extend(kinds.iter().map(|&kind| (kind, handle)));
            }
        }
    }

    /// Takes what division `id` added out of its replies and puts its part in again.
    fn place(&mut self, id: u64, frame: &DecodedFrame, needs: &LazyNeeds<'_>) -> io::Result<()> {
        let Some(state) = self.divisions.get_mut(&id) else {
            return Ok(());
        };
        let facts = state.facts;
        if let Some(added) = state.added.take() {
            self.take(id, &added);
        }
        let Some(placed) = super::super::place(&facts.row(), &self.countries.links) else {
            return Ok(());
        };
        if !self.templates.contains_key(&placed.template) {
            let template = self.template(needs, frame, placed.template);
            self.templates.insert(placed.template, template);
        }
        let share = self.share(id, placed);
        let numbers = self.numbers_of(&facts);
        let added = Added {
            placed,
            share,
            numbers,
        };
        self.put(id, &added);
        if let Some(state) = self.divisions.get_mut(&id) {
            state.added = Some(added);
        }
        Ok(())
    }

    /// A division whose numbers changed alone: their differences into its template's sums,
    /// or its part again where one appeared or went.
    fn renumber(&mut self, id: u64, frame: &DecodedFrame, needs: &LazyNeeds<'_>) -> io::Result<()> {
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
            return self.place(id, frame, needs);
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

    /// The division's pool as its replies read it: design and amount by row, or none when it
    /// was not read.
    fn pool(&self, id: u64) -> Option<Vec<(u64, i64)>> {
        let designs = |rows: &[Row]| rows.iter().map(|row| (row.1, row.2)).collect();
        if self.complete[NEW]
            && let Some(rows) = self.new.get(&id).filter(|rows| !rows.is_empty())
        {
            return Some(designs(rows));
        }
        let daily = self.supplies.pool.read.is_some();
        let existed = self
            .census
            .as_ref()
            .filter(|_| daily)
            .map(|ids| ids.binary_search(&id).is_ok());
        if daily {
            let held = self.pooled.get(&id).map_or(&[][..], |rows| &rows[..]);
            if !held.is_empty() || existed == Some(true) {
                return Some(designs(held));
            }
        }
        if existed == Some(false) && daily && self.complete[NEW] {
            return Some(Vec::new());
        }
        None
    }

    /// A placed division's part: its pool against its template's need.
    fn share(&mut self, id: u64, placed: Placed) -> Share {
        let pool = self.pool(id);
        let read = self.supplies.designs.read;
        if let Some(pool) = &pool {
            let handles: Vec<u64> = pool.iter().map(|row| row.0).collect();
            self.look_up(read, read, None, &handles);
        }
        let fixed = &self.fixed;
        let template = &self.templates[&placed.template];
        let mut notes = Vec::new();
        if template.required.is_none() {
            notes.push(fixed.required);
        }
        if let Err(note) = &template.need {
            notes.push(*note);
        }
        if pool.is_none() {
            notes.push(fixed.pool);
        }
        let mut holdings: BTreeMap<u32, Holding> = BTreeMap::new();
        let mut handles = BTreeSet::new();
        let mut unclassified = false;
        for &(handle, amount) in pool.iter().flatten() {
            handles.insert(handle);
            let kind = match self.look(read, handle).and_then(|look| look.kind) {
                Some(kind) => kind,
                None => {
                    notes.push(fixed.unclassified);
                    unclassified = true;
                    self.unclassified
                }
            };
            let holding = holdings.entry(kind).or_insert_with(|| Holding {
                kind,
                total: 0,
                variants: Vec::new(),
            });
            holding.total += i128::from(amount);
            match holding
                .variants
                .binary_search_by_key(&handle, |&(held, _)| held)
            {
                Ok(at) => holding.variants[at].1 += i128::from(amount),
                Err(at) => holding.variants.insert(at, (handle, i128::from(amount))),
            }
        }
        let compared = match &template.need {
            Ok(need) if pool.is_some() && !unclassified => {
                let mut keys: BTreeSet<u32> = need.iter().map(|&(kind, _)| kind).collect();
                keys.extend(holdings.keys().copied());
                let mut compared = Compared {
                    types: Vec::with_capacity(keys.len()),
                    needed: 0,
                    filled: 0,
                    short: Vec::new(),
                };
                for kind in keys {
                    let amount = need
                        .iter()
                        .find(|&&(need, _)| need == kind)
                        .map_or(0, |&(_, amount)| i128::from(amount));
                    let in_pool = holdings.get(&kind).map_or(0, |holding| holding.total);
                    let shortfall = (amount - in_pool).max(0);
                    let filled = in_pool.clamp(0, amount.max(0));
                    compared.needed += amount;
                    compared.filled += filled;
                    if shortfall > 0 {
                        compared.short.push(kind);
                    }
                    compared.types.push((kind, amount, filled, shortfall));
                }
                Some(compared)
            }
            _ => None,
        };
        let listed: Vec<u32> = match (&compared, &template.need) {
            (None, Ok(need)) => need.iter().map(|&(kind, _)| kind).collect(),
            _ => Vec::new(),
        };
        let mut kinds: BTreeSet<u32> = holdings.keys().copied().collect();
        kinds.extend(
            compared
                .iter()
                .flat_map(|compared| compared.types.iter().map(|types| types.0)),
        );
        kinds.extend(listed.iter().copied());
        notes.sort_unstable();
        Share {
            holdings: holdings.into_values().collect(),
            compared,
            listed,
            kinds: kinds.into_iter().collect(),
            notes,
            handles: handles.into_iter().collect(),
            required: template.required,
        }
    }

    /// Puts division `id`'s part into its replies.
    fn put(&mut self, id: u64, added: &Added) {
        self.change(id, added, 1);
        let template = added.placed.template;
        self.by_template.entry(template).or_default().insert(id);
        for &handle in &added.share.handles {
            self.by_handle.entry(handle).or_default().insert(id);
        }
    }

    /// Takes what a division added out of its replies; `id` names it while it is kept.
    fn take(&mut self, id: u64, added: &Added) {
        self.change(id, added, -1);
        let template = added.placed.template;
        if let Some(users) = self.by_template.get_mut(&template) {
            users.remove(&id);
            if users.is_empty() {
                self.by_template.remove(&template);
            }
        }
        for handle in &added.share.handles {
            if let Some(users) = self.by_handle.get_mut(handle) {
                users.remove(&id);
                if users.is_empty() {
                    self.by_handle.remove(handle);
                }
            }
        }
    }

    /// Adds (`sign` 1) or takes (-1) what a division adds to each reply it is listed in.
    fn change(&mut self, id: u64, added: &Added, sign: i128) {
        let _ = id;
        let Added {
            placed,
            share,
            numbers,
        } = added;
        let fixed = (self.fixed.manpower, self.fixed.maximum);
        for (side, slot) in placed.slots.into_iter().enumerate() {
            let Some(slot) = slot else {
                if sign > 0 {
                    self.unlinked[side] += 1;
                } else {
                    self.unlinked[side] -= 1;
                }
                self.notes_dirty = true;
                continue;
            };
            let sums = self.sums.entry((slot, side as u8)).or_default();
            let dirty = self.dirty.entry((slot, side as u8)).or_default();
            dirty.counts = true;
            count(&mut sums.divisions, sign);
            match &share.compared {
                Some(compared) if !compared.short.is_empty() => count(&mut sums.short, sign),
                Some(_) => {}
                None => {
                    count(&mut sums.unknown, sign);
                    dirty.unknown = true;
                }
            }
            for &kind in &share.kinds {
                count(&mut sums.types.entry(kind).or_default().refs, sign);
                dirty.types.insert(kind);
            }
            for holding in &share.holdings {
                let total = sums.types.entry(holding.kind).or_default();
                add(&mut total.held, holding.total, sign);
                for &(handle, amount) in &holding.variants {
                    let variant = total.variants.entry(handle).or_default();
                    count(&mut variant.0, sign);
                    add(&mut variant.1, amount, sign);
                    if variant.0 == 0 {
                        total.variants.remove(&handle);
                    }
                    dirty.variants.insert((holding.kind, handle));
                }
            }
            if let Some(compared) = &share.compared {
                for &(kind, need, filled, shortfall) in &compared.types {
                    let total = sums.types.entry(kind).or_default();
                    count(&mut total.known, sign);
                    add(&mut total.needed, need, sign);
                    add(&mut total.filled, filled, sign);
                    add(&mut total.missing, shortfall, sign);
                    if shortfall > 0 {
                        count(&mut total.short, sign);
                    }
                }
            }
            for &kind in &share.kinds {
                if sums.types.get(&kind).is_some_and(|total| total.refs == 0) {
                    sums.types.remove(&kind);
                }
            }
            let template = sums.templates.entry(placed.template).or_default();
            count(&mut template.divisions, sign);
            if share.compared.is_none() {
                count(&mut template.unknown, sign);
            }
            for ((missing, sum), value) in [
                (&mut template.manpower, numbers.manpower),
                (&mut template.required, share.required),
                (&mut template.maximum, numbers.maximum),
            ] {
                match value {
                    Some(value) => add(sum, i128::from(value), sign),
                    None => count(missing, sign),
                }
            }
            add(&mut template.hp, i128::from(numbers.hp), sign);
            if let Some(compared) = &share.compared {
                count(&mut template.known, sign);
                add(&mut template.needed, compared.needed, sign);
                add(&mut template.held, compared.filled, sign);
                for &kind in &compared.short {
                    counted(&mut template.short, kind, sign);
                }
            }
            if template.divisions == 0 {
                sums.templates.remove(&placed.template);
            }
            dirty.templates.insert(placed.template);
            let own = [
                (numbers.manpower.is_none(), fixed.0),
                (numbers.maximum.is_none(), fixed.1),
            ];
            for note in share.notes.iter().copied().chain(
                own.into_iter()
                    .filter(|(lacks, _)| *lacks)
                    .map(|(_, note)| note),
            ) {
                counted(&mut sums.notes, note, sign);
                dirty.notes = true;
            }
            for &handle in &share.handles {
                let before = sums.handles.contains_key(&handle);
                counted(&mut sums.handles, handle, sign);
                if before != sums.handles.contains_key(&handle) {
                    dirty.notes = true;
                }
            }
            if sums.divisions == 0 {
                self.sums.remove(&(slot, side as u8));
            }
        }
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
        let alike = match (definition(was, handle), definition(now, handle)) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                (
                    a.equipment_token,
                    a.creator_link,
                    a.origin_link,
                    a.append_variant_name,
                    &a.display_name,
                    &a.variant_name,
                ) == (
                    b.equipment_token,
                    b.creator_link,
                    b.origin_link,
                    b.append_variant_name,
                    &b.display_name,
                    &b.variant_name,
                ) && (SharedView::ptr_eq(
                    &was.tables[TableId::LexicalTokens.index()],
                    &now.tables[TableId::LexicalTokens.index()],
                ) || spelling(was, b.equipment_token) == spelling(now, b.equipment_token))
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
                let tokens =
                    |frame: &DecodedFrame| frame.tables[TableId::LexicalTokens.index()].clone();
                let same_tokens = match (self.reads.get(&other), self.reads.get(&hour)) {
                    (Some(was), Some(now)) => same(&tokens(was), &tokens(now)),
                    _ => false,
                };
                (same_tokens && self.alike(other, hour)).then(|| {
                    let changed: HashSet<u64> = definitions.iter().copied().collect();
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
                let frame = read.and_then(|hour| self.reads.get(&hour));
                let designs = army_content::Designs::new(frame, self.cat.as_ref());
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
        let refused = |note: u32| StockPart {
            totals: None,
            notes: vec![(note, 1)],
            designs: Vec::new(),
        };
        let Some(stock) = &self.stock else {
            return refused(self.fixed.stock_unread);
        };
        let same = match &stock.ids {
            None => Err("its country identity was not recorded"),
            Some(ids) => match ids.get(&holder.slot) {
                None => Err("its country identity was not recorded"),
                Some(&id) if id == holder.id => Ok(()),
                Some(_) => Err("it belongs to another country identity"),
            },
        };
        let grouped = stock.grouped.as_ref().map(|(slots, unlinked)| {
            (
                slots.get(&holder.slot).cloned().unwrap_or_default(),
                *unlinked,
            )
        });
        if let Err(why) = same {
            let note = format!("National equipment stockpile is left out: {why} at its read");
            return refused(self.literal(&note));
        }
        let Some((rows, unlinked)) = grouped else {
            return refused(self.fixed.stock_links);
        };
        let read = self.supplies.stock_designs.read;
        let handles: Vec<u64> = rows.iter().map(|row| row.0).collect();
        self.look_up(read, read, None, &handles);
        let mut notes: BTreeMap<u32, u32> = BTreeMap::new();
        let mut complete = true;
        if unlinked > 0 {
            *notes.entry(self.fixed.stock_unlinked).or_default() += unlinked as u32;
            complete = false;
        }
        let mut totals: BTreeMap<u32, i128> = BTreeMap::new();
        let mut looked = BTreeMap::new();
        for (handle, amount) in rows {
            let look = self.look(read, handle).cloned();
            if !looked.contains_key(&handle) {
                for &(note, times) in look.iter().flat_map(|look| &look.notes) {
                    *notes.entry(note).or_default() += times as u32;
                }
                looked.insert(handle, look.as_ref().and_then(|look| look.raw));
            }
            match look.and_then(|look| look.kind) {
                Some(kind) => *totals.entry(kind).or_default() += i128::from(amount),
                None => {
                    *notes.entry(self.fixed.stock_unclassified).or_default() += 1;
                    complete = false;
                }
            }
        }
        StockPart {
            totals: complete.then_some(totals),
            notes: notes.into_iter().collect(),
            designs: looked.into_iter().collect(),
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
            for &tag in self.slots.get(&slot).into_iter().flatten() {
                let into = touched.entry((tag, side)).or_default();
                into.counts |= reply.counts;
                into.unknown |= reply.unknown;
                into.types.extend(reply.types.iter().copied());
                into.variants.extend(reply.variants.iter().copied());
                into.templates.extend(reply.templates.iter().copied());
                into.notes |= reply.notes;
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
                stock_dirty.contains(&tag),
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
            self.reply((tag, side, holder.slot), &dirty, true, changes);
        }
    }

    /// The dirty values of country `tag`'s reply for `side`, its holder at `slot`; every
    /// type's stock and the stockpile's designs too when `stock`.
    fn reply(
        &mut self,
        (tag, side, slot): (u32, u8, u32),
        dirty: &Dirty,
        stock: bool,
        changes: &mut Changes,
    ) {
        let read = self.supplies.designs.read;
        let empty = Sums::default();
        let sums = self.sums.get(&(slot, side)).unwrap_or(&empty);
        let part = self.stock_parts.get(&tag);
        let mut put = |field: Field, value: Option<i128>| {
            changes.push(Change {
                tag,
                side,
                field,
                value: value.map(Value::Number),
            });
        };
        if dirty.counts {
            put(Field::Divisions, Some(i128::from(sums.divisions)));
            put(Field::Short, Some(i128::from(sums.short)));
            put(Field::Unknown, Some(i128::from(sums.unknown)));
        }
        let mut types: BTreeSet<u32> = dirty.types.clone();
        if dirty.unknown || stock {
            types.extend(sums.types.keys().copied());
        }
        let unclassified = self.unclassified;
        for kind in types {
            let total = sums.types.get(&kind);
            let known =
                |value: fn(&TypeSum) -> i128| total.filter(|total| total.known > 0).map(value);
            let stocked = total.and_then(|_| {
                let totals = part?.totals.as_ref()?;
                (kind != unclassified).then(|| totals.get(&kind).copied().unwrap_or(0))
            });
            for (measure, value) in [
                (Measure::Held, total.map(|total| total.held)),
                (Measure::Needed, known(|total| total.needed)),
                (Measure::Filled, known(|total| total.filled)),
                (Measure::Missing, known(|total| total.missing)),
                (Measure::Stock, stocked),
                (Measure::Short, total.map(|total| i128::from(total.short))),
                (Measure::Unknown, total.map(|_| i128::from(sums.unknown))),
            ] {
                put(Field::Type(kind, measure), value);
            }
            if dirty.types.contains(&kind)
                && let Some(total) = total
            {
                for (&handle, &(_, amount)) in &total.variants {
                    put(Field::Variant(kind, handle), Some(amount));
                }
            }
        }
        let looks = self.looks.get(&read);
        let design = |handle: u64| {
            read?;
            looks?.get(&handle)?.raw
        };
        for &(kind, handle) in &dirty.variants {
            let amount = sums
                .types
                .get(&kind)
                .and_then(|total| total.variants.get(&handle))
                .map(|&(_, amount)| amount);
            changes.push(Change {
                tag,
                side,
                field: Field::Variant(kind, handle),
                value: amount.map(Value::Number),
            });
            changes.push(Change {
                tag,
                side,
                field: Field::Design(kind, handle),
                value: amount.and(design(handle)).map(Value::Operand),
            });
        }
        // A type given again gives its designs again.
        for &kind in &dirty.types {
            if let Some(total) = sums.types.get(&kind) {
                for &handle in total.variants.keys() {
                    changes.push(Change {
                        tag,
                        side,
                        field: Field::Design(kind, handle),
                        value: design(handle).map(Value::Operand),
                    });
                }
            }
        }
        for &handle in &dirty.templates {
            let total = sums.templates.get(&handle);
            let present = |value: (u32, i128)| (value.0 == 0).then_some(value.1);
            let template = self.templates.get(&handle);
            let mut put = |part: Part, value: Option<Value>| {
                changes.push(Change {
                    tag,
                    side,
                    field: Field::Template(handle, part),
                    value: total.and(value),
                });
            };
            let number = |value: Option<i128>| value.map(Value::Number);
            put(
                Part::Divisions,
                number(total.map(|total| i128::from(total.divisions))),
            );
            put(
                Part::Unknown,
                number(total.map(|total| i128::from(total.unknown))),
            );
            put(
                Part::TypesShort,
                number(total.map(|total| total.short.len() as i128)),
            );
            put(
                Part::Manpower,
                number(total.and_then(|total| present(total.manpower))),
            );
            put(
                Part::Required,
                number(total.and_then(|total| present(total.required))),
            );
            put(Part::Hp, number(total.map(|total| total.hp)));
            put(
                Part::HpMax,
                number(total.and_then(|total| present(total.maximum))),
            );
            put(
                Part::Needed,
                number(
                    total
                        .filter(|total| total.known > 0)
                        .map(|total| total.needed),
                ),
            );
            put(
                Part::Held,
                number(
                    total
                        .filter(|total| total.known > 0)
                        .map(|total| total.held),
                ),
            );
            put(
                Part::Naming,
                template.map(|template| Value::Operand(template.naming)),
            );
            put(
                Part::Artwork,
                template.map(|template| Value::Operand(template.artwork)),
            );
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
            let designs: BTreeMap<u64, u32> = part
                .into_iter()
                .flat_map(|part| &part.designs)
                .filter_map(|&(handle, raw)| Some((handle, read.and(raw)?)))
                .collect();
            for (&handle, &raw) in &designs {
                changes.push(Change {
                    tag,
                    side,
                    field: Field::StockDesign(handle),
                    value: Some(Value::Operand(raw)),
                });
            }
            // A design the stockpile no longer holds ends; the emit ends what it never held.
            let ended: Vec<u64> = self
                .stock_given
                .remove(&(tag, side))
                .unwrap_or_default()
                .into_iter()
                .filter(|handle| !designs.contains_key(handle))
                .collect();
            for handle in ended {
                changes.push(Change {
                    tag,
                    side,
                    field: Field::StockDesign(handle),
                    value: None,
                });
            }
            self.stock_given
                .insert((tag, side), designs.into_keys().collect());
        }
    }

    /// The bytes it holds on the heap, its frames' views too: handed on, it outlives the walk.
    pub(crate) fn heap(&self) -> usize {
        fn hashed<K, V>(map: &HashMap<K, V>) -> usize {
            map.capacity() * (size_of::<(K, V)>() + 1)
        }
        fn ordered<K, V>(map: &BTreeMap<K, V>) -> usize {
            map.len() * (size_of::<(K, V)>() + 8)
        }
        let rows = |map: &HashMap<u64, Arc<[Row]>>| {
            hashed(map)
                + map
                    .values()
                    .map(|rows| 16 + rows.len() * size_of::<Row>())
                    .sum::<usize>()
        };
        let share = |share: &Share| {
            share
                .holdings
                .iter()
                .map(|holding| size_of::<Holding>() + holding.variants.capacity() * 24)
                .sum::<usize>()
                + share.compared.as_ref().map_or(0, |compared| {
                    compared.types.capacity() * 52 + compared.short.capacity() * 4
                })
                + (share.listed.capacity() + share.kinds.capacity() + share.notes.capacity()) * 4
                + share.handles.capacity() * 8
        };
        let mut bytes = hashed(&self.divisions)
            + self
                .divisions
                .values()
                .filter_map(|division| division.added.as_ref())
                .map(|added| size_of::<Added>() + share(&added.share))
                .sum::<usize>()
            + hashed(&self.maxima)
            + hashed(&self.manpower)
            + rows(&self.new)
            + rows(&self.daily)
            + rows(&self.pooled)
            + rows(&self.stock_rows)
            + self.daily_since.capacity() * 9
            + self.census.as_ref().map_or(0, |ids| ids.capacity() * 8)
            + self.countries.heap();
        bytes += self
            .sums
            .values()
            .map(|sums| {
                size_of::<Sums>()
                    + ordered(&sums.types)
                    + sums
                        .types
                        .values()
                        .map(|total| ordered(&total.variants))
                        .sum::<usize>()
                    + ordered(&sums.templates)
                    + sums
                        .templates
                        .values()
                        .map(|total| ordered(&total.short))
                        .sum::<usize>()
                    + ordered(&sums.notes)
                    + ordered(&sums.handles)
            })
            .sum::<usize>();
        bytes += hashed(&self.by_template)
            + hashed(&self.by_handle)
            + self
                .by_template
                .values()
                .chain(self.by_handle.values())
                .map(|ids| ids.capacity() * 9)
                .sum::<usize>();
        bytes += hashed(&self.templates)
            + self
                .templates
                .values()
                .map(|template| template.need.as_ref().map_or(0, |need| need.len() * 16))
                .sum::<usize>();
        bytes += self
            .looks
            .values()
            .map(|looks| hashed(looks) + looks.len() * (size_of::<Look>() + 16))
            .sum::<usize>();
        bytes += self
            .stock_parts
            .values()
            .map(|part| {
                part.totals.as_ref().map_or(0, ordered)
                    + part.notes.capacity() * 8
                    + part.designs.capacity() * 16
            })
            .sum::<usize>()
            + self.stock.as_ref().map_or(0, |stock| {
                stock.grouped.as_ref().map_or(0, |(slots, _)| {
                    hashed(slots)
                        + slots
                            .values()
                            .map(|rows| rows.capacity() * 16)
                            .sum::<usize>()
                })
            });
        bytes += self.given.values().map(ordered).sum::<usize>()
            + self
                .stock_given
                .values()
                .map(|handles| handles.capacity() * 8)
                .sum::<usize>();
        bytes += self.tags.iter().map(|tag| 32 + tag.len()).sum::<usize>()
            + hashed(&self.numbers)
            + ordered(&self.listed)
            + hashed(&self.slots);
        // Operands, by their texts.
        bytes += self
            .notes
            .keys()
            .map(|note| {
                64 + note
                    .text
                    .iter()
                    .chain(&note.tables)
                    .map(String::len)
                    .sum::<usize>()
            })
            .sum::<usize>()
            + (self.namings.len() + self.artworks.len()) * 128
            + self
                .designs
                .keys()
                .map(|design| 256 + design.0.token.len() + design.0.display_name.len())
                .sum::<usize>()
            + hashed(&self.kinds);
        bytes + views(self.reads.values().chain(&self.frame))
    }
}

/// The bytes the views of `frames` hold, a view several of them share counted once.
fn views<'f>(frames: impl Iterator<Item = &'f DecodedFrame>) -> usize {
    let mut seen: HashSet<*const TableView> = HashSet::new();
    frames
        .flat_map(|frame| frame.tables.iter())
        .filter(|view| seen.insert(SharedView::as_ptr(view)))
        .map(|view| view_heap_bytes(view))
        .sum()
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

fn put_rows(out: &mut Vec<u8>, map: &HashMap<u64, Arc<[Row]>>) -> io::Result<()> {
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

fn rows_of(input: &mut Decoder<'_>) -> io::Result<HashMap<u64, Arc<[Row]>>> {
    let owners = input.count(12)?;
    let mut map = HashMap::with_capacity(owners);
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

fn put_values(out: &mut Vec<u8>, map: &HashMap<u64, i64>) -> io::Result<()> {
    put_count(out, map.len())?;
    for (&id, &value) in map {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

fn values_of(input: &mut Decoder<'_>) -> io::Result<HashMap<u64, i64>> {
    let count = input.count(16)?;
    let mut map = HashMap::with_capacity(count);
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
        for frame in self.reads.values() {
            frame.encode_checkpoint(out)?;
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
                        let mut slots = HashMap::with_capacity(count);
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
            engine.reads.insert(frame.hour, frame);
        }
        input.finish()?;
        engine.open = false;
        Ok(engine)
    }
}

/// One value table's rows applied, its changes noted as numeric, or as structural where a
/// value appears or goes.
fn apply_values(rows: &Rows<(u64, i64)>, values: &mut HashMap<u64, i64>, touched: &mut Touched) {
    let mut note = |id: u64, before: Option<i64>, now: Option<i64>| match (before, now) {
        (Some(before), Some(now)) => {
            if before != now {
                touched.numeric.insert(id);
            }
        }
        (None, None) => {}
        _ => {
            touched.structural.insert(id);
        }
    };
    match rows {
        Rows::Same => {}
        Rows::Whole(rows) => {
            let now: HashMap<u64, i64> = rows.iter().copied().collect();
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
/// ordinal) in ordinal order; the owners whose rows changed go to `changed`.
fn apply_rows(rows: &Rows<Entry>, held: &mut HashMap<u64, Arc<[Row]>>, changed: &mut HashSet<u64>) {
    let row = |entry: &Entry| (entry.key as u32, entry.variant, entry.amount);
    match rows {
        Rows::Same => {}
        Rows::Whole(rows) => {
            let mut now: HashMap<u64, Vec<Row>> = HashMap::new();
            for entry in rows {
                now.entry((entry.key >> 32) as u64)
                    .or_default()
                    .push(row(entry));
            }
            let mut next = HashMap::with_capacity(now.len());
            for (owner, mut rows) in now {
                rows.sort_unstable_by_key(|row| row.0);
                match held.remove(&owner) {
                    Some(was) if *was == *rows => {
                        next.insert(owner, was);
                    }
                    _ => {
                        changed.insert(owner);
                        next.insert(owner, Arc::from(rows));
                    }
                }
            }
            changed.extend(held.keys().copied());
            *held = next;
        }
        Rows::Changed(rows, gone) => {
            let mut edited: HashMap<u64, Vec<Row>> = HashMap::new();
            let take = |owner: u64, held: &mut HashMap<u64, Arc<[Row]>>| -> Vec<Row> {
                held.get(&owner)
                    .map(|rows| rows.to_vec())
                    .unwrap_or_default()
            };
            for &key in gone {
                let owner = (key >> 32) as u64;
                let rows = match edited.get_mut(&owner) {
                    Some(rows) => rows,
                    None => {
                        let rows = take(owner, held);
                        edited.entry(owner).or_insert(rows)
                    }
                };
                if let Ok(at) = rows.binary_search_by_key(&(key as u32), |row| row.0) {
                    rows.remove(at);
                }
            }
            for entry in rows {
                let owner = (entry.key >> 32) as u64;
                let row = row(entry);
                let rows = match edited.get_mut(&owner) {
                    Some(rows) => rows,
                    None => {
                        let rows = take(owner, held);
                        edited.entry(owner).or_insert(rows)
                    }
                };
                match rows.binary_search_by_key(&row.0, |held| held.0) {
                    Ok(at) => rows[at] = row,
                    Err(at) => rows.insert(at, row),
                }
            }
            for (owner, rows) in edited {
                let same = held.get(&owner).is_some_and(|was| **was == *rows);
                if same {
                    continue;
                }
                changed.insert(owner);
                if rows.is_empty() {
                    held.remove(&owner);
                } else {
                    held.insert(owner, Arc::from(rows));
                }
            }
        }
    }
}
