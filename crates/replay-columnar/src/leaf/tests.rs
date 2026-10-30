use super::*;

fn slots_round_trip(slots: &[u32]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.slots(slots).unwrap();
    let bytes = writer.into_bytes();
    let mut reader = Reader::new(&bytes);
    let mut read = Vec::new();
    reader.slots(&mut read).unwrap();
    reader.finish().unwrap();
    assert_eq!(read, slots);
    bytes
}

fn positions_round_trip(of: usize, positions: &[u32]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.positions(of, positions).unwrap();
    let bytes = writer.into_bytes();
    let mut reader = Reader::new(&bytes);
    let mut read = Vec::new();
    reader.positions(of, &mut read).unwrap();
    reader.finish().unwrap();
    assert_eq!(read, positions);
    bytes
}

#[test]
fn varints_round_trip_at_every_width_and_refuse_overlong_forms() {
    for value in [
        0,
        1,
        127,
        128,
        16_383,
        16_384,
        u64::from(u32::MAX),
        u64::MAX,
    ] {
        let mut writer = Writer::new();
        writer.varint(value).unwrap();
        assert_eq!(writer.bytes().len(), varint_len(value));
        let bytes = writer.into_bytes();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.varint().unwrap(), value);
        reader.finish().unwrap();
    }
    assert!(
        Reader::new(&[0x80, 0x00]).varint().is_err(),
        "overlong zero"
    );
    assert!(Reader::new(&[0xff; 10]).varint().is_err(), "past 64 bits");
    assert!(Reader::new(&[0x80]).varint().is_err(), "ends inside");
    assert!(
        Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x10])
            .varint_u32()
            .is_err()
    );
}

#[test]
fn slot_lists_choose_the_shorter_form_and_read_back_exactly() {
    slots_round_trip(&[]);
    let sparse = slots_round_trip(&[3, 900, 70_000, u32::MAX]);
    assert_eq!(sparse[1], SLOT_DELTAS);
    let dense: Vec<u32> = (0..1500).filter(|slot| slot % 3 != 0).collect();
    let bytes = slots_round_trip(&dense);
    assert_eq!(bytes[2], SLOT_BITMAP);
    assert!(bytes.len() < 1500 / 8 + 16, "{} bytes", bytes.len());
}

#[test]
fn positions_choose_the_shorter_form_and_read_back_exactly() {
    positions_round_trip(0, &[]);
    positions_round_trip(10, &[]);
    let runs = positions_round_trip(5000, &[0, 1, 2, 3, 4000, 4001]);
    assert_eq!(runs[0], POSITION_RUNS);
    let scattered: Vec<u32> = (0..3700).step_by(2).collect();
    let bytes = positions_round_trip(3700, &scattered);
    assert_eq!(bytes[0], POSITION_BITMAP);
}

#[test]
fn bitmaps_and_runs_are_written_word_by_word() {
    // Runs: their count, then each one's gap from the last one's end and its length less one.
    let runs = positions_round_trip(10, &[0, 1, 2, 5]);
    assert_eq!(runs, [POSITION_RUNS, 2, 0, 2, 2, 0]);
    // Every other position of 64: the form, padding to a word's width, then the word.
    let alternate: Vec<u32> = (0..64).step_by(2).collect();
    let mut bitmap = vec![POSITION_BITMAP, 0, 0, 0, 0, 0, 0, 0];
    bitmap.extend_from_slice(&0x5555_5555_5555_5555_u64.to_le_bytes());
    assert_eq!(positions_round_trip(64, &alternate), bitmap);
    // Slots 0 to 63 and 130: their count, the form and three words, the middle one empty.
    let mut slots: Vec<u32> = (0..64).collect();
    slots.push(130);
    let mut bitmap = vec![65, SLOT_BITMAP, 3, 0, 0, 0, 0, 0];
    for word in [u64::MAX, 0, 1 << 2] {
        bitmap.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(slots_round_trip(&slots), bitmap);
}

#[test]
fn values_align_from_the_buffer_start_and_padding_must_be_zero() {
    let mut writer = Writer::new();
    writer.u8(7).unwrap();
    writer.values(&[1u64, u64::MAX]).unwrap();
    writer.values(&[-2i16]).unwrap();
    let bytes = writer.into_bytes();
    assert_eq!(bytes.len(), 8 + 16 + 2);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.u8().unwrap(), 7);
    let mut wide = Vec::new();
    reader.values::<u64>(2, &mut wide).unwrap();
    assert_eq!(wide, [1, u64::MAX]);
    let mut narrow = Vec::new();
    reader.values::<i16>(1, &mut narrow).unwrap();
    assert_eq!(narrow, [-2]);
    reader.finish().unwrap();

    let mut padded = bytes.clone();
    padded[3] = 1;
    let mut reader = Reader::new(&padded);
    reader.u8().unwrap();
    assert!(
        reader.values::<u64>(2, &mut wide).is_err(),
        "nonzero padding"
    );
    assert!(
        Reader::new(&bytes[..10])
            .values::<u64>(2, &mut wide)
            .is_err()
    );
}

#[test]
fn malformed_lists_are_refused_by_name() {
    let mut writer = Writer::new();
    assert!(writer.slots(&[2, 2]).is_err(), "repeated slot");
    assert!(writer.positions(4, &[4]).is_err(), "position past the list");
    let mut read = Vec::new();
    // A bitmap whose bits disagree with its count.
    let mut writer = Writer::new();
    writer.varint(2).unwrap();
    writer.u8(SLOT_BITMAP).unwrap();
    writer.varint(1).unwrap();
    writer.values(&[1u64]).unwrap();
    assert!(Reader::new(writer.bytes()).slots(&mut read).is_err());
    // A run reaching past the list.
    let mut writer = Writer::new();
    writer.u8(POSITION_RUNS).unwrap();
    writer.varint(1).unwrap();
    writer.varint(3).unwrap();
    writer.varint(1).unwrap();
    assert!(Reader::new(writer.bytes()).positions(4, &mut read).is_err());
    assert!(
        Reader::new(&[9]).positions(4, &mut read).is_err(),
        "unknown form"
    );
    let leftover = [POSITION_RUNS, 0, 0];
    let mut reader = Reader::new(&leftover);
    reader.positions(0, &mut read).unwrap();
    assert!(reader.finish().is_err(), "trailing bytes");
}

#[test]
fn a_slot_bitmap_past_the_last_u32_slot_is_refused_before_its_words() {
    // The last word's top bit is the last slot; a word more would name slot 2^32 and up.
    assert_eq!(SLOT_WORDS * 64 - 1, u32::MAX as usize);
    let mut read = Vec::new();
    let mut writer = Writer::new();
    writer.varint(1).unwrap();
    writer.u8(SLOT_BITMAP).unwrap();
    writer.varint(SLOT_WORDS as u64 + 1).unwrap();
    let error = Reader::new(writer.bytes()).slots(&mut read).unwrap_err();
    assert_eq!(error.to_string(), "Slot bitmap passes the last slot");
}

#[test]
fn integers_lengths_and_keys_round_trip_as_varints() {
    let mut writer = Writer::new();
    writer.ints(&[0i64, -1, 1, i64::MIN, i64::MAX]).unwrap();
    writer.ints(&[0u16, u16::MAX]).unwrap();
    writer.lengths(&[0, 3, 3, 10]).unwrap();
    writer.keys(&[5, 3, u128::MAX, 0, 1 << 100]).unwrap();
    let bytes = writer.into_bytes();
    let mut reader = Reader::new(&bytes);
    let (mut signed, mut narrow, mut ends, mut keys) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    reader.ints::<i64>(5, &mut signed).unwrap();
    reader.ints::<u16>(2, &mut narrow).unwrap();
    reader.lengths(4, &mut ends).unwrap();
    reader.keys(5, &mut keys).unwrap();
    reader.finish().unwrap();
    assert_eq!(signed, [0, -1, 1, i64::MIN, i64::MAX]);
    assert_eq!(narrow, [0, u16::MAX]);
    assert_eq!(ends, [0, 3, 3, 10]);
    assert_eq!(keys, [5, 3, u128::MAX, 0, 1 << 100]);
    // A varint wider than its column is refused.
    let mut writer = Writer::new();
    writer.varint(1 << 16).unwrap();
    assert!(
        Reader::new(writer.bytes())
            .ints::<u16>(1, &mut narrow)
            .is_err()
    );
    assert!(
        Writer::new().lengths(&[3, 2]).is_err(),
        "ends that decrease"
    );
}

#[test]
fn a_slot_list_past_its_end_is_refused_before_anything_is_allocated() {
    let mut read = Vec::new();
    let mut writer = Writer::new();
    writer.slots(&(0..4096).collect::<Vec<u32>>()).unwrap();
    // 4,096 slots where the table holds one: refused by the count alone.
    let error = Reader::new(writer.bytes())
        .slots_below(1, &mut read)
        .unwrap_err();
    assert_eq!(error.to_string(), "Slot list holds more slots than it may");
    assert_eq!(read.capacity(), 0);
    let mut writer = Writer::new();
    writer.slots(&[2, 9]).unwrap();
    let error = Reader::new(writer.bytes())
        .slots_below(9, &mut read)
        .unwrap_err();
    assert_eq!(error.to_string(), "Slot list passes the last slot");
    let mut writer = Writer::new();
    writer.slots(&[2, 8]).unwrap();
    Reader::new(writer.bytes()).slots_below(9, &mut read).unwrap();
    assert_eq!(read, [2, 8]);
}
