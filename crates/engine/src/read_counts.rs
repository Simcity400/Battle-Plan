//! Source requests, measured below all owned snapshots and caches.
//!
//! Wrap the raw source once, before constructing any acquisition adapters.
//! Batch methods delegate to that source's overrides, never to this wrapper's
//! scalar fallback. Thread-local counters need no per-read allocation, lock or
//! atomic. Owners drain them at table/job boundaries, including on unwind.

use crate::{
    Memory,
    memory::{RecordLayout, RecordSources, SliceMemory},
    read_plan::{Outcome, PreparedPlan},
};
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
use std::ptr::NonNull;
use std::{cell::Cell, ops::AddAssign};

/// Requested work at the raw Memory boundary, not bytes retained in a snapshot.
/// Calls include empty requests and rejected/faulted calls. Spans exclude empty
/// destinations and count the completed prefix plus its first failed/rejected
/// source request; a plan control rejection has no following source request.
/// Bytes sum those spans' requested lengths, including an entire faulted span
/// whose partial copied length is unknown. Invalid destination geometry has no
/// source requests. Neither successful-byte counts nor cache-line traffic is
/// inferred from these counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadCounts {
    pub copy_calls: u64,
    pub scattered_calls: u64,
    pub record_calls: u64,
    pub plan_calls: u64,
    pub spans: u64,
    pub bytes: u64,
}

thread_local! {
    static COUNTS: Cell<ReadCounts> = const { Cell::new(ReadCounts::ZERO) };
}

impl ReadCounts {
    pub const ZERO: Self = Self {
        copy_calls: 0,
        scattered_calls: 0,
        record_calls: 0,
        plan_calls: 0,
        spans: 0,
        bytes: 0,
    };

    /// Drain this thread's raw requests. Nested job owners must retain and
    /// restore the previous value so caller-assisted work is not charged twice.
    pub fn take_current() -> Self {
        COUNTS.with(|counts| counts.replace(Self::ZERO))
    }

    /// Restore a suspended owner or add a batch's source requests.
    #[inline]
    pub fn add_current(self) {
        COUNTS.with(|counts| self.add_to(counts));
    }

    #[inline]
    fn add_to(self, counts: &Cell<Self>) {
        let mut value = counts.get();
        value += self;
        counts.set(value);
    }
}

impl AddAssign for ReadCounts {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.copy_calls = self.copy_calls.saturating_add(rhs.copy_calls);
        self.scattered_calls = self.scattered_calls.saturating_add(rhs.scattered_calls);
        self.record_calls = self.record_calls.saturating_add(rhs.record_calls);
        self.plan_calls = self.plan_calls.saturating_add(rhs.plan_calls);
        self.spans = self.spans.saturating_add(rhs.spans);
        self.bytes = self.bytes.saturating_add(rhs.bytes);
    }
}

/// A single accounting boundary around a raw source (LiveMemory in production,
/// a synthetic fixture in tests). Put caching/recording adapters outside it:
/// owned reads then bypass this boundary and misses retain the native fast path.
/// On Windows x64 MSVC, the private pointer makes this neither Send nor Sync.
pub struct CountedMemory<M>(
    pub M,
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))] NonNull<Cell<ReadCounts>>,
);

impl<M> CountedMemory<M> {
    pub fn new(source: M) -> Self {
        Self(
            source,
            #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
            COUNTS.with(|counts| NonNull::from(counts)),
        )
    }

    #[inline]
    fn add_counts(&self, counts: ReadCounts) {
        #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
        {
            // SAFETY: on this target, pinned std stores const/no-Drop COUNTS
            // in fixed native TLS until thread/module exit. Private !Send/!Sync
            // handle stays on that thread; collector quiescence ends all uses
            // before DLL unload. No destructor destroys the Cell; resets only
            // replace its contents. This short shared borrow permits nesting.
            counts.add_to(unsafe { self.1.as_ref() });
        }
        #[cfg(not(all(windows, target_env = "msvc", target_arch = "x86_64")))]
        counts.add_current();
    }
}

impl<M: Memory> Memory for CountedMemory<M> {
    #[inline]
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        let len = std::mem::size_of::<T>();
        self.add_counts(ReadCounts {
            copy_calls: 1,
            spans: u64::from(len != 0),
            bytes: len as u64,
            ..ReadCounts::ZERO
        });
        self.0.read_scalar(address)
    }

    #[inline]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.add_counts(ReadCounts {
            copy_calls: 1,
            spans: u64::from(!destination.is_empty()),
            bytes: destination.len() as u64,
            ..ReadCounts::ZERO
        });
        self.0.copy(address, destination)
    }

    #[inline]
    fn copy_uninit<'a>(
        &self,
        address: usize,
        destination: &'a mut [std::mem::MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        self.add_counts(ReadCounts {
            copy_calls: 1,
            spans: u64::from(!destination.is_empty()),
            bytes: destination.len() as u64,
            ..ReadCounts::ZERO
        });
        self.0.copy_uninit(address, destination)
    }

    #[inline]
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.0.copy_scattered(addresses, destinations);
        self.add_counts(count_scattered(destinations, completed));
        completed
    }

    #[inline]
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.0.copy_scattered_prefetched(addresses, destinations);
        self.add_counts(count_scattered(destinations, completed));
        completed
    }

    #[inline(always)]
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.0.copy_records(sources, layout, destination);
        let (spans, bytes) = match layout.count(sources, destination.len()) {
            Some(count) => {
                let requested = completed
                    .saturating_add(usize::from(completed < count))
                    .min(count);
                (
                    layout.completed_spans(requested),
                    layout.completed_bytes(requested),
                )
            }
            None => (0, 0),
        };
        self.add_counts(ReadCounts {
            record_calls: 1,
            spans: spans as u64,
            bytes: bytes as u64,
            ..ReadCounts::ZERO
        });
        completed
    }

    #[inline]
    fn execute_read_plan(&self, plan: &PreparedPlan, destination: &mut [u8]) -> Option<Outcome> {
        let result = self.0.execute_read_plan(plan, destination);
        let requested = match result {
            Some(Outcome::Complete) => plan.reads().len(),
            Some(Outcome::ControlChanged { completed_reads }) => completed_reads,
            Some(Outcome::ReadFailed { completed_reads }) => completed_reads.saturating_add(1),
            None => 0,
        };
        let (spans, bytes) = plan.requested_work(requested);
        self.add_counts(ReadCounts {
            plan_calls: 1,
            spans,
            bytes,
            ..ReadCounts::ZERO
        });
        result
    }

    fn immutable_view(&self, address: usize, len: usize) -> Option<SliceMemory<'_>> {
        self.0.immutable_view(address, len)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        self.0.authenticated_countries(base, game_state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        self.0.modifier_accumulator_value(address, key)
    }

    fn core_supply_access(
        &self,
        base: usize,
        game_state: usize,
        owner_raw: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        self.0.core_supply_access(base, game_state, owner_raw, area)
    }

    fn movement_primary_root(
        &self,
        base: usize,
        member: usize,
    ) -> Option<Result<usize, &'static str>> {
        self.0.movement_primary_root(base, member)
    }

    fn movement_air_defense_value(
        &self,
        defense: i64,
        denominator: i64,
        reduction: i64,
    ) -> Option<Result<i64, &'static str>> {
        self.0
            .movement_air_defense_value(defense, denominator, reduction)
    }
}

#[inline]
fn count_scattered<const N: usize>(destinations: &[&mut [u8]; N], completed: usize) -> ReadCounts {
    let mut counts = ReadCounts {
        scattered_calls: 1,
        ..ReadCounts::ZERO
    };
    for destination in destinations.iter().take(completed.saturating_add(1)) {
        counts.spans = counts
            .spans
            .saturating_add(u64::from(!destination.is_empty()));
        counts.bytes = counts.bytes.saturating_add(destination.len() as u64);
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        memory::RecordField,
        read_plan::{Control, Read},
    };

    fn healthy_fixture(memory: &impl Memory, base: usize) {
        let mut copied = [0; 3];
        assert!(memory.copy(base, &mut copied));
        assert_eq!(copied, [7; 3]);
        let mut first = [0; 4];
        let mut second = [0; 6];
        assert_eq!(
            memory.copy_scattered([base + 8, base + 16], &mut [&mut first, &mut second]),
            2
        );
        assert_eq!(first, [7; 4]);
        assert_eq!(second, [7; 6]);
        let mut prefetched = [0; 2];
        assert_eq!(
            memory.copy_scattered_prefetched(
                [usize::MAX, base + 24, 0],
                &mut [&mut [], &mut prefetched, &mut []],
            ),
            3
        );
        assert_eq!(prefetched, [7; 2]);
        let fields = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 4,
                len: 2,
            },
        ];
        let layout = RecordLayout::new(&fields, 6).unwrap();
        let mut records = [0; 12];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[base, base + 16]),
                &layout,
                &mut records,
            ),
            4
        );
        assert_eq!(records, [7; 12]);
        let plan = PreparedPlan::new(
            vec![
                Read {
                    address: base + 32,
                    destination_offset: 0,
                    len: 3,
                },
                Read {
                    address: base + 40,
                    destination_offset: 3,
                    len: 5,
                },
            ],
            vec![],
            vec![],
            8,
        )
        .unwrap();
        let mut planned = [0; 8];
        assert_eq!(
            memory.execute_read_plan(&plan, &mut planned),
            Some(Outcome::Complete)
        );
        assert_eq!(planned, [7; 8]);
    }

    const HEALTHY_COUNTS: ReadCounts = ReadCounts {
        copy_calls: 1,
        scattered_calls: 2,
        record_calls: 1,
        plan_calls: 1,
        spans: 10,
        bytes: 35,
    };

    fn fault_fixture(memory: &impl Memory, base: usize) {
        assert!(!memory.copy(1, &mut [0; 4]));
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                copy_calls: 1,
                spans: 1,
                bytes: 4,
                ..ReadCounts::ZERO
            }
        );
        for prefetched in [false, true] {
            let mut first = [0xa5; 2];
            let mut failed = [0xa5; 6];
            let mut later = [0xa5; 8];
            let addresses = [base, usize::MAX, 1, base];
            let destinations = &mut [&mut first[..], &mut [], &mut failed[..], &mut later[..]];
            let completed = if prefetched {
                memory.copy_scattered_prefetched(addresses, destinations)
            } else {
                memory.copy_scattered(addresses, destinations)
            };
            assert_eq!(completed, 2);
            assert_eq!(first, [7; 2]);
            assert_eq!(later, [0xa5; 8]);
            assert_eq!(
                ReadCounts::take_current(),
                ReadCounts {
                    scattered_calls: 1,
                    spans: 2,
                    bytes: 8,
                    ..ReadCounts::ZERO
                }
            );
        }
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: 4,
                destination_offset: 4,
                len: 8,
            },
            RecordField {
                source_offset: 14,
                destination_offset: 12,
                len: 2,
            },
        ];
        let layout = RecordLayout::new(&fields, 14).unwrap();
        let mut records = [0xa5; 42];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[base, 1, base]),
                &layout,
                &mut records
            ),
            5
        );
        assert_eq!(records[..14], [7; 14]);
        assert_eq!(records[18..], [0xa5; 24]);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                record_calls: 1,
                spans: 4,
                bytes: 18,
                ..ReadCounts::ZERO
            }
        );
        let plan = PreparedPlan::new(
            vec![
                Read {
                    address: usize::MAX,
                    destination_offset: 0,
                    len: 0,
                },
                Read {
                    address: base,
                    destination_offset: 0,
                    len: 3,
                },
                Read {
                    address: 1,
                    destination_offset: 3,
                    len: 8,
                },
                Read {
                    address: base,
                    destination_offset: 11,
                    len: 1,
                },
            ],
            vec![],
            vec![],
            12,
        )
        .unwrap();
        let mut planned = [0xa5; 12];
        assert_eq!(
            memory.execute_read_plan(&plan, &mut planned),
            Some(Outcome::ReadFailed { completed_reads: 2 })
        );
        assert_eq!(planned[..3], [7; 3]);
        assert_eq!(planned[11], 0xa5);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                plan_calls: 1,
                spans: 2,
                bytes: 11,
                ..ReadCounts::ZERO
            }
        );
    }

    fn control_fixture(memory: &impl Memory, base: usize) {
        for (boundary, spans, bytes) in [(0, 0, 0), (1, 0, 0), (2, 1, 4), (3, 1, 4), (4, 2, 8)] {
            let plan = PreparedPlan::new(
                vec![
                    Read {
                        address: usize::MAX,
                        destination_offset: 0,
                        len: 0,
                    },
                    Read {
                        address: base,
                        destination_offset: 0,
                        len: 4,
                    },
                    Read {
                        address: 0,
                        destination_offset: 4,
                        len: 0,
                    },
                    Read {
                        address: base + 4,
                        destination_offset: 4,
                        len: 4,
                    },
                ],
                vec![Control {
                    after_reads: boundary,
                    offset: 0,
                    len: 1,
                    expected_offset: 0,
                }],
                vec![99],
                8,
            )
            .unwrap();
            let mut output = [0xa5; 8];
            assert_eq!(
                memory.execute_read_plan(&plan, &mut output),
                Some(Outcome::ControlChanged {
                    completed_reads: boundary
                })
            );
            assert_eq!(&output[..bytes as usize], &[7; 8][..bytes as usize]);
            assert_eq!(&output[bytes as usize..], &[0xa5; 8][bytes as usize..]);
            assert_eq!(
                ReadCounts::take_current(),
                ReadCounts {
                    plan_calls: 1,
                    spans,
                    bytes,
                    ..ReadCounts::ZERO
                }
            );
        }
    }

    fn rejected_source_fixture(memory: &impl Memory, base: usize, rejected: usize) {
        assert!(!memory.copy(rejected, &mut [0; 4]));
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                copy_calls: 1,
                spans: 1,
                bytes: 4,
                ..ReadCounts::ZERO
            }
        );
        for prefetched in [false, true] {
            let mut first = [0xa5; 2];
            let mut failed = [0xa5; 4];
            let mut later = [0xa5; 6];
            let addresses = [base, usize::MAX, rejected, base];
            let destinations = &mut [&mut first[..], &mut [], &mut failed[..], &mut later[..]];
            let completed = if prefetched {
                memory.copy_scattered_prefetched(addresses, destinations)
            } else {
                memory.copy_scattered(addresses, destinations)
            };
            assert_eq!(completed, 2);
            assert_eq!(first, [7; 2]);
            assert_eq!(failed, [0xa5; 4]);
            assert_eq!(later, [0xa5; 6]);
            assert_eq!(
                ReadCounts::take_current(),
                ReadCounts {
                    scattered_calls: 1,
                    spans: 2,
                    bytes: 6,
                    ..ReadCounts::ZERO
                }
            );
        }
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 4,
                len: 2,
            },
        ];
        let mut output = [0xa5; 18];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[base, rejected, base]),
                &RecordLayout::new(&fields, 6).unwrap(),
                &mut output,
            ),
            4
        );
        assert_eq!(output[..6], [7; 6]);
        assert_eq!(output[6..], [0xa5; 12]);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                record_calls: 1,
                spans: 3,
                bytes: 10,
                ..ReadCounts::ZERO
            }
        );
    }

    #[test]
    fn exact_healthy_fixture_counts_do_not_double_count_scalar_batch_fallbacks() {
        ReadCounts::take_current();
        healthy_fixture(
            &CountedMemory::new(SliceMemory {
                base: 0x1000,
                bytes: &[7; 64],
            }),
            0x1000,
        );
        assert_eq!(ReadCounts::take_current(), HEALTHY_COUNTS);
        assert_eq!(ReadCounts::take_current(), ReadCounts::ZERO);
    }

    #[test]
    fn failed_batches_count_the_fault_request_but_not_the_unexecuted_suffix() {
        ReadCounts::take_current();
        fault_fixture(
            &CountedMemory::new(SliceMemory {
                base: 0x1000,
                bytes: &[7; 64],
            }),
            0x1000,
        );
    }

    #[test]
    fn a_partial_fault_keeps_the_original_result_and_charges_the_full_request() {
        struct Partial(std::cell::RefCell<Vec<usize>>);
        impl Memory for Partial {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.borrow_mut().push(address);
                if address == 0x1004 {
                    destination[0] = 0xe7;
                    false
                } else {
                    destination.fill(7);
                    true
                }
            }
        }
        ReadCounts::take_current();
        let memory = CountedMemory::new(Partial(std::cell::RefCell::new(Vec::new())));
        let mut first = [0xa5; 2];
        let mut failed = [0xa5; 6];
        let mut later = [0xa5; 3];
        assert_eq!(
            memory.copy_scattered(
                [0x1000, 0x1004, 0x1008],
                &mut [&mut first, &mut failed, &mut later],
            ),
            1
        );
        assert_eq!(first, [7; 2]);
        assert_eq!(failed, [0xe7, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]);
        assert_eq!(later, [0xa5; 3]);
        assert_eq!(*memory.0.0.borrow(), [0x1000, 0x1004]);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                scattered_calls: 1,
                spans: 2,
                bytes: 8,
                ..ReadCounts::ZERO
            }
        );
        memory.0.0.borrow_mut().clear();
        assert_eq!(crate::read::<u32>(&memory, 0x1000), Some(0x0707_0707));
        assert_eq!(crate::read::<u32>(&memory, 0x1004), None);
        assert_eq!(*memory.0.0.borrow(), [0x1000, 0x1004]);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                copy_calls: 2,
                spans: 2,
                bytes: 8,
                ..ReadCounts::ZERO
            }
        );
    }

    #[test]
    fn changed_controls_count_only_the_completed_nonempty_reads() {
        ReadCounts::take_current();
        control_fixture(
            &CountedMemory::new(SliceMemory {
                base: 0x1000,
                bytes: &[7; 8],
            }),
            0x1000,
        );
    }

    #[test]
    fn rejected_null_and_overflow_sources_include_only_the_requested_prefix() {
        ReadCounts::take_current();
        let memory = CountedMemory::new(SliceMemory {
            base: 0x1000,
            bytes: &[7; 64],
        });
        for rejected in [0, usize::MAX] {
            rejected_source_fixture(&memory, 0x1000, rejected);
        }
    }

    #[test]
    fn record_source_arithmetic_failure_counts_the_rejected_field_after_empty_fields() {
        ReadCounts::take_current();
        let memory = CountedMemory::new(SliceMemory {
            base: 0x1000,
            bytes: &[7; 8],
        });
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 4,
                len: 0,
            },
        ];
        let mut output = [0xa5; 8];
        assert_eq!(
            memory.copy_records(
                RecordSources::Strided {
                    base: 0x1000,
                    stride: usize::MAX - 0xfff,
                    count: 2
                },
                &RecordLayout::new(&fields, 4).unwrap(),
                &mut output,
            ),
            4
        );
        assert_eq!(output, [7, 7, 7, 7, 0xa5, 0xa5, 0xa5, 0xa5]);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                record_calls: 1,
                spans: 2,
                bytes: 8,
                ..ReadCounts::ZERO
            }
        );

        let fields = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 2,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 2,
                len: 0,
            },
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 2,
                len: 4,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 6,
                len: 2,
            },
        ];
        output.fill(0xa5);
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0x1000]),
                &RecordLayout::new(&fields, 8).unwrap(),
                &mut output,
            ),
            2
        );
        assert_eq!(output, [7, 7, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                record_calls: 1,
                spans: 2,
                bytes: 6,
                ..ReadCounts::ZERO
            }
        );
    }

    #[cfg(all(windows, target_arch = "x86_64", target_env = "msvc"))]
    #[test]
    fn native_batches_plans_faults_and_controls_match_scalar_accounting() {
        ReadCounts::take_current();
        let source = [7_u8; 64];
        // SAFETY: the owned source remains alive through every guarded read;
        // the unreadable null-page address is accessed only by the native guard.
        let memory = CountedMemory::new(unsafe { crate::memory::LiveMemory::new() });
        let base = source.as_ptr() as usize;
        healthy_fixture(&memory, base);
        assert_eq!(ReadCounts::take_current(), HEALTHY_COUNTS);
        fault_fixture(&memory, base);
        control_fixture(&memory, base);
        for rejected in [0, usize::MAX, 0x0000_8000_0000_0000] {
            rejected_source_fixture(&memory, base, rejected);
        }
        assert_eq!(crate::read::<u32>(&memory, base + 1), Some(0x0707_0707));
        assert_eq!(crate::read::<u32>(&memory, 1), None);
        assert_eq!(crate::read::<u32>(&memory, 0x0000_8000_0000_0000), None);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                copy_calls: 3,
                spans: 3,
                bytes: 12,
                ..ReadCounts::ZERO
            }
        );
        for rejected in [0, usize::MAX, usize::MAX - 3] {
            assert_eq!(crate::read::<u32>(&memory, rejected), None);
            assert_eq!(crate::read::<[u8; 0]>(&memory, rejected), Some([]));
        }
        assert_eq!(ReadCounts::take_current(), ReadCounts::ZERO);
    }

    #[test]
    fn empty_calls_and_invalid_destination_geometry_have_calls_without_source_work() {
        ReadCounts::take_current();
        let memory = CountedMemory::new(SliceMemory {
            base: 0x1000,
            bytes: &[7; 8],
        });
        assert!(memory.copy(0x1000, &mut []));
        assert!(!memory.copy(usize::MAX, &mut []));
        assert_eq!(
            memory.copy_scattered([0, usize::MAX], &mut [&mut [], &mut []]),
            2
        );
        assert_eq!(memory.copy_scattered_prefetched([], &mut []), 0);
        let empty = RecordLayout::new(&[], 0).unwrap();
        assert_eq!(
            memory.copy_records(RecordSources::Pointers(&[0, usize::MAX]), &empty, &mut []),
            0
        );
        let fields = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 1,
        }];
        let layout = RecordLayout::new(&fields, 1).unwrap();
        assert_eq!(
            memory.copy_records(RecordSources::Pointers(&[0x1000]), &layout, &mut []),
            0
        );
        let empty_fields = [RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        }; 2];
        let overflow_sources = RecordSources::Strided {
            base: 0,
            stride: usize::MAX,
            count: usize::MAX,
        };
        assert_eq!(
            memory.copy_records(
                overflow_sources,
                &RecordLayout::new(&empty_fields, 0).unwrap(),
                &mut []
            ),
            0
        );
        assert_eq!(
            memory.copy_records(
                overflow_sources,
                &RecordLayout::new(&empty_fields[..1], 0).unwrap(),
                &mut []
            ),
            usize::MAX
        );
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[0x1000; 2]),
                &RecordLayout::new(&fields, usize::MAX).unwrap(),
                &mut [0; 8],
            ),
            0
        );
        let empty_plan = PreparedPlan::new(vec![], vec![], vec![], 0).unwrap();
        assert_eq!(
            memory.execute_read_plan(&empty_plan, &mut []),
            Some(Outcome::Complete)
        );
        let empty_read = PreparedPlan::new(
            vec![Read {
                address: usize::MAX,
                destination_offset: 0,
                len: 0,
            }],
            vec![],
            vec![],
            0,
        )
        .unwrap();
        assert_eq!(
            memory.execute_read_plan(&empty_read, &mut []),
            Some(Outcome::Complete)
        );
        let plan = PreparedPlan::new(
            vec![Read {
                address: 0x1000,
                destination_offset: 0,
                len: 4,
            }],
            vec![],
            vec![],
            4,
        )
        .unwrap();
        assert_eq!(memory.execute_read_plan(&plan, &mut [0; 3]), None);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                copy_calls: 2,
                scattered_calls: 2,
                record_calls: 5,
                plan_calls: 3,
                ..ReadCounts::ZERO
            }
        );
    }

    #[test]
    fn unsupported_plan_execution_is_forwarded_and_charged_only_as_a_call() {
        struct Unsupported;
        impl Memory for Unsupported {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("unsupported plans must not fall back to scalar reads");
            }

            fn execute_read_plan(
                &self,
                plan: &PreparedPlan,
                destination: &mut [u8],
            ) -> Option<Outcome> {
                assert_eq!(plan.reads()[0].address, 0x1000);
                assert_eq!(destination, &[0xa5; 4]);
                None
            }
        }
        ReadCounts::take_current();
        let memory = CountedMemory::new(Unsupported);
        let plan = PreparedPlan::new(
            vec![Read {
                address: 0x1000,
                destination_offset: 0,
                len: 4,
            }],
            vec![],
            vec![],
            4,
        )
        .unwrap();
        assert_eq!(memory.execute_read_plan(&plan, &mut [0xa5; 4]), None);
        assert_eq!(
            ReadCounts::take_current(),
            ReadCounts {
                plan_calls: 1,
                ..ReadCounts::ZERO
            }
        );
    }

    #[test]
    fn cached_capabilities_forward_arguments_and_failures_without_source_charges() {
        use crate::{ccountry::AuthenticatedRoster, modifier_accumulator::Value};

        const VIEW: &[u8] = &[7, 8, 9];
        struct Capabilities;
        impl Memory for Capabilities {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("owned capabilities must not fall back to source reads");
            }

            fn immutable_view(&self, address: usize, len: usize) -> Option<SliceMemory<'_>> {
                if address != 0x1000 {
                    return None;
                }
                VIEW.get(..len).map(|bytes| SliceMemory {
                    base: address,
                    bytes,
                })
            }

            fn authenticated_countries(
                &self,
                base: usize,
                game_state: usize,
            ) -> Option<AuthenticatedRoster<'_>> {
                ((base, game_state) == (11, 12)).then_some(AuthenticatedRoster::new(
                    &[],
                    [crate::cpdx_array::Array { data: 0, count: 0 }; 2],
                ))
            }

            fn modifier_accumulator_value(
                &self,
                address: usize,
                key: u32,
            ) -> Option<Result<Value, &'static str>> {
                match (address, key) {
                    (7, 8) => Some(Ok(Value {
                        raw: i64::MIN,
                        present: true,
                    })),
                    (7, 9) => Some(Err("source modifier unavailable")),
                    _ => None,
                }
            }

            fn core_supply_access(
                &self,
                base: usize,
                game_state: usize,
                owner_raw: u32,
                area: usize,
            ) -> Option<Result<bool, &'static str>> {
                ((base, game_state, owner_raw, area) == (1, 2, 3, 4)).then_some(Ok(true))
            }

            fn movement_primary_root(
                &self,
                base: usize,
                member: usize,
            ) -> Option<Result<usize, &'static str>> {
                ((base, member) == (5, 6)).then_some(Err("source root unavailable"))
            }

            fn movement_air_defense_value(
                &self,
                defense: i64,
                denominator: i64,
                reduction: i64,
            ) -> Option<Result<i64, &'static str>> {
                ((defense, denominator, reduction) == (i64::MIN, 0, i64::MAX)).then_some(Ok(-7))
            }
        }

        ReadCounts::take_current();
        let memory = CountedMemory::new(Capabilities);
        let view = memory.immutable_view(0x1000, 3).unwrap();
        let mut bytes = [0; 3];
        assert!(view.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [7, 8, 9]);
        assert!(memory.immutable_view(0x1001, 3).is_none());
        assert!(memory.immutable_view(0x1000, 4).is_none());
        assert!(memory.authenticated_countries(11, 12).is_some());
        assert!(memory.authenticated_countries(12, 11).is_none());
        assert_eq!(
            memory.modifier_accumulator_value(7, 8),
            Some(Ok(Value {
                raw: i64::MIN,
                present: true
            }))
        );
        assert_eq!(
            memory.modifier_accumulator_value(7, 9),
            Some(Err("source modifier unavailable"))
        );
        assert_eq!(memory.modifier_accumulator_value(8, 8), None);
        assert_eq!(memory.core_supply_access(1, 2, 3, 4), Some(Ok(true)));
        assert_eq!(memory.core_supply_access(1, 2, 3, 5), None);
        assert_eq!(
            memory.movement_primary_root(5, 6),
            Some(Err("source root unavailable"))
        );
        assert_eq!(memory.movement_primary_root(6, 5), None);
        assert_eq!(
            memory.movement_air_defense_value(i64::MIN, 0, i64::MAX),
            Some(Ok(-7))
        );
        assert_eq!(
            memory.movement_air_defense_value(i64::MIN, 1, i64::MAX),
            None
        );
        assert_eq!(ReadCounts::take_current(), ReadCounts::ZERO);
    }

    #[test]
    fn requests_remain_local_to_the_thread_that_executed_them() {
        ReadCounts::take_current();
        let threads: Vec<_> = (1..=4)
            .map(|n| {
                std::thread::spawn(move || {
                    ReadCounts::take_current();
                    let memory = CountedMemory::new(SliceMemory {
                        base: 0x1000,
                        bytes: &[7; 64],
                    });
                    for _ in 0..n {
                        healthy_fixture(&memory, 0x1000);
                    }
                    ReadCounts::take_current()
                })
            })
            .collect();
        for (i, thread) in threads.into_iter().enumerate() {
            let mut expected = ReadCounts::ZERO;
            for _ in 0..=i {
                expected += HEALTHY_COUNTS;
            }
            assert_eq!(thread.join().unwrap(), expected);
        }
        assert_eq!(ReadCounts::take_current(), ReadCounts::ZERO);
    }

    #[test]
    fn draining_and_restoring_nested_work_preserves_counts_after_unwind() {
        ReadCounts::take_current();
        let memory = CountedMemory::new(SliceMemory {
            base: 0x1000,
            bytes: &[7; 64],
        });
        assert!(memory.copy(0x1000, &mut [0; 4]));
        let suspended = ReadCounts::take_current();
        assert!(
            std::panic::catch_unwind(|| {
                healthy_fixture(
                    &CountedMemory::new(SliceMemory {
                        base: 0x1000,
                        bytes: &[7; 64],
                    }),
                    0x1000,
                );
                panic!("synthetic job stops after its reads");
            })
            .is_err()
        );
        let nested = ReadCounts::take_current();
        assert_eq!(nested, HEALTHY_COUNTS);
        suspended.add_current();
        // The earlier wrapper must still address the Cell drained by the
        // nested wrapper and restored after its unwind.
        assert!(memory.copy(0x1000, &mut [0; 4]));
        let mut expected = suspended;
        expected += suspended;
        assert_eq!(ReadCounts::take_current(), expected);
        assert_eq!(ReadCounts::take_current(), ReadCounts::ZERO);
    }

    #[test]
    fn addition_and_tls_preserve_large_integers_and_saturate_every_counter() {
        ReadCounts::take_current();
        assert_eq!(ReadCounts::default(), ReadCounts::ZERO);
        let first = ReadCounts {
            copy_calls: (1 << 53) + 1,
            scattered_calls: u64::MAX - 2,
            record_calls: u64::MAX,
            plan_calls: (1 << 53) + 3,
            spans: u64::MAX - 7,
            bytes: u64::MAX - 8,
        };
        let delta = ReadCounts {
            copy_calls: 1,
            scattered_calls: 4,
            record_calls: 1,
            plan_calls: 2,
            spans: 8,
            bytes: 8,
        };
        let expected = ReadCounts {
            copy_calls: (1 << 53) + 2,
            scattered_calls: u64::MAX,
            record_calls: u64::MAX,
            plan_calls: (1 << 53) + 5,
            spans: u64::MAX,
            bytes: u64::MAX,
        };
        let mut total = first;
        total += delta;
        assert_eq!(total, expected);
        first.add_current();
        delta.add_current();
        assert_eq!(ReadCounts::take_current(), expected);
        let maximum = ReadCounts {
            copy_calls: u64::MAX,
            scattered_calls: u64::MAX,
            record_calls: u64::MAX,
            plan_calls: u64::MAX,
            spans: u64::MAX,
            bytes: u64::MAX,
        };
        total += maximum;
        assert_eq!(total, maximum);
        maximum.add_current();
        delta.add_current();
        assert_eq!(ReadCounts::take_current(), maximum);
    }
}
