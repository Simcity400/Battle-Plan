use crate::{
    LogState, Timeline,
    changes::{Changes, Logged},
    invalid,
    lane::Lane,
    pages::{Pages, Run, Span, WorldPages},
};
use replay::campaign::Replay;
use replay_columnar::{
    state::Kind,
    store::{TableState, Value},
    unit::{
        BLOCK,
        reader::{self, LogEvent},
    },
};
use replay_data::{BranchId, Hour};
use std::{io, sync::Arc};

#[cfg(test)]
mod tests;

pub fn branch_id(value: &str) -> io::Result<BranchId> {
    value.parse().map_err(|_| invalid(format!("--branch takes a branch id, a whole number, not {value}")))
}

pub fn played(replay: &Replay, pages: &WorldPages, span: Span, branch: Option<BranchId>) -> io::Result<Timeline> {
    match (span, branch) {
        (Span::Whole, branch) => {
            let main = replay.timelines().map(|timelines| timelines.main).or_else(|| replay.index.first().map(|block| block.branch));
            let branch = branch.or(main).ok_or_else(|| invalid("The recording holds no recorded hour"))?;
            let pieces = replay.pieces(branch);
            if pieces.is_empty() {
                return Err(invalid(format!("The recording holds no branch {branch}")));
            }
            Timeline::branch(pages, pages.branches(), branch, &pieces)
        }
        (_, None) => Timeline::new(pages, pages.pieces().to_vec()),
        (_, Some(_)) => Err(invalid("--branch plays a branch of the whole recording: give it without --run or --from")),
    }
}

pub fn game(run: &Run, hour: u32) -> Hour {
    run.first_hour + hour as Hour
}

pub fn held_kinds(pages: &dyn Pages, table: usize) -> &[Kind] {
    let kinds = pages.kinds(table).expect("a schema table");
    &kinds[..pages.held(table).min(kinds.len())]
}

pub fn lane(pages: &Arc<dyn Pages>, table: usize, columns: &[usize]) -> io::Result<Lane> {
    Lane::new(Arc::clone(pages), 0, table, columns, true)
}

pub fn step_to(lane: &mut Lane, run: &Run, to: u32, changes: &mut Changes, mut each: impl FnMut(&Lane, &Changes)) -> io::Result<()> {
    loop {
        lane.fetch_through(game(run, to))?;
        let before = lane.head();
        let head = lane.step(game(run, to), changes)?;
        each(lane, changes);
        if head >= game(run, to) {
            return Ok(());
        }
        if Some(head) == before {
            return Err(invalid(format!("Table {} stalls at hour {head}", lane.table())));
        }
    }
}

pub fn samples(pages: &dyn Pages, run: usize, table: usize, count: usize, (start, end): (u32, u32)) -> Vec<u32> {
    let hours = end + 1;
    let blocks = pages.runs()[run].blocks();
    let restored: Vec<u32> = (0..blocks)
        .filter(|&block| pages.table_entry(run, block, table).is_ok_and(|entry| entry.restore.is_some()))
        .map(|index| (index as u32 + 1) * BLOCK - 1)
        .collect();
    let mut chosen = Vec::new();
    for (at, &end) in restored.iter().enumerate() {
        if end + 1 < hours {
            chosen.push(end + 1);
        }
        if let Some(&following) = restored.get(at + 1) {
            chosen.push(following - 1);
        }
    }
    let wanted = count * 2 / 3;
    if chosen.len() > wanted && wanted > 0 {
        let every = chosen.len() as f64 / wanted as f64;
        chosen = (0..wanted).map(|at| chosen[(at as f64 * every) as usize]).collect();
    }
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64 ^ table as u64;
    chosen.retain(|&hour| hour > start && hour <= end);
    while chosen.len() < count && end > start {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        chosen.push(start + 1 + ((seed >> 33) % u64::from(end - start)) as u32);
    }
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}

pub fn targets(run: &Run, step: u32, sampled: &[u32], (start, end): (u32, u32)) -> Vec<u32> {
    let mut targets: Vec<u32> = (1..).map(|at| start + at * step).take_while(|&hour| hour <= end).collect();
    targets.extend((0..run.blocks()).filter_map(|index| run.block_hours(index).map(|hours| index as u32 * BLOCK + u32::from(hours) - 1)));
    targets.extend(sampled.iter().copied());
    targets.push(end);
    targets.retain(|&hour| hour > start && hour <= end);
    targets.sort_unstable();
    targets.dedup();
    targets
}

pub fn block_end(run: &Run, hour: u32) -> Option<usize> {
    let block = (hour / BLOCK) as usize;
    let hours = run.block_hours(block)?;
    (hour + 1 == block as u32 * BLOCK + u32::from(hours)).then_some(block)
}

pub fn differ(a: &TableState, b: &TableState, columns: &[usize], first: &mut Option<String>) -> u64 {
    let mut differing = 0;
    let slots = a.slots().max(b.slots());
    for slot in 0..slots {
        let (live_a, live_b) = (a.live().get(slot), b.live().get(slot));
        if live_a != live_b || a.key(slot) != b.key(slot) {
            differing += 1;
            first.get_or_insert_with(|| format!("slot {slot}: live {live_a} {live_b}, key {:?} {:?}", a.key(slot), b.key(slot)));
            continue;
        }
        if !live_a {
            continue;
        }
        for &column in columns {
            if a.value(column, slot) != b.value(column, slot) {
                differing += 1;
                first.get_or_insert_with(|| format!("slot {slot} column {column}: {:?} {:?}", a.value(column, slot), b.value(column, slot)));
            }
        }
    }
    differing
}

#[derive(Clone, Debug, Default)]
pub struct Checked {
    pub steps: u64,
    pub block_ends: u64,
    pub a: u64,
    pub b: u64,
    pub refused: u64,
    pub seeks: u64,
    pub c: u64,
    pub d: u64,
    pub appended: u64,
    pub forgotten: u64,
    pub read_rows: u64,
    pub narrow_rows: [u64; 2],
    pub first: Option<String>,
}

#[derive(Default)]
struct Listing {
    count: u64,
    live: Vec<u64>,
    listed: u64,
}

impl Listing {
    fn of(state: &LogState) -> Self {
        let mut listing = Self { count: state.count(), ..Self::default() };
        state.live().ones().for_each(|ordinal| listing.set(ordinal, true));
        listing
    }

    fn get(&self, ordinal: u64) -> bool {
        self.live.get((ordinal / 64) as usize).is_some_and(|word| word >> (ordinal % 64) & 1 == 1)
    }

    fn set(&mut self, ordinal: u64, live: bool) {
        let word = (ordinal / 64) as usize;
        if word >= self.live.len() {
            self.live.resize(word + 1, 0);
        }
        let bit = 1 << (ordinal % 64);
        let was = self.live[word] & bit != 0;
        self.listed = self.listed + u64::from(live && !was) - u64::from(!live && was);
        if live {
            self.live[word] |= bit;
        } else {
            self.live[word] &= !bit;
        }
    }

    fn apply(&mut self, logged: &Logged) -> u64 {
        let appended = logged.appended();
        let mut refused = u64::from(appended.start != self.count);
        for ordinal in appended.clone() {
            refused += u64::from(self.get(ordinal));
            self.set(ordinal, true);
        }
        self.count = self.count.max(appended.end);
        for run in logged.forgotten() {
            for ordinal in run.from..run.to {
                refused += u64::from(!self.get(ordinal));
                self.set(ordinal, false);
            }
        }
        refused + u64::from(logged.count() != self.count || logged.listed() != self.listed)
    }

    fn differ(&self, state: &LogState, first: &mut Option<String>) -> u64 {
        let live = state.live();
        let differing = u64::from(state.count() != self.count) + u64::from(live.len() != self.listed) + live.ones().filter(|&ordinal| !self.get(ordinal)).count() as u64;
        if differing > 0 {
            first.get_or_insert_with(|| format!("log: mirror {} rows, {} live; lane {} rows, {} live", self.count, self.listed, state.count(), live.len()));
        }
        differing
    }
}

fn same_log(a: &LogState, b: &LogState) -> bool {
    a.count() == b.count() && a.live().len() == b.live().len() && a.live().ones().eq(b.live().ones())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogRow {
    Appended(u64, Hour, Option<u128>, Vec<String>),
    Forgotten(Hour, u64, u64),
}

impl LogRow {
    fn rows(&self) -> u64 {
        match self {
            Self::Appended(..) => 1,
            Self::Forgotten(_, from, to) => to - from,
        }
    }

    fn narrowed(&self, columns: &[usize]) -> Self {
        match self {
            Self::Appended(ordinal, hour, key, cells) => Self::Appended(*ordinal, *hour, *key, columns.iter().map(|&column| cells[column].clone()).collect()),
            other => other.clone(),
        }
    }
}

pub fn log_rows(changes: &Changes, columns: &[usize], out: &mut Vec<LogRow>) {
    let Some(logged) = changes.log() else { return };
    let keys = logged.keys();
    for (place, ordinal, hour) in logged.rows() {
        let cells = columns.iter().map(|&column| logged.column(column).map_or_else(|| "not handed".to_owned(), |written| format!("{:?}", written.value(place as usize)))).collect();
        out.push(LogRow::Appended(ordinal, hour, keys.and_then(|keys| keys.get(place as usize).copied()), cells));
    }
    out.extend(logged.forgotten().iter().map(|run| LogRow::Forgotten(run.hour, run.from, run.to)));
}

pub fn read_rows(pages: &dyn Pages, table: usize, columns: &[usize], hours: std::ops::RangeInclusive<Hour>) -> io::Result<Vec<LogRow>> {
    let run = &pages.runs()[0];
    let held = held_kinds(pages, table);
    let read: Vec<usize> = (0..held.len()).collect();
    let table_pages = pages.table_pages(0, table, 0, run.blocks())?;
    let mut rows = Vec::new();
    reader::log_span(pages.extents(0), &table_pages, held, table, &read, run.first_hour, hours, &mut |hour, event| match event {
        LogEvent::Appended { ordinal, key, rows: cells, at, .. } => {
            let cells = columns.iter().map(|&column| format!("{:?}", if column < held.len() { cells.value(column, at) } else { Value::Unrecorded })).collect();
            rows.push(LogRow::Appended(ordinal, hour, Some(key), cells));
        }
        LogEvent::Forgotten { from, to, .. } => rows.push(LogRow::Forgotten(hour, from, to)),
        LogEvent::Named { .. } => {}
    })?;
    Ok(rows)
}

fn joined(mut rows: Vec<LogRow>) -> Vec<LogRow> {
    rows.sort_unstable();
    let mut joined: Vec<LogRow> = Vec::with_capacity(rows.len());
    for row in rows {
        if let (Some(LogRow::Forgotten(hour, _, to)), LogRow::Forgotten(next, from, end)) = (joined.last_mut(), &row)
            && hour == next
            && to == from
        {
            *to = *end;
            continue;
        }
        joined.push(row);
    }
    joined
}

pub fn differ_rows(got: Vec<LogRow>, wanted: Vec<LogRow>, what: &str, first: &mut Option<String>) -> u64 {
    let (got, wanted) = (joined(got), joined(wanted));
    if got == wanted {
        return 0;
    }
    let only_got = got.iter().filter(|row| wanted.binary_search(row).is_err()).count();
    let only_wanted = wanted.iter().filter(|row| got.binary_search(row).is_err()).count();
    first.get_or_insert_with(|| format!("{what}: stepped {:?}, wanted {:?}", got.iter().find(|row| wanted.binary_search(row).is_err()), wanted.iter().find(|row| got.binary_search(row).is_err())));
    (only_got + only_wanted).max(1) as u64
}

fn log_of(lane: &Lane) -> io::Result<LogState> {
    lane.log().cloned().ok_or_else(|| invalid("A log's lane holds no log"))
}

fn against_restore(pages: &dyn Pages, table: usize, block: usize, held: &LogState, checked: &mut Checked) -> io::Result<()> {
    checked.block_ends += 1;
    let table_pages = pages.table_pages(0, table, 0, block + 1)?;
    let rebuilt = reader::log_state(pages.extents(0), &table_pages, held_kinds(pages, table), table, block)?;
    let (count, listed) = (rebuilt.count().unwrap_or(0), rebuilt.listed().ok_or_else(|| invalid("A log's restore lists no rows"))?);
    let wrong = u64::from(count != held.count()) + (0..count.max(held.count())).filter(|&ordinal| listed.get(ordinal) != held.live().get(ordinal)).count() as u64;
    if wrong > 0 {
        checked.first.get_or_insert_with(|| format!("block {block}: the reader lists {count} rows, the lane {}", held.count()));
    }
    checked.a += wrong;
    Ok(())
}

pub fn log_table(pages: &Arc<dyn Pages>, table: usize, window: (u32, u32), step: u32, samples_wanted: usize) -> io::Result<Checked> {
    let run = &pages.runs()[0];
    let kinds = pages.kinds(table).ok_or_else(|| invalid(format!("Table {table} has no schema")))?;
    let all: Vec<usize> = (0..kinds.len()).collect();
    let narrow: [Vec<usize>; 2] = [all.iter().copied().step_by(3).collect(), Vec::new()];
    let (start, end) = window;
    let mut full = lane(pages, table, &all)?;
    let mut lanes = narrow.iter().map(|columns| lane(pages, table, columns)).collect::<io::Result<Vec<_>>>()?;
    for lane in std::iter::once(&mut full).chain(&mut lanes) {
        lane.seek(game(run, start))?;
    }
    let mut checked = Checked::default();
    let mut mirror = Listing::of(&log_of(&full)?);
    if let Some(block) = block_end(run, start) {
        against_restore(&**pages, table, block, &log_of(&full)?, &mut checked)?;
    }
    let sampled = samples(&**pages, 0, table, samples_wanted, window);
    let (mut changes, mut scratch) = (Changes::default(), Changes::default());
    let (mut pending, mut delivered) = (Vec::new(), [Vec::new(), Vec::new()]);
    let mut read_to = game(run, start);
    for target in targets(run, step, &sampled, window) {
        let since = pending.len();
        step_to(&mut full, run, target, &mut changes, |lane, changes| {
            checked.steps += 1;
            let (Some(logged), Some(state)) = (changes.log(), lane.log()) else {
                checked.b += 1;
                checked.first.get_or_insert_with(|| "a log's step hands no log".to_owned());
                return;
            };
            checked.appended += logged.len() as u64;
            checked.forgotten += logged.forgotten().iter().map(|run| run.to - run.from).sum::<u64>();
            checked.refused += mirror.apply(logged);
            checked.b += mirror.differ(state, &mut checked.first);
            log_rows(changes, &all, &mut pending);
        })?;
        let held = log_of(&full)?;
        for (at, (lane, columns)) in lanes.iter_mut().zip(&narrow).enumerate() {
            let got = &mut delivered[at];
            step_to(lane, run, target, &mut scratch, |_, changes| log_rows(changes, columns, got))?;
            let wanted: Vec<LogRow> = pending[since..].iter().map(|row| row.narrowed(columns)).collect();
            checked.narrow_rows[at] += wanted.iter().map(LogRow::rows).sum::<u64>();
            checked.d += differ_rows(std::mem::take(got), wanted, &format!("a lane of {} columns", columns.len()), &mut checked.first);
            if !same_log(&log_of(lane)?, &held) || lane.status() != full.status() {
                checked.d += 1;
                checked.first.get_or_insert_with(|| format!("to {target}: a lane of {} columns holds another log or status", columns.len()));
            }
        }
        let at_block = block_end(run, target);
        if let Some(block) = at_block {
            against_restore(&**pages, table, block, &held, &mut checked)?;
        }
        if at_block.is_some() || target == end {
            let wanted = read_rows(&**pages, table, &all, read_to + 1..=game(run, target))?;
            checked.read_rows += wanted.iter().map(LogRow::rows).sum::<u64>();
            checked.a += differ_rows(std::mem::take(&mut pending), wanted, &format!("to {target} against the reader"), &mut checked.first);
            read_to = game(run, target);
        }
        if sampled.binary_search(&target).is_ok() {
            checked.seeks += 1;
            let mut sought = lane(pages, table, &all)?;
            sought.seek(game(run, target))?;
            if !same_log(&log_of(&sought)?, &held) || sought.status() != full.status() || sought.head() != full.head() {
                checked.c += 1;
                checked.first.get_or_insert_with(|| format!("seek to {target}: log {:?} {:?}", sought.log().map(|log| (log.count(), log.live().len())), (held.count(), held.live().len())));
            }
        }
    }
    Ok(checked)
}
