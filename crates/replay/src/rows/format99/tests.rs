use super::*;
use crate::{
    cadence::{Cadence, cadence},
    rows::compatibility::{EFFECT_INPUTS_FORMAT, RECORDER_INPUTS_FORMAT},
};

fn check98<T: RowCodec + Fields + std::fmt::Debug + PartialEq>(table: TableId, row: T) {
    let format = FormatVersion::read(RECORDER_INPUTS_FORMAT).unwrap();
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
    let gaps = table
        .last_change()
        .appended
        .iter()
        .filter(|field| field.introduced > format.version())
        .fold(0, |mask, field| mask | (1 << field.index));
    assert_eq!(decoded.not_recorded_fields(), gaps);
    assert_eq!(decoded.recorded_fields_bytes().unwrap(), prefix.len());
    assert!(decoded.encode_fields(&mut Vec::new()).is_err());
    assert!(decoded.delta(None).is_err());

    let delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    for kind in &T::SCHEMA[..fields] {
        kind.decode_change(&mut input).unwrap();
    }
    let mut bytes = Vec::new();
    table.encode_key(row.key(), 0, &mut bytes).unwrap();
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
    bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
    let mut input = Decoder::new(&bytes);
    input.format = format.version();
    input.recorded_fields = fields;
    let delta = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(delta.apply::<T>(None).unwrap(), decoded);
}

#[test]
fn format99_appends_leave_format98_prefixes_and_values_available() {
    check98(
        TableId::CountryAppearance,
        CountryAppearanceRow {
            country: u32::MAX,
            name_tag: b"GENERATOR".to_vec(),
            original_tag: Recorded::Value(Some(vec![0xff, 0, b'O'])),
            original_tag_reference: Recorded::Value(Some(i32::MIN)),
            ..Default::default()
        },
    );
    check98(
        TableId::UnitLeaders,
        UnitLeaderRow {
            id: u32::MAX,
            reference_type: 4713,
            category: 1,
            kind: engine::cunit_leader::KIND_ARMY,
            formation_reference: Recorded::Value(Some(u64::MAX)),
            ..Default::default()
        },
    );
    check98(
        TableId::DivisionTemplateIcons,
        DivisionTemplateIconRow {
            template: u64::MAX,
            priority_unit_name: vec![0xff, 0, b'P'],
            custom_icon: i32::MIN,
            formation_division_count: Recorded::Value(Some(i32::MIN)),
            ..Default::default()
        },
    );
    check98(
        TableId::RailwayGunCounterInputs,
        RailwayGunCounterInputRow {
            gun: (5000 << 32) | 4713,
            current_strength: i64::MIN,
            ordinary_path: vec![u32::MAX, 0],
            name: Recorded::Value(Some(vec![0xff, 0, b'G'])),
            eta: Recorded::Value(Some(RailwayGunEtaOperands {
                numerator: i64::MIN,
                current_speed: i64::MAX,
            })),
            supply: Recorded::Value(Some(UnitSupplyInputs {
                asked: i64::MIN,
                disruption: i64::MAX,
                origins: Some(vec![UnitSupplyOrigin {
                    province: -1,
                    requested: i64::MIN,
                    received: i64::MAX,
                }]),
                ..Default::default()
            })),
            ..Default::default()
        },
    );
    check98(
        TableId::AirWings,
        AirWingRow {
            id: (7 << 32) | 69,
            planes: -1,
            pool_ordinal: 5000,
            name: Recorded::Value(Some(vec![0xff, 0, b'W'])),
            ..Default::default()
        },
    );
    check98(
        TableId::AdvisorRoles,
        AdvisorRoleRow {
            character: 7,
            idea: vec![0xff, 0],
            modifier_cache: Recorded::Value(Some(AdvisorModifierCache {
                entries: vec![CachedModifier {
                    definition: 3,
                    value: i64::MIN,
                }],
            })),
            tooltip_keys: Recorded::Value(Some(vec![vec![0xff, 0, b'K']])),
            ..Default::default()
        },
    );
    check98(
        TableId::CountryCounterInputs,
        CountryCounterInputRow {
            country: 5000,
            owned_states: Recorded::Value(Some(vec![2, 1, 2])),
            mio_policies: Recorded::Value(Some(vec![u32::MAX, 0])),
            neighbors: Recorded::Value(Some(vec![5001, 0, 5001])),
            ..Default::default()
        },
    );
    check98(
        TableId::SimulationFeatures,
        SimulationFeatureRow {
            human_ai: 0xff,
            weather_suspend: Recorded::Value(Some([2, 3])),
            instant_decisions: Recorded::Value(Some(0xfe)),
            ..Default::default()
        },
    );
    check98(
        TableId::OccupationSettings,
        OccupationSettingsRow {
            country: 5000,
            law: vec![0xff, 0],
            occupied: Recorded::Value(Some(vec![5002, 5001, 5002])),
            ..Default::default()
        },
    );
    check98(
        TableId::DivisionStatus,
        DivisionStatusRow {
            division: u64::MAX,
            supply_grace_remaining: Recorded::Value(i64::MIN),
            country_ordinal: Recorded::Value(Some(u32::MAX)),
            invalid_orders_reason: Recorded::Value(Some(i32::MIN)),
            supply: Recorded::Value(Some(UnitSupplyInputs {
                received: i64::MAX,
                disruption: i64::MIN,
                origins: None,
                ..Default::default()
            })),
            attrition_inputs: Recorded::Value(ArmyAttritionInputs {
                upkeep: Some(i64::MIN),
                terrain: Some(Some(AttritionTerrain {
                    name: Some(vec![0xff, 0, b'T']),
                    attrition: Some(i64::MAX),
                    equipment: None,
                })),
                border: Some(Some(AttritionBorder {
                    state: None,
                    country: Some(u32::MAX),
                    factor: Some(-1),
                })),
                ..Default::default()
            }),
            dig_in_factor: Recorded::Value(Some(i64::MIN)),
            name: Recorded::Value(Some(vec![0xff, 0, b'C'])),
            ..Default::default()
        },
    );
    check98(
        TableId::OrdersGroupCounterInputs,
        OrdersGroupCounterInputRow {
            group: u64::MAX,
            withdrawing: 0xff,
            hq_distance: Recorded::Value(Some(i32::MIN)),
            leader_enroute_days: Recorded::Value(Some(i32::MAX)),
            border_war_presence: Recorded::Value(Some(0xff)),
            invasion_source_distance: Recorded::Value(Some(i32::MIN)),
        },
    );
    check98(
        TableId::Scientists,
        ScientistRow {
            character: 7,
            country: 5000,
            trait_count: i32::MIN,
            active: Recorded::Value(Some(0xff)),
            ..Default::default()
        },
    );
    check98(
        TableId::CountryLeaderRoles,
        CountryLeaderRoleRow {
            character: 7,
            country: 5000,
            ideology: i32::MIN,
            current_ideology: Recorded::Value(Some(Some(u32::MAX))),
        },
    );
}

#[test]
fn format99_new_tables_have_independent_lifetimes_and_native_cadences() {
    let old = FormatVersion::read(RECORDER_INPUTS_FORMAT).unwrap();
    for table in [
        TableId::TimedIdeas,
        TableId::DecisionRandomItems,
        TableId::CapturedOperativeReferences,
        TableId::ReservedCountryNaming,
        TableId::AttritionModifierBreakdowns,
    ] {
        assert_eq!(table.last_change().introduced, EFFECT_INPUTS_FORMAT);
        assert!(old.table_refusal(table).unwrap().contains("not recorded"));
        assert!(FormatVersion::CURRENT.table_refusal(table).is_none());
    }
    assert_eq!(cadence(TableId::TimedIdeas), Cadence::Hourly);
    assert_eq!(cadence(TableId::DecisionRandomItems), Cadence::Daily(6));
    assert_eq!(
        cadence(TableId::CapturedOperativeReferences),
        Cadence::Daily(16)
    );
    assert_eq!(cadence(TableId::ReservedCountryNaming), Cadence::Hourly);
    assert_eq!(
        cadence(TableId::AttritionModifierBreakdowns),
        Cadence::Hourly
    );
    assert_eq!(cadence(TableId::AirWings), Cadence::Hourly);
    assert_eq!(old.fields(TableId::AirWings), 15);
    assert_eq!(old.fields(TableId::AdvisorRoles), 5);
    assert_eq!(old.fields(TableId::CountryCounterInputs), 9);
    assert_eq!(old.fields(TableId::DivisionStatus), 20);
    assert_eq!(old.fields(TableId::OrdersGroupCounterInputs), 1);
    assert_eq!(old.fields(TableId::UnitLeaders), 22);
    assert_eq!(old.fields(TableId::DivisionTemplateIcons), 13);
    assert_eq!(old.fields(TableId::SimulationFeatures), 3);
    assert_eq!(
        old.fields(TableId::Decisions),
        TableId::Decisions.field_count()
    );
}

#[test]
fn format99_ability_operands_preserve_null_and_signed_values() {
    fn round_trip<T: RowCodec + Fields + std::fmt::Debug + PartialEq>(row: T) {
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<T>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
    }
    for formation_reference in [None, Some(0), Some(u64::MAX)] {
        round_trip(UnitLeaderRow {
            id: 7,
            reference_type: 4713,
            category: 0,
            kind: engine::cunit_leader::KIND_ARMY,
            formation_reference: Recorded::Value(formation_reference),
            ..Default::default()
        });
    }
    for operand in [None, Some(i32::MIN), Some(-7), Some(0), Some(i32::MAX)] {
        round_trip(DivisionTemplateIconRow {
            template: u64::MAX,
            model_unreadable: 1,
            formation_division_count: Recorded::Value(operand),
            ..Default::default()
        });
        round_trip(OrdersGroupCounterInputRow {
            group: u64::MAX,
            hq_distance: Recorded::Value(Some(17)),
            invasion_source_distance: Recorded::Value(operand),
            ..Default::default()
        });
    }
}

#[test]
fn format99_current_ideology_distinguishes_null_tokens_and_refusal() {
    let old = CountryLeaderRoleRow {
        character: 7,
        country: 5000,
        ideology: -7,
        current_ideology: Recorded::Value(Some(Some(11))),
    };
    for current in [None, Some(None), Some(Some(0)), Some(Some(u32::MAX))] {
        let row = CountryLeaderRoleRow {
            current_ideology: Recorded::Value(current),
            ..old.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<CountryLeaderRoleRow>(0, &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        let delta = row.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 1);
        assert_eq!(delta.apply(Some(old.clone())).unwrap(), row);
        check98(TableId::CountryLeaderRoles, row);
    }
}

#[test]
fn format99_invalid_order_reason_preserves_signed_bits_and_field_local_gaps() {
    let old = DivisionStatusRow {
        division: u64::MAX,
        invalid_orders: 0xff,
        supply_grace_maximum: Recorded::Value(i64::MAX),
        invalid_orders_reason: Recorded::Value(Some(42)),
        ..Default::default()
    };
    for reason in [
        None,
        Some(i32::MIN),
        Some(-1),
        Some(0),
        Some(7),
        Some(8),
        Some(i32::MAX),
    ] {
        let row = DivisionStatusRow {
            invalid_orders_reason: Recorded::Value(reason),
            ..old.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<DivisionStatusRow>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
        let delta = row.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 21);
        assert_eq!(delta.apply(Some(old.clone())).unwrap(), row);
        check98(TableId::DivisionStatus, row);
    }
}

#[test]
fn format99_hq_distance_preserves_negative_words_and_legacy_withdrawing() {
    let old = OrdersGroupCounterInputRow {
        group: u64::MAX,
        withdrawing: 0xff,
        hq_distance: Recorded::Value(Some(42)),
        ..Default::default()
    };
    for distance in [
        None,
        Some(i32::MIN),
        Some(-7),
        Some(-1),
        Some(0),
        Some(i32::MAX),
    ] {
        let row = OrdersGroupCounterInputRow {
            hq_distance: Recorded::Value(distance),
            ..old.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<OrdersGroupCounterInputRow>(0, &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        let delta = row.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 1);
        assert_eq!(delta.apply(Some(old.clone())).unwrap(), row);
        check98(TableId::OrdersGroupCounterInputs, row);
    }
}

#[test]
fn format99_tooltip_keys_preserve_string_bytes_slot_order_and_refusals() {
    let old = AdvisorRoleRow {
        character: 5000,
        modifier_cache: Recorded::Value(Some(AdvisorModifierCache::default())),
        tooltip_keys: Recorded::Value(Some(vec![b"previous".to_vec()])),
        ..Default::default()
    };
    for keys in [
        Some(vec![vec![0xff, 0, b'B'], vec![], vec![b'A'], vec![b'A']]),
        Some(vec![]),
        None,
    ] {
        let row = AdvisorRoleRow {
            tooltip_keys: Recorded::Value(keys),
            ..old.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<AdvisorRoleRow>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
        let delta = row.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 5);
        assert_eq!(delta.apply(Some(old.clone())).unwrap(), row);
    }
}

#[test]
fn format99_rosters_round_trip_at_5000_countries_with_raw_extremes() {
    let timed = (1..=5000)
        .map(|country| TimedIdeaRow {
            country,
            ordinal: u32::MAX,
            token: vec![0xff, 0, country as u8],
            days: i32::MIN,
        })
        .collect();
    let random = (1..=5000)
        .map(|country| DecisionRandomItemRow {
            country,
            ordinal: 3,
            token: vec![0xff, 0, country as u8],
            count: i32::MIN,
            target: u32::MAX,
        })
        .collect();
    let captured = (1..=5000)
        .map(|country| CapturedOperativeReferenceRow {
            country,
            ordinal: 2,
            operative: u64::MAX,
            owner: (country % 2 == 0).then_some(u32::MAX),
        })
        .collect();
    let countries = (1..=5000)
        .map(|country| CountryCounterInputRow {
            country,
            mio_policies: Recorded::Value(Some(vec![u32::MAX, 0, u32::MAX])),
            neighbors: Recorded::Value(Some(vec![country + 1, country, country + 1])),
            ..Default::default()
        })
        .collect();
    for rows in [
        TableRows::TimedIdeas(timed),
        TableRows::DecisionRandomItems(random),
        TableRows::CapturedOperativeReferences(captured),
        TableRows::CountryCounterInputs(countries),
    ] {
        let mut bytes = Vec::new();
        rows.encode_rows(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(rows.table().decode_rows(5000, &mut input).unwrap(), rows);
        input.finish().unwrap();
    }
}

#[test]
fn format99_stored_days_change_only_the_timed_idea_counter() {
    let old = TimedIdeaRow {
        country: 5000,
        token: vec![0xff, 0],
        days: i32::MAX,
        ..Default::default()
    };
    let next = TimedIdeaRow {
        days: -1,
        ..old.clone()
    };
    let delta = next.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 1);
    assert_eq!(delta.apply(Some(old)).unwrap(), next);
}

#[test]
fn format99_reserved_naming_is_a_validated_singleton() {
    let row = ReservedCountryNamingRow {
        tag: vec![0xff, 0],
        group: Some(vec![]),
        overlord: i32::MIN,
        built: 7,
        original_tag: Recorded::Value(Some(vec![0xff, 0, b'R'])),
        original_tag_reference: Recorded::Value(Some(i32::MAX)),
        original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![7, 0, 7, u32::MAX]])),
        country_array: Recorded::Value(Some(vec![0, 7, 0, 7, u32::MAX])),
        owned_state_count: Recorded::Value(Some(i32::MIN)),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    assert_eq!(row.key(), 0);
    assert_eq!(TableId::ReservedCountryNaming.key_bits(), 0);
    assert!(
        ReservedCountryNamingRow { built: 8, ..row }
            .delta(None)
            .is_err()
    );
}

#[test]
fn format99_reserved_original_tag_retains_presence_states_and_suffix_delta() {
    let row = ReservedCountryNamingRow {
        tag: b"ZERO".to_vec(),
        built: 7,
        original_tag: Recorded::Value(Some(vec![0xff, 0])),
        ..Default::default()
    };
    for tag in [None, Some(Vec::new()), Some(vec![0xff, 0, b'R'])] {
        let next = ReservedCountryNamingRow {
            original_tag: Recorded::Value(tag),
            ..row.clone()
        };
        let delta = next.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 8);
        assert_eq!(delta.apply(Some(row.clone())).unwrap(), next);
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
            next
        );
        input.finish().unwrap();
    }
    let gap = ReservedCountryNamingRow {
        original_tag: Recorded::NotRecorded,
        ..row
    };
    assert_eq!(gap.not_recorded_fields(), 1 << 8);
    assert!(gap.encode_fields(&mut Vec::new()).is_err());
    assert_eq!(
        FormatVersion::CURRENT.fields(TableId::ReservedCountryNaming),
        13
    );
    let field = TableId::ReservedCountryNaming.last_change().appended[0];
    assert_eq!(
        (field.index, field.name, field.introduced),
        (8, "original_tag", EFFECT_INPUTS_FORMAT)
    );
    assert!(
        FormatVersion::read(RECORDER_INPUTS_FORMAT)
            .unwrap()
            .table_refusal(TableId::ReservedCountryNaming)
            .unwrap()
            .contains("not recorded")
    );
}

#[test]
fn format99_positive_original_tag_changes_only_suffix_and_retains_presence() {
    let row = CountryAppearanceRow {
        country: u32::MAX,
        name_tag: b"GENERATOR".to_vec(),
        original_tag: Recorded::Value(Some(b"OWN".to_vec())),
        ..Default::default()
    };
    for tag in [None, Some(Vec::new()), Some(b"OTHER".to_vec())] {
        let next = CountryAppearanceRow {
            original_tag: Recorded::Value(tag),
            ..row.clone()
        };
        let delta = next.delta(Some(&row)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 19);
        assert_eq!(delta.apply(Some(row.clone())).unwrap(), next);
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<CountryAppearanceRow>(0, &mut input).unwrap(),
            next
        );
        input.finish().unwrap();
    }
    assert_eq!(
        FormatVersion::CURRENT.fields(TableId::CountryAppearance),
        21
    );
    assert_eq!(
        FormatVersion::read(RECORDER_INPUTS_FORMAT)
            .unwrap()
            .fields(TableId::CountryAppearance),
        19
    );
}

#[test]
fn format99_raw_original_references_and_actual_buckets_keep_independent_masks() {
    let positive = CountryAppearanceRow {
        country: 7,
        original_tag: Recorded::Value(Some(Vec::new())),
        original_tag_reference: Recorded::Value(Some(9)),
        ..Default::default()
    };
    for reference in [None, Some(i32::MIN), Some(-1), Some(0), Some(i32::MAX)] {
        let next = CountryAppearanceRow {
            original_tag_reference: Recorded::Value(reference),
            ..positive.clone()
        };
        let delta = next.delta(Some(&positive)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 20);
        assert_eq!(delta.apply(Some(positive.clone())).unwrap(), next);
    }
    let gap = CountryAppearanceRow {
        original_tag_reference: Recorded::NotRecorded,
        ..positive
    };
    assert_eq!(gap.not_recorded_fields(), 1 << 20);
    assert!(gap.encode_fields(&mut Vec::new()).is_err());

    let reserved = ReservedCountryNamingRow {
        tag: b"ZERO".to_vec(),
        original_tag: Recorded::Value(Some(Vec::new())),
        original_tag_reference: Recorded::Value(Some(9)),
        original_tag_buckets: Recorded::Value(Some(vec![vec![], vec![7, 0, 7, u32::MAX]])),
        country_array: Recorded::Value(Some(vec![0, 7, 7, 0])),
        ..Default::default()
    };
    for reference in [None, Some(i32::MIN), Some(0), Some(i32::MAX)] {
        let next = ReservedCountryNamingRow {
            original_tag_reference: Recorded::Value(reference),
            ..reserved.clone()
        };
        let delta = next.delta(Some(&reserved)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 9);
        assert_eq!(delta.apply(Some(reserved.clone())).unwrap(), next);
    }
    for buckets in [
        None,
        Some(vec![]),
        Some(vec![vec![], vec![7, 7, 0, u32::MAX]]),
    ] {
        let next = ReservedCountryNamingRow {
            original_tag_buckets: Recorded::Value(buckets),
            ..reserved.clone()
        };
        let delta = next.delta(Some(&reserved)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 10);
        assert_eq!(delta.apply(Some(reserved.clone())).unwrap(), next);
        let mut encoded = Vec::new();
        encode_row(&next, 0, &mut encoded).unwrap();
        let mut input = Decoder::new(&encoded);
        assert_eq!(
            decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
            next
        );
        input.finish().unwrap();
    }
    for array in [None, Some(vec![]), Some(vec![0, 7, 0, 7, u32::MAX])] {
        let next = ReservedCountryNamingRow {
            country_array: Recorded::Value(array),
            ..reserved.clone()
        };
        let delta = next.delta(Some(&reserved)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 11);
        assert_eq!(delta.apply(Some(reserved.clone())).unwrap(), next);
        let mut encoded = Vec::new();
        encode_row(&next, 0, &mut encoded).unwrap();
        let mut input = Decoder::new(&encoded);
        assert_eq!(
            decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
            next
        );
        input.finish().unwrap();
    }
    let gap = ReservedCountryNamingRow {
        original_tag_reference: Recorded::NotRecorded,
        original_tag_buckets: Recorded::NotRecorded,
        country_array: Recorded::NotRecorded,
        ..reserved
    };
    assert_eq!(gap.not_recorded_fields(), (1 << 9) | (1 << 10) | (1 << 11));
    assert!(gap.encode_fields(&mut Vec::new()).is_err());
}

#[test]
fn format99_reserved_owned_state_count_retains_signed_zero_failure_and_gap() {
    let previous = ReservedCountryNamingRow {
        tag: b"ZERO".to_vec(),
        original_tag_reference: Recorded::Value(Some(9)),
        owned_state_count: Recorded::Value(Some(5)),
        ..Default::default()
    };
    for count in [None, Some(i32::MIN), Some(-1), Some(0), Some(i32::MAX)] {
        let next = ReservedCountryNamingRow {
            owned_state_count: Recorded::Value(count),
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 12);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
        let mut encoded = Vec::new();
        encode_row(&next, 0, &mut encoded).unwrap();
        let mut input = Decoder::new(&encoded);
        assert_eq!(
            decode_row::<ReservedCountryNamingRow>(0, &mut input).unwrap(),
            next
        );
        input.finish().unwrap();
    }
    let gap = ReservedCountryNamingRow {
        owned_state_count: Recorded::NotRecorded,
        ..previous
    };
    assert_eq!(gap.not_recorded_fields(), 1 << 12);
    assert!(gap.encode_fields(&mut Vec::new()).is_err());
    assert!(
        TableId::ReservedCountryNaming
            .last_change()
            .appended
            .iter()
            .any(|field| field.index == 12
                && field.name == "owned_state_count"
                && field.introduced == EFFECT_INPUTS_FORMAT)
    );
}
