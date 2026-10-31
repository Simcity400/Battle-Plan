//! Nuclear stockpile facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! at each read of a complete Nukes table, the stockpile of every holder slot whose rows changed
//! since its last read in the segment (every slot's after a whole hour), with the read's counts
//! where they changed; and the tags the walk's countries resolve ([`dashboard_world`]) where
//! they changed. What a tag held before the segment is the ordered emit's ([`super::emit`]). An
//! hour that changes nothing the stockpile reads carries nothing but its hour and its read.

use super::{NukeCount, displayed_amount, name_token};
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{LexicalTokenRow, NukeRow, TableId},
    scope::CaptureMode,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Nukes,
    TableId::LexicalTokens,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A holder slot's stockpile as the top bar shows it: each type's whole amount by ordinal,
/// ascending, and their total, unknown for a hole in the slot's native array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stock {
    pub counts: Vec<NukeCount>,
    pub total: Option<i32>,
}

impl Stock {
    fn heap(&self) -> usize {
        size_of::<Self>() + self.counts.capacity() * size_of::<NukeCount>()
    }
}

/// The tags the complete Countries and CountryLinks resolve, as the row census resolves them:
/// each tag's first holder by id the links resolve.
#[derive(Debug, PartialEq, Eq)]
pub struct Countries {
    /// Each UTF-8 tag's canonical slot and id, by tag.
    pub tags: Vec<(Arc<str>, (u32, u32))>,
    /// Resolved tags that are not UTF-8.
    pub unreadable: usize,
}

impl Countries {
    fn heap(&self) -> usize {
        self.tags.capacity() * size_of::<(Arc<str>, (u32, u32))>()
            + self.tags.iter().map(|(tag, _)| tag.len()).sum::<usize>()
    }
}

/// A read's holder stockpiles: every slot holding rows (the others hold none), or the slots
/// whose rows changed since their last read. None where a slot holds no row, or two of one type.
pub struct Stocks {
    pub whole: bool,
    pub slots: Vec<(u32, Option<Arc<Stock>>)>,
}

/// What a read counts, as the row census counts it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Census {
    /// Rows no link resolves, and each row repeating its holder's type.
    pub unreadable: usize,
    /// Rows of each ordinal no native name maps.
    pub refused: Vec<(u32, usize)>,
    /// The ordinals the rows hold, ascending.
    pub types: Vec<u32>,
    /// Every row and tag resolves, and no slot a tag holds holds a nonzero or unknown stockpile.
    pub empty: bool,
}

impl Census {
    fn heap(&self) -> usize {
        self.refused.capacity() * size_of::<(u32, usize)>() + self.types.capacity() * 4
    }
}

/// What the ordered emit reads of an hour that changes what the stockpile reads.
pub struct Facts {
    /// Whether the Nukes table holds every row, and why not, where its status changed.
    pub complete: bool,
    pub reason: Option<String>,
    /// The countries, at a whole hour or where they changed: none while Countries or
    /// CountryLinks is not complete.
    pub countries: Option<Option<Arc<Countries>>>,
    /// Whether this hour's item is the first to carry `countries`, which it is charged for.
    pub charged: bool,
    /// The stockpiles read, at a read of a complete table while the countries are complete.
    pub stocks: Option<Stocks>,
    /// The read's counts where they changed.
    pub census: Option<Census>,
    /// Each native type's spelling, once a segment.
    pub tokens: Vec<(u32, String)>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the Nukes table was read (`slot_read`).
    pub read: bool,
    /// None when the hour changes nothing the stockpile reads.
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        // The projector and every hour until the countries change share them.
        let countries = match &facts.countries {
            Some(Some(countries)) if facts.charged => size_of::<Countries>() + countries.heap(),
            _ => 0,
        };
        size_of::<Facts>()
            + countries
            + facts.reason.as_ref().map_or(0, String::capacity)
            + facts.stocks.as_ref().map_or(0, |stocks| {
                stocks.slots.capacity() * size_of::<(u32, Option<Arc<Stock>>)>()
                    + stocks
                        .slots
                        .iter()
                        .flat_map(|(_, stock)| stock)
                        .map(|stock| stock.heap())
                        .sum::<usize>()
            })
            + facts.census.as_ref().map_or(0, Census::heap)
            + facts.tokens.capacity() * size_of::<(u32, String)>()
            + facts
                .tokens
                .iter()
                .map(|(_, spelling)| spelling.capacity())
                .sum::<usize>()
    }
}

/// A Nukes row: its holder slot, none where no link resolves its country, and its ordinal.
#[derive(Clone, Copy)]
struct Row {
    holder: Option<u32>,
    ordinal: u32,
    /// Its position among its holder's rows.
    at: usize,
}

/// The Nukes rows by holder slot and by ordinal, kept from the hour's changes.
#[derive(Default)]
struct Rows {
    rows: Vec<Option<Row>>,
    holders: HashMap<u32, Vec<u32>>,
    /// Rows no link resolves.
    unlinked: usize,
    /// Rows of each ordinal.
    ordinals: BTreeMap<u32, usize>,
}

impl Rows {
    fn clear(&mut self) {
        self.rows.clear();
        self.holders.clear();
        self.unlinked = 0;
        self.ordinals.clear();
    }

    /// Forgets the row at `slot`; its holder, when a link resolves it.
    fn remove(&mut self, slot: u32) -> Option<u32> {
        let row = self.rows.get_mut(slot as usize).and_then(Option::take)?;
        match self.ordinals.get_mut(&row.ordinal) {
            Some(count) if *count > 1 => *count -= 1,
            _ => {
                self.ordinals.remove(&row.ordinal);
            }
        }
        let Some(holder) = row.holder else {
            self.unlinked -= 1;
            return None;
        };
        let own = self.holders.get_mut(&holder).expect("indexed holder rows");
        own.swap_remove(row.at);
        if let Some(&moved) = own.get(row.at)
            && let Some(Some(other)) = self.rows.get_mut(moved as usize)
        {
            other.at = row.at;
        }
        if own.is_empty() {
            self.holders.remove(&holder);
        }
        Some(holder)
    }

    fn insert(&mut self, slot: u32, holder: Option<u32>, ordinal: u32) {
        self.remove(slot);
        let index = slot as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        *self.ordinals.entry(ordinal).or_default() += 1;
        let at = match holder {
            Some(holder) => {
                let own = self.holders.entry(holder).or_default();
                own.push(slot);
                own.len() - 1
            }
            None => {
                self.unlinked += 1;
                0
            }
        };
        self.rows[index] = Some(Row {
            holder,
            ordinal,
            at,
        });
    }

    fn holder(&self, slot: u32) -> Option<Option<u32>> {
        self.rows
            .get(slot as usize)
            .copied()
            .flatten()
            .map(|row| row.holder)
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<Option<Row>>()
            + self.holders.capacity() * size_of::<(u32, Vec<u32>)>()
            + self
                .holders
                .values()
                .map(|rows| rows.capacity() * 4)
                .sum::<usize>()
            + self.ordinals.len() * size_of::<(u32, usize)>()
    }
}

/// A holder slot's last read: its stockpile, its rows repeating a type, and whether it is a
/// whole stockpile of nothing.
struct Read {
    stock: Option<Arc<Stock>>,
    repeated: usize,
    zero: bool,
}

/// The countries the walk's roster resolves, the slots of every resolved tag, and whether every
/// tag resolves and is UTF-8.
fn countries(roster: &Roster) -> (Countries, HashSet<u32>, bool) {
    let mut resolved = Countries {
        tags: Vec::new(),
        unreadable: 0,
    };
    let mut slots = HashSet::new();
    let mut valid = true;
    for tag in roster.held() {
        let Tagged::Found(country) = roster.tagged(tag) else {
            valid = false;
            continue;
        };
        slots.insert(country.slot);
        match std::str::from_utf8(roster.tag(tag)) {
            Ok(text) => resolved
                .tags
                .push((Arc::from(text), (country.slot, country.id))),
            Err(_) => {
                resolved.unreadable += 1;
                valid = false;
            }
        }
    }
    resolved
        .tags
        .sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    (resolved, slots, valid)
}

/// The reason the row decoder gives a Nukes table that is not complete.
fn reason(state: &State, status: columns::Status, mode: CaptureMode) -> io::Result<String> {
    let table = TableId::Nukes;
    Ok(match status {
        columns::Status::Unavailable { reason } => {
            let reason = state
                .dictionary()
                .get(reason)
                .ok_or_else(|| io::Error::other("A Nukes reason names no dictionary entry"))?;
            String::from_utf8_lossy(reason).into_owned()
        }
        columns::Status::Truncated { missing } => {
            format!("{table:?} is truncated ({missing} missing rows)")
        }
        columns::Status::NotRecorded => format!("{table:?} is not recorded in a {mode} recording"),
        columns::Status::Complete => {
            return Err(io::Error::other("A complete Nukes table has no reason"));
        }
    })
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    amount: usize,
    literal: usize,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    status: Option<columns::Status>,
    /// Whether Countries and CountryLinks were complete.
    roster: bool,
    countries: Arc<Countries>,
    /// Whether no item carried `countries` yet.
    uncharged: bool,
    /// The slots of every resolved tag, and whether every tag resolves and is UTF-8.
    slots: HashSet<u32>,
    valid: bool,
    rows: Rows,
    /// Holder slots whose rows changed since their last read.
    dirty: HashSet<u32>,
    /// The next read reads every slot.
    stale: bool,
    reads: HashMap<u32, Read>,
    /// The reads' repeated rows, and the slots a tag holds whose stockpile is not nothing.
    repeated: usize,
    nonzero: usize,
    /// The last counts sent, none in this segment yet, and whether they may have changed.
    census: Option<Census>,
    recount: bool,
    /// Each native type's spelling sent in this segment.
    spelled: [bool; 2],
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"nukes projector: cold";

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        Ok(Self {
            format,
            mode,
            amount: columns::field(TableId::Nukes, "amount_raw")?,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            cold: true,
            last: None,
            status: None,
            roster: false,
            countries: Arc::new(Countries {
                tags: Vec::new(),
                unreadable: 0,
            }),
            uncharged: true,
            slots: HashSet::new(),
            valid: false,
            rows: Rows::default(),
            dirty: HashSet::new(),
            stale: true,
            reads: HashMap::new(),
            repeated: 0,
            nonzero: 0,
            census: None,
            recount: true,
            spelled: [false; 2],
        })
    }

    /// The raw country and ordinal of the Nukes row at `slot`.
    fn key(state: &State, slot: u32) -> io::Result<(u32, u32)> {
        let key = columns::table(state, TableId::Nukes)?
            .key(slot)
            .ok_or_else(|| io::Error::other("A Nukes slot holds no row"))?;
        Ok(((key >> 32) as u32, key as u32))
    }

    fn amount(&self, state: &State, slot: u32) -> io::Result<i64> {
        let held = columns::table(state, TableId::Nukes)?;
        if !held.recorded(self.amount, slot) {
            let row: NukeRow = columns::row(state, TableId::Nukes, slot, self.format.version())?;
            return Ok(row.amount_raw);
        }
        Ok(columns::plain::<i64>(held, self.amount)?[slot as usize])
    }

    /// The Nukes rows by holder: read whole, or by the hour's changes.
    fn index(&mut self, state: &State, roster: &Roster, whole: bool) -> io::Result<()> {
        let table = TableId::Nukes;
        self.recount = true;
        if whole {
            self.rows.clear();
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            for slot in live {
                let (country, ordinal) = Self::key(state, slot)?;
                self.rows.insert(slot, roster.slot_of(country), ordinal);
            }
            return Ok(());
        }
        let edits = columns::changes(state, table)?;
        let amount = 1_u64 << self.amount;
        for &slot in &edits.deleted {
            if let Some(holder) = self.rows.remove(slot) {
                self.dirty.insert(holder);
            }
        }
        for &slot in &edits.inserted {
            let (country, ordinal) = Self::key(state, slot)?;
            let holder = roster.slot_of(country);
            self.rows.insert(slot, holder, ordinal);
            self.dirty.extend(holder);
        }
        let written = edits
            .changed
            .iter()
            .zip(&edits.masks)
            .chain(edits.fill_slots.iter().zip(&edits.fill_masks));
        for (&slot, &mask) in written {
            if mask & amount != 0
                && let Some(Some(holder)) = self.rows.holder(slot)
            {
                self.dirty.insert(holder);
            }
        }
        Ok(())
    }

    /// A holder's read: its stockpile from its `rows`, as `counts_by_slot` and `stockpile` read
    /// it.
    fn read(&self, state: &State, rows: &[u32]) -> io::Result<Read> {
        let mut held = Vec::with_capacity(rows.len());
        for &row in rows {
            let ordinal = self.rows.rows[row as usize]
                .ok_or_else(|| io::Error::other("A holder's Nukes row is not indexed"))?
                .ordinal;
            held.push((ordinal, self.amount(state, row)?));
        }
        held.sort_unstable_by_key(|&(ordinal, _)| ordinal);
        let mut counts = Vec::with_capacity(held.len());
        let mut repeated = 0;
        for pair in held.chunk_by(|left, right| left.0 == right.0) {
            repeated += pair.len() - 1;
            counts.push(NukeCount {
                ordinal: pair[0].0,
                value: displayed_amount(pair[0].1),
            });
        }
        if repeated != 0 {
            return Ok(Read {
                stock: None,
                repeated,
                zero: false,
            });
        }
        // Only a hole inside the slot's own array prefix makes its total unknown.
        let total = counts
            .iter()
            .enumerate()
            .all(|(index, count)| usize::try_from(count.ordinal) == Ok(index))
            .then(|| {
                counts
                    .iter()
                    .fold(0i32, |total, count| total.wrapping_add(count.value))
            });
        Ok(Read {
            zero: total == Some(0),
            stock: Some(Arc::new(Stock { counts, total })),
            repeated: 0,
        })
    }

    /// Forgets `slot`'s last read, returning it.
    fn unread(&mut self, slot: u32) -> Option<Read> {
        let read = self.reads.remove(&slot)?;
        self.repeated -= read.repeated;
        self.nonzero -= usize::from(!read.zero && self.slots.contains(&slot));
        Some(read)
    }

    /// The stockpiles a read reads: every slot's, or those that changed since their last.
    fn stocks(&mut self, state: &State) -> io::Result<Stocks> {
        let _cpu = replay_frame::cpu::scope("nukes.columnar.stocks", None);
        let whole = std::mem::take(&mut self.stale);
        let mut slots: Vec<u32> = if whole {
            self.recount = true;
            self.reads.clear();
            self.repeated = 0;
            self.nonzero = 0;
            self.dirty.clear();
            self.rows.holders.keys().copied().collect()
        } else {
            self.dirty.drain().collect()
        };
        slots.sort_unstable();
        let mut read = Vec::with_capacity(slots.len());
        for slot in slots {
            self.recount = true;
            let held = self.unread(slot);
            let Some(rows) = self.rows.holders.get(&slot) else {
                if held.is_some() {
                    read.push((slot, None));
                }
                continue;
            };
            let slot_read = self.read(state, rows)?;
            self.repeated += slot_read.repeated;
            self.nonzero += usize::from(!slot_read.zero && self.slots.contains(&slot));
            // The emit carries a stockpile its slot's rows left as it was.
            if held.is_none_or(|held| held.stock != slot_read.stock) {
                read.push((slot, slot_read.stock.clone()));
            }
            self.reads.insert(slot, slot_read);
        }
        Ok(Stocks { whole, slots: read })
    }

    /// The read's counts, where they may have changed since the last sent.
    fn census(&mut self) -> Option<Census> {
        if !std::mem::take(&mut self.recount) && self.census.is_some() {
            return None;
        }
        let census = Census {
            unreadable: self.rows.unlinked + self.repeated,
            refused: self
                .rows
                .ordinals
                .iter()
                .filter(|&(&ordinal, _)| name_token(ordinal).is_none())
                .map(|(&ordinal, &count)| (ordinal, count))
                .collect(),
            types: self.rows.ordinals.keys().copied().collect(),
            empty: self.rows.unlinked == 0 && self.valid && self.nonzero == 0,
        };
        if self.census.as_ref() == Some(&census) {
            return None;
        }
        self.census = Some(census.clone());
        Some(census)
    }

    /// Each native type's spelling LexicalTokens holds, not sent in this segment yet.
    fn tokens(&mut self, state: &State) -> io::Result<Vec<(u32, String)>> {
        let table = TableId::LexicalTokens;
        let held = columns::table(state, table)?;
        let mut tokens = Vec::new();
        for ordinal in 0..2 {
            if self.spelled[ordinal as usize] {
                continue;
            }
            let token = name_token(ordinal).expect("native mapping");
            let Some(slot) = held.slot(u128::from(token)) else {
                continue;
            };
            let literal = if held.recorded(self.literal, slot) {
                let text = *columns::text(held, self.literal)?
                    .get(slot as usize)
                    .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
                state
                    .dictionary()
                    .get(text)
                    .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                    .to_vec()
            } else {
                columns::row::<LexicalTokenRow>(state, table, slot, self.format.version())?.literal
            };
            if let Ok(text) = String::from_utf8(literal) {
                tokens.push((ordinal, text));
                self.spelled[ordinal as usize] = true;
            }
        }
        Ok(tokens)
    }

    fn heap(&self) -> usize {
        size_of::<Countries>()
            + self.countries.heap()
            + self.slots.capacity() * 4
            + self.rows.heap()
            + self.dirty.capacity() * 4
            + self.reads.capacity() * size_of::<(u32, Read)>()
            + self
                .reads
                .values()
                .flat_map(|read| &read.stock)
                .map(|stock| stock.heap())
                .sum::<usize>()
            + self.census.as_ref().map_or(0, Census::heap)
    }
}

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

// Every segment starts cold: its first hour reads every holder and country again; what came
// before is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("nukes.columnar", Some("nukes"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.spelled = [false; 2];
            self.census = None;
        }
        self.cold = false;
        self.last = Some(at);
        let status = columns::status(state, TableId::Nukes);
        let complete = status == columns::Status::Complete;
        let shaped = opens || self.status != Some(status);
        self.status = Some(status);
        let complete_roster = roster.complete();
        let regrouped = opens || roster.rebuilt() || !roster.changed_links().is_empty();
        let became = opens || complete_roster != self.roster;
        self.roster = complete_roster;
        let mut countries = None;
        if became || roster.changed() {
            let (resolved, slots, valid) = self::countries(roster);
            self.slots = slots;
            self.valid = valid;
            if !complete_roster {
                if became {
                    countries = Some(None);
                }
            } else if became || resolved != *self.countries {
                self.countries = Arc::new(resolved);
                self.uncharged = true;
                countries = Some(Some(Arc::clone(&self.countries)));
            }
            // The slots the tags hold count anew, and a tag moved reads its new slot.
            self.stale = true;
        }
        if !complete {
            self.rows.clear();
            self.dirty.clear();
            self.stale = true;
        } else if shaped || regrouped {
            self.index(state, roster, true)?;
            self.stale = true;
        } else if touched(state, TableId::Nukes)? {
            self.index(state, roster, false)?;
        }
        if !complete_roster {
            self.stale = true;
        }
        let flags = metrics(hour.manifest);
        let read = dashboard_text::metadata::slot_read(self.format, &flags, TableId::Nukes, at);
        let reason = (shaped && !complete)
            .then(|| reason(state, status, self.mode))
            .transpose()?;
        let (stocks, census) = if read && complete && complete_roster {
            let stocks = self.stocks(state)?;
            let census = self.census();
            (
                (stocks.whole || !stocks.slots.is_empty()).then_some(stocks),
                census,
            )
        } else {
            (None, None)
        };
        let lexical = TableId::LexicalTokens;
        let tokens = if self.spelled != [true; 2]
            && columns::complete(state, lexical)
            && (opens || hour.world.restated(lexical) || touched(state, lexical)?)
        {
            self.tokens(state)?
        } else {
            Vec::new()
        };
        let quiet = !shaped
            && countries.is_none()
            && stocks.is_none()
            && census.is_none()
            && tokens.is_empty();
        let facts = (!quiet).then(|| {
            let charged = matches!(countries, Some(Some(_))) && std::mem::take(&mut self.uncharged);
            Box::new(Facts {
                complete,
                reason,
                countries,
                charged,
                stocks,
                census,
                tokens,
            })
        });
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            read,
            facts,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved nukes projector",
            ));
        }
        *self = Self::new(self.format, self.mode)?;
        Ok(())
    }
}
