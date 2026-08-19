//! The leaf's own suite: prove the walk and the encoder are generated from ONE

use alloc::vec;
use alloc::vec::Vec;

use crate::encode::{encode_payload, HeaderWords, ProvinceRow, RegionRow};
use crate::{
    body_layout, exact_consumption, read_header, HeaderWord, ProvinceCursor, ProvinceField,
    ProvinceFlag, RegionCursor, RegionField, RegionFlag, RowStep, Section, Violation,
    ViolationKind, HEADER_LEN, PROVINCE_KNOWN_FLAGS, PROVINCE_ROW_LEN, REGION_KNOWN_FLAGS,
    REGION_ROW_LEN,
};

fn sample_words(region_count: u32, province_count: u32) -> HeaderWords {
    HeaderWords {
        region_count,
        changed_rows: region_count + province_count,
        status_flags: 0x0000_2000,
        province_count,
        observed_region_rows: 305,
        observed_province_rows: 20000,
        scale_raw: 32768,
        active_region_count: 1,
        max_water_raw: 100,
        max_snow_raw: 200,
    }
}

fn sample_region(id: u32) -> RegionRow {
    RegionRow {
        region_id: id,
        flags: RegionFlag::RainLight.bit() | RegionFlag::ActiveSignal.bit(),
        temperature_raw: -1234,
        phenomenon: 3,
        next_weather_change_raw: 77,
        active_modifier_count: 2,
        intensity: 128,
    }
}

fn sample_province(id: u32) -> ProvinceRow {
    ProvinceRow {
        province_id: id,
        flags: ProvinceFlag::TempPresent.bit() | ProvinceFlag::MapRecolor.bit(),
        temperature_raw: 500,
        temperature_change_raw: -25,
        water_raw: 10,
        snow_raw: 0,
        custom_modifier_count: 1,
    }
}

fn walk(payload: &[u8]) -> Result<(Vec<[u32; 8]>, Vec<[u32; 8]>), Violation> {
    let header = read_header(payload)?;
    let layout = body_layout(payload, &header)?;
    let mut regions = Vec::new();
    let mut cursor = RegionCursor::new(&layout);
    let regions_end = loop {
        match cursor.next(payload)? {
            RowStep::Row { index, view } => {
                assert_eq!(index, regions.len());
                let mut words = [0u32; 8];
                for (i, field) in RegionField::ALL.iter().copied().enumerate() {
                    words[i] = view.u32(field);
                }
                regions.push(words);
            }
            RowStep::End(at) => break at,
        }
    };
    assert_eq!(regions_end, layout.provinces_at);
    let mut provinces = Vec::new();
    let mut cursor = ProvinceCursor::new(&layout);
    let end = loop {
        match cursor.next(payload)? {
            RowStep::Row { index, view } => {
                assert_eq!(index, provinces.len());
                let mut words = [0u32; 8];
                for (i, field) in ProvinceField::ALL.iter().copied().enumerate() {
                    words[i] = view.u32(field);
                }
                provinces.push(words);
            }
            RowStep::End(at) => break at,
        }
    };
    exact_consumption(end, payload.len())?;
    Ok((regions, provinces))
}

/// layouts, both `KNOWN_FLAGS` masks, the walk and the encoder are generated
#[test]
fn every_declared_word_and_field_is_written_and_delimited_and_the_payload_tiles_exactly() {
    for (region_count, province_count) in [(0u32, 0u32), (1, 0), (0, 1), (3, 2)] {
        let words = sample_words(region_count, province_count);
        let regions: Vec<RegionRow> = (0..region_count).map(sample_region).collect();
        let provinces: Vec<ProvinceRow> = (0..province_count).map(sample_province).collect();
        let payload = encode_payload(
            &words,
            99,
            regions.iter().copied(),
            provinces.iter().copied(),
        );

        assert_eq!(
            payload.len(),
            HEADER_LEN
                + region_count as usize * REGION_ROW_LEN
                + province_count as usize * PROVINCE_ROW_LEN
        );

        let header = read_header(&payload).expect("header");
        assert_eq!(header.elapsed_wall_ms(), 99);
        for which in HeaderWord::ALL {
            assert_eq!(header.word(which), words.word(which), "{which:?}");
        }

        let (walked_regions, walked_provinces) = walk(&payload).expect("walks");
        assert_eq!(walked_regions.len(), region_count as usize);
        assert_eq!(walked_provinces.len(), province_count as usize);
        for (i, row) in regions.iter().enumerate() {
            assert_eq!(walked_regions[i][0], row.region_id);
            assert_eq!(walked_regions[i][1], row.flags);
            assert_eq!(walked_regions[i][2] as i32, row.temperature_raw);
            assert_eq!(walked_regions[i][3], row.phenomenon);
            assert_eq!(walked_regions[i][4], row.next_weather_change_raw);
            assert_eq!(walked_regions[i][5], row.active_modifier_count);
            assert_eq!(walked_regions[i][6], row.intensity);
            assert_eq!(walked_regions[i][7], 0, "reserved is written as zero");
        }
        for (i, row) in provinces.iter().enumerate() {
            assert_eq!(walked_provinces[i][0], row.province_id);
            assert_eq!(walked_provinces[i][1], row.flags);
            assert_eq!(walked_provinces[i][2] as i32, row.temperature_raw);
            assert_eq!(walked_provinces[i][3] as i32, row.temperature_change_raw);
            assert_eq!(walked_provinces[i][4] as i32, row.water_raw);
            assert_eq!(walked_provinces[i][5] as i32, row.snow_raw);
            assert_eq!(walked_provinces[i][6], row.custom_modifier_count);
            assert_eq!(walked_provinces[i][7], 0, "reserved is written as zero");
        }
    }

    let mut region_seen = 0u32;
    let mut province_seen = 0u32;
    for (regions, provinces) in flag_coverage_fixtures() {
        let words = sample_words(regions.len() as u32, provinces.len() as u32);
        let payload = encode_payload(
            &words,
            0,
            regions.iter().copied(),
            provinces.iter().copied(),
        );
        let (walked_regions, walked_provinces) = walk(&payload).expect("walks");
        for row in walked_regions {
            region_seen |= row[1];
        }
        for row in walked_provinces {
            province_seen |= row[1];
        }
    }
    assert_eq!(
        region_seen, REGION_KNOWN_FLAGS,
        "the fixtures do not exercise every declared region flag"
    );
    assert_eq!(
        province_seen, PROVINCE_KNOWN_FLAGS,
        "the fixtures do not exercise every declared province flag"
    );
}

fn flag_coverage_fixtures() -> Vec<(Vec<RegionRow>, Vec<ProvinceRow>)> {
    let region_rows: Vec<RegionRow> = [
        RegionFlag::RainLight.bit(),
        RegionFlag::RainHeavy.bit(),
        RegionFlag::Snow.bit(),
        RegionFlag::Blizzard.bit(),
        RegionFlag::ArcticWater.bit(),
        RegionFlag::ActiveSignal.bit(),
        RegionFlag::PhenomenonActive.bit(),
        RegionFlag::Sandstorm.bit(),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, bit)| RegionRow {
        region_id: i as u32,
        flags: bit,
        ..RegionRow::default()
    })
    .collect();
    let province_rows: Vec<ProvinceRow> = [
        ProvinceFlag::TempPresent.bit(),
        ProvinceFlag::TempChangePresent.bit(),
        ProvinceFlag::WaterPresent.bit(),
        ProvinceFlag::SnowPresent.bit(),
        ProvinceFlag::MudPresent.bit(),
        ProvinceFlag::MudActive.bit(),
        ProvinceFlag::CustomModifierCountPresent.bit(),
        ProvinceFlag::StructValidated.bit(),
        ProvinceFlag::MapRecolor.bit(),
        ProvinceFlag::DynamicLut.bit(),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, bit)| ProvinceRow {
        province_id: i as u32,
        flags: bit,
        ..ProvinceRow::default()
    })
    .collect();
    vec![(region_rows, province_rows)]
}

#[test]
fn the_known_flag_masks_are_the_shape_the_readers_test_against() {
    let mut region_mask = 0u32;
    for flag in RegionFlag::ALL {
        assert_eq!(region_mask & flag.bit(), 0, "{flag:?} reuses a bit");
        region_mask |= flag.bit();
    }
    let mut province_mask = 0u32;
    for flag in ProvinceFlag::ALL {
        assert_eq!(province_mask & flag.bit(), 0, "{flag:?} reuses a bit");
        province_mask |= flag.bit();
    }
    assert_eq!(region_mask, REGION_KNOWN_FLAGS);
    assert_eq!(province_mask, PROVINCE_KNOWN_FLAGS);
    assert_eq!(REGION_KNOWN_FLAGS, 0b1111_1111);
    assert_eq!(PROVINCE_KNOWN_FLAGS, 0b11_1111_1111);
}

#[test]
fn a_row_flag_that_every_reader_walks_is_delimited_at_the_same_offset_in_both_tables() {
    assert_eq!(RegionField::Flags.offset(), 4);
    assert_eq!(ProvinceField::Flags.offset(), 4);
    assert_eq!(RegionField::RegionId.offset(), 0);
    assert_eq!(ProvinceField::ProvinceId.offset(), 0);
}

#[test]
fn the_header_bytes_are_the_bytes_the_layout_says() {
    let payload = encode_payload(
        &HeaderWords {
            region_count: 1,
            changed_rows: 2,
            status_flags: 3,
            province_count: 4,
            observed_region_rows: 5,
            observed_province_rows: 6,
            scale_raw: 7,
            active_region_count: 8,
            max_water_raw: 9,
            max_snow_raw: 10,
        },
        0x0102_0304_0506_0708,
        [].into_iter(),
        [].into_iter(),
    );
    assert_eq!(
        &payload[..HEADER_LEN],
        &[
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
            0x06, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x09, 0x00,
            0x00, 0x00, 0x0a, 0x00, 0x00, 0x00,
        ]
    );
}

#[test]
fn a_region_rows_bytes_are_the_bytes_the_layout_says() {
    let payload = encode_payload(
        &sample_words(1, 0),
        0,
        [RegionRow {
            region_id: 0x11,
            flags: 0x22,
            temperature_raw: -1,
            phenomenon: 0x44,
            next_weather_change_raw: 0x55,
            active_modifier_count: 0x66,
            intensity: 0x77,
        }]
        .into_iter(),
        [].into_iter(),
    );
    assert_eq!(
        &payload[HEADER_LEN..HEADER_LEN + REGION_ROW_LEN],
        &[
            0x11, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x44, 0x00,
            0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00, 0x77, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ]
    );
}

#[test]
fn a_province_rows_bytes_are_the_bytes_the_layout_says() {
    let payload = encode_payload(
        &sample_words(0, 1),
        0,
        [].into_iter(),
        [ProvinceRow {
            province_id: 0x11,
            flags: 0x22,
            temperature_raw: 0x33,
            temperature_change_raw: -2,
            water_raw: 0x55,
            snow_raw: 0x66,
            custom_modifier_count: 0x77,
        }]
        .into_iter(),
    );
    assert_eq!(
        &payload[HEADER_LEN..HEADER_LEN + PROVINCE_ROW_LEN],
        &[
            0x11, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00, 0xfe, 0xff,
            0xff, 0xff, 0x55, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00, 0x00, 0x77, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
        ]
    );
}

#[test]
fn the_region_table_comes_first_and_the_province_table_follows_it() {
    let payload = encode_payload(
        &sample_words(2, 1),
        0,
        [sample_region(0), sample_region(1)].into_iter(),
        [sample_province(9)].into_iter(),
    );
    let header = read_header(&payload).expect("header");
    let layout = body_layout(&payload, &header).expect("layout");
    assert_eq!(layout.regions_at, HEADER_LEN);
    assert_eq!(layout.provinces_at, HEADER_LEN + 2 * REGION_ROW_LEN);
    assert_eq!(layout.end, payload.len());
    assert_eq!(
        crate::u32_at(&payload, layout.provinces_at),
        9,
        "province_id at the province table's start"
    );
}

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
fn a_payload_longer_than_its_counts_imply_is_a_length_mismatch_not_a_trailing_section() {
    let mut payload = encode_payload(
        &sample_words(1, 0),
        0,
        [sample_region(0)].into_iter(),
        [].into_iter(),
    );
    payload.push(0xAA);
    let header = read_header(&payload).expect("header");
    let err = body_layout(&payload, &header).expect_err("mismatch");
    assert_eq!(
        err.kind,
        ViolationKind::LengthMismatch {
            expected: HEADER_LEN + REGION_ROW_LEN,
            available: HEADER_LEN + REGION_ROW_LEN + 1,
        }
    );
}

#[test]
fn a_payload_shorter_than_its_counts_imply_is_the_same_refusal() {
    let mut payload = encode_payload(
        &sample_words(2, 0),
        0,
        [sample_region(0), sample_region(1)].into_iter(),
        [].into_iter(),
    );
    payload.truncate(payload.len() - 1);
    let header = read_header(&payload).expect("header");
    let err = body_layout(&payload, &header).expect_err("mismatch");
    assert_eq!(
        err.kind,
        ViolationKind::LengthMismatch {
            expected: HEADER_LEN + 2 * REGION_ROW_LEN,
            available: HEADER_LEN + 2 * REGION_ROW_LEN - 1,
        }
    );
}

#[test]
fn a_count_that_cannot_fit_a_usize_refuses_as_an_overflow_not_a_truncation() {
    let mut words = sample_words(0, 0);
    words.region_count = u32::MAX;
    let payload = encode_payload(&words, 0, [].into_iter(), [].into_iter());
    let header = read_header(&payload).expect("header");
    let err = body_layout(&payload, &header).expect_err("mismatch");
    match err.kind {
        ViolationKind::LengthMismatch { expected, .. } => {
            assert_eq!(expected, HEADER_LEN + u32::MAX as usize * REGION_ROW_LEN);
        }
        other => panic!("expected a length mismatch, got {other:?}"),
    }
}

#[test]
fn the_cursors_report_end_repeatedly_and_never_move_past_it() {
    let payload = encode_payload(
        &sample_words(1, 1),
        0,
        [sample_region(0)].into_iter(),
        [sample_province(0)].into_iter(),
    );
    let header = read_header(&payload).expect("header");
    let layout = body_layout(&payload, &header).expect("layout");
    let mut cursor = RegionCursor::new(&layout);
    assert!(matches!(cursor.next(&payload), Ok(RowStep::Row { .. })));
    let at = match cursor.next(&payload) {
        Ok(RowStep::End(at)) => at,
        other => panic!("expected End, got {other:?}"),
    };
    assert_eq!(at, layout.provinces_at);
    assert_eq!(cursor.next(&payload), Ok(RowStep::End(at)));
    assert_eq!(cursor.offset(), at);
}

#[test]
fn an_empty_payload_of_no_rows_is_exactly_the_header() {
    let payload = encode_payload(&sample_words(0, 0), 0, [].into_iter(), [].into_iter());
    assert_eq!(payload.len(), HEADER_LEN);
    let (regions, provinces) = walk(&payload).expect("walks");
    assert!(regions.is_empty());
    assert!(provinces.is_empty());
}
