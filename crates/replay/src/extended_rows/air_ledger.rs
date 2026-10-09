use super::*;

/// Exact per-write daily-history changes through `140C4D180/140C4D420`.
/// The key is an increasing event ordinal within this recording branch.
/// `game_hour` dates the native application, independently of the hourly drain.
/// Country references resolve through that frame's CountryLinks. `region`
/// resolves through the recording's verified content region inventory.
///
/// `amount_raw` is the signed fixed5 increment at both `family_base` and
/// `family_base + native_cause` (twice in one word for cause zero). Display
/// divisor is 100,000. Saturation is refused. The native writer books requested
/// loss counts, which can exceed actual removals from an exhausted wing pool.
/// `credited_enemy` distinguishes own losses from enemy credit. `opponent`
/// retains the opposite tag from the wrapper, with null for nonpositive tags.
/// `daily_slot` is the authenticated physical daily cursor at the write,
/// including newly allocated histories; save equality uses it across midnight.
/// The engine writer map owns ordinal names and address evidence.
/// A complete hourly table certifies zero for country/region pairs with no
/// events. Lost and first/recovery partial intervals remain unavailable.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirLossEventRow {
    pub id: u64,
    pub game_hour: i32,
    pub country: u32,
    pub region: u32,
    pub native_cause: u32,
    pub family_base: u32,
    pub credited_enemy: bool,
    pub amount_raw: i64,
    pub opponent: Option<u32>,
    pub daily_slot: u32,
}

typed_codec!(AirLossEventRow, 64,
|row| row.id as u128,
|key| Self { id: key as u64, ..Default::default() },
{ 0 => (game_hour): I32, 1 => (country): U32, 2 => (region): U32,
  3 => (native_cause): U32, 4 => (family_base): U32,
  5 => (credited_enemy): Bool, 6 => (amount_raw): I64, 7 => (opponent): OptionalU32,
  8 => (daily_slot): U32 }, |row| {
    if !engine::air_loss_sites::FAMILY_BASES.contains(&row.family_base)
        || row.family_base.checked_add(row.native_cause).is_none_or(|index| index as usize >= engine::air_loss_sites::FIELD_COUNT)
        || row.amount_raw == 0
        || row.daily_slot >= engine::cloop_history::DAILY_WINDOW_CAPACITY
    {
        return Err(invalid("invalid air loss family, cause, amount or daily slot"));
    }
    Ok(())
});

/// Exact hourly native daily-update roster predicate (`1406F35B0`).
/// Every canonical country is present; false is distinct from a missing witness.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirDailyRosterRow {
    pub country: u32,
    pub eligible: bool,
}
typed_codec!(AirDailyRosterRow, 32,
|row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (eligible): Bool }, |_row| Ok(()));

/// Stable source classes; executable caller addresses stay in engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BuildingBombingDamageCause {
    #[default]
    Unknown = 0,
    AirMission = 1,
    AirNaval = 2,
    LandCombat = 3,
    Scripted = 4,
    Weekly = 5,
    ProgramTeardown = 6,
    AbortDismantle = 7,
}

impl BuildingBombingDamageCause {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::AirMission => "air_mission",
            Self::AirNaval => "air_naval",
            Self::LandCombat => "land_combat",
            Self::Scripted => "scripted",
            Self::Weekly => "weekly",
            Self::ProgramTeardown => "program_teardown",
            Self::AbortDismantle => "abort_dismantle",
        }
    }
}

impl TryFrom<u8> for BuildingBombingDamageCause {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Ok(match value {
            0 => Self::Unknown,
            1 => Self::AirMission,
            2 => Self::AirNaval,
            3 => Self::LandCombat,
            4 => Self::Scripted,
            5 => Self::Weekly,
            6 => Self::ProgramTeardown,
            7 => Self::AbortDismantle,
            _ => return Err(invalid("unsupported building bombing damage cause")),
        })
    }
}

impl From<engine::building_damage_sites::Cause> for BuildingBombingDamageCause {
    fn from(cause: engine::building_damage_sites::Cause) -> Self {
        use engine::building_damage_sites::Cause;
        match cause {
            Cause::Unknown => Self::Unknown,
            Cause::AirMission => Self::AirMission,
            Cause::AirNaval => Self::AirNaval,
            Cause::LandCombat => Self::LandCombat,
            Cause::Scripted => Self::Scripted,
            Cause::Weekly => Self::Weekly,
            Cause::ProgramTeardown => Self::ProgramTeardown,
            Cause::AbortDismantle => Self::AbortDismantle,
        }
    }
}

impl NativeValue for BuildingBombingDamageCause {
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        let Field::U8(value) = field else {
            return Err(invalid("field type differs from schema"));
        };
        Self::try_from(value)
    }
}

/// One signed health removal through CBuilding::ApplyDamage@1410C6F60.
/// ApplyBuildingBombingDamage@140F663C0 and naval damage@1419681E0 reach
/// this shared writer; other callers retain their source classification.
/// The key is an increasing event ordinal within this recording branch, never
/// a building handle. `game_hour` dates the application, independently of the
/// later hourly drain. No-wing damage retains a null attacker and its source.
/// A wing's attacker is GetEffectiveCountryTag@140F45950, not its roster owner.
/// `victim` is the location controller at application, which the air wrapper
/// credits (province+0x188 / state+0xCC), rather than status+0x70 repair routing.
/// Zero is the native reserved null-country reference (`140BA5140/140BA56A0`):
/// retain such damage with an unknown victim identity, never a guessed country.
/// `province` distinguishes province buildings from state buildings. Type
/// tokens resolve through recorded LexicalTokens and the effective content.
/// A province building whose native province has no state records state zero
/// (`get_state@1410C6520 -> 141162450`, province+0xC0); that is known absence.
/// State-scoped buildings retain their actual state id.
///
/// `amount_raw` is pre-minus-post native CBuilding health after clamps; a full
/// level is 10,000,000 (`get_level_health@1410C64D0`). `credited_raw` is the
/// function's returned native budget and can differ under damage modifiers.
/// Negative modifiers can increase health (ApplyDamageStep@1410C7000);
/// those gains remain exact negative removals rather than unavailable damage.
/// `native_cause` preserves the source-kind word at DamageSource+0 (argument
/// five of ApplyDamage@1410C6F60), independently of the stable caller class.
/// This is an hourly event ledger: a complete empty
/// table is zero events, an unavailable interval is unknown. Its first branch
/// interval and every interval after a lost event are unavailable.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BuildingBombingDamageRow {
    pub id: u64,
    pub game_hour: i32,
    pub attacker: Option<u32>,
    pub victim: u32,
    pub state: u32,
    pub province: Option<u32>,
    pub building_type: u32,
    pub native_cause: u32,
    pub cause: BuildingBombingDamageCause,
    pub amount_raw: i64,
    pub credited_raw: i64,
}

typed_codec!(BuildingBombingDamageRow, 64, |row| row.id as u128,
|key| Self { id: key as u64, ..Default::default() },
{ 0 => (game_hour): I32, 1 => (attacker): OptionalU32, 2 => (victim): U32,
  3 => (state): U32, 4 => (province): OptionalU32, 5 => (building_type): U32,
  6 => (native_cause): U32, 7 => (cause): U8, 8 => (amount_raw): I64,
  9 => (credited_raw): I64 }, |row| {
    if row.amount_raw == 0 { Err(invalid("building bombing damage must be nonzero")) }
    else { Ok(()) }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_event_compact_context_roundtrip_and_validation() {
        let mut row = AirLossEventRow {
            id: u64::MAX,
            game_hour: -24,
            country: 4999,
            region: 40001,
            native_cause: 6,
            family_base: 71,
            credited_enemy: true,
            amount_raw: i64::MIN,
            opponent: Some(5001),
            daily_slot: 29,
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<AirLossEventRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            row
        );
        let previous = row.clone();
        row.amount_raw = -7;
        row.game_hour = 0;
        row.daily_slot = 0;
        let delta = row.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 0) | (1 << 6) | (1 << 8));
        assert_eq!(delta.apply(Some(previous)).unwrap(), row);
        for bad in [
            AirLossEventRow {
                amount_raw: 0,
                ..row.clone()
            },
            AirLossEventRow {
                family_base: 31,
                ..row.clone()
            },
            AirLossEventRow {
                native_cause: u32::MAX,
                ..row.clone()
            },
            AirLossEventRow {
                native_cause: 8,
                family_base: 81,
                ..row.clone()
            },
            AirLossEventRow {
                daily_slot: engine::cloop_history::DAILY_WINDOW_CAPACITY,
                ..row.clone()
            },
        ] {
            assert!(TableRows::AirLossEvents(vec![bad]).validate_keys().is_err());
        }
        assert!(TableId::AirLossEvents.check_key(1_u128 << 64).is_err());
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        bytes[20] = 2;
        assert!(AirLossEventRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
        bytes[20] = 1;
        bytes.pop();
        assert!(AirLossEventRow::decode_fields(0, &mut Decoder::new(&bytes)).is_err());
    }

    #[test]
    fn building_damage_cause_and_null_attribution_roundtrip_losslessly() {
        for value in 0..=7 {
            let cause = BuildingBombingDamageCause::try_from(value).unwrap();
            let row = BuildingBombingDamageRow {
                id: u64::MAX,
                game_hour: i32::MIN,
                attacker: if value.is_multiple_of(2) {
                    None
                } else {
                    Some(u32::MAX)
                },
                victim: 4999,
                state: 40_001,
                province: Some(80_001),
                building_type: 60_001,
                native_cause: u32::MAX,
                cause,
                amount_raw: if value.is_multiple_of(2) {
                    i64::MAX
                } else {
                    i64::MIN
                },
                credited_raw: i64::MIN,
            };
            let mut bytes = Vec::new();
            encode_row(&row, 0, &mut bytes).unwrap();
            assert_eq!(
                decode_row::<BuildingBombingDamageRow>(0, &mut Decoder::new(&bytes)).unwrap(),
                row
            );
            assert_eq!(
                row.delta(None)
                    .unwrap()
                    .unwrap()
                    .apply::<BuildingBombingDamageRow>(None)
                    .unwrap(),
                row
            );
        }
        for invalid_cause in [8, 255] {
            assert!(BuildingBombingDamageCause::try_from(invalid_cause).is_err());
            assert!(
                <BuildingBombingDamageCause as NativeValue>::from_field(Field::U8(invalid_cause))
                    .is_err()
            );
        }
        assert!(
            TableRows::BuildingBombingDamage(vec![BuildingBombingDamageRow {
                amount_raw: 0,
                ..Default::default()
            }])
            .validate_keys()
            .is_err()
        );
    }

    #[test]
    #[ignore = "synthetic codec scale measurement; run with --ignored --nocapture"]
    fn air_ledgers_five_thousand_country_sparse_scale() {
        use std::time::Instant;

        // Synthetic identities intentionally exceed the vanilla inventories.
        // Three active regions per country model sparse country-region demand.
        let losses: Vec<_> = (0..5_000_u32)
            .flat_map(|country| {
                (0..3_u32).map(move |region| AirLossEventRow {
                    id: country as u64 * 3 + region as u64,
                    game_hour: 72,
                    country,
                    region: 40_000 + region,
                    native_cause: region,
                    family_base: if region == 2 { 71 } else { 30 },
                    credited_enemy: region == 2,
                    amount_raw: 100_000 + country as i64,
                    opponent: Some((country + 1) % 5000),
                    daily_slot: region,
                })
            })
            .collect();
        let damage: Vec<_> = (0..5_000_u32)
            .map(|country| BuildingBombingDamageRow {
                id: country as u64,
                game_hour: 72,
                attacker: Some(country),
                victim: (country + 1) % 5_000,
                state: 50_000 + country,
                province: None,
                building_type: 60_000 + country,
                native_cause: 0,
                cause: BuildingBombingDamageCause::AirMission,
                amount_raw: 100_000 + country as i64,
                credited_raw: 200_000 + country as i64,
            })
            .collect();

        fn measure<T: RowCodec + std::fmt::Debug>(rows: &[T]) -> (usize, u128, u128) {
            let start = Instant::now();
            let mut bytes = Vec::new();
            let mut previous = 0;
            for row in rows {
                encode_row(row, previous, &mut bytes).unwrap();
                previous = row.key();
            }
            let encode_us = start.elapsed().as_micros();
            let start = Instant::now();
            let mut input = Decoder::new(&bytes);
            let mut previous = 0;
            for expected in rows {
                let row = decode_row::<T>(previous, &mut input).unwrap();
                assert_eq!(&row, expected);
                previous = row.key();
            }
            input.finish().unwrap();
            (bytes.len(), encode_us, start.elapsed().as_micros())
        }

        let (air_bytes, air_encode_us, air_decode_us) = measure(&losses);
        let (damage_bytes, damage_encode_us, damage_decode_us) = measure(&damage);
        assert_eq!(losses.len(), 15_000);
        assert_eq!(damage.len(), 5_000);
        eprintln!(
            "air ledger synthetic countries=5000 sparse_air_rows={} air_bytes={air_bytes} air_encode_us={air_encode_us} air_decode_us={air_decode_us} damage_rows={} damage_bytes={damage_bytes} damage_encode_us={damage_encode_us} damage_decode_us={damage_decode_us}",
            losses.len(),
            damage.len()
        );
    }
}
