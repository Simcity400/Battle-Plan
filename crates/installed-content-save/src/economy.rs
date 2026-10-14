//! Literal economy and supply state carried by a text save.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, block, boolean, bytes, decimal, field, integer, invalid, required_block,
    required_bytes, scalar, typed_handle,
};
use installed_content::parser::{Item, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    /// Preserves absence of the supply root independently of empty country systems.
    pub supply: Option<BTreeMap<Vec<u8>, Supply>>,
    /// The whole equipment market may be absent; this is not an empty census.
    pub purchases: Option<BTreeMap<u64, Purchase>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    pub fuel: Option<Fuel>,
    /// Missing status is distinct from a present status whose gains are zero.
    pub experience: Option<Experience>,
    /// None preserves an absent resources object; an empty export list is distinct.
    pub exports: Option<Vec<ResourceExchange>>,
    /// The resource pools of the same object; `None` with it.
    pub pools: Option<ResourcePools>,
    /// None when the country writes no `logistics` block.
    pub logistics: Option<Logistics>,
    /// The country's outgoing lend-lease exchanges under `resources.lend_lease`,
    /// in save order; None preserves an absent resources object.
    pub lend_lease: Option<Vec<LendLease>>,
}

/// One saved lend-lease exchange of its giver.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LendLease {
    /// The exchange's full typed reference (low u32 type, high u32 id).
    pub id: u64,
    pub receiver: Vec<u8>,
    /// The exchange's `total_delivered` equipment pool.
    pub delivered: super::production::Pool,
    /// The exchange's `equipment_sunk` pool.
    pub sunk: super::production::Pool,
}

/// One saved resource pool: each nonzero fixed5 amount in resource index
/// order, keyed by its token, with zero amounts omitted. The content loader
/// appends a repeated declaration as another resource, so a name can repeat;
/// every occurrence is kept in written order.
pub type ResourcePool = Vec<(Vec<u8>, Decimal)>;

/// The resource pools emitted by the country resources writer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourcePools {
    /// The `produced` pool.
    pub produced: ResourcePool,
    /// The `transfer_overlord_subject` pool.
    pub transfer_overlord_subject: ResourcePool,
    /// The `imported` pool.
    pub imported: ResourcePool,
    /// `to_use`: three unkeyed pools backed by a Fenwick tree. The writer
    /// emits prefix(i) - prefix(i - 1) for odd i and the pool itself for
    /// even i, so the saved entries are the first pool, the second less
    /// the first, and the third.
    pub to_use: [ResourcePool; 3],
    /// The `to_export` pool.
    pub to_export: ResourcePool,
    /// The `base_export` pool.
    pub base_export: ResourcePool,
    /// The `exported` pool.
    pub exported: ResourcePool,
}

/// A saved `logistics` block. The writer includes its `history` only for
/// a country that is not AI-controlled, so an AI-controlled country's
/// block is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Logistics {
    Empty,
    /// Each series the writer emits (a field count above zero), by index.
    History(BTreeMap<u32, LoopHistory>),
}

/// One saved loop history: `history_queue` with its daily, twelve-record
/// and third windows in order, written as `0`, `1` and `2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopHistory {
    pub windows: [LoopWindow; 3],
}

/// One saved history window: its capacity, current slot and wrapped flag,
/// then `data`, every record's values in record
/// order (fixed5, five trimmed decimals). The writer omits `data` when every
/// value is zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopWindow {
    pub max_elements: i32,
    pub offset: i32,
    pub is_full: bool,
    pub data: Option<Vec<Decimal>>,
}

impl LoopWindow {
    /// The raw values of the record at `slot`, each record holding `fields`
    /// values. Omitted data is every value zero; data that does not hold
    /// `max_elements` records of `fields` values is refused.
    pub fn record(&self, slot: usize, fields: usize) -> io::Result<Vec<i64>> {
        let records = usize::try_from(self.max_elements)
            .map_err(|_| invalid("save logistics window has a negative capacity"))?;
        if slot >= records {
            return Err(invalid("save logistics slot is outside its window"));
        }
        let Some(data) = &self.data else {
            return Ok(vec![0; fields]);
        };
        if records.checked_mul(fields) != Some(data.len()) {
            return Err(invalid(
                "save logistics data is not the window's records of the expected size",
            ));
        }
        Ok(data[slot * fields..(slot + 1) * fields]
            .iter()
            .map(|value| value.raw)
            .collect())
    }
}

/// Exact native signed /32768 values. The writer omits the first eight fields
/// exactly when zero; presence is retained so only the comparator applies
/// that proven rule. The training divisor is always emitted, including zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Experience {
    pub army_raw: Option<i64>,
    pub army_daily_raw: Option<i64>,
    pub army_daily_training_raw: Option<i64>,
    pub navy_raw: Option<i64>,
    pub navy_daily_raw: Option<i64>,
    pub navy_daily_training_raw: Option<i64>,
    pub air_raw: Option<i64>,
    pub air_daily_raw: Option<i64>,
    /// Smoothed native training denominator, not a current integer army count.
    pub armies_for_training_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceExchange {
    pub id: u64,
    /// The enclosing country owns the export; receiver is its explicit saved recipient.
    pub receiver: Option<Vec<u8>>,
    pub resource: Option<Vec<u8>>,
    pub efficiency: Decimal,
    pub efficiency_due_to_lost_convoys: Decimal,
    pub delivered: Decimal,
    pub required_cic: i32,
    pub lended_cic: i32,
    /// The base exchange writer emits this block precisely when required != 0.
    /// Its inner assigned/required field mapping is not assumed here.
    pub convoy_subscriber_present: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Purchase {
    pub id: u64,
    pub seller: Vec<u8>,
    pub buyer: Vec<u8>,
    pub speed: i32,
    pub route_efficiency_sum: Decimal,
    pub route_days: i32,
    pub factory_cic_progress: Decimal,
    pub equipment_transfer_progress: Decimal,
    pub collected: Decimal,
    pub completed_deliveries: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Supply {
    /// Present system with omitted child collections is empty: the writer
    /// emits each collection precisely when its count != 0.
    pub motorization: Vec<Motorization>,
    pub disrupted: Vec<Disrupted>,
    pub foreign_homebases: Vec<ForeignHomebase>,
    pub temp_nodes: Vec<TemporaryNode>,
}

/// `temp_nodes`, emitted in array order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryNode {
    /// Repeated provinces stay separate: lookup selects the first match.
    pub ordinal: u32,
    pub province: i32,
    /// Base status is saved as Boolean truth.
    pub base: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Motorization {
    pub node_index: u32,
    pub map_entry_index: u32,
    pub province: i32,
    pub node_type: i32,
    pub actor: Vec<u8>,
    pub level: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disrupted {
    /// Signed save spelling preserves the full native u32 words.
    pub province: i32,
    pub node_type: i32,
    pub value: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignHomebase {
    /// Position is significant; repeated provinces must remain separate.
    pub ordinal: u32,
    pub province: u32,
    pub inner_province: u32,
    pub supply: Decimal,
    pub start: Decimal,
    pub penalty: Decimal,
    pub duration: Decimal,
    pub hours: Decimal,
    pub decay: Decimal,
    pub base: bool,
    /// The source string is serialized as `add`. The text
    /// writer emits even an empty string, stopping at its first NUL byte.
    pub source: Vec<u8>,
}

/// One saved `fuel_status`, in the fuel writer's fixed order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fuel {
    /// Decimal spelling of a native /32768 quantity, not raw /100000 fuel.
    pub current: FuelQuantity,
    /// Decimal spelling of a native /32768 quantity, not raw /100000 fuel.
    pub maximum: FuelQuantity,
    /// Native signed /100000 modifier values.
    pub gain: Decimal,
    pub cost: Decimal,
    pub gain_per_oil: Decimal,
    pub gain_from_states: Decimal,
    /// Native /32768 quantities written through the same fixed15 formatter.
    pub lend_lease_gain: FuelQuantity,
    pub lend_lease_consumption: FuelQuantity,
    /// Consumer entries in array order; each saved index is its position.
    pub consumers: Vec<FuelConsumer>,
    pub history_index: i32,
    pub remaining_hours: i32,
    /// The pending entry (index -1), then the ring entries in slot order.
    pub history: Vec<FuelHistory>,
}

/// One `fuel_consumer_data` block; the writer's `index` is the entry's
/// array position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuelConsumer {
    pub index: u32,
    pub priority: i32,
    pub received: Decimal,
    pub requested: Decimal,
}

/// One `history` block. Fuel, consumed and other are
/// /32768 quantities; gain and the three-value lists are /100000.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuelHistory {
    /// -1 for the pending entry, otherwise the ring slot.
    pub index: i32,
    pub fuel: FuelQuantity,
    pub consumed: FuelQuantity,
    pub gain: Decimal,
    pub other: FuelQuantity,
    pub requested: [Decimal; FUEL_CONSUMER_VALUES],
    pub received: [Decimal; FUEL_CONSUMER_VALUES],
}

/// History ring entries written after the pending entry: the writer loops
/// over exactly 24 slots whatever the ring's own header holds.
pub const FUEL_HISTORY_RING: usize = 24;
/// Values per history list; the list writer always writes three.
pub const FUEL_CONSUMER_VALUES: usize = 3;

/// Save text numerator over 100000. This is deliberately wider than i64:
/// formatting an i64 /32768 quantity can exceed i64 in decimal fixed5 units.
/// It is not a reconstruction of the native /32768 value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuelQuantity {
    pub decimal_numerator: i128,
    pub lexeme: Vec<u8>,
}

impl FuelQuantity {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes == b"--281474976710656.00000" {
            return Err(invalid(
                "native fixed15 text serializer cannot represent i64::MIN as a valid decimal",
            ));
        }
        let (negative, unsigned) = match bytes.first() {
            Some(b'-') => (true, &bytes[1..]),
            Some(b'+') => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        let mut parts = unsigned.split(|&byte| byte == b'.');
        let integer = parts.next().unwrap();
        let fraction = parts.next();
        if integer.is_empty()
            || !integer.iter().all(u8::is_ascii_digit)
            || parts.next().is_some()
            || fraction.is_some_and(|part| part.is_empty() || !part.iter().all(u8::is_ascii_digit))
        {
            return Err(invalid("invalid saved fixed15 quantity"));
        }
        let fraction = fraction.unwrap_or_default();
        if fraction
            .get(5..)
            .is_some_and(|tail| tail.iter().any(|&digit| digit != b'0'))
        {
            return Err(invalid("fixed15 quantity exceeds save formatter precision"));
        }
        let mut numerator = 0_i128;
        for digit in integer
            .iter()
            .copied()
            .chain(fraction.iter().copied().take(5))
            .chain(std::iter::repeat_n(b'0', 5 - fraction.len().min(5)))
        {
            numerator = numerator
                .checked_mul(10)
                .and_then(|value| value.checked_add(i128::from(digit - b'0')))
                .ok_or_else(|| invalid("saved fixed15 quantity exceeds i128"))?;
        }
        Ok(Self {
            decimal_numerator: if negative { -numerator } else { numerator },
            lexeme: bytes.to_vec(),
        })
    }

    /// The formatter truncates abs(raw)*100000/32768 then emits its
    /// original sign. The ratio exceeds one, so every valid output has one
    /// exact preimage. Re-encoding rejects non-native decimal spellings.
    pub fn raw(&self) -> io::Result<i64> {
        let magnitude = self
            .decimal_numerator
            .checked_abs()
            .ok_or_else(|| invalid("saved fixed15 magnitude exceeds i128"))?;
        let scaled = magnitude
            .checked_mul(32_768)
            .ok_or_else(|| invalid("saved fixed15 quantity exceeds native range"))?;
        let raw = scaled / 100_000 + i128::from(scaled % 100_000 != 0);
        if raw > i128::from(i64::MAX) || raw * 100_000 / 32_768 != magnitude {
            return Err(invalid(
                "saved fixed15 decimal has no exact supported native preimage",
            ));
        }
        Ok(if self.decimal_numerator < 0 {
            -(raw as i64)
        } else {
            raw as i64
        })
    }
}

fn required_decimal(items: &[Item], key: &[u8]) -> io::Result<Decimal> {
    decimal(items, key)?.ok_or_else(|| {
        invalid(format!(
            "save economy lacks {}",
            String::from_utf8_lossy(key)
        ))
    })
}

fn required_integer<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<T> {
    integer(&required_bytes(items, key)?)
}

fn fixed15(items: &[Item], key: &[u8]) -> io::Result<Option<i64>> {
    field(items, key)?
        .map(|value| FuelQuantity::parse(scalar(value)?)?.raw())
        .transpose()
}

/// A block's items in the order a native writer emits them. Every key must be
/// the one the writer puts at that position; nothing is looked up out of order.
struct Sequence<'a> {
    items: std::iter::Peekable<std::slice::Iter<'a, Item>>,
    block: &'static str,
}

impl<'a> Sequence<'a> {
    fn new(items: &'a [Item], block: &'static str) -> Self {
        Self {
            items: items.iter().peekable(),
            block,
        }
    }

    fn next(&mut self, key: &str) -> io::Result<&'a Value> {
        let item = self
            .items
            .next()
            .ok_or_else(|| invalid(format!("save {} ends before its {key}", self.block)))?;
        if item.key.as_deref() != Some(key.as_bytes()) {
            return Err(invalid(format!(
                "save {} has {} where its writer puts {key}",
                self.block,
                item.key
                    .as_deref()
                    .map_or("an anonymous value".into(), String::from_utf8_lossy)
            )));
        }
        Ok(&item.value)
    }

    fn peek_is(&mut self, key: &str) -> bool {
        self.items
            .peek()
            .is_some_and(|item| item.key.as_deref() == Some(key.as_bytes()))
    }

    fn scalar(&mut self, key: &str) -> io::Result<&'a [u8]> {
        scalar(self.next(key)?)
    }

    fn quantity(&mut self, key: &str) -> io::Result<FuelQuantity> {
        FuelQuantity::parse(self.scalar(key)?)
    }

    fn decimal(&mut self, key: &str) -> io::Result<Decimal> {
        Decimal::parse(self.scalar(key)?)
    }

    fn integer<T: std::str::FromStr>(&mut self, key: &str) -> io::Result<T> {
        integer(self.scalar(key)?)
    }

    fn end(mut self) -> io::Result<()> {
        match self.items.next() {
            None => Ok(()),
            Some(_) => Err(invalid(format!(
                "save {} continues after its writer's last field",
                self.block
            ))),
        }
    }
}

/// The fuel writer emits fuel, max_fuel,
/// fuel_gain, fuel_cost, fuel_gain_per_oil, fuel_gain_from_states and the two
/// lend-lease quantities, one `fuel_consumer_data` per consumer entry with its
/// position as index, history_index, remaining_hours, the pending `history`
/// (index -1) and FUEL_HISTORY_RING ring entries in slot order. Any other
/// sequence is refused rather than reinterpreted.
fn parse_fuel(items: &[Item]) -> io::Result<Fuel> {
    let mut fields = Sequence::new(items, "fuel_status");
    let current = fields.quantity("fuel")?;
    let maximum = fields.quantity("max_fuel")?;
    let gain = fields.decimal("fuel_gain")?;
    let cost = fields.decimal("fuel_cost")?;
    let gain_per_oil = fields.decimal("fuel_gain_per_oil")?;
    let gain_from_states = fields.decimal("fuel_gain_from_states")?;
    let lend_lease_gain = fields.quantity("fuel_gain_from_lend_lease")?;
    let lend_lease_consumption = fields.quantity("fuel_consumption_from_lend_lease")?;
    let mut consumers = Vec::new();
    while fields.peek_is("fuel_consumer_data") {
        let mut entry = Sequence::new(
            block(fields.next("fuel_consumer_data")?)?,
            "fuel_consumer_data",
        );
        let index = entry.integer::<u32>("index")?;
        if usize::try_from(index).ok() != Some(consumers.len()) {
            return Err(invalid(
                "save fuel_consumer_data index is not its array position",
            ));
        }
        consumers.push(FuelConsumer {
            index,
            priority: entry.integer("priority")?,
            received: entry.decimal("received")?,
            requested: entry.decimal("requested")?,
        });
        entry.end()?;
    }
    let history_index = fields.integer("history_index")?;
    let remaining_hours = fields.integer("remaining_hours")?;
    let mut history = Vec::with_capacity(1 + FUEL_HISTORY_RING);
    for expected in -1..FUEL_HISTORY_RING as i32 {
        let mut entry = Sequence::new(block(fields.next("history")?)?, "fuel history");
        let index = entry.integer::<i32>("index")?;
        if index != expected {
            return Err(invalid(format!(
                "save fuel history index {index} where its writer puts {expected}"
            )));
        }
        let fuel = entry.quantity("fuel")?;
        let consumed = entry.quantity("consumed")?;
        let gain = entry.decimal("fuel_gain")?;
        let other = entry.quantity("other")?;
        let requested = consumer_values(entry.next("requested")?)?;
        let received = consumer_values(entry.next("received")?)?;
        entry.end()?;
        history.push(FuelHistory {
            index,
            fuel,
            consumed,
            gain,
            other,
            requested,
            received,
        });
    }
    fields.end()?;
    Ok(Fuel {
        current,
        maximum,
        gain,
        cost,
        gain_per_oil,
        gain_from_states,
        lend_lease_gain,
        lend_lease_consumption,
        consumers,
        history_index,
        remaining_hours,
        history,
    })
}

/// One fuel-history list: exactly three anonymous /100000 values.
fn consumer_values(value: &Value) -> io::Result<[Decimal; FUEL_CONSUMER_VALUES]> {
    let items = block(value)?;
    if items.len() != FUEL_CONSUMER_VALUES || items.iter().any(|item| item.key.is_some()) {
        return Err(invalid(
            "save fuel history list is not three anonymous values",
        ));
    }
    let values = items
        .iter()
        .map(|item| Decimal::parse(scalar(&item.value)?))
        .collect::<io::Result<Vec<_>>>()?;
    Ok(values.try_into().expect("three values"))
}

/// The logistics writer saves `history` with one block per series in index
/// order, keyed by the index; each holds a `history_queue`. Any other
/// layout is refused.
fn parse_logistics(items: &[Item]) -> io::Result<Logistics> {
    let mut fields = Sequence::new(items, "logistics");
    if items.is_empty() {
        return Ok(Logistics::Empty);
    }
    let mut series = BTreeMap::new();
    for item in block(fields.next("history")?)? {
        let index = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("save logistics series lacks its index"))
            .and_then(integer::<u32>)?;
        if series.keys().next_back().is_some_and(|&last| last >= index) {
            return Err(invalid("save logistics series are not in index order"));
        }
        let mut history = Sequence::new(block(&item.value)?, "logistics series");
        let queue = block(history.next("history_queue")?)?;
        history.end()?;
        let mut queue = Sequence::new(queue, "logistics history_queue");
        let mut window = |key| queue.next(key).and_then(block).and_then(parse_window);
        let windows = [window("0")?, window("1")?, window("2")?];
        queue.end()?;
        series.insert(index, LoopHistory { windows });
    }
    fields.end()?;
    Ok(Logistics::History(series))
}

fn parse_window(items: &[Item]) -> io::Result<LoopWindow> {
    let mut fields = Sequence::new(items, "logistics window");
    let max_elements = fields.integer("max_elements")?;
    let offset = fields.integer("offset")?;
    let is_full = boolean(fields.next("is_full")?)?;
    let data = if fields.peek_is("data") {
        let values = block(fields.next("data")?)?;
        if values.iter().any(|item| item.key.is_some()) {
            return Err(invalid("save logistics data holds a keyed value"));
        }
        Some(
            values
                .iter()
                .map(|item| Decimal::parse(scalar(&item.value)?))
                .collect::<io::Result<Vec<_>>>()?,
        )
    } else {
        None
    };
    fields.end()?;
    Ok(LoopWindow {
        max_elements,
        offset,
        is_full,
        data,
    })
}

fn parse_experience(items: &[Item]) -> io::Result<Option<Experience>> {
    let Some(status) = field(items, b"experience_status")? else {
        return Ok(None);
    };
    let status = block(status)?;
    Ok(Some(Experience {
        army_raw: fixed15(status, b"army_experience")?,
        army_daily_raw: fixed15(status, b"army_experience_daily")?,
        army_daily_training_raw: fixed15(status, b"army_experience_daily_training")?,
        navy_raw: fixed15(status, b"navy_experience")?,
        navy_daily_raw: fixed15(status, b"navy_experience_daily")?,
        navy_daily_training_raw: fixed15(status, b"navy_experience_daily_training")?,
        air_raw: fixed15(status, b"air_experience")?,
        air_daily_raw: fixed15(status, b"air_experience_daily")?,
        armies_for_training_raw: fixed15(status, b"num_armies_for_training")?
            .ok_or_else(|| invalid("experience status lacks num_armies_for_training"))?,
    }))
}

fn pair(items: &[Item]) -> io::Result<(i32, i32)> {
    let [first, second] = items else {
        return Err(invalid(
            "supply node id must contain exactly two signed integers",
        ));
    };
    if first.key.is_some() || second.key.is_some() {
        return Err(invalid("supply node id contains a keyed value"));
    }
    Ok((
        integer(scalar(&first.value)?)?,
        integer(scalar(&second.value)?)?,
    ))
}

fn parse_supply(items: &[Item]) -> io::Result<Supply> {
    let mut supply = Supply::default();
    if let Some(value) = field(items, b"settings")? {
        for (node_index, node) in block(value)?.iter().enumerate() {
            if node.key.as_deref() != Some(b"node") {
                return Err(invalid("unexpected supply settings entry"));
            }
            let node = block(&node.value)?;
            let (province, node_type) = pair(required_block(node, b"id")?)?;
            let data = required_block(node, b"data")?;
            for (map_entry_index, setting) in required_block(data, b"motorization_level")?
                .iter()
                .enumerate()
            {
                let actor = setting
                    .key
                    .as_ref()
                    .filter(|key| !key.is_empty())
                    .ok_or_else(|| invalid("motorization setting lacks country tag"))?;
                supply.motorization.push(Motorization {
                    node_index: u32::try_from(node_index)
                        .map_err(|_| invalid("supply node index exceeds u32"))?,
                    map_entry_index: u32::try_from(map_entry_index)
                        .map_err(|_| invalid("motorization index exceeds u32"))?,
                    province,
                    node_type,
                    actor: actor.clone(),
                    level: integer(scalar(&setting.value)?)?,
                });
            }
        }
    }
    if let Some(value) = field(items, b"disrupted_supply")? {
        for node in block(value)? {
            if node.key.as_deref() != Some(b"node") {
                return Err(invalid("unexpected disrupted supply entry"));
            }
            let node = block(&node.value)?;
            let (province, node_type) = pair(required_block(node, b"id")?)?;
            supply.disrupted.push(Disrupted {
                province,
                node_type,
                value: required_decimal(node, b"value")?,
            });
        }
    }
    if let Some(value) = field(items, b"temp_nodes")? {
        for (ordinal, entry) in block(value)?.iter().enumerate() {
            if entry.key.is_some() {
                return Err(invalid("temporary supply node must be an anonymous block"));
            }
            let fields = block(&entry.value)?;
            supply.temp_nodes.push(TemporaryNode {
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| invalid("temporary supply node ordinal exceeds u32"))?,
                // The temporary-node writer always emits province/base, including zero/no.
                province: required_integer(fields, b"province")?,
                base: boolean(
                    field(fields, b"base")?
                        .ok_or_else(|| invalid("temporary supply node lacks base"))?,
                )?,
            });
        }
    }
    if let Some(value) = field(items, b"foreign_homebase_nodes")? {
        for (ordinal, entry) in block(value)?.iter().enumerate() {
            if entry.key.is_some() {
                return Err(invalid("foreign homebase entry must be an anonymous pair"));
            }
            let [province, value] = block(&entry.value)? else {
                return Err(invalid(
                    "foreign homebase entry must contain province and value",
                ));
            };
            if province.key.is_some() || value.key.is_some() {
                return Err(invalid("foreign homebase pair contains keyed entry"));
            }
            let fields = block(&value.value)?;
            supply.foreign_homebases.push(ForeignHomebase {
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| invalid("foreign homebase ordinal exceeds u32"))?,
                province: integer::<i32>(scalar(&province.value)?)? as u32,
                inner_province: required_integer::<i32>(fields, b"province")? as u32,
                supply: required_decimal(fields, b"supply")?,
                start: required_decimal(fields, b"start")?,
                penalty: required_decimal(fields, b"penalty")?,
                duration: required_decimal(fields, b"duration")?,
                hours: required_decimal(fields, b"hours")?,
                decay: required_decimal(fields, b"decay")?,
                base: boolean(
                    field(fields, b"base")?
                        .ok_or_else(|| invalid("foreign homebase lacks base"))?,
                )?,
                source: bytes(fields, b"add")?
                    .ok_or_else(|| invalid("text foreign homebase lacks add source field"))?,
            });
        }
    }
    Ok(supply)
}

fn parse_purchases(items: &[Item]) -> io::Result<Option<BTreeMap<u64, Purchase>>> {
    let Some(market) = field(items, b"equipment_market")? else {
        return Ok(None);
    };
    let mut purchases = BTreeMap::new();
    for item in required_block(block(market)?, b"contracts")? {
        if item.key.as_deref() != Some(b"purchase_contract") {
            return Err(invalid("unexpected purchase contract entry"));
        }
        let fields = block(&item.value)?;
        let id = typed_handle(
            field(fields, b"id")?.ok_or_else(|| invalid("purchase contract lacks id"))?,
        )?;
        let definition = required_block(fields, b"contract_definition")?;
        let draft = required_block(definition, b"contract_draft")?;
        let route = required_block(fields, b"delivery_route_handler")?;
        let delivery = required_block(fields, b"contract_delivery_state")?;
        let meta = required_block(fields, b"contract_meta")?;
        let purchase = Purchase {
            id,
            seller: required_bytes(draft, b"seller")?,
            buyer: required_bytes(draft, b"buyer")?,
            speed: required_integer(draft, b"speed")?,
            route_efficiency_sum: required_decimal(route, b"efficiency")?,
            route_days: required_integer(route, b"days")?,
            factory_cic_progress: required_decimal(delivery, b"factory_cic_progress")?,
            equipment_transfer_progress: required_decimal(
                delivery,
                b"equipment_transfer_progress",
            )?,
            collected: required_decimal(delivery, b"collected")?,
            completed_deliveries: required_integer(meta, b"completed")?,
        };
        if purchases.insert(id, purchase).is_some() {
            return Err(invalid("duplicate full purchase contract handle"));
        }
    }
    Ok(Some(purchases))
}

fn parse_pool(items: &[Item]) -> io::Result<ResourcePool> {
    items
        .iter()
        .map(|item| {
            let resource = item
                .key
                .as_ref()
                .filter(|key| !key.is_empty())
                .ok_or_else(|| invalid("saved resource pool entry lacks a resource"))?;
            Ok((resource.clone(), Decimal::parse(scalar(&item.value)?)?))
        })
        .collect()
}

fn parse_pools(resources: Option<&[Item]>) -> io::Result<Option<ResourcePools>> {
    let Some(resources) = resources else {
        return Ok(None);
    };
    let fields = resources;
    let pool = |key: &[u8]| {
        field(fields, key)?
            .ok_or_else(|| {
                invalid(format!(
                    "saved country resources lack {}",
                    String::from_utf8_lossy(key)
                ))
            })
            .and_then(|value| parse_pool(block(value)?))
    };
    let to_use = block(
        field(fields, b"to_use")?.ok_or_else(|| invalid("saved country resources lack to_use"))?,
    )?;
    let [first, second, third] = to_use else {
        return Err(invalid("saved to_use does not hold three pools"));
    };
    let mut parts = [first, second, third].into_iter().map(|item| {
        if item.key.is_some() {
            return Err(invalid("saved to_use pool is keyed"));
        }
        parse_pool(block(&item.value)?)
    });
    let mut next = || parts.next().unwrap();
    let to_use = [next()?, next()?, next()?];
    Ok(Some(ResourcePools {
        produced: pool(b"produced")?,
        transfer_overlord_subject: pool(b"transfer_overlord_subject")?,
        imported: pool(b"imported")?,
        to_use,
        to_export: pool(b"to_export")?,
        base_export: pool(b"base_export")?,
        exported: pool(b"exported")?,
    }))
}

fn parse_exports(resources: Option<&[Item]>) -> io::Result<Option<Vec<ResourceExchange>>> {
    let Some(resources) = resources else {
        return Ok(None);
    };
    let mut exports = Vec::new();
    for entry in resources
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"export"))
    {
        let fields = block(&entry.value)?;
        // The exchange writer saves these fields without scalar omission.
        exports.push(ResourceExchange {
            id: typed_handle(
                field(fields, b"id")?.ok_or_else(|| invalid("resource exchange lacks id"))?,
            )?,
            receiver: bytes(fields, b"receiver")?,
            resource: bytes(fields, b"resource")?,
            efficiency: required_decimal(fields, b"efficiency")?,
            efficiency_due_to_lost_convoys: required_decimal(
                fields,
                b"efficiency_due_to_lost_convoys",
            )?,
            delivered: required_decimal(fields, b"delivered")?,
            required_cic: required_integer(fields, b"required_cic")?,
            lended_cic: required_integer(fields, b"lended_cic")?,
            convoy_subscriber_present: field(fields, b"convoys_subscriber")?
                .map(block)
                .transpose()?
                .is_some(),
        });
    }
    Ok(Some(exports))
}

fn parse_lend_lease(resources: Option<&[Item]>) -> io::Result<Option<Vec<LendLease>>> {
    let Some(resources) = resources else {
        return Ok(None);
    };
    let mut exchanges = Vec::new();
    for entry in resources
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"lend_lease"))
    {
        let fields = block(&entry.value)?;
        let pool = |key: &[u8]| super::production::parse_pool(required_block(fields, key)?);
        exchanges.push(LendLease {
            id: typed_handle(
                field(fields, b"id")?.ok_or_else(|| invalid("lend-lease exchange lacks id"))?,
            )?,
            receiver: required_bytes(fields, b"receiver")?,
            delivered: pool(b"total_delivered")?,
            sunk: pool(b"equipment_sunk")?,
        });
    }
    Ok(Some(exchanges))
}

fn parse_supply_root(items: &[Item]) -> io::Result<Option<BTreeMap<Vec<u8>, Supply>>> {
    let Some(root) = field(items, b"supply_system_2")? else {
        return Ok(None);
    };
    let mut systems = BTreeMap::new();
    for entry in required_block(block(root)?, b"countries")? {
        let tag = entry
            .key
            .as_ref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("supply country lacks tag"))?;
        if systems
            .insert(tag.clone(), parse_supply(block(&entry.value)?)?)
            .is_some()
        {
            return Err(invalid("duplicate supply country in save"));
        }
    }
    Ok(Some(systems))
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let country_items = countries.required()?;
    let mut countries = BTreeMap::new();
    for item in country_items {
        let tag = item
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("save economy country lacks tag"))?;
        let fields = block(&item.value)?;
        let resources = field(fields, b"resources")?.map(block).transpose()?;
        let fuel = field(fields, b"fuel_status")?
            .map(|value| parse_fuel(block(value)?))
            .transpose()?;
        if countries
            .insert(
                tag.to_vec(),
                Country {
                    fuel,
                    experience: parse_experience(fields)?,
                    exports: parse_exports(resources)?,
                    lend_lease: parse_lend_lease(resources)?,
                    pools: parse_pools(resources)?,
                    logistics: field(fields, b"logistics")?
                        .map(|value| parse_logistics(block(value)?))
                        .transpose()?,
                },
            )
            .is_some()
        {
            return Err(invalid("duplicate economy country tag in save"));
        }
    }
    Ok(Pack {
        countries,
        supply: parse_supply_root(items)?,
        purchases: parse_purchases(items)?,
    })
}

#[cfg(test)]
mod tests;
