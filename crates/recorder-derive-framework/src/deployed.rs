use crate::framework::{Columns, Hour, columns};
use replay_columnar::store::{TableView, Value as Held};
use replay_data::{
    format::FormatVersion,
    rows::{CountryCounterInputRow, CountryLinkRow, CountryRow, DivisionRow, RowCodec, SentVolunteerRow, TableId},
};
use std::{collections::{HashMap, HashSet}, io};

pub fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub const SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
pub const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
pub const DIVISION: [usize; 4] = columns::<DivisionRow, 4>(["roster_owner", "logical_owner", "expeditionary_owner", "template"]);
pub const COUNTERS: [usize; 1] = columns::<CountryCounterInputRow, 1>(["sent_expeditionaries"]);
pub const VOLUNTEER: [usize; 1] = columns::<SentVolunteerRow, 1>(["division"]);

pub fn row<R: RowCodec>(view: &TableView<'_>, slot: u32, columns: &[usize]) -> io::Result<R> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("Slot {slot} holds no row")))?;
    let mask = columns.iter().fold(0_u64, |mask, &column| mask | 1 << column);
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(view, slot, column, format))
}

pub fn number(view: &TableView<'_>, column: usize, slot: u32) -> Option<u64> {
    match view.value(column, slot) {
        Held::Number(bits) => Some(bits),
        _ => None,
    }
}

#[derive(Default)]
pub struct Identity {
    pub links: HashMap<u32, u32>,
    pub countries: HashSet<u32>,
    pub aliases: HashMap<u32, Vec<u32>>,
}

impl Identity {
    pub fn read(hour: &Hour<'_>) -> Self {
        let mut identity = Self::default();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(key), Some(canonical)) = (links.key(slot), number(&links, LINK[0], slot)) else { continue };
            identity.links.insert(key as u32, canonical as u32);
            identity.aliases.entry(canonical as u32).or_default().push(key as u32);
        }
        identity.aliases.values_mut().for_each(|raws| raws.sort_unstable());
        let countries = hour.table(TableId::Countries);
        identity.countries.extend(countries.rows().filter_map(|slot| number(&countries, SLOT[0], slot)).map(|canonical| canonical as u32));
        identity
    }

    pub fn resolve(&self, raw: u32) -> Option<u32> {
        (raw != 0).then(|| self.links.get(&raw).copied()).flatten()
    }

    pub fn aliases(&self, slot: u32) -> &[u32] {
        self.aliases.get(&slot).map_or(&[], Vec::as_slice)
    }

    pub fn changed(hour: &Hour<'_>) -> bool {
        hour.wrote(TableId::CountryLinks, Columns::These(&LINK)) || hour.wrote(TableId::Countries, Columns::These(&SLOT))
    }
}
