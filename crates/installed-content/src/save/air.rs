//! Exact native air save fields, including ordered retained history.
use std::io;

use super::{
    Decimal, block, boolean, bytes, decimal, field, integer, invalid, object_id, required_block,
    required_bytes, scalar, typed_handle,
};
use crate::parser::{Item, Value};

/// Native CLoopHistory metric indices; these are unrelated to region indices.
pub const STATISTIC_CHANNELS: [usize; 24] = [
    0, 1, 2, 3, 10, 17, 24, 36, 37, 38, 39, 40, 41, 42, 43, 44, 51, 58, 65, 77, 78, 79, 80, 81,
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// Vectors retain repeated parents; the comparator refuses ambiguous joins.
    pub bases: Vec<AirBase>,
    pub aces: Vec<Ace>,
    pub wings: Vec<Wing>,
    pub regions: Vec<Region>,
    pub statistics: Vec<Statistics>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirBase {
    pub id: u64,
    pub state: Option<u32>,
    /// C57070 writes the host only for stateless bases.
    pub host: Option<u64>,
    pub capacity: i32,
    pub kind: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ace {
    pub id: u64,
    pub country: Vec<u8>,
    pub ordinal: u32,
    pub modifier: Option<Vec<u8>>,
    pub alive: bool,
    pub handled: bool,
    pub kill_type: u32,
    pub killer_ace: u64,
    /// Omission proves a nonpositive country word, not its exact null bits.
    pub killer_country: Option<Vec<u8>>,
    pub name: Vec<u8>,
    pub surname: Vec<u8>,
    pub callsign: Vec<u8>,
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
    /// F539B0 writes primary+998 as `name` outside checksum mode.
    /// 1424ADF40 -> 1424AF5A0 escapes quotes/backslashes; the parser decodes them.
    /// Missing text remains distinct from a successfully saved empty name.
    pub name: Option<Vec<u8>>,
    /// F539B0 emits accidents only when signed-positive.
    pub accidents: Option<i32>,
    pub experience: Decimal,
    pub equipment: EquipmentPool,
    /// The wing writer emits this only when its full reference resolves.
    pub ace: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub country: Vec<u8>,
    pub region: u32,
    pub owner: Vec<u8>,
    /// First native vector followed by second; order within each is retained.
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
    /// Native record+90 is the copied equipment pool's allow-zero byte.
    pub equipment: EquipmentPool,
    /// Preserve both physical damage channels and their order, including duplicates.
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
    /// Only the current native 360-day annual-ring bucket's selected 24 fields.
    pub channels: [i64; 24],
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
    bytes(items, name)?.ok_or_else(|| invalid("missing serialized air string"))
}

/// Several native u32 bit fields use the signed integer text writer.
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
    })
}

fn parse_ace(country: &[u8], ordinal: u32, fields: &[Item]) -> io::Result<Ace> {
    // 60F1F0 omits alive=true, handled=false, kill_type=0, the null killer
    // reference, and an empty callsign. Name and surname are always emitted.
    Ok(Ace {
        id: object_id(fields)?,
        country: country.to_vec(),
        ordinal,
        modifier: bytes(fields, b"modifier")?,
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
        callsign: bytes(fields, b"callsign")?.unwrap_or_default(),
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

fn parse_wing(fields: &[Item]) -> io::Result<Wing> {
    Ok(Wing {
        id: object_id(fields)?,
        name: bytes(fields, b"name")?,
        accidents: field(fields, b"air_accidents")?
            .map(|v| integer(scalar(v)?))
            .transpose()?,
        experience: decimal(fields, b"experience")?
            .ok_or_else(|| invalid("air wing lacks experience"))?,
        equipment: parse_equipment(required_block(fields, b"equipment")?)?,
        ace: field(fields, b"ace")?.map(typed_handle).transpose()?,
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
    // 1505BB0 serializes all five 89-field rows in physical slot order if any
    // field is nonzero. Omitted data therefore proves zero, including at the
    // current slot. 24AE9A0 uses the same fixed5 formatter as scalar 24AE590.
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

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let mut pack = Pack::default();
    if let Some(countries) = field(items, b"countries")? {
        for country in block(countries)? {
            let tag = country
                .key
                .as_deref()
                .ok_or_else(|| invalid("air ace country lacks tag"))?;
            for (ordinal, ace) in repeated(block(&country.value)?, b"ace").enumerate() {
                pack.aces.push(parse_ace(
                    tag,
                    u32::try_from(ordinal).map_err(|_| invalid("air ace ordinal exceeds u32"))?,
                    block(ace)?,
                )?);
            }
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
            for wing in repeated(block(pool)?, b"air_wings") {
                pack.wings.push(parse_wing(block(wing)?)?);
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
            }
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn wing_name_preserves_exact_bytes_and_distinguishes_empty_from_absent() {
        let parse_name = |text: &[u8]| {
            let doc = parser::parse(text).unwrap();
            parse_wing(&doc.items).unwrap().name
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
        assert!(parse_wing(&doc.items).is_err());
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
    fn ace_omissions_keep_native_defaults_and_all_reference_bits() {
        let doc = parser::parse(br#"countries={GER={ace={id={id=4294967295 type=4294967295} name="" surname="S"} ace={id={id=1 type=4} name="N" surname="S" alive=no handled=yes kill_type=-1 killer_name={id=9 type=2147483648} killer_country="ENG"}}}"#).unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.aces[0].id, u64::MAX);
        assert!(pack.aces[0].alive);
        assert!(!pack.aces[0].handled);
        assert!(pack.aces[0].callsign.is_empty());
        assert_eq!(pack.aces[1].ordinal, 1);
        assert_eq!(pack.aces[1].kill_type, u32::MAX);
        assert_eq!(pack.aces[1].killer_ace, (9_u64 << 32) | (1 << 31));
    }

    #[test]
    fn duplicate_record_keys_and_equipment_are_preserved_in_native_order() {
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
}
