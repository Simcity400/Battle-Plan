
use crate::{Memory, memory::is_canonical_range};
use engine_allocation as allocation;
use std::io;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Hint {
    pub after_reads: usize,
    pub address: usize,
    pub len: usize,
}

#[derive(Debug)]
#[repr(C)]
pub(crate) struct NativeRead {
    pub address: usize,
    pub destination_offset: u32,
    pub len: u32,
}

#[derive(Debug)]
#[repr(C)]
pub(crate) struct NativeControl {
    pub after_reads: u32,
    pub offset: u32,
    pub len: u32,
    pub expected_offset: u32,
}

#[derive(Debug)]
#[repr(C)]
pub(crate) struct NativeHint {
    pub address: usize,
    pub after_reads: u32,
    pub len: u32,
}

#[derive(Debug)]
pub(crate) struct NativePlan {
    pub reads: Box<[NativeRead]>,
    pub controls: Box<[NativeControl]>,
    pub hints: Box<[NativeHint]>,
}

impl NativePlan {
    fn try_new(reads: &[Read], controls: &[Control]) -> io::Result<Option<Self>> {
        if u32::try_from(reads.len()).is_err()
            || reads.iter().any(|read| {
                u32::try_from(read.destination_offset).is_err() || u32::try_from(read.len).is_err()
            })
            || controls.iter().any(|control| {
                u32::try_from(control.after_reads).is_err()
                    || u32::try_from(control.offset).is_err()
                    || u32::try_from(control.len).is_err()
                    || u32::try_from(control.expected_offset).is_err()
            })
        {
            return Ok(None);
        }
        let mut packed_reads = allocation::vec(reads.len())?;
        for read in reads {
            packed_reads.push(NativeRead {
                address: read.address,
                destination_offset: read.destination_offset as u32,
                len: read.len as u32,
            });
        }
        let mut packed_controls = allocation::vec(controls.len())?;
        for control in controls {
            packed_controls.push(NativeControl {
                after_reads: control.after_reads as u32,
                offset: control.offset as u32,
                len: control.len as u32,
                expected_offset: control.expected_offset as u32,
            });
        }
        Ok(Some(Self {
            reads: allocation::boxed_slice(packed_reads)?,
            controls: allocation::boxed_slice(packed_controls)?,
            hints: Box::new([]),
        }))
    }

    fn capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.reads)
            + std::mem::size_of_val(&*self.controls)
            + std::mem::size_of_val(&*self.hints)
    }
}

#[derive(Debug)]
pub struct PreparedPlan {
    reads: Box<[Read]>,
    controls: Box<[Control]>,
    hints: Box<[Hint]>,
    expected: Box<[u8]>,
    destination_len: usize,
    requested_spans: u64,
    requested_bytes: u64,
    native: Option<NativePlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    ControlChanged {
        completed_reads: usize,
    },
    ReadFailed {
        completed_reads: usize,
    },
}

impl PreparedPlan {
    pub fn try_new(
        reads: Vec<Read>,
        controls: Vec<Control>,
        expected: Vec<u8>,
        destination_len: usize,
    ) -> io::Result<Option<Self>> {
        let work = (|| {
            let mut requested_spans = 0_u64;
            let mut requested_bytes = 0_u64;
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
                requested_spans = requested_spans.saturating_add(u64::from(read.len != 0));
                requested_bytes = requested_bytes.saturating_add(read.len as u64);
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
            Some((requested_spans, requested_bytes))
        })();
        let Some((requested_spans, requested_bytes)) = work else {
            return Ok(None);
        };
        Ok(Some(Self {
            native: NativePlan::try_new(&reads, &controls)?,
            reads: allocation::boxed_slice(reads)?,
            controls: allocation::boxed_slice(controls)?,
            hints: Box::new([]),
            expected: allocation::boxed_slice(expected)?,
            destination_len,
            requested_spans,
            requested_bytes,
        }))
    }

    #[cfg(test)]
    pub fn new(
        reads: Vec<Read>,
        controls: Vec<Control>,
        expected: Vec<u8>,
        destination_len: usize,
    ) -> Option<Self> {
        Self::try_new(reads, controls, expected, destination_len).unwrap()
    }

    pub fn reads(&self) -> &[Read] {
        &self.reads
    }

    pub fn controls(&self) -> &[Control] {
        &self.controls
    }

    pub fn try_with_hints(mut self, hints: Vec<Hint>) -> io::Result<Option<Self>> {
        let mut previous = 0;
        for hint in &hints {
            if hint.after_reads < previous
                || hint.after_reads > self.reads.len()
                || (hint.len != 0
                    && (hint.address == 0 || !is_canonical_range(hint.address, hint.len)))
            {
                return Ok(None);
            }
            previous = hint.after_reads;
        }
        self.hints = allocation::boxed_slice(hints)?;
        if self.hints.iter().any(|hint| {
            u32::try_from(hint.after_reads).is_err() || u32::try_from(hint.len).is_err()
        }) {
            self.native = None;
        } else if let Some(native) = &mut self.native {
            let mut packed = allocation::vec(self.hints.len())?;
            for hint in &self.hints {
                packed.push(NativeHint {
                    address: hint.address,
                    after_reads: hint.after_reads as u32,
                    len: hint.len as u32,
                });
            }
            native.hints = allocation::boxed_slice(packed)?;
        }
        Ok(Some(self))
    }

    #[cfg(test)]
    pub fn with_hints(self, hints: Vec<Hint>) -> Option<Self> {
        self.try_with_hints(hints).unwrap()
    }

    pub fn hints(&self) -> &[Hint] {
        &self.hints
    }

    pub(crate) fn native(&self) -> Option<&NativePlan> {
        self.native.as_ref()
    }

    pub fn expected(&self) -> &[u8] {
        &self.expected
    }

    pub fn destination_len(&self) -> usize {
        self.destination_len
    }

    pub(crate) fn requested_work(&self, count: usize) -> (u64, u64) {
        if count >= self.reads.len() {
            return (self.requested_spans, self.requested_bytes);
        }
        self.reads[..count]
            .iter()
            .fold((0_u64, 0_u64), |(spans, bytes), read| {
                (
                    spans.saturating_add(u64::from(read.len != 0)),
                    bytes.saturating_add(read.len as u64),
                )
            })
    }

    pub fn capacity_bytes(&self) -> usize {
        std::mem::size_of_val(self.reads())
            .saturating_add(std::mem::size_of_val(self.controls()))
            .saturating_add(std::mem::size_of_val(self.hints()))
            .saturating_add(self.expected.len())
            .saturating_add(self.native.as_ref().map_or(0, NativePlan::capacity_bytes))
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
    fn compact_geometry_and_hints_reject_narrowing_without_changing_public_bounds() {
        assert_eq!(std::mem::size_of::<NativeRead>(), 16);
        assert_eq!(std::mem::size_of::<NativeControl>(), 16);
        assert_eq!(std::mem::size_of::<NativeHint>(), 16);
        let make = || {
            PreparedPlan::new(
                vec![read(0x1000, 0, 8)],
                vec![control(1, 0, 8, 0)],
                vec![7; 8],
                8,
            )
            .unwrap()
        };
        for hints in [
            vec![Hint {
                after_reads: 2,
                address: 0x1000,
                len: 1,
            }],
            vec![Hint {
                after_reads: 1,
                address: 0,
                len: 1,
            }],
            vec![Hint {
                after_reads: 1,
                address: usize::MAX,
                len: 2,
            }],
            vec![
                Hint {
                    after_reads: 1,
                    address: 0x1000,
                    len: 1,
                },
                Hint {
                    after_reads: 0,
                    address: 0x1000,
                    len: 1,
                },
            ],
        ] {
            assert!(make().with_hints(hints).is_none());
        }
        let hints = vec![
            Hint {
                after_reads: 0,
                address: 0x1000,
                len: 8,
            },
            Hint {
                after_reads: 1,
                address: 1,
                len: 1,
            },
        ];
        let plan = make().with_hints(hints.clone()).unwrap();
        assert_eq!(plan.hints(), hints);
        assert_eq!(plan.native().unwrap().hints.len(), 2);
        assert_eq!(
            source(&[7; 8]).execute_read_plan(&plan, &mut [0; 8]),
            Some(Outcome::Complete)
        );
        let wide = u32::MAX as usize + 1;
        let plan = PreparedPlan::new(vec![read(0, wide, 0)], vec![], vec![], wide).unwrap();
        assert!(plan.native().is_none());
        assert_eq!(plan.reads()[0].destination_offset, wide);
        let plan = make()
            .with_hints(vec![Hint {
                after_reads: 1,
                address: 0x1000,
                len: wide,
            }])
            .unwrap();
        assert!(plan.native().is_none());
        assert_eq!(plan.hints()[0].len, wide);
    }

    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
    #[test]
    fn native_compact_controls_keep_every_byte_and_fixed_width_boundary() {
        let bytes = [7_u8; 128];
        let address = bytes.as_ptr() as usize;
        let memory = unsafe { crate::memory::LiveMemory::new() };
        for width in [1, 2, 3, 4, 8, 12, 16, 24, 26, 32, 48, 64] {
            for changed in (0..width).map(Some).chain([None]) {
                let mut expected = vec![7; width];
                if let Some(index) = changed {
                    expected[index] ^= 1;
                }
                let plan = PreparedPlan::new(
                    vec![read(address + 1, 2, width)],
                    vec![control(1, 2, width, 0)],
                    expected,
                    128,
                )
                .unwrap()
                .with_hints(vec![
                    Hint {
                        after_reads: 0,
                        address: address + 1,
                        len: width,
                    },
                    Hint {
                        after_reads: 1,
                        address,
                        len: 1,
                    },
                ])
                .unwrap();
                let mut output = [0xCC; 128];
                assert_eq!(
                    memory.execute_read_plan(&plan, &mut output),
                    Some(if changed.is_some() {
                        Outcome::ControlChanged { completed_reads: 1 }
                    } else {
                        Outcome::Complete
                    })
                );
                assert_eq!(&output[..2], &[0xCC; 2]);
                assert_eq!(&output[2..2 + width], &bytes[..width]);
                assert!(output[2 + width..].iter().all(|&byte| byte == 0xCC));
            }
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
        assert_eq!(plan.requested_work(0), (0, 0));
        assert_eq!(plan.requested_work(1), (0, 0));
        assert_eq!(plan.requested_work(2), (1, 1));
        assert_eq!(plan.requested_work(usize::MAX), (1, 1));
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
