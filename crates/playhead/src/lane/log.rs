
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

pub type Ordinal = u64;

pub fn is_log(pages: &dyn Pages, table: usize) -> io::Result<bool> {
    let mut class = None;
    for (index, run) in pages.runs().iter().enumerate() {
        if run.blocks() == 0 {
            continue;
        }
        let log = pages.table_entry(index, 0, table)?.log;
        if class.is_some_and(|held| held != log) {
            return Err(invalid(format!("Table {table} is a log in one run and holds state in run {index}")));
        }
        class = Some(log);
    }
    Ok(class == Some(true))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveSet {
    words: VecDeque<(u64, u64)>,
    listed: u64,
    empty: usize,
}

impl LiveSet {
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

    pub fn lowest(&self) -> Option<Ordinal> {
        self.words.iter().find(|(_, bits)| *bits != 0).map(|&(word, bits)| word * 64 + Ordinal::from(bits.trailing_zeros()))
    }

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

    pub(crate) fn append(&mut self, ordinal: Ordinal) {
        let (word, bit) = (ordinal / 64, 1_u64 << (ordinal % 64));
        match self.words.back_mut() {
            Some((last, bits)) if *last == word => *bits |= bit,
            _ => self.words.push_back((word, bit)),
        }
        self.listed += 1;
    }

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

pub(super) struct LogUnder {
    cursor: LogCursor,
    block: usize,
    last: u16,
    pub(super) bytes: usize,
    pub(super) tracks: usize,
}

#[derive(Default)]
pub(super) struct Logs {
    pub(super) state: LogState,
    pub(super) under: Option<LogUnder>,
}

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
        other @ LogEvent::Named { .. } => return Err(unread(&other)),
    }
    Ok(())
}

pub(crate) fn unread(event: &LogEvent<'_>) -> io::Error {
    invalid(format!("A log hands the playhead an event it does not read: {event:?}"))
}

impl Lane {
    pub fn log(&self) -> Option<&LogState> {
        self.log.as_deref().map(|logs| &logs.state)
    }

    pub(super) fn seek_log(&mut self, hour: Hour) -> io::Result<Sought> {
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let target = (i64::from(hour) - i64::from(run.first_hour)).clamp(0, i64::from(run.hours()) - 1) as u32;
        let block = (target / BLOCK) as usize;
        let block_hours = run.block_hours(block).ok_or_else(|| invalid(format!("Run {} holds no block {block}", self.run)))?;
        let target_entry = self.entry_at(block)?;
        let ends_block = target + 1 == block as u32 * BLOCK + u32::from(block_hours);
        let at = if ends_block { Some(block) } else { block.checked_sub(1) };
        let restore_entry = match at {
            Some(at) if at == block => target_entry.clone(),
            Some(at) => pages.table_entry(self.run, at, self.table)?,
            None if block == 0 => target_entry.clone(),
            None => pages.table_entry(self.run, 0, self.table)?,
        };
        let entry = match at {
            Some(_) => restore_entry.restore.as_ref(),
            None => restore_entry.first.as_ref(),
        };
        let entry = entry.ok_or_else(|| invalid(format!("Log table {} has no restore before hour {hour}", self.table)))?;
        let started = Instant::now();
        let (restored, meta) = reader::restore(pages.extents(self.run), entry, &self.kinds, &[])?;
        drop(restore_entry);
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
        let mut next = match at {
            Some(at) => {
                let hours = run.block_hours(at).ok_or_else(|| invalid(format!("Run {} holds no block {at}", self.run)))?;
                at as u32 * BLOCK + u32::from(hours)
            }
            None => 1,
        };
        let next_block = (next / BLOCK) as usize;
        let cached = (next_block == block).then_some((block, &target_entry));
        let mut chain = Chain { next: next_part(&*pages, self.run, self.table, next_block, 0, cached)?, prefix: None };
        self.status_until = 0;
        let mut sought = Sought { restore_ns, restored: at, blocks: u32::from(target >= next), ..Sought::default() };
        let replaying = Instant::now();
        let start = block as u32 * BLOCK;
        let parts = &target_entry.parts;
        let under = parts.partition_point(|part| u32::from(part.first) <= target - start);
        if target >= next && under > 1 {
            let next_first = parts[under - 1].first;
            let (prefix, fetch_ns) = self.walk_lean(block, &target_entry, under - 1, (next - start) as u16)?;
            next = start + u32::from(next_first);
            let hour = run.first_hour + next as Hour;
            chain = Chain { next: Some((block, under - 1, hour)), prefix: Some((block, prefix)) };
            (sought.fetch_ns, sought.parts) = (fetch_ns, under as u64 - 1);
        }
        drop(target_entry);
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

    fn walk_lean(&mut self, block: usize, entry: &crate::pages::EntryRef, count: usize, from: u16) -> io::Result<(Prefix, u64)> {
        let pages = Arc::clone(&self.pages);
        let run = &pages.runs()[self.run];
        let parts = &entry.parts;
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

    fn advance_log(&mut self, target: u32, mut logged: Option<&mut Logged>) -> io::Result<Stop> {
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
            let Self { table, kinds, columns, ahead, retired, texts, known, log, next: lane_next, .. } = self;
            let logs = log.as_deref_mut().expect("a log lane");
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
        self.next = Some(next);
        Ok(Stop::Reached)
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub run: usize,
    pub table: usize,
    pub ordinals: Vec<Ordinal>,
    pub columns: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Read {
    pub blocks: u32,
    pub parts: u32,
    pub walked: u32,
    pub stored: u64,
    pub decompressed: u64,
    pub fetch_ns: u64,
    pub rows_ns: u64,
}

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

pub fn rows(pages: &dyn Pages, wanted: &Wanted) -> io::Result<Rows> {
    let table = wanted.table;
    let run = pages.runs().get(wanted.run).ok_or_else(|| invalid(format!("No run {}", wanted.run)))?;
    let kinds: Vec<Kind> = pages.kinds(table).ok_or_else(|| invalid(format!("No table {table} in the schema")))?.to_vec();
    let table_pages = pages.table_pages(wanted.run, table, 0, run.blocks())?;
    if !reader::class_of(&table_pages, table)? {
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
    for (index, page) in table_pages.iter().enumerate() {
        let entry = &*page.entry;
        let end = table_pages.get(index + 1).map_or(Ordinal::MAX, |next| next.entry.count);
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
