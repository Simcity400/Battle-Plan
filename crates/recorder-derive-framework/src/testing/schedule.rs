use replay_columnar::{manifest::Status, state::{Cell, Kind}, store::Store};
use replay_data::rows::{TABLE_COUNT, TableId};
use std::io;

pub const LOG: TableId = TableId::MarketEvents;
pub const ORDERED: TableId = TableId::WarEvents;
pub const EVENTS_TABLES: [TableId; 2] = [TableId::ArmyTotals, TableId::ArmyMix];
pub const TOY: TableId = TableId::NuclearStrikes;
pub const TOY_KINDS: [Kind; 4] = [Kind::U64, Kind::Bytes, Kind::Opaque, Kind::OptionalI32];

pub(crate) fn schema() -> Vec<Vec<Kind>> {
    let mut schema = vec![Vec::new(); TABLE_COUNT];
    schema[TableId::Countries.index()] = vec![Kind::U32, Kind::Bytes];
    schema[TableId::CountryLinks.index()] = vec![Kind::U32];
    schema[TableId::TagHolders.index()] = vec![Kind::Bytes];
    schema[TableId::TagNames.index()] = vec![Kind::Bytes];
    schema[TableId::Campaign.index()] = vec![Kind::Bytes];
    schema[TableId::CountryStability.index()] = vec![Kind::I64, Kind::I64, Kind::I64];
    schema[LOG.index()] = vec![Kind::U64, Kind::I32];
    schema[ORDERED.index()] = vec![Kind::U64, Kind::OptionalI64, Kind::Bytes, Kind::Opaque];
    let [by_key, by_ordinal] = EVENTS_TABLES;
    schema[by_key.index()] = vec![Kind::U64, Kind::U64];
    schema[by_ordinal.index()] = vec![Kind::U64];
    schema[TOY.index()] = TOY_KINDS.to_vec();
    schema
}

pub fn store() -> Store {
    Store::new(schema(), vec![false; TABLE_COUNT])
}

pub fn apply(store: &mut Store, hour: i32, countries: &[(u32, u32, u32)], links: &[(u32, u32)], status: Status) {
    let appends: &[&[u8]] = if hour == 0 { &[b"GER", b"ENG"] } else { &[] };
    apply_with(store, hour, appends, countries, links, status);
}

pub fn apply_with(store: &mut Store, hour: i32, appends: &[&[u8]], countries: &[(u32, u32, u32)], links: &[(u32, u32)], status: Status) {
    store.begin_hour(hour, appends).unwrap();
    for table in [TableId::Countries, TableId::CountryLinks] {
        let index = table.index();
        let held = if table == TableId::Countries { status } else { Status::Complete };
        store.status(index, held, true, Some(hour), true).unwrap();
        let live: Vec<u32> = store.view(index).rows().collect();
        let mut changes = store.table(index).unwrap();
        for slot in live {
            changes.delete(slot).unwrap();
        }
        match table {
            TableId::Countries => {
                for &(id, slot, tag) in countries {
                    let at = changes.insert(u128::from(id)).unwrap();
                    changes.write(at, 0, Cell::U32(slot)).unwrap();
                    changes.write(at, 1, Cell::Bytes(tag)).unwrap();
                }
            }
            _ => {
                for &(id, slot) in links {
                    let at = changes.insert(u128::from(id)).unwrap();
                    changes.write(at, 0, Cell::U32(slot)).unwrap();
                }
            }
        }
    }
    store.settle();
}

pub fn reason(store: &Store, table: usize) -> String {
    match store.view(table).status() {
        Status::Unavailable { reason } => String::from_utf8_lossy(store.strings().get(reason).unwrap()).into_owned(),
        other => panic!("{other:?}"),
    }
}

pub fn rows_of(store: &Store, table: TableId) -> Vec<(u128, Option<Vec<u8>>)> {
    let view = store.view(table.index());
    let mut rows: Vec<_> = view
        .rows()
        .map(|slot| (view.key(slot).unwrap(), view.text(0, slot).and_then(|id| view.string(id)).map(<[u8]>::to_vec)))
        .collect();
    rows.sort_unstable();
    rows
}

#[derive(Default)]
pub struct Memory {
    pub bytes: Vec<u8>,
    pub pages: Vec<replay_columnar::unit::BlockPage>,
}

impl replay_columnar::unit::PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let at = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(at)
    }

    fn block(&mut self, page: replay_columnar::unit::BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, at: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[at as usize..at as usize + stored as usize]);
        Ok(())
    }
}
