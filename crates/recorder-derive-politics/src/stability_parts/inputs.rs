
use recorder_derive_framework::framework::{Columns, Hour, columns};
use replay_data::{
    format::FormatVersion,
    rows::{CharacterRow, CountryStabilityInputRow, CountryStabilityLineRow, LexicalTokenRow, RowCodec, SliceReadRow, TableId},
};
use std::{collections::HashMap, io};

pub(super) fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const INPUT_COLUMNS: u64 = (1 << 35) - 1;
const CHARACTER: [usize; 7] =
    columns::<CharacterRow, 7>(["name_token_id", "token", "template_present", "template_tag", "template_file", "template_line", "template_generated"]);
const LITERAL: [usize; 1] = columns::<LexicalTokenRow, 1>(["literal"]);
const SAMPLE_HOUR: [usize; 1] = columns::<SliceReadRow, 1>(["sample_hour"]);

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

fn edited<'a>(hour: &'a Hour<'a>, table: TableId) -> impl Iterator<Item = u128> + 'a {
    let held = hour.table(table);
    let slots = hour.written_rows(table, Columns::Every);
    slots.into_iter().filter_map(move |slot| held.key(slot)).chain(hour.deleted_keys(table))
}

pub(super) fn live(hour: &Hour<'_>, id: u32) -> bool {
    hour.table(TableId::Countries).slot(u128::from(id)).is_some()
}

pub(super) fn touched_countries(hour: &Hour<'_>, out: &mut Vec<u32>) {
    let changes = hour.changes(TableId::Countries);
    let held = hour.table(TableId::Countries);
    out.extend(changes.inserted.iter().filter_map(|&slot| held.key(slot)).map(|key| key as u32));
    out.extend(changes.deleted.iter().map(|&(_, key)| key as u32));
}

pub(super) fn inputs(hour: &Hour<'_>, id: u32) -> io::Result<Option<CountryStabilityInputRow>> {
    let held = hour.table(TableId::CountryStabilityInputs);
    held.slot(u128::from(id)).map(|slot| row(hour, TableId::CountryStabilityInputs, slot, INPUT_COLUMNS)).transpose()
}

pub(super) fn every_input(hour: &Hour<'_>, out: &mut Vec<u32>) {
    let held = hour.table(TableId::CountryStabilityInputs);
    out.extend(held.rows().filter_map(|slot| held.key(slot)).map(|key| key as u32));
}

pub(super) fn touched_inputs(hour: &Hour<'_>, out: &mut Vec<u32>) {
    out.extend(edited(hour, TableId::CountryStabilityInputs).map(|key| key as u32));
}

fn read_key(table: TableId, id: u32) -> u128 {
    (table.index() as u128) << 32 | u128::from(id)
}

pub(super) fn inputs_read(hour: &Hour<'_>, id: u32) -> Option<i32> {
    read_hour(hour, TableId::CountryStabilityInputs, id)
}

pub(super) fn lines_read(hour: &Hour<'_>, id: u32) -> Option<i32> {
    read_hour(hour, TableId::CountryStabilityLines, id)
}

fn read_hour(hour: &Hour<'_>, table: TableId, id: u32) -> Option<i32> {
    let held = hour.table(TableId::SliceReads);
    let slot = held.slot(read_key(table, id))?;
    held.number(SAMPLE_HOUR[0], slot).map(|hour| hour as i32)
}

pub(super) fn touched_reads(hour: &Hour<'_>, out: &mut Vec<u32>) {
    let ours = [TableId::CountryStabilityInputs.index() as u128, TableId::CountryStabilityLines.index() as u128];
    out.extend(edited(hour, TableId::SliceReads).filter(|key| ours.contains(&(key >> 32))).map(|key| key as u32));
}

#[derive(Default)]
pub(super) struct Lines(HashMap<u32, Vec<u64>>);

fn line_country(key: u128) -> u32 {
    (key >> 64) as u32
}

impl Lines {
    pub(super) fn rebuild(&mut self, hour: &Hour<'_>) {
        self.0.clear();
        let held = hour.table(TableId::CountryStabilityLines);
        for key in held.rows().filter_map(|slot| held.key(slot)) {
            self.0.entry(line_country(key)).or_default().push(key as u64);
        }
        for keys in self.0.values_mut() {
            keys.sort_unstable();
        }
    }

    pub(super) fn follow(&mut self, hour: &Hour<'_>, out: &mut Vec<u32>) {
        let changes = hour.changes(TableId::CountryStabilityLines);
        if changes.whole {
            self.rebuild(hour);
            out.extend(self.0.keys().copied());
            return;
        }
        let held = hour.table(TableId::CountryStabilityLines);
        for &(_, key) in &changes.deleted {
            let country = line_country(key);
            if let Some(keys) = self.0.get_mut(&country) {
                if let Ok(at) = keys.binary_search(&(key as u64)) {
                    keys.remove(at);
                }
                if keys.is_empty() {
                    self.0.remove(&country);
                }
            }
            out.push(country);
        }
        for key in changes.inserted.iter().filter_map(|&slot| held.key(slot)) {
            let keys = self.0.entry(line_country(key)).or_default();
            if let Err(at) = keys.binary_search(&(key as u64)) {
                keys.insert(at, key as u64);
            }
            out.push(line_country(key));
        }
        out.extend(changes.written.iter().filter_map(|&(slot, _)| held.key(slot)).map(line_country));
    }

    pub(super) fn of(&self, hour: &Hour<'_>, id: u32) -> io::Result<Vec<CountryStabilityLineRow>> {
        let Some(keys) = self.0.get(&id) else {
            return Ok(Vec::new());
        };
        let held = hour.table(TableId::CountryStabilityLines);
        keys.iter()
            .map(|&below| {
                let key = u128::from(id) << 64 | u128::from(below);
                let slot = held.slot(key).ok_or_else(|| invalid("A stability line's key holds no row"))?;
                row(hour, TableId::CountryStabilityLines, slot, u64::MAX)
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Naming {
    pub(super) script: Option<Box<[u8]>>,
    pub(super) saved: Option<Box<[u8]>>,
    pub(super) template: Option<Option<Template>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Template {
    pub(super) tag: Option<Box<[u8]>>,
    pub(super) file: Box<[u8]>,
    pub(super) line: i32,
    pub(super) generated: u8,
}

impl Naming {
    pub(super) fn none() -> Self {
        Self { script: Some(Box::default()), saved: Some(Box::default()), template: Some(None) }
    }

    pub(super) fn unknown() -> Self {
        Self { script: None, saved: None, template: None }
    }
}

pub(super) fn character(hour: &Hour<'_>, id: u32) -> Naming {
    if !hour.complete(TableId::Characters) {
        return Naming::unknown();
    }
    let held = hour.table(TableId::Characters);
    let Some(slot) = held.slot(u128::from(id)) else {
        return Naming::unknown();
    };
    let text = |column: usize| held.text(column, slot).and_then(|text| held.string(text)).map(Box::from);
    let script = held.number(CHARACTER[0], slot).and_then(|token| match u32::try_from(token) {
        Err(_) => Some(Box::default()),
        Ok(token) => match spelling(hour, token) {
            Some(literal) => Some(literal),
            None if token == 0 => Some(Box::default()),
            None => None,
        },
    });
    let template = match held.number(CHARACTER[2], slot) {
        None => None,
        Some(0) => Some(None),
        Some(_) => (|| {
            Some(Some(Template {
                tag: held.number(CHARACTER[3], slot).and_then(|token| spelling(hour, token as u32)),
                file: text(CHARACTER[4])?,
                line: held.number(CHARACTER[5], slot)? as i32,
                generated: held.number(CHARACTER[6], slot)? as u8,
            }))
        })(),
    };
    Naming { script, saved: text(CHARACTER[1]), template }
}

fn spelling(hour: &Hour<'_>, token: u32) -> Option<Box<[u8]>> {
    if !hour.complete(TableId::LexicalTokens) {
        return None;
    }
    let held = hour.table(TableId::LexicalTokens);
    let slot = held.slot(u128::from(token))?;
    held.text(LITERAL[0], slot).and_then(|text| held.string(text)).map(Box::from)
}
