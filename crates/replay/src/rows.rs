//! Engine-native rows and the common field-mask codec used by every table.

use std::io;

use crate::codec::{Decoder, invalid, put_bytes, put_varint};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TableId {
    Clock = 0,
    Campaign = 1,
    Countries = 2,
    CountryLinks = 3,
    Provinces = 4,
    States = 5,
    Divisions = 6,
}

impl TableId {
    pub const ALL: [Self; 7] = [
        Self::Clock,
        Self::Campaign,
        Self::Countries,
        Self::CountryLinks,
        Self::Provinces,
        Self::States,
        Self::Divisions,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn field_count(self) -> usize {
        self.schema().len()
    }

    pub const fn mask_width(self) -> usize {
        self.field_count().div_ceil(8)
    }

    pub const fn full_mask(self) -> u64 {
        (1_u64 << self.field_count()) - 1
    }

    pub(crate) fn minimum_row_bytes(self) -> usize {
        1 + self
            .schema()
            .iter()
            .map(|kind| kind.minimum_row_bytes())
            .sum::<usize>()
    }

    pub(crate) fn minimum_delta_bytes(self) -> usize {
        1 + self.mask_width()
            + self
                .schema()
                .iter()
                .map(|kind| kind.minimum_change_bytes())
                .min()
                .unwrap_or(0)
    }

    const fn schema(self) -> &'static [Kind] {
        use Kind::*;
        match self {
            Self::Clock => &[I32, U64, I64, I32],
            Self::Campaign => &[Bytes],
            Self::Countries => &[U32, Bytes, Bytes, OptionalI64, I64, I64, I64, OptionalI64],
            Self::CountryLinks => &[U32],
            Self::Provinces => &[U32, U32, U32],
            Self::States => &[U32, U32],
            Self::Divisions => &[U32, U32, U32, U32, U64, I64, I64],
        }
    }

    pub(crate) fn check_key(self, key: u64) -> io::Result<()> {
        if self != Self::Divisions && u32::try_from(key).is_err() {
            return Err(invalid(format!("{self:?} row key exceeds u32")));
        }
        if matches!(self, Self::Clock | Self::Campaign) && key != 0 {
            return Err(invalid("singleton row key must be zero"));
        }
        Ok(())
    }

    pub(crate) fn encode_key(
        self,
        key: u64,
        previous: u64,
        output: &mut Vec<u8>,
    ) -> io::Result<()> {
        self.check_key(key)?;
        let difference = key
            .checked_sub(previous)
            .ok_or_else(|| invalid("row keys decreased"))?;
        put_varint(output, u128::from(difference));
        Ok(())
    }

    pub(crate) fn decode_key(self, previous: u64, decoder: &mut Decoder<'_>) -> io::Result<u64> {
        let difference = u64::try_from(decoder.varint()?)
            .map_err(|_| invalid("row key difference exceeds u64"))?;
        let key = previous
            .checked_add(difference)
            .ok_or_else(|| invalid("row key addition overflow"))?;
        self.check_key(key)?;
        Ok(key)
    }

    fn check_mask(self, mask: u64) -> io::Result<()> {
        if mask == 0 || mask & !self.full_mask() != 0 {
            return Err(invalid("delta field mask is empty or has unknown bits"));
        }
        Ok(())
    }
}

impl TryFrom<u8> for TableId {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Self::ALL
            .get(usize::from(value))
            .copied()
            .ok_or_else(|| invalid("unknown table id"))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClockRow {
    pub hour: i32,
    pub play_time: u64,
    pub play_session_started_at: i64,
    pub seed: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CampaignRow {
    pub id: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryRow {
    /// Raw CCountry link id, which need not equal its canonical array slot.
    pub id: u32,
    pub canonical_slot: u32,
    pub tag: Vec<u8>,
    pub name: Vec<u8>,
    pub political_power_raw: Option<i64>,
    pub command_power_raw: i64,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub convoys_raw: Option<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryLinkRow {
    /// Raw country reference used by provinces and other engine tables.
    pub id: u32,
    pub canonical_slot: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceRow {
    pub id: u32,
    pub state: u32,
    /// Raw country link id; zero is retained as read from the engine.
    pub owner: u32,
    /// Raw country link id; resolve through CountryLinks to a canonical slot.
    pub controller: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateRow {
    pub id: u32,
    /// Raw country link id; zero is retained as read from the engine.
    pub owner: u32,
    /// Raw country link id; resolve through CountryLinks to a canonical slot.
    pub controller: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionRow {
    /// Full native handle: type in the low 32 bits, id in the high 32 bits.
    pub id: u64,
    /// Raw country reference of the roster containing this division.
    pub roster_owner: u32,
    /// Raw CUnit owner reference.
    pub logical_owner: u32,
    /// Raw expeditionary-owner reference, including the engine's null value.
    pub expeditionary_owner: u32,
    pub province: u32,
    /// Full native standard-template handle, with the same type/id packing as id.
    pub template: u64,
    /// Native signed fixed-point hit points, with denominator 100,000.
    pub hit_points: i64,
    /// Native signed fixed-point organisation, with denominator 100,000.
    pub organisation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Clock(ClockRow),
    Campaign(CampaignRow),
    Country(CountryRow),
    CountryLink(CountryLinkRow),
    Province(ProvinceRow),
    State(StateRow),
    Division(DivisionRow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaRow {
    pub key: u64,
    pub mask: u64,
    /// Selected fields in schema order: exact numeric differences as zigzag varints,
    /// with byte fields and optional presence markers retained. Inserts use zero.
    pub values: Vec<u8>,
}

impl Row {
    pub const fn table(&self) -> TableId {
        match self {
            Self::Clock(_) => TableId::Clock,
            Self::Campaign(_) => TableId::Campaign,
            Self::Country(_) => TableId::Countries,
            Self::CountryLink(_) => TableId::CountryLinks,
            Self::Province(_) => TableId::Provinces,
            Self::State(_) => TableId::States,
            Self::Division(_) => TableId::Divisions,
        }
    }

    pub const fn key(&self) -> u64 {
        match self {
            Self::Clock(_) | Self::Campaign(_) => 0,
            Self::Country(row) => row.id as u64,
            Self::CountryLink(row) => row.id as u64,
            Self::Province(row) => row.id as u64,
            Self::State(row) => row.id as u64,
            Self::Division(row) => row.id,
        }
    }

    pub fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> {
        if let Some(previous) = previous {
            self.check_previous(previous)?;
            if self == previous {
                return Ok(None);
            }
        }
        let fields = self.fields();
        let old_fields = previous.map(Self::fields);
        let mut delta = DeltaRow {
            key: self.key(),
            mask: 0,
            values: Vec::new(),
        };
        for (index, field) in fields[..self.table().field_count()].iter().enumerate() {
            if old_fields.as_ref().is_none_or(|old| old[index] != *field) {
                delta.mask |= 1_u64 << index;
                field
                    .change(old_fields.as_ref().map_or(Field::Empty, |old| old[index]))?
                    .encode(&mut delta.values)?;
            }
        }
        Ok(Some(delta))
    }

    fn check_previous(&self, previous: &Self) -> io::Result<()> {
        if self.table() != previous.table() || self.key() != previous.key() {
            return Err(invalid(
                "delta row and baseline have different tables or keys",
            ));
        }
        Ok(())
    }

    fn fields(&self) -> [Field<'_>; MAX_FIELDS] {
        use Field::*;
        match self {
            Self::Clock(row) => padded([
                I32(row.hour),
                U64(row.play_time),
                I64(row.play_session_started_at),
                I32(row.seed),
            ]),
            Self::Campaign(row) => padded([Bytes(&row.id)]),
            Self::Country(row) => padded([
                U32(row.canonical_slot),
                Bytes(&row.tag),
                Bytes(&row.name),
                OptionalI64(row.political_power_raw),
                I64(row.command_power_raw),
                I64(row.stability_raw),
                I64(row.war_support_raw),
                OptionalI64(row.convoys_raw),
            ]),
            Self::CountryLink(row) => padded([U32(row.canonical_slot)]),
            Self::Province(row) => padded([U32(row.state), U32(row.owner), U32(row.controller)]),
            Self::State(row) => padded([U32(row.owner), U32(row.controller)]),
            Self::Division(row) => padded([
                U32(row.roster_owner),
                U32(row.logical_owner),
                U32(row.expeditionary_owner),
                U32(row.province),
                U64(row.template),
                I64(row.hit_points),
                I64(row.organisation),
            ]),
        }
    }

    fn from_fields(table: TableId, key: u64, fields: [Field<'_>; MAX_FIELDS]) -> io::Result<Self> {
        use Field::*;
        // Narrow table ids may be cast only after this check; division handles stay u64.
        table.check_key(key)?;
        Ok(match (table, fields) {
            (
                TableId::Clock,
                [
                    I32(hour),
                    U64(play_time),
                    I64(play_session_started_at),
                    I32(seed),
                    ..,
                ],
            ) => Self::Clock(ClockRow {
                hour,
                play_time,
                play_session_started_at,
                seed,
            }),
            (TableId::Campaign, [Bytes(id), ..]) => Self::Campaign(CampaignRow { id: id.to_vec() }),
            (
                TableId::Countries,
                [
                    U32(canonical_slot),
                    Bytes(tag),
                    Bytes(name),
                    OptionalI64(political_power_raw),
                    I64(command_power_raw),
                    I64(stability_raw),
                    I64(war_support_raw),
                    OptionalI64(convoys_raw),
                ],
            ) => Self::Country(CountryRow {
                id: key as u32,
                canonical_slot,
                tag: tag.to_vec(),
                name: name.to_vec(),
                political_power_raw,
                command_power_raw,
                stability_raw,
                war_support_raw,
                convoys_raw,
            }),
            (TableId::CountryLinks, [U32(canonical_slot), ..]) => {
                Self::CountryLink(CountryLinkRow {
                    id: key as u32,
                    canonical_slot,
                })
            }
            (TableId::Provinces, [U32(state), U32(owner), U32(controller), ..]) => {
                Self::Province(ProvinceRow {
                    id: key as u32,
                    state,
                    owner,
                    controller,
                })
            }
            (TableId::States, [U32(owner), U32(controller), ..]) => Self::State(StateRow {
                id: key as u32,
                owner,
                controller,
            }),
            (
                TableId::Divisions,
                [
                    U32(roster_owner),
                    U32(logical_owner),
                    U32(expeditionary_owner),
                    U32(province),
                    U64(template),
                    I64(hit_points),
                    I64(organisation),
                    ..,
                ],
            ) => Self::Division(DivisionRow {
                id: key,
                roster_owner,
                logical_owner,
                expeditionary_owner,
                province,
                template,
                hit_points,
                organisation,
            }),
            _ => return Err(invalid("row fields do not match table schema")),
        })
    }
}

impl DeltaRow {
    pub fn apply(&self, table: TableId, previous: Option<&Row>) -> io::Result<Row> {
        table.check_key(self.key)?;
        table.check_mask(self.mask)?;
        if let Some(previous) = previous {
            if previous.table() != table || previous.key() != self.key {
                return Err(invalid(
                    "delta row and baseline have different tables or keys",
                ));
            }
        } else if self.mask != table.full_mask() {
            return Err(invalid("inserted row requires every field"));
        }
        let mut fields = previous.map_or([Field::Empty; MAX_FIELDS], Row::fields);
        let mut decoder = Decoder::new(&self.values);
        for (index, kind) in table.schema().iter().enumerate() {
            if self.mask & (1_u64 << index) != 0 {
                fields[index] = kind
                    .decode_change(&mut decoder)?
                    .apply(*kind, fields[index])?;
            }
        }
        decoder.finish()?;
        Row::from_fields(table, self.key, fields)
    }
}

pub fn encode_row(
    table: TableId,
    row: &Row,
    previous_key: u64,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    if row.table() != table {
        return Err(invalid("row belongs to a different table"));
    }
    table.encode_key(row.key(), previous_key, output)?;
    for field in &row.fields()[..table.field_count()] {
        field.encode(output)?;
    }
    Ok(())
}

pub fn decode_row(table: TableId, previous_key: u64, decoder: &mut Decoder<'_>) -> io::Result<Row> {
    let key = table.decode_key(previous_key, decoder)?;
    let mut fields = [Field::Empty; MAX_FIELDS];
    for (index, kind) in table.schema().iter().enumerate() {
        fields[index] = kind.decode(decoder)?;
    }
    Row::from_fields(table, key, fields)
}

pub fn encode_delta(
    table: TableId,
    delta: &DeltaRow,
    previous_key: u64,
    output: &mut Vec<u8>,
) -> io::Result<()> {
    table.check_key(delta.key)?;
    table.check_mask(delta.mask)?;
    let mut decoder = Decoder::new(&delta.values);
    for (index, kind) in table.schema().iter().enumerate() {
        if delta.mask & (1_u64 << index) != 0 {
            kind.decode_change(&mut decoder)?;
        }
    }
    decoder.finish()?;
    table.encode_key(delta.key, previous_key, output)?;
    output.extend_from_slice(&delta.mask.to_le_bytes()[..table.mask_width()]);
    output.extend_from_slice(&delta.values);
    Ok(())
}

pub fn decode_delta(
    table: TableId,
    previous_key: u64,
    decoder: &mut Decoder<'_>,
) -> io::Result<DeltaRow> {
    let key = table.decode_key(previous_key, decoder)?;
    let mut mask_bytes = [0; 8];
    mask_bytes[..table.mask_width()].copy_from_slice(decoder.take(table.mask_width())?);
    let mask = u64::from_le_bytes(mask_bytes);
    table.check_mask(mask)?;
    let mut values = Vec::new();
    for (index, kind) in table.schema().iter().enumerate() {
        if mask & (1_u64 << index) != 0 {
            kind.decode_change(decoder)?.encode(&mut values)?;
        }
    }
    Ok(DeltaRow { key, mask, values })
}

const MAX_FIELDS: usize = 8;

fn padded<const N: usize>(fields: [Field<'_>; N]) -> [Field<'_>; MAX_FIELDS] {
    let mut result = [Field::Empty; MAX_FIELDS];
    result[..N].copy_from_slice(&fields);
    result
}

#[derive(Clone, Copy)]
enum Kind {
    U32,
    I32,
    U64,
    I64,
    Bytes,
    OptionalI64,
}

impl Kind {
    fn minimum_row_bytes(self) -> usize {
        match self {
            Self::U32 | Self::I32 | Self::Bytes => 4,
            Self::U64 | Self::I64 => 8,
            Self::OptionalI64 => 1,
        }
    }

    fn minimum_change_bytes(self) -> usize {
        match self {
            Self::Bytes => 4,
            _ => 1,
        }
    }

    fn decode_change<'a>(self, decoder: &mut Decoder<'a>) -> io::Result<Change<'a>> {
        if matches!(self, Self::Bytes) {
            let Field::Bytes(bytes) = self.decode(decoder)? else {
                unreachable!()
            };
            return Ok(Change::Bytes(bytes));
        }
        if matches!(self, Self::OptionalI64) {
            match decoder.u8()? {
                0 => return Ok(Change::Optional(None)),
                1 => {}
                _ => return Err(invalid("invalid optional integer tag")),
            }
        }
        let encoded = decoder.varint()?;
        let difference = ((encoded >> 1) as i128) ^ -((encoded & 1) as i128);
        let maximum = if matches!(self, Self::U32 | Self::I32) {
            u128::from(u32::MAX)
        } else {
            u128::from(u64::MAX)
        };
        if difference.unsigned_abs() > maximum {
            return Err(invalid("numeric difference exceeds native field range"));
        }
        Ok(if matches!(self, Self::OptionalI64) {
            Change::Optional(Some(difference))
        } else {
            Change::Number(difference)
        })
    }

    fn number(self, value: i128) -> io::Result<Field<'static>> {
        let range = |_| invalid("numeric delta exceeds native field range");
        Ok(match self {
            Self::U32 => Field::U32(u32::try_from(value).map_err(range)?),
            Self::I32 => Field::I32(i32::try_from(value).map_err(range)?),
            Self::U64 => Field::U64(u64::try_from(value).map_err(range)?),
            Self::I64 => Field::I64(i64::try_from(value).map_err(range)?),
            Self::OptionalI64 => Field::OptionalI64(Some(i64::try_from(value).map_err(range)?)),
            Self::Bytes => return Err(invalid("numeric change applied to byte field")),
        })
    }

    fn decode<'a>(self, decoder: &mut Decoder<'a>) -> io::Result<Field<'a>> {
        Ok(match self {
            Self::U32 => Field::U32(decoder.u32()?),
            Self::I32 => Field::I32(decoder.i32()?),
            Self::U64 => Field::U64(decoder.u64()?),
            Self::I64 => Field::I64(decoder.i64()?),
            Self::Bytes => {
                let length = usize::try_from(decoder.u32()?)
                    .map_err(|_| invalid("row byte length exceeds address space"))?;
                Field::Bytes(decoder.take(length)?)
            }
            Self::OptionalI64 => Field::OptionalI64(match decoder.u8()? {
                0 => None,
                1 => Some(decoder.i64()?),
                _ => return Err(invalid("invalid optional integer tag")),
            }),
        })
    }
}

enum Change<'a> {
    Number(i128),
    Optional(Option<i128>),
    Bytes(&'a [u8]),
}

impl<'a> Change<'a> {
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let difference = match self {
            Self::Number(value) => *value,
            Self::Optional(Some(value)) => {
                output.push(1);
                *value
            }
            Self::Optional(None) => {
                output.push(0);
                return Ok(());
            }
            Self::Bytes(bytes) => return put_bytes(output, bytes),
        };
        let zigzag = ((difference as u128) << 1) ^ ((difference >> 127) as u128);
        put_varint(output, zigzag);
        Ok(())
    }

    fn apply(self, kind: Kind, previous: Field<'a>) -> io::Result<Field<'a>> {
        match self {
            Self::Bytes(bytes) => Ok(Field::Bytes(bytes)),
            Self::Optional(None) => Ok(Field::OptionalI64(None)),
            Self::Number(difference) | Self::Optional(Some(difference)) => {
                let value = previous
                    .number()?
                    .checked_add(difference)
                    .ok_or_else(|| invalid("numeric delta addition overflow"))?;
                kind.number(value)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field<'a> {
    Empty,
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bytes(&'a [u8]),
    OptionalI64(Option<i64>),
}

impl<'a> Field<'a> {
    fn number(self) -> io::Result<i128> {
        Ok(match self {
            Self::Empty | Self::OptionalI64(None) => 0,
            Self::U32(value) => i128::from(value),
            Self::I32(value) => i128::from(value),
            Self::U64(value) => i128::from(value),
            Self::I64(value) | Self::OptionalI64(Some(value)) => i128::from(value),
            Self::Bytes(_) => return Err(invalid("byte field used as numeric baseline")),
        })
    }

    fn change(self, previous: Self) -> io::Result<Change<'a>> {
        if let Self::Bytes(bytes) = self {
            return Ok(Change::Bytes(bytes));
        }
        if self == Self::OptionalI64(None) {
            return Ok(Change::Optional(None));
        }
        let difference = self
            .number()?
            .checked_sub(previous.number()?)
            .ok_or_else(|| invalid("numeric delta subtraction overflow"))?;
        Ok(if matches!(self, Self::OptionalI64(_)) {
            Change::Optional(Some(difference))
        } else {
            Change::Number(difference)
        })
    }

    fn encode(self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Empty => return Err(invalid("missing row field")),
            Self::U32(value) => output.extend_from_slice(&value.to_le_bytes()),
            Self::I32(value) => output.extend_from_slice(&value.to_le_bytes()),
            Self::U64(value) => output.extend_from_slice(&value.to_le_bytes()),
            Self::I64(value) => output.extend_from_slice(&value.to_le_bytes()),
            Self::Bytes(value) => put_bytes(output, value)?,
            Self::OptionalI64(value) => match value {
                None => output.push(0),
                Some(value) => {
                    output.push(1);
                    output.extend_from_slice(&value.to_le_bytes());
                }
            },
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn country() -> CountryRow {
        CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            name: vec![b'G', 0xff, b'r'],
            political_power_raw: Some(i64::MIN),
            command_power_raw: -9_007_199_254_740_993,
            stability_raw: 100_001,
            war_support_raw: 99_999,
            convoys_raw: Some(i64::MAX),
        }
    }

    fn division() -> DivisionRow {
        DivisionRow {
            id: 0x8000_0001_0000_0029,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 123,
            template: 0x8000_0002_0000_002a,
            hit_points: -9_007_199_254_740_993,
            organisation: 100_001,
        }
    }

    #[test]
    fn full_rows_and_insertions_preserve_native_values_and_raw_aliases() {
        let rows = [
            Row::Clock(ClockRow {
                hour: i32::MIN,
                play_time: u64::MAX,
                play_session_started_at: i64::MIN,
                seed: i32::MAX,
            }),
            Row::Campaign(CampaignRow {
                id: vec![0xff, 0, b'a'],
            }),
            Row::Country(country()),
            Row::CountryLink(CountryLinkRow {
                id: 2,
                canonical_slot: 1,
            }),
            Row::Province(ProvinceRow {
                id: 123,
                state: 456,
                owner: 7,
                controller: 2,
            }),
            Row::State(StateRow {
                id: u32::MAX,
                owner: 0,
                controller: u32::MAX,
            }),
            Row::Division(DivisionRow {
                id: u64::MAX,
                roster_owner: u32::MAX,
                logical_owner: 0,
                expeditionary_owner: u32::MAX,
                province: u32::MAX,
                template: u64::MAX,
                hit_points: i64::MIN,
                organisation: i64::MAX,
            }),
        ];
        for row in rows {
            let table = row.table();
            let mut bytes = Vec::new();
            encode_row(table, &row, 0, &mut bytes).unwrap();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(decode_row(table, 0, &mut decoder).unwrap(), row);
            decoder.finish().unwrap();

            let delta = row.delta(None).unwrap().unwrap();
            assert_eq!(delta.mask, table.full_mask());
            assert_eq!(delta.apply(table, None).unwrap(), row);
            bytes.clear();
            encode_delta(table, &delta, 0, &mut bytes).unwrap();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(decode_delta(table, 0, &mut decoder).unwrap(), delta);
            decoder.finish().unwrap();
        }
    }

    #[test]
    fn one_native_scalar_tick_costs_three_bytes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        let old = Row::Country(old);
        let new = Row::Country(new);
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 5);
        assert_eq!(delta.values, [2]);
        let mut bytes = Vec::new();
        encode_delta(TableId::Countries, &delta, 0, &mut bytes).unwrap();
        assert_eq!(bytes, [7, 0x20, 2]);
        assert_eq!(delta.apply(TableId::Countries, Some(&old)).unwrap(), new);
    }

    #[test]
    fn state_masks_keep_owner_and_controller_as_independent_raw_references() {
        let old = Row::State(StateRow {
            id: 0x1234_5678,
            owner: 7,
            controller: 2,
        });
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        for mask in 1..=TableId::States.full_mask() {
            let new = Row::State(StateRow {
                id: old.key().try_into().unwrap(),
                owner: if mask & 1 != 0 { 0 } else { 7 },
                controller: if mask & 2 != 0 { u32::MAX } else { 2 },
            });
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::States, &delta, 0, &mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                5 + 1 + usize::from(mask & 1 != 0) + 5 * usize::from(mask & 2 != 0)
            );
            assert_eq!(&bytes[..5], &[0xf8, 0xac, 0xd1, 0x91, 0x01]);
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::States, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(decoded.apply(TableId::States, Some(&old)).unwrap(), new);
            if mask != TableId::States.full_mask() {
                assert!(decoded.apply(TableId::States, None).is_err());
            }
        }
    }

    #[test]
    fn division_rows_keep_full_handles_and_native_fixed_point_bytes() {
        for id in [0, 1_u64 << 63, u64::MAX] {
            let row = Row::Division(DivisionRow { id, ..division() });
            let mut bytes = Vec::new();
            encode_row(TableId::Divisions, &row, 0, &mut bytes).unwrap();
            let key_len = if id == 0 { 1 } else { 10 };
            assert_eq!(bytes.len(), key_len + 40);
            assert_eq!(
                &bytes[key_len + 16..key_len + 24],
                &division().template.to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 24..key_len + 32],
                &division().hit_points.to_le_bytes()
            );
            assert_eq!(
                &bytes[key_len + 32..key_len + 40],
                &division().organisation.to_le_bytes()
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row(TableId::Divisions, 0, &mut input).unwrap(), row);
            input.finish().unwrap();
            for cut in 0..bytes.len() {
                assert!(
                    decode_row(TableId::Divisions, 0, &mut Decoder::new(&bytes[..cut])).is_err()
                );
            }
            let delta = row.delta(None).unwrap().unwrap();
            bytes.clear();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), key_len + 1 + delta.values.len());
            assert_eq!(bytes[key_len], 0x7f);
            assert_eq!(delta.apply(TableId::Divisions, None).unwrap(), row);
        }
    }

    #[test]
    fn division_masks_preserve_every_field_subset_without_quantization() {
        let original = division();
        let old = Row::Division(original.clone());
        for mask in 1..=TableId::Divisions.full_mask() {
            let mut updated = original.clone();
            if mask & 1 != 0 {
                updated.roster_owner = u32::MAX;
            }
            if mask & 2 != 0 {
                updated.logical_owner = 0;
            }
            if mask & 4 != 0 {
                updated.expeditionary_owner = 7;
            }
            if mask & 8 != 0 {
                updated.province = u32::MAX;
            }
            if mask & 16 != 0 {
                updated.template = u64::MAX;
            }
            if mask & 32 != 0 {
                updated.hit_points = i64::MIN;
            }
            if mask & 64 != 0 {
                updated.organisation = i64::MAX;
            }
            let new = Row::Division(updated);
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::Divisions, &delta, 0, &mut bytes).unwrap();
            assert_eq!(bytes.len(), 10 + 1 + delta.values.len());
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::Divisions, 0, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(decoded.apply(TableId::Divisions, Some(&old)).unwrap(), new);
            assert_eq!(
                old.delta(Some(&new))
                    .unwrap()
                    .unwrap()
                    .apply(TableId::Divisions, Some(&new))
                    .unwrap(),
                old
            );
            if mask != TableId::Divisions.full_mask() {
                assert!(decoded.apply(TableId::Divisions, None).is_err());
            }
        }
    }

    #[test]
    fn numeric_differences_preserve_the_full_signed_and_unsigned_spans() {
        let clock_low = Row::Clock(ClockRow {
            hour: i32::MIN,
            play_time: 0,
            play_session_started_at: i64::MIN,
            seed: i32::MIN,
        });
        let clock_high = Row::Clock(ClockRow {
            hour: i32::MAX,
            play_time: u64::MAX,
            play_session_started_at: i64::MAX,
            seed: i32::MAX,
        });
        let division_low = Row::Division(DivisionRow {
            template: 0,
            hit_points: i64::MIN,
            organisation: i64::MAX,
            ..division()
        });
        let division_high = Row::Division(DivisionRow {
            template: u64::MAX,
            hit_points: i64::MAX,
            organisation: i64::MIN,
            ..division()
        });
        for (low, high) in [(clock_low, clock_high), (division_low, division_high)] {
            for (old, new) in [(&low, &high), (&high, &low)] {
                let table = new.table();
                let delta = new.delta(Some(old)).unwrap().unwrap();
                let mut bytes = Vec::new();
                encode_delta(table, &delta, 0, &mut bytes).unwrap();
                let mut input = Decoder::new(&bytes);
                let restored = decode_delta(table, 0, &mut input).unwrap();
                input.finish().unwrap();
                assert_eq!(restored.apply(table, Some(old)).unwrap(), *new);
                for cut in 0..bytes.len() {
                    assert!(decode_delta(table, 0, &mut Decoder::new(&bytes[..cut])).is_err());
                }
            }
        }
    }

    #[test]
    fn numeric_changes_refuse_out_of_range_differences_and_reconstructed_values() {
        let cases = [
            (
                Row::Clock(ClockRow {
                    play_time: u64::MAX,
                    ..ClockRow::default()
                }),
                2,
                vec![2],
            ),
            (Row::Clock(ClockRow::default()), 2, vec![1]),
            (
                Row::Clock(ClockRow {
                    seed: i32::MAX,
                    ..ClockRow::default()
                }),
                8,
                vec![2],
            ),
            (
                Row::Clock(ClockRow {
                    play_session_started_at: i64::MIN,
                    ..ClockRow::default()
                }),
                4,
                vec![1],
            ),
            (
                Row::State(StateRow {
                    owner: u32::MAX,
                    ..StateRow::default()
                }),
                1,
                vec![2],
            ),
            (
                Row::Country(CountryRow {
                    political_power_raw: Some(i64::MAX),
                    ..country()
                }),
                8,
                vec![1, 2],
            ),
        ];
        for (old, mask, values) in cases {
            let table = old.table();
            let change = DeltaRow {
                key: old.key(),
                mask,
                values,
            };
            let mut bytes = Vec::new();
            // These differences are legal; their addition to this baseline is not.
            encode_delta(table, &change, 0, &mut bytes).unwrap();
            assert!(change.apply(table, Some(&old)).is_err());
            let decoded = decode_delta(table, 0, &mut Decoder::new(&bytes)).unwrap();
            assert!(decoded.apply(table, Some(&old)).is_err());
        }
        for (table, mask, difference) in [
            (TableId::States, 1, 1_u128 << 32),
            (TableId::Clock, 2, 1_u128 << 64),
        ] {
            let mut values = Vec::new();
            put_varint(&mut values, difference * 2);
            let invalid = DeltaRow {
                key: 0,
                mask,
                values: values.clone(),
            };
            assert!(encode_delta(table, &invalid, 0, &mut Vec::new()).is_err());
            let mut wire = vec![0, mask as u8];
            wire.extend(values);
            assert!(decode_delta(table, 0, &mut Decoder::new(&wire)).is_err());
        }
    }

    #[test]
    fn table_keys_keep_native_ranges_and_refuse_overflow() {
        for table in TableId::ALL {
            let key = match table {
                TableId::Clock | TableId::Campaign => 0,
                TableId::Divisions => u64::MAX,
                _ => u64::from(u32::MAX),
            };
            let mut bytes = Vec::new();
            table.encode_key(key, 0, &mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                if key == 0 {
                    1
                } else if table == TableId::Divisions {
                    10
                } else {
                    5
                }
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(table.decode_key(0, &mut input).unwrap(), key);
            input.finish().unwrap();
            if table == TableId::Divisions {
                continue;
            }
            for oversized in [u64::from(u32::MAX) + 1, u64::MAX] {
                let before = bytes.clone();
                assert!(table.encode_key(oversized, 0, &mut bytes).is_err());
                assert_eq!(bytes, before);
                let invalid = DeltaRow {
                    key: oversized,
                    mask: table.full_mask(),
                    values: Vec::new(),
                };
                assert!(invalid.apply(table, None).is_err());
                assert!(encode_delta(table, &invalid, 0, &mut bytes).is_err());
                assert_eq!(bytes, before);
            }
        }
    }

    #[test]
    fn field_masks_handle_every_subset_and_option_transitions() {
        let old = country();
        for mask in 1_u64..=TableId::Countries.full_mask() {
            let mut new = old.clone();
            if mask & 1 != 0 {
                new.canonical_slot = 3;
            }
            if mask & 2 != 0 {
                new.tag = vec![0xfe];
            }
            if mask & 4 != 0 {
                new.name.clear();
            }
            if mask & 8 != 0 {
                new.political_power_raw = None;
            }
            if mask & 16 != 0 {
                new.command_power_raw = i64::MAX;
            }
            if mask & 32 != 0 {
                new.stability_raw = i64::MIN;
            }
            if mask & 64 != 0 {
                new.war_support_raw = -1;
            }
            if mask & 128 != 0 {
                new.convoys_raw = None;
            }
            let old = Row::Country(old.clone());
            let new = Row::Country(new);
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            assert_eq!(delta.apply(TableId::Countries, Some(&old)).unwrap(), new);
            let reverse = old.delta(Some(&new)).unwrap().unwrap();
            assert_eq!(reverse.apply(TableId::Countries, Some(&new)).unwrap(), old);
        }
    }

    #[test]
    fn malformed_deltas_cannot_invent_a_baseline_or_ignore_bytes() {
        let row = Row::Province(ProvinceRow {
            id: 5,
            state: 3,
            owner: 1,
            controller: 2,
        });
        let partial = DeltaRow {
            key: 5,
            mask: 2,
            values: vec![12], // Owner changes from 1 to 7: zigzag(6).
        };
        assert!(partial.apply(TableId::Provinces, None).is_err());
        assert!(partial.apply(TableId::Countries, Some(&row)).is_err());
        for delta in [
            DeltaRow {
                mask: 0,
                ..partial.clone()
            },
            DeltaRow {
                mask: 8,
                ..partial.clone()
            },
            DeltaRow {
                values: vec![0x80],
                ..partial.clone()
            },
            DeltaRow {
                values: vec![0x80, 0],
                ..partial.clone()
            },
            DeltaRow {
                values: vec![0; 3],
                ..partial.clone()
            },
            DeltaRow {
                values: vec![0; 5],
                ..partial.clone()
            },
        ] {
            assert!(delta.apply(TableId::Provinces, Some(&row)).is_err());
            assert!(encode_delta(TableId::Provinces, &delta, 0, &mut Vec::new()).is_err());
        }
        let wrong_key = DeltaRow { key: 6, ..partial };
        assert!(wrong_key.apply(TableId::Provinces, Some(&row)).is_err());
        let optional = DeltaRow {
            key: 7,
            mask: 8,
            values: vec![2],
        };
        assert!(
            optional
                .apply(TableId::Countries, Some(&Row::Country(country())))
                .is_err()
        );
        assert!(decode_delta(TableId::Countries, 0, &mut Decoder::new(&[7, 8, 2])).is_err());
        assert!(decode_delta(TableId::Provinces, 0, &mut Decoder::new(&[5, 0x80])).is_err());
    }

    #[test]
    fn row_decode_rejects_nonzero_singletons_and_truncated_fields() {
        let row = Row::Campaign(CampaignRow {
            id: b"campaign".to_vec(),
        });
        let mut bytes = Vec::new();
        encode_row(TableId::Campaign, &row, 0, &mut bytes).unwrap();
        for end in 0..bytes.len() {
            assert!(decode_row(TableId::Campaign, 0, &mut Decoder::new(&bytes[..end])).is_err());
        }
        bytes[0] = 1;
        assert!(decode_row(TableId::Campaign, 0, &mut Decoder::new(&bytes)).is_err());
        assert!(encode_row(TableId::Clock, &row, 0, &mut Vec::new()).is_err());
        assert!(TableId::try_from(255).is_err());
    }
}
