//! Compare grouped reads with independent standalone resolution across prefixes and failures.
use super::super::direct_tests::{LAYOUTS, alias_fixture};
use super::super::{
    BlockIndex, Check, Context, Directory, HEADER, Layout, Parallel, Present, Replay, read_header,
    table_updates,
};
use super::{Key, private, rebuild, same};
use crate::{frame::recorded::Updates, rows::TableId};
use engine_allocation as allocation;
use private::hooks;
use std::{io, sync::OnceLock};

/// A block's directory and header, parsed once for any number of reads.
struct Block {
    bytes: Vec<u8>,
    directory: Directory,
    present: Present,
    hours: Vec<crate::Hour>,
    entries: Vec<u8>,
}

impl Block {
    fn new(replay: &Replay, index: &BlockIndex) -> Self {
        let bytes = replay.block_bytes(index).unwrap().into_owned();
        let directory = Directory::parse(&bytes, index).unwrap();
        let present = Present::new(replay.format.version(), replay.capture_mode);
        let header = directory.raw(&bytes, HEADER).unwrap().unwrap();
        let (hours, entries) = read_header(&header, index, directory.frames, &present).unwrap();
        Self {
            bytes,
            directory,
            present,
            hours,
            entries,
        }
    }

    fn read(
        &self,
        replay: &Replay,
        index: &BlockIndex,
        selected: Option<&[TableId]>,
        through: Option<usize>,
        old: bool,
        tables: &[TableId],
    ) -> Vec<io::Result<Updates>> {
        hooks::with(|hooks| hooks.group_old = Some(old));
        let through = through.unwrap_or(self.hours.len()).min(self.hours.len());
        let memo: Vec<_> = TableId::ALL.iter().map(|_| OnceLock::new()).collect();
        let context = Context {
            replay,
            index,
            position: replay.block_position(index),
            block: &self.bytes,
            directory: &self.directory,
            format: replay.format,
            present: &self.present,
            hours: &self.hours[..through],
            entries: &self.entries,
            selected,
            complete: through == self.hours.len(),
            memo: &memo,
            group: &OnceLock::new(),
        };
        tables
            .iter()
            .map(|&table| table_updates(&context, table, &mut Check::disabled()))
            .collect()
    }
}

/// Every table's result, in table order, from one read.
fn results(
    replay: &Replay,
    index: &BlockIndex,
    selected: Option<&[TableId]>,
    through: Option<usize>,
    old: bool,
) -> Vec<io::Result<Updates>> {
    Block::new(replay, index).read(replay, index, selected, through, old, &TableId::ALL)
}

/// Group results against each table resolved alone, table by table.
fn compare(
    replay: &Replay,
    index: &BlockIndex,
    selected: Option<&[TableId]>,
    through: Option<usize>,
    what: &str,
) {
    let grouped = results(replay, index, selected, through, false);
    let alone = results(replay, index, selected, through, true);
    for ((table, grouped), alone) in TableId::ALL.iter().zip(&grouped).zip(&alone) {
        same(*table, alone, grouped, what);
    }
}

fn selections() -> [Option<Vec<TableId>>; 6] {
    [
        None,
        Some(private::TABLES.to_vec()),
        Some(vec![TableId::Divisions]),
        Some(vec![TableId::LandCombatDivisions]),
        Some(vec![TableId::LandCombatDivisionModifiers]),
        Some(vec![TableId::DivisionMovement, TableId::Divisions]),
    ]
}

/// Verify standalone equality across layouts, selections, prefixes and independently warmed caches.
#[test]
fn group_reads_resolve_each_member_as_its_own_read_alone() {
    hooks::with(|hooks| hooks.verify_group = true);
    let warm: [&[TableId]; 3] = [
        private::TABLES,
        &[TableId::DivisionStatus],
        &[TableId::Divisions],
    ];
    for layout in LAYOUTS {
        let (replay, _) = alias_fixture(layout);
        for index in &replay.index {
            for selected in selections() {
                for through in [Some(1), Some(3), None] {
                    let what = format!("{layout:?} {selected:?} through {through:?}");
                    compare(&replay, index, selected.as_deref(), through, &what);
                }
                for warm in warm {
                    let read = |old| {
                        replay.cache_decoded_sections(false);
                        replay.cache_decoded_sections(true);
                        results(&replay, index, Some(warm), None, true);
                        results(&replay, index, selected.as_deref(), None, old)
                    };
                    let (grouped, alone) = (read(false), read(true));
                    for ((table, grouped), alone) in TableId::ALL.iter().zip(&grouped).zip(&alone) {
                        let what = format!("{layout:?} {selected:?} after {warm:?}");
                        same(*table, alone, grouped, &what);
                    }
                }
                replay.cache_decoded_sections(false);
            }
        }
    }
}

/// A narrow grouped read must decode exactly the sources its standalone read needs.
#[test]
fn a_narrow_selection_decodes_only_what_its_read_alone_decodes() {
    let (replay, _) = alias_fixture(Layout::Chosen);
    let index = &replay.index[0];
    for selected in selections().into_iter().flatten() {
        let decodes = |old| {
            let (_, mut decodes) = crate::container::test_work::table_decodes(|| {
                results(&replay, index, Some(selected.as_slice()), None, old)
            });
            decodes.sort_unstable_by_key(|&(branch, first, table, hours)| {
                (branch, first, table.index(), hours)
            });
            decodes
        };
        let grouped = decodes(false);
        assert_eq!(grouped, decodes(true), "{selected:?}");
        if selected == [TableId::Divisions] {
            let tables: Vec<_> = grouped.iter().map(|&(_, _, table, _)| table).collect();
            assert!(tables.contains(&TableId::DivisionStatus), "{tables:?}");
            assert!(!tables.contains(&TableId::DivisionMovement), "{tables:?}");
            assert!(
                !tables.contains(&TableId::LandCombatDivisions),
                "{tables:?}"
            );
        }
    }
}

/// Compare exact standalone errors across raw, source and per-hour failure injection.
#[test]
fn injected_failures_reach_each_member_as_its_read_alone_reports_them() {
    use TableId::{DivisionPanelStats, DivisionStatus, Divisions, LandCombatDivisions};
    let (replay, _) = alias_fixture(Layout::Chosen);
    let index = &replay.index[0];
    let last = Directory::parse(&replay.block_bytes(index).unwrap(), index)
        .unwrap()
        .frames
        - 1;
    let mid = last / 2;
    type Injected = (TableId, usize, bool);
    let cases: [(&[TableId], &[Injected]); 10] = [
        (&[DivisionStatus], &[]),
        (&[DivisionPanelStats], &[]),
        (&[LandCombatDivisions], &[]),
        (&[], &[(DivisionStatus, 0, false)]),
        (&[], &[(DivisionStatus, 0, true)]),
        (&[], &[(DivisionStatus, mid, false)]),
        (&[], &[(DivisionStatus, last, true)]),
        (&[], &[(Divisions, 0, false)]),
        (&[], &[(Divisions, 0, true)]),
        (&[], &[(LandCombatDivisions, mid, true)]),
    ];
    for (raw, hours) in cases {
        for form in [
            hooks::Failure::Text,
            hooks::Failure::Bare,
            hooks::Failure::Os(8),
        ] {
            hooks::with(|hooks| {
                hooks.fail_raw = raw.iter().map(|&table| (table, form)).collect();
                hooks.fail = hours
                    .iter()
                    .map(|&(table, at, after)| (table, at, after, form))
                    .collect();
            });
            for selected in selections() {
                compare(
                    &replay,
                    index,
                    selected.as_deref(),
                    None,
                    &format!("raw {raw:?} hours {hours:?} form {form:?} {selected:?}"),
                );
            }
        }
    }
}

#[test]
fn competing_failures_keep_the_standalone_source_loading_winner() {
    use TableId::{
        DivisionMovement, DivisionPanelStats, DivisionStatus, Divisions,
        LandCombatDivisionModifiers, LandCombatDivisions,
    };
    struct Case<'a> {
        table: TableId,
        raw: &'a [TableId],
        hours: &'a [(TableId, usize, bool)],
        full: (String, i32),
        prefix: (String, i32),
    }
    let (replay, _) = alias_fixture(Layout::Chosen);
    let index = &replay.index[0];
    let block = Block::new(&replay, index);
    let mid = (block.hours.len() - 1) / 2;
    assert!(
        mid > 1,
        "the short prefix must end before the later source failure"
    );
    // Standalone loads whole source columns in dependencies() order before its after hook.
    let cases = [
        Case {
            table: Divisions,
            raw: &[],
            hours: &[(DivisionStatus, mid, false), (Divisions, 0, true)],
            full: (
                format!("injected alias failure: DivisionStatus hour {mid} before"),
                8,
            ),
            prefix: ("injected alias failure: Divisions hour 0 after".into(), 14),
        },
        Case {
            table: Divisions,
            raw: &[],
            hours: &[(DivisionStatus, 0, true), (Divisions, mid, false)],
            full: (
                "injected alias failure: DivisionStatus hour 0 after".into(),
                8,
            ),
            prefix: (
                "injected alias failure: DivisionStatus hour 0 after".into(),
                8,
            ),
        },
        Case {
            table: LandCombatDivisionModifiers,
            raw: &[],
            hours: &[
                (DivisionStatus, mid, true),
                (LandCombatDivisionModifiers, 1, false),
            ],
            full: (
                format!("injected alias failure: DivisionStatus hour {mid} after"),
                8,
            ),
            prefix: (
                "injected alias failure: LandCombatDivisionModifiers hour 1 before".into(),
                14,
            ),
        },
        Case {
            table: Divisions,
            raw: &[DivisionStatus],
            hours: &[(Divisions, 0, false)],
            full: ("injected alias failure: Divisions hour 0 before".into(), 14),
            prefix: ("injected alias failure: Divisions hour 0 before".into(), 14),
        },
        Case {
            table: Divisions,
            raw: &[DivisionStatus],
            hours: &[(Divisions, 0, true)],
            full: ("injected alias failure: DivisionStatus raw".into(), 8),
            prefix: ("injected alias failure: DivisionStatus raw".into(), 8),
        },
        Case {
            table: LandCombatDivisions,
            raw: &[DivisionStatus, DivisionPanelStats],
            hours: &[],
            full: ("injected alias failure: DivisionPanelStats raw".into(), 14),
            prefix: ("injected alias failure: DivisionPanelStats raw".into(), 14),
        },
        Case {
            table: LandCombatDivisions,
            raw: &[],
            hours: &[(DivisionMovement, 0, true), (DivisionStatus, mid, false)],
            full: (
                format!("injected alias failure: DivisionStatus hour {mid} before"),
                14,
            ),
            prefix: (
                "injected alias failure: DivisionMovement hour 0 after".into(),
                8,
            ),
        },
    ];
    for case in cases {
        for through in [Some(2), None] {
            let (text, code) = if through.is_some() {
                &case.prefix
            } else {
                &case.full
            };
            for os in [false, true] {
                for warmed in [false, true] {
                    let form = |ordinal: usize| {
                        if os {
                            hooks::Failure::Os([8, 14][ordinal])
                        } else {
                            hooks::Failure::Text
                        }
                    };
                    let read = |old| {
                        replay.cache_decoded_sections(false);
                        hooks::with(|hooks| {
                            hooks.fail_raw.clear();
                            hooks.fail.clear();
                        });
                        if warmed {
                            replay.cache_decoded_sections(true);
                            let warm = [TableId::ProvinceUnitOrder, TableId::SimulationFeatures];
                            for result in block.read(&replay, index, Some(&warm), None, true, &warm)
                            {
                                result.unwrap();
                            }
                            for table in warm {
                                assert!(
                                    replay
                                        .decoded_sections
                                        .ready(
                                            Key::new(&replay, index, table),
                                            block.hours.len(),
                                            false,
                                        )
                                        .is_some(),
                                    "{table:?} must be independently warmed"
                                );
                            }
                        }
                        hooks::with(|hooks| {
                            hooks.fail_raw = case
                                .raw
                                .iter()
                                .enumerate()
                                .map(|(at, &table)| (table, form(at)))
                                .collect();
                            hooks.fail = case
                                .hours
                                .iter()
                                .enumerate()
                                .map(|(at, &(table, hour, after))| {
                                    (table, hour, after, form(case.raw.len() + at))
                                })
                                .collect();
                        });
                        block
                            .read(
                                &replay,
                                index,
                                Some(&[case.table]),
                                through,
                                old,
                                &[case.table],
                            )
                            .pop()
                            .expect("the selected member's result")
                    };
                    let grouped = read(false);
                    let alone = read(true);
                    let expected = Err(if os {
                        let error = io::Error::from_raw_os_error(*code);
                        assert!(allocation::is_allocation_error(&error));
                        error
                    } else {
                        crate::codec::invalid(text.clone())
                    });
                    let what = format!("through {through:?} OS {os} warmed {warmed}");
                    same(case.table, &expected, &alone, &what);
                    same(case.table, &expected, &grouped, &what);
                    same(case.table, &alone, &grouped, &what);
                }
            }
        }
    }
    hooks::with(|hooks| {
        hooks.fail_raw.clear();
        hooks.fail.clear();
    });
    replay.cache_decoded_sections(false);
}

/// Error reconstruction preserves kind, text, OS code and allocation classification.
#[test]
fn rebuilt_errors_keep_kind_os_code_and_text() {
    let errors = [
        crate::codec::invalid("a refused row"),
        io::Error::from(io::ErrorKind::OutOfMemory),
        io::Error::from(io::ErrorKind::InvalidData),
        io::Error::from_raw_os_error(8),
        io::Error::from_raw_os_error(14),
        io::Error::from_raw_os_error(2),
        io::Error::other("another failure"),
        crate::codec::invalid("injected alias failure: Divisions hour 0 before"),
    ];
    for error in &errors {
        let rebuilt = rebuild(error);
        assert_eq!(rebuilt.kind(), error.kind(), "{error}");
        assert_eq!(rebuilt.raw_os_error(), error.raw_os_error(), "{error}");
        assert_eq!(rebuilt.to_string(), error.to_string());
        assert_eq!(
            allocation::is_allocation_error(&rebuilt),
            allocation::is_allocation_error(error),
            "{error}"
        );
        if error.get_ref().is_none() && error.raw_os_error().is_none() {
            assert!(rebuilt.get_ref().is_none(), "{error}");
        }
    }
}

/// Deny each counted growth; allocation failure must remain the read's result.
#[test]
fn group_allocation_failures_are_the_read_result() {
    const CHILD: &str = "BATTLE_PLAN_ALIAS_GROUP_ALLOCATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "container::grouped::group::tests::group_allocation_failures_are_the_read_result",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    thread_local! {
        static ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }
    fn fail_allocation() -> bool {
        let attempt = ATTEMPTS.get() + 1;
        ATTEMPTS.set(attempt);
        FAIL_AT.get() == Some(attempt)
    }
    let (replay, _) = alias_fixture(Layout::Chosen);
    let index = &replay.index[0];
    let block = Block::new(&replay, index);
    let mut checked = 0;
    for member in [TableId::Divisions, TableId::LandCombatDivisionModifiers] {
        let selected = [member];
        let read = |fail_at| {
            ATTEMPTS.set(0);
            FAIL_AT.set(fail_at);
            engine_allocation::set_stop_hook(Some(fail_allocation));
            let mut results =
                block.read(&replay, index, Some(&selected[..]), None, false, &selected);
            engine_allocation::set_stop_hook(None);
            results.pop().expect("the member's result")
        };
        read(None).unwrap();
        let attempts = ATTEMPTS.get();
        for ordinal in 1..=attempts {
            assert_eq!(
                read(Some(ordinal)).unwrap_err().kind(),
                io::ErrorKind::OutOfMemory,
                "{member:?} checked allocation {ordinal}"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the fixture must reach checked allocations");
}

/// A worker panic must propagate; clearing it must restore the serial result.
#[test]
fn a_member_panic_ends_a_parallel_read_without_hanging() {
    let (replay, _) = alias_fixture(Layout::Chosen);
    let parallel = Parallel {
        workers: 8,
        on_worker: &|| {},
    };
    let index = &replay.index[0];
    hooks::with(|hooks| {
        hooks.group_old = Some(false);
        hooks.group_panic = Some((TableId::DivisionStatus, 2));
    });
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        replay.read_block_parallel(index, &parallel)
    }));
    assert!(read.is_err(), "the panic must reach the read");
    hooks::with(|hooks| hooks.group_panic = None);
    assert_eq!(
        replay.read_block_parallel(index, &parallel).unwrap(),
        replay.read_block(index).unwrap()
    );
}

#[test]
fn overlapping_groups_decode_a_shared_member_twice_and_admit_it_once() {
    let shared = TableId::DivisionStatus;
    hooks::with(|hooks| hooks.verify_group = false);
    // Check the overlap before installing the gate so a changed fixture fails without waiting.
    let (probe, _) = alias_fixture(Layout::Chosen);
    let divisions = [TableId::Divisions];
    let (_, decodes) = crate::container::test_work::table_decodes(|| {
        results(&probe, &probe.index[0], Some(&divisions[..]), None, false)
    });
    assert!(
        decodes.iter().any(|&(_, _, table, _)| table == shared),
        "the fixture's Divisions no longer reads DivisionStatus, which this test needs: {decodes:?}"
    );
    let expected = [TableId::Divisions, shared].map(|selected| {
        Block::new(&probe, &probe.index[0])
            .read(
                &probe,
                &probe.index[0],
                Some(&[selected]),
                None,
                true,
                &[selected],
            )
            .pop()
            .unwrap()
    });
    let (replay, _) = alias_fixture(Layout::Chosen);
    let index = &replay.index[0];
    replay.cache_decoded_sections(true);
    replay.log_section_decodes();
    hooks::with(|hooks| {
        hooks.raw_gate = Some((shared, std::sync::Arc::new(hooks::Gate::new(2))));
    });
    std::thread::scope(|scope| {
        for (selected, expected) in [TableId::Divisions, shared].into_iter().zip(&expected) {
            let replay = &replay;
            scope.spawn(move || {
                let read = Block::new(replay, index).read(
                    replay,
                    index,
                    Some(&[selected][..]),
                    None,
                    false,
                    &[selected],
                );
                assert!(read[0].is_ok(), "{selected:?}");
                same(selected, expected, &read[0], "overlapping groups");
            });
        }
    });
    hooks::with(|hooks| hooks.raw_gate = None);
    let wire: Vec<_> = replay
        .wire_decodes()
        .into_iter()
        .filter(|decode| decode.table == shared.index())
        .collect();
    let admitted: Vec<_> = replay
        .section_decodes()
        .into_iter()
        .filter(|decode| decode.table == shared.index())
        .collect();
    let direct = wire
        .iter()
        .filter(|decode| decode.route == crate::container::WireRoute::Direct)
        .count();
    assert_eq!(direct, 2, "a wire decode in each group: {wire:?}");
    assert_eq!(admitted.len(), 1, "one admission: {admitted:?}");
    assert!(admitted[0].returned && !admitted[0].keyframe_only);
}
