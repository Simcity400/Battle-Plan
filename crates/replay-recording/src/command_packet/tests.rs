use super::{test_support::{batch, event}, *};

#[test]
fn codec_retains_negative_deltas_unknowns_and_boundary_integers() {
    let mut batch = batch();
    batch.events[0].hour = i32::MAX;
    batch.events[0].sequence = u32::MAX;
    batch.events[1].hour = i32::MIN;
    batch.events[1].sequence = 0;
    batch.events[1].command_country = Some(u32::MAX);
    batch.events[2].actor = 0;
    batch.events[2].player = -2;
    batch.events[2].origin = Origin::Unresolved;
    batch.events[2].command_country = None;
    let packet = Packet::pack(4, 1, 2, 0, 100, 220, 2, &batch).unwrap();
    assert_eq!(packet.unpack().unwrap(), batch);
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(bytes.len(), packet.encoded_bytes().unwrap());
    let decoded = Packet::decode(&mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded, packet);
    for end in 0..bytes.len() {
        assert!(Packet::decode(&mut Decoder::new(&bytes[..end])).is_err());
    }
    let mut corrupt = packet;
    corrupt.raw_length += 1;
    assert!(corrupt.unpack().is_err());
}

#[test]
fn compact_columns_keep_full_width_actor_residuals_and_backward_span_gaps() {
    let mut batch = batch();
    batch.events = (0..600)
        .map(|i| Event {
            actor: if i % 2 == 0 { u64::MAX } else { 0 },
            player: if i % 2 == 0 { i32::MIN } else { i32::MAX },
            command_country: if i % 2 == 0 { None } else { Some(u32::MAX) },
            qpc: i,
            hour: if i % 2 == 0 { i32::MAX } else { i32::MIN },
            sequence: if i % 2 == 0 { u32::MAX } else { 0 },
            kind: u32::MAX,
            origin: Origin::Unresolved,
            paused: i % 3 == 0,
        })
        .collect();
    batch.spans = vec![
        Span {
            from_qpc: 0,
            to_qpc: u64::MAX,
            hour: i32::MIN,
            complete: true,
            paused: false,
        },
        Span {
            from_qpc: 0,
            to_qpc: 0,
            hour: i32::MAX,
            complete: false,
            paused: true,
        },
    ];
    let packet = Packet::pack(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        0,
        u64::MAX,
        u64::MAX,
        &batch,
    )
    .unwrap();
    assert!(packet.compressed);
    assert_eq!(packet.unpack().unwrap(), batch);
    let mut bytes = Vec::new();
    packet.encode(&mut bytes).unwrap();
    assert_eq!(packet.encoded_bytes().unwrap(), bytes.len());
    assert_eq!(Packet::decode(&mut Decoder::new(&bytes)).unwrap(), packet);
    for events in [Vec::new(), vec![event(1, 1, u64::MAX)]] {
        let batch = Batch {
            events,
            ..Default::default()
        };
        let packet = Packet::pack(1, 1, 1, 0, u64::MAX, u64::MAX, 0, &batch).unwrap();
        assert!(!packet.compressed);
        assert_eq!(packet.unpack().unwrap(), batch);
    }
}
