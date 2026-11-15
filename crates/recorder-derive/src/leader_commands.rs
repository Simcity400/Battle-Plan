//! `LeaderCommandSpans` and `LeaderCommands`: the formations (orders groups and fleets) each
//! leader commands. A span starts at the first hour a formation's leader column names the leader
//! and ends the hour before it names another or none, the formation is deleted, or the writer next
//! runs after hours it did not (its last hour run). An open span is a `LeaderCommandSpans` row,
//! keyed by the formation, holding the leader, the first hour, the kind (an orders group's army or
//! army group then, a fleet) and the formation's stored name then; an ended span is a
//! `LeaderCommands` row of the log appended the hour it ended and forgotten at the writer's next
//! hour, keyed by the leader, the hour and its place among the leader's rows of the hour.
//!
//! State: the last hour the writer ran (a span ending in hours it did not run ends there); the
//! open spans are the table's.

#[cfg(test)]
mod tests;

use crate::framework::{Columns, Hour, Keys, Output, Read, Rows, Value, Watch, Writer, columns};
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

/// A formation table: its source number, its leader column and its name column.
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

/// An opaque cell's bytes as it is held; none where it is not recorded.
fn bytes(held: Held<'_>) -> Option<Vec<u8>> {
    match held {
        Held::List(bytes) => Some(bytes.to_vec()),
        _ => None,
    }
}

fn opaque(out: &mut Rows, bytes: Option<Vec<u8>>) -> Value {
    bytes.map_or(Value::Unrecorded, |bytes| out.opaque(bytes))
}

/// A span that ended this hour, before its place among its leader's rows.
struct Ended {
    leader: u64,
    source: u8,
    formation: u64,
    cells: Vec<Value>,
}

#[derive(Default)]
pub struct LeaderCommands {
    /// The last hour it ran.
    last: Option<i32>,
}

impl LeaderCommands {
    /// Ends the open span of `key` at `last`, its row deleted.
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

    /// Opens a span of `source`'s formation in `slot` under its leader at the hour, unless it has
    /// none.
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

    /// The leader `source`'s formation in `slot` names: none for zero or a cell without one.
    fn leader(hour: &Hour<'_>, source: &Source, slot: u32) -> u64 {
        hour.table(source.table).number(source.leader, slot).map_or(0, |leader| leader as u64)
    }

    /// Every open span ended at the last hour run, and every formation's leader read whole.
    fn whole(&self, hour: &Hour<'_>, out: &mut Rows, ended: &mut Vec<Ended>) -> io::Result<()> {
        let open = crate::forces::held(out, TableId::LeaderCommandSpans);
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

    /// The formations the hour deleted, or whose leader column it wrote or inserted.
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

    // Hourly: its log rows are forgotten at its next hour, and a span ending in hours it did not run ends at its last.
    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        crate::forces::forget_listed(hour, out, TableId::LeaderCommands);
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
