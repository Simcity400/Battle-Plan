//! One table's selected cells kept at the playback head of one run. A seek restores them at the
//! table's last restore before the hour and replays the parts since; afterwards the lane asks for
//! the parts it needs as a [`Fetch`] of plain data that runs on any thread, and is given what it
//! read ([`Lane::give`]). A step never touches the file: it advances the cells over the parts
//! given, one cursor a part, and fills one owned change set ([`Changes`]). A part passed leaves
//! with its cursor ([`Lane::retire_into`]) to be freed on another thread.

use crate::{
    changes::{Changes, Passing, Texts, Written},
    invalid,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{TableState, Value},
    unit::{
        BLOCK,
        reader::{self, Advanced, Cursor, Lives, LogCursor, Part, Prefix, Spent},
    },
};
use replay_data::Hour;
use std::{
    collections::{HashMap, VecDeque},
    io,
    sync::Arc,
    time::Instant,
};

pub mod log;
#[cfg(test)]
pub(crate) mod tests;

/// Bytes the reader holds per open track: the track and its next hour.
const TRACK_BYTES: usize = 34;

/// Every string a lane named since its seek, by id: what a reader rebuilt from the lane in memory
/// names, where [`Lane::texts`] holds only those since the last change set.
#[derive(Default)]
pub(crate) struct Known(HashMap<u32, Box<[u8]>>);

impl Known {
    pub(crate) fn note(&mut self, id: u32, text: &[u8]) {
        if self.0.get(&id).is_none_or(|held| **held != *text) {
            self.0.insert(id, text.into());
        }
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    fn texts(&self) -> Texts {
        let mut texts = Texts::default();
        for (&id, text) in &self.0 {
            texts.push(id, text);
        }
        texts
    }

    fn bytes(&self) -> usize {
        self.0.iter().map(|(_, text)| text.len() + 24).sum()
    }
}

/// A part's cursor once the head has passed it, to be freed on another thread.
pub enum Passed {
    State(Cursor),
    Log(LogCursor),
}

/// A part fetched for a lane: its block and index there.
pub struct Given {
    block: usize,
    index: usize,
    part: Part,
}

/// Where a lane's fetching stands: the next part to read, and the streams of the newest part read
/// while the next one may be chained to them.
#[derive(Default)]
pub struct Chain {
    next: Option<(usize, usize)>,
    prefix: Option<(usize, Prefix)>,
}

/// A lane's next read, as plain data: the parts to fetch and what the first is chained to.
pub struct Fetch {
    pages: Arc<dyn Pages>,
    run: usize,
    table: usize,
    columns: Arc<[usize]>,
    chain: Chain,
    through: Hour,
}

/// What a [`Fetch`] read.
pub struct Fetched {
    given: Vec<Given>,
    chain: Chain,
    pub tracks: usize,
    pub stored: usize,
    pub decompressed: usize,
    pub ns: u64,
}

impl Fetched {
    pub fn parts(&self) -> usize {
        self.given.len()
    }
}

impl Fetch {
    pub fn table(&self) -> usize {
        self.table
    }

    /// The game hour the first part it reads starts at.
    pub fn first_hour(&self) -> Hour {
        let run = &self.pages.runs()[self.run];
        self.chain.next.map_or(Hour::MAX, |(block, index)| part_hour(run, self.table, block, index))
    }

    /// Reads, verifies, decompresses and opens the next part, then those after it that start by
    /// hour `through`, each chained to the one before it in its block.
    pub fn run(self) -> io::Result<Fetched> {
        let Self { pages, run, table, columns, mut chain, through } = self;
        let held = &pages.runs()[run];
        let started = Instant::now();
        let (mut given, mut tracks, mut stored, mut decompressed) = (Vec::new(), 0, 0, 0);
        while let Some((block, index)) = chain.next {
            if !given.is_empty() && part_hour(held, table, block, index) > through {
                break;
            }
            let page = &held.pages[block];
            let parts = &page.tables[table].parts;
            let last = parts.get(index + 1).map_or(page.hours - 1, |following| following.first - 1);
            let previous = chain.prefix.as_ref().filter(|(at, _)| *at == block && index > 0).map(|(_, prefix)| prefix);
            let part = reader::fetch(pages.extents(run), &parts[index], last, &columns, previous)?;
            tracks += part.track_count();
            stored += part.stored;
            decompressed += part.decompressed;
            chain.next = next_part(held, table, block, index + 1);
            chain.prefix = chain.next.filter(|&(next, _)| next == block).map(|_| (block, part.prefix()));
            given.push(Given { block, index, part });
        }
        Ok(Fetched { given, chain, tracks, stored, decompressed, ns: started.elapsed().as_nanos() as u64 })
    }
}

fn part_hour(run: &Run, table: usize, block: usize, index: usize) -> Hour {
    run.first_hour + (block as u32 * BLOCK + u32::from(run.pages[block].tables[table].parts[index].first)) as Hour
}

/// The first part at or after part `index` of block `block`.
fn next_part(run: &Run, table: usize, mut block: usize, mut index: usize) -> Option<(usize, usize)> {
    while let Some(page) = run.pages.get(block) {
        if index < page.tables[table].parts.len() {
            return Some((block, index));
        }
        block += 1;
        index = 0;
    }
    None
}

/// What a lane holds, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub cells: usize,
    /// Parts given and not let go: their count and bytes decompressed.
    pub parts: usize,
    pub part_bytes: usize,
    /// Their open tracks, and the reader's bytes for them.
    pub tracks: usize,
    pub track_bytes: usize,
}

impl Held {
    pub fn add(&mut self, other: &Self) {
        self.cells += other.cells;
        self.parts += other.parts;
        self.part_bytes += other.part_bytes;
        self.tracks += other.tracks;
        self.track_bytes += other.track_bytes;
    }

    pub fn total(&self) -> usize {
        self.cells + self.part_bytes + self.track_bytes
    }
}

/// What a seek took.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sought {
    pub restore_ns: u64,
    /// Fetching the parts since the restore, and advancing over them.
    pub fetch_ns: u64,
    pub replay_ns: u64,
    pub parts: u64,
    /// The block whose end it restored (none for the run's first restore), and blocks replayed.
    pub restored: Option<usize>,
    pub blocks: u32,
    /// The reader's own count of what it replayed (counted while the lane is timed).
    pub spent: Spent,
}

/// The part under the head: its cursor, and the strings it names first, given once it is entered.
struct Under {
    cursor: Cursor,
    block: usize,
    last: u16,
    bytes: usize,
    tracks: usize,
    texts: Vec<u32>,
}

/// A passing table's rows born and gone inside the step under way, each handed over by the reader
/// as it goes ([`Cursor::advance_seeing`]) with its cells at its last live hour. A bit a slot says
/// which rows were live at the step's start, so a row older than the step costs one test when it
/// goes; only the rows that pass are copied. It holds a bit a slot and the step's passing rows.
#[derive(Default)]
struct Passage {
    /// Live at the step's start and not gone since, by slot; none until stepped after a seek.
    live: Option<Vec<u64>>,
    /// Each passing row's run hour of delete, slot and key, in delete order.
    gone: Vec<(u32, u32, u128)>,
    /// Per selected column, the passing rows' cells in delete order.
    staged: Vec<Written>,
    failed: Option<io::Error>,
    /// The step's inserts and its passing rows, each in slot order, and the pairs (insert, row).
    inserts: Vec<u32>,
    by_slot: Vec<u32>,
    pairs: Vec<(u32, u32)>,
}

impl Passage {
    fn begin(&mut self, cells: &TableState, columns: &[usize]) {
        if self.live.is_none() {
            self.live = Some(cells.live().words().to_vec());
        }
        self.staged.resize_with(columns.len(), Written::default);
        for (staged, &column) in self.staged.iter_mut().zip(columns) {
            staged.start(column, cells.column(column).expect("a selected column"), true);
        }
        self.gone.clear();
    }

    /// Row `key` at `slot`, deleted at run hour `hour`: kept unless it was live at the step's
    /// start.
    fn see(&mut self, hour: u32, slot: u32, key: u128, cells: &TableState, columns: &[usize]) {
        let live = self.live.as_mut().expect("a step begun");
        if let Some(word) = live.get_mut(slot as usize / 64) {
            let bit = 1_u64 << (slot % 64);
            if *word & bit != 0 {
                *word &= !bit;
                return;
            }
        }
        let row = self.gone.len() as u32;
        self.gone.push((hour, slot, key));
        for (staged, &column) in self.staged.iter_mut().zip(columns) {
            if let Err(error) = staged.push(row, cells.value(column, slot)) {
                self.failed.get_or_insert(error);
            }
        }
    }

    /// The rows gone, in birth order, into `out`, each paired with its insert among the step's
    /// (run hour, slot, key) `inserts`: the k-th insert at a slot is the k-th passing row there.
    /// The bits are brought to the step's end over the slots its lives name.
    #[expect(clippy::too_many_arguments, reason = "the lane's fields, borrowed apart")]
    fn write(&mut self, out: &mut Passing, keyed: bool, cells: &TableState, columns: &[usize], inserts: &[(u32, u32, u128)], deletes: &[(u32, u32, u128)], game: impl Fn(u32) -> Hour) -> io::Result<()> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let live = self.live.as_mut().expect("a step begun");
        let words = cells.live().words();
        if live.len() < words.len() {
            live.resize(words.len(), 0);
        }
        for &(_, slot, _) in deletes.iter().chain(inserts) {
            let (word, bit) = (slot as usize / 64, 1_u64 << (slot % 64));
            live[word] = (live[word] & !bit) | (words.get(word).copied().unwrap_or(0) & bit);
        }
        self.pairs.clear();
        if !self.gone.is_empty() {
            let gone = &self.gone;
            self.inserts.clear();
            self.inserts.extend(0..inserts.len() as u32);
            self.inserts.sort_by_key(|&at| inserts[at as usize].1);
            self.by_slot.clear();
            self.by_slot.extend(0..gone.len() as u32);
            self.by_slot.sort_by_key(|&at| gone[at as usize].1);
            let mut next = 0;
            for &row in &self.by_slot {
                let (hour, slot, key) = gone[row as usize];
                while next < self.inserts.len() && inserts[self.inserts[next] as usize].1 < slot {
                    next += 1;
                }
                let insert = *self.inserts.get(next).filter(|&&at| inserts[at as usize].1 == slot).ok_or_else(|| invalid(format!("A row gone from slot {slot} at run hour {hour} was neither live at its step's start nor inserted in it")))?;
                let (born, _, inserted) = inserts[insert as usize];
                if inserted != key || born >= hour {
                    return Err(invalid(format!("Slot {slot}'s row gone at run hour {hour} is not the one inserted there at {born}")));
                }
                self.pairs.push((insert, row));
                next += 1;
            }
            self.pairs.sort_unstable_by_key(|&(insert, _)| insert);
        }
        out.keyed = keyed;
        out.columns.resize_with(columns.len(), Written::default);
        for ((written, &column), staged) in out.columns.iter_mut().zip(columns).zip(&self.staged) {
            written.start(column, cells.column(column).expect("a selected column"), false);
            for &(_, row) in &self.pairs {
                match staged.value(row as usize) {
                    Value::Number(bits) => written.fit(bits),
                    Value::Text(id) => written.fit(u64::from(id)),
                    _ => {}
                }
            }
            for (place, &(_, row)) in self.pairs.iter().enumerate() {
                written.push(place as u32, staged.value(row as usize))?;
            }
        }
        for &(insert, row) in &self.pairs {
            let (hour, _, key) = self.gone[row as usize];
            out.born.push(game(inserts[insert as usize].0));
            out.last.push(game(hour) - 1);
            if keyed {
                out.keys.push(key);
            }
        }
        Ok(())
    }

    fn bytes(&self) -> usize {
        self.live.as_ref().map_or(0, |live| live.capacity() * 8)
    }
}

/// Why advancing stopped short of its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Reached,
    /// A part is not given yet.
    Waiting,
    /// A packed block ended: the cells are renumbered before anything else.
    Packed,
}

/// One table's selected cells at the head of one run.
pub struct Lane {
    pages: Arc<dyn Pages>,
    run: usize,
    table: usize,
    kinds: Vec<Kind>,
    columns: Arc<[usize]>,
    keyed: bool,
    cells: TableState,
    under: Option<Under>,
    /// Parts given and not under the head yet, in order.
    ahead: VecDeque<Given>,
    /// The next run hour to apply; none before the first seek.
    next: Option<u32>,
    /// None before the first seek and while a fetch is out.
    chain: Option<Chain>,
    retired: Vec<Passed>,
    /// Strings named since the last change set.
    texts: Texts,
    known: Known,
    status: (Status, Option<Hour>),
    /// The run hour the status must be read again at, and whether its sample hour is each hour.
    status_until: u32,
    status_moving: bool,
    timed: bool,
    spent: Spent,
    written: Vec<Vec<u32>>,
    lives: Lives,
    deletes: Vec<(u32, u32, u128)>,
    inserts: Vec<(u32, u32, u128)>,
    /// Cursor calls the step took: more than one may name a slot twice.
    pieces: u32,
    marks: Vec<u64>,
    passage: Option<Box<Passage>>,
    /// A log table's state at the head and the part under it: such a lane holds no cell.
    log: Option<Box<log::Logs>>,
    /// Restores and cursors of a table holding state it opened.
    #[cfg(test)]
    pub(crate) state_reads: usize,
}

impl Lane {
    /// A lane of `columns` (indexes into the table's kinds) of `table` in run `run`, with the
    /// rows' keys in its change sets when `keyed`; it holds nothing until sought. A log table's
    /// lane reads it as a log ([`Lane::log`], [`Changes::log`]).
    pub fn new(pages: Arc<dyn Pages>, run: usize, table: usize, columns: &[usize], keyed: bool) -> io::Result<Self> {
        let held_run = pages.runs().get(run).ok_or_else(|| invalid(format!("No run {run}")))?;
        let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("No table {table} in the schema")))?.to_vec();
        if held_run.pages.first().is_none_or(|page| page.tables.len() <= table) {
            return Err(invalid(format!("Run {run} holds no table {table}")));
        }
        for (at, &column) in columns.iter().enumerate() {
            if column >= kinds.len() || columns[..at].contains(&column) {
                return Err(invalid(format!("Column {column} of table {table} is selected twice or not in it")));
            }
        }
        let log = reader::class_of(&held_run.pages, table)?.then(Box::default);
        Ok(Self {
            pages,
            run,
            table,
            cells: TableState::new(&kinds),
            kinds,
            columns: columns.into(),
            keyed,
            under: None,
            ahead: VecDeque::new(),
            next: None,
            chain: None,
            retired: Vec::new(),
            texts: Texts::default(),
            known: Known::default(),
            status: (Status::NotRecorded, None),
            status_until: 0,
            status_moving: false,
            timed: false,
            spent: Spent::default(),
            written: vec![Vec::new(); columns.len()],
            lives: Lives::default(),
            deletes: Vec::new(),
            inserts: Vec::new(),
            pieces: 0,
            marks: Vec::new(),
            passage: None,
            log,
            #[cfg(test)]
            state_reads: 0,
        })
    }

    pub fn table(&self) -> usize {
        self.table
    }

    pub fn run(&self) -> usize {
        self.run
    }

    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    pub fn keyed(&self) -> bool {
        self.keyed
    }

    /// Its change sets hold the rows born and gone inside a step ([`Changes::passing`]).
    pub fn set_passing(&mut self, passing: bool) {
        self.passage = passing.then(Box::default);
    }

    pub fn passing(&self) -> bool {
        self.passage.is_some()
    }

    /// What it was declared to read: its columns, whether keyed, and whether passing.
    pub fn declared(&self) -> (&[usize], bool, bool) {
        (self.columns(), self.keyed(), self.passing())
    }

    /// The selected cells, live bits and keys at the head. Its own status is [`Self::status`].
    pub fn cells(&self) -> &TableState {
        &self.cells
    }

    pub fn status(&self) -> (Status, Option<Hour>) {
        self.status
    }

    /// The text of the head's refusal reason, while the part under the head or the seek's restore
    /// names it: a part names its refusal's reason as it names a text cell's.
    pub fn reason(&self) -> Option<&[u8]> {
        let Status::Unavailable { reason } = self.status.0 else { return None };
        self.under
            .as_ref()
            .and_then(|under| under.cursor.text(reason))
            .or_else(|| self.texts.iter().find(|&(id, _)| id == reason).map(|(_, text)| text))
    }

    /// The last game hour applied.
    pub fn head(&self) -> Option<Hour> {
        self.next.map(|next| self.game(next) - 1)
    }

    /// Times the reader's work by kind ([`Self::spent`]).
    pub fn time(&mut self, timed: bool) {
        self.timed = timed;
        if let Some(under) = &mut self.under {
            under.cursor.time(timed);
        }
    }

    /// The reader's own time and counts, over the lane's life.
    pub fn spent(&self) -> Spent {
        let mut spent = self.spent;
        if let Some(under) = &self.under {
            add(&mut spent, &under.cursor.spent);
        }
        spent
    }

    pub fn held(&self) -> Held {
        let mut held = Held { cells: self.cells.bytes().total() + self.passage.as_ref().map_or(0, |passage| passage.bytes()) + self.known.bytes(), ..Held::default() };
        if let Some(logs) = &self.log {
            held.cells += logs.state.bytes();
            if let Some(under) = &logs.under {
                held.parts += 1;
                held.part_bytes += under.bytes;
                held.tracks += under.tracks;
            }
        }
        if let Some(under) = &self.under {
            held.parts += 1;
            held.part_bytes += under.bytes;
            held.tracks += under.tracks;
        }
        for given in &self.ahead {
            held.parts += 1;
            held.part_bytes += given.part.decompressed;
            held.tracks += given.part.track_count();
        }
        held.track_bytes = held.tracks * TRACK_BYTES;
        held
    }

    fn game(&self, run_hour: u32) -> Hour {
        self.pages.runs()[self.run].first_hour + run_hour as Hour
    }

    /// The parts it needs read: those starting by game hour `through`, and at least the one after
    /// the last given. None while a fetch is out, before a seek, or when nothing is needed.
    pub fn want(&mut self, through: Hour) -> Option<Fetch> {
        let (block, index) = self.chain.as_ref()?.next?;
        if !self.ahead.is_empty() && part_hour(&self.pages.runs()[self.run], self.table, block, index) > through {
            return None;
        }
        let chain = self.chain.take()?;
        Some(Fetch { pages: Arc::clone(&self.pages), run: self.run, table: self.table, columns: Arc::clone(&self.columns), chain, through })
    }

    /// Whether the parts given reach game hour `through`: stepping there waits on no read.
    pub(crate) fn holds_through(&self, through: Hour) -> bool {
        self.chain.as_ref().is_some_and(|chain| chain.next.is_none_or(|(block, index)| part_hour(&self.pages.runs()[self.run], self.table, block, index) > through))
    }

    /// What its [`Fetch`] read.
    pub fn give(&mut self, fetched: Fetched) -> io::Result<()> {
        if self.chain.is_some() {
            return Err(invalid(format!("Table {}'s lane is given parts it did not ask for", self.table)));
        }
        self.ahead.extend(fetched.given);
        self.chain = Some(fetched.chain);
        Ok(())
    }

    /// Fetches on this thread what [`Self::want`] names for `through`; the nanoseconds and parts.
    pub fn fetch_through(&mut self, through: Hour) -> io::Result<(u64, usize)> {
        let (mut ns, mut parts) = (0, 0);
        while let Some(fetch) = self.want(through) {
            let fetched = fetch.run()?;
            ns += fetched.ns;
            parts += fetched.parts();
            self.give(fetched)?;
        }
        Ok((ns, parts))
    }

    /// Cursors of parts passed, for another thread to free.
    pub fn retire_into(&mut self, out: &mut Vec<Passed>) {
        out.append(&mut self.retired);
    }

    /// The cells at the end of game hour `hour` (the run's first restore for an hour before it):
    /// the table's last restore at or before the hour's block, then every part since, fetched in
    /// order and advanced in whole parts. [`Self::reset_into`] gives its change set.
    pub fn seek(&mut self, hour: Hour) -> io::Result<Sought> {
        if self.log.is_some() {
            return self.seek_log(hour);
        }
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let hours = run.hours();
        let target = (i64::from(hour) - i64::from(run.first_hour)).clamp(0, i64::from(hours) - 1) as u32;
        let block = (target / BLOCK) as usize;
        let ends_block = target + 1 == block as u32 * BLOCK + u32::from(run.pages[block].hours);
        let at = if ends_block { Some(block) } else { block.checked_sub(1) };
        let refused = || invalid(format!("No restore of table {} is written", self.table));
        let (restored, entry) = match at {
            Some(at) => reader::restored_before(&run.pages, self.table, at).ok_or_else(refused)?,
            None => (0, run.pages[0].tables[self.table].first.as_ref().ok_or_else(refused)?),
        };
        let from_first = at.is_none() || run.pages[restored].tables[self.table].restore.is_none();
        let started = Instant::now();
        #[cfg(test)]
        {
            self.state_reads += 1;
        }
        let (cells, meta) = reader::restore(pages.extents(self.run), entry, &self.kinds, &self.columns)?;
        let restore_ns = started.elapsed().as_nanos() as u64;
        if let Some(under) = self.under.take() {
            add(&mut self.spent, &under.cursor.spent);
        }
        let before = self.spent;
        self.cells = cells;
        self.ahead.clear();
        self.retired.clear();
        self.texts.clear();
        self.known.clear();
        for (id, text) in &meta.strings {
            self.texts.push(*id, text);
            self.known.note(*id, text);
        }
        self.written.iter_mut().for_each(Vec::clear);
        self.deletes.clear();
        self.inserts.clear();
        if let Some(passage) = &mut self.passage {
            passage.live = None;
        }
        let next = if from_first { 1 } else { restored as u32 * BLOCK + u32::from(run.pages[restored].hours) };
        self.next = Some(next);
        self.chain = Some(Chain { next: next_part(run, self.table, (next / BLOCK) as usize, 0), prefix: None });
        self.status_until = 0;
        let mut sought = Sought { restore_ns, restored: (!from_first).then_some(restored), blocks: (target + 1).div_ceil(BLOCK).saturating_sub(next / BLOCK), ..Sought::default() };
        let replaying = Instant::now();
        let (fetch_ns, parts) = self.fetch_through(self.game(target))?;
        (sought.fetch_ns, sought.parts) = (fetch_ns, parts as u64);
        if target >= next && self.advance(target, false)? != Stop::Reached {
            return Err(invalid(format!("Table {} cannot be replayed to hour {hour}: a part is missing", self.table)));
        }
        self.retired.clear();
        sought.replay_ns = (replaying.elapsed().as_nanos() as u64).saturating_sub(sought.fetch_ns);
        self.read_status()?;
        let mut spent = self.spent();
        subtract(&mut spent, &before);
        sought.spent = spent;
        Ok(sought)
    }

    /// The reset change set at the head: the flag, the head and the status, and no row. Readers
    /// rebuild from the lane itself ([`crate::Rebuild`]).
    pub fn reset_into(&self, out: &mut Changes) -> io::Result<()> {
        if self.log.is_some() {
            return self.reset_log(out);
        }
        let head = self.head().ok_or_else(|| invalid("A lane is reset before it is sought"))?;
        out.begin(self.table, &self.columns, self.keyed, true);
        for (written, &column) in out.columns.iter_mut().zip(self.columns.iter()) {
            written.fill(self.cells.column(column).expect("a selected column"))?;
        }
        out.status = Some(self.status);
        out.set_head(head, 0, false);
        Ok(())
    }

    /// The strings named since the last change set; after a seek, every string named since it,
    /// those of the restore included.
    pub fn texts(&self) -> &Texts {
        &self.texts
    }

    /// Its reset is shown: the strings of the seek are no step's.
    pub(crate) fn clear_texts(&mut self) {
        self.texts.clear();
    }

    /// [`Self::texts`] becomes every string named since the seek, for a reader rebuilt from the lane
    /// in memory; the strings since the last change set are given back to [`Self::hide_known`].
    pub(crate) fn show_known(&mut self) -> Texts {
        std::mem::replace(&mut self.texts, self.known.texts())
    }

    pub(crate) fn hide_known(&mut self, pending: Texts) {
        self.texts = pending;
    }

    /// Advances the cells to the end of game hour `to`, as far as the parts given reach, and fills
    /// `out` with the step's change set; the head reached. A step that reaches a packed block's
    /// end stops there with the renumber.
    pub fn step(&mut self, to: Hour, out: &mut Changes) -> io::Result<Hour> {
        if self.log.is_some() {
            return self.step_log(to, out);
        }
        let Some(next) = self.next else { return Err(invalid("A lane steps before it is sought")) };
        out.begin(self.table, &self.columns, self.keyed, false);
        self.pieces = 0;
        if let Some(passage) = &mut self.passage {
            passage.begin(&self.cells, &self.columns);
        }
        let last = self.pages.runs()[self.run].hours() - 1;
        let target = (i64::from(to) - i64::from(self.game(0))).clamp(0, i64::from(last)) as u32;
        let stop = if target >= next { self.advance(target, true)? } else { Stop::Reached };
        if let Some(passage) = &mut self.passage {
            let first = self.pages.runs()[self.run].first_hour;
            passage.write(&mut out.passing, self.keyed, &self.cells, &self.columns, &self.inserts, &self.deletes, |hour| first + hour as Hour)?;
        }
        self.finish(out)?;
        if stop == Stop::Packed {
            out.set_renumber(self.cells.rows());
            self.cells.pack()?;
            if let Some(passage) = &mut self.passage {
                passage.live = None;
            }
        }
        let head = self.head().expect("sought");
        out.set_head(head, (self.game(target) - head).max(0) as u32, stop == Stop::Waiting);
        Ok(head)
    }

    /// Applies every hour through run hour `target` the parts given reach. With `collect` the
    /// slots written and the lives go to the change set; without, a packed block's end is
    /// renumbered at once.
    fn advance(&mut self, target: u32, collect: bool) -> io::Result<Stop> {
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let mut next = self.next.expect("sought");
        while next <= target {
            let block = (next / BLOCK) as usize;
            let page = &run.pages[block];
            let start = block as u32 * BLOCK;
            let end = target.min(start + u32::from(page.hours) - 1);
            let entry = &page.tables[self.table];
            let hour = (next - start) as u16;
            if self.under.as_ref().is_some_and(|under| under.block != block || under.last < hour) {
                self.pass()?;
            }
            if !entry.parts.is_empty() {
                if self.under.is_none() {
                    let index = entry.parts.partition_point(|part| part.first <= hour).checked_sub(1).ok_or_else(|| invalid(format!("Table {}'s first part in block {block} starts after its hour {hour}", self.table)))?;
                    match self.ahead.front() {
                        None => {
                            self.next = Some(next);
                            return Ok(Stop::Waiting);
                        }
                        Some(given) if (given.block, given.index) != (block, index) => {
                            return Err(invalid(format!("Table {}'s lane holds part {} of block {} where it needs part {index} of block {block}", self.table, given.index, given.block)));
                        }
                        Some(_) => {}
                    }
                    let given = self.ahead.pop_front().expect("held");
                    self.enter(given, hour);
                }
                let under = self.under.as_mut().expect("entered");
                let to = end.min(start + u32::from(under.last));
                let (from, last) = (hour, (to - start) as u16);
                let reached = match self.passage.as_deref_mut().filter(|_| collect) {
                    None => under.cursor.advance(last, last - from + 1, &mut self.cells, &mut self.written, &mut self.lives)?,
                    Some(passage) => {
                        let columns = &self.columns;
                        let mut gone = |hour: u16, slot: u32, key: u128, cells: &TableState| passage.see(start + u32::from(hour), slot, key, cells, columns);
                        under.cursor.advance_seeing(last, last - from + 1, &mut self.cells, &mut self.written, &mut self.lives, &mut gone)?
                    }
                };
                if reached != Advanced::Reached {
                    return Err(invalid(format!("Table {}'s cursor misses hours its part holds", self.table)));
                }
                next = to + 1;
                for id in under.texts.drain(..) {
                    if let Some(text) = under.cursor.text(id) {
                        self.texts.push(id, text);
                        self.known.note(id, text);
                    }
                }
                if last == under.last {
                    self.pass()?;
                }
                if collect {
                    self.pieces += 1;
                    self.deletes.extend(self.lives.deleted.iter().map(|&(hour, slot, key)| (start + u32::from(hour), slot, key)));
                    self.inserts.extend(self.lives.inserted.iter().map(|&(hour, slot, key)| (start + u32::from(hour), slot, key)));
                } else {
                    self.written.iter_mut().for_each(Vec::clear);
                }
                self.lives.deleted.clear();
                self.lives.inserted.clear();
            } else {
                next = end + 1;
            }
            if next == start + u32::from(page.hours) && entry.packed {
                self.next = Some(next);
                if collect {
                    return Ok(Stop::Packed);
                }
                self.cells.pack()?;
            }
        }
        self.next = Some(next);
        Ok(Stop::Reached)
    }

    /// The head moves into a fetched part at block hour `hour`: a cursor of its own.
    fn enter(&mut self, given: Given, hour: u16) {
        #[cfg(test)]
        {
            self.state_reads += 1;
        }
        let mut cursor = Cursor::new(&self.kinds, &self.columns, hour);
        cursor.time(self.timed);
        let texts = given.part.strings().iter().map(|&(id, _)| id).collect();
        let (bytes, tracks, last) = (given.part.decompressed, given.part.track_count(), given.part.last);
        cursor.push(given.part);
        self.under = Some(Under { cursor, block: given.block, last, bytes, tracks, texts });
    }

    /// The part under the head is passed: checked to hold nothing past it, its cursor retired.
    fn pass(&mut self) -> io::Result<()> {
        let mut under = self.under.take().expect("a part under the head");
        under.cursor.advance(under.last + 1, 1, &mut self.cells, &mut self.written, &mut self.lives)?;
        add(&mut self.spent, &under.cursor.spent);
        self.retired.push(Passed::State(under.cursor));
        Ok(())
    }

    /// The step's change set from what the reader wrote: lives netted, each slot once, values.
    /// Within one part a slot is written twice only when a lifetime names it, so only a step that
    /// crosses a part has every slot checked.
    fn finish(&mut self, out: &mut Changes) -> io::Result<()> {
        let words = (self.cells.slots() as usize).div_ceil(64);
        if self.marks.len() < words {
            self.marks.resize(words, 0);
        }
        let crossed = self.pieces > 1;
        let lived = !self.deletes.is_empty() || !self.inserts.is_empty();
        if lived {
            self.net_lives(out);
        }
        let Self { cells, columns, written, marks, deletes, inserts, .. } = self;
        let Changes { columns: changed, inserted, .. } = out;
        let named = || deletes.iter().chain(inserts.iter()).map(|&(_, slot, _)| slot);
        if lived && !crossed {
            named().for_each(|slot| {
                mark(marks, slot);
            });
        }
        for ((changed, written), &column) in changed.iter_mut().zip(written.iter_mut()).zip(columns.iter()) {
            if !crossed && !lived {
                std::mem::swap(&mut changed.slots, written);
            } else if !crossed {
                changed.slots.extend(written.iter().copied().filter(|&slot| !held(marks, slot)));
                changed.slots.extend_from_slice(&inserted.slots);
            } else {
                let live = cells.live();
                for &slot in written.iter().chain(inserted.slots.iter()) {
                    if live.get(slot) && !mark(marks, slot) {
                        changed.slots.push(slot);
                    }
                }
                changed.slots.iter().for_each(|&slot| unmark(marks, slot));
            }
            written.clear();
            changed.fill(cells.column(column).expect("a selected column"))?;
        }
        if lived && !crossed {
            named().for_each(|slot| unmark(marks, slot));
        }
        self.deletes.clear();
        self.inserts.clear();
        std::mem::swap(&mut out.texts, &mut self.texts);
        self.texts.clear();
        let held = self.status;
        self.read_status()?;
        if self.status != held {
            out.status = Some(self.status);
        }
        Ok(())
    }

    /// The rows live before the step and gone after it, and those live after it and new, from the
    /// step's lives in hour order (deletes before inserts within an hour).
    fn net_lives(&mut self, out: &mut Changes) {
        let first = self.game(0);
        let Self { deletes, inserts, marks, cells, .. } = self;
        let Changes { deleted, inserted, .. } = out;
        let game = |hour: u32| first + hour as Hour;
        // A slot is deleted twice, or inserted twice, only with the other between.
        if inserts.is_empty() || deletes.is_empty() {
            deletes.iter().for_each(|&(hour, slot, key)| deleted.push(game(hour), slot, key));
            inserts.iter().filter(|&&(_, slot, key)| cells.key(slot) == Some(key)).for_each(|&(hour, slot, key)| inserted.push(game(hour), slot, key));
            return;
        }
        let (mut d, mut i) = (0, 0);
        while d < deletes.len() || i < inserts.len() {
            if i == inserts.len() || (d < deletes.len() && deletes[d].0 <= inserts[i].0) {
                let (hour, slot, key) = deletes[d];
                if !mark(marks, slot) {
                    deleted.push(game(hour), slot, key);
                }
                d += 1;
            } else {
                mark(marks, inserts[i].1);
                i += 1;
            }
        }
        deletes.iter().chain(inserts.iter()).for_each(|&(_, slot, _)| unmark(marks, slot));
        for &(hour, slot, key) in inserts.iter().rev() {
            if !mark(marks, slot) && cells.key(slot) == Some(key) {
                inserted.push(game(hour), slot, key);
            }
        }
        inserted.reverse();
        inserts.iter().for_each(|&(_, slot, _)| unmark(marks, slot));
    }

    /// The status and sample hour at the head, read again only where its runs change.
    fn read_status(&mut self) -> io::Result<()> {
        let head = self.next.expect("sought") - 1;
        if head < self.status_until {
            if self.status_moving {
                self.status.1 = Some(self.game(head));
            }
            return Ok(());
        }
        let block = (head / BLOCK) as usize;
        let page = &self.pages.runs()[self.run].pages[block];
        let entry = &page.tables[self.table];
        let start = block as u32 * BLOCK;
        let first = self.game(start);
        let hour = (head - start) as u16;
        let (status, _, sampled, change) = reader::status(entry, hour, first)?;
        let until = change.map_or(page.hours, |change| change);
        let last = until - 1;
        self.status_moving = sampled == Some(first + Hour::from(hour)) && last > hour && reader::status(entry, last, first)?.2 == Some(first + Hour::from(last));
        self.status_until = start + u32::from(until);
        self.status = (status, sampled);
        Ok(())
    }
}

#[inline]
fn mark(marks: &mut [u64], slot: u32) -> bool {
    let (word, bit) = (slot as usize / 64, 1_u64 << (slot % 64));
    let held = marks[word] & bit != 0;
    marks[word] |= bit;
    held
}

#[inline]
fn held(marks: &[u64], slot: u32) -> bool {
    marks[slot as usize / 64] & 1_u64 << (slot % 64) != 0
}

#[inline]
fn unmark(marks: &mut [u64], slot: u32) {
    marks[slot as usize / 64] &= !(1_u64 << (slot % 64));
}

pub fn add(total: &mut Spent, more: &Spent) {
    total.numeric_events += more.numeric_events;
    total.numeric += more.numeric;
    total.numeric_scanned += more.numeric_scanned;
    total.numeric_visited += more.numeric_visited;
    total.list_events += more.list_events;
    total.list += more.list;
    total.lives += more.lives;
    total.lifetimes += more.lifetimes;
}

fn subtract(total: &mut Spent, less: &Spent) {
    total.numeric_events -= less.numeric_events;
    total.numeric -= less.numeric;
    total.numeric_scanned -= less.numeric_scanned;
    total.numeric_visited -= less.numeric_visited;
    total.list_events -= less.list_events;
    total.list -= less.list;
    total.lives -= less.lives;
    total.lifetimes -= less.lifetimes;
}
