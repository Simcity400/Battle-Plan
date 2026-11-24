use super::*;
use crate::format::FormatVersion;
use crate::rows::compatibility::LAND_MODIFIER_ROWS_FORMAT;
use crate::test_support::OldDivision;
use crate::value::Value;

fn member(ordinal: u32, entries: Vec<CombatModifierValue>) -> LandCombatDivisionModifierRow {
    LandCombatDivisionModifierRow {
        combat: u64::MAX,
        side: 1,
        ordinal,
        entries,
        division: None,
    }
}

fn entry(id: u32, value_a: i64, value_b: i64) -> CombatModifierValue {
    CombatModifierValue {
        id,
        value_a,
        value_b,
    }
}


fn opaque(value: &impl Value, output: &mut Vec<u8>) {
    let mut payload = Vec::new();
    Value::encode(value, &mut payload).unwrap();
    output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    output.extend(payload);
}

#[test]
fn land_combat_division_modifiers_hold_a_row_a_member_keyed_as_its_combat_row() {
    let table = TableId::LandCombatDivisionModifiers;
    assert_eq!(table.field_name(0), Some("entries"));
    assert_eq!(table.field_name(1), Some("division"));
    assert_eq!(table.field_count(), 2);
    let row = member(u32::MAX, vec![entry(7, 11, -29), entry(8, 1, -1)]);
    assert_eq!(
        row.key(),
        LandCombatDivisionRow {
            combat: row.combat,
            side: row.side,
            ordinal: row.ordinal,
            ..Default::default()
        }
        .key()
    );
    for row in [
        row,
        member(0, Vec::new()),
        member(
            3,
            vec![
                entry(u32::MAX, i64::MIN, i64::MAX),
                entry(0, -10, 0),
                entry(0, -10, 0),
            ],
        ),
    ] {
        let mut bytes = Vec::new();
        encode_row(&row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            decode_row::<LandCombatDivisionModifierRow>(0, &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
    }
}

#[test]
fn one_division_whose_entries_change_stores_only_its_row() {
    let before = [
        member(0, vec![entry(1, 1_000, 0)]),
        member(1, vec![entry(1, 1_000, 0), entry(2, 50, -50)]),
    ];
    let mut after = before.clone();
    after[1].entries[1].value_a = 60;
    assert!(before[0].delta(Some(&before[0])).unwrap().is_none());
    let delta = after[1].delta(Some(&before[1])).unwrap().unwrap();
    assert_eq!(delta.mask, 1);
    assert_eq!(delta.apply(Some(before[1].clone())).unwrap(), after[1]);
    let mut drifted = after[1].clone();
    drifted.entries[1].value_a = 69;
    drifted.entries[1].value_b = -1;
    drifted.round_to_display();
    assert_eq!(drifted.entries[1], entry(2, 69, -1));
    let delta = drifted.delta(Some(&after[1])).unwrap().unwrap();
    assert_eq!(delta.mask, 1);
    assert_eq!(delta.apply(Some(after[1].clone())).unwrap(), drifted);
}

#[test]
fn land_combat_modifiers_keep_their_side_rows_framing_and_border_values() {
    let table = TableId::LandCombatModifiers;
    assert_eq!(table.field_name(0), Some("border_values"));
    assert_eq!(table.field_name(1), Some("divisions"));
    assert!(matches!(table.schema(), [Kind::Opaque, Kind::Opaque]));
    let row = LandCombatModifierRow {
        combat: 62,
        side: 1,
        border_values: Some([i64::MIN, 0, i64::MAX]),
        divisions: Retired,
    };
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    let mut current = Vec::new();
    table.encode_key(row.key(), 0, &mut current).unwrap();
    opaque(&row.border_values, &mut current);
    current.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(bytes, current);
    let mut input = Decoder::new(&bytes);
    assert_eq!(
        decode_row::<LandCombatModifierRow>(0, &mut input).unwrap(),
        row
    );
    input.finish().unwrap();

    let previous = FormatVersion::read(LAND_MODIFIER_ROWS_FORMAT - 1).unwrap();
    let mut old = Vec::new();
    table.encode_key(row.key(), 0, &mut old).unwrap();
    opaque(&row.border_values, &mut old);
    opaque(
        &vec![
            OldDivision {
                division: (0x35 << 32) | 0x33,
                ordinal: 0,
                entries: vec![entry(4, 10, -10), entry(4, 10, -10)],
            },
            OldDivision {
                division: 9,
                ordinal: 1,
                entries: Vec::new(),
            },
        ],
        &mut old,
    );
    let mut input = Decoder::new(&old);
    input.format = previous.version();
    assert_eq!(
        decode_row::<LandCombatModifierRow>(0, &mut input).unwrap(),
        row
    );
    input.finish().unwrap();
    assert_eq!(previous.fields(table), 2);
    assert!(previous.table_refusal(table).is_none());
    let reason = previous
        .table_refusal(TableId::LandCombatDivisionModifiers)
        .unwrap();
    assert!(reason.contains("LandCombatDivisionModifiers"), "{reason}");
    assert!(
        FormatVersion::CURRENT
            .table_refusal(TableId::LandCombatDivisionModifiers)
            .is_none()
    );
}
