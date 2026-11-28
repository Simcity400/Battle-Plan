
use crate::{
    changes::{Changes, Passing, Texts, Written},
    invalid,
    pages::{EntryRef, Pages},
};
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{Strings, TableState, TableView, Value},
    unit::{
        BLOCK,
        reader::{self, Advanced, Cursor, Lives, LogCursor, Part, Prefix, Spent},
    },
};
use replay_data::Hour;
use std::{
    collections::VecDeque,
    io,
    sync::Arc,
    time::Instant,
};

pub mod log;
#[cfg(test)]
pub(crate) mod tests;

const TRACK_BYTES: usize = 34;

#[derive(Default)]
pub(crate) struct Known(Strings);

impl Known {
    pub(crate) fn note(&mut self, id: u32, text: &[u8]) {
        self.0.hold(id, text);
    }

    pub(crate) fn clear(&mut self) {
        self.0 = Strings::default();
    }

    fn texts(&self) -> Texts {
        let mut texts = Texts::default();
        for (id, text) in self.0.iter() {
            texts.push(id, text);
        }
        texts
    }

    fn bytes(&self) -> usize {
        self.0.bytes()
    }
}

#[expect(clippy::large_enum_variant, reason = "retired passes are mostly state cursors, so boxing them would add an allocation to nearly every retirement")]
pub enum Passed {
    State(Cursor),
    Log(LogCursor),
}

pub struct Given {
    block: usize,
    index: usize,
    part: Part,
}

#[derive(Default)]
pub struct Chain {
    next: Option<(usize, usize, Hour)>,
    prefix: Option<(usize, Prefix)>,
}

pub struct Fetch {
    pages: Arc<dyn Pages>,
    run: usize,
    table: usize,
    columns: Arc<[usize]>,
    chain: Chain,
    through: Hour,
}

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

    pub fn first_hour(&self) -> Hour {
        self.chain.next.map_or(Hour::MAX, |(_, _, hour)| hour)
    }

    pub fn run(self) -> io::Result<Fetched> {
        let Self { pages, run, table, columns, mut chain, through } = self;
        let held = &pages.runs()[run];
        let started = Instant::now();
        let (mut given, mut tracks, mut stored, mut decompressed) = (Vec::new(), 0, 0, 0);
        let mut cached: Option<(usize, EntryRef)> = None;
        while let Some((block, index, hour)) = chain.next {
            if !given.is_empty() && hour > through {
                break;
            }
            let entry = match &cached {
                Some((at, entry)) if *at == block => entry.clone(),
                _ => {
                    let entry = pages.table_entry(run, block, table)?;
                    cached = Some((block, entry.clone()));
                    entry
                }
            };
            let hours = held.block_hours(block).ok_or_else(|| invalid(format!("Run {run} holds no block {block}")))?;
            let parts = &entry.parts;
            let last = parts.get(index + 1).map_or(hours - 1, |following| following.first - 1);
            let previous = chain.prefix.as_ref().filter(|(at, _)| *at == block && index > 0).map(|(_, prefix)| prefix);
            let part = reader::fetch(pages.extents(run), &parts[index], last, &columns, previous)?;
            tracks += part.track_count();
            stored += part.stored;
            decompressed += part.decompressed;
            chain.next = next_part(&*pages, run, table, block, index + 1, Some((block, &entry)))?;
            chain.prefix = chain.next.filter(|&(next, _, _)| next == block).map(|_| (block, part.prefix()));
            given.push(Given { block, index, part });
        }
        Ok(Fetched { given, chain, tracks, stored, decompressed, ns: started.elapsed().as_nanos() as u64 })
    }
}

fn next_part(pages: &dyn Pages, run: usize, table: usize, mut block: usize, mut index: usize, cached: Option<(usize, &EntryRef)>) -> io::Result<Option<(usize, usize, Hour)>> {
    let held = &pages.runs()[run];
    let blocks = held.blocks();
    while block < blocks {
        let owned;
        let entry: &EntryRef = match cached {
            Some((at, entry)) if at == block => entry,
            _ => {
                owned = pages.table_entry(run, block, table)?;
                &owned
            }
        };
        if let Some(part) = entry.parts.get(index) {
            let hour = held.first_hour + (block as u32 * BLOCK + u32::from(part.first)) as Hour;
            return Ok(Some((block, index, hour)));
        }
        block += 1;
        index = 0;
    }
    Ok(None)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub cells: usize,
    pub parts: usize,
    pub part_bytes: usize,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct Sought {
    pub restore_ns: u64,
    pub fetch_ns: u64,
    pub replay_ns: u64,
    pub parts: u64,
    pub restored: Option<usize>,
    pub blocks: u32,
    pub spent: Spent,
}

struct Under {
    cursor: Cursor,
    block: usize,
    last: u16,
    bytes: usize,
    tracks: usize,
    texts: Vec<u32>,
}

#[derive(Default)]
struct Passage {
    live: Option<Vec<u64>>,
    gone: Vec<(u32, u32, u128)>,
    staged: Vec<Written>,
    failed: Option<io::Error>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Reached,
    Waiting,
    Packed,
}

pub struct Lane {
    pages: Arc<dyn Pages>,
    run: usize,
    table: usize,
    kinds: Vec<Kind>,
    declared: Arc<[usize]>,
    columns: Arc<[usize]>,
    keyed: bool,
    cells: TableState,
    block_entry: Option<(usize, EntryRef)>,
    under: Option<Under>,
    ahead: VecDeque<Given>,
    next: Option<u32>,
    chain: Option<Chain>,
    retired: Vec<Passed>,
    texts: Texts,
    known: Known,
    status: (Status, Option<Hour>),
    status_until: u32,
    status_moving: bool,
    timed: bool,
    spent: Spent,
    written: Vec<Vec<u32>>,
    lives: Lives,
    deletes: Vec<(u32, u32, u128)>,
    inserts: Vec<(u32, u32, u128)>,
    pieces: u32,
    marks: Vec<u64>,
    marks_at: Vec<usize>,
    read: Vec<u32>,
    prev_read: Vec<u32>,
    passage: Option<Box<Passage>>,
    log: Option<Box<log::Logs>>,
    #[cfg(test)]
    pub(crate) state_reads: usize,
    #[cfg(test)]
    pub(crate) read_compares: usize,
}

pub struct Selected<'a> {
    keys: &'a replay_frame::frame::Keys,
    cells: TableState,
    kinds: Vec<Kind>,
    strings: replay_columnar::store::Strings,
    pub sampled: Option<Hour>,
}

impl Selected<'_> {
    pub fn view(&self, keys: &replay_frame::frame::Keys) -> io::Result<TableView<'_>> {
        if self.keys != keys { return Err(invalid("Keys are outside the loaded selection")); }
        Ok(TableView::new(&self.cells, &self.kinds, &self.strings))
    }
}

fn ends_at_checkpoint(target: u32, hours: u16, status: Status) -> bool {
    !matches!(status, Status::Truncated { .. }) && target % BLOCK + 1 == u32::from(hours)
}

pub fn table_status(pages: &dyn Pages, run: usize, table: usize, hour: Hour) -> io::Result<(Status, Option<Hour>, Option<String>)> {
    let held = pages.runs().get(run).ok_or_else(|| invalid(format!("No run {run}")))?;
    let target = (i64::from(hour) - i64::from(held.first_hour)).clamp(0, i64::from(held.hours()) - 1) as u32;
    let entry = pages.table_entry(run, (target / BLOCK) as usize, table)?;
    if entry.log { return Err(invalid(format!("Table {table} is a log: its rows are read by ordinal, never at an hour"))); }
    let (status, _, sampled, _) = reader::status(&entry, (target % BLOCK) as u16, held.first_hour + (target / BLOCK * BLOCK) as Hour)?;
    if let Some(reason) = pages.withheld_at(run, table, hour) { return Ok((Status::Unavailable { reason: 0 }, sampled, Some(reason.to_owned()))); }
    let reason = if let Status::Unavailable { reason } = status {
        let block = (target / BLOCK) as usize;
        let hours = held.block_hours(block).ok_or_else(|| invalid("The run holds no block for this hour"))?;
        let end = ends_at_checkpoint(target, hours, status);
        let mut named = None;
        for at in (0..=block).rev() {
            let entry = if at == block { entry.clone() } else { pages.table_entry(run, at, table)? };
            let source = pages.extents(run);
            if let Some(restore) = entry.restore.as_ref().filter(|_| at < block || end) {
                named = reader::restore_text(source, restore, reason)?;
                break;
            }
            let hour = if at == block { (target % BLOCK) as u16 } else { BLOCK as u16 - 1 };
            named = reader::part_text(source, &entry.parts, hour, reason)?;
            if named.is_some() { break; }
            if let Some(first) = &entry.first {
                named = reader::restore_text(source, first, reason)?;
                break;
            }
        }
        Some(named.and_then(|bytes| String::from_utf8(bytes).ok()).unwrap_or_else(|| "A table's reason is not text".into()))
    } else { None };
    Ok((status, sampled, reason))
}

pub fn selected_at<'a>(pages: &dyn Pages, run: usize, table: usize, columns: &[usize], hour: Hour, keys: &'a replay_frame::frame::Keys) -> io::Result<Option<Selected<'a>>> {
    let held = pages.runs().get(run).ok_or_else(|| invalid(format!("No run {run}")))?;
    let target = (i64::from(hour) - i64::from(held.first_hour)).clamp(0, i64::from(held.hours()) - 1) as u32;
    let block = (target / BLOCK) as usize;
    let Some(hours) = held.block_hours(block) else { return Ok(None); };
    if (target != 0 && target % BLOCK + 1 != u32::from(hours)) || pages.withheld_at(run, table, hour).is_some() { return Ok(None); }
    let entry = pages.table_entry(run, block, table)?;
    if entry.log { return Ok(None); }
    let (status, _, sampled, _) = reader::status(&entry, (target % BLOCK) as u16, held.first_hour + (target / BLOCK * BLOCK) as Hour)?;
    if status != Status::Complete { return Ok(None); }
    let end = ends_at_checkpoint(target, hours, status).then_some(entry.restore.as_ref()).flatten();
    let Some(restore) = end.or_else(|| (target == 0).then_some(entry.first.as_ref()).flatten()) else { return Ok(None); };
    let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("No table {table} in the schema")))?;
    let kinds = &kinds[..pages.held(table).min(kinds.len())];
    let mut sorted = None;
    if let replay_frame::frame::Keys::These(keys) = keys && !keys.is_sorted() {
        let mut keys = keys.clone();
        keys.sort_unstable();
        sorted = Some(replay_frame::frame::Keys::These(keys));
    }
    let wants = sorted.as_ref().unwrap_or(keys);
    let (cells, strings) = reader::restore_selected(pages.extents(run), restore, kinds, columns, &|key| wants.contains(key))?;
    Ok(Some(Selected { keys, cells, kinds: kinds.to_vec(), strings, sampled }))
}

impl Lane {
    pub fn new(pages: Arc<dyn Pages>, run: usize, table: usize, columns: &[usize], keyed: bool) -> io::Result<Self> {
        let held_run = pages.runs().get(run).ok_or_else(|| invalid(format!("No run {run}")))?;
        let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("No table {table} in the schema")))?.to_vec();
        if held_run.blocks() == 0 {
            return Err(invalid(format!("Run {run} holds no table {table}")));
        }
        let first_entry = pages.table_entry(run, 0, table)?;
        for (at, &column) in columns.iter().enumerate() {
            if column >= kinds.len() || columns[..at].contains(&column) {
                return Err(invalid(format!("Column {column} of table {table} is selected twice or not in it")));
            }
        }
        let log = first_entry.log.then(Box::default);
        drop(first_entry);
        let kinds = kinds[..pages.held(table).min(kinds.len())].to_vec();
        let read: Arc<[usize]> = columns.iter().copied().filter(|&column| column < kinds.len()).collect();
        Ok(Self {
            pages,
            run,
            table,
            cells: TableState::new(&kinds),
            block_entry: None,
            kinds,
            declared: columns.into(),
            written: vec![Vec::new(); read.len()],
            columns: read,
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
            lives: Lives::default(),
            deletes: Vec::new(),
            inserts: Vec::new(),
            pieces: 0,
            marks: Vec::new(),
            marks_at: Vec::new(),
            read: Vec::new(),
            prev_read: Vec::new(),
            passage: None,
            log,
            #[cfg(test)]
            state_reads: 0,
            #[cfg(test)]
            read_compares: 0,
        })
    }

    pub fn table(&self) -> usize {
        self.table
    }

    pub fn run(&self) -> usize {
        self.run
    }

    pub fn requested(&self) -> &[usize] {
        &self.declared
    }

    pub fn reads(&self) -> &[usize] {
        &self.columns
    }

    pub fn keyed(&self) -> bool {
        self.keyed
    }

    pub fn format(&self) -> u16 {
        self.pages.format()
    }

    pub fn set_passing(&mut self, passing: bool) {
        self.passage = passing.then(Box::default);
    }

    pub fn passing(&self) -> bool {
        self.passage.is_some()
    }

    pub fn declared(&self) -> (&[usize], bool, bool) {
        (self.requested(), self.keyed(), self.passing())
    }

    pub fn cells(&self) -> &TableState {
        &self.cells
    }

    pub fn view(&self) -> TableView<'_> {
        TableView::new(&self.cells, &self.kinds, &self.known.0)
    }

    fn pack(&mut self) -> io::Result<()> {
        let live: Vec<u32> = if self.read.is_empty() { Vec::new() } else { self.cells.rows().collect() };
        self.cells.pack()?;
        if !self.read.is_empty() {
            self.read = self.read.iter().filter_map(|slot| live.binary_search(slot).ok().map(|at| at as u32)).collect();
        }
        if self.keyed {
            self.cells.try_index_keys()?;
        }
        Ok(())
    }

    pub fn status(&self) -> (Status, Option<Hour>) {
        self.status
    }

    pub fn reason(&self) -> Option<&[u8]> {
        let Status::Unavailable { reason } = self.status.0 else { return None };
        self.under.as_ref().and_then(|under| under.cursor.text(reason)).or_else(|| self.known.0.get(reason))
    }

    pub fn read_rows(&self) -> Option<&[u32]> {
        matches!(self.status.0, Status::Truncated { .. }).then_some(self.read.as_slice())
    }

    pub fn head(&self) -> Option<Hour> {
        self.next.map(|next| self.game(next) - 1)
    }

    pub fn withheld(&self) -> Option<&str> {
        self.pages.withheld_at(self.run, self.table, self.head()?)
    }

    pub fn time(&mut self, timed: bool) {
        self.timed = timed;
        if let Some(under) = &mut self.under {
            under.cursor.time(timed);
        }
    }

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

    pub(super) fn entry_at(&mut self, block: usize) -> io::Result<EntryRef> {
        if let Some((at, entry)) = &self.block_entry
            && *at == block
        {
            return Ok(entry.clone());
        }
        let entry = self.pages.table_entry(self.run, block, self.table)?;
        if entry.log != self.log.is_some() {
            return Err(invalid(format!("Table {} is a log in one block of its run and holds state in another", self.table)));
        }
        self.block_entry = Some((block, entry.clone()));
        Ok(entry)
    }

    pub fn want(&mut self, through: Hour) -> Option<Fetch> {
        let (_, _, hour) = self.chain.as_ref()?.next?;
        if !self.ahead.is_empty() && hour > through {
            return None;
        }
        let chain = self.chain.take()?;
        Some(Fetch { pages: Arc::clone(&self.pages), run: self.run, table: self.table, columns: Arc::clone(&self.columns), chain, through })
    }

    pub(crate) fn holds_through(&self, through: Hour) -> bool {
        self.chain.as_ref().is_some_and(|chain| chain.next.is_none_or(|(_, _, hour)| hour > through))
    }

    pub fn give(&mut self, fetched: Fetched) -> io::Result<()> {
        if self.chain.is_some() {
            return Err(invalid(format!("Table {}'s lane is given parts it did not ask for", self.table)));
        }
        self.ahead.extend(fetched.given);
        self.chain = Some(fetched.chain);
        Ok(())
    }

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

    pub fn retire_into(&mut self, out: &mut Vec<Passed>) {
        out.append(&mut self.retired);
    }

    pub fn seek(&mut self, hour: Hour) -> io::Result<Sought> {
        if self.log.is_some() {
            return self.seek_log(hour);
        }
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let hours = run.hours();
        let target = (i64::from(hour) - i64::from(run.first_hour)).clamp(0, i64::from(hours) - 1) as u32;
        let block = (target / BLOCK) as usize;
        let block_hours = run.block_hours(block).ok_or_else(|| invalid(format!("Run {} holds no block {block}", self.run)))?;
        let target_entry = self.entry_at(block)?;
        let status = reader::status(&target_entry, (target % BLOCK) as u16, self.game(block as u32 * BLOCK))?.0;
        let ends_block = ends_at_checkpoint(target, block_hours, status);
        let at = if ends_block { Some(block) } else { block.checked_sub(1) };
        let refused = || invalid(format!("No restore of table {} is written", self.table));
        let (restored, restore_entry, from_first) = match at {
            Some(at) => {
                let mut found = None;
                for index in (0..=at).rev() {
                    let entry = if index == block { target_entry.clone() } else { pages.table_entry(self.run, index, self.table)? };
                    if entry.restore.is_some() {
                        found = Some((index, entry, false));
                        break;
                    }
                    if entry.first.is_some() {
                        found = Some((index, entry, true));
                        break;
                    }
                }
                found.ok_or_else(refused)?
            }
            None => {
                let entry = if block == 0 { target_entry.clone() } else { pages.table_entry(self.run, 0, self.table)? };
                if entry.first.is_none() {
                    return Err(refused());
                }
                (0, entry, true)
            }
        };
        let entry = if from_first { restore_entry.first.as_ref().ok_or_else(refused)? } else { restore_entry.restore.as_ref().ok_or_else(refused)? };
        let started = Instant::now();
        #[cfg(test)]
        {
            self.state_reads += 1;
        }
        let (cells, meta) = if self.keyed {
            reader::restore_keyed(pages.extents(self.run), entry, &self.kinds, &self.columns)?
        } else {
            reader::restore(pages.extents(self.run), entry, &self.kinds, &self.columns)?
        };
        drop(restore_entry);
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
        self.read.clear();
        self.prev_read.clear();
        if let Some(passage) = &mut self.passage {
            passage.live = None;
        }
        let restored_hours = run.block_hours(restored).ok_or_else(|| invalid(format!("Run {} holds no block {restored}", self.run)))?;
        let next = if from_first { 1 } else { restored as u32 * BLOCK + u32::from(restored_hours) };
        self.next = Some(next);
        let next_block = (next / BLOCK) as usize;
        let cached = (next_block == block).then_some((block, &target_entry));
        self.chain = Some(Chain { next: next_part(&*pages, self.run, self.table, next_block, 0, cached)?, prefix: None });
        drop(target_entry);
        self.status_until = 0;
        let mut sought = Sought { restore_ns, restored: (!from_first).then_some(restored), blocks: (target + 1).div_ceil(BLOCK).saturating_sub(next / BLOCK), ..Sought::default() };
        let replaying = Instant::now();
        let (fetch_ns, parts) = self.fetch_through(self.game(target))?;
        (sought.fetch_ns, sought.parts) = (fetch_ns, parts as u64);
        if target >= next && self.advance(target, false, None)? != Stop::Reached {
            return Err(invalid(format!("Table {} cannot be replayed to hour {hour}: a part is missing", self.table)));
        }
        self.retired.clear();
        sought.replay_ns = (replaying.elapsed().as_nanos() as u64).saturating_sub(sought.fetch_ns);
        self.read_status()?;
        if from_first && target == 0 && matches!(self.status.0, Status::Truncated { .. }) {
            self.read.extend(self.cells.rows());
        }
        let mut spent = self.spent();
        subtract(&mut spent, &before);
        sought.spent = spent;
        Ok(sought)
    }

    pub fn reset_into(&self, out: &mut Changes) -> io::Result<()> {
        if self.log.is_some() {
            return self.reset_log(out);
        }
        let head = self.head().ok_or_else(|| invalid("A lane is reset before it is sought"))?;
        out.begin(self.table, &self.columns, self.keyed, true);
        for (written, &column) in out.columns.iter_mut().zip(self.columns.iter()) {
            written.fill(self.cells.column(column).expect("a selected column"))?;
        }
        self.report_status(out);
        out.set_head(head, 0, false);
        Ok(())
    }

    pub fn texts(&self) -> &Texts {
        &self.texts
    }

    pub(crate) fn clear_texts(&mut self) {
        self.texts = Texts::default();
    }

    pub(crate) fn texts_into(&mut self, out: &mut Changes) {
        std::mem::swap(&mut out.texts, &mut self.texts);
        self.texts.clear();
    }

    pub(crate) fn report_status(&self, out: &mut Changes) {
        out.status = Some(self.status);
        if let Status::Unavailable { reason } = self.status.0
            && !out.texts.ids.contains(&reason)
            && let Some(text) = self.reason()
        {
            out.texts.push(reason, text);
        }
    }

    pub(crate) fn show_known(&mut self) -> Texts {
        std::mem::replace(&mut self.texts, self.known.texts())
    }

    pub(crate) fn hide_known(&mut self, pending: Texts) {
        self.texts = pending;
    }

    pub fn step(&mut self, to: Hour, out: &mut Changes) -> io::Result<Hour> {
        self.step_hours(to, out, None)
    }

    pub fn step_hours(&mut self, to: Hour, out: &mut Changes, hours: Option<&mut Vec<Changes>>) -> io::Result<Hour> {
        if self.log.is_some() {
            return self.step_log(to, out);
        }
        let Some(next) = self.next else { return Err(invalid("A lane steps before it is sought")) };
        out.begin(self.table, &self.columns, self.keyed, false);
        if matches!(self.status.0, Status::Truncated { .. }) {
            self.prev_read.clear();
            self.prev_read.extend_from_slice(&self.read);
        }
        self.pieces = 0;
        if let Some(passage) = &mut self.passage {
            passage.begin(&self.cells, &self.columns);
        }
        let last = self.pages.runs()[self.run].hours() - 1;
        let target = (i64::from(to) - i64::from(self.game(0))).clamp(0, i64::from(last)) as u32;
        let stop = if target >= next { self.advance(target, true, hours)? } else { Stop::Reached };
        if let Some(passage) = &mut self.passage {
            let first = self.pages.runs()[self.run].first_hour;
            passage.write(&mut out.passing, self.keyed, &self.cells, &self.columns, &self.inserts, &self.deletes, |hour| first + hour as Hour)?;
        }
        self.finish(out)?;
        if stop == Stop::Packed {
            out.set_renumber(self.cells.rows());
            self.pack()?;
            if let Some(passage) = &mut self.passage {
                passage.live = None;
            }
        }
        let head = self.head().expect("sought");
        out.set_head(head, (self.game(target) - head).max(0) as u32, stop == Stop::Waiting);
        Ok(head)
    }

    fn advance(&mut self, target: u32, collect: bool, mut hours: Option<&mut Vec<Changes>>) -> io::Result<Stop> {
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let mut next = self.next.expect("sought");
        while next <= target {
            let block = (next / BLOCK) as usize;
            let block_hours = run.block_hours(block).ok_or_else(|| invalid(format!("Run {} holds no block {block}", self.run)))?;
            let start = block as u32 * BLOCK;
            let end = target.min(start + u32::from(block_hours) - 1);
            let entry = self.entry_at(block)?;
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
                let to = end.min(start + u32::from(self.under.as_ref().expect("entered").last));
                let (from, last) = (hour, (to - start) as u16);
                let reached = {
                    let Self { under, cells, written, lives, passage, columns, marks, marks_at, table, keyed, .. } = self;
                    let cursor = &mut under.as_mut().expect("entered").cursor;
                    let mut passage = passage.as_deref_mut().filter(|_| collect);
                    match hours.as_deref_mut().filter(|_| collect) {
                        None => cursor_advance(cursor, last, last - from + 1, cells, written, lives, passage, columns, start)?,
                        Some(out) => {
                            let mut reached = Advanced::Reached;
                            let first = run.first_hour + start as Hour;
                            for at in from..=last {
                                marks_at.clear();
                                marks_at.extend(written.iter().map(Vec::len));
                                let lived = (lives.deleted.len(), lives.inserted.len());
                                reached = cursor_advance(cursor, at, 1, cells, written, lives, passage.as_deref_mut(), columns, start)?;
                                hour_changes(out, *table, columns, *keyed, first + Hour::from(at), cells, written, marks_at, marks, lives, lived, first)?;
                                if reached != Advanced::Reached {
                                    break;
                                }
                            }
                            reached
                        }
                    }
                };
                if reached != Advanced::Reached {
                    return Err(invalid(format!("Table {}'s cursor misses hours its part holds", self.table)));
                }
                self.note_read(block, last)?;
                next = to + 1;
                {
                    let Self { under, texts, known, .. } = self;
                    let under = under.as_mut().expect("entered");
                    for id in under.texts.drain(..) {
                        if let Some(text) = under.cursor.text(id) {
                            texts.push(id, text);
                            known.note(id, text);
                        }
                    }
                }
                if last == self.under.as_ref().expect("entered").last {
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
                self.read.clear();
                next = end + 1;
            }
            if next == start + u32::from(block_hours) && entry.packed {
                self.next = Some(next);
                if collect {
                    return Ok(Stop::Packed);
                }
                self.pack()?;
            }
        }
        self.next = Some(next);
        Ok(Stop::Reached)
    }

    fn note_read(&mut self, block: usize, hour: u16) -> io::Result<()> {
        self.read.clear();
        let entry = self.entry_at(block)?;
        if !matches!(reader::status(&entry, hour, self.game(block as u32 * BLOCK))?.0, Status::Truncated { .. }) {
            return Ok(());
        }
        let Some(under) = &self.under else { return Ok(()) };
        let (mut touched, mut read) = (Vec::new(), Vec::new());
        under.cursor.marks(hour, hour, &mut touched, &mut read)?;
        let live = self.cells.live();
        let begun = self.lives.inserted.iter().filter(|&&(at, ..)| at == hour).map(|&(_, slot, _)| slot);
        self.read.extend(read.iter().map(|&(_, slot)| slot).chain(begun).filter(|&slot| live.get(slot)));
        self.read.sort_unstable();
        self.read.dedup();
        Ok(())
    }

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

    fn pass(&mut self) -> io::Result<()> {
        let mut under = self.under.take().expect("a part under the head");
        under.cursor.advance(under.last + 1, 1, &mut self.cells, &mut self.written, &mut self.lives)?;
        add(&mut self.spent, &under.cursor.spent);
        self.retired.push(Passed::State(under.cursor));
        Ok(())
    }

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
        out.status_left = self.status_left(held.0)?;
        self.read_status()?;
        if self.status != held {
            self.report_status(out);
        }
        if self.status.0 == held.0 && matches!(self.status.0, Status::Truncated { .. }) {
            #[cfg(test)]
            {
                self.read_compares += 1;
            }
            if self.prev_read != self.read {
                out.read_rows_changed = true;
            }
        }
        Ok(())
    }

    fn status_left(&self, held: Status) -> io::Result<Option<Hour>> {
        let head = self.next.expect("sought") - 1;
        let mut hour = self.status_until;
        while hour <= head {
            let block = (hour / BLOCK) as usize;
            let block_hours = self.pages.runs()[self.run].block_hours(block).ok_or_else(|| invalid(format!("Run {} holds no block {block}", self.run)))?;
            let entry = match &self.block_entry {
                Some((at, entry)) if *at == block => entry.clone(),
                _ => self.pages.table_entry(self.run, block, self.table)?,
            };
            let start = block as u32 * BLOCK;
            let (status, _, _, change) = reader::status(&entry, (hour - start) as u16, self.game(start))?;
            if status != held {
                return Ok(Some(self.game(hour)));
            }
            hour = start + u32::from(change.unwrap_or(block_hours));
        }
        Ok(None)
    }

    fn net_lives(&mut self, out: &mut Changes) {
        let first = self.game(0);
        let Self { deletes, inserts, marks, cells, .. } = self;
        let Changes { deleted, inserted, .. } = out;
        let game = |hour: u32| first + hour as Hour;
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

    fn read_status(&mut self) -> io::Result<()> {
        let head = self.next.expect("sought") - 1;
        if head < self.status_until {
            if self.status_moving {
                self.status.1 = Some(self.game(head));
            }
            return Ok(());
        }
        let block = (head / BLOCK) as usize;
        let block_hours = self.pages.runs()[self.run].block_hours(block).ok_or_else(|| invalid(format!("Run {} holds no block {block}", self.run)))?;
        let entry = self.entry_at(block)?;
        let start = block as u32 * BLOCK;
        let first = self.game(start);
        let hour = (head - start) as u16;
        let (status, _, sampled, change) = reader::status(&entry, hour, first)?;
        let until = change.map_or(block_hours, |change| change);
        let last = until - 1;
        self.status_moving = sampled == Some(first + Hour::from(hour)) && last > hour && reader::status(&entry, last, first)?.2 == Some(first + Hour::from(last));
        self.status_until = start + u32::from(until);
        self.status = (status, sampled);
        Ok(())
    }
}

#[expect(clippy::too_many_arguments, reason = "the lane's fields, borrowed apart")]
fn cursor_advance(cursor: &mut Cursor, to: u16, span: u16, cells: &mut TableState, written: &mut [Vec<u32>], lives: &mut Lives, passage: Option<&mut Passage>, columns: &[usize], start: u32) -> io::Result<Advanced> {
    match passage {
        None => cursor.advance(to, span, cells, written, lives),
        Some(passage) => {
            let mut gone = |hour: u16, slot: u32, key: u128, cells: &TableState| passage.see(start + u32::from(hour), slot, key, cells, columns);
            cursor.advance_seeing(to, span, cells, written, lives, &mut gone)
        }
    }
}

#[expect(clippy::too_many_arguments, reason = "the lane's fields, borrowed apart")]
fn hour_changes(out: &mut Vec<Changes>, table: usize, columns: &[usize], keyed: bool, hour: Hour, cells: &TableState, written: &[Vec<u32>], at: &[usize], marks: &mut Vec<u64>, lives: &Lives, lived: (usize, usize), first: Hour) -> io::Result<()> {
    let wrote = written.iter().zip(at).any(|(held, &mark)| held.len() > mark);
    let (deleted, inserted) = (&lives.deleted[lived.0.min(lives.deleted.len())..], &lives.inserted[lived.1.min(lives.inserted.len())..]);
    if !wrote && deleted.is_empty() && inserted.is_empty() {
        return Ok(());
    }
    let words = (cells.slots() as usize).div_ceil(64);
    if marks.len() < words {
        marks.resize(words, 0);
    }
    let mut changes = Changes::default();
    changes.begin(table, columns, keyed, false);
    let live = cells.live();
    for ((changed, held), &mark) in changes.columns.iter_mut().zip(written).zip(at) {
        for &slot in held[mark.min(held.len())..].iter().chain(inserted.iter().map(|(_, slot, _)| slot)) {
            if live.get(slot) && !self::mark(marks, slot) {
                changed.slots.push(slot);
            }
        }
        changed.slots.iter().for_each(|&slot| unmark(marks, slot));
        changed.fill(cells.column(changed.column()).expect("a selected column"))?;
    }
    for &(at, slot, key) in deleted {
        changes.deleted.push(first + Hour::from(at), slot, key);
    }
    for &(at, slot, key) in inserted {
        changes.inserted.push(first + Hour::from(at), slot, key);
    }
    changes.set_head(hour, 0, false);
    out.push(changes);
    Ok(())
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
