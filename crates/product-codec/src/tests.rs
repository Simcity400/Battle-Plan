use super::*;

product! {
    #[derive(Debug, PartialEq)]
    enum Choice { 3 => Empty {}, 9 => Value { id: i64, weight: f32 } }
    #[derive(Debug, PartialEq)]
    struct Packet {
        sequence: u64, text: String, values: Vec<Option<Choice>>,
        names: BTreeMap<u16, Arc<String>>, floats: Arc<[f64]>, fixed: [u8; 3],
    }
}

#[test]
fn explicit_little_endian_counts_tags_and_strings_match_hand_written_bytes() {
    assert_eq!(
        encode(&0x0123_4567_89ab_cdef_u64).unwrap(),
        [0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01]
    );
    assert_eq!(encode(&-2_i16).unwrap(), [0xfe, 0xff]);
    assert_eq!(
        encode(&vec![Some(Choice::Empty {}), None]).unwrap(),
        [2, 0, 0, 0, 0, 0, 0, 0, 1, 3, 0]
    );
    assert_eq!(
        encode(&"\u{e9}".to_string()).unwrap(),
        [2, 0, 0, 0, 0, 0, 0, 0, 0xc3, 0xa9]
    );
    let mut expected = vec![9];
    expected.extend_from_slice(&(-7_i64).to_le_bytes());
    expected.extend_from_slice(&0x8000_0000_u32.to_le_bytes());
    assert_eq!(
        encode(&Choice::Value {
            id: -7,
            weight: -0.0
        })
        .unwrap(),
        expected
    );
    let mut input = Decoder::new(&[0; 8]);
    assert_eq!(input.count64(1).unwrap(), 0);
    input.finish().unwrap();
}

#[test]
fn generated_products_round_trip_nested_containers_and_empty_values() {
    for packet in [
        Packet {
            sequence: u64::MAX,
            text: "snow\0\u{96ea}".into(),
            values: vec![
                None,
                Some(Choice::Empty {}),
                Some(Choice::Value {
                    id: i64::MIN,
                    weight: 0.125,
                }),
            ],
            names: [
                (7, Arc::new("seven".into())),
                (1500, Arc::new(String::new())),
            ]
            .into_iter()
            .collect(),
            floats: vec![f64::INFINITY, -0.0, f64::NEG_INFINITY].into(),
            fixed: [1, 2, 3],
        },
        Packet {
            sequence: 0,
            text: String::new(),
            values: vec![],
            names: BTreeMap::new(),
            floats: Vec::new().into(),
            fixed: [0; 3],
        },
    ] {
        let bytes = encode(&packet).unwrap();
        assert_eq!(decode::<Packet>(&bytes).unwrap(), packet);
        for end in 0..bytes.len() {
            assert!(
                decode::<Packet>(&bytes[..end]).is_err(),
                "accepted prefix {end}"
            );
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode::<Packet>(&trailing).is_err());
    }
}

#[test]
fn floats_keep_every_bit_including_nan_payloads_and_signed_zero() {
    for bits in [
        0,
        0x8000_0000,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_1234,
        0x7f80_0001,
        u32::MAX,
    ] {
        let encoded = encode(&f32::from_bits(bits)).unwrap();
        assert_eq!(encoded, bits.to_le_bytes());
        assert_eq!(decode::<f32>(&encoded).unwrap().to_bits(), bits);
    }
    for bits in [
        0,
        0x8000_0000_0000_0000,
        0x7ff0_0000_0000_0000,
        0x7ff8_0000_0000_4321,
        u64::MAX,
    ] {
        let encoded = encode(&f64::from_bits(bits)).unwrap();
        assert_eq!(encoded, bits.to_le_bytes());
        assert_eq!(decode::<f64>(&encoded).unwrap().to_bits(), bits);
    }
}

#[test]
fn hostile_counts_tags_utf8_and_trailing_bytes_are_rejected() {
    for length in [1, u64::from(u32::MAX) + 1, u64::MAX] {
        let bytes = length.to_le_bytes();
        assert!(decode::<Vec<u8>>(&bytes).is_err());
        assert!(decode::<Arc<[u32]>>(&bytes).is_err());
        assert!(decode::<String>(&bytes).is_err());
        assert!(decode::<BTreeMap<u8, u8>>(&bytes).is_err());
    }
    assert!(decode::<Choice>(&[0]).is_err());
    assert!(decode::<bool>(&[2]).is_err());
    assert!(decode::<Option<u8>>(&[2, 0]).is_err());
    assert!(decode::<String>(&[1, 0, 0, 0, 0, 0, 0, 0, 0xff]).is_err());
    assert!(decode::<u8>(&[0, 0]).is_err());
    assert!(decode::<[u64; 2]>(&[0; 8]).is_err());
}

#[test]
fn map_decoding_refuses_duplicate_and_out_of_order_keys() {
    for entries in [[1, 7, 1, 8], [2, 7, 1, 8]] {
        let mut bytes = 2_u64.to_le_bytes().to_vec();
        bytes.extend_from_slice(&entries);
        assert!(decode::<BTreeMap<u8, u8>>(&bytes).is_err());
    }
    let bytes = [2, 0, 0, 0, 0, 0, 0, 0, 1, 7, 2, 8];
    assert_eq!(
        decode::<BTreeMap<u8, u8>>(&bytes).unwrap(),
        [(1, 7), (2, 8)].into_iter().collect()
    );
}

#[test]
fn schema_changes_for_field_names_types_order_nested_types_and_enum_tags() {
    mod original {
        product! { pub struct Shape { pub first: u32, pub second: Vec<u8> } }
    }
    mod rename {
        product! { pub struct Shape { pub changed: u32, pub second: Vec<u8> } }
    }
    mod retype {
        product! { pub struct Shape { pub first: i32, pub second: Vec<u8> } }
    }
    mod reorder {
        product! { pub struct Shape { pub second: Vec<u8>, pub first: u32 } }
    }
    mod nested {
        product! { pub struct Shape { pub first: u32, pub second: Vec<u16> } }
    }
    mod added {
        product! { pub struct Shape { pub first: u32, pub second: Vec<u8>, pub third: bool } }
    }
    let original = schema_digest::<original::Shape>();
    for changed in [
        schema_digest::<rename::Shape>(),
        schema_digest::<retype::Shape>(),
        schema_digest::<reorder::Shape>(),
        schema_digest::<nested::Shape>(),
        schema_digest::<added::Shape>(),
    ] {
        assert_ne!(original, changed);
    }
    mod tag_a {
        product! { pub enum Shape { 1 => Empty {} } }
    }
    mod tag_b {
        product! { pub enum Shape { 2 => Empty {} } }
    }
    assert_ne!(
        schema_digest::<tag_a::Shape>(),
        schema_digest::<tag_b::Shape>()
    );
    assert_ne!(schema_digest::<[u8; 2]>(), schema_digest::<[u8; 3]>());
    assert_ne!(schema_digest::<Arc<u8>>(), schema_digest::<u8>());
}

#[test]
fn fixed_width_integer_extrema_match_native_little_endian_bytes() {
    macro_rules! integers {
        ($($ty:ty),*) => {$(for value in [<$ty>::MIN, 0, <$ty>::MAX] {
            let bytes = encode(&value).unwrap(); assert_eq!(bytes, value.to_le_bytes());
            assert_eq!(decode::<$ty>(&bytes).unwrap(), value);
        })*};
    }
    integers!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);
    assert_eq!(
        encode(&usize::MAX).unwrap(),
        (usize::MAX as u64).to_le_bytes()
    );
}

#[test]
fn empty_products_have_checked_collection_markers() {
    product! { #[derive(Debug, PartialEq)] struct Empty {} }
    let values = vec![Empty {}, Empty {}];
    let bytes = encode(&values).unwrap();
    assert_eq!(bytes, [2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decode::<Vec<Empty>>(&bytes).unwrap(), values);
    assert!(decode::<Empty>(&[1]).is_err());
}

#[test]
fn empty_arrays_round_trip_and_still_bound_collection_counts() {
    let empty: [u16; 0] = [];
    assert_eq!(encode(&empty).unwrap(), [0]);
    assert_eq!(decode::<[u16; 0]>(&[0]).unwrap(), empty);
    assert!(decode::<[u16; 0]>(&[]).is_err());
    assert!(decode::<[u16; 0]>(&[1]).is_err());
    let values = vec![empty, empty];
    let bytes = encode(&values).unwrap();
    assert_eq!(bytes, [2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(decode::<Vec<[u16; 0]>>(&bytes).unwrap(), values);
    assert!(decode::<Vec<[u16; 0]>>(&u64::MAX.to_le_bytes()).is_err());
}
