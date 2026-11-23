
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::rows::{TableId, TagHolderRow};
use std::{collections::HashMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const COUNTRY_SLOT: usize = 0;
const COUNTRY_TAG: usize = 1;
const LINK_SLOT: usize = 0;

const READS: &[Read] = &[
    Read {
        table: TableId::Countries,
        columns: Columns::These(&[COUNTRY_SLOT, COUNTRY_TAG]),
        watch: Watch::Changed,
        required: false,
    },
    Read {
        table: TableId::CountryLinks,
        columns: Columns::These(&[LINK_SLOT]),
        watch: Watch::Changed,
        required: false,
    },
];


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Holder {
    tag: u32,
    found: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Open {
    epoch: u32,
    holder: Holder,
    first: i32,
}

#[derive(Default)]
pub struct TagHolders {
    numbers: HashMap<Box<[u8]>, u32>,
    held: HashMap<u32, Open>,
    unknown: Option<(u32, u32, i32)>,
    epochs: u32,
    named: Vec<(u32, u32)>,
    whole: bool,
}

fn key(number: u32, epoch: u32, found: Option<(u32, u32)>) -> u128 {
    let (slot, id) = found.unwrap_or((TagHolderRow::NONE, TagHolderRow::NONE));
    u128::from(number) << 96 | u128::from(epoch) << 64 | u128::from(slot) << 32 | u128::from(id)
}

fn u32_cell(table: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::U32, Held::Number(bits)) => Some(bits as u32),
        (Kind::Bytes, Held::Text(id)) => Some(id),
        _ => None,
    }
}

impl TagHolders {
    fn holders(&mut self, hour: &Hour<'_>, countries: &TableView<'_>, links: &TableView<'_>) -> io::Result<HashMap<u32, Holder>> {
        let mut by_id: Vec<(u32, u32, u32)> = Vec::with_capacity(countries.live().count());
        for slot in countries.rows() {
            let Some(id) = countries.key(slot).map(|key| key as u32) else { continue };
            let (Some(canonical), Some(tag)) = (u32_cell(countries, COUNTRY_SLOT, slot), u32_cell(countries, COUNTRY_TAG, slot)) else {
                continue;
            };
            by_id.push((id, canonical, tag));
        }
        by_id.sort_unstable();
        let mut holders: HashMap<u32, Holder> = HashMap::new();
        for (id, canonical, tag) in by_id {
            let bytes = hour.text(tag).ok_or_else(|| invalid("A country names a tag its run never added"))?;
            let next = self.numbers.len() as u32;
            let number = *self.numbers.entry(bytes.into()).or_insert_with(|| {
                self.named.push((next, tag));
                next
            });
            let resolved = links
                .slot(u128::from(id))
                .and_then(|slot| u32_cell(links, LINK_SLOT, slot))
                .is_some_and(|slot| slot == canonical);
            let holder = holders.entry(number).or_insert(Holder { tag, found: None });
            if holder.found.is_none() && resolved {
                *holder = Holder {
                    tag,
                    found: Some((canonical, id)),
                };
            }
        }
        Ok(holders)
    }

    fn epoch(&mut self) -> io::Result<u32> {
        let epoch = self.epochs;
        self.epochs = self.epochs.checked_add(1).ok_or_else(|| invalid("TagHolders' rows pass 2^32 in one run"))?;
        Ok(epoch)
    }

    fn close(number: u32, open: Open, out: &mut Rows) {
        out.delete(TableId::TagHolders, key(number, open.epoch, open.holder.found));
    }
}

impl Writer for TagHolders {
    fn name(&self) -> &'static str {
        "TagHolders"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::TagHolders), Output::State(TableId::TagNames)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn strings(&self) -> &'static [&'static str] {
        &TagHolderRow::REASONS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let (countries, links) = (hour.complete(TableId::Countries), hour.complete(TableId::CountryLinks));
        if !countries || !links {
            if self.unknown.is_none() {
                for (number, open) in self.held.drain() {
                    Self::close(number, open, out);
                }
                let at = match (countries, links) {
                    (false, true) => 0,
                    (true, false) => 1,
                    _ => 2,
                };
                let epoch = self.epoch()?;
                out.set(TableId::TagHolders, key(TagHolderRow::UNKNOWN, epoch, Some((at, TagHolderRow::NONE))), vec![Value::Bytes(hour.strings[at as usize])]);
                self.unknown = Some((epoch, at, hour.hour));
            }
            self.whole = false;
            return Ok(());
        }
        if let Some((epoch, at, _)) = self.unknown.take() {
            out.delete(TableId::TagHolders, key(TagHolderRow::UNKNOWN, epoch, Some((at, TagHolderRow::NONE))));
        }
        if self.whole
            && !hour.fresh
            && !hour.wrote(TableId::Countries, Columns::These(&[COUNTRY_SLOT, COUNTRY_TAG]))
            && !hour.wrote(TableId::CountryLinks, Columns::These(&[LINK_SLOT]))
        {
            return Ok(());
        }
        let (countries, links) = (hour.table(TableId::Countries), hour.table(TableId::CountryLinks));
        let holders = self.holders(hour, &countries, &links)?;
        for (number, tag) in self.named.drain(..) {
            out.set(TableId::TagNames, u128::from(number), vec![Value::Bytes(tag)]);
        }
        let gone: Vec<u32> = self.held.keys().copied().filter(|number| !holders.contains_key(number)).collect();
        for number in gone {
            let open = self.held.remove(&number).expect("held");
            Self::close(number, open, out);
        }
        let mut holders: Vec<(u32, Holder)> = holders.into_iter().collect();
        holders.sort_unstable_by_key(|&(number, _)| number);
        for (number, holder) in holders {
            if let Some(&open) = self.held.get(&number) {
                if open.holder == holder {
                    continue;
                }
                Self::close(number, open, out);
            }
            let epoch = self.epoch()?;
            out.set(TableId::TagHolders, key(number, epoch, holder.found), vec![Value::Bytes(holder.tag)]);
            self.held.insert(number, Open { epoch, holder, first: hour.hour });
        }
        self.whole = true;
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        let mut numbers: Vec<(&Box<[u8]>, &u32)> = self.numbers.iter().collect();
        numbers.sort_unstable_by_key(|(_, number)| **number);
        blob.varint(numbers.len() as u64)?;
        for (bytes, _) in numbers {
            blob.varint(bytes.len() as u64)?;
            blob.raw(bytes)?;
        }
        let mut held: Vec<(&u32, &Open)> = self.held.iter().collect();
        held.sort_unstable_by_key(|(number, _)| **number);
        blob.varint(held.len() as u64)?;
        for (number, open) in held {
            blob.varint(u64::from(*number))?;
            blob.varint(u64::from(open.epoch))?;
            blob.varint(u64::from(open.holder.tag))?;
            blob.varint(u64::from(open.first as u32))?;
            match open.holder.found {
                None => blob.u8(0)?,
                Some((slot, id)) => {
                    blob.u8(1)?;
                    blob.varint(u64::from(slot))?;
                    blob.varint(u64::from(id))?;
                }
            }
        }
        match self.unknown {
            None => blob.u8(0)?,
            Some((epoch, reason, first)) => {
                blob.u8(1)?;
                blob.varint(u64::from(epoch))?;
                blob.varint(u64::from(reason))?;
                blob.varint(u64::from(first as u32))?;
            }
        }
        blob.varint(u64::from(self.epochs))?;
        blob.u8(u8::from(self.whole))
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(blob);
        *self = Self::default();
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("TagHolders' state counts more tags than it holds"));
        }
        for number in 0..count as u32 {
            let length = reader.count()?;
            self.numbers.insert(reader.raw(length)?.into(), number);
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("TagHolders' state counts more holders than it holds"));
        }
        for _ in 0..count {
            let number = reader.varint_u32()?;
            let epoch = reader.varint_u32()?;
            let tag = reader.varint_u32()?;
            let first = reader.varint_u32()? as i32;
            let found = match reader.u8()? {
                0 => None,
                _ => Some((reader.varint_u32()?, reader.varint_u32()?)),
            };
            self.held.insert(number, Open { epoch, holder: Holder { tag, found }, first });
        }
        self.unknown = match reader.u8()? {
            0 => None,
            _ => Some((reader.varint_u32()?, reader.varint_u32()?, reader.varint_u32()? as i32)),
        };
        self.epochs = reader.varint_u32()?;
        self.whole = reader.u8()? != 0;
        reader.finish()
    }
}
