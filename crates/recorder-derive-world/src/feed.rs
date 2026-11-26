
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::leaf::{Reader, Writer as Bytes};
use replay_data::{
    event_feed_rows::{DECISION_EDGE_REFUSALS, EventFeedCoverage, EventFeedEvent, EventFeedHistoryRow, EventFeedSource as Source},
    rows::{Field, FeedEventRow, Opaque, Recorded, RowCodec, TableId, TagHolderRow},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

mod inputs;
mod open;
#[cfg(test)]
mod tests;

use inputs::{ChargeKey, Counted, Names, OWNED, Pack, RELATION, SLOT};
use open::{Links, Open};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const READS: &[Read] = &[
    Read {
        table: TableId::EventFeedHistory,
        columns: Columns::Every,
        watch: Watch::EveryRead,
        required: true,
    },
    Read {
        table: TableId::Countries,
        columns: Columns::These(&SLOT),
        watch: Watch::Changed,
        required: false,
    },
    Read {
        table: TableId::CountryLinks,
        columns: Columns::These(&SLOT),
        watch: Watch::Changed,
        required: false,
    },
    Read {
        table: TableId::CountryCounterInputs,
        columns: Columns::These(&OWNED),
        watch: Watch::EveryRead,
        required: false,
    },
    Read {
        table: TableId::RelationTuples,
        columns: Columns::These(&RELATION),
        watch: Watch::Dropped,
        required: false,
    },
    Read {
        table: TableId::CountryPowerChanges,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: false,
    },
    Read {
        table: TableId::PoliticsCaptureCoverage,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: false,
    },
];

#[derive(Default)]
pub struct Feed {
    living: Vec<u128>,
    links: Links,
    last: Option<(u64, u64)>,
    first: Option<i32>,
    owning: HashSet<u32>,
    counted: Option<i32>,
    counters_whole: bool,
    relations_whole: bool,
}

fn token<'a>(pack: &'a EventFeedHistoryRow, event: &EventFeedEvent) -> Option<&'a [u8]> {
    match event.script {
        None => Some(&[]),
        Some(script) if script.token_unrecorded => None,
        Some(script) => pack.script_tokens.get(script.token_start as usize..script.token_start as usize + script.token_len as usize),
    }
}

fn charge_key(event: &EventFeedEvent, token: Option<&[u8]>) -> Option<ChargeKey> {
    let timeout = match event.source {
        Source::DecisionSelected => false,
        Source::DecisionTimeoutEffect => true,
        _ => return None,
    };
    let token = token.filter(|token| !token.is_empty())?;
    Some((event.hour, event.actor.id, token.into(), timeout))
}

fn cell(out: &mut Rows, field: Field<'_>) -> io::Result<Value> {
    Ok(match field {
        Field::NotRecorded => Value::Unrecorded,
        Field::U8(value) => Value::U8(value),
        Field::I8(value) => Value::I8(value),
        Field::U16(value) => Value::U16(value),
        Field::I16(value) => Value::I16(value),
        Field::U32(value) => Value::U32(value),
        Field::I32(value) => Value::I32(value),
        Field::U64(value) => Value::U64(value),
        Field::I64(value) => Value::I64(value),
        Field::Bool(value) => Value::Bool(value),
        Field::OptionalI32(value) => Value::OptionalI32(value),
        Field::OptionalU32(value) => Value::OptionalU32(value),
        Field::OptionalI64(value) => Value::OptionalI64(value),
        Field::OptionalU64(value) => Value::OptionalU64(value),
        Field::Bytes(text) => out.text(text),
        Field::Opaque(Opaque::Encoded(bytes, _)) => out.opaque(bytes.to_vec()),
        Field::Opaque(Opaque::Native(value)) => {
            let mut bytes = Vec::new();
            value.encode_value(&mut bytes)?;
            out.opaque(bytes)
        }
        Field::Empty | Field::U32List(_) | Field::U64List(_) => return Err(invalid("A FeedEvents field of a kind no column holds")),
    })
}

impl Feed {
    fn write(&mut self, out: &mut Rows, row: &FeedEventRow) -> io::Result<()> {
        let mut cells = Vec::with_capacity(TableId::FeedEvents.field_count());
        row.visit_wire(&mut |_, field| {
            cells.push(cell(out, field)?);
            Ok(())
        })?;
        let key = row.key();
        out.set(TableId::FeedEvents, key, cells);
        self.living.push(key);
        Ok(())
    }

    fn occurrences(&mut self, hour: &Hour<'_>, read: &Pack, open: &mut Open, ordinal: &mut u32, out: &mut Rows<'_>) -> io::Result<()> {
        let pack = &read.row;
        let names = Names::new(hour);
        let names = names.as_ref();
        let complete = pack.coverage == EventFeedCoverage::Complete && pack.script_refusals == 0;
        let linked = pack.coverage == EventFeedCoverage::Complete && pack.script_refusals & DECISION_EDGE_REFUSALS == 0;
        open.observe_read(out, &mut self.links, pack.read_hour, linked)?;
        if !complete {
            self.derived(out, hour.hour, ordinal, FeedEventRow::READ_INCOMPLETE, |row| {
                row.read_hour = pack.read_hour;
                row.coverage = Some(pack.coverage as u32);
                row.outstanding = Some(pack.outstanding);
                row.refusals = Some(u32::from(pack.script_refusals));
                row.incomplete = Some(FeedEventRow::incomplete_of(pack.coverage as u8, pack.script_refusals));
                row.failure = pack.failure.clone().unwrap_or_default();
            })?;
        }
        let mut executions: HashMap<ChargeKey, u32> = HashMap::new();
        for event in &pack.events {
            if let Some(key) = charge_key(event, token(pack, event)) {
                *executions.entry(key).or_default() += 1;
            }
        }
        let charges = if executions.is_empty() { None } else { inputs::charges(hour)? };
        let verdicts = match &pack.windows {
            Recorded::Value(windows) => windows.verdicts.as_ref().ok().map(|verdicts| verdicts.iter().map(|entry| (entry.sequence, u32::from(entry.verdict.code()))).collect::<HashMap<_, _>>()),
            Recorded::NotRecorded => None,
        };
        for event in &pack.events {
            if event.generation == FeedEventRow::DERIVED {
                return Err(invalid("An occurrence of the generation derived rows are keyed by"));
            }
            if self.last.is_some_and(|(generation, sequence)| generation == event.generation && event.sequence <= sequence) {
                return Err(invalid(format!("Occurrence {} of generation {} does not follow the last one read", event.sequence, event.generation)));
            }
            self.last = Some((event.generation, event.sequence));
            if event.source == Source::ScriptState {
                continue;
            }
            let token = token(pack, event);
            let opener = open.link(out, &mut self.links, event, token, pack.read_hour)?;
            let charge = charge_key(event, token);
            let charged = matches!(event.source, Source::DecisionSelected | Source::DecisionTimeoutEffect);
            let proven = charge.as_ref().filter(|key| executions.get(*key) == Some(&1)).and_then(|key| charges.as_ref()?.get(key));
            let [political_power, command_power] = match proven {
                Some(sums) => (*sums).map(|sum| sum.and_then(|sum| i64::try_from(sum).ok()).map_or(Recorded::NotRecorded, |sum| Recorded::Value(Some(sum)))),
                None if charged => [Recorded::NotRecorded, Recorded::NotRecorded],
                None => [Recorded::Value(None), Recorded::Value(None)],
            };
            let script = event.script;
            let scopes = match script {
                None => Recorded::Value(Vec::new()),
                Some(script) if script.scope_unrecorded => Recorded::NotRecorded,
                Some(script) => pack
                    .script_scopes
                    .get(script.scope_start as usize..script.scope_start as usize + script.scope_len as usize)
                    .map_or(Recorded::NotRecorded, |scopes| Recorded::Value(scopes.to_vec())),
            };
            let mut row = FeedEventRow {
                generation: event.generation,
                sequence: event.sequence,
                hour: event.hour,
                read_hour: pack.read_hour,
                kind: event.source as u8,
                actor_reference: event.actor.reference,
                actor_id: Recorded::Value(event.actor.id),
                actor_instance: Recorded::Value(event.actor.instance),
                actor_slot: inputs::slot(names, event.actor.reference, Some(event.actor.id)),
                target_reference: event.target.reference,
                target_id: Recorded::Value(event.target.id),
                target_instance: Recorded::Value(event.target.instance),
                target_slot: inputs::slot(names, event.target.reference, Some(event.target.id)),
                token: token.map_or(Recorded::NotRecorded, |token| Recorded::Value(token.to_vec())),
                native_event_kind: script.and_then(|script| script.native_event).map(|native| native.kind),
                native_event_id: script.and_then(|script| script.native_event).map(|native| native.id),
                instance: script.and_then(|script| script.instance),
                option: script.and_then(|script| script.option),
                country_target: script.and_then(|script| script.country_target),
                numeric_target: script.and_then(|script| script.numeric_target),
                lifecycle: script.and_then(|script| script.lifecycle),
                days: script.and_then(|script| script.days),
                scopes,
                opener: opener.map(|(sequence, _)| sequence),
                opener_hour: opener.map(|(_, hour)| hour),
                backlog: read.backlog,
                dated: None,
                political_power,
                command_power,
                window: match event.source {
                    Source::ScriptEvent | Source::NewsEvent => verdicts.as_ref().and_then(|verdicts| verdicts.get(&event.sequence)).map_or(Recorded::NotRecorded, |code| Recorded::Value(Some(*code))),
                    _ => Recorded::Value(None),
                },
                ..FeedEventRow::default()
            };
            match event.cause {
                Recorded::Value(cause) => {
                    row.route = Recorded::Value(cause.route as u8);
                    row.within = Recorded::Value(cause.within as u8);
                    row.conference = Recorded::Value(cause.conference);
                    row.state = Recorded::Value(cause.state);
                    row.ally_reference = Recorded::Value(cause.ally.map(|ally| ally.reference));
                    row.ally_id = Recorded::Value(cause.ally.map(|ally| ally.id));
                    row.ally_instance = Recorded::Value(cause.ally.map(|ally| ally.instance));
                    row.ally_slot = match cause.ally {
                        None => Recorded::Value(None),
                        Some(ally) => match inputs::slot(names, ally.reference, Some(ally.id)) {
                            Recorded::Value(slot) => Recorded::Value(Some(slot)),
                            Recorded::NotRecorded => Recorded::NotRecorded,
                        },
                    };
                    row.reason = Recorded::Value(cause.reason);
                    row.flags = Recorded::Value(cause.flags);
                }
                Recorded::NotRecorded => {
                    row.route = Recorded::NotRecorded;
                    row.within = Recorded::NotRecorded;
                    row.conference = Recorded::NotRecorded;
                    row.state = Recorded::NotRecorded;
                    row.ally_reference = Recorded::NotRecorded;
                    row.ally_id = Recorded::NotRecorded;
                    row.ally_instance = Recorded::NotRecorded;
                    row.ally_slot = Recorded::NotRecorded;
                    row.reason = Recorded::NotRecorded;
                    row.flags = Recorded::NotRecorded;
                }
            }
            self.write(out, &row)?;
        }
        self.windows(hour, names, pack, ordinal, out)?;
        Ok(())
    }

    fn windows(&mut self, hour: &Hour<'_>, names: Option<&Names<'_>>, pack: &EventFeedHistoryRow, ordinal: &mut u32, out: &mut Rows) -> io::Result<()> {
        let Recorded::Value(windows) = &pack.windows else {
            return Ok(());
        };
        let mut unrecorded = 0;
        let mut failure = Vec::new();
        for (bit, outcome) in [(FeedEventRow::VERDICTS_UNRECORDED, windows.verdicts.as_ref().err()), (FeedEventRow::PRESENTATIONS_UNRECORDED, windows.presentations_unrecorded.as_ref())] {
            if let Some(reason) = outcome {
                unrecorded |= bit;
                if !failure.is_empty() {
                    failure.extend_from_slice(b"; ");
                }
                failure.extend_from_slice(reason);
            }
        }
        if unrecorded != 0 {
            self.derived(out, hour.hour, ordinal, FeedEventRow::WINDOWS_UNRECORDED, |row| {
                row.read_hour = pack.read_hour;
                row.incomplete = Some(unrecorded);
                row.failure = failure;
            })?;
        }
        for presentation in &windows.presentations {
            let presentation = presentation.clone();
            self.derived(out, hour.hour, ordinal, FeedEventRow::WINDOW_PRESENTED, |row| {
                row.hour = presentation.native_hour.unwrap_or(pack.read_hour);
                row.read_hour = pack.read_hour;
                row.actor_reference = presentation.actor;
                row.actor_slot = inputs::slot(names, presentation.actor, None);
                row.native_event_kind = Some(presentation.event_kind);
                row.native_event_id = Some(presentation.event_id);
                row.instance = Some(presentation.instance);
                row.token = presentation.token.clone();
                row.presentation = Recorded::Value(Some(presentation));
            })?;
        }
        Ok(())
    }

    fn derived(&mut self, out: &mut Rows, at: i32, ordinal: &mut u32, kind: u8, fill: impl FnOnce(&mut FeedEventRow)) -> io::Result<()> {
        let mut row = FeedEventRow {
            generation: FeedEventRow::DERIVED,
            sequence: u64::from(at as u32) << 32 | u64::from(*ordinal),
            hour: at,
            read_hour: at,
            kind,
            actor_id: Recorded::NotRecorded,
            actor_instance: Recorded::NotRecorded,
            actor_slot: Recorded::Value(TagHolderRow::NONE),
            target_id: Recorded::NotRecorded,
            target_instance: Recorded::NotRecorded,
            target_slot: Recorded::Value(TagHolderRow::NONE),
            route: Recorded::NotRecorded,
            within: Recorded::NotRecorded,
            conference: Recorded::NotRecorded,
            state: Recorded::NotRecorded,
            ally_reference: Recorded::NotRecorded,
            ally_id: Recorded::NotRecorded,
            ally_instance: Recorded::NotRecorded,
            ally_slot: Recorded::NotRecorded,
            reason: Recorded::NotRecorded,
            flags: Recorded::NotRecorded,
            token: Recorded::Value(Vec::new()),
            scopes: Recorded::Value(Vec::new()),
            political_power: Recorded::Value(None),
            command_power: Recorded::Value(None),
            ..FeedEventRow::default()
        };
        fill(&mut row);
        *ordinal += 1;
        self.write(out, &row)
    }

    fn existence(&mut self, hour: &Hour<'_>, names: Option<&Names<'_>>, before: Option<i32>, ordinal: &mut u32, out: &mut Rows) -> io::Result<()> {
        let whole = self.counters_whole || hour.fresh;
        let (counters, deleted) = match inputs::counters(hour, whole) {
            Counted::Incomplete => {
                self.counters_whole = true;
                return Ok(());
            }
            Counted::Unread => return Ok(()),
            Counted::Read(counters, deleted) => (counters, deleted),
        };
        let at = hour.hour;
        let baseline = hour.first || before.is_none();
        let mut fallen = Vec::new();
        let held = std::mem::take(&mut self.owning);
        let (held, mut owning) = if whole { (held, HashSet::with_capacity(counters.rows.len())) } else { (HashSet::new(), held) };
        for &slot in &counters.rows {
            let Some((country, owned)) = counters.owned(slot) else { continue };
            if owned > 0 {
                owning.insert(country);
                continue;
            }
            let owned_before = if whole { held.contains(&country) } else { owning.remove(&country) };
            if owned == 0 && owned_before {
                fallen.push(country);
            }
        }
        for country in deleted {
            owning.remove(&country);
        }
        self.owning = owning;
        self.counters_whole = false;
        if baseline {
            return Ok(());
        }
        fallen.sort_unstable();
        for country in fallen {
            self.derived(out, at, ordinal, FeedEventRow::CEASED_TO_EXIST, |row| {
                row.actor_reference = country;
                row.actor_id = Recorded::Value(country);
                row.actor_slot = inputs::slot(names, country, Some(country));
                row.dated = before;
            })?;
        }
        Ok(())
    }

    fn guarantees(&mut self, hour: &Hour<'_>, names: Option<&Names<'_>>, first: i32, ordinal: &mut u32, out: &mut Rows) -> io::Result<()> {
        let whole = self.relations_whole || hour.fresh;
        let Some(read) = inputs::guarantees(hour, whole) else {
            self.relations_whole = true;
            return Ok(());
        };
        let at = hour.hour;
        self.relations_whole = false;
        let table = TableId::FeedGuarantees;
        let key = |(first_country, second_country, start): inputs::Guarantee| u128::from(first_country) << 64 | u128::from(second_country) << 32 | u128::from(start as u32);
        let row = |out: &Rows<'_>, seen: u128| {
            let tracked = out.table(table);
            tracked.slot(seen).map(|slot| (tracked.number(0, slot).map(|hour| hour as i32), tracked.number(1, slot).unwrap_or(0)))
        };
        let mut touched: Vec<inputs::Guarantee> = match whole {
            true => {
                for seen in recorder_derive_framework::forces::held(out, table) {
                    if !read.held.contains_key(&((seen >> 64) as u32, (seen >> 32) as u32, seen as u32 as i32)) {
                        out.delete(table, seen);
                    }
                }
                read.held.keys().copied().collect()
            }
            false => read.moved.keys().chain(&read.listed).copied().collect(),
        };
        touched.retain(|&(_, _, start)| start > first);
        touched.sort_unstable();
        touched.dedup();
        for found in touched {
            let (first_country, second_country, start) = found;
            let seen = key(found);
            let held = row(out, seen);
            let now = read.held.get(&found).copied().unwrap_or(0);
            let rows = match whole {
                true => now,
                false => (held.map_or(0, |(_, rows)| rows) + read.moved.get(&found).copied().unwrap_or(0)).max(now),
            };
            if rows <= 0 {
                if held.is_some() {
                    out.delete(table, seen);
                }
                continue;
            }
            let mut written = held.and_then(|(written, _)| written);
            let grant = written.is_none() && start <= at && read.listed.contains(&found);
            if grant {
                written = Some(at);
            }
            if held != Some((written, rows)) {
                out.set(table, seen, vec![Value::OptionalI32(written), Value::U32(rows as u32)]);
            }
            if !grant {
                continue;
            }
            self.derived(out, at, ordinal, FeedEventRow::GUARANTEE_GRANTED, |row| {
                row.actor_reference = first_country;
                row.actor_slot = inputs::slot(names, first_country, None);
                row.target_reference = second_country;
                row.target_slot = inputs::slot(names, second_country, None);
                row.dated = Some(start);
            })?;
        }
        Ok(())
    }
}

fn put_i32(blob: &mut Bytes, value: Option<i32>) -> io::Result<()> {
    match value {
        Some(value) => {
            blob.u8(1)?;
            blob.raw(&value.to_le_bytes())
        }
        None => blob.u8(0),
    }
}

fn get_i32(reader: &mut Reader<'_>) -> io::Result<Option<i32>> {
    Ok(match reader.u8()? {
        0 => None,
        _ => Some(i32::from_le_bytes(reader.raw(4)?.try_into().expect("4 bytes"))),
    })
}

fn put_u64(blob: &mut Bytes, value: Option<u64>) -> io::Result<()> {
    match value {
        Some(value) => {
            blob.u8(1)?;
            blob.varint(value)
        }
        None => blob.u8(0),
    }
}

fn get_u64(reader: &mut Reader<'_>) -> io::Result<Option<u64>> {
    Ok(match reader.u8()? {
        0 => None,
        _ => Some(reader.varint()?),
    })
}

fn count(reader: &mut Reader<'_>, blob: &[u8]) -> io::Result<usize> {
    let count = reader.count()?;
    if count > blob.len() {
        return Err(invalid("Feed's state counts more items than it holds"));
    }
    Ok(count)
}

impl Writer for Feed {
    fn name(&self) -> &'static str {
        "Feed"
    }

    fn version(&self) -> u32 {
        5
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::FeedEvents), Output::State(TableId::FeedOpenEvents), Output::State(TableId::FeedOpenDecisions), Output::State(TableId::FeedGuarantees)]
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
        for key in self.living.drain(..) {
            out.delete(TableId::FeedEvents, key);
        }
        let first = *self.first.get_or_insert(hour.hour);
        let mut ordinal = 0;
        let counted = self.counted;
        if inputs::counters_read(hour) {
            self.counted = Some(hour.hour);
        }
        if let Some(pack) = inputs::pack(hour)? {
            let mut open = Open::default();
            self.occurrences(hour, &pack, &mut open, &mut ordinal, out)?;
            open.flush(out);
        }
        if !(hour.fresh || self.counters_whole || self.relations_whole || inputs::derived_inputs_moved(hour)) {
            return Ok(());
        }
        let names = Names::new(hour);
        self.existence(hour, names.as_ref(), counted, &mut ordinal, out)?;
        self.guarantees(hour, names.as_ref(), first, &mut ordinal, out)
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        let events = hour.table(TableId::FeedEvents);
        self.living = events.rows().filter_map(|slot| events.key(slot)).collect();
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        put_i32(blob, self.first)?;
        put_i32(blob, self.counted)?;
        blob.u8(u8::from(self.counters_whole) | u8::from(self.relations_whole) << 1)?;
        put_u64(blob, self.last.map(|(generation, _)| generation))?;
        put_u64(blob, self.last.map(|(_, sequence)| sequence))?;
        put_u64(blob, self.links.generation)?;
        put_i32(blob, self.links.read)?;
        blob.u8(u8::from(self.links.covered))?;
        let mut owning: Vec<u32> = self.owning.iter().copied().collect();
        owning.sort_unstable();
        blob.varint(owning.len() as u64)?;
        for country in owning {
            blob.varint(u64::from(country))?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        self.first = get_i32(&mut reader)?;
        self.counted = get_i32(&mut reader)?;
        let whole = reader.u8()?;
        (self.counters_whole, self.relations_whole) = (whole & 1 != 0, whole & 2 != 0);
        self.last = get_u64(&mut reader)?.zip(get_u64(&mut reader)?);
        self.links.generation = get_u64(&mut reader)?;
        self.links.read = get_i32(&mut reader)?;
        self.links.covered = reader.u8()? != 0;
        for _ in 0..count(&mut reader, blob)? {
            self.owning.insert(reader.varint_u32()?);
        }
        reader.finish()
    }
}
