//! `LookSpans` and `LookFacts`: what each tag shows, as rows whose keys hold the fact, so the
//! tables' lifetimes are the run's history ([`replay_data::rows::LookSpanRow`]). A tag's look is
//! its holder (from `TagHolders`), the holder's recorded map colour, the naming operands of its
//! slot and of every overlord its name reaches with which of slot zero's names its reserved row
//! has built, and its flag inputs ([`Look`]): everything a reader builds its name and chooses its
//! flag from, in the reader's language and mod stack. Each distinct look is a `LookFacts` row,
//! never deleted, holding it encoded; a tag's span names its fact.
//!
//! While Countries, CountryLinks, CountryAppearance or ReservedCountryNaming is not complete no
//! look is known: no tag's span lives, and an UNKNOWN span ([`TagHolderRow::UNKNOWN`]) does. Read again only at hours a column the looks read changes;
//! otherwise the hour costs nothing.

use crate::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use installed_content_country_names::country_names::{CountryNames, Operands};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::{
    format::FormatVersion,
    rows::{AutonomyRow, CountryAppearanceRow, PoliticalPartyRow, ReservedCountryNamingRow, PoliticsRow, Recorded, RowCodec, TableId, TagHolderRow, mask},
    scope::CaptureMode,
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The tables a look is known only while complete.
const WHOLE: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::ReservedCountryNaming];
/// Every table a look reads.
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

/// A country's flag inputs, as recorded: its appearance's cosmetic tag and original tag (with
/// the reference choosing it), its ruling party and that party's ideology group token. A part
/// it has no row of is None.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlagInputs {
    pub appearance: Option<(Vec<u8>, Recorded<Option<i32>>, Recorded<Option<Vec<u8>>>)>,
    pub ruling: Option<Option<u32>>,
    pub group: Option<Vec<u8>>,
}

/// What a tag shows at an hour: its holder, colour, naming chain and flag inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub slot: u32,
    pub id: u32,
    pub colour: Option<[u8; 3]>,
    /// The naming operands of the holder's slot and every overlord its name reaches, in order.
    pub chain: Vec<(u32, Result<Operands, String>)>,
    pub built: Option<u8>,
    pub flag: FlagInputs,
}

fn put(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn take(&mut self) -> io::Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self.0.split_first().ok_or_else(|| invalid("A look ends early"))?;
            self.0 = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("A look's number is too long"))
    }

    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let length = self.take()? as usize;
        if length > self.0.len() {
            return Err(invalid("A look ends early"));
        }
        let (bytes, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(bytes.to_vec())
    }

    fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid("A look's text is not UTF-8"))
    }
}

fn put_recorded<T>(out: &mut Vec<u8>, value: &Recorded<Option<T>>, each: impl Fn(&mut Vec<u8>, &T)) {
    match value {
        Recorded::NotRecorded => out.push(0),
        Recorded::Value(None) => out.push(1),
        Recorded::Value(Some(value)) => {
            out.push(2);
            each(out, value);
        }
    }
}

fn take_recorded<T>(cursor: &mut Cursor<'_>, each: impl Fn(&mut Cursor<'_>) -> io::Result<T>) -> io::Result<Recorded<Option<T>>> {
    Ok(match cursor.take()? {
        0 => Recorded::NotRecorded,
        1 => Recorded::Value(None),
        _ => Recorded::Value(Some(each(cursor)?)),
    })
}

impl Look {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![1];
        put(&mut out, u64::from(self.slot));
        put(&mut out, u64::from(self.id));
        match self.colour {
            None => out.push(0),
            Some(rgb) => {
                out.push(1);
                for value in rgb {
                    put(&mut out, u64::from(value));
                }
            }
        }
        put(&mut out, self.built.map_or(0, |built| u64::from(built) + 1));
        put(&mut out, self.chain.len() as u64);
        for (slot, operands) in &self.chain {
            put(&mut out, u64::from(*slot));
            match operands {
                Ok(operands) => {
                    out.push(0);
                    let (overlord, texts) = operands.parts();
                    put(&mut out, overlord.map_or(0, |slot| u64::from(slot) + 1));
                    for text in texts {
                        put_bytes(&mut out, text.as_bytes());
                    }
                }
                Err(reason) => {
                    out.push(1);
                    put_bytes(&mut out, reason.as_bytes());
                }
            }
        }
        match &self.flag.appearance {
            None => out.push(0),
            Some((cosmetic, reference, original)) => {
                out.push(1);
                put_bytes(&mut out, cosmetic);
                put_recorded(&mut out, reference, |out, value| put(out, u64::from(*value as u32)));
                put_recorded(&mut out, original, |out, value| put_bytes(out, value));
            }
        }
        put(&mut out, match self.flag.ruling {
            None => 0,
            Some(None) => 1,
            Some(Some(party)) => u64::from(party) + 2,
        });
        match &self.flag.group {
            None => out.push(0),
            Some(group) => {
                out.push(1);
                put_bytes(&mut out, group);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor(bytes);
        if cursor.take()? != 1 {
            return Err(invalid("A look of another version"));
        }
        let slot = cursor.take()? as u32;
        let id = cursor.take()? as u32;
        let colour = match cursor.take()? {
            0 => None,
            _ => {
                let mut rgb = [0u8; 3];
                for value in &mut rgb {
                    *value = cursor.take()? as u8;
                }
                Some(rgb)
            }
        };
        let built = match cursor.take()? {
            0 => None,
            built => Some((built - 1) as u8),
        };
        let mut chain = Vec::new();
        for _ in 0..cursor.take()? {
            let slot = cursor.take()? as u32;
            let operands = match cursor.take()? {
                0 => {
                    let overlord = match cursor.take()? {
                        0 => None,
                        slot => Some((slot - 1) as u32),
                    };
                    let mut texts: [String; 6] = Default::default();
                    for text in &mut texts {
                        *text = cursor.text()?;
                    }
                    Ok(Operands::from_parts(overlord, texts))
                }
                _ => Err(cursor.text()?),
            };
            chain.push((slot, operands));
        }
        let appearance = match cursor.take()? {
            0 => None,
            _ => {
                let cosmetic = cursor.bytes()?;
                let reference = take_recorded(&mut cursor, |cursor| Ok(cursor.take()? as u32 as i32))?;
                let original = take_recorded(&mut cursor, |cursor| cursor.bytes())?;
                Some((cosmetic, reference, original))
            }
        };
        let ruling = match cursor.take()? {
            0 => None,
            1 => Some(None),
            party => Some(Some((party - 2) as u32)),
        };
        let group = match cursor.take()? {
            0 => None,
            _ => Some(cursor.bytes()?),
        };
        Ok(Self { slot, id, colour, chain, built, flag: FlagInputs { appearance, ruling, group } })
    }
}

#[derive(Default)]
pub struct Looks {
    rebuild: bool,
    /// Each live span by tag number: its fact and epoch.
    open: HashMap<u32, (u32, u32)>,
    /// Each fact's number, by its encoding.
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
    /// Each holder row's tag number, slot and raw id, as `TagHolders` holds them this hour.
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

    /// Every held tag's look this hour.
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
            // The operands of the slot and of every overlord its name reaches, each once.
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
        // Epochs are given in tag order, so a run and its resume number rows alike.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_reads_back_as_it_was_written() {
        let look = Look {
            slot: 7,
            id: 1234,
            colour: Some([1, 2, 255]),
            chain: vec![
                (7, Ok(Operands::from_parts(Some(3), ["GER".into(), "".into(), "_sub".into(), "".into(), "_fascism".into(), "".into()]))),
                (3, Err("CountryAppearance is incomplete this hour".into())),
            ],
            built: Some(2),
            flag: FlagInputs {
                appearance: Some((b"GER_cos".to_vec(), Recorded::Value(Some(-1)), Recorded::NotRecorded)),
                ruling: Some(Some(5)),
                group: Some(b"fascism".to_vec()),
            },
        };
        assert_eq!(Look::decode(&look.encode()).unwrap(), look);
        let bare = Look { flag: FlagInputs { ruling: Some(None), ..FlagInputs::default() }, ..Look::default() };
        assert_eq!(Look::decode(&bare.encode()).unwrap(), bare);
    }
}
