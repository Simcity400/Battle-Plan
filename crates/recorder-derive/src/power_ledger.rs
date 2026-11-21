
use crate::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
};
use replay_data::rows::{
    CountryPowerChangeRow, CountryPowerDailyCostRow, PoliticsCaptureCoverageRow as Streams, PowerCause, PowerCurrency, PowerDayPartRow as Part, PowerHoldRow as Hold, PowerIdentity,
    PowerItemRow as Item, PowerObjectKind, PowerOperation, PowerSite, PowerSpendCoverageRow as Coverage, PowerSpendTotalRow as Total, Recorded, TableId, TagHolderRow,
    Field, Opaque, RowCodec,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    mem,
    io,
};

mod inputs;
mod tally;
#[cfg(test)]
mod tests;

use inputs::{Witness, invalid};
use tally::{Payment, midnight};

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&inputs::COUNTRY_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&inputs::LINK_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryPowerChanges, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryPowerDaily, columns: Columns::Every, watch: Watch::Dropped, required: false },
    Read { table: TableId::CountryPowerDailyCosts, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::PoliticsCaptureCoverage, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryCommandPower, columns: Columns::These(&inputs::ALLOCATED), watch: Watch::Changed, required: false },
];

const TABLES: &[Output] = &[Output::State(TableId::PowerSpendTotals), Output::State(TableId::PowerItems), Output::State(TableId::PowerDayParts), Output::State(TableId::PowerHolds), Output::State(TableId::PowerSpendCoverage)];

const NONE: u32 = TagHolderRow::NONE;
const POLITICAL: u8 = PowerCurrency::Political as u8;

fn currency_bit(currency: PowerCurrency) -> u8 {
    match currency {
        PowerCurrency::Political => Streams::PP,
        PowerCurrency::Command => Streams::CP,
    }
}

fn total_key(holder: u32, currency: u8, ordinal: u32) -> u128 {
    u128::from(holder) << 40 | u128::from(currency) << 32 | u128::from(ordinal)
}

fn item_key(holder: u32, ordinal: u32) -> u128 {
    u128::from(holder) << 32 | u128::from(ordinal)
}

fn part_key(holder: u32, currency: u8, ordinal: u32) -> u128 {
    total_key(holder, currency, ordinal)
}

fn hold_key(holder: u32, hold: u64) -> u128 {
    u128::from(holder) << 64 | u128::from(hold)
}

struct Hash(u64);

impl Hash {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
        self
    }

    fn word(&mut self, value: u64) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    fn text(&mut self, bytes: &[u8]) -> &mut Self {
        self.word(bytes.len() as u64).bytes(bytes)
    }

    fn option(&mut self, value: Option<u64>) -> &mut Self {
        match value {
            Some(value) => self.word(1).word(value),
            None => self.word(0),
        }
    }

    fn finish(&self) -> u64 {
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }
}

const ITEM_SEED: u64 = 0xCBF2_9CE4_8422_2325;
const HOLD_SEED: u64 = 0x8422_2325_CBF2_9CE4;
const CHECK_SEED: u64 = 0x6C62_272E_07BB_0142;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemId {
    kind: u8,
    currency: PowerCurrency,
    cause: PowerCause,
    component: Option<u32>,
    charged: bool,
    identity: PowerIdentity,
}

impl ItemId {
    fn change(row: &CountryPowerChangeRow) -> Self {
        if row.currency == PowerCurrency::Command && row.cause == PowerCause::AbilityUpkeep {
            return Self { kind: Item::ABILITY_UPKEEP, currency: row.currency, cause: row.cause, component: None, charged: false, identity: PowerIdentity::default() };
        }
        Self { kind: Item::CHANGE, currency: row.currency, cause: row.cause, component: None, charged: row.requested_delta_raw.is_some_and(|raw| raw < 0), identity: row.identity.clone() }
    }

    fn cost(row: &CountryPowerDailyCostRow) -> Self {
        Self { kind: Item::COST, currency: PowerCurrency::Political, cause: row.cause, component: Some(row.component as u32), charged: row.cost_raw > 0, identity: row.identity.clone() }
    }

    fn of(row: &Item) -> Self {
        Self {
            kind: row.kind,
            currency: row.currency,
            cause: row.cause,
            component: row.component,
            charged: row.charged,
            identity: PowerIdentity {
                action: row.action,
                item_token: row.item_token,
                item_key: row.item_key.clone(),
                slot_key: row.slot_key.clone(),
                character: row.character,
                object_kind: row.object_kind,
                object_reference: row.object_reference,
                target_country: row.target_country,
                target_state: row.target_state,
                target_region: row.target_region,
                origin_kind: row.origin_kind,
                origin_token: row.origin_token,
                origin_key: row.origin_key.clone(),
            },
        }
    }

    fn hash(&self) -> u64 {
        let identity = &self.identity;
        let mut hash = Hash::new(ITEM_SEED);
        hash.word(u64::from(self.kind))
            .word(self.currency as u64)
            .word(self.cause as u64)
            .option(self.component.map(u64::from))
            .word(u64::from(self.charged))
            .word(identity.action as u64)
            .option(identity.item_token.map(u64::from))
            .text(&identity.item_key)
            .text(&identity.slot_key)
            .option(identity.character.map(u64::from))
            .word(identity.object_kind as u64)
            .option(identity.object_reference)
            .option(identity.target_country.map(u64::from))
            .option(identity.target_state.map(u64::from))
            .option(identity.target_region.map(u64::from))
            .word(identity.origin_kind as u64)
            .option(identity.origin_token.map(u64::from))
            .text(&identity.origin_key);
        hash.finish()
    }

    fn row(&self, holder: u32, ordinal: u32, first: &First) -> Item {
        let identity = &self.identity;
        Item {
            holder,
            ordinal,
            kind: self.kind,
            currency: self.currency,
            cause: self.cause,
            component: self.component,
            charged: self.charged,
            site: first.site,
            action: identity.action,
            item_token: identity.item_token,
            item_key: identity.item_key.clone(),
            slot_key: identity.slot_key.clone(),
            character: identity.character,
            object_kind: identity.object_kind,
            object_reference: identity.object_reference,
            target_country: identity.target_country,
            target_state: identity.target_state,
            target_region: identity.target_region,
            origin_kind: identity.origin_kind,
            origin_token: identity.origin_token,
            origin_key: identity.origin_key.clone(),
            first_hour: first.hour,
            first_read: first.read,
            first_country: first.country,
            first_epoch: first.epoch,
            first_sequence: first.sequence,
        }
    }
}

struct First {
    site: PowerSite,
    hour: i32,
    read: i32,
    country: u32,
    epoch: u64,
    sequence: u64,
}

fn hold_of(identity: &PowerIdentity) -> Option<(u64, u64)> {
    let of = |seed: u64| -> Option<u64> {
        let mut hash = Hash::new(seed);
        if identity.object_kind != PowerObjectKind::None {
            hash.word(0)
                .word(identity.object_kind as u64)
                .option(Some(identity.object_reference?))
                .text(&identity.item_key)
                .option(identity.item_token.map(u64::from))
                .text(&identity.slot_key)
                .option(identity.target_country.map(u64::from))
                .option(identity.target_state.map(u64::from))
                .option(identity.target_region.map(u64::from));
        } else if let Some(character) = identity.character {
            hash.word(1).word(u64::from(character)).text(&identity.item_key).option(identity.item_token.map(u64::from)).text(&identity.slot_key);
        } else if !identity.item_key.is_empty() || identity.item_token.is_some() {
            hash.word(2)
                .text(&identity.item_key)
                .option(identity.item_token.map(u64::from))
                .option(identity.target_country.map(u64::from))
                .option(identity.target_state.map(u64::from))
                .option(identity.target_region.map(u64::from));
        } else {
            hash.word(3).word(u64::from(identity.target_region?));
        }
        Some(hash.finish())
    };
    Some((of(HOLD_SEED)?, of(CHECK_SEED)?))
}

fn is_hold(row: &CountryPowerChangeRow) -> bool {
    row.currency == PowerCurrency::Command && (matches!(row.site, PowerSite::AllocateCommandPower | PowerSite::ReleaseCommandPower | PowerSite::GroundCrew) || row.cause == PowerCause::AllocationRefund)
}

fn charge(row: &CountryPowerChangeRow) -> Option<Option<i128>> {
    if is_hold(row) || (row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political) || !matches!(row.operation, PowerOperation::Add | PowerOperation::Subtract) {
        return None;
    }
    match row.requested_delta_raw.map(i128::from) {
        Some(raw) if raw < 0 => Some(Some(-raw)),
        Some(_) => None,
        None if row.operation == PowerOperation::Subtract => Some(None),
        None => None,
    }
}

fn skipped(cause: PowerCause) -> bool {
    matches!(cause, PowerCause::DailyGain | PowerCause::CapCorrection | PowerCause::ResetSetup | PowerCause::Console)
}

fn recurring(cause: PowerCause) -> bool {
    matches!(cause, PowerCause::DailyGain | PowerCause::AbilityUpkeep)
}

type Holds = BTreeMap<u64, Hold>;

#[derive(Clone, Debug, Default)]
struct RawHour {
    daily_gains: Vec<i32>,
    unknown_adds: Vec<(u64, i32, u8)>,
    unmatched: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct CostHour {
    unmatched: bool,
    negative: bool,
}

struct HoldRow {
    row: CountryPowerChangeRow,
    covered: bool,
}

#[derive(Default)]
struct Pending {
    totals: BTreeMap<u128, Total>,
    items: Vec<(Item, ItemId)>,
    fresh: HashMap<(u32, u64), Vec<usize>>,
    parts: BTreeMap<u128, Part>,
    withdrawn: Vec<u128>,
    holds: Vec<HoldRow>,
    costs: HashMap<u32, CostHour>,
    raws: HashMap<u32, RawHour>,
    daily: BTreeMap<u32, u32>,
    recovered: HashSet<(u32, u64)>,
    backlog: bool,
}

struct Batch {
    holder: u32,
    day: i32,
    sole: bool,
    moved: bool,
    raw: u32,
    rows: Vec<CountryPowerDailyCostRow>,
    held: Vec<Part>,
}

struct Move {
    raw: u32,
    from: u32,
    sole: bool,
}

#[derive(Default)]
pub struct PowerLedger {
    rebuild: bool,
    recover: bool,
    last: Option<i32>,
    run_first: Option<i32>,
    links: HashMap<u32, u32>,
    aliases: HashMap<u32, Vec<u32>>,
    live: HashSet<u32>,
    items: HashMap<(u32, u64), Vec<u32>>,
    next: HashMap<u32, u32>,
    parts: HashMap<u32, (i32, Vec<u128>)>,
    holds: HashMap<u32, Holds>,
    steady: HashSet<u32>,
    idle: bool,
}

impl PowerLedger {
    fn slot(&self, raw: u32) -> u32 {
        if raw == 0 {
            return NONE;
        }
        self.links.get(&raw).copied().unwrap_or(NONE)
    }

    fn sole(&self, holder: u32, raw: u32) -> bool {
        holder != NONE && self.aliases.get(&holder).is_some_and(|raws| raws.as_slice() == [raw])
    }

    fn read_identities(&mut self, hour: &Hour<'_>) -> (Vec<Move>, HashMap<u32, Vec<u32>>) {
        let links = mem::take(&mut self.links);
        let aliases = mem::take(&mut self.aliases);
        for (raw, slot) in inputs::links(hour) {
            self.links.insert(raw, slot);
        }
        self.aliases = aliases_of(&self.links);
        self.live = inputs::countries(hour).into_iter().map(|(_, slot)| slot).collect();
        let mut moves: Vec<Move> = links
            .into_iter()
            .filter(|&(raw, from)| from != NONE && self.slot(raw) != from)
            .map(|(raw, from)| Move { raw, from, sole: aliases.get(&from).is_some_and(|raws| raws.as_slice() == [raw]) })
            .collect();
        moves.sort_unstable_by_key(|moved| moved.raw);
        (moves, aliases)
    }

    fn read_tables(&mut self, own: &Rows<'_>) -> io::Result<()> {
        self.items.clear();
        self.next.clear();
        for row in inputs::every::<Item>(&own.table(TableId::PowerItems))? {
            self.items.entry((row.holder, ItemId::of(&row).hash())).or_default().push(row.ordinal);
            let next = self.next.entry(row.holder).or_default();
            *next = (*next).max(row.ordinal.saturating_add(1));
        }
        self.parts.clear();
        for row in inputs::every::<Part>(&own.table(TableId::PowerDayParts))? {
            let open = self.parts.entry(row.holder).or_insert((row.day, Vec::new()));
            open.0 = open.0.min(row.day);
            open.1.push(row.key());
        }
        self.holds.clear();
        for row in inputs::every::<Hold>(&own.table(TableId::PowerHolds))? {
            self.holds.entry(row.holder).or_default().insert(row.hold, row);
        }
        self.steady = inputs::every::<Coverage>(&own.table(TableId::PowerSpendCoverage))?.into_iter().filter(|row| row.daily_mismatch & Coverage::STEADY_MISMATCH != 0).map(|row| row.holder).collect();
        Ok(())
    }

    fn witness(&self, witnesses: Option<&replay_columnar::store::TableView<'_>>, holder: u32) -> Option<Witness> {
        let witnesses = witnesses?;
        self.aliases.get(&holder)?.iter().rev().find_map(|&raw| inputs::witness(witnesses, raw))
    }

    fn total<'p>(own: &Rows<'_>, pending: &'p mut Pending, key: u128, make: impl FnOnce() -> Total) -> io::Result<&'p mut Total> {
        if !pending.totals.contains_key(&key) {
            let held = inputs::keyed::<Total>(&own.table(TableId::PowerSpendTotals), key)?.unwrap_or_else(make);
            pending.totals.insert(key, held);
        }
        Ok(pending.totals.get_mut(&key).expect("just held"))
    }

    fn item_total<'p>(own: &Rows<'_>, pending: &'p mut Pending, holder: u32, currency: u8, ordinal: u32) -> io::Result<&'p mut Total> {
        Self::total(own, pending, total_key(holder, currency, ordinal), || tally::item(holder, currency, ordinal))
    }

    fn gains_total<'p>(own: &Rows<'_>, pending: &'p mut Pending, holder: u32, currency: u8) -> io::Result<&'p mut Total> {
        Self::total(own, pending, total_key(holder, currency, Total::GAINS), || tally::gains(holder, currency))
    }

    fn held_part(own: &Rows<'_>, pending: &Pending, key: u128, day: i32) -> io::Result<Option<Part>> {
        if let Some(part) = pending.parts.get(&key) {
            return Ok(Some(part.clone()));
        }
        Ok(inputs::keyed::<Part>(&own.table(TableId::PowerDayParts), key)?.filter(|part| part.day == day))
    }

    fn open_part(&mut self, pending: &mut Pending, part: Part) {
        let key = part.key();
        let open = self.parts.entry(part.holder).or_insert((part.day, Vec::new()));
        if !open.1.contains(&key) {
            open.1.push(key);
        }
        pending.parts.insert(key, part);
    }

    fn item_row(own: &Rows<'_>, pending: &Pending, holder: u32, ordinal: u32) -> io::Result<Item> {
        if let Some((item, _)) = pending.items.iter().find(|(item, _)| item.holder == holder && item.ordinal == ordinal) {
            return Ok(item.clone());
        }
        inputs::keyed::<Item>(&own.table(TableId::PowerItems), item_key(holder, ordinal))?.ok_or_else(|| invalid(format!("PowerItems holds no item {ordinal} of holder {holder} a day part names")))
    }

    fn known_item(&self, own: &Rows<'_>, pending: &Pending, holder: u32, id: &ItemId) -> io::Result<Option<u32>> {
        let hash = id.hash();
        if let Some(fresh) = pending.fresh.get(&(holder, hash))
            && let Some(&at) = fresh.iter().find(|&&at| pending.items[at].1 == *id)
        {
            return Ok(Some(pending.items[at].0.ordinal));
        }
        for &ordinal in self.items.get(&(holder, hash)).map_or(&[][..], Vec::as_slice) {
            if let Some(row) = inputs::keyed::<Item>(&own.table(TableId::PowerItems), item_key(holder, ordinal))?
                && ItemId::of(&row) == *id
            {
                return Ok(Some(ordinal));
            }
        }
        Ok(None)
    }

    fn item(&mut self, own: &Rows<'_>, pending: &mut Pending, holder: u32, id: ItemId, first: First) -> io::Result<u32> {
        let hash = id.hash();
        if let Some(fresh) = pending.fresh.get(&(holder, hash))
            && let Some(&at) = fresh.iter().find(|&&at| pending.items[at].1 == id)
        {
            return Ok(pending.items[at].0.ordinal);
        }
        let numbered_now = |ordinal: u32| pending.fresh.get(&(holder, hash)).is_some_and(|fresh| fresh.iter().any(|&at| pending.items[at].0.ordinal == ordinal));
        for &ordinal in self.items.get(&(holder, hash)).map_or(&[][..], Vec::as_slice) {
            let Some(row) = inputs::keyed::<Item>(&own.table(TableId::PowerItems), item_key(holder, ordinal))? else {
                if numbered_now(ordinal) {
                    continue;
                }
                return Err(invalid(format!("PowerItems holds no item {ordinal} of holder {holder} the ledger numbered")));
            };
            if ItemId::of(&row) == id {
                return Ok(ordinal);
            }
        }
        let next = self.next.entry(holder).or_default();
        let ordinal = *next;
        if ordinal == Total::GAINS {
            return Err(invalid(format!("Holder {holder}'s power items pass the ordinals")));
        }
        *next += 1;
        self.items.entry((holder, hash)).or_default().push(ordinal);
        pending.fresh.entry((holder, hash)).or_default().push(pending.items.len());
        pending.items.push((id.row(holder, ordinal, &first), id));
        Ok(ordinal)
    }

    fn settle(own: &Rows<'_>, pending: &mut Pending, part: &Part) -> io::Result<()> {
        if part.ordinal == Total::GAINS {
            let total = Self::gains_total(own, pending, part.holder, POLITICAL)?;
            for _ in 0..part.payments {
                tally::unmatched(total);
            }
            return Ok(());
        }
        let Some(sequence) = part.sequence else { return Ok(()) };
        let item = Self::item_row(own, pending, part.holder, part.ordinal)?;
        let total = Self::item_total(own, pending, part.holder, part.currency, part.ordinal)?;
        if !item.charged {
            for _ in 0..part.payments {
                match part.spent {
                    Recorded::Value(_) => tally::count(total, true),
                    Recorded::NotRecorded => tally::unknown(total),
                }
            }
            return Ok(());
        }
        let spent = match part.spent {
            Recorded::Value(spent) => Some(i128::from(spent)),
            Recorded::NotRecorded => None,
        };
        for at in 0..part.payments {
            let payment = Payment {
                recurring: recurring(item.cause),
                upkeep: item.cause == PowerCause::AbilityUpkeep,
                hour: part.day,
                read: part.read,
                sequence,
                ordinal: part.cost_ordinal.unwrap_or(0),
                charge: if at == 0 { spent } else { spent.map(|_| 0) },
                covered: part.covered,
                realized: None,
            };
            tally::pay(total, &payment);
        }
        Ok(())
    }

    fn roll(&mut self, hour: &Hour<'_>, pending: &mut Pending, out: &mut Rows<'_>) -> io::Result<()> {
        let day = midnight(hour.hour);
        let ended: Vec<u32> = self.parts.iter().filter(|(_, (opened, _))| *opened != day).map(|(&holder, _)| holder).collect();
        let withdrawn: HashSet<u128> = pending.withdrawn.iter().copied().collect();
        for holder in ended {
            let (_, keys) = self.parts.remove(&holder).expect("held");
            for key in keys.into_iter().filter(|key| !withdrawn.contains(key)) {
                let part = match pending.parts.remove(&key) {
                    Some(part) => Some(part),
                    None => inputs::keyed::<Part>(&out.table(TableId::PowerDayParts), key)?,
                };
                match part {
                    Some(part) if part.day != day => {
                        Self::settle(out, pending, &part)?;
                        out.delete(TableId::PowerDayParts, key);
                    }
                    Some(part) => {
                        pending.parts.insert(key, part);
                    }
                    None => {}
                }
            }
        }
        Ok(())
    }

    fn withhold_ended(&self, own: &Rows<'_>, pending: &mut Pending, today: i32) -> io::Result<()> {
        for (_, (day, keys)) in self.parts.iter().filter(|(holder, (day, _))| **holder != NONE && *day != today) {
            for &key in keys {
                if let Some(mut part) = Self::held_part(own, pending, key, *day)?
                    && part.cost_ordinal.is_some()
                {
                    part.covered = false;
                    part.spent = Recorded::NotRecorded;
                    pending.parts.insert(key, part);
                }
            }
        }
        Ok(())
    }

    fn reconcile(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, before: &HashMap<u32, Vec<u32>>, moved: &HashSet<u32>) -> io::Result<()> {
        let today = midnight(hour.hour);
        let mut holders: Vec<(u32, i32)> = self.parts.iter().filter(|(holder, _)| **holder != NONE).map(|(&holder, (day, _))| (holder, *day)).collect();
        holders.sort_unstable();
        for (holder, day) in holders {
            let keys = self.parts.get(&holder).map_or_else(Vec::new, |(_, keys)| keys.clone());
            let mut batches: BTreeMap<u64, Vec<Part>> = BTreeMap::new();
            for key in keys {
                if let Some(part) = Self::held_part(own, pending, key, day)?
                    && part.cost_ordinal.is_some()
                    && let Some(sequence) = part.sequence
                {
                    batches.entry(sequence).or_default().push(part);
                }
            }
            if batches.is_empty() {
                continue;
            }
            let raws = before.get(&holder).map_or(&[][..], Vec::as_slice);
            let sole = raws.len() == 1;
            let mut found: BTreeMap<u64, (u32, Vec<CountryPowerDailyCostRow>)> = BTreeMap::new();
            let mut ambiguous = HashSet::new();
            for &raw in raws {
                for cost in inputs::costs_of(hour, raw)?.into_iter().filter(|cost| midnight(cost.day_hour) == day) {
                    if batches.contains_key(&cost.pp_sequence) {
                        pending.recovered.insert((raw, cost.pp_sequence));
                    }
                    let entry = found.entry(cost.pp_sequence).or_insert_with(|| (raw, Vec::new()));
                    if entry.0 == raw {
                        entry.1.push(cost);
                    } else {
                        ambiguous.insert(cost.pp_sequence);
                    }
                }
            }
            for (sequence, held) in batches {
                match found.remove(&sequence) {
                    Some((raw, rows)) if !ambiguous.contains(&sequence) => {
                        self.reconcile_batch(hour, own, witnesses, pending, Batch { holder, day, sole, moved: moved.contains(&raw), raw, rows, held })?;
                    }
                    None if sole && self.emptied(hour, witnesses, raws[0], day, sequence)? => {
                        let gone: HashSet<u128> = held.iter().map(Part::key).collect();
                        for &key in &gone {
                            pending.parts.remove(&key);
                            pending.withdrawn.push(key);
                        }
                        if let Some((_, open)) = self.parts.get_mut(&holder) {
                            open.retain(|key| !gone.contains(key));
                        }
                    }
                    None if day == today => {}
                    _ => {
                        for mut part in held {
                            part.covered = false;
                            part.spent = Recorded::NotRecorded;
                            pending.parts.insert(part.key(), part);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn emptied(&self, hour: &Hour<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, raw: u32, day: i32, sequence: u64) -> io::Result<bool> {
        let at = hour.hour;
        Ok(inputs::fresh_daily(hour, raw)?.is_some_and(|daily| {
            daily.day_hour == day
                && daily.pp_sequence == Some(sequence)
                && daily.pp_cost_raw == Some(0)
                && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(daily.epoch, at, Streams::PP_DAILY_COSTS, at))
        }))
    }

    fn reconcile_batch(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, batch: Batch) -> io::Result<()> {
        let Batch { holder, day, sole, moved, raw, rows, held } = batch;
        let at = hour.hour;
        let Some(sequence) = rows.first().map(|cost| cost.pp_sequence) else { return Ok(()) };
        let epoch = rows[0].epoch;
        let daily = inputs::fresh_daily(hour, raw)?;
        let matched = rows.iter().all(|cost| cost.epoch == epoch) && daily.as_ref().is_some_and(|daily| daily.epoch == epoch && daily.day_hour == day && daily.pp_sequence == Some(sequence));
        let total: i128 = rows.iter().map(|cost| i128::from(cost.cost_raw)).sum();
        let reconciles = matched && daily.as_ref().is_some_and(|daily| daily.pp_cost_raw.is_some_and(|cost| i128::from(cost) == total));
        let unmatched = held.iter().any(|part| part.ordinal == Total::GAINS);
        let mut remaining: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
        for cost in rows.iter().filter(|cost| cost.cost_raw != 0) {
            let ordinal = if unmatched {
                Total::GAINS
            } else {
                let first = First { site: cost.site, hour: day, read: at, country: raw, epoch, sequence: u64::from(cost.ordinal) };
                self.item(own, pending, holder, ItemId::cost(cost), first)?
            };
            let entry = remaining.entry(ordinal).or_insert((0, 0, cost.ordinal));
            entry.0 += i128::from(cost.cost_raw);
            entry.1 += 1;
            entry.2 = entry.2.min(cost.ordinal);
        }
        let exact = |part: &Part| remaining.get(&part.ordinal).is_some_and(|&(sum, payments, _)| part.payments == payments && matches!(part.spent, Recorded::Value(spent) if i128::from(spent) == sum));
        let opened: Vec<u32> = remaining.keys().copied().filter(|ordinal| held.iter().all(|part| part.ordinal != *ordinal)).collect();
        let changed = !held.iter().all(exact) || !opened.is_empty();
        if matched || changed {
            if !reconciles {
                self.steady.insert(holder);
            } else if sole {
                self.steady.remove(&holder);
            }
        }
        if !changed {
            return Ok(());
        }
        if moved {
            for mut part in held {
                part.covered = false;
                part.spent = Recorded::NotRecorded;
                pending.parts.insert(part.key(), part);
            }
            return Ok(());
        }
        let seen = pending.costs.entry(holder).or_default();
        for cost in rows.iter().filter(|cost| cost.cost_raw != 0) {
            seen.unmatched |= !matched;
            seen.negative |= matched && cost.cost_raw < 0;
        }
        let covered = reconciles && !self.steady.contains(&holder) && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(epoch, day, Streams::PP | Streams::PP_DAILY_COSTS, at));
        let mut withdrawn = HashSet::new();
        for mut part in held {
            let key = part.key();
            match remaining.get(&part.ordinal) {
                _ if exact(&part) => {
                    if part.covered && !reconciles {
                        part.covered = false;
                        pending.parts.insert(key, part);
                    }
                }
                Some(&(sum, payments, first)) if sole => {
                    part.spent = i64::try_from(sum).map_or(Recorded::NotRecorded, Recorded::Value);
                    part.payments = payments;
                    part.cost_ordinal = Some(first);
                    part.covered = covered;
                    pending.parts.insert(key, part);
                }
                None if sole => {
                    withdrawn.insert(key);
                    pending.parts.remove(&key);
                    pending.withdrawn.push(key);
                }
                _ => {
                    part.covered = false;
                    part.spent = Recorded::NotRecorded;
                    pending.parts.insert(key, part);
                }
            }
        }
        if !withdrawn.is_empty()
            && let Some((_, open)) = self.parts.get_mut(&holder)
        {
            open.retain(|key| !withdrawn.contains(key));
        }
        for ordinal in opened {
            let key = part_key(holder, POLITICAL, ordinal);
            if let Some(mut other) = Self::held_part(own, pending, key, day)? {
                other.covered = false;
                other.spent = Recorded::NotRecorded;
                pending.parts.insert(key, other);
                continue;
            }
            let (sum, payments, first) = remaining[&ordinal];
            let part = Part {
                holder,
                currency: POLITICAL,
                ordinal,
                day,
                spent: i64::try_from(sum).map_or(Recorded::NotRecorded, Recorded::Value),
                payments,
                covered,
                sequence: Some(sequence),
                cost_ordinal: Some(first),
                read: at,
            };
            self.open_part(pending, part);
        }
        Ok(())
    }

    fn changes(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending) -> io::Result<()> {
        let at = hour.hour;
        let complete = inputs::changes_complete(hour);
        let changes = inputs::changes(hour)?;
        pending.backlog = changes.backlog;
        if changes.backlog {
            return Ok(());
        }
        for row in changes.rows {
            let bit = currency_bit(row.currency);
            let covered = complete && witnesses.and_then(|view| inputs::witness(view, row.country)).is_some_and(|witness| witness.covers(row.epoch, row.hour, bit, at));
            let raw = pending.raws.entry(row.country).or_default();
            if row.cause == PowerCause::DailyGain && row.currency == PowerCurrency::Political {
                raw.daily_gains.push(row.hour);
                raw.unmatched |= !inputs::daily(hour, row.country)?.is_some_and(|daily| daily.epoch == row.epoch && daily.day_hour == row.hour && daily.pp_sequence == Some(row.sequence));
            } else if row.operation == PowerOperation::Add && row.requested_delta_raw.is_none() {
                raw.unknown_adds.push((row.epoch, row.hour, bit));
            }
            if is_hold(&row) {
                pending.holds.push(HoldRow { row: row.clone(), covered });
            }
            let charge = charge(&row);
            if charge.is_none() && skipped(row.cause) {
                continue;
            }
            let holder = self.slot(row.country);
            let first = First { site: row.site, hour: row.hour, read: at, country: row.country, epoch: row.epoch, sequence: row.sequence };
            let ordinal = self.item(own, pending, holder, ItemId::change(&row), first)?;
            let total = Self::item_total(own, pending, holder, row.currency as u8, ordinal)?;
            let Some(charge) = charge else {
                tally::count(total, false);
                continue;
            };
            let payment = Payment {
                recurring: recurring(row.cause),
                upkeep: row.cause == PowerCause::AbilityUpkeep,
                hour: row.hour,
                read: at,
                sequence: row.sequence,
                ordinal: 0,
                charge,
                covered,
                realized: Some(row.realized_delta_raw()),
            };
            tally::pay(total, &payment);
            let day = midnight(at);
            if row.cause != PowerCause::DailyGain && midnight(row.hour) == day {
                let key = part_key(holder, row.currency as u8, ordinal);
                let mut part = Self::held_part(own, pending, key, day)?.unwrap_or(Part {
                    holder,
                    currency: row.currency as u8,
                    ordinal,
                    day,
                    spent: Recorded::Value(0),
                    payments: 0,
                    covered: true,
                    sequence: None,
                    cost_ordinal: None,
                    read: at,
                });
                part.spent = match (part.spent, charge) {
                    (Recorded::Value(spent), Some(charge)) => i64::try_from(i128::from(spent) + charge).map_or(Recorded::NotRecorded, Recorded::Value),
                    _ => Recorded::NotRecorded,
                };
                part.payments += 1;
                part.covered &= covered;
                self.open_part(pending, part);
            }
        }
        Ok(())
    }

    fn days(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, moves: &[Move]) -> io::Result<()> {
        let run_first = self.run_first.unwrap_or(hour.hour);
        for day in inputs::closed_days(hour)? {
            if day.day_hour > run_first {
                let holder = self.slot(day.country);
                for (currency, gain) in tally::day_gains(&day).iter().enumerate() {
                    tally::gain(Self::gains_total(own, pending, holder, currency as u8)?, *gain, 1);
                }
            }
        }
        for raw in inputs::moved_daily(hour) {
            pending.daily.insert(self.slot(raw), raw);
        }
        let today = midnight(hour.hour);
        let complete = inputs::costs_complete(hour);
        for moved in moves {
            self.leave(hour, own, pending, today, moved, complete)?;
        }
        let mut publications = inputs::publications(hour)?;
        if !pending.recovered.is_empty() {
            let recovered = mem::take(&mut pending.recovered);
            publications.rows.retain_mut(|publication| {
                let held = !publication.costs.is_empty();
                publication.costs.retain(|cost| !recovered.contains(&(publication.country, cost.pp_sequence)));
                !held || !publication.costs.is_empty()
            });
        }
        if !publications.backlog {
            let reread: BTreeSet<u32> = moves.iter().map(|moved| moved.raw).filter(|&raw| self.slot(raw) != NONE).collect();
            let published: HashSet<u32> = publications.rows.iter().filter(|publication| !(publication.deleted_only && reread.contains(&publication.country))).map(|publication| publication.country).collect();
            for publication in publications.rows.iter().filter(|publication| published.contains(&publication.country)) {
                self.publish(hour, own, witnesses, pending, run_first, publication)?;
            }
            for raw in reread.into_iter().filter(|raw| complete && !published.contains(raw)) {
                let costs: Vec<CountryPowerDailyCostRow> = inputs::costs_of(hour, raw)?.into_iter().filter(|cost| midnight(cost.day_hour) == today).collect();
                if !costs.is_empty() {
                    self.publish(hour, own, witnesses, pending, run_first, &inputs::Publication { country: raw, costs, deleted_only: false })?;
                }
            }
            let open: Vec<u32> = self
                .parts
                .iter()
                .filter(|(_, (day, keys))| *day == today && !keys.is_empty())
                .filter(|(holder, _)| **holder != NONE)
                .flat_map(|(holder, _)| self.aliases.get(holder).into_iter().flatten().copied())
                .filter(|raw| !published.contains(raw))
                .collect();
            for raw in open {
                if inputs::holds_no_costs(hour, raw)? {
                    self.publish(hour, own, witnesses, pending, run_first, &inputs::Publication { country: raw, costs: Vec::new(), deleted_only: true })?;
                }
            }
        }
        for (holder, raw) in std::mem::take(&mut pending.daily) {
            if let Some(steady) = self.mismatch(hour, own, pending, holder, raw)? {
                if steady {
                    self.steady.insert(holder);
                } else {
                    self.steady.remove(&holder);
                }
            }
        }
        Ok(())
    }

    fn mismatch(&self, hour: &Hour<'_>, own: &Rows<'_>, pending: &Pending, holder: u32, raw: u32) -> io::Result<Option<bool>> {
        let Some(daily) = inputs::daily(hour, raw)? else { return Ok(Some(false)) };
        let Some(sequence) = daily.pp_sequence else { return Ok(Some(false)) };
        let mut sum = 0i128;
        let mut found = false;
        for &key in self.parts.get(&holder).map_or(&[][..], |(_, keys)| keys.as_slice()) {
            let Some(part) = Self::held_part(own, pending, key, daily.day_hour)? else { continue };
            if part.ordinal == Total::GAINS || part.sequence != Some(sequence) {
                continue;
            }
            found = true;
            sum += match part.spent {
                Recorded::Value(spent) => i128::from(spent),
                Recorded::NotRecorded => return Ok(Some(true)),
            };
        }
        Ok(found.then(|| daily.pp_cost_raw.is_none_or(|cost| i128::from(cost) != sum)))
    }

    fn publish(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, run_first: i32, publication: &inputs::Publication) -> io::Result<()> {
        if publication.deleted_only {
            return self.delete(hour, own, witnesses, pending, publication);
        }
        let at = hour.hour;
        let raw = publication.country;
        let holder = self.slot(raw);
        let daily = inputs::daily(hour, raw)?;
        let today = midnight(at);
        let sole = self.sole(holder, raw);
        let replaced: HashSet<u64> = publication.costs.iter().filter(|cost| midnight(cost.day_hour) == today).map(|cost| cost.pp_sequence).collect();
        let mut produced = HashSet::new();
        let mut applications: BTreeMap<(u64, i32, u64), Vec<&CountryPowerDailyCostRow>> = BTreeMap::new();
        for cost in &publication.costs {
            applications.entry((cost.epoch, cost.day_hour, cost.pp_sequence)).or_default().push(cost);
        }
        for ((epoch, day_hour, sequence), costs) in applications {
            let matched = daily.as_ref().is_some_and(|daily| daily.epoch == epoch && daily.day_hour == day_hour && daily.pp_sequence == Some(sequence));
            let seen = pending.costs.entry(holder).or_default();
            for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                seen.unmatched |= !matched;
                seen.negative |= matched && cost.cost_raw < 0;
            }
            if matched {
                let sum: i128 = costs.iter().map(|cost| i128::from(cost.cost_raw)).sum();
                let steady = daily.as_ref().is_some_and(|daily| daily.pp_cost_raw.is_none_or(|cost| i128::from(cost) != sum));
                if steady {
                    self.steady.insert(holder);
                } else {
                    self.steady.remove(&holder);
                }
                pending.daily.remove(&holder);
            }
            if day_hour <= run_first {
                continue;
            }
            let covered = !self.steady.contains(&holder) && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(epoch, day_hour, Streams::PP | Streams::PP_DAILY_COSTS, at));
            let mut items: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
            for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                let ordinal = if matched {
                    let first = First { site: cost.site, hour: day_hour, read: at, country: raw, epoch, sequence: u64::from(cost.ordinal) };
                    self.item(own, pending, holder, ItemId::cost(cost), first)?
                } else {
                    Total::GAINS
                };
                let entry = items.entry(ordinal).or_insert((0, 0, cost.ordinal));
                entry.0 += i128::from(cost.cost_raw);
                entry.1 += 1;
                entry.2 = entry.2.min(cost.ordinal);
            }
            let day = midnight(day_hour);
            let opens = day == today;
            for (ordinal, (sum, payments, first)) in items {
                let key = part_key(holder, POLITICAL, ordinal);
                let held = if opens { Self::held_part(own, pending, key, day)? } else { None };
                let part = Part {
                    holder,
                    currency: POLITICAL,
                    ordinal,
                    day,
                    spent: i64::try_from(sum).map_or(Recorded::NotRecorded, Recorded::Value),
                    payments,
                    covered: covered && held.as_ref().is_none_or(|held| held.covered),
                    sequence: Some(sequence),
                    cost_ordinal: Some(first),
                    read: held.as_ref().map_or(at, |held| held.read),
                };
                if opens {
                    produced.insert(key);
                    self.open_part(pending, part);
                } else {
                    Self::settle(own, pending, &part)?;
                }
            }
        }
        if !replaced.is_empty() && holder != NONE {
            self.withdraw(own, pending, holder, today, &replaced, &produced, sole)?;
        }
        Ok(())
    }

    fn delete(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, publication: &inputs::Publication) -> io::Result<()> {
        let at = hour.hour;
        let raw = publication.country;
        let holder = self.slot(raw);
        if holder == NONE {
            return Ok(());
        }
        let today = midnight(at);
        let sole = self.sole(holder, raw);
        let daily = inputs::fresh_daily(hour, raw)?.filter(|daily| midnight(daily.day_hour) == today);
        let mut survivors: BTreeMap<u64, Vec<&CountryPowerDailyCostRow>> = BTreeMap::new();
        for cost in publication.costs.iter().filter(|cost| midnight(cost.day_hour) == today) {
            survivors.entry(cost.pp_sequence).or_default().push(cost);
        }
        if publication.costs.is_empty()
            && let Some(daily) = &daily
            && daily.pp_cost_raw == Some(0)
            && let Some(sequence) = daily.pp_sequence
            && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(daily.epoch, at, Streams::PP_DAILY_COSTS, at))
        {
            survivors.insert(sequence, Vec::new());
        }
        let open = self.parts.get(&holder).map_or_else(Vec::new, |(_, keys)| keys.clone());
        let mut withdrawn = HashSet::new();
        for (sequence, costs) in survivors {
            let mut held = Vec::new();
            for &key in &open {
                if let Some(part) = Self::held_part(own, pending, key, today)?
                    && part.cost_ordinal.is_some()
                    && part.sequence == Some(sequence)
                {
                    held.push(part);
                }
            }
            let unmatched = held.iter().any(|part| part.ordinal == Total::GAINS);
            let mut remaining: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
            for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                let ordinal = if unmatched { Some(Total::GAINS) } else { self.known_item(own, pending, holder, &ItemId::cost(cost))? };
                if let Some(ordinal) = ordinal {
                    let entry = remaining.entry(ordinal).or_insert((0, 0, cost.ordinal));
                    entry.0 += i128::from(cost.cost_raw);
                    entry.1 += 1;
                    entry.2 = entry.2.min(cost.ordinal);
                }
            }
            let total: i128 = costs.iter().map(|cost| i128::from(cost.cost_raw)).sum();
            let applied = daily.as_ref().filter(|daily| daily.pp_sequence == Some(sequence) && costs.iter().all(|cost| cost.epoch == daily.epoch));
            let reconciles = applied.is_some_and(|daily| daily.pp_cost_raw.is_some_and(|cost| i128::from(cost) == total));
            let exact = |part: &Part| remaining.get(&part.ordinal).is_some_and(|&(sum, payments, _)| part.payments == payments && matches!(part.spent, Recorded::Value(spent) if i128::from(spent) == sum));
            if held.iter().all(exact) {
                continue;
            }
            if !reconciles {
                self.steady.insert(holder);
            } else if sole {
                self.steady.remove(&holder);
            }
            if pending.daily.get(&holder) == Some(&raw) {
                pending.daily.remove(&holder);
            }
            for mut part in held {
                let key = part.key();
                match remaining.get(&part.ordinal) {
                    _ if exact(&part) => {
                        if part.covered && !reconciles {
                            part.covered = false;
                            pending.parts.insert(key, part);
                        }
                    }
                    Some(&(sum, payments, first)) if sole => {
                        part.spent = i64::try_from(sum).map_or(Recorded::NotRecorded, Recorded::Value);
                        part.payments = payments;
                        part.cost_ordinal = Some(first);
                        part.covered &= reconciles;
                        pending.parts.insert(key, part);
                    }
                    None if sole => {
                        withdrawn.insert(key);
                        pending.parts.remove(&key);
                        pending.withdrawn.push(key);
                    }
                    _ => {
                        part.covered = false;
                        part.spent = Recorded::NotRecorded;
                        pending.parts.insert(key, part);
                    }
                }
            }
        }
        if !withdrawn.is_empty()
            && let Some((_, open)) = self.parts.get_mut(&holder)
        {
            open.retain(|key| !withdrawn.contains(key));
        }
        Ok(())
    }

    fn withdraw(&mut self, own: &Rows<'_>, pending: &mut Pending, holder: u32, today: i32, replaced: &HashSet<u64>, produced: &HashSet<u128>, sole: bool) -> io::Result<()> {
        let Some((_, open)) = self.parts.get(&holder) else { return Ok(()) };
        let mut dropped = Vec::new();
        for &key in open.iter().filter(|key| !produced.contains(*key)) {
            if let Some(part) = Self::held_part(own, pending, key, today)?
                && part.cost_ordinal.is_some()
                && part.sequence.is_some_and(|sequence| replaced.contains(&sequence))
            {
                dropped.push(part);
            }
        }
        if sole {
            if let Some((_, open)) = self.parts.get_mut(&holder) {
                let gone: HashSet<u128> = dropped.iter().map(Part::key).collect();
                open.retain(|key| !gone.contains(key));
            }
            for part in dropped {
                pending.parts.remove(&part.key());
                pending.withdrawn.push(part.key());
            }
        } else {
            for mut part in dropped {
                part.covered = false;
                part.spent = Recorded::NotRecorded;
                pending.parts.insert(part.key(), part);
            }
        }
        Ok(())
    }

    fn leave(&mut self, hour: &Hour<'_>, own: &Rows<'_>, pending: &mut Pending, today: i32, moved: &Move, complete: bool) -> io::Result<()> {
        let Some((_, open)) = self.parts.get(&moved.from) else { return Ok(()) };
        let open = open.clone();
        let sequences = |raw: u32| -> io::Result<HashSet<u64>> { Ok(inputs::costs_of(hour, raw)?.into_iter().filter(|cost| midnight(cost.day_hour) == today).map(|cost| cost.pp_sequence).collect()) };
        let mut kept = HashSet::new();
        if complete && !moved.sole {
            for &raw in self.aliases.get(&moved.from).map_or(&[][..], Vec::as_slice) {
                kept.extend(sequences(raw)?);
            }
            for sequence in sequences(moved.raw)? {
                kept.remove(&sequence);
            }
        }
        let follow = complete && self.slot(moved.raw) != NONE;
        let mut gone = HashSet::new();
        for key in open {
            let Some(mut part) = Self::held_part(own, pending, key, today)? else { continue };
            if part.cost_ordinal.is_none() || part.sequence.is_some_and(|sequence| kept.contains(&sequence)) {
                continue;
            }
            if moved.sole && follow {
                gone.insert(key);
                pending.parts.remove(&key);
                pending.withdrawn.push(key);
            } else {
                part.covered = false;
                part.spent = Recorded::NotRecorded;
                pending.parts.insert(key, part);
            }
        }
        if let Some((_, open)) = self.parts.get_mut(&moved.from) {
            open.retain(|key| !gone.contains(key));
        }
        Ok(())
    }

    fn coverage(&self, hour: &Hour<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, adjacent: bool, pending: &Pending, out: &mut Rows) -> io::Result<bool> {
        let at = hour.hour;
        let changes_known = inputs::changes_complete(hour);
        let daily_known = inputs::daily_complete(hour);
        let input_reason = inputs::input_reason(hour).unwrap_or_default().into_bytes();
        for key in crate::forces::held(out, TableId::PowerSpendCoverage) {
            if !self.live.contains(&(key as u32)) {
                out.delete(TableId::PowerSpendCoverage, key);
            }
        }
        let mut current = false;
        for &holder in &self.live {
            let witness = self.witness(witnesses, holder);
            let (streams, gap) = match witness {
                None => (0, Some(Coverage::NO_CAPTURE)),
                Some(witness) if witness.through != at => (0, Some(Coverage::STALE)),
                Some(witness) => (witness.streams, None),
            };
            current |= gap.is_none();
            let raw = witness.map(|witness| witness.country).or_else(|| self.aliases.get(&holder).and_then(|raws| raws.last().copied()));
            let facts = raw.and_then(|raw| pending.raws.get(&raw));
            let window = witness.filter(|_| gap.is_none()).map_or(at..=at, |witness| witness.since..=at);
            let unmatched = facts.is_some_and(|facts| facts.unmatched);
            let steady = self.steady.contains(&holder);
            let unreconciled = steady || unmatched;
            let daily_mismatch = u8::from(steady) * Coverage::STEADY_MISMATCH | u8::from(unmatched) * Coverage::GAIN_UNMATCHED;
            let has_daily = facts.is_some_and(|facts| facts.daily_gains.iter().any(|hour| window.contains(hour)));
            let unknown = facts.map_or(0, |facts| {
                facts.unknown_adds.iter().filter(|(epoch, hour, _)| witness.is_some_and(|witness| witness.epoch == *epoch) && window.contains(hour)).fold(0, |bits, &(.., bit)| bits | bit)
            });
            let daily_costs = streams & Streams::PP_DAILY_COSTS != 0;
            let mut political = streams & Streams::PP != 0 && changes_known && daily_known && !unreconciled && (!has_daily || daily_costs) && unknown & Streams::PP == 0;
            let mut command = streams & Streams::CP != 0 && changes_known && unknown & Streams::CP == 0;
            let mut reason = if !changes_known {
                Coverage::CHANGES_UNREAD
            } else if !daily_known || unreconciled || (has_daily && !daily_costs) {
                Coverage::DAILY_COSTS_UNREAD
            } else if unknown == Streams::PP {
                Coverage::POLITICAL_OPERAND
            } else if unknown == Streams::CP {
                Coverage::COMMAND_OPERAND
            } else if unknown != 0 {
                Coverage::BOTH_OPERANDS
            } else if let Some(gap) = gap {
                gap
            } else if !political || !command {
                Coverage::STREAMS
            } else {
                Coverage::KNOWN
            };
            if let Some(costs) = pending.costs.get(&holder) {
                if costs.unmatched {
                    political = false;
                    reason = Coverage::COST_UNMATCHED;
                } else if costs.negative {
                    political = false;
                    reason = Coverage::NEGATIVE_COST;
                }
            }
            if pending.backlog {
                (political, command, reason) = (false, false, Coverage::BACKLOG);
            }
            let before = inputs::held_coverage(&out.table(TableId::PowerSpendCoverage), holder);
            let stretch = |now: bool, was: Option<(bool, Option<i32>)>| match (now, was) {
                (false, _) => None,
                (true, Some((true, Some(first)))) if adjacent => Some(first),
                (true, _) => Some(at),
            };
            let political_since = stretch(political, before.as_ref().map(|row| (row.political, row.political_since)));
            let command_since = stretch(command, before.as_ref().map(|row| (row.command, row.command_since)));
            let row = Coverage { holder, political, command, reason, political_since, command_since, daily_mismatch, input_reason: input_reason.clone() };
            let cells = cells(&row, out)?;
            out.set(TableId::PowerSpendCoverage, row.key(), cells);
        }
        Ok(current)
    }

    fn holds(&mut self, hour: &Hour<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, adjacent: bool, pending: &Pending, out: &mut Rows) -> io::Result<()> {
        let at = hour.hour;
        let complete = inputs::changes_complete(hour);
        let mut rows: HashMap<(u32, u64), Vec<&HoldRow>> = HashMap::new();
        let mut holders: Vec<u32> = self.holds.keys().copied().collect();
        for held in &pending.holds {
            rows.entry((held.row.country, held.row.epoch)).or_default().push(held);
            holders.push(self.slot(held.row.country));
        }
        holders.sort_unstable();
        holders.dedup();
        for holder in holders {
            let mut holds = self.holds.remove(&holder).unwrap_or_default();
            let before = holds.clone();
            if holder == NONE || !self.live.contains(&holder) {
                holds.clear();
            } else {
                let witness = self.witness(witnesses, holder);
                let identity = witness.map(|witness| (witness.country, witness.epoch));
                if holds.values().next().map(|hold| (hold.capture_country, hold.capture_epoch)) != identity {
                    holds.clear();
                }
                if !adjacent || !complete || !witness.is_some_and(|witness| witness.streams & Streams::CP != 0 && (witness.since..=witness.through).contains(&at)) {
                    holds.clear();
                }
                if let Some(identity) = identity.filter(|_| !pending.backlog)
                    && let Some(rows) = rows.get_mut(&identity)
                {
                    rows.sort_by_key(|held| held.row.sequence);
                    for held in rows.iter() {
                        if held.covered {
                            apply(&mut holds, holder, identity, &held.row)?;
                        } else {
                            holds.clear();
                        }
                    }
                }
                let total = inputs::allocated(hour, holder);
                let attributed: i128 = holds.values().map(|hold| i128::from(hold.amount)).sum();
                if total.is_none_or(|total| total == 0 || attributed > i128::from(total)) {
                    holds.clear();
                }
            }
            for &hold in before.keys().filter(|hold| !holds.contains_key(*hold)) {
                out.delete(TableId::PowerHolds, hold_key(holder, hold));
            }
            for row in holds.values().filter(|row| before.get(&row.hold) != Some(*row)) {
                let cells = cells(row, out)?;
                out.set(TableId::PowerHolds, row.key(), cells);
            }
            if !holds.is_empty() {
                self.holds.insert(holder, holds);
            }
        }
        Ok(())
    }
}

fn aliases_of(links: &HashMap<u32, u32>) -> HashMap<u32, Vec<u32>> {
    let mut aliases: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&raw, &slot) in links {
        aliases.entry(slot).or_default().push(raw);
    }
    aliases.values_mut().for_each(|raws| raws.sort_unstable());
    aliases
}

fn apply(active: &mut Holds, holder: u32, identity: (u32, u64), row: &CountryPowerChangeRow) -> io::Result<()> {
    if row.operation == PowerOperation::AllocationClamp {
        return Ok(());
    }
    let amount = row.requested_delta_raw;
    let key = hold_of(&row.identity);
    if row.cause == PowerCause::AllocationRefund {
        let Some(((hold, _), amount)) = key.zip(amount).filter(|(_, amount)| *amount > 0) else {
            active.clear();
            return Ok(());
        };
        match active.get_mut(&hold) {
            Some(held) if amount < held.amount => held.amount -= amount,
            Some(held) if amount == held.amount => {
                active.remove(&hold);
            }
            _ => active.clear(),
        }
        return Ok(());
    }
    if row.site == PowerSite::ReleaseCommandPower {
        active.clear();
        return Ok(());
    }
    if row.site != PowerSite::AllocateCommandPower {
        return Ok(());
    }
    let Some(((hold, check), amount)) = key.zip(amount).filter(|(_, amount)| *amount < 0) else { return Ok(()) };
    let amount = amount.checked_neg().ok_or_else(|| invalid("A command power allocation's amount has no negation"))?;
    match active.get_mut(&hold) {
        Some(held) if held.check != check => return Err(invalid("Two command power holds' operands share a hash")),
        Some(held) => held.amount = held.amount.checked_add(amount).ok_or_else(|| invalid("A command power hold passes i64"))?,
        None => {
            let operands = &row.identity;
            active.insert(
                hold,
                Hold {
                    holder,
                    hold,
                    capture_country: identity.0,
                    capture_epoch: identity.1,
                    amount,
                    check,
                    cause: row.cause,
                    object_kind: operands.object_kind,
                    object_reference: operands.object_reference,
                    ability: row.cause == PowerCause::AbilityActivation || operands.object_kind == PowerObjectKind::Ability,
                    item_token: operands.item_token,
                    item_key: operands.item_key.clone(),
                    slot_key: operands.slot_key.clone(),
                    character: operands.character,
                    target_country: operands.target_country,
                    target_state: operands.target_state,
                    target_region: operands.target_region,
                },
            );
        }
    }
    Ok(())
}

fn moved(hour: &Hour<'_>) -> bool {
    READS.iter().any(|read| {
        let changes = hour.changes(read.table);
        changes.status_changed || changes.appended > 0 || !changes.forgotten.is_empty() || hour.wrote(read.table, read.columns)
    })
}

fn partial(hour: &Hour<'_>, out: &mut Rows) {
    for table in [TableId::CountryPowerChanges, TableId::CountryPowerDailyCosts] {
        let status = hour.status(table);
        if status != Status::Complete {
            for derived in [TableId::PowerSpendTotals, TableId::PowerDayParts] {
                match status {
                    Status::NotRecorded => out.unavailable(derived, format!("{table:?} is not recorded").as_bytes()),
                    other => out.status(derived, other),
                }
            }
            return;
        }
    }
}

impl Writer for PowerLedger {
    fn name(&self) -> &'static str {
        "PowerLedger"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Output] {
        TABLES
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
        if hour.first || self.run_first.is_none() {
            self.run_first = Some(at);
        }
        let whole = hour.fresh || self.rebuild;
        let adjacent = !hour.fresh && self.last.and_then(|last| last.checked_add(1)) == Some(at);
        let rolls = self.parts.values().any(|(day, _)| *day != midnight(at));
        if !whole && self.idle && !rolls && !moved(hour) {
            self.last = Some(at);
            return Ok(());
        }
        if hour.first {
            self.recover = false;
        } else if hour.fresh {
            self.recover = true;
        }
        let (moves, before) = if whole || inputs::identities_moved(hour) { self.read_identities(hour) } else { (Vec::new(), HashMap::new()) };
        if whole {
            self.read_tables(out)?;
        }
        let witnesses = inputs::witnesses(hour);
        let mut pending = Pending::default();
        if self.recover && inputs::daily_complete(hour) {
            let before = if before.is_empty() { self.aliases.clone() } else { before };
            let moved: HashSet<u32> = moves.iter().map(|moved| moved.raw).collect();
            self.reconcile(hour, out, witnesses.as_ref(), &mut pending, &before, &moved)?;
            self.recover = false;
        } else if self.recover {
            self.withhold_ended(out, &mut pending, midnight(at))?;
        }
        self.roll(hour, &mut pending, out)?;
        self.changes(hour, out, witnesses.as_ref(), &mut pending)?;
        self.days(hour, out, witnesses.as_ref(), &mut pending, &moves)?;
        let current = self.coverage(hour, witnesses.as_ref(), adjacent, &pending, out)?;
        self.holds(hour, witnesses.as_ref(), adjacent, &pending, out)?;
        for (item, _) in &pending.items {
            let cells = cells(item, out)?;
            out.set(TableId::PowerItems, item.key(), cells);
        }
        for (key, total) in &pending.totals {
            let cells = cells(total, out)?;
            out.set(TableId::PowerSpendTotals, *key, cells);
        }
        for (key, part) in &pending.parts {
            let cells = cells(part, out)?;
            out.set(TableId::PowerDayParts, *key, cells);
        }
        for &key in &pending.withdrawn {
            out.delete(TableId::PowerDayParts, key);
        }
        partial(hour, out);
        self.idle = !current && self.holds.is_empty();
        self.rebuild = false;
        self.last = Some(at);
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        for hour in [self.last, self.run_first] {
            blob.varint(hour.map_or(0, |hour| u64::from(hour as u32) + 1))?;
        }
        blob.varint(u64::from(self.recover))?;
        let mut links: Vec<(u32, u32)> = self.links.iter().map(|(&raw, &slot)| (raw, slot)).collect();
        links.sort_unstable();
        blob.varint(links.len() as u64)?;
        for (raw, slot) in links {
            blob.varint(u64::from(raw))?;
            blob.varint(u64::from(slot))?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        let mut hour = || -> io::Result<Option<i32>> {
            Ok(match reader.varint()? {
                0 => None,
                held => Some(u32::try_from(held - 1).map_err(|_| invalid("PowerLedger's state names an hour past 32 bits"))? as i32),
            })
        };
        self.last = hour()?;
        self.run_first = hour()?;
        self.recover = match reader.varint()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("PowerLedger's state names a recovery that is neither due nor done")),
        };
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("PowerLedger's state counts more links than it holds"));
        }
        for _ in 0..count {
            let raw = reader.varint_u32()?;
            let slot = reader.varint_u32()?;
            if self.links.insert(raw, slot).is_some() {
                return Err(invalid(format!("PowerLedger's state links country {raw} twice")));
            }
        }
        reader.finish()?;
        self.aliases = aliases_of(&self.links);
        self.rebuild = true;
        Ok(())
    }
}

fn cells<R: RowCodec>(row: &R, out: &mut Rows) -> io::Result<Vec<Value>> {
    let mut cells = Vec::new();
    row.visit_wire(&mut |column, field| {
        if column != cells.len() {
            return Err(invalid("A row's fields came out of order"));
        }
        cells.push(match field {
            Field::NotRecorded => Value::Unrecorded,
            Field::U8(value) => Value::U8(value),
            Field::I8(value) => Value::I8(value),
            Field::U16(value) => Value::U16(value),
            Field::I16(value) => Value::I16(value),
            Field::U32(value) => Value::U32(value),
            Field::I32(value) => Value::I32(value),
            Field::U64(value) => Value::U64(value),
            Field::I64(value) => Value::I64(value),
            Field::Bool(value) => Value::Bool(value),
            Field::OptionalI32(value) => Value::OptionalI32(value),
            Field::OptionalU32(value) => Value::OptionalU32(value),
            Field::OptionalI64(value) => Value::OptionalI64(value),
            Field::OptionalU64(value) => Value::OptionalU64(value),
            Field::Bytes(text) => out.text(text),
            Field::Opaque(Opaque::Encoded(bytes, _)) => out.opaque(bytes.to_vec()),
            Field::Opaque(Opaque::Native(value)) => {
                let mut bytes = Vec::new();
                value.encode_value(&mut bytes)?;
                out.opaque(bytes)
            }
            Field::Empty | Field::U32List(_) | Field::U64List(_) => return Err(invalid("A power row field of a kind its tables do not hold")),
        });
        Ok(())
    })?;
    Ok(cells)
}
