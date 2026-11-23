use collector_snapshots::snapshot_plan::SnapshotPlan;
use engine_native::Memory;
use std::cell::RefCell;

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() {
    let memory = engine_native::memory::SliceMemory {
        base: 0x1000,
        bytes: &[0x11; 32],
    };
    let mut plan = SnapshotPlan::default();
    plan.begin(&memory, false);
    let mut output = [0; 4];
    let view = plan.view(&memory);
    view.prefetch(0x1000, 12);
    let mut complete = true;
    for address in [0x1000, 0x1004, 0x1008] {
        complete &= view.copy(address, &mut output);
    }
    plan.finish(complete);
    plan.prepare_with_data(&[1]);
    if collector_memory::terminal::failed() {
        return;
    }
    plan.begin(&memory, true);
    plan.invalidate();
    plan.begin(&memory, false);
    let mut output = [0; 16];
    let view = plan.view(&memory);
    view.prefetch(0x1000, 32);
    let complete = view.copy(0x1000, &mut output) && view.copy(0x1010, &mut output);
    plan.finish(complete);
    plan.prepare();
}

#[test]
fn portrait_dependencies_preserve_each_primary_fault_and_sorted_rows() {
    use collector_characters::extended::characters::characters_test_kit as kit;
    use engine_native::{ccharacter, character_portrait_entry, walk};
    struct Source<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault: Option<(usize, usize)>,
    }
    impl Memory for Source<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            self.fault != Some((address, output.len()))
                && kit::memory(self.bytes).copy(address, output)
        }
    }
    let bytes = kit::mapped_fixture();
    fn walk(
        memory: &impl Memory,
    ) -> (
        engine_native::cprovince::UnitOrderWalk,
        Vec<ccharacter::PortraitRow>,
        Vec<character_portrait_entry::Row<engine_native::capture_text::Owned>>,
    ) {
        let mut rows = vec![ccharacter::PortraitRow::default(); 4];
        let mut entries =
            vec![character_portrait_entry::Row::<engine_native::capture_text::Owned>::default(); 4];
        let result = ccharacter::walk_portraits(
            memory,
            kit::BASE,
            kit::GAME_STATE,
            &mut rows,
            &mut entries,
            ccharacter::PortraitWalk::default(),
        );
        (result, rows, entries)
    }
    let reference = Source {
        bytes: &bytes,
        calls: RefCell::new(vec![]),
        fault: None,
    };
    let expected = walk(&reference);
    assert!(matches!(expected.0.outcome, walk::Outcome::Rows { len: 3 }));
    for fault in reference
        .calls
        .into_inner()
        .into_iter()
        .map(Some)
        .chain([None])
    {
        let mut plan = SnapshotPlan::default();
        assert!(!plan.begin(&kit::memory(&bytes), false));
        let baseline = walk(&plan.view(&kit::memory(&bytes)));
        assert!(plan.finish(true));
        plan.prepare();
        let actual = Source {
            bytes: &bytes,
            calls: RefCell::new(vec![]),
            fault,
        };
        let result = if plan.begin(&actual, true) {
            baseline
        } else {
            let result = walk(&plan.view(&actual));
            assert!(plan.finish(matches!(result.0.outcome, walk::Outcome::Rows { .. })));
            result
        };
        let reference = Source {
            bytes: &bytes,
            calls: RefCell::new(vec![]),
            fault,
        };
        assert_eq!(result, walk(&reference));
        assert_eq!(*actual.calls.borrow(), *reference.calls.borrow());
    }
}
