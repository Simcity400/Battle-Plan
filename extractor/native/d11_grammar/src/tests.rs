//! Handwritten byte goldens, coverage drift-killers and structural refusals.
//! No fixture in this file is produced by `crate::encode`.

use alloc::{vec, vec::Vec};

use crate::encode;
use crate::*;

fn base_golden() -> Vec<u8> {
    // 40-byte header, one stockpile, one military line, no naval line, one
    // construction row. The byte fixture below spells the exact grammar.
    vec![
        1, 0, 0, 0, 0, 0, 0, 0, // elapsed_wall_ms
        1, 0, 0, 0, // country_count
        2, 0, 0, 0, // changed_rows
        3, 0, 0, 0, // status_flags
        160, 134, 1, 0, // scale_raw = 100000
        1, 0, 0, 0, // stockpile_count
        1, 0, 0, 0, // military_count
        0, 0, 0, 0, // naval_count
        1, 0, 0, 0, // construction_count
        11, 0, 0, 0, 22, 0, 0, 0, 33, 0, 0, 0, 0, 0, 0, 0, // stockpile
        11, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, // line +0..15
        4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, // line +16..31
        11, 0, 0, 0, 9, 0, 0, 0, 10, 0, 0, 0, 3, 0, 0, 0, // construction +0..15
        4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0, 7, 0, 0, 0, // construction +16..31
    ]
}

fn plic_v1_golden() -> Vec<u8> {
    vec![
        b'P', b'L', b'I', b'C', 1, 0, 20, 0, // prefix
        1, 0, 0, 0, // country_rows
        1, 0, 0, 0, // id_rows
        0, 0, 0, 0, // reserved
        1, 0, 0, 0, 1, 0, 0, 0, // country: tag, count
        9, 0, 0, 0, // v1 item
    ]
}

fn plic_v2_golden() -> Vec<u8> {
    vec![
        b'P', b'L', b'I', b'C', 2, 0, 20, 0, // prefix
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, // counts, reserved
        1, 0, 0, 0, 1, 0, 0, 0, // country
        9, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, // v2 item
    ]
}

fn pvj_golden() -> Vec<u8> {
    vec![
        b'P', b'V', b'J', b'1', 1, 0, 20, 0, 36, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0,
        0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
    ]
}

fn pfr_golden() -> Vec<u8> {
    vec![
        b'P', b'F', b'R', b'1', 1, 0, 20, 0, 36, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0,
        0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
    ]
}

fn ppr_golden() -> Vec<u8> {
    vec![
        b'P', b'P', b'R', b'1', 1, 0, 20, 0, 36, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0,
        0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
    ]
}

fn pls_golden() -> Vec<u8> {
    vec![
        b'P', b'L', b'S', b'1', 1, 0, 20, 0, 36, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0,
        0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
    ]
}

fn pef_golden() -> Vec<u8> {
    vec![
        b'P', b'E', b'F', b'1', 1, 0, 20, 0, 56, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, // header
        1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, // key
        4, 0, 0, 0, 0, 0, 0, 0, // efficiency_raw at +12
        5, 0, 0, 0, 0, 0, 0, 0, // cap at +20
        6, 0, 0, 0, 0, 0, 0, 0, // growth at +28
    ]
}

fn pmi_golden() -> Vec<u8> {
    vec![
        b'P', b'M', b'I', b'1', 1, 0, 20, 0, 36, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0,
        0, 0, 3, 0, 0, 0, 4, 0, 0, 0,
    ]
}

fn pcp_golden() -> Vec<u8> {
    vec![
        b'P', b'C', b'P', b'1', 1, 0, 20, 0, 28, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 7, 0,
        0, 0,
    ]
}

fn pcv_golden() -> Vec<u8> {
    vec![
        b'P', b'C', b'V', b'1', 1, 0, 28, 0, 92, 0, 0, 0, // prefix + segment
        1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, // counts + reserved
        1, 0, 0, 0, 31, 0, 0, 0, // tag, flags
        1, 0, 0, 0, 0, 0, 0, 0, // owned_raw
        2, 0, 0, 0, 3, 0, 0, 0, // assigned, available
        0, 0, 0, 0, 1, 0, 0, 0, // variant start/len
        0, 0, 0, 0, 1, 0, 0, 0, // reservation start/len
        7, 0, 0, 0, 70, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, // variant
        1, 0, 0, 0, 2, 0, 0, 0, // reservation
    ]
}

fn pdq_golden() -> Vec<u8> {
    vec![
        b'P', b'D', b'Q', b'1', 1, 0, 20, 0, 23, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 9, 8, 7,
    ]
}

fn golden(trailer: Trailer) -> Vec<u8> {
    match trailer {
        Trailer::Plic => plic_v1_golden(),
        Trailer::Pvj1 => pvj_golden(),
        Trailer::Pfr1 => pfr_golden(),
        Trailer::Ppr1 => ppr_golden(),
        Trailer::Pls1 => pls_golden(),
        Trailer::Pef1 => pef_golden(),
        Trailer::Pmi1 => pmi_golden(),
        Trailer::Pcp1 => pcp_golden(),
        Trailer::Pcv1 => pcv_golden(),
        Trailer::Pdq1 => pdq_golden(),
    }
}

fn consume_rows<N>(mut rows: BlockCursor<N>) -> Result<N, Violation> {
    while let RowStep::Row { .. } = rows.next() {}
    Ok(rows.into_end()?.next)
}

fn consume_body(body: TrailerBody, payload: &[u8]) -> Result<TrailerCursor, Violation> {
    let resume = match body {
        TrailerBody::Plic(cursor) => {
            let items = consume_rows(cursor.countries())?;
            consume_rows(items.items())?
        }
        TrailerBody::Pvj1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Pfr1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Ppr1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Pls1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Pef1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Pmi1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Pcp1(cursor) => consume_rows(cursor.rows())?,
        TrailerBody::Pcv1(cursor) => {
            let mut countries = cursor.countries();
            loop {
                match countries.next(payload)? {
                    PcvCountryStep::Row { .. } => {}
                    PcvCountryStep::RecoverableRow { ranges, .. } => {
                        panic!("golden PCV1 has bad row ranges: {ranges:?}")
                    }
                    PcvCountryStep::RecoverableFinal { defect, .. } => {
                        panic!("golden PCV1 has bad final ranges: {defect:?}")
                    }
                    PcvCountryStep::End(_) => break,
                }
            }
            let variants = match countries.into_end()? {
                PcvCountryEnd::Complete(end) => end.next,
                PcvCountryEnd::Recoverable { defect, .. } => {
                    panic!("golden PCV1 skipped final recovery: {defect:?}")
                }
            };
            let reservations = consume_rows(variants.variants())?;
            consume_rows(reservations.reservations())?
        }
        TrailerBody::Pdq1(cursor) => consume_rows(cursor.inner_payload())?,
    };
    Ok(resume.resume())
}

fn walk_one(payload: &[u8]) -> Result<(Trailer, usize), Violation> {
    let mut cursor = TrailerCursor::new(0);
    let trailer = match cursor.next(payload)? {
        TrailerStep::Magic { trailer, .. } => trailer,
        TrailerStep::End(_) => panic!("fixture is empty"),
    };
    let body = cursor.read_body(payload)?;
    assert_eq!(body.trailer(), trailer);
    let mut cursor = consume_body(body, payload)?;
    match cursor.next(payload)? {
        TrailerStep::End(end) => Ok((trailer, end)),
        TrailerStep::Magic { .. } => panic!("single fixture carried two trailers"),
    }
}

#[test]
fn the_handwritten_base_golden_walks_all_three_declared_blocks() {
    let payload = base_golden();
    let header = read_base_header(&payload).expect("header");
    assert_eq!(header.stockpile_count, 1);
    assert_eq!(header.military_count, 1);
    assert_eq!(header.naval_count, 0);
    assert_eq!(header.construction_count, 1);
    let stockpiles = BaseCursor::new(&payload, &header)
        .expect("layout")
        .stockpiles();
    let lines = consume_rows(stockpiles).expect("stockpiles");
    let construction = consume_rows(lines.lines()).expect("lines");
    let mut trailers = consume_rows(construction.construction()).expect("construction");
    assert_eq!(
        trailers.next(&payload).expect("end"),
        TrailerStep::End(payload.len())
    );
}

#[test]
fn the_base_encoder_matches_the_literal_golden() {
    let stockpiles = [StockpileRow {
        tag_db_id: 11,
        variant_id: 22,
        amount_raw: 33,
    }];
    let military = [LineRow {
        tag_db_id: 11,
        kind: 1,
        line_id: 2,
        active_factories: 3,
        cost_raw: 4,
        speed_raw: 5,
        produced_raw: 6,
        amount: 7,
    }];
    let construction = [ConstructionRow {
        tag_db_id: 11,
        line_id: 9,
        template_id: 10,
        active_factories: 3,
        cost_raw: 4,
        produced_raw: 5,
        priority: 6,
        reserved: 7,
    }];
    assert_eq!(
        encode::encode_base(
            1,
            encode::BaseWords {
                country_count: 1,
                changed_rows: 2,
                status_flags: 3,
                scale_raw: 100_000,
            },
            &stockpiles,
            &military,
            &[],
            &construction,
        )
        .expect("encode"),
        base_golden()
    );
}

#[test]
fn every_handwritten_trailer_golden_walks_to_its_exact_end() {
    for trailer in Trailer::ALL {
        let bytes = golden(trailer);
        assert_eq!(
            walk_one(&bytes).expect(trailer.label()),
            (trailer, bytes.len())
        );
    }
    assert_eq!(
        walk_one(&plic_v2_golden()).expect("PLIC v2"),
        (Trailer::Plic, plic_v2_golden().len())
    );
}

fn encoded(trailer: Trailer) -> Vec<u8> {
    let key = LineKey {
        tag_db_id: 1,
        kind: 2,
        line_id: 3,
    };
    match trailer {
        Trailer::Plic => {
            let items = [PlicV1Item { variant_id: 9 }];
            encode::encode_plic_v1(&[encode::PlicCountryV1 {
                tag_db_id: 1,
                items: &items,
            }])
            .expect("encode PLIC")
        }
        Trailer::Pvj1 => encode::encode_pvj1(&[PvjRow {
            key,
            equipment_variant_id: 4,
        }])
        .expect("encode PVJ1"),
        Trailer::Pfr1 => encode::encode_pfr1(&[PfrRow {
            key,
            requested_factories: 4,
        }])
        .expect("encode PFR1"),
        Trailer::Ppr1 => encode::encode_ppr1(&[PprRow { key, priority: 4 }]).expect("encode PPR1"),
        Trailer::Pls1 => encode::encode_pls1(&[PlsRow { key, subtype: 4 }]).expect("encode PLS1"),
        Trailer::Pef1 => encode::encode_pef1(&[PefRow {
            key,
            efficiency_raw: 4,
            efficiency_cap_raw: 5,
            efficiency_growth_raw: 6,
        }])
        .expect("encode PEF1"),
        Trailer::Pmi1 => encode::encode_pmi1(&[PmiRow { key, mio_id: 4 }]).expect("encode PMI1"),
        Trailer::Pcp1 => encode::encode_pcp1(&[PcpRow {
            tag_db_id: 1,
            flags: 7,
        }])
        .expect("encode PCP1"),
        Trailer::Pcv1 => {
            let variants = [PcvVariantRow {
                variant_id: 7,
                equip_type: 70,
                amount_raw: 8,
            }];
            let reservations = [PcvReservationRow {
                priority: 1,
                count: 2,
            }];
            encode::encode_pcv1(&[encode::PcvCountryInput {
                tag_db_id: 1,
                flags: 31,
                owned_raw: 1,
                assigned: 2,
                available: 3,
                variants: &variants,
                reservations: &reservations,
            }])
            .expect("encode PCV1")
        }
        Trailer::Pdq1 => encode::encode_pdq1(&[9, 8, 7]).expect("encode PDQ1"),
    }
}

#[test]
fn every_family_has_an_encoder_golden() {
    let with_goldens = [
        Trailer::Plic,
        Trailer::Pvj1,
        Trailer::Pfr1,
        Trailer::Ppr1,
        Trailer::Pls1,
        Trailer::Pef1,
        Trailer::Pmi1,
        Trailer::Pcp1,
        Trailer::Pcv1,
        Trailer::Pdq1,
    ];
    for trailer in Trailer::ALL {
        assert!(
            with_goldens.contains(&trailer),
            "{} has no golden",
            trailer.label()
        );
        assert_eq!(
            encoded(trailer),
            golden(trailer),
            "{} encoder",
            trailer.label()
        );
    }
}

#[test]
fn the_plic_v2_encoder_matches_its_independent_literal_golden() {
    let items = [PlicV2Item {
        variant_id: 9,
        visible_order: 2,
        creator_tag_db_id: 3,
        origin_tag_db_id: 4,
    }];
    assert_eq!(
        encode::encode_plic_v2(&[encode::PlicCountryV2 {
            tag_db_id: 1,
            items: &items,
        }])
        .expect("encode"),
        plic_v2_golden()
    );
}

#[test]
fn every_family_fixture_covers_its_declared_count_words() {
    for trailer in Trailer::ALL {
        let bytes = golden(trailer);
        let actual = match trailer {
            Trailer::Plic => {
                (u32_at(&bytes, 8) != 0) as u32 | (((u32_at(&bytes, 12) != 0) as u32) << 1)
            }
            Trailer::Pvj1
            | Trailer::Pfr1
            | Trailer::Ppr1
            | Trailer::Pls1
            | Trailer::Pef1
            | Trailer::Pmi1
            | Trailer::Pcp1 => (u32_at(&bytes, 12) != 0) as u32,
            Trailer::Pcv1 => {
                (u32_at(&bytes, 12) != 0) as u32
                    | (((u32_at(&bytes, 16) != 0) as u32) << 1)
                    | (((u32_at(&bytes, 20) != 0) as u32) << 2)
            }
            Trailer::Pdq1 => (u32_at(&bytes, 12) != 0) as u32,
        };
        let known = match trailer {
            Trailer::Plic => KNOWN_PLIC_DECLARED_WORDS,
            Trailer::Pvj1
            | Trailer::Pfr1
            | Trailer::Ppr1
            | Trailer::Pls1
            | Trailer::Pef1
            | Trailer::Pmi1
            | Trailer::Pcp1 => KNOWN_FIXED_DECLARED_WORDS,
            Trailer::Pcv1 => KNOWN_PCV_DECLARED_WORDS,
            Trailer::Pdq1 => KNOWN_PDQ_DECLARED_WORDS,
        };
        assert_eq!(actual, known, "{} declared words", trailer.label());
    }
}

#[test]
fn flag_fixture_unions_equal_the_declared_known_masks() {
    let pcp_union = u32_at(&pcp_golden(), 24);
    let pcv_union = u32_at(&pcv_golden(), 32);
    assert_eq!(pcp_union, KNOWN_PCP_FLAGS);
    assert_eq!(pcv_union, KNOWN_PCV_FLAGS);
}

#[test]
fn each_family_refuses_truncation_with_extent_operands() {
    for trailer in Trailer::ALL {
        let mut bytes = golden(trailer);
        bytes.pop();
        let err = TrailerCursor::new(0)
            .read_after_next(&bytes)
            .expect_err(trailer.label());
        assert!(
            matches!(
                err.kind,
                ViolationKind::Need {
                    section: Section::TrailerBody(t),
                    needed,
                    available
                } if t == trailer && needed == bytes.len() + 1 && available == bytes.len()
            ),
            "{}: {:?}",
            trailer.label(),
            err
        );
    }
}

trait ReadAfterNext {
    fn read_after_next(self, payload: &[u8]) -> Result<TrailerBody, Violation>;
}

impl ReadAfterNext for TrailerCursor {
    fn read_after_next(mut self, payload: &[u8]) -> Result<TrailerBody, Violation> {
        self.next(payload)?;
        self.read_body(payload)
    }
}

#[test]
fn each_family_refuses_an_overdeclared_segment_with_the_declared_operands() {
    for trailer in Trailer::ALL {
        let mut bytes = golden(trailer);
        if trailer == Trailer::Plic {
            // Keep the derived extent present, but over-declare id_rows against
            // sum(country.count), which is PLIC's equivalent declaration.
            bytes[12..16].copy_from_slice(&2u32.to_le_bytes());
            bytes.extend_from_slice(&10u32.to_le_bytes());
            let err = TrailerCursor::new(0)
                .read_after_next(&bytes)
                .expect_err("PLIC overdeclared");
            assert_eq!(
                err.kind,
                ViolationKind::PlicItemCount {
                    declared_items: 2,
                    summed_items: 1,
                }
            );
            continue;
        }
        let declared = u32_at(&bytes, 8) as usize + 1;
        bytes[8..12].copy_from_slice(&(declared as u32).to_le_bytes());
        let err = TrailerCursor::new(0)
            .read_after_next(&bytes)
            .expect_err(trailer.label());
        assert!(
            matches!(
                err.kind,
                ViolationKind::DeclaredExtent {
                    trailer: t,
                    declared_len,
                    computed_len
                } if t == trailer && declared_len == declared && computed_len + 1 == declared_len
            ),
            "{}: {:?}",
            trailer.label(),
            err
        );
    }
}

#[test]
fn each_family_refuses_a_non_advancing_declaration() {
    for trailer in Trailer::ALL {
        let mut bytes = golden(trailer);
        if trailer == Trailer::Plic {
            bytes[6..8].copy_from_slice(&0u16.to_le_bytes());
            let err = TrailerCursor::new(0)
                .read_after_next(&bytes)
                .expect_err("PLIC header_len zero");
            assert_eq!(
                err.kind,
                ViolationKind::HeaderLen {
                    trailer,
                    declared: 0,
                    expected: 20,
                }
            );
        } else {
            bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
            let err = TrailerCursor::new(0)
                .read_after_next(&bytes)
                .expect_err(trailer.label());
            assert!(matches!(
                err.kind,
                ViolationKind::DeclaredExtent {
                    trailer: t,
                    declared_len: 0,
                    computed_len
                } if t == trailer && computed_len >= trailer.header_len()
            ));
        }
    }
}

#[test]
fn pcv_range_defects_are_recoverable_steps_and_the_walk_can_resume() {
    let mut bytes = pcv_golden();
    bytes[52..56].copy_from_slice(&2u32.to_le_bytes()); // variant_start, expected 0
    let mut parent = TrailerCursor::new(0);
    parent.next(&bytes).expect("magic");
    let TrailerBody::Pcv1(cursor) = parent.read_body(&bytes).expect("outer body") else {
        panic!("PCV1 expected")
    };
    let mut countries = cursor.countries();
    assert!(matches!(
        countries.next(&bytes).expect("range step"),
        PcvCountryStep::RecoverableRow {
            ranges: PcvCountryRanges {
                variants: PcvRangeStatus::OutOfBounds {
                    expected_start: 0,
                    declared_start: 2,
                    len: 1,
                    end: 3,
                    declared_count: 1
                },
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        countries.next(&bytes).expect("end step"),
        PcvCountryStep::End(_)
    ));
    let variants = match countries.into_end().expect("country end") {
        PcvCountryEnd::Complete(end) => end.next,
        PcvCountryEnd::Recoverable { defect, .. } => {
            panic!("observed recovery remained pending: {defect:?}")
        }
    };
    let reservations = consume_rows(variants.variants()).expect("variants");
    let resume = consume_rows(reservations.reservations()).expect("reservations");
    let mut parent = resume.resume();
    assert_eq!(
        parent.next(&bytes).expect("end"),
        TrailerStep::End(bytes.len())
    );
}

#[test]
fn pcv_out_of_bounds_is_never_yielded_as_canonical() {
    let mut bytes = pcv_golden();
    bytes[56..60].copy_from_slice(&2u32.to_le_bytes()); // len 2, only one variant exists
    let mut parent = TrailerCursor::new(0);
    parent.next(&bytes).expect("magic");
    let TrailerBody::Pcv1(cursor) = parent.read_body(&bytes).expect("outer body") else {
        panic!("PCV1 expected")
    };
    let mut countries = cursor.countries();
    assert!(matches!(
        countries.next(&bytes).expect("country"),
        PcvCountryStep::RecoverableRow {
            ranges: PcvCountryRanges {
                variants: PcvRangeStatus::OutOfBounds {
                    expected_start: 0,
                    declared_start: 0,
                    len: 2,
                    end: 2,
                    declared_count: 1,
                },
                ..
            },
            ..
        }
    ));
}

#[test]
fn pcv_running_starts_follow_lengths_not_a_bad_declared_start() {
    let variants_a = [PcvVariantRow {
        variant_id: 1,
        equip_type: 70,
        amount_raw: 1,
    }];
    let variants_b = [PcvVariantRow {
        variant_id: 2,
        equip_type: 70,
        amount_raw: 1,
    }];
    let countries = [
        encode::PcvCountryInput {
            tag_db_id: 1,
            flags: 0,
            owned_raw: 0,
            assigned: 0,
            available: 0,
            variants: &variants_a,
            reservations: &[],
        },
        encode::PcvCountryInput {
            tag_db_id: 2,
            flags: 0,
            owned_raw: 0,
            assigned: 0,
            available: 0,
            variants: &variants_b,
            reservations: &[],
        },
    ];
    let mut bytes = encode::encode_pcv1(&countries).expect("fixture shape");
    bytes[52..56].copy_from_slice(&1u32.to_le_bytes()); // first expected 0
    bytes[92..96].copy_from_slice(&2u32.to_le_bytes()); // second expected 1, not first start + len
    let mut parent = TrailerCursor::new(0);
    parent.next(&bytes).expect("magic");
    let TrailerBody::Pcv1(cursor) = parent.read_body(&bytes).expect("body") else {
        panic!("PCV1 expected")
    };
    let mut rows = cursor.countries();
    assert!(matches!(
        rows.next(&bytes).expect("first"),
        PcvCountryStep::RecoverableRow {
            ranges: PcvCountryRanges {
                variants: PcvRangeStatus::NonCanonical {
                    expected_start: 0,
                    declared_start: 1,
                    len: 1,
                    ..
                },
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        rows.next(&bytes).expect("second"),
        PcvCountryStep::RecoverableRow {
            ranges: PcvCountryRanges {
                variants: PcvRangeStatus::OutOfBounds {
                    expected_start: 1,
                    declared_start: 2,
                    len: 1,
                    declared_count: 2,
                    ..
                },
                ..
            },
            ..
        }
    ));
}

#[test]
fn every_public_row_reader_refuses_a_short_or_arbitrary_slice() {
    assert!(stockpile_row_at(&[], 0).is_err());
    assert!(line_row_at(&[], 0).is_err());
    assert!(construction_row_at(&[], 0).is_err());
    assert!(pvj_row_at(&[], 0).is_err());
    assert!(pfr_row_at(&[], 0).is_err());
    assert!(ppr_row_at(&[], 0).is_err());
    assert!(pls_row_at(&[], 0).is_err());
    assert!(pef_row_at(&[], 0).is_err());
    assert!(pmi_row_at(&[], 0).is_err());
    assert!(pcp_row_at(&[], 0).is_err());
    assert!(plic_country_row_at(&[], 0).is_err());
    assert!(plic_v1_item_at(&[], 0).is_err());
    assert!(plic_v2_item_at(&[], 0).is_err());
    assert!(pcv_country_row_at(&[], 0).is_err());
    assert!(pcv_variant_row_at(&[], 0).is_err());
    assert!(pcv_reservation_row_at(&[], 0).is_err());

    let bytes = pcv_golden();
    let mut parent = TrailerCursor::new(0);
    parent.next(&bytes).expect("magic");
    let TrailerBody::Pcv1(cursor) = parent.read_body(&bytes).expect("body") else {
        panic!("PCV1 expected")
    };
    let mut countries = cursor.countries();
    assert!(countries.next(&[]).is_err());
    assert!(matches!(
        countries
            .next(&bytes)
            .expect("retry with validated payload"),
        PcvCountryStep::Row { index: 0, .. }
    ));
}

#[test]
fn pcv_final_coverage_cannot_be_skipped_by_into_end() {
    let mut bytes = pcv_golden();
    bytes.drain(28..68); // remove the sole country row
    bytes.truncate(bytes.len() - PCV_RESERVATION_ROW_LEN); // remove reservation block
    bytes[8..12].copy_from_slice(&44u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&0u32.to_le_bytes()); // no countries
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes()); // one uncovered variant
    bytes[20..24].copy_from_slice(&0u32.to_le_bytes());
    let mut parent = TrailerCursor::new(0);
    parent.next(&bytes).expect("magic");
    let TrailerBody::Pcv1(cursor) = parent.read_body(&bytes).expect("body") else {
        panic!("PCV1 expected")
    };
    match cursor.countries().into_end().expect("empty block ends") {
        PcvCountryEnd::Recoverable {
            defect:
                PcvFinalCoverage {
                    covered_variants: Some(0),
                    declared_variants: 1,
                    covered_reservations: Some(0),
                    declared_reservations: 0,
                },
            ..
        } => {}
        other => panic!("coverage was skipped: {other:?}"),
    }
}

#[test]
fn base_truncation_names_the_block_that_is_missing() {
    let payload = base_golden();
    for (available, section, needed) in [
        (BASE_HEADER_LEN, Section::BaseStockpiles, 56usize),
        (56, Section::BaseLines, 88),
        (88, Section::BaseConstruction, 120),
    ] {
        let short = &payload[..available];
        let header = read_base_header(short).expect("header is present");
        let err = base_layout(short, &header).expect_err("body is truncated");
        assert_eq!(
            err.at,
            match section {
                Section::BaseStockpiles => BASE_HEADER_LEN,
                Section::BaseLines => 56,
                Section::BaseConstruction => 88,
                Section::BaseHeader
                | Section::TrailerMagic
                | Section::TrailerHeader(_)
                | Section::TrailerBody(_) => unreachable!(),
            }
        );
        assert_eq!(
            err.kind,
            ViolationKind::Need {
                section,
                needed,
                available,
            }
        );
    }
}

#[test]
fn pdq_exposes_the_inner_span_and_leaves_finality_to_the_site() {
    let bytes = pdq_golden();
    let mut cursor = TrailerCursor::new(0);
    cursor.next(&bytes).expect("magic");
    let TrailerBody::Pdq1(pdq) = cursor.read_body(&bytes).expect("body") else {
        panic!("PDQ1 expected")
    };
    assert_eq!(
        pdq.inner_payload_span(),
        Span {
            start: 20,
            len: 3,
            end: 23
        }
    );
    assert!(pdq.header().ends_at_payload_end(bytes.len()));
    assert!(!pdq.header().ends_at_payload_end(bytes.len() + 1));
}

#[test]
fn block_end_refuses_an_unconsumed_block_as_a_caller_defect() {
    let payload = base_golden();
    let header = read_base_header(&payload).expect("header");
    let err = BaseCursor::new(&payload, &header)
        .expect("layout")
        .stockpiles()
        .into_end()
        .expect_err("unconsumed stockpile");
    assert_eq!(
        err.kind,
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed {
            owner: BlockOwner::Base,
            block: DeclaredBlock::Stockpiles,
            remaining: 1,
        })
    );
}

#[test]
fn unknown_duplicate_version_header_and_reserved_fail_closed() {
    let mut unknown = pvj_golden();
    unknown[0..4].copy_from_slice(b"NOPE");
    let mut cursor = TrailerCursor::new(0);
    assert!(matches!(
        cursor.next(&unknown).expect_err("unknown" ).kind,
        ViolationKind::UnknownTrailerMagic { magic, .. } if magic == *b"NOPE"
    ));

    let mut version = pvj_golden();
    version[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(
        TrailerCursor::new(0)
            .read_after_next(&version)
            .expect_err("version")
            .kind,
        ViolationKind::UnknownVersion { version: 2, .. }
    ));

    let mut header = pvj_golden();
    header[6..8].copy_from_slice(&19u16.to_le_bytes());
    assert!(matches!(
        TrailerCursor::new(0)
            .read_after_next(&header)
            .expect_err("header")
            .kind,
        ViolationKind::HeaderLen {
            declared: 19,
            expected: 20,
            ..
        }
    ));

    let mut reserved = pvj_golden();
    reserved[16..20].copy_from_slice(&9u32.to_le_bytes());
    assert!(matches!(
        TrailerCursor::new(0)
            .read_after_next(&reserved)
            .expect_err("reserved")
            .kind,
        ViolationKind::ReservedWord {
            value: 9,
            at: 16,
            ..
        }
    ));

    let first = pvj_golden();
    let mut both = first.clone();
    both.extend_from_slice(&first);
    let mut cursor = TrailerCursor::new(0);
    cursor.next(&both).expect("first magic");
    let cursor = consume_body(cursor.read_body(&both).expect("first body"), &both).expect("first");
    let mut cursor = cursor;
    assert!(matches!(
        cursor.next(&both).expect_err("duplicate").kind,
        ViolationKind::DuplicateTrailer {
            trailer: Trailer::Pvj1,
            at: 36
        }
    ));
}
