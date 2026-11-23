use super::*;
use engine_native_memory::memory::SliceMemory;

fn member(text: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x100 + text.len() + 1];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[CACHED_NAME + 16..CACHED_NAME + 24].copy_from_slice(&(text.len() as u64).to_le_bytes());
    let capacity = if text.len() <= 15 { 15 } else { text.len() };
    bytes[CACHED_NAME + 24..CACHED_NAME + 32].copy_from_slice(&(capacity as u64).to_le_bytes());
    if text.len() <= 15 {
        bytes[CACHED_NAME..CACHED_NAME + text.len()].copy_from_slice(text);
    } else {
        bytes[CACHED_NAME..CACHED_NAME + 8].copy_from_slice(&0x1100_u64.to_le_bytes());
        bytes[0x100..0x100 + text.len()].copy_from_slice(text);
    }
    bytes
}

fn read_name(
    bytes: &[u8],
    capacity: usize,
) -> (Result<Option<Span>, &'static str>, Vec<u8>, usize) {
    let mut owned = vec![0; capacity];
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Growing(&mut owned),
        len: 0,
        required: 0,
    };
    let result = cached_name(
        &SliceMemory {
            base: 0x1000,
            bytes,
        },
        0,
        0x1000,
        &mut names,
    );
    let required = names.required;
    (result, owned, required)
}

fn count_name(bytes: &[u8]) -> (Result<Option<Span>, &'static str>, usize) {
    let mut names = Names {
        bytes: engine_native_memory::walk::Buffer::Count,
        len: 0,
        required: 0,
    };
    let result = cached_name(
        &SliceMemory {
            base: 0x1000,
            bytes,
        },
        0,
        0x1000,
        &mut names,
    );
    (result, names.required)
}

#[test]
fn cached_army_name_keeps_empty_inline_heap_and_embedded_nul_bytes() {
    for text in [
        &b""[..],
        &b"Guard\0suffix"[..],
        &b"Legacy generated name %d"[..],
        &[0xff][..],
    ] {
        let bytes = member(text);
        let (result, owned, required) = read_name(&bytes, text.len());
        assert_eq!(result, Ok(Some((0, text.len()))));
        assert_eq!(owned, text);
        assert_eq!(required, text.len());
        assert_eq!(
            cached_name_demand(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000
            ),
            Ok(text.len())
        );
    }
    let mut bytes = member(b"Retained");
    bytes[TYPE..TYPE + 4].copy_from_slice(&i32::MIN.to_le_bytes());
    bytes[TEMPLATE_REFERENCE..TEMPLATE_REFERENCE + 8].fill(0xff);
    assert_eq!(read_name(&bytes[..CACHED_NAME + 32], 8).1, b"Retained");
}

#[test]
fn cached_army_name_counts_only_validated_demand_without_truncating() {
    let text = b"A heap name longer than the admitted byte arena";
    let bytes = member(text);
    assert_eq!(count_name(&bytes), (Ok(None), text.len()));
    let (result, owned, required) = read_name(&bytes, 4);
    assert_eq!(result, Ok(Some((0, text.len()))));
    assert_eq!(required, text.len());
    assert_eq!(&owned[..text.len()], text);
    assert_eq!(read_name(&member(b""), 0).0, Ok(Some((0, 0))));
    let mut bad = bytes.clone();
    bad[0x100 + text.len()] = 1;
    assert_eq!(count_name(&bad).1, 0);
    assert_eq!(read_name(&bad, 4).2, 0);
    assert_eq!(read_name(&bad, 4).0, Err(CACHED[3]));
}

#[test]
fn cached_army_name_refuses_identity_headers_and_short_payloads_independently() {
    let bytes = member(b"A retained heap string");
    let mut bad = bytes.clone();
    bad[0] ^= 1;
    assert_eq!(
        read_name(&bad, 64).0,
        Err("division cached name member identity unreadable or invalid")
    );
    assert_eq!(
        read_name(&bytes[..CACHED_NAME + 31], 64).0,
        Err("division cached name header unreadable")
    );
    bad = bytes.clone();
    bad[CACHED_NAME + 24..CACHED_NAME + 32].copy_from_slice(&1_u64.to_le_bytes());
    assert_eq!(read_name(&bad, 64).0, Err(CACHED[0]));
    bad = bytes.clone();
    bad[CACHED_NAME..CACHED_NAME + 8].fill(0);
    assert_eq!(read_name(&bad, 64).0, Err(CACHED[1]));
    assert_eq!(read_name(&bytes[..0x104], 64).0, Err(CACHED[3]));
}

#[test]
fn member_span_cache_matches_separate_reads_with_one_heap_copy() {
    struct Reads<'a>(SliceMemory<'a>, std::cell::RefCell<Vec<(usize, usize)>>);
    impl Memory for Reads<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.1.borrow_mut().push((address, destination.len()));
            self.0.copy(address, destination)
        }
    }
    let from_span = |bytes: &[u8], capacity: usize| {
        let span: [u8; INPUT_SPAN] = bytes[..INPUT_SPAN].try_into().unwrap();
        let memory = Reads(
            SliceMemory {
                base: 0x1000,
                bytes,
            },
            Default::default(),
        );
        let (mut arena, mut len, mut required) = (vec![0; capacity], 0, 0);
        let result =
            cached_name_from_member(&memory, 0, &span, &mut arena, &mut len, &mut required);
        let reads = memory.1.take();
        (result, arena, required, reads)
    };
    for text in [&b""[..], &b"Retained"[..], &b"Legacy generated name %d"[..]] {
        let bytes = member(text);
        let (result, arena, required, reads) = from_span(&bytes, 64);
        let (expected, owned, expected_required) = read_name(&bytes, 64);
        assert_eq!(result, expected);
        assert_eq!(arena[..text.len()], owned[..text.len()]);
        assert_eq!(required, expected_required);
        let heap = text.len() > 15;
        assert_eq!(
            reads,
            if heap {
                vec![(0x1100, text.len() + 1)]
            } else {
                vec![]
            }
        );
    }
    let text = b"A heap name longer than the admitted byte arena";
    let mut bad = member(text);
    bad[0x100 + text.len()] = 1;
    assert_eq!(from_span(&bad, 64).0, Err(CACHED[3]));
    assert_eq!(from_span(&bad, 64).2, 0);
    let bytes = member(text);
    assert_eq!(from_span(&bytes[..0x104], 64).0, Err(CACHED[3]));
    let (result, arena, required, _) = from_span(&bytes, 4);
    assert_eq!(result, Ok(Some((0, text.len()))));
    assert_eq!((&arena[..text.len()], required), (&text[..], text.len()));
    let mut wrong = member(b"Retained");
    wrong[0] ^= 1;
    assert_eq!(
        from_span(&wrong, 64).0,
        Err("division cached name member identity unreadable or invalid")
    );
}
