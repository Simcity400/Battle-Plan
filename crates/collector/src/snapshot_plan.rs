//! Certify an unchanged owned result using its exact ordered read dependencies.
//! Compilation happens on recycle. A stopped pass supplies its observed prefix
//! to the ordinary reader, including its failed read; it never retries memory.
use engine::{
    Memory,
    memory::{RecordLayout, RecordSources},
    read_plan::{Control, Hint, Outcome, PreparedPlan, Read},
};
use std::cell::{Cell, Ref, RefCell};

#[derive(Debug, Default)]
struct Trace {
    reads: Vec<Read>,
    bytes: Vec<u8>,
    hints: Vec<Hint>,
    eligible: bool,
}

impl Trace {
    fn record(&mut self, address: usize, bytes: &[u8], ok: bool) {
        if !ok
            || self.reads.len() == self.reads.capacity()
            || bytes.len() > self.bytes.capacity() - self.bytes.len()
        {
            self.eligible = false;
        }
        if !self.eligible || bytes.is_empty() {
            return;
        }
        self.reads.push(Read {
            address,
            destination_offset: self.bytes.len(),
            len: bytes.len(),
        });
        self.bytes.extend_from_slice(bytes);
    }
}

#[derive(Debug)]
pub(crate) struct SnapshotPlan {
    trace: RefCell<Trace>,
    compiled: Option<PreparedPlan>,
    destination: Vec<u8>,
    prefix: usize,
    failed: Option<usize>,
    cursor: Cell<usize>,
    blocked: Cell<bool>,
    rebuild: bool,
    tracking: Cell<bool>,
    complete: bool,
}

impl SnapshotPlan {
    pub fn new(reads: usize, bytes: usize) -> Self {
        Self {
            trace: RefCell::new(Trace {
                reads: Vec::with_capacity(reads),
                bytes: Vec::with_capacity(bytes),
                hints: Vec::with_capacity(reads),
                eligible: false,
            }),
            compiled: None,
            destination: Vec::new(),
            prefix: 0,
            failed: None,
            cursor: Cell::new(0),
            blocked: Cell::new(false),
            rebuild: false,
            tracking: Cell::new(true),
            complete: false,
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        let trace = self.trace.borrow();
        trace.reads.capacity() * std::mem::size_of::<Read>()
            + trace.bytes.capacity()
            + trace.hints.capacity() * std::mem::size_of::<Hint>()
            + self.destination.capacity()
            + self
                .compiled
                .as_ref()
                .map_or(0, PreparedPlan::capacity_bytes)
    }

    pub fn invalidate(&mut self) {
        self.complete = false;
        self.trace.get_mut().eligible = false;
        self.rebuild = true;
    }

    /// True certifies identical controlled observations. Explicit data reads
    /// remain fresh in completed_bytes and must be applied by the caller.
    /// The caller separately certifies any values omitted through mutation reuse.
    pub fn begin(&mut self, memory: &impl Memory, certified: bool) -> bool {
        self.complete = false;
        self.prefix = 0;
        self.failed = None;
        self.cursor.set(0);
        self.blocked.set(false);
        if certified
            && !self.rebuild
            && let Some(plan) = &self.compiled
        {
            match memory.execute_read_plan(plan, &mut self.destination) {
                Some(Outcome::Complete) => {
                    self.complete = true;
                    return true;
                }
                Some(Outcome::ControlChanged { completed_reads }) => self.prefix = completed_reads,
                Some(Outcome::ReadFailed { completed_reads }) => {
                    self.prefix = completed_reads + 1;
                    self.failed = Some(completed_reads);
                }
                None => {}
            }
        }
        let trace = self.trace.get_mut();
        trace.reads.clear();
        trace.bytes.clear();
        trace.hints.clear();
        trace.eligible = true;
        self.rebuild = true;
        false
    }

    /// Omit freshly acquired payload reads from control discovery. These reads
    /// must not select any later tracked source address. The caller must prove
    /// payload reuse independently (e.g. uninterrupted mutation revisions).
    /// Owned controls still retain their ordinary first-observation semantics.
    pub fn untracked_reads(&self) -> Untracked<'_> {
        Untracked {
            tracking: &self.tracking,
            previous: self.tracking.replace(false),
        }
    }

    pub fn view<'a, M: Memory>(&'a self, memory: &'a M) -> Observed<'a, M> {
        Observed { plan: self, memory }
    }

    pub fn finish(&mut self, complete: bool) -> bool {
        let valid = !self.blocked.get() && self.cursor.get() == self.prefix;
        self.trace.get_mut().eligible &= complete && valid;
        valid
    }

    /// Pending dependencies, inspected only off-thread during preparation.
    pub fn pending_reads(&self) -> Option<Ref<'_, [Read]>> {
        let trace = self.trace.borrow();
        if self.rebuild && trace.eligible {
            Some(Ref::map(trace, |trace| trace.reads.as_slice()))
        } else {
            None
        }
    }

    /// Fresh data from the most recent fully completed execution, never from
    /// a stopped prefix or an unexecuted prepared program.
    pub fn completed_bytes(&self, offset: usize, len: usize) -> Option<&[u8]> {
        self.complete.then_some(())?;
        self.destination.get(offset..offset.checked_add(len)?)
    }

    /// Called off-thread alongside the owning row/arena growth.
    pub fn prepare(&mut self) {
        self.prepare_with_data(&[]);
    }

    /// Sorted read indexes omitted from equality comparisons, but still read
    /// and fault-checked in original order. They must not select later source
    /// addresses or affect their validation. Callers apply these fresh values
    /// through completed_bytes before publishing their retained result.
    pub fn prepare_with_data(&mut self, data_reads: &[usize]) {
        if !self.rebuild {
            return;
        }
        self.compiled = None;
        let trace = self.trace.get_mut();
        if trace.eligible {
            let controls = trace
                .reads
                .iter()
                .enumerate()
                .filter(|(index, _)| data_reads.binary_search(index).is_err())
                .map(|(index, read)| Control {
                    after_reads: index + 1,
                    offset: read.destination_offset,
                    len: read.len,
                    expected_offset: read.destination_offset,
                })
                .collect();
            self.compiled = PreparedPlan::new(
                trace.reads.clone(),
                controls,
                trace.bytes.clone(),
                trace.bytes.len(),
            )
            .and_then(|plan| plan.with_hints(trace.hints.clone()));
            self.destination.resize(trace.bytes.len(), 0);
        }
        self.rebuild = false;
    }
}

pub(crate) struct Untracked<'a> {
    tracking: &'a Cell<bool>,
    previous: bool,
}
impl Drop for Untracked<'_> {
    fn drop(&mut self) {
        self.tracking.set(self.previous);
    }
}

pub(crate) struct Observed<'a, M> {
    plan: &'a SnapshotPlan,
    memory: &'a M,
}

impl<M: Memory> Memory for Observed<'_, M> {
    fn prefetch(&self, address: usize, len: usize) {
        if !self.plan.tracking.get() {
            self.memory.prefetch(address, len);
            return;
        }
        let mut trace = self.plan.trace.borrow_mut();
        if trace.eligible
            && trace.hints.len() < trace.hints.capacity()
            && len != 0
            && address != 0
            && crate::raw::is_canonical_range(address, len)
        {
            let after_reads = trace.reads.len();
            trace.hints.push(Hint {
                after_reads,
                address,
                len,
            });
        }
        self.memory.prefetch(address, len);
    }

    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if output.is_empty() {
            return true;
        }
        if self.plan.blocked.get() {
            return false;
        }
        if !self.plan.tracking.get() {
            return self.memory.copy(address, output);
        }
        let index = self.plan.cursor.get();
        let ok = if index < self.plan.prefix {
            let read = &self
                .plan
                .compiled
                .as_ref()
                .expect("observed prefix")
                .reads()[index];
            if read.address != address || read.len != output.len() {
                self.plan.blocked.set(true);
                return false;
            }
            self.plan.cursor.set(index + 1);
            if self.plan.failed == Some(index) {
                false
            } else {
                output.copy_from_slice(
                    &self.plan.destination
                        [read.destination_offset..read.destination_offset + read.len],
                );
                true
            }
        } else {
            self.memory.copy(address, output)
        };
        self.plan.trace.borrow_mut().record(address, output, ok);
        ok
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        if self.plan.blocked.get()
            || (self.plan.tracking.get() && self.plan.cursor.get() < self.plan.prefix)
        {
            for (index, (&address, output)) in addresses.iter().zip(outputs.iter_mut()).enumerate()
            {
                if !output.is_empty()
                    && (address == 0
                        || address.checked_add(output.len()).is_none()
                        || !self.copy(address, output))
                {
                    return index;
                }
            }
            return N;
        }
        let completed = self.memory.copy_scattered(addresses, outputs);
        if self.plan.tracking.get() {
            let mut trace = self.plan.trace.borrow_mut();
            for index in 0..N.min(completed + 1) {
                trace.record(addresses[index], outputs[index], index < completed);
            }
        }
        completed
    }

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        if self.plan.blocked.get()
            || (self.plan.tracking.get() && self.plan.cursor.get() < self.plan.prefix)
        {
            return self.copy_scattered(addresses, outputs);
        }
        let completed = self.memory.copy_scattered_prefetched(addresses, outputs);
        if self.plan.tracking.get() {
            let mut trace = self.plan.trace.borrow_mut();
            for index in 0..N.min(completed + 1) {
                trace.record(addresses[index], outputs[index], index < completed);
            }
        }
        completed
    }

    /// One gather outside a stopped prefix; inside it, one observation at a
    /// time in row-major order, like `copy_scattered`.
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let fields = layout.fields();
        let Some(count) = sources.len().checked_mul(fields.len()).filter(|_| {
            sources
                .len()
                .checked_mul(layout.stride())
                .is_some_and(|len| len <= destination.len())
        }) else {
            return 0;
        };
        let field = |index: usize| {
            let row = index / fields.len();
            let field = fields[index % fields.len()];
            let address = sources
                .pointer(row)
                .filter(|&pointer| pointer != 0)
                .and_then(|pointer| pointer.checked_add(field.source_offset))
                .filter(|&address| address.checked_add(field.len).is_some());
            let start = row * layout.stride() + field.destination_offset;
            (address, start..start + field.len)
        };
        if self.plan.blocked.get()
            || (self.plan.tracking.get() && self.plan.cursor.get() < self.plan.prefix)
        {
            for index in 0..count {
                let (address, range) = field(index);
                if range.is_empty() {
                    continue;
                }
                if !address.is_some_and(|address| self.copy(address, &mut destination[range])) {
                    return index;
                }
            }
            return count;
        }
        let completed = self.memory.copy_records(sources, layout, destination);
        if self.plan.tracking.get() {
            let mut trace = self.plan.trace.borrow_mut();
            for index in 0..count.min(completed + 1) {
                let (address, range) = field(index);
                trace.record(address.unwrap_or(0), &destination[range], index < completed);
            }
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untracked_payload_keeps_native_batches_and_does_not_consume_control_prefix() {
        struct Source {
            bytes: [u8; 8],
            calls: RefCell<Vec<usize>>,
            batches: Cell<usize>,
        }
        impl Memory for Source {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push(address);
                output.copy_from_slice(&self.bytes[address - 1..address - 1 + output.len()]);
                true
            }
            fn copy_scattered<const N: usize>(
                &self,
                addresses: [usize; N],
                outputs: &mut [&mut [u8]; N],
            ) -> usize {
                self.batches.set(self.batches.get() + 1);
                for (address, output) in addresses.into_iter().zip(outputs.iter_mut()) {
                    self.copy(address, output);
                }
                N
            }
        }
        fn walk(plan: &SnapshotPlan, source: &Source) -> [u8; 3] {
            let view = plan.view(source);
            let a = engine::read::<u8>(&view, 1).unwrap();
            let b = {
                let _payload = plan.untracked_reads();
                let mut b = [0];
                assert_eq!(view.copy_scattered([8], &mut [&mut b]), 1);
                b[0]
            };
            let c = engine::read::<u8>(&view, 7).unwrap();
            [a, b, c]
        }
        let mut source = Source {
            bytes: [1, 2, 3, 4, 5, 6, 7, 8],
            calls: RefCell::new(vec![]),
            batches: Cell::new(0),
        };
        let mut plan = SnapshotPlan::new(8, 8);
        assert!(!plan.begin(&source, false));
        assert_eq!(walk(&plan, &source), [1, 8, 7]);
        assert!(plan.finish(true));
        assert_eq!(source.batches.get(), 1);
        plan.prepare();
        source.bytes[6] = 42;
        source.bytes[7] = 99;
        source.calls.borrow_mut().clear();
        assert!(!plan.begin(&source, true));
        assert_eq!(walk(&plan, &source), [1, 99, 42]);
        assert!(plan.finish(true));
        assert_eq!(source.batches.get(), 2);
        assert_eq!(*source.calls.borrow(), vec![1, 7, 8]);
    }

    #[test]
    fn portrait_dependencies_preserve_each_primary_fault_and_sorted_rows() {
        use crate::extended::characters::characters_test_kit as kit;
        use engine::{ccharacter, character_portrait_entry, walk};
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
            engine::cprovince::UnitOrderWalk,
            [ccharacter::PortraitRow; 4],
            [character_portrait_entry::Row; 4],
        ) {
            let mut rows = [ccharacter::PortraitRow::default(); 4];
            let mut entries = [character_portrait_entry::Row::default(); 4];
            let result = ccharacter::walk_portraits(
                memory,
                kit::BASE,
                kit::GAME_STATE,
                &mut rows,
                &mut entries,
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
            let mut plan = SnapshotPlan::new(128, 2048);
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

    struct Source {
        bytes: [u8; 8],
        calls: RefCell<Vec<usize>>,
        fail: Option<usize>,
    }
    impl Memory for Source {
        fn copy(&self, address: usize, out: &mut [u8]) -> bool {
            self.calls.borrow_mut().push(address);
            if self.fail == Some(address) {
                out.fill(99);
                return false;
            }
            out.copy_from_slice(&self.bytes[address - 1..address - 1 + out.len()]);
            true
        }
    }
    fn walk(memory: &impl Memory) -> Option<[u8; 3]> {
        let a = engine::read::<u8>(memory, 1)?;
        let b = engine::read::<u8>(memory, usize::from(a))?;
        let c = engine::read::<u8>(memory, 7)?;
        Some([a, b, c])
    }
    #[test]
    fn changed_dependency_and_each_fault_resume_without_repeating_any_observation() {
        for changed in [false, true] {
            for fail in [None, Some(1), Some(3), Some(7)] {
                let baseline = Source {
                    bytes: [3, 0, 5, 6, 0, 0, 9, 0],
                    calls: RefCell::new(vec![]),
                    fail: None,
                };
                let mut plan = SnapshotPlan::new(8, 32);
                assert!(!plan.begin(&baseline, true));
                assert_eq!(walk(&plan.view(&baseline)), Some([3, 5, 9]));
                assert!(plan.finish(true));
                plan.prepare();
                let mut fresh = Source {
                    bytes: baseline.bytes,
                    calls: RefCell::new(vec![]),
                    fail,
                };
                if changed {
                    fresh.bytes[0] = 4;
                }
                let reference = Source {
                    bytes: fresh.bytes,
                    calls: RefCell::new(vec![]),
                    fail,
                };
                let expected = walk(&reference);
                let actual = if plan.begin(&fresh, true) {
                    Some([3, 5, 9])
                } else {
                    let result = walk(&plan.view(&fresh));
                    assert!(plan.finish(result.is_some()));
                    result
                };
                assert_eq!(actual, expected);
                assert_eq!(*fresh.calls.borrow(), *reference.calls.borrow());
            }
        }
    }
    #[test]
    fn repeated_addresses_keep_occurrences_and_prefix_mismatch_stops_live_reads() {
        let source = Source {
            bytes: [3; 8],
            calls: RefCell::new(vec![]),
            fail: None,
        };
        let mut plan = SnapshotPlan::new(8, 32);
        assert!(!plan.begin(&source, true));
        let view = plan.view(&source);
        assert_eq!(engine::read::<u8>(&view, 1), Some(3));
        assert_eq!(engine::read::<u8>(&view, 1), Some(3));
        plan.finish(true);
        plan.prepare();
        source.calls.borrow_mut().clear();
        assert!(plan.begin(&source, true));
        assert_eq!(*source.calls.borrow(), [1, 1]);
        let changed = Source {
            bytes: [4; 8],
            calls: RefCell::new(vec![]),
            fail: None,
        };
        assert!(!plan.begin(&changed, true));
        assert_eq!(engine::read::<u8>(&plan.view(&changed), 2), None);
        assert_eq!(engine::read::<u8>(&plan.view(&changed), 7), None);
        assert!(!plan.finish(true));
        assert_eq!(*changed.calls.borrow(), [1]);
    }
}
