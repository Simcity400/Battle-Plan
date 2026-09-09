//! Engine-native rows and the common field-mask codec used by every table.

use std::io;

use crate::codec::{Decoder, invalid, put_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TableId {
    Clock = 0,
    Campaign = 1,
    Countries = 2,
    CountryLinks = 3,
    Provinces = 4,
    States = 5,
}

impl TableId {
    pub const ALL: [Self; 6] = [
        Self::Clock,
        Self::Campaign,
        Self::Countries,
        Self::CountryLinks,
        Self::Provinces,
        Self::States,
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

    const fn schema(self) -> &'static [Kind] {
        use Kind::*;
        match self {
            Self::Clock => &[I32, U64, I64, I32],
            Self::Campaign => &[Bytes],
            Self::Countries => &[U32, Bytes, Bytes, OptionalI64, I64, I64, I64, OptionalI64],
            Self::CountryLinks => &[U32],
            Self::Provinces => &[U32, U32, U32],
            Self::States => &[U32, U32],
        }
    }

    fn check_key(self, key: u32) -> io::Result<()> {
        if matches!(self, Self::Clock | Self::Campaign) && key != 0 {
            return Err(invalid("singleton row key must be zero"));
        }
        Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Clock(ClockRow),
    Campaign(CampaignRow),
    Country(CountryRow),
    CountryLink(CountryLinkRow),
    Province(ProvinceRow),
    State(StateRow),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaRow {
    pub key: u32,
    pub mask: u64,
    /// Only the selected fields, in schema order and in native little-endian form.
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
        }
    }

    pub const fn key(&self) -> u32 {
        match self {
            Self::Clock(_) | Self::Campaign(_) => 0,
            Self::Country(row) => row.id,
            Self::CountryLink(row) => row.id,
            Self::Province(row) => row.id,
            Self::State(row) => row.id,
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
                field.encode(&mut delta.values)?;
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
        }
    }

    fn from_fields(table: TableId, key: u32, fields: [Field<'_>; MAX_FIELDS]) -> io::Result<Self> {
        use Field::*;
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
                id: key,
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
                    id: key,
                    canonical_slot,
                })
            }
            (TableId::Provinces, [U32(state), U32(owner), U32(controller), ..]) => {
                Self::Province(ProvinceRow {
                    id: key,
                    state,
                    owner,
                    controller,
                })
            }
            (TableId::States, [U32(owner), U32(controller), ..]) => Self::State(StateRow {
                id: key,
                owner,
                controller,
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
                fields[index] = kind.decode(&mut decoder)?;
            }
        }
        decoder.finish()?;
        Row::from_fields(table, self.key, fields)
    }
}

pub fn encode_row(table: TableId, row: &Row, output: &mut Vec<u8>) -> io::Result<()> {
    if row.table() != table {
        return Err(invalid("row belongs to a different table"));
    }
    output.extend_from_slice(&row.key().to_le_bytes());
    for field in &row.fields()[..table.field_count()] {
        field.encode(output)?;
    }
    Ok(())
}

pub fn decode_row(table: TableId, decoder: &mut Decoder<'_>) -> io::Result<Row> {
    let key = decoder.u32()?;
    table.check_key(key)?;
    let mut fields = [Field::Empty; MAX_FIELDS];
    for (index, kind) in table.schema().iter().enumerate() {
        fields[index] = kind.decode(decoder)?;
    }
    Row::from_fields(table, key, fields)
}

pub fn encode_delta(table: TableId, delta: &DeltaRow, output: &mut Vec<u8>) -> io::Result<()> {
    table.check_key(delta.key)?;
    table.check_mask(delta.mask)?;
    let mut decoder = Decoder::new(&delta.values);
    for (index, kind) in table.schema().iter().enumerate() {
        if delta.mask & (1_u64 << index) != 0 {
            kind.decode(&mut decoder)?;
        }
    }
    decoder.finish()?;
    output.extend_from_slice(&delta.key.to_le_bytes());
    output.extend_from_slice(&delta.mask.to_le_bytes()[..table.mask_width()]);
    output.extend_from_slice(&delta.values);
    Ok(())
}

pub fn decode_delta(table: TableId, decoder: &mut Decoder<'_>) -> io::Result<DeltaRow> {
    let key = decoder.u32()?;
    table.check_key(key)?;
    let mut mask_bytes = [0; 8];
    mask_bytes[..table.mask_width()].copy_from_slice(decoder.take(table.mask_width())?);
    let mask = u64::from_le_bytes(mask_bytes);
    table.check_mask(mask)?;
    let mut values = Vec::new();
    for (index, kind) in table.schema().iter().enumerate() {
        if mask & (1_u64 << index) != 0 {
            kind.decode(decoder)?.encode(&mut values)?;
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

impl Field<'_> {
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
        ];
        for row in rows {
            let table = row.table();
            let mut bytes = Vec::new();
            encode_row(table, &row, &mut bytes).unwrap();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(decode_row(table, &mut decoder).unwrap(), row);
            decoder.finish().unwrap();

            let delta = row.delta(None).unwrap().unwrap();
            assert_eq!(delta.mask, table.full_mask());
            assert_eq!(delta.apply(table, None).unwrap(), row);
            bytes.clear();
            encode_delta(table, &delta, &mut bytes).unwrap();
            let mut decoder = Decoder::new(&bytes);
            assert_eq!(decode_delta(table, &mut decoder).unwrap(), delta);
            decoder.finish().unwrap();
        }
    }

    #[test]
    fn one_native_scalar_change_costs_key_mask_and_eight_bytes() {
        let old = country();
        let mut new = old.clone();
        new.stability_raw += 1;
        let old = Row::Country(old);
        let new = Row::Country(new);
        assert_eq!(old.delta(Some(&old)).unwrap(), None);
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 5);
        assert_eq!(delta.values, 100_002_i64.to_le_bytes());
        let mut bytes = Vec::new();
        encode_delta(TableId::Countries, &delta, &mut bytes).unwrap();
        assert_eq!(bytes.len(), 4 + 1 + 8);
        assert_eq!(&bytes[..5], &[7, 0, 0, 0, 0x20]);
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
                id: old.key(),
                owner: if mask & 1 != 0 { 0 } else { 7 },
                controller: if mask & 2 != 0 { u32::MAX } else { 2 },
            });
            let delta = new.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            let mut bytes = Vec::new();
            encode_delta(TableId::States, &delta, &mut bytes).unwrap();
            assert_eq!(bytes.len(), 4 + 1 + mask.count_ones() as usize * 4);
            assert_eq!(&bytes[..4], &old.key().to_le_bytes());
            let mut input = Decoder::new(&bytes);
            let decoded = decode_delta(TableId::States, &mut input).unwrap();
            input.finish().unwrap();
            assert_eq!(decoded.apply(TableId::States, Some(&old)).unwrap(), new);
            if mask != TableId::States.full_mask() {
                assert!(decoded.apply(TableId::States, None).is_err());
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
            values: 7_u32.to_le_bytes().to_vec(),
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
                values: vec![0; 3],
                ..partial.clone()
            },
            DeltaRow {
                values: vec![0; 5],
                ..partial.clone()
            },
        ] {
            assert!(delta.apply(TableId::Provinces, Some(&row)).is_err());
            assert!(encode_delta(TableId::Provinces, &delta, &mut Vec::new()).is_err());
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
        assert!(decode_delta(TableId::Countries, &mut Decoder::new(&[7, 0, 0, 0, 8, 2])).is_err());
        assert!(decode_delta(TableId::Provinces, &mut Decoder::new(&[5, 0, 0, 0, 0x80])).is_err());
    }

    #[test]
    fn row_decode_rejects_nonzero_singletons_and_truncated_fields() {
        let row = Row::Campaign(CampaignRow {
            id: b"campaign".to_vec(),
        });
        let mut bytes = Vec::new();
        encode_row(TableId::Campaign, &row, &mut bytes).unwrap();
        for end in 0..bytes.len() {
            assert!(decode_row(TableId::Campaign, &mut Decoder::new(&bytes[..end])).is_err());
        }
        bytes[0] = 1;
        assert!(decode_row(TableId::Campaign, &mut Decoder::new(&bytes)).is_err());
        assert!(encode_row(TableId::Clock, &row, &mut Vec::new()).is_err());
        assert!(TableId::try_from(255).is_err());
    }
}
