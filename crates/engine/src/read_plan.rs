//! Prepared exact reads with ordered comparisons of recorder-owned controls.
//!
//! A comparison runs after its stated number of logical reads, before the next
//! read. Its builder must prove that those bytes include every dependency used
//! to select later source addresses. Bounds validation alone does not establish
//! that proof, an object lifetime, or a coherent game-state snapshot. In
//! particular, controls at zero reads inspect the caller's initial destination.

use crate::{Memory, memory::is_canonical_range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Read {
    pub address: usize,
    pub destination_offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Control {
    pub after_reads: usize,
    pub offset: usize,
    pub len: usize,
    pub expected_offset: usize,
}

/// Immutable geometry and owned expected controls. Preparation may allocate;
/// execution never allocates and never reads a source beyond the stopped prefix.
#[derive(Debug)]
pub struct PreparedPlan {
    reads: Box<[Read]>,
    controls: Box<[Control]>,
    expected: Box<[u8]>,
    destination_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    ControlChanged {
        completed_reads: usize,
    },
    /// The next read was attempted and may have copied an unknown partial span.
    /// No later read or comparison has run.
    ReadFailed {
        completed_reads: usize,
    },
}

impl PreparedPlan {
    pub fn new(
        reads: Vec<Read>,
        controls: Vec<Control>,
        expected: Vec<u8>,
        destination_len: usize,
    ) -> Option<Self> {
        for read in &reads {
            if read.destination_offset.checked_add(read.len)? > destination_len {
                return None;
            }
            if read.len != 0
                && (read.address == 0
                    || read.address.checked_add(read.len).is_none()
                    || !is_canonical_range(read.address, read.len))
            {
                return None;
            }
        }
        let mut previous = 0;
        for control in &controls {
            if control.after_reads < previous
                || control.after_reads > reads.len()
                || control.offset.checked_add(control.len)? > destination_len
                || control.expected_offset.checked_add(control.len)? > expected.len()
            {
                return None;
            }
            previous = control.after_reads;
        }
        Some(Self {
            reads: reads.into_boxed_slice(),
            controls: controls.into_boxed_slice(),
            expected: expected.into_boxed_slice(),
            destination_len,
        })
    }

    pub fn reads(&self) -> &[Read] {
        &self.reads
    }

    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    pub fn expected(&self) -> &[u8] {
        &self.expected
    }

    pub fn destination_len(&self) -> usize {
        self.destination_len
    }

    /// Owned descriptor/control buffers; the containing plan is charged by its owner.
    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of_val(self.reads())
            .saturating_add(std::mem::size_of_val(self.controls()))
            .saturating_add(self.expected.len())
    }

    pub(crate) fn execute(
        &self,
        memory: &(impl Memory + ?Sized),
        destination: &mut [u8],
    ) -> Option<Outcome> {
        if destination.len() < self.destination_len {
            return None;
        }
        let mut controls = self.controls.iter().peekable();
        for completed in 0..=self.reads.len() {
            while let Some(control) = controls.peek()
                && control.after_reads == completed
            {
                if destination[control.offset..control.offset + control.len]
                    != self.expected[control.expected_offset..control.expected_offset + control.len]
                {
                    return Some(Outcome::ControlChanged {
                        completed_reads: completed,
                    });
                }
                controls.next();
            }
            let Some(read) = self.reads.get(completed) else {
                return Some(Outcome::Complete);
            };
            if read.len != 0
                && !memory.copy(
                    read.address,
                    &mut destination[read.destination_offset..read.destination_offset + read.len],
                )
            {
                return Some(Outcome::ReadFailed {
                    completed_reads: completed,
                });
            }
        }
        unreachable!("the final control boundary returns before another read")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Observed {
        bytes: Vec<u8>,
        calls: RefCell<Vec<(usize, usize)>>,
        fail_at: Option<usize>,
    }

    impl Memory for Observed {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fail_at == Some(address) {
                if let Some(first) = destination.first_mut() {
                    *first = 0xE7;
                }
                return false;
            }
            let Some(start) = address.checked_sub(0x1000) else {
                return false;
            };
            let Some(source) = self.bytes.get(start..start + destination.len()) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }

    fn source(bytes: &[u8]) -> Observed {
        Observed {
            bytes: bytes.to_vec(),
            calls: RefCell::new(Vec::new()),
            fail_at: None,
        }
    }

    fn read(address: usize, offset: usize, len: usize) -> Read {
        Read {
            address,
            destination_offset: offset,
            len,
        }
    }

    fn control(after_reads: usize, offset: usize, len: usize, expected_offset: usize) -> Control {
        Control {
            after_reads,
            offset,
            len,
            expected_offset,
        }
    }

    #[test]
    fn changed_parent_stops_before_any_child_read_and_payload_changes_are_fresh() {
        let plan = PreparedPlan::new(
            vec![read(0x1000, 0, 1), read(0x1001, 1, 2)],
            vec![control(1, 0, 1, 0)],
            vec![7],
            3,
        )
        .unwrap();
        let changed = source(&[8, 91, 92]);
        let mut output = [0xCC; 3];
        assert_eq!(
            changed.execute_read_plan(&plan, &mut output),
            Some(Outcome::ControlChanged { completed_reads: 1 })
        );
        assert_eq!(&*changed.calls.borrow(), &[(0x1000, 1)]);
        assert_eq!(output, [8, 0xCC, 0xCC]);

        let stable = source(&[7, 91, 92]);
        assert_eq!(
            stable.execute_read_plan(&plan, &mut output),
            Some(Outcome::Complete)
        );
        assert_eq!(output, [7, 91, 92]);
        assert_eq!(&*stable.calls.borrow(), &[(0x1000, 1), (0x1001, 2)]);
    }

    #[test]
    fn read_fault_preserves_successful_prefix_and_partial_failed_destination() {
        let plan = PreparedPlan::new(
            vec![read(0x1000, 0, 1), read(0x1001, 1, 2), read(0x1003, 3, 1)],
            vec![control(2, 1, 2, 0)],
            vec![0, 0],
            4,
        )
        .unwrap();
        let mut memory = source(&[4, 5, 6, 7]);
        memory.fail_at = Some(0x1001);
        let mut output = [0xCC; 4];
        assert_eq!(
            memory.execute_read_plan(&plan, &mut output),
            Some(Outcome::ReadFailed { completed_reads: 1 })
        );
        assert_eq!(&*memory.calls.borrow(), &[(0x1000, 1), (0x1001, 2)]);
        assert_eq!(output, [4, 0xE7, 0xCC, 0xCC]);
    }

    #[test]
    fn controls_at_zero_same_position_empty_reads_and_final_boundary_all_execute() {
        let memory = source(&[9]);
        let plan = PreparedPlan::new(
            vec![read(usize::MAX, 0, 0), read(0x1000, 1, 1)],
            vec![
                control(0, 0, 1, 0),
                control(1, 0, 1, 0),
                control(2, 1, 1, 1),
                control(2, 0, 1, 2),
            ],
            vec![3, 9, 4],
            2,
        )
        .unwrap();
        let mut output = [3, 0];
        assert_eq!(
            memory.execute_read_plan(&plan, &mut output),
            Some(Outcome::ControlChanged { completed_reads: 2 })
        );
        assert_eq!(&*memory.calls.borrow(), &[(0x1000, 1)]);
        assert_eq!(output, [3, 9]);

        let empty = PreparedPlan::new(vec![], vec![control(0, 0, 1, 0)], vec![4], 1).unwrap();
        assert_eq!(
            memory.execute_read_plan(&empty, &mut [3]),
            Some(Outcome::ControlChanged { completed_reads: 0 })
        );
        assert_eq!(
            memory.execute_read_plan(&empty, &mut [4]),
            Some(Outcome::Complete)
        );
    }

    #[test]
    fn geometry_and_control_order_are_rejected_before_execution() {
        for bad in [
            read(0, 0, 1),
            read(usize::MAX, 0, 1),
            read(0x0000_8000_0000_0000, 0, 1),
            read(0x1000, 2, 1),
            read(0x1000, usize::MAX, 2),
        ] {
            assert!(PreparedPlan::new(vec![bad], vec![], vec![], 2).is_none());
        }
        for controls in [
            vec![control(2, 0, 0, 0)],
            vec![control(1, 0, 0, 0), control(0, 0, 0, 0)],
            vec![control(1, 1, 2, 0)],
            vec![control(1, 0, 1, 2)],
            vec![control(1, usize::MAX, 1, 0)],
            vec![control(1, 0, 1, usize::MAX)],
        ] {
            assert!(PreparedPlan::new(vec![read(0x1000, 0, 1)], controls, vec![0; 2], 2).is_none());
        }
        let memory = source(&[1, 2]);
        let plan = PreparedPlan::new(vec![read(0x1000, 0, 2)], vec![], vec![], 2).unwrap();
        assert_eq!(memory.execute_read_plan(&plan, &mut [0]), None);
        assert!(memory.calls.borrow().is_empty());
        assert!(
            PreparedPlan::new(
                vec![read(usize::MAX, 2, 0)],
                vec![control(1, 2, 0, 0)],
                vec![],
                2
            )
            .is_some()
        );
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    #[test]
    fn native_controls_and_faults_stop_at_the_same_read_boundary() {
        use crate::memory::LiveMemory;

        let source = [7_u8, 11, 12];
        let address = source.as_ptr() as usize;
        // SAFETY: source is immutable owned memory throughout execution. The
        // deliberately unreadable null-page address is accessed only by SEH.
        let memory = unsafe { LiveMemory::new() };
        let plan = PreparedPlan::new(
            vec![read(address, 0, 1), read(1, 1, 1), read(address + 1, 2, 1)],
            vec![control(1, 0, 1, 0)],
            vec![8],
            3,
        )
        .unwrap();
        let mut output = [0xCC; 3];
        assert_eq!(
            memory.execute_read_plan(&plan, &mut output),
            Some(Outcome::ControlChanged { completed_reads: 1 })
        );
        assert_eq!(output, [7, 0xCC, 0xCC]);

        let fault = PreparedPlan::new(plan.reads().to_vec(), vec![], vec![], 3).unwrap();
        assert_eq!(
            memory.execute_read_plan(&fault, &mut output),
            Some(Outcome::ReadFailed { completed_reads: 1 })
        );
        assert_eq!(output[2], 0xCC);

        let complete = PreparedPlan::new(
            vec![read(usize::MAX, 0, 0), read(address, 0, 3)],
            vec![
                control(0, 3, 1, 0),
                control(1, 3, 1, 0),
                control(2, 0, 1, 1),
                control(2, 3, 1, 2),
            ],
            vec![5, 7, 6],
            4,
        )
        .unwrap();
        let mut output = [0, 0, 0, 5];
        assert_eq!(
            memory.execute_read_plan(&complete, &mut output),
            Some(Outcome::ControlChanged { completed_reads: 2 })
        );
        assert_eq!(output, [7, 11, 12, 5]);

        let final_ok = PreparedPlan::new(
            complete.reads().to_vec(),
            complete.controls()[..3].to_vec(),
            vec![5, 7],
            4,
        )
        .unwrap();
        assert_eq!(
            memory.execute_read_plan(&final_ok, &mut output),
            Some(Outcome::Complete)
        );
        assert_eq!(memory.execute_read_plan(&final_ok, &mut [0; 3]), None);
        let no_reads = PreparedPlan::new(vec![], vec![control(0, 0, 1, 0)], vec![2], 1).unwrap();
        assert_eq!(
            memory.execute_read_plan(&no_reads, &mut [1]),
            Some(Outcome::ControlChanged { completed_reads: 0 })
        );
        assert_eq!(
            memory.execute_read_plan(&no_reads, &mut [2]),
            Some(Outcome::Complete)
        );
    }
}
