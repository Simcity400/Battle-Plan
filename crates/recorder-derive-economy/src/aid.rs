
use recorder_derive_framework::{
    deployed::{COUNTERS, Identity, VOLUNTEER, invalid, number, row},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::{
    military_aid,
    rows::{CountryCounterInputRow, CountryLinkRow, CountryRow, DivisionRow, MilitaryAidRow, Recorded, RowCodec, TableId, TagHolderRow, VolunteerHostRow},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

const NONE: u32 = TagHolderRow::NONE;
const CHANNELS: [u8; 3] = [MilitaryAidRow::VOLUNTEERS, MilitaryAidRow::EXPEDITIONARIES, MilitaryAidRow::TALLIES];

const SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const DIVISION: [usize; 3] = columns::<DivisionRow, 3>(["roster_owner", "logical_owner", "expeditionary_owner"]);
const HOST: [usize; 2] = columns::<VolunteerHostRow, 2>(["host", "deployed_divisions"]);

const fn read(table: TableId, columns: &'static [usize], required: bool) -> Read {
    Read { table, columns: Columns::These(columns), watch: Watch::Changed, required }
}

const READS: &[Read] = &[
    read(TableId::Countries, &SLOT, true),
    read(TableId::CountryLinks, &LINK, true),
    read(TableId::Divisions, &DIVISION, true),
    read(TableId::SentVolunteers, &VOLUNTEER, true),
    read(TableId::CountryCounterInputs, &COUNTERS, true),
    read(TableId::VolunteerHosts, &HOST, false),
];

type Channel = Option<BTreeMap<u32, i64>>;

fn key(sender: u32, host: u32, channel: u8) -> u128 {
    MilitaryAidRow { sender, host, channel, divisions: Recorded::NotRecorded }.key()
}

#[derive(Default)]
pub struct Aid {
    rebuild: bool,
    identity: Identity,
    volunteers: HashMap<u32, BTreeMap<u32, u64>>,
    hosts: HashMap<u32, BTreeMap<u32, (u32, i32)>>,
    tallied: bool,
    references: HashMap<u32, Vec<u64>>,
    users: HashMap<u64, Vec<u32>>,
    held: HashMap<(u32, u8), Vec<u32>>,
    orphans: [HashSet<u32>; 3],
}

impl Aid {
    fn division(hour: &Hour<'_>, id: u64) -> io::Result<Option<DivisionRow>> {
        if id == 0 {
            return Ok(None);
        }
        let view = hour.table(TableId::Divisions);
        view.slot(u128::from(id)).map(|at| row(&view, at, &DIVISION)).transpose()
    }

    fn expeditionaries(hour: &Hour<'_>, raw: u32) -> io::Result<Option<Option<Vec<u64>>>> {
        let view = hour.table(TableId::CountryCounterInputs);
        let Some(at) = view.slot(u128::from(raw)) else { return Ok(None) };
        let row: CountryCounterInputRow = row(&view, at, &COUNTERS)?;
        Ok(Some(match row.sent_expeditionaries {
            Recorded::Value(Some(list)) => Some(list.to_vec()),
            _ => None,
        }))
    }

    fn volunteers(&self, hour: &Hour<'_>, slot: u32, references: &mut Vec<u64>) -> io::Result<Channel> {
        let mut lists = self.identity.aliases(slot).iter().filter_map(|raw| self.volunteers.get(raw)).filter(|list| !list.is_empty());
        let Some(list) = lists.next() else { return Ok(Some(BTreeMap::new())) };
        references.extend(list.values().copied());
        if lists.next().is_some() || list.keys().enumerate().any(|(at, &ordinal)| ordinal as usize != at) {
            return Ok(None);
        }
        let mut hosts = BTreeMap::new();
        for &id in list.values() {
            let Some(division) = Self::division(hour, id)? else { continue };
            let Some(host) = self.identity.resolve(military_aid::volunteer_host(&division)) else { return Ok(None) };
            *hosts.entry(host).or_default() += 1;
        }
        Ok(Some(hosts))
    }

    fn expeditionary(&self, hour: &Hour<'_>, slot: u32, references: &mut Vec<u64>) -> io::Result<Channel> {
        let mut rows = Vec::new();
        for &raw in self.identity.aliases(slot) {
            if let Some(list) = Self::expeditionaries(hour, raw)? {
                rows.push(list);
            }
        }
        let [Some(list)] = rows.as_slice() else { return Ok(None) };
        references.extend(list.iter().copied());
        let mut hosts = BTreeMap::new();
        for &id in list {
            let Some(division) = Self::division(hour, id)? else { continue };
            if self.identity.resolve(military_aid::owner(&division)) != Some(slot) {
                return Ok(None);
            }
            let Some(host) = self.identity.resolve(division.roster_owner) else { return Ok(None) };
            *hosts.entry(host).or_default() += 1;
        }
        Ok(Some(hosts))
    }

    fn tallies(&self, slot: u32) -> Channel {
        if !self.tallied {
            return None;
        }
        let mut vectors = self.identity.aliases(slot).iter().filter_map(|raw| self.hosts.get(raw)).filter(|vector| !vector.is_empty());
        let Some(vector) = vectors.next() else { return Some(BTreeMap::new()) };
        if vectors.next().is_some() {
            return None;
        }
        let mut hosts = BTreeMap::new();
        for &(host, count) in vector.values() {
            hosts.entry(self.identity.resolve(host)?).or_insert(i64::from(count));
        }
        Some(hosts)
    }

    fn sender(&mut self, hour: &Hour<'_>, slot: u32, out: &mut Rows) -> io::Result<()> {
        for id in self.references.remove(&slot).unwrap_or_default() {
            if let Some(users) = self.users.get_mut(&id) {
                users.retain(|&user| user != slot);
                if users.is_empty() {
                    self.users.remove(&id);
                }
            }
        }
        let mut channels: [Channel; 3] = [Some(BTreeMap::new()), Some(BTreeMap::new()), Some(BTreeMap::new())];
        if self.identity.countries.contains(&slot) {
            let mut references = Vec::new();
            channels = [self.volunteers(hour, slot, &mut references)?, self.expeditionary(hour, slot, &mut references)?, self.tallies(slot)];
            references.retain(|&id| id != 0);
            references.sort_unstable();
            references.dedup();
            for &id in &references {
                self.users.entry(id).or_default().push(slot);
            }
            if !references.is_empty() {
                self.references.insert(slot, references);
            }
        }
        for (channel, now) in CHANNELS.into_iter().zip(channels) {
            let rows: Vec<(u32, Value)> = match now {
                Some(hosts) => hosts.into_iter().map(|(host, count)| (host, Value::I64(count))).collect(),
                None => vec![(NONE, Value::Unrecorded)],
            };
            self.replace(slot, channel, rows, out);
        }
        Ok(())
    }

    fn replace(&mut self, sender: u32, channel: u8, rows: Vec<(u32, Value)>, out: &mut Rows) {
        let held = self.held.remove(&(sender, channel)).unwrap_or_default();
        for host in held {
            if !rows.iter().any(|&(now, _)| now == host) {
                out.delete(TableId::MilitaryAid, key(sender, host, channel));
            }
        }
        let hosts: Vec<u32> = rows.iter().map(|&(host, _)| host).collect();
        for (host, count) in rows {
            out.set(TableId::MilitaryAid, key(sender, host, channel), vec![count]);
        }
        if !hosts.is_empty() {
            self.held.insert((sender, channel), hosts);
        }
    }

    fn orphan(&self, hour: &Hour<'_>, raw: u32, channel: u8) -> io::Result<bool> {
        Ok(match channel {
            MilitaryAidRow::VOLUNTEERS => self.volunteers.get(&raw).is_some_and(|list| !list.is_empty()),
            MilitaryAidRow::TALLIES => self.hosts.get(&raw).is_some_and(|vector| !vector.is_empty()),
            _ => match Self::expeditionaries(hour, raw)? {
                None => false,
                Some(list) => list.is_none_or(|list| !list.is_empty()),
            },
        })
    }

    fn note(&mut self, hour: &Hour<'_>, raw: u32, channel: u8) -> io::Result<Option<u32>> {
        if let Some(slot) = self.identity.resolve(raw) {
            return Ok(Some(slot));
        }
        let at = usize::from(channel);
        if self.orphan(hour, raw, channel)? {
            self.orphans[at].insert(raw);
        } else {
            self.orphans[at].remove(&raw);
        }
        Ok(None)
    }

    fn orphan_rows(&mut self, out: &mut Rows) {
        for channel in CHANNELS {
            let rows = if self.orphans[usize::from(channel)].is_empty() { Vec::new() } else { vec![(NONE, Value::Unrecorded)] };
            self.replace(NONE, channel, rows, out);
        }
    }

    fn host(&mut self, view: &TableView<'_>, slot: u32) -> Option<u32> {
        let found = view.key(slot)?;
        let (host, count) = (number(view, HOST[0], slot)?, number(view, HOST[1], slot)?);
        let raw = (found >> 32) as u32;
        self.hosts.entry(raw).or_default().insert(found as u32, (host as u32, count as i32));
        Some(raw)
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        *self = Self { identity: Identity::read(hour), tallied: hour.complete(TableId::VolunteerHosts), ..Self::default() };
        let table = out.table(TableId::MilitaryAid);
        for found in table.rows().filter_map(|slot| table.key(slot)) {
            let row = MilitaryAidRow::from_wire_fields(found, 0, &mut |_| Err(invalid("A military aid key is read alone")))?;
            self.held.entry((row.sender, row.channel)).or_default().push(row.host);
        }
        let volunteers = hour.table(TableId::SentVolunteers);
        for slot in volunteers.rows() {
            let (Some(found), Some(division)) = (volunteers.key(slot), number(&volunteers, VOLUNTEER[0], slot)) else { continue };
            self.volunteers.entry((found >> 32) as u32).or_default().insert(found as u32, division);
        }
        let raws: Vec<u32> = self.volunteers.keys().copied().collect();
        for raw in raws {
            self.note(hour, raw, MilitaryAidRow::VOLUNTEERS)?;
        }
        let counters = hour.table(TableId::CountryCounterInputs);
        let raws: Vec<u32> = counters.rows().filter_map(|slot| counters.key(slot)).map(|found| found as u32).collect();
        for raw in raws {
            self.note(hour, raw, MilitaryAidRow::EXPEDITIONARIES)?;
        }
        if self.tallied {
            let hosts = hour.table(TableId::VolunteerHosts);
            let raws: Vec<u32> = hosts.rows().filter_map(|slot| self.host(&hosts, slot)).collect();
            for raw in raws {
                self.note(hour, raw, MilitaryAidRow::TALLIES)?;
            }
        }
        let mut senders: Vec<u32> = self.identity.countries.iter().copied().chain(self.held.keys().map(|&(sender, _)| sender)).filter(|&sender| sender != NONE).collect();
        senders.sort_unstable();
        senders.dedup();
        for slot in senders {
            self.sender(hour, slot, out)?;
        }
        self.orphan_rows(out);
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let mut senders = HashSet::new();
        let divisions = hour.table(TableId::Divisions);
        let changed = hour.deleted_keys(TableId::Divisions).chain(hour.written_rows(TableId::Divisions, Columns::These(&DIVISION)).into_iter().filter_map(|slot| divisions.key(slot)));
        for id in changed {
            senders.extend(self.users.get(&(id as u64)).into_iter().flatten().copied());
        }
        let deleted: Vec<u128> = hour.deleted_keys(TableId::SentVolunteers).collect();
        for found in deleted {
            let raw = (found >> 32) as u32;
            if let Some(list) = self.volunteers.get_mut(&raw) {
                list.remove(&(found as u32));
                if list.is_empty() {
                    self.volunteers.remove(&raw);
                }
            }
            senders.extend(self.note(hour, raw, MilitaryAidRow::VOLUNTEERS)?);
        }
        let volunteers = hour.table(TableId::SentVolunteers);
        for slot in hour.written_rows(TableId::SentVolunteers, Columns::These(&VOLUNTEER)) {
            let (Some(found), Some(division)) = (volunteers.key(slot), number(&volunteers, VOLUNTEER[0], slot)) else { continue };
            let raw = (found >> 32) as u32;
            self.volunteers.entry(raw).or_default().insert(found as u32, division);
            senders.extend(self.note(hour, raw, MilitaryAidRow::VOLUNTEERS)?);
        }
        let counters = hour.table(TableId::CountryCounterInputs);
        let mut raws: Vec<u32> = hour.deleted_keys(TableId::CountryCounterInputs).map(|found| found as u32).collect();
        raws.extend(hour.written_rows(TableId::CountryCounterInputs, Columns::These(&COUNTERS)).into_iter().filter_map(|slot| counters.key(slot)).map(|found| found as u32));
        for raw in raws {
            senders.extend(self.note(hour, raw, MilitaryAidRow::EXPEDITIONARIES)?);
        }
        if self.tallied {
            let deleted: Vec<u128> = hour.deleted_keys(TableId::VolunteerHosts).collect();
            for found in deleted {
                let raw = (found >> 32) as u32;
                if let Some(vector) = self.hosts.get_mut(&raw) {
                    vector.remove(&(found as u32));
                    if vector.is_empty() {
                        self.hosts.remove(&raw);
                    }
                }
                senders.extend(self.note(hour, raw, MilitaryAidRow::TALLIES)?);
            }
            let hosts = hour.table(TableId::VolunteerHosts);
            for slot in hour.written_rows(TableId::VolunteerHosts, Columns::These(&HOST)) {
                if let Some(raw) = self.host(&hosts, slot) {
                    senders.extend(self.note(hour, raw, MilitaryAidRow::TALLIES)?);
                }
            }
        }
        let mut senders: Vec<u32> = senders.into_iter().collect();
        senders.sort_unstable();
        for slot in senders {
            self.sender(hour, slot, out)?;
        }
        self.orphan_rows(out);
        Ok(())
    }
}

impl Writer for Aid {
    fn name(&self) -> &'static str {
        "Aid"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::MilitaryAid)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.fresh || self.rebuild || Identity::changed(hour) || hour.complete(TableId::VolunteerHosts) != self.tallied {
            self.rebuild(hour, out)?;
        } else {
            self.apply(hour, out)?;
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Aid's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
