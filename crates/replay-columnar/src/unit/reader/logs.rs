
use super::{Advanced, Extents, Part, Prefix, SKIP, fetch, inserted_list, numbers};
use crate::{
    invalid,
    state::Kind,
    store::{TableState, Value},
    unit::{BLOCK, BlockPage, get_varint, get_wide, restore as held, unzig, unzig_wide},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    ops::{Range, RangeInclusive},
};

#[derive(Clone, Copy, Debug)]
pub enum LogEvent<'a> {
    Appended { hour: u16, ordinal: u64, key: u128, rows: &'a TableState, at: u32 },
    Forgotten { hour: u16, from: u64, to: u64 },
    Named { id: u32, text: &'a [u8] },
}

pub fn class_of(pages: &[BlockPage], table: usize) -> io::Result<bool> {
    let mut entries = pages.iter().map(|page| page.tables.get(table).map(|entry| entry.log).ok_or_else(|| invalid(format!("No table {table} in a block page"))));
    let Some(first) = entries.next().transpose()? else { return Ok(false) };
    for log in entries {
        if log? != first {
            return Err(invalid(format!("Table {table} is a log in one block of its run and holds state in another")));
        }
    }
    Ok(first)
}

fn not_a_log(table: usize) -> io::Error {
    invalid(format!("Table {table} holds state: it has no rows by hour to read as a log"))
}

pub fn log_state(source: &dyn Extents, pages: &[BlockPage], kinds: &[Kind], table: usize, at: usize) -> io::Result<TableState> {
    log_state_named(source, pages, kinds, table, at).map(|(state, _)| state)
}

pub fn log_state_named(source: &dyn Extents, pages: &[BlockPage], kinds: &[Kind], table: usize, at: usize) -> io::Result<(TableState, HashMap<u32, Vec<u8>>)> {
    if !class_of(pages.get(..=at).ok_or_else(|| invalid("No such block page"))?, table)? {
        return Err(not_a_log(table));
    }
    let entry = pages[at].tables[table].restore.as_ref().ok_or_else(|| invalid(format!("Log table {table} has no restore at its block's end")))?;
    let (state, meta) = super::restore(source, entry, kinds, &[])?;
    Ok((state, meta.strings.into_iter().collect()))
}

pub(super) fn log_restore(source: &dyn Extents, entry: &held::RestoreEntry, kinds: &[Kind]) -> io::Result<(TableState, held::Meta)> {
    super::reading(|reading| {
        let stream = |id: u32| entry.streams.iter().find(|(stream, _)| *stream == id).and_then(|(_, chunks)| chunks.first());
        let (mut raw, mut prefix) = (Vec::new(), Vec::new());
        let meta_chunk = stream(held::META).ok_or_else(|| invalid("A restore holds no meta stream"))?;
        super::chunk_and_prefix(source, meta_chunk, reading, &mut raw, &mut prefix)?;
        let mut table = TableState::log(kinds, 0, Default::default());
        let meta = held::read_meta(&raw[..meta_chunk.frame.raw as usize], kinds, &mut table)?;
        let rows = stream(held::ROWS).ok_or_else(|| invalid("A log's restore holds no live set"))?;
        super::chunk_and_prefix(source, rows, reading, &mut raw, &mut prefix)?;
        let listed = held::read_listed(&raw[..rows.frame.raw as usize], meta.count)?;
        table.set_log_rows(meta.count, listed);
        Ok((table, meta))
    })
}

pub struct LogCursor {
    kinds: Vec<Kind>,
    columns: Vec<usize>,
    list: Vec<bool>,
    parts: VecDeque<Part>,
    passed: Option<Prefix>,
    hour: u16,
    texts: HashMap<u32, Vec<u8>>,
    ordinals: Vec<u64>,
    targets: Vec<u32>,
}

impl LogCursor {
    pub fn new(kinds: &[Kind], columns: &[usize], hour: u16) -> Self {
        Self {
            list: columns.iter().map(|&column| matches!(kinds.get(column), Some(Kind::U32List | Kind::U64List | Kind::Opaque))).collect(),
            kinds: kinds.to_vec(),
            columns: columns.to_vec(),
            parts: VecDeque::new(),
            passed: None,
            hour,
            texts: HashMap::new(),
            ordinals: Vec::new(),
            targets: Vec::new(),
        }
    }

    pub fn reset(&mut self, hour: u16) {
        self.parts.clear();
        self.passed = None;
        self.hour = hour;
        self.texts.clear();
    }

    pub fn push(&mut self, part: Part) {
        for (id, text) in &part.strings {
            self.texts.insert(*id, text.clone());
        }
        self.passed = None;
        self.parts.push_back(part);
    }

    pub fn prefix(&self) -> Option<Prefix> {
        self.parts.back().map(Part::prefix).or_else(|| self.passed.clone())
    }

    pub fn text(&self, id: u32) -> Option<&[u8]> {
        self.texts.get(&id).map(Vec::as_slice)
    }

    pub fn hour(&self) -> u16 {
        self.hour
    }

    pub fn advance(&mut self, to: u16, each: &mut dyn FnMut(LogEvent<'_>)) -> io::Result<Advanced> {
        if self.parts.iter().any(|part| !part.is_log()) {
            return Err(invalid("A state table's part was given to a log's cursor: a table holding state is read with a Cursor"));
        }
        while self.hour <= to {
            let Some(part) = self.parts.front() else {
                return Ok(Advanced::Missing { last: self.hour.checked_sub(1) });
            };
            if self.hour > part.last {
                finished(part)?;
                let part = self.parts.pop_front().expect("a part under way");
                self.passed = (self.parts.is_empty() && u32::from(part.last) + 1 < BLOCK).then(|| part.prefix());
                continue;
            }
            if self.hour < part.first {
                return Err(invalid("A unit part starts after the hour its cursor is at"));
            }
            let end = to.min(part.last);
            self.hours(end, each)?;
            self.hour = end + 1;
        }
        Ok(Advanced::Reached)
    }

    fn hours(&mut self, to: u16, each: &mut dyn FnMut(LogEvent<'_>)) -> io::Result<()> {
        let part = self.parts.front_mut().expect("a part under way");
        let Some(span) = part.lifetimes else { return Ok(()) };
        let frames = part.frames.clone();
        let raw = frames.upto(span);
        let mut life = part.life;
        while life.at < raw.len() {
            let mut at = life.at;
            let hour = u32::from(life.hour) + get_varint(raw, &mut at)?.min(u64::from(BLOCK) + 1) as u32;
            if hour > u32::from(to) {
                break;
            }
            let hour = hour as u16;
            let forgotten = get_varint(raw, &mut at)? as usize;
            let appended = get_varint(raw, &mut at)? as usize;
            if forgotten.saturating_add(appended) > raw.len() {
                return Err(invalid("A log's hour counts more rows than its stream holds"));
            }
            self.ordinals.clear();
            for count in [forgotten, appended] {
                let mut previous = 0_u64;
                for _ in 0..count {
                    previous = previous.checked_add_signed(unzig(get_varint(raw, &mut at)?)).ok_or_else(|| invalid("A log names an ordinal outside 64 bits"))?;
                    self.ordinals.push(previous);
                }
            }
            let (forgets, appends) = self.ordinals.split_at(forgotten);
            if appends.windows(2).any(|pair| pair[1] != pair[0].wrapping_add(1)) {
                return Err(invalid("A log's hour appends rows at ordinals that do not follow one another"));
            }
            if forgets.windows(2).any(|pair| pair[1] <= pair[0]) {
                return Err(invalid("A log's hour forgets its rows out of order or twice"));
            }
            let mut rows = TableState::new(&self.kinds);
            let mut keys = Vec::with_capacity(appends.len());
            for index in 0..appends.len() as u32 {
                life.key = life.key.wrapping_add(unzig_wide(get_wide(raw, &mut at)?) as u128);
                rows.insert_row(index, life.key)?;
                keys.push(life.key);
            }
            self.targets.clear();
            self.targets.extend(0..appends.len() as u32);
            for (selected, &column) in self.columns.iter().enumerate() {
                let Some(opened) = part.columns[selected].as_mut() else {
                    for &slot in &self.targets {
                        rows.unrecord_cell(column, slot)?;
                    }
                    continue;
                };
                if self.list[selected] {
                    let raw = frames.upto(opened.span);
                    for &slot in &self.targets {
                        inserted_list(opened, raw, &mut rows, column, slot)?;
                    }
                } else {
                    let held = rows.numbers(column).ok_or_else(|| invalid("A numeric stream for a column that holds no numbers"))?;
                    numbers::inserted(frames.padded(opened.span), &mut opened.inserted, opened.inserted_end, held, &self.targets)?;
                }
            }
            for (index, (&ordinal, &key)) in appends.iter().zip(&keys).enumerate() {
                let at = index as u32;
                each(LogEvent::Appended { hour, ordinal, key, rows: &rows, at });
            }
            let mut start = 0;
            while start < forgets.len() {
                let mut end = start + 1;
                while end < forgets.len() && forgets[end] == forgets[end - 1] + 1 {
                    end += 1;
                }
                each(LogEvent::Forgotten { hour, from: forgets[start], to: forgets[end - 1] + 1 });
                start = end;
            }
            life.at = at;
            life.hour = hour;
            part.lives += (forgotten + appended) as u64;
        }
        part.life = life;
        Ok(())
    }
}

fn finished(part: &Part) -> io::Result<()> {
    if part.lifetimes.is_some_and(|span| part.life.at != span.end as usize) {
        return Err(invalid("A log's part holds hours past it"));
    }
    if part.columns.iter().flatten().any(|opened| opened.inserted != opened.inserted_end) {
        return Err(invalid("A log's part holds values of more rows than it appends"));
    }
    Ok(())
}


fn hour_of(event: &LogEvent<'_>) -> Option<u16> {
    match *event {
        LogEvent::Appended { hour, .. } | LogEvent::Forgotten { hour, .. } => Some(hour),
        LogEvent::Named { .. } => None,
    }
}

fn row_of<'a>(event: &LogEvent<'a>) -> Option<(u64, &'a TableState, u32)> {
    match *event {
        LogEvent::Appended { ordinal, rows, at, .. } => Some((ordinal, rows, at)),
        LogEvent::Forgotten { .. } | LogEvent::Named { .. } => None,
    }
}

struct Names<'a> {
    source: &'a dyn Extents,
    kinds: &'a [Kind],
    table: usize,
    texts: Vec<usize>,
    handed: HashSet<u32>,
    listed: HashMap<u32, Vec<u8>>,
    restore: Option<&'a held::RestoreEntry>,
    restored: Option<HashMap<u32, Vec<u8>>>,
}

impl<'a> Names<'a> {
    fn new(source: &'a dyn Extents, kinds: &'a [Kind], table: usize, columns: &[usize]) -> Self {
        Self {
            source,
            kinds,
            table,
            texts: columns.iter().copied().filter(|&column| kinds.get(column) == Some(&Kind::Bytes)).collect(),
            handed: HashSet::new(),
            listed: HashMap::new(),
            restore: None,
            restored: None,
        }
    }

    fn block(&mut self, pages: &'a [BlockPage], at: usize) {
        self.listed.clear();
        self.restored = None;
        self.restore = match at {
            0 => pages[0].tables[self.table].first.as_ref(),
            at => pages[at - 1].tables[self.table].restore.as_ref(),
        };
    }

    fn part(&mut self, part: &mut Part) {
        if self.texts.is_empty() {
            return;
        }
        for (id, text) in std::mem::take(&mut part.strings) {
            self.listed.entry(id).or_insert(text);
        }
    }

    fn hand(&mut self, ordinal: u64, rows: &TableState, at: u32, each: &mut dyn FnMut(LogEvent<'_>)) -> io::Result<()> {
        for index in 0..self.texts.len() {
            let Value::Text(id) = rows.value(self.texts[index], at) else { continue };
            if self.handed.contains(&id) {
                continue;
            }
            if !self.listed.contains_key(&id) && self.restored.is_none() {
                let strings = match self.restore {
                    Some(entry) => super::restore(self.source, entry, self.kinds, &[])?.1.strings,
                    None => Vec::new(),
                };
                self.restored = Some(strings.into_iter().collect());
            }
            let text = self.listed.get(&id).or_else(|| self.restored.as_ref()?.get(&id)).ok_or_else(|| {
                invalid(format!("Log table {}'s row {ordinal} names string {id}, which neither its block's parts nor the restore before them list", self.table))
            })?;
            each(LogEvent::Named { id, text });
            self.handed.insert(id);
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments, reason = "the block's place and the call's filters, given apart")]
fn read_block<'a>(
    source: &dyn Extents,
    pages: &'a [BlockPage],
    at: usize,
    kinds: &[Kind],
    table: usize,
    columns: &[usize],
    wanted: &dyn Fn(u16) -> bool,
    keep: &dyn Fn(u16) -> bool,
    names: &mut Names<'a>,
    each: &mut dyn FnMut(u16, LogEvent<'_>),
) -> io::Result<()> {
    let page = &pages[at];
    let entry = &page.tables[table];
    let Some(last) = entry.parts.iter().rposition(|part| wanted(part.first)) else { return Ok(()) };
    names.block(pages, at);
    let mut cursor = LogCursor::new(kinds, columns, entry.parts[0].first);
    let mut next = 0;
    let end = entry.parts.get(last + 1).map_or(page.hours - 1, |following| following.first - 1);
    loop {
        let mut failed = None;
        let advanced = cursor.advance(end, &mut |event| {
            let Some(hour) = hour_of(&event) else { return };
            if failed.is_some() || !keep(hour) {
                return;
            }
            if let Some((ordinal, rows, at)) = row_of(&event) {
                super::record_log_span_row(columns.len());
                if let Err(error) = names.hand(ordinal, rows, at, &mut |named| each(hour, named)) {
                    failed = Some(error);
                    return;
                }
            }
            each(hour, event);
        })?;
        if let Some(error) = failed {
            return Err(error);
        }
        match advanced {
            Advanced::Reached => return Ok(()),
            Advanced::Missing { .. } => {
                let Some(part) = entry.parts.get(next) else { return Ok(()) };
                let part_last = entry.parts.get(next + 1).map_or(page.hours - 1, |following| following.first - 1);
                let mut part = fetch(source, part, part_last, columns, cursor.prefix().as_ref())?;
                names.part(&mut part);
                cursor.push(part);
                next += 1;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Asked<'a> {
    At(&'a [u64]),
    Run(u64, u64),
}

impl Asked<'_> {
    fn get(&self, index: usize) -> Option<u64> {
        match *self {
            Asked::At(ordinals) => ordinals.get(index).copied(),
            Asked::Run(from, to) => from.checked_add(index as u64).filter(|&ordinal| ordinal < to),
        }
    }

    fn last(&self) -> Option<u64> {
        match *self {
            Asked::At(ordinals) => ordinals.last().copied(),
            Asked::Run(from, to) => (to > from).then(|| to - 1),
        }
    }

    fn len(&self) -> u64 {
        match *self {
            Asked::At(ordinals) => ordinals.len() as u64,
            Asked::Run(from, to) => to.saturating_sub(from),
        }
    }

    fn within(&self, from: u64, to: u64) -> Self {
        match *self {
            Asked::At(ordinals) => Asked::At(&ordinals[ordinals.partition_point(|&ordinal| ordinal < from)..ordinals.partition_point(|&ordinal| ordinal < to)]),
            Asked::Run(start, end) => {
                let start = start.max(from);
                Asked::Run(start, end.min(to).max(start))
            }
        }
    }
}

#[inline(always)]
fn wide(raw: &[u8], at: &mut usize) -> io::Result<u128> {
    match raw.get(*at..*at + 2) {
        Some(&[low, _]) if low < 0x80 => {
            *at += 1;
            Ok(u128::from(low))
        }
        Some(&[low, high]) if high < 0x80 => {
            *at += 2;
            Ok(u128::from(low & 0x7f) | u128::from(high) << 7)
        }
        _ => get_wide(raw, at),
    }
}

pub(super) fn sum_keys(raw: &[u8], at: &mut usize, count: u64) -> io::Result<i128> {
    let (mut small, mut large, mut left) = (0_i64, 0_i128, count);
    while left > 0 {
        if left >= 4
            && let Some(word) = raw.get(*at..*at + 8)
        {
            let word = u64::from_le_bytes(word.try_into().expect("eight bytes"));
            let stops = word & 0x8080_8080_8080_8080;
            if left >= 8 && stops == 0 {
                small += (0..8).map(|byte| unzig(word >> (8 * byte) & 0x7f)).sum::<i64>();
                (*at, left) = (*at + 8, left - 8);
                continue;
            }
            if stops == 0x0080_0080_0080_0080 {
                let values = (word & 0x007f_007f_007f_007f) | (word >> 1 & 0x3f80_3f80_3f80_3f80);
                small += (0..4).map(|lane| unzig(values >> (16 * lane) & 0x3fff)).sum::<i64>();
                (*at, left) = (*at + 8, left - 4);
                continue;
            }
        }
        left -= 1;
        match raw.get(*at..*at + 2) {
            Some(&[low, _]) if low < 0x80 => {
                *at += 1;
                small += unzig(u64::from(low));
            }
            Some(&[low, high]) if high < 0x80 => {
                *at += 2;
                small += unzig(u64::from(low & 0x7f) | u64::from(high) << 7);
            }
            _ => large = large.wrapping_add(unzig_wide(get_wide(raw, at)?)),
        }
    }
    Ok(large.wrapping_add(i128::from(small)))
}

#[derive(Clone, Copy)]
struct Found {
    hour: u16,
    key: u128,
    index: u64,
    ordinal: u64,
}

fn part_rows(part: &mut Part, kinds: &[Kind], columns: &[usize], ordinal: u64, asked: Asked<'_>, found: &mut Vec<Found>) -> io::Result<TableState> {
    found.clear();
    let frames = part.frames.clone();
    if let (Some(span), Some(mut target)) = (part.lifetimes, asked.get(0)) {
        let raw = frames.upto(span);
        let (mut at, mut hour, mut key, mut next) = (span.start as usize, u32::from(part.first), 0_u128, ordinal);
        'hours: while at < raw.len() {
            hour += get_varint(raw, &mut at)?.min(u64::from(BLOCK) + 1) as u32;
            if hour > u32::from(part.last) {
                return Err(invalid("A log's part holds hours past it"));
            }
            let forgotten = get_varint(raw, &mut at)? as usize;
            let appended = get_varint(raw, &mut at)? as usize;
            if forgotten.saturating_add(appended) > raw.len() {
                return Err(invalid("A log's hour counts more rows than its stream holds"));
            }
            at = numbers::skip(raw, at, raw.len(), forgotten).ok_or_else(|| invalid("A log's hour forgets more rows than its stream holds"))?;
            if appended == 0 {
                continue;
            }
            let first = 0_u64.checked_add_signed(unzig(get_varint(raw, &mut at)?));
            let follows = raw.get(at..at + appended - 1).is_some_and(|gaps| gaps.iter().fold(true, |follows, &gap| follows & (gap == 2)));
            if first != Some(next) || !follows {
                return Err(invalid("A log's part appends rows at ordinals that do not follow one another"));
            }
            at += appended - 1;
            let (mut row, end) = (next, next + appended as u64);
            while row < end {
                let stop = target.clamp(row, end);
                key = key.wrapping_add(sum_keys(raw, &mut at, stop - row)? as u128);
                row = stop;
                if row == end {
                    break;
                }
                key = key.wrapping_add(unzig_wide(wide(raw, &mut at)?) as u128);
                found.push(Found { hour: hour as u16, key, index: row - ordinal, ordinal: row });
                row += 1;
                match asked.get(found.len()) {
                    Some(following) => target = following,
                    None => break 'hours,
                }
            }
            next = end;
        }
    }
    let mut rows = TableState::new(kinds);
    for (slot, row) in found.iter().enumerate() {
        rows.insert_row(slot as u32, row.key)?;
    }
    for (selected, &column) in columns.iter().enumerate() {
        let Some(opened) = part.columns[selected].as_mut() else {
            for slot in 0..found.len() as u32 {
                rows.unrecord_cell(column, slot)?;
            }
            continue;
        };
        let list = matches!(kinds.get(column), Some(Kind::U32List | Kind::U64List | Kind::Opaque));
        let mut passed = 0_u64;
        for slot in 0..found.len() {
            let index = found[slot].index;
            let skipped = usize::try_from(index - passed).map_err(|_| invalid("A log's part passes the address space"))?;
            if list {
                let raw = frames.upto(opened.span);
                for _ in 0..skipped {
                    inserted_list(opened, raw, &mut rows, column, SKIP)?;
                }
                inserted_list(opened, raw, &mut rows, column, slot as u32)?;
            } else {
                let bytes = frames.padded(opened.span);
                opened.inserted = numbers::skip(bytes, opened.inserted, opened.inserted_end, skipped).ok_or_else(|| invalid("A log's part appends more rows than its column holds values"))?;
                let held = rows.numbers(column).ok_or_else(|| invalid("A numeric stream for a column that holds no numbers"))?;
                numbers::inserted(bytes, &mut opened.inserted, opened.inserted_end, held, &[slot as u32])?;
            }
            passed = index + 1;
        }
    }
    super::record_log_rows_at(found.len(), columns.len());
    Ok(rows)
}

#[allow(clippy::too_many_arguments, reason = "the block's place and the call's state, given apart")]
fn read_rows<'a>(
    source: &dyn Extents,
    pages: &'a [BlockPage],
    at: usize,
    kinds: &[Kind],
    table: usize,
    columns: &[usize],
    asked: Asked<'_>,
    names: &mut Names<'a>,
    each: &mut dyn FnMut(LogEvent<'_>),
) -> io::Result<u64> {
    let page = &pages[at];
    let entry = &page.tables[table];
    let holding = |ordinal: u64| entry.parts.partition_point(|part| part.ordinal <= ordinal).checked_sub(1);
    let Some(furthest) = asked.last().and_then(holding) else { return Ok(0) };
    names.block(pages, at);
    let (mut prefix, mut handed, mut found) = (None::<Prefix>, 0, Vec::new());
    for index in 0..=furthest {
        let part_entry = &entry.parts[index];
        let last = entry.parts.get(index + 1).map_or(page.hours - 1, |following| following.first - 1);
        let mut part = fetch(source, part_entry, last, columns, prefix.as_ref())?;
        prefix = Some(part.prefix());
        names.part(&mut part);
        let wanted = asked.within(part_entry.ordinal, entry.parts.get(index + 1).map_or(u64::MAX, |following| following.ordinal));
        if wanted.len() == 0 {
            continue;
        }
        let rows = part_rows(&mut part, kinds, columns, part_entry.ordinal, wanted, &mut found)?;
        for (slot, row) in found.iter().enumerate() {
            let (hour, ordinal, key, at) = (row.hour, row.ordinal, row.key, slot as u32);
            names.hand(ordinal, &rows, at, each)?;
            each(LogEvent::Appended { hour, ordinal, key, rows: &rows, at });
        }
        handed += found.len() as u64;
        if (found.len() as u64) < wanted.len() && index < furthest {
            let missing = wanted.get(found.len()).expect("a row asked");
            return Err(invalid(format!("Log table {table}'s part {index} of block {} holds no row {missing}, which its ordinals say it holds", page.block)));
        }
    }
    Ok(handed)
}

fn rows_of(source: &dyn Extents, pages: &[BlockPage], kinds: &[Kind], table: usize, columns: &[usize], asked: Asked<'_>, each: &mut dyn FnMut(LogEvent<'_>)) -> io::Result<()> {
    let exact = matches!(asked, Asked::At(_));
    let absent = |ordinal: u64| invalid(format!("Log table {table} appended no row {ordinal} in this run"));
    let mut names = Names::new(source, kinds, table, columns);
    let asked = match asked {
        Asked::At(ordinals) => Asked::At(ordinals),
        Asked::Run(from, to) => Asked::Run(from, to.max(from)),
    };
    if exact && let Some(first) = asked.get(0) && pages.first().is_none_or(|page| first < page.tables[table].count) {
        return Err(absent(first));
    }
    for index in 0..pages.len() {
        let start = pages[index].tables[table].count;
        let end = pages.get(index + 1).map_or(u64::MAX, |next| next.tables[table].count);
        let block = asked.within(start, end);
        if block.len() == 0 {
            continue;
        }
        let handed = read_rows(source, pages, index, kinds, table, columns, block, &mut names, each)?;
        if let Some(missing) = block.get(handed as usize)
            && (exact || index + 1 < pages.len())
        {
            return Err(absent(missing));
        }
    }
    Ok(())
}

pub fn log_rows(source: &dyn Extents, pages: &[BlockPage], kinds: &[Kind], table: usize, columns: &[usize], ordinals: Range<u64>, each: &mut dyn FnMut(LogEvent<'_>)) -> io::Result<()> {
    if !class_of(pages, table)? {
        return Err(not_a_log(table));
    }
    rows_of(source, pages, kinds, table, columns, Asked::Run(ordinals.start, ordinals.end), each)
}

pub fn log_rows_at(source: &dyn Extents, pages: &[BlockPage], kinds: &[Kind], table: usize, columns: &[usize], ordinals: &[u64], each: &mut dyn FnMut(LogEvent<'_>)) -> io::Result<()> {
    if !class_of(pages, table)? {
        return Err(not_a_log(table));
    }
    if let Some(pair) = ordinals.windows(2).find(|pair| pair[1] <= pair[0]) {
        return Err(invalid(format!("Rows of log table {table} are asked twice or out of order: {} after {}", pair[1], pair[0])));
    }
    rows_of(source, pages, kinds, table, columns, Asked::At(ordinals), each)
}

pub fn log_span(source: &dyn Extents, pages: &[BlockPage], kinds: &[Kind], table: usize, columns: &[usize], run_first: i32, hours: RangeInclusive<i32>, each: &mut dyn FnMut(i32, LogEvent<'_>)) -> io::Result<()> {
    if !class_of(pages, table)? {
        return Err(not_a_log(table));
    }
    let mut names = Names::new(source, kinds, table, columns);
    for (index, page) in pages.iter().enumerate() {
        let first = run_first + (page.block * BLOCK) as i32;
        if first > *hours.end() || first + i32::from(page.hours) <= *hours.start() {
            continue;
        }
        let wanted = |hour: u16| first + i32::from(hour) <= *hours.end();
        let keep = |hour: u16| hours.contains(&(first + i32::from(hour)));
        read_block(source, pages, index, kinds, table, columns, &wanted, &keep, &mut names, &mut |hour, event| each(first + i32::from(hour), event))?;
    }
    Ok(())
}
