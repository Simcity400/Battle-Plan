
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::Writer as Bytes,
    manifest::Status,
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::rows::{GroupSpanRow as Span, TableId, TagHolderRow, WarEventRow};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const READS: &[Read] = &[
    Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: false },
    Read { table: TableId::HumanControllers, columns: Columns::These(&[0]), watch: Watch::Changed, required: false },
    Read { table: TableId::Factions, columns: Columns::These(&[0, 1]), watch: Watch::Changed, required: false },
    Read { table: TableId::FactionMembers, columns: Columns::These(&[0, 1]), watch: Watch::Changed, required: false },
    Read { table: TableId::WarRelations, columns: Columns::These(&[0, 1, 7]), watch: Watch::Changed, required: false },
    Read { table: TableId::Capitulation, columns: Columns::These(&[0, 1]), watch: Watch::Changed, required: false },
    Read { table: TableId::LexicalTokens, columns: Columns::These(&[0]), watch: Watch::Changed, required: false },
];

type Fact = (u8, u32, u32);

#[derive(Default)]
struct History {
    open: HashMap<Fact, (u32, u32)>,
}

#[derive(Default)]
pub struct Groups {
    rebuild: bool,
    links: HashMap<u32, u32>,
    links_known: bool,
    humans: History,
    factions: History,
    wars: History,
    epochs: u32,
    numbers: HashMap<u64, u32>,
    names: HashMap<(u32, Option<u32>, u32), u32>,
    starts: HashSet<(u32, u32, i32)>,
    capitulations: HashSet<(u32, i32)>,
    events: Vec<u128>,
    deadline: Option<i32>,
    statuses: Vec<Option<Status>>,
    empty: u32,
}

fn cell(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::U8 | Kind::I8 | Kind::U32 | Kind::I32 | Kind::U64 | Kind::OptionalU32, Held::Number(bits)) => Some(bits as i64),
        (Kind::OptionalU32, Held::Missing) => Some(-1),
        (Kind::Bytes, Held::Text(id)) => Some(i64::from(id)),
        _ => None,
    }
}

fn optional_u32(table: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::OptionalU32, Held::Number(bits)) => Some(bits as u32),
        _ => None,
    }
}

fn ongoing(start: i32, end: i32, hour: i32) -> bool {
    end <= start || hour < end
}

fn span_key((kind, group, value): Fact, epoch: u32) -> u128 {
    u128::from(kind) << 96 | u128::from(group) << 64 | u128::from(value) << 32 | u128::from(epoch)
}

impl Groups {
    fn epoch(&mut self) -> io::Result<u32> {
        let epoch = self.epochs;
        self.epochs = self.epochs.checked_add(1).ok_or_else(|| invalid("Group rows pass 2^32 in one run"))?;
        Ok(epoch)
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn settle(&mut self, which: usize, now: BTreeMap<Fact, u32>, out: &mut Rows) -> io::Result<()> {
        let mut history = std::mem::take(match which {
            0 => &mut self.humans,
            1 => &mut self.factions,
            _ => &mut self.wars,
        });
        history.open.retain(|fact, (epoch, _)| {
            let keep = now.contains_key(fact);
            if !keep {
                out.delete(TableId::GroupSpans, span_key(*fact, *epoch));
            }
            keep
        });
        for (fact, ordinal) in now {
            match history.open.get_mut(&fact) {
                Some((epoch, held)) => {
                    if *held != ordinal {
                        *held = ordinal;
                        out.set(TableId::GroupSpans, span_key(fact, *epoch), vec![Value::U32(ordinal)]);
                    }
                }
                None => {
                    let epoch = self.epoch()?;
                    out.set(TableId::GroupSpans, span_key(fact, epoch), vec![Value::U32(ordinal)]);
                    history.open.insert(fact, (epoch, ordinal));
                }
            }
        }
        *match which {
            0 => &mut self.humans,
            1 => &mut self.factions,
            _ => &mut self.wars,
        } = history;
        Ok(())
    }

    fn unobserved(group: u32) -> BTreeMap<Fact, u32> {
        BTreeMap::from([((Span::UNOBSERVED, group, 0), 0)])
    }

    fn humans(&mut self, hour: &Hour<'_>) -> io::Result<BTreeMap<Fact, u32>> {
        if !self.links_known || !hour.complete(TableId::HumanControllers) {
            return Ok(Self::unobserved(Span::HUMANS));
        }
        let rows = hour.table(TableId::HumanControllers);
        let mut now = BTreeMap::new();
        for slot in rows.rows() {
            let (Some(raw), Some(count)) = (rows.key(slot), cell(&rows, 0, slot)) else { continue };
            if count <= 0 {
                continue;
            }
            match self.slot(raw as u32) {
                Some(member) => now.insert((Span::MEMBER, Span::HUMANS, member), 0),
                None => now.insert((Span::UNOBSERVED, Span::HUMANS, 0), 0),
            };
        }
        Ok(now)
    }

    fn factions(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<BTreeMap<Fact, u32>> {
        if !self.links_known || !hour.complete(TableId::Factions) || !hour.complete(TableId::FactionMembers) {
            return Ok(Self::unobserved(Span::FACTIONS));
        }
        let factions = hour.table(TableId::Factions);
        let lexical = hour.complete(TableId::LexicalTokens).then(|| hour.table(TableId::LexicalTokens));
        let mut now = BTreeMap::new();
        let mut present: HashMap<u64, u32> = HashMap::new();
        for slot in factions.rows() {
            let Some(id) = factions.key(slot).map(|key| key as u64) else { continue };
            let next = self.numbers.len() as u32 + 1;
            let number = *self.numbers.entry(id).or_insert_with(|| {
                out.set(TableId::Groups, u128::from(next), vec![Value::U64(id)]);
                next
            });
            present.insert(id, number);
            now.insert((Span::PRESENT, number, 0), 0);
            let literal = cell(&factions, 0, slot).unwrap_or(0) as u32;
            let token = optional_u32(&factions, 1, slot);
            let spelling = match (token, &lexical) {
                (Some(token), Some(lexical)) => lexical.slot(u128::from(token)).and_then(|at| cell(lexical, 0, at)).map(|id| id as u32),
                _ => None,
            };
            let operands = (literal, token, spelling.unwrap_or(u32::MAX));
            let next = self.names.len() as u32;
            let name = *self.names.entry(operands).or_insert_with(|| {
                let spelling = spelling.unwrap_or(self.empty);
                out.set(TableId::GroupNames, u128::from(next), vec![Value::Bytes(literal), Value::OptionalU32(token), Value::Bytes(spelling)]);
                next
            });
            now.insert((Span::NAME, number, name), 0);
        }
        let members = hour.table(TableId::FactionMembers);
        for slot in members.rows() {
            let (Some(raw), Some(faction), Some(ordinal)) = (members.key(slot), cell(&members, 0, slot), cell(&members, 1, slot)) else { continue };
            let Some(&number) = present.get(&(faction as u64)) else { continue };
            match self.slot(raw as u32) {
                Some(member) => {
                    now.insert((Span::MEMBER, number, member), ordinal as u32);
                    if ordinal == 0 {
                        now.insert((Span::LEADER, number, member), 0);
                    }
                }
                None => {
                    now.insert((Span::INCOMPLETE, number, 0), 0);
                }
            }
        }
        Ok(now)
    }

    fn wars(&mut self, seen: &Hour<'_>, out: &mut Rows) -> io::Result<BTreeMap<Fact, u32>> {
        let hour = seen.hour;
        self.deadline = None;
        if !self.links_known || !seen.complete(TableId::WarRelations) {
            return Ok(Self::unobserved(Span::WARS));
        }
        let rows = seen.table(TableId::WarRelations);
        let mut now = BTreeMap::new();
        for slot in rows.rows() {
            let (Some(key), Some(start), Some(end), Some(instigator)) = (rows.key(slot), cell(&rows, 0, slot), cell(&rows, 1, slot), cell(&rows, 7, slot)) else {
                continue;
            };
            let (first, second, start, end) = ((key >> 32) as u32, key as u32, start as i32, end as i32);
            let (a, b) = (self.slot(first), self.slot(second));
            if self.starts.insert((first, second, start)) {
                let (attacking, defending) = if instigator != 0 { (a, b) } else { (b, a) };
                let key = u128::from(WarEventRow::START) << 96
                    | u128::from(start as u32) << 64
                    | u128::from(attacking.unwrap_or(TagHolderRow::NONE)) << 32
                    | u128::from(defending.unwrap_or(TagHolderRow::NONE));
                out.set(TableId::WarEvents, key, vec![Value::U8(0)]);
                self.events.push(key);
            }
            if end > start && end > hour {
                self.deadline = Some(self.deadline.map_or(end, |held| held.min(end)));
            }
            if !ongoing(start, end, hour) {
                continue;
            }
            match (a, b) {
                (Some(a), Some(b)) => now.insert((Span::WAR, a.min(b), a.max(b)), 0),
                _ => now.insert((Span::UNOBSERVED, Span::WARS, 0), 0),
            };
        }
        Ok(now)
    }

    fn capitulations(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if !self.links_known || !hour.complete(TableId::Capitulation) {
            return Ok(());
        }
        let rows = hour.table(TableId::Capitulation);
        for slot in rows.rows() {
            let (Some(raw), Some(flag), Some(when)) = (rows.key(slot), cell(&rows, 0, slot), cell(&rows, 1, slot)) else { continue };
            if flag == 0 || !self.capitulations.insert((raw as u32, when as i32)) {
                continue;
            }
            let country = self.slot(raw as u32).unwrap_or(TagHolderRow::NONE);
            let key = u128::from(WarEventRow::CAPITULATION) << 96 | u128::from(when as u32) << 64 | u128::from(country) << 32;
            out.set(TableId::WarEvents, key, vec![Value::U8(0)]);
            self.events.push(key);
        }
        Ok(())
    }

    fn moved(&mut self, hour: &Hour<'_>, at: usize, table: TableId, columns: Columns) -> bool {
        let status = Some(hour.status(table));
        let restated = self.statuses[at] != status;
        self.statuses[at] = status;
        restated || hour.wrote(table, columns)
    }
}

impl Writer for Groups {
    fn name(&self) -> &'static str {
        "Groups"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::GroupSpans), Output::State(TableId::Groups), Output::State(TableId::GroupNames), Output::State(TableId::WarEvents)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn strings(&self) -> &'static [&'static str] {
        &[""]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        self.empty = hour.strings[0];
        for key in self.events.drain(..) {
            out.delete(TableId::WarEvents, key);
        }
        if self.statuses.is_empty() {
            self.statuses = vec![None; READS.len()];
        }
        let whole = hour.fresh || std::mem::take(&mut self.rebuild);
        let mut moved = [false; READS.len()];
        for (at, read) in READS.iter().enumerate() {
            moved[at] = self.moved(hour, at, read.table, read.columns) || whole;
        }
        let [links, humans, factions, members, wars, capitulations, lexical] = moved;
        if links {
            self.links_known = hour.complete(TableId::CountryLinks);
            self.links.clear();
            let held = hour.table(TableId::CountryLinks);
            for slot in held.rows() {
                let (Some(key), Some(canonical)) = (held.key(slot), cell(&held, 0, slot)) else { continue };
                self.links.insert(key as u32, canonical as u32);
            }
        }
        if links || humans {
            let now = self.humans(hour)?;
            self.settle(0, now, out)?;
        }
        if links || factions || members || lexical {
            let now = self.factions(hour, out)?;
            self.settle(1, now, out)?;
        }
        if links || wars || self.deadline == Some(hour.hour) {
            let now = self.wars(hour, out)?;
            self.settle(2, now, out)?;
        }
        if links || capitulations {
            self.capitulations(hour, out)?;
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        blob.varint(u64::from(self.epochs))?;
        for history in [&self.humans, &self.factions, &self.wars] {
            let mut open: Vec<(&Fact, &(u32, u32))> = history.open.iter().collect();
            open.sort_unstable();
            blob.varint(open.len() as u64)?;
            for (&(kind, group, value), &(epoch, ordinal)) in open {
                blob.u8(kind)?;
                for number in [group, value, epoch, ordinal] {
                    blob.varint(u64::from(number))?;
                }
            }
        }
        let mut numbers: Vec<(&u64, &u32)> = self.numbers.iter().collect();
        numbers.sort_unstable();
        blob.varint(numbers.len() as u64)?;
        for (&faction, &number) in numbers {
            blob.varint(faction)?;
            blob.varint(u64::from(number))?;
        }
        let mut names: Vec<(&(u32, Option<u32>, u32), &u32)> = self.names.iter().collect();
        names.sort_unstable();
        blob.varint(names.len() as u64)?;
        for (&(literal, token, spelling), &number) in names {
            blob.varint(u64::from(literal))?;
            blob.varint(token.map_or(0, |token| u64::from(token) + 1))?;
            blob.varint(u64::from(spelling))?;
            blob.varint(u64::from(number))?;
        }
        let mut starts: Vec<&(u32, u32, i32)> = self.starts.iter().collect();
        starts.sort_unstable();
        blob.varint(starts.len() as u64)?;
        for &(first, second, start) in starts {
            blob.varint(u64::from(first))?;
            blob.varint(u64::from(second))?;
            blob.varint(u64::from(start as u32))?;
        }
        let mut capitulations: Vec<&(u32, i32)> = self.capitulations.iter().collect();
        capitulations.sort_unstable();
        blob.varint(capitulations.len() as u64)?;
        for &(country, when) in capitulations {
            blob.varint(u64::from(country))?;
            blob.varint(u64::from(when as u32))?;
        }
        blob.varint(self.events.len() as u64)?;
        for &key in &self.events {
            blob.varint((key >> 64) as u64)?;
            blob.varint(key as u64)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = replay_columnar::leaf::Reader::new(blob);
        let count = |reader: &mut replay_columnar::leaf::Reader<'_>| -> io::Result<usize> {
            let count = reader.count()?;
            if count > blob.len() {
                return Err(invalid("Groups' state counts more than it holds"));
            }
            Ok(count)
        };
        self.epochs = reader.varint_u32()?;
        for which in 0..3 {
            let mut history = History::default();
            for _ in 0..count(&mut reader)? {
                let kind = reader.u8()?;
                let (group, value, epoch, ordinal) = (reader.varint_u32()?, reader.varint_u32()?, reader.varint_u32()?, reader.varint_u32()?);
                history.open.insert((kind, group, value), (epoch, ordinal));
            }
            *match which {
                0 => &mut self.humans,
                1 => &mut self.factions,
                _ => &mut self.wars,
            } = history;
        }
        for _ in 0..count(&mut reader)? {
            let faction = reader.varint()?;
            self.numbers.insert(faction, reader.varint_u32()?);
        }
        for _ in 0..count(&mut reader)? {
            let literal = reader.varint_u32()?;
            let token = match reader.varint_u32()? {
                0 => None,
                token => Some(token - 1),
            };
            let spelling = reader.varint_u32()?;
            self.names.insert((literal, token, spelling), reader.varint_u32()?);
        }
        for _ in 0..count(&mut reader)? {
            let (first, second, start) = (reader.varint_u32()?, reader.varint_u32()?, reader.varint_u32()? as i32);
            self.starts.insert((first, second, start));
        }
        for _ in 0..count(&mut reader)? {
            let (country, when) = (reader.varint_u32()?, reader.varint_u32()? as i32);
            self.capitulations.insert((country, when));
        }
        for _ in 0..count(&mut reader)? {
            let (high, low) = (reader.varint()?, reader.varint()?);
            self.events.push(u128::from(high) << 64 | u128::from(low));
        }
        self.rebuild = true;
        reader.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder_derive_framework::testing;

    #[test]
    fn a_fact_held_lives_on_a_fact_gone_closes_and_one_back_is_a_new_stretch() {
        let mut groups = Groups::default();
        let member = |slot| ((Span::MEMBER, Span::HUMANS, slot), 0);
        let store = testing::store();
        let mut written = |facts| {
            testing::logging(&store, groups.tables(), |out| {
                groups.settle(0, facts, out)?;
                Ok(out.keys(TableId::GroupSpans))
            }).unwrap()
        };
        assert_eq!(written(BTreeMap::from([member(5), member(6)])).0, vec![span_key((Span::MEMBER, 0, 5), 0), span_key((Span::MEMBER, 0, 6), 1)]);
        assert_eq!(written(BTreeMap::from([member(5)])), (vec![], vec![span_key((Span::MEMBER, 0, 6), 1)]));
        assert_eq!(written(BTreeMap::from([member(5), member(6)])), (vec![span_key((Span::MEMBER, 0, 6), 2)], vec![]));
    }

    #[test]
    fn a_war_is_ongoing_until_an_end_after_its_start() {
        assert!(ongoing(100, 0, 500), "an end not after the start is no end");
        assert!(ongoing(100, 300, 299));
        assert!(!ongoing(100, 300, 300));
    }
}
