//! Literal economy and supply state carried by a text save.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, block, boolean, bytes, decimal, field, integer, invalid, required_block,
    required_bytes, scalar, typed_handle,
};
use crate::parser::Item;

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
}

/// Exact native signed /32768 values. 141294210 omits the first eight fields
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
    /// The enclosing country owns the export; receiver is the explicit +88.
    pub receiver: Option<Vec<u8>>,
    pub resource: Option<Vec<u8>>,
    pub efficiency: Decimal,
    pub efficiency_due_to_lost_convoys: Decimal,
    pub delivered: Decimal,
    pub required_cic: i32,
    pub lended_cic: i32,
    /// Base CAE9F0 emits this block precisely when native required != 0.
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
    /// Present system with omitted child collections is empty: 14121C6D0
    /// emits each of these three collections precisely when its count != 0.
    pub motorization: Vec<Motorization>,
    pub disrupted: Vec<Disrupted>,
    pub foreign_homebases: Vec<ForeignHomebase>,
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
    /// Native source string serialized as token 0x35EA (`add`). The text
    /// writer emits even an empty string, stopping at its first NUL byte.
    pub source: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fuel {
    /// Decimal spelling of a native /32768 quantity, not raw /100000 fuel.
    pub current: FuelQuantity,
    /// Decimal spelling of a native /32768 quantity, not raw /100000 fuel.
    pub maximum: FuelQuantity,
    /// Native signed /100000 modifier values.
    pub gain: Decimal,
    pub cost: Decimal,
}

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

    /// Native 1424AAC50 truncates abs(raw)*100000/32768 then emits its
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

fn parse_exports(items: &[Item]) -> io::Result<Option<Vec<ResourceExchange>>> {
    let Some(resources) = field(items, b"resources")? else {
        return Ok(None);
    };
    let mut exports = Vec::new();
    for entry in block(resources)?
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"export"))
    {
        let fields = block(&entry.value)?;
        // CAF770 -> CAE9F0 writes these fields without scalar omission.
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
    let mut countries = BTreeMap::new();
    for item in required_block(items, b"countries")? {
        let tag = item
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("save economy country lacks tag"))?;
        let fields = block(&item.value)?;
        let fuel = field(fields, b"fuel_status")?
            .map(|value| {
                // CCountryFuelStatus writer 1410E39B0 emits all four fields.
                let fields = block(value)?;
                Ok::<_, io::Error>(Fuel {
                    current: FuelQuantity::parse(&required_bytes(fields, b"fuel")?)?,
                    maximum: FuelQuantity::parse(&required_bytes(fields, b"max_fuel")?)?,
                    gain: required_decimal(fields, b"fuel_gain")?,
                    cost: required_decimal(fields, b"fuel_cost")?,
                })
            })
            .transpose()?;
        if countries
            .insert(
                tag.to_vec(),
                Country {
                    fuel,
                    experience: parse_experience(fields)?,
                    exports: parse_exports(fields)?,
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
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn fuel_preserves_dormant_literal_and_signed_modifier_precision() {
        let document = parser::parse(
            b"countries={GER={fuel_status={fuel=-1.00000 max_fuel=371500.00000 fuel_gain=-0.00001 fuel_cost=90071992547.40993}} D01={}}",
        )
        .unwrap();
        let pack = parse(&document.items).unwrap();
        let fuel = pack.countries[b"GER".as_slice()].fuel.as_ref().unwrap();
        assert_eq!(fuel.current.lexeme, b"-1.00000");
        assert_eq!(fuel.current.raw().unwrap(), -32_768);
        assert_eq!(fuel.gain.raw, -1);
        assert_eq!(fuel.cost.raw, 9_007_199_254_740_993);
        assert!(pack.countries[b"D01".as_slice()].fuel.is_none());
    }

    #[test]
    fn fuel_does_not_invent_omitted_or_rounded_values() {
        for input in [
            b"countries={GER={fuel_status={fuel=1 max_fuel=2 fuel_gain=3}}}".as_slice(),
            b"countries={GER={fuel_status={fuel=1 max_fuel=2 fuel_gain=0.000001 fuel_cost=0}}}",
            b"countries={GER={} GER={}}",
        ] {
            assert!(parse(&parser::parse(input).unwrap().items).is_err());
        }
    }

    #[test]
    fn fuel_text_preserves_full_quantity_range_without_native_bit_claim() {
        let quantity = FuelQuantity::parse(b"281474976710656.00000").unwrap();
        assert_eq!(quantity.decimal_numerator, 28_147_497_671_065_600_000);
        assert_eq!(
            FuelQuantity::parse(b"-0.00003").unwrap().decimal_numerator,
            -3
        );
        assert_eq!(FuelQuantity::parse(b"-0.00003").unwrap().raw().unwrap(), -1);
        assert!(quantity.raw().is_err());
        assert!(FuelQuantity::parse(b"0.00001").unwrap().raw().is_err());
        for raw in [i64::MIN + 1, -32_768, -1, 0, 1, 32_767, i64::MAX] {
            let numerator = i128::from(raw).abs() * 100_000 / 32_768;
            let literal = format!(
                "{}{}.{:05}",
                if raw < 0 { "-" } else { "" },
                numerator / 100_000,
                numerator % 100_000
            );
            assert_eq!(
                FuelQuantity::parse(literal.as_bytes())
                    .unwrap()
                    .raw()
                    .unwrap(),
                raw
            );
        }
        assert!(FuelQuantity::parse(b"1.000001").is_err());
        assert!(FuelQuantity::parse(b"1e2").is_err());
    }

    #[test]
    fn experience_keeps_all_nine_fixed15_channels_and_fractional_divisor() {
        let document = parser::parse(
            br#"countries={GER={experience_status={
            army_experience=281474976710655.99996
            army_experience_daily=-0.00003
            army_experience_daily_training=0.00006
            navy_experience=-281474976710655.99996
            navy_experience_daily=0.00009
            navy_experience_daily_training=-0.00009
            air_experience=0.00003 air_experience_daily=0.00000
            num_armies_for_training=32.70385
        }}}"#,
        )
        .unwrap();
        let pack = parse(&document.items).unwrap();
        let experience = pack.countries[b"GER".as_slice()]
            .experience
            .as_ref()
            .unwrap();
        assert_eq!(experience.army_raw, Some(i64::MAX));
        assert_eq!(experience.army_daily_raw, Some(-1));
        assert_eq!(experience.army_daily_training_raw, Some(2));
        assert_eq!(experience.navy_raw, Some(i64::MIN + 1));
        assert_eq!(experience.navy_daily_raw, Some(3));
        assert_eq!(experience.navy_daily_training_raw, Some(-3));
        assert_eq!(experience.air_raw, Some(1));
        assert_eq!(experience.air_daily_raw, Some(0));
        assert_eq!(experience.armies_for_training_raw, 1_071_640);
    }

    #[test]
    fn experience_keeps_zero_omissions_separate_from_missing_status() {
        let document = parser::parse(
            b"countries={GER={experience_status={num_armies_for_training=0.00000}} D01={}}",
        )
        .unwrap();
        let pack = parse(&document.items).unwrap();
        let experience = pack.countries[b"GER".as_slice()]
            .experience
            .as_ref()
            .unwrap();
        assert_eq!(
            [
                experience.army_raw,
                experience.army_daily_raw,
                experience.army_daily_training_raw,
                experience.navy_raw,
                experience.navy_daily_raw,
                experience.navy_daily_training_raw,
                experience.air_raw,
                experience.air_daily_raw,
            ],
            [None; 8]
        );
        assert_eq!(experience.armies_for_training_raw, 0);
        assert!(pack.countries[b"D01".as_slice()].experience.is_none());
        assert!(
            parse(
                &parser::parse(b"countries={GER={experience_status={}}}")
                    .unwrap()
                    .items
            )
            .is_err()
        );
    }

    #[test]
    fn experience_refuses_non_native_decimals_and_duplicate_channels() {
        for fields in [
            "num_armies_for_training=0 army_experience=0.00001",
            "num_armies_for_training=0.00001",
            "num_armies_for_training=0 army_experience_daily_training=1 navy_experience_daily=0.000001",
            "num_armies_for_training=0 air_experience=0 air_experience=0",
            "num_armies_for_training=0 navy_experience=--281474976710656.00000",
        ] {
            let input = format!("countries={{GER={{experience_status={{{fields}}}}}}}");
            assert!(
                parse(&parser::parse(input.as_bytes()).unwrap().items).is_err(),
                "{fields}"
            );
        }
    }

    #[test]
    fn supply_preserves_native_word_bits_order_alias_tags_and_zero_levels() {
        let document = parser::parse(
            br#"
            settings={
                node={id={-2147483648 -1} data={disabled=no motorization_level={GER=0 GER=255}}}
                node={id={1 2} data={disabled=yes motorization_level={}}}
                node={id={3 4} data={disabled=no motorization_level={D01=1}}}
            }
            disrupted_supply={node={id={-1 -2147483648} value=-90071992547.40993}}
        "#,
        )
        .unwrap();
        let supply = parse_supply(&document.items).unwrap();
        assert_eq!(supply.motorization.len(), 3);
        assert_eq!(supply.motorization[0].province, i32::MIN);
        assert_eq!(supply.motorization[0].level, 0);
        assert_eq!(supply.motorization[1].level, 255);
        assert_eq!(supply.motorization[2].node_index, 2);
        assert_eq!(supply.motorization[1].map_entry_index, 1);
        assert_eq!(supply.disrupted[0].province as u32, u32::MAX);
        assert_eq!(supply.disrupted[0].value.raw, -9_007_199_254_740_993);
        assert_eq!(parse_supply(&[]).unwrap(), Supply::default());
        let document = parser::parse(b"countries={} supply_system_2={countries={GER={}}}").unwrap();
        let pack = parse(&document.items).unwrap();
        assert!(
            pack.supply.unwrap()[b"GER".as_slice()]
                .motorization
                .is_empty()
        );
    }

    #[test]
    fn purchase_parser_preserves_full_typed_id_and_rejects_duplicate_identity() {
        let contract = br#"purchase_contract={
            id={id=4294967295 type=81}
            contract_definition={contract_draft={seller=GER buyer=D01 speed=-1}}
            delivery_route_handler={efficiency=-0.00001 days=-2147483648}
            contract_delivery_state={factory_cic_progress=90071992547.40993 equipment_transfer_progress=0 collected=-1}
            contract_meta={completed=3}
        }"#;
        let mut input = b"countries={} equipment_market={contracts={".to_vec();
        input.extend_from_slice(contract);
        input.extend_from_slice(b"}}");
        let parsed = parse(&parser::parse(&input).unwrap().items).unwrap();
        let purchases = parsed.purchases.unwrap();
        let id = (u64::from(u32::MAX) << 32) | 81;
        assert_eq!(purchases[&id].route_efficiency_sum.raw, -1);
        assert_eq!(purchases[&id].route_days, i32::MIN);
        assert_eq!(purchases[&id].completed_deliveries, 3);
        input.truncate(input.len() - 2);
        input.extend_from_slice(contract);
        input.extend_from_slice(b"}}");
        assert!(parse(&parser::parse(&input).unwrap().items).is_err());
    }

    #[test]
    fn foreign_homebases_keep_duplicate_provinces_and_inner_identity() {
        let row = br#"{7 {supply=-0.00001 start=1 penalty=2 add=" A\\B " duration=72 province=8 hours=0 decay=1 base=yes}}"#;
        let mut input = b"foreign_homebase_nodes={".to_vec();
        input.extend_from_slice(row);
        input.extend_from_slice(row);
        input.extend_from_slice(b"}");
        let supply = parse_supply(&parser::parse(&input).unwrap().items).unwrap();
        assert_eq!(supply.foreign_homebases.len(), 2);
        assert_eq!(supply.foreign_homebases[0].province, 7);
        assert_eq!(supply.foreign_homebases[0].inner_province, 8);
        assert_eq!(supply.foreign_homebases[0].source, b" A\\B ");
        assert_eq!(supply.foreign_homebases[0].supply.raw, -1);
        assert_eq!(supply.foreign_homebases[1].ordinal, 1);
    }

    #[test]
    fn foreign_homebase_add_is_literal_and_empty_is_distinct_from_missing() {
        let input = br#"foreign_homebase_nodes={
            {10007 {supply=0.32 start=3.3 penalty=0.84 add="Fleet Docking Rights Supply"
                duration=72 province=10007 hours=0 base=yes decay=1}}
            {7 {supply=0 start=0 penalty=0 add="" duration=0 province=7 hours=0 base=no decay=0}}
        }"#;
        let supply = parse_supply(&parser::parse(input).unwrap().items).unwrap();
        assert_eq!(
            supply.foreign_homebases[0].source,
            b"Fleet Docking Rights Supply"
        );
        assert!(supply.foreign_homebases[1].source.is_empty());
        for field in ["", "source=\"unsupported guessed key\""] {
            let input = format!(
                "foreign_homebase_nodes={{{{7 {{supply=0 start=0 penalty=0 {field} duration=0 province=7 hours=0 base=no decay=0}}}}}}"
            );
            assert!(parse_supply(&parser::parse(input.as_bytes()).unwrap().items).is_err());
        }
    }

    #[test]
    fn exchanges_preserve_full_ids_negative_deliveries_and_optional_presence() {
        let document = parser::parse(
            br#"countries={GER={resources={
            export={id={id=4294967295 type=4713} receiver=D01 resource=steel
                efficiency=1 efficiency_due_to_lost_convoys=-0.00001 delivered=-90071992547.40993
                required_cic=-1 lended_cic=2}
            export={id={id=1 type=4713} efficiency=0 efficiency_due_to_lost_convoys=0 delivered=0
                required_cic=0 lended_cic=0 convoys_subscriber={}}
        }} D01={}}"#,
        )
        .unwrap();
        let parsed = parse(&document.items).unwrap();
        let exports = parsed.countries[b"GER".as_slice()]
            .exports
            .as_ref()
            .unwrap();
        assert_eq!(exports[0].id, (u64::from(u32::MAX) << 32) | 4713);
        assert_eq!(exports[0].delivered.raw, -9_007_199_254_740_993);
        assert!(!exports[0].convoy_subscriber_present);
        assert!(exports[1].convoy_subscriber_present);
        assert!(exports[1].receiver.is_none());
        assert!(exports[1].resource.is_none());
        assert!(parsed.countries[b"D01".as_slice()].exports.is_none());
    }
}
