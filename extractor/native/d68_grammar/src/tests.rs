//! The leaf's own suite. Two jobs: prove the walk and the encoder are generated
//! from ONE table, and pin the bytes that table implies against a hand-written
//! layout so a mistake both sides make is still caught.

use alloc::vec::Vec;

use crate::encode::{encode_payload, SessionHeader, TokenTooLong};
use crate::{
    exact_consumption, read_header, read_scalar_block, Field, Section, Slot, TokenCursor,
    TokenStep, Violation, ViolationKind, HEADER_LEN, KNOWN_FLAGS, MAX_TOKEN_LEN, SCALAR_FIELDS,
    SCALAR_OFFSETS, STRING_BLOCK_OFFSET, TOKEN_FIELDS,
};

// --- Helpers ----------------------------------------------------------------

fn walk(payload: &[u8]) -> Result<(u32, [u32; 5], Vec<(Field, Vec<u8>)>), Violation> {
    let header = read_header(payload)?;
    let scalars = read_scalar_block(payload)?;
    let mut cursor = TokenCursor::new();
    let mut tokens = Vec::new();
    let end = loop {
        match cursor.next(payload)? {
            TokenStep::Token { field, bytes } => tokens.push((field, bytes.to_vec())),
            TokenStep::End(at) => break at,
        }
    };
    exact_consumption(end, payload.len())?;
    Ok((header.presence, scalars.words(), tokens))
}

/// Every representable shape of the payload, handwritten. The drift-killer
/// asserts this covers every declared field, so a ninth field that nothing here
/// exercises fails the suite rather than passing it silently.
fn representable_headers() -> Vec<SessionHeader<'static>> {
    let mut out = Vec::new();
    // nothing carried
    out.push(SessionHeader::default());
    // every field carried
    out.push(SessionHeader {
        player_tag_db_id: Some(7),
        dlcs: Some(0x0000_00ff),
        start_date_raw: Some(1936),
        save_version: Some(3),
        session: Some(42),
        ideology: Some("fascism"),
        cosmetic_tag: Some("GER"),
        version: Some("1.19.2"),
    });
    // one field at a time, so every flag appears alone
    out.push(SessionHeader {
        player_tag_db_id: Some(1),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        dlcs: Some(1),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        start_date_raw: Some(1),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        save_version: Some(1),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        session: Some(1),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        ideology: Some("x"),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        cosmetic_tag: Some("x"),
        ..SessionHeader::default()
    });
    out.push(SessionHeader {
        version: Some("x"),
        ..SessionHeader::default()
    });
    out
}

// --- The drift-killer --------------------------------------------------------

/// kills: a declared d68 field written by the encoder and not delimited by the
/// walk, or delimited and not written; a scalar offset the encoder and the walk
/// disagree about; a token order the two disagree about; and a ninth field that
/// no fixture exercises.
///
/// The mechanism is `Field::ALL`: the flag mask, the scalar offsets, the token
/// order, `KNOWN_FLAGS`, the walk and the encoder are all generated from that
/// one table by exhaustive `match`es with no wildcard arm, and all four
/// consuming sites also match on `Field` with no wildcard.
#[test]
fn every_declared_field_is_written_and_delimited_and_the_payload_tiles_exactly() {
    let headers = representable_headers();
    let mut covered = 0u32;

    for header in &headers {
        let payload = encode_payload(header, 1234, 0).expect("fixtures are in range");
        let (presence, words, tokens) = walk(&payload).expect("the encoder's bytes walk");

        // 1. the presence word the encoder wrote is the one the walk reads back
        assert_eq!(presence, header.presence(), "presence round-trip");
        covered |= presence;

        // 2. every scalar field's word is at the offset the table declares, and
        //    an absent one packs zero
        for (i, field) in SCALAR_FIELDS.iter().copied().enumerate() {
            let expected = match field {
                Field::Player => header.player_tag_db_id,
                Field::Dlcs => header.dlcs,
                Field::StartDate => header.start_date_raw,
                Field::SaveVersion => header.save_version,
                Field::Session => header.session,
                Field::Ideology | Field::CosmeticTag | Field::Version => None,
            };
            assert_eq!(words[i], expected.unwrap_or(0), "{field:?} word");
            assert_eq!(
                field.slot(),
                Slot::Scalar {
                    offset: SCALAR_OFFSETS[i]
                },
                "{field:?} slot"
            );
            assert_eq!(
                field.is_present(presence),
                expected.is_some(),
                "{field:?} presence bit"
            );
        }

        // 3. every token field is delimited exactly once, in wire order, with
        //    the bytes the producer was handed
        assert_eq!(tokens.len(), TOKEN_FIELDS.len(), "token count");
        for (i, field) in TOKEN_FIELDS.iter().copied().enumerate() {
            let expected = match field {
                Field::Ideology => header.ideology,
                Field::CosmeticTag => header.cosmetic_tag,
                Field::Version => header.version,
                Field::Player
                | Field::Dlcs
                | Field::StartDate
                | Field::SaveVersion
                | Field::Session => None,
            };
            assert_eq!(tokens[i].0, field, "token {i} field");
            assert_eq!(
                tokens[i].1.as_slice(),
                expected.unwrap_or("").as_bytes(),
                "{field:?} token bytes"
            );
            assert_eq!(
                field.is_present(presence),
                expected.is_some(),
                "{field:?} presence bit"
            );
        }
    }

    // 4. the fixtures exercised every declared field. Without this, a ninth
    //    field with a `None` default would be walked by nobody and every
    //    assertion above would still pass.
    assert_eq!(
        covered, KNOWN_FLAGS,
        "the fixtures do not cover every declared presence flag"
    );
}

#[test]
fn known_flags_is_exactly_the_declared_table() {
    let mut mask = 0u32;
    for field in Field::ALL {
        assert_eq!(mask & field.flag(), 0, "{field:?} reuses a bit");
        mask |= field.flag();
    }
    assert_eq!(mask, KNOWN_FLAGS);
    // Eight fields, eight contiguous bits from zero -- the shape every reader's
    // `presence & !KNOWN_FLAGS` rule is written against.
    assert_eq!(KNOWN_FLAGS, 0b1111_1111);
}

#[test]
fn the_two_field_lists_partition_the_table() {
    let mut seen = Vec::new();
    for field in SCALAR_FIELDS {
        assert!(matches!(field.slot(), Slot::Scalar { .. }));
        seen.push(field);
    }
    for field in TOKEN_FIELDS {
        assert!(matches!(field.slot(), Slot::Token { .. }));
        seen.push(field);
    }
    seen.sort();
    let mut all = Field::ALL.to_vec();
    all.sort();
    assert_eq!(seen, all, "every field is in exactly one list");
}

// --- Byte goldens ------------------------------------------------------------
//
// A round-trip test cannot catch a mistake BOTH sides make: widen the token
// length prefix from two bytes to four in the encoder AND in the cursor and
// every round-trip above still passes. These bytes are written out from the
// documented layout, not produced by the crate.

#[test]
fn the_header_bytes_are_the_bytes_the_layout_says() {
    let payload = encode_payload(&SessionHeader::default(), 0x0102_0304_0506_0708, 0x0000_2000)
        .expect("no tokens");
    assert_eq!(
        &payload[0..HEADER_LEN],
        &[
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // elapsed_wall_ms u64 le
            0x00, 0x00, 0x00, 0x00, // presence u32 le -- nothing carried
            0x00, 0x20, 0x00, 0x00, // status_flags u32 le
        ]
    );
    let header = read_header(&payload).expect("header");
    assert_eq!(header.elapsed_wall_ms, 0x0102_0304_0506_0708);
    assert_eq!(header.presence, 0);
    assert_eq!(header.status_flags, 0x0000_2000);
}

#[test]
fn the_scalar_block_bytes_are_the_bytes_the_layout_says() {
    let payload = encode_payload(
        &SessionHeader {
            player_tag_db_id: Some(0x0000_0001),
            dlcs: Some(0x0000_0002),
            start_date_raw: Some(0x0000_0003),
            save_version: Some(0x0000_0004),
            session: Some(0x0000_0005),
            ..SessionHeader::default()
        },
        0,
        0,
    )
    .expect("no tokens");
    assert_eq!(
        &payload[HEADER_LEN..STRING_BLOCK_OFFSET],
        &[
            0x01, 0x00, 0x00, 0x00, // +16 player_tag_db_id
            0x02, 0x00, 0x00, 0x00, // +20 dlcs
            0x03, 0x00, 0x00, 0x00, // +24 start_date_raw
            0x04, 0x00, 0x00, 0x00, // +28 save_version
            0x05, 0x00, 0x00, 0x00, // +32 session
        ]
    );
    // the five scalar flags, and only those
    assert_eq!(crate::u32_at(&payload, 8), 0b0001_1111);
}

#[test]
fn the_token_block_bytes_are_the_bytes_the_layout_says() {
    let payload = encode_payload(
        &SessionHeader {
            ideology: Some("ab"),
            // cosmetic_tag absent -- still writes its zero prefix
            version: Some("c"),
            ..SessionHeader::default()
        },
        0,
        0,
    )
    .expect("short tokens");
    assert_eq!(
        &payload[STRING_BLOCK_OFFSET..],
        &[
            0x02, 0x00, b'a', b'b', // ideology: u16 len 2 then the bytes
            0x00, 0x00, // cosmetic_tag: absent, a zero-length prefix STILL WRITTEN
            0x01, 0x00, b'c', // version
        ]
    );
    assert_eq!(payload.len(), STRING_BLOCK_OFFSET + 9);
    assert_eq!(
        crate::u32_at(&payload, 8),
        Field::Ideology.flag() | Field::Version.flag()
    );
}

#[test]
fn an_empty_payload_of_absent_fields_is_exactly_forty_two_bytes() {
    // 16 header + 20 scalars + three two-byte zero prefixes.
    let payload = encode_payload(&SessionHeader::default(), 0, 0).expect("no tokens");
    assert_eq!(payload.len(), HEADER_LEN + 20 + 6);
    assert_eq!(payload.len(), 42);
    assert!(walk(&payload).is_ok());
}

// --- Refusals ----------------------------------------------------------------

#[test]
fn a_short_header_refuses_with_its_operands() {
    let err = read_header(&[0u8; HEADER_LEN - 1]).expect_err("short");
    assert_eq!(
        err,
        Violation {
            at: 0,
            kind: ViolationKind::Need {
                section: Section::Header,
                needed: HEADER_LEN,
                available: HEADER_LEN - 1,
            },
        }
    );
}

#[test]
fn a_header_only_payload_has_no_scalar_block() {
    let payload = [0u8; HEADER_LEN];
    // the header itself still reads -- this is the early status exit the fold
    // depends on
    assert!(read_header(&payload).is_ok());
    let err = read_scalar_block(&payload).expect_err("no scalars");
    assert_eq!(
        err,
        Violation {
            at: HEADER_LEN,
            kind: ViolationKind::Need {
                section: Section::ScalarBlock,
                needed: STRING_BLOCK_OFFSET,
                available: HEADER_LEN,
            },
        }
    );
}

#[test]
fn a_truncated_token_prefix_names_its_field() {
    let mut payload = encode_payload(&SessionHeader::default(), 0, 0).expect("no tokens");
    // drop the last token's prefix by one byte
    payload.truncate(payload.len() - 1);
    let err = walk(&payload).expect_err("truncated");
    assert_eq!(
        err.kind,
        ViolationKind::Need {
            section: Section::TokenPrefix(Field::Version),
            needed: STRING_BLOCK_OFFSET + 6,
            available: STRING_BLOCK_OFFSET + 5,
        }
    );
}

#[test]
fn a_token_longer_than_the_ceiling_refuses_before_the_bytes_bound() {
    let mut payload = encode_payload(&SessionHeader::default(), 0, 0).expect("no tokens");
    // claim MAX+1 bytes for `ideology` without supplying them: the ceiling rule
    // must win, because that is the order every reader checks in
    let len = (MAX_TOKEN_LEN + 1) as u16;
    payload[STRING_BLOCK_OFFSET..STRING_BLOCK_OFFSET + 2].copy_from_slice(&len.to_le_bytes());
    let err = walk(&payload).expect_err("too long");
    assert_eq!(
        err.kind,
        ViolationKind::TokenTooLong {
            field: Field::Ideology,
            len: MAX_TOKEN_LEN + 1,
            max: MAX_TOKEN_LEN,
        }
    );
    assert_eq!(err.at, STRING_BLOCK_OFFSET + 2);
}

#[test]
fn a_token_at_the_ceiling_is_legal() {
    let token: alloc::string::String = core::iter::repeat('a').take(MAX_TOKEN_LEN).collect();
    let payload = encode_payload(
        &SessionHeader {
            ideology: Some(&token),
            ..SessionHeader::default()
        },
        0,
        0,
    )
    .expect("at the ceiling");
    let (_, _, tokens) = walk(&payload).expect("walks");
    assert_eq!(tokens[0].1.len(), MAX_TOKEN_LEN);
}

#[test]
fn the_encoder_refuses_a_token_over_the_ceiling_instead_of_truncating_it() {
    let token: alloc::string::String = core::iter::repeat('a').take(MAX_TOKEN_LEN + 1).collect();
    let err = encode_payload(
        &SessionHeader {
            version: Some(&token),
            ..SessionHeader::default()
        },
        0,
        0,
    )
    .expect_err("over the ceiling");
    assert_eq!(
        err,
        TokenTooLong {
            field: Field::Version,
            len: MAX_TOKEN_LEN + 1,
            max: MAX_TOKEN_LEN,
        }
    );
}

#[test]
fn truncated_token_bytes_refuse_with_the_end_offset() {
    let mut payload = encode_payload(
        &SessionHeader {
            ideology: Some("abcd"),
            ..SessionHeader::default()
        },
        0,
        0,
    )
    .expect("short token");
    payload.truncate(STRING_BLOCK_OFFSET + 2 + 2);
    let err = walk(&payload).expect_err("truncated");
    assert_eq!(
        err.kind,
        ViolationKind::Need {
            section: Section::TokenBytes(Field::Ideology),
            needed: STRING_BLOCK_OFFSET + 2 + 4,
            available: STRING_BLOCK_OFFSET + 4,
        }
    );
}

#[test]
fn trailing_bytes_refuse_with_both_operands() {
    let mut payload = encode_payload(&SessionHeader::default(), 0, 0).expect("no tokens");
    payload.push(0xAA);
    let err = walk(&payload).expect_err("trailing");
    assert_eq!(
        err.kind,
        ViolationKind::TrailingBytes {
            consumed: 42,
            available: 43,
        }
    );
}

#[test]
fn the_cursor_reports_end_repeatedly_and_never_moves_past_it() {
    let payload = encode_payload(&SessionHeader::default(), 0, 0).expect("no tokens");
    let mut cursor = TokenCursor::new();
    for _ in 0..TOKEN_FIELDS.len() {
        assert!(matches!(
            cursor.next(&payload),
            Ok(TokenStep::Token { .. })
        ));
    }
    let at = match cursor.next(&payload) {
        Ok(TokenStep::End(at)) => at,
        other => panic!("expected End, got {other:?}"),
    };
    assert_eq!(at, payload.len());
    // asking again is not a misuse state: it is the same answer
    assert_eq!(cursor.next(&payload), Ok(TokenStep::End(at)));
    assert_eq!(cursor.offset(), at);
}

#[test]
fn a_scalar_word_lookup_of_a_token_field_is_none_not_a_panic() {
    let payload = encode_payload(&SessionHeader::default(), 0, 0).expect("no tokens");
    let block = read_scalar_block(&payload).expect("scalars");
    assert_eq!(block.word(Field::Player), Some(0));
    assert_eq!(block.word(Field::Ideology), None);
    assert_eq!(block.word(Field::CosmeticTag), None);
    assert_eq!(block.word(Field::Version), None);
}
