#[test]
fn selected_parsing_still_rejects_malformed_omitted_updates() {
    let frame = FrameEncoder::new().encode(source(1, 0), true).unwrap();
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    let selected = [TableId::Countries];
    let projected = Frame::decode_selected(&mut Decoder::new(&bytes), Some(&selected)).unwrap();
    assert_eq!(
        projected.tables[TableId::Countries.index()],
        frame.tables[TableId::Countries.index()]
    );
    assert!(matches!(
        projected.tables[TableId::Clock.index()],
        TableUpdate::Unavailable { .. }
    ));
    let mut header = Decoder::new(&bytes);
    header.u64().unwrap();
    header.i32().unwrap();
    header.bool().unwrap();
    header.u8().unwrap();
    header.u64().unwrap();
    for _ in TableId::ALL
        .into_iter()
        .filter(|&table| crate::frame::current_absence(table).is_none())
    {
        header.u64().unwrap();
    }
    header.u64().unwrap();
    header.u64().unwrap();
    let offset = bytes.len() - header.remaining();
    bytes[offset] = u8::MAX;
    assert!(Frame::decode_selected(&mut Decoder::new(&bytes), Some(&selected)).is_err());
}

#[test]
fn selected_reconstruction_matches_full_decoder_through_baseline_loss() {
    let mut encoder = super::FrameEncoder::new();
    let mut full = super::FrameDecoder::new();
    let selected = [
        crate::rows::TableId::Countries,
        crate::rows::TableId::Campaign,
    ];
    let mut projected = super::FrameDecoder::for_tables(&selected);
    let mut omitted: Option<super::SharedView> = None;
    for hour in 0..8 {
        let mut frame = encoder.encode(source(1, hour), hour == 6).unwrap();
        if hour == 2 {
            frame.tables[crate::rows::TableId::Countries.index()] =
                super::TableUpdate::Unavailable {
                    reason: "lost baseline".into(),
                };
        }
        let expected = full.advance(&frame).unwrap();
        let actual = projected.advance(&frame).unwrap();
        for table in selected {
            assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
        }
        let clock = &actual.tables[crate::rows::TableId::Clock.index()];
        assert!(matches!(**clock, super::TableView::Unavailable { .. }));
        assert!(omitted.is_none_or(|previous| super::SharedView::ptr_eq(&previous, clock)));
        omitted = Some(clock.clone());
    }
    let mut invalid = encoder.encode(source(1, 8), true).unwrap();
    invalid.tables[crate::rows::TableId::Clock.index()] =
        super::TableUpdate::Unchanged { baseline: 7 };
    assert!(
        projected.advance(&invalid).is_err(),
        "omitted tables still obey frame-wide validation"
    );
}

use super::*;
use crate::rows::{
    CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionRow, FactionMemberRow, FactionRow,
    ProvinceRow, RowCodec, StateRow, WarRelationRow, tables,
};

fn selected_delta_test_wire(
    table: TableId,
    status: u8,
    payload: &[u8],
    format: crate::format::FormatVersion,
    keyframe: bool,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&1_i32.to_le_bytes());
    bytes.push(u8::from(keyframe));
    if format.version() >= crate::rows::compatibility::CAPTURE_MODE_FORMAT {
        bytes.push(crate::scope::CaptureMode::Full.byte());
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    let stored =
        |recorded| super::wire::table_stored(recorded, format, crate::scope::CaptureMode::Full);
    for recorded in TableId::ALL {
        if stored(recorded) {
            bytes.extend_from_slice(&u64::from(recorded == table).to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    for recorded in TableId::ALL {
        if !stored(recorded) {
            continue;
        }
        if recorded == table {
            bytes.push(status);
            bytes.extend_from_slice(payload);
        } else {
            bytes.push(4);
            crate::codec::put_string(&mut bytes, "synthetic unavailable").unwrap();
        }
    }
    bytes
}

fn selected_delta_test_row(
    table: TableId,
    format: crate::format::FormatVersion,
    key_difference: u128,
    mask: u64,
    fields: &[u8],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    crate::codec::put_varint(&mut bytes, key_difference);
    let width = table
        .last_change()
        .fields(table, format.version())
        .div_ceil(8);
    bytes.extend_from_slice(&mask.to_le_bytes()[..width]);
    bytes.extend_from_slice(fields);
    bytes
}

fn selected_delta_test_payload(changed: &[Vec<u8>], deleted_differences: &[u128]) -> Vec<u8> {
    let mut bytes = 0_i32.to_le_bytes().to_vec();
    put_count(&mut bytes, changed.len()).unwrap();
    for row in changed {
        bytes.extend_from_slice(row);
    }
    put_count(&mut bytes, deleted_differences.len()).unwrap();
    for &difference in deleted_differences {
        crate::codec::put_varint(&mut bytes, difference);
    }
    bytes
}

fn selected_delta_test_error(
    table: TableId,
    payload: &[u8],
    format: crate::format::FormatVersion,
    keyframe: bool,
    expected: &str,
) {
    let bytes = selected_delta_test_wire(table, 1, payload, format, keyframe);
    let mut observed = Vec::new();
    for selected in [None, Some(&[table][..]), Some(&[][..])] {
        let mut input = Decoder::new(&bytes);
        let error = Frame::decode_version(&mut input, selected, format).unwrap_err();
        assert_eq!(
            error.to_string(),
            expected,
            "{table:?}, selected={selected:?}"
        );
        observed.push((error.kind(), error.to_string(), input.remaining()));
    }
    assert_eq!(observed[0], observed[1]);
    assert_eq!(
        observed[0], observed[2],
        "unselected reads retain error precedence and consumption"
    );
}

#[test]
fn unselected_delta_validation_preserves_field_errors_and_first_error_precedence() {
    let format = crate::format::FormatVersion::CURRENT;
    let country = TableId::Countries;
    let row = |mask, values: &[u8]| selected_delta_test_row(country, format, 1, mask, values);
    let mut oversized = Vec::new();
    crate::codec::put_varint(&mut oversized, 1_u128 << 34);
    let mut overflow = vec![0xff; 18];
    overflow.push(4);
    let mut overwide_key = Vec::new();
    crate::codec::put_varint(&mut overwide_key, u128::from(u32::MAX) + 1);
    overwide_key.extend_from_slice(&row(1, &[0])[1..]);
    let mut noncanonical_key = vec![0x81, 0];
    noncanonical_key.extend_from_slice(&row(1, &[0])[1..]);
    let mut overflowing_key = overflow.clone();
    overflowing_key.extend_from_slice(&row(1, &[0])[1..]);
    for (rows, deleted, error) in [
        (vec![row(0, &[])], vec![], "invalid recorded delta mask"),
        (
            vec![row(1_u64 << 15, &[])],
            vec![],
            "invalid recorded delta mask",
        ),
        (
            vec![noncanonical_key],
            vec![],
            "noncanonical overlong varint",
        ),
        (vec![overflowing_key], vec![], "varint exceeds u128"),
        (vec![overwide_key], vec![], "row key exceeds native width"),
        (
            vec![row(1, &[0x80, 0])],
            vec![],
            "noncanonical overlong varint",
        ),
        (vec![row(1, &overflow)], vec![], "varint exceeds u128"),
        (
            vec![row(1, &oversized)],
            vec![],
            "numeric difference exceeds native field range",
        ),
        (
            vec![row(1 << 2, &[2])],
            vec![],
            "invalid optional integer tag",
        ),
        (
            vec![row(1 << 1, &u32::MAX.to_le_bytes())],
            vec![],
            "truncated field",
        ),
        (
            vec![
                row(1, &[0]),
                selected_delta_test_row(country, format, 0, 1, &[0]),
            ],
            vec![],
            "delta keys must be unique and ascending",
        ),
        (
            vec![],
            vec![1, 0],
            "delta keys must be unique and ascending",
        ),
        (
            vec![row(1, &[0])],
            vec![1],
            "delta both changes and deletes a key",
        ),
        (
            vec![
                selected_delta_test_row(country, format, u128::from(u32::MAX), 1, &[0]),
                row(1, &[0]),
            ],
            vec![],
            "row key exceeds native width",
        ),
    ] {
        let payload = selected_delta_test_payload(&rows, &deleted);
        for keyframe in [false, true] {
            selected_delta_test_error(country, &payload, format, keyframe, error);
        }
    }
    let payload = selected_delta_test_payload(
        &[
            row(1, &[0]),
            selected_delta_test_row(country, format, 0, 1 << 2, &[2]),
        ],
        &[],
    );
    selected_delta_test_error(
        country,
        &payload,
        format,
        true,
        "invalid optional integer tag",
    );
    let payload = selected_delta_test_payload(&[row(1, &[0])], &[]);
    selected_delta_test_error(
        country,
        &payload,
        format,
        true,
        "keyframe contains a dependent or partial table",
    );
    for (table, mask, error) in [
        (
            TableId::LandCombats,
            1 << 3,
            "collection length exceeds remaining payload",
        ),
        (
            TableId::ArmyOrders,
            1 << 4,
            "collection length exceeds remaining payload",
        ),
        (TableId::Provinces, 1 << 7, "truncated field"),
    ] {
        let payload = selected_delta_test_payload(
            &[selected_delta_test_row(
                table,
                format,
                1,
                mask,
                &u32::MAX.to_le_bytes(),
            )],
            &[],
        );
        selected_delta_test_error(table, &payload, format, false, error);
    }
    let singleton = TableId::Clock;
    let zero = selected_delta_test_row(singleton, format, 0, 1, &[0]);
    let payload = selected_delta_test_payload(&[zero.clone(), zero], &[]);
    selected_delta_test_error(
        singleton,
        &payload,
        format,
        false,
        "singleton table cannot contain multiple rows",
    );
    let payload = selected_delta_test_payload(
        &[selected_delta_test_row(singleton, format, 1, 1, &[0])],
        &[],
    );
    selected_delta_test_error(
        singleton,
        &payload,
        format,
        false,
        "row key exceeds native width",
    );
    let wide = TableId::AirTheatreMembers;
    assert_eq!(wide.key_bits(), 128);
    let payload = selected_delta_test_payload(
        &[
            selected_delta_test_row(wide, format, u128::MAX, 1, &[0]),
            selected_delta_test_row(wide, format, 1, 1, &[0]),
        ],
        &[],
    );
    selected_delta_test_error(wide, &payload, format, false, "row key addition overflow");
    let payload = selected_delta_test_payload(&[], &[u128::MAX, 1]);
    selected_delta_test_error(wide, &payload, format, false, "row key addition overflow");
}

#[test]
fn unselected_delta_rows_preserve_every_truncated_prefix_error() {
    let format = crate::format::FormatVersion::CURRENT;
    for (table, mask, fields) in [
        (TableId::Countries, 1, vec![0x80, 1]),
        (TableId::Countries, 1 << 2, vec![1, 0x80, 1]),
        (TableId::Countries, 1 << 1, vec![3, 0, 0, 0, 1, 2, 3]),
        (
            TableId::LandCombats,
            1 << 3,
            vec![1, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8],
        ),
        (TableId::ArmyOrders, 1 << 4, vec![1, 0, 0, 0, 1, 2, 3, 4]),
        (TableId::Fleets, 1 << 2, vec![1, 0, 0, 0, 255]),
    ] {
        let bytes = selected_delta_test_row(table, format, 1, mask, &fields);
        for length in 0..bytes.len() {
            let mut owned = Decoder::new(&bytes[..length]);
            let mut borrowed = Decoder::new(&bytes[..length]);
            let a = rows::decode_delta(table, 0, &mut owned).unwrap_err();
            let b = rows::validate_delta_row(table, 0, &mut borrowed).unwrap_err();
            assert_eq!(
                (a.kind(), a.to_string(), owned.remaining()),
                (b.kind(), b.to_string(), borrowed.remaining()),
                "{table:?}, prefix={length}"
            );
        }
        let mut owned = Decoder::new(&bytes);
        let mut borrowed = Decoder::new(&bytes);
        assert_eq!(
            rows::decode_delta(table, 0, &mut owned).unwrap().key,
            rows::validate_delta_row(table, 0, &mut borrowed).unwrap()
        );
        owned.finish().unwrap();
        borrowed.finish().unwrap();
    }
}

#[test]
fn unselected_deltas_keep_historical_masks_and_wire_only_opaque_validation() {
    use crate::format::FormatVersion;
    let mut widths =
        std::collections::BTreeMap::<TableId, std::collections::BTreeSet<usize>>::new();
    for version in crate::rows::compatibility::PROVEN_FROM..=crate::FORMAT_VERSION {
        let Ok(format) = FormatVersion::read(version) else {
            continue;
        };
        for table in [
            TableId::Clock,
            TableId::Countries,
            TableId::Provinces,
            TableId::AirWings,
        ] {
            if format.table_refusal(table).is_some() {
                continue;
            }
            let fields = table.last_change().fields(table, version);
            widths.entry(table).or_default().insert(fields.div_ceil(8));
            let payload = selected_delta_test_payload(
                &[selected_delta_test_row(table, format, 0, 1, &[0])],
                &[],
            );
            let bytes = selected_delta_test_wire(table, 1, &payload, format, false);
            for selected in [None, Some(&[table][..]), Some(&[][..])] {
                let mut input = Decoder::new(&bytes);
                let frame = Frame::decode_version(&mut input, selected, format).unwrap();
                input.finish().unwrap();
                if selected.is_none_or(|selection| selection.contains(&table)) {
                    let TableUpdate::Delta { changed, .. } = &frame.tables[table.index()] else {
                        panic!()
                    };
                    assert_eq!(changed[0].format, version);
                    assert_eq!(changed[0].recorded_fields, fields);
                } else {
                    assert!(matches!(
                        frame.tables[table.index()],
                        TableUpdate::Unavailable { .. }
                    ));
                }
            }
            if fields < fields.div_ceil(8) * 8 {
                let payload = selected_delta_test_payload(
                    &[selected_delta_test_row(
                        table,
                        format,
                        0,
                        1_u64 << fields,
                        &[0],
                    )],
                    &[],
                );
                selected_delta_test_error(
                    table,
                    &payload,
                    format,
                    false,
                    "invalid recorded delta mask",
                );
            }
        }
    }
    assert!(
        widths.values().any(|versions| versions.len() > 1),
        "the fixture must exercise a historical mask-width change"
    );

    let table = TableId::Fleets;
    let format = FormatVersion::CURRENT;
    let mut invalid_nested_name = Vec::new();
    crate::codec::put_bytes(&mut invalid_nested_name, &[255]).unwrap();
    let payload = selected_delta_test_payload(
        &[selected_delta_test_row(
            table,
            format,
            1,
            1 << 2,
            &invalid_nested_name,
        )],
        &[],
    );
    let bytes = selected_delta_test_wire(table, 1, &payload, format, false);
    let mut retained = None;
    for selected in [None, Some(&[table][..]), Some(&[][..])] {
        let mut input = Decoder::new(&bytes);
        let frame = Frame::decode_version(&mut input, selected, format).unwrap();
        input.finish().unwrap();
        if selected.is_none() {
            retained = Some(frame);
        }
    }
    let mut first = source(1, 0);
    first.tables[table.index()] =
        TableView::Complete(TableRows::Fleets(vec![crate::rows::FleetRow {
            id: 1,
            ..Default::default()
        }]))
        .into();
    let baseline = FrameEncoder::new().encode(first, true).unwrap();
    let mut decoder = FrameDecoder::new();
    decoder.apply(&baseline).unwrap();
    let reconstructed = decoder.apply(&retained.unwrap()).unwrap();
    assert!(
        matches!(
            &*reconstructed.tables[table.index()],
            TableView::Unavailable { .. }
        ),
        "typed nested validation still occurs during reconstruction"
    );

    let mut snapshot = 1_u32.to_le_bytes().to_vec();
    crate::codec::put_varint(&mut snapshot, 1);
    snapshot.extend_from_slice(&0_u32.to_le_bytes());
    snapshot.extend_from_slice(&0_u32.to_le_bytes());
    snapshot.extend_from_slice(&invalid_nested_name);
    snapshot.push(0);
    let bytes = selected_delta_test_wire(table, 0, &snapshot, format, false);
    for selected in [None, Some(&[][..])] {
        assert!(Frame::decode_version(&mut Decoder::new(&bytes), selected, format).is_err());
    }
}

#[test]
fn unselected_country_deltas_never_own_discarded_values() {
    let table = TableId::Countries;
    let mut first = source(1, 0);
    let TableView::Complete(TableRows::Countries(template)) = &*first.tables[table.index()] else {
        panic!()
    };
    let template = template[0].clone();
    first.tables[table.index()] = TableView::Complete(TableRows::Countries(
        (0..5)
            .map(|id| CountryRow {
                id,
                canonical_slot: id + 1,
                ..template.clone()
            })
            .collect(),
    ))
    .into();
    let mut second = first.clone();
    second.hour = 1;
    if let TableView::Complete(TableRows::Clock(rows)) = &mut *second.tables[TableId::Clock.index()]
    {
        rows[0].hour = 1;
    }
    if let TableView::Complete(TableRows::Countries(rows)) = &mut *second.tables[table.index()] {
        for row in rows {
            row.command_power_raw += 1;
        }
    }
    let mut encoder = FrameEncoder::new();
    let baseline = encoder.encode(first, true).unwrap();
    let frame = encoder.encode(second, false).unwrap();
    let TableUpdate::Delta { changed, .. } = &frame.tables[table.index()] else {
        panic!("fixture must encode deltas")
    };
    assert_eq!(changed.len(), 5);
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    let (full, full_owned) =
        rows::delta_decode_ownership::count(|| Frame::decode(&mut Decoder::new(&bytes)).unwrap());
    let selected = [table];
    let (projected, selected_owned) = rows::delta_decode_ownership::count(|| {
        Frame::decode_selected(&mut Decoder::new(&bytes), Some(&selected)).unwrap()
    });
    let (omitted, omitted_owned) = rows::delta_decode_ownership::count(|| {
        Frame::decode_selected(&mut Decoder::new(&bytes), Some(&[])).unwrap()
    });
    assert!(full_owned >= 5);
    assert_eq!(selected_owned, 5);
    assert_eq!(
        omitted_owned, 0,
        "unselected fields must stay borrowed instead of allocating DeltaRow values"
    );
    assert_eq!(projected.tables[table.index()], full.tables[table.index()]);
    assert!(matches!(
        omitted.tables[table.index()],
        TableUpdate::Unavailable { .. }
    ));
    let mut full_decoder = FrameDecoder::new();
    let mut selected_decoder = FrameDecoder::for_tables(&selected);
    full_decoder.apply(&baseline).unwrap();
    selected_decoder.apply(&baseline).unwrap();
    let expected = full_decoder.apply(&full).unwrap();
    let actual = selected_decoder.apply(&projected).unwrap();
    assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    assert_eq!(actual.tables[table.index()].sampled_at(), Some(1));
    assert_eq!(
        actual.tables[table.index()].sampled_at(),
        expected.tables[table.index()].sampled_at()
    );
}

const FACTION_ID: u64 = 0xfedc_ba98_0000_004d;

#[test]
fn unchanged_tables_are_shared_and_a_delta_copies_on_write() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let original = source(1, 0);
    let first = decoder
        .apply(&encoder.encode(original.clone(), true).unwrap())
        .unwrap();
    let mut next = source(1, 1);
    change_country(&mut next, 5);
    let frame = encoder.encode(next.clone(), false).unwrap();
    let countries = TableId::Countries.index();
    let campaign = TableId::Campaign.index();
    assert!(matches!(
        frame.tables[campaign],
        TableUpdate::Unchanged { .. }
    ));
    assert!(matches!(frame.tables[countries], TableUpdate::Delta { .. }));
    let second = decoder.apply(&frame).unwrap();
    assert!(SharedView::ptr_eq(
        &first.tables[campaign],
        &second.tables[campaign]
    ));
    assert!(!SharedView::ptr_eq(
        &first.tables[countries],
        &second.tables[countries]
    ));
    assert_eq!(first.tables[countries], original.tables[countries]);
    assert_eq!(second.tables[countries], next.tables[countries]);
}

fn moving_countries(hours: Hour, step: i64) -> Vec<Frame> {
    let mut encoder = FrameEncoder::new();
    (0..hours)
        .map(|hour| {
            let mut state = source(1, hour);
            change_country(&mut state, i64::from(hour) * step);
            encoder.encode(state, hour == 0).unwrap()
        })
        .collect()
}

fn assert_same_frame(actual: &DecodedFrame, expected: &DecodedFrame) {
    assert_eq!(actual, expected);
    for (actual, expected) in actual.tables.iter().zip(&expected.tables) {
        assert_eq!(actual.sampled_at(), expected.sampled_at());
    }
}

#[test]
fn a_view_kept_until_the_next_hour_is_copied_once_then_reused() {
    let countries = TableId::Countries.index();
    let frames = moving_countries(6, 3);
    let (mut decoder, mut alone) = (FrameDecoder::new(), FrameDecoder::new());
    let (copies, reuses) = (
        crate::cpu::delta_copies()[countries],
        crate::cpu::delta_reuses()[countries],
    );
    let mut held = None;
    let mut earlier: Vec<ViewIdentity> = Vec::new();
    for frame in &frames {
        let actual = decoder.advance(frame).unwrap();
        assert_same_frame(actual, alone.advance(frame).unwrap());
        let view = &actual.tables[countries];
        assert!(earlier.iter().all(|identity| !identity.is(view)));
        earlier.push(view.identity());
        held = Some(actual.clone());
    }
    assert!(held.is_some());
    assert_eq!(crate::cpu::delta_copies()[countries][0] - copies[0], 1);
    assert_eq!(crate::cpu::delta_reuses()[countries][0] - reuses[0], 4);
}

#[test]
fn an_unchanged_hour_releases_the_replaced_view() {
    let countries = TableId::Countries.index();
    let mut encoder = FrameEncoder::new();
    let frames: Vec<Frame> = [0, 5, 5, 9]
        .into_iter()
        .enumerate()
        .map(|(hour, value)| {
            let mut state = source(1, hour as Hour);
            change_country(&mut state, value);
            encoder.encode(state, hour == 0).unwrap()
        })
        .collect();
    assert!(matches!(
        frames[2].tables[countries],
        TableUpdate::Unchanged { .. }
    ));
    let mut decoder = FrameDecoder::new();
    let zero = decoder.advance(&frames[0]).unwrap().clone();
    let first = zero.tables[countries].identity();
    let one = decoder.advance(&frames[1]).unwrap().clone();
    drop(zero);
    assert!(first.is_alive(), "the decoder keeps the view it replaced");
    let two = decoder.advance(&frames[2]).unwrap().clone();
    assert!(!first.is_alive());
    drop(one);
    let copies = crate::cpu::delta_copies()[countries];
    let reuses = crate::cpu::delta_reuses()[countries];
    let three = decoder.advance(&frames[3]).unwrap().clone();
    drop((two, three));
    assert_eq!(crate::cpu::delta_copies()[countries][0] - copies[0], 1);
    assert_eq!(crate::cpu::delta_reuses()[countries], reuses);
}

#[test]
fn a_frame_log_refuses_change_facts() {
    let countries = TableId::Countries.index();
    let frames = moving_countries(2, 4);
    let mut decoder = FrameDecoder::new().with_change_facts();
    let first = decoder.advance(&frames[0]).unwrap().clone();
    let second = decoder.advance(&frames[1]).unwrap().clone();
    assert!(first.tables.iter().all(|view| view.2.is_none()));
    assert!(
        second.tables[countries]
            .changes_since(&first.tables[countries].identity())
            .is_some()
    );
    let mut log = FrameLog::new();
    log.push(&first).unwrap();
    let refused = log.push(&second).unwrap_err();
    assert_eq!(refused.to_string(), "a frame log keeps no change facts");
    assert_eq!(log.len(), 1);
}

#[test]
fn unselected_views_are_told_apart_by_what_they_hold_on_every_thread() {
    let here = not_selected();
    let there = std::thread::spawn(not_selected).join().unwrap();
    assert!(SharedView::ptr_eq(&here, &not_selected()));
    assert!(!SharedView::ptr_eq(&here, &there));
    assert!(here.is_not_selected() && there.is_not_selected());
    let identity = there.identity();
    drop(there);
    assert!(identity.not_selected() && here.identity().not_selected());
    let rows: SharedView = TableView::Unavailable {
        reason: "Table not recorded".into(),
    }
    .into();
    assert!(!rows.is_not_selected() && !rows.identity().not_selected());
}

#[test]
fn a_selected_decoder_moves_the_unselected_view_from_hour_to_hour() {
    let mut encoder = FrameEncoder::new();
    let mut full = FrameDecoder::new();
    let selected = [TableId::Countries, TableId::Campaign];
    let mut projected = FrameDecoder::for_tables(&selected);
    let unselected = not_selected();
    let hours = [
        (1, 0),
        (1, 1),
        (1, 2),
        (1, 3),
        (1, 4),
        (2, 0),
        (2, 1),
        (2, 2),
    ];
    let ((), shares) = count_unselected(|| {
        let mut before: Option<(DecodedFrame, Vec<ViewIdentity>)> = None;
        for (branch, hour) in hours {
            let mut state = source(branch, hour);
            change_country(&mut state, i64::from(hour) * 7);
            let keyframe = hour == 0 || hour == 3;
            let frame = encoder.encode(state, keyframe).unwrap();
            let expected = full.apply(&frame).unwrap();
            let actual = projected.advance(&frame).unwrap();
            for table in TableId::ALL {
                let index = table.index();
                if !selected.contains(&table) {
                    assert!(SharedView::ptr_eq(&actual.tables[index], &unselected));
                    continue;
                }
                assert_eq!(actual.tables[index], expected.tables[index]);
                assert_eq!(
                    actual.tables[index].sampled_at(),
                    expected.tables[index].sampled_at()
                );
                if let Some((expected_before, actual_before)) = &before {
                    let position = selected.iter().position(|t| *t == table).unwrap();
                    assert_eq!(
                        actual_before[position].is(&actual.tables[index]),
                        SharedView::ptr_eq(&expected_before.tables[index], &expected.tables[index]),
                        "{table:?} shares the hour before's view in one decoder only at {branch}/{hour}"
                    );
                }
            }
            let identities = selected
                .iter()
                .map(|table| actual.tables[table.index()].identity())
                .collect();
            before = Some((expected, identities));
        }
    });
    assert_eq!(shares, TABLE_COUNT - selected.len());
}

#[test]
fn timed_equal_acquisition_advances_origin_without_copying_carried_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let index = TableId::Campaign.index();
    let first = decoder
        .apply(&encoder.encode(source(1, 0), true).unwrap())
        .unwrap();
    let refreshed = decoder
        .apply(&encoder.encode(source(1, 1), false).unwrap())
        .unwrap();
    assert!(SharedView::ptr_eq(
        &first.tables[index],
        &refreshed.tables[index]
    ));
    assert_eq!(first.tables[index], refreshed.tables[index]);
    assert_eq!(first.tables[index].sampled_at(), Some(0));
    assert_eq!(refreshed.tables[index].sampled_at(), Some(1));

    let mut carried = encoder.encode(source(1, 2), false).unwrap();
    carried.metrics.table_ns[index] = 0;
    let carried = decoder.apply(&carried).unwrap();
    assert!(SharedView::ptr_eq(
        &refreshed.tables[index],
        &carried.tables[index]
    ));
    assert_eq!(carried.tables[index].sampled_at(), Some(1));
    let mut edited = carried.tables[index].clone();
    let _ = &mut *edited;
    assert_eq!(edited.sampled_at(), None);
    assert_eq!(carried.tables[index].sampled_at(), Some(1));
}

#[test]
fn untimed_keyframes_and_branch_changes_do_not_invent_acquisition_origins() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let index = TableId::Campaign.index();
    decoder
        .apply(&encoder.encode(source(1, 0), true).unwrap())
        .unwrap();
    for (branch, hour) in [(1, 1), (2, 0)] {
        let mut frame = encoder.encode(source(branch, hour), true).unwrap();
        frame.metrics.table_ns[index] = 0;
        let decoded = decoder.apply(&frame).unwrap();
        assert_eq!(decoded.tables[index].sampled_at(), None);
    }
    let mut frame = encoder.encode(source(2, 1), false).unwrap();
    frame.metrics.table_ns[index] = 0;
    assert_eq!(
        decoder.apply(&frame).unwrap().tables[index].sampled_at(),
        None
    );
}

#[test]
fn changed_rows_need_their_own_timing_to_certify_an_acquisition_origin() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let index = TableId::Countries.index();
    decoder
        .apply(&encoder.encode(source(1, 0), true).unwrap())
        .unwrap();
    let mut changed = source(1, 1);
    change_country(&mut changed, 5);
    let mut frame = encoder.encode(changed, false).unwrap();
    assert!(matches!(frame.tables[index], TableUpdate::Delta { .. }));
    frame.metrics.table_ns[index] = 0;
    assert_eq!(
        decoder.apply(&frame).unwrap().tables[index].sampled_at(),
        None
    );
    let decoded = decoder
        .apply(&encoder.encode(source(1, 2), false).unwrap())
        .unwrap();
    assert_eq!(decoded.tables[index].sampled_at(), Some(2));
}

#[test]
fn timed_failed_partial_and_unselected_updates_have_no_acquisition_origin() {
    let index = TableId::Countries.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    decoder
        .apply(&encoder.encode(source(1, 0), true).unwrap())
        .unwrap();
    for hour in 1..=3 {
        let mut frame = encoder.encode(source(1, hour), false).unwrap();
        frame.tables[index] = match hour {
            1 => TableUpdate::Unavailable {
                reason: "failed walk".into(),
            },
            2 => TableUpdate::Unchanged { baseline: 1 },
            _ => {
                let TableView::Complete(rows) = source(1, hour).tables[index].clone().into_inner()
                else {
                    unreachable!()
                };
                TableUpdate::Truncated { rows, missing: 1 }
            }
        };
        assert_eq!(
            decoder.apply(&frame).unwrap().tables[index].sampled_at(),
            None
        );
    }
    let mut selected = FrameDecoder::for_tables(&[TableId::Campaign]);
    let frame = encoder.encode(source(2, 0), true).unwrap();
    assert_eq!(
        selected.apply(&frame).unwrap().tables[index].sampled_at(),
        None
    );
}

fn shared_input(source: DecodedFrame) -> FrameInput {
    let mut input: FrameInput = source.into();
    input.tables = input.tables.map(|input| match input {
        TableInput::Sampled(TableView::Complete(rows)) => {
            TableInput::SharedComplete(Arc::new(rows))
        }
        other => other,
    });
    input
}

fn reuse_committed_rows(input: &mut FrameInput, encoder: &FrameEncoder) {
    for (index, input) in input.tables.iter_mut().enumerate() {
        if let TableInput::SharedComplete(current) = input
            && let Some((_, BaselineRows::Shared(previous))) = &encoder.state.tables[index]
            && current.as_ref() == previous.as_ref()
        {
            *current = Arc::clone(previous);
        }
    }
}

#[test]
fn shared_complete_rows_match_owned_wire_and_adopt_each_validated_refresh() {
    for reuse_allocation in [false, true] {
        let mut shared = FrameEncoder::new();
        let mut owned = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut previous_rows = None;
        for (branch, hour, value, force) in [
            (1, 100, 1, false),
            (1, 101, 1, false),
            (1, 107, 2, false),
            (1, 108, 2, false),
            (1, 820, 2, false),
            (1, 821, 2, true),
            (2, 100, 3, false),
            (1, 100, 4, false),
        ] {
            let mut source = source(branch, hour);
            change_country(&mut source, value);
            let mut input = shared_input(source.clone());
            if reuse_allocation {
                reuse_committed_rows(&mut input, &shared);
            }
            let allocations: [_; TABLE_COUNT] =
                std::array::from_fn(|index| match &input.tables[index] {
                    TableInput::SharedComplete(rows) => Some(Arc::clone(rows)),
                    _ if crate::frame::current_absence(TableId::ALL[index]).is_some() => None,
                    _ => panic!("fixture has complete rows"),
                });
            let expected = owned.encode(source.clone(), force).unwrap();
            let actual = shared.encode_input(input, force).unwrap();
            assert_eq!(wire(&actual), wire(&expected));
            assert_eq!(decoder.apply(&actual).unwrap(), source);
            for (index, allocation) in allocations
                .iter()
                .enumerate()
                .filter_map(|(index, allocation)| Some((index, allocation.as_ref()?)))
            {
                let (baseline_hour, BaselineRows::Shared(baseline)) =
                    shared.state.tables[index].as_ref().unwrap()
                else {
                    panic!("complete shared rows must stay shared")
                };
                assert_eq!(*baseline_hour, hour);
                assert!(Arc::ptr_eq(baseline, allocation));
                assert_eq!(Arc::strong_count(allocation), 2);
            }
            if let Some(previous) = previous_rows.take() {
                if Weak::ptr_eq(
                    &previous,
                    &Arc::downgrade(allocations[TableId::Countries.index()].as_ref().unwrap()),
                ) {
                    assert!(reuse_allocation);
                    assert!(Weak::upgrade(&previous).is_some());
                } else {
                    assert!(Weak::upgrade(&previous).is_none());
                }
            }
            previous_rows = Some(Arc::downgrade(
                allocations[TableId::Countries.index()].as_ref().unwrap(),
            ));
        }
    }
}

#[test]
fn shared_baselines_survive_cadence_and_owned_equal_inputs_without_reallocation() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let table = TableId::AirAces;
    let first = shared_input(source(1, 0));
    let TableInput::SharedComplete(rows) = &first.tables[table.index()] else {
        panic!()
    };
    let allocation = Arc::clone(rows);
    decoder
        .apply(&encoder.encode_input(first, false).unwrap())
        .unwrap();
    for hour in 1..4 {
        let expected = source(1, hour);
        let mut input: FrameInput = expected.clone().into();
        if hour != 2 {
            input.tables[table.index()] = TableInput::Sampled(TableView::Unavailable {
                reason: crate::cadence::SKIPPED_REASON.into(),
            });
        }
        let frame = encoder.encode_input(input, false).unwrap();
        assert_eq!(
            frame.tables[table.index()],
            TableUpdate::Unchanged { baseline: hour - 1 }
        );
        assert_eq!(decoder.apply(&frame).unwrap(), expected);
        let (_, BaselineRows::Shared(baseline)) =
            encoder.state.tables[table.index()].as_ref().unwrap()
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(baseline, &allocation));
    }
}

#[test]
fn shared_partial_and_unavailable_clear_baselines_until_a_complete_snapshot() {
    let table = TableId::Characters;
    for partial in [false, true] {
        let mut encoder = FrameEncoder::new();
        let initial = shared_input(source(1, 0));
        let TableInput::SharedComplete(rows) = &initial.tables[table.index()] else {
            panic!()
        };
        let allocation = Arc::clone(rows);
        encoder.encode_input(initial, false).unwrap();
        let mut failed = shared_input(source(1, 1));
        failed.tables[table.index()] = TableInput::Sampled(if partial {
            TableView::Truncated {
                rows: TableRows::empty(table),
                missing: 1,
            }
        } else {
            TableView::Unavailable {
                reason: "fresh read failed".into(),
            }
        });
        encoder.encode_input(failed, false).unwrap();
        assert!(encoder.state.tables[table.index()].is_none());
        assert_eq!(Arc::strong_count(&allocation), 1);
        let mut restored = shared_input(source(1, 2));
        restored.tables[table.index()] = TableInput::SharedComplete(Arc::clone(&allocation));
        let restored = encoder.encode_input(restored, false).unwrap();
        assert!(matches!(
            restored.tables[table.index()],
            TableUpdate::Snapshot(_)
        ));
    }
}

#[test]
fn committed_rows_follow_the_last_complete_sample_and_clear_with_its_loss() {
    let table = TableId::Countries;
    let mut encoder = FrameEncoder::new();
    assert!(encoder.committed::<tables::Countries>().is_none());
    let first = source(1, 0);
    let rows = first.complete::<tables::Countries>().unwrap().to_vec();
    encoder.encode(first, true).unwrap();
    assert_eq!(
        encoder.committed::<tables::Countries>(),
        Some((0, rows.as_slice()))
    );
    let mut failed = shared_input(source(1, 1));
    failed.tables[table.index()] = TableInput::Sampled(TableView::Unavailable {
        reason: "synthetic read failure".into(),
    });
    encoder.encode_input(failed, false).unwrap();
    assert!(encoder.committed::<tables::Countries>().is_none());
}

#[test]
fn shared_validation_is_atomic_and_preserves_existing_comparison_tokens() {
    let mut encoder = FrameEncoder::new();
    encoder
        .encode_input(shared_input(variable_source(1, 0, 1)), false)
        .unwrap();
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    let (_, BaselineRows::Shared(rows)) = encoder.state.tables[TableId::ScriptVariables.index()]
        .as_ref()
        .unwrap()
    else {
        panic!()
    };
    let allocation = Arc::clone(rows);
    let duplicate = TableRows::Countries(vec![
        variable_source(1, 1, 1)
            .complete::<tables::Countries>()
            .unwrap()[0]
            .clone();
        2
    ]);
    for (table, rows) in [
        (TableId::Countries, duplicate),
        (TableId::CountryIntel, TableRows::Clock(Vec::new())),
        (
            TableId::Clock,
            TableRows::Clock(vec![ClockRow {
                hour: 99,
                ..Default::default()
            }]),
        ),
    ] {
        let mut invalid = shared_input(variable_source(1, 1, 2));
        invalid.tables[table.index()] = TableInput::SharedComplete(Arc::new(rows));
        reuse_committed_rows(&mut invalid, &encoder);
        assert!(encoder.encode_input(invalid, false).is_err());
        assert_eq!(
            encoder
                .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
                .unwrap()
                .0,
            token
        );
        let (_, BaselineRows::Shared(rows)) = encoder.state.tables
            [TableId::ScriptVariables.index()]
        .as_ref()
        .unwrap() else {
            panic!()
        };
        assert!(Arc::ptr_eq(rows, &allocation));
        assert_eq!(encoder.state.last_hour, Some(0));
        assert!(
            encoder
                .state
                .tables
                .iter()
                .flatten()
                .all(|(hour, _)| *hour == 0)
        );
    }
    let mut compared = shared_input(variable_source(1, 1, 1));
    compared.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
    let frame = encoder.encode_input(compared, false).unwrap();
    assert_eq!(
        frame.tables[TableId::ScriptVariables.index()],
        TableUpdate::Unchanged { baseline: 0 }
    );
    let (_, BaselineRows::Shared(rows)) = encoder.state.tables[TableId::ScriptVariables.index()]
        .as_ref()
        .unwrap()
    else {
        panic!()
    };
    assert!(Arc::ptr_eq(rows, &allocation));
}

#[test]
fn shared_committed_identity_checks_clock_mutation_table_and_encoder_reset() {
    let mut encoder = FrameEncoder::new();
    let first = shared_input(source(1, 0));
    let TableInput::SharedComplete(clock) = &first.tables[TableId::Clock.index()] else {
        panic!()
    };
    let clock = Arc::clone(clock);
    let TableInput::SharedComplete(countries) = &first.tables[TableId::Countries.index()] else {
        panic!()
    };
    let countries = Arc::clone(countries);
    encoder.encode_input(first, false).unwrap();
    let revision = encoder.revision;
    let mut mutated = Arc::new(countries.try_clone().unwrap());
    let rows = Arc::get_mut(&mut mutated)
        .unwrap()
        .rows_mut::<tables::Countries>()
        .unwrap();
    rows.push(rows[0].clone());
    assert!(!Arc::ptr_eq(&mutated, &countries));
    for (table, rows, reason) in [
        (
            TableId::Clock,
            clock,
            "clock row hour differs from frame hour",
        ),
        (
            TableId::Countries,
            mutated,
            "table rows must have matching types and unique ascending keys",
        ),
        (
            TableId::CountryIntel,
            Arc::clone(&countries),
            "table rows must have matching types and unique ascending keys",
        ),
    ] {
        let mut invalid = shared_input(source(1, 1));
        reuse_committed_rows(&mut invalid, &encoder);
        invalid.tables[table.index()] = TableInput::SharedComplete(rows);
        assert_eq!(
            encoder
                .encode_input(invalid, false)
                .unwrap_err()
                .to_string(),
            reason
        );
        assert_eq!(encoder.revision, revision);
        assert_eq!(encoder.state.last_hour, Some(0));
        let (_, BaselineRows::Shared(previous)) = encoder.state.tables[TableId::Countries.index()]
            .as_ref()
            .unwrap()
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(previous, &countries));
    }
    encoder = FrameEncoder::new();
    let mut recovered = shared_input(source(1, 1));
    recovered.tables[TableId::Countries.index()] =
        TableInput::SharedComplete(Arc::clone(&countries));
    let recovered = encoder.encode_input(recovered, false).unwrap();
    assert!(recovered.keyframe);
    assert_eq!(
        recovered.tables[TableId::Countries.index()],
        TableUpdate::Snapshot((*countries).clone())
    );
}

fn variable_source(branch: BranchId, hour: Hour, value: i64) -> DecodedFrame {
    let mut frame = source(branch, hour);
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 7,
            seed: crate::rows::Recorded::Value(None),
            entries: vec![rows::ScriptVariableEntry {
                name: vec![0, 0xff],
                value_raw: value,
            }],
        }]));
    frame
}

#[test]
fn compared_variables_encode_identically_without_replacing_owned_baseline_rows() {
    let mut ordinary = FrameEncoder::new();
    let mut compared = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut previous_name = None;
    for hour in 0..4 {
        let value = if hour < 2 { i64::MIN } else { i64::MAX };
        let source = variable_source(1, hour, value);
        let mut input: FrameInput = source.clone().into();
        if hour == 1 || hour == 3 {
            let (token, previous) = compared
                .comparison_baseline::<tables::ScriptVariables>(1, hour, false)
                .unwrap();
            assert_eq!(
                previous,
                source.complete::<tables::ScriptVariables>().unwrap()
            );
            previous_name = Some(previous[0].entries[0].name.as_ptr());
            input.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
        }
        let expected = ordinary.encode(source.clone(), false).unwrap();
        let actual = compared.encode_input(input, false).unwrap();
        assert_eq!(wire(&actual), wire(&expected));
        assert_eq!(decoder.apply(&actual).unwrap(), source);
        if let Some(pointer) = previous_name.take() {
            let (_, baseline) = compared
                .comparison_baseline::<tables::ScriptVariables>(1, hour + 1, false)
                .unwrap();
            assert_eq!(baseline[0].entries[0].name.as_ptr(), pointer);
        }
    }
}

#[test]
fn compared_variables_require_this_encoder_current_branch_hour_and_non_keyframe() {
    let mut encoder = FrameEncoder::new();
    assert!(
        encoder
            .comparison_baseline::<tables::ScriptVariables>(1, 0, false)
            .is_none()
    );
    encoder.encode(variable_source(1, 0, 1), false).unwrap();
    assert!(
        encoder
            .comparison_baseline::<tables::Clock>(1, 1, false)
            .is_none()
    );
    for (branch, hour, force) in [
        (2, 1, false),
        (1, 0, false),
        (1, 2, false),
        (1, 1, true),
        (1, 720, false),
    ] {
        assert!(
            encoder
                .comparison_baseline::<tables::ScriptVariables>(branch, hour, force)
                .is_none()
        );
    }
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    let input = |branch, hour| {
        let mut input: FrameInput = variable_source(branch, hour, 1).into();
        input.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
        input
    };
    assert!(encoder.encode_input(input(1, 1), true).is_err());
    assert!(encoder.encode_input(input(2, 1), false).is_err());
    assert!(encoder.encode_input(input(1, 2), false).is_err());
    let mut wrong_table = input(1, 1);
    wrong_table.tables[TableId::Clock.index()] = TableInput::ComparedEqual(token);
    assert!(encoder.encode_input(wrong_table, false).is_err());
    let mut other = FrameEncoder::new();
    other.encode(variable_source(1, 0, 99), false).unwrap();
    assert!(
        other.encode_input(input(1, 1), false).is_err(),
        "another encoder has the same hours but different rows"
    );
    assert!(
        FrameEncoder::new()
            .encode_input(input(1, 1), false)
            .is_err()
    );
    assert!(encoder.encode_input(input(1, 1), false).is_ok());
}

#[test]
fn compared_variables_noncomplete_baselines_clear_and_resume_with_snapshot() {
    for partial in [false, true] {
        let mut encoder = FrameEncoder::new();
        encoder.encode(variable_source(1, 0, 1), false).unwrap();
        let mut source = variable_source(1, 1, 1);
        *source.tables[TableId::ScriptVariables.index()] = if partial {
            TableView::Truncated {
                rows: TableRows::ScriptVariables(Vec::new()),
                missing: 1,
            }
        } else {
            TableView::Unavailable {
                reason: "current read failed".into(),
            }
        };
        encoder.encode(source, false).unwrap();
        assert!(
            encoder
                .comparison_baseline::<tables::ScriptVariables>(1, 2, false)
                .is_none()
        );
        let restored = encoder.encode(variable_source(1, 2, 1), false).unwrap();
        assert!(matches!(
            restored.tables[TableId::ScriptVariables.index()],
            TableUpdate::Snapshot(_)
        ));
    }
}

#[test]
fn compared_variables_token_does_not_survive_branch_round_trip_with_identical_hours() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(variable_source(1, 0, 1), false).unwrap();
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    encoder.encode(variable_source(2, 0, 2), false).unwrap();
    encoder.encode(variable_source(1, 0, 3), false).unwrap();
    let mut stale: FrameInput = variable_source(1, 1, 1).into();
    stale.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
    assert!(encoder.encode_input(stale, false).is_err());
    let fresh = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    assert_ne!(fresh, token);
    let mut current: FrameInput = variable_source(1, 1, 3).into();
    current.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(fresh);
    assert!(encoder.encode_input(current, false).is_ok());
}

#[test]
fn compared_variables_do_not_commit_before_later_table_validation_succeeds() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(variable_source(1, 0, 1), false).unwrap();
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    let mut input: FrameInput = variable_source(1, 1, 1).into();
    input.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
    let mut invalid = input.clone();
    invalid.tables[TableId::CountryIntel.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::Clock(Vec::new())));
    assert!(encoder.encode_input(invalid, false).is_err());
    assert_eq!(
        encoder
            .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
            .unwrap()
            .0,
        token
    );
    assert_eq!(
        encoder.encode_input(input, false).unwrap().tables[TableId::ScriptVariables.index()],
        TableUpdate::Unchanged { baseline: 0 }
    );
}

#[test]
fn row_replacements_match_full_frames_and_keep_untouched_allocations() {
    let mut rows: Vec<_> = (0..4)
        .map(|index| rows::SupplyNodeRow {
            country: 7,
            index,
            path: vec![index, 0, index, u32::MAX],
            ..Default::default()
        })
        .collect();
    let current = |hour, rows: &[rows::SupplyNodeRow]| {
        let mut frame = source(1, hour);
        *frame.tables[TableId::SupplyNodes.index()] =
            TableView::Complete(TableRows::SupplyNodes(rows.to_vec()));
        frame
    };
    let mut sparse = FrameEncoder::new();
    let mut ordinary = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let first = sparse.encode(current(0, &rows), false).unwrap();
    ordinary.encode(current(0, &rows), false).unwrap();
    decoder.apply(&first).unwrap();
    let (_, original) = sparse
        .comparison_baseline::<tables::SupplyNodes>(1, 1, false)
        .unwrap();
    let allocation = original.as_ptr();
    let untouched_path = original[3].path.as_ptr();
    for hour in 1..=4 {
        let indices = match hour {
            1 => {
                rows[1].available = i64::MIN;
                rows[1].path.push(99);
                vec![1]
            }
            2 => vec![1],
            3 => {
                rows[0].consumed = i64::MAX;
                rows[2].path.reverse();
                vec![0, 2]
            }
            _ => vec![],
        };
        let (token, _) = sparse
            .comparison_baseline::<tables::SupplyNodes>(1, hour, false)
            .unwrap();
        assert!(token.allows_row_updates());
        let complete = current(hour, &rows);
        let mut input: FrameInput = complete.clone().into();
        input.tables[TableId::SupplyNodes.index()] = TableInput::ComparedChanges {
            token,
            rows: TableRows::SupplyNodes(
                indices.iter().map(|&index| rows[index].clone()).collect(),
            ),
            indices,
        };
        let actual = sparse.encode_bytes(input, false).unwrap();
        let expected = ordinary
            .encode_bytes(complete.clone().into(), false)
            .unwrap();
        assert_eq!(actual.bytes, expected.bytes);
        assert_eq!(decoder.apply(&actual.decode().unwrap()).unwrap(), complete);
        let (_, retained) = sparse
            .comparison_baseline::<tables::SupplyNodes>(1, hour + 1, false)
            .unwrap();
        assert_eq!(retained.as_ptr(), allocation);
        assert_eq!(retained[3].path.as_ptr(), untouched_path);
    }
}

#[test]
fn row_replacements_reject_bad_positions_tokens_and_later_errors_atomically() {
    fn encode_edited(
        encoder: &mut FrameEncoder,
        valid: &FrameInput,
        force: bool,
        edit: impl FnOnce(&mut FrameInput),
    ) -> std::io::Result<Frame> {
        let mut input = valid.clone();
        edit(&mut input);
        encoder.encode_input(input, force)
    }

    enum Invalid {
        Replacement(Vec<usize>, TableRows),
        LaterTable(TableId),
        Branch,
        Hour,
    }

    let mut encoder = FrameEncoder::new();
    encoder.encode(source(1, 0), false).unwrap();
    let (token, old) = encoder
        .comparison_baseline::<tables::Provinces>(1, 1, false)
        .unwrap();
    let original = old.to_vec();
    let mut changed = original[0].clone();
    changed.controller = u32::MAX;
    let mut valid: FrameInput = source(1, 1).into();
    valid.tables[TableId::Provinces.index()] = TableInput::ComparedChanges {
        token,
        indices: vec![0],
        rows: TableRows::Provinces(vec![changed.clone()]),
    };
    let mode = valid.capture_mode().unwrap();
    let later = TableId::ALL
        .into_iter()
        .rfind(|table| {
            table
                .last_change()
                .present(crate::format::FormatVersion::CURRENT.version())
                && crate::scope::records(mode, *table)
        })
        .unwrap();
    assert!(later.index() > TableId::Provinces.index());
    let cases = [
        Invalid::Replacement(vec![], TableRows::Provinces(vec![changed.clone()])),
        Invalid::Replacement(vec![0, 0], TableRows::Provinces(vec![changed.clone(); 2])),
        Invalid::Replacement(
            vec![1, 0],
            TableRows::Provinces(vec![original[1].clone(), changed.clone()]),
        ),
        Invalid::Replacement(vec![2], TableRows::Provinces(vec![changed.clone()])),
        Invalid::Replacement(vec![1], TableRows::Provinces(vec![changed.clone()])),
        Invalid::Replacement(vec![0], TableRows::States(vec![])),
        Invalid::LaterTable(later),
        Invalid::Branch,
        Invalid::Hour,
    ];
    for case in cases {
        assert!(
            encode_edited(&mut encoder, &valid, false, |input| match case {
                Invalid::Replacement(indices, rows) => {
                    input.tables[TableId::Provinces.index()] = TableInput::ComparedChanges {
                        token,
                        indices,
                        rows,
                    };
                }
                Invalid::LaterTable(table) => {
                    input.tables[table.index()] =
                        TableView::Complete(TableRows::Clock(Vec::new())).into();
                }
                Invalid::Branch => input.branch += 1,
                Invalid::Hour => input.hour += 1,
            })
            .is_err()
        );
        let (current, retained) = encoder
            .comparison_baseline::<tables::Provinces>(1, 1, false)
            .unwrap();
        assert_eq!(current, token);
        assert_eq!(retained, original);
    }
    assert!(encode_edited(&mut encoder, &valid, true, |_| {}).is_err());
    let mut other = FrameEncoder::new();
    other.encode(source(1, 0), false).unwrap();
    assert!(encode_edited(&mut other, &valid, false, |_| {}).is_err());
    let mut expected = source(1, 1);
    *expected.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![changed, original[1].clone()]));
    let actual = encode_edited(&mut encoder, &valid, false, |_| {}).unwrap();
    let expected = other.encode(expected, false).unwrap();
    assert_eq!(actual, expected);
    assert!(encode_edited(&mut encoder, &valid, false, |input| input.hour = 2).is_err());
}

#[test]
fn row_replacements_cannot_edit_shared_or_cross_row_validated_baselines() {
    for shared in [false, true] {
        let mut encoder = FrameEncoder::new();
        let first = source(1, 0);
        encoder
            .encode_input(
                if shared {
                    shared_input(first)
                } else {
                    first.into()
                },
                false,
            )
            .unwrap();
        let (token, rows) = encoder
            .comparison_baseline::<tables::Countries>(1, 1, false)
            .unwrap();
        assert!(!token.allows_row_updates());
        let mut input: FrameInput = source(1, 1).into();
        input.tables[TableId::Countries.index()] = TableInput::ComparedChanges {
            token,
            indices: vec![0],
            rows: TableRows::Countries(rows.to_vec()),
        };
        assert!(encoder.encode_input(input, false).is_err());
        let (token, rows) = encoder
            .comparison_baseline::<tables::Provinces>(1, 1, false)
            .unwrap();
        assert_eq!(token.allows_row_updates(), !shared);
        if shared {
            let mut input: FrameInput = source(1, 1).into();
            input.tables[TableId::Provinces.index()] = TableInput::ComparedChanges {
                token,
                indices: vec![0],
                rows: TableRows::Provinces(vec![rows[0].clone()]),
            };
            assert!(encoder.encode_input(input, false).is_err());
        }
        let (token, _) = encoder
            .comparison_baseline::<tables::AirAces>(1, 1, false)
            .unwrap();
        assert!(!token.allows_row_updates());
    }
}

#[test]
fn cadence_marker_carries_identical_validated_rows_without_input_clones() {
    let mut ordinary = FrameEncoder::new();
    let mut carried = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..8 {
        let full = source(1, hour);
        let mut compact = full.clone();
        if hour != 0 {
            for table in TableId::ALL {
                if crate::cadence::is_staggered(table) {
                    assert!(carried.can_carry(table, 1, hour, false));
                    *compact.tables[table.index()] = TableView::Unavailable {
                        reason: crate::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
        }
        let expected = ordinary.encode(full.clone(), false).unwrap();
        let actual = carried.encode(compact, false).unwrap();
        assert_eq!(actual, expected, "hour {hour}");
        assert_eq!(decoder.apply(&actual).unwrap(), full);
    }
}

#[test]
fn cadence_carry_requires_complete_same_branch_non_keyframe_baseline() {
    let mut encoder = FrameEncoder::new();
    let table = TableId::AirAces;
    assert!(!encoder.can_carry(table, 1, 1, false));
    encoder.encode(source(1, 0), false).unwrap();
    assert!(encoder.can_carry(table, 1, 1, false));
    for (branch, hour, force) in [(2, 1, false), (1, 0, false), (1, 1, true), (1, 720, false)] {
        assert!(!encoder.can_carry(table, branch, hour, force));
    }
    assert!(!encoder.can_carry(TableId::Clock, 1, 1, false));
    let mut missing = source(1, 1);
    *missing.tables[table.index()] = TableView::Unavailable {
        reason: "fault".into(),
    };
    encoder.encode(missing, false).unwrap();
    assert!(!encoder.can_carry(table, 1, 2, false));
    let mut marker = source(1, 2);
    *marker.tables[table.index()] = TableView::Unavailable {
        reason: crate::cadence::SKIPPED_REASON.into(),
    };
    assert!(matches!(
        encoder.encode(marker, false).unwrap().tables[table.index()],
        TableUpdate::Unavailable { .. }
    ));
    let mut partial = source(1, 3);
    let TableView::Complete(rows) = (*partial.tables[table.index()]).clone() else {
        unreachable!()
    };
    *partial.tables[table.index()] = TableView::Truncated { rows, missing: 1 };
    encoder.encode(partial, false).unwrap();
    assert!(!encoder.can_carry(table, 1, 4, false));
}

#[test]
fn cadence_marker_cannot_turn_a_missing_hourly_capture_into_fresh_rows() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(source(1, 0), false).unwrap();
    let mut skipped = source(1, 1);
    for table in [TableId::Characters, TableId::Divisions, TableId::Countries] {
        assert!(!crate::cadence::is_staggered(table));
        assert!(!encoder.can_carry(table, 1, 1, false));
        *skipped.tables[table.index()] = TableView::Unavailable {
            reason: crate::cadence::SKIPPED_REASON.into(),
        };
    }
    let frame = encoder.encode(skipped, false).unwrap();
    for table in [TableId::Characters, TableId::Divisions, TableId::Countries] {
        assert!(matches!(
            &frame.tables[table.index()],
            TableUpdate::Unavailable { reason } if reason == crate::cadence::SKIPPED_REASON
        ));
        assert!(encoder.state.tables[table.index()].is_none());
    }
}

fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
    crate::frame::retire_absent(DecodedFrame {
        branch,
        hour,
        metrics: Metrics {
            total_ns: 4_567_890,
            table_ns: std::array::from_fn(|index| (index as u64 + 1) * 100),
            pool_bytes: 123_456,
            dropped_frames: 2,
        },
        tables: {
            let initial = [
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    play_time: u64::MAX,
                    play_session_started_at: 123_456,
                    seed: -789,
                    speed: 4,
                    debug_current_ref_id: Some(u32::MAX),
                    unit: Some(4012),
                    order_index: Some(7693),
                    front_index: Some(8230),
                    theatre_index: Some(112),
                    theater_group_index: Some(924),
                    military_deployment_line_index: Some(7841),
                    military_deployment_conveyor_index: Some(744),
                    equipment_variant_index: Some(6295),
                    country_leader_index: Some(1),
                    navy_id: Some(8263),
                    land_combat_id: Some(15_359),
                    multiplayer_random_count: Some(-1_333_963_091),
                }])),
                TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                    id: b"campaign".to_vec(),
                }])),
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    border_land_doctrine_level: crate::rows::Recorded::Value(None),
                    id: 7,
                    canonical_slot: 1,
                    tag: b"GER".to_vec(),
                    political_power_raw: Some(-9_007_199_254_740_993),
                    command_power_raw: 1,
                    stability_raw: 100_001,
                    war_support_raw: 99_999,
                    convoys_raw: None,
                    peace_conference_participant: 0,
                    manpower_residual: 0,
                }])),
                TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                    id: 2,
                    canonical_slot: 1,
                }])),
                TableView::Complete(TableRows::Provinces(vec![province(1, 7), province(2, 2)])),
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 8,
                    owner: 7,
                    controller: 2,
                    movement_infrastructure: 0,
                    ..StateRow::default()
                }])),
                TableView::Complete(TableRows::Divisions(vec![division(
                    0x1234_5678_0000_0035,
                    300_000,
                    100_000,
                )])),
                TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                    first_country: 2,
                    second_country: 7,
                    start_hour: -12,
                    end_hour: i32::MIN,
                    cancel: 0x80,
                    first_casualties: 9_007_199_254_740_993,
                    second_casualties: i64::MAX,
                    first_unknown_casualties: i64::MIN,
                    second_unknown_casualties: -1,
                    first_was_instigator: 0xfe,
                }])),
                TableView::Complete(TableRows::Factions(vec![faction(FACTION_ID, b"Faction")])),
                TableView::Complete(TableRows::FactionMembers(vec![
                    faction_member(2, FACTION_ID, 1),
                    faction_member(7, FACTION_ID, 0),
                ])),
                TableView::Complete(TableRows::Players(Vec::new())),
                TableView::Complete(TableRows::HumanControllers(Vec::new())),
                TableView::Complete(TableRows::LandCombats(Vec::new())),
                TableView::Complete(TableRows::Fleets(Vec::new())),
                TableView::Complete(TableRows::TaskForces(Vec::new())),
                TableView::Complete(TableRows::Ships(Vec::new())),
                TableView::Complete(TableRows::NavalCombats(Vec::new())),
                TableView::Complete(TableRows::NavalCombatParticipants(Vec::new())),
                TableView::Complete(TableRows::AirWings(Vec::new())),
                TableView::Complete(TableRows::ArmyOrders(Vec::new())),
            ];
            let mut tables = std::array::from_fn(|index| {
                let table = TableId::ALL[index];
                TableView::Complete(rows::sample_extended_table(table, 0).unwrap_or_else(|| {
                    if table.key_bits() == 0 {
                        TableRows::one_default(table)
                    } else {
                        TableRows::empty(table)
                    }
                }))
            });
            for (target, source) in tables.iter_mut().zip(initial) {
                *target = source;
            }
            tables.map(Into::into)
        },
    })
}

fn province(id: u32, controller: u32) -> ProvinceRow {
    ProvinceRow {
        id,
        state: 8,
        owner: 7,
        controller,
        victory_points: 50,
        fort_positive: 0,
        move_gate: 0,
        port_sea: 0,
        attack: Default::default(),
        selected_areas: Default::default(),
        fort_bunker: Default::default(),
        controller_area: Default::default(),
        owner_area: Default::default(),
        area_node: Default::default(),
    }
}

fn division(id: u64, hit_points: i64, organisation: i64) -> DivisionRow {
    DivisionRow {
        id,
        roster_owner: 7,
        logical_owner: 2,
        expeditionary_owner: 0,
        province: 1,
        template: 0xfedc_ba98_0000_002b,
        hit_points,
        organisation,
        orders_group: 0x0000_0039_0000_0035,
        reveal_skip: 0,
        theatre: Some(0).into(),
    }
}

fn faction(id: u64, name: &[u8]) -> FactionRow {
    FactionRow {
        id,
        name: name.to_vec(),
        ..FactionRow::default()
    }
}

fn faction_member(country: u32, faction: u64, ordinal: u32) -> FactionMemberRow {
    FactionMemberRow {
        country,
        faction,
        ordinal,
    }
}

fn change_country(frame: &mut DecodedFrame, value: i64) {
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    let country = &mut rows[0];
    country.command_power_raw = value;
}

fn set_hour(frame: &mut DecodedFrame, hour: Hour) {
    frame.hour = hour;
    let TableView::Complete(TableRows::Clock(rows)) = &mut *frame.tables[TableId::Clock.index()]
    else {
        panic!()
    };
    rows[0].hour = hour;
}

fn wire(frame: &Frame) -> Frame {
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    let mut decoder = Decoder::new(&bytes);
    let result = Frame::decode(&mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(&result, frame);
    result
}

fn unavailable(view: &TableView, expected: &str) {
    let TableView::Unavailable { reason } = view else {
        panic!("expected unavailable: {view:?}")
    };
    assert!(
        reason.contains(expected),
        "{reason:?} did not contain {expected:?}"
    );
}

#[test]
fn padded_snapshot_counts_are_rejected_before_allocating_or_reading_rows() {
    let claimed = 16_384_u32;
    assert_eq!(TableId::Divisions.minimum_row_bytes(), 51);
    assert_eq!(TableId::Campaign.minimum_row_bytes(), 5);
    for table in TableId::ALL {
        let padding = claimed as usize * table.minimum_row_bytes() - 1;
        let mut bytes = claimed.to_le_bytes().to_vec();
        bytes.resize(bytes.len() + padding, 0);
        assert_eq!(Decoder::new(&bytes).count(1).unwrap(), claimed as usize);
        let mut input = Decoder::new(&bytes);
        let error = decode_rows(table, &mut input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "collection length exceeds remaining payload"
        );
        assert_eq!(
            input.remaining(),
            padding,
            "read a row before rejecting {table:?}"
        );
    }
}

#[test]
fn changed_row_counts_include_a_selected_value_and_singletons_never_reserve_many_rows() {
    let claimed = 16_384_u32;
    let padding = claimed as usize * 2;
    let mut bytes = claimed.to_le_bytes().to_vec();
    bytes.resize(bytes.len() + padding, 0);
    assert_eq!(Decoder::new(&bytes).count(2).unwrap(), claimed as usize);
    assert_eq!(TableId::Divisions.minimum_delta_bytes(), 4);
    assert_eq!(TableId::Campaign.minimum_delta_bytes(), 6);
    let mut input = Decoder::new(&bytes);
    let error = decode_row_count(
        TableId::Divisions,
        &mut input,
        TableId::Divisions.minimum_delta_bytes(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "collection length exceeds remaining payload"
    );
    assert_eq!(input.remaining(), padding);

    for table in [TableId::Clock, TableId::Campaign] {
        let payload = 2 * table.minimum_row_bytes();
        let mut bytes = 2_u32.to_le_bytes().to_vec();
        bytes.resize(4 + payload, 0);
        let mut input = Decoder::new(&bytes);
        let error = decode_rows(table, &mut input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "singleton table cannot contain multiple rows"
        );
        assert_eq!(input.remaining(), payload);
    }
}

#[test]
fn unchanged_advances_each_baseline_and_missing_hours_do_not_clear_it() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in [100, 101, 107, 108] {
        let mut expected = source(1, hour);
        if hour >= 107 {
            change_country(&mut expected, i64::MAX);
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        match hour {
            100 => assert!(
                TableId::ALL
                    .into_iter()
                    .filter(|&table| crate::frame::current_absence(table).is_none())
                    .all(|table| matches!(frame.tables[table.index()], TableUpdate::Snapshot(_)))
            ),
            101 => assert_eq!(
                frame.tables[TableId::Countries.index()],
                TableUpdate::Unchanged { baseline: 100 }
            ),
            107 => {
                assert!(matches!(
                    frame.tables[TableId::Countries.index()],
                    TableUpdate::Delta { baseline: 101, .. }
                ));
                assert_eq!(
                    frame.tables[TableId::Provinces.index()],
                    TableUpdate::Unchanged { baseline: 101 }
                );
            }
            108 => assert_eq!(
                frame.tables[TableId::Countries.index()],
                TableUpdate::Unchanged { baseline: 107 }
            ),
            _ => unreachable!(),
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn borrowed_advances_and_owned_results_keep_independent_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let initial = source(1, 100);
    let frame = wire(&encoder.encode(initial.clone(), false).unwrap());
    assert_eq!(decoder.advance(&frame).unwrap(), &initial);

    let mut expected = source(1, 101);
    change_country(&mut expected, i64::MAX);
    let frame = wire(&encoder.encode(expected.clone(), false).unwrap());
    let mut owned = decoder.apply(&frame).unwrap();
    assert_eq!(owned, expected);
    for table in [TableId::Campaign, TableId::Countries, TableId::Factions] {
        let TableView::Complete(rows) = &mut *owned.tables[table.index()] else {
            panic!()
        };
        match rows {
            TableRows::Campaign(rows) => rows[0].id.clear(),
            TableRows::Countries(rows) => {
                let row = &mut rows[0];
                row.tag.clear();
                row.political_power_raw = None;
            }
            TableRows::Factions(rows) => rows[0].name.clear(),
            _ => unreachable!(),
        }
    }
    for hour in [107, 108] {
        let mut expected = source(1, hour);
        change_country(&mut expected, i64::MAX);
        let frame = wire(&encoder.encode(expected.clone(), false).unwrap());
        assert_eq!(
            frame.tables[TableId::Countries.index()],
            TableUpdate::Unchanged {
                baseline: if hour == 107 { 101 } else { 107 }
            }
        );
        assert_eq!(decoder.advance(&frame).unwrap(), &expected);
    }
}

#[test]
fn unavailable_and_truncated_tables_require_their_own_next_snapshot() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let first = source(1, 0);
    assert_eq!(
        decoder
            .apply(&encoder.encode(first.clone(), false).unwrap())
            .unwrap(),
        first
    );
    let mut failed = source(1, 1);
    *failed.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "political status unreadable".into(),
    };
    *failed.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![province(1, 2)]),
        missing: 42,
    };
    let frame = encoder.encode(failed.clone(), false).unwrap();
    assert!(!frame.keyframe);
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), failed);

    let recovered = source(1, 2);
    let frame = encoder.encode(recovered.clone(), false).unwrap();
    for table in [TableId::Countries, TableId::Provinces] {
        assert!(matches!(
            frame.tables[table.index()],
            TableUpdate::Snapshot(_)
        ));
    }
    assert!(matches!(
        frame.tables[TableId::Clock.index()],
        TableUpdate::Delta { baseline: 1, .. }
    ));
    assert_eq!(
        frame.tables[TableId::Campaign.index()],
        TableUpdate::Unchanged { baseline: 1 }
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), recovered);
    let frame = encoder.encode(source(1, 3), false).unwrap();
    assert_eq!(
        frame.tables[TableId::Provinces.index()],
        TableUpdate::Unchanged { baseline: 2 }
    );
}

#[test]
fn state_control_deltas_and_failures_keep_separate_baselines_from_provinces() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let state_index = TableId::States.index();
    for hour in 0..=6 {
        let mut expected = source(1, hour);
        if hour > 0 {
            *expected.tables[state_index] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 8,
                    owner: 0,
                    controller: u32::MAX,
                    movement_infrastructure: 0,
                    ..StateRow::default()
                }]));
        }
        if hour == 2 {
            *expected.tables[state_index] = TableView::Unavailable {
                reason: "state table unreadable".into(),
            };
        } else if hour == 4 {
            *expected.tables[state_index] = TableView::Truncated {
                rows: TableRows::States(vec![StateRow {
                    id: 8,
                    owner: 0,
                    controller: 2,
                    movement_infrastructure: 0,
                    ..StateRow::default()
                }]),
                missing: 10,
            };
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        match hour {
            0 | 3 | 5 => assert!(matches!(
                frame.tables[state_index],
                TableUpdate::Snapshot(_)
            )),
            1 => {
                let TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                } = &frame.tables[state_index]
                else {
                    panic!("state ownership change did not produce a delta")
                };
                assert_eq!(*baseline, 0);
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].mask, 3);
                assert!(deleted.is_empty());
            }
            6 => assert_eq!(
                frame.tables[state_index],
                TableUpdate::Unchanged { baseline: 5 }
            ),
            _ => {}
        }
        if hour > 0 {
            assert_eq!(
                frame.tables[TableId::Provinces.index()],
                TableUpdate::Unchanged { baseline: hour - 1 }
            );
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn division_handles_and_native_health_survive_insert_update_delete_and_dropped_hours() {
    let first_id = 0xfedc_ba98_0000_0035;
    let second_id = 0xfedc_ba98_0000_0036;
    let inserted_id = 0xffff_ffff_0000_0035;
    let index = TableId::Divisions.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 10);
    *first.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(first_id, 300_000, 100_000),
        division(second_id, 300_000, 100_000),
    ]));
    let frame = encoder.encode(first.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut next = source(1, 11);
    *next.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(second_id, i64::MAX, i64::MIN),
        division(inserted_id, 9_007_199_254_740_000, -100_000),
    ]));
    let frame = encoder.encode(next.clone(), false).unwrap();
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[index]
    else {
        panic!("division changes did not produce a delta")
    };
    assert_eq!(*baseline, 10);
    assert_eq!(deleted, &[u128::from(first_id)]);
    assert_eq!(changed.len(), 2);
    assert_eq!(
        (changed[0].key, changed[0].mask),
        (u128::from(second_id), 0b110_0000)
    );
    assert_eq!(
        changed[0]
            .apply(Some(division(second_id, 300_000, 100_000)))
            .unwrap(),
        division(second_id, i64::MAX, i64::MIN)
    );
    assert_eq!(
        (changed[1].key, changed[1].mask),
        (u128::from(inserted_id), TableId::Divisions.full_mask())
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), next);

    let mut resting = source(1, 12);
    resting.tables[index] = next.tables[index].clone();
    let frame = encoder.encode(resting.clone(), false).unwrap();
    assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 11 });
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), resting);

    let mut after_gap = source(1, 19);
    *after_gap.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(second_id, i64::MAX, i64::MIN + 1),
        division(inserted_id, 9_007_199_254_740_000, -100_000),
    ]));
    let frame = encoder.encode(after_gap.clone(), false).unwrap();
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[index]
    else {
        panic!("organisation change did not produce a delta")
    };
    assert_eq!(*baseline, 12);
    assert!(deleted.is_empty());
    assert_eq!(changed.len(), 1);
    assert_eq!(
        (changed[0].key, changed[0].mask),
        (u128::from(second_id), 0b100_0000)
    );
    assert_eq!(changed[0].values, [2]);
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), after_gap);
}

#[test]
fn division_health_is_stored_at_display_precision() {
    let id = 0xfedc_ba98_0000_0035;
    let index = TableId::Divisions.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 10);
    *first.tables[index] =
        TableView::Complete(TableRows::Divisions(vec![division(id, 129_999, 129_999)]));
    let frame = encoder.encode(first.clone(), false).unwrap();
    *first.tables[index] =
        TableView::Complete(TableRows::Divisions(vec![division(id, 120_000, 120_000)]));
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut drifted = source(1, 11);
    *drifted.tables[index] =
        TableView::Complete(TableRows::Divisions(vec![division(id, 125_000, 120_001)]));
    let frame = encoder.encode(drifted, false).unwrap();
    assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 10 });
    decoder.apply(&wire(&frame)).unwrap();

    let mut moved = source(1, 12);
    *moved.tables[index] =
        TableView::Complete(TableRows::Divisions(vec![division(id, 150_000, 99_999)]));
    let frame = encoder.encode(moved, false).unwrap();
    let TableUpdate::Delta { changed, .. } = &frame.tables[index] else {
        panic!("health changes did not produce a delta")
    };
    assert_eq!(
        (changed[0].mask, changed[0].values.as_slice()),
        (0b110_0000, &[6, 5][..])
    );
    let decoded = decoder.apply(&wire(&frame)).unwrap();
    assert_eq!(
        decoded.complete::<tables::Divisions>().unwrap(),
        [division(id, 150_000, 90_000)]
    );
}

#[test]
fn division_failures_and_partial_keyframes_clear_only_their_own_baseline() {
    let index = TableId::Divisions.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..=6 {
        let mut expected = source(1, hour);
        if matches!(hour, 0 | 4) {
            *expected.tables[index] = TableView::Truncated {
                rows: TableRows::Divisions(vec![division(0x1234_5678_0000_0035, 300_000, 100_000)]),
                missing: 17,
            };
        } else if hour == 2 {
            *expected.tables[index] = TableView::Unavailable {
                reason: "division reference unreadable".into(),
            };
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        match hour {
            1 | 3 | 5 => assert!(matches!(frame.tables[index], TableUpdate::Snapshot(_))),
            6 => assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 5 }),
            _ => {}
        }
        if hour > 0 {
            assert_eq!(
                frame.tables[TableId::Provinces.index()],
                TableUpdate::Unchanged { baseline: hour - 1 }
            );
        }
        let actual = decoder.apply(&wire(&frame)).unwrap();
        if hour == 0 {
            unavailable(&actual.tables[index], "17 rows not read");
            expected.tables[index] = actual.tables[index].clone();
        }
        assert_eq!(actual, expected);
    }

    let expected = source(1, 7);
    let mut invalid = encoder.encode(expected.clone(), false).unwrap();
    invalid.tables[index] = TableUpdate::Unchanged { baseline: 999 };
    let actual = decoder.apply(&wire(&invalid)).unwrap();
    unavailable(&actual.tables[index], "baseline at hour 999");
    for table in TableId::ALL
        .into_iter()
        .filter(|table| *table != TableId::Divisions)
    {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    }
    let later = encoder.encode(source(1, 8), false).unwrap();
    unavailable(
        &decoder.apply(&wire(&later)).unwrap().tables[index],
        "baseline at hour 7",
    );
    let recovered = source(1, 9);
    let frame = encoder.encode(recovered.clone(), true).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), recovered);
}

#[test]
fn war_faction_and_membership_failures_recover_independently() {
    let tables = [
        TableId::WarRelations,
        TableId::Factions,
        TableId::FactionMembers,
    ];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..=4 {
        let mut expected = source(1, hour);
        if (1..=3).contains(&hour) {
            let table = tables[hour as usize - 1];
            *expected.tables[table.index()] = if table == TableId::Factions {
                TableView::Truncated {
                    rows: TableRows::Factions(vec![faction(FACTION_ID, b"Incomplete name walk")]),
                    missing: 2,
                }
            } else {
                TableView::Unavailable {
                    reason: format!("{table:?} walk unreadable"),
                }
            };
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        for (offset, table) in tables.into_iter().enumerate() {
            let failure_hour = offset as i32 + 1;
            if hour == 0 || hour == failure_hour + 1 {
                assert!(matches!(
                    frame.tables[table.index()],
                    TableUpdate::Snapshot(_)
                ));
            } else if hour != failure_hour {
                assert_eq!(
                    frame.tables[table.index()],
                    TableUpdate::Unchanged { baseline: hour - 1 },
                    "{table:?} lost its baseline when another table failed"
                );
            }
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    let mut partial = source(1, 5);
    for table in tables {
        let TableView::Complete(rows) = &*partial.tables[table.index()] else {
            panic!()
        };
        *partial.tables[table.index()] = TableView::Truncated {
            rows: rows.clone(),
            missing: 3,
        };
    }
    let frame = encoder.encode(partial.clone(), true).unwrap();
    assert!(frame.keyframe);
    let actual = decoder.apply(&wire(&frame)).unwrap();
    for table in tables {
        unavailable(&actual.tables[table.index()], "3 rows not read");
        partial.tables[table.index()] = actual.tables[table.index()].clone();
    }
    assert_eq!(actual, partial);
    let expected = source(1, 6);
    let frame = encoder.encode(expected.clone(), false).unwrap();
    assert!(!frame.keyframe);
    for table in tables {
        assert!(matches!(
            frame.tables[table.index()],
            TableUpdate::Snapshot(_)
        ));
    }
    assert_eq!(
        frame.tables[TableId::Countries.index()],
        TableUpdate::Unchanged { baseline: 5 }
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn faction_leader_reordering_changes_only_member_ordinals_before_insert_and_delete() {
    let factions = TableId::Factions.index();
    let members = TableId::FactionMembers.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let first = source(1, 0);
    let frame = encoder.encode(first.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut reordered = source(1, 1);
    *reordered.tables[members] = TableView::Complete(TableRows::FactionMembers(vec![
        faction_member(2, FACTION_ID, 0),
        faction_member(7, FACTION_ID, 1),
    ]));
    let frame = encoder.encode(reordered.clone(), false).unwrap();
    assert_eq!(
        frame.tables[factions],
        TableUpdate::Unchanged { baseline: 0 }
    );
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[members]
    else {
        panic!("leader reordering did not produce membership deltas")
    };
    assert_eq!(*baseline, 0);
    assert!(deleted.is_empty());
    assert_eq!(
        changed
            .iter()
            .map(|row| (row.key, row.mask))
            .collect::<Vec<_>>(),
        [(2, 2), (7, 2)]
    );
    assert_eq!(changed[0].values, [1]);
    assert_eq!(changed[1].values, [2]);
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), reordered);

    let new_faction = u64::MAX;
    let mut replaced = source(1, 2);
    *replaced.tables[factions] =
        TableView::Complete(TableRows::Factions(vec![faction(new_faction, b"")]));
    *replaced.tables[members] = TableView::Complete(TableRows::FactionMembers(vec![
        faction_member(2, new_faction, 1),
        faction_member(9, new_faction, 0),
    ]));
    let frame = encoder.encode(replaced.clone(), false).unwrap();
    for (table, expected_changes, expected_deleted) in [
        (
            TableId::Factions,
            vec![(u128::from(new_faction), TableId::Factions.full_mask())],
            vec![u128::from(FACTION_ID)],
        ),
        (
            TableId::FactionMembers,
            vec![(2, 3), (9, TableId::FactionMembers.full_mask())],
            vec![7],
        ),
    ] {
        let TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } = &frame.tables[table.index()]
        else {
            panic!("faction replacement did not produce a {table:?} delta")
        };
        assert_eq!(*baseline, 1);
        assert_eq!(
            changed
                .iter()
                .map(|row| (row.key, row.mask))
                .collect::<Vec<_>>(),
            expected_changes
        );
        assert_eq!(deleted, &expected_deleted);
    }
    assert_eq!(
        frame.tables[TableId::WarRelations.index()],
        TableUpdate::Unchanged { baseline: 1 }
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), replaced);
}

#[test]
fn empty_diplomacy_tables_are_complete_baselines_for_insertions_and_deletions() {
    let tables = [
        TableId::WarRelations,
        TableId::Factions,
        TableId::FactionMembers,
    ];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..=5 {
        let mut expected = source(1, hour);
        if matches!(hour, 0 | 1 | 3 | 4) {
            for table in tables {
                *expected.tables[table.index()] = TableView::Complete(TableRows::empty(table));
            }
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        for table in tables {
            let update = &frame.tables[table.index()];
            match hour {
                0 => assert_eq!(update, &TableUpdate::Snapshot(TableRows::empty(table))),
                1 | 4 => assert_eq!(update, &TableUpdate::Unchanged { baseline: hour - 1 }),
                2 | 3 | 5 => {
                    let TableUpdate::Delta {
                        baseline,
                        changed,
                        deleted,
                    } = update
                    else {
                        panic!("{table:?} lost its complete empty baseline")
                    };
                    assert_eq!(*baseline, hour - 1);
                    let populated = source(1, hour);
                    let TableView::Complete(rows) = &*populated.tables[table.index()] else {
                        panic!()
                    };
                    if hour == 3 {
                        assert!(changed.is_empty());
                        assert_eq!(
                            deleted,
                            &rows.iter().map(|row| row.key()).collect::<Vec<_>>()
                        );
                    } else {
                        assert!(deleted.is_empty());
                        assert_eq!(changed.len(), rows.len());
                        assert!(changed.iter().all(|row| row.mask == table.full_mask()));
                    }
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn key_difference_contexts_restart_for_changed_deleted_and_each_frame() {
    let table = TableId::Divisions;
    let index = table.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 0);
    *first.tables[index] = TableView::Complete(TableRows::Divisions(
        [0, 1, u64::MAX - 1, u64::MAX]
            .into_iter()
            .map(|key| division(key, 300_000, 100_000))
            .collect(),
    ));
    let frame = encoder.encode(first.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut next = source(1, 1);
    *next.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(0, 310_000, 100_000),
        division(2, 300_000, 100_000),
        division(u64::MAX, 300_000, 110_000),
    ]));
    let frame = encoder.encode(next.clone(), false).unwrap();
    let mut bytes = Vec::new();
    let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
    let header_size = bytes.len() - sizes.iter().sum::<usize>();
    let table_start = header_size + sizes[..index].iter().sum::<usize>();
    let mut input = Decoder::new(&bytes[table_start..table_start + sizes[index]]);
    assert_eq!(input.u8().unwrap(), 1);
    assert_eq!(input.i32().unwrap(), 0);
    assert_eq!(input.count(1 + table.mask_width()).unwrap(), 3);
    for (previous_key, expected_key) in [(0, 0), (0, 2), (2, u128::from(u64::MAX))] {
        assert_eq!(
            rows::decode_delta(table, previous_key, &mut input)
                .unwrap()
                .key,
            expected_key
        );
    }
    assert_eq!(input.count(1).unwrap(), 2);
    assert_eq!(input.varint().unwrap(), 1);
    assert_eq!(input.varint().unwrap(), u128::from(u64::MAX - 2));
    input.finish().unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), next);

    let mut empty = source(1, 2);
    *empty.tables[index] = TableView::Complete(TableRows::empty(table));
    let frame = encoder.encode(empty.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), empty);
    let mut inserted = source(1, 3);
    *inserted.tables[index] =
        TableView::Complete(TableRows::Divisions(vec![division(0, i64::MIN, i64::MAX)]));
    let frame = encoder.encode(inserted.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), inserted);
}

#[test]
fn malformed_key_differences_cannot_wrap_snapshot_or_deleted_keys() {
    let table = TableId::Divisions;
    let mut bytes = Vec::new();
    put_count(&mut bytes, 2).unwrap();
    rows::encode_row(&division(u64::MAX, 1, 2), 0, &mut bytes).unwrap();
    rows::encode_row(&division(1, 3, 4), 0, &mut bytes).unwrap();
    assert!(decode_rows(table, &mut Decoder::new(&bytes)).is_err());

    let mut encoder = FrameEncoder::new();
    encoder.encode(source(1, 0), false).unwrap();
    let frame = encoder.encode(source(1, 1), false).unwrap();
    bytes.clear();
    let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
    let table_start =
        bytes.len() - sizes.iter().sum::<usize>() + sizes[..table.index()].iter().sum::<usize>();
    bytes.truncate(table_start);
    bytes.push(1);
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    put_count(&mut bytes, 0).unwrap();
    put_count(&mut bytes, 2).unwrap();
    crate::codec::put_varint(&mut bytes, u128::from(u64::MAX));
    crate::codec::put_varint(&mut bytes, 1);
    assert!(Frame::decode(&mut Decoder::new(&bytes)).is_err());
}

#[test]
fn direct_delta_application_rejects_narrow_key_overflow_and_nonzero_singleton_keys() {
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(source(1, 0), false).unwrap();
    let expected = source(1, 1);
    let next = encoder.encode(expected.clone(), false).unwrap();
    for (table, invalid_key) in [
        (TableId::Provinces, u128::from(u32::MAX) + 1),
        (TableId::Campaign, 1),
    ] {
        for delete in [false, true] {
            let mut bad = next.clone();
            let TableView::Complete(values) = &*expected.tables[table.index()] else {
                panic!()
            };
            let mut change = match values {
                TableRows::Provinces(rows) => rows[0].delta(None),
                TableRows::Campaign(rows) => rows[0].delta(None),
                _ => unreachable!(),
            }
            .unwrap()
            .unwrap();
            change.key = invalid_key;
            bad.tables[table.index()] = TableUpdate::Delta {
                baseline: 0,
                changed: if delete { vec![] } else { vec![change] },
                deleted: if delete { vec![invalid_key] } else { vec![] },
            };
            let reason = table.check_key(invalid_key).unwrap_err().to_string();
            assert_eq!(bad.encode(&mut Vec::new()).unwrap_err().to_string(), reason);
            let mut decoder = FrameDecoder::new();
            decoder.apply(&first).unwrap();
            let actual = decoder.apply(&bad).unwrap();
            unavailable(&actual.tables[table.index()], &reason);
            for other in TableId::ALL.into_iter().filter(|other| *other != table) {
                assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
            }
            let later = FrameEncoder::new()
                .encode(source(1, 2).clone(), false)
                .unwrap();
            assert_eq!(decoder.apply(&later).unwrap(), source(1, 2));
        }
    }
}

#[test]
fn truncated_keyframe_names_missing_rows_and_never_supplies_a_baseline() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut expected = source(1, 0);
    *expected.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![province(1, 7)]),
        missing: 12,
    };
    let frame = encoder.encode(expected.clone(), false).unwrap();
    assert!(frame.keyframe);
    assert!(frame.tables.iter().all(|t| matches!(
        t,
        TableUpdate::Snapshot(_)
            | TableUpdate::Unavailable { .. }
            | TableUpdate::Incompatible { .. }
            | TableUpdate::NotRecorded { .. }
    )));
    let actual = decoder.apply(&wire(&frame)).unwrap();
    unavailable(
        &actual.tables[TableId::Provinces.index()],
        "12 rows not read",
    );
    for table in TableId::ALL
        .into_iter()
        .filter(|t| *t != TableId::Provinces)
    {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    }
    let available = source(1, 1);
    let frame = encoder.encode(available.clone(), false).unwrap();
    assert!(matches!(
        frame.tables[TableId::Provinces.index()],
        TableUpdate::Snapshot(_)
    ));
    assert_eq!(decoder.apply(&frame).unwrap(), available);
}

#[test]
fn keyframes_follow_elapsed_hours_and_reset_on_segments_and_branches() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for (hour, keyframe) in [
        (-10, true),
        (709, false),
        (710, true),
        (1429, false),
        (1430, true),
    ] {
        let expected = source(1, hour);
        let frame = encoder.encode(expected.clone(), false).unwrap();
        assert_eq!(frame.keyframe, keyframe, "hour {hour}");
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        if keyframe {
            assert_eq!(FrameDecoder::new().apply(&frame).unwrap(), expected);
        }
    }
    let expected = source(1, 1431);
    let segment = encoder.encode(expected.clone(), true).unwrap();
    assert!(segment.keyframe);
    assert_eq!(
        FrameDecoder::new().apply(&wire(&segment)).unwrap(),
        expected
    );
    assert_eq!(decoder.apply(&segment).unwrap(), expected);
    for (branch, hour) in [(2, 1431), (3, -50)] {
        let mut expected = source(branch, hour);
        change_country(&mut expected, -55);
        let frame = encoder.encode(expected.clone(), false).unwrap();
        assert!(frame.keyframe);
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn missing_baseline_isolates_one_table_and_a_snapshot_restores_it() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    decoder
        .apply(&encoder.encode(source(1, 0), false).unwrap())
        .unwrap();
    let mut expected = source(1, 1);
    change_country(&mut expected, -123_456_789);
    let mut frame = encoder.encode(expected.clone(), false).unwrap();
    frame.tables[TableId::Provinces.index()] = TableUpdate::Unchanged { baseline: 999 };
    let actual = decoder.apply(&wire(&frame)).unwrap();
    unavailable(
        &actual.tables[TableId::Provinces.index()],
        "baseline at hour 999",
    );
    for table in TableId::ALL
        .into_iter()
        .filter(|t| *t != TableId::Provinces)
    {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    }
    let frame = encoder.encode(source(1, 2), false).unwrap();
    unavailable(
        &decoder.apply(&frame).unwrap().tables[TableId::Provinces.index()],
        "baseline at hour 1",
    );
    let expected = source(1, 3);
    let frame = encoder.encode(expected.clone(), true).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn insertion_deletion_and_field_updates_reconstruct_the_direct_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    decoder
        .apply(&encoder.encode(source(1, 0), false).unwrap())
        .unwrap();
    let mut expected = source(1, 1);
    *expected.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![province(2, 99), province(3, 0)]));
    let frame = encoder.encode(expected.clone(), false).unwrap();
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[TableId::Provinces.index()]
    else {
        panic!()
    };
    assert_eq!(*baseline, 0);
    assert_eq!(deleted, &[1]);
    assert_eq!(
        changed.iter().map(|r| (r.key, r.mask)).collect::<Vec<_>>(),
        [(2, 4), (3, TableId::Provinces.full_mask())]
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn sorted_delta_merge_handles_insertions_around_deleted_and_retained_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 0);
    *first.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(2, 2),
        province(4, 4),
        province(6, 6),
    ]));
    decoder
        .apply(&encoder.encode(first.clone(), false).unwrap())
        .unwrap();
    let mut expected = source(1, 1);
    *expected.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(1, 1),
        province(4, 99),
        province(5, 5),
        province(7, 7),
    ]));
    let frame = encoder.encode(expected.clone(), false).unwrap();
    let TableUpdate::Delta {
        changed, deleted, ..
    } = &frame.tables[TableId::Provinces.index()]
    else {
        panic!()
    };
    assert_eq!(
        changed.iter().map(|row| row.key).collect::<Vec<_>>(),
        [1, 4, 5, 7]
    );
    assert_eq!(deleted, &[2, 6]);
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn invalid_delta_operations_do_not_damage_other_tables_or_retain_partial_rows() {
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(source(1, 0), false).unwrap();
    let next_source = source(1, 1);
    let next = encoder.encode(next_source.clone(), false).unwrap();
    let later = encoder.encode(source(1, 2), false).unwrap();
    let change = province(2, 99)
        .delta(Some(&province(2, 2)))
        .unwrap()
        .unwrap();
    let cases = [
        (vec![change.clone()], vec![2], "changes and deletes"),
        (
            vec![change.clone(), change.clone()],
            vec![],
            "unique and ascending",
        ),
        (vec![], vec![1, 1], "unique and ascending"),
        (vec![], vec![1, 999], "absent key"),
        (
            vec![DeltaRow {
                format: crate::FORMAT_VERSION,
                recorded_fields: TableId::Provinces.field_count(),
                key: 3,
                mask: 4,
                values: vec![198, 1],
            }],
            vec![1],
            "requires every field",
        ),
        (
            vec![DeltaRow {
                mask: 1 << TableId::Provinces.field_count(),
                ..change
            }],
            vec![],
            "unknown bits",
        ),
    ];
    for (changed, deleted, reason) in cases {
        let mut decoder = FrameDecoder::new();
        decoder.apply(&first).unwrap();
        let mut bad = next.clone();
        bad.tables[TableId::Provinces.index()] = TableUpdate::Delta {
            baseline: 0,
            changed,
            deleted,
        };
        let actual = decoder.apply(&bad).unwrap();
        unavailable(&actual.tables[TableId::Provinces.index()], reason);
        for table in TableId::ALL
            .into_iter()
            .filter(|t| *t != TableId::Provinces)
        {
            assert_eq!(
                actual.tables[table.index()],
                next_source.tables[table.index()]
            );
        }
        let actual = decoder.apply(&later).unwrap();
        unavailable(
            &actual.tables[TableId::Provinces.index()],
            "baseline at hour 1",
        );
    }
    let mut empty_reason = next;
    empty_reason.tables[TableId::Provinces.index()] = TableUpdate::Unavailable {
        reason: String::new(),
    };
    let mut decoder = FrameDecoder::new();
    decoder.apply(&first).unwrap();
    unavailable(
        &decoder.apply(&empty_reason).unwrap().tables[TableId::Provinces.index()],
        "needs a reason",
    );
}

#[test]
fn invalid_input_and_keyframes_are_rejected_without_advancing_state() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let initial = encoder.encode(source(1, 0), false).unwrap();
    decoder.apply(&initial).unwrap();
    assert!(encoder.encode(source(1, 0), false).is_err());
    assert!(decoder.apply(&initial).is_err());
    let mut invalid_source = source(1, 1);
    change_country(&mut invalid_source, i64::MAX);
    *invalid_source.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![province(2, 2), province(1, 7)]));
    assert!(encoder.encode(invalid_source.clone(), false).is_err());
    let mut invalid_last_source = source(1, 1);
    change_country(&mut invalid_last_source, i64::MIN);
    *invalid_last_source.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![province(1, 7)]));
    *invalid_last_source.tables[TableId::ArmyOrders.index()] =
        TableView::Complete(TableRows::ArmyOrders(vec![
            crate::rows::ArmyOrderRow::default(),
            crate::rows::ArmyOrderRow::default(),
        ]));
    assert!(encoder.encode(invalid_last_source.clone(), false).is_err());
    let expected = source(1, 1);
    let valid = encoder.encode(expected.clone(), false).unwrap();
    for table in TableId::ALL
        .into_iter()
        .filter(|&id| id != TableId::Clock && crate::frame::current_absence(id).is_none())
    {
        assert_eq!(
            valid.tables[table.index()],
            TableUpdate::Unchanged { baseline: 0 },
            "{table:?} changed its baseline before the final table was validated"
        );
    }
    let mut invalid_keyframe = valid.clone();
    invalid_keyframe.keyframe = true;
    assert!(invalid_keyframe.encode(&mut Vec::new()).is_err());
    assert!(decoder.apply(&invalid_keyframe).is_err());
    let mut invalid_last_table = FrameEncoder::new().encode(expected.clone(), false).unwrap();
    invalid_last_table.tables[TableId::ArmyOrders.index()] = TableUpdate::Unchanged { baseline: 0 };
    assert!(decoder.apply(&invalid_last_table).is_err());
    let mut bytes = Vec::new();
    valid.encode(&mut bytes).unwrap();
    bytes[12] = 1;
    assert!(Frame::decode(&mut Decoder::new(&bytes)).is_err());
    assert_eq!(decoder.apply(&wire(&valid)).unwrap(), expected);
}

#[test]
fn complete_singletons_and_clock_hours_are_validated_without_losing_other_tables() {
    let expected = source(1, 0);
    let valid = FrameEncoder::new().encode(expected.clone(), false).unwrap();
    let wrong_clock = ClockRow {
        hour: 99,
        ..ClockRow::default()
    };
    for (table, rows, reason) in [
        (TableId::Clock, TableRows::Clock(vec![]), "exactly one row"),
        (
            TableId::Campaign,
            TableRows::Campaign(vec![]),
            "exactly one row",
        ),
        (
            TableId::SunkShips,
            TableRows::SunkShips(vec![]),
            "exactly one row",
        ),
        (
            TableId::Clock,
            TableRows::Clock(vec![wrong_clock]),
            "hour differs",
        ),
    ] {
        let mut bad_source = expected.clone();
        *bad_source.tables[table.index()] = TableView::Complete(rows.clone());
        assert!(
            FrameEncoder::new()
                .encode(bad_source.clone(), false)
                .is_err()
        );
        let mut bad_frame = valid.clone();
        bad_frame.tables[table.index()] = TableUpdate::Snapshot(rows);
        assert!(bad_frame.encode(&mut Vec::new()).is_err());
        let actual = FrameDecoder::new().apply(&bad_frame).unwrap();
        unavailable(&actual.tables[table.index()], reason);
        for other in TableId::ALL.into_iter().filter(|other| *other != table) {
            assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
        }
    }
}

#[test]
fn singleton_invariants_apply_after_deltas_and_unchanged_updates() {
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(source(1, 0), false).unwrap();
    let expected = source(1, 1);
    let next = encoder.encode(expected.clone(), false).unwrap();
    for (table, update, reason) in [
        (
            TableId::Clock,
            TableUpdate::Delta {
                baseline: 0,
                changed: vec![DeltaRow {
                    format: crate::FORMAT_VERSION,
                    recorded_fields: TableId::Clock.field_count(),
                    key: 0,
                    mask: 1,
                    values: vec![198, 1],
                }],
                deleted: vec![],
            },
            "hour differs",
        ),
        (
            TableId::Clock,
            TableUpdate::Unchanged { baseline: 0 },
            "hour differs",
        ),
        (
            TableId::Campaign,
            TableUpdate::Delta {
                baseline: 0,
                changed: vec![],
                deleted: vec![0],
            },
            "exactly one row",
        ),
    ] {
        let mut decoder = FrameDecoder::new();
        decoder.apply(&first).unwrap();
        let mut bad = next.clone();
        bad.tables[table.index()] = update;
        let actual = decoder.apply(&wire(&bad)).unwrap();
        unavailable(&actual.tables[table.index()], reason);
        for other in TableId::ALL.into_iter().filter(|other| *other != table) {
            assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
        }
    }
}

#[test]
fn duplicate_canonical_country_slots_are_ambiguous_in_snapshots_and_deltas() {
    let first_source = source(1, 0);
    let TableView::Complete(TableRows::Countries(countries)) =
        &*first_source.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    let country = &countries[0];
    let mut alias = country.clone();
    alias.id = 9;
    let duplicate = TableRows::Countries(vec![countries[0].clone(), alias.clone()]);
    for view in [
        TableView::Complete(duplicate.clone()),
        TableView::Truncated {
            rows: duplicate,
            missing: 1,
        },
    ] {
        let mut bad = first_source.clone();
        *bad.tables[TableId::Countries.index()] = view;
        assert!(FrameEncoder::new().encode(bad.clone(), false).is_err());
    }
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(first_source.clone(), false).unwrap();
    let expected = source(1, 1);
    let mut next = encoder.encode(expected.clone(), false).unwrap();
    next.tables[TableId::Countries.index()] = TableUpdate::Delta {
        baseline: 0,
        changed: vec![alias.delta(None).unwrap().unwrap()],
        deleted: vec![],
    };
    let mut decoder = FrameDecoder::new();
    decoder.apply(&first).unwrap();
    let actual = decoder.apply(&wire(&next)).unwrap();
    unavailable(
        &actual.tables[TableId::Countries.index()],
        "duplicate canonical slots",
    );
    assert_eq!(
        actual.tables[TableId::CountryLinks.index()],
        expected.tables[TableId::CountryLinks.index()]
    );
    assert_eq!(
        actual.tables[TableId::Provinces.index()],
        expected.tables[TableId::Provinces.index()]
    );
}

#[test]
fn prepared_record_keeps_baselines_until_complete_spool_write() {
    let mut encoder = FrameEncoder::new();
    let mut pending = encoder.prepare_bytes(source(1, 100).into(), false).unwrap();
    assert_eq!(encoder.state.last_hour, None);
    assert_eq!(encoder.revision, 0);
    assert!(encoder.state.tables.iter().all(Option::is_none));
    let encoded = pending.take_frame();
    assert_eq!(encoded.decode().unwrap().hour, 100);
    drop(pending);
    let mut retry = encoder.prepare_bytes(source(1, 101).into(), false).unwrap();
    assert!(retry.frame().decode().unwrap().keyframe);
    let _written = retry.take_frame();
    encoder.commit(retry).unwrap();
    assert_eq!(encoder.state.last_hour, Some(101));
    assert_eq!(encoder.revision, 1);
    assert!(
        !encoder
            .prepare_bytes(source(1, 102).into(), false)
            .unwrap()
            .frame()
            .decode()
            .unwrap()
            .keyframe
    );
}

#[test]
fn checked_table_clones_preserve_recorded_fields() {
    for tick in 0..4 {
        for table in TableId::ALL {
            if let Some(rows) = rows::sample_extended_table(table, tick) {
                assert_eq!(rows.try_clone().unwrap(), rows, "{table:?} tick {tick}");
            }
        }
    }
    for (table, view) in TableId::ALL.into_iter().zip(source(1, 100).tables) {
        let TableView::Complete(rows) = &*view else {
            assert!(crate::frame::current_absence(table).is_some(), "{table:?}");
            continue;
        };
        assert_eq!(rows.try_clone().unwrap(), *rows);
    }
}

#[test]
fn table_segments_encoded_beside_the_frame_write_its_bytes() {
    for mode in [CaptureMode::Full, CaptureMode::Map] {
        let frame_input = |hour| match mode {
            CaptureMode::Full => FrameInput::from(source(1, hour)),
            CaptureMode::Map => map_input(source(1, hour)),
        };
        let mut whole = FrameEncoder::new();
        let mut split = FrameEncoder::new();
        for hour in 0..4 {
            let mut input = frame_input(hour);
            if hour == 2 {
                input.tables[TableId::Clock.index()] = TableView::Unavailable {
                    reason: "fixture clock fault".into(),
                }
                .into();
            }
            let expected = whole.encode_bytes(input.clone(), hour == 3).unwrap();
            let context = split.frame_context(1, hour, hour == 3);
            let encoder = &split;
            let encoded: Vec<_> = std::thread::scope(|scope| {
                let workers: Vec<_> = (0..2)
                    .map(|lane| {
                        let input = &input;
                        scope.spawn(move || {
                            TableId::ALL
                                .into_iter()
                                .filter(|table| {
                                    table.index() % 3 == lane
                                        && wire::table_stored(
                                            *table,
                                            crate::format::FormatVersion::CURRENT,
                                            mode,
                                        )
                                })
                                .map(|table| {
                                    let segment = encoder
                                        .encode_segment(
                                            &context,
                                            table,
                                            &input.tables[table.index()],
                                        )
                                        .unwrap();
                                    (table, segment)
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().unwrap())
                    .collect()
            });
            let mut segments = std::array::from_fn(|_| None);
            for (table, segment) in encoded {
                segments[table.index()] = Some(segment);
            }
            let mut prepared = split
                .prepare_bytes_with(input, hour == 3, &mut segments)
                .unwrap();
            let actual = prepared.take_frame();
            split.commit(prepared).unwrap();
            assert_eq!(actual.bytes, expected.bytes, "{mode} hour {hour}");
        }
        let input = frame_input(4);
        let other_hour = split.frame_context(1, 5, false);
        let mut segments = std::array::from_fn(|_| None);
        segments[TableId::Countries.index()] = Some(
            split
                .encode_segment(
                    &other_hour,
                    TableId::Countries,
                    &input.tables[TableId::Countries.index()],
                )
                .unwrap(),
        );
        assert!(
            split
                .prepare_bytes_with(input, false, &mut segments)
                .is_err()
        );
        let stale = FrameEncoder::new().frame_context(1, 4, false);
        let countries = frame_input(4).tables[TableId::Countries.index()].clone();
        assert!(
            split
                .encode_segment(&stale, TableId::Countries, &countries)
                .is_err()
        );
    }
}

#[test]
fn commit_hands_back_replaced_baselines_and_unkept_inputs() {
    let mut retiring = FrameEncoder::new();
    let mut plain = FrameEncoder::new();
    let countries = |tag: &[u8]| {
        Arc::new(TableRows::Countries(vec![CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: tag.to_vec(),
            ..Default::default()
        }]))
    };
    let first = countries(b"AAA");
    let replaced = Arc::downgrade(&first);
    let mut held = Vec::new();
    for (hour, rows) in [(0, first), (1, countries(b"BBB"))] {
        let mut input: FrameInput = source(1, hour).into();
        input.tables[TableId::Countries.index()] = TableInput::SharedComplete(rows);
        let mut prepared = retiring.prepare_bytes(input.clone(), false).unwrap();
        let bytes = prepared.take_frame().bytes;
        held.push(retiring.commit_retiring(prepared).unwrap());
        assert_eq!(bytes, plain.encode_bytes(input, false).unwrap().bytes);
    }
    let retired = held.pop().unwrap();
    assert!(retired.len() > 1);
    assert!(replaced.upgrade().is_some());
    drop(retired);
    assert!(replaced.upgrade().is_none());
    let next: FrameInput = source(1, 2).into();
    assert_eq!(
        retiring.encode_bytes(next.clone(), false).unwrap().bytes,
        plain.encode_bytes(next, false).unwrap().bytes
    );
}

#[test]
fn contained_validation_keeps_good_segments_and_rebaselines_refused_table() {
    let _details = crate::codec::error_details_scope();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let baseline = encoder.encode(source(1, 0), false).unwrap();
    decoder.apply(&baseline).unwrap();
    let mut input: FrameInput = source(1, 1).into();
    input.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 64,
            continent: crate::rows::Recorded::NotRecorded,
            ..Default::default()
        }]))
        .into();
    let context = encoder.frame_context(1, 1, false);
    let mut segments = std::array::from_fn(|_| None);
    segments[TableId::Countries.index()] = Some(
        encoder
            .encode_segment(
                &context,
                TableId::Countries,
                &input.tables[TableId::Countries.index()],
            )
            .unwrap(),
    );
    let mut refused = Vec::new();
    let mut prepared = encoder
        .prepare_contained(input, false, &mut segments, |table, error| {
            refused.push(table);
            Ok(format!("spool validation failed: {error}"))
        })
        .unwrap();
    let frame = prepared.take_frame().decode().unwrap();
    encoder.commit(prepared).unwrap();
    let decoded = decoder.apply(&frame).unwrap();
    assert_eq!(refused, [TableId::States]);
    assert!(
        matches!(&*decoded.tables[TableId::States.index()], TableView::Unavailable { reason } if reason.contains("row 64 column continent"))
    );
    assert_eq!(
        decoded.tables[TableId::Countries.index()],
        FrameDecoder::new()
            .apply(&FrameEncoder::new().encode(source(1, 1), true).unwrap())
            .unwrap()
            .tables[TableId::Countries.index()]
    );
    let recovered = encoder.encode(source(1, 2), false).unwrap();
    assert!(!recovered.keyframe);
    assert!(matches!(
        recovered.tables[TableId::States.index()],
        TableUpdate::Snapshot(_)
    ));
    decoder.apply(&recovered).unwrap();
}

#[test]
fn containment_cannot_hide_clock_or_encoder_context_failures() {
    let mut encoder = FrameEncoder::new();
    let mut bad_clock: FrameInput = source(1, 0).into();
    bad_clock.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(Vec::new())).into();
    let mut segments = std::array::from_fn(|_| None);
    assert!(
        encoder
            .prepare_contained(bad_clock, false, &mut segments, |_, _| panic!(
                "clock failure contained"
            ))
            .is_err()
    );
    encoder.encode(source(1, 0), false).unwrap();
    assert!(
        encoder
            .prepare_contained(source(1, 0).into(), false, &mut segments, |_, _| panic!(
                "frame failure contained"
            ))
            .is_err()
    );
    let (token, previous) = encoder
        .comparison_baseline::<tables::Countries>(1, 1, false)
        .unwrap();
    let rows = TableRows::Countries(previous.to_vec());
    let mut bad_indices: FrameInput = source(1, 1).into();
    bad_indices.tables[TableId::Countries.index()] = TableInput::ComparedChanges {
        token,
        indices: Vec::new(),
        rows,
    };
    assert!(
        encoder
            .prepare_contained(bad_indices, false, &mut segments, |_, _| panic!(
                "encoder replacement indices were contained"
            ))
            .is_err()
    );
}

fn map_input(source: DecodedFrame) -> FrameInput {
    let mut map = FrameInput::from(source);
    map.scope_to(crate::scope::CaptureMode::Map);
    map.tables[TableId::DivisionModelInputs.index()] = TableView::Complete(
        crate::rows::sample_extended_table(TableId::DivisionModelInputs, 1).unwrap(),
    )
    .into();
    map
}

#[test]
fn raw_frame_table_sizes_match_storage_for_unmarked_full_and_map() {
    for mode in [CaptureMode::Full, CaptureMode::Map] {
        let input = match mode {
            CaptureMode::Full => FrameInput::from(source(1, 0)),
            CaptureMode::Map => map_input(source(1, 0)),
        };
        let mut frame = FrameEncoder::new().encode_input(input, true).unwrap();
        if mode == CaptureMode::Full {
            frame.tables[TableId::DivisionModelInputs.index()] = TableUpdate::Snapshot(
                crate::rows::sample_extended_table(TableId::DivisionModelInputs, 1).unwrap(),
            );
            frame.metrics.table_ns[TableId::DivisionModelInputs.index()] = 123;
        }
        assert_eq!(frame.capture_mode().unwrap(), mode);
        let mut bytes = Vec::new();
        let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
        assert_eq!(frame.table_encoded_sizes().unwrap(), sizes, "{mode}");
        assert_eq!(
            frame
                .table_encoded_sizes_version(crate::format::FormatVersion::CURRENT)
                .unwrap(),
            sizes,
            "{mode}"
        );
        let decoded = Frame::decode(&mut Decoder::new(&bytes)).unwrap();
        for table in TableId::ALL
            .into_iter()
            .filter(|&table| !crate::scope::records(mode, table))
        {
            assert_eq!(sizes[table.index()], 0, "{mode} {table:?}");
            assert_eq!(
                decoded.metrics.table_ns[table.index()],
                0,
                "{mode} {table:?}"
            );
            let format = crate::format::FormatVersion::CURRENT.version();
            if table.last_change().present(format) {
                assert_eq!(
                    decoded.tables[table.index()],
                    TableUpdate::NotRecorded { mode },
                    "{mode} {table:?}"
                );
            } else {
                assert!(
                    matches!(
                        &decoded.tables[table.index()],
                        TableUpdate::Unavailable { reason }
                            if Some(reason.as_str()) == table.last_change().table_gap(table, format).as_deref()
                    ),
                    "{mode} {table:?}: {:?}",
                    decoded.tables[table.index()]
                );
            }
        }
    }
}

#[test]
fn a_map_frame_stores_nothing_for_the_tables_its_mode_leaves_out() {
    use crate::scope::{self, CaptureMode};
    let full = source(1, 0);
    let map = map_input(full.clone());
    assert_eq!(map.capture_mode().unwrap(), CaptureMode::Map);
    let full_frame = FrameEncoder::new().encode(full, true).unwrap();
    let map_frame = FrameEncoder::new().encode_input(map, true).unwrap();
    assert_eq!(full_frame.capture_mode().unwrap(), CaptureMode::Full);
    assert_eq!(map_frame.capture_mode().unwrap(), CaptureMode::Map);
    let (mut full_bytes, mut map_bytes) = (Vec::new(), Vec::new());
    let full_sizes = full_frame.encode_with_sizes(&mut full_bytes).unwrap();
    let map_sizes = map_frame.encode_with_sizes(&mut map_bytes).unwrap();
    let not_recorded = TableUpdate::NotRecorded {
        mode: CaptureMode::Map,
    };
    let only_full = |table: TableId| {
        table.last_change().present(crate::FORMAT_VERSION)
            && !scope::records(CaptureMode::Map, table)
    };
    let only_map = |table: TableId| {
        table.last_change().present(crate::FORMAT_VERSION)
            && !scope::records(CaptureMode::Full, table)
    };
    let (mut omitted, mut added) = (0, 0);
    let mut left_out = 0;
    for table in TableId::ALL {
        let index = table.index();
        if only_full(table) {
            assert_eq!(map_frame.tables[index], not_recorded, "{table:?}");
            assert_eq!(map_sizes[index], 0, "{table:?}");
            omitted += 8 + full_sizes[index];
            left_out += 1;
        } else if only_map(table) {
            assert_eq!(
                full_frame.tables[index],
                TableUpdate::NotRecorded {
                    mode: CaptureMode::Full
                },
                "{table:?}"
            );
            assert_eq!(full_sizes[index], 0, "{table:?}");
            added += 8 + map_sizes[index];
        } else {
            assert_eq!(
                map_frame.tables[index], full_frame.tables[index],
                "{table:?}"
            );
            assert_eq!(map_sizes[index], full_sizes[index], "{table:?}");
        }
    }
    let expected = TableId::ALL
        .into_iter()
        .filter(|&table| only_full(table))
        .count();
    assert_eq!(left_out, expected);
    assert!(left_out > 0 && scope::records(CaptureMode::Map, TableId::Divisions));
    assert!(added > 8 && only_map(TableId::DivisionModelInputs));
    assert_eq!(full_bytes.len() + added - map_bytes.len(), omitted);
    let decoded = FrameDecoder::new()
        .apply(&Frame::decode(&mut Decoder::new(&map_bytes)).unwrap())
        .unwrap();
    let full_decoded = FrameDecoder::new().apply(&full_frame).unwrap();
    assert_eq!(decoded.capture_mode().unwrap(), CaptureMode::Map);
    for table in TableId::ALL {
        let view = &*decoded.tables[table.index()];
        if map_frame.tables[table.index()] == not_recorded {
            assert_eq!(
                *view,
                TableView::NotRecorded {
                    mode: CaptureMode::Map
                }
            );
            assert_eq!(decoded.metrics.table_ns[table.index()], 0);
        } else if only_map(table) {
            assert!(matches!(view, TableView::Complete(_)), "{table:?}");
        } else {
            assert_eq!(*view, *full_decoded.tables[table.index()], "{table:?}");
        }
    }
    let error = decoded
        .complete::<crate::rows::tables::DivisionStatus>()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("DivisionStatus is not recorded in a map recording"),
        "{error}"
    );
    let mut checkpoint = Vec::new();
    decoded.encode_checkpoint(&mut checkpoint).unwrap();
    let restored = DecodedFrame::decode_checkpoint(&mut Decoder::new(&checkpoint)).unwrap();
    assert_eq!(restored, decoded);
}

#[test]
fn a_frame_refuses_a_mix_of_capture_modes_and_tables_its_mode_leaves_out() {
    use crate::scope::CaptureMode;
    let map = map_input(source(1, 0));
    assert_eq!(map.capture_mode().unwrap(), CaptureMode::Map);
    let mut extra = map.clone();
    extra.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::empty(TableId::DivisionStatus)).into();
    let mut mixed = map.clone();
    mixed.tables[TableId::Divisions.index()] = TableView::NotRecorded {
        mode: CaptureMode::Full,
    }
    .into();
    for input in [extra, mixed] {
        assert!(input.capture_mode().is_err());
        assert!(FrameEncoder::new().prepare_bytes(input, true).is_err());
    }
    let mut missing = map;
    missing.tables[TableId::Divisions.index()] = TableView::NotRecorded {
        mode: CaptureMode::Map,
    }
    .into();
    let mut full = FrameInput::from(source(1, 0));
    full.tables[TableId::DivisionStatus.index()] = TableView::NotRecorded {
        mode: CaptureMode::Full,
    }
    .into();
    for (input, table, mode) in [
        (missing, TableId::Divisions, CaptureMode::Map),
        (full, TableId::DivisionStatus, CaptureMode::Full),
    ] {
        assert_eq!(input.capture_mode().unwrap(), mode);
        let encoded = FrameEncoder::new().encode_input(input, true).unwrap();
        assert_eq!(encoded.tables[table.index()], TableUpdate::NotRecorded { mode });
    }
    let mut unnamed = FrameInput::from(source(1, 0));
    unnamed.tables[TableId::DivisionModelInputs.index()] =
        TableView::Complete(TableRows::empty(TableId::DivisionModelInputs)).into();
    assert_eq!(unnamed.capture_mode().unwrap(), CaptureMode::Full);
    let encoded = FrameEncoder::new().encode_input(unnamed, true).unwrap();
    assert_eq!(
        encoded.tables[TableId::DivisionModelInputs.index()],
        TableUpdate::NotRecorded {
            mode: CaptureMode::Full
        }
    );
}

#[test]
fn a_frame_keeping_some_tables_leaves_the_rest_to_the_native_capture() {
    use crate::scope::{self, CaptureMode};
    let kept = [TableId::Clock, TableId::Countries];
    for mode in [CaptureMode::Full, CaptureMode::Map] {
        let input = |hour| match mode {
            CaptureMode::Full => FrameInput::from(source(1, hour)),
            CaptureMode::Map => map_input(source(1, hour)),
        };
        let mut encoder = FrameEncoder::new();
        let (mut whole, mut cut) = (FrameDecoder::new(), FrameDecoder::new());
        for hour in 0..3 {
            let mut prepared = encoder.prepare_bytes(input(hour), hour == 0).unwrap();
            let complete = prepared.frame().decode().unwrap();
            let encoded = prepared
                .take_frame_keeping(|table| kept.contains(&table))
                .unwrap();
            encoder.commit(prepared).unwrap();
            assert_eq!(encoded.keyframe(), hour == 0, "{mode} hour {hour}");
            let frame = encoded.decode().unwrap();
            assert_eq!(frame.capture_mode().unwrap(), mode);
            assert_eq!(frame.metrics, complete.metrics, "{mode} hour {hour}");
            let mut bytes = Vec::new();
            let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
            assert_eq!(bytes[..], encoded.bytes[EncodedFrame::PREFIX_BYTES..]);
            assert_eq!(sizes, frame.table_encoded_sizes().unwrap());
            let expected = whole.apply(&complete).unwrap();
            let decoded = cut.apply(&frame).unwrap();
            for table in TableId::ALL {
                let index = table.index();
                if kept.contains(&table) {
                    assert_eq!(frame.tables[index], complete.tables[index], "{mode} {table:?}");
                    assert_eq!(
                        *decoded.tables[index], *expected.tables[index],
                        "{mode} {table:?} hour {hour}"
                    );
                } else if table.last_change().present(crate::FORMAT_VERSION) {
                    assert_eq!(
                        frame.tables[index],
                        TableUpdate::NotRecorded { mode },
                        "{mode} {table:?}"
                    );
                    assert_eq!(
                        sizes[index],
                        usize::from(scope::records(mode, table)),
                        "{mode} {table:?}"
                    );
                    assert!(!decoded.recorded(table), "{mode} {table:?}");
                }
            }
        }
    }
}

#[test]
fn prepare_contained_keeping_matches_take_frame_keeping_and_keeps_baselines() {
    let _details = crate::codec::error_details_scope();
    let kept = [TableId::Clock, TableId::Countries];
    let mut full = FrameEncoder::new();
    let mut pruned = FrameEncoder::new();
    let mut full_refused = Vec::new();
    let mut pruned_refused = Vec::new();
    for hour in 0..4 {
        let mut frame = FrameInput::from(source(1, hour));
        if hour == 2 {
            frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
                StateRow {
                    id: 64,
                    continent: crate::rows::Recorded::NotRecorded,
                    ..Default::default()
                },
            ]))
            .into();
        }
        let mut full_segments = std::array::from_fn(|_| None);
        let mut pruned_segments = std::array::from_fn(|_| None);
        let mut prepared_full = full
            .prepare_contained(frame.clone(), hour == 0, &mut full_segments, |table, error| {
                let message = error.to_string();
                full_refused.push((hour, table, message.clone()));
                Ok(message)
            })
            .unwrap();
        let mut prepared_pruned = pruned
            .prepare_contained_keeping(
                frame,
                hour == 0,
                &mut pruned_segments,
                |table| kept.contains(&table),
                |table, error| {
                    let message = error.to_string();
                    pruned_refused.push((hour, table, message.clone()));
                    Ok(message)
                },
            )
            .unwrap();
        let expected = prepared_full
            .take_frame_keeping(|table| kept.contains(&table))
            .unwrap();
        let actual_ptr = prepared_pruned.frame().bytes.as_ptr();
        let actual = prepared_pruned
            .take_frame_keeping(|table| kept.contains(&table))
            .unwrap();
        assert_eq!(actual.bytes.as_ptr(), actual_ptr, "hour {hour}");
        assert_eq!(actual.bytes, expected.bytes, "hour {hour}");
        assert_eq!(
            prepared_pruned.input().tables[TableId::States.index()],
            prepared_full.input().tables[TableId::States.index()],
            "hour {hour}"
        );
        full.commit(prepared_full).unwrap();
        pruned.commit(prepared_pruned).unwrap();
        for table in TableId::ALL {
            assert_eq!(
                pruned.committed_rows(table),
                full.committed_rows(table),
                "hour {hour} {table:?}"
            );
        }
    }
    assert_eq!(pruned_refused, full_refused);
    assert_eq!(pruned_refused.len(), 1);
}

#[test]
fn changed_key_facts_and_field_stamps_keep_no_source_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder =
        FrameDecoder::for_tables(&[TableId::Campaign, TableId::Countries]).with_change_facts();
    let first = encoder.encode(source(1, 0), true).unwrap();
    let first = decoder.apply(&first).unwrap();
    let original = first.tables[TableId::Countries.index()].identity();
    let slot_mask = 1;
    let power_mask = 1 << 3;
    let mut next = source(1, 1);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        rows[0].command_power_raw = 99;
    } else {
        panic!("fixture countries");
    }
    let update = encoder.encode(next, false).unwrap();
    let second = decoder.apply(&update).unwrap();
    let view = &second.tables[TableId::Countries.index()];
    let changes = view.changes_since(&original).unwrap();
    assert_eq!(changes.changed().len(), 1);
    assert_eq!(changes.row(7), changes.changed().first());
    assert_eq!(changes.row(7).unwrap().fields, power_mask);
    assert!(!changes.row(7).unwrap().inserted);
    assert!(changes.deleted().is_empty());
    assert!(!changes.touches(slot_mask));
    assert!(original.fields_unchanged(view, slot_mask));
    assert!(!original.fields_unchanged(view, power_mask));
    let mut next = source(1, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        rows[0].command_power_raw = 100;
    }
    let update = encoder.encode(next, false).unwrap();
    let third = decoder.apply(&update).unwrap();
    let view = &third.tables[TableId::Countries.index()];
    assert!(
        view.changes_since(&original).is_none(),
        "row facts are one exact predecessor, not a chain"
    );
    assert!(
        original.fields_unchanged(view, slot_mask),
        "field stamps span skipped unrelated deltas"
    );
    assert!(!original.fields_unchanged(view, power_mask));
    let old = view.identity();
    let mut next = source(1, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        rows.clear();
        rows.push(CountryRow {
            id: 8,
            canonical_slot: 2,
            tag: b"NEW".to_vec(),
            ..Default::default()
        });
    }
    let update = encoder.encode(next, false).unwrap();
    let fourth = decoder.apply(&update).unwrap();
    let view = &fourth.tables[TableId::Countries.index()];
    let changes = view.changes_since(&old).unwrap();
    assert_eq!(changes.deleted(), &[7]);
    assert!(changes.row(8).unwrap().inserted);
    assert!(changes.touches(0));
    assert!(!old.fields_unchanged(view, 0));
    drop(decoder);
    drop(first);
    drop(second);
    drop(third);
    let unselected = not_selected();
    for (table, view) in TableId::ALL.into_iter().zip(fourth.tables) {
        if matches!(table, TableId::Campaign | TableId::Countries) {
            assert!(view.try_unwrap().is_ok(), "{table:?}");
        } else {
            assert!(SharedView::ptr_eq(&view, &unselected), "{table:?}");
        }
    }
    drop(original);
    drop(old);
}

#[test]
fn relevant_edits_can_bridge_skipped_unrelated_deltas_but_not_skipped_membership_or_relevant_edits()
{
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&[TableId::Countries]).with_change_facts();
    let slot_bit = (0..TableId::Countries.schema().len())
        .find(|&i| TableId::Countries.field_name(i) == Some("canonical_slot"))
        .unwrap();
    let fields = 1 << slot_bit;
    let mut input = source(1, 0);
    let update = encoder.encode(input.clone(), true).unwrap();
    let first = decoder.apply(&update).unwrap();
    let identity = first.tables[TableId::Countries.index()].identity();
    for hour in 1..=2 {
        set_hour(&mut input, hour);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].command_power_raw += 1;
        }
        let update = encoder.encode(input.clone(), false).unwrap();
        decoder.advance(&update).unwrap();
    }
    set_hour(&mut input, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 999;
    }
    let update = encoder.encode(input.clone(), false).unwrap();
    let current = decoder.apply(&update).unwrap();
    let view = &current.tables[TableId::Countries.index()];
    assert!(view.changes_since(&identity).is_none());
    let changes = view.changes_for(&identity, fields).unwrap();
    assert_eq!(changes.changed().len(), 1);
    assert!(changes.row(7).unwrap().fields & fields != 0);
    set_hour(&mut input, 4);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = first.complete::<tables::Countries>().unwrap()[0].canonical_slot;
    }
    let update = encoder.encode(input.clone(), false).unwrap();
    let current = decoder.apply(&update).unwrap();
    assert!(
        current.tables[TableId::Countries.index()]
            .changes_for(&identity, fields)
            .is_none()
    );
    set_hour(&mut input, 5);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows.clear();
    }
    let update = encoder.encode(input.clone(), false).unwrap();
    decoder.advance(&update).unwrap();
    set_hour(&mut input, 6);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 7,
            ..Default::default()
        });
    }
    let update = encoder.encode(input, false).unwrap();
    let current = decoder.apply(&update).unwrap();
    assert!(
        current.tables[TableId::Countries.index()]
            .changes_for(&identity, 0)
            .is_none()
    );
}

#[test]
fn hour_input_distinguishes_edits_quiet_and_discontinuities_without_owning_rows() {
    use super::{ChangeDiscontinuity, HourInput, TableChanges};
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&[TableId::Countries]).with_change_facts();
    let update = encoder.encode(source(1, 0), true).unwrap();
    decoder.advance(&update).unwrap();
    let first = Arc::clone(decoder.shared_current().unwrap());
    assert!(matches!(
        HourInput::new(&first)
            .changes()
            .since(TableId::Countries, None, u64::MAX),
        TableChanges::Discontinuity(ChangeDiscontinuity::Initial)
    ));
    let mut identity = first.tables[TableId::Countries.index()].identity();
    let mut input = source(1, 1);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows[0].command_power_raw = 777;
    }
    let update = encoder.encode(input.clone(), false).unwrap();
    decoder.advance(&update).unwrap();
    let current = decoder.shared_current().unwrap();
    assert!(Arc::ptr_eq(current, decoder.shared_current().unwrap()));
    let TableChanges::Rows(changes) =
        HourInput::new(current)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX)
    else {
        panic!("changed rows")
    };
    assert_eq!(changes.changed().len(), 1);
    assert_eq!(changes.changed()[0].key, 7);
    assert_eq!(changes.changed()[0].fields, 1 << 3);
    assert!(changes.deleted().is_empty());
    identity = current.tables[TableId::Countries.index()].identity();
    set_hour(&mut input, 2);
    let update = encoder.encode(input.clone(), false).unwrap();
    let current = decoder.advance(&update).unwrap();
    assert!(matches!(
        HourInput::new(current)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX),
        TableChanges::Unchanged
    ));
    set_hour(&mut input, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *input.tables[TableId::Countries.index()]
    {
        rows.clear();
    }
    let update = encoder.encode(input.clone(), false).unwrap();
    let current = decoder.advance(&update).unwrap();
    let TableChanges::Rows(changes) =
        HourInput::new(current)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX)
    else {
        panic!("deleted rows")
    };
    assert_eq!(changes.deleted(), &[7]);
    for branch in [1, 2] {
        let update = encoder.encode(source(branch, 4), true).unwrap();
        let current = decoder.advance(&update).unwrap();
        assert!(matches!(
            HourInput::new(current)
                .changes()
                .since(TableId::Countries, Some(&identity), u64::MAX),
            TableChanges::Discontinuity(_)
        ));
    }
    let saved = decoder.current().unwrap().clone();
    let restored_identity = saved.tables[TableId::Countries.index()].identity();
    let resumed =
        FrameDecoder::resume_for_tables_with_change_facts(&[TableId::Countries], saved).unwrap();
    assert!(matches!(
        HourInput::new(resumed.current().unwrap()).changes().since(
            TableId::Countries,
            Some(&restored_identity),
            u64::MAX
        ),
        TableChanges::Discontinuity(_)
    ));
    assert!(matches!(
        HourInput::new(resumed.current().unwrap()).changes().since(
            TableId::Countries,
            Some(&identity),
            u64::MAX
        ),
        TableChanges::Discontinuity(_)
    ));
    let mut gap = source(2, 5);
    gap.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "synthetic source gap".into(),
    }
    .into();
    let update = encoder.encode(gap, false).unwrap();
    let current = decoder.advance(&update).unwrap();
    assert!(matches!(
        HourInput::new(current)
            .changes()
            .since(TableId::Countries, Some(&identity), u64::MAX),
        TableChanges::Discontinuity(ChangeDiscontinuity::Unavailable)
    ));
    drop(first);
    drop(decoder);
    drop(resumed);
    drop(identity);
    drop(restored_identity);
}

#[test]
fn an_unread_hour_breaks_even_a_carried_tables_change_span() {
    use super::{HourInput, TableChanges};
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&[TableId::Countries]).with_change_facts();
    let update = encoder.encode(source(1, 0), true).unwrap();
    let first = decoder.advance(&update).unwrap();
    let previous = first.tables[TableId::Countries.index()].identity();
    let update = encoder.encode(source(1, 2), false).unwrap();
    let current = decoder.advance(&update).unwrap();
    assert!(matches!(
        HourInput::new(current)
            .changes()
            .since(TableId::Countries, Some(&previous), u64::MAX),
        TableChanges::Discontinuity(_)
    ));
}

#[test]
fn historical_delta_facts_keep_recorded_field_masks_and_missing_suffixes() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::for_tables(&[TableId::Countries]).with_change_facts();
    let first = encoder.encode(source(1, 0), true).unwrap();
    let first = decoder.advance(&first).unwrap();
    let original = first.tables[TableId::Countries.index()].identity();
    let mut next = source(1, 1);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        rows[0].command_power_raw += 1;
    } else {
        panic!("fixture countries");
    }
    let mut update = encoder.encode(next, false).unwrap();
    let changed = if let TableUpdate::Delta { changed, .. } =
        &mut update.tables[TableId::Countries.index()]
    {
        changed
    } else {
        panic!("fixture delta");
    };
    assert_eq!(changed.len(), 1);
    changed[0].format = crate::FORMAT_VERSION - 1;
    let mut suffix = changed[0].clone();
    suffix.recorded_fields = 4;
    assert_eq!(
        decoder::change_fields(TableId::Countries, &suffix),
        suffix.mask | (TableId::Countries.full_mask() & !15)
    );
    suffix.recorded_fields = 64;
    assert_eq!(decoder::change_fields(TableId::Countries, &suffix), suffix.mask);
    let second = decoder.advance(&update).unwrap();
    let view = &second.tables[TableId::Countries.index()];
    let facts = view.changes_since(&original).unwrap();
    assert_eq!(facts.row(7).unwrap().fields, 1 << 3);
    assert!(original.fields_unchanged(view, 1));
    assert!(!original.fields_unchanged(view, 1 << 3));
}

#[test]
fn a_cells_view_names_the_table_and_field_its_row_cannot_leave_unrecorded() {
    use crate::rows::{ArmyTotalRow, Recorded, TABLE_COUNT};
    use crate::scope::CaptureMode;
    use replay_columnar::{
        manifest::Status,
        state::{Cell, Kind},
        store::Store,
    };
    let format = crate::format::FormatVersion::CURRENT.version();
    let (army, drops) = (TableId::ArmyTotals, TableId::BalanceDrops);
    let mut kinds = vec![Vec::new(); TABLE_COUNT];
    kinds[army.index()] = vec![Kind::I64; army.field_count()];
    kinds[drops.index()] = vec![Kind::I64; drops.field_count()];
    let mut store = Store::new(kinds, vec![false; TABLE_COUNT]);
    store.begin_run(0);
    store.begin_hour(0, &[]).unwrap();
    let mut slots = Vec::new();
    for (table, key) in [(army, 3_u128), (drops, 9 << 32)] {
        store
            .status(table.index(), Status::Complete, true, Some(0), true)
            .unwrap();
        let mut changes = store.table(table.index()).unwrap();
        let slot = changes.insert(key).unwrap();
        for column in 0..table.field_count() {
            changes.write(slot, column, Cell::I64(7)).unwrap();
        }
        changes.unrecord(slot, 1).unwrap();
        slots.push((key, slot));
    }
    store.settle();
    let alone = |table: TableId, at: usize| {
        cells_rows(table, &store.view(table.index()), &slots[at..=at], format)
    };
    let view = |table: TableId| {
        cells_view(
            table,
            &store.view(table.index()),
            Status::Complete,
            None,
            CaptureMode::Full,
            format,
        )
    };
    let seven = Recorded::Value(7);
    let expected = ArmyTotalRow {
        slot: 3,
        owner_divisions: seven,
        owner_strength_raw: Recorded::NotRecorded,
        host_divisions: seven,
        host_strength_raw: seven,
        owner_battalions: seven,
        host_battalions: seven,
        owner_manpower: seven,
        host_manpower: seven,
    };
    assert_eq!(
        alone(army, 0).unwrap(),
        TableRows::ArmyTotals(vec![expected.clone()])
    );
    assert_eq!(
        view(army).unwrap(),
        TableView::Complete(TableRows::ArmyTotals(vec![expected]))
    );
    let refused = "BalanceDrops.after is not recorded and its row cannot say so";
    assert_eq!(view(drops).unwrap_err().to_string(), refused);
    let error = alone(drops, 1).unwrap_err();
    assert!(Unsayable::of(&error).is_some(), "{error}");
    assert_eq!(error.to_string(), refused);
}

#[test]
fn rows_built_at_their_slots_are_the_view_s_rows_for_every_column_kind() {
    use crate::rows::{Field, Kind as Wire, List, Opaque, TABLE_COUNT};
    use crate::scope::CaptureMode;
    use replay_columnar::{
        manifest::Status,
        state::{Cell, Kind},
        store::Store,
    };
    const KINDS: [Kind; 17] = [
        Kind::U8,
        Kind::I8,
        Kind::U16,
        Kind::I16,
        Kind::U32,
        Kind::I32,
        Kind::U64,
        Kind::I64,
        Kind::Bool,
        Kind::OptionalI32,
        Kind::OptionalU32,
        Kind::OptionalI64,
        Kind::OptionalU64,
        Kind::Bytes,
        Kind::U32List,
        Kind::U64List,
        Kind::Opaque,
    ];
    fn kind_at(wire: Wire) -> usize {
        match wire {
            Wire::U8 => 0,
            Wire::I8 => 1,
            Wire::U16 => 2,
            Wire::I16 => 3,
            Wire::U32 => 4,
            Wire::I32 => 5,
            Wire::U64 => 6,
            Wire::I64 => 7,
            Wire::Bool => 8,
            Wire::OptionalI32 => 9,
            Wire::OptionalU32 => 10,
            Wire::OptionalI64 => 11,
            Wire::OptionalU64 => 12,
            Wire::Bytes => 13,
            Wire::U32List => 14,
            Wire::U64List => 15,
            Wire::Opaque => 16,
        }
    }
    enum Owned {
        Cell(Cell<'static>),
        Text(Vec<u8>),
        U32s(Vec<u32>),
        U64s(Vec<u64>),
        Opaque(Vec<u8>),
        Unrecorded,
    }
    fn owned(field: Field<'_>) -> Option<Owned> {
        Some(match field {
            Field::Empty => return None,
            Field::NotRecorded => Owned::Unrecorded,
            Field::U8(value) => Owned::Cell(Cell::U8(value)),
            Field::I8(value) => Owned::Cell(Cell::I8(value)),
            Field::U16(value) => Owned::Cell(Cell::U16(value)),
            Field::I16(value) => Owned::Cell(Cell::I16(value)),
            Field::U32(value) => Owned::Cell(Cell::U32(value)),
            Field::I32(value) => Owned::Cell(Cell::I32(value)),
            Field::U64(value) => Owned::Cell(Cell::U64(value)),
            Field::I64(value) => Owned::Cell(Cell::I64(value)),
            Field::Bool(value) => Owned::Cell(Cell::Bool(value)),
            Field::OptionalI32(value) => Owned::Cell(Cell::OptionalI32(value)),
            Field::OptionalU32(value) => Owned::Cell(Cell::OptionalU32(value)),
            Field::OptionalI64(value) => Owned::Cell(Cell::OptionalI64(value)),
            Field::OptionalU64(value) => Owned::Cell(Cell::OptionalU64(value)),
            Field::Bytes(bytes) => Owned::Text(bytes.to_vec()),
            Field::U32List(List::Native(values)) => Owned::U32s(values.to_vec()),
            Field::U32List(List::Encoded(bytes)) => Owned::U32s(
                bytes
                    .chunks_exact(4)
                    .map(|item| u32::from_le_bytes(item.try_into().unwrap()))
                    .collect(),
            ),
            Field::U64List(List::Native(values)) => Owned::U64s(values.to_vec()),
            Field::U64List(List::Encoded(bytes)) => Owned::U64s(
                bytes
                    .chunks_exact(8)
                    .map(|item| u64::from_le_bytes(item.try_into().unwrap()))
                    .collect(),
            ),
            Field::Opaque(Opaque::Native(value)) => {
                let mut bytes = Vec::new();
                value.encode_value(&mut bytes).ok()?;
                Owned::Opaque(bytes)
            }
            Field::Opaque(Opaque::Encoded(bytes, _)) => Owned::Opaque(bytes.to_vec()),
        })
    }
    let format = crate::format::FormatVersion::CURRENT.version();
    let mut tables = Vec::new();
    for table in TableId::ALL {
        let template = TableRows::one_default(table);
        let mut fields = Vec::new();
        let visited = template.first().unwrap().visit_wire(&mut |_, field| {
            fields.push(owned(field));
            Ok(())
        });
        let Some(fields) = visited
            .ok()
            .and_then(|()| fields.into_iter().collect::<Option<Vec<_>>>())
        else {
            continue;
        };
        if fields.len() != table.field_count() {
            continue;
        }
        let keys: Vec<u128> = if table.key_bits() == 0 {
            vec![0]
        } else {
            (1..=3).filter(|&key| table.check_key(key).is_ok()).collect()
        };
        let text = fields.iter().position(|field| matches!(field, Owned::Text(_)));
        tables.push((table, keys, fields, text));
    }
    let mut kinds = vec![Vec::new(); TABLE_COUNT];
    for (table, ..) in &tables {
        kinds[table.index()] = table
            .schema()
            .iter()
            .map(|&wire| KINDS[kind_at(wire)])
            .collect();
    }
    let mut store = Store::new(kinds, vec![false; TABLE_COUNT]);
    store.begin_run(0);
    store.begin_hour(0, &[]).unwrap();
    let mut slots = vec![Vec::new(); TABLE_COUNT];
    for (table, keys, fields, text) in &tables {
        let own: Vec<Option<u32>> = (0..keys.len())
            .map(|at| text.map(|_| store.intern(format!("row {at}").as_bytes()).unwrap()))
            .collect();
        let held: Vec<Option<u32>> = fields
            .iter()
            .map(|field| match field {
                Owned::Text(bytes) => Some(store.intern(bytes).unwrap()),
                _ => None,
            })
            .collect();
        store
            .status(table.index(), Status::Complete, true, Some(0), true)
            .unwrap();
        let mut changes = store.table(table.index()).unwrap();
        for (at, &key) in keys.iter().enumerate() {
            let slot = changes.insert(key).unwrap();
            for (column, field) in fields.iter().enumerate() {
                let cell = match field {
                    Owned::Unrecorded => {
                        changes.unrecord(slot, column).unwrap();
                        continue;
                    }
                    Owned::Cell(cell) => *cell,
                    Owned::Text(_) if *text == Some(column) => Cell::Bytes(own[at].unwrap()),
                    Owned::Text(_) => Cell::Bytes(held[column].unwrap()),
                    Owned::U32s(values) => Cell::U32List(values),
                    Owned::U64s(values) => Cell::U64List(values),
                    Owned::Opaque(bytes) => Cell::Opaque(bytes),
                };
                changes.write(slot, column, cell).unwrap();
            }
            slots[table.index()].push((key, slot));
        }
    }
    store.settle();
    let mut covered = [false; KINDS.len()];
    for (table, _, _, text) in &tables {
        let view = store.view(table.index());
        let Ok(TableView::Complete(rows)) =
            cells_view(*table, &view, Status::Complete, None, CaptureMode::Full, format)
        else {
            continue;
        };
        let keyed = &slots[table.index()];
        assert_eq!(rows.len(), keyed.len(), "{table:?}");
        assert_eq!(cells_rows(*table, &view, keyed, format).unwrap(), rows, "{table:?}");
        for (at, one) in keyed.iter().enumerate() {
            let alone = cells_rows(*table, &view, std::slice::from_ref(one), format).unwrap();
            assert_eq!(alone.first(), rows.get(at), "{table:?} slot {}", one.1);
        }
        if text.is_some() && rows.len() > 1 {
            assert_ne!(rows.get(0), rows.get(1), "{table:?}: each slot's row is its own");
        }
        for &wire in table.schema() {
            covered[kind_at(wire)] = true;
        }
    }
    let missing: Vec<Kind> = KINDS
        .into_iter()
        .zip(covered)
        .filter(|(_, covered)| !covered)
        .map(|(kind, _)| kind)
        .collect();
    assert!(missing.is_empty(), "no table built holds a column of {missing:?}");
}
