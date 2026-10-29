//! Row fields the current format no longer records.
use super::*;
use crate::rows::compatibility::RETIRED_INPUTS_FORMAT;

/// A row field whose capture ended at [`RETIRED_INPUTS_FORMAT`], listed in
/// [`compatibility::RETIRED_FIELDS`]. The field keeps its position and Opaque
/// length framing, so the table's other fields stay readable in every
/// recording. Current writers store an empty payload, which never changes
/// after the row is inserted. An older recording's payload is discarded
/// through its framing without being decoded, as a retired table is; no
/// consumer receives it. Use it only as a whole Opaque row field: inside a
/// nested value it would consume the rest of its parent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Retired;

impl Value for Retired {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(0)
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self)
    }
    fn encode_reserved(&self, _output: &mut Vec<u8>) -> io::Result<()> {
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < RETIRED_INPUTS_FORMAT {
            input.take(input.remaining())?;
        }
        Ok(Self)
    }
}
opaque_value!(Retired);

/// A retired part of a nested value, listed in
/// [`compatibility::RETIRED_FIELDS`] as `field.part` and placed where the part
/// was. It holds nothing and current writers store nothing; a recording older
/// than [`RETIRED_INPUTS_FORMAT`] holds the part's bytes, which decode through
/// `T` and are dropped, so the rest of the nested value stays readable.
pub struct RetiredNested<T>(std::marker::PhantomData<fn() -> T>);

impl<T> Default for RetiredNested<T> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}
impl<T> Clone for RetiredNested<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for RetiredNested<T> {}
impl<T> PartialEq for RetiredNested<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl<T> Eq for RetiredNested<T> {}
impl<T> std::fmt::Debug for RetiredNested<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RetiredNested")
    }
}

/// Whether the test-only historical writer, which stands in for format 101
/// (`with_historical_stamp`), is encoding: that writer stores the part as a
/// recording older than [`RETIRED_INPUTS_FORMAT`] held it, with its default
/// value (a [`RetiredKept`] part's held one), so its fixtures carry the
/// layout their format number names.
fn historical_writer() -> bool {
    historical_stamp().is_some()
}

impl<T: Value + Default> Value for RetiredNested<T> {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    fn encoded_bytes(&self) -> io::Result<usize> {
        if historical_writer() {
            T::default().encoded_bytes()
        } else {
            Ok(0)
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        if historical_writer() {
            T::default().encode_reserved(output)
        } else {
            Ok(())
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < RETIRED_INPUTS_FORMAT {
            T::decode(input)?;
        }
        Ok(Self::default())
    }
}

/// A retired part of a nested value whose older value a consumer still
/// reads, listed in [`compatibility::RETIRED_FIELDS`] as `field.part` and
/// placed where the part was. Current writers store nothing and it holds
/// `None`; a recording older than [`RETIRED_INPUTS_FORMAT`] holds the part's
/// bytes, which decode through `T` and are kept, for the consumer that falls
/// back to them where the value's current field is not recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetiredKept<T>(Option<T>);

impl<T> Default for RetiredKept<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> RetiredKept<T> {
    /// An older recording's decoded value.
    #[cfg(any(test, feature = "test-support"))]
    pub fn older_value(value: T) -> Self {
        Self(Some(value))
    }
    /// The value an older recording holds; `None` in current recordings.
    pub fn older(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T: Value + Default> Value for RetiredKept<T> {
    fn heap_bytes(&self) -> usize {
        Value::heap_bytes(&self.0)
    }
    const MINIMUM_ENCODED_BYTES: usize = 0;
    fn encoded_bytes(&self) -> io::Result<usize> {
        if historical_writer() {
            match &self.0 {
                Some(value) => value.encoded_bytes(),
                None => T::default().encoded_bytes(),
            }
        } else {
            Ok(0)
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.0.as_ref().map(Value::try_clone).transpose().map(Self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        if historical_writer() {
            match &self.0 {
                Some(value) => value.encode_reserved(output),
                None => T::default().encode_reserved(output),
            }
        } else {
            Ok(())
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        if input.format < RETIRED_INPUTS_FORMAT {
            T::decode(input).map(|value| Self(Some(value)))
        } else {
            Ok(Self(None))
        }
    }
}

/// A fixed-width signed row field whose capture ended at
/// [`RETIRED_INPUTS_FORMAT`], listed in [`compatibility::RETIRED_FIELDS`]
/// beside the table that still records the value. Its eight-byte framing
/// cannot shrink, so current writers store zero, which never changes after
/// the row is inserted and so costs a delta nothing. An older recording's
/// value is decoded and followed through its deltas, so the table's other
/// fields stay readable, but no consumer receives it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RetiredI64(i64);

#[cfg(any(test, feature = "test-support"))]
impl RetiredI64 {
    /// An older recording's decoded value, for fixtures.
    pub const fn holding(value: i64) -> Self {
        Self(value)
    }
}

impl NativeValue for RetiredI64 {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn field(&self) -> Field<'_> {
        Field::I64(self.0)
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::I64(value) => Ok(Self(value)),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

/// [`RetiredI64`]'s rule for the other fixed-width kinds: the field keeps its
/// framing, current writers store zero (or `None`), and an older recording's
/// value is decoded and followed through its deltas for no consumer.
macro_rules! retired_fixed {
    ($(#[$doc:meta])* $name:ident($ty:ty), $kind:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name($ty);

        #[cfg(any(test, feature = "test-support"))]
        impl $name {
            /// An older recording's decoded value, for fixtures.
            pub const fn holding(value: $ty) -> Self {
                Self(value)
            }
        }

        impl NativeValue for $name {
            fn heap_bytes(&self) -> usize {
                0
            }
            fn try_clone(&self) -> io::Result<Self> {
                Ok(*self)
            }
            fn field(&self) -> Field<'_> {
                Field::$kind(self.0)
            }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                match field {
                    Field::$kind(value) => Ok(Self(value)),
                    _ => Err(invalid("field type differs from schema")),
                }
            }
        }
    };
}
retired_fixed!(
    /// A retired `I32` field.
    RetiredI32(i32),
    I32
);
retired_fixed!(
    /// A retired `U8` field.
    RetiredU8(u8),
    U8
);
retired_fixed!(
    /// A retired `Bool` field: current writers store false.
    RetiredBool(bool),
    Bool
);
retired_fixed!(
    /// A retired `U32` field.
    RetiredU32(u32),
    U32
);
retired_fixed!(
    /// A retired `U64` field.
    RetiredU64(u64),
    U64
);
retired_fixed!(
    /// A retired `OptionalI64` field: current writers store `None`.
    RetiredOptionalI64(Option<i64>),
    OptionalI64
);

/// A retired `U32List` field: current writers store an empty list, which
/// never changes after the row is inserted; an older recording's list is
/// decoded and followed through its deltas for no consumer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RetiredU32List(Vec<u32>);

#[cfg(any(test, feature = "test-support"))]
impl RetiredU32List {
    /// An older recording's decoded list, for fixtures.
    pub fn holding(value: Vec<u32>) -> Self {
        Self(value)
    }
}

impl NativeValue for RetiredU32List {
    fn heap_bytes(&self) -> usize {
        self.0.capacity() * size_of::<u32>()
    }
    fn try_clone(&self) -> io::Result<Self> {
        allocation::copy_vec(&self.0).map(Self)
    }
    fn field(&self) -> Field<'_> {
        Field::U32List(List::Native(&self.0))
    }
    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U32List(value) => value.to_vec().map(Self),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_fields_store_nothing_and_discard_only_older_payloads() {
        let mut bytes = Vec::new();
        Value::encode(&Retired, &mut bytes).unwrap();
        assert!(bytes.is_empty());
        // An older payload is discarded whole, whatever it held.
        let payload = [7_u8, 0, 0, 0, 255];
        let mut input = Decoder::new(&payload);
        input.format = RETIRED_INPUTS_FORMAT - 1;
        assert_eq!(Retired::decode(&mut input).unwrap(), Retired);
        input.finish().unwrap();
        // A current recording holds none; leftover bytes stay unread.
        let mut input = Decoder::new(&payload);
        assert_eq!(Retired::decode(&mut input).unwrap(), Retired);
        assert!(input.finish().is_err());
    }

    #[test]
    fn retired_fixed_width_fields_store_zero_and_follow_older_values_through_deltas() {
        // A current writer's row never marks the tombstone as changed.
        let current = DivisionLandInputsRow {
            division: 7,
            naval_transfer: true,
            ..Default::default()
        };
        let mut previous = current.clone();
        previous.naval_transfer = false;
        let delta = current.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 4);
        // An older recording's value decodes and follows its deltas across
        // the whole native range, so the rest of the row stays exact.
        let older = DivisionLandInputsRow {
            division: 7,
            received_supply: RetiredI64(i64::MIN),
            remaining_grace: RetiredI64(-1),
            ..Default::default()
        };
        let mut later = older.clone();
        later.received_supply = RetiredI64(i64::MAX);
        later.naval_transfer = true;
        let delta = later.delta(Some(&older)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 1) | (1 << 4));
        assert_eq!(delta.apply(Some(older)).unwrap(), later);
    }

    #[test]
    fn every_fixed_width_kind_follows_older_values_and_current_rows_hold_none() {
        let older = InvasionPreparationRow {
            owner_group: 3,
            unit_owner: 9,
            zero_war_present: RetiredU8::holding(u8::MAX),
            co_belligerents: RetiredU32List::holding(vec![u32::MAX, 0, 7]),
            ..Default::default()
        };
        let mut later = older.clone();
        later.zero_war_retired = RetiredU8::holding(1);
        later.co_belligerents = RetiredU32List::holding(vec![2]);
        let delta = later.delta(Some(&older)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 10) | (1 << 12));
        assert_eq!(delta.apply(Some(older)).unwrap(), later);
        let older = CapitulationRow {
            country: 5,
            last_surrender_hour: RetiredI32::holding(i32::MIN),
            legitimacy_raw: RetiredI64(i64::MAX),
            ..Default::default()
        };
        let mut later = older.clone();
        later.exile_army_leaders = RetiredI32::holding(-1);
        let delta = later.delta(Some(&older)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 5);
        assert_eq!(delta.apply(Some(older)).unwrap(), later);
        let older = LandCombatProgressRow {
            combat: 1,
            direction_attacker: RetiredU64::holding(u64::MAX),
            direction_province: RetiredU32::holding(u32::MAX),
            ..Default::default()
        };
        let current = LandCombatProgressRow {
            combat: 1,
            ..Default::default()
        };
        assert_eq!(current.delta(Some(&older)).unwrap().unwrap().mask, 0b1100);
        let older = ProductionLineOutputRow {
            line: 1,
            instant_raw: Recorded::Value(RetiredOptionalI64::holding(Some(i64::MIN))),
            ..Default::default()
        };
        let mut later = older.clone();
        later.instant_raw = Recorded::Value(RetiredOptionalI64::holding(None));
        assert_eq!(
            later
                .delta(Some(&older))
                .unwrap()
                .unwrap()
                .apply(Some(older))
                .unwrap(),
            later
        );
        let older = ProductionLineModifiersRow {
            line: 1,
            conversion: RetiredBool::holding(true),
            ..Default::default()
        };
        assert_eq!(
            ProductionLineModifiersRow {
                line: 1,
                ..Default::default()
            }
            .delta(Some(&older))
            .unwrap()
            .unwrap()
            .mask,
            1 << 7
        );
    }

    /// The format-101 layout of a land combat side, before its hover-only
    /// and oracle-only parts retired.
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    struct HistoricalSide {
        committed: Vec<u64>,
        reserves: Vec<u64>,
        tactic_id: i32,
        tactic_kind: u8,
        tactic_counter_id: u32,
        tactic_token: Vec<u8>,
        air_counts: Vec<i32>,
        anti_air_attack: i64,
        air_kills: u32,
        damage: [i64; 6],
        organisation_loss: [i64; 24],
        organisation_loss_count: u32,
        commander: u64,
        commander_skill: i32,
        commander_category: i32,
        commander_country: u32,
        commander_experience: i64,
        commander_name_token: Vec<u8>,
        commander_description_token: Vec<u8>,
        commander_formation: u64,
        presentation: CombatCommanderPresentation,
        traits: Vec<LandCombatCommanderTrait>,
        timed_traits: Vec<LandCombatCommanderTimedTrait>,
        learning_traits: Vec<LandCombatCommanderLearningTrait>,
    }
    crate::value::value_struct!(HistoricalSide { committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8, tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>, anti_air_attack: i64, air_kills: u32, damage: [i64; 6], organisation_loss: [i64; 24], organisation_loss_count: u32, commander: u64, commander_skill: i32, commander_category: i32, commander_country: u32, commander_experience: i64, commander_name_token: Vec<u8>, commander_description_token: Vec<u8>, commander_formation: u64, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, timed_traits: Vec<LandCombatCommanderTimedTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait> });

    #[test]
    fn retired_nested_parts_decode_older_bytes_and_current_sides_store_none() {
        let historical = HistoricalSide {
            committed: vec![1, u64::MAX],
            air_counts: vec![-1],
            anti_air_attack: i64::MIN,
            air_kills: 3,
            damage: [1, 2, 3, 4, 500_005, 600_000],
            organisation_loss: [-7; 24],
            organisation_loss_count: 24,
            commander: 9,
            commander_experience: i64::MAX,
            commander_name_token: b"NAME".to_vec(),
            commander_description_token: b"DESCRIPTION".to_vec(),
            commander_formation: 11,
            timed_traits: vec![LandCombatCommanderTimedTrait {
                token_id: 1,
                definition_ordinal: 2,
                remaining_days: -1,
            }],
            learning_traits: vec![Default::default()],
            ..Default::default()
        };
        let mut bytes = Vec::new();
        Value::encode(&historical, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        input.format = RETIRED_INPUTS_FORMAT - 1;
        let side = LandCombatSideDetails::decode(&mut input).unwrap();
        input.finish().unwrap();
        // The anti-air cache and the prevented lanes keep their format-101
        // values, exact, for the CAS hover (SIDE_INPUTS_FORMAT); the other
        // retired parts are dropped.
        let kept = LandCombatSideDetails {
            committed: vec![1, u64::MAX],
            air_counts: vec![-1],
            anti_air_attack: RetiredKept::older_value(i64::MIN),
            air_kills: 3,
            damage: [1, 2],
            prevented_damage: RetiredKept::older_value([500_005, 600_000]),
            // Format 101 held the 24 samples; the side keeps their sum.
            organisation_loss_sum: -7 * 24,
            organisation_loss_count: 24,
            commander: 9,
            commander_name_token: b"NAME".to_vec(),
            commander_formation: 11,
            ..Default::default()
        };
        assert_eq!(side, kept);
        // The current layout stores none of the retired parts, and decodes
        // none.
        let current_side = LandCombatSideDetails {
            anti_air_attack: RetiredKept::default(),
            prevented_damage: RetiredKept::default(),
            ..kept.clone()
        };
        let mut current = Vec::new();
        Value::encode(&kept, &mut current).unwrap();
        let mut input = Decoder::new(&current);
        assert_eq!(
            LandCombatSideDetails::decode(&mut input).unwrap(),
            current_side
        );
        input.finish().unwrap();
        let mut reference = Vec::new();
        Value::encode(
            &HistoricalSide {
                committed: kept.committed.clone(),
                air_counts: kept.air_counts.clone(),
                air_kills: 3,
                damage: [1, 2, 0, 0, 0, 0],
                organisation_loss: [-7; 24],
                organisation_loss_count: 24,
                commander: 9,
                commander_name_token: b"NAME".to_vec(),
                commander_formation: 11,
                ..Default::default()
            },
            &mut reference,
        )
        .unwrap();
        // Anti-air, four damage lanes, 23 more loss samples than the sum,
        // experience, an empty token and two empty trait lists.
        assert_eq!(
            reference.len() - current.len(),
            8 + 4 * 8 + 23 * 8 + 8 + 4 + 4 + 4
        );
    }

    /// The field one default row stores at `index`.
    fn stored(rows: TableRows, index: usize) -> String {
        let table = rows.table();
        let mut bytes = Vec::new();
        rows.encode_rows(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        table.decode_key(0, &mut input).unwrap();
        for (at, kind) in table.schema().iter().enumerate() {
            let field = kind.decode(&mut input).unwrap();
            if at == index {
                return match field {
                    Field::Opaque(Opaque::Encoded(payload, _)) => {
                        format!("opaque {} bytes", payload.len())
                    }
                    Field::I64(value) => format!("i64 {value}"),
                    Field::I32(value) => format!("i32 {value}"),
                    Field::U32(value) => format!("u32 {value}"),
                    Field::U64(value) => format!("u64 {value}"),
                    Field::U8(value) => format!("u8 {value}"),
                    Field::Bool(value) => format!("bool {value}"),
                    Field::OptionalI64(value) => format!("optional {value:?}"),
                    Field::U32List(value) => format!("list {}", value.to_vec().unwrap().len()),
                    _ => panic!("{table:?} field {index} has no tombstone kind"),
                };
            }
        }
        panic!("{table:?} has no field {index}");
    }

    #[test]
    fn every_catalogued_retired_field_is_an_empty_or_zero_tombstone() {
        for field in compatibility::RETIRED_FIELDS {
            assert!(
                (RETIRED_INPUTS_FORMAT..=crate::FORMAT_VERSION).contains(&field.retired),
                "{}",
                field.name
            );
            if field.name.contains('.') {
                // A nested retirement changes its parent's nested layout.
                continue;
            }
            let rows = match field.table {
                TableId::Provinces => TableRows::Provinces(vec![Default::default()]),
                TableId::DivisionMovement => TableRows::DivisionMovement(vec![Default::default()]),
                TableId::DivisionStatus => TableRows::DivisionStatus(vec![Default::default()]),
                TableId::DivisionLandInputs => {
                    TableRows::DivisionLandInputs(vec![Default::default()])
                }
                TableId::CountryCounterInputs => {
                    TableRows::CountryCounterInputs(vec![Default::default()])
                }
                TableId::OrdersGroupCounterInputs => {
                    TableRows::OrdersGroupCounterInputs(vec![Default::default()])
                }
                TableId::WeatherRegions => TableRows::WeatherRegions(vec![Default::default()]),
                TableId::CountryAppearance => {
                    TableRows::CountryAppearance(vec![Default::default()])
                }
                TableId::RailwayGunCounterInputs => {
                    TableRows::RailwayGunCounterInputs(vec![Default::default()])
                }
                TableId::AirBaseRegistrations => {
                    TableRows::AirBaseRegistrations(vec![Default::default()])
                }
                TableId::ReservedCountryNaming => {
                    TableRows::ReservedCountryNaming(vec![Default::default()])
                }
                TableId::LandCombatDivisions => {
                    TableRows::LandCombatDivisions(vec![Default::default()])
                }
                TableId::LandCombatProgress => {
                    TableRows::LandCombatProgress(vec![Default::default()])
                }
                TableId::LandCombatModifiers => {
                    TableRows::LandCombatModifiers(vec![Default::default()])
                }
                TableId::ShipModels => TableRows::ShipModels(vec![Default::default()]),
                TableId::UnitLeaders => TableRows::UnitLeaders(vec![Default::default()]),
                TableId::CountryFuel => TableRows::CountryFuel(vec![Default::default()]),
                TableId::Politics => TableRows::Politics(vec![Default::default()]),
                TableId::Capitulation => TableRows::Capitulation(vec![Default::default()]),
                TableId::ProductionLines => TableRows::ProductionLines(vec![Default::default()]),
                TableId::ProductionLineOutput => {
                    TableRows::ProductionLineOutput(vec![Default::default()])
                }
                TableId::ProductionLineModifiers => {
                    TableRows::ProductionLineModifiers(vec![Default::default()])
                }
                TableId::InvasionPreparations => {
                    TableRows::InvasionPreparations(vec![Default::default()])
                }
                TableId::CountryMarket => TableRows::CountryMarket(vec![Default::default()]),
                table => panic!("add a default row for {table:?}"),
            };
            let stored = stored(rows, field.index);
            assert!(
                [
                    "opaque 0 bytes",
                    "i64 0",
                    "i32 0",
                    "u32 0",
                    "u64 0",
                    "u8 0",
                    "bool false",
                    "optional None",
                    "list 0",
                ]
                .contains(&stored.as_str()),
                "{:?}.{} stores {stored}",
                field.table,
                field.name
            );
        }
    }
}
