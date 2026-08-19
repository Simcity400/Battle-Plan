use alloc::vec;
use alloc::vec::Vec;

use crate::encode::{
    country_row_bytes, encode_active_row, encode_header, encode_xui09_tail,
    xui09_country_vector_body, A29Provenance, ActiveRow, Ahead, DerivedAuthority, Xui09Tail,
};
use crate::*;

#[test]
fn the_tail_table_is_a_partition_of_the_flag_word() {
    let mut seen = 0u32;
    for tail in ActiveTail::ALL {
        let bit = tail.flag_bit();
        assert_eq!(bit.count_ones(), 1, "{tail:?} must claim exactly one bit");
        assert_eq!(bit & seen, 0, "{tail:?} reuses a bit another tail claims");
        assert_eq!(
            bit & ACTIVE_VALUE_FLAGS,
            0,
            "{tail:?} reuses a bit that gates a fixed field"
        );
        assert_eq!(
            bit & ACTIVE_KNOWN_FLAGS,
            bit,
            "{tail:?} is declared but not known"
        );
        seen |= bit;
    }
    assert_eq!(
        ACTIVE_KNOWN_FLAGS,
        ACTIVE_VALUE_FLAGS | seen,
        "the known-flag word is not exactly the value flags plus the tail table"
    );
    assert_eq!(
        ACTIVE_KNOWN_FLAGS.count_ones() as usize,
        2 + ActiveTail::ALL.len(),
        "the known-flag word has a bit no declaration accounts for"
    );
}

#[test]
fn every_declared_tail_is_written_and_delimited_and_the_row_tiles_exactly() {
    let mut exercised = 0u32;
    for shape in representable_rows() {
        exercised |= shape.row.flags();
    }
    assert_eq!(
        exercised,
        ACTIVE_KNOWN_FLAGS,
        "a declared flag bit is never set by any fixture: {:#x} unexercised",
        ACTIVE_KNOWN_FLAGS & !exercised
    );
    for shape in representable_rows() {
        let mut bytes = Vec::new();
        encode_active_row(&shape.row, &mut bytes);
        let flags = shape.row.flags();

        let (mut cursor, unknown) = ActiveRowCursor::begin(&bytes, 0).expect("a written row walks");
        assert_eq!(unknown, None, "the encoder wrote an unassigned flag bit");
        assert_eq!(cursor.flags(), flags, "the walk read a different flag word");

        let mut covered = ACTIVE_ROW_FIXED_LEN;
        let mut walked: Vec<ActiveTail> = Vec::new();
        let end = loop {
            match cursor.next_step().expect("a written row has no truncation") {
                ActiveStep::Section(section) => {
                    let prefix = if section.tail == ActiveTail::TechName {
                        2
                    } else {
                        0
                    };
                    assert_eq!(
                        section.span.start - prefix,
                        covered,
                        "{:?} does not begin where the previous section ended",
                        section.tail
                    );
                    covered = section.span.end;
                    walked.push(section.tail);
                }
                ActiveStep::Recoverable(violation) => {
                    panic!("a representable row broke a rule: {violation:?}")
                }
                ActiveStep::End(end) => break end,
            }
        };
        assert_eq!(covered, end, "the row's last section does not end the row");
        assert_eq!(
            end,
            bytes.len(),
            "the walk and the encoder disagree on the row's length"
        );
        assert_eq!(
            end,
            active_row_len(&bytes, 0).expect("the length-only walk agrees"),
            "the length-only entry point disagrees with the cursor"
        );

        let expected: Vec<ActiveTail> = ActiveTail::ALL
            .into_iter()
            .filter(|tail| flags & tail.flag_bit() != 0)
            .collect();
        assert_eq!(
            walked, expected,
            "the walk did not yield every present tail, in wire order"
        );
    }
}

#[test]
fn the_encoded_length_is_the_sum_of_the_declared_section_lengths() {
    for shape in representable_rows() {
        let mut bytes = Vec::new();
        encode_active_row(&shape.row, &mut bytes);
        let flags = shape.row.flags();
        let mut expected = ACTIVE_ROW_FIXED_LEN;
        for tail in ActiveTail::ALL {
            if flags & tail.flag_bit() == 0 {
                continue;
            }
            expected += match tail.fixed_len() {
                Some(declared) => declared,
                None => 2 + shape.row.tech_name.map_or(0, str::len),
            };
        }
        assert_eq!(
            bytes.len(),
            expected,
            "flags {flags:#x}: the encoder and the declared lengths disagree"
        );
    }
}

#[test]
fn the_country_row_is_the_bytes_the_layout_says() {
    assert_eq!(
        country_row_bytes(0x0102_0304, Some(7), None, true),
        [
            0x04, 0x03, 0x02, 0x01, 0x05, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0xff, 0xff,
            0xff, 0xff,
        ]
    );
}

#[test]
fn the_country_vector_bodies_are_the_bytes_the_layout_says() {
    assert_eq!(
        xui09_country_vector_body(&[(0, 1), (1, 2), (2, 3)], &[7, 9]),
        vec![
            0x01, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x02,
            0x03, 0x07, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00,
        ]
    );
    assert_eq!(
        xui09_country_vector_body(&[(5, 1), (0, 2)], &[]),
        vec![
            0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00,
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x02,
        ]
    );
}

#[test]
fn an_active_row_with_a_name_a_quartet_and_a_derived_authority_is_the_bytes_the_layout_says() {
    let row = ActiveRow {
        tag_db_id: 1,
        slot_index: 2,
        tech_handle_raw: 3,
        tech_def_index_raw: 3,
        progress_raw: 4,
        live_progress_raw: Some(5),
        base_required_points_raw: None,
        tech_name: Some("ab"),
        ahead: Some(Ahead::Derived {
            quartet: [1, 2, 3, 4],
            authority: DerivedAuthority {
                authority: 1,
                producer_game_hour_raw: 6,
                penalty_factor_raw: 7,
            },
        }),
    };
    let mut bytes = Vec::new();
    encode_active_row(&row, &mut bytes);
    #[rustfmt::skip]
    let expected: Vec<u8> = vec![
        1, 0, 0, 0,
        2, 0, 0, 0,
        3, 0, 0, 0,
        3, 0, 0, 0,
        4, 0, 0, 0,
        0b10_1101, 0, 0, 0,
        5, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        2, 0, b'a', b'b',
        1, 0, 0, 0, 0, 0, 0, 0,
        2, 0, 0, 0, 0, 0, 0, 0,
        3, 0, 0, 0, 0, 0, 0, 0,
        4, 0, 0, 0, 0, 0, 0, 0,
        1, 0, 0, 0,
        6, 0, 0, 0,
        7, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(bytes, expected);
    assert_eq!(
        bytes.len(),
        ACTIVE_ROW_FIXED_LEN + 4 + AHEAD_QUARTET_LEN + A29_DERIVED_AUTHORITY_LEN
    );
}

#[test]
fn the_xui09_tail_header_and_sets_are_the_bytes_the_layout_says() {
    let body = xui09_country_vector_body(&[(0, 9)], &[]);
    let vectors: [(u32, &[u8]); 1] = [(11, &body)];
    let mut bytes = Vec::new();
    encode_xui09_tail(
        Xui09Tail {
            patch: true,
            dictionary: [(4u32, "x")].into_iter(),
            measured: [11u32].into_iter(),
            refused: [(12u32, 3u32)].into_iter(),
            vectors: vectors.iter().map(|(tag, body)| (*tag, *body)),
        },
        &mut bytes,
    );
    #[rustfmt::skip]
    let expected_header: Vec<u8> = vec![
        b'X', b'U', b'0', b'9',
        1, 0, 0, 0,
        1, 0, 0, 0,
        1, 0, 0, 0,
        1, 0, 0, 0,
        1, 0, 0, 0,
        4, 0, 0, 0, 1, 0, b'x',
        11, 0, 0, 0,
        12, 0, 0, 0, 3, 0, 0, 0,
        11, 0, 0, 0,
    ];
    assert_eq!(&bytes[..expected_header.len()], &expected_header[..]);
    assert_eq!(&bytes[expected_header.len()..], &body[..]);
    assert_eq!(
        bytes.len(),
        xui09_tail_len(&bytes, 0).expect("a written tail walks")
    );
}

#[test]
fn exactly_the_unassigned_bits_are_reported_unknown() {
    for flags in 0u32..64 {
        let bytes = fixed_row_with_flags(flags);
        let (_, unknown) = ActiveRowCursor::begin(&bytes, 0).expect("40 bytes are present");
        assert_eq!(
            unknown.is_some(),
            flags & !ACTIVE_KNOWN_FLAGS != 0,
            "flags {flags:#x}"
        );
    }
    let bytes = fixed_row_with_flags(1 << 6);
    let (_, unknown) = ActiveRowCursor::begin(&bytes, 0).expect("40 bytes are present");
    assert_eq!(
        unknown.map(|violation| violation.kind),
        Some(ViolationKind::UnknownFlags {
            word: FlagWord::ActiveRow,
            value: 1 << 6,
            unknown: 1 << 6,
        })
    );
}

#[test]
fn the_combination_rules_fire_in_the_readers_order_and_stay_walkable() {
    let quartet = ActiveTail::AheadQuartet.flag_bit();
    let observed = ActiveTail::ObservedAuthority.flag_bit();
    let derived = ActiveTail::DerivedAuthority.flag_bit();
    for (flags, expected) in [
        (
            observed,
            vec![RuleViolation::ObservedAuthorityWithoutQuartet],
        ),
        (derived, vec![RuleViolation::DerivedAuthorityWithoutQuartet]),
        (
            observed | derived,
            vec![
                RuleViolation::ObservedAuthorityWithoutQuartet,
                RuleViolation::DerivedAuthorityWithoutQuartet,
                RuleViolation::BothAuthorities,
            ],
        ),
        (
            quartet | observed | derived,
            vec![RuleViolation::BothAuthorities],
        ),
        (quartet, vec![RuleViolation::QuartetWithoutAuthority]),
    ] {
        let bytes = row_bytes_for_flags(flags);
        let (mut cursor, unknown) = ActiveRowCursor::begin(&bytes, 0).expect("row begins");
        assert_eq!(unknown, None, "flags {flags:#x} are all assigned");
        let mut rules = Vec::new();
        loop {
            match cursor.next_step().expect("the row is long enough") {
                ActiveStep::Section(_) => {}
                ActiveStep::Recoverable(violation) => match violation.kind {
                    ViolationKind::Rule { rule, flags: word } => {
                        assert_eq!(word, flags);
                        rules.push(rule);
                    }
                    other => panic!("unexpected recoverable violation {other:?}"),
                },
                ActiveStep::End(end) => {
                    assert_eq!(end, bytes.len(), "the walk still delimited the whole row");
                    break;
                }
            }
        }
        assert_eq!(rules, expected, "flags {flags:#x}");
    }
}

#[test]
fn truncation_is_reported_against_the_section_it_happens_in() {
    let row = ActiveRow {
        tag_db_id: 1,
        slot_index: 0,
        tech_handle_raw: 0,
        tech_def_index_raw: 0,
        progress_raw: 0,
        live_progress_raw: None,
        base_required_points_raw: None,
        tech_name: Some("abc"),
        ahead: Some(Ahead::Derived {
            quartet: [0, 0, 0, 0],
            authority: DerivedAuthority {
                authority: 1,
                producer_game_hour_raw: 0,
                penalty_factor_raw: 0,
            },
        }),
    };
    let mut bytes = Vec::new();
    encode_active_row(&row, &mut bytes);
    let full = bytes.len();
    let mut sections = Vec::new();
    for cut in 0..full {
        let violation = active_row_len(&bytes[..cut], 0).expect_err("a cut row cannot be walked");
        match violation.kind {
            ViolationKind::Need {
                section, available, ..
            } => {
                assert_eq!(available, cut);
                if sections.last() != Some(&section) {
                    sections.push(section);
                }
            }
            other => panic!("cut {cut} gave {other:?}"),
        }
    }
    assert_eq!(
        sections,
        vec![
            Section::ActiveFixed,
            Section::TechNamePrefix,
            Section::TechNameBytes,
            Section::AheadQuartet,
            Section::DerivedAuthority,
        ],
        "the row's sections are not reported in wire order"
    );
    assert_eq!(active_row_len(&bytes, 0), Ok(full));
}

/// validates a refusal's reason as it reads it; the coarse bounds check those
#[test]
fn a_measured_set_truncation_carries_the_aggregate_the_length_readers_print() {
    let mut bytes = Vec::new();
    encode_xui09_tail(
        Xui09Tail {
            patch: false,
            dictionary: core::iter::empty(),
            measured: [1u32, 2, 3].into_iter(),
            refused: [(4u32, 1u32)].into_iter(),
            vectors: core::iter::empty(),
        },
        &mut bytes,
    );
    let sets_start = XUI09_TAIL_HEADER_LEN;
    let sets_len = 3 * XUI09_MEASURED_ENTRY_LEN + XUI09_REFUSAL_ENTRY_LEN;
    assert_eq!(bytes.len(), sets_start + sets_len);
    for cut in sets_start..bytes.len() {
        let violation = xui09_tail_len(&bytes[..cut], 0).expect_err("a cut tail cannot be walked");
        match violation.kind {
            ViolationKind::Need {
                section,
                aggregate: Some(aggregate),
                ..
            } => {
                assert!(
                    matches!(section, Section::Xui09MeasuredTag | Section::Xui09Refusal),
                    "cut {cut}: {section:?}"
                );
                assert_eq!(aggregate.section, Section::Xui09MeasuredAndRefusalSets);
                assert_eq!(aggregate.at, sets_start);
                assert_eq!(aggregate.len, sets_len);
            }
            other => panic!("cut {cut} gave {other:?}"),
        }
    }
}

#[test]
fn a_country_vector_yields_its_header_before_its_rows_are_needed() {
    let body = xui09_country_vector_body(&[(0, 1), (1, 2)], &[3]);
    let vectors: [(u32, &[u8]); 1] = [(77, &body)];
    let mut bytes = Vec::new();
    encode_xui09_tail(
        Xui09Tail {
            patch: false,
            dictionary: core::iter::empty(),
            measured: [77u32].into_iter(),
            refused: core::iter::empty(),
            vectors: vectors.iter().map(|(tag, body)| (*tag, *body)),
        },
        &mut bytes,
    );
    let header_end = XUI09_TAIL_HEADER_LEN + XUI09_MEASURED_ENTRY_LEN + XUI09_VECTOR_HEADER_LEN;
    let cut = &bytes[..header_end];
    let (mut cursor, unknown) = Xui09Cursor::begin(cut, 0)
        .expect("the tail header is present")
        .expect("a tail is present");
    assert_eq!(unknown, None);
    assert_eq!(
        cursor.next_step(),
        Ok(Xui09Step::MeasuredTag { tag_db_id: 77 })
    );
    assert_eq!(
        cursor.next_step(),
        Ok(Xui09Step::CountryVectorHeader {
            tag_db_id: 77,
            flags: XUI09_COUNTRY_FLAG_DENSE,
            dense: true,
            state_count: 2,
            completed_count: 1,
        }),
        "the header must arrive before the rows are bounds-checked"
    );
    let rows = cursor.next_step().expect_err("the rows are not there");
    assert!(matches!(
        rows.kind,
        ViolationKind::Need {
            section: Section::Xui09VectorRows,
            ..
        }
    ));
}

#[test]
fn unassigned_xui09_flags_are_recoverable_and_the_length_walk_ignores_them() {
    let mut bytes = Vec::new();
    encode_xui09_tail(
        Xui09Tail {
            patch: false,
            dictionary: core::iter::empty(),
            measured: core::iter::empty(),
            refused: core::iter::empty(),
            vectors: core::iter::empty(),
        },
        &mut bytes,
    );
    let full = bytes.len();
    bytes[4..8].copy_from_slice(&(XUI09_FLAG_PATCH | 1 << 7).to_le_bytes());
    let (_, unknown) = Xui09Cursor::begin(&bytes, 0)
        .expect("the header is present")
        .expect("a tail is present");
    assert_eq!(
        unknown.map(|violation| violation.kind),
        Some(ViolationKind::UnknownFlags {
            word: FlagWord::Xui09Tail,
            value: XUI09_FLAG_PATCH | 1 << 7,
            unknown: 1 << 7,
        })
    );
    assert_eq!(xui09_tail_len(&bytes, 0), Ok(full));
}

#[test]
fn an_absent_tail_is_zero_bytes_and_not_an_error() {
    let payload = [0u8; 8];
    assert_eq!(xui09_tail_len(&payload, payload.len()), Ok(0));
    assert!(matches!(
        Xui09Cursor::begin(&payload, payload.len()),
        Ok(None)
    ));
}

#[test]
fn only_xu09_is_a_tail() {
    let mut bytes = [0u8; XUI09_TAIL_HEADER_LEN];
    bytes[0..4].copy_from_slice(b"XU08");
    assert_eq!(
        xui09_tail_len(&bytes, 0).map_err(|violation| violation.kind),
        Err(ViolationKind::BadMagic {
            found: u32::from_le_bytes(*b"XU08"),
            expected: XUI09_TAIL_MAGIC,
        })
    );
}

#[test]
fn the_header_and_the_body_layout_are_what_the_readers_expect() {
    let mut payload = Vec::new();
    encode_header(
        &Header {
            elapsed_wall_ms: 9,
            country_count: 2,
            changed_rows: 2,
            status_flags: 0,
            progress_scale_raw: TECH_PROGRESS_SCALE_RAW,
            live_scale_raw: TECH_LIVE_SCALE_RAW,
            active_count: 1,
        },
        &mut payload,
    );
    assert_eq!(payload.len(), HEADER_LEN);
    payload.extend_from_slice(&country_row_bytes(1, Some(0), Some(0), true));
    payload.extend_from_slice(&country_row_bytes(2, None, None, false));
    let header = read_header(&payload).expect("the header is present");
    assert_eq!(header.country_count, 2);
    assert_eq!(header.active_count, 1);
    assert_eq!(
        body_layout(&payload, &header)
            .expect_err("one active row is missing")
            .kind,
        ViolationKind::Need {
            section: Section::PayloadMinimum,
            at: 0,
            len: HEADER_LEN + 2 * COUNTRY_ROW_LEN + ACTIVE_ROW_FIXED_LEN,
            available: payload.len(),
            aggregate: None,
        }
    );
    payload.extend_from_slice(&fixed_row_with_flags(0));
    let layout = body_layout(&payload, &header).expect("the minimum is met");
    assert_eq!(
        layout.countries,
        HEADER_LEN..HEADER_LEN + 2 * COUNTRY_ROW_LEN
    );
    assert_eq!(layout.active_start, layout.countries.end);
    assert_eq!(layout.minimum_len, payload.len());
    let row = read_country_row(&payload, HEADER_LEN).expect("the first row is present");
    assert_eq!(row.tag_db_id, 1);
    assert!(row.completed_captured() && row.slots_captured() && row.active_research_captured());
    assert_eq!(row.unknown_flags(), 0);
    let absent = read_country_row(&payload, HEADER_LEN + COUNTRY_ROW_LEN).expect("second row");
    assert!(!absent.completed_captured());
    assert_eq!(absent.completed_tech_count, COUNTRY_COUNT_ABSENT_SENTINEL);
}

#[test]
fn the_views_read_the_fields_at_the_declared_offsets() {
    let provenance = A29Provenance {
        authority: 3,
        localization_branch: 2,
        callback_sequence: 0x1122_3344_5566_7788,
        callback_thread_id: 41,
        producer_game_hour_raw: 60_832_407,
        callback_qpc_ticks: 900,
        qpc_frequency_hz: 10_000_000,
        source_record_sha256: [1u8; 32],
        ui_action_gate_return_rva: 0x01BC_F384,
        selected_present_candidate_count: 5,
        selected_present_ordinal: 2,
        selected_present_game_hour_raw: 60_832_407,
        selected_present_candidate_id: 0x0102_0304_0506_0708,
        selected_present_qpc_ticks: 1000,
        selected_present_cutoff_qpc_ticks: 1001,
        selected_present_candidate_set_sha256: [2u8; 32],
        selected_present_receipt_sha256: [3u8; 32],
    };
    let row = ActiveRow {
        tag_db_id: 1,
        slot_index: 2,
        tech_handle_raw: 3,
        tech_def_index_raw: 3,
        progress_raw: 0,
        live_progress_raw: None,
        base_required_points_raw: None,
        tech_name: None,
        ahead: Some(Ahead::Observed {
            quartet: [10, 20, 0, 20],
            provenance: &provenance,
        }),
    };
    let mut bytes = Vec::new();
    encode_active_row(&row, &mut bytes);
    let (mut cursor, _) = ActiveRowCursor::begin(&bytes, 0).expect("row begins");
    let mut quartet = None;
    let mut receipt = None;
    loop {
        match cursor.next_step().expect("the row is complete") {
            ActiveStep::Section(section) => match section.tail {
                ActiveTail::AheadQuartet => {
                    quartet = AheadQuartetView::new(&bytes, section.span);
                }
                ActiveTail::ObservedAuthority => {
                    receipt = A29ProvenanceView::new(&bytes, section.span);
                }
                ActiveTail::TechName | ActiveTail::DerivedAuthority => {
                    panic!("this row carries neither")
                }
            },
            ActiveStep::Recoverable(violation) => panic!("{violation:?}"),
            ActiveStep::End(_) => break,
        }
    }
    assert_eq!(quartet.expect("a quartet").tuple(), (10, 20, 0, 20));
    let receipt = receipt.expect("a receipt");
    assert_eq!(receipt.authority(), 3);
    assert_eq!(receipt.localization_branch(), 2);
    assert_eq!(receipt.callback_sequence(), 0x1122_3344_5566_7788);
    assert_eq!(receipt.callback_thread_id(), 41);
    assert_eq!(receipt.producer_game_hour_raw(), 60_832_407);
    assert_eq!(receipt.callback_qpc_ticks(), 900);
    assert_eq!(receipt.qpc_frequency_hz(), 10_000_000);
    assert_eq!(receipt.source_record_sha256(), &[1u8; 32]);
    assert_eq!(receipt.ui_action_gate_return_rva(), 0x01BC_F384);
    assert_eq!(receipt.selected_present_candidate_count(), 5);
    assert_eq!(receipt.selected_present_ordinal(), 2);
    assert_eq!(receipt.selected_present_game_hour_raw(), 60_832_407);
    assert_eq!(
        receipt.selected_present_candidate_id(),
        0x0102_0304_0506_0708
    );
    assert_eq!(receipt.selected_present_qpc_ticks(), 1000);
    assert_eq!(receipt.selected_present_cutoff_qpc_ticks(), 1001);
    assert_eq!(receipt.selected_present_candidate_set_sha256(), &[2u8; 32]);
    assert_eq!(receipt.selected_present_receipt_sha256(), &[3u8; 32]);
    assert!(
        A29ProvenanceView::new(&bytes, 0..4).is_none(),
        "a wrong-width span is refused"
    );
}

#[test]
fn an_empty_name_is_written_as_an_absent_one() {
    let mut with_empty = Vec::new();
    encode_active_row(&row_with_name(Some("")), &mut with_empty);
    let mut without = Vec::new();
    encode_active_row(&row_with_name(None), &mut without);
    assert_eq!(with_empty, without);
    assert_eq!(
        u32_at(&with_empty, 20) & ActiveTail::TechName.flag_bit(),
        0,
        "an empty name must not claim the name tail"
    );
}

#[test]
fn exact_consumption_is_one_rule() {
    assert_eq!(exact_consumption(4, 4), Ok(()));
    assert_eq!(
        exact_consumption(4, 7).map_err(|violation| violation.kind),
        Err(ViolationKind::TrailingBytes {
            consumed: 4,
            total: 7
        })
    );
}

const PROVENANCE: A29Provenance = A29Provenance {
    authority: 3,
    localization_branch: 1,
    callback_sequence: 1,
    callback_thread_id: 1,
    producer_game_hour_raw: 1,
    callback_qpc_ticks: 1,
    qpc_frequency_hz: 1,
    source_record_sha256: [1u8; 32],
    ui_action_gate_return_rva: 0x01BC_F384,
    selected_present_candidate_count: 1,
    selected_present_ordinal: 0,
    selected_present_game_hour_raw: 1,
    selected_present_candidate_id: 1,
    selected_present_qpc_ticks: 1,
    selected_present_cutoff_qpc_ticks: 1,
    selected_present_candidate_set_sha256: [1u8; 32],
    selected_present_receipt_sha256: [1u8; 32],
};

struct Shape {
    row: ActiveRow<'static>,
}

fn base_row() -> ActiveRow<'static> {
    ActiveRow {
        tag_db_id: 47,
        slot_index: 1,
        tech_handle_raw: 2,
        tech_def_index_raw: 2,
        progress_raw: 3,
        live_progress_raw: None,
        base_required_points_raw: None,
        tech_name: None,
        ahead: None,
    }
}

fn row_with_name(name: Option<&'static str>) -> ActiveRow<'static> {
    ActiveRow {
        tech_name: name,
        ..base_row()
    }
}

fn representable_rows() -> Vec<Shape> {
    let mut shapes = Vec::new();
    for live in [None, Some(5i64)] {
        for base in [None, Some(7i64)] {
            for name in [None, Some("infantry_weapons")] {
                for ahead in [
                    None,
                    Some(Ahead::Observed {
                        quartet: [1, 2, 0, 2],
                        provenance: &PROVENANCE,
                    }),
                    Some(Ahead::Derived {
                        quartet: [1, 2, 0, 2],
                        authority: DerivedAuthority {
                            authority: 1,
                            producer_game_hour_raw: 1,
                            penalty_factor_raw: 1,
                        },
                    }),
                ] {
                    shapes.push(Shape {
                        row: ActiveRow {
                            live_progress_raw: live,
                            base_required_points_raw: base,
                            tech_name: name,
                            ahead,
                            ..base_row()
                        },
                    });
                }
            }
        }
    }
    shapes
}

fn fixed_row_with_flags(flags: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; ACTIVE_ROW_FIXED_LEN];
    bytes[20..24].copy_from_slice(&flags.to_le_bytes());
    bytes
}

fn row_bytes_for_flags(flags: u32) -> Vec<u8> {
    let mut bytes = fixed_row_with_flags(flags);
    for tail in ActiveTail::ALL {
        if flags & tail.flag_bit() == 0 {
            continue;
        }
        match tail {
            ActiveTail::TechName => {
                bytes.extend_from_slice(&2u16.to_le_bytes());
                bytes.extend_from_slice(b"ab");
            }
            ActiveTail::AheadQuartet
            | ActiveTail::ObservedAuthority
            | ActiveTail::DerivedAuthority => {
                bytes.extend(core::iter::repeat(0u8).take(tail.fixed_len().unwrap_or(0)));
            }
        }
    }
    bytes
}
