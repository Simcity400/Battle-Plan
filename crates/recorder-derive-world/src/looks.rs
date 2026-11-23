
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use content_reader_country_names::country_names::CountryNames;
use replay_columnar::leaf::Writer as Bytes;
use replay_data::{
    format::FormatVersion,
    rows::{AutonomyRow, CountryAppearanceRow, PoliticalPartyRow, ReservedCountryNamingRow, PoliticsRow, RowCodec, TableId, TagHolderRow, mask},
    scope::CaptureMode,
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const WHOLE: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::ReservedCountryNaming];
const TABLES: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::ReservedCountryNaming,
    TableId::TagHolders,
];

const APPEARANCE_NAMES: [&str; 9] = [
    "red",
    "green",
    "blue",
    "cosmetic_tag",
    "name_tag",
    "leader_ideology",
    "overlord",
    "original_tag",
    "original_tag_reference",
];
const RESERVED_NAMES: [&str; 8] = ["tag", "name_tag", "cosmetic_tag", "leader_ideology", "group", "overlord", "autonomy", "built"];
const APPEARANCE: u64 = mask::<CountryAppearanceRow>(&APPEARANCE_NAMES);
const RULING: u64 = mask::<PoliticsRow>(&["ruling_party"]);
const TOKEN: u64 = mask::<PoliticalPartyRow>(&["token"]);
const APPEARANCE_COLUMNS: [usize; 9] = columns::<CountryAppearanceRow, 9>(APPEARANCE_NAMES);
const RULING_COLUMNS: [usize; 1] = columns::<PoliticsRow, 1>(["ruling_party"]);
const AUTONOMY_COLUMNS: [usize; 1] = columns::<AutonomyRow, 1>(["state"]);
const RESERVED_COLUMNS: [usize; 8] = columns::<ReservedCountryNamingRow, 8>(RESERVED_NAMES);
const TOKEN_COLUMNS: [usize; 1] = columns::<PoliticalPartyRow, 1>(["token"]);

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&[0, 1]), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryAppearance, columns: Columns::These(&APPEARANCE_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::Politics, columns: Columns::These(&RULING_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::PoliticalParties, columns: Columns::These(&TOKEN_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::Autonomy, columns: Columns::These(&AUTONOMY_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::ReservedCountryNaming, columns: Columns::These(&RESERVED_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::TagHolders, columns: Columns::Every, watch: Watch::Changed, required: false },
];

pub use replay_looks::{FlagInputs, Look};

#[derive(Default)]
pub struct Looks {
    rebuild: bool,
    open: HashMap<u32, (u32, u32)>,
    facts: HashMap<Vec<u8>, u32>,
    epochs: u32,
}

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, key: u128, mask: u64) -> io::Result<Option<R>> {
    let held = hour.table(table);
    let Some(slot) = held.slot(key) else { return Ok(None) };
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format)).map(Some)
}

fn span_key(number: u32, fact: u32, epoch: u32) -> u128 {
    u128::from(number) << 64 | u128::from(fact) << 32 | u128::from(epoch)
}

impl Looks {
    fn holders(hour: &Hour<'_>) -> io::Result<Vec<(u32, u32, u32)>> {
        let rows = hour.table(TableId::TagHolders);
        let mut out = Vec::new();
        for slot in rows.rows() {
            let Some(key) = rows.key(slot) else { continue };
            let (number, holder, id) = ((key >> 96) as u32, (key >> 32) as u32, key as u32);
            if number != TagHolderRow::UNKNOWN && holder != TagHolderRow::NONE {
                out.push((number, holder, id));
            }
        }
        Ok(out)
    }

    fn looks(&mut self, hour: &Hour<'_>) -> io::Result<BTreeMap<u32, Look>> {
        let frame = replay_frame::frame::stored_frame(
            TABLES.iter().map(|&table| (table, hour.table(table))),
            (hour.branch, hour.hour),
            CaptureMode::Full,
            FormatVersion::CURRENT.version(),
        )?;
        let (operands, built) = CountryNames::recorded_operands(&frame);
        let mut out = BTreeMap::new();
        for (number, slot, id) in Self::holders(hour)? {
            let mut chain = Vec::new();
            let mut next = Some(slot);
            let mut reaches = Vec::new();
            while let Some(at) = next.filter(|at| !reaches.contains(at)) {
                reaches.push(at);
                next = None;
                if let Some(held) = operands.get(&at) {
                    next = held.as_ref().ok().and_then(|held| held.overlord);
                    chain.push((at, held.clone()));
                }
            }
            let key = u128::from(id);
            let appearance: Option<CountryAppearanceRow> = row(hour, TableId::CountryAppearance, key, APPEARANCE)?;
            let politics: Option<PoliticsRow> = row(hour, TableId::Politics, key, RULING)?;
            let ruling = politics.map(|row| row.ruling_party);
            let group = match ruling.flatten() {
                Some(party) => row::<PoliticalPartyRow>(hour, TableId::PoliticalParties, key << 32 | u128::from(party), TOKEN)?.map(|row| row.token),
                None => None,
            };
            out.insert(number, Look {
                slot,
                id,
                colour: appearance.as_ref().map(|row| row.map_color()),
                built: reaches.contains(&0).then_some(built).flatten(),
                chain,
                flag: FlagInputs {
                    appearance: appearance.map(|row| (row.cosmetic_tag, row.original_tag_reference, row.original_tag)),
                    ruling,
                    group,
                },
            });
        }
        Ok(out)
    }

    fn epoch(&mut self) -> io::Result<u32> {
        let epoch = self.epochs;
        self.epochs = self.epochs.checked_add(1).ok_or_else(|| invalid("Look rows pass 2^32 in one run"))?;
        Ok(epoch)
    }
}

impl Writer for Looks {
    fn name(&self) -> &'static str {
        "Looks"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::LookSpans), Output::State(TableId::LookFacts)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let whole = hour.fresh || std::mem::take(&mut self.rebuild);
        if !whole && !READS.iter().any(|read| hour.wrote(read.table, read.columns)) {
            return Ok(());
        }
        let known = WHOLE.iter().all(|&table| hour.complete(table));
        let looks = if known { self.looks(hour)? } else { BTreeMap::new() };
        let mut now: HashMap<u32, u32> = HashMap::with_capacity(looks.len() + 1);
        if !known {
            now.insert(TagHolderRow::UNKNOWN, 0);
        }
        for (number, look) in looks {
            let encoded = look.encode();
            let next = self.facts.len() as u32;
            let fact = match self.facts.get(&encoded) {
                Some(&fact) => fact,
                None => {
                    let text = out.text(&encoded);
                    out.set(TableId::LookFacts, u128::from(next), vec![text]);
                    self.facts.insert(encoded, next);
                    next
                }
            };
            now.insert(number, fact);
        }
        let open = std::mem::take(&mut self.open);
        for (&number, &(fact, epoch)) in &open {
            if now.get(&number) != Some(&fact) {
                out.delete(TableId::LookSpans, span_key(number, fact, epoch));
            }
        }
        let mut now: Vec<(u32, u32)> = now.into_iter().collect();
        now.sort_unstable();
        for (number, fact) in now {
            match open.get(&number) {
                Some(&(held, epoch)) if held == fact => {
                    self.open.insert(number, (fact, epoch));
                }
                _ => {
                    let epoch = self.epoch()?;
                    out.set(TableId::LookSpans, span_key(number, fact, epoch), vec![Value::U8(0)]);
                    self.open.insert(number, (fact, epoch));
                }
            }
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        blob.varint(u64::from(self.epochs))?;
        let mut open: Vec<(&u32, &(u32, u32))> = self.open.iter().collect();
        open.sort_unstable();
        blob.varint(open.len() as u64)?;
        for (&number, &(fact, epoch)) in open {
            for value in [number, fact, epoch] {
                blob.varint(u64::from(value))?;
            }
        }
        let mut facts: Vec<(&Vec<u8>, &u32)> = self.facts.iter().collect();
        facts.sort_unstable_by_key(|(_, number)| **number);
        blob.varint(facts.len() as u64)?;
        for (encoded, _) in facts {
            blob.varint(encoded.len() as u64)?;
            blob.raw(encoded)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = replay_columnar::leaf::Reader::new(blob);
        self.epochs = reader.varint_u32()?;
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("Looks' state counts more spans than it holds"));
        }
        for _ in 0..count {
            let (number, fact, epoch) = (reader.varint_u32()?, reader.varint_u32()?, reader.varint_u32()?);
            self.open.insert(number, (fact, epoch));
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("Looks' state counts more facts than it holds"));
        }
        for number in 0..count as u32 {
            let length = reader.count()?;
            self.facts.insert(reader.raw(length)?.to_vec(), number);
        }
        self.rebuild = true;
        reader.finish()
    }
}
