//! Allocation failures are not invalid prediction heads or unavailable anchors.
use super::*;
use crate::rows::CountryRow;
use std::cell::Cell;

thread_local! {
    static ATTEMPTS: Cell<usize> = const { Cell::new(0) };
    static FAIL_AT: Cell<Option<usize>> = const { Cell::new(None) };
}

fn fail_allocation() -> bool {
    let attempt = ATTEMPTS.get() + 1;
    ATTEMPTS.set(attempt);
    FAIL_AT.get() == Some(attempt)
}

#[test]
fn streaming_allocation_failure_is_fatal_and_releases_its_block_credit() {
    if !child("streaming_allocation_failure_is_fatal_and_releases_its_block_credit") {
        return;
    }
    use crate::container::{ReaderExecution, ReaderSelection, SequentialReader};
    let (replay, _) = super::direct_tests::alias_fixture(Layout::Chosen);
    let replay = std::sync::Arc::new(replay);
    replay.log_section_decodes();
    for fail_at in [1, 8, 32] {
        let mut reader = SequentialReader::new(
            ReaderSelection::new(&TableId::ALL).unwrap(),
            ReaderExecution {
                workers: 1,
                read_ahead: false,
            },
        )
        .unwrap();
        let (error, attempts) = checked(Some(fail_at), || {
            reader.advance(&replay, || Ok(())).err().unwrap()
        });
        assert!(engine_allocation::is_allocation_error(&error));
        assert!(attempts >= fail_at);
        assert_eq!(reader.census().raw_blocks, 0);
        assert_eq!(reader.cursor().next_block, 0);
        assert!(reader.current().is_none());
        assert!(replay.section_decodes().is_empty());
    }
}

fn checked<T>(fail_at: Option<usize>, work: impl FnOnce() -> T) -> (T, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            engine_allocation::set_stop_hook(None);
        }
    }
    ATTEMPTS.set(0);
    FAIL_AT.set(fail_at);
    engine_allocation::set_stop_hook(Some(fail_allocation));
    let reset = Reset;
    let result = crate::codec::with_error_details(work);
    drop(reset);
    (result, ATTEMPTS.get())
}

// Hooks are process-global: isolate them even when the ordinary libtest runner
// executes several tests together. The denial ordinal is local to this thread.
fn child(name: &str) -> bool {
    const CHILD: &str = "BATTLE_PLAN_GROUPED_ALLOCATION_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("container::grouped::allocation_tests::{name}"),
            "--nocapture",
        ])
        .env(CHILD, name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

#[test]
fn shadow_diagnostic_allocation_failures_propagate_at_every_checked_growth() {
    if !child("shadow_diagnostic_allocation_failures_propagate_at_every_checked_growth") {
        return;
    }
    let table = TableId::DivisionMovement;
    let format = crate::FORMAT_VERSION;
    let mut head = Vec::new();
    put_varint(&mut head, zigzag(-11));
    let path = [1_u32.to_le_bytes(), 99_u32.to_le_bytes()].concat();
    let values = [head.as_slice(), path.as_slice()].concat();
    let mask = (1 << 0) | (1 << 8);
    let mut plain = vec![DELTA];
    plain.extend_from_slice(&9_i32.to_le_bytes());
    plain.extend_from_slice(&1_u32.to_le_bytes());
    put_varint(&mut plain, 1);
    plain.extend_from_slice(&u64::to_le_bytes(mask)[..table.mask_width()]);
    plain.extend_from_slice(&values);
    plain.extend_from_slice(&0_u32.to_le_bytes());
    let initial = Shadow::from([(
        1,
        BTreeMap::from([
            (0, 10_u32.to_le_bytes().to_vec()),
            (8, [1_u32.to_le_bytes(), 50_u32.to_le_bytes()].concat()),
        ]),
    )]);
    let expected = Shadow::from([(1, BTreeMap::from([(8, path.clone())]))]);
    for route in 0..4 {
        let apply = |shadow: &mut Shadow| -> io::Result<()> {
            match route {
                0 => advance_shadow(table, format, &plain, shadow),
                1 => {
                    observe_shadow(table, DELTA, 1, 0, &head, shadow)?;
                    observe_shadow(table, DELTA, 1, 8, &path, shadow)
                }
                2 => direct::Delta::new(table, table.field_count(), format, 9, shadow).row(
                    1,
                    mask,
                    values.clone(),
                ),
                _ => direct::Delta::new(table, table.field_count(), format, 9, shadow).row_spans(
                    1,
                    mask,
                    values.clone(),
                    &[
                        direct::Span {
                            field: 0,
                            end: head.len(),
                            parsed: true,
                        },
                        direct::Span {
                            field: 8,
                            end: values.len(),
                            parsed: false,
                        },
                    ],
                ),
            }
        };
        let mut healthy = initial.clone();
        let (result, attempts) = checked(None, || apply(&mut healthy));
        result.unwrap();
        // 10 - 11 is an ordinary invalid head, not a table refusal. It still
        // removes only the head and continues observing the later list.
        assert_eq!(healthy, expected, "route {route}");
        assert!(attempts > 0, "fixture must allocate the diagnostic");
        for ordinal in 1..=attempts {
            let mut shadow = initial.clone();
            let (result, _) = checked(Some(ordinal), || apply(&mut shadow));
            assert_eq!(
                result.unwrap_err().kind(),
                io::ErrorKind::OutOfMemory,
                "route {route}, checked allocation {ordinal}"
            );
            if ordinal == 1 {
                // The first allocation builds the head's diagnostic. Neither
                // its old base nor any later field may change after it fails.
                assert_eq!(shadow, initial, "route {route}");
            }
        }
    }
}

struct File(std::path::PathBuf);
impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn same_anchors(actual: &Anchors, expected: &Anchors) {
    assert_eq!(actual.position, expected.position);
    for (actual, expected) in actual
        .snapshots
        .iter()
        .chain(&actual.latest)
        .zip(expected.snapshots.iter().chain(&expected.latest))
    {
        match (actual, expected) {
            (Some(a), Some(b)) => {
                assert_eq!(
                    (a.hour, a.block, a.depth, a.root, a.charge),
                    (b.hour, b.block, b.depth, b.root, b.charge)
                );
                assert_eq!(a.rows, b.rows);
                assert_eq!(a.shadow, b.shadow);
            }
            (None, None) => {}
            _ => panic!("anchor presence differs"),
        }
    }
}

#[test]
fn anchor_seed_allocation_failures_propagate_at_every_checked_growth() {
    if !child("anchor_seed_allocation_failures_propagate_at_every_checked_growth") {
        return;
    }
    let resident = test_recording::build(b"anchor-allocation-fixture", &[1, 721], |_, tables| {
        tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"synthetic".to_vec(),
                ..Default::default()
            }]));
        Metrics::default()
    });
    assert_eq!(resident.index.len(), 2);
    let file = File(std::env::temp_dir().join(format!(
        "grouped-anchor-allocation-{}.battleplan",
        std::process::id()
    )));
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file.0)
        .unwrap()
        .write_all(resident.bytes())
        .unwrap();
    let held = Replay::open_path(&file.0).unwrap();
    let expected = Anchors::seed(&resident).unwrap();
    assert!(expected.latest[TableId::Countries.index()].is_some());
    let (result, attempts) = checked(None, || Anchors::seed(&held));
    same_anchors(&result.unwrap(), &expected);
    assert!(
        attempts > 1,
        "fixture must allocate the last block and seed its tables"
    );
    for ordinal in 1..=attempts {
        let held = Replay::open_path(&file.0).unwrap();
        let (result, _) = checked(Some(ordinal), || Anchors::seed(&held));
        match result {
            Err(error) => assert_eq!(
                error.kind(),
                io::ErrorKind::OutOfMemory,
                "checked allocation {ordinal}"
            ),
            Ok(_) => panic!("checked allocation {ordinal} was swallowed"),
        }
    }
    same_anchors(&Anchors::seed(&held).unwrap(), &expected);

    // Damage still leaves no candidates. If creating that diagnostic fails,
    // however, the allocation failure must escape directory admission too.
    let mut damaged = resident.bytes().to_vec();
    damaged[resident.index.last().unwrap().offset as usize + 4] ^= 1;
    std::fs::write(&file.0, damaged).unwrap();
    let held = Replay::open_path(&file.0).unwrap();
    let (result, attempts) = checked(None, || Anchors::seed(&held));
    let empty = result.unwrap();
    assert!(
        empty
            .snapshots
            .iter()
            .chain(&empty.latest)
            .all(Option::is_none)
    );
    assert!(
        attempts > 1,
        "fixture must reach a checked damage diagnostic"
    );
    for ordinal in 1..=attempts {
        let held = Replay::open_path(&file.0).unwrap();
        let (result, _) = checked(Some(ordinal), || Anchors::seed(&held));
        match result {
            Err(error) => assert_eq!(
                error.kind(),
                io::ErrorKind::OutOfMemory,
                "damaged directory checked allocation {ordinal}"
            ),
            Ok(_) => panic!("damaged directory allocation {ordinal} was swallowed"),
        }
    }
}

#[test]
fn anchor_candidate_preserves_allocation_identity_and_ordinary_refusals() {
    for code in [8, 14, 1450, 1451, 1452, 1453, 1454, 1455, 1816] {
        let error = anchor_candidate::<()>(Err(io::Error::from_raw_os_error(code))).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(code));
    }
    let error = anchor_candidate::<()>(Err(io::ErrorKind::OutOfMemory.into())).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    for kind in [
        io::ErrorKind::InvalidData,
        io::ErrorKind::NotFound,
        io::ErrorKind::UnexpectedEof,
    ] {
        assert!(anchor_candidate::<()>(Err(kind.into())).unwrap().is_none());
    }
    assert_eq!(anchor_candidate(Ok(7)).unwrap(), Some(7));
}

#[test]
fn rejoining_chunk_columns_propagates_allocation_failures() {
    if !child("rejoining_chunk_columns_propagates_allocation_failures") {
        return;
    }
    let columns = [b"synthetic column".as_slice(); FIELDS + 1];
    let raw = chunked::frame(&columns).unwrap();
    let (result, attempts) = checked(None, || chunked::join([Ok(raw.clone())]));
    assert_eq!(result.unwrap(), raw);
    assert!(attempts >= columns.len() + 1);
    for ordinal in 1..=attempts {
        let (result, _) = checked(Some(ordinal), || chunked::join([Ok(raw.clone())]));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::OutOfMemory);
    }
}
