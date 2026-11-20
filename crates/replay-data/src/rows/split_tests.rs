//! Tests of rows declared in replay-rows that read the registry, kept beside it.
use super::*;

#[path = "../extended_rows/border_effects/tests.rs"]
mod extended_rows_border_effect_tests;

mod rows_retired_tests {
    use super::*;
    use crate::rows::compatibility::RETIRED_INPUTS_FORMAT;
    use crate::rows::test_rows::HistoricalSide;

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

mod extended_rows_air_ledger_tests {
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
                daily_slot: engine_reference_values::native_values::air_history::DAILY_WINDOW_CAPACITY,
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

#[path = "../extended_rows/equipment_destruction/tests.rs"]
mod extended_rows_equipment_destruction_tests;

mod extended_rows_fresh_eta_tests {
    use super::*;
    use crate::value::Value;

    fn round_trip<T: Value + std::fmt::Debug + PartialEq>(value: &T) -> Vec<u8> {
        let mut bytes = Vec::new();
        Value::encode(value, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(T::decode(&mut input).unwrap(), *value);
        input.finish().unwrap();
        bytes
    }

    #[test]
    fn historical_gap_refusal_null_building_and_zero_health_stay_distinct() {
        type Field = Recorded<Option<RailwayBuildingState>>;
        let historical = <Field as Value>::not_recorded().unwrap();
        assert_eq!(historical, Recorded::NotRecorded);
        assert_eq!(Value::recorded_bytes(&historical).unwrap(), 0);
        assert!(Value::encode(&historical, &mut Vec::new()).is_err());

        let refusal = Recorded::Value(None);
        let null = Recorded::Value(Some(RailwayBuildingState { building: None }));
        let zero = Recorded::Value(Some(RailwayBuildingState {
            building: Some(RailwayBuildingHealth {
                level: 0,
                healthy: 0,
                partial: None,
            }),
        }));
        let encoded = [
            round_trip::<Field>(&refusal),
            round_trip::<Field>(&null),
            round_trip::<Field>(&zero),
        ];
        assert_eq!(encoded[0], [0]);
        assert_eq!(encoded[1], [1, 0]);
        assert_eq!(encoded[2], [1, 1, 0, 0, 0, 0, 0]);
    }

    fn check_suffix<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        index: usize,
        previous: &T,
        row: &T,
    ) {
        assert_eq!(table.field_count(), T::SCHEMA.len());
        assert_eq!(table.mask_width(), T::SCHEMA.len().div_ceil(8));
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(&T::decode_fields(row.key(), &mut input).unwrap(), row);
        input.finish().unwrap();
        bytes.clear();
        let delta = row.delta(Some(previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << index);
        encode_delta(table, &delta, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(&delta.apply(Some(previous.clone())).unwrap(), row);
    }

    #[test]
    fn fresh_suffixes_round_trip_and_change_without_legacy_fields() {
        let previous = StateRow {
            id: u32::MAX,
            movement_infrastructure: 918,
            ..Default::default()
        };
        for value in [0, i64::MIN, i64::MAX] {
            check_suffix(
                TableId::States,
                7,
                &previous,
                &StateRow {
                    railway_infrastructure: Some(value).into(),
                    ..previous.clone()
                },
            );
        }
        let previous = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: -1,
            ..Default::default()
        };
        for building in [
            None,
            Some(RailwayBuildingHealth {
                level: 1,
                healthy: 0,
                partial: Some(i64::MIN),
            }),
        ] {
            check_suffix(
                TableId::RailCooldowns,
                1,
                &previous,
                &RailCooldownRow {
                    building: Some(RailwayBuildingState { building }).into(),
                    ..previous.clone()
                },
            );
        }
    }

    fn historical_prefix<T: RowCodec + Fields + std::fmt::Debug>(
        table: TableId,
        index: usize,
        row: &T,
    ) -> T {
        let introduced = table
            .last_change()
            .appended
            .iter()
            .find(|field| field.index == index)
            .expect("fresh suffix has a compatibility entry")
            .introduced;
        let format = crate::format::FormatVersion::read(introduced - 1).unwrap();
        let fields = format.fields(table);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        for kind in &T::SCHEMA[..fields] {
            kind.decode(&mut input).unwrap();
        }
        let prefix = &bytes[..bytes.len() - input.remaining()];
        let mut input = Decoder::new(prefix);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = T::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(
            decoded.not_recorded_fields(),
            full_mask(T::SCHEMA.len()) ^ full_mask(fields)
        );
        assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
        assert!(decoded.encode_fields(&mut Vec::new()).is_err());

        let delta = row.delta(None).unwrap().unwrap();
        let mut input = Decoder::new(&delta.values);
        for kind in &T::SCHEMA[..fields] {
            kind.decode_change(&mut input).unwrap();
        }
        bytes.clear();
        table.encode_key(row.key(), 0, &mut bytes).unwrap();
        bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
        bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
        let mut input = Decoder::new(&bytes);
        input.format = format.version();
        input.recorded_fields = fields;
        let delta = decode_delta(table, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(delta.apply::<T>(None).unwrap(), decoded);
        decoded
    }

    #[test]
    fn selected_railway_suffix_preserves_signed_native_domains_and_legacy_absence() {
        let previous = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: -1,
            building: Some(RailwayBuildingState { building: None }).into(),
            ..Default::default()
        };
        let input = RailwaySelectionInputs {
            levels: Some(vec![i32::MIN, 0, i32::MAX]),
            pair_count: Some(i32::MIN),
            building_level: Some(Some(i16::MIN)),
        };
        let row = RailCooldownRow {
            selection: Some(input.clone()).into(),
            ..previous.clone()
        };
        check_suffix(TableId::RailCooldowns, 2, &previous, &row);
        check_suffix(
            TableId::RailCooldowns,
            3,
            &row,
            &RailCooldownRow {
                selection_hour: Some(i32::MIN).into(),
                ..row.clone()
            },
        );
        let current = RailCooldownRow {
            selection_hour: Some(12).into(),
            ..row.clone()
        };
        check_suffix(
            TableId::RailCooldowns,
            3,
            &current,
            &RailCooldownRow {
                selection_hour: Some(13).into(),
                ..current.clone()
            },
        );
        let bound = RailCooldownRow {
            building_bound: Some(true).into(),
            ..current.clone()
        };
        for building_bound in [Some(false), None] {
            check_suffix(
                TableId::RailCooldowns,
                4,
                &bound,
                &RailCooldownRow {
                    building_bound: building_bound.into(),
                    ..bound.clone()
                },
            );
        }
        let old = historical_prefix(TableId::RailCooldowns, 4, &bound);
        assert_eq!(old.building_bound, Recorded::NotRecorded);
        assert_eq!(old.selection, bound.selection);
        assert_eq!(old.selection_hour, bound.selection_hour);
        let old = historical_prefix(TableId::RailCooldowns, 1, &row);
        assert_eq!(old.selection, Recorded::NotRecorded);
        assert_eq!(old.selection_hour, Recorded::NotRecorded);
        assert_eq!(old.building, Recorded::NotRecorded);
        assert_eq!(old.cooldown_ticks, -1);
        // All three suffixes share their introduction, so this selects the
        // same historical prefix rather than one containing `building`.
        let old = historical_prefix(TableId::RailCooldowns, 2, &row);
        assert_eq!(old.building, Recorded::NotRecorded);
        assert_eq!(old.selection, Recorded::NotRecorded);
        assert_eq!(old.selection_hour, Recorded::NotRecorded);
        let missing = round_trip(&Recorded::<Option<RailwaySelectionInputs>>::Value(None));
        let empty = round_trip(&Recorded::Value(Some(RailwaySelectionInputs {
            levels: Some(Vec::new()),
            pair_count: Some(0),
            building_level: Some(None),
        })));
        assert_ne!(missing, empty);
        let unqueried = round_trip(&RailwaySelectionInputs {
            building_level: None,
            ..input.clone()
        });
        let null = round_trip(&RailwaySelectionInputs {
            building_level: Some(None),
            ..input.clone()
        });
        let zero = round_trip(&RailwaySelectionInputs {
            building_level: Some(Some(0)),
            ..input
        });
        assert_ne!(unqueried, null);
        assert_ne!(null, zero);
    }

    #[test]
    fn movement_state_rail_prefixes_and_current_country_deltas_preserve_values() {
        let row = CountryCounterInputRow {
            forward_visibility: crate::rows::Recorded::Value(None),
            country: u32::MAX,
            first_division: u64::MAX,
            movement: Recorded::Value(Retired),
            ..Default::default()
        };
        let next = CountryCounterInputRow {
            owned_state_count: i32::MIN,
            air_combat_modifiers_known: true.into(),
            air_nav_efficiency: Some(i64::MAX).into(),
            ..row.clone()
        };
        let delta = next.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 1) | (1 << 29) | (1 << 33));
        assert_eq!(delta.apply(Some(row)).unwrap(), next);
        assert_eq!(next.movement, Recorded::Value(Retired));
        let row = StateRow {
            id: u32::MAX,
            controller: 71,
            movement_infrastructure: i64::MIN,
            railway_infrastructure: Some(0).into(),
            ..Default::default()
        };
        let decoded = historical_prefix(TableId::States, 7, &row);
        assert_eq!(decoded.railway_infrastructure, Recorded::NotRecorded);
        assert_eq!(decoded.controller, row.controller);
        assert_eq!(decoded.movement_infrastructure, row.movement_infrastructure);
        let row = RailCooldownRow {
            province: u32::MAX,
            cooldown_ticks: i32::MIN,
            building: Some(RailwayBuildingState { building: None }).into(),
            selection: Default::default(),
            selection_hour: Default::default(),
            building_bound: Default::default(),
        };
        let decoded = historical_prefix(TableId::RailCooldowns, 1, &row);
        assert_eq!(decoded.building, Recorded::NotRecorded);
        assert_eq!(decoded.cooldown_ticks, row.cooldown_ticks);
    }

    /// The retired table keeps its published framing, which older recordings
    /// need to discard it.
    #[test]
    fn published_eta_snapshot_and_delta_keep_all_eighteen_fields_and_bytes() {
        let row = DivisionEtaInputsRow {
            division: u64::MAX,
            sample_hour: i32::MIN,
            speed_modifier: i64::MIN,
            speed_base: i64::MAX,
            speed_org: Some(-1),
            speed_fuel: None,
            current_speed: -7,
            terrain_speed: vec![TerrainSpeed {
                terrain: 17,
                modifier: i64::MIN,
            }],
            fort_penalty: -2,
            snow_penalty: -3,
            river_adjustment: -4,
            river_leader: -5,
            class: u32::MAX,
            rail_country_modifier: Some(i64::MIN),
            rail_state_modifier: None,
            rail_friendly_state_modifier: Some(i64::MAX),
            leg_river_combined: vec![i64::MIN, 0, i64::MAX],
            leg_transport_speed: vec![-1, 0, 1],
            leg_rail_speed: vec![7, -7, 0],
        };
        // Accepted99 fields 5/14/15/16 are Opaque: each u32 byte-length
        // envelope precedes the vector's separate u32 element count. Keep
        // this golden independent of the row, Field and Value encoders.
        let mut expected = Vec::new();
        expected.extend_from_slice(&i64::MIN.to_le_bytes()); // 0: speed_modifier
        expected.extend_from_slice(&i64::MAX.to_le_bytes()); // 1: speed_base
        expected.push(1); // 2: speed_org = Some(-1)
        expected.extend_from_slice(&(-1_i64).to_le_bytes());
        expected.push(0); // 3: speed_fuel = None
        expected.extend_from_slice(&(-7_i64).to_le_bytes()); // 4: current_speed
        expected.extend_from_slice(&16_u32.to_le_bytes()); // 5: opaque payload length
        expected.extend_from_slice(&1_u32.to_le_bytes()); // terrain entry count
        expected.extend_from_slice(&17_i32.to_le_bytes());
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        for value in [-2_i64, -3, -4, -5] {
            // 6..9: fort, snow, river, leader
            expected.extend_from_slice(&value.to_le_bytes());
        }
        expected.extend_from_slice(&u32::MAX.to_le_bytes()); // 10: class
        expected.push(1); // 11: rail_country_modifier = Some(MIN)
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        expected.push(0); // 12: rail_state_modifier = None
        expected.push(1); // 13: rail_friendly_state_modifier = Some(MAX)
        expected.extend_from_slice(&i64::MAX.to_le_bytes());
        for values in [[i64::MIN, 0, i64::MAX], [-1, 0, 1], [7, -7, 0]] {
            expected.extend_from_slice(&28_u32.to_le_bytes()); // 14..16: opaque length
            expected.extend_from_slice(&3_u32.to_le_bytes()); // vector element count
            for value in values {
                expected.extend_from_slice(&value.to_le_bytes());
            }
        }
        expected.extend_from_slice(&i32::MIN.to_le_bytes()); // 17: sample_hour
        let mut actual = Vec::new();
        row.encode_fields(&mut actual).unwrap();
        assert_eq!(actual.len(), 209);
        for (offset, length, count) in [
            (34, 16_u32, 1_u32),
            (109, 28, 3),
            (141, 28, 3),
            (173, 28, 3),
        ] {
            assert_eq!(&actual[offset..offset + 4], &length.to_le_bytes());
            assert_eq!(&actual[offset + 4..offset + 8], &count.to_le_bytes());
        }
        assert_eq!(actual, expected);
        let format = crate::format::FormatVersion::read(99).unwrap();
        let fields = format.fields(TableId::DivisionEtaInputs);
        assert_eq!(fields, 18);
        assert_eq!(DivisionEtaInputsRow::SCHEMA.len(), fields);
        assert_eq!(TableId::DivisionEtaInputs.mask_width(), 3);
        let mut input = Decoder::new(&actual);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = DivisionEtaInputsRow::decode_fields(row.key(), &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, row);
        assert_eq!(decoded.not_recorded_fields(), 0);
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(delta.mask, full_mask(18));
        let mut wire = Vec::new();
        encode_delta(TableId::DivisionEtaInputs, &delta, 0, &mut wire).unwrap();
        assert_eq!(
            &wire[..13],
            &[
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01, 0xff, 0xff, 0x03,
            ]
        );
        let mut input = Decoder::new(&wire);
        input.format = format.version();
        input.recorded_fields = fields;
        let decoded = decode_delta(TableId::DivisionEtaInputs, 0, &mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded.mask, full_mask(18));
        assert_eq!(decoded.apply::<DivisionEtaInputsRow>(None).unwrap(), row);
    }
}

mod extended_rows_game_score_tests {
    use super::*;

    fn country(country: u32) -> CountryGameScoreInputRow {
        CountryGameScoreInputRow {
            country,
            army_count: Some(0),
            navy_count: Some(0),
            land_count: Some(0),
            air_state: CountryGameScoreInputRow::AIR_ABSENT,
            ..Default::default()
        }
    }

    #[test]
    fn game_score_missing_counts_and_air_access_never_become_zero() {
        let mut row = country(4999);
        let before = row.clone();
        row.army_count = None;
        row.army_error = b"division header unreadable".to_vec();
        row.air_state = CountryGameScoreInputRow::AIR_PRESENT;
        row.first_sites = vec![GameScoreAirSite {
            base: u64::MAX,
            access: GameScoreAirSite::ACCESS_UNKNOWN,
            error: b"air container unreadable".to_vec(),
        }];
        let delta = row.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.apply(Some(before)).unwrap(), row);
        assert!(
            TableRows::CountryGameScoreInputs(vec![row.clone()])
                .validate_keys()
                .is_ok()
        );
        for bad in [
            CountryGameScoreInputRow {
                army_error: Vec::new(),
                ..row.clone()
            },
            CountryGameScoreInputRow {
                army_count: Some(0),
                ..row.clone()
            },
            CountryGameScoreInputRow {
                air_state: 3,
                ..row.clone()
            },
            CountryGameScoreInputRow {
                air_state: CountryGameScoreInputRow::AIR_ABSENT,
                ..row.clone()
            },
        ] {
            assert!(bad.delta(None).is_err());
        }
        row.first_sites[0].access = 3;
        assert!(row.delta(None).is_err());
    }

    #[test]
    fn game_score_fleet_witnesses_keep_repeats_and_native_wrapping() {
        let row = CountryGameScoreInputRow {
            navy_count: Some(-2),
            fleets: vec![
                GameScoreFleetCount {
                    fleet: u64::MAX,
                    task_forces: Some(i32::MAX),
                    error: Vec::new(),
                };
                2
            ],
            ..country(1)
        };
        assert_eq!(
            row.delta(None)
                .unwrap()
                .unwrap()
                .apply::<CountryGameScoreInputRow>(None)
                .unwrap(),
            row
        );
        assert!(
            CountryGameScoreInputRow {
                navy_count: Some(0),
                ..row.clone()
            }
            .delta(None)
            .is_err()
        );
        let mut missing = row;
        missing.fleets[1].task_forces = None;
        missing.fleets[1].error = b"fleet count unreadable".to_vec();
        assert!(missing.delta(None).is_err());
        missing.navy_count = None;
        missing.navy_error = b"fleet count unreadable".to_vec();
        assert!(missing.delta(None).is_ok());
    }

    #[test]
    fn game_score_multiplier_and_wing_reads_keep_independent_unknowns() {
        let wing = AirWingGameScoreStatsRow {
            id: u64::MAX,
            planes: Some(0),
            stat_error: b"mission stat unreadable".to_vec(),
            mission_index: Some(-1),
            ..Default::default()
        };
        assert_eq!(
            wing.delta(None)
                .unwrap()
                .unwrap()
                .apply::<AirWingGameScoreStatsRow>(None)
                .unwrap(),
            wing
        );
        for index in [i32::from(i8::MIN) - 1, i32::from(i8::MAX) + 1] {
            assert!(
                AirWingGameScoreStatsRow {
                    mission_index: Some(index),
                    ..wing.clone()
                }
                .delta(None)
                .is_err()
            );
        }
        assert!(
            AirWingGameScoreStatsRow {
                mission_index: Some(-2),
                ..wing.clone()
            }
            .delta(None)
            .is_ok()
        );
        assert!(
            AirWingGameScoreStatsRow {
                stat_raw: Some(0),
                ..wing
            }
            .delta(None)
            .is_err()
        );
        let multipliers = GameScoreMultiplierRow {
            total_raw: Some(0),
            army_raw: Some(i64::MIN),
            navy_raw: Some(i64::MAX),
            air_raw: Some(0),
            industry_raw: Some(0),
            land_raw: Some(0),
            ..Default::default()
        };
        assert!(multipliers.delta(None).is_ok());
        assert!(
            GameScoreMultiplierRow {
                id: 1,
                ..multipliers.clone()
            }
            .delta(None)
            .is_err()
        );
        assert!(
            GameScoreMultiplierRow {
                land_raw: None,
                ..multipliers
            }
            .delta(None)
            .is_err()
        );
    }
}

mod extended_rows_map_presentation_fresh_railway_eta_tests {
    use super::*;

    #[test]
    fn fresh_packet_keeps_refused_zero_and_signed_extreme_operands_distinct() {
        for inputs in [
            FreshRailwayCounterEta {
                sample_hour: PacketHour::Host,
                maximum_speed: Some(i64::MIN),
                land_supply_factor: Some(i64::MAX),
                orders_group: Some(u64::MAX),
            },
            FreshRailwayCounterEta {
                sample_hour: PacketHour::Host,
                maximum_speed: Some(0),
                land_supply_factor: None,
                orders_group: Some(0),
            },
            FreshRailwayCounterEta::default(),
        ] {
            let mut bytes = Vec::new();
            inputs.encode(&mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                crate::value::Value::encoded_bytes(&inputs).unwrap()
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(FreshRailwayCounterEta::decode(&mut input).unwrap(), inputs);
            input.finish().unwrap();
            assert!(
                FreshRailwayCounterEta::decode(&mut Decoder::new(&bytes[..bytes.len() - 1]))
                    .is_err()
            );
        }
        assert!(FreshRailwayCounterEta::decode(&mut Decoder::new(&[2])).is_err());
    }

    #[test]
    fn only_fresh_suffix_changes_and_an_explicit_refusal_clears_it() {
        let previous = RailwayGunCounterInputRow {
            gun: u64::MAX,
            name: Some(vec![0xff]).into(),
            eta: Some(RailwayGunEtaOperands {
                numerator: i64::MIN,
                current_speed: 7,
            })
            .into(),
            supply: None.into(),
            withdrawal: Some(0).into(),
            fresh: None.into(),
            combat_count: 1,
            combat_references: Some(UnitCombatReferences {
                sample_hour: PacketHour::Host,
                entries: vec![UnitCombatReference {
                    combat: u64::MAX,
                    kind: 0,
                }],
            })
            .into(),
            ..Default::default()
        };
        let row = RailwayGunCounterInputRow {
            fresh: Some(FreshRailwayCounterEta {
                sample_hour: PacketHour::Host,
                maximum_speed: Some(0),
                land_supply_factor: None,
                orders_group: Some(u64::MAX),
            })
            .into(),
            ..previous.clone()
        };
        assert_eq!(RailwayGunCounterInputRow::SCHEMA.len(), 30);
        assert_eq!(TableId::RailwayGunCounterInputs.mask_width(), 4);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            RailwayGunCounterInputRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        for (old, new) in [(&previous, &row), (&row, &previous)] {
            let delta = new.delta(Some(old)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 27);
            assert_eq!(delta.apply(Some(old.clone())).unwrap(), *new);
        }
        let historical = RailwayGunCounterInputRow {
            fresh: Recorded::NotRecorded,
            ..row
        };
        assert_ne!(historical.fresh, previous.fresh);
        assert!(historical.encode_fields(&mut Vec::new()).is_err());
    }
}

#[path = "../extended_rows/map_presentation/tests.rs"]
mod extended_rows_map_presentation_weather_tests;

#[path = "../extended_rows/map_presentation/launch_sources_tests.rs"]
mod extended_rows_map_presentation_launch_sources_tests;

#[path = "../extended_rows/map_presentation/reverse_zero_tests.rs"]
mod extended_rows_map_presentation_reverse_zero_tests;

#[path = "../extended_rows/map_presentation/forward_visibility_tests.rs"]
mod extended_rows_map_presentation_forward_visibility_tests;

mod extended_rows_market_tests {
    use super::*;
    #[test]
    fn shared_market_values_keep_underlying_prepared_codecs() {
        use crate::checkpoint::values::CheckpointValue;
        fn check<T>(actual: T, expected: &impl CheckpointValue)
        where
            T: CheckpointValue + PartialEq + std::fmt::Debug,
        {
            let mut bytes = Vec::new();
            CheckpointValue::encode(&actual, &mut bytes).unwrap();
            let mut underlying = Vec::new();
            CheckpointValue::encode(expected, &mut underlying).unwrap();
            assert_eq!(bytes, underlying);
            let mut input = Decoder::new(&bytes);
            assert_eq!(T::decode(&mut input).unwrap(), actual);
            assert_eq!(input.remaining(), 0);
            assert!(T::decode(&mut Decoder::new(&bytes[..bytes.len() - 1])).is_err());
        }
        let pool = MarketPool {
            allow_zero: true,
            items: vec![MarketEquipment {
                variant: u64::MAX,
                amount_raw: -1,
            }],
        };
        check(SharedMarketPool::from(pool.clone()), &pool);
        let prices = vec![MarketPriceLevel {
            variant: 71,
            level: 2,
        }];
        check(SharedMarketPriceLevels::from(prices.clone()), &prices);
        let subsidies = vec![MarketSubsidy {
            cic_raw: 17,
            archetype: 71,
            kind: 1,
            targets: vec![5000],
            trigger: b"synthetic".to_vec(),
        }];
        check(SharedMarketSubsidies::from(subsidies.clone()), &subsidies);
        let variables = vec![MarketVariable {
            name: b"synthetic".to_vec(),
            amount_raw: i64::MAX,
        }];
        check(SharedMarketVariables::from(variables.clone()), &variables);
    }

    #[test]
    fn contract_and_country_scalar_clones_share_all_carried_market_collections() {
        use crate::rows::RowCodec;
        let pool: SharedMarketPool = MarketPool {
            items: vec![
                MarketEquipment {
                    variant: 31,
                    amount_raw: 99
                };
                5000
            ],
            ..Default::default()
        }
        .into();
        let levels: SharedMarketPriceLevels = vec![
            MarketPriceLevel {
                variant: 31,
                level: 2
            };
            5000
        ]
        .into();
        let subsidies: SharedMarketSubsidies = vec![MarketSubsidy {
            targets: vec![5000; 5000],
            ..Default::default()
        }]
        .into();
        let variables: SharedMarketVariables = vec![MarketVariable {
            name: vec![b'x'; 5000],
            amount_raw: 17,
        }]
        .into();
        let old = MarketContractTermsRow {
            equipment: pool.clone(),
            prices: pool.clone(),
            price_levels: levels.clone(),
            subsidies: subsidies.clone(),
            variables: variables.clone(),
            runtime: MarketContractRuntime {
                route_equipment: pool.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut next = RowCodec::try_clone(&old).unwrap();
        next.runtime.period_days += 1;
        assert_eq!(next.equipment.items.as_ptr(), pool.items.as_ptr());
        assert_eq!(next.prices.items.as_ptr(), pool.items.as_ptr());
        assert_eq!(
            next.runtime.route_equipment.items.as_ptr(),
            pool.items.as_ptr()
        );
        assert_eq!(next.price_levels.as_ptr(), levels.as_ptr());
        assert_eq!(next.subsidies.as_ptr(), subsidies.as_ptr());
        assert_eq!(next.variables.as_ptr(), variables.as_ptr());
        next.runtime.route_equipment.items[0].amount_raw = 7;
        assert_eq!(old.runtime.route_equipment.items[0].amount_raw, 99);
        let country = CountryMarketRow {
            inventory: pool.clone(),
            prices: levels.clone(),
            subsidies: subsidies.clone(),
            ..Default::default()
        };
        let mut changed = RowCodec::try_clone(&country).unwrap();
        changed.auto_send_market_access = true;
        assert_eq!(changed.inventory.items.as_ptr(), pool.items.as_ptr());
        assert_eq!(changed.prices.as_ptr(), levels.as_ptr());
        assert_eq!(changed.subsidies.as_ptr(), subsidies.as_ptr());
    }

    #[test]
    fn delivery_refusals_round_trip_with_holder_identity_and_named_reason() {
        for mode in [3, 4, 20] {
            for ordinal in 1..=5 {
                let rows = TableRows::MarketEvents(vec![MarketEventRow {
                    id: 1,
                    kind: 34,
                    mode,
                    ordinal,
                    country: 5000,
                    agreement: u64::MAX,
                    ..Default::default()
                }]);
                let mut bytes = Vec::new();
                rows.encode_rows(&mut bytes).unwrap();
                let decoded = crate::rows::TableId::MarketEvents
                    .decode_rows(1, &mut Decoder::new(&bytes))
                    .unwrap();
                assert_eq!(decoded, rows);
                let TableRows::MarketEvents(rows) = decoded else {
                    unreachable!()
                };
                assert!(
                    rows[0]
                        .delivery_unavailable_reason()
                        .unwrap()
                        .starts_with("Delivery amount unavailable:")
                );
            }
        }
    }
    #[test]
    fn contract_scalar_tick_has_an_independent_delta_from_the_route_pool() {
        use crate::rows::RowCodec;
        for count in [0, 5000] {
            let old = MarketContractTermsRow {
                country: 5000,
                id: 17,
                kind: 1,
                runtime: MarketContractRuntime {
                    route_equipment: MarketPool {
                        items: (0..count)
                            .map(|variant| MarketEquipment {
                                variant,
                                amount_raw: 99,
                            })
                            .collect(),
                        ..Default::default()
                    }
                    .into(),
                    period_days: 8,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut next = old.clone();
            next.runtime.period_days += 1;
            let delta = next.delta(Some(&old)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 11);
            assert_eq!(
                delta.values.len(),
                1,
                "scalar tick serialized the route manifest"
            );
            assert_eq!(delta.apply(Some(old)).unwrap(), next);
            let route_old = next.clone();
            next.runtime.route_equipment.allow_zero = true;
            let delta = next.delta(Some(&route_old)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 6);
            assert_eq!(delta.apply(Some(route_old)).unwrap(), next);
        }
    }
    #[test]
    fn agreement_scalar_clone_shares_equipment_until_an_equipment_write() {
        use crate::rows::RowCodec;
        let old = LendLeaseAgreementRow {
            monthly: MarketPool {
                items: vec![
                    MarketEquipment {
                        variant: 31,
                        amount_raw: 99
                    };
                    5000
                ],
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        let mut next = RowCodec::try_clone(&old).unwrap();
        next.fuel_sent_raw += 1;
        assert!(std::sync::Arc::ptr_eq(&old.monthly.0, &next.monthly.0));
        next.monthly.items[0].amount_raw = 17;
        assert!(!std::sync::Arc::ptr_eq(&old.monthly.0, &next.monthly.0));
        assert_eq!(old.monthly.items[0].amount_raw, 99);
    }
    #[test]
    fn market_round_trip_preserves_distinct_terms_and_transient_occurrences() {
        let pool = MarketPool {
            allow_zero: true,
            items: vec![
                MarketEquipment {
                    variant: 3_u64 << 32 | 31,
                    amount_raw: -7,
                },
                MarketEquipment {
                    variant: 2_u64 << 32 | 31,
                    amount_raw: 9_000_000_017,
                },
                MarketEquipment {
                    variant: 3_u64 << 32 | 31,
                    amount_raw: 0,
                },
            ],
        };
        let tables = [
            TableRows::CountryMarket(vec![CountryMarketRow {
                country: 5_000,
                inventory: pool.clone().into(),
                prices: vec![MarketPriceLevel {
                    variant: 3_u64 << 32 | 31,
                    level: 1,
                }]
                .into(),
                subsidies: vec![MarketSubsidy {
                    cic_raw: -1,
                    archetype: 123,
                    kind: 0,
                    targets: vec![5_000, 1, 5_000],
                    trigger: vec![],
                }]
                .into(),
                subsidy_total_raw: 17,
                auto_send_market_access: true,
                ..Default::default()
            }]),
            TableRows::MarketContractTerms(vec![MarketContractTermsRow {
                country: 5_000,
                id: u64::MAX,
                kind: 1,
                buyer: 4_999,
                speed: 7,
                runtime: MarketContractRuntime {
                    route_equipment: pool.clone().into(),
                    route_length: -7,
                    route_type: 254,
                    route_request: i32::MAX,
                    route_sunk_convoys_raw: i64::MIN,
                    period_days: 19,
                    meta_cic_raw: i64::MAX,
                    meta_convoys_raw: -9,
                    meta_efficiency_raw: 100001,
                    history_factory_raw: 3,
                    history_equipment_raw: 4,
                    history_collected_raw: 5,
                },
                variables: vec![
                    MarketVariable {
                        name: b"random".to_vec(),
                        amount_raw: -1,
                    },
                    MarketVariable {
                        name: Vec::new(),
                        amount_raw: 0,
                    },
                ]
                .into(),
                variable_state: u64::MAX,
                equipment: pool.clone().into(),
                prices: MarketPool {
                    allow_zero: false,
                    items: vec![MarketEquipment {
                        variant: 3_u64 << 32 | 31,
                        amount_raw: 12345,
                    }],
                }
                .into(),
                ..Default::default()
            }]),
            TableRows::LendLeaseAgreements(vec![LendLeaseAgreementRow {
                country: 5_000,
                id: u64::MAX,
                receiver: 4_999,
                revision: 123,
                monthly: pool.clone().into(),
                fuel_daily_raw: 32_769,
                fuel_percentage_raw: -10_001,
                ..Default::default()
            }]),
            TableRows::MarketEvents(vec![MarketEventRow {
                id: 17,
                occurrence: 15,
                agreement: u64::MAX,
                country: 5_000,
                kind: 3,
                variant: 3_u64 << 32 | 31,
                amount_raw: -7,
                ..Default::default()
            }]),
        ];
        for rows in tables {
            let mut bytes = Vec::new();
            rows.encode_rows(&mut bytes).unwrap();
            let mut decoder = crate::codec::Decoder::new(&bytes);
            assert_eq!(
                rows.table().decode_rows(rows.len(), &mut decoder).unwrap(),
                rows
            );
            decoder.finish().unwrap();
        }
    }
}

#[path = "../extended_rows/movement/order_children_tests.rs"]
mod extended_rows_movement_order_children_tests;

#[path = "../extended_rows/movement/order_execution_tests.rs"]
mod extended_rows_movement_order_execution_tests;

#[path = "../extended_rows/naval_weather/tests.rs"]
mod extended_rows_naval_weather_tests;

#[path = "../extended_rows/unit_combat_references/tests.rs"]
mod extended_rows_unit_combat_references_tests;
