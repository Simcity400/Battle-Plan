
mod inputs;
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use inputs::{Naming, Template, invalid};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::{
    country_stability,
    rows::{CountryStabilityInputRow, CountryStabilityLineRow, StabilityPartKeyRow, StabilityPartRow, TableId},
};
use std::{collections::HashMap, io};

const READS: &[Read] = &[
    Read {
        table: TableId::CountryStabilityInputs,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::CountryStabilityLines,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::SliceReads,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::Countries,
        columns: Columns::These(&[]),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::Characters,
        columns: Columns::These(&[]),
        watch: Watch::Changed,
        required: false,
    },
    Read {
        table: TableId::LexicalTokens,
        columns: Columns::These(&[]),
        watch: Watch::Changed,
        required: false,
    },
];

const LINES_OF: [(&str, &[u8]); 2] = [("stabilityFactor", b"stability_factor"), ("warSupportFactor", b"war_support_factor")];

const MEASURES: [u8; 2] = [StabilityPartRow::STABILITY, StabilityPartRow::WAR_SUPPORT];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Source {
    kind: u8,
    identity: Box<[u8]>,
    identity_id: u32,
    secondary: i64,
    naming: Naming,
}

impl Source {
    fn plain(kind: u8, identity: &[u8]) -> Self {
        Self { kind, identity: identity.into(), identity_id: 0, secondary: 0, naming: Naming::none() }
    }

    fn line(line: &CountryStabilityLineRow, character: &mut dyn FnMut(u32) -> Naming) -> Self {
        match line.kind {
            CountryStabilityLineRow::UNPAIRED => match content_reader_localisation::localisation::display_text(&line.text) {
                Ok(text) => Self::plain(CountryStabilityLineRow::UNPAIRED, text.as_bytes()),
                Err(_) => Self::plain(StabilityPartKeyRow::REFUSED_LABEL, &line.text),
            },
            kind => {
                let naming = match kind {
                    CountryStabilityLineRow::ADVISOR | CountryStabilityLineRow::COUNTRY_LEADER => character(line.identity_id),
                    _ => Naming::none(),
                };
                Self { kind, identity: line.identity.as_slice().into(), identity_id: line.identity_id, secondary: line.secondary, naming }
            }
        }
    }
}

fn add(parts: &mut Vec<(Source, i64)>, source: Source, value: i64) {
    match parts.iter_mut().find(|(held, _)| *held == source) {
        Some((_, held)) => *held = held.wrapping_add(value),
        None => parts.push((source, value)),
    }
}

fn parts(
    row: &CountryStabilityInputRow,
    measure: u8,
    lines: &[CountryStabilityLineRow],
    character: &mut dyn FnMut(u32) -> Naming,
) -> Option<Vec<(Source, i64)>> {
    let terms = match measure {
        StabilityPartRow::STABILITY => country_stability::stability(row),
        _ => country_stability::war_support(row),
    };
    let value = terms.value?;
    let (lines_term, key_name) = LINES_OF[usize::from(measure)];
    let at = CountryStabilityInputRow::KEY_NAMES.iter().position(|name| *name == key_name)?;
    let key = *row.keys.get(at)?;
    let total = *row.totals().get(at)?;
    let mut parts = vec![(Source::plain(StabilityPartKeyRow::TOTAL, b""), value)];
    for &(name, raw) in &terms.terms {
        let raw = raw?;
        if name != lines_term {
            add(&mut parts, Source::plain(StabilityPartKeyRow::TERM, name.as_bytes()), raw);
            continue;
        }
        let mut sum = 0_i64;
        for line in lines.iter().filter(|line| key != u32::MAX && line.key == key) {
            sum = sum.wrapping_add(line.value);
            if line.value != 0 {
                add(&mut parts, Source::line(line, character), line.value);
            }
        }
        if let Some(remainder) = total.map(|total| total.wrapping_sub(sum)) {
            add(&mut parts, Source::plain(StabilityPartKeyRow::REMAINDER, b""), remainder);
        }
    }
    if let Some(unclamped) = terms.unclamped {
        add(&mut parts, Source::plain(StabilityPartKeyRow::CLAMP, b""), value.wrapping_sub(unclamped));
    }
    parts.retain(|(source, value)| *value != 0 || source.kind == StabilityPartKeyRow::TOTAL);
    Some(parts)
}

fn part_key(country: u32, measure: u8, handle: u32) -> u128 {
    u128::from(country) << 40 | u128::from(measure) << 32 | u128::from(handle)
}

fn dictionary_row(view: &TableView<'_>, slot: u32) -> io::Result<(u32, Source)> {
    let unread = || invalid("A StabilityPartKeys row is not whole");
    let handle = view.key(slot).ok_or_else(unread)? as u32;
    let text = |column: usize| -> io::Result<Option<Box<[u8]>>> {
        if !view.recorded(column, slot) {
            return Ok(None);
        }
        view.text(column, slot).and_then(|id| view.string(id)).map(|text| Some(text.into())).ok_or_else(unread)
    };
    let number = |column: usize| view.number(column, slot).ok_or_else(unread);
    let template = match view.recorded(6, slot) {
        false => None,
        true if number(6)? == 0 => Some(None),
        true => Some(Some(Template { tag: text(7)?, file: text(8)?.ok_or_else(unread)?, line: number(9)? as i32, generated: number(10)? as u8 })),
    };
    let source = Source {
        kind: number(0)? as u8,
        identity: text(1)?.ok_or_else(unread)?,
        identity_id: number(2)? as u32,
        secondary: number(3)?,
        naming: Naming { script: text(4)?, saved: text(5)?, template },
    };
    Ok((handle, source))
}

#[derive(Default)]
pub struct StabilityParts {
    rebuild: bool,
    handles: HashMap<Source, u32>,
    next: u32,
    held: HashMap<u32, Held>,
    lines: inputs::Lines,
}

#[derive(Default)]
struct Held {
    handles: [Vec<u32>; 2],
    lined: bool,
    named: bool,
}

impl StabilityParts {
    fn restore(&mut self, hour: &Hour<'_>, keys: &TableView<'_>, parts: &TableView<'_>) -> io::Result<()> {
        self.handles.clear();
        self.next = 0;
        for slot in keys.rows() {
            let (handle, source) = dictionary_row(keys, slot)?;
            self.next = self.next.max(handle.checked_add(1).ok_or_else(|| invalid("Stability part handles are spent"))?);
            self.handles.insert(source, handle);
        }
        self.held.clear();
        for key in parts.rows().filter_map(|slot| parts.key(slot)) {
            let measure = usize::from((key >> 32) as u8);
            let held = self.held.entry((key >> 40) as u32).or_insert_with(|| Held { named: true, ..Held::default() });
            held.handles.get_mut(measure).ok_or_else(|| invalid("A StabilityParts row's measure is past war support"))?.push(key as u32);
        }
        self.lines.rebuild(hour);
        self.rebuild = false;
        Ok(())
    }

    fn handle(&mut self, source: Source, out: &mut Rows) -> io::Result<u32> {
        if let Some(&handle) = self.handles.get(&source) {
            return Ok(handle);
        }
        let handle = self.next;
        self.next = handle.checked_add(1).ok_or_else(|| invalid("Stability part handles are spent"))?;
        let mut text = |text: Option<&[u8]>| text.map_or(Value::Unrecorded, |text| out.text(text));
        let identity = text(Some(&*source.identity));
        let script = text(source.naming.script.as_deref());
        let saved = text(source.naming.saved.as_deref());
        let template = |known: fn(&Template) -> Value, none: Value| match &source.naming.template {
            None => Value::Unrecorded,
            Some(None) => none,
            Some(Some(template)) => known(template),
        };
        let present = template(|_| Value::U8(1), Value::U8(0));
        let line = template(|template| Value::I32(template.line), Value::I32(0));
        let generated = template(|template| Value::U8(template.generated), Value::U8(0));
        let (tag, file) = match &source.naming.template {
            None => (Value::Unrecorded, Value::Unrecorded),
            Some(None) => (text(Some(&b""[..])), text(Some(&b""[..]))),
            Some(Some(template)) => (text(template.tag.as_deref()), text(Some(&*template.file))),
        };
        out.set(
            TableId::StabilityPartKeys,
            u128::from(handle),
            vec![
                Value::U8(source.kind),
                identity,
                Value::U32(source.identity_id),
                Value::I64(source.secondary),
                script,
                saved,
                present,
                tag,
                file,
                line,
                generated,
            ],
        );
        self.handles.insert(source, handle);
        Ok(handle)
    }

    fn country(&mut self, hour: &Hour<'_>, id: u32, out: &mut Rows) -> io::Result<()> {
        let read = inputs::inputs_read(hour, id).filter(|_| inputs::live(hour, id));
        let row = match read {
            Some(_) => inputs::inputs(hour, id)?,
            None => None,
        };
        let (Some(read), Some(row)) = (read, row) else {
            if let Some(held) = self.held.remove(&id) {
                for (measure, handles) in MEASURES.into_iter().zip(held.handles) {
                    for handle in handles {
                        out.delete(TableId::StabilityParts, part_key(id, measure, handle));
                    }
                }
            }
            return Ok(());
        };
        let lined = Self::lined(hour, id, read);
        let lines = if lined { self.lines.of(hour, id)? } else { Vec::new() };
        let named = lines.iter().any(|line| matches!(line.kind, CountryStabilityLineRow::ADVISOR | CountryStabilityLineRow::COUNTRY_LEADER));
        let mut character = |character| inputs::character(hour, character);
        let set = |out: &mut Rows<'_>, key: u128, value: Value| out.set(TableId::StabilityParts, key, vec![value]);
        let mut now: [Vec<u32>; 2] = Default::default();
        for (measure, handles) in MEASURES.into_iter().zip(&mut now) {
            match lined.then(|| parts(&row, measure, &lines, &mut character)).flatten() {
                Some(parts) => {
                    for (source, value) in parts {
                        let handle = self.handle(source, out)?;
                        set(out, part_key(id, measure, handle), Value::I64(value));
                        handles.push(handle);
                    }
                }
                None => {
                    let handle = self.handle(Source::plain(StabilityPartKeyRow::TOTAL, b""), out)?;
                    set(out, part_key(id, measure, handle), Value::Unrecorded);
                    handles.push(handle);
                }
            }
        }
        if let Some(held) = self.held.get(&id) {
            for ((measure, before), after) in MEASURES.into_iter().zip(&held.handles).zip(&now) {
                for &handle in before {
                    if !after.contains(&handle) {
                        out.delete(TableId::StabilityParts, part_key(id, measure, handle));
                    }
                }
            }
        }
        self.held.insert(id, Held { handles: now, lined, named });
        Ok(())
    }

    fn lined(hour: &Hour<'_>, id: u32, read: i32) -> bool {
        inputs::lines_read(hour, id).is_none_or(|lines| lines == read)
    }

    fn reread_alike(&self, hour: &Hour<'_>, id: u32) -> bool {
        let Some(held) = self.held.get(&id).filter(|held| !held.named) else { return false };
        let read = inputs::inputs_read(hour, id).filter(|_| inputs::live(hour, id));
        read.is_some_and(|read| Self::lined(hour, id, read) == held.lined)
    }
}

impl Writer for StabilityParts {
    fn name(&self) -> &'static str {
        "StabilityParts"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::StabilityParts), Output::State(TableId::StabilityPartKeys)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let mut dirty = Vec::new();
        if hour.fresh || self.rebuild {
            self.restore(hour, &out.table(TableId::StabilityPartKeys), &out.table(TableId::StabilityParts))?;
            inputs::every_input(hour, &mut dirty);
            dirty.extend(self.held.keys().copied());
        } else {
            self.lines.follow(hour, &mut dirty);
            inputs::touched_inputs(hour, &mut dirty);
            inputs::touched_countries(hour, &mut dirty);
            dirty.sort_unstable();
            let mut reread = Vec::new();
            inputs::touched_reads(hour, &mut reread);
            reread.retain(|id| dirty.binary_search(id).is_err() && !self.reread_alike(hour, *id));
            dirty.extend(reread);
        }
        dirty.sort_unstable();
        dirty.dedup();
        for id in dirty {
            self.country(hour, id, out)?;
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("StabilityParts' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        let _ = self.restore(hour, &hour.table(TableId::StabilityPartKeys), &hour.table(TableId::StabilityParts));
    }
}
