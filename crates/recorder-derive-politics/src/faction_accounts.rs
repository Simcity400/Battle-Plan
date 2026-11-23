
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::rows::{CountryLinkRow, CountryRow, FactionAccountOwnerRow, FactionAccountRow, FactionMemberRow, TableId};
use std::{
    collections::{HashMap, hash_map::Entry},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const COUNTRY: [usize; 2] = columns::<CountryRow, 2>(["canonical_slot", "tag"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const MEMBER: [usize; 1] = columns::<FactionMemberRow, 1>(["faction"]);
const ACCOUNT: [usize; 1] = columns::<FactionAccountRow, 1>(["country"]);
const OWNER: [usize; 4] = columns::<FactionAccountOwnerRow, 4>(["faction", "slot", "id", "member"]);

const READS: &[Read] = &[
    Read {
        table: TableId::FactionAccounts,
        columns: Columns::These(&ACCOUNT),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::FactionMembers,
        columns: Columns::These(&MEMBER),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::Countries,
        columns: Columns::These(&COUNTRY),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::CountryLinks,
        columns: Columns::These(&LINK),
        watch: Watch::Changed,
        required: true,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Holder {
    id: u32,
    tag: Box<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Owned {
    faction: u64,
    slot: u32,
    holder: Holder,
    member: bool,
}

#[derive(Default)]
struct Indexes {
    links: HashMap<u32, u32>,
    holders: HashMap<u32, Option<Holder>>,
    members: HashMap<u32, Option<u64>>,
    joined: bool,
}

impl Indexes {
    fn holder(&self, country: u32) -> Option<(u32, &Holder)> {
        let slot = *self.links.get(&country)?;
        Some((slot, self.holders.get(&slot)?.as_ref()?))
    }

    fn read(hour: &Hour<'_>) -> io::Result<Self> {
        let mut indexes = Self::default();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(id), Some(canonical)) = (links.key(slot), links.number(LINK[0], slot)) else { continue };
            indexes.links.insert(id as u32, canonical as u32);
        }
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            let (Some(id), Some(canonical)) = (countries.key(slot), countries.number(COUNTRY[0], slot)) else { continue };
            let tag = countries.text(COUNTRY[1], slot).and_then(|text| hour.text(text)).ok_or_else(|| invalid("A country names a tag its run never added"))?;
            let holder = Holder { id: id as u32, tag: tag.into() };
            match indexes.holders.entry(canonical as u32) {
                Entry::Vacant(entry) => {
                    entry.insert(Some(holder));
                }
                Entry::Occupied(mut entry) => {
                    entry.insert(None);
                }
            }
        }
        let members = hour.table(TableId::FactionMembers);
        indexes.joined = true;
        for slot in members.rows() {
            let (Some(country), Some(faction)) = (members.key(slot), members.number(MEMBER[0], slot)) else { continue };
            let Some((canonical, _)) = indexes.holder(country as u32) else {
                indexes.joined = false;
                continue;
            };
            let faction = faction as u64;
            indexes
                .members
                .entry(canonical)
                .and_modify(|held| {
                    if *held != Some(faction) {
                        *held = None;
                    }
                })
                .or_insert(Some(faction));
        }
        Ok(indexes)
    }
}

#[derive(Default)]
pub struct FactionAccounts {
    held: HashMap<u32, Owned>,
    indexes: Option<Indexes>,
    last: Option<i32>,
}

fn key(account: u32) -> u128 {
    u128::from(account) << 64
}

fn own(key: u128) -> Option<u32> {
    (key as u64 == 0).then_some((key >> 64) as u32)
}

impl FactionAccounts {
    fn owner(indexes: &Indexes, country: Option<u32>, before: Option<&Owned>) -> Option<Owned> {
        let (slot, holder) = indexes.holder(country?)?;
        let before = before.filter(|before| before.slot == slot && before.holder == *holder);
        match indexes.members.get(&slot) {
            Some(Some(faction)) => Some(Owned { faction: *faction, slot, holder: holder.clone(), member: true }),
            Some(None) => None,
            None if !indexes.joined => None,
            None => before.map(|before| Owned { member: false, ..before.clone() }),
        }
    }

    fn settle(&mut self, account: u32, country: Option<u32>, out: &mut Rows) {
        let indexes = self.indexes.as_ref().expect("indexes read before accounts");
        let now = Self::owner(indexes, country, self.held.get(&account));
        if self.held.get(&account) == now.as_ref() {
            return;
        }
        match now {
            Some(owned) => {
                out.set(TableId::FactionAccountOwners, u128::from(account), vec![Value::U64(owned.faction), Value::U32(owned.slot), Value::U32(owned.holder.id), Value::U8(u8::from(owned.member))]);
                self.held.insert(account, owned);
            }
            None => {
                out.delete(TableId::FactionAccountOwners, u128::from(account));
                self.held.remove(&account);
            }
        }
    }

    fn accounts(hour: &Hour<'_>) -> HashMap<u32, u32> {
        let table = hour.table(TableId::FactionAccounts);
        table
            .rows()
            .filter_map(|slot| Some((own(table.key(slot)?)?, table.number(ACCOUNT[0], slot)? as u32)))
            .collect()
    }
}

impl Writer for FactionAccounts {
    fn name(&self) -> &'static str {
        "FactionAccounts"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::FactionAccountOwners)]
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
        let at = hour.hour;
        let broke = hour.fresh || self.last.is_none_or(|last| last.checked_add(1) != Some(at));
        self.last = Some(at);
        let identity = hour.wrote(TableId::Countries, Columns::These(&COUNTRY)) || hour.wrote(TableId::CountryLinks, Columns::These(&LINK)) || hour.wrote(TableId::FactionMembers, Columns::These(&MEMBER));
        if broke {
            let kept: Vec<u32> = self.held.keys().copied().collect();
            for account in kept {
                if self.held.get(&account).is_some_and(|owned| !owned.member) {
                    out.delete(TableId::FactionAccountOwners, u128::from(account));
                    self.held.remove(&account);
                }
            }
        }
        if broke || identity || self.indexes.is_none() {
            self.indexes = Some(Indexes::read(hour)?);
            let accounts = Self::accounts(hour);
            let mut every: Vec<u32> = accounts.keys().chain(self.held.keys()).copied().collect();
            every.sort_unstable();
            every.dedup();
            for account in every {
                self.settle(account, accounts.get(&account).copied(), out);
            }
            return Ok(());
        }
        if !hour.wrote(TableId::FactionAccounts, Columns::These(&ACCOUNT)) {
            return Ok(());
        }
        let table = hour.table(TableId::FactionAccounts);
        let mut touched: Vec<u32> = hour.written_rows(TableId::FactionAccounts, Columns::These(&ACCOUNT)).into_iter().filter_map(|slot| own(table.key(slot)?)).collect();
        touched.extend(hour.deleted_keys(TableId::FactionAccounts).filter_map(own));
        touched.sort_unstable();
        touched.dedup();
        for account in touched {
            let country = table.slot(key(account)).and_then(|slot| table.number(ACCOUNT[0], slot)).map(|country| country as u32);
            self.settle(account, country, out);
        }
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        *self = Self::default();
        let Ok(indexes) = Indexes::read(hour) else { return };
        let table = hour.table(TableId::FactionAccountOwners);
        for slot in table.rows() {
            let (Some(account), Some(faction), Some(canonical), Some(id), Some(member)) =
                (table.key(slot), table.number(OWNER[0], slot), table.number(OWNER[1], slot), table.number(OWNER[2], slot), table.number(OWNER[3], slot))
            else {
                continue;
            };
            let Some(Some(holder)) = indexes.holders.get(&(canonical as u32)) else { continue };
            if holder.id != id as u32 {
                continue;
            }
            self.held.insert(account as u32, Owned { faction: faction as u64, slot: canonical as u32, holder: holder.clone(), member: member != 0 });
        }
        self.indexes = Some(indexes);
        self.last = Some(hour.hour);
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("FactionAccounts' state holds nothing"));
        }
        *self = Self::default();
        Ok(())
    }
}

#[cfg(test)]
#[path = "faction_accounts/tests.rs"]
mod tests;
