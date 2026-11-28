use recorder_derive_framework::framework::{Columns, Hour, Keys, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
};
use replay_data::rows::{
    CountryPowerChangeRow, CountryPowerDailyCostRow, CountryPowerDailyRow, Field, Opaque, PoliticsCaptureCoverageRow as Streams, PowerCause, PowerCurrency, PowerDayPartRow as Part,
    PowerHoldRow as Hold, PowerIdentity, PowerItemRow as Item, PowerObjectKind, PowerOperation, PowerSite, PowerSpendCoverageRow as Coverage, PowerSpendDayRow as SpendDay,
    PowerSpendGainRow as SpendGain, PowerSpendPaymentRow as SpendPayment, PowerSpendTotalRow as Total, Recorded, RowCodec, TableId, TagHolderRow,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io, mem,
};

mod inputs;
mod tally;
#[cfg(test)]
mod tests;

use inputs::{Witness, invalid};
use tally::{DayRuns, ItemDays, Payment, midnight};

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&inputs::COUNTRY_SLOT), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&inputs::LINK_SLOT), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryPowerChanges, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryPowerDaily, columns: Columns::Every, watch: Watch::Dropped, required: false },
    Read { table: TableId::CountryPowerDailyCosts, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::PoliticsCaptureCoverage, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryCommandPower, columns: Columns::These(&inputs::ALLOCATED), watch: Watch::Changed, required: false },
];

const TABLES: &[Output] = &[
    Output::State(TableId::PowerItems),
    Output::State(TableId::PowerDayParts),
    Output::State(TableId::PowerHolds),
    Output::State(TableId::PowerSpendCoverage),
    Output::Log(TableId::PowerSpendDays, Keys::Hour(40)),
    Output::Log(TableId::PowerSpendPayments, Keys::Hour(40)),
    Output::Log(TableId::PowerSpendGains, Keys::Hour(40)),
];

const NONE: u32 = TagHolderRow::NONE;
const POLITICAL: u8 = PowerCurrency::Political as u8;
const MAX_BATCH_PAYMENTS: u64 = (u32::MAX as u64) + 1;

fn currency_bit(currency: PowerCurrency) -> u8 {
    match currency {
        PowerCurrency::Political => Streams::PP,
        PowerCurrency::Command => Streams::CP,
    }
}

fn item_key(holder: u32, ordinal: u32) -> u128 {
    u128::from(holder) << 32 | u128::from(ordinal)
}

fn part_key(holder: u32, currency: u8, ordinal: u32) -> u128 {
    u128::from(holder) << 40 | u128::from(currency) << 32 | u128::from(ordinal)
}

fn hold_key(holder: u32, hold: u64) -> u128 {
    u128::from(holder) << 64 | u128::from(hold)
}

fn encode_signed(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

fn decode_signed(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
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

fn recurring(kind: u8, cause: PowerCause) -> bool {
    kind != Item::CHANGE || matches!(cause, PowerCause::DailyGain | PowerCause::AbilityUpkeep)
}

type Holds = BTreeMap<u64, Hold>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct BatchKey {
    raw: u32,
    epoch: u64,
    day: i32,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CostPart {
    spent: Recorded<i64>,
    payments: i64,
    covered: bool,
    cost_ordinal: u32,
    read: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CostBatch {
    holder: u32,
    unmatched: bool,
    parts: BTreeMap<u32, CostPart>,
}

impl CostBatch {
    fn part_row(&self, key: BatchKey, ordinal: u32, part: &CostPart) -> Part {
        Part {
            holder: self.holder,
            currency: POLITICAL,
            ordinal,
            day: key.day,
            spent: part.spent,
            payments: part.payments,
            covered: part.covered,
            sequence: Some(key.sequence),
            cost_ordinal: Some(part.cost_ordinal),
            read: part.read,
        }
    }
}

fn project(holder: u32, ordinal: u32, parts: &[(BatchKey, &CostPart)]) -> Part {
    let day = parts[0].0.day;
    let mut sum: Option<i128> = Some(0);
    let mut payments: i64 = 0;
    let mut covered = true;
    let mut sequence = parts[0].0.sequence;
    let mut cost_ordinal = parts[0].1.cost_ordinal;
    let mut read = parts[0].1.read;
    for &(key, part) in parts {
        sum = match (sum, part.spent) {
            (Some(total), Recorded::Value(spent)) => Some(total + i128::from(spent)),
            _ => None,
        };
        payments = payments.saturating_add(part.payments);
        covered &= part.covered;
        if (key.sequence, part.cost_ordinal) > (sequence, cost_ordinal) {
            sequence = key.sequence;
            cost_ordinal = part.cost_ordinal;
        }
        read = read.min(part.read);
    }
    let spent = sum.and_then(|total| i64::try_from(total).ok());
    Part {
        holder,
        currency: POLITICAL,
        ordinal,
        day,
        spent: spent.map_or(Recorded::NotRecorded, Recorded::Value),
        payments,
        covered: covered && spent.is_some(),
        sequence: Some(sequence),
        cost_ordinal: Some(cost_ordinal),
        read,
    }
}

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

#[derive(Clone, Copy, Debug)]
struct DayEvent {
    days: i64,
    payments: i64,
    charge: Recorded<i64>,
    unknown: i64,
    uncovered: i64,
    realized_gained: Recorded<i64>,
    realized_lost: Recorded<i64>,
    realized_unknown: i64,
    credits: i64,
}

impl Default for DayEvent {
    fn default() -> Self {
        Self {
            days: 0,
            payments: 0,
            charge: Recorded::Value(0),
            unknown: 0,
            uncovered: 0,
            realized_gained: Recorded::Value(0),
            realized_lost: Recorded::Value(0),
            realized_unknown: 0,
            credits: 0,
        }
    }
}

impl DayEvent {
    fn add(cell: &mut Recorded<i64>, amount: i128) {
        if let Recorded::Value(held) = *cell {
            *cell = i64::try_from(i128::from(held) + amount).map_or(Recorded::NotRecorded, Recorded::Value);
        }
    }

    fn pay(&mut self, payment: &Payment, new_day: bool, newly_uncovered: bool) {
        self.days += i64::from(new_day);
        self.payments = self.payments.saturating_add(payment.count);
        Self::add(&mut self.charge, payment.charge.unwrap_or(0));
        if payment.charge.is_none() {
            self.unknown = self.unknown.saturating_add(payment.count);
        }
        self.uncovered += i64::from(newly_uncovered);
        match payment.realized {
            Some(moved) => {
                Self::add(&mut self.realized_gained, moved.max(0));
                Self::add(&mut self.realized_lost, moved.min(0));
            }
            None => self.realized_unknown = self.realized_unknown.saturating_add(payment.count),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PaymentEvent {
    known: Recorded<i64>,
    payments: i64,
    unknown: i64,
    uncovered: i64,
    latest: Option<(i32, u64, u32)>,
}

impl Default for PaymentEvent {
    fn default() -> Self {
        Self { known: Recorded::Value(0), payments: 0, unknown: 0, uncovered: 0, latest: None }
    }
}

impl PaymentEvent {
    fn pay(&mut self, payment: &Payment) {
        if payment.count <= 0 {
            return;
        }
        DayEvent::add(&mut self.known, payment.charge.unwrap_or(0));
        self.payments = self.payments.saturating_add(payment.count);
        if payment.charge.is_none() {
            self.unknown = self.unknown.saturating_add(payment.count);
        }
        if !payment.covered {
            self.uncovered = self.uncovered.saturating_add(payment.count);
        }
        let latest = (payment.hour, payment.sequence, payment.ordinal);
        if self.latest.is_none_or(|held| latest >= held) {
            self.latest = Some(latest);
        }
    }

    fn unknown_credit(&mut self, times: i64) {
        if times > 0 {
            self.unknown = self.unknown.saturating_add(times);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct GainEvent {
    gained: Recorded<i64>,
    capped: Recorded<i64>,
    days: i64,
    unknown_days: i64,
    unmatched: i64,
}

impl Default for GainEvent {
    fn default() -> Self {
        Self { gained: Recorded::Value(0), capped: Recorded::Value(0), days: 0, unknown_days: 0, unmatched: 0 }
    }
}

impl GainEvent {
    fn try_add_opt(cell: Recorded<i64>, amount: Option<i128>) -> Option<Recorded<i64>> {
        match (cell, amount) {
            (Recorded::Value(held), Some(add)) => i64::try_from(i128::from(held) + add).ok().map(Recorded::Value),
            _ => Some(Recorded::NotRecorded),
        }
    }

    fn try_merge(&mut self, gained: Option<i128>, capped: Option<i128>, days: i64, unknown_days: i64, unmatched: i64) -> bool {
        let Some(next_gained) = Self::try_add_opt(self.gained, gained) else {
            return false;
        };
        let Some(next_capped) = Self::try_add_opt(self.capped, capped) else {
            return false;
        };
        let Some(next_days) = self.days.checked_add(days) else {
            return false;
        };
        let Some(next_unknown) = self.unknown_days.checked_add(unknown_days) else {
            return false;
        };
        let Some(next_unmatched) = self.unmatched.checked_add(unmatched) else {
            return false;
        };
        self.gained = next_gained;
        self.capped = next_capped;
        self.days = next_days;
        self.unknown_days = next_unknown;
        self.unmatched = next_unmatched;
        true
    }

    fn is_nonempty(&self) -> bool {
        self.days > 0
            || self.unknown_days > 0
            || self.unmatched > 0
            || self.gained != Recorded::Value(0)
            || self.capped != Recorded::Value(0)
    }
}

fn push_gain(events: &mut Vec<GainEvent>, gained: Option<i128>, capped: Option<i128>, days: i64, unknown_days: i64, unmatched: i64) {
    if let Some(last) = events.last_mut()
        && last.try_merge(gained, capped, days, unknown_days, unmatched)
    {
        return;
    }
    let mut rem_gained = gained.unwrap_or(0);
    let mut rem_capped = capped.unwrap_or(0);
    let mut first = true;
    loop {
        let g = match gained {
            Some(_) => {
                let piece = rem_gained.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
                rem_gained -= i128::from(piece);
                Recorded::Value(piece)
            }
            None if first => Recorded::NotRecorded,
            None => Recorded::Value(0),
        };
        let c = match capped {
            Some(_) => {
                let piece = rem_capped.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
                rem_capped -= i128::from(piece);
                Recorded::Value(piece)
            }
            None if first => Recorded::NotRecorded,
            None => Recorded::Value(0),
        };
        let event = GainEvent {
            gained: g,
            capped: c,
            days: if first { days } else { 0 },
            unknown_days: if first { unknown_days } else { 0 },
            unmatched: if first { unmatched } else { 0 },
        };
        if event.is_nonempty() {
            events.push(event);
        }
        first = false;
        if rem_gained == 0 && rem_capped == 0 {
            break;
        }
    }
}

fn push_gain_day(events: &mut Vec<GainEvent>, (gained, capped, read): tally::GainDay) {
    let (gained, unknown_days) = match gained {
        Some(gained) => (Some(i128::from(gained)), 0),
        None => (Some(0), 1),
    };
    push_gain(events, gained, Some(i128::from(capped)), i64::from(read), unknown_days, 0);
}

fn instant_gain(row: &CountryPowerChangeRow) -> Option<(Option<i128>, Option<i128>, i64)> {
    let before = row.balance_before_raw;
    let after = match row.realized_delta_raw {
        Recorded::Value(delta) => Some(before.wrapping_add(delta)),
        Recorded::NotRecorded => None,
    };
    if is_hold(row) {
        return match row.operation {
            PowerOperation::AllocationClamp => match after {
                Some(after) => {
                    let delta = i128::from(after) - i128::from(before);
                    (delta < 0).then_some((Some(0), Some(-delta), 0))
                }
                None => Some((Some(0), None, 1)),
            },
            PowerOperation::Refund => {
                let after = after?;
                let delta = i128::from(after) - i128::from(before);
                let requested = row.requested_delta_raw.map(i128::from).filter(|&req| req > delta && delta >= 0)?;
                Some((Some(0), Some(requested - delta), 0))
            }
            _ => None,
        };
    }
    if skipped(row.cause) {
        return None;
    }
    match row.operation {
        PowerOperation::Grant | PowerOperation::Refund | PowerOperation::Add => match row.requested_delta_raw {
            Some(req) if req > 0 => {
                let req = i128::from(req);
                let capped = after.map(|after| {
                    let realized = i128::from(after) - i128::from(before);
                    if realized < req { req - realized } else { 0 }
                });
                Some((Some(req), capped, i64::from(capped.is_none())))
            }
            Some(_) => None,
            None => Some((None, None, 1)),
        },
        PowerOperation::Set => match row.requested_delta_raw {
            Some(req) if req > 0 => {
                let req = i128::from(req);
                let capped = after.map(|after| {
                    let realized = i128::from(after) - i128::from(before);
                    if realized < req { req - realized } else { 0 }
                });
                Some((Some(req), capped, i64::from(capped.is_none())))
            }
            Some(_) => None,
            None if after.is_some_and(|after| after <= before) => None,
            None => Some((None, None, 1)),
        },
        PowerOperation::Subtract | PowerOperation::Reset | PowerOperation::AllocationClamp | PowerOperation::Clamp => None,
    }
}

#[derive(Default)]
struct Pending {
    day_events: BTreeMap<(u32, u8, u32, i32), DayEvent>,
    payment_events: BTreeMap<(u32, u8, u32), PaymentEvent>,
    gain_events: BTreeMap<(u32, u8, Option<i32>), Vec<GainEvent>>,
    items: Vec<(Item, ItemId)>,
    fresh: HashMap<(u32, u64), Vec<usize>>,
    change_parts: BTreeMap<u128, Part>,
    dirty_costs: BTreeSet<(u32, u32)>,
    holds: Vec<HoldRow>,
    costs: HashMap<u32, CostHour>,
    raws: HashMap<u32, RawHour>,
    daily: BTreeSet<u32>,
    recovered: HashSet<BatchKey>,
    backlog: bool,
}

struct Batch {
    key: BatchKey,
    batch: CostBatch,
    rows: Vec<CountryPowerDailyCostRow>,
    moved: bool,
    move_on_change: bool,
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
    change_parts: HashMap<u32, (i32, Vec<u128>)>,
    batches: BTreeMap<BatchKey, CostBatch>,
    by_raw: HashMap<u32, Vec<BatchKey>>,
    by_holder: HashMap<u32, Vec<BatchKey>>,
    settled: HashMap<u32, DayRuns>,
    item_days: HashMap<(u32, u8, u32), ItemDays>,
    holds: HashMap<u32, Holds>,
    mismatched: HashSet<u32>,
    seen_changes: HashMap<u32, (u64, u64)>,
    idle: bool,
}

impl PowerLedger {
    fn see_change(&mut self, country: u32, epoch: u64, sequence: u64) -> bool {
        let mark = (epoch, sequence);
        match self.seen_changes.entry(country) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if *entry.get() >= mark {
                    return false;
                }
                entry.insert(mark);
                true
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(mark);
                true
            }
        }
    }

    fn slot(&self, raw: u32) -> u32 {
        if raw == 0 {
            return NONE;
        }
        self.links.get(&raw).copied().unwrap_or(NONE)
    }

    fn holder_steady(&self, holder: u32) -> bool {
        self.aliases.get(&holder).is_some_and(|raws| raws.iter().any(|raw| self.mismatched.contains(raw)))
            || self.holder_batches(holder).iter().any(|key| self.mismatched.contains(&key.raw))
    }

    fn index_batches(&mut self) {
        self.by_raw.clear();
        self.by_holder.clear();
        for (&key, batch) in &self.batches {
            self.by_raw.entry(key.raw).or_default().push(key);
            self.by_holder.entry(batch.holder).or_default().push(key);
        }
    }

    fn insert_batch(&mut self, key: BatchKey, batch: CostBatch, pending: &mut Pending) {
        if batch.parts.is_empty() {
            self.remove_batch(key, pending);
            return;
        }
        if self.batches.contains_key(&key) {
            self.remove_batch(key, pending);
        }
        for &ordinal in batch.parts.keys() {
            pending.dirty_costs.insert((batch.holder, ordinal));
        }
        self.by_raw.entry(key.raw).or_default().push(key);
        self.by_holder.entry(batch.holder).or_default().push(key);
        self.batches.insert(key, batch);
    }

    fn remove_batch(&mut self, key: BatchKey, pending: &mut Pending) -> Option<CostBatch> {
        let batch = self.batches.remove(&key)?;
        for &ordinal in batch.parts.keys() {
            pending.dirty_costs.insert((batch.holder, ordinal));
        }
        if let Some(keys) = self.by_raw.get_mut(&key.raw) {
            keys.retain(|held| *held != key);
            if keys.is_empty() {
                self.by_raw.remove(&key.raw);
            }
        }
        if let Some(keys) = self.by_holder.get_mut(&batch.holder) {
            keys.retain(|held| *held != key);
            if keys.is_empty() {
                self.by_holder.remove(&batch.holder);
            }
        }
        Some(batch)
    }

    fn mark_batch_unknown(&mut self, key: BatchKey, pending: &mut Pending) {
        let Some(batch) = self.batches.get_mut(&key) else { return };
        for (&ordinal, part) in &mut batch.parts {
            part.covered = false;
            part.spent = Recorded::NotRecorded;
            pending.dirty_costs.insert((batch.holder, ordinal));
        }
    }

    fn raw_batches(&self, raw: u32) -> Vec<BatchKey> {
        self.by_raw.get(&raw).cloned().unwrap_or_default()
    }

    fn holder_batches(&self, holder: u32) -> &[BatchKey] {
        self.by_holder.get(&holder).map_or(&[][..], Vec::as_slice)
    }

    fn record_settled(&mut self, raw: u32, day: i32) {
        self.settled.entry(raw).or_default().insert(day);
    }

    fn unrecord_settled(&mut self, raw: u32, day: i32) {
        if let Some(runs) = self.settled.get_mut(&raw) {
            runs.remove(day);
            if runs.0.is_empty() {
                self.settled.remove(&raw);
            }
        }
    }

    fn read_identities(&mut self, hour: &Hour<'_>) -> Vec<u32> {
        let links = mem::take(&mut self.links);
        for (raw, slot) in inputs::links(hour) {
            self.links.insert(raw, slot);
        }
        self.aliases = aliases_of(&self.links);
        self.live = inputs::countries(hour).into_iter().map(|(_, slot)| slot).collect();
        let mut moves = BTreeSet::new();
        for (&raw, &from) in &links {
            if self.slot(raw) != from {
                moves.insert(raw);
            }
        }
        for (&raw, &to) in &self.links {
            if links.get(&raw).copied().unwrap_or(NONE) != to {
                moves.insert(raw);
            }
        }
        moves.into_iter().collect()
    }

    fn read_tables(&mut self, own: &Rows<'_>) -> io::Result<()> {
        self.items.clear();
        self.next.clear();
        for row in inputs::every::<Item>(&own.table(TableId::PowerItems))? {
            self.items.entry((row.holder, ItemId::of(&row).hash())).or_default().push(row.ordinal);
            let next = self.next.entry(row.holder).or_default();
            *next = (*next).max(row.ordinal.saturating_add(1));
        }
        self.change_parts.clear();
        for row in inputs::every::<Part>(&own.table(TableId::PowerDayParts))? {
            if row.cost_ordinal.is_none() {
                let open = self.change_parts.entry(row.holder).or_insert((row.day, Vec::new()));
                open.0 = open.0.min(row.day);
                open.1.push(row.key());
            }
        }
        self.holds.clear();
        for row in inputs::every::<Hold>(&own.table(TableId::PowerHolds))? {
            self.holds.entry(row.holder).or_default().insert(row.hold, row);
        }
        Ok(())
    }

    fn witness(&self, witnesses: Option<&replay_columnar::store::TableView<'_>>, holder: u32) -> Option<Witness> {
        let witnesses = witnesses?;
        self.aliases.get(&holder)?.iter().rev().find_map(|&raw| inputs::witness(witnesses, raw))
    }

    fn held_change_part(own: &Rows<'_>, pending: &Pending, key: u128, day: i32) -> io::Result<Option<Part>> {
        if let Some(part) = pending.change_parts.get(&key) {
            return Ok(Some(part.clone()));
        }
        Ok(inputs::keyed::<Part>(&own.table(TableId::PowerDayParts), key)?.filter(|part| part.day == day && part.cost_ordinal.is_none()))
    }

    fn open_change_part(&mut self, pending: &mut Pending, part: Part) {
        let key = part.key();
        let open = self.change_parts.entry(part.holder).or_insert((part.day, Vec::new()));
        if !open.1.contains(&key) {
            open.1.push(key);
        }
        pending.change_parts.insert(key, part);
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

    fn settle(&mut self, own: &Rows<'_>, pending: &mut Pending, part: &Part) -> io::Result<()> {
        if part.payments <= 0 {
            return Ok(());
        }
        if part.ordinal == Total::GAINS {
            if part.payments > 0 {
                push_gain(pending.gain_events.entry((part.holder, POLITICAL, None)).or_default(), Some(0), Some(0), 0, 0, part.payments);
            }
            return Ok(());
        }
        let Some(sequence) = part.sequence else { return Ok(()) };
        let item = Self::item_row(own, pending, part.holder, part.ordinal)?;
        if !item.charged {
            match part.spent {
                Recorded::Value(_) => {
                    let event = pending.day_events.entry((part.holder, part.currency, part.ordinal, part.day)).or_default();
                    event.credits = event.credits.saturating_add(part.payments);
                }
                Recorded::NotRecorded => {
                    pending.payment_events.entry((part.holder, part.currency, part.ordinal)).or_default().unknown_credit(part.payments);
                }
            }
            return Ok(());
        }
        let spent = match part.spent {
            Recorded::Value(spent) => Some(i128::from(spent)),
            Recorded::NotRecorded => None,
        };
        let payment = Payment {
            recurring: recurring(item.kind, item.cause),
            hour: part.day,
            sequence,
            ordinal: part.cost_ordinal.unwrap_or(0),
            charge: spent,
            covered: part.covered,
            realized: None,
            count: part.payments,
        };
        if payment.recurring {
            let key = (part.holder, part.currency, part.ordinal);
            let seen = self.item_days.entry(key).or_default();
            if let Some((day, new_day, newly_uncovered)) = tally::count_day(seen, &payment) {
                pending.day_events.entry((part.holder, part.currency, part.ordinal, day)).or_default().pay(&payment, new_day, newly_uncovered);
            }
            if seen.counted.0.is_empty() {
                self.item_days.remove(&key);
            }
        } else {
            pending.payment_events.entry((part.holder, part.currency, part.ordinal)).or_default().pay(&payment);
        }
        Ok(())
    }

    fn roll(&mut self, hour: &Hour<'_>, pending: &mut Pending, out: &mut Rows<'_>) -> io::Result<()> {
        let day = midnight(hour.hour);
        let ended_changes: Vec<u32> = self.change_parts.iter().filter(|(_, (opened, _))| *opened != day).map(|(&holder, _)| holder).collect();
        for holder in ended_changes {
            let (_, keys) = self.change_parts.remove(&holder).expect("held");
            for key in keys {
                if let Some(part) = inputs::keyed::<Part>(&out.table(TableId::PowerDayParts), key)?
                    && part.day != day
                {
                    out.delete(TableId::PowerDayParts, key);
                }
            }
        }
        let ended_batches: Vec<BatchKey> = self.batches.keys().copied().filter(|key| key.day != day).collect();
        for key in ended_batches {
            if let Some(batch) = self.remove_batch(key, pending) {
                for (&ordinal, part) in &batch.parts {
                    self.settle(out, pending, &batch.part_row(key, ordinal, part))?;
                }
                if !batch.parts.is_empty() {
                    self.record_settled(key.raw, key.day);
                }
            }
        }
        Ok(())
    }

    fn withhold_ended(&mut self, pending: &mut Pending, today: i32) {
        let ended: Vec<BatchKey> = self.batches.keys().copied().filter(|key| key.day != today).collect();
        for key in ended {
            self.mark_batch_unknown(key, pending);
        }
    }

    fn confirms_zero(witnesses: Option<&replay_columnar::store::TableView<'_>>, raw: u32, at: i32, day: i32, epoch: u64, run_first: i32) -> bool {
        day > run_first && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(epoch, at, Streams::PP_DAILY_COSTS, at))
    }

    fn confirms_empty(witnesses: Option<&replay_columnar::store::TableView<'_>>, raw: u32, at: i32, day: i32, run_first: i32, daily: Option<&CountryPowerDailyRow>) -> bool {
        daily.is_some_and(|daily| {
            midnight(daily.day_hour) == day
                && daily.pp_cost_raw == Some(0)
                && daily.pp_sequence.is_some()
                && Self::confirms_zero(witnesses, raw, at, day, daily.epoch, run_first)
        })
    }

    fn reconcile(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, moved: &HashSet<u32>) -> io::Result<()> {
        let today = midnight(hour.hour);
        let run_first = self.run_first.unwrap_or(hour.hour);
        let written_days = inputs::written_cost_days(hour)?;
        let deleted_countries = inputs::deleted_cost_countries(hour);
        let keys: Vec<BatchKey> = self.batches.keys().copied().collect();
        for key in keys {
            let Some(batch) = self.batches.get(&key).cloned() else { continue };
            let all_day_rows: Vec<CountryPowerDailyCostRow> = inputs::costs_of(hour, key.raw)?.into_iter().filter(|cost| midnight(cost.day_hour) == key.day).collect();
            let rows: Vec<CountryPowerDailyCostRow> = all_day_rows.iter().filter(|cost| cost.epoch == key.epoch && cost.pp_sequence == key.sequence).cloned().collect();
            let moved_country = moved.contains(&key.raw) || batch.holder != self.slot(key.raw);
            let fresh = inputs::fresh_daily(hour, key.raw)?;
            let applied = fresh.as_ref().filter(|daily| daily.epoch == key.epoch && midnight(daily.day_hour) == key.day && daily.pp_sequence == Some(key.sequence));
            let confirmed_zero = Self::confirms_empty(witnesses, key.raw, hour.hour, key.day, run_first, fresh.as_ref());
            if key.day != today
                && (moved_country || rows.is_empty())
                && written_days.contains(&(key.raw, key.day))
                && !self.settled.get(&key.raw).is_some_and(|runs| runs.contains(key.day))
                && (all_day_rows.iter().any(|cost| cost.cost_raw != 0) || confirmed_zero)
            {
                self.remove_batch(key, pending);
                continue;
            }
            if rows.is_empty() {
                let emptied = confirmed_zero
                    && all_day_rows
                        .iter()
                        .all(|cost| fresh.as_ref().is_some_and(|daily| cost.epoch == daily.epoch && Some(cost.pp_sequence) == daily.pp_sequence) && cost.cost_raw == 0);
                let held_sum = batch.parts.values().try_fold(0i128, |sum, part| match part.spent {
                    Recorded::Value(spent) => Some(sum + i128::from(spent)),
                    Recorded::NotRecorded => None,
                });
                let agrees = applied.is_some_and(|daily| held_sum.is_some() && daily.pp_cost_raw.map(i128::from) == held_sum);
                let contradicted = applied.is_some_and(|daily| held_sum.zip(daily.pp_cost_raw).is_some_and(|(sum, cost)| sum != i128::from(cost)));
                if emptied {
                    self.remove_batch(key, pending);
                    self.mismatched.remove(&key.raw);
                    if !self.raw_batches(key.raw).iter().any(|held| held.day == key.day) {
                        self.record_settled(key.raw, key.day);
                    }
                } else if key.day == today && agrees {
                } else {
                    if contradicted {
                        self.mismatched.insert(key.raw);
                    }
                    self.mark_batch_unknown(key, pending);
                }
                continue;
            }
            if !moved_country {
                pending.recovered.insert(key);
            }
            let move_on_change = key.day != today && moved_country && deleted_countries.contains(&key.raw) && !self.settled.get(&key.raw).is_some_and(|runs| runs.contains(key.day));
            self.reconcile_batch(hour, own, witnesses, pending, Batch { key, batch, rows, moved: moved_country, move_on_change }, applied)?;
        }
        Ok(())
    }

    fn reconcile_batch(
        &mut self,
        hour: &Hour<'_>,
        own: &Rows<'_>,
        witnesses: Option<&replay_columnar::store::TableView<'_>>,
        pending: &mut Pending,
        work: Batch,
        applied: Option<&CountryPowerDailyRow>,
    ) -> io::Result<()> {
        let Batch { key, mut batch, rows, moved, move_on_change } = work;
        let at = hour.hour;
        let holder = batch.holder;
        let matched = applied.is_some();
        let total: i128 = rows.iter().map(|cost| i128::from(cost.cost_raw)).sum();
        let reconciles = matched && applied.is_some_and(|daily| daily.pp_cost_raw.is_some_and(|cost| i128::from(cost) == total));
        let mut remaining: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
        let mut unknown_new = false;
        for cost in rows.iter().filter(|cost| cost.cost_raw != 0) {
            let ordinal = if batch.unmatched {
                Some(Total::GAINS)
            } else if moved {
                self.known_item(own, pending, holder, &ItemId::cost(cost))?
            } else {
                let first = First { site: cost.site, hour: key.day, read: at, country: key.raw, epoch: key.epoch, sequence: u64::from(cost.ordinal) };
                Some(self.item(own, pending, holder, ItemId::cost(cost), first)?)
            };
            let Some(ordinal) = ordinal else {
                unknown_new = true;
                continue;
            };
            let entry = remaining.entry(ordinal).or_insert((0, 0, cost.ordinal));
            entry.0 += i128::from(cost.cost_raw);
            entry.1 += 1;
            entry.2 = entry.2.min(cost.ordinal);
        }
        let exact = |ordinal: u32, part: &CostPart| {
            remaining.get(&ordinal).is_some_and(|&(sum, payments, _)| part.payments == payments && matches!(part.spent, Recorded::Value(spent) if i128::from(spent) == sum))
        };
        let reordered = batch.parts.iter().any(|(&ordinal, part)| remaining.get(&ordinal).is_some_and(|&(_, _, first)| part.cost_ordinal != first));
        let opened: Vec<u32> = remaining.keys().copied().filter(|ordinal| !batch.parts.contains_key(ordinal)).collect();
        let changed = unknown_new || !batch.parts.iter().all(|(&ordinal, part)| exact(ordinal, part)) || !opened.is_empty();
        if matched || changed {
            if !reconciles {
                self.mismatched.insert(key.raw);
            } else {
                self.mismatched.remove(&key.raw);
            }
        }
        if !changed {
            if reordered && let Some(held) = self.batches.get_mut(&key) {
                for (&ordinal, part) in &mut held.parts {
                    if let Some(&(_, _, first)) = remaining.get(&ordinal)
                        && part.cost_ordinal != first
                    {
                        part.cost_ordinal = first;
                        pending.dirty_costs.insert((held.holder, ordinal));
                    }
                }
            }
            if !matched {
                pending.daily.insert(key.raw);
            }
            return Ok(());
        }
        if moved && !move_on_change {
            self.mark_batch_unknown(key, pending);
            return Ok(());
        }
        if move_on_change {
            let to = self.slot(key.raw);
            pending.recovered.insert(key);
            pending.daily.remove(&key.raw);
            let seen = pending.costs.entry(to).or_default();
            for cost in rows.iter().filter(|cost| cost.cost_raw != 0) {
                seen.unmatched |= !matched;
                seen.negative |= matched && cost.cost_raw < 0;
            }
            let covered = reconciles
                && !self.mismatched.contains(&key.raw)
                && witnesses.and_then(|view| inputs::witness(view, key.raw)).is_some_and(|witness| witness.covers(key.epoch, key.day, Streams::PP | Streams::PP_DAILY_COSTS, at));
            let mut moved_parts: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
            for cost in rows.iter().filter(|cost| cost.cost_raw != 0) {
                let ordinal = if batch.unmatched {
                    Total::GAINS
                } else {
                    let first = First { site: cost.site, hour: key.day, read: at, country: key.raw, epoch: key.epoch, sequence: u64::from(cost.ordinal) };
                    self.item(own, pending, to, ItemId::cost(cost), first)?
                };
                let entry = moved_parts.entry(ordinal).or_insert((0, 0, cost.ordinal));
                entry.0 += i128::from(cost.cost_raw);
                entry.1 += 1;
                entry.2 = entry.2.min(cost.ordinal);
            }
            let mut next_parts = BTreeMap::new();
            for (ordinal, (sum, payments, first)) in moved_parts {
                let spent = i64::try_from(sum).ok();
                next_parts.insert(
                    ordinal,
                    CostPart {
                        spent: spent.map_or(Recorded::NotRecorded, Recorded::Value),
                        payments,
                        covered: covered && spent.is_some(),
                        cost_ordinal: first,
                        read: at,
                    },
                );
            }
            let confirmed_empty = next_parts.is_empty() && reconciles && Self::confirms_empty(witnesses, key.raw, at, key.day, self.run_first.unwrap_or(at), applied);
            batch.holder = to;
            batch.parts = next_parts;
            self.insert_batch(key, batch, pending);
            if confirmed_empty && !self.raw_batches(key.raw).iter().any(|held| held.day == key.day) {
                self.record_settled(key.raw, key.day);
            }
            return Ok(());
        }
        let seen = pending.costs.entry(holder).or_default();
        for cost in rows.iter().filter(|cost| cost.cost_raw != 0) {
            seen.unmatched |= !matched;
            seen.negative |= matched && cost.cost_raw < 0;
        }
        let covered = reconciles && !self.mismatched.contains(&key.raw) && witnesses.and_then(|view| inputs::witness(view, key.raw)).is_some_and(|witness| witness.covers(key.epoch, key.day, Streams::PP | Streams::PP_DAILY_COSTS, at));
        let mut next_parts = BTreeMap::new();
        for (ordinal, mut part) in batch.parts {
            match remaining.get(&ordinal) {
                Some(&(_, _, first)) if exact(ordinal, &part) => {
                    part.cost_ordinal = first;
                    if part.covered && !reconciles {
                        part.covered = false;
                    }
                    next_parts.insert(ordinal, part);
                }
                Some(&(sum, payments, first)) => {
                    let spent = i64::try_from(sum).ok();
                    part.spent = spent.map_or(Recorded::NotRecorded, Recorded::Value);
                    part.payments = payments;
                    part.cost_ordinal = first;
                    part.covered = covered && spent.is_some();
                    next_parts.insert(ordinal, part);
                }
                None => {}
            }
        }
        for ordinal in opened {
            let (sum, payments, first) = remaining[&ordinal];
            let spent = i64::try_from(sum).ok();
            next_parts.insert(
                ordinal,
                CostPart {
                    spent: spent.map_or(Recorded::NotRecorded, Recorded::Value),
                    payments,
                    covered: covered && spent.is_some(),
                    cost_ordinal: first,
                    read: at,
                },
            );
        }
        let confirmed_empty = next_parts.is_empty() && reconciles && Self::confirms_empty(witnesses, key.raw, at, key.day, self.run_first.unwrap_or(at), applied);
        batch.parts = next_parts;
        self.insert_batch(key, batch, pending);
        if confirmed_empty && !self.raw_batches(key.raw).iter().any(|held| held.day == key.day) {
            self.record_settled(key.raw, key.day);
        }
        Ok(())
    }

    fn changes(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending) -> io::Result<()> {
        let at = hour.hour;
        let complete = inputs::changes_complete(hour);
        let changes = inputs::changes(hour)?;
        pending.backlog = changes.backlog;
        if changes.backlog {
            for row in changes.rows {
                if instant_gain(&row).is_some() || charge(&row).is_some() {
                    self.see_change(row.country, row.epoch, row.sequence);
                }
            }
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
            let holder = self.slot(row.country);
            let gain = instant_gain(&row);
            let charge = charge(&row);
            let fresh = (gain.is_some() || charge.is_some())
                && self.see_change(row.country, row.epoch, row.sequence);
            if let Some((gained, capped, unknown_days)) = gain
                && fresh
            {
                push_gain(
                    pending.gain_events.entry((holder, row.currency as u8, Some(row.hour))).or_default(),
                    gained,
                    capped,
                    0,
                    unknown_days,
                    0,
                );
            }
            if charge.is_none() && skipped(row.cause) {
                continue;
            }
            let first = First { site: row.site, hour: row.hour, read: at, country: row.country, epoch: row.epoch, sequence: row.sequence };
            let id = ItemId::change(&row);
            let item_kind = id.kind;
            let ordinal = self.item(own, pending, holder, id, first)?;
            let Some(charge) = charge else {
                continue;
            };
            if !fresh {
                continue;
            }
            let payment = Payment {
                recurring: recurring(item_kind, row.cause),
                hour: row.hour,
                sequence: row.sequence,
                ordinal: 0,
                charge,
                covered,
                realized: Some(row.realized_delta_raw()),
                count: 1,
            };
            if payment.recurring {
                let item_key = (holder, row.currency as u8, ordinal);
                let seen = self.item_days.entry(item_key).or_default();
                if let Some((day, new_day, newly_uncovered)) = tally::count_day(seen, &payment) {
                    pending.day_events.entry((holder, row.currency as u8, ordinal, day)).or_default().pay(&payment, new_day, newly_uncovered);
                }
                if seen.counted.0.is_empty() {
                    self.item_days.remove(&item_key);
                }
            } else {
                pending.payment_events.entry((holder, row.currency as u8, ordinal)).or_default().pay(&payment);
            }
            let day = midnight(at);
            if row.cause != PowerCause::DailyGain && midnight(row.hour) == day {
                let key = part_key(holder, row.currency as u8, ordinal);
                let mut part = Self::held_change_part(own, pending, key, day)?.unwrap_or(Part {
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
                part.covered &= covered && matches!(part.spent, Recorded::Value(_));
                self.open_change_part(pending, part);
            }
        }
        Ok(())
    }

    fn days(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, moves: &[u32]) -> io::Result<()> {
        let run_first = self.run_first.unwrap_or(hour.hour);
        for day in inputs::closed_days(hour)? {
            if day.day_hour > run_first {
                let holder = self.slot(day.country);
                for (currency, gain) in tally::day_gains(&day).iter().enumerate() {
                    push_gain_day(pending.gain_events.entry((holder, currency as u8, None)).or_default(), *gain);
                }
            }
        }
        for raw in inputs::moved_daily(hour) {
            pending.daily.insert(raw);
        }
        let today = midnight(hour.hour);
        let complete = inputs::costs_complete(hour);
        let mut reread = BTreeSet::new();
        for &raw in moves {
            if self.leave(hour, pending, today, raw, complete)? {
                reread.insert(raw);
            }
        }
        let mut publications = inputs::publications(hour)?;
        if !pending.recovered.is_empty() {
            let recovered = mem::take(&mut pending.recovered);
            publications.rows.retain_mut(|publication| {
                let held = !publication.costs.is_empty();
                publication.costs.retain(|cost| {
                    !recovered.contains(&BatchKey {
                        raw: publication.country,
                        epoch: cost.epoch,
                        day: midnight(cost.day_hour),
                        sequence: cost.pp_sequence,
                    })
                });
                !held || !publication.costs.is_empty()
            });
        }
        if !publications.backlog {
            let published: HashSet<u32> = publications
                .rows
                .iter()
                .filter(|publication| !(publication.deleted_only && reread.contains(&publication.country)))
                .map(|publication| publication.country)
                .collect();
            for publication in publications.rows.iter().filter(|publication| published.contains(&publication.country)) {
                self.publish(hour, own, witnesses, pending, run_first, publication)?;
            }
            for raw in reread.into_iter().filter(|raw| complete && !published.contains(raw)) {
                let costs: Vec<CountryPowerDailyCostRow> = inputs::costs_of(hour, raw)?.into_iter().filter(|cost| midnight(cost.day_hour) == today).collect();
                if !costs.is_empty() {
                    self.publish(hour, own, witnesses, pending, run_first, &inputs::Publication { country: raw, costs, deleted_only: false })?;
                }
            }
            if complete && witnesses.is_some() {
                for zero in inputs::fresh_zero_dailies(hour) {
                    if zero.day <= run_first {
                        continue;
                    }
                    let raw = zero.country;
                    let has_today = self.raw_batches(raw).iter().any(|key| key.day == today);
                    let already_settled = self.settled.get(&raw).is_some_and(|runs| runs.contains(zero.day));
                    let needs_clear = (zero.day == today && has_today) || (self.mismatched.contains(&raw) && !has_today);
                    if already_settled && !needs_clear {
                        continue;
                    }
                    if !Self::confirms_zero(witnesses, raw, hour.hour, zero.day, zero.epoch, run_first)
                        || !inputs::no_nonzero_costs_for(hour, raw, zero.epoch, zero.day, zero.sequence)?
                    {
                        continue;
                    }
                    if zero.day == today {
                        let keys: Vec<BatchKey> = self.raw_batches(raw).into_iter().filter(|key| key.day == today).collect();
                        for key in keys {
                            self.remove_batch(key, pending);
                        }
                    }
                    if !self.raw_batches(raw).iter().any(|key| key.day == today) {
                        self.mismatched.remove(&raw);
                    }
                    if !self.raw_batches(raw).iter().any(|key| key.day == zero.day) {
                        self.record_settled(raw, zero.day);
                    }
                    pending.daily.remove(&raw);
                }
            }
        }
        for raw in mem::take(&mut pending.daily) {
            if let Some(steady) = self.mismatch(hour, raw)? {
                if steady {
                    self.mismatched.insert(raw);
                } else {
                    self.mismatched.remove(&raw);
                }
            }
        }
        Ok(())
    }

    fn mismatch(&self, hour: &Hour<'_>, raw: u32) -> io::Result<Option<bool>> {
        let today = midnight(hour.hour);
        let has_open = || self.raw_batches(raw).into_iter().any(|key| key.day == today && self.batches.get(&key).is_some_and(|batch| !batch.unmatched));
        let Some(daily) = inputs::daily(hour, raw)? else {
            return Ok(Some(has_open()));
        };
        let Some(sequence) = daily.pp_sequence else {
            return Ok(Some(has_open()));
        };
        let key = BatchKey { raw, epoch: daily.epoch, day: midnight(daily.day_hour), sequence };
        let Some(batch) = self.batches.get(&key).filter(|batch| !batch.unmatched && batch.holder == self.slot(raw)) else {
            return Ok(None);
        };
        let mut sum = 0i128;
        for part in batch.parts.values() {
            match part.spent {
                Recorded::Value(spent) => sum += i128::from(spent),
                Recorded::NotRecorded => return Ok(Some(true)),
            }
        }
        Ok(Some(daily.pp_cost_raw.is_none_or(|cost| i128::from(cost) != sum)))
    }

    fn publish(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, run_first: i32, publication: &inputs::Publication) -> io::Result<()> {
        if publication.deleted_only {
            return self.delete(hour, own, witnesses, pending, run_first, publication);
        }
        let at = hour.hour;
        let raw = publication.country;
        let holder = self.slot(raw);
        let daily = inputs::daily(hour, raw)?;
        let fresh = inputs::fresh_daily(hour, raw)?;
        let today = midnight(at);
        let mut applications: BTreeMap<(u64, i32, u64), Vec<&CountryPowerDailyCostRow>> = BTreeMap::new();
        for cost in &publication.costs {
            applications.entry((cost.epoch, midnight(cost.day_hour), cost.pp_sequence)).or_default().push(cost);
        }
        let today_keys: HashSet<BatchKey> = applications
            .keys()
            .filter(|&&(_, day, _)| day == today && day > run_first)
            .map(|&(epoch, day, sequence)| BatchKey { raw, epoch, day, sequence })
            .collect();
        if !today_keys.is_empty() {
            let replaced: Vec<BatchKey> = self.raw_batches(raw).into_iter().filter(|key| key.day == today && !today_keys.contains(key)).collect();
            for old in replaced {
                self.remove_batch(old, pending);
            }
        }
        let mut newly_settled = Vec::new();
        for ((epoch, day, sequence), costs) in applications {
            let opens = day == today;
            if !opens && self.settled.get(&raw).is_some_and(|runs| runs.contains(day)) {
                continue;
            }
            let matched = daily.as_ref().is_some_and(|daily| daily.epoch == epoch && midnight(daily.day_hour) == day && daily.pp_sequence == Some(sequence));
            let seen = pending.costs.entry(holder).or_default();
            for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                seen.unmatched |= !matched;
                seen.negative |= matched && cost.cost_raw < 0;
            }
            if matched {
                let sum: i128 = costs.iter().map(|cost| i128::from(cost.cost_raw)).sum();
                let steady = daily.as_ref().is_some_and(|daily| daily.pp_cost_raw.is_none_or(|cost| i128::from(cost) != sum));
                if steady {
                    self.mismatched.insert(raw);
                } else {
                    self.mismatched.remove(&raw);
                }
                pending.daily.remove(&raw);
            }
            if day <= run_first {
                continue;
            }
            let covered = !self.mismatched.contains(&raw) && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(epoch, day, Streams::PP | Streams::PP_DAILY_COSTS, at));
            let mut items: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
            for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                let ordinal = if matched {
                    let first = First { site: cost.site, hour: day, read: at, country: raw, epoch, sequence: u64::from(cost.ordinal) };
                    self.item(own, pending, holder, ItemId::cost(cost), first)?
                } else {
                    Total::GAINS
                };
                let entry = items.entry(ordinal).or_insert((0, 0, cost.ordinal));
                entry.0 += i128::from(cost.cost_raw);
                entry.1 += 1;
                entry.2 = entry.2.min(cost.ordinal);
            }
            let confirmed_empty = items.is_empty() && matched && Self::confirms_empty(witnesses, raw, at, day, run_first, fresh.as_ref());
            if opens {
                let key = BatchKey { raw, epoch, day, sequence };
                let old = self.remove_batch(key, pending).filter(|old| old.holder == holder);
                let mut parts = BTreeMap::new();
                for (ordinal, (sum, payments, first)) in items {
                    let spent = i64::try_from(sum).ok();
                    let held = old.as_ref().and_then(|batch| batch.parts.get(&ordinal));
                    parts.insert(
                        ordinal,
                        CostPart {
                            spent: spent.map_or(Recorded::NotRecorded, Recorded::Value),
                            payments,
                            covered: covered && spent.is_some() && held.is_none_or(|held| held.covered),
                            cost_ordinal: first,
                            read: held.map_or(at, |held| held.read),
                        },
                    );
                }
                let had_parts = !parts.is_empty();
                self.insert_batch(key, CostBatch { holder, unmatched: !matched, parts }, pending);
                if had_parts {
                    self.unrecord_settled(raw, day);
                } else if confirmed_empty {
                    newly_settled.push(day);
                }
            } else {
                for (ordinal, (sum, payments, first)) in &items {
                    let spent = i64::try_from(*sum).ok();
                    let part = Part {
                        holder,
                        currency: POLITICAL,
                        ordinal: *ordinal,
                        day,
                        spent: spent.map_or(Recorded::NotRecorded, Recorded::Value),
                        payments: *payments,
                        covered: covered && spent.is_some(),
                        sequence: Some(sequence),
                        cost_ordinal: Some(*first),
                        read: at,
                    };
                    self.settle(own, pending, &part)?;
                }
                if !items.is_empty() || confirmed_empty {
                    newly_settled.push(day);
                }
            }
        }
        for day in newly_settled {
            if !self.raw_batches(raw).iter().any(|key| key.day == day) {
                self.record_settled(raw, day);
            }
        }
        Ok(())
    }

    fn delete(&mut self, hour: &Hour<'_>, own: &Rows<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, pending: &mut Pending, run_first: i32, publication: &inputs::Publication) -> io::Result<()> {
        let at = hour.hour;
        let raw = publication.country;
        let holder = self.slot(raw);
        let today = midnight(at);
        let daily = inputs::fresh_daily(hour, raw)?.filter(|daily| midnight(daily.day_hour) == today);
        if publication.costs.is_empty() && Self::confirms_empty(witnesses, raw, at, today, run_first, daily.as_ref()) {
            let keys: Vec<BatchKey> = self.raw_batches(raw).into_iter().filter(|key| key.day == today).collect();
            for key in keys {
                self.remove_batch(key, pending);
            }
            self.mismatched.remove(&raw);
            pending.daily.remove(&raw);
            self.record_settled(raw, today);
            return Ok(());
        }
        let mut survivors: BTreeMap<(u64, u64), Vec<&CountryPowerDailyCostRow>> = BTreeMap::new();
        for cost in publication.costs.iter().filter(|cost| midnight(cost.day_hour) == today) {
            survivors.entry((cost.epoch, cost.pp_sequence)).or_default().push(cost);
        }
        for ((epoch, sequence), costs) in survivors {
            let key = BatchKey { raw, epoch, day: today, sequence };
            let Some(mut batch) = self.batches.get(&key).cloned() else { continue };
            let total: i128 = costs.iter().map(|cost| i128::from(cost.cost_raw)).sum();
            let applied = daily.as_ref().filter(|daily| daily.epoch == epoch && daily.pp_sequence == Some(sequence));
            let reconciles = applied.is_some_and(|daily| daily.pp_cost_raw.is_some_and(|cost| i128::from(cost) == total));
            let confirmed_zero = reconciles && Self::confirms_empty(witnesses, raw, at, today, run_first, applied);
            if batch.holder != holder {
                if !reconciles {
                    self.mismatched.insert(raw);
                } else {
                    self.mismatched.remove(&raw);
                }
                pending.daily.remove(&raw);
                let covered = reconciles
                    && !self.mismatched.contains(&raw)
                    && witnesses.and_then(|view| inputs::witness(view, raw)).is_some_and(|witness| witness.covers(epoch, today, Streams::PP | Streams::PP_DAILY_COSTS, at));
                let mut remaining: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
                for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                    let ordinal = if batch.unmatched {
                        Total::GAINS
                    } else {
                        let first = First { site: cost.site, hour: today, read: at, country: raw, epoch, sequence: u64::from(cost.ordinal) };
                        self.item(own, pending, holder, ItemId::cost(cost), first)?
                    };
                    let entry = remaining.entry(ordinal).or_insert((0, 0, cost.ordinal));
                    entry.0 += i128::from(cost.cost_raw);
                    entry.1 += 1;
                    entry.2 = entry.2.min(cost.ordinal);
                }
                let mut next_parts = BTreeMap::new();
                for (ordinal, (sum, payments, first)) in remaining {
                    let spent = i64::try_from(sum).ok();
                    next_parts.insert(
                        ordinal,
                        CostPart {
                            spent: spent.map_or(Recorded::NotRecorded, Recorded::Value),
                            payments,
                            covered: covered && spent.is_some(),
                            cost_ordinal: first,
                            read: at,
                        },
                    );
                }
                let emptied = next_parts.is_empty();
                batch.holder = holder;
                batch.parts = next_parts;
                self.insert_batch(key, batch, pending);
                if emptied && confirmed_zero && !self.raw_batches(raw).iter().any(|held| held.day == today) {
                    self.record_settled(raw, today);
                }
                continue;
            }
            let mut remaining: BTreeMap<u32, (i128, i64, u32)> = BTreeMap::new();
            for cost in costs.iter().filter(|cost| cost.cost_raw != 0) {
                let ordinal = if batch.unmatched { Some(Total::GAINS) } else { self.known_item(own, pending, holder, &ItemId::cost(cost))? };
                if let Some(ordinal) = ordinal {
                    let entry = remaining.entry(ordinal).or_insert((0, 0, cost.ordinal));
                    entry.0 += i128::from(cost.cost_raw);
                    entry.1 += 1;
                    entry.2 = entry.2.min(cost.ordinal);
                }
            }
            let exact = |ordinal: u32, part: &CostPart| {
                remaining.get(&ordinal).is_some_and(|&(sum, payments, _)| part.payments == payments && matches!(part.spent, Recorded::Value(spent) if i128::from(spent) == sum))
            };
            let reordered = batch.parts.iter().any(|(&ordinal, part)| remaining.get(&ordinal).is_some_and(|&(_, _, first)| part.cost_ordinal != first));
            if batch.parts.iter().all(|(&ordinal, part)| exact(ordinal, part)) {
                if reordered && let Some(held) = self.batches.get_mut(&key) {
                    for (&ordinal, part) in &mut held.parts {
                        if let Some(&(_, _, first)) = remaining.get(&ordinal)
                            && part.cost_ordinal != first
                        {
                            part.cost_ordinal = first;
                            pending.dirty_costs.insert((held.holder, ordinal));
                        }
                    }
                }
                continue;
            }
            if !reconciles {
                self.mismatched.insert(raw);
            } else {
                self.mismatched.remove(&raw);
            }
            pending.daily.remove(&raw);
            let mut next_parts = BTreeMap::new();
            for (ordinal, mut part) in batch.parts {
                match remaining.get(&ordinal) {
                    Some(&(_, _, first)) if exact(ordinal, &part) => {
                        part.cost_ordinal = first;
                        if part.covered && !reconciles {
                            part.covered = false;
                        }
                        next_parts.insert(ordinal, part);
                    }
                    Some(&(sum, payments, first)) => {
                        let spent = i64::try_from(sum).ok();
                        part.spent = spent.map_or(Recorded::NotRecorded, Recorded::Value);
                        part.payments = payments;
                        part.cost_ordinal = first;
                        part.covered = part.covered && reconciles && spent.is_some();
                        next_parts.insert(ordinal, part);
                    }
                    None => {}
                }
            }
            let emptied = next_parts.is_empty();
            batch.parts = next_parts;
            self.insert_batch(key, batch, pending);
            if emptied && confirmed_zero && !self.raw_batches(raw).iter().any(|held| held.day == today) {
                self.record_settled(raw, today);
            }
        }
        Ok(())
    }

    fn leave(&mut self, hour: &Hour<'_>, pending: &mut Pending, today: i32, raw: u32, complete: bool) -> io::Result<bool> {
        let to = self.slot(raw);
        let keys: Vec<BatchKey> = self
            .raw_batches(raw)
            .into_iter()
            .filter(|key| {
                key.day == today
                    && self.batches.get(key).is_some_and(|batch| {
                        batch.holder != to || (to != NONE && batch.parts.values().any(|part| part.spent == Recorded::NotRecorded || !part.covered))
                    })
            })
            .collect();
        if keys.is_empty() {
            return Ok(false);
        }
        let current: HashSet<(u64, i32, u64)> = if complete && to != NONE {
            inputs::costs_of(hour, raw)?
                .into_iter()
                .filter(|cost| midnight(cost.day_hour) == today)
                .map(|cost| (cost.epoch, midnight(cost.day_hour), cost.pp_sequence))
                .collect()
        } else {
            HashSet::new()
        };
        let mut reread = false;
        for key in keys {
            let same_holder = self.batches.get(&key).is_some_and(|batch| batch.holder == to);
            if current.contains(&(key.epoch, key.day, key.sequence)) {
                if !same_holder {
                    self.remove_batch(key, pending);
                }
                reread = true;
            } else if !same_holder {
                self.mark_batch_unknown(key, pending);
            }
        }
        Ok(reread)
    }

    fn coverage(&self, hour: &Hour<'_>, witnesses: Option<&replay_columnar::store::TableView<'_>>, adjacent: bool, pending: &Pending, out: &mut Rows) -> io::Result<bool> {
        let at = hour.hour;
        let changes_known = inputs::changes_complete(hour);
        let daily_known = inputs::daily_complete(hour);
        let input_reason = inputs::input_reason(hour).unwrap_or_default().into_bytes();
        for key in recorder_derive_framework::forces::held(out, TableId::PowerSpendCoverage) {
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
            let steady = self.holder_steady(holder);
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

fn partial(hour: &Hour<'_>, out: &mut Rows, logged_gains: bool) {
    let changes = hour.status(TableId::CountryPowerChanges);
    let costs = hour.status(TableId::CountryPowerDailyCosts);
    let set = |out: &mut Rows, derived: TableId, status: Status, not_recorded: &[u8]| match status {
        Status::NotRecorded => out.unavailable(derived, not_recorded),
        other => out.status(derived, other),
    };
    if changes != Status::Complete {
        set(out, TableId::PowerDayParts, changes, b"CountryPowerChanges is not recorded");
        set(out, TableId::PowerSpendDays, changes, b"CountryPowerChanges is not recorded");
        set(out, TableId::PowerSpendPayments, changes, b"CountryPowerChanges is not recorded");
        if !logged_gains || matches!(changes, Status::Truncated { .. }) {
            set(out, TableId::PowerSpendGains, changes, b"CountryPowerChanges is not recorded");
        }
    } else if costs != Status::Complete {
        set(out, TableId::PowerDayParts, costs, b"CountryPowerDailyCosts is not recorded");
        set(out, TableId::PowerSpendDays, costs, b"CountryPowerDailyCosts is not recorded");
    }
}

fn write_day_runs(blob: &mut Bytes, runs: &DayRuns) -> io::Result<()> {
    blob.varint(runs.0.len() as u64)?;
    let mut prev_last: Option<i32> = None;
    for &(first, last) in &runs.0 {
        if first.rem_euclid(24) != 0 || last.rem_euclid(24) != 0 {
            return Err(invalid("PowerLedger's state holds a day run that is not aligned to midnight"));
        }
        if let Some(prev) = prev_last
            && prev.checked_add(24).is_none_or(|next| first <= next)
        {
            return Err(invalid("PowerLedger's state holds unsorted, overlapping or unmerged day runs"));
        }
        let span = (i64::from(last) - i64::from(first)) / 24;
        blob.varint(u64::from(first as u32))?;
        blob.varint(u64::try_from(span).map_err(|_| invalid("PowerLedger's state holds an inverted day run"))?)?;
        prev_last = Some(last);
    }
    Ok(())
}

fn read_day_runs(reader: &mut Reader<'_>, limit: usize, allow_empty: bool) -> io::Result<DayRuns> {
    let run_count = reader.count()?;
    if (!allow_empty && run_count == 0) || run_count > limit {
        return Err(invalid("PowerLedger's state names an invalid day run count"));
    }
    let mut runs = Vec::with_capacity(run_count);
    let mut prev_last: Option<i32> = None;
    for _ in 0..run_count {
        let first = reader.varint_u32()? as i32;
        if first.rem_euclid(24) != 0 {
            return Err(invalid(format!("PowerLedger's state names day {first} that is not a midnight")));
        }
        let span = reader.varint()?;
        let last = i64::try_from(span)
            .ok()
            .and_then(|days| days.checked_mul(24))
            .and_then(|hours| i64::from(first).checked_add(hours))
            .and_then(|end| i32::try_from(end).ok())
            .ok_or_else(|| invalid("PowerLedger's state holds a day run past 32 bits"))?;
        if let Some(prev) = prev_last
            && prev.checked_add(24).is_none_or(|next| first <= next)
        {
            return Err(invalid("PowerLedger's state holds unsorted, overlapping or unmerged day runs"));
        }
        prev_last = Some(last);
        runs.push((first, last));
    }
    Ok(DayRuns(runs))
}

fn validate_batch_parts(batch: &CostBatch) -> io::Result<()> {
    if batch.parts.is_empty() {
        return Err(invalid("PowerLedger's state names an empty open batch"));
    }
    let mut spans = Vec::with_capacity(batch.parts.len());
    for (&ordinal, part) in &batch.parts {
        if batch.unmatched != (ordinal == Total::GAINS) {
            return Err(invalid("PowerLedger's state pairs an unmatched batch with a matched item or vice versa"));
        }
        if part.covered && matches!(part.spent, Recorded::NotRecorded) {
            return Err(invalid("PowerLedger's state marks an unknown part as covered"));
        }
        let payments = u64::try_from(part.payments)
            .ok()
            .filter(|&count| (1..=MAX_BATCH_PAYMENTS).contains(&count))
            .ok_or_else(|| invalid("PowerLedger's state holds an impossible payment count"))?;
        spans.push((part.cost_ordinal, payments));
    }
    spans.sort_unstable_by_key(|&(first, _)| first);
    for pair in spans.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(invalid("PowerLedger's state assigns two batch parts the same first cost ordinal"));
        }
    }
    let mut suffix = 0u64;
    for &(first, payments) in spans.iter().rev() {
        suffix = suffix
            .checked_add(payments)
            .filter(|&count| u64::from(first).checked_add(count).is_some_and(|end| end <= MAX_BATCH_PAYMENTS))
            .ok_or_else(|| invalid("PowerLedger's state holds batch parts past the source ordinal range"))?;
    }
    Ok(())
}

impl Writer for PowerLedger {
    fn name(&self) -> &'static str {
        "PowerLedger"
    }

    fn version(&self) -> u32 {
        5
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
        let missing_identity = [TableId::Countries, TableId::CountryLinks]
            .into_iter()
            .find_map(|table| {
                let status = hour.status(table);
                (status != Status::Complete).then_some((table, status))
            });
        if let Some((table, status)) = missing_identity {
            for log in [TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
                recorder_derive_framework::forces::forget_listed(hour, out, log);
            }
            for row in inputs::changes(hour)?.rows {
                if instant_gain(&row).is_some() || charge(&row).is_some() {
                    self.see_change(row.country, row.epoch, row.sequence);
                }
            }
            let reason = match status {
                Status::NotRecorded => format!("{}: not recorded", table.words()),
                Status::Truncated { missing } => format!("{}: read only in part ({missing} rows not read)", table.words()),
                Status::Unavailable { .. } => format!("{}: not available", table.words()),
                Status::Complete => unreachable!(),
            };
            for output in TABLES {
                match status {
                    Status::Unavailable { .. } => out.status(output.table(), status),
                    _ => out.unavailable(output.table(), reason.as_bytes()),
                }
            }
            self.rebuild = true;
            self.recover |= !hour.first;
            self.idle = false;
            self.last = None;
            return Ok(());
        }
        let whole = hour.fresh || self.rebuild;
        let adjacent = !hour.fresh && self.last.and_then(|last| last.checked_add(1)) == Some(at);
        let rolls = self.batches.keys().any(|key| key.day != midnight(at)) || self.change_parts.values().any(|(day, _)| *day != midnight(at));
        if !whole && self.idle && !rolls && !moved(hour) {
            partial(hour, out, false);
            self.last = Some(at);
            return Ok(());
        }
        for log in [TableId::PowerSpendDays, TableId::PowerSpendPayments, TableId::PowerSpendGains] {
            recorder_derive_framework::forces::forget_listed(hour, out, log);
        }
        if hour.first {
            self.recover = false;
        } else if hour.fresh {
            self.recover = true;
        }
        let moves = if whole || inputs::identities_moved(hour) { self.read_identities(hour) } else { Vec::new() };
        if whole {
            self.read_tables(out)?;
        }
        let witnesses = inputs::witnesses(hour);
        let mut pending = Pending::default();
        if self.recover && inputs::costs_complete(hour) {
            let moved_set: HashSet<u32> = moves.iter().copied().collect();
            self.reconcile(hour, out, witnesses.as_ref(), &mut pending, &moved_set)?;
            self.recover = false;
        } else if self.recover {
            self.withhold_ended(&mut pending, midnight(at));
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
        for (key, part) in &pending.change_parts {
            let cells = cells(part, out)?;
            out.set(TableId::PowerDayParts, *key, cells);
        }
        if !pending.dirty_costs.is_empty() {
            let mut by_holder: BTreeMap<u32, BTreeMap<u32, Vec<(BatchKey, &CostPart)>>> = BTreeMap::new();
            for (holder, ordinal) in pending.dirty_costs {
                by_holder.entry(holder).or_default().insert(ordinal, Vec::new());
            }
            for (holder, mut ordinals) in by_holder {
                for &batch_key in self.holder_batches(holder) {
                    if let Some(batch) = self.batches.get(&batch_key) {
                        for (&ordinal, part) in &batch.parts {
                            if let Some(parts) = ordinals.get_mut(&ordinal) {
                                parts.push((batch_key, part));
                            }
                        }
                    }
                }
                for (ordinal, parts) in ordinals {
                    let key = part_key(holder, POLITICAL, ordinal);
                    if parts.is_empty() {
                        if out.table(TableId::PowerDayParts).slot(key).is_some() {
                            out.delete(TableId::PowerDayParts, key);
                        }
                    } else {
                        let part = project(holder, ordinal, &parts);
                        let cells = cells(&part, out)?;
                        out.set(TableId::PowerDayParts, key, cells);
                    }
                }
            }
        }
        let logged_days = !pending.day_events.is_empty();
        let mut previous_group: Option<(u32, u8)> = None;
        let mut entry = 0u32;
        for ((holder, currency, ordinal, day), event) in pending.day_events {
            if previous_group == Some((holder, currency)) {
                entry = entry.checked_add(1).ok_or_else(|| invalid(format!("Holder {holder}'s power spend days pass u32 entries in one hour")))?;
            } else {
                previous_group = Some((holder, currency));
                entry = 0;
            }
            let row = SpendDay {
                holder,
                hour: at,
                currency,
                entry,
                ordinal,
                day,
                days: event.days,
                payments: event.payments,
                charge: event.charge,
                unknown: event.unknown,
                uncovered: event.uncovered,
                realized_gained: event.realized_gained,
                realized_lost: event.realized_lost,
                realized_unknown: event.realized_unknown,
                credits: event.credits,
            };
            let cells = cells(&row, out)?;
            out.append(TableId::PowerSpendDays, row.key(), cells);
        }
        let mut logged_payments = false;
        let mut previous_group: Option<(u32, u8)> = None;
        let mut entry = 0u32;
        for ((holder, currency, ordinal), event) in pending.payment_events {
            if event.payments == 0 && event.unknown == 0 {
                continue;
            }
            logged_payments = true;
            if previous_group == Some((holder, currency)) {
                entry = entry.checked_add(1).ok_or_else(|| invalid(format!("Holder {holder}'s power spend payments pass u32 entries in one hour")))?;
            } else {
                previous_group = Some((holder, currency));
                entry = 0;
            }
            let row = SpendPayment {
                holder,
                hour: at,
                currency,
                entry,
                ordinal,
                known: event.known,
                payments: event.payments,
                unknown: event.unknown,
                uncovered: event.uncovered,
                latest_hour: event.latest.map(|(hour, _, _)| hour),
                latest_sequence: event.latest.map(|(_, sequence, _)| sequence),
                latest_ordinal: event.latest.map(|(_, _, ordinal)| ordinal),
            };
            let cells = cells(&row, out)?;
            out.append(TableId::PowerSpendPayments, row.key(), cells);
        }
        let mut logged_gains = false;
        let mut previous_group: Option<(u32, u8)> = None;
        let mut entry = 0u32;
        for ((holder, currency, event_hour), events) in pending.gain_events {
            for event in events {
                if !event.is_nonempty() {
                    continue;
                }
                logged_gains = true;
                if previous_group == Some((holder, currency)) {
                    entry = entry.checked_add(1).ok_or_else(|| invalid(format!("Holder {holder}'s power spend gains pass u32 entries in one hour")))?;
                } else {
                    previous_group = Some((holder, currency));
                    entry = 0;
                }
                let row = SpendGain {
                    holder,
                    hour: at,
                    currency,
                    entry,
                    event_hour,
                    gained: event.gained,
                    capped: event.capped,
                    days: event.days,
                    unknown_days: event.unknown_days,
                    unmatched: event.unmatched,
                };
                let cells = cells(&row, out)?;
                out.append(TableId::PowerSpendGains, row.key(), cells);
            }
        }
        partial(hour, out, logged_gains);
        self.idle = !current
            && self.holds.is_empty()
            && !logged_days
            && !logged_payments
            && !logged_gains
            && inputs::changes_complete(hour)
            && inputs::costs_complete(hour);
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
        let mut mismatched: Vec<u32> = self.mismatched.iter().copied().collect();
        mismatched.sort_unstable();
        blob.varint(mismatched.len() as u64)?;
        for raw in mismatched {
            blob.varint(u64::from(raw))?;
        }
        let mut settled: Vec<(u32, &DayRuns)> = self.settled.iter().filter(|(_, runs)| !runs.0.is_empty()).map(|(&raw, runs)| (raw, runs)).collect();
        settled.sort_unstable_by_key(|&(raw, _)| raw);
        blob.varint(settled.len() as u64)?;
        for (raw, runs) in settled {
            blob.varint(u64::from(raw))?;
            write_day_runs(blob, runs)?;
        }
        let mut item_days: Vec<((u32, u8, u32), &ItemDays)> = self.item_days.iter().filter(|(_, seen)| !seen.counted.0.is_empty()).map(|(&key, seen)| (key, seen)).collect();
        item_days.sort_unstable_by_key(|&(key, _)| key);
        blob.varint(item_days.len() as u64)?;
        for ((holder, currency, ordinal), seen) in item_days {
            if PowerCurrency::try_from(currency).is_err() || ordinal == Total::GAINS {
                return Err(invalid("PowerLedger's state names an invalid item day key"));
            }
            if !seen.uncovered.is_subset_of(&seen.counted) {
                return Err(invalid("PowerLedger's state marks an uncounted day uncovered"));
            }
            blob.varint(u64::from(holder))?;
            blob.varint(u64::from(currency))?;
            blob.varint(u64::from(ordinal))?;
            write_day_runs(blob, &seen.counted)?;
            write_day_runs(blob, &seen.uncovered)?;
        }
        blob.varint(self.batches.len() as u64)?;
        for (&key, batch) in &self.batches {
            validate_batch_parts(batch)?;
            blob.varint(u64::from(key.raw))?;
            blob.varint(key.epoch)?;
            blob.varint(u64::from(key.day as u32))?;
            blob.varint(key.sequence)?;
            blob.varint(u64::from(batch.holder))?;
            blob.varint(u64::from(batch.unmatched))?;
            blob.varint(batch.parts.len() as u64)?;
            for (&ordinal, part) in &batch.parts {
                blob.varint(u64::from(ordinal))?;
                let flags = u64::from(part.covered) | (u64::from(matches!(part.spent, Recorded::Value(_))) << 1);
                blob.varint(flags)?;
                if let Recorded::Value(spent) = part.spent {
                    blob.varint(encode_signed(spent))?;
                }
                blob.varint(part.payments as u64)?;
                blob.varint(u64::from(part.cost_ordinal))?;
                blob.varint(u64::from(part.read as u32))?;
            }
        }
        let mut seen_changes: Vec<(u32, (u64, u64))> = self.seen_changes.iter().map(|(&country, &mark)| (country, mark)).collect();
        seen_changes.sort_unstable_by_key(|&(country, _)| country);
        blob.varint(seen_changes.len() as u64)?;
        for (country, (epoch, sequence)) in seen_changes {
            blob.varint(u64::from(country))?;
            blob.varint(epoch)?;
            blob.varint(sequence)?;
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
        let mismatched = reader.count()?;
        if mismatched > blob.len() {
            return Err(invalid("PowerLedger's state counts more mismatched countries than it holds"));
        }
        for _ in 0..mismatched {
            let raw = reader.varint_u32()?;
            if !self.mismatched.insert(raw) {
                return Err(invalid(format!("PowerLedger's state marks country {raw} mismatched twice")));
            }
        }
        let settled = reader.count()?;
        if settled > blob.len() {
            return Err(invalid("PowerLedger's state counts more settled countries than it holds"));
        }
        for _ in 0..settled {
            let raw = reader.varint_u32()?;
            let runs = read_day_runs(&mut reader, blob.len(), false)?;
            if self.settled.insert(raw, runs).is_some() {
                return Err(invalid(format!("PowerLedger's state settles country {raw} twice")));
            }
        }
        let item_days = reader.count()?;
        if item_days > blob.len() {
            return Err(invalid("PowerLedger's state counts more item day histories than it holds"));
        }
        for _ in 0..item_days {
            let holder = reader.varint_u32()?;
            let currency = u8::try_from(reader.varint()?).map_err(|_| invalid("PowerLedger's state names an item currency past u8"))?;
            if PowerCurrency::try_from(currency).is_err() {
                return Err(invalid("PowerLedger's state names an unsupported item currency"));
            }
            let ordinal = reader.varint_u32()?;
            if ordinal == Total::GAINS {
                return Err(invalid("PowerLedger's state records item days on the gains row"));
            }
            let counted = read_day_runs(&mut reader, blob.len(), false)?;
            let uncovered = read_day_runs(&mut reader, blob.len(), true)?;
            if !uncovered.is_subset_of(&counted) {
                return Err(invalid("PowerLedger's state marks an uncounted day uncovered"));
            }
            if self.item_days.insert((holder, currency, ordinal), ItemDays { counted, uncovered }).is_some() {
                return Err(invalid("PowerLedger's state holds duplicate item day histories"));
            }
        }
        let batches = reader.count()?;
        if batches > blob.len() {
            return Err(invalid("PowerLedger's state counts more open batches than it holds"));
        }
        for _ in 0..batches {
            let raw = reader.varint_u32()?;
            let epoch = reader.varint()?;
            let day = reader.varint_u32()? as i32;
            if day.rem_euclid(24) != 0 {
                return Err(invalid(format!("PowerLedger's state names batch day {day} that is not a midnight")));
            }
            let sequence = reader.varint()?;
            let holder = reader.varint_u32()?;
            let unmatched = match reader.varint()? {
                0 => false,
                1 => true,
                _ => return Err(invalid("PowerLedger's state names an unmatched flag that is neither set nor clear")),
            };
            let count = reader.count()?;
            if count == 0 || count > blob.len() {
                return Err(invalid("PowerLedger's state names an invalid batch part count"));
            }
            let mut parts = BTreeMap::new();
            for _ in 0..count {
                let ordinal = reader.varint_u32()?;
                let flags = reader.varint()?;
                if flags == 1 || flags > 3 {
                    return Err(invalid("PowerLedger's state names invalid part flags"));
                }
                let covered = flags & 1 != 0;
                let spent = if flags & 2 != 0 { Recorded::Value(decode_signed(reader.varint()?)) } else { Recorded::NotRecorded };
                let raw_payments = reader.varint()?;
                if !(1..=MAX_BATCH_PAYMENTS).contains(&raw_payments) {
                    return Err(invalid("PowerLedger's state holds an impossible payment count"));
                }
                let payments = raw_payments as i64;
                let cost_ordinal = reader.varint_u32()?;
                let read = reader.varint_u32()? as i32;
                if parts.insert(ordinal, CostPart { spent, payments, covered, cost_ordinal, read }).is_some() {
                    return Err(invalid(format!("PowerLedger's state names item {ordinal} twice in one batch")));
                }
            }
            let batch = CostBatch { holder, unmatched, parts };
            validate_batch_parts(&batch)?;
            let key = BatchKey { raw, epoch, day, sequence };
            if self.batches.insert(key, batch).is_some() {
                return Err(invalid("PowerLedger's state holds a duplicate open batch"));
            }
        }
        let seen_changes = reader.count()?;
        if seen_changes > blob.len() {
            return Err(invalid("PowerLedger's state counts more seen change marks than it holds"));
        }
        for _ in 0..seen_changes {
            let country = reader.varint_u32()?;
            let epoch = reader.varint()?;
            let sequence = reader.varint()?;
            if self.seen_changes.insert(country, (epoch, sequence)).is_some() {
                return Err(invalid(format!("PowerLedger's state repeats seen change mark for country {country}")));
            }
        }
        reader.finish()?;
        self.aliases = aliases_of(&self.links);
        self.index_batches();
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
