//! A log table's lane: a log's rows are appended once and later forgotten, never changed, so the
//! lane holds no key and no cell. It holds the log's count and live set at the head and the part
//! under it; a step hands over each row appended, with its hour and selected cells read as it
//! passes, and each run of rows forgotten. A reader that shows the rows listed now asks for their
//! cells by ordinal ([`rows`]).

use super::{Chain, Lane, Passed, Sought, Stop, next_part};
use crate::{
    changes::{Changes, Forgotten, Logged, Texts, Written},
    invalid,
    pages::Pages,
};
use replay_columnar::{
    state::Kind,
    store::{Listed, TableState, Value},
    unit::{
        BLOCK,
        reader::{self, Advanced, LogCursor, LogEvent, Prefix},
    },
};
use replay_data::Hour;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::Arc,
    time::Instant,
};

#[cfg(test)]
pub(crate) mod tests;

/// A log row's place in its run: the rows appended before it.
pub type Ordinal = u64;

/// Whether `table` is a log in every run of `pages`; a table a log in one run and state in
/// another is refused by name.
pub fn is_log(pages: &dyn Pages, table: usize) -> io::Result<bool> {
    let mut class = None;
    for (index, run) in pages.runs().iter().enumerate() {
        let log = reader::class_of(&run.pages, table)?;
        if class.is_some_and(|held| held != log) {
            return Err(invalid(format!("Table {table} is a log in one run and holds state in run {index}")));
        }
        class = Some(log);
    }
    Ok(class == Some(true))
}

/// A log's rows the game still lists: every 64-ordinal word holding a listed row, with its index,
/// in order. Words emptied in the middle stay until they are half of those held, so a forget
/// moves nothing; what it holds follows the rows listed, never the rows ever appended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveSet {
    words: VecDeque<(u64, u64)>,
    listed: u64,
    empty: usize,
}

impl LiveSet {
    /// The rows a reader's live set lists.
    pub fn from_listed(listed: &Listed) -> Self {
        let mut set = Self::default();
        for ordinal in listed.ones() {
            set.append(ordinal);
        }
        set
    }

    pub fn len(&self) -> u64 {
        self.listed
    }

    pub fn is_empty(&self) -> bool {
        self.listed == 0
    }

    pub fn get(&self, ordinal: Ordinal) -> bool {
        self.find(ordinal / 64).is_ok_and(|at| self.words[at].1 >> (ordinal % 64) & 1 == 1)
    }

    /// The lowest ordinal listed.
    pub fn lowest(&self) -> Option<Ordinal> {
        self.words.iter().find(|(_, bits)| *bits != 0).map(|&(word, bits)| word * 64 + Ordinal::from(bits.trailing_zeros()))
    }

    /// Every ordinal listed, rising.
    pub fn ones(&self) -> impl Iterator<Item = Ordinal> + '_ {
        self.words.iter().flat_map(|&(word, bits)| {
            let mut rest = bits;
            std::iter::from_fn(move || {
                (rest != 0).then(|| {
                    let bit = rest.trailing_zeros();
                    rest &= rest - 1;
                    word * 64 + Ordinal::from(bit)
                })
            })
        })
    }

    pub fn bytes(&self) -> usize {
        self.words.capacity() * size_of::<(u64, u64)>()
    }

    fn find(&self, word: u64) -> Result<usize, usize> {
        self.words.binary_search_by_key(&word, |&(held, _)| held)
    }

    /// Listed from `ordinal` on, which is past every one held.
    pub(crate) fn append(&mut self, ordinal: Ordinal) {
        let (word, bit) = (ordinal / 64, 1_u64 << (ordinal % 64));
        match self.words.back_mut() {
            Some((last, bits)) if *last == word => *bits |= bit,
            _ => self.words.push_back((word, bit)),
        }
        self.listed += 1;
    }

    /// Rows `from..to` no longer listed; how many of them were.
    pub(crate) fn forget(&mut self, from: Ordinal, to: Ordinal) -> u64 {
        let mut at = self.find(from / 64).unwrap_or_else(|at| at);
        let mut cleared = 0;
        while let Some((word, bits)) = self.words.get_mut(at) {
            let base = *word * 64;
            if base >= to {
                break;
            }
            let (low, high) = (from.max(base) - base, to.min(base + 64) - base);
            let mask = if high - low == 64 { u64::MAX } else { ((1_u64 << (high - low)) - 1) << low };
            let gone = *bits & mask;
            if gone != 0 && gone == *bits {
                self.empty += 1;
            }
            *bits &= !mask;
            cleared += u64::from(gone.count_ones());
            at += 1;
        }
        self.listed -= cleared;
        while self.words.front().is_some_and(|&(_, bits)| bits == 0) {
            self.words.pop_front();
            self.empty -= 1;
        }
        if self.empty > 0 && self.empty * 2 > self.words.len() {
            self.words.retain(|&(_, bits)| bits != 0);
            self.empty = 0;
        }
        if self.words.capacity() > 64 && self.words.len() * 4 < self.words.capacity() {
            self.words.shrink_to_fit();
        }
        cleared
    }
}

/// A log's state at the head: rows appended ever and the rows listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogState {
    pub(crate) count: Ordinal,
    pub(crate) live: LiveSet,
}

impl LogState {
    pub fn count(&self) -> Ordinal {
        self.count
    }

    pub fn live(&self) -> &LiveSet {
        &self.live
    }

    pub fn bytes(&self) -> usize {
        size_of::<Self>() + self.live.bytes()
    }
}

/// The part under a log lane's head and its cursor.
pub(super) struct LogUnder {
    cursor: LogCursor,
    block: usize,
    last: u16,
    pub(super) bytes: usize,
    pub(super) tracks: usize,
}

/// What a log lane holds besides what every lane does.
#[derive(Default)]
pub(super) struct Logs {
    pub(super) state: LogState,
    pub(super) under: Option<LogUnder>,
}

/// Applies one event at or after block hour `from` (one before was applied by an earlier read)
/// to the log's state, and to the step's change set when there is one.
fn apply(event: LogEvent<'_>, from: u16, start: Hour, state: &mut LogState, logged: Option<&mut Logged>) -> io::Result<()> {
    match event {
        LogEvent::Appended { hour, .. } | LogEvent::Forgotten { hour, .. } if hour < from => {}
        LogEvent::Appended { hour, ordinal, key, rows, at } => {
            if ordinal != state.count {
                return Err(invalid(format!("A log appends row {ordinal} where it holds {} rows", state.count)));
            }
            state.count += 1;
            state.live.append(ordinal);
            if let Some(logged) = logged {
                logged.push(start + Hour::from(hour), key, rows, at)?;
            }
        }
        LogEvent::Forgotten { hour, from: first, to } => {
            if to > state.count || state.live.forget(first, to) != to - first {
                return Err(invalid(format!("A log forgets rows {first} to {to}, which it does not list")));
            }
            if let Some(logged) = logged {
                logged.forgotten.push(Forgotten { hour: start + Hour::from(hour), from: first, to });
            }
        }
        // Every variant is named: a new one must fail to compile here, never fall into an arm.
        other @ LogEvent::Named { .. } => return Err(unread(&other)),
    }
    Ok(())
}

/// A log event the playhead does not read yet, by name: never passed over in silence.
pub(crate) fn unread(event: &LogEvent<'_>) -> io::Error {
    invalid(format!("A log hands the playhead an event it does not read: {event:?}"))
}

impl Lane {
    /// The log's count and live set at the head, for a log table.
    pub fn log(&self) -> Option<&LogState> {
        self.log.as_deref().map(|logs| &logs.state)
    }

    /// A log's state at the end of game hour `hour`: its restore at the end of the block before the
    /// hour's (or at the hour's own block's end, or the run's first restore), then the hour's block
    /// through it: its parts before the one holding the hour walked with no column (their cells
    /// decompressed only as the next part's prefix), then that one entered as a step enters it.
    pub(super) fn seek_log(&mut self, hour: Hour) -> io::Result<Sought> {
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let target = (i64::from(hour) - i64::from(run.first_hour)).clamp(0, i64::from(run.hours()) - 1) as u32;
        let block = (target / BLOCK) as usize;
        let ends_block = target + 1 == block as u32 * BLOCK + u32::from(run.pages[block].hours);
        let at = if ends_block { Some(block) } else { block.checked_sub(1) };
        let entry = match at {
            Some(at) => run.pages[at].tables[self.table].restore.as_ref(),
            None => run.pages[0].tables[self.table].first.as_ref(),
        };
        let entry = entry.ok_or_else(|| invalid(format!("Log table {} has no restore before hour {hour}", self.table)))?;
        let started = Instant::now();
        let (restored, meta) = reader::restore(pages.extents(self.run), entry, &self.kinds, &[])?;
        let restore_ns = started.elapsed().as_nanos() as u64;
        let (Some(count), Some(listed)) = (restored.count(), restored.listed()) else {
            return Err(invalid(format!("Table {}'s restore is not a log's", self.table)));
        };
        let logs = self.log.as_deref_mut().expect("a log lane");
        logs.under = None;
        logs.state = LogState { count, live: LiveSet::from_listed(listed) };
        self.ahead.clear();
        self.retired.clear();
        self.texts.clear();
        self.known.clear();
        for (id, text) in &meta.strings {
            self.texts.push(*id, text);
            self.known.note(*id, text);
        }
        let mut next = at.map_or(1, |at| at as u32 * BLOCK + u32::from(run.pages[at].hours));
        let mut chain = Chain { next: next_part(run, self.table, (next / BLOCK) as usize, 0), prefix: None };
        self.status_until = 0;
        let mut sought = Sought { restore_ns, restored: at, blocks: u32::from(target >= next), ..Sought::default() };
        let replaying = Instant::now();
        let start = block as u32 * BLOCK;
        let parts = &run.pages[block].tables[self.table].parts;
        let under = parts.partition_point(|part| u32::from(part.first) <= target - start);
        if target >= next && under > 1 {
            let (prefix, fetch_ns) = self.walk_lean(block, under - 1, (next - start) as u16)?;
            next = start + u32::from(parts[under - 1].first);
            chain = Chain { next: Some((block, under - 1)), prefix: Some((block, prefix)) };
            (sought.fetch_ns, sought.parts) = (fetch_ns, under as u64 - 1);
        }
        self.next = Some(next);
        self.chain = Some(chain);
        let (fetch_ns, parts) = self.fetch_through(self.game(target))?;
        (sought.fetch_ns, sought.parts) = (sought.fetch_ns + fetch_ns, sought.parts + parts as u64);
        if target >= next && self.advance_log(target, None)? != Stop::Reached {
            return Err(invalid(format!("Log table {} cannot be read to hour {hour}: a part is missing", self.table)));
        }
        self.retired.clear();
        sought.replay_ns = (replaying.elapsed().as_nanos() as u64).saturating_sub(sought.fetch_ns);
        self.read_status()?;
        Ok(sought)
    }

    /// Parts `0..count` of block `block` read into the log's state from block hour `from` with no
    /// column, and their selected columns decompressed in order for the part after them: its
    /// prefix, and the nanoseconds fetching took.
    fn walk_lean(&mut self, block: usize, count: usize, from: u16) -> io::Result<(Prefix, u64)> {
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let parts = &run.pages[block].tables[self.table].parts;
        let extents = pages.extents(self.run);
        let mut cursor = LogCursor::new(&self.kinds, &[], parts[0].first);
        let (mut lean, mut full, mut fetch_ns) = (None::<Prefix>, None::<Prefix>, 0);
        for index in 0..count {
            let last = parts[index + 1].first - 1;
            let began = Instant::now();
            let walked = reader::fetch(extents, &parts[index], last, &[], lean.as_ref())?;
            let decompressed = reader::fetch(extents, &parts[index], last, &self.columns, full.as_ref())?;
            fetch_ns += began.elapsed().as_nanos() as u64;
            for (id, text) in decompressed.strings() {
                self.texts.push(*id, text);
                self.known.note(*id, text);
            }
            (lean, full) = (Some(walked.prefix()), Some(decompressed.prefix()));
            cursor.push(walked);
        }
        let begins = run.first_hour + (block as u32 * BLOCK) as Hour;
        let state = &mut self.log.as_deref_mut().expect("a log lane").state;
        let mut failed = None;
        let advanced = cursor.advance(parts[count].first - 1, &mut |event| {
            if failed.is_none()
                && let Err(error) = apply(event, from, begins, state, None)
            {
                failed = Some(error);
            }
        })?;
        if let Some(error) = failed {
            return Err(error);
        }
        if advanced != Advanced::Reached {
            return Err(invalid(format!("Log table {}'s parts before hour {} of block {block} do not reach it", self.table, parts[count].first)));
        }
        Ok((full.expect("a part walked"), fetch_ns))
    }

    /// A log's reset: the flag, the head, the status, its count and rows listed, and no row; the
    /// live set is the lane's ([`Lane::log`]), read by a rebuild.
    pub(super) fn reset_log(&self, out: &mut Changes) -> io::Result<()> {
        let head = self.head().ok_or_else(|| invalid("A lane is reset before it is sought"))?;
        let state = &self.log.as_deref().expect("a log lane").state;
        out.begin(self.table, &self.columns, self.keyed, true);
        out.is_log = true;
        out.log.begin(self.run, state.count, self.keyed, &self.columns, &self.cells);
        out.log.finish(state.count, state.live.len(), &self.cells);
        out.status = Some(self.status);
        out.set_head(head, 0, false);
        Ok(())
    }

    /// A log's step to the end of game hour `to`, as far as the parts given reach.
    pub(super) fn step_log(&mut self, to: Hour, out: &mut Changes) -> io::Result<Hour> {
        let Some(next) = self.next else { return Err(invalid("A lane steps before it is sought")) };
        out.begin(self.table, &self.columns, self.keyed, false);
        out.is_log = true;
        let count = self.log.as_deref().expect("a log lane").state.count;
        out.log.begin(self.run, count, self.keyed, &self.columns, &self.cells);
        let last = self.pages.runs()[self.run].hours() - 1;
        let target = (i64::from(to) - i64::from(self.game(0))).clamp(0, i64::from(last)) as u32;
        let stop = if target >= next { self.advance_log(target, Some(&mut out.log))? } else { Stop::Reached };
        let state = &self.log.as_deref().expect("a log lane").state;
        out.log.finish(state.count, state.live.len(), &self.cells);
        std::mem::swap(&mut out.texts, &mut self.texts);
        self.texts.clear();
        let held = self.status;
        out.status_left = self.status_left(held.0)?;
        self.read_status()?;
        if self.status != held {
            out.status = Some(self.status);
        }
        let head = self.head().expect("sought");
        out.set_head(head, (self.game(target) - head).max(0) as u32, stop == Stop::Waiting);
        Ok(head)
    }

    /// Reads every hour through run hour `target` the parts given reach into the log's state, and
    /// into `logged` when given.
    fn advance_log(&mut self, target: u32, mut logged: Option<&mut Logged>) -> io::Result<Stop> {
        let Self { pages, run, table, kinds, columns, ahead, retired, texts, known, log, next: lane_next, .. } = self;
        let run = &pages.runs()[*run];
        let logs = log.as_deref_mut().expect("a log lane");
        let mut next = lane_next.expect("sought");
        while next <= target {
            let block = (next / BLOCK) as usize;
            let page = &run.pages[block];
            let start = block as u32 * BLOCK;
            let end = target.min(start + u32::from(page.hours) - 1);
            let entry = &page.tables[*table];
            let hour = (next - start) as u16;
            if logs.under.as_ref().is_some_and(|under| under.block != block || under.last < hour) {
                pass(logs, retired)?;
            }
            if entry.parts.is_empty() {
                next = end + 1;
                continue;
            }
            if logs.under.is_none() {
                let index = entry.parts.partition_point(|part| part.first <= hour).checked_sub(1).ok_or_else(|| invalid(format!("Log table {table}'s first part in block {block} starts after its hour {hour}")))?;
                match ahead.front() {
                    None => {
                        *lane_next = Some(next);
                        return Ok(Stop::Waiting);
                    }
                    Some(given) if (given.block, given.index) != (block, index) => {
                        return Err(invalid(format!("Log table {table}'s lane holds part {} of block {} where it needs part {index} of block {block}", given.index, given.block)));
                    }
                    Some(_) => {}
                }
                let given = ahead.pop_front().expect("held");
                for (id, text) in given.part.strings() {
                    texts.push(*id, text);
                    known.note(*id, text);
                }
                let mut cursor = LogCursor::new(kinds, columns, hour);
                let (bytes, tracks, last) = (given.part.decompressed, given.part.track_count(), given.part.last);
                cursor.push(given.part);
                logs.under = Some(LogUnder { cursor, block, last, bytes, tracks });
            }
            let under = logs.under.as_mut().expect("entered");
            let to = end.min(start + u32::from(under.last));
            let last = (to - start) as u16;
            let begins = run.first_hour + start as Hour;
            let (state, mut failed) = (&mut logs.state, None);
            let advanced = under.cursor.advance(last, &mut |event| {
                if failed.is_none()
                    && let Err(error) = apply(event, hour, begins, state, logged.as_deref_mut())
                {
                    failed = Some(error);
                }
            })?;
            if let Some(error) = failed {
                return Err(error);
            }
            if advanced != Advanced::Reached {
                return Err(invalid(format!("Log table {table}'s cursor misses hours its part holds")));
            }
            next = to + 1;
            if last == under.last {
                pass(logs, retired)?;
            }
        }
        *lane_next = Some(next);
        Ok(Stop::Reached)
    }
}

/// The part under a log lane's head is passed: checked to hold nothing past it, its cursor
/// retired to be freed on another thread.
fn pass(logs: &mut Logs, retired: &mut Vec<Passed>) -> io::Result<()> {
    let mut under = logs.under.take().expect("a part under the head");
    let mut past = false;
    under.cursor.advance(under.last + 1, &mut |_| past = true)?;
    if past {
        return Err(invalid("A log's part hands over hours past its last"));
    }
    retired.push(Passed::Log(under.cursor));
    Ok(())
}

/// Rows of a log asked for by ordinal: the table, its run and the columns wanted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub run: usize,
    pub table: usize,
    pub ordinals: Vec<Ordinal>,
    pub columns: Vec<usize>,
}

/// What reading a [`Wanted`] took.
#[derive(Clone, Copy, Debug, Default)]
pub struct Read {
    pub blocks: u32,
    /// Parts fetched (each block's from its first: they chain), and walked (holding a row asked).
    pub parts: u32,
    pub walked: u32,
    pub stored: u64,
    pub decompressed: u64,
    /// Fetching the parts (read, verify, decompress, open) and handing their rows over.
    pub fetch_ns: u64,
    pub rows_ns: u64,
}

/// The rows a [`Wanted`] named, owned, in rising ordinal: each one's ordinal, game hour and key,
/// its cells of the columns asked (a column's `slots()` are the rows' places), and the strings
/// its text cells name.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    pub(crate) ordinals: Vec<Ordinal>,
    pub(crate) hours: Vec<Hour>,
    pub(crate) keys: Vec<u128>,
    pub(crate) columns: Vec<Written>,
    pub(crate) texts: Texts,
    pub(crate) read: Read,
}

impl Rows {
    pub fn ordinals(&self) -> &[Ordinal] {
        &self.ordinals
    }

    pub fn hours(&self) -> &[Hour] {
        &self.hours
    }

    pub fn keys(&self) -> &[u128] {
        &self.keys
    }

    /// Per column asked, in the order asked.
    pub fn columns(&self) -> &[Written] {
        &self.columns
    }

    pub fn texts(&self) -> &Texts {
        &self.texts
    }

    pub fn read(&self) -> Read {
        self.read
    }

    pub fn len(&self) -> usize {
        self.ordinals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ordinals.is_empty()
    }
}

/// The rows `wanted` names, read from the parts that appended them: per block holding any, its
/// parts from the first (they chain) through the one appending the highest asked. Every ordinal
/// asked must be a row of the run, refused by name otherwise.
pub fn rows(pages: &dyn Pages, wanted: &Wanted) -> io::Result<Rows> {
    let table = wanted.table;
    let run = pages.runs().get(wanted.run).ok_or_else(|| invalid(format!("No run {}", wanted.run)))?;
    let kinds: Vec<Kind> = pages.kinds(table).ok_or_else(|| invalid(format!("No table {table} in the schema")))?.to_vec();
    if !reader::class_of(&run.pages, table)? {
        return Err(invalid(format!("Table {table} holds state: it has no rows by ordinal")));
    }
    for (at, &column) in wanted.columns.iter().enumerate() {
        if column >= kinds.len() || wanted.columns[..at].contains(&column) {
            return Err(invalid(format!("Column {column} of table {table} is asked twice or not in it")));
        }
    }
    let mut ordinals = wanted.ordinals.clone();
    ordinals.sort_unstable();
    ordinals.dedup();
    let template = TableState::new(&kinds);
    let mut rows = Rows { columns: vec![Written::default(); wanted.columns.len()], ..Rows::default() };
    for (written, &column) in rows.columns.iter_mut().zip(&wanted.columns) {
        written.start(column, template.column(column).expect("a column of the table"), true);
    }
    let extents = pages.extents(wanted.run);
    let mut named = HashSet::new();
    let mut ids = Vec::new();
    let mut from = 0;
    for (index, page) in run.pages.iter().enumerate() {
        let entry = &page.tables[table];
        let end = run.pages.get(index + 1).map_or(Ordinal::MAX, |next| next.tables[table].count);
        let upto = from + ordinals[from..].partition_point(|&ordinal| ordinal < end);
        if upto == from {
            continue;
        }
        let asked = &ordinals[from..upto];
        let holding = |ordinal: Ordinal| entry.parts.partition_point(|part| part.ordinal <= ordinal).checked_sub(1);
        let mut walk: Vec<usize> = asked.iter().filter_map(|&ordinal| holding(ordinal)).collect();
        walk.dedup();
        let Some(&furthest) = walk.last() else {
            from = upto;
            continue;
        };
        let start = run.first_hour + (page.block * BLOCK) as Hour;
        let (mut prefix, mut strings, mut at, mut failed) = (None::<Prefix>, HashMap::new(), 0, None);
        rows.read.blocks += 1;
        for index in 0..=furthest {
            let part_last = entry.parts.get(index + 1).map_or(page.hours - 1, |following| following.first - 1);
            let began = Instant::now();
            let part = reader::fetch(extents, &entry.parts[index], part_last, &wanted.columns, prefix.as_ref())?;
            rows.read.fetch_ns += began.elapsed().as_nanos() as u64;
            (rows.read.parts, rows.read.stored, rows.read.decompressed) = (rows.read.parts + 1, rows.read.stored + part.stored as u64, rows.read.decompressed + part.decompressed as u64);
            prefix = Some(part.prefix());
            for (id, text) in part.strings() {
                strings.entry(*id).or_insert_with(|| text.clone());
            }
            if walk.binary_search(&index).is_err() {
                continue;
            }
            rows.read.walked += 1;
            let began = Instant::now();
            let mut cursor = LogCursor::new(&kinds, &wanted.columns, entry.parts[index].first);
            cursor.push(part);
            let advanced = cursor.advance(part_last, &mut |event| {
                let (hour, ordinal, key, appended, slot) = match event {
                    LogEvent::Appended { hour, ordinal, key, rows, at } => (hour, ordinal, key, rows, at),
                    LogEvent::Forgotten { .. } => return,
                    other @ LogEvent::Named { .. } => {
                        failed.get_or_insert_with(|| unread(&other));
                        return;
                    }
                };
                while at < asked.len() && asked[at] < ordinal {
                    at += 1;
                }
                if at == asked.len() || asked[at] != ordinal || failed.is_some() {
                    return;
                }
                let place = rows.ordinals.len() as u32;
                rows.ordinals.push(ordinal);
                rows.hours.push(start + Hour::from(hour));
                rows.keys.push(key);
                for written in &mut rows.columns {
                    let value = appended.value(written.column, slot);
                    if let Value::Text(id) = value {
                        ids.push(id);
                    }
                    if let Err(error) = written.push(place, value) {
                        failed = Some(error);
                    }
                }
            })?;
            rows.read.rows_ns += began.elapsed().as_nanos() as u64;
            if let Some(error) = failed.take() {
                return Err(error);
            }
            if advanced != Advanced::Reached {
                return Err(invalid(format!("Log table {table}'s part {index} of block {} does not reach its last hour", page.block)));
            }
        }
        for id in ids.drain(..) {
            if named.insert(id)
                && let Some(text) = strings.get(&id)
            {
                rows.texts.push(id, text);
            }
        }
        from = upto;
    }
    if let Some(&absent) = ordinals.iter().find(|&&ordinal| rows.ordinals.binary_search(&ordinal).is_err()) {
        return Err(invalid(format!("Log table {table} holds no row {absent} in run {}", wanted.run)));
    }
    for (written, &column) in rows.columns.iter_mut().zip(&wanted.columns) {
        written.narrow(template.column(column).expect("a column of the table"));
    }
    Ok(rows)
}
