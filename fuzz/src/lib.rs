#[path = "../fixtures.rs"]
pub mod fixtures;

/// Give raw block/frame mutations valid outer framing and a recomputed trailer CRC.
pub fn raw_recording(raw: &[u8]) -> std::io::Result<replay::container::Replay> {
    use replay::container::Replay;
    use std::sync::OnceLock;

    static BASE: OnceLock<Replay> = OnceLock::new();
    let base = BASE.get_or_init(|| fixtures::recording(false).expect("synthetic recording"));
    let block = &base.index[0];
    let compressed = zstd::encode_all(raw, 3)?;
    let start = block.offset as usize;
    let trailer_start = (block.offset + block.length) as usize;
    let mut bytes = base.bytes()[..start].to_vec();
    bytes.extend_from_slice(&compressed);
    let new_trailer_start = bytes.len();
    bytes.extend_from_slice(&base.bytes()[trailer_start..]);
    let footer = bytes.len() - 24;
    bytes[footer - 16..footer - 8].copy_from_slice(&(compressed.len() as u64).to_le_bytes());
    bytes[footer - 8..footer].copy_from_slice(&(raw.len() as u64).to_le_bytes());
    bytes[footer + 4..footer + 12].copy_from_slice(&(new_trailer_start as u64).to_le_bytes());
    // Derive the index's hour span from complete decoded payloads. Arbitrary
    // malformed payloads retain the single-hour seed; valid mutated sequences
    // can now reach snapshot/delta baselines at every recorded hour.
    let mut input = replay_data::codec::Decoder::new(raw);
    if let Ok(count) = input.count(4) {
        let mut first = None;
        let mut last = None;
        for _ in 0..count {
            let decoded = (|| {
                let length = input.u32()? as usize;
                let mut payload = replay_data::codec::Decoder::new(input.take(length)?);
                let frame = replay_frame::frame::Frame::decode(&mut payload)?;
                payload.finish()?;
                Ok::<_, std::io::Error>(frame)
            })();
            let Ok(frame) = decoded else {
                break;
            };
            first.get_or_insert(frame.hour);
            last = Some(frame.hour);
        }
        if let (Some(first), Some(last)) = (first, last) {
            // The one index entry's last 32 bytes contain its hour span/extents.
            bytes[footer - 32..footer - 28].copy_from_slice(&first.to_le_bytes());
            bytes[footer - 28..footer - 24].copy_from_slice(&last.to_le_bytes());
        }
    }
    let checksum = crc32fast::hash(&bytes[new_trailer_start..footer]);
    bytes[footer + 20..].copy_from_slice(&checksum.to_le_bytes());
    Replay::open(bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn raw_wrapper_reaches_matching_multiframe_snapshot_delta_baselines() {
        let source = super::fixtures::recording_sequence(false, true).unwrap();
        let index = &source.index[0];
        let raw = zstd::decode_all(
            &source.bytes()[index.offset as usize..(index.offset + index.length) as usize],
        )
        .unwrap();
        let recording = super::raw_recording(&raw).unwrap();
        assert_eq!(
            (recording.index[0].first_hour, recording.index[0].last_hour),
            (0, 1)
        );
        let replay::container::SeekResult::Frame(frame) = recording.seek(1, 1).unwrap() else {
            panic!("delta is unreachable");
        };
        assert_eq!(
            frame
                .complete::<replay_data::rows::tables::Countries>()
                .unwrap()[0]
                .command_power_raw,
            1
        );
    }
}
