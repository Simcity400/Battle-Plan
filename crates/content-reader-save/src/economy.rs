
use std::{collections::BTreeMap, io};
pub mod market;

use super::{
    Decimal, block, boolean, bytes, decimal, field, integer, invalid, required_block,
    required_bytes, scalar, typed_handle,
};
use content_reader_parser::parser::{Item, Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub supply: Option<BTreeMap<Vec<u8>, Supply>>,
    pub purchases: Option<BTreeMap<u64, Purchase>>,
    pub market_terms: Option<Vec<market::Terms>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    pub market: Option<market::CountryMarket>,
    pub fuel: Option<Fuel>,
    pub experience: Option<Experience>,
    pub exports: Option<Vec<ResourceExchange>>,
    pub pools: Option<ResourcePools>,
    pub logistics: Option<Logistics>,
    pub lend_lease: Option<Vec<LendLease>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LendLease {
    pub terms: Option<market::LeaseTerms>,
    pub id: u64,
    pub receiver: Vec<u8>,
    pub efficiency: Decimal,
    pub efficiency_due_to_lost_convoys: Decimal,
    pub request: Option<i32>,
    pub convoys: ConvoyCounts,
    pub delivered: super::production::Pool,
    pub sunk: super::production::Pool,
}

pub type ResourcePool = Vec<(Vec<u8>, Decimal)>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourcePools {
    pub produced: ResourcePool,
    pub transfer_overlord_subject: ResourcePool,
    pub imported: ResourcePool,
    pub to_use: [ResourcePool; 3],
    pub to_export: ResourcePool,
    pub base_export: ResourcePool,
    pub exported: ResourcePool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Logistics {
    Empty,
    History(BTreeMap<u32, LoopHistory>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopHistory {
    pub windows: [LoopWindow; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopWindow {
    pub max_elements: i32,
    pub offset: i32,
    pub is_full: bool,
    pub data: Option<Vec<Decimal>>,
}

impl LoopWindow {
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
    pub armies_for_training_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceExchange {
    pub id: u64,
    pub receiver: Option<Vec<u8>>,
    pub resource: Option<Vec<u8>>,
    pub efficiency: Decimal,
    pub efficiency_due_to_lost_convoys: Decimal,
    pub request: Option<i32>,
    pub delivered: Decimal,
    pub required_cic: i32,
    pub lended_cic: i32,
    pub convoys: ConvoyCounts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConvoyCounts {
    pub assigned: i32,
    pub required: i32,
}

pub(super) fn convoy_counts(fields: &[Item]) -> io::Result<ConvoyCounts> {
    let Some(value) = field(fields, b"convoys_subscriber")? else {
        return Ok(ConvoyCounts::default());
    };
    let subscriber = block(value)?;
    Ok(ConvoyCounts {
        assigned: required_integer(subscriber, b"convoys")?,
        required: required_integer(subscriber, b"total")?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractClient {
    pub id: u64,
    pub efficiency: Decimal,
    pub efficiency_due_to_lost_convoys: Decimal,
    pub request: Option<i32>,
    pub convoys: ConvoyCounts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Purchase {
    pub id: u64,
    pub seller: Vec<u8>,
    pub buyer: Vec<u8>,
    pub speed: i32,
    pub client: ContractClient,
    pub route_efficiency_sum: Decimal,
    pub route_days: i32,
    pub factory_cic_progress: Decimal,
    pub equipment_transfer_progress: Decimal,
    pub collected: Decimal,
    pub completed_deliveries: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Supply {
    pub motorization: Vec<Motorization>,
    pub disrupted: Vec<Disrupted>,
    pub foreign_homebases: Vec<ForeignHomebase>,
    pub temp_nodes: Vec<TemporaryNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporaryNode {
    pub ordinal: u32,
    pub province: i32,
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
    pub province: i32,
    pub node_type: i32,
    pub value: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignHomebase {
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
    pub source: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fuel {
    pub current: FuelQuantity,
    pub maximum: FuelQuantity,
    pub gain: Decimal,
    pub cost: Decimal,
    pub gain_per_oil: Decimal,
    pub gain_from_states: Decimal,
    pub lend_lease_gain: FuelQuantity,
    pub lend_lease_consumption: FuelQuantity,
    pub consumers: Vec<FuelConsumer>,
    pub history_index: i32,
    pub remaining_hours: i32,
    pub history: Vec<FuelHistory>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuelConsumer {
    pub index: u32,
    pub priority: i32,
    pub received: Decimal,
    pub requested: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuelHistory {
    pub index: i32,
    pub fuel: FuelQuantity,
    pub consumed: FuelQuantity,
    pub gain: Decimal,
    pub other: FuelQuantity,
    pub requested: [Decimal; FUEL_CONSUMER_VALUES],
    pub received: [Decimal; FUEL_CONSUMER_VALUES],
}

pub const FUEL_HISTORY_RING: usize = 24;
pub const FUEL_CONSUMER_VALUES: usize = 3;

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

fn optional_integer<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Option<T>> {
    bytes(items, key)?.map(|value| integer(&value)).transpose()
}

fn fixed15(items: &[Item], key: &[u8]) -> io::Result<Option<i64>> {
    field(items, key)?
        .map(|value| FuelQuantity::parse(scalar(value)?)?.raw())
        .transpose()
}

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
        let client = required_block(route, b"convoy_client")?;
        let delivery = required_block(fields, b"contract_delivery_state")?;
        let meta = required_block(fields, b"contract_meta")?;
        let purchase = Purchase {
            id,
            seller: required_bytes(draft, b"seller")?,
            buyer: required_bytes(draft, b"buyer")?,
            speed: required_integer(draft, b"speed")?,
            client: ContractClient {
                id: typed_handle(
                    field(client, b"id")?
                        .ok_or_else(|| invalid("contract convoy client lacks id"))?,
                )?,
                efficiency: required_decimal(client, b"efficiency")?,
                efficiency_due_to_lost_convoys: required_decimal(
                    client,
                    b"efficiency_due_to_lost_convoys",
                )?,
                request: optional_integer(client, b"request")?,
                convoys: convoy_counts(client)?,
            },
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
            request: optional_integer(fields, b"request")?,
            delivered: required_decimal(fields, b"delivered")?,
            required_cic: required_integer(fields, b"required_cic")?,
            lended_cic: required_integer(fields, b"lended_cic")?,
            convoys: convoy_counts(fields)?,
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
            terms: market::lease(fields)?,
            id: typed_handle(
                field(fields, b"id")?.ok_or_else(|| invalid("lend-lease exchange lacks id"))?,
            )?,
            receiver: required_bytes(fields, b"receiver")?,
            efficiency: required_decimal(fields, b"efficiency")?,
            efficiency_due_to_lost_convoys: required_decimal(
                fields,
                b"efficiency_due_to_lost_convoys",
            )?,
            request: optional_integer(fields, b"request")?,
            convoys: convoy_counts(fields)?,
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
                    market: market::country(fields)?,
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
        market_terms: market::terms(items)?,
    })
}

#[cfg(test)]
mod tests;
