use super::*;
thread_local! {
    static DENY_ALLOCATIONS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn allocations_denied() -> bool {
    DENY_ALLOCATIONS.with(|deny| deny.get())
}

fn with_allocations_denied<T>(read: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            DENY_ALLOCATIONS.set(false);
            engine_allocation::set_stop_hook(None);
        }
    }
    // These fault tests run in isolated child processes because the hook is global.
    engine_allocation::set_stop_hook(Some(allocations_denied));
    DENY_ALLOCATIONS.set(true);
    let _reset = Reset;
    read()
}

#[test]
fn declared_lengths_do_not_abort_on_allocation_failure() {
    const CHILD: &str = "battle-plan_HOSTILE_ALLOCATION_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
        match mode.as_str() {
            "reconstruct" => {
                let mut frame = replay.read_block(&replay.index[0]).unwrap().remove(0);
                frame.tables[TableId::Campaign.index()] =
                    TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                        id: vec![b'x'; 2 * 1024 * 1024],
                    }]));
                let mut decoder = FrameDecoder::new();
                let result = with_allocations_denied(|| decoder.advance(&frame).map(|_| ()));
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
            }
            "whole" | "stream" | "growth" => {
                let mut captured = records(1, &[1]);
                if mode == "growth" {
                    let Record::Frame(frame) = &mut captured[2] else {
                        unreachable!()
                    };
                    frame.tables[TableId::Countries.index()] =
                        TableUpdate::Snapshot(TableRows::Countries(vec![
                            crate::rows::CountryRow {
                                id: 1,
                                canonical_slot: 1,
                                tag: vec![b'x'; 2 * 1024 * 1024],
                                ..Default::default()
                            },
                        ]));
                }
                let hostile = Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap();
                let result = with_allocations_denied(|| {
                    if mode == "whole" {
                        hostile.read_block(&hostile.index[0]).map(|_| ())
                    } else {
                        hostile.visit_block(&hostile.index[0], None, |_| Ok(true))
                    }
                });
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
            }
            "trailer" => {
                let mut prefix = replay.bytes()[..replay.trailer_start].to_vec();
                prefix.extend_from_slice(TRAILER_MAGIC);
                // The count is backed by bytes, but cannot allocate its owning index here.
                let count = 100_000_u32;
                for _ in 0..5 {
                    prefix.extend_from_slice(&0_u32.to_le_bytes());
                }
                // No earlier trailer.
                prefix.extend_from_slice(&[0; 16]);
                prefix.extend_from_slice(&count.to_le_bytes());
                prefix.resize(prefix.len() + count as usize * 40, 0);
                let trailer = &prefix[replay.trailer_start..];
                let checksum = crc32fast::hash(trailer);
                let length = trailer.len() as u64;
                prefix.extend_from_slice(FOOTER_MAGIC);
                prefix.extend_from_slice(&(replay.trailer_start as u64).to_le_bytes());
                prefix.extend_from_slice(&length.to_le_bytes());
                prefix.extend_from_slice(&checksum.to_le_bytes());
                let error = with_allocations_denied(|| Replay::open(prefix))
                    .err()
                    .unwrap();
                assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
            }
            _ => panic!("unknown synthetic allocation mode"),
        }
        return;
    }
    for mode in ["whole", "stream", "growth", "trailer", "reconstruct"] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "container::tests::security_tests::declared_lengths_do_not_abort_on_allocation_failure",
                "--nocapture",
            ])
            .env(CHILD, mode)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > std::time::Duration::from_secs(120) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("synthetic allocation child hung: {mode}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn frame_count_and_hours_are_checked_before_payload_decoding() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let hostile = edited_directory(&replay, 0, |directory| {
        directory[0] = 2;
    });
    let block = &hostile.index[0];
    assert_eq!(
        hostile.read_block(block).unwrap_err().to_string(),
        "grouped frame count disagrees with its hour range"
    );
    assert_eq!(
        hostile
            .visit_block(block, None, |_| Ok(true))
            .unwrap_err()
            .to_string(),
        "grouped frame count disagrees with its hour range"
    );
    let mut order = BlockOrder::new(&replay.index[0]);
    let mut frame = replay.read_block(&replay.index[0]).unwrap().remove(0);
    frame.hour = 2;
    assert!(order.next(&frame).is_err());
}

#[test]
fn partial_reader_checks_actual_stream_length_before_trailer_allocation() {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let mut cursor = std::io::Cursor::new(replay.bytes());
    let error = Replay::read_partial_from(&mut cursor, u64::MAX).unwrap_err();
    assert_eq!(error.to_string(), "replay length differs from stream");
}

#[test]
fn cancelled_or_exhausted_visitors_keep_their_error() {
    let grouped = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    for selected in [None, Some(&[TableId::Clock][..])] {
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::OutOfMemory] {
            let replay = Replay::open(grouped.bytes().to_vec()).unwrap();
            let error = replay
                .visit_block(&replay.index[0], selected, |_| Err(io::Error::from(kind)))
                .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert!(!replay.is_verified(&replay.index[0]));
        }
    }
}

#[test]
fn selected_seek_preserves_allocation_errors_from_wire_and_reconstruction() {
    const CHILD: &str = "BATTLE_PLAN_SELECTED_SEEK_ALLOCATION_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let mut captured = records(1, &[1]);
        let frame = captured
            .iter_mut()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        frame.tables[TableId::Countries.index()] =
            TableUpdate::Snapshot(TableRows::Countries(vec![crate::rows::CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: vec![b'x'; 2 * 1024 * 1024],
                ..Default::default()
            }]));
        let recording = Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap();
        engine_allocation::set_stop_hook(Some(allocations_denied));
        let mut checks = 0;
        if mode == "wire" {
            DENY_ALLOCATIONS.set(true);
        }
        let result = recording.seek_selected_with(1, 1, Some(&[TableId::Countries]), || {
            checks += 1;
            if mode == "reconstruct" && checks == 2 {
                DENY_ALLOCATIONS.set(true);
            }
            Ok(())
        });
        DENY_ALLOCATIONS.set(false);
        engine_allocation::set_stop_hook(None);
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
        assert!(matches!(
            recording
                .seek_selected(1, 1, Some(&[TableId::Countries]))
                .unwrap(),
            SeekResult::Frame(_)
        ));
        return;
    }
    for mode in ["wire", "reconstruct"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "container::tests::security_tests::selected_seek_preserves_allocation_errors_from_wire_and_reconstruction", "--nocapture"])
            .env(CHILD, mode).output().unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn selected_seek_keeps_cancellation_as_an_error_and_real_damage_as_a_gap() {
    let recording = Replay::from_records(CAMPAIGN.into(), &[records(1, &[1])]).unwrap();
    let mut checks = 0;
    let error = recording
        .seek_selected_with(1, 1, None, || {
            checks += 1;
            if checks == 2 {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let broken = edited_directory(&recording, 0, |directory| {
        directory[0] = 0;
    });
    assert!(matches!(
        broken.seek_selected(1, 1, None).unwrap(),
        SeekResult::Gap { .. }
    ));
    assert!(matches!(
        recording.seek(1, 1).unwrap(),
        SeekResult::Frame(_)
    ));
}
