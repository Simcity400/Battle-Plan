use std::io;

use super::{
    Decimal, block, boolean, bytes, decimal, field, integer, invalid, object_id, required_block,
    required_bytes, scalar, typed_handle,
};
use content_reader_parser::parser::{Item, Value};

pub const STATISTIC_CHANNELS: [usize; 24] = [
    0, 1, 2, 3, 10, 17, 24, 36, 37, 38, 39, 40, 41, 42, 43, 44, 51, 58, 65, 77, 78, 79, 80, 81,
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub bases: Vec<AirBase>,
    pub aces: Vec<Ace>,
    pub wings: Vec<Wing>,
    pub regions: Vec<Region>,
    pub statistics: Vec<Statistics>,
    pub daily_statistics: Vec<DailyStatistics>,
    pub daily_errors: Vec<String>,
    pub ground_crews: Vec<GroundCrews>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundCrews {
    pub country: Vec<u8>,
    pub region: u32,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirBase {
    pub id: u64,
    pub state: Option<u32>,
    pub host: Option<u64>,
    pub capacity: i32,
    pub kind: i32,
    pub containers: Vec<AirBaseCountry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirBaseCountry {
    pub country: Vec<u8>,
    pub fuel_demand: Decimal,
    pub fuel_allocated: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ace {
    pub id: u64,
    pub country: Vec<u8>,
    pub ordinal: u32,
    pub modifier: Option<Vec<u8>>,
    pub female: bool,
    pub portrait: i32,
    pub alive: bool,
    pub handled: bool,
    pub kill_type: u32,
    pub killer_ace: u64,
    pub killer_country: Option<Vec<u8>>,
    pub name: Vec<u8>,
    pub surname: Vec<u8>,
    pub callsign: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub amount: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentPool {
    pub entries: Vec<Equipment>,
    pub allow_zero: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wing {
    pub id: u64,
    pub pool_type: Vec<u8>,
    pub name: Option<Vec<u8>>,
    pub accidents: Option<i32>,
    pub experience: Decimal,
    pub equipment: EquipmentPool,
    pub ace: Option<u64>,
    pub raid: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub country: Vec<u8>,
    pub region: u32,
    pub owner: Vec<u8>,
    pub entries: Vec<CombatRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombatRecord {
    pub side: u8,
    pub date: i32,
    pub wing_id: u32,
    pub classification: bool,
    pub planes: i32,
    pub mission_mask: u32,
    pub selection: bool,
    pub attributed_country: Vec<u8>,
    pub equipment: EquipmentPool,
    pub damage: [Vec<Damage>; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Damage {
    pub cause: u32,
    pub amount: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statistics {
    pub country: Vec<u8>,
    pub region: u32,
    pub year_slot: u32,
    pub year_window_wrapped: bool,
    pub channels: [i64; 24],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailyStatistics {
    pub country: Vec<u8>,
    pub region: u32,
    pub current_slot: u32,
    pub wrapped: bool,
    pub previous_fields: [i64; 89],
    pub current_fields: [i64; 89],
}

fn number<T: std::str::FromStr>(items: &[Item], name: &[u8]) -> io::Result<T> {
    integer(&required_bytes(items, name)?)
}

fn flag(items: &[Item], name: &[u8]) -> io::Result<bool> {
    boolean(field(items, name)?.ok_or_else(|| invalid("missing serialized air boolean"))?)
}

fn optional_flag(items: &[Item], name: &[u8], default: bool) -> io::Result<bool> {
    Ok(field(items, name)?
        .map(boolean)
        .transpose()?
        .unwrap_or(default))
}

fn exact_bytes(items: &[Item], name: &[u8]) -> io::Result<Vec<u8>> {
    bytes(items, name)?.ok_or_else(|| {
        invalid(format!(
            "missing serialized air string {}",
            String::from_utf8_lossy(name)
        ))
    })
}

fn word(items: &[Item], name: &[u8]) -> io::Result<u32> {
    let value: i64 = number(items, name)?;
    if !(i64::from(i32::MIN)..=i64::from(u32::MAX)).contains(&value) {
        return Err(invalid("air word exceeds native 32 bits"));
    }
    Ok(value as u32)
}

fn nullable_handle(value: &Value) -> io::Result<u64> {
    let fields = block(value)?;
    Ok((u64::from(number::<u32>(fields, b"id")?) << 32)
        | u64::from(number::<u32>(fields, b"type")?))
}

fn repeated<'a>(items: &'a [Item], name: &'a [u8]) -> impl Iterator<Item = &'a Value> {
    items
        .iter()
        .filter(move |item| item.key.as_deref() == Some(name))
        .map(|item| &item.value)
}

fn parse_base(fields: &[Item]) -> io::Result<AirBase> {
    let state = field(fields, b"state")?
        .map(|v| integer(scalar(v)?))
        .transpose()?;
    let host = field(fields, b"carrier")?
        .map(nullable_handle)
        .transpose()?;
    if state.is_some() == host.is_some() {
        return Err(invalid(
            "air base must serialize exactly one state or carrier anchor",
        ));
    }
    Ok(AirBase {
        id: object_id(fields)?,
        state,
        host,
        capacity: number(fields, b"capacity")?,
        kind: number(fields, b"base")?,
        containers: repeated(fields, b"countries")
            .map(|value| {
                let fields = block(value)?;
                let value = |key, default| -> io::Result<Decimal> {
                    field(fields, key)?
                        .map(|value| Decimal::parse(scalar(value)?))
                        .transpose()
                        .map(|value| {
                            value.unwrap_or(Decimal {
                                raw: default,
                                fractional_digits: 0,
                                lexeme: default.to_string().into_bytes(),
                            })
                        })
                };
                Ok(AirBaseCountry {
                    country: required_bytes(fields, b"country")?,
                    fuel_demand: value(b"fuel_consumption", 0)?,
                    fuel_allocated: value(b"received", 0)?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?,
    })
}

fn parse_ace(country: &[u8], ordinal: u32, fields: &[Item]) -> io::Result<Ace> {
    Ok(Ace {
        id: object_id(fields)?,
        country: country.to_vec(),
        ordinal,
        modifier: bytes(fields, b"modifier")?,
        female: optional_flag(fields, b"is_female", false)?,
        portrait: number(fields, b"portrait")?,
        alive: optional_flag(fields, b"alive", true)?,
        handled: optional_flag(fields, b"handled", false)?,
        kill_type: if field(fields, b"kill_type")?.is_some() {
            word(fields, b"kill_type")?
        } else {
            0
        },
        killer_ace: field(fields, b"killer_name")?
            .map(nullable_handle)
            .transpose()?
            .unwrap_or(0),
        killer_country: bytes(fields, b"killer_country")?,
        name: exact_bytes(fields, b"name")?,
        surname: exact_bytes(fields, b"surname")?,
        callsign: bytes(fields, b"callsign")?,
    })
}

fn parse_equipment(fields: &[Item]) -> io::Result<EquipmentPool> {
    let entries = repeated(fields, b"equipment")
        .map(|value| {
            let entry = block(value)?;
            Ok(Equipment {
                variant: typed_handle(
                    field(entry, b"id")?.ok_or_else(|| invalid("air equipment lacks typed id"))?,
                )?,
                amount: decimal(entry, b"amount")?
                    .ok_or_else(|| invalid("air equipment lacks amount"))?,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let allow_zero = flag(fields, b"allow_zero_entries")?;
    if !allow_zero && entries.iter().any(|entry| entry.amount.raw == 0) {
        return Err(invalid(
            "zero air equipment entry contradicts native save omission rule",
        ));
    }
    Ok(EquipmentPool {
        entries,
        allow_zero,
    })
}

fn parse_wing(pool_type: &[u8], fields: &[Item]) -> io::Result<Wing> {
    Ok(Wing {
        id: object_id(fields)?,
        pool_type: pool_type.to_vec(),
        name: bytes(fields, b"name")?,
        accidents: field(fields, b"air_accidents")?
            .map(|v| integer(scalar(v)?))
            .transpose()?,
        experience: decimal(fields, b"experience")?
            .ok_or_else(|| invalid("air wing lacks experience"))?,
        equipment: parse_equipment(required_block(fields, b"equipment")?)?,
        ace: field(fields, b"ace")?.map(typed_handle).transpose()?,
        raid: field(fields, b"raid_instance")?
            .map(typed_handle)
            .transpose()?,
    })
}

fn parse_record(side: u8, fields: &[Item]) -> io::Result<CombatRecord> {
    let mut damage = [Vec::new(), Vec::new()];
    for (channel, name) in [b"sender".as_slice(), b"receiver"].into_iter().enumerate() {
        for value in repeated(fields, name) {
            let entry = block(value)?;
            damage[channel].push(Damage {
                cause: word(entry, b"type")?,
                amount: decimal(entry, b"value")?
                    .ok_or_else(|| invalid("air damage lacks value"))?,
            });
        }
    }
    Ok(CombatRecord {
        side,
        date: number(fields, b"time")?,
        wing_id: number(fields, b"id")?,
        classification: flag(fields, b"ground_attack")?,
        planes: number(fields, b"count")?,
        mission_mask: word(fields, b"mission")?,
        selection: flag(fields, b"destination")?,
        attributed_country: exact_bytes(fields, b"tag")?,
        equipment: parse_equipment(required_block(fields, b"equipment")?)?,
        damage,
    })
}

fn parse_region(country: &[u8], region: u32, fields: &[Item]) -> io::Result<Region> {
    let mut entries = Vec::new();
    for (side, name) in [b"friend".as_slice(), b"enemy"].into_iter().enumerate() {
        for value in repeated(fields, name) {
            entries.push(parse_record(side as u8, block(value)?)?);
        }
    }
    Ok(Region {
        country: country.to_vec(),
        region,
        owner: exact_bytes(fields, b"tag")?,
        entries,
    })
}

fn parse_statistics(country: &[u8], region: u32, fields: &[Item]) -> io::Result<Statistics> {
    let annual = required_block(required_block(fields, b"history_queue")?, b"2")?;
    let capacity: u32 = number(annual, b"max_elements")?;
    let year_slot: u32 = number(annual, b"offset")?;
    if capacity != 5 || year_slot >= capacity {
        return Err(invalid(
            "air annual history has invalid native capacity or cursor",
        ));
    }
    let year_window_wrapped = flag(annual, b"is_full")?;
    let mut channels = [0; 24];
    if let Some(data) = field(annual, b"data")? {
        let data = block(data)?;
        if data.len() != 5 * 89 {
            return Err(invalid(
                "air annual history data does not contain five 89-field rows",
            ));
        }
        for (index, item) in data.iter().enumerate() {
            if item.key.is_some() {
                return Err(invalid("air history data must be an ordered scalar array"));
            }
            let value = Decimal::parse(scalar(&item.value)?)?;
            if index / 89 == year_slot as usize
                && let Some(channel) = STATISTIC_CHANNELS
                    .iter()
                    .position(|&channel| channel == index % 89)
            {
                channels[channel] = value.raw;
            }
        }
    }
    Ok(Statistics {
        country: country.to_vec(),
        region,
        year_slot,
        year_window_wrapped,
        channels,
    })
}

fn parse_daily_statistics(
    country: &[u8],
    region: u32,
    fields: &[Item],
) -> io::Result<Option<DailyStatistics>> {
    let queue = required_block(fields, b"history_queue")?;
    let Some(daily) = field(queue, b"0")? else {
        return Ok(None);
    };
    let daily = block(daily)?;
    let capacity: u32 = number(daily, b"max_elements")?;
    let current_slot: u32 = number(daily, b"offset")?;
    if capacity != 30 || current_slot >= capacity {
        return Err(invalid(
            "air daily history has invalid native capacity or cursor",
        ));
    }
    let wrapped = flag(daily, b"is_full")?;
    let previous_slot = (current_slot + capacity - 1) % capacity;
    let mut previous_fields = [0; 89];
    let mut current_fields = [0; 89];
    if let Some(data) = field(daily, b"data")? {
        let data = block(data)?;
        if data.len() != 30 * 89 {
            return Err(invalid(
                "air daily history data does not contain thirty 89-field rows",
            ));
        }
        for (index, item) in data.iter().enumerate() {
            if item.key.is_some() {
                return Err(invalid("air history data must be an ordered scalar array"));
            }
            let value = Decimal::parse(scalar(&item.value)?)?;
            if index / 89 == current_slot as usize {
                current_fields[index % 89] = value.raw;
            }
            if index / 89 == previous_slot as usize {
                previous_fields[index % 89] = value.raw;
            }
        }
    }
    Ok(Some(DailyStatistics {
        country: country.to_vec(),
        region,
        current_slot,
        wrapped,
        previous_fields,
        current_fields,
    }))
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut pack = Pack::default();
    for country in countries.iter() {
        let tag = country
            .key
            .as_deref()
            .ok_or_else(|| invalid("air ace country lacks tag"))?;
        for region in repeated(block(&country.value)?, b"strategic_region_data") {
            let fields = block(region)?;
            pack.ground_crews.push(GroundCrews {
                country: tag.to_vec(),
                region: number(fields, b"strategic_region")?,
                enabled: optional_flag(fields, b"more_ground_crews_enabled", false)?,
            });
        }
        for (ordinal, ace) in repeated(block(&country.value)?, b"ace").enumerate() {
            pack.aces.push(parse_ace(
                tag,
                u32::try_from(ordinal).map_err(|_| invalid("air ace ordinal exceeds u32"))?,
                block(ace)?,
            )?);
        }
    }
    let Some(air) = field(items, b"strategic_air")? else {
        return Ok(pack);
    };
    for item in block(air)? {
        let tag = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("strategic air entry lacks key"))?;
        let fields = block(&item.value)?;
        if matches!(tag, b"air_base" | b"rocket_site" | b"gun_emplacement") {
            pack.bases.push(parse_base(fields)?);
            continue;
        }
        for pool in repeated(fields, b"air_wing_pool") {
            let pool = block(pool)?;
            let pool_type = exact_bytes(pool, b"definition")?;
            for wing in repeated(pool, b"air_wings") {
                pack.wings.push(parse_wing(&pool_type, block(wing)?)?);
            }
        }
        if let Some(history) = field(fields, b"combat_history")? {
            for region in block(history)? {
                let id = integer(
                    region
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("air combat region lacks id"))?,
                )?;
                pack.regions
                    .push(parse_region(tag, id, block(&region.value)?)?);
            }
        }
        if let Some(history) = field(fields, b"history")? {
            for region in block(history)? {
                let id = integer(
                    region
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("air statistics region lacks id"))?,
                )?;
                pack.statistics
                    .push(parse_statistics(tag, id, block(&region.value)?)?);
                match parse_daily_statistics(tag, id, block(&region.value)?) {
                    Ok(Some(daily)) => pack.daily_statistics.push(daily),
                    Ok(None) => {}
                    Err(error) => pack.daily_errors.push(format!(
                        "{} region {id}: {error}",
                        String::from_utf8_lossy(tag)
                    )),
                }
            }
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    #[test]
    fn regional_ground_crews_follow_country_blocks_and_false_omission() {
        let doc = content_reader_parser::parser::parse(br#"countries={ AAA={ strategic_region_data={ strategic_region=7 more_ground_crews_enabled=yes } strategic_region_data={ strategic_region=8 } } BBB={} }"#).unwrap();
        let pack = super::parse(&doc.items).unwrap();
        assert_eq!(
            pack.ground_crews,
            [
                super::GroundCrews {
                    country: b"AAA".to_vec(),
                    region: 7,
                    enabled: true
                },
                super::GroundCrews {
                    country: b"AAA".to_vec(),
                    region: 8,
                    enabled: false
                }
            ]
        );
    }
    use super::*;
    use content_reader_parser::parser;

    #[test]
    fn state_container_fuel_preserves_order_repeats_zero_omission_and_exact_signed_words() {
        let doc = parser::parse(
            br#"strategic_air={ air_base={ id={id=7 type=65} state=2 capacity=400 base=0
            countries={country=AAA fuel_consumption=-0.00003 received=1.23456}
            countries={country=BBB}
            countries={country=AAA fuel_consumption=-0.00003 received=1.23456}
        }}"#,
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        let rows = &pack.bases[0].containers;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].country, b"AAA");
        assert_eq!(rows[0].fuel_demand.raw, -3);
        assert_eq!(rows[0].fuel_allocated.raw, 123456);
        assert_eq!(rows[1].fuel_demand.raw, 0);
        assert_eq!(rows[1].fuel_allocated.raw, 0);
        assert_eq!(rows[0], rows[2]);
        assert!(parse(&parser::parse(br#"strategic_air={air_base={id={id=7 type=65} state=2 countries={country=AAA received=1 received=2}}}"#).unwrap().items).is_err());
    }

    #[test]
    fn ace_callsign_preserves_omitted_and_emitted_empty_keys() {
        for (field, expected) in [
            ("", None),
            ("callsign=\"\"", Some(b"".as_slice())),
            ("callsign=Custom", Some(b"Custom".as_slice())),
        ] {
            let doc = parser::parse(
                format!("id={{id=1 type=8}} name=N surname=S portrait=0 {field}").as_bytes(),
            )
            .unwrap();
            let ace = parse_ace(b"AAA", 0, &doc.items).unwrap();
            assert_eq!(ace.callsign.as_deref(), expected);
        }
    }

    #[test]
    fn pool_definition_uses_saved_key_and_retains_wing_values() {
        let doc = parser::parse(
            br#"strategic_air={TST={
                air_wing_pool={
                    definition=fixture_aircraft
                    air_wings={
                        id={ id=17 type=23 }
                        experience=0.125
                        name="Synthetic \\wing"
                        equipment={
                            equipment={ id={ id=31 type=29 } amount=0 }
                            equipment={ id={ id=32 type=29 } amount=3.25 }
                            allow_zero_entries=yes
                        }
                    }
                }
            }}"#,
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        let [wing] = pack.wings.as_slice() else {
            panic!("expected one synthetic wing");
        };
        assert_eq!(wing.pool_type, b"fixture_aircraft");
        assert_eq!(wing.id, (17_u64 << 32) | 23);
        assert_eq!(wing.name.as_deref(), Some(b"Synthetic \\wing".as_slice()));
        assert_eq!(wing.experience.raw, 12_500);
        assert!(wing.equipment.allow_zero);
        assert_eq!(wing.equipment.entries.len(), 2);
        assert_eq!(wing.equipment.entries[0].variant, (31_u64 << 32) | 29);
        assert_eq!(wing.equipment.entries[0].amount.raw, 0);
        assert_eq!(wing.equipment.entries[1].variant, (32_u64 << 32) | 29);
        assert_eq!(wing.equipment.entries[1].amount.raw, 325_000);
    }

    #[test]
    fn wing_name_preserves_exact_bytes_and_distinguishes_empty_from_absent() {
        let parse_name = |text: &[u8]| {
            let doc = parser::parse(text).unwrap();
            parse_wing(b"fixture_plane", &doc.items).unwrap().name
        };
        let fields = b"id={id=1 type=69} experience=0 equipment={allow_zero_entries=no}";
        assert_eq!(parse_name(fields), None);
        let mut empty = fields.to_vec();
        empty.extend_from_slice(br#" name="""#);
        assert_eq!(parse_name(&empty), Some(Vec::new()));
        let mut bytes = fields.to_vec();
        bytes.extend_from_slice(b" name=\"A\x80\xffB\"");
        assert_eq!(parse_name(&bytes), Some(b"A\x80\xffB".to_vec()));
        let mut escaped = fields.to_vec();
        escaped.extend_from_slice(br#" name="User \"quoted\" \\wing""#);
        assert_eq!(
            parse_name(&escaped),
            Some(br#"User "quoted" \wing"#.to_vec())
        );
        let mut duplicate = fields.to_vec();
        duplicate.extend_from_slice(br#" name="A" name="B""#);
        let doc = parser::parse(&duplicate).unwrap();
        assert!(parse_wing(b"fixture_plane", &doc.items).is_err());
    }

    #[test]
    fn pool_type_and_live_raid_keep_exact_saved_bytes_and_both_handle_words() {
        let doc = parser::parse(
            br#"strategic_air={CUSTOM={
            air_wing_pool={definition=ModTransport air_wings={id={id=1 type=69}
                experience=0 equipment={allow_zero_entries=no}
                raid_instance={id=4294967295 type=2147483732}}
                air_wings={id={id=2 type=69} experience=0
                    equipment={allow_zero_entries=no}}}
            air_wing_pool={definition=none air_wings={id={id=3 type=69}
                experience=0 equipment={allow_zero_entries=no}}}
        }}"#,
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.wings[0].pool_type, b"ModTransport");
        assert_eq!(pack.wings[1].pool_type, b"ModTransport");
        assert_eq!(pack.wings[2].pool_type, b"none");
        assert_eq!(
            pack.wings[0].raid,
            Some((u64::from(u32::MAX) << 32) | 2_147_483_732)
        );
        assert_eq!(pack.wings[1].raid, None);
        assert_eq!(pack.wings[2].raid, None);
    }

    #[test]
    fn pool_type_presence_and_live_raid_corruption_are_never_defaulted() {
        let text = "strategic_air={CUSTOM={air_wing_pool={definition=ModTransport \
            air_wings={id={id=1 type=69} experience=0 \
            equipment={allow_zero_entries=no} raid_instance={id=1 type=84}}}}}";
        for malformed in [
            text.replace("definition=ModTransport", ""),
            text.replace("definition=ModTransport", "type=ModTransport"),
            text.replace(
                "definition=ModTransport",
                "definition=ModTransport definition=Other",
            ),
            text.replace("definition=ModTransport", "definition={}"),
            text.replace(
                "raid_instance={id=1 type=84}",
                "raid_instance={id=0 type=0}",
            ),
            text.replace("raid_instance={id=1 type=84}", "raid_instance={id=1}"),
            text.replace(
                "raid_instance={id=1 type=84}",
                "raid_instance={id=4294967296 type=84}",
            ),
            text.replace(
                "raid_instance={id=1 type=84}",
                "raid_instance={id=1 type=84} raid_instance={id=2 type=84}",
            ),
        ] {
            let doc = parser::parse(malformed.as_bytes()).unwrap();
            assert!(parse(&doc.items).is_err(), "{malformed}");
        }
        let missing =
            parser::parse(text.replace("definition=ModTransport", "").as_bytes()).unwrap();
        assert_eq!(
            parse(&missing.items).unwrap_err().to_string(),
            "missing serialized air string definition"
        );
    }

    #[test]
    fn base_anchors_preserve_null_host_and_signed_fields_without_inventing_location() {
        let fields =
            parser::parse(b"id={id=1 type=4294967295} carrier={id=0 type=0} capacity=-2 base=2")
                .unwrap();
        let base = parse_base(&fields.items).unwrap();
        assert_eq!(base.id, (1_u64 << 32) | u64::from(u32::MAX));
        assert_eq!(base.host, Some(0));
        assert_eq!(base.state, None);
        assert_eq!(base.capacity, -2);
        let fields =
            parser::parse(b"id={id=1 type=65} state=4 carrier={id=3 type=47} capacity=0 base=0")
                .unwrap();
        assert!(parse_base(&fields.items).is_err());
    }

    #[test]
    fn ace_omissions_keep_writer_defaults_and_all_reference_bits() {
        let doc = parser::parse(br#"countries={GER={ace={id={id=4294967295 type=4294967295} name="" surname="S" portrait=0} ace={id={id=1 type=4} name="N" surname="S" modifier="authored_type" is_female=yes portrait=-1 alive=no handled=yes kill_type=-1 killer_name={id=9 type=2147483648} killer_country="ENG"}}}"#).unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.aces[0].id, u64::MAX);
        assert!(pack.aces[0].alive);
        assert!(!pack.aces[0].handled);
        assert!(pack.aces[0].callsign.is_none());
        assert!(!pack.aces[0].female);
        assert_eq!(pack.aces[0].portrait, 0);
        assert_eq!(pack.aces[0].modifier, None);
        assert!(pack.aces[1].female);
        assert_eq!(pack.aces[1].portrait, -1);
        assert_eq!(
            pack.aces[1].modifier.as_deref(),
            Some(&b"authored_type"[..])
        );
        assert_eq!(pack.aces[1].ordinal, 1);
        assert_eq!(pack.aces[1].kill_type, u32::MAX);
        assert_eq!(pack.aces[1].killer_ace, (9_u64 << 32) | (1 << 31));
    }

    #[test]
    fn duplicate_record_keys_and_equipment_are_preserved_in_saved_order() {
        let fields = parser::parse(br#"tag="GER" enemy={id=0 equipment={allow_zero_entries=yes} time=-4 mission=-1 count=-2 tag="---" ground_attack=no destination=yes receiver={type=-1 value=-0.00001} receiver={type=-1 value=1}} friend={id=0 equipment={equipment={id={id=1 type=70} amount=0} equipment={id={id=1 type=70} amount=-2} allow_zero_entries=yes} time=-4 mission=0 count=0 tag="GER" ground_attack=no destination=no} friend={id=0 equipment={allow_zero_entries=no} time=-4 mission=0 count=0 tag="GER" ground_attack=no destination=no}"#).unwrap();
        let row = parse_region(b"GER", 23, &fields.items).unwrap();
        assert_eq!(
            row.entries.iter().map(|r| r.side).collect::<Vec<_>>(),
            [0, 0, 1]
        );
        assert_eq!(row.entries[0].equipment.entries[1].amount.raw, -200_000);
        assert_eq!(row.entries[2].mission_mask, u32::MAX);
        assert_eq!(row.entries[2].damage[1].len(), 2);
        assert_eq!(row.entries[2].damage[1][0].amount.raw, -1);
    }

    #[test]
    fn annual_cursor_selects_physical_slot_and_zero_omission_is_exact() {
        let empty =
            parser::parse(b"history_queue={2={max_elements=5 offset=4 is_full=yes}}").unwrap();
        let row = parse_statistics(b"GER", 0, &empty.items).unwrap();
        assert_eq!(row.channels, [0; 24]);
        assert_eq!(row.year_slot, 4);
        let mut values = vec!["0"; 445];
        values[89 + 81] = "-92233720368547.75808";
        values[81] = "7";
        let text = format!(
            "history_queue={{2={{max_elements=5 offset=1 is_full=no data={{ {} }} }} }}",
            values.join(" ")
        );
        let doc = parser::parse(text.as_bytes()).unwrap();
        let row = parse_statistics(b"GER", 23, &doc.items).unwrap();
        assert_eq!(row.channels[23], i64::MIN);
        let malformed =
            parser::parse(b"history_queue={2={max_elements=5 offset=5 is_full=no}}").unwrap();
        assert!(parse_statistics(b"GER", 23, &malformed.items).is_err());
    }

    #[test]
    fn daily_previous_slot_preserves_every_history_field_and_wraps_at_zero() {
        for current_slot in [0_usize, 1, 29] {
            let previous_slot = (current_slot + 29) % 30;
            let mut values = vec!["0".to_owned(); 30 * 89];
            for channel in 0..89 {
                values[previous_slot * 89 + channel] = format!("{}.{channel:05}", channel + 1);
            }
            values[previous_slot * 89 + 88] = "-92233720368547.75808".to_owned();
            values[current_slot * 89 + 30] = "9".to_owned();
            let text = format!(
                "history_queue={{0={{max_elements=30 offset={current_slot} is_full=yes data={{ {} }} }} }}",
                values.join(" ")
            );
            let doc = parser::parse(text.as_bytes()).unwrap();
            let row = parse_daily_statistics(b"mod country", 4001, &doc.items)
                .unwrap()
                .unwrap();
            assert_eq!(row.country, b"mod country");
            assert_eq!(row.region, 4001);
            assert_eq!(row.current_slot, current_slot as u32);
            assert!(row.wrapped);
            for (channel, value) in row.previous_fields.iter().enumerate().take(88) {
                assert_eq!(*value, (channel as i64 + 1) * 100_000 + channel as i64);
            }
            assert_eq!(row.previous_fields[88], i64::MIN);
            assert_eq!(row.current_fields[30], 900000);
        }
    }

    #[test]
    fn daily_zero_omission_is_exact_but_missing_daily_queue_stays_absent() {
        let doc =
            parser::parse(b"history_queue={0={max_elements=30 offset=1 is_full=no}}").unwrap();
        let row = parse_daily_statistics(b"mod country", 4001, &doc.items)
            .unwrap()
            .unwrap();
        assert_eq!(row.previous_fields, [0; 89]);
        assert_eq!(row.current_fields, [0; 89]);
        assert!(!row.wrapped);
        let doc =
            parser::parse(b"history_queue={2={max_elements=5 offset=4 is_full=yes}}").unwrap();
        assert!(
            parse_daily_statistics(b"mod country", 4001, &doc.items)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn daily_malformed_metadata_and_arrays_are_refused() {
        for fields in [
            "max_elements=29 offset=0 is_full=no",
            "max_elements=31 offset=0 is_full=no",
            "max_elements=30 offset=30 is_full=no",
            "max_elements=30 offset=-1 is_full=no",
            "max_elements=30 offset=1 is_full=maybe",
            "max_elements=30 offset=1",
            "max_elements=30 offset=1 is_full=no data={0}",
            "max_elements=30 offset=1 is_full=no data={}",
            "max_elements=30 offset=1 is_full=no offset=2",
        ] {
            let text = format!("history_queue={{0={{ {fields} }} }}");
            let doc = parser::parse(text.as_bytes()).unwrap();
            assert!(
                parse_daily_statistics(b"tag", 3, &doc.items).is_err(),
                "{fields}"
            );
        }
        for bad_value in ["x=0", "{}", "bad", "92233720368547.75808"] {
            let mut values = vec!["0"; 30 * 89];
            values[89 + 88] = bad_value;
            let text = format!(
                "history_queue={{0={{max_elements=30 offset=1 is_full=no data={{ {} }} }} }}",
                values.join(" ")
            );
            let doc = parser::parse(text.as_bytes()).unwrap();
            assert!(
                parse_daily_statistics(b"tag", 3, &doc.items).is_err(),
                "{bad_value}"
            );
        }
    }

    #[test]
    fn daily_repeated_country_region_coordinates_remain_visible_to_oracle() {
        let text = b"strategic_air={\"mod country\"={history={4001={history_queue={0={max_elements=30 offset=1 is_full=no} 2={max_elements=5 offset=1 is_full=no}}} 4001={history_queue={0={max_elements=30 offset=2 is_full=no} 2={max_elements=5 offset=1 is_full=no}}}}}}";
        let doc = parser::parse(text).unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.daily_statistics.len(), 2);
        assert_eq!(pack.statistics.len(), 2);
        assert_eq!(pack.daily_statistics[0].current_slot, 1);
        assert_eq!(pack.daily_statistics[1].current_slot, 2);
        assert_eq!(pack.daily_statistics[0].country, b"mod country");
        assert_eq!(pack.daily_statistics[0].region, 4001);
    }

    #[test]
    fn malformed_daily_queue_does_not_abort_other_air_fields() {
        let text = b"strategic_air={tag={history={3={history_queue={0={max_elements=30 offset=30 is_full=no} 2={max_elements=5 offset=1 is_full=no}}}}}}";
        let doc = parser::parse(text).unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.statistics.len(), 1);
        assert_eq!(pack.daily_errors.len(), 1);
        assert!(pack.daily_statistics.is_empty());
    }
}
