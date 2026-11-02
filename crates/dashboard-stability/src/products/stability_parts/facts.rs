//! Stability's parts, projected hour by hour from a columnar walk ([`replay::segments`]): each
//! country whose parts or naming may have changed, with its holder, its supplying read and
//! whether its lines were recorded with that read, its naming, and the read's parts where the
//! segment has not handed them on for that holder yet. A segment's first hour, and an hour
//! after a skipped one, projects every country.
//!
//! A read's parts are worked out from its inputs row and its lines as the walker's columns
//! hold them, read only for a country whose parts are handed on. A country's line parts are
//! kept from read to read while its lines and the characters they name stay as they were, so a
//! read whose inputs changed works out only its terms, and one whose inputs did not either is
//! handed on as the same parts. Each part's id is numbered once a segment, and its names handed
//! on only where they differ from those the segment handed on for it last.
//!
//! A part naming a character names it as the read's own hour recorded it. The projection names
//! a read taken at its hour itself, and hands on each character whose recorded naming may have
//! changed (every character at a segment's first hour), so the serial emit names a read an
//! hour after it meets first, as a segment's first hours do, as the read's hour held them
//! ([`super::emit`]).

use super::{Addend, Character, LINES_OF, addends};
use crate::products::country_stability::{
    self as stability, BreakdownLine, breakdown, formula, modifier, note_refused,
    recorded_character_row,
};
use dashboard_text::{
    catalogue::Refusals,
    identity::{CivilWars, NamingCountry},
};
use dashboard_world::{Hold, Needs, Roster, Tag, World};
use replay::{
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::rows::{
    CharacterRow, CountryStabilityInputRow, CountryStabilityLineRow, LexicalTokenRow, TableId,
    tables,
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::{
    cell::OnceCell,
    collections::{BTreeSet, HashMap},
    io,
    mem::{size_of, take},
    sync::{Arc, LazyLock},
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &READ;

const READ: [TableId; 13] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryStabilityInputs,
    TableId::CountryStabilityLines,
    TableId::SliceReads,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::CountryCivilWars,
    TableId::CountryCounterInputs,
    TableId::CountryWars,
    TableId::WarRelations,
];

/// Positions in [`READ`].
const INPUTS: usize = 2;
const LINES: usize = 3;
const CHARACTERS: usize = 5;
const TOKENS: usize = 6;

/// The Characters fields a character line's naming reads.
const CHARACTER_FIELDS: [&str; 7] = [
    "name_token_id",
    "token",
    "template_present",
    "template_tag",
    "template_file",
    "template_line",
    "template_generated",
];

/// How a projection asks the walk's frame for the tables it reads as rows: what a character
/// line's naming reads, and what a country's naming reads, its civil war among it, each in
/// the fields read.
struct Asks {
    characters: Vec<(TableId, Hold)>,
    naming: Vec<(TableId, Hold)>,
    both: Vec<(TableId, Hold)>,
}

fn asks() -> &'static Asks {
    static ASKS: LazyLock<Asks> = LazyLock::new(|| {
        let field_mask = dashboard_base::prepared::frame_changes::field_mask;
        let characters = vec![
            (
                TableId::Characters,
                Hold::Fields(field_mask(TableId::Characters, &CHARACTER_FIELDS))
                    .of(TableId::Characters),
            ),
            (TableId::LexicalTokens, Hold::Whole),
        ];
        let mut naming: Vec<(TableId, Hold)> =
            [TableId::CountryAppearance, TableId::ReservedCountryNaming]
                .into_iter()
                .map(|table| (table, Hold::Whole))
                .collect();
        for read in dashboard_text::identity::civil_war_reads() {
            let mut fields = read.fields;
            // An original-or-own tag falls back to the country's own tag.
            if read.table == TableId::Countries {
                fields |= field_mask(TableId::Countries, &["tag"]);
            }
            match naming.iter_mut().find(|(table, _)| *table == read.table) {
                Some((_, Hold::Fields(held))) => *held |= fields,
                Some((_, Hold::Whole)) => {}
                None => naming.push((read.table, Hold::Fields(fields))),
            }
        }
        for (table, hold) in &mut naming {
            *hold = hold.of(*table);
        }
        let both = characters.iter().chain(&naming).copied().collect();
        Asks {
            characters,
            naming,
            both,
        }
    });
    &ASKS
}

/// What a projection reads of its walk's world.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        frame: asks().both.clone(),
        ..Needs::default()
    }
}

/// A part's names: its formula term, or its line and the character the line names.
#[derive(Debug, PartialEq)]
pub struct PartNames {
    pub term: Option<String>,
    pub line: Option<BreakdownLine>,
    pub character: Option<Character>,
}

impl PartNames {
    fn heap(&self) -> usize {
        self.term.as_ref().map_or(0, String::capacity)
            + self.line.as_ref().map_or(0, super::line_heap)
            + self.character.as_ref().map_or(0, character_heap)
    }
}

/// A read's parts named by its own hour: each measure's parts by part id with their raw
/// values, None where not all are known; the names of those whose names differ from what the
/// segment handed on for them last; and the tree labels its lines refused.
#[derive(Debug)]
pub struct Named {
    pub measures: [Option<Vec<(u32, i64)>>; 2],
    pub names: Vec<(u8, u32, Arc<PartNames>)>,
    pub refused: Refusals,
}

/// A read met after its hour: its addends, character lines left for the emit to name.
#[derive(Debug)]
pub struct Unnamed {
    pub measures: [Option<Vec<Addend>>; 2],
    pub refused: Refusals,
}

/// A read's parts for its holder.
#[derive(Debug)]
pub enum Parts {
    /// The parts the segment handed on last for the country, at a new read of the same rows.
    Same,
    Named(Box<Named>),
    Unnamed(Box<Unnamed>),
}

/// One country at a projected hour.
#[derive(Debug)]
pub struct Country {
    /// Its tag's index in the projection ([`Hour::named`]).
    pub index: u32,
    /// Its canonical slot and id.
    pub holder: (u32, u32),
    /// Its inputs' read hour.
    pub read: i32,
    /// Its lines were recorded with that read; nothing else is known of an unlined read.
    pub lined: bool,
    /// Its naming at the hour, where lined.
    pub naming: Option<Arc<NamingCountry>>,
    /// The read's parts, where the segment met that read for this holder first.
    pub parts: Option<Parts>,
}

/// The characters the hour recorded, as a character line names them.
#[derive(Debug)]
pub enum Characters {
    /// Characters is not complete: no character is known at the hour.
    Unknown,
    /// Every character the hour holds; any other is gone.
    Every(Vec<(u32, Character)>),
    /// Those whose naming may have changed since the hour before; None where gone.
    Changed(Vec<(u32, Option<Character>)>),
}

/// Every country's inputs read hour, by country id, where SliceReads dates them.
#[derive(Debug)]
pub enum Reads {
    /// Every dated country; any other is dated no more.
    Every(Vec<(u32, i32)>),
    /// Those the hour dated anew since the hour before; None where no longer dated.
    Changed(Vec<(u32, Option<i32>)>),
}

/// One projected hour.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// Part ids the projection numbers from this hour on, by index.
    pub ids: Vec<(u32, Arc<str>)>,
    /// The tags with a current holder, inputs and a read, ascending, when they may have
    /// changed.
    pub valid: Option<Vec<u32>>,
    /// Where the inputs and their reads were read whole, the reads the hour dates: no later
    /// hour dates a read before the earliest of them again.
    pub reads: Option<Reads>,
    pub characters: Characters,
    /// Countries whose parts or naming may have changed, at least every one that did.
    pub countries: Vec<Country>,
}

fn character_heap(character: &Character) -> usize {
    character.0.as_ref().map_or(0, String::capacity)
        + character.1.capacity()
        + character
            .2
            .as_ref()
            .map_or_else(String::capacity, |(tag, file, _, _)| {
                tag.capacity() + file.capacity()
            })
}

impl Parts {
    fn heap(&self) -> usize {
        match self {
            Self::Same => 0,
            Self::Named(named) => {
                size_of::<Named>()
                    + named
                        .measures
                        .iter()
                        .flatten()
                        .map(|parts| parts.capacity() * size_of::<(u32, i64)>())
                        .sum::<usize>()
                    + named.names.capacity() * size_of::<(u8, u32, Arc<PartNames>)>()
                    + named
                        .names
                        .iter()
                        .map(|(_, _, names)| size_of::<PartNames>() + names.heap())
                        .sum::<usize>()
            }
            Self::Unnamed(unnamed) => {
                size_of::<Unnamed>()
                    + unnamed
                        .measures
                        .iter()
                        .flatten()
                        .map(|addends| {
                            addends
                                .iter()
                                .map(|addend| size_of::<Addend>() + addend.heap())
                                .sum::<usize>()
                        })
                        .sum::<usize>()
            }
        }
    }
}

impl segments::Item for Hour {
    fn heap(&self) -> usize {
        let tags = |named: &Vec<(u32, Arc<str>)>| {
            named.capacity() * size_of::<(u32, Arc<str>)>()
                + named.iter().map(|(_, tag)| tag.len()).sum::<usize>()
        };
        tags(&self.named)
            + tags(&self.ids)
            + self.valid.as_ref().map_or(0, |valid| valid.capacity() * 4)
            + match &self.reads {
                Some(Reads::Every(every)) => every.capacity() * size_of::<(u32, i32)>(),
                Some(Reads::Changed(changed)) => {
                    changed.capacity() * size_of::<(u32, Option<i32>)>()
                }
                None => 0,
            }
            + match &self.characters {
                Characters::Unknown => 0,
                Characters::Every(every) => {
                    every.capacity() * size_of::<(u32, Character)>()
                        + every
                            .iter()
                            .map(|(_, character)| character_heap(character))
                            .sum::<usize>()
                }
                Characters::Changed(changed) => {
                    changed.capacity() * size_of::<(u32, Option<Character>)>()
                        + changed
                            .iter()
                            .filter_map(|(_, character)| character.as_ref())
                            .map(character_heap)
                            .sum::<usize>()
                }
            }
            + self.countries.capacity() * size_of::<Country>()
            + self
                .countries
                .iter()
                .filter_map(|country| country.parts.as_ref())
                .map(Parts::heap)
                .sum::<usize>()
    }
}

/// The columns a projection reads in place.
struct Fields {
    sample_hour: usize,
    start_hour: usize,
    end_hour: usize,
    /// The CountryAppearance fields a country's naming reads.
    appearance: u64,
    /// The Characters fields a character line's naming reads, and its token columns.
    character: u64,
    name_token: usize,
    template_tag: usize,
    /// The fields of each table a civil war reads, and whether it ends with its rows' end hour.
    civil: Vec<(TableId, u64, bool)>,
}

/// A measure's parts of a read's lines for one recorded key: its lines' total, and their parts
/// by id, nonzero, merged by source in tree order; None where a refused label's recorded bytes
/// are not found.
struct LineParts {
    sum: i64,
    values: Option<Vec<(u32, i64, Arc<PartNames>)>>,
}

/// A country's parts as last worked out at their read's hour, kept for its next reads.
struct Kept {
    holder: (u32, u32),
    tag: Box<str>,
    /// The hour its character lines were named at.
    at: i32,
    row: CountryStabilityInputRow,
    /// Each measure's line parts; None where the read holds no total for its key.
    lines: [Option<LineParts>; 2],
    /// The characters its character lines name.
    characters: Vec<u32>,
    /// Its country's inputs and lines changes ([`Held`]) as of the rows it was worked out of.
    inputs_seen: u64,
    lines_seen: u64,
}

/// What the segment handed on of a tag: the holder and read whose parts it handed on, the
/// naming it read for a holder, the parts it worked out at their read's hour, and each
/// measure's names as last handed on, by part id.
#[derive(Default)]
struct Handed {
    worked: Option<((u32, u32), i32)>,
    naming: Option<((u32, u32), Arc<NamingCountry>)>,
    kept: Option<Kept>,
    sent: [HashMap<u32, Arc<PartNames>>; 2],
}

/// A country's line keys, ascending, and how often its inputs row and its lines may have
/// changed: a kept part is current while the counts it saw stand.
#[derive(Default)]
struct Held {
    lines: Vec<u128>,
    inputs_moved: u64,
    lines_moved: u64,
}

impl Held {
    fn inputs_changed(&mut self) {
        self.inputs_moved = self.inputs_moved.wrapping_add(1);
    }

    fn lines_changed(&mut self) {
        self.lines_moved = self.lines_moved.wrapping_add(1);
    }

    fn seen(held: Option<&Self>) -> (u64, u64) {
        held.map_or((0, 0), |held| (held.inputs_moved, held.lines_moved))
    }
}

/// How a projected country's parts are handed on.
enum Work {
    /// Its lines were not recorded with its read.
    Unlined,
    /// The segment handed them on for this holder and read.
    Handed,
    Same,
    /// Its inputs row anew, over its kept line parts.
    Terms(CountryStabilityInputRow),
    /// Its inputs row and lines anew.
    Lines(CountryStabilityInputRow, Vec<CountryStabilityLineRow>),
    /// A read met after its hour.
    Late(CountryStabilityInputRow, Vec<CountryStabilityLineRow>),
}

/// A projected country: its id, tag, index, holder, inputs slot and read.
#[derive(Clone, Copy)]
struct Projected {
    id: u32,
    tag: Tag,
    index: u32,
    holder: (u32, u32),
    slot: u32,
    read: i32,
}

/// The lists an hour fills, kept from hour to hour.
#[derive(Default)]
struct Scratch {
    dirty: Vec<u32>,
    renamed: Vec<u32>,
    redated: Vec<u32>,
    characters: Vec<u32>,
    gone: Vec<u32>,
    spelled: Vec<u32>,
    projected: Vec<Projected>,
    plans: Vec<(Work, bool)>,
}

/// One segment's projection.
pub struct Projector {
    format: u16,
    fields: Fields,
    /// Each of [`TABLES`]' status the hour before.
    statuses: Option<[columns::Status; READ.len()]>,
    numbers: HashMap<Tag, u32>,
    ids: HashMap<Arc<str>, u32>,
    /// Each formula term's part id and names.
    terms: HashMap<&'static str, (u32, Arc<PartNames>)>,
    /// The names of a part with neither term nor line.
    plain: Arc<PartNames>,
    last: Option<i32>,
    handed: HashMap<Tag, Handed>,
    countries: HashMap<u32, Held>,
    /// The hour each character's naming last may have changed, from `characters_from` on.
    renamed: HashMap<u32, i32>,
    characters_from: Option<i32>,
    /// The countries, by id, whose naming read a civil war (a dynamic country's).
    dynamic: BTreeSet<u32>,
    /// Those whose naming refused an input: a refusal names its hour, so it is read again
    /// every hour, as a dynamic country's naming is.
    failing: BTreeSet<u32>,
    /// The hour before read its inputs and their reads whole.
    admitted: bool,
    /// The frame the world's views are lent into for an hour, given back before it ends.
    frame: DecodedFrame,
    scratch: Scratch,
}

/// The country of `id` and its tag, when it holds the roster's place for its tag: the tag's
/// first holder by id, even unresolved; only a resolved holder with a UTF-8 tag has parts.
fn member(roster: &Roster, id: u32) -> Option<(Tag, (u32, u32), &str)> {
    let country = roster.get(id)?;
    if !country.resolved || roster.holders(country.tag).next()?.id != id {
        return None;
    }
    Some((
        country.tag,
        (country.slot, country.id),
        std::str::from_utf8(roster.tag(country.tag)).ok()?,
    ))
}

/// Visits the keys of the rows the hour changed, inserted or filled in `table`, and of those
/// it deleted: whether it inserted or deleted any.
fn touched(
    state: &replay::segments::State,
    table: TableId,
    mut visit: impl FnMut(u128),
) -> io::Result<bool> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    for &slot in changes
        .changed
        .iter()
        .chain(&changes.inserted)
        .chain(&changes.fill_slots)
    {
        if let Some(key) = held.key(slot) {
            visit(key);
        }
    }
    changes.deleted_keys.iter().copied().for_each(&mut visit);
    Ok(!changes.inserted.is_empty() || !changes.deleted.is_empty())
}

/// The country a line's key leads with.
fn line_country(key: u128) -> u32 {
    (key >> 64) as u32
}

/// The world's views lent into `frame` for the projection's asks, given back when dropped.
struct Lent<'o> {
    frame: &'o mut DecodedFrame,
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        dashboard_world::release(self.frame, &asks().both);
    }
}

/// A character row's naming as a character line names it, with the hour's tokens.
fn character_of(row: &CharacterRow, lexical: Option<&[LexicalTokenRow]>) -> Character {
    let found = recorded_character_row(row, lexical);
    (
        found.script.map(str::to_owned),
        found.saved.to_owned(),
        found
            .template
            .map(|template| (template.tag, template.file, template.line, template.generated)),
    )
}

/// Character `id` as the hour's `characters` hold it.
fn recorded(
    characters: Option<&[CharacterRow]>,
    lexical: Option<&[LexicalTokenRow]>,
    id: u32,
) -> Option<Character> {
    let rows = characters?;
    let at = rows.binary_search_by_key(&id, |row| row.id).ok()?;
    Some(character_of(&rows[at], lexical))
}

/// Whether `line` names a character, whose naming completes its part's id.
fn names_character(line: &CountryStabilityLineRow) -> bool {
    matches!(
        line.kind,
        CountryStabilityLineRow::ADVISOR | CountryStabilityLineRow::COUNTRY_LEADER
    )
}

/// Part `id`'s number, announced in `announced` when new.
fn intern(
    ids: &mut HashMap<Arc<str>, u32>,
    announced: &mut Vec<(u32, Arc<str>)>,
    id: &str,
) -> u32 {
    if let Some(&index) = ids.get(id) {
        return index;
    }
    let index = ids.len() as u32;
    let id: Arc<str> = Arc::from(id);
    ids.insert(Arc::clone(&id), index);
    announced.push((index, id));
    index
}

/// A measure's line parts of `lines` (one country's, by line and key) for recorded key
/// `index`, as the breakdown's modifier lists them and its parts name them: an unpaired line
/// by its text or its refused label's recorded bytes, a character line completed by its
/// character as `character` names it.
fn line_parts(
    lines: &[CountryStabilityLineRow],
    index: Option<u32>,
    character: &dyn Fn(u32) -> Option<Character>,
    intern: &mut dyn FnMut(&str) -> u32,
) -> LineParts {
    let Some(key) = index else {
        return LineParts {
            sum: 0,
            values: Some(Vec::new()),
        };
    };
    let mut sum = 0_i64;
    let mut values: Vec<(u32, i64, Arc<PartNames>)> = Vec::new();
    let mut known = true;
    for line in lines.iter().filter(|line| line.key == key) {
        sum = sum.wrapping_add(line.value);
        let kind = stability::kind(line.kind);
        let text = (line.kind == CountryStabilityLineRow::UNPAIRED)
            .then(|| stability::display_text(&line.text).ok())
            .flatten();
        let unpaired = match (kind, &text) {
            ("unpaired", Some(text)) => Some(format!("unpaired:{text}")),
            ("unpaired", None) => {
                match lines.binary_search_by_key(&(line.line, key), |recorded| {
                    (recorded.line, recorded.key)
                }) {
                    Ok(at) => Some(format!("unpaired-refused:{}", lines[at].text.escape_ascii())),
                    Err(_) => {
                        known = false;
                        continue;
                    }
                }
            }
            _ => None,
        };
        if line.value == 0 {
            continue;
        }
        let identity = String::from_utf8_lossy(&line.identity).into_owned();
        let (id, recorded) = match unpaired {
            Some(id) => (id, None),
            None => {
                let base = format!(
                    "line:{kind}:{identity}:{}:{}",
                    line.identity_id, line.secondary
                );
                match matches!(kind, "advisor" | "countryLeader")
                    .then(|| character(line.identity_id))
                    .flatten()
                {
                    Some(found) => (
                        format!(
                            "{base}:{}:{}",
                            found.0.as_deref().unwrap_or_default(),
                            found.1
                        ),
                        Some(found),
                    ),
                    None => (base, None),
                }
            }
        };
        let id = intern(&id);
        // Two lines of one source add up, as the game's total does.
        if let Some((_, raw, _)) = values.iter_mut().find(|(held, _, _)| *held == id) {
            *raw = raw.wrapping_add(line.value);
            continue;
        }
        values.push((
            id,
            line.value,
            Arc::new(PartNames {
                term: None,
                line: Some(BreakdownLine {
                    line: line.line,
                    kind: kind.into(),
                    identity,
                    identity_id: line.identity_id,
                    secondary_raw: line.secondary.to_string(),
                    value_raw: line.value.to_string(),
                    text,
                    label: None,
                    group: None,
                    group_kind: None,
                }),
                character: recorded,
            }),
        ));
    }
    LineParts {
        sum,
        values: known.then_some(values),
    }
}

/// The recorded key index and total of measure `measure`'s lines in `row`; None where the
/// read holds no such key.
fn key_of(row: &CountryStabilityInputRow, measure: usize) -> Option<(Option<u32>, Option<i64>)> {
    let at = CountryStabilityInputRow::KEY_NAMES
        .iter()
        .position(|name| *name == LINES_OF[measure].1.as_bytes())?;
    let recorded = *row.keys.get(at)?;
    let total = *row.totals().get(at)?;
    Some(((recorded != u32::MAX).then_some(recorded), total))
}

/// The ids and names a measure's terms, remainder and clamp are handed on by.
struct Plain<'p> {
    ids: &'p mut HashMap<Arc<str>, u32>,
    announced: &'p mut Vec<(u32, Arc<str>)>,
    terms: &'p mut HashMap<&'static str, (u32, Arc<PartNames>)>,
    plain: &'p Arc<PartNames>,
}

impl Plain<'_> {
    fn term(&mut self, name: &'static str) -> (u32, Arc<PartNames>) {
        if let Some((id, names)) = self.terms.get(name) {
            return (*id, Arc::clone(names));
        }
        let id = intern(self.ids, self.announced, &format!("term:{name}"));
        let names = Arc::new(PartNames {
            term: Some(name.to_owned()),
            line: None,
            character: None,
        });
        self.terms.insert(name, (id, Arc::clone(&names)));
        (id, names)
    }

    fn plain(&mut self, id: &str) -> (u32, Arc<PartNames>) {
        (intern(self.ids, self.announced, id), Arc::clone(self.plain))
    }
}

/// Measure `measure`'s parts at a read of `row`, its line parts `lines`, as the breakdown's
/// parts add up to its displayed value: its terms in order, the line term as its lines and
/// what they leave of their total, then the clamp; None where any is not known.
fn measure(
    row: &CountryStabilityInputRow,
    measure: usize,
    lines: &LineParts,
    total: Option<i64>,
    plain: &mut Plain<'_>,
) -> Option<Vec<(u32, i64, Arc<PartNames>)>> {
    let terms = if measure == 0 {
        formula::stability(row)
    } else {
        formula::war_support(row)
    };
    let value = terms.value?;
    let mut parts = Vec::new();
    for &(name, raw) in &terms.terms {
        let raw = raw?;
        if name == LINES_OF[measure].0 {
            parts.extend(
                lines
                    .values
                    .as_ref()?
                    .iter()
                    .map(|(id, raw, names)| (*id, *raw, Arc::clone(names))),
            );
            if let Some(remainder) = total
                .map(|total| total.wrapping_sub(lines.sum))
                .filter(|remainder| *remainder != 0)
            {
                let (id, names) = plain.plain("remainder");
                parts.push((id, remainder, names));
            }
            continue;
        }
        if raw != 0 {
            let (id, names) = plain.term(name);
            parts.push((id, raw, names));
        }
    }
    if let Some(unclamped) = terms.unclamped {
        let clamp = value.wrapping_sub(unclamped);
        if clamp != 0 {
            let (id, names) = plain.plain("clamp");
            parts.push((id, clamp, names));
        }
    }
    Some(parts)
}

/// `parts` as handed on, each one's names among `names` where they differ from what `sent`
/// holds of it.
fn hand(
    sent: &mut HashMap<u32, Arc<PartNames>>,
    index: u8,
    parts: Option<Vec<(u32, i64, Arc<PartNames>)>>,
    names: &mut Vec<(u8, u32, Arc<PartNames>)>,
) -> Option<Vec<(u32, i64)>> {
    let parts = parts?;
    let mut values = Vec::with_capacity(parts.len());
    for (id, raw, part) in parts {
        values.push((id, raw));
        let same = sent
            .get(&id)
            .is_some_and(|held| Arc::ptr_eq(held, &part) || **held == *part);
        if !same {
            sent.insert(id, Arc::clone(&part));
            names.push((index, id, part));
        }
    }
    Some(values)
}

/// A read's addends of each measure and the tree labels its lines refuse, for `tag`.
fn late_addends(
    row: &CountryStabilityInputRow,
    lines: &[CountryStabilityLineRow],
    tag: &str,
) -> ([Option<Vec<Addend>>; 2], Refusals) {
    let mut refused = Refusals::default();
    let totals = row.totals();
    let formulas = [
        breakdown(formula::stability(row)),
        breakdown(formula::war_support(row)),
    ];
    // Escape recorded bytes so distinct refused labels remain distinct sources.
    let refused_label = |line: &BreakdownLine, key: Option<u32>| {
        let at = lines
            .binary_search_by_key(&(line.line, key?), |recorded| (recorded.line, recorded.key))
            .ok()?;
        Some(lines[at].text.escape_ascii().to_string())
    };
    let mut measures: [Option<Vec<Addend>>; 2] = [None, None];
    for (index, (term, key)) in LINES_OF.into_iter().enumerate() {
        let Some(at) = CountryStabilityInputRow::KEY_NAMES
            .iter()
            .position(|name| *name == key.as_bytes())
        else {
            continue;
        };
        let (Some(&recorded), Some(&total)) = (row.keys.get(at), totals.get(at)) else {
            continue;
        };
        let held = modifier(
            row,
            (CountryStabilityInputRow::KEY_NAMES[at], recorded),
            total,
            Some(lines),
            &|_| None,
            tag,
            &mut refused,
        );
        measures[index] = addends(&formulas[index], Some(&held), term, &refused_label);
    }
    note_refused(row, lines, tag, &mut refused);
    (measures, refused)
}

impl Projector {
    pub fn new(format: u16) -> io::Result<Self> {
        let civil = dashboard_text::identity::civil_war_reads()
            .into_iter()
            .map(|read| (read.table, read.fields, read.ends))
            .collect();
        Ok(Self {
            format,
            fields: Fields {
                sample_hour: columns::field(TableId::SliceReads, "sample_hour")?,
                start_hour: columns::field(TableId::WarRelations, "start_hour")?,
                end_hour: columns::field(TableId::WarRelations, "end_hour")?,
                appearance: dashboard_base::prepared::frame_changes::field_mask(
                    TableId::CountryAppearance,
                    &[
                        "name_tag",
                        "cosmetic_tag",
                        "original_tag",
                        "original_tag_reference",
                    ],
                ),
                character: dashboard_base::prepared::frame_changes::field_mask(
                    TableId::Characters,
                    &CHARACTER_FIELDS,
                ),
                name_token: columns::field(TableId::Characters, "name_token_id")?,
                template_tag: columns::field(TableId::Characters, "template_tag")?,
                civil,
            },
            statuses: None,
            numbers: HashMap::new(),
            ids: HashMap::new(),
            terms: HashMap::new(),
            plain: Arc::new(PartNames {
                term: None,
                line: None,
                character: None,
            }),
            last: None,
            handed: HashMap::new(),
            countries: HashMap::new(),
            renamed: HashMap::new(),
            characters_from: None,
            dynamic: BTreeSet::new(),
            failing: BTreeSet::new(),
            admitted: false,
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
            scratch: Scratch::default(),
        })
    }

    fn number(&mut self, tag: Tag, text: &str, named: &mut Vec<(u32, Arc<str>)>) -> u32 {
        if let Some(&index) = self.numbers.get(&tag) {
            return index;
        }
        let index = self.numbers.len() as u32;
        self.numbers.insert(tag, index);
        named.push((index, Arc::from(text)));
        index
    }

    /// Whether the hour may have changed a country's civil war: it wrote what one reads, or a
    /// war relation ended at it.
    fn civil_touched(&self, hour: &segments::Hour<'_, World>) -> io::Result<bool> {
        for &(table, fields, ends) in &self.fields.civil {
            if hour.world.touched(table, fields) {
                return Ok(true);
            }
            if !ends || !columns::complete(hour.state, table) {
                continue;
            }
            let held = columns::table(hour.state, table)?;
            let starts = columns::plain::<i32>(held, self.fields.start_hour)?;
            let ends = columns::plain::<i32>(held, self.fields.end_hour)?;
            let ended = held.live().ones().any(|slot| {
                match (starts.get(slot as usize), ends.get(slot as usize)) {
                    (Some(&start), Some(&end)) => end > start && end == hour.hour,
                    _ => false,
                }
            });
            if ended {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Follows each country's line keys through the hour's changes, every one anew where the
    /// lines' status changed, and notes the countries whose lines changed.
    fn follow_lines(&mut self, state: &segments::State, anew: bool) -> io::Result<()> {
        let held = columns::table(state, TableId::CountryStabilityLines)?;
        if anew {
            for country in self.countries.values_mut() {
                country.lines.clear();
            }
            for slot in held.live().ones() {
                if let Some(key) = held.key(slot) {
                    self.countries
                        .entry(line_country(key))
                        .or_default()
                        .lines
                        .push(key);
                }
            }
            for country in self.countries.values_mut() {
                country.lines.sort_unstable();
                country.lines_changed();
            }
            return Ok(());
        }
        let changes = columns::changes(state, TableId::CountryStabilityLines)?;
        for &key in &changes.deleted_keys {
            let country = self.countries.entry(line_country(key)).or_default();
            if let Ok(at) = country.lines.binary_search(&key) {
                country.lines.remove(at);
            }
            country.lines_changed();
        }
        for &slot in &changes.inserted {
            if let Some(key) = held.key(slot) {
                let country = self.countries.entry(line_country(key)).or_default();
                if let Err(at) = country.lines.binary_search(&key) {
                    country.lines.insert(at, key);
                }
                country.lines_changed();
            }
        }
        for &slot in changes.changed.iter().chain(&changes.fill_slots) {
            if let Some(key) = held.key(slot) {
                self.countries
                    .entry(line_country(key))
                    .or_default()
                    .lines_changed();
            }
        }
        Ok(())
    }

    /// Country `id`'s lines as the walker holds them, by line and key.
    fn lines_of(&self, state: &segments::State, id: u32) -> io::Result<Vec<CountryStabilityLineRow>> {
        let Some(country) = self.countries.get(&id) else {
            return Ok(Vec::new());
        };
        let held = columns::table(state, TableId::CountryStabilityLines)?;
        let mut lines = Vec::with_capacity(country.lines.len());
        for &key in &country.lines {
            let slot = held.slot(key).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "A stability line's key holds no row",
                )
            })?;
            lines.push(columns::row(
                state,
                TableId::CountryStabilityLines,
                slot,
                self.format,
            )?);
        }
        Ok(lines)
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("parts.columnar", Some("parts"));
        if hour.starts {
            self.numbers.clear();
        }
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let statuses: [columns::Status; READ.len()] =
            std::array::from_fn(|table| columns::status(state, READ[table]));
        let skipped = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(at));
        let first = hour.starts || self.last.is_none();
        self.last = Some(at);
        let anew = first || skipped;
        let moved = |table: usize| {
            anew || self
                .statuses
                .is_none_or(|held| held[table] != statuses[table])
        };
        let (inputs_moved, lines_moved, characters_moved) =
            (moved(INPUTS), moved(LINES), moved(CHARACTERS) || moved(TOKENS));
        let restated = anew || self.statuses != Some(statuses);
        self.statuses = Some(statuses);
        let inputs_truncated = columns::truncated(state, TableId::CountryStabilityInputs);
        // A truncated read names its own rows each hour: every country may change.
        let full = restated || inputs_truncated.is_some() || roster.rebuilt();
        if anew {
            // Nothing handed before is known to the emit's state of this hour.
            self.handed.clear();
            self.dynamic.clear();
            self.failing.clear();
        }
        let mut scratch = take(&mut self.scratch);
        let result = self.project(
            hour,
            roster,
            at,
            (full, restated),
            (inputs_moved, lines_moved, characters_moved),
            inputs_truncated,
            &mut scratch,
        );
        self.scratch = scratch;
        out.push(result?)
    }
}

impl Projector {
    #[allow(clippy::too_many_arguments)]
    fn project(
        &mut self,
        hour: &segments::Hour<'_, World>,
        roster: &Roster,
        at: i32,
        (full, restated): (bool, bool),
        (inputs_moved, lines_moved, characters_moved): (bool, bool, bool),
        inputs_truncated: Option<&[u32]>,
        scratch: &mut Scratch,
    ) -> io::Result<Hour> {
        let state = hour.state;
        {
            let _cpu = replay_frame::cpu::scope("parts.columnar.changes", None);
            self.follow_lines(state, lines_moved)?;
            if inputs_moved {
                for country in self.countries.values_mut() {
                    country.inputs_changed();
                }
            }
        }
        let Scratch {
            dirty,
            renamed,
            redated,
            characters: renamed_characters,
            gone: gone_characters,
            spelled,
            projected,
            plans,
        } = scratch;
        dirty.clear();
        renamed.clear();
        redated.clear();
        renamed_characters.clear();
        gone_characters.clear();
        spelled.clear();
        projected.clear();
        plans.clear();
        // Countries whose naming may have changed go to `renamed`.
        let mut membership = full;
        if full {
            dirty.extend(roster.iter().map(|country| country.id));
            renamed.extend(roster.iter().map(|country| country.id));
        } else if roster.changed() {
            // A changed tag may give its place to another of its holders.
            dirty.extend_from_slice(roster.changed_ids());
            for &tag in roster.changed_tags() {
                dirty.extend(roster.holders(tag).map(|country| country.id));
            }
            renamed.extend_from_slice(&dirty[..]);
            membership = true;
        }
        {
            let countries = &mut self.countries;
            membership |= touched(state, TableId::CountryStabilityInputs, |key| {
                dirty.push(key as u32);
                countries.entry(key as u32).or_default().inputs_changed();
            })?;
        }
        let of = |key: u128, table: TableId| (key >> 32) as u32 == table as u32;
        let reads = columns::changes(state, TableId::SliceReads)?;
        let reads_table = columns::table(state, TableId::SliceReads)?;
        for &slot in reads.inserted.iter() {
            membership |= reads_table
                .key(slot)
                .is_some_and(|key| of(key, TableId::CountryStabilityInputs));
        }
        membership |= reads
            .deleted_keys
            .iter()
            .any(|&key| of(key, TableId::CountryStabilityInputs));
        // Countries whose inputs read the hour dated anew go to `redated`.
        touched(state, TableId::SliceReads, |key| {
            if of(key, TableId::CountryStabilityInputs) {
                dirty.push(key as u32);
                redated.push(key as u32);
            } else if of(key, TableId::CountryStabilityLines) {
                dirty.push(key as u32);
            }
        })?;
        redated.sort_unstable();
        redated.dedup();
        {
            let held = columns::table(state, TableId::CountryAppearance)?;
            let edits = columns::changes(state, TableId::CountryAppearance)?;
            for (slots, masks) in [
                (&edits.changed, &edits.masks),
                (&edits.fill_slots, &edits.fill_masks),
            ] {
                for (&slot, &mask) in slots.iter().zip(masks.iter()) {
                    if mask & self.fields.appearance != 0
                        && let Some(key) = held.key(slot)
                    {
                        renamed.push(key as u32);
                    }
                }
            }
            for &slot in &edits.inserted {
                if let Some(key) = held.key(slot) {
                    renamed.push(key as u32);
                }
            }
            renamed.extend(edits.deleted_keys.iter().map(|&key| key as u32));
        }
        if hour
            .world
            .touched(TableId::ReservedCountryNaming, u64::MAX)
        {
            renamed.extend(roster.at_slot(0).map(|country| country.id));
        }
        if !self.dynamic.is_empty() && self.civil_touched(hour)? {
            renamed.extend(self.dynamic.iter().copied());
        }
        renamed.extend(self.failing.iter().copied());
        dirty.extend_from_slice(&renamed[..]);
        dirty.sort_unstable();
        dirty.dedup();

        let available = [
            columns::complete(state, TableId::CountryStabilityInputs) || inputs_truncated.is_some(),
            columns::complete(state, TableId::SliceReads),
        ];
        let inputs = columns::table(state, TableId::CountryStabilityInputs)?;
        let input_slot = |id: u32| -> Option<u32> {
            if !available[0] {
                return None;
            }
            let slot = inputs.slot(u128::from(id))?;
            match inputs_truncated {
                Some(read) => read.binary_search(&slot).is_ok().then_some(slot),
                None => Some(slot),
            }
        };
        let sample_hours = columns::plain::<i32>(reads_table, self.fields.sample_hour)?;
        let dated_at = |table: TableId, id: u32| -> Option<i32> {
            if !available[1] {
                return None;
            }
            let key = (table as u128) << 32 | u128::from(id);
            let slot = reads_table.slot(key)?;
            sample_hours.get(slot as usize).copied()
        };
        let read_hour = |id: u32| dated_at(TableId::CountryStabilityInputs, id);
        let lines_complete = columns::complete(state, TableId::CountryStabilityLines);
        // Its lines were recorded with the read: SliceReads names an owner only while it has
        // rows, so a country read with no lines has none.
        let lined = |id: u32, read: i32| {
            lines_complete
                && dated_at(TableId::CountryStabilityLines, id).is_none_or(|lines| lines == read)
        };
        // Every inputs read the hour dates, whatever the country: the hour before's changed by
        // the hour's, unless an hour between them did not read them whole.
        let admitted =
            columns::complete(state, TableId::CountryStabilityInputs) && available[1];
        let dated = if !admitted {
            None
        } else if restated || !self.admitted {
            let mut every = Vec::new();
            for slot in reads_table.live().ones() {
                if let Some(key) = reads_table.key(slot)
                    && of(key, TableId::CountryStabilityInputs)
                    && let Some(&read) = sample_hours.get(slot as usize)
                {
                    every.push((key as u32, read));
                }
            }
            Some(Reads::Every(every))
        } else {
            Some(Reads::Changed(
                redated.iter().map(|&id| (id, read_hour(id))).collect(),
            ))
        };
        self.admitted = admitted;
        let complete = roster.complete();
        let mut named = Vec::new();
        let valid = if membership {
            let mut valid = Vec::new();
            if complete {
                for country in roster.iter() {
                    let Some((tag, _, text)) = member(roster, country.id) else {
                        continue;
                    };
                    if input_slot(country.id).is_some() && read_hour(country.id).is_some() {
                        valid.push(self.number(tag, text, &mut named));
                    }
                }
            }
            valid.sort_unstable();
            valid.dedup();
            Some(valid)
        } else {
            None
        };
        // Characters whose naming may have changed: their rows, or the tokens spelling them.
        let characters_complete = columns::complete(state, TableId::Characters);
        if characters_moved {
            self.renamed.clear();
            self.characters_from = Some(at);
        }
        if characters_complete && !characters_moved {
            let held = columns::table(state, TableId::Characters)?;
            let edits = columns::changes(state, TableId::Characters)?;
            for (slots, masks) in [
                (&edits.changed, &edits.masks),
                (&edits.fill_slots, &edits.fill_masks),
            ] {
                for (&slot, &mask) in slots.iter().zip(masks.iter()) {
                    if mask & self.fields.character != 0
                        && let Some(key) = held.key(slot)
                    {
                        renamed_characters.push(key as u32);
                    }
                }
            }
            for &slot in &edits.inserted {
                if let Some(key) = held.key(slot) {
                    renamed_characters.push(key as u32);
                }
            }
            gone_characters.extend(
                edits
                    .deleted_keys
                    .iter()
                    .map(|&key| key as u32)
                    .filter(|&key| held.slot(u128::from(key)).is_none()),
            );
            touched(state, TableId::LexicalTokens, |key| {
                spelled.push(key as u32);
            })?;
            if !spelled.is_empty() {
                spelled.sort_unstable();
                spelled.dedup();
                let names = columns::plain::<i32>(held, self.fields.name_token)?;
                let templates = columns::plain::<u32>(held, self.fields.template_tag)?;
                for slot in held.live().ones() {
                    let index = slot as usize;
                    let by_name = names
                        .get(index)
                        .and_then(|&token| u32::try_from(token).ok())
                        .is_some_and(|token| spelled.binary_search(&token).is_ok());
                    let template = templates
                        .get(index)
                        .is_some_and(|token| spelled.binary_search(token).is_ok());
                    if (by_name || template)
                        && let Some(key) = held.key(slot)
                    {
                        renamed_characters.push(key as u32);
                    }
                }
            }
            renamed_characters.sort_unstable();
            renamed_characters.dedup();
            for &id in renamed_characters.iter().chain(gone_characters.iter()) {
                self.renamed.insert(id, at);
            }
        }
        for &id in dirty.iter() {
            let Some((tag, holder, text)) = member(roster, id).filter(|_| complete) else {
                continue;
            };
            let (Some(slot), Some(read)) = (input_slot(id), read_hour(id)) else {
                continue;
            };
            let index = self.number(tag, text, &mut named);
            projected.push(Projected {
                id,
                tag,
                index,
                holder,
                slot,
                read,
            });
        }
        // A naming read before may be stale: read it again.
        for id in renamed.iter() {
            if let Some(country) = roster.get(*id)
                && let Some(entry) = self.handed.get_mut(&country.tag)
            {
                entry.naming = None;
            }
        }

        let format = self.format;
        {
            let _cpu = replay_frame::cpu::scope("parts.columnar.plan", None);
            for projected in projected.iter() {
                let Projected {
                    id,
                    tag,
                    holder,
                    slot,
                    read,
                    ..
                } = *projected;
                let entry = self.handed.entry(tag).or_default();
                if !lined(id, read) {
                    plans.push((Work::Unlined, false));
                    continue;
                }
                let naming = !matches!(&entry.naming, Some((seen, _)) if *seen == holder);
                let work = if entry.worked == Some((holder, read)) {
                    Work::Handed
                } else if read != at {
                    Work::Late(
                        columns::row(state, TableId::CountryStabilityInputs, slot, format)?,
                        Vec::new(),
                    )
                } else {
                    let (inputs_moved, lines_moved) = Held::seen(self.countries.get(&id));
                    let text = std::str::from_utf8(roster.tag(tag)).unwrap_or_default();
                    let renamed_since = &self.renamed;
                    let kept = entry.kept.as_ref().filter(|kept| {
                        kept.holder == holder
                            && *kept.tag == *text
                            && kept.lines_seen == lines_moved
                            && self.characters_from.is_some_and(|from| kept.at >= from)
                            && kept.characters.iter().all(|character| {
                                renamed_since
                                    .get(character)
                                    .is_none_or(|&renamed| renamed <= kept.at)
                            })
                    });
                    match kept {
                        Some(kept) if kept.inputs_seen == inputs_moved => Work::Same,
                        Some(kept) => {
                            let row: CountryStabilityInputRow = columns::row(
                                state,
                                TableId::CountryStabilityInputs,
                                slot,
                                format,
                            )?;
                            if row == kept.row {
                                Work::Same
                            } else if row.keys == kept.row.keys {
                                Work::Terms(row)
                            } else {
                                Work::Lines(row, Vec::new())
                            }
                        }
                        None => Work::Lines(
                            columns::row(state, TableId::CountryStabilityInputs, slot, format)?,
                            Vec::new(),
                        ),
                    }
                };
                plans.push((work, naming));
            }
            // The lines of every read worked out anew.
            for (projected, (work, _)) in projected.iter().zip(plans.iter_mut()) {
                if let Work::Lines(_, lines) | Work::Late(_, lines) = work {
                    *lines = self.lines_of(state, projected.id)?;
                }
            }
        }
        let wants_characters = (characters_complete
            && (restated || !renamed_characters.is_empty()))
            || plans.iter().any(|(work, _)| {
                matches!(work, Work::Lines(_, lines) if lines.iter().any(names_character))
            });
        let wants_naming = plans.iter().any(|&(_, naming)| naming);

        let Self {
            frame,
            handed,
            countries,
            dynamic,
            failing,
            ids,
            terms,
            plain: plain_names,
            ..
        } = self;
        let asked = match (wants_characters, wants_naming) {
            (true, true) => Some(&asks().both),
            (true, false) => Some(&asks().characters),
            (false, true) => Some(&asks().naming),
            (false, false) => None,
        };
        let mut lent = None;
        if let Some(asked) = asked {
            let _cpu = replay_frame::cpu::scope("parts.columnar.frame", None);
            let held = Lent { frame };
            hour.world.frame_held_into(state, asked, &mut *held.frame)?;
            lent = Some(held);
        }
        let frame: Option<&DecodedFrame> = lent.as_ref().map(|held| &*held.frame);
        let characters = frame.and_then(|frame| frame.complete::<tables::Characters>().ok());
        let lexical = frame.and_then(|frame| frame.complete::<tables::LexicalTokens>().ok());
        let recorded_characters = match characters.filter(|_| characters_complete) {
            None if !characters_complete || restated => Characters::Unknown,
            Some(rows) if restated => Characters::Every(
                rows.iter()
                    .map(|row| (row.id, character_of(row, lexical)))
                    .collect(),
            ),
            // The frame is held whenever a character's naming may have changed.
            rows => Characters::Changed(
                renamed_characters
                    .iter()
                    .map(|&id| (id, recorded(rows, lexical, id)))
                    .chain(gone_characters.iter().map(|&id| (id, None)))
                    .collect(),
            ),
        };
        let appearances =
            frame.and_then(|frame| frame.complete::<tables::CountryAppearance>().ok());
        let rows = frame.and_then(|frame| frame.complete::<tables::Countries>().ok());
        // The hour's civil wars, indexed once for every dynamic country named.
        let wars: OnceCell<Result<CivilWars<'_>, String>> = OnceCell::new();
        let civil_war = |slot: u32| -> Result<bool, String> {
            let frame = frame.ok_or("the hour's civil-war tables are not held")?;
            wars.get_or_init(|| CivilWars::new(frame))
                .as_ref()
                .map_err(Clone::clone)?
                .holds(slot)
        };
        let character = |id: u32| recorded(characters, lexical, id);
        let mut announced = Vec::new();
        let mut items = Vec::with_capacity(projected.len());
        let _cpu = replay_frame::cpu::scope("parts.columnar.parts", None);
        for (projected, (plan, read_naming)) in projected.iter().zip(plans.drain(..)) {
            let Projected {
                id,
                tag,
                index,
                holder,
                read,
                ..
            } = *projected;
            if let Work::Unlined = plan {
                items.push(Country {
                    index,
                    holder,
                    read,
                    lined: false,
                    naming: None,
                    parts: None,
                });
                continue;
            }
            let text = std::str::from_utf8(roster.tag(tag)).unwrap_or_default();
            let entry = handed.entry(tag).or_default();
            let naming = match (&entry.naming, read_naming) {
                (Some((_, naming)), false) => Arc::clone(naming),
                _ => {
                    let row = rows.and_then(|rows| {
                        rows.binary_search_by_key(&id, |row| row.id)
                            .ok()
                            .map(|at| &rows[at])
                    });
                    let naming = match (row, frame) {
                        (Some(row), Some(frame)) => {
                            let appearance = appearances.and_then(|rows| {
                                rows.binary_search_by_key(&id, |row| row.country)
                                    .ok()
                                    .map(|at| &rows[at])
                            });
                            NamingCountry::read_with_civil_war(frame, row, appearance, &civil_war)
                        }
                        _ => NamingCountry {
                            slot: holder.0,
                            look: None,
                        },
                    };
                    let look = naming.look.as_ref().filter(|look| !look.name_tag.is_empty());
                    if look.is_some() {
                        dynamic.insert(id);
                    } else {
                        dynamic.remove(&id);
                    }
                    if look.is_some_and(|look| {
                        look.civil_war.is_err() || look.original_or_own_tag.is_err()
                    }) {
                        failing.insert(id);
                    } else {
                        failing.remove(&id);
                    }
                    let naming = Arc::new(naming);
                    entry.naming = Some((holder, Arc::clone(&naming)));
                    naming
                }
            };
            let seen = Held::seen(countries.get(&id));
            let mut plain = Plain {
                ids: &mut *ids,
                announced: &mut announced,
                terms: &mut *terms,
                plain: &*plain_names,
            };
            let parts = match plan {
                Work::Unlined | Work::Handed => None,
                Work::Same => {
                    if let Some(kept) = &mut entry.kept {
                        kept.inputs_seen = seen.0;
                    }
                    Some(Parts::Same)
                }
                Work::Terms(row) => {
                    let kept = entry.kept.as_mut().ok_or_else(|| {
                        io::Error::other("A stability parts read kept no line parts")
                    })?;
                    kept.inputs_seen = seen.0;
                    let mut names = Vec::new();
                    let measures: [Option<Vec<(u32, i64)>>; 2] = std::array::from_fn(|index| {
                        let parts = kept.lines[index].as_ref().and_then(|lines| {
                            let (_, total) = key_of(&row, index)?;
                            measure(&row, index, lines, total, &mut plain)
                        });
                        hand(&mut entry.sent[index], index as u8, parts, &mut names)
                    });
                    kept.row = row;
                    Some(Parts::Named(Box::new(Named {
                        measures,
                        names,
                        refused: Refusals::default(),
                    })))
                }
                Work::Lines(row, lines) => {
                    let mut refused = Refusals::default();
                    note_refused(&row, &lines, text, &mut refused);
                    let parts: [Option<LineParts>; 2] = std::array::from_fn(|index| {
                        let (key, _) = key_of(&row, index)?;
                        Some(line_parts(&lines, key, &character, &mut |id| {
                            intern(plain.ids, plain.announced, id)
                        }))
                    });
                    let mut names = Vec::new();
                    let measures: [Option<Vec<(u32, i64)>>; 2] = std::array::from_fn(|index| {
                        let parts = parts[index].as_ref().and_then(|lines| {
                            let (_, total) = key_of(&row, index)?;
                            measure(&row, index, lines, total, &mut plain)
                        });
                        hand(&mut entry.sent[index], index as u8, parts, &mut names)
                    });
                    let mut characters_named: Vec<u32> = lines
                        .iter()
                        .filter(|line| names_character(line))
                        .map(|line| line.identity_id)
                        .collect();
                    characters_named.sort_unstable();
                    characters_named.dedup();
                    entry.kept = Some(Kept {
                        holder,
                        tag: text.into(),
                        at,
                        row,
                        lines: parts,
                        characters: characters_named,
                        inputs_seen: seen.0,
                        lines_seen: seen.1,
                    });
                    Some(Parts::Named(Box::new(Named {
                        measures,
                        names,
                        refused,
                    })))
                }
                Work::Late(row, lines) => {
                    let (measures, refused) = late_addends(&row, &lines, text);
                    // The emit names these: what it holds of each part is no longer what was
                    // handed on.
                    entry.kept = None;
                    entry.sent = Default::default();
                    Some(Parts::Unnamed(Box::new(Unnamed { measures, refused })))
                }
            };
            if parts.is_some() {
                entry.worked = Some((holder, read));
            }
            items.push(Country {
                index,
                holder,
                read,
                lined: true,
                naming: Some(naming),
                parts,
            });
        }
        Ok(Hour {
            branch: hour.branch,
            hour: at,
            named,
            ids: announced,
            valid,
            reads: dated,
            characters: recorded_characters,
            countries: items,
        })
    }
}

#[cfg(test)]
#[test]
fn the_read_positions_name_their_tables() {
    assert_eq!(READ[INPUTS], TableId::CountryStabilityInputs);
    assert_eq!(READ[LINES], TableId::CountryStabilityLines);
    assert_eq!(READ[CHARACTERS], TableId::Characters);
    assert_eq!(READ[TOKENS], TableId::LexicalTokens);
}
