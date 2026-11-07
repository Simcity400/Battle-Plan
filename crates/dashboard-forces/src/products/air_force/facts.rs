//! Air force facts, projected hour by hour from a columnar walk ([`replay::segments`]): the wing
//! census by holder slot and ownership, kept from each hour's changed wings and equipment banks
//! and sent only for the slots whose counts changed; the tags the walk's countries resolve
//! ([`dashboard_world`]) where they changed; and what the variant catalogue reads of the hour.
//! Counts name equipment by variant id: the ordered emit ([`super::emit`]), which holds the
//! catalogue over the branch, spells them as it names them then. An hour that changes nothing
//! the census reads carries only its counters.

use super::MAX_EXACT;
use dashboard_text::metadata::slot_read;
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columnar_frames::metrics,
    columns,
    campaign::Replay,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    display::stepped_format,
    format::FormatVersion,
    rows::{AirWingRow, LexicalTokenRow, TableId, VariantDefinitionRow, WingEquipmentRow},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::AirWings,
    TableId::WingEquipment,
    TableId::VariantDefinitions,
    TableId::LexicalTokens,
    TableId::VariantChanges,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// What a wing's planes are counted under: a variant, or none the bank identifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(replay_spool::Spooled)]
pub enum Part {
    Variant(u64),
    Unidentified,
}

/// A slot's wings under one ownership: whole planes (the recorded word), wings, planes by part
/// (None where a wing's bank cannot split exactly) and wings by the parts they hold, both
/// ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Counts {
    pub planes: i64,
    pub wings: i64,
    pub parts: Vec<(Part, Option<i64>)>,
    pub signatures: Vec<(Arc<[Part]>, i64)>,
}

impl Counts {
    pub(super) fn heap(&self) -> usize {
        self.parts.capacity() * size_of::<(Part, Option<i64>)>()
            + self
                .signatures
                .iter()
                .map(|(parts, _)| size_of::<(Arc<[Part]>, i64)>() + parts.len() * size_of::<Part>())
                .sum::<usize>()
    }
}

/// What the variant catalogue reads of an hour, as the row census's catalogue reads it.
#[derive(Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Names {
    /// Names carry.
    Carried,
    /// Names are not known from this hour until the next complete read.
    Forget,
    /// A complete read: each definition's equipment token spelling ("" where it has none), of
    /// every definition when `whole`, else of those that changed since the segment's last.
    Read {
        whole: bool,
        tokens: Vec<(u64, Arc<str>)>,
    },
}

/// The hour's census reads, counted as the row census counts its hours.
pub const UNREADABLE_ROSTER: u8 = 1;
pub const UNREADABLE_EFFECTIVE: u8 = 2;
pub const MISSING: u8 = 4;
pub const REFUSED: u8 = 8;
pub const REMAINDERS: u8 = 16;

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// The tags whose countries are live, with their slot and id, where they may have changed:
    /// none while the countries or their links are not complete.
    pub live: Option<Vec<(u32, (u32, u32))>>,
    /// Whether each ownership's census holds every wing, attributed.
    pub complete: [bool; 2],
    /// Slot counts by ownership: every slot holding wings when `whole` (the others hold none),
    /// else those that changed.
    pub whole: bool,
    pub counts: Vec<(u8, u32, Arc<Counts>)>,
    /// The hour's counted reads ([`UNREADABLE_ROSTER`] ...).
    pub flags: u8,
    /// The tags the countries hold that are not UTF-8, counted each hour the countries are
    /// complete, where they changed.
    pub undisplayable: Option<Vec<Vec<u8>>>,
    pub names: Names,
}

fn exact(value: i128) -> io::Result<i64> {
    i64::try_from(value)
        .ok()
        .filter(|value| value.unsigned_abs() <= MAX_EXACT as u64)
        .ok_or_else(|| io::Error::other("Air force count exceeds exact integer range"))
}

/// A wing's planes by part and its signature, as the row census counts them: the recorded
/// plane word truncates its fixed5 bank divided by 100000.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Bank {
    parts: Vec<(Part, Option<i64>)>,
    signature: Arc<[Part]>,
}

/// One wing as counted: its owners' slots (none where unattributed), its planes and bank,
/// none for a negative plane word, and its notes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Wing {
    owners: [Option<u32>; 2],
    /// Its roster owner and effective country, raw.
    raw: [u32; 2],
    planes: i64,
    bank: Option<Bank>,
    notes: u8,
}

/// Reversible part evidence: unknown contributions apart from known sums.
#[derive(Clone, Copy, Default)]
struct PartSum {
    sum: i128,
    entries: i64,
    unknown: i64,
}

#[derive(Default)]
struct Total {
    planes: i128,
    wings: i128,
    parts: BTreeMap<Part, PartSum>,
    signatures: BTreeMap<Arc<[Part]>, i64>,
}

impl Total {
    fn change(&mut self, planes: i64, bank: &Bank, sign: i64) {
        self.planes += i128::from(sign) * i128::from(planes);
        self.wings += i128::from(sign);
        for &(part, amount) in &bank.parts {
            let held = self.parts.entry(part).or_default();
            held.entries += sign;
            match amount {
                Some(amount) => held.sum += i128::from(sign) * i128::from(amount),
                None => held.unknown += sign,
            }
            if held.entries == 0 {
                self.parts.remove(&part);
            }
        }
        let count = self
            .signatures
            .entry(Arc::clone(&bank.signature))
            .or_default();
        *count += sign;
        if *count == 0 {
            self.signatures.remove(&bank.signature);
        }
    }

    fn counts(&self) -> io::Result<Counts> {
        Ok(Counts {
            planes: exact(self.planes)?,
            wings: exact(self.wings)?,
            parts: self
                .parts
                .iter()
                .map(|(&part, held)| {
                    Ok((
                        part,
                        (held.unknown == 0).then(|| exact(held.sum)).transpose()?,
                    ))
                })
                .collect::<io::Result<_>>()?,
            signatures: self
                .signatures
                .iter()
                .map(|(parts, &count)| (Arc::clone(parts), count))
                .collect(),
        })
    }
}

/// The fields a projection reads in place.
struct Fields {
    owner: usize,
    country_override: usize,
    planes: usize,
    variant: usize,
    amount: usize,
    equipment_token: usize,
    literal: usize,
    /// AirWings' owner, country_override and planes, as change-mask bits.
    wing_mask: u64,
    equipment_mask: u64,
}

/// The catalogue's operands kept between reads: each definition's equipment token, the
/// definitions each token names, and the definitions whose spelling may have changed since
/// the last read sent.
#[derive(Default)]
struct Spelling {
    /// The next read sends every definition: nothing was sent in this segment, or the
    /// catalogue forgot since.
    all: bool,
    pending: HashSet<u64>,
    tokens: HashMap<u64, u32>,
    definitions: HashMap<u32, Vec<u64>>,
}

impl Spelling {
    fn heap(&self) -> usize {
        self.pending.capacity() * 8
            + self.tokens.capacity() * 12
            + self.definitions.capacity() * size_of::<(u32, Vec<u64>)>()
            + self
                .definitions
                .values()
                .map(|ids| ids.capacity() * 8)
                .sum::<usize>()
    }

    fn unlink(&mut self, id: u64) {
        if let Some(token) = self.tokens.remove(&id)
            && let Some(ids) = self.definitions.get_mut(&token)
        {
            ids.retain(|&held| held != id);
            if ids.is_empty() {
                self.definitions.remove(&token);
            }
        }
    }

    fn link(&mut self, id: u64, token: u32) {
        self.unlink(id);
        self.tokens.insert(id, token);
        self.definitions.entry(token).or_default().push(id);
    }
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    fields: Fields,
    /// Nothing is held: the next census is read whole.
    cold: bool,
    last: Option<i32>,
    equipment: Option<bool>,
    /// Each tag's index, and the tags sent live.
    numbers: HashMap<Arc<str>, u32>,
    live: Option<Vec<(u32, (u32, u32))>>,
    /// The slots a found tag holds.
    present: HashSet<u32>,
    undisplayable: Option<Vec<Vec<u8>>>,
    wings: HashMap<u64, Wing>,
    /// Each wing's WingEquipment slots.
    banks: HashMap<u64, Vec<u32>>,
    totals: [HashMap<u32, Total>; 2],
    unattributed: [i64; 2],
    notes: [i64; 3],
    signatures: HashSet<Arc<[Part]>>,
    /// The counts last sent, by ownership and slot.
    sent: [HashMap<u32, Arc<Counts>>; 2],
    dirty: [HashSet<u32>; 2],
    spelling: Spelling,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"air projector: cold";

/// Whether the hour inserted, deleted or changed a row of `table`, or read it in part.
fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// The slots of `table` the hour inserted or wrote any of `mask` at.
fn written(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
        .filter(|(_, written)| **written & mask != 0)
        .map(|(&slot, _)| slot)
        .chain(changes.inserted.iter().copied())
        .collect();
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

impl Projector {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        Self::with(replay.format)
    }

    pub fn with(format: FormatVersion) -> io::Result<Self> {
        let field = |table, name| columns::field(table, name);
        let owner = field(TableId::AirWings, "owner")?;
        let country_override = field(TableId::AirWings, "country_override")?;
        let planes = field(TableId::AirWings, "planes")?;
        let variant = field(TableId::WingEquipment, "variant")?;
        let amount = field(TableId::WingEquipment, "amount_raw")?;
        Ok(Self {
            format,
            fields: Fields {
                owner,
                country_override,
                planes,
                variant,
                amount,
                equipment_token: field(TableId::VariantDefinitions, "equipment_token")?,
                literal: field(TableId::LexicalTokens, "literal")?,
                wing_mask: 1 << owner | 1 << country_override | 1 << planes,
                equipment_mask: 1 << variant | 1 << amount,
            },
            cold: true,
            last: None,
            equipment: None,
            numbers: HashMap::new(),
            live: None,
            present: HashSet::new(),
            undisplayable: None,
            wings: HashMap::new(),
            banks: HashMap::new(),
            totals: Default::default(),
            unattributed: [0; 2],
            notes: [0; 3],
            signatures: HashSet::new(),
            sent: Default::default(),
            dirty: Default::default(),
            spelling: Spelling {
                all: true,
                ..Spelling::default()
            },
        })
    }

    /// The tags the countries resolve, with their slots, and the tags that are not UTF-8.
    fn countries(
        &mut self,
        roster: &Roster,
        named: &mut Vec<(u32, Arc<str>)>,
    ) -> (Vec<(u32, (u32, u32))>, Vec<Vec<u8>>) {
        let mut live = Vec::new();
        let mut undisplayable = Vec::new();
        let complete = roster.complete();
        self.present.clear();
        for tag in roster.held() {
            let bytes = roster.tag(tag);
            let Ok(text) = std::str::from_utf8(bytes) else {
                undisplayable.push(bytes.to_vec());
                if complete && let Tagged::Found(country) = roster.tagged(tag) {
                    self.present.insert(country.slot);
                }
                continue;
            };
            if !complete {
                continue;
            }
            let Tagged::Found(country) = roster.tagged(tag) else {
                continue;
            };
            self.present.insert(country.slot);
            let index = match self.numbers.get(text) {
                Some(&index) => index,
                None => {
                    let index = self.numbers.len() as u32;
                    let tag: Arc<str> = Arc::from(text);
                    self.numbers.insert(Arc::clone(&tag), index);
                    named.push((index, tag));
                    index
                }
            };
            live.push((index, (country.slot, country.id)));
        }
        live.sort_unstable();
        undisplayable.sort_unstable();
        (live, undisplayable)
    }

    /// The slot `raw` counts for: its link's, where a found tag holds that slot.
    fn owner(&self, roster: &Roster, raw: u32) -> Option<u32> {
        roster
            .slot_of(raw)
            .filter(|slot| self.present.contains(slot))
    }

    fn signature(&mut self, parts: Vec<Part>) -> Arc<[Part]> {
        if let Some(held) = self.signatures.get(parts.as_slice()) {
            return Arc::clone(held);
        }
        let held: Arc<[Part]> = Arc::from(parts);
        self.signatures.insert(Arc::clone(&held));
        held
    }

    /// The wing at AirWings `slot`, its bank read from `banks` (none while WingEquipment is not
    /// complete).
    fn wing(
        &mut self,
        state: &State,
        roster: &Roster,
        slot: u32,
        entries: Option<&[(u64, i64)]>,
    ) -> io::Result<(u64, Wing)> {
        let table = TableId::AirWings;
        let held = columns::table(state, table)?;
        let id = held
            .key(slot)
            .ok_or_else(|| io::Error::other("An AirWings slot holds no row"))?
            as u64;
        let fields = &self.fields;
        let (owner, country_override, planes) =
            if [fields.owner, fields.country_override, fields.planes]
                .iter()
                .all(|&field| held.recorded(field, slot))
            {
                let at = slot as usize;
                (
                    columns::plain::<u32>(held, fields.owner)?[at],
                    columns::plain::<i32>(held, fields.country_override)?[at],
                    columns::plain::<i32>(held, fields.planes)?[at],
                )
            } else {
                let row: AirWingRow = columns::row(state, table, slot, self.format.version())?;
                (row.owner, row.country_override, row.planes)
            };
        // The effective country uses a signed-positive override.
        let effective = if country_override > 0 {
            country_override as u32
        } else {
            owner
        };
        let mut notes = 0;
        let planes = i64::from(planes);
        let bank = if planes < 0 {
            notes |= REFUSED;
            None
        } else {
            Some(self.bank(planes, entries, &mut notes)?)
        };
        Ok((
            id,
            Wing {
                owners: [self.owner(roster, owner), self.owner(roster, effective)],
                raw: [owner, effective],
                planes,
                bank,
                notes,
            },
        ))
    }

    /// A wing's bank as the row census splits it: an unrecorded bank leaves its planes
    /// unidentified; one that cannot split exactly leaves each variant unknown.
    fn bank(
        &mut self,
        planes: i64,
        entries: Option<&[(u64, i64)]>,
        notes: &mut u8,
    ) -> io::Result<Bank> {
        let Some(entries) = entries else {
            *notes |= MISSING;
            let signature = self.signature(vec![Part::Unidentified]);
            return Ok(Bank {
                parts: vec![(Part::Unidentified, Some(planes))],
                signature,
            });
        };
        let mut signature: Vec<Part> = entries
            .iter()
            .filter(|(_, amount)| *amount > 0)
            .map(|&(variant, _)| Part::Variant(variant))
            .collect();
        signature.sort_unstable();
        signature.dedup();
        if signature.is_empty() {
            signature.push(Part::Unidentified);
        }
        let signature = self.signature(signature);
        let scale = engine_native::equipment_entry::FIXED_POINT_SCALE;
        let summed: i128 = entries.iter().map(|&(_, amount)| i128::from(amount)).sum();
        let splittable = entries
            .iter()
            .all(|&(_, amount)| amount >= 0 && amount % scale == 0)
            && summed <= i128::from(planes) * i128::from(scale);
        let mut parts: BTreeMap<Part, Option<i64>> = BTreeMap::new();
        if !splittable {
            *notes |= REFUSED;
            for &(variant, _) in entries {
                parts.insert(Part::Variant(variant), None);
            }
            parts.insert(Part::Unidentified, Some(planes));
        } else {
            let mut identified: i128 = 0;
            for &(variant, amount) in entries {
                let amount = amount / scale;
                identified += i128::from(amount);
                exact(identified)?;
                let held = parts.entry(Part::Variant(variant)).or_insert(Some(0));
                if let Some(held) = held {
                    *held = exact(i128::from(*held) + i128::from(amount))?;
                }
            }
            let remainder = i128::from(planes) - identified;
            if remainder != 0 {
                *notes |= REMAINDERS;
                parts.insert(Part::Unidentified, Some(exact(remainder)?));
            }
        }
        Ok(Bank {
            parts: parts.into_iter().collect(),
            signature,
        })
    }

    /// Counts `wing` in (`sign` 1) or out (-1).
    fn count(&mut self, wing: &Wing, sign: i64) {
        for (channel, owner) in wing.owners.iter().enumerate() {
            match (owner, &wing.bank) {
                (Some(slot), Some(bank)) => {
                    let total = self.totals[channel].entry(*slot).or_default();
                    total.change(wing.planes, bank, sign);
                    if total.wings == 0 {
                        self.totals[channel].remove(slot);
                    }
                    self.dirty[channel].insert(*slot);
                }
                _ => self.unattributed[channel] += sign,
            }
        }
        for (at, note) in [MISSING, REFUSED, REMAINDERS].into_iter().enumerate() {
            if wing.notes & note != 0 {
                self.notes[at] += sign;
            }
        }
    }

    /// The bank entries of every WingEquipment slot of `wing`.
    fn entries(&self, state: &State, slots: &[u32]) -> io::Result<Vec<(u64, i64)>> {
        let table = TableId::WingEquipment;
        let held = columns::table(state, table)?;
        let version = self.format.version();
        let step = table
            .display_step(self.fields.amount)
            .filter(|_| stepped_format(version));
        let variants = columns::plain::<u64>(held, self.fields.variant)?;
        let amounts = columns::plain::<i64>(held, self.fields.amount)?;
        let mut entries = Vec::with_capacity(slots.len());
        for &slot in slots {
            if !held.recorded(self.fields.variant, slot) || !held.recorded(self.fields.amount, slot)
            {
                let row: WingEquipmentRow = columns::row(state, table, slot, version)?;
                entries.push((row.variant, row.amount_raw));
                continue;
            }
            let at = slot as usize;
            let missing = || io::Error::other("A WingEquipment slot holds no cell");
            let units = *amounts.get(at).ok_or_else(missing)?;
            let amount = match step {
                Some(step) => step.value(units)?,
                None => units,
            };
            entries.push((*variants.get(at).ok_or_else(missing)?, amount));
        }
        Ok(entries)
    }

    /// Reads every wing and bank anew.
    fn read_whole(&mut self, state: &State, roster: &Roster, equipment: bool) -> io::Result<()> {
        for channel in 0..2 {
            self.dirty[channel].extend(self.totals[channel].keys().copied());
            self.dirty[channel].extend(self.sent[channel].keys().copied());
        }
        self.wings.clear();
        self.banks.clear();
        self.totals = Default::default();
        self.unattributed = [0; 2];
        self.notes = [0; 3];
        if equipment {
            let table = TableId::WingEquipment;
            let held = columns::table(state, table)?;
            for slot in held.live().ones() {
                let key = held
                    .key(slot)
                    .ok_or_else(|| io::Error::other("A WingEquipment slot holds no row"))?;
                self.banks.entry((key >> 32) as u64).or_default().push(slot);
            }
        }
        let slots: Vec<u32> = columns::table(state, TableId::AirWings)?
            .live()
            .ones()
            .collect();
        for slot in slots {
            self.replace(state, roster, slot, equipment)?;
        }
        Ok(())
    }

    /// Counts the wing at AirWings `slot` again.
    fn replace(
        &mut self,
        state: &State,
        roster: &Roster,
        slot: u32,
        equipment: bool,
    ) -> io::Result<()> {
        let id = columns::table(state, TableId::AirWings)?
            .key(slot)
            .ok_or_else(|| io::Error::other("An AirWings slot holds no row"))?
            as u64;
        let entries = if equipment {
            let slots = self.banks.get(&id).map_or(&[][..], Vec::as_slice);
            Some(self.entries(state, slots)?)
        } else {
            None
        };
        let (id, wing) = self.wing(state, roster, slot, entries.as_deref())?;
        let old = self.wings.remove(&id);
        if old.as_ref() != Some(&wing) {
            if let Some(old) = &old {
                self.count(old, -1);
            }
            self.count(&wing, 1);
        }
        self.wings.insert(id, wing);
        Ok(())
    }

    /// The hour's changed wings and banks.
    fn read_changes(&mut self, state: &State, roster: &Roster, equipment: bool) -> io::Result<()> {
        let mut changed: HashSet<u64> = HashSet::new();
        if equipment {
            let table = TableId::WingEquipment;
            if touched(state, table)? {
                let changes = columns::changes(state, table)?;
                for (&slot, &key) in changes.deleted.iter().zip(&changes.deleted_keys) {
                    let wing = (key >> 32) as u64;
                    if let Some(slots) = self.banks.get_mut(&wing) {
                        slots.retain(|&held| held != slot);
                        if slots.is_empty() {
                            self.banks.remove(&wing);
                        }
                    }
                    changed.insert(wing);
                }
                let held = columns::table(state, table)?;
                for &slot in &changes.inserted {
                    let key = held
                        .key(slot)
                        .ok_or_else(|| io::Error::other("A WingEquipment slot holds no row"))?;
                    let wing = (key >> 32) as u64;
                    self.banks.entry(wing).or_default().push(slot);
                    changed.insert(wing);
                }
                for slot in written(state, table, self.fields.equipment_mask)? {
                    if let Some(key) = held.key(slot) {
                        changed.insert((key >> 32) as u64);
                    }
                }
            }
        }
        let table = TableId::AirWings;
        let held = columns::table(state, table)?;
        if touched(state, table)? {
            let changes = columns::changes(state, table)?;
            for &key in &changes.deleted_keys {
                if let Some(old) = self.wings.remove(&(key as u64)) {
                    self.count(&old, -1);
                }
                changed.remove(&(key as u64));
            }
            for slot in written(state, table, self.fields.wing_mask)? {
                if let Some(key) = held.key(slot) {
                    changed.insert(key as u64);
                }
            }
        }
        let mut slots: Vec<u32> = changed
            .into_iter()
            .filter_map(|wing| held.slot(u128::from(wing)))
            .collect();
        slots.sort_unstable();
        for slot in slots {
            self.replace(state, roster, slot, equipment)?;
        }
        Ok(())
    }

    /// Every attributed wing's owners again, after the links or the found tags changed.
    fn reattribute(&mut self, roster: &Roster) {
        let mut wings = std::mem::take(&mut self.wings);
        for wing in wings.values_mut() {
            let owners = [
                self.owner(roster, wing.raw[0]),
                self.owner(roster, wing.raw[1]),
            ];
            if owners != wing.owners {
                self.count(wing, -1);
                wing.owners = owners;
                self.count(wing, 1);
            }
        }
        self.wings = wings;
    }

    /// The slot counts that changed since last sent; every slot's when `whole`.
    fn counts(&mut self, whole: bool) -> io::Result<Vec<(u8, u32, Arc<Counts>)>> {
        let mut out = Vec::new();
        for channel in 0..2 {
            if whole {
                self.dirty[channel].clear();
                let mut sent = HashMap::with_capacity(self.totals[channel].len());
                let mut slots: Vec<u32> = self.totals[channel].keys().copied().collect();
                slots.sort_unstable();
                for slot in slots {
                    let counts = self.totals[channel][&slot].counts()?;
                    let counts = match self.sent[channel].get(&slot) {
                        Some(held) if **held == counts => Arc::clone(held),
                        _ => Arc::new(counts),
                    };
                    out.push((channel as u8, slot, Arc::clone(&counts)));
                    sent.insert(slot, counts);
                }
                self.sent[channel] = sent;
                continue;
            }
            let mut slots: Vec<u32> = self.dirty[channel].drain().collect();
            slots.sort_unstable();
            for slot in slots {
                let counts = match self.totals[channel].get(&slot) {
                    Some(total) => Some(total.counts()?),
                    None => None,
                };
                let held = self.sent[channel].get(&slot);
                let counts = match (counts, held) {
                    (Some(counts), Some(held)) if **held == counts => continue,
                    (None, None) => continue,
                    (Some(counts), _) => Arc::new(counts),
                    (None, Some(_)) => Arc::new(Counts::default()),
                };
                if *counts == Counts::default() {
                    self.sent[channel].remove(&slot);
                } else {
                    self.sent[channel].insert(slot, Arc::clone(&counts));
                }
                out.push((channel as u8, slot, counts));
            }
        }
        Ok(out)
    }

    /// The equipment token spelling of `token`: its literal, "" where none is recorded or it is
    /// not UTF-8.
    fn token_spelling(&self, state: &State, token: u32) -> io::Result<Arc<str>> {
        if token == 0 {
            return Ok(Arc::from(""));
        }
        let table = TableId::LexicalTokens;
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(Arc::from(""));
        };
        let literal = if held.recorded(self.fields.literal, slot) {
            let text = *columns::text(held, self.fields.literal)?
                .get(slot as usize)
                .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
            state
                .dictionary()
                .get(text)
                .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(state, table, slot, self.format.version())?.literal
        };
        Ok(Arc::from(std::str::from_utf8(&literal).unwrap_or_default()))
    }

    /// The equipment token of the VariantDefinitions row at `slot`.
    fn equipment_token(&self, state: &State, slot: u32) -> io::Result<(u64, u32)> {
        let table = TableId::VariantDefinitions;
        let held = columns::table(state, table)?;
        let id = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A VariantDefinitions slot holds no row"))?
            as u64;
        let field = self.fields.equipment_token;
        let token = if held.recorded(field, slot) {
            columns::plain::<u32>(held, field)?[slot as usize]
        } else {
            columns::row::<VariantDefinitionRow>(state, table, slot, self.format.version())?
                .equipment_token
        };
        Ok((id, token))
    }

    /// Notes the definitions whose spelling the hour may have changed.
    fn track_spelling(&mut self, state: &State) -> io::Result<()> {
        if self.spelling.all {
            return Ok(());
        }
        let definitions = TableId::VariantDefinitions;
        let lexical = TableId::LexicalTokens;
        if !columns::complete(state, definitions) || !columns::complete(state, lexical) {
            self.spelling.all = true;
            return Ok(());
        }
        if touched(state, definitions)? {
            if columns::changes(state, definitions)?.truncated {
                self.spelling.all = true;
                return Ok(());
            }
            for slot in written(state, definitions, 1 << self.fields.equipment_token)? {
                let (id, token) = self.equipment_token(state, slot)?;
                self.spelling.link(id, token);
                self.spelling.pending.insert(id);
            }
        }
        if touched(state, lexical)? {
            let changes = columns::changes(state, lexical)?;
            if changes.truncated {
                self.spelling.all = true;
                return Ok(());
            }
            let held = columns::table(state, lexical)?;
            let mut tokens: Vec<u32> = written(state, lexical, 1 << self.fields.literal)?
                .into_iter()
                .filter_map(|slot| held.key(slot).map(|key| key as u32))
                .collect();
            tokens.extend(changes.deleted_keys.iter().map(|&key| key as u32));
            for token in tokens {
                if let Some(ids) = self.spelling.definitions.get(&token) {
                    self.spelling.pending.extend(ids.iter().copied());
                }
            }
        }
        Ok(())
    }

    /// What the catalogue reads of the hour.
    fn names(
        &mut self,
        state: &State,
        hour: &segments::Hour<'_, World>,
        roster: &Roster,
    ) -> io::Result<Names> {
        let format = self.format;
        let flags = metrics(hour.manifest);
        // VariantChanges is a nonempty witness; none of its payload is a naming operand.
        let read = slot_read(format, &flags, TableId::VariantDefinitions, hour.hour)
            || (columns::complete(state, TableId::VariantChanges)
                && columns::table(state, TableId::VariantChanges)?
                    .live()
                    .ones()
                    .next()
                    .is_some());
        let complete = columns::complete(state, TableId::VariantDefinitions);
        if !read {
            if complete {
                self.track_spelling(state)?;
                return Ok(Names::Carried);
            }
            self.spelling.all = true;
            return Ok(Names::Forget);
        }
        if !complete || !roster.complete() || !columns::complete(state, TableId::LexicalTokens) {
            self.spelling.all = true;
            return Ok(Names::Forget);
        }
        self.track_spelling(state)?;
        let whole = std::mem::take(&mut self.spelling.all);
        let ids: Vec<u64> = if whole {
            self.spelling.pending.clear();
            self.spelling.tokens.clear();
            self.spelling.definitions.clear();
            let held = columns::table(state, TableId::VariantDefinitions)?;
            let slots: Vec<u32> = held.live().ones().collect();
            let mut ids = Vec::with_capacity(slots.len());
            for slot in slots {
                let (id, token) = self.equipment_token(state, slot)?;
                self.spelling.link(id, token);
                ids.push(id);
            }
            ids
        } else {
            self.spelling.pending.drain().collect()
        };
        let held = columns::table(state, TableId::VariantDefinitions)?;
        let mut spelled: HashMap<u32, Arc<str>> = HashMap::new();
        let mut tokens = Vec::with_capacity(ids.len());
        for id in ids {
            // A deleted definition keeps the name the catalogue last read.
            if held.slot(u128::from(id)).is_none() {
                continue;
            }
            let Some(&token) = self.spelling.tokens.get(&id) else {
                continue;
            };
            let spelling = match spelled.get(&token) {
                Some(spelling) => Arc::clone(spelling),
                None => {
                    let spelling = self.token_spelling(state, token)?;
                    spelled.insert(token, Arc::clone(&spelling));
                    spelling
                }
            };
            tokens.push((id, spelling));
        }
        tokens.sort_unstable_by_key(|&(id, _)| id);
        Ok(Names::Read { whole, tokens })
    }

    fn heap(&self) -> usize {
        self.numbers.capacity() * size_of::<(Arc<str>, u32)>()
            + self.numbers.keys().map(|tag| tag.len()).sum::<usize>()
            + self
                .live
                .as_ref()
                .map_or(0, |live| live.capacity() * size_of::<(u32, (u32, u32))>())
            + self.present.capacity() * 4
            + self.wings.capacity() * size_of::<(u64, Wing)>()
            + self
                .wings
                .values()
                .filter_map(|wing| wing.bank.as_ref())
                .map(|bank| bank.parts.capacity() * size_of::<(Part, Option<i64>)>())
                .sum::<usize>()
            + self.banks.capacity() * size_of::<(u64, Vec<u32>)>()
            + self
                .banks
                .values()
                .map(|slots| slots.capacity() * 4)
                .sum::<usize>()
            + self
                .totals
                .iter()
                .map(|totals| {
                    totals.capacity() * size_of::<(u32, Total)>()
                        + totals
                            .values()
                            .map(|total| {
                                total.parts.len() * (size_of::<(Part, PartSum)>() + 16)
                                    + total.signatures.len()
                                        * (size_of::<(Arc<[Part]>, i64)>() + 16)
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .signatures
                .iter()
                .map(|parts| 16 + parts.len() * size_of::<Part>())
                .sum::<usize>()
            + self
                .sent
                .iter()
                .map(|sent| {
                    sent.capacity() * size_of::<(u32, Arc<Counts>)>()
                        + sent
                            .values()
                            .map(|counts| size_of::<Counts>() + counts.heap())
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .dirty
                .iter()
                .map(|dirty| dirty.capacity() * 4)
                .sum::<usize>()
            + self.spelling.heap()
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("air.columnar", Some("air"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.cold = false;
            self.live = None;
            self.undisplayable = None;
            self.equipment = None;
            self.spelling.all = true;
            self.spelling.pending.clear();
        }
        self.last = Some(at);
        let names = self.names(state, hour, roster)?;
        let mut named = Vec::new();
        let regrouped = opens || roster.changed();
        let countries = roster.countries_complete();
        let mut live = None;
        let mut undisplayable = None;
        if !countries {
            if self.live.as_ref().is_none_or(|held| !held.is_empty()) {
                self.live = Some(Vec::new());
                live = Some(Vec::new());
            }
            self.present.clear();
        } else if regrouped {
            let (found, unreadable) = self.countries(roster, &mut named);
            if self.live.as_ref() != Some(&found) {
                live = Some(found.clone());
                self.live = Some(found);
            }
            if self.undisplayable.as_ref() != Some(&unreadable) {
                undisplayable = Some(unreadable.clone());
                self.undisplayable = Some(unreadable);
            }
        }
        if !countries
            && self
                .undisplayable
                .as_ref()
                .is_none_or(|held| !held.is_empty())
        {
            // The row census counts none while the countries are not complete.
            self.undisplayable = Some(Vec::new());
            undisplayable = Some(Vec::new());
        }
        let wings = columns::complete(state, TableId::AirWings);
        let mut facts = Hour {
            branch: hour.branch,
            hour: at,
            named,
            live,
            complete: [false; 2],
            whole: false,
            counts: Vec::new(),
            flags: 0,
            undisplayable,
            names,
        };
        if !countries {
            self.cold = true;
            return out.push(facts);
        }
        if !roster.links_complete() || !wings {
            self.cold = true;
            facts.flags = UNREADABLE_ROSTER | UNREADABLE_EFFECTIVE;
            return out.push(facts);
        }
        let equipment = columns::complete(state, TableId::WingEquipment);
        let whole = std::mem::take(&mut self.cold) || self.equipment != Some(equipment);
        self.equipment = Some(equipment);
        {
            let _cpu = replay_frame::cpu::scope("air.columnar.census", None);
            if whole {
                self.read_whole(state, roster, equipment)?;
            } else {
                if regrouped {
                    self.reattribute(roster);
                }
                self.read_changes(state, roster, equipment)?;
            }
        }
        facts.whole = whole;
        facts.counts = self.counts(whole)?;
        facts.complete = self.unattributed.map(|count| count == 0);
        for (complete, flag) in facts
            .complete
            .iter()
            .zip([UNREADABLE_ROSTER, UNREADABLE_EFFECTIVE])
        {
            if !complete {
                facts.flags |= flag;
            }
        }
        for (count, flag) in self.notes.iter().zip([MISSING, REFUSED, REMAINDERS]) {
            if *count > 0 {
                facts.flags |= flag;
            }
        }
        out.push(facts)
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved air projector",
            ));
        }
        *self = Self::with(self.format)?;
        Ok(())
    }
}
