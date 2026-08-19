use super::deployment::*;
use alloc::{vec, vec::Vec};

fn v1_golden() -> Vec<u8> {
    vec![
        9, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x12, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1,
        0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 8, 0, 0, 0, 64, 0, 0, 0, 9, 0, 0, 0, 52, 0, 0, 0, 1, 0, 0, 0, 10, 0, 0, 0, 11, 0, 0, 0,
        1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'N', 1, 0, 0, 0, b'R', 0, 0, 0, 0, 12, 0, 0, 0, 63, 0,
        0, 0, 13, 0, 0, 0, 14, 0, 0, 0, 0x1f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0,
        0, 0, 0x40, 3, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 1, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 70, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0,
        0x3f, 1, 0, 0, 0, b'A',
    ]
}

fn v2_golden() -> Vec<u8> {
    vec![
        9, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x12, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 1,
        0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 8, 0, 0, 0, 64, 0, 0, 0, 9, 0, 0, 0, 52, 0, 0, 0, 1, 0, 0, 0, 10, 0, 0, 0, 11, 0, 0, 0,
        1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, b'N', 1, 0, 0, 0, b'R', 0, 0, 0, 0, 12, 0, 0, 0, 63, 0,
        0, 0, 13, 0, 0, 0, 14, 0, 0, 0, 0x3f, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0x3f, 0, 0, 0, 0, 0,
        0, 0, 0x40, 3, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 2, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 70, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0,
        0x3f, 1, 0, 0, 0, b'A', 1, 0, 0, 0, 11, 0, 0, 0, 70, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0x40,
    ]
}

fn consume(payload: &[u8]) -> Result<DeploymentEnd, Violation> {
    let mut countries = walk(payload)?;
    loop {
        match countries.next(payload)? {
            CountryStep::End(end) => return Ok(end),
            CountryStep::Country { conveyors, .. } => {
                countries = conveyors.open(payload)?.discard_all(payload)?;
            }
        }
    }
}

#[test]
fn hand_assembled_v1_and_v2_frames_pin_the_versioned_boundary() {
    let v1 = v1_golden();
    let v2 = v2_golden();
    let end1 = consume(&v1).expect("v1 structural walk");
    let end2 = consume(&v2).expect("v2 structural walk");
    assert_eq!(end1.at, v1.len());
    assert_eq!(end2.at, v2.len());
    assert_eq!(end1.header.schema_version, SchemaVersion::V1);
    assert_eq!(end2.header.schema_version, SchemaVersion::V2);
    assert_eq!(end1.totals.equipment, 1);
    assert_eq!(end2.totals.equipment, 2);
    assert!(end1.totals.matches_declared(&end1.header));
    assert!(end2.totals.matches_declared(&end2.header));
}

#[test]
fn nested_rows_return_string_spans_and_only_the_final_child_restores_each_parent() {
    let bytes = v2_golden();
    let CountryStep::Country {
        row: country,
        conveyors,
        ..
    } = walk(&bytes).unwrap().next(&bytes).unwrap()
    else {
        panic!("country expected")
    };
    assert_eq!(country.conveyor_count, 1);
    let conveyor = conveyors.open(&bytes).unwrap().next(&bytes).unwrap();
    let named = conveyor.name.read_name(&bytes).unwrap();
    assert_eq!(&bytes[named.name.start..named.name.end], b"N");
    let role = named.role.read_role(&bytes).unwrap();
    assert_eq!(&bytes[role.row.role.start..role.row.role.end], b"R");
    let line = role.lines.open(&bytes).unwrap().next(&bytes).unwrap();
    assert_eq!(line.row.flags, LINE_KNOWN_FLAGS);
    let first = line.equipment.open(&bytes).unwrap().next(&bytes).unwrap();
    let first_span = first.row.archetype.expect("v2 flagged archetype");
    assert_eq!(&bytes[first_span.start..first_span.end], b"A");
    let second = first.next.next(&bytes).unwrap();
    assert_eq!(second.row.archetype, None);
    let lines = second.next.into_end().unwrap();
    let conveyors = lines.into_end().unwrap();
    let countries = conveyors.into_end().unwrap();
    assert!(matches!(
        countries.next(&bytes).unwrap(),
        CountryStep::End(_)
    ));
}

#[test]
fn encoders_match_the_independent_literal_goldens() {
    let v1_equipment = [EquipmentInputV1 {
        ordinal: 0,
        equipment_id: 10,
        id_type: EQUIPMENT_ID_TYPE,
        flags: 0,
        amount_bits: 1.0f64.to_bits(),
        archetype: b"A",
    }];
    let v1_lines = [LineInput {
        ordinal: 0,
        line_id: 12,
        id_type: LINE_ID_TYPE,
        name_order: 13,
        amount: 14,
        flags: 0x1f,
        training_bits: 1.0f64.to_bits(),
        max_training_bits: 2.0f64.to_bits(),
        manpower_have: 3,
        manpower_need: 4,
        max_manpower: 5,
        reserved: 0,
        equipment: &v1_equipment,
    }];
    let v1_conveyors = [ConveyorInput {
        ordinal: 0,
        conveyor_id: 8,
        id_type: CONVEYOR_ID_TYPE,
        template_id: 9,
        template_id_type: TEMPLATE_ID_TYPE,
        amount: 1,
        location: 10,
        priority: 11,
        flags: 1,
        name: b"N",
        role: b"R",
        lines: &v1_lines,
    }];
    let v1_countries = [CountryInput {
        tag_db_id: 7,
        flags: 1,
        reserved: 0,
        conveyors: &v1_conveyors,
    }];
    let words = HeaderWords {
        elapsed_wall_ms: 9,
        changed_country_rows: 1,
        status_flags: 0x12,
    };
    assert_eq!(
        encode_v1(words, &v1_countries).expect("v1 encode"),
        v1_golden()
    );

    let v2_equipment = [
        EquipmentInputV2 {
            ordinal: 0,
            equipment_id: 10,
            id_type: EQUIPMENT_ID_TYPE,
            flags_without_archetype: 0,
            amount_bits: 1.0f64.to_bits(),
            archetype: Some(b"A"),
        },
        EquipmentInputV2 {
            ordinal: 1,
            equipment_id: 11,
            id_type: EQUIPMENT_ID_TYPE,
            flags_without_archetype: 0,
            amount_bits: 2.0f64.to_bits(),
            archetype: None,
        },
    ];
    let v2_lines = [LineInput {
        ordinal: 0,
        line_id: 12,
        id_type: LINE_ID_TYPE,
        name_order: 13,
        amount: 14,
        flags: LINE_KNOWN_FLAGS,
        training_bits: 1.0f64.to_bits(),
        max_training_bits: 2.0f64.to_bits(),
        manpower_have: 3,
        manpower_need: 4,
        max_manpower: 5,
        reserved: 0,
        equipment: &v2_equipment,
    }];
    let v2_conveyors = [ConveyorInput {
        ordinal: 0,
        conveyor_id: 8,
        id_type: CONVEYOR_ID_TYPE,
        template_id: 9,
        template_id_type: TEMPLATE_ID_TYPE,
        amount: 1,
        location: 10,
        priority: 11,
        flags: 1,
        name: b"N",
        role: b"R",
        lines: &v2_lines,
    }];
    let v2_countries = [CountryInput {
        tag_db_id: 7,
        flags: 1,
        reserved: 0,
        conveyors: &v2_conveyors,
    }];
    let encoded_v2 = encode_v2(words, &v2_countries).expect("v2 encode");
    let golden_v2 = v2_golden();
    if encoded_v2 != golden_v2 {
        let first = encoded_v2
            .iter()
            .zip(&golden_v2)
            .position(|(left, right)| left != right);
        panic!(
            "v2 golden drift: encoded={}, golden={}, first={first:?}",
            encoded_v2.len(),
            golden_v2.len()
        );
    }
}

#[test]
fn line_flag_fixture_union_equals_the_generated_known_mask() {
    const LINE_AT: usize = HEADER_LEN + COUNTRY_ROW_LEN + CONVEYOR_ROW_LEN + 5 + 5;
    let mut union = 0u32;
    for fixture in [v1_golden(), v2_golden()] {
        union |= u32::from_le_bytes(
            fixture
                [LINE_AT + DqLineField::Flags.offset()..LINE_AT + DqLineField::Flags.offset() + 4]
                .try_into()
                .unwrap(),
        );
    }
    assert_eq!(union, LINE_KNOWN_FLAGS);
}

#[test]
fn reading_either_version_with_the_other_boundary_rule_is_refused() {
    let mut v1_as_v2 = v1_golden();
    v1_as_v2[HeaderField::SchemaVersion.offset()..HeaderField::SchemaVersion.offset() + 4]
        .copy_from_slice(&2u32.to_le_bytes());
    assert!(matches!(
        consume(&v1_as_v2).unwrap_err().kind,
        DqViolationKind::TrailingBytes { .. }
    ));

    let mut v2_as_v1 = v2_golden();
    v2_as_v1[HeaderField::SchemaVersion.offset()..HeaderField::SchemaVersion.offset() + 4]
        .copy_from_slice(&1u32.to_le_bytes());
    assert!(matches!(
        consume(&v2_as_v1).unwrap_err().kind,
        DqViolationKind::Need {
            section: DqSection::Archetype,
            ..
        } | DqViolationKind::TrailingBytes { .. }
            | DqViolationKind::CountCannotFit { .. }
    ));
}

#[test]
fn unconsumed_nested_blocks_are_caller_defects() {
    let CountryStep::Country { conveyors, .. } =
        walk(&v2_golden()).unwrap().next(&v2_golden()).unwrap()
    else {
        panic!("country expected")
    };
    let fixed = conveyors
        .open(&v2_golden())
        .unwrap()
        .next(&v2_golden())
        .unwrap();
    let named = fixed.name.read_name(&v2_golden()).unwrap();
    let role = named.role.read_role(&v2_golden()).unwrap();
    let lines = role.lines.open(&v2_golden()).unwrap();
    assert_eq!(
        lines.into_end().unwrap_err().kind,
        DqViolationKind::CallerDefect(DqCallerDefect::BlockNotConsumed {
            block: Block::Lines,
            remaining: 1,
        })
    );
}

#[test]
fn parent_fields_are_yielded_before_later_child_structure_is_checked() {
    let mut country_bad = v2_golden();
    country_bad
        [HEADER_LEN + CountryField::Flags.offset()..HEADER_LEN + CountryField::Flags.offset() + 4]
        .copy_from_slice(&0x8000_0000u32.to_le_bytes());
    country_bad[HEADER_LEN + CountryField::ConveyorCount.offset()
        ..HEADER_LEN + CountryField::ConveyorCount.offset() + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    let CountryStep::Country { row, conveyors, .. } =
        walk(&country_bad).unwrap().next(&country_bad).unwrap()
    else {
        panic!("country expected")
    };
    assert_eq!(row.flags, 0x8000_0000);
    assert_eq!(row.conveyor_count, u32::MAX);
    assert!(matches!(
        conveyors.open(&country_bad).unwrap_err().kind,
        DqViolationKind::CountCannotFit {
            block: Block::Conveyors,
            ..
        }
    ));

    let mut string_bad = v2_golden();
    let name_byte = HEADER_LEN + COUNTRY_ROW_LEN + CONVEYOR_ROW_LEN + STRING_PREFIX_LEN;
    string_bad[name_byte] = 0xff;
    let role_prefix = name_byte + 1;
    string_bad[role_prefix..role_prefix + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let CountryStep::Country { conveyors, .. } =
        walk(&string_bad).unwrap().next(&string_bad).unwrap()
    else {
        panic!("country expected")
    };
    let fixed = conveyors
        .open(&string_bad)
        .unwrap()
        .next(&string_bad)
        .unwrap();
    let named = fixed.name.read_name(&string_bad).unwrap();
    assert_eq!(&string_bad[named.name.start..named.name.end], &[0xff]);
    assert!(matches!(
        named.role.read_role(&string_bad).unwrap_err().kind,
        DqViolationKind::Need {
            section: DqSection::ConveyorRole,
            ..
        }
    ));

    let mut line_bad = v2_golden();
    const LINE_AT: usize = HEADER_LEN + COUNTRY_ROW_LEN + CONVEYOR_ROW_LEN + 5 + 5;
    line_bad[LINE_AT + DqLineField::Flags.offset()..LINE_AT + DqLineField::Flags.offset() + 4]
        .copy_from_slice(&0x8000_0000u32.to_le_bytes());
    line_bad[LINE_AT + DqLineField::EquipmentCount.offset()
        ..LINE_AT + DqLineField::EquipmentCount.offset() + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    let CountryStep::Country { conveyors, .. } = walk(&line_bad).unwrap().next(&line_bad).unwrap()
    else {
        panic!("country expected")
    };
    let fixed = conveyors.open(&line_bad).unwrap().next(&line_bad).unwrap();
    let named = fixed.name.read_name(&line_bad).unwrap();
    let role = named.role.read_role(&line_bad).unwrap();
    let line = role.lines.open(&line_bad).unwrap().next(&line_bad).unwrap();
    assert_eq!(line.row.flags, 0x8000_0000);
    assert_eq!(line.row.equipment_count, u32::MAX);
    assert!(matches!(
        line.equipment.open(&line_bad).unwrap_err().kind,
        DqViolationKind::CountCannotFit {
            block: Block::Equipment,
            ..
        }
    ));
}

#[test]
fn declared_totals_are_exposed_but_not_leaf_fatal() {
    let mut bytes = v2_golden();
    bytes[HeaderField::DeclaredEquipment.offset()..HeaderField::DeclaredEquipment.offset() + 4]
        .copy_from_slice(&1u32.to_le_bytes());
    let end = consume(&bytes).expect("totals are a site policy");
    assert_eq!(end.totals.equipment, 2);
    assert!(!end.totals.matches_declared(&end.header));
}

#[test]
fn header_counts_are_bounded_before_any_allocation_or_row_walk() {
    for (field, block, minimum_row_len) in [
        (HeaderField::CountryCount, Block::Countries, COUNTRY_ROW_LEN),
        (
            HeaderField::DeclaredConveyors,
            Block::Conveyors,
            CONVEYOR_ROW_LEN,
        ),
        (HeaderField::DeclaredLines, Block::Lines, LINE_ROW_LEN),
        (
            HeaderField::DeclaredEquipment,
            Block::Equipment,
            EQUIPMENT_ROW_LEN,
        ),
    ] {
        let mut bytes = vec![0u8; HEADER_LEN];
        bytes[HeaderField::SchemaVersion.offset()..HeaderField::SchemaVersion.offset() + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        bytes[field.offset()..field.offset() + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            walk(&bytes).unwrap().next(&bytes).unwrap_err().kind,
            DqViolationKind::CountCannotFit {
                block,
                count: u32::MAX,
                remaining: 0,
                minimum_row_len,
            }
        );
    }
}

#[test]
fn zero_country_direct_completion_cannot_bypass_global_count_prechecks() {
    let mut bytes = vec![0u8; HEADER_LEN];
    bytes[HeaderField::SchemaVersion.offset()..HeaderField::SchemaVersion.offset() + 4]
        .copy_from_slice(&2u32.to_le_bytes());
    bytes[HeaderField::DeclaredConveyors.offset()..HeaderField::DeclaredConveyors.offset() + 4]
        .copy_from_slice(&1u32.to_le_bytes());
    let expected = DqViolationKind::CountCannotFit {
        block: Block::Conveyors,
        count: 1,
        remaining: 0,
        minimum_row_len: CONVEYOR_ROW_LEN,
    };
    assert_eq!(
        walk(&bytes).unwrap().next(&bytes).unwrap_err().kind,
        expected
    );
    assert_eq!(
        walk(&bytes).unwrap().into_end(&bytes).unwrap_err().kind,
        expected
    );
}

#[test]
fn string_bytes_are_spans_and_the_leaf_has_no_site_budget_or_charset_rule() {
    let long_non_utf8 = vec![0xff; 16 * 1024 + 1];
    let equipment = [EquipmentInputV2 {
        ordinal: 0,
        equipment_id: 1,
        id_type: EQUIPMENT_ID_TYPE,
        flags_without_archetype: 0,
        amount_bits: 0,
        archetype: Some(&long_non_utf8),
    }];
    let lines = [LineInput {
        ordinal: 0,
        line_id: 1,
        id_type: LINE_ID_TYPE,
        name_order: 0,
        amount: 0,
        flags: 0,
        training_bits: 0,
        max_training_bits: 0,
        manpower_have: 0,
        manpower_need: 0,
        max_manpower: 0,
        reserved: 0,
        equipment: &equipment,
    }];
    let conveyors = [ConveyorInput {
        ordinal: 0,
        conveyor_id: 1,
        id_type: CONVEYOR_ID_TYPE,
        template_id: 1,
        template_id_type: TEMPLATE_ID_TYPE,
        amount: 0,
        location: 0,
        priority: 0,
        flags: 0,
        name: &[0xff],
        role: b"",
        lines: &lines,
    }];
    let countries = [CountryInput {
        tag_db_id: 1,
        flags: 1,
        reserved: 0,
        conveyors: &conveyors,
    }];
    let bytes = encode_v2(
        HeaderWords {
            elapsed_wall_ms: 0,
            changed_country_rows: 1,
            status_flags: 0,
        },
        &countries,
    )
    .unwrap();
    assert_eq!(consume(&bytes).unwrap().at, bytes.len());
}

#[test]
fn encode_refuses_an_encoding_that_changes_between_preflight_and_write() {
    use core::cell::Cell;

    struct ShiftingCountry {
        seen: Cell<bool>,
        conveyors: Vec<ConveyorInput<'static, EquipmentInputV2<'static>>>,
    }

    impl CountryEncoding for ShiftingCountry {
        type Conveyor = ConveyorInput<'static, EquipmentInputV2<'static>>;

        fn tag_db_id(&self) -> u32 {
            7
        }
        fn flags(&self) -> u32 {
            0
        }
        fn reserved(&self) -> u32 {
            0
        }
        fn conveyors(&self) -> &[Self::Conveyor] {
            if self.seen.replace(true) {
                &[]
            } else {
                &self.conveyors
            }
        }
    }

    let country = ShiftingCountry {
        seen: Cell::new(false),
        conveyors: vec![ConveyorInput {
            ordinal: 0,
            conveyor_id: 1,
            id_type: 1,
            template_id: 2,
            template_id_type: 1,
            amount: 3,
            location: 4,
            priority: 5,
            flags: 0,
            name: b"n",
            role: b"r",
            lines: &[],
        }],
    };

    let words = HeaderWords {
        elapsed_wall_ms: 1,
        changed_country_rows: 1,
        status_flags: 0,
    };
    let result = encode_v2(words, core::slice::from_ref(&country));
    match result {
        Err(DqEncodeError::PreflightMismatch { declared, written }) => {
            assert!(
                declared > written,
                "the preflight sized a conveyor the write omitted: {declared} vs {written}"
            );
        }
        other => panic!("expected PreflightMismatch, got {other:?}"),
    }
}

#[test]
fn encode_header_counts_come_from_the_write_not_the_preflight() {
    use core::cell::Cell;

    struct ShiftingConveyor {
        seen: Cell<bool>,
        lines: Vec<LineInput<'static, EquipmentInputV2<'static>>>,
        long_name: Vec<u8>,
    }

    impl ConveyorEncoding for ShiftingConveyor {
        type Line = LineInput<'static, EquipmentInputV2<'static>>;

        fn ordinal(&self) -> u32 {
            0
        }
        fn conveyor_id(&self) -> u32 {
            1
        }
        fn id_type(&self) -> u32 {
            1
        }
        fn template_id(&self) -> u32 {
            2
        }
        fn template_id_type(&self) -> u32 {
            1
        }
        fn amount(&self) -> i32 {
            3
        }
        fn location(&self) -> u32 {
            4
        }
        fn priority(&self) -> u32 {
            5
        }
        fn flags(&self) -> u32 {
            0
        }
        fn name(&self) -> &[u8] {
            if self.seen.get() {
                &self.long_name
            } else {
                &[]
            }
        }
        fn role(&self) -> &[u8] {
            &[]
        }
        fn lines(&self) -> &[Self::Line] {
            if self.seen.replace(true) {
                &[]
            } else {
                &self.lines
            }
        }
    }

    struct OneCountry {
        conveyors: Vec<ShiftingConveyor>,
    }

    impl CountryEncoding for OneCountry {
        type Conveyor = ShiftingConveyor;

        fn tag_db_id(&self) -> u32 {
            7
        }
        fn flags(&self) -> u32 {
            0
        }
        fn reserved(&self) -> u32 {
            0
        }
        fn conveyors(&self) -> &[Self::Conveyor] {
            &self.conveyors
        }
    }

    let country = OneCountry {
        conveyors: vec![ShiftingConveyor {
            seen: Cell::new(false),
            lines: vec![LineInput {
                ordinal: 0,
                line_id: 1,
                id_type: 1,
                name_order: 0,
                amount: 1,
                flags: 0,
                training_bits: 0,
                max_training_bits: 0,
                manpower_have: 0,
                manpower_need: 0,
                max_manpower: 0,
                reserved: 0,
                equipment: &[],
            }],
            long_name: vec![b'x'; LINE_ROW_LEN],
        }],
    };

    let words = HeaderWords {
        elapsed_wall_ms: 1,
        changed_country_rows: 1,
        status_flags: 0,
    };
    let result = encode_v2(words, core::slice::from_ref(&country));
    match result {
        Err(DqEncodeError::PreflightMismatch { .. }) => {}
        Ok(frame) => {
            let declared_lines = u32::from_le_bytes([frame[28], frame[29], frame[30], frame[31]]);
            assert_eq!(
                declared_lines, 0,
                "the header declares {declared_lines} line(s) but the body carries none"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
