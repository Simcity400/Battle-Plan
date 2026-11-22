
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Keys, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    store::Value as Held,
};
use replay_data::rows::{FleetRow, LeaderCommandRow as Command, OrdersGroupRow, TableId};
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const GROUP: [usize; 3] = columns::<OrdersGroupRow, 3>(["leader", "is_army_group", "name"]);
const FLEET: [usize; 2] = columns::<FleetRow, 2>(["leader", "name"]);

const READS: &[Read] = &[
    Read { table: TableId::OrdersGroups, columns: Columns::These(&GROUP), watch: Watch::Changed, required: true },
    Read { table: TableId::Fleets, columns: Columns::These(&FLEET), watch: Watch::Changed, required: true },
];

struct Source {
    source: u8,
    table: TableId,
    leader: usize,
    name: usize,
}

static SOURCES: [Source; 2] = [
    Source { source: Command::ORDERS_GROUP, table: TableId::OrdersGroups, leader: GROUP[0], name: GROUP[2] },
    Source { source: Command::FLEET, table: TableId::Fleets, leader: FLEET[0], name: FLEET[1] },
];

fn span_key(source: u8, formation: u64) -> u128 {
    u128::from(source) << 64 | u128::from(formation)
}

fn bytes(held: Held<'_>) -> Option<Vec<u8>> {
    match held {
        Held::List(bytes) => Some(bytes.to_vec()),
        _ => None,
    }
}

fn opaque(out: &mut Rows, bytes: Option<Vec<u8>>) -> Value {
    bytes.map_or(Value::Unrecorded, |bytes| out.opaque(bytes))
}

struct Ended {
    leader: u64,
    source: u8,
    formation: u64,
    cells: Vec<Value>,
}

#[derive(Default)]
pub struct LeaderCommands {
    last: Option<i32>,
}

impl LeaderCommands {
    fn end(out: &mut Rows, key: u128, last: i32, ended: &mut Vec<Ended>) {
        let spans = out.table(TableId::LeaderCommandSpans);
        let Some(slot) = spans.slot(key) else { return };
        let (leader, first, kind, name) = (spans.number(0, slot), spans.number(1, slot), spans.number(2, slot), bytes(spans.value(3, slot)));
        let name = opaque(out, name);
        out.delete(TableId::LeaderCommandSpans, key);
        let (Some(leader), Some(first), Some(kind)) = (leader, first, kind) else { return };
        let (source, formation) = ((key >> 64) as u8, key as u64);
        let cells = vec![Value::U8(source), Value::U64(formation), Value::U8(kind as u8), Value::I32(first as i32), Value::I32(last), name];
        ended.push(Ended { leader: leader as u64, source, formation, cells });
    }

    fn open(hour: &Hour<'_>, out: &mut Rows, source: &Source, slot: u32, leader: u64) -> io::Result<()> {
        let view = hour.table(source.table);
        let formation = view.key(slot).ok_or_else(|| invalid(format!("{:?} slot {slot} holds no row", source.table)))? as u64;
        let kind = match source.source {
            Command::FLEET => Command::NAVY,
            _ if view.number(GROUP[1], slot).is_some_and(|group| group != 0) => Command::ARMY_GROUP,
            _ => Command::ARMY,
        };
        let name = opaque(out, bytes(view.value(source.name, slot)));
        out.set(TableId::LeaderCommandSpans, span_key(source.source, formation), vec![Value::U64(leader), Value::I32(hour.hour), Value::U8(kind), name]);
        Ok(())
    }

    fn leader(hour: &Hour<'_>, source: &Source, slot: u32) -> u64 {
        hour.table(source.table).number(source.leader, slot).map_or(0, |leader| leader as u64)
    }

    fn whole(&self, hour: &Hour<'_>, out: &mut Rows, ended: &mut Vec<Ended>) -> io::Result<()> {
        let open = recorder_derive_framework::forces::held(out, TableId::LeaderCommandSpans);
        for key in open {
            match self.last {
                Some(last) => Self::end(out, key, last, ended),
                None => out.delete(TableId::LeaderCommandSpans, key),
            }
        }
        for source in &SOURCES {
            let slots: Vec<u32> = hour.table(source.table).rows().collect();
            for slot in slots {
                let leader = Self::leader(hour, source, slot);
                if leader != 0 {
                    Self::open(hour, out, source, slot, leader)?;
                }
            }
        }
        Ok(())
    }

    fn changed(hour: &Hour<'_>, out: &mut Rows, ended: &mut Vec<Ended>) -> io::Result<()> {
        let at = hour.hour;
        for source in &SOURCES {
            for key in hour.deleted_keys(source.table) {
                if hour.table(source.table).slot(key).is_none() {
                    Self::end(out, span_key(source.source, key as u64), at - 1, ended);
                }
            }
            let view = hour.table(source.table);
            for slot in hour.written_rows(source.table, Columns::These(std::slice::from_ref(&source.leader))) {
                let formation = view.key(slot).ok_or_else(|| invalid(format!("{:?} slot {slot} holds no row", source.table)))? as u64;
                let key = span_key(source.source, formation);
                let leader = Self::leader(hour, source, slot);
                let spans = out.table(TableId::LeaderCommandSpans);
                let held = spans.slot(key).and_then(|span| spans.number(0, span)).map(|leader| leader as u64);
                if held == Some(leader) {
                    continue;
                }
                if held.is_some() {
                    Self::end(out, key, at - 1, ended);
                }
                if leader != 0 {
                    Self::open(hour, out, source, slot, leader)?;
                }
            }
        }
        Ok(())
    }
}

impl Writer for LeaderCommands {
    fn name(&self) -> &'static str {
        "LeaderCommands"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::LeaderCommandSpans), Output::Log(TableId::LeaderCommands, Keys::Hour(32))]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        recorder_derive_framework::forces::forget_listed(hour, out, TableId::LeaderCommands);
        let mut ended = Vec::new();
        if hour.fresh {
            self.whole(hour, out, &mut ended)?;
        } else {
            Self::changed(hour, out, &mut ended)?;
        }
        self.last = Some(hour.hour);
        ended.sort_unstable_by_key(|span| (span.leader, span.source, span.formation));
        let mut ordinal = 0u32;
        let mut previous = None;
        for span in ended {
            if previous != Some(span.leader) {
                ordinal = 0;
                previous = Some(span.leader);
            }
            let key = u128::from(span.leader) << 64 | u128::from(hour.hour as u32) << 32 | u128::from(ordinal);
            out.append(TableId::LeaderCommands, key, span.cells);
            ordinal += 1;
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        match self.last {
            Some(last) => {
                blob.u8(1)?;
                blob.raw(&last.to_le_bytes())
            }
            None => blob.u8(0),
        }
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        if blob.is_empty() {
            return Ok(());
        }
        let mut reader = Reader::new(blob);
        if reader.u8()? != 0 {
            self.last = Some(i32::from_le_bytes(reader.raw(4)?.try_into().map_err(|_| invalid("LeaderCommands' state holds a cut hour"))?));
        }
        reader.finish()
    }
}
