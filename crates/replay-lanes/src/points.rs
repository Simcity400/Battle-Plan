
use replay_recording::{cells::{self, Change, Held, Stretch, TableReader}, columnar_frames::Recording};
use crate::lanes::{Keys, StatusRun};
use replay_data::rows::TableId;
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::Strings,
    summary::{self, Cells, Piece},
    unit::BLOCK,
};
use std::{collections::HashMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub hour: i32,
    pub key: u128,
    pub covered: bool,
    pub value: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cell {
    Number(i64),
    Text(u32),
    Missing,
    Unrecorded,
    Written(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Raw {
    Bits(u64),
    Missing,
    Unrecorded,
    Written(Option<i32>),
}

impl Raw {
    fn of(held: Held, written: Option<i32>) -> Self {
        match held {
            Held::Bits(bits) => Self::Bits(bits),
            Held::Missing => Self::Missing,
            Held::Unrecorded => Self::Unrecorded,
            Held::List => Self::Written(written),
        }
    }

    fn apply(&mut self, change: Change, hour: i32) {
        *self = match change {
            Change::Difference(change) => Self::Bits(match *self {
                Self::Bits(bits) => bits,
                _ => 0,
            }
            .wrapping_add(change as u64)),
            Change::Value(value) => Self::Bits(value),
            Change::Missing => Self::Missing,
            Change::Unrecorded => Self::Unrecorded,
            Change::Written => Self::Written(Some(hour)),
        };
    }
}

fn number(kind: Kind, bits: u64) -> i64 {
    match kind {
        Kind::U8 => i64::from(bits as u8),
        Kind::I8 => i64::from(bits as i8),
        Kind::U16 => i64::from(bits as u16),
        Kind::I16 => i64::from(bits as i16),
        Kind::U32 | Kind::OptionalU32 => i64::from(bits as u32),
        Kind::I32 | Kind::OptionalI32 => i64::from(bits as i32),
        Kind::Bool => (bits & 1) as i64,
        _ => bits as i64,
    }
}

#[derive(Default)]
struct Texts {
    by_id: HashMap<u32, u32, foldhash::fast::RandomState>,
    by_text: HashMap<Box<[u8]>, u32, foldhash::fast::RandomState>,
    held: Vec<(u32, Box<[u8]>)>,
}

impl Texts {
    fn index(&mut self, id: u32, strings: &Strings) -> io::Result<u32> {
        if let Some(&index) = self.by_id.get(&id) {
            return Ok(index);
        }
        let text = strings.get(id).ok_or_else(|| invalid(format!("A text cell names string {id}, which its run does not hold")))?;
        let index = match self.by_text.get(text) {
            Some(&index) => index,
            None => {
                let index = self.held.len() as u32;
                self.held.push((id, text.into()));
                self.by_text.insert(text.into(), index);
                index
            }
        };
        self.by_id.insert(id, index);
        Ok(index)
    }
}

trait Look {
    type State: Clone + PartialEq;

    fn hidden(&self) -> Self::State;

    fn shown(&mut self, key: u128, raw: &[Raw], hour: i32, before: Option<&Self::State>, strings: &Strings) -> io::Result<Self::State>;

    fn texts(&mut self) -> &mut Texts;
}

struct Row<'a> {
    columns: &'a [usize],
    values: &'a [Option<i64>],
    key: u128,
}

impl Cells for Row<'_> {
    fn number(&self, column: usize, _: u32) -> Option<i64> {
        self.values[self.columns.binary_search(&column).ok()?]
    }

    fn key(&self, _: u32) -> Option<u128> {
        Some(self.key)
    }

    fn slot(&self, key: u128) -> Option<u32> {
        (key == self.key).then_some(0)
    }

    fn each_row(&self, each: &mut dyn FnMut(u32)) {
        each(0);
    }
}

struct Valued {
    columns: Vec<usize>,
    kinds: Vec<Kind>,
    source: Option<summary::Source>,
    texts: Texts,
    values: Vec<Option<i64>>,
}

impl Look for Valued {
    type State = (bool, Option<i64>);

    fn hidden(&self) -> Self::State {
        (false, None)
    }

    fn shown(&mut self, key: u128, raw: &[Raw], _: i32, _: Option<&Self::State>, strings: &Strings) -> io::Result<Self::State> {
        let Some(source) = self.source else { return Ok((true, None)) };
        self.values.clear();
        for (at, &column) in self.columns.iter().enumerate() {
            let kind = self.kinds[column];
            self.values.push(match raw[at] {
                Raw::Bits(bits) if cells::numeric(kind) => Some(number(kind, bits)),
                Raw::Bits(id) if matches!(kind, Kind::Bytes) => {
                    let index = self.texts.index(id as u32, strings)?;
                    Some(i64::from(self.texts.held[index as usize].0))
                }
                _ => None,
            });
        }
        let row = Row { columns: &self.columns, values: &self.values, key };
        Ok((true, source.value(&row, 0)))
    }

    fn texts(&mut self) -> &mut Texts {
        &mut self.texts
    }
}

struct Celled {
    columns: Vec<usize>,
    kinds: Vec<Kind>,
    texts: Texts,
}

impl Look for Celled {
    type State = Option<Box<[Cell]>>;

    fn hidden(&self) -> Self::State {
        None
    }

    fn shown(&mut self, _: u128, raw: &[Raw], hour: i32, before: Option<&Self::State>, strings: &Strings) -> io::Result<Self::State> {
        let mut out = Vec::with_capacity(raw.len());
        for (at, &column) in self.columns.iter().enumerate() {
            let kind = self.kinds[column];
            out.push(match raw[at] {
                Raw::Bits(id) if matches!(kind, Kind::Bytes) => Cell::Text(self.texts.index(id as u32, strings)?),
                Raw::Bits(bits) => Cell::Number(number(kind, bits)),
                Raw::Missing => Cell::Missing,
                Raw::Unrecorded => Cell::Unrecorded,
                Raw::Written(Some(written)) => Cell::Written(written),
                Raw::Written(None) => match before {
                    Some(Some(cells)) => match cells[at] {
                        Cell::Written(written) => Cell::Written(written),
                        _ => Cell::Written(hour),
                    },
                    _ => Cell::Written(hour),
                },
            });
        }
        Ok(Some(out.into()))
    }

    fn texts(&mut self) -> &mut Texts {
        &mut self.texts
    }
}

enum Wanted {
    All,
    These(Vec<u128>),
    Range(u128, u128),
    Ranges(Vec<(u128, u128)>),
}

impl Wanted {
    fn of(keys: Keys<'_>) -> Self {
        match keys {
            Keys::All => Self::All,
            Keys::These(keys) => {
                let mut keys = keys.to_vec();
                keys.sort_unstable();
                keys.dedup();
                Self::These(keys)
            }
            Keys::Range(low, high) => Self::Range(low, high),
            Keys::Ranges(ranges) => Self::Ranges(ranges.to_vec()),
        }
    }

    fn wants(&self, key: u128) -> bool {
        match self {
            Self::All => true,
            Self::These(keys) => keys.binary_search(&key).is_ok(),
            Self::Range(low, high) => *low <= key && key <= *high,
            Self::Ranges(ranges) => ranges.get(ranges.partition_point(|range| range.1 < key)).is_some_and(|range| range.0 <= key),
        }
    }

    fn places(&self, keys: &[u128]) -> Vec<usize> {
        match self {
            Self::All => (0..keys.len()).collect(),
            Self::These(wanted) => wanted.iter().filter_map(|key| keys.binary_search(key).ok()).collect(),
            Self::Range(low, high) => (keys.partition_point(|key| key < low)..keys.partition_point(|key| key <= high)).collect(),
            Self::Ranges(ranges) => ranges.iter().flat_map(|(low, high)| keys.partition_point(|key| key < low)..keys.partition_point(|key| key <= high)).collect(),
        }
    }
}

struct Occupant {
    key: u128,
    slot: u32,
    from: u16,
    until: u16,
    raw: Vec<Raw>,
    inserted: bool,
    settled: Option<u16>,
    events: Vec<(u16, usize, Change)>,
}

struct Reading<'a, L: Look> {
    columns: &'a [usize],
    places: &'a [usize],
    wanted: &'a Wanted,
    look: &'a mut L,
    state: HashMap<u128, L::State, foldhash::fast::RandomState>,
}

impl<L: Look> Reading<'_, L> {
    fn raw(&self, held: impl Iterator<Item = Held>, written: Option<i32>) -> Vec<Raw> {
        let mut raw = vec![Raw::Unrecorded; self.columns.len()];
        for (&place, held) in self.places.iter().zip(held) {
            raw[place] = Raw::of(held, written);
        }
        raw
    }

    fn stretch(&mut self, stretch: &Stretch, born: &Born, strings: &Strings) -> io::Result<Vec<(i32, u128, L::State)>> {
        let (from, to) = (stretch.from, stretch.to);
        let at = |hour: u16| usize::from(hour - from);
        let game = |hour: u16| stretch.block_first + i32::from(hour);
        let available: Vec<bool> = stretch.statuses.iter().map(|status| matches!(status, Status::Complete | Status::Truncated { .. })).collect();
        let truncated: Vec<bool> = stretch.statuses.iter().map(|status| matches!(status, Status::Truncated { .. })).collect();
        let mut read = stretch.read.clone();
        read.sort_unstable();
        let mut every: Vec<u16> = vec![from];
        for hour in from + 1..=to {
            let (now, before) = (at(hour), at(hour) - 1);
            if available[now] != available[before] || truncated[now] || truncated[before] {
                every.push(hour);
            }
        }
        let mut occupants: Vec<Occupant> = stretch
            .start
            .iter()
            .map(|(slot, key, held)| Occupant {
                key: *key,
                slot: *slot,
                from,
                until: u16::MAX,
                raw: if self.wanted.wants(*key) { self.raw(held.iter().copied(), None) } else { Vec::new() },
                inserted: false,
                settled: None,
                events: Vec::new(),
            })
            .collect();
        let mut by_slot: HashMap<u32, Vec<usize>, foldhash::fast::RandomState> = HashMap::default();
        for (index, occupant) in occupants.iter().enumerate() {
            by_slot.entry(occupant.slot).or_default().push(index);
        }
        let mut lives: Vec<(u16, bool, u32, u128)> = stretch.deleted.iter().map(|&(hour, slot, key)| (hour, false, slot, key)).collect();
        lives.extend(stretch.inserted.iter().map(|&(hour, slot, key)| (hour, true, slot, key)));
        lives.sort_unstable_by_key(|&(hour, insert, slot, _)| (hour, insert, slot));
        for (hour, insert, slot, key) in lives {
            let held = by_slot.entry(slot).or_default();
            if insert {
                held.push(occupants.len());
                let (raw, settled) = match born.get(&(hour, slot)) {
                    Some(cells) => (cells.clone(), Some(hour)),
                    None if self.wanted.wants(key) => (vec![Raw::Unrecorded; self.columns.len()], None),
                    None => (Vec::new(), None),
                };
                occupants.push(Occupant { key, slot, from: hour, until: u16::MAX, raw, inserted: true, settled, events: Vec::new() });
            } else {
                let index = held
                    .iter()
                    .copied()
                    .rev()
                    .find(|&index| occupants[index].key == key && occupants[index].until == u16::MAX)
                    .ok_or_else(|| invalid(format!("Block {}'s delete of slot {slot} names no live row", stretch.block)))?;
                occupants[index].until = hour;
            }
        }
        for event in &stretch.events {
            let index = by_slot
                .get(&event.slot)
                .and_then(|held| held.iter().copied().find(|&index| occupants[index].from <= event.hour && event.hour < occupants[index].until))
                .ok_or_else(|| invalid(format!("Block {}'s track of slot {} at hour {} names no live row", stretch.block, event.slot, event.hour)))?;
            if self.wanted.wants(occupants[index].key) {
                occupants[index].events.push((event.hour, self.places[event.column], event.change));
            }
        }
        let mut changes = Vec::new();
        for occupant in &mut occupants {
            if !self.wanted.wants(occupant.key) {
                continue;
            }
            occupant.events.sort_by_key(|&(hour, ..)| hour);
            let mut hours: Vec<u16> = occupant.events.iter().map(|&(hour, ..)| hour).collect();
            hours.extend(every.iter().copied().filter(|&hour| occupant.from <= hour && hour < occupant.until));
            if occupant.inserted {
                hours.push(occupant.from);
            }
            if occupant.until <= to {
                hours.push(occupant.until);
            }
            hours.sort_unstable();
            hours.dedup();
            let mut next = 0;
            for hour in hours {
                while let Some(&(at_hour, column, change)) = occupant.events.get(next).filter(|(at_hour, ..)| *at_hour <= hour) {
                    next += 1;
                    if occupant.settled.is_some_and(|settled| at_hour <= settled) {
                        continue;
                    }
                    occupant.raw[column].apply(change, game(at_hour));
                }
                let live = hour < occupant.until;
                let index = at(hour);
                let shown = live
                    && available[index]
                    && (!truncated[index] || read.binary_search(&(hour, occupant.slot)).is_ok() || (occupant.inserted && occupant.from == hour));
                let before = self.state.get(&occupant.key);
                let state = if shown { self.look.shown(occupant.key, &occupant.raw, game(hour), before, strings)? } else { self.look.hidden() };
                if before != Some(&state) {
                    self.state.insert(occupant.key, state.clone());
                    changes.push((game(hour), occupant.key, state));
                }
            }
        }
        changes.sort_unstable_by_key(|&(hour, key, _)| (hour, key));
        Ok(changes)
    }

    fn hide(&mut self, hour: i32) -> Vec<(i32, u128, L::State)> {
        let hidden = self.look.hidden();
        let mut changes: Vec<(i32, u128, L::State)> = self
            .state
            .iter_mut()
            .filter(|(_, state)| **state != hidden)
            .map(|(&key, state)| {
                *state = hidden.clone();
                (hour, key, hidden.clone())
            })
            .collect();
        changes.sort_unstable_by_key(|&(_, key, _)| key);
        changes
    }
}

type Born = HashMap<(u16, u32), Vec<Raw>, foldhash::fast::RandomState>;

fn born<L: Look>(recording: &Recording, (run, table): (u32, TableId), probe: &mut Option<TableReader>, stretch: &Stretch, reading: &Reading<'_, L>) -> io::Result<Born> {
    let mut out = Born::default();
    for &(hour, slot, key) in &stretch.inserted {
        if !reading.wanted.wants(key) {
            continue;
        }
        let probe = match probe {
            Some(probe) => probe,
            None => probe.insert({
                let mut reader = TableReader::new(recording, run, table, Some(reading.columns))?;
                reader.index_keys();
                reader
            }),
        };
        probe.go_to(recording, stretch.block * BLOCK + u32::from(hour))?;
        let Some(at) = probe.slot(key) else { continue };
        let view = probe.view();
        let raw = reading.raw(probe.columns().iter().map(|&column| Held::of(&view, column, at)), Some(stretch.block_first + i32::from(hour)));
        out.insert((hour, slot), raw);
    }
    Ok(out)
}

fn blocks_read(recording: &Recording, run: u32, table: TableId, wanted: &Wanted, blocks: std::ops::Range<u32>) -> io::Result<Option<Vec<bool>>> {
    let by_key = |shape: &summary::Shape| !matches!(shape, summary::Shape::Log(_));
    if matches!(wanted, Wanted::All) || blocks.is_empty() || !replay_recording::summaries::registry()?.iter().any(|(held, shape)| *held == table.index() && by_key(shape)) {
        return Ok(None);
    }
    let held = recording.run(run)?;
    let first_hour = i64::from(held.first_hour);
    let count = blocks.len();
    let (mut summarized, mut covered) = (vec![false; count], vec![false; count]);
    let block_of = |hour: i64| ((hour - first_hour).max(0) / i64::from(BLOCK)).clamp(i64::from(blocks.start), i64::from(blocks.end - 1)) as usize - blocks.start as usize;
    for extent in crate::lanes::pieces(recording, run, table.index(), blocks.clone(), 1)? {
        let bytes = recording.stored_bytes(&extent)?;
        let piece = Piece::read(&bytes)?;
        let directory = &piece.directory;
        let (start, end) = (directory.first_hour, directory.first_hour + i64::from(directory.hours));
        for block in blocks.clone() {
            let (first, hours) = held.block(block as usize);
            let (begin, past) = (first_hour + i64::from(first), first_hour + i64::from(first + hours));
            if start <= begin && past <= end {
                summarized[(block - blocks.start) as usize] = true;
            }
        }
        let size = summary::bucket_hours(directory.level);
        for at in wanted.places(&piece.keys) {
            for (bucket, life) in piece.life(at)?.iter().enumerate() {
                if life.hours == 0 {
                    continue;
                }
                let bucket_start = (directory.first_bucket + bucket as i64) * size;
                let (lived, past) = (bucket_start + i64::from(life.first), bucket_start + i64::from(life.end));
                if past <= first_hour + i64::from(blocks.start * BLOCK) || lived >= first_hour + i64::from(blocks.end * BLOCK) {
                    continue;
                }
                for block in block_of(lived)..=block_of(past - 1) {
                    covered[block] = true;
                }
            }
        }
    }
    Ok(Some(summarized.iter().zip(&covered).map(|(&summarized, &covered)| !summarized || covered).collect()))
}

fn ordinals(recording: &Recording, run: u32, from: i32, to: i32) -> io::Result<(u32, u32)> {
    let held = recording.run(run)?;
    let ordinal = |hour: i32| held.ordinal(hour).ok_or_else(|| invalid("The run holds no such hour"));
    Ok((ordinal(from)?, ordinal(to)?))
}

fn push_complete(out: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    if first > last {
        return;
    }
    match out.last_mut() {
        Some((_, prev)) if *prev + 1 >= first => *prev = (*prev).max(last),
        _ => out.push((first, last)),
    }
}

fn push_status(out: &mut Vec<StatusRun>, first: i32, last: i32, status: Status) {
    let first = out.last().map_or(first, |prev| first.max(prev.last + 1));
    if first > last {
        return;
    }
    match out.last_mut() {
        Some(prev) if prev.status == status && prev.last + 1 == first => prev.last = last,
        _ => out.push(StatusRun { first, last, status }),
    }
}

pub(crate) fn table_statuses(recording: &Recording, run: u32, table: TableId) -> io::Result<Vec<StatusRun>> {
    let held = recording.run(run)?;
    let mut out = Vec::new();
    for block in 0..held.pages() as u32 {
        let entry = recording.table_entry(run, block, table.index())?.unit;
        replay_columnar::unit::reader::statuses(&entry, held.block(block as usize).1 as u16, held.block_first_hour(block), &mut |first, last, status| {
            push_status(&mut out, first, last, status);
        })?;
    }
    Ok(out)
}

fn read<L: Look>(
    recording: &Recording,
    (run, table): (u32, TableId),
    columns: &[usize],
    keys: Keys<'_>,
    (from, to): (i32, i32),
    look: impl FnOnce(Vec<usize>, Vec<Kind>) -> L,
    initial_witness: bool,
    mut statuses: Option<&mut Vec<StatusRun>>,
    each: &mut dyn FnMut(i32, u128, &L::State) -> bool,
) -> io::Result<(Vec<usize>, Vec<Box<[u8]>>)> {
    let (mut first, last) = ordinals(recording, run, from, to)?;
    let mut from = from;
    if recording.predates(table.index()).is_none() {
        match recording.table_coverage(table.index(), run) {
            replay_recording::columnar_frames::TableCoverage::Valid => {}
            replay_recording::columnar_frames::TableCoverage::WithheldAll(reason) => {
                return Err(replay_recording::columnar_frames::Damage::wrap(
                    replay_recording::columnar_frames::DamageScope::Withheld { table: table.index() as u32 },
                    invalid(reason.to_owned()),
                ));
            }
            replay_recording::columnar_frames::TableCoverage::WithheldBefore { ordinal, reason } => {
                if last < ordinal {
                    return Err(replay_recording::columnar_frames::Damage::wrap(
                        replay_recording::columnar_frames::DamageScope::Withheld { table: table.index() as u32 },
                        invalid(reason.to_owned()),
                    ));
                }
                if first < ordinal {
                    from += (ordinal - first) as i32;
                    first = ordinal;
                }
            }
        }
    }
    let mut asked = columns.to_vec();
    asked.sort_unstable();
    asked.dedup();
    let mut reader = TableReader::new(recording, run, table, Some(&asked))?;
    reader.index_keys();
    reader.keep_tracks();
    reader.seek(recording, first)?;
    let kinds = reader.kinds().to_vec();
    asked.retain(|&column| column < kinds.len());
    let places: Vec<usize> = reader.columns().iter().map(|column| asked.binary_search(column).expect("a reader's columns are asked")).collect();
    let wanted = Wanted::of(keys);
    let mut look = look(asked.clone(), kinds.clone());
    let mut follow = || -> io::Result<()> {
        let mut reading = Reading { columns: &asked, places: &places, wanted: &wanted, look: &mut look, state: HashMap::default() };
        {
            if let Some(out) = statuses.as_deref_mut() {
                push_status(out, from, from, reader.status());
            }
            let view = reader.view();
            let read = if first == 0 && !initial_witness && matches!(reader.status(), Status::Truncated { .. }) {
                Some(&[][..])
            } else {
                reader.read_rows()
            };
            let available = reader.available();
            let mut start: Vec<(u128, L::State)> = Vec::new();
            let mut state = |key: u128, slot: Option<u32>| -> io::Result<()> {
                let state = match slot.filter(|&slot| available && read.is_none_or(|read| read.binary_search(&slot).is_ok())) {
                    Some(slot) => {
                        let raw = reading.raw(reader.columns().iter().map(|&column| Held::of(&view, column, slot)), Some(from));
                        reading.look.shown(key, &raw, from, None, reader.strings())?
                    }
                    None => reading.look.hidden(),
                };
                start.push((key, state));
                Ok(())
            };
            match &wanted {
                Wanted::These(keys) => keys.iter().try_for_each(|&key| state(key, view.slot(key)))?,
                _ => view.rows().try_for_each(|slot| match view.key(slot) {
                    Some(key) if wanted.wants(key) => state(key, Some(slot)),
                    _ => Ok(()),
                })?,
            }
            start.sort_unstable_by_key(|(key, _)| *key);
            for (key, state) in start {
                if !each(from, key, &state) {
                    return Ok(());
                }
                reading.state.insert(key, state);
            }
        }
        let first_block = (first + 1).min(last) / BLOCK;
        let needed = if first < last { blocks_read(recording, run, table, &wanted, first_block..last / BLOCK + 1)? } else { None };
        let needs = |block: u32| needed.as_ref().is_none_or(|needed| needed[(block - first_block) as usize]);
        let held = recording.run(run)?;
        let mut probe = None;
        let mut at = first;
        while at < last {
            let block = (at + 1) / BLOCK;
            if !needs(block) {
                for (hour, key, state) in reading.hide(held.first_hour + (at + 1) as i32) {
                    if !each(hour, key, &state) {
                        return Ok(());
                    }
                }
                let mut resume = block;
                while resume <= last / BLOCK && !needs(resume) {
                    resume += 1;
                }
                let next_at = last.min(resume * BLOCK - 1);
                if let Some(out) = statuses.as_deref_mut() {
                    let skip_first = held.first_hour + (at + 1) as i32;
                    let skip_last = held.first_hour + next_at as i32;
                    for skipped in block..resume {
                        let entry = recording.table_entry(run, skipped, table.index())?.unit;
                        replay_columnar::unit::reader::statuses(&entry, held.block(skipped as usize).1 as u16, held.block_first_hour(skipped), &mut |start, end, status| {
                            push_status(out, start.max(skip_first), end.min(skip_last), status);
                        })?;
                    }
                }
                at = next_at;
                if at < last {
                    reader.go_to(recording, at)?;
                }
                continue;
            }
            let end = last.min((block + 1) * BLOCK - 1);
            let stretch = reader.read_stretch(recording, end)?;
            if let Some(out) = statuses.as_deref_mut() {
                for (offset, &status) in stretch.statuses.iter().enumerate() {
                    let hour = stretch.block_first + i32::from(stretch.from) + offset as i32;
                    push_status(out, hour, hour, status);
                }
            }
            let born = born(recording, (run, table), &mut probe, &stretch, &reading)?;
            for (hour, key, state) in reading.stretch(&stretch, &born, reader.strings())? {
                if !each(hour, key, &state) {
                    return Ok(());
                }
            }
            at = end;
        }
        Ok(())
    };
    follow()?;
    Ok((asked, std::mem::take(&mut look.texts().held).into_iter().map(|(_, text)| text).collect()))
}

pub fn points(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: &[usize],
    source: Option<summary::Source>,
    keys: Keys<'_>,
    from: i32,
    to: i32,
    each: &mut dyn FnMut(Point) -> bool,
) -> io::Result<()> {
    read(
        recording,
        (run, table),
        columns,
        keys,
        (from, to),
        |columns, kinds| Valued { columns, kinds, source, texts: Texts::default(), values: Vec::new() },
        false,
        None,
        &mut |hour, key, &(covered, value): &(bool, Option<i64>)| each(Point { hour, key, covered, value }),
    )?;
    Ok(())
}

pub fn witnessed_points(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: &[usize],
    source: Option<summary::Source>,
    keys: Keys<'_>,
    from: i32,
    to: i32,
    each: &mut dyn FnMut(Point) -> bool,
) -> io::Result<()> {
    read(
        recording,
        (run, table),
        columns,
        keys,
        (from, to),
        |columns, kinds| Valued { columns, kinds, source, texts: Texts::default(), values: Vec::new() },
        true,
        None,
        &mut |hour, key, &(covered, value): &(bool, Option<i64>)| each(Point { hour, key, covered, value }),
    )?;
    Ok(())
}

pub fn points_with_complete(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: &[usize],
    source: Option<summary::Source>,
    keys: Keys<'_>,
    from: i32,
    to: i32,
    each: &mut dyn FnMut(Point) -> bool,
) -> io::Result<Vec<(i32, i32)>> {
    let mut statuses = Vec::new();
    read(
        recording,
        (run, table),
        columns,
        keys,
        (from, to),
        |columns, kinds| Valued { columns, kinds, source, texts: Texts::default(), values: Vec::new() },
        true,
        Some(&mut statuses),
        &mut |hour, key, &(covered, value): &(bool, Option<i64>)| each(Point { hour, key, covered, value }),
    )?;
    let mut complete = Vec::new();
    for held in statuses.into_iter().filter(|held| matches!(held.status, Status::Complete)) {
        push_complete(&mut complete, held.first, held.last);
    }
    Ok(complete)
}

pub fn cells(
    recording: &Recording,
    run: u32,
    table: TableId,
    columns: &[usize],
    keys: Keys<'_>,
    from: i32,
    to: i32,
    statuses: Option<&mut Vec<StatusRun>>,
    each: &mut dyn FnMut(i32, u128, Option<&[Cell]>) -> bool,
) -> io::Result<(Vec<usize>, Vec<Box<[u8]>>)> {
    read(
        recording,
        (run, table),
        columns,
        keys,
        (from, to),
        |columns, kinds| Celled { columns, kinds, texts: Texts::default() },
        true,
        statuses,
        &mut |hour, key, cells: &Option<Box<[Cell]>>| each(hour, key, cells.as_deref()),
    )
}
