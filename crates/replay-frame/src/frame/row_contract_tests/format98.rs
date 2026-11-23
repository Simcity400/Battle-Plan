use super::*;
use crate::{
    cadence::{Cadence, cadence},
    frame::{Frame, FrameDecoder, TableUpdate, TableView},
};

fn format97() -> FormatVersion {
    FormatVersion::read(97).unwrap()
}

fn decoder97(bytes: &[u8], fields: usize) -> Decoder<'_> {
    let mut input = Decoder::new(bytes);
    input.format = 97;
    input.recorded_fields = fields;
    input
}

fn fields97<T: RowCodec + Fields>(table: TableId, row: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    for kind in &T::SCHEMA[..format97().fields(table)] {
        kind.decode(&mut input).unwrap();
    }
    bytes[..bytes.len() - input.remaining()].to_vec()
}

fn check97<T: RowCodec + Fields + std::fmt::Debug + PartialEq>(table: TableId, row: T) {
    let fields = format97().fields(table);
    let bytes = fields97(table, &row);
    let mut input = decoder97(&bytes, fields);
    let snapshot = T::decode_fields(row.key(), &mut input)
        .unwrap_or_else(|error| panic!("{table:?}: {error}"));
    input.finish().unwrap();
    let mask = table
        .last_change()
        .appended
        .iter()
        .filter(|field| field.introduced > 97)
        .fold(0, |mask, field| mask | (1 << field.index));
    assert_eq!(snapshot.not_recorded_fields(), mask);
    assert_eq!(snapshot.recorded_fields_bytes().unwrap(), bytes.len());
    assert!(snapshot.encode_fields(&mut Vec::new()).is_err());
    assert!(snapshot.delta(None).is_err());
    let delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    for kind in &T::SCHEMA[..fields] {
        kind.decode_change(&mut input).unwrap();
    }
    let mut bytes = Vec::new();
    table.encode_key(row.key(), 0, &mut bytes).unwrap();
    bytes.extend_from_slice(&full_mask(fields).to_le_bytes()[..fields.div_ceil(8)]);
    bytes.extend_from_slice(&delta.values[..delta.values.len() - input.remaining()]);
    let mut input = decoder97(&bytes, fields);
    let decoded = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.format, 97);
    assert_eq!(decoded.recorded_fields, fields);
    assert_eq!(decoded.apply::<T>(None).unwrap(), snapshot);
}

#[test]
fn format97_supported_prefixes_keep_appended_fields_absent() {
    check97(
        TableId::ScriptVariables,
        ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 5000,
            entries: vec![ScriptVariableEntry {
                name: vec![0xff, 0],
                value_raw: i64::MIN,
            }],
            seed: Recorded::Value(None),
        },
    );
    check97(
        TableId::TrainInventory,
        TrainInventoryRow {
            country: 5000,
            node_count: 3,
            allocations: vec![TrainAllocation {
                variant: u64::MAX,
                count: i32::MIN,
            }],
            ..Default::default()
        },
    );
    check97(
        TableId::AdvisorRoles,
        AdvisorRoleRow {
            character: 7,
            ordinal: 2,
            country: 5000,
            idea: vec![0xff, 0],
            ..Default::default()
        },
    );
    check97(
        TableId::CharacterOperatives,
        CharacterOperativeRow {
            character: 7,
            reference: 9_u64 << 32 | 4713,
            resolved: true,
            category: 3,
            owner: 5000,
            trait_definitions: vec![4, 2, 4],
            ..Default::default()
        },
    );
    check97(
        TableId::OperativeMemberships,
        OperativeMembershipRow {
            id: 9_u64 << 32 | 4713,
            roster_country: 5000,
            roster_kind: 3,
            owner_country: 5001,
            experience_raw: i64::MIN,
            ..Default::default()
        },
    );
    check97(
        TableId::VariantDefinitions,
        VariantDefinitionRow {
            id: u64::MAX,
            icon: vec![0xff, 0],
            ..Default::default()
        },
    );
}

#[test]
fn current_country_baseline_updates_keep_five_byte_masks_and_unchanged_combat_fields() {
    let previous = CountryCounterInputRow {
        country: 5000,
        pride_of_the_fleet: u64::MAX,
        owned_state_count: 3,
        controlled_state_count: 2,
        first_division: 9,
        owned_states: Some(vec![2, 1, 2]).into(),
        air_combat_modifiers_known: true.into(),
        air_superiority_efficiency: Some(i64::MIN).into(),
        ..Default::default()
    };
    let next = CountryCounterInputRow {
        owned_state_count: 4,
        first_division: 10,
        ..previous.clone()
    };
    let table = TableId::CountryCounterInputs;
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 0b1010);
    assert_eq!(table.mask_width(), 5);
    let mut bytes = Vec::new();
    encode_delta(table, &delta, 0, &mut bytes).unwrap();
    encode_delta(table, &delta, delta.key, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let first = decode_delta(table, 0, &mut input).unwrap();
    assert_eq!(first.format, crate::FORMAT_VERSION);
    assert_eq!(first.recorded_fields, CountryCounterInputRow::SCHEMA.len());
    let row = first.apply(Some(previous)).unwrap();
    assert_eq!(row, next);
    let second = decode_delta(table, delta.key, &mut input).unwrap();
    let twice = CountryCounterInputRow {
        owned_state_count: 5,
        first_division: 11,
        ..row.clone()
    };
    assert_eq!(second.apply(Some(row)).unwrap(), twice);
    input.finish().unwrap();
}
#[test]
fn format98_operands_preserve_bytes_order_and_independent_changed_fields() {
    let naming = NameOperands {
        kind: 0,
        order: i32::MIN,
        ordered: 0xff,
        programmatic: 2,
        custom: vec![0xff, 0, 0xc3, 0x28],
        group: b"synthetic_group".to_vec(),
    };
    let old = DivisionStatusRow {
        division: u64::MAX,
        ..Default::default()
    };
    let row = DivisionStatusRow {
        naming: Recorded::Value(Some(naming.clone())),
        supply_grace_maximum: Recorded::Value(i64::MAX),
        supply_grace_remaining: Recorded::Value(i64::MIN),
        ..old.clone()
    };
    let delta = row.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 7 << 14);
    assert_eq!(delta.apply(Some(old)).unwrap(), row);
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<DivisionStatusRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        row
    );
    let advisor = AdvisorRoleRow {
        character: 4,
        modifier_cache: Recorded::Value(Some(AdvisorModifierCache {
            entries: vec![
                CachedModifier {
                    definition: 9,
                    value: i64::MIN,
                },
                CachedModifier {
                    definition: 2,
                    value: i64::MAX,
                },
                CachedModifier {
                    definition: 9,
                    value: 0,
                },
            ],
        })),
        ..Default::default()
    };
    let old = AdvisorRoleRow {
        character: 4,
        ..Default::default()
    };
    let delta = advisor.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 4);
    assert_eq!(delta.apply(Some(old)).unwrap(), advisor);
    let leader = UnitLeaderRow {
        id: 8,
        kind: 3,
        codename: Recorded::Value(Some(NameOperands {
            kind: 2,
            ..naming.clone()
        })),
        ..Default::default()
    };
    assert_eq!(
        leader
            .delta(None)
            .unwrap()
            .unwrap()
            .apply::<UnitLeaderRow>(None)
            .unwrap(),
        leader
    );
    let operative = CharacterOperativeRow {
        character: 5,
        reference: 8,
        resolved: true,
        category: 3,
        codename: Recorded::Value(Some(NameOperands { kind: 2, ..naming })),
        name: Recorded::Value(Some(vec![0xff, 0])),
        ..Default::default()
    };
    assert_eq!(
        operative
            .delta(None)
            .unwrap()
            .unwrap()
            .apply::<CharacterOperativeRow>(None)
            .unwrap(),
        operative
    );
    let membership = OperativeMembershipRow {
        id: 8,
        roster_country: 5000,
        ordinal: Recorded::Value(u32::MAX),
        ..Default::default()
    };
    assert_eq!(
        membership
            .delta(None)
            .unwrap()
            .unwrap()
            .apply::<OperativeMembershipRow>(None)
            .unwrap(),
        membership
    );
    assert_eq!(cadence(TableId::OperativeMemberships), Cadence::Daily(16));
    assert_eq!(cadence(TableId::AvailableAdvisors), Cadence::Sliced);
    assert_eq!(cadence(TableId::DivisionStatus), Cadence::Hourly);
}

#[test]
fn format98_country_lists_and_lean_available_advisors_round_trip() {
    let countries = (1..=3)
        .map(|country| CountryCounterInputRow {
            forward_visibility: crate::rows::Recorded::Value(None),
            country,
            owned_state_count: 3,
            controlled_state_count: 2,
            owned_states: Recorded::Value(Some(vec![country + 1, country, country + 1])),
            controlled_states: Recorded::Value(Some(vec![country, country + 2])),
            core_states: Recorded::Value(Some(vec![country + 3, country])),
            army_leaders: Recorded::Value(Some(vec![u64::MAX - u64::from(country), 0])),
            navy_leaders: Recorded::Value(Some(vec![u64::from(country) << 32 | 4713])),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let advisors = (1..=3)
        .map(|country| AvailableAdvisorRow {
            country,
            ordinal: 3,
            character: country + 9000,
            idea: vec![0xff, 0, country as u8],
        })
        .collect::<Vec<_>>();
    for rows in [
        TableRows::CountryCounterInputs(countries),
        TableRows::AvailableAdvisors(advisors),
    ] {
        let mut bytes = Vec::new();
        rows.encode_rows(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(rows.table().decode_rows(3, &mut input).unwrap(), rows);
        input.finish().unwrap();
    }
}

fn nested_field97<V: Value>(bytes: &[u8], nested: &V, tail: usize) -> Vec<u8> {
    let mut needle = Vec::new();
    nested.encode(&mut needle).unwrap();
    assert!(needle[needle.len() - tail..].iter().all(|byte| *byte == 0));
    let mut result = Vec::new();
    let mut remaining = bytes;
    while let Some(at) = remaining
        .windows(needle.len())
        .position(|window| window == needle)
    {
        result.extend_from_slice(&remaining[..at]);
        result.extend_from_slice(&needle[..needle.len() - tail]);
        remaining = &remaining[at + needle.len()..];
    }
    result.extend_from_slice(remaining);
    assert!(result.len() < bytes.len());
    result
}

fn nested97<T: RowCodec + Fields + std::fmt::Debug + PartialEq, V: Value>(
    table: TableId,
    row: T,
    nested: &V,
    tail: usize,
) -> T {
    let mut needle = Vec::new();
    nested.encode(&mut needle).unwrap();
    let recorded = format97().fields(table);
    let recorded_mask = if recorded == 64 {
        u64::MAX
    } else {
        (1_u64 << recorded) - 1
    };
    let mut fields = Vec::new();
    row.encode_fields(&mut fields).unwrap();
    let mut input = Decoder::new(&fields);
    let mut old = Vec::new();
    for (index, kind) in T::SCHEMA.iter().enumerate() {
        let field = kind.decode(&mut input).unwrap();
        if index >= recorded {
            continue;
        }
        if let Field::Opaque(Opaque::Encoded(bytes, _)) = field
            && bytes.windows(needle.len()).any(|window| window == needle)
        {
            put_bytes(&mut old, &nested_field97(bytes, nested, tail)).unwrap();
        } else {
            field.encode(&mut old).unwrap();
        }
    }
    let mut input = decoder97(&old, recorded);
    let snapshot = T::decode_fields(row.key(), &mut input).unwrap();
    assert_eq!(snapshot.recorded_fields_bytes().unwrap(), old.len());
    assert!(snapshot.encode_fields(&mut Vec::new()).is_err());
    input.finish().unwrap();
    let delta = row.delta(None).unwrap().unwrap();
    let mut input = Decoder::new(&delta.values);
    let mut values = Vec::new();
    for (index, kind) in T::SCHEMA.iter().enumerate() {
        let change = kind.decode_change(&mut input).unwrap();
        if index >= recorded {
            continue;
        }
        if let Change::Opaque(Opaque::Encoded(bytes, _)) = &change
            && bytes.windows(needle.len()).any(|window| window == needle)
        {
            put_bytes(&mut values, &nested_field97(bytes, nested, tail)).unwrap();
        } else {
            change.encode(&mut values).unwrap();
        }
    }
    let mut wire = Vec::new();
    table.encode_key(row.key(), 0, &mut wire).unwrap();
    wire.extend_from_slice(&(delta.mask & recorded_mask).to_le_bytes()[..recorded.div_ceil(8)]);
    wire.extend(values);
    let mut input = decoder97(&wire, usize::MAX);
    let decoded = decode_delta(table, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply::<T>(None).unwrap(), snapshot);
    let mut input = decoder97(&decoded.values, usize::MAX);
    let mut replacement = DeltaRow {
        format: 97,
        key: decoded.key,
        mask: 0,
        recorded_fields: decoded.recorded_fields,
        values: Vec::new(),
    };
    let prefix = &needle[..needle.len() - tail];
    for (index, kind) in T::SCHEMA.iter().enumerate().take(recorded) {
        let change = kind.decode_change(&mut input).unwrap();
        if let Change::Opaque(Opaque::Encoded(bytes, _)) = &change
            && bytes.windows(prefix.len()).any(|window| window == prefix)
        {
            replacement.mask |= 1 << index;
            change.encode(&mut replacement.values).unwrap();
        }
    }
    assert_ne!(replacement.mask, 0);
    assert_eq!(replacement.apply(Some(snapshot.clone())).unwrap(), snapshot);
    let mut output = vec![0x55];
    assert!(encode_delta(table, &decoded, 0, &mut output).is_err());
    assert_eq!(output, [0x55]);
    snapshot
}

#[test]
fn format97_nested_naval_commanders_decode_without_consuming_following_fields() {
    let commander = NavalCommanderInputs {
        commander: 0x1234_5678_9abc_def0,
        name: vec![0xff, 0, 8],
        ..Default::default()
    };
    let sides = std::array::from_fn(|_| NavalCombatSideDetails {
        commander_inputs: Some(commander.clone()),
        units: vec![4, u64::MAX, 2],
        ..Default::default()
    });
    let live = nested97(
        TableId::NavalCombatDetails,
        NavalCombatDetailsRow {
            combat: 7,
            sides,
            custom_sea: Some(vec![1, 0]),
            ..Default::default()
        },
        &commander,
        4,
    );
    for side in &live.sides {
        let inputs = side.commander_inputs.as_ref().unwrap();
        assert_eq!(inputs.timed_traits, Recorded::NotRecorded);
        assert_eq!(inputs.name, commander.name);
        assert_eq!(side.units, [4, u64::MAX, 2]);
    }
    assert_eq!(live.custom_sea, Some(vec![1, 0]));
    let gaps = TableRows::NavalCombatDetails(vec![live.clone()]);
    assert_eq!(
        gaps.field_gaps().collect::<Vec<_>>(),
        [("NavalCommanderInputs.timed_traits", 2)]
    );
    let frame = crate::frame::DecodedFrame {
        branch: 1,
        hour: 1,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            if index == TableId::NavalCombatDetails.index() {
                TableView::Complete(gaps.clone()).into()
            } else {
                TableView::Unavailable {
                    reason: "synthetic unavailable".into(),
                }
                .into()
            }
        }),
    };
    let error = crate::frame::FrameEncoder::new()
        .encode(frame, false)
        .unwrap_err();
    assert!(error.to_string().contains("not recorded"), "{error}");
    let mut bytes = vec![0x55];
    assert!(
        !live
            .encode_change_reserved(Some(&live), 0, &mut bytes)
            .unwrap()
    );
    assert_eq!(bytes, [0x55]);
    assert!(live.delta(Some(&live)).unwrap().is_none());
    assert!(live.delta(None).is_err());
    let sides = std::array::from_fn(|_| NavalResultSideDetails {
        commander_context: Some(commander.clone()),
        hq_commander_context: Some(commander.clone()),
        hq_influences: vec![u64::MAX, 3],
        ..Default::default()
    });
    let result = nested97(
        TableId::NavalCombatResultDetails,
        NavalCombatResultDetailsRow { result: 9, sides },
        &commander,
        4,
    );
    for side in &result.sides {
        for inputs in [&side.commander_context, &side.hq_commander_context] {
            assert_eq!(inputs.as_ref().unwrap().timed_traits, Recorded::NotRecorded);
        }
        assert_eq!(side.hq_influences, [u64::MAX, 3]);
    }
    let gaps = TableRows::NavalCombatResultDetails(vec![result]);
    assert_eq!(
        gaps.field_gaps().collect::<Vec<_>>(),
        [("NavalCommanderInputs.timed_traits", 4)]
    );
}

use crate::test_support::empty_frame97;

#[test]
fn format97_frame_keeps_new_tables_unavailable_after_current_encoding() {
    let bytes = empty_frame97(8, true);
    let mut input = decoder97(&bytes, usize::MAX);
    let frame = Frame::decode_version(&mut input, None, format97()).unwrap();
    input.finish().unwrap();
    let last_legacy = TableId::NavalModifierBreakdowns;
    let sizes = frame.table_encoded_sizes_version(format97()).unwrap();
    for table in [TableId::AvailableAdvisors, TableId::OwnedWargoals] {
        assert_eq!(sizes[table.index()], 0);
    }
    let table_count = TableId::ALL
        .into_iter()
        .filter(|table| table.last_change().present(97))
        .count();
    assert_eq!(
        sizes.iter().sum::<usize>(),
        bytes.len() - 37 - 8 * table_count
    );
    assert_eq!(
        frame.metrics.table_ns[last_legacy.index()],
        last_legacy as u64 + 1
    );
    for table in [TableId::AvailableAdvisors, TableId::OwnedWargoals] {
        assert_eq!(frame.metrics.table_ns[table.index()], 0);
        assert!(matches!(
            &frame.tables[table.index()],
            TableUpdate::Unavailable { reason } if reason.contains("not recorded")
        ));
    }
    let decoded = FrameDecoder::new().apply(&frame).unwrap();
    for table in [TableId::AvailableAdvisors, TableId::OwnedWargoals] {
        assert!(matches!(
            &*decoded.tables[table.index()],
            TableView::Unavailable { reason } if reason.contains("not recorded")
        ));
    }
    let current = crate::frame::FrameEncoder::new()
        .encode(decoded, false)
        .unwrap();
    let mut bytes = Vec::new();
    current.encode(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(Frame::decode(&mut input).unwrap(), current);
    input.finish().unwrap();
}

#[test]
fn format97_border_width_and_nested_minimum_keep_following_inputs_aligned() {
    let border = LandBorderWarInputs {
        transfer_state: 2,
        sides: std::array::from_fn(|_| LandBorderWarSide {
            state: -7,
            win_event: vec![0xff, 0],
            ..Default::default()
        }),
        combat_width: Recorded::Value(Default::default()),
    };
    let land = crate::rows::with_historical_stamp(0, || {
        nested97(
            TableId::LandCombatDetails,
            LandCombatDetailsRow {
                combat: 4,
                border_war: Recorded::Value(Some(border.clone())),
                ..Default::default()
            },
            &border,
            8,
        )
    });
    let Recorded::Value(Some(saved_border)) = &land.border_war else {
        panic!("missing border inputs")
    };
    assert_eq!(saved_border.combat_width, Recorded::NotRecorded);
    assert_eq!(saved_border.sides, border.sides);
    assert_eq!(land.side_inputs, Recorded::NotRecorded);
    let gaps = TableRows::LandCombatDetails(vec![land]);
    assert_eq!(
        gaps.field_gaps().collect::<Vec<_>>(),
        [("side_inputs", 1), ("LandBorderWarInputs.combat_width", 1)]
    );
    let commanders = vec![NavalCommanderInputs::default(); 2];
    let mut bytes = 2_u32.to_le_bytes().to_vec();
    for commander in &commanders {
        let mut current = Vec::new();
        commander.encode(&mut current).unwrap();
        bytes.extend_from_slice(&current[..current.len() - 4]);
    }
    let mut input = decoder97(&bytes, usize::MAX);
    assert_eq!(
        Vec::<NavalCommanderInputs>::decode(&mut input).unwrap(),
        commanders
            .into_iter()
            .map(|mut commander| {
                commander.timed_traits = Recorded::NotRecorded;
                commander
            })
            .collect::<Vec<_>>()
    );
    input.finish().unwrap();
}

#[test]
fn format98_optional_empty_inputs_remain_distinct_from_absent_inputs() {
    let old = CountryCounterInputRow {
        forward_visibility: crate::rows::Recorded::Value(None),
        country: 5000,
        ..Default::default()
    };
    let new = CountryCounterInputRow {
        owned_states: Recorded::Value(Some(Vec::new())),
        controlled_states: Recorded::Value(Some(Vec::new())),
        core_states: Recorded::Value(Some(Vec::new())),
        army_leaders: Recorded::Value(Some(Vec::new())),
        navy_leaders: Recorded::Value(Some(Vec::new())),
        ..old.clone()
    };
    let delta = new.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 0b1_1111 << 4);
    assert_eq!(delta.apply(Some(old)).unwrap(), new);
    let old = CharacterOperativeRow {
        character: 7,
        reference: 9,
        resolved: true,
        ..Default::default()
    };
    let new = CharacterOperativeRow {
        name: Recorded::Value(Some(Vec::new())),
        ..old.clone()
    };
    let delta = new.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 7);
    assert_eq!(delta.apply(Some(old)).unwrap(), new);
    for timed_traits in [Vec::new(), vec![LandCombatCommanderTimedTrait::default()]] {
        let commander = NavalCommanderInputs {
            timed_traits: Recorded::Value(timed_traits),
            ..Default::default()
        };
        let mut bytes = Vec::new();
        commander.encode(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(NavalCommanderInputs::decode(&mut input).unwrap(), commander);
        input.finish().unwrap();
    }
    let leader = UnitLeaderRow {
        id: 8,
        codename: Recorded::Value(Some(NameOperands::default())),
        ..Default::default()
    };
    assert!(leader.check_key_fields().is_err());
    let leader = UnitLeaderRow { kind: 3, ..leader };
    assert!(leader.check_key_fields().is_ok());
    let operative = CharacterOperativeRow {
        character: 7,
        reference: 9,
        resolved: true,
        category: 0,
        codename: Recorded::Value(Some(NameOperands {
            kind: 2,
            ..Default::default()
        })),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    encode_row(&operative, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<CharacterOperativeRow>(0, &mut input).unwrap(),
        operative
    );
    input.finish().unwrap();
}

#[test]
fn format98_wargoals_variant_version_and_border_width_round_trip() {
    let goals = (1..=5000)
        .map(|country| OwnedWargoalRow {
            country,
            ordinal: u32::MAX,
            recipient: u32::MAX - country,
            goal_type: i32::MIN + country as i32,
            states: vec![country, 0, country],
        })
        .collect::<Vec<_>>();
    let rows = TableRows::OwnedWargoals(goals);
    let mut bytes = Vec::new();
    rows.encode_rows(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        TableId::OwnedWargoals
            .decode_rows(5000, &mut input)
            .unwrap(),
        rows
    );
    input.finish().unwrap();
    assert_eq!(cadence(TableId::OwnedWargoals), Cadence::Hourly);
    let old = VariantDefinitionRow {
        id: u64::MAX,
        ..Default::default()
    };
    let new = VariantDefinitionRow {
        version: Recorded::Value(u8::MAX),
        ..old.clone()
    };
    let delta = new.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 14);
    assert_eq!(delta.apply(Some(old)).unwrap(), new);
    let old = LandCombatDetailsRow {
        combat: 9,
        border_war: Recorded::Value(Some(LandBorderWarInputs::default())),
        ..Default::default()
    };
    let new = LandCombatDetailsRow {
        border_war: Recorded::Value(Some(LandBorderWarInputs {
            combat_width: Recorded::Value(i64::MIN),
            ..Default::default()
        })),
        ..old.clone()
    };
    let delta = new.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(delta.apply(Some(old)).unwrap(), new);
}

#[test]
fn format98_scope_seeds_and_supply_vehicles_preserve_zero_absence_and_raw_extremes() {
    for scope_kind in 0..=3 {
        let old = ScriptVariablesRow {
            scope_kind,
            scope_id: if scope_kind == 0 { 0 } else { 5000 },
            entries: vec![ScriptVariableEntry {
                name: b"synthetic_variable".to_vec(),
                value_raw: i64::MIN,
            }],
            seed: Recorded::Value(None),
        };
        for seed in [Some(0), Some(u32::MAX), None] {
            let new = ScriptVariablesRow {
                seed: Recorded::Value(seed),
                ..old.clone()
            };
            let delta = new.delta(Some(&old)).unwrap();
            if seed.is_some() {
                let delta = delta.unwrap();
                assert_eq!(delta.mask, 1 << 1);
                assert_eq!(delta.apply(Some(old.clone())).unwrap(), new);
            } else {
                assert!(delta.is_none());
            }
            let mut bytes = Vec::new();
            encode_row(&new, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                decode_row::<ScriptVariablesRow>(0, &mut input).unwrap(),
                new
            );
            input.finish().unwrap();
        }
    }
    let old = TrainInventoryRow {
        country: 5000,
        ..Default::default()
    };
    for (allocated_trucks, required_trucks_raw, required_trains) in [
        (Some(0), Some(0), Some(0)),
        (Some(i32::MIN), Some(i64::MIN), Some(i32::MAX)),
        (None, Some(i64::MAX), None),
    ] {
        let new = TrainInventoryRow {
            allocated_trucks: Recorded::Value(allocated_trucks),
            required_trucks_raw: Recorded::Value(required_trucks_raw),
            required_trains: Recorded::Value(required_trains),
            ..old.clone()
        };
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        let mask = (u64::from(allocated_trucks.is_some()) << 3)
            | (u64::from(required_trucks_raw.is_some()) << 4)
            | (u64::from(required_trains.is_some()) << 5);
        assert_eq!(delta.mask, mask);
        assert_eq!(delta.apply(Some(old.clone())).unwrap(), new);
        let clear = old.delta(Some(&new)).unwrap().unwrap();
        assert_eq!(clear.mask, mask);
        assert_eq!(clear.apply(Some(new.clone())).unwrap(), old);
        let mut bytes = Vec::new();
        encode_row(&new, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<TrainInventoryRow>(0, &mut input).unwrap(), new);
        input.finish().unwrap();
    }
    assert_eq!(cadence(TableId::ScriptVariables), Cadence::Hourly);
    assert_eq!(cadence(TableId::TrainInventory), Cadence::Sliced);
}

#[test]
fn format98_scope_seeds_and_supply_vehicles_round_trip() {
    let scopes = (1..=3)
        .map(|country| ScriptVariablesRow {
            scope_kind: 1,
            scope_id: country,
            entries: Vec::new(),
            seed: Recorded::Value(Some(u32::MAX - country)),
        })
        .collect::<Vec<_>>();
    let inventories = (1..=3)
        .map(|country| TrainInventoryRow {
            country,
            allocated_trucks: Recorded::Value(Some(country as i32)),
            required_trucks_raw: Recorded::Value(Some(i64::from(country) * 100_000 + 1)),
            required_trains: Recorded::Value(Some(-(country as i32))),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    for rows in [
        TableRows::ScriptVariables(scopes),
        TableRows::TrainInventory(inventories),
    ] {
        let mut bytes = Vec::new();
        rows.encode_rows(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(rows.table().decode_rows(3, &mut input).unwrap(), rows);
        input.finish().unwrap();
    }
}

#[test]
fn state_continent_append_preserves_format98_prefix_and_distinct_failures() {
    let row = StateRow {
        id: 5000,
        owner: 11,
        controller: 22,
        continent: Recorded::Value(Some(991)),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    row.encode_fields(&mut bytes).unwrap();
    let mut prefix = Decoder::new(&bytes);
    for kind in &StateRow::SCHEMA[..6] {
        kind.decode(&mut prefix).unwrap();
    }
    let end = bytes.len() - prefix.remaining();
    let mut input = Decoder::new(&bytes[..end]);
    input.format = 98;
    input.recorded_fields = 6;
    let old = StateRow::decode_fields(row.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(old.owner, 11);
    assert_eq!(old.controller, 22);
    assert_eq!(old.continent, Recorded::NotRecorded);
    assert_eq!(FormatVersion::read(98).unwrap().fields(TableId::States), 6);
    assert_eq!(FormatVersion::read(99).unwrap().fields(TableId::States), 6);
    for continent in [None, Some(0), Some(991), Some(u32::MAX)] {
        let new = StateRow {
            continent: Recorded::Value(continent),
            ..row.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&new, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<StateRow>(0, &mut input).unwrap(), new);
        input.finish().unwrap();
    }
}

#[test]
fn format_143_train_inventory_withholds_required_trucks_raw_and_skips_legacy_variant_stats() {
    let format = replay_data::rows::compatibility::VARIANT_STAT_WITNESS_FORMAT - 1;
    let train = TrainInventoryRow {
        country: 5000,
        node_count: 4,
        allocated_trucks: Recorded::Value(Some(12)),
        required_trucks_raw: Recorded::Value(Some(0)),
        required_trains: Recorded::Value(Some(3)),
        ..Default::default()
    };
    let mut train_bytes = Vec::new();
    train.encode_fields(&mut train_bytes).unwrap();
    let mut input = Decoder::new(&train_bytes);
    input.format = format;
    input.recorded_fields = TrainInventoryRow::SCHEMA.len();
    let decoded = TrainInventoryRow::decode_fields(train.key(), &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.allocated_trucks, Recorded::Value(Some(12)));
    assert_eq!(decoded.required_trains, Recorded::Value(Some(3)));
    assert_eq!(decoded.required_trucks_raw, Recorded::NotRecorded);

    let mut delta = train.delta(None).unwrap().unwrap();
    delta.format = format;
    let applied = delta.apply::<TrainInventoryRow>(None).unwrap();
    assert_eq!(applied.allocated_trucks, Recorded::Value(Some(12)));
    assert_eq!(applied.required_trains, Recorded::Value(Some(3)));
    assert_eq!(applied.required_trucks_raw, Recorded::NotRecorded);

    let next_train = TrainInventoryRow {
        allocated_trucks: Recorded::Value(Some(18)),
        required_trucks_raw: Recorded::Value(Some(100_000)),
        ..train.clone()
    };
    let mut second_delta = next_train.delta(Some(&train)).unwrap().unwrap();
    second_delta.format = format;
    let updated = second_delta.apply(Some(decoded)).unwrap();
    assert_eq!(updated.allocated_trucks, Recorded::Value(Some(18)));
    assert_eq!(updated.required_trains, Recorded::Value(Some(3)));
    assert_eq!(updated.required_trucks_raw, Recorded::NotRecorded);

    let mut legacy_snapshot = Vec::new();
    legacy_snapshot.extend_from_slice(&1_u32.to_le_bytes());
    TableId::VariantStats
        .encode_key((1_u128 << 32) | 70, 0, &mut legacy_snapshot)
        .unwrap();
    for lane in 0..10_i64 {
        legacy_snapshot.extend_from_slice(&(lane * 100_000).to_le_bytes());
    }
    legacy_snapshot.extend_from_slice(&0_u64.to_le_bytes());
    legacy_snapshot.extend_from_slice(&0_i32.to_le_bytes());
    legacy_snapshot.extend_from_slice(&0_u32.to_le_bytes());
    let mut skip_in = Decoder::new(&legacy_snapshot);
    skip_in.format = format;
    TableId::VariantStats
        .skip_snapshot(13, &mut skip_in)
        .unwrap();
    skip_in.finish().unwrap();

    let mut legacy_delta = Vec::new();
    legacy_delta.extend_from_slice(&1_u32.to_le_bytes());
    TableId::VariantStats
        .encode_key((1_u128 << 32) | 70, 0, &mut legacy_delta)
        .unwrap();
    legacy_delta.extend_from_slice(&1_u16.to_le_bytes());
    replay_data::codec::put_varint(&mut legacy_delta, 1_000_000);
    legacy_delta.extend_from_slice(&0_u32.to_le_bytes());
    let mut skip_delta_in = Decoder::new(&legacy_delta);
    skip_delta_in.format = format;
    TableId::VariantStats
        .skip_delta(13, &mut skip_delta_in)
        .unwrap();
    skip_delta_in.finish().unwrap();
}
