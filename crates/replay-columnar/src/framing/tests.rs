use super::*;

#[test]
fn manifests_read_back_in_order_up_to_the_unit_end() {
    let mut unit = Writer::new();
    for manifest in [&b"first"[..], &[], &[7; 200]] {
        put_manifest(&mut unit, manifest).unwrap();
    }
    let bytes = unit.into_bytes();
    let mut reader = Reader::new(&bytes);
    let mut read = Vec::new();
    while let Some(manifest) = next_manifest(&mut reader, bytes.len()).unwrap() {
        read.push(manifest.to_vec());
    }
    assert_eq!(read, [b"first".to_vec(), Vec::new(), vec![7; 200]]);
}

#[test]
fn sections_start_aligned_and_read_back_by_their_lengths() {
    let mut unit = Writer::new();
    let sections: [&[u8]; 3] = [&[1, 2, 3], &[4; 9], &[5]];
    for section in sections {
        put_section(&mut unit, section).unwrap();
        assert_eq!((unit.bytes().len() - section.len()) % 8, 0);
    }
    let bytes = unit.into_bytes();
    let mut reader = Reader::new(&bytes);
    for section in sections {
        assert_eq!(next_section(&mut reader, section.len()).unwrap(), section);
    }
    reader.finish().unwrap();
}

#[test]
fn a_table_unit_ends_with_at_most_the_padding_to_a_multiple_of_eight() {
    let ended = |tail: &[u8]| {
        let mut unit = Writer::new();
        put_section(&mut unit, &[1, 2, 3]).unwrap();
        let mut bytes = unit.into_bytes();
        bytes.extend_from_slice(tail);
        let mut reader = Reader::new(&bytes);
        next_section(&mut reader, 3).unwrap();
        finish_sections(&reader).is_ok()
    };
    assert!(ended(&[]));
    assert!(ended(&[0; 5]));
    assert!(!ended(&[0; 4]));
    assert!(!ended(&[0, 0, 0, 0, 1]));
    assert!(!ended(&[0; 13]));
}
