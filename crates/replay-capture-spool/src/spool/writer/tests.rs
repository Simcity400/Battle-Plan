use super::{test_support::*, *};
use replay_frame::{branches::{BranchReason, Parentage}, frame::{Metrics, TableUpdate}};
use std::{cell::RefCell, rc::Rc, sync::Mutex};

fn controlled_clock<B: SegmentBackend>(spool: &mut SegmentedSpool<B>) -> Arc<Mutex<Instant>> {
    let now = Arc::new(Mutex::new(Instant::now()));
    spool.clock.fixed = Some({
        let now = Arc::clone(&now);
        Arc::new(move || *now.lock().unwrap())
    });
    spool.last_sync = *now.lock().unwrap();
    now
}

fn advance(clock: &Mutex<Instant>, by: Duration) {
    *clock.lock().unwrap() += by;
}

fn valid_segment() -> Vec<u8> {
    [
        branch(),
        Record::Frame(frame(10, true)),
        Record::Frame(frame(11, false)),
    ]
    .iter()
    .flat_map(|record| encode_record(record).unwrap())
    .collect()
}

#[test]
fn record_reader_streams_large_segments_with_one_payload_buffer() {
    struct SyntheticSegment {
        bytes: Vec<u8>,
        position: usize,
        next_hour: i32,
        end_hour: i32,
    }
    fn large_frame(hour: i32) -> Record {
        let mut frame = frame(hour, hour == 10);
        frame.tables[0] = TableUpdate::Unavailable {
            reason: "x".repeat(32 * 1024),
        };
        Record::Frame(frame)
    }
    impl Read for SyntheticSegment {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.position == self.bytes.len() {
                if self.next_hour == self.end_hour {
                    return Ok(0);
                }
                self.bytes = encode_record(&large_frame(self.next_hour))?;
                self.next_hour += 1;
                self.position = 0;
            }
            let length = out.len().min(self.bytes.len() - self.position);
            out[..length].copy_from_slice(&self.bytes[self.position..self.position + length]);
            self.position += length;
            Ok(length)
        }
    }
    let context = encode_record(&branch()).unwrap();
    let frame_bytes = encode_record(&large_frame(10)).unwrap().len();
    let count = 8;
    let length = context.len() + count * frame_bytes;
    let source = SyntheticSegment {
        bytes: context,
        position: 0,
        next_hour: 10,
        end_hour: 10 + count as i32,
    };
    let mut reader = RecordReader::new(source, length);
    let mut frames = 0;
    while let Some(record) = reader.next_record().unwrap() {
        if let Record::Frame(frame) = record {
            assert_eq!(frame.hour, 10 + frames);
            frames += 1;
        }
        assert!(reader.payload.capacity() <= frame_bytes * 2);
    }
    assert_eq!(frames, count as i32);
    assert_eq!(reader.accepted.valid_bytes, length);
}

#[test]
fn recover_reader_preserves_prefix_damage_and_propagates_resource_failure() {
    let good = valid_segment();
    for cut in [0, RECORD_HEADER_BYTES - 1, good.len() - 1, good.len()] {
        let old = recover_prefix(&good[..cut]);
        let streamed = recover_reader(
            io::Cursor::new(&good[..cut]),
            cut,
            |record, _| record,
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(streamed.records, old.records);
        assert_eq!(streamed.valid_bytes, old.valid_bytes);
        assert_eq!(streamed.damage, old.damage);
    }
    for kind in [io::ErrorKind::OutOfMemory, io::ErrorKind::PermissionDenied] {
        let error = recover_reader(
            io::Cursor::new(&good),
            good.len(),
            |record, _| record,
            |_| Err(io::Error::from(kind)),
        )
        .unwrap_err();
        assert_eq!(error.kind(), kind);
    }
    let mut header = good[..RECORD_HEADER_BYTES].to_vec();
    header[6..10].copy_from_slice(&(MAX_RECORD_BYTES as u32).to_le_bytes());
    let mut reader = RecordReader::new(io::Cursor::new(&header), header.len());
    assert!(
        reader
            .next_record()
            .unwrap_err()
            .to_string()
            .contains("truncated record payload")
    );
    assert_eq!(reader.payload.capacity(), 0);
    struct FailedRead;
    impl Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::OutOfMemory))
        }
    }
    assert_eq!(
        recover_reader(
            FailedRead,
            RECORD_HEADER_BYTES,
            |record, _| record,
            |_| Ok(())
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::OutOfMemory
    );
}

#[test]
fn control_and_context_bounds_match_encoded_fields() {
    for present in [false, true] {
        let controls = [
            ControlRecord::Detach {
                session: 3,
                last_hour: present.then_some(-7),
            },
            ControlRecord::ClockWitness {
                session: 3,
                qpc: 17,
                hour: -7,
                load_signal: present.then_some(i64::MIN),
            },
            ControlRecord::GapSummary {
                session: 3,
                dropped_frames: 1,
                reason: "synthetic \u{03bb} gap".into(),
            },
            ControlRecord::Partial {
                session: 3,
                reason: "synthetic \u{03bb} failure".into(),
                failed_hour: present.then_some(-7),
            },
            ControlRecord::EventLoss {
                session: 3,
                branch: 7,
                after_hour: -8,
                hour: -7,
            },
        ];
        for control in controls {
            let expected = control.encoded_bytes().unwrap();
            let mut payload = Vec::new();
            control.encode(&mut payload).unwrap();
            assert_eq!(payload.len(), expected);
            let encoded = encode_record(&Record::Control(control.clone())).unwrap();
            assert_eq!(encoded.len(), RECORD_HEADER_BYTES + expected);
            assert_eq!(
                ControlRecord::decode(&mut Decoder::new(&payload)).unwrap(),
                control
            );
        }
        let Record::BranchStart(mut start) = branch() else {
            unreachable!()
        };
        start.loaded_hour = present.then_some(9);
        start.parent = if present {
            Parentage::Probable { branch: 1, hour: 9 }
        } else {
            Parentage::Unknown
        };
        let expected = start.encoded_bytes();
        let encoded = encode_record(&Record::BranchStart(start)).unwrap();
        assert_eq!(
            encoded.len(),
            RECORD_HEADER_BYTES + size_of::<u8>() + expected
        );
    }
    let gap = Gap {
        branch: 7,
        first_hour: -7,
        last_hour: -7,
        reason: "synthetic \u{03bb} gap".into(),
    };
    let expected = gap.encoded_bytes().unwrap();
    let encoded = encode_record(&Record::Gap(gap)).unwrap();
    assert_eq!(
        encoded.len(),
        RECORD_HEADER_BYTES + size_of::<u8>() + expected
    );
}

#[test]
fn partial_capture_control_roundtrips_and_prevents_later_frames() {
    for failed_hour in [Some(12), None] {
        let partial = ControlRecord::Partial {
            session: 3,
            reason: "collector allocation failed".into(),
            failed_hour,
        };
        let encoded = encode_record(&Record::Control(partial.clone())).unwrap();
        assert_eq!(encoded[RECORD_HEADER_BYTES], 7);
        let mut input = Decoder::new(&encoded[RECORD_HEADER_BYTES..]);
        assert_eq!(
            Record::decode(&mut input).unwrap(),
            Record::Control(partial.clone())
        );
        input.finish().unwrap();
        let mut segment = valid_segment();
        segment.extend_from_slice(&encoded);
        segment.extend_from_slice(
            &encode_record(&Record::Control(ControlRecord::Detach {
                session: 3,
                last_hour: Some(11),
            }))
            .unwrap(),
        );
        let recovered = recover_prefix(&segment);
        assert!(recovered.damage.is_none());
        assert_eq!(recovered.records.len(), 5);

        let mut illegal = valid_segment();
        illegal.extend_from_slice(&encoded);
        illegal.extend_from_slice(&encode_record(&Record::Frame(frame(12, false))).unwrap());
        let recovered = recover_prefix(&illegal);
        assert_eq!(recovered.records.len(), 4);
        assert!(recovered.damage.is_some());
    }
    assert!(
        encode_record(&Record::Control(ControlRecord::Partial {
            session: 3,
            reason: " ".into(),
            failed_hour: None,
        }))
        .is_err()
    );
    let records = [
        branch(),
        Record::Gap(Gap {
            branch: 7,
            first_hour: 10,
            last_hour: 10,
            reason: "collector allocation failed".into(),
        }),
        Record::Control(ControlRecord::Partial {
            session: 3,
            reason: "collector allocation failed".into(),
            failed_hour: Some(10),
        }),
        Record::Control(ControlRecord::Detach {
            session: 3,
            last_hour: None,
        }),
    ];
    let bytes: Vec<_> = records
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect();
    let recovered = recover_prefix(&bytes);
    assert_eq!(recovered.records, records);
    assert!(recovered.damage.is_none());
}

#[test]
fn event_loss_control_needs_its_branch_context_and_ordered_reads() {
    let loss = |branch, after_hour, hour| ControlRecord::EventLoss {
        session: 3,
        branch,
        after_hour,
        hour,
    };
    let encoded = encode_record(&Record::Control(loss(7, 11, 12))).unwrap();
    assert_eq!(encoded[RECORD_HEADER_BYTES], 8);
    let mut segment = valid_segment();
    segment.extend_from_slice(&encoded);
    let recovered = recover_prefix(&segment);
    assert!(recovered.damage.is_none());
    assert_eq!(recovered.records[3], Record::Control(loss(7, 11, 12)));

    let mut other = valid_segment();
    other.extend_from_slice(&encode_record(&Record::Control(loss(8, 11, 12))).unwrap());
    assert!(recover_prefix(&other).damage.is_some());
    assert!(encode_record(&Record::Control(loss(7, 12, 12))).is_err());
    let mut reversed = encoded[RECORD_HEADER_BYTES..].to_vec();
    reversed[17..21].copy_from_slice(&13_i32.to_le_bytes());
    assert!(Record::decode(&mut Decoder::new(&reversed)).is_err());

    let older = FormatVersion::read(replay_data::rows::compatibility::EVENT_LOSS_FORMAT - 1).unwrap();
    let error =
        ControlRecord::decode_version(&mut Decoder::new(&encoded[RECORD_HEADER_BYTES..]), older);
    assert!(error.is_err());
}

#[test]
fn each_byte_cut_recovers_exactly_the_complete_prefix() {
    let records = [
        branch(),
        Record::Frame(frame(10, true)),
        Record::Frame(frame(11, false)),
    ];
    let encoded: Vec<_> = records
        .iter()
        .map(|record| encode_record(record).unwrap())
        .collect();
    let bytes: Vec<_> = encoded.iter().flatten().copied().collect();
    let ends: Vec<_> = encoded
        .iter()
        .scan(0, |sum, record| {
            *sum += record.len();
            Some(*sum)
        })
        .collect();
    for cut in 0..=bytes.len() {
        let result = recover_prefix(&bytes[..cut]);
        let count = ends.iter().filter(|&&end| end <= cut).count();
        assert_eq!(result.records, records[..count], "cut {cut}");
        assert_eq!(
            result.valid_bytes,
            count.checked_sub(1).map_or(0, |i| ends[i])
        );
        if cut != 0 && cut != ends[1] && cut != ends[2] {
            assert!(result.damage.is_some(), "cut {cut}");
        }
    }
}

#[test]
fn old_spool_magic_is_refused_before_decoding_any_records() {
    let mut bytes = encode_record(&branch()).unwrap();
    bytes.drain(4..6);
    bytes[..4].copy_from_slice(b"H4SP");
    let recovered = recover_prefix(&bytes);
    assert_eq!(recovered.valid_bytes, 0);
    assert!(recovered.records.is_empty());
    let reason = recovered.damage.unwrap();
    assert!(reason.contains("unsupported spool record magic \"H4SP\""));
    assert!(reason.contains("expected \"H4SV\""));
}

#[test]
fn spool_versions_are_checked_on_every_record_before_payload_decoding() {
    let prefix = valid_segment();
    for version in [0, 90, 95, FORMAT_VERSION + 1, u16::MAX] {
        for with_prefix in [false, true] {
            let mut wrong = encode_record(&Record::Frame(frame(12, false))).unwrap();
            wrong[4..6].copy_from_slice(&version.to_le_bytes());
            wrong[RECORD_HEADER_BYTES] = u8::MAX;
            let crc = crc32fast::hash(&wrong[RECORD_HEADER_BYTES..]);
            wrong[10..14].copy_from_slice(&crc.to_le_bytes());
            let mut bytes = if with_prefix {
                prefix.clone()
            } else {
                Vec::new()
            };
            let valid_bytes = bytes.len();
            bytes.extend(wrong);
            bytes.extend_from_slice(&prefix);
            let recovered = recover_prefix(&bytes);
            assert_eq!(recovered.valid_bytes, valid_bytes);
            assert_eq!(recovered.records.len(), if with_prefix { 3 } else { 0 });
            assert_eq!(
                recovered.damage.as_deref(),
                Some(
                    format!(
                        "segment byte {valid_bytes}: {}",
                        FormatVersion::read(version).unwrap_err()
                    )
                    .as_str()
                ),
            );
        }
    }
}

#[test]
fn corruption_never_hides_valid_later_segments_or_resynchronizes_inside_one() {
    let good = valid_segment();
    let first_size = encode_record(&branch()).unwrap().len();
    for byte in [0, 4, 6, 10, RECORD_HEADER_BYTES, first_size + 10] {
        let mut broken = good.clone();
        broken[byte] ^= 0x80;
        broken.extend_from_slice(&good);
        let result = recover_prefix(&broken);
        assert!(result.damage.is_some());
        assert_eq!(result.records.len(), usize::from(byte >= first_size));
        assert_eq!(recover_prefix(&good).records.len(), 3);
    }
    let mut oversized = good.clone();
    oversized[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(recover_prefix(&oversized).records.is_empty());
}

#[test]
fn valid_crc_does_not_make_bad_payload_or_context_valid() {
    let good = valid_segment();
    let mut unknown = encode_record(&branch()).unwrap();
    unknown[RECORD_HEADER_BYTES] = 255;
    let crc = crc32fast::hash(&unknown[RECORD_HEADER_BYTES..]);
    unknown[10..14].copy_from_slice(&crc.to_le_bytes());
    assert!(
        recover_prefix(&unknown)
            .damage
            .unwrap()
            .contains("unknown spool")
    );
    let no_context = encode_record(&Record::Frame(frame(10, true))).unwrap();
    assert!(recover_prefix(&no_context).records.is_empty());
    let mut dependent_start = encode_record(&branch()).unwrap();
    dependent_start.extend(encode_record(&Record::Frame(frame(10, false))).unwrap());
    assert_eq!(recover_prefix(&dependent_start).records.len(), 1);
    let mut backwards = good;
    backwards.extend(encode_record(&Record::Frame(frame(10, false))).unwrap());
    assert_eq!(recover_prefix(&backwards).records.len(), 3);
    let mut wrong_branch = frame(12, false);
    wrong_branch.branch = 8;
    let mut wrong = valid_segment();
    wrong.extend(encode_record(&Record::Frame(wrong_branch)).unwrap());
    assert_eq!(recover_prefix(&wrong).records.len(), 3);
}

#[test]
fn control_payloads_preserve_optional_witness_values_and_detach_ends_segment() {
    let records = [
        Record::Control(ControlRecord::ClockWitness {
            session: 3,
            qpc: u64::MAX,
            hour: i32::MIN,
            load_signal: Some(i64::MIN),
        }),
        Record::Control(ControlRecord::ClockWitness {
            session: 3,
            qpc: 0,
            hour: 0,
            load_signal: None,
        }),
        Record::Control(ControlRecord::Detach {
            session: 3,
            last_hour: None,
        }),
        Record::Control(ControlRecord::Detach {
            session: 3,
            last_hour: Some(i32::MAX),
        }),
    ];
    for record in records {
        let mut bytes = Vec::new();
        record.encode(&mut bytes).unwrap();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(Record::decode(&mut decoder).unwrap(), record);
        decoder.finish().unwrap();
    }
    let mut bytes = valid_segment();
    bytes.extend(
        encode_record(&Record::Control(ControlRecord::Detach {
            session: 3,
            last_hour: Some(11),
        }))
        .unwrap(),
    );
    let valid_bytes = bytes.len();
    bytes.extend(encode_record(&Record::Frame(frame(12, false))).unwrap());
    let recovered = recover_prefix(&bytes);
    assert_eq!(recovered.valid_bytes, valid_bytes);
    assert!(recovered.damage.unwrap().contains("after detach"));
}

#[test]
fn compact_controls_use_the_same_wire_payload_in_spools_and_trailers() {
    let session = 7_u64;
    let controls = [
        (
            ControlRecord::Detach {
                session,
                last_hour: Some(-2),
            },
            [
                vec![4],
                session.to_le_bytes().to_vec(),
                vec![1],
                (-2_i32).to_le_bytes().to_vec(),
            ]
            .concat(),
        ),
        (
            ControlRecord::ClockWitness {
                session,
                qpc: 13,
                hour: 42,
                load_signal: None,
            },
            [
                vec![5],
                session.to_le_bytes().to_vec(),
                13_u64.to_le_bytes().to_vec(),
                42_i32.to_le_bytes().to_vec(),
                vec![0],
            ]
            .concat(),
        ),
        (
            ControlRecord::GapSummary {
                session,
                dropped_frames: 3,
                reason: "loss".into(),
            },
            [
                vec![6],
                session.to_le_bytes().to_vec(),
                3_u64.to_le_bytes().to_vec(),
                4_u32.to_le_bytes().to_vec(),
                b"loss".to_vec(),
            ]
            .concat(),
        ),
    ];
    for (control, expected) in controls {
        assert_eq!(control.session(), session);
        let mut trailer_payload = Vec::new();
        control.encode(&mut trailer_payload).unwrap();
        assert_eq!(trailer_payload, expected);
        let spool_record = encode_record(&Record::Control(control.clone())).unwrap();
        assert_eq!(spool_record[RECORD_HEADER_BYTES..], expected);
        let mut decoder = Decoder::new(&trailer_payload);
        assert_eq!(ControlRecord::decode(&mut decoder).unwrap(), control);
        decoder.finish().unwrap();
        for length in 0..expected.len() {
            assert!(ControlRecord::decode(&mut Decoder::new(&expected[..length])).is_err());
        }
    }
    for tag in [0, 1, 2, 3, 7, u8::MAX] {
        assert!(ControlRecord::decode(&mut Decoder::new(&[tag])).is_err());
    }
}

#[test]
fn terminal_capture_during_spool_write_never_promotes_the_inflight_record() {
    thread_local! {
        static PENDING: std::cell::Cell<Option<Option<(u32, u64)>>> = const { std::cell::Cell::new(None) };
        static STOP_AT_START: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    fn write_started(preceding: Option<(u32, u64)>) {
        PENDING.set(Some(preceding));
        if STOP_AT_START.get() {
            CAPTURE_STOPPED.set(true);
        }
    }
    struct StopHook;
    impl Drop for StopHook {
        fn drop(&mut self) {
            CAPTURE_STOPPED.set(false);
            STOP_AT_START.set(false);
            engine_allocation::set_stop_hook(None);
        }
    }
    engine_allocation::set_stop_hook(Some(|| CAPTURE_STOPPED.get()));
    let _stop_hook = StopHook;
    for (fault, stop_at_start) in [
        (None, false),
        (None, true),
        (Some(Fault::StopDuringWrite), false),
        (Some(Fault::StopDuringFlush), false),
    ] {
        CAPTURE_STOPPED.set(false);
        STOP_AT_START.set(false);
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        let mut spool = SegmentedSpool::new(backend);
        spool.set_write_start_hook(write_started);
        spool.set_commit_hook(|_, _, _| PENDING.set(None));
        spool.start_segment(&[branch()], &frame(10, true)).unwrap();
        assert_eq!(PENDING.get(), None);
        let watermark = spool.committed_watermark().unwrap();
        let encoded = frame(11, false).encoded().unwrap();
        shared.borrow_mut().fault = fault;
        STOP_AT_START.set(stop_at_start);
        CAPTURE_STOPPED.set(fault.is_none() && !stop_at_start);
        assert_eq!(
            spool.append_encoded_frame(encoded).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        assert!(!spool.is_open());
        assert_eq!(spool.committed_watermark(), Some(watermark));
        assert_eq!(PENDING.get(), Some(Some(watermark)));
        CAPTURE_STOPPED.set(false);
        let memory = shared.borrow();
        let bytes = &memory.files[&0];
        assert_eq!(bytes.len() > watermark.1 as usize, fault.is_some());
        let recovered = recover_prefix(&bytes[..watermark.1 as usize]);
        assert!(recovered.damage.is_none());
        assert!(matches!(
            recovered.records.last(),
            Some(Record::Frame(Frame { hour: 10, .. }))
        ));
    }
}

#[test]
fn write_start_hook_uses_each_campaign_spools_preceding_watermark() {
    thread_local! {
        static PENDING: std::cell::Cell<Option<Option<(u32, u64)>>> = const { std::cell::Cell::new(None) };
        static COMMITTED: std::cell::Cell<Option<(u32, u64)>> = const { std::cell::Cell::new(None) };
        static FIRST_WRITE: std::cell::Cell<Option<Option<(u32, u64)>>> = const { std::cell::Cell::new(None) };
        static LAST_WRITE: std::cell::Cell<Option<Option<(u32, u64)>>> = const { std::cell::Cell::new(None) };
    }
    fn started(preceding: Option<(u32, u64)>) {
        if FIRST_WRITE.get().is_none() {
            FIRST_WRITE.set(Some(preceding));
        }
        PENDING.set(Some(preceding));
        LAST_WRITE.set(Some(preceding));
    }
    fn committed(segment: u32, bytes: u64, _complete_hour: bool) {
        assert!(PENDING.get().is_some());
        PENDING.set(None);
        COMMITTED.set(Some((segment, bytes)));
    }
    let mut first = SegmentedSpool::new(MemoryBackend::default());
    first.set_write_start_hook(started);
    first.set_commit_hook(committed);
    first.start_segment(&[branch()], &frame(10, true)).unwrap();
    assert_eq!(FIRST_WRITE.get(), Some(None));
    let first_watermark = first.committed_watermark();
    let mut second = SegmentedSpool::with_next_segment_index(MemoryBackend::default(), 7);
    second.set_write_start_hook(started);
    second.set_commit_hook(committed);
    second.start_segment(&[branch()], &frame(10, true)).unwrap();
    let second_watermark = second.committed_watermark();
    assert_eq!(COMMITTED.get(), second_watermark);
    first.append(&Record::Frame(frame(11, false))).unwrap();
    assert_eq!(LAST_WRITE.get(), Some(first_watermark));
    assert_ne!(COMMITTED.get(), second_watermark);
    assert_eq!(PENDING.get(), None);
    let preceding = first.committed_watermark();
    first.finish_segment().unwrap();
    FIRST_WRITE.set(None);
    first.start_segment(&[branch()], &frame(10, true)).unwrap();
    assert_eq!(FIRST_WRITE.get(), Some(preceding));
}

#[test]
fn committed_frame_watermark_precedes_postwrite_durability_allocation_failure() {
    thread_local! {
        static WATERMARK: std::cell::Cell<Option<(u32, u64, bool)>> = const { std::cell::Cell::new(None) };
        static ATTEMPTED: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    }
    fn committed(segment: u32, bytes: u64, complete_hour: bool) {
        if complete_hour {
            ATTEMPTED.with(|hour| hour.set(None));
        }
        WATERMARK.with(|watermark| watermark.set(Some((segment, bytes, complete_hour))));
    }
    let backend = MemoryBackend::default();
    let shared = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let clock = controlled_clock(&mut spool);
    spool.set_commit_hook(committed);
    spool.start_segment(&[branch()], &frame(10, true)).unwrap();
    ATTEMPTED.with(|hour| hour.set(Some(11)));
    shared.borrow_mut().fault = Some(Fault::StartSyncOom);
    advance(&clock, SYNC_INTERVAL);
    assert_eq!(
        spool
            .append(&Record::Frame(frame(11, false)))
            .unwrap_err()
            .kind(),
        io::ErrorKind::OutOfMemory
    );
    assert!(!spool.is_open());
    let memory = shared.borrow();
    let bytes = &memory.files[&0];
    assert_eq!(spool.committed_watermark(), Some((0, bytes.len() as u64)));
    assert_eq!(
        WATERMARK.with(std::cell::Cell::get),
        Some((0, bytes.len() as u64, true))
    );
    assert_eq!(ATTEMPTED.with(std::cell::Cell::get), None);
    let recovered = recover_prefix(bytes);
    assert!(recovered.damage.is_none());
    assert!(matches!(
        recovered.records.last(),
        Some(Record::Frame(Frame { hour: 11, .. }))
    ));
}

#[test]
fn sealed_records_publish_watermarks_only_after_the_real_write_and_flush() {
    use replay_frame::frame::{DecodedFrame, FrameEncoder, TableView};
    thread_local! {
        static STORE: RefCell<Option<Rc<RefCell<Memory>>>> = const { RefCell::new(None) };
        static START: std::cell::Cell<(Option<(u32, u64)>, usize)> = const { std::cell::Cell::new((None, 0)) };
        static COMMITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    fn sealed(hour: i32) -> SealedRecord {
        let source = DecodedFrame {
            branch: 7,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                replay_frame::frame::current_absence(replay_data::rows::TableId::ALL[index])
                    .unwrap_or_else(|| TableView::Unavailable {
                        reason: "fixture".into(),
                    })
                    .into()
            }),
        };
        let mut prepared = FrameEncoder::new()
            .prepare_bytes(source.into(), true)
            .unwrap();
        SealedRecord::frame(prepared.take_frame()).unwrap()
    }
    fn started(preceding: Option<(u32, u64)>) {
        STORE.with_borrow(|store| {
            START.set((preceding, store.as_ref().unwrap().borrow().flushes));
        });
    }
    fn committed(segment: u32, bytes: u64, _frame: bool) {
        STORE.with_borrow(|store| {
            let memory = store.as_ref().unwrap().borrow();
            assert!(memory.flushes > START.get().1);
            assert_eq!(bytes as usize, memory.files[&segment].len());
        });
        COMMITS.set(COMMITS.get() + 1);
    }
    for fault in [Fault::Flush, Fault::StartSyncOom] {
        let backend = MemoryBackend::default();
        let memory = backend.0.clone();
        STORE.set(Some(memory.clone()));
        COMMITS.set(0);
        let mut spool = SegmentedSpool::new(backend);
        let clock = controlled_clock(&mut spool);
        spool.set_write_start_hook(started);
        spool.set_commit_hook(committed);
        let records = vec![SealedRecord::context(branch()).unwrap(), sealed(10)];
        assert_eq!(COMMITS.get(), 0);
        assert_eq!(spool.committed_watermark(), None);
        spool.start_sealed_segment(records).unwrap();
        assert_eq!(COMMITS.get(), 2);
        assert_eq!(spool.committed_frames(), 1);
        let preceding = spool.committed_watermark();
        let record = sealed(11);
        assert_eq!(spool.committed_watermark(), preceding);
        memory.borrow_mut().fault = Some(fault);
        advance(&clock, SYNC_INTERVAL);
        assert!(spool.append_sealed(record).is_err());
        assert_eq!(START.get().0, preceding);
        if matches!(fault, Fault::Flush) {
            assert_eq!(COMMITS.get(), 2);
            assert_eq!(spool.committed_frames(), 1);
            assert_eq!(spool.committed_watermark(), preceding);
        } else {
            assert_eq!(COMMITS.get(), 3);
            assert_eq!(spool.committed_frames(), 2);
            assert_ne!(spool.committed_watermark(), preceding);
        }
        STORE.set(None);
    }
}

#[test]
fn pending_periodic_sync_allows_ordered_frames_but_detach_remains_forced() {
    let backend = MemoryBackend::default();
    let shared = backend.0.clone();
    shared.borrow_mut().background = true;
    let mut spool = SegmentedSpool::new(backend);
    let clock = controlled_clock(&mut spool);
    spool.start_segment(&[branch()], &frame(10, true)).unwrap();
    assert_eq!(shared.borrow().syncs, 1);
    advance(&clock, SYNC_INTERVAL);
    spool.sync_if_due().unwrap();
    assert!(shared.borrow().sync_pending);
    for hour in 11..20 {
        advance(&clock, SYNC_INTERVAL);
        spool.append(&Record::Frame(frame(hour, false))).unwrap();
    }
    assert_eq!(shared.borrow().sync_requests, 1);
    assert_eq!(shared.borrow().syncs, 1);
    spool
        .append(&Record::Control(ControlRecord::Detach {
            session: 3,
            last_hour: Some(19),
        }))
        .unwrap();
    assert_eq!(shared.borrow().syncs, 2);
    assert!(!shared.borrow().sync_pending);
    let recovered = recover_prefix(&shared.borrow().files[&0]);
    assert!(recovered.damage.is_none());
}

#[test]
fn background_sync_failure_closes_segment_before_another_record_is_written() {
    for idle in [false, true] {
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        shared.borrow_mut().background = true;
        let mut spool = SegmentedSpool::new(backend);
        let clock = controlled_clock(&mut spool);
        spool.start_segment(&[branch()], &frame(10, true)).unwrap();
        advance(&clock, SYNC_INTERVAL);
        spool.sync_if_due().unwrap();
        spool.append(&Record::Frame(frame(11, false))).unwrap();
        let length = shared.borrow().files[&0].len();
        {
            let mut memory = shared.borrow_mut();
            memory.fault = Some(Fault::Sync);
            memory.sync_ready = true;
        }
        let result = if idle {
            spool.sync_if_due()
        } else {
            spool.append(&Record::Frame(frame(12, false)))
        };
        assert!(result.is_err());
        assert!(!spool.is_open());
        assert!(spool.last_error().is_some());
        assert_eq!(shared.borrow().files[&0].len(), length);
        assert!(spool.append(&Record::Frame(frame(12, false))).is_err());
        shared.borrow_mut().fault = None;
        spool.start_segment(&[branch()], &frame(12, true)).unwrap();
        assert_eq!(spool.next_segment_index(), 2);
    }
}

#[test]
fn forced_boundaries_surface_a_still_pending_durability_failure() {
    for detach in [false, true] {
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        shared.borrow_mut().background = true;
        let mut spool = SegmentedSpool::new(backend);
        let clock = controlled_clock(&mut spool);
        spool.start_segment(&[branch()], &frame(10, true)).unwrap();
        advance(&clock, SYNC_INTERVAL);
        spool.sync_if_due().unwrap();
        assert!(shared.borrow().sync_pending);
        shared.borrow_mut().fault = Some(Fault::Sync);
        let result = if detach {
            spool.append(&Record::Control(ControlRecord::Detach {
                session: 3,
                last_hour: Some(10),
            }))
        } else {
            spool.finish_segment()
        };
        assert!(result.is_err());
        assert!(!spool.is_open());
        assert_eq!(shared.borrow().syncs, 2);
    }
}

#[test]
fn encoded_frames_keep_their_buffer_and_match_the_existing_record_bytes() {
    for keyframe in [false, true] {
        let source = frame(10, keyframe);
        let expected = encode_record(&Record::Frame(source.clone())).unwrap();
        let prepared = source.encoded().unwrap();
        let (pointer, capacity) = prepared.record_storage();
        let actual = encode_frame_record(prepared).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.as_ptr(), pointer);
        assert_eq!(actual.capacity(), capacity);
    }
}

#[test]
fn segment_context_errors_precede_payload_encoding_without_creating_files() {
    let backend = MemoryBackend::default();
    let shared = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let mut malformed = frame(10, true);
    malformed.tables[replay_data::rows::TableId::Clock.index()] = TableUpdate::Unchanged { baseline: 9 };
    assert_eq!(
        spool
            .start_segment(&[], &malformed)
            .unwrap_err()
            .to_string(),
        "segment start needs context and a keyframe",
    );
    assert_eq!(
        spool
            .start_segment(&[Record::Frame(frame(10, true))], &malformed)
            .unwrap_err()
            .to_string(),
        "segment context contains frame or detach",
    );
    assert_eq!(
        spool
            .start_segment(&[branch()], &malformed)
            .unwrap_err()
            .to_string(),
        "keyframe contains a dependent or partial table",
    );
    assert_eq!(spool.next_segment_index(), 0);
    assert!(shared.borrow().files.is_empty());
    spool.start_segment(&[branch()], &frame(10, true)).unwrap();
    assert_eq!(
        spool
            .start_segment(&[], &malformed)
            .unwrap_err()
            .to_string(),
        "close the current segment before starting another",
    );
}

#[test]
fn failed_write_flush_or_sync_permanently_abandons_that_segment() {
    for fault in [Fault::WriteAfter(0), Fault::Flush, Fault::Sync] {
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        let mut spool = SegmentedSpool::new(backend);
        let clock = controlled_clock(&mut spool);
        spool.start_segment(&[branch()], &frame(10, true)).unwrap();
        let old_length = shared.borrow().files[&0].len();
        shared.borrow_mut().fault = Some(match fault {
            Fault::WriteAfter(_) => Fault::WriteAfter(old_length + RECORD_HEADER_BYTES + 5),
            other => other,
        });
        advance(&clock, SYNC_INTERVAL);
        assert!(
            spool
                .append_encoded_frame(frame(11, false).encoded().unwrap())
                .is_err()
        );
        assert!(!spool.is_open());
        assert!(spool.last_error().is_some());
        let abandoned = shared.borrow().files[&0].clone();
        shared.borrow_mut().fault = None;
        assert!(
            spool
                .append_encoded_frame(frame(12, false).encoded().unwrap())
                .is_err()
        );
        assert!(spool.start_segment(&[branch()], &frame(12, false)).is_err());
        let gap = Record::Gap(Gap {
            branch: 7,
            first_hour: 11,
            last_hour: 11,
            reason: "spool I/O failure".into(),
        });
        spool
            .start_segment(&[branch(), gap], &frame(12, true))
            .unwrap();
        spool
            .append_encoded_frame(frame(13, false).encoded().unwrap())
            .unwrap();
        spool.finish_segment().unwrap();
        assert_eq!(spool.next_segment_index(), 2);
        assert_eq!(shared.borrow().files[&0], abandoned);
        let later = recover_prefix(&shared.borrow().files[&1]);
        assert!(later.damage.is_none());
        assert_eq!(later.records.len(), 4);
        let earlier = recover_prefix(&abandoned);
        assert_eq!(
            earlier.records.len(),
            if matches!(fault, Fault::WriteAfter(_)) {
                2
            } else {
                3
            }
        );
    }
}

#[test]
fn failed_bootstrap_and_create_skip_segment_numbers_and_can_restart() {
    for fault in [
        Fault::Create,
        Fault::WriteAfter(15),
        Fault::Flush,
        Fault::Sync,
    ] {
        let backend = MemoryBackend::default();
        let shared = backend.0.clone();
        let mut spool = SegmentedSpool::new(backend);
        shared.borrow_mut().fault = Some(fault);
        assert!(spool.start_segment(&[branch()], &frame(10, true)).is_err());
        assert!(!spool.is_open());
        assert_eq!(spool.next_segment_index(), 1);
        shared.borrow_mut().fault = None;
        spool.start_segment(&[branch()], &frame(11, true)).unwrap();
        assert!(recover_prefix(&shared.borrow().files[&1]).damage.is_none());
    }
}

#[test]
fn gap_only_context_requires_explicit_loss_and_never_replaces_a_keyframe() {
    let mut state = SegmentState::default();
    state.accept(&branch()).unwrap();
    let mut next = match branch() {
        Record::BranchStart(value) => value,
        _ => unreachable!(),
    };
    next.id = 8;
    next.loaded_hour = Some(99);
    next.first_hour = 100;
    next.reason = BranchReason::LoadSignal;
    assert!(state.accept(&Record::BranchStart(next.clone())).is_err());
    for (first_hour, last_hour, reason) in [(9, 10, "lost"), (11, 10, "lost"), (10, 10, "")] {
        assert!(
            state
                .accept(&Record::Gap(Gap {
                    branch: 7,
                    first_hour,
                    last_hour,
                    reason: reason.into(),
                }))
                .is_err()
        );
        assert!(state.accept(&Record::BranchStart(next.clone())).is_err());
    }
    state
        .accept(&Record::Gap(Gap {
            branch: 7,
            first_hour: 10,
            last_hour: 12,
            reason: "spool I/O failure".into(),
        }))
        .unwrap();
    state.accept(&Record::BranchStart(next.clone())).unwrap();
    let mut dependent = frame(100, false);
    dependent.branch = 8;
    assert!(state.accept(&Record::Frame(dependent.clone())).is_err());
    next.id = 9;
    assert!(state.accept(&Record::BranchStart(next)).is_err());
    assert!(
        state
            .accept(&Record::Control(ControlRecord::Detach {
                session: 3,
                last_hour: None,
            }))
            .is_err()
    );
    dependent.keyframe = true;
    state.accept(&Record::Frame(dependent)).unwrap();
}

#[test]
fn every_record_flushes_and_pause_timeout_and_detach_sync() {
    let backend = MemoryBackend::default();
    let shared = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let clock = controlled_clock(&mut spool);
    spool.start_segment(&[branch()], &frame(10, true)).unwrap();
    assert_eq!(shared.borrow().syncs, 1);
    let flushes = shared.borrow().flushes;
    spool.append(&Record::Frame(frame(11, false))).unwrap();
    assert_eq!(shared.borrow().flushes, flushes + 1);
    spool.sync_if_due().unwrap();
    assert_eq!(shared.borrow().syncs, 1);
    advance(&clock, SYNC_INTERVAL - Duration::from_nanos(1));
    spool.sync_if_due().unwrap();
    assert_eq!(shared.borrow().syncs, 1, "a sync is not due before its interval");
    advance(&clock, Duration::from_nanos(1));
    spool.sync_if_due().unwrap();
    assert_eq!(shared.borrow().syncs, 2, "a sync is due at its interval");
    spool
        .append(&Record::Control(ControlRecord::Detach {
            session: 3,
            last_hour: Some(11),
        }))
        .unwrap();
    assert_eq!(shared.borrow().syncs, 3);
    assert!(spool.append(&Record::Frame(frame(12, false))).is_err());
}

#[test]
fn periodic_syncs_reuse_one_persistent_durability_thread() {
    let directory = std::env::temp_dir().join(format!(
        "battleplan-durability-worker-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    let mut spool = SegmentedSpool::new(FileBackend::new(&directory).unwrap());
    let clock = controlled_clock(&mut spool);
    spool.start_segment(&[branch()], &frame(10, true)).unwrap();
    for hour in 11..14 {
        advance(&clock, SYNC_INTERVAL);
        spool.append(&Record::Frame(frame(hour, false))).unwrap();
        let requested = spool.background_sync_work().unwrap().requested;
        assert_eq!(requested, u64::from((hour - 10) as u32));
        spool.sink.as_mut().unwrap().finish_pending().unwrap();
        assert_eq!(spool.background_sync_work().unwrap().completed, requested);
    }
    let work = spool.background_sync_work().unwrap();
    assert_eq!((work.requested, work.completed, work.threads), (3, 3, 1));
    assert!(!work.pending);
    advance(&clock, SYNC_INTERVAL);
    spool.append(&Record::Frame(frame(14, false))).unwrap();
    spool.finish_segment().unwrap();
    let bytes = fs::read(directory.join("spool.0000.h4s")).unwrap();
    let recovered = recover_prefix(&bytes);
    assert!(recovered.damage.is_none());
    assert!(matches!(
        recovered.records.last(),
        Some(Record::Frame(Frame { hour: 14, .. }))
    ));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn occupation_keyframe_then_delta_spool_preserves_nested_packets_and_carry() {
    use replay_data::rows::*;
    use replay_frame::frame::{DecodedFrame, FrameDecoder, FrameEncoder, TableView};
    let table = TableId::StateResistance;
    let hover = OccupationHover {
        exile_host: 3,
        legitimacy: 50_000,
        flow_from: 8,
        flow: 20_000,
        gain_ratio: 99_000,
        added: vec![OccupationAddedTarget {
            value: -100_000,
            controller: 1,
            occupied: 2,
            localisation: b"fixture_target".to_vec(),
        }],
        compliance_effects: vec![OccupationEffect {
            key: 7,
            value: 5_000,
        }],
        compliance_extra: OccupationEffectExtra {
            child_sources: 1,
            custom_tooltips: 0,
        },
        ..Default::default()
    };
    let mut row = StateResistanceRow {
        state: 9,
        country: 2,
        level: 4_106_000,
        speed: -10,
        compliance: 901_000,
        compliance_speed: 8,
        active: 1,
        hover: Recorded::Value(Some(hover)),
        map: Recorded::Value(Some(OccupationMap {
            stored_target: Some(5_000_000),
            core_country: Some(3),
            resistance: vec![
                b"fixture_resistance".to_vec(),
                b"fixture_resistance".to_vec(),
            ],
            compliance: vec![b"fixture_compliance".to_vec()],
        })),
        modifier_sources: Recorded::Value(vec![OccupationModifierSource {
            section: 2,
            source: 0,
            key: engine_occupation::cresistance::hover::DECAY,
            total: Some(123),
            nodes: vec![
                ModifierBreakdownNode {
                    children: 1,
                    value: 123,
                    name: Vec::new(),
                },
                ModifierBreakdownNode {
                    children: 0,
                    value: 123,
                    name: b"fixture source".to_vec(),
                },
            ],
            failure: Vec::new(),
        }]),
    };
    let inactive = StateResistanceRow {
        state: 10,
        hover: Recorded::Value(None),
        map: Recorded::Value(None),
        modifier_sources: Recorded::Value(Vec::new()),
        ..Default::default()
    };
    let mut encoder = FrameEncoder::new();
    let backend = MemoryBackend::default();
    let shared = backend.0.clone();
    let mut spool = SegmentedSpool::new(backend);
    let mut frames = Vec::new();
    let mut expected = Vec::new();
    for hour in 10..14 {
        if hour == 11 {
            row.level += 1;
        }
        if hour == 12 {
            let Recorded::Value(Some(hover)) = &mut row.hover else {
                unreachable!()
            };
            hover.compliance_effects[0].value = 7_500;
            hover.added[0].localisation = b"changed_target".to_vec();
            let Recorded::Value(sources) = &mut row.modifier_sources else {
                unreachable!()
            };
            sources[0].total = None;
            sources[0].failure = b"fixture root total unavailable".to_vec();
            sources[0].nodes.clear();
            let Recorded::Value(Some(map)) = &mut row.map else {
                unreachable!()
            };
            map.stored_target = None;
            map.core_country = Some(4);
            map.resistance.remove(0);
            map.compliance.push(b"fixture_added_compliance".to_vec());
        }
        let rows = TableRows::StateResistance(vec![row.clone(), inactive.clone()]);
        let mut source = DecodedFrame {
            branch: 7,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Unavailable {
                    reason: match replay_frame::frame::current_absence(TableId::ALL[index]) {
                        Some(TableView::Unavailable { reason }) => reason,
                        _ => "unused".into(),
                    },
                }
                .into()
            }),
        };
        if hour != 13 {
            source.metrics.table_ns[table.index()] = 1;
        }
        source.tables[table.index()] = TableView::Complete(rows.clone()).into();
        let mut prepared = encoder.prepare_bytes(source.into(), hour == 10).unwrap();
        let encoded = prepared.take_frame();
        frames.push(encoded.decode().unwrap());
        if hour == 10 {
            spool.start_encoded_segment(&[branch()], encoded).unwrap();
        } else {
            spool.append_encoded_frame(encoded).unwrap();
        }
        encoder.commit(prepared).unwrap();
        expected.push(rows);
    }
    assert!(matches!(
        frames[0].tables[table.index()],
        TableUpdate::Snapshot(_)
    ));
    for (frame, mask) in [
        (&frames[1], 1 << 1),
        (&frames[2], (1 << 6) | (1 << 7) | (1 << 8)),
    ] {
        let TableUpdate::Delta {
            changed, deleted, ..
        } = &frame.tables[table.index()]
        else {
            panic!("changed occupation row must be a delta");
        };
        assert!(deleted.is_empty());
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].mask, mask);
    }
    assert!(matches!(
        frames[3].tables[table.index()],
        TableUpdate::Unchanged { .. }
    ));
    spool.finish_segment().unwrap();
    let recovered = recover_prefix(&shared.borrow().files[&0]);
    assert!(recovered.damage.is_none(), "{:?}", recovered.damage);
    assert_eq!(recovered.records.len(), 5);
    let mut decoder = FrameDecoder::new();
    let mut count = 0;
    for record in recovered.records {
        if let Record::Frame(frame) = record {
            let output = decoder.apply(&frame).unwrap();
            assert_eq!(
                output.tables[table.index()],
                TableView::Complete(expected[count].clone())
            );
            count += 1;
        }
    }
    assert_eq!(count, 4);
}

use replay_data::FORMAT_VERSION;

#[test]
fn records_decoded_on_several_threads_keep_the_sequential_prefix_and_first_damage() {
    let mut records = vec![branch()];
    records.extend((10..40).map(|hour| Record::Frame(frame(hour, hour == 10))));
    let encoded: Vec<Vec<u8>> = records
        .iter()
        .map(|record| encode_record(record).unwrap())
        .collect();
    let whole: Vec<u8> = encoded.concat();
    let recover = |bytes: &[u8], threads| {
        recover_records(
            io::Cursor::new(bytes),
            bytes.len(),
            |record, _| record,
            |_| Ok(()),
            false,
            threads,
        )
        .unwrap()
    };
    let mut damaged = vec![whole.clone()];
    let mut start = 0;
    for (at, record) in encoded.iter().enumerate() {
        if at == 7 {
            let mut bytes = whole.clone();
            bytes[start + RECORD_HEADER_BYTES + 3] ^= 1;
            damaged.push(bytes);
        }
        if at == 19 {
            let mut bytes = whole[..start].to_vec();
            bytes.extend_from_slice(&encoded[at - 1]);
            bytes.extend_from_slice(&whole[start..]);
            damaged.push(bytes);
            damaged.push(whole[..start + record.len() / 2].to_vec());
        }
        start += record.len();
    }
    for bytes in &damaged {
        let sequential = recover(bytes, 1);
        assert_eq!(recover_prefix(bytes).records, sequential.records);
        for threads in [2, 3, 8] {
            let parallel = recover(bytes, threads);
            assert_eq!(parallel.records, sequential.records);
            assert_eq!(parallel.valid_bytes, sequential.valid_bytes);
            assert_eq!(parallel.damage, sequential.damage);
            assert_eq!(parallel.format, sequential.format);
        }
    }
    assert_eq!(recover(&whole, 4).records, records);
    assert!(recover(&damaged[1], 4).damage.unwrap().contains("CRC"));
}

#[test]
fn recovered_frames_keep_their_offsets_and_are_read_again_whole() {
    let bytes = valid_segment();
    let kept = recover_reader(
        io::Cursor::new(&bytes),
        bytes.len(),
        Recovered::keep,
        |_| Ok(()),
    )
    .unwrap();
    let whole = recover_prefix(&bytes);
    assert_eq!(kept.valid_bytes, whole.valid_bytes);
    let mut cursor = io::Cursor::new(&bytes);
    for (kept, whole) in kept.records.iter().zip(&whole.records) {
        match (kept, whole) {
            (Recovered::Frame(metadata), Record::Frame(frame)) => {
                assert_eq!(
                    &read_frame_at(&mut cursor, metadata.placement, bytes.len()).unwrap(),
                    frame
                );
                assert_eq!(metadata.campaign, frame.tables[TableId::Campaign.index()]);
                let only = metadata.frame().unwrap();
                assert_eq!(
                    only.tables[TableId::Clock.index()],
                    frame.tables[TableId::Clock.index()]
                );
            }
            (kept, whole) => {
                assert_eq!(*kept, Recovered::keep(whole.clone(), Placement::default()))
            }
        }
    }
    let at = |offset| Placement {
        offset,
        ..Default::default()
    };
    assert!(read_frame_at(&mut cursor, at(0), bytes.len()).is_err());
    assert!(read_frame_at(&mut cursor, at(bytes.len()), bytes.len()).is_err());
}

#[test]
fn a_frame_record_changed_since_recovery_is_refused() {
    let records = [
        branch(),
        Record::Frame(frame(10, true)),
        Record::Frame(frame(11, false)),
        Record::Frame(frame(12, false)),
    ];
    let bytes: Vec<u8> = records
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect();
    let kept = recover_reader(
        io::Cursor::new(&bytes),
        bytes.len(),
        Recovered::keep,
        |_| Ok(()),
    )
    .unwrap();
    let Recovered::Frame(metadata) = &kept.records[2] else {
        unreachable!()
    };
    let read =
        |bytes: &[u8]| read_frame_at(&mut io::Cursor::new(bytes), metadata.placement, bytes.len());
    assert_eq!(read(&bytes).unwrap().hour, 11);
    let mut changed = frame(11, false);
    changed.tables[1] = TableUpdate::Unavailable {
        reason: "changed".into(),
    };
    let mut replacing = records.to_vec();
    replacing[2] = Record::Frame(changed);
    let replaced: Vec<u8> = replacing
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect();
    let error = read(&replaced).unwrap_err().to_string();
    assert!(error.contains("changed since it was recovered"), "{error}");
    let mut mutated = bytes.clone();
    mutated[metadata.placement.offset + RECORD_HEADER_BYTES + 5] ^= 1;
    assert!(read(&mutated).is_err());
    let truncated = &bytes[..metadata.placement.offset + RECORD_HEADER_BYTES + 3];
    assert!(read(truncated).is_err());
}

#[test]
fn a_tail_grown_in_steps_finishes_as_the_whole_recovery() {
    let mut records = vec![branch()];
    records.extend((10..30).map(|hour| Record::Frame(frame(hour, hour == 10))));
    let mut bytes: Vec<u8> = records
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect();
    let whole = bytes.len();
    let mut damaged = bytes.clone();
    damaged.truncate(whole - 9);
    let mut corrupt = bytes.clone();
    corrupt[whole / 2] ^= 0x40;
    for bytes in [&mut bytes, &mut damaged, &mut corrupt] {
        let full = recover_records(
            io::Cursor::new(&bytes[..]),
            bytes.len(),
            Recovered::keep,
            |_| Ok(()),
            true,
            1,
        )
        .unwrap();
        for step in [1, 37, 1_000, 9_999] {
            let mut tail = Tail::default();
            let mut length = 0;
            while length < bytes.len() {
                length = (length + step).min(bytes.len());
                tail.grow(&mut io::Cursor::new(&bytes[..]), length, Recovered::keep)
                    .unwrap();
            }
            assert!(tail.valid_bytes() <= full.valid_bytes);
            let mut observed = 0;
            let finished = tail
                .finish(
                    &mut io::Cursor::new(&bytes[..]),
                    bytes.len(),
                    Recovered::keep,
                    |_| {
                        observed += 1;
                        Ok(())
                    },
                )
                .unwrap()
                .unwrap();
            assert_eq!(finished.records, full.records, "{step}");
            assert_eq!(finished.valid_bytes, full.valid_bytes);
            assert_eq!(finished.damage, full.damage);
            assert_eq!(finished.format, full.format);
            assert_eq!(observed, full.records.len());
        }
    }
    let mut tail = Tail::default();
    tail.grow(
        &mut io::Cursor::new(&bytes[..]),
        bytes.len(),
        Recovered::keep,
    )
    .unwrap();
    assert!(
        tail.finish(
            &mut io::Cursor::new(&bytes[..]),
            10,
            Recovered::keep,
            |_| Ok(())
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn a_tail_of_a_segment_replaced_since_it_was_read_is_not_continued() {
    let records = [
        branch(),
        Record::Frame(frame(10, true)),
        Record::Frame(frame(11, false)),
    ];
    let bytes: Vec<u8> = records
        .iter()
        .flat_map(|record| encode_record(record).unwrap())
        .collect();
    let mut tail = Tail::default();
    tail.grow(&mut io::Cursor::new(&bytes), bytes.len(), Recovered::keep)
        .unwrap();
    let mut changed = frame(11, false);
    changed.tables[1] = TableUpdate::Unavailable {
        reason: "changed".into(),
    };
    let replaced: Vec<u8> = [
        records[0].clone(),
        records[1].clone(),
        Record::Frame(changed),
    ]
    .iter()
    .flat_map(|record| encode_record(record).unwrap())
    .collect();
    let finish = |tail: Tail<Recovered>, bytes: &[u8]| {
        tail.finish(
            &mut io::Cursor::new(bytes),
            bytes.len(),
            Recovered::keep,
            |_| Ok(()),
        )
        .unwrap()
    };
    let same = || {
        let mut tail = Tail::default();
        tail.grow(&mut io::Cursor::new(&bytes), bytes.len(), Recovered::keep)
            .unwrap();
        tail
    };
    assert!(finish(same(), &bytes).is_some());
    assert!(finish(tail, &replaced).is_none());
    assert!(finish(same(), &bytes[..bytes.len() - 1]).is_none());
}

#[test]
fn a_closed_context_only_failure_segment_keeps_explicit_gaps_without_a_fake_keyframe() {
    let bytes = [
        branch(),
        Record::Gap(Gap {
            branch: 7,
            first_hour: 10,
            last_hour: 12,
            reason: "spool write failed".into(),
        }),
    ]
    .iter()
    .flat_map(|record| encode_record(record).unwrap())
    .collect::<Vec<_>>();
    let recovered = recover_prefix(&bytes);
    assert!(recovered.damage.is_none());
    assert_eq!(recovered.records.len(), 2);
    assert!(
        !recovered
            .records
            .iter()
            .any(|record| matches!(record, Record::Frame(_)))
    );
    assert!(
        recover_prefix(&encode_record(&branch()).unwrap())
            .damage
            .is_some()
    );
}

fn context_tail() -> [Record; 4] {
    [
        session(),
        branch(),
        Record::Control(ControlRecord::ClockWitness {
            session: 3,
            qpc: 100,
            hour: 10,
            load_signal: Some(19),
        }),
        Record::Control(ControlRecord::ClockWitness {
            session: 3,
            qpc: 200,
            hour: 11,
            load_signal: Some(19),
        }),
    ]
}

#[test]
fn complete_session_context_tails_recover_without_fabricating_a_keyframe() {
    let records = context_tail();
    let encoded: Vec<_> = records
        .iter()
        .map(|record| encode_record(record).unwrap())
        .collect();
    for count in 1..=records.len() {
        let bytes = encoded[..count].concat();
        for threads in [1, 4] {
            let recovered = recover_records(
                io::Cursor::new(&bytes),
                bytes.len(),
                |record, _| record,
                |_| Ok(()),
                true,
                threads,
            )
            .unwrap();
            assert_eq!(recovered.records, records[..count]);
            assert_eq!(recovered.valid_bytes, bytes.len());
            assert!(recovered.damage.is_none(), "{:?}", recovered.damage);
        }
        let mut reader = RecordReader::new(io::Cursor::new(&bytes), bytes.len());
        for record in &records[..count] {
            assert_eq!(reader.next_record().unwrap().as_ref(), Some(record));
        }
        assert_eq!(reader.next_record().unwrap(), None);
        let mut tail = Tail::default();
        tail.grow(&mut io::Cursor::new(&bytes), bytes.len(), |record, _| {
            record
        })
        .unwrap();
        let recovered = tail
            .finish(
                &mut io::Cursor::new(&bytes),
                bytes.len(),
                |record, _| record,
                |_| Ok(()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(recovered.records, records[..count]);
        assert_eq!(recovered.valid_bytes, bytes.len());
        assert!(recovered.damage.is_none());
    }
}

#[test]
fn session_context_tails_keep_torn_envelopes_and_missing_keyframes_damaged() {
    let records = context_tail();
    let encoded: Vec<_> = records
        .iter()
        .map(|record| encode_record(record).unwrap())
        .collect();
    let bytes = encoded.concat();
    let ends: Vec<_> = encoded
        .iter()
        .scan(0, |end, record| {
            *end += record.len();
            Some(*end)
        })
        .collect();
    for cut in 0..=bytes.len() {
        let recovered = recover_prefix(&bytes[..cut]);
        let count = ends.iter().filter(|&&end| end <= cut).count();
        let valid_bytes = count.checked_sub(1).map_or(0, |at| ends[at]);
        assert_eq!(recovered.records, records[..count], "cut {cut}");
        assert_eq!(recovered.valid_bytes, valid_bytes, "cut {cut}");
        let torn = cut != 0 && !ends.contains(&cut);
        assert_eq!(recovered.damage.is_some(), torn, "cut {cut}");
        if torn {
            let detail = if cut - valid_bytes < RECORD_HEADER_BYTES {
                "truncated record header"
            } else {
                "truncated record payload"
            };
            assert_eq!(
                recovered.damage.unwrap(),
                format!("segment byte {valid_bytes}: {detail}")
            );
        }
    }
    let end = ends[1];
    let mut corrupt = bytes.clone();
    corrupt[end + RECORD_HEADER_BYTES] ^= 1;
    assert_eq!(
        recover_prefix(&corrupt).damage.unwrap(),
        format!("segment byte {end}: record CRC mismatch")
    );
    let mut torn_frame = bytes.clone();
    let encoded_frame = encode_record(&Record::Frame(frame(10, true))).unwrap();
    torn_frame.extend_from_slice(&encoded_frame[..encoded_frame.len() - 1]);
    assert_eq!(
        recover_prefix(&torn_frame).damage.unwrap(),
        format!("segment byte {}: truncated record payload", bytes.len())
    );
    let mut dependent = bytes.clone();
    dependent.extend(encode_record(&Record::Frame(frame(10, false))).unwrap());
    assert_eq!(
        recover_prefix(&dependent).damage.unwrap(),
        format!(
            "segment byte {}: branch or segment does not start with a keyframe",
            bytes.len()
        )
    );
    let mut frame_bearing = bytes;
    frame_bearing.extend(encoded_frame);
    let Record::BranchStart(mut next) = branch() else {
        unreachable!()
    };
    next.id = 8;
    next.reason = BranchReason::LoadSignal;
    frame_bearing.extend(encode_record(&Record::BranchStart(next)).unwrap());
    assert_eq!(
        recover_prefix(&frame_bearing).damage.as_deref(),
        Some("segment ended before initial keyframe")
    );
}
