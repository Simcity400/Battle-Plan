use super::*;
use std::cell::Cell;

#[derive(Default)]
struct ReferenceOrder {
    baseline: Option<u64>,
    entries: Vec<(u64, usize)>,
}
impl ReferenceOrder {
    fn clear(&mut self) {
        self.baseline = None;
        self.entries.clear();
    }
}
impl<T: Clone> Store<T> {
    fn reference_held(
        &self,
        index: usize,
        seed: &DivisionSeed,
        certificate: (u64, u64),
        order: &mut ReferenceOrder,
    ) -> Option<&Held<T>> {
        let at = if self
            .held
            .get(index)
            .is_some_and(|held| held.division == seed.division)
        {
            Some(index)
        } else {
            if self.held.is_empty() {
                return None;
            }
            if self.baseline.is_none() || order.baseline != self.baseline {
                order.clear();
                if engine_allocation::advisory(|| {
                    engine_allocation::reserve(&mut order.entries, self.held.len())
                })
                .is_ok()
                {
                    order.entries.extend(
                        self.held
                            .iter()
                            .enumerate()
                            .map(|(at, held)| (held.division, at)),
                    );
                    order.entries.sort_unstable_by_key(|entry| entry.0);
                    order.baseline = self.baseline;
                } else {
                    return self.held.iter().find(|held| {
                        held.division == seed.division
                            && held.army == seed.pointer
                            && self.matches(held, certificate)
                    });
                }
            }
            order
                .entries
                .binary_search_by_key(&seed.division, |entry| entry.0)
                .ok()
                .map(|at| order.entries[at].1)
        };
        at.and_then(|at| self.held.get(at))
            .filter(|held| held.division == seed.division)
            .filter(|held| held.army == seed.pointer && self.matches(held, certificate))
    }

    fn reference_value(
        &self,
        base: usize,
        index: usize,
        seed: &DivisionSeed,
        certificate: Option<(u64, u64)>,
        order: &mut ReferenceOrder,
        read: impl FnOnce() -> Result<T, &'static str>,
    ) -> Option<Result<T, &'static str>> {
        let certificate = certificate?;
        if let Some(held) = self.reference_held(index, seed, certificate, order) {
            return Some(Ok(held.value.clone()));
        }
        if super::identity(base, seed).is_err() {
            return None;
        }
        Some(read())
    }
}

#[test]
fn shifted_scalar_ranges_match_the_original_lookup_and_faults() {
    const COUNT: usize = 128;
    let build = |duplicate, daily, baseline| Store {
        held: (0..COUNT)
            .map(|index| Held {
                division: (if duplicate && index > 63 {
                    index - 64
                } else {
                    index
                }) as u64
                    + 1,
                army: index * 0x1000,
                certificate: (1, index as u64 % 3),
                value: (index as i64).wrapping_mul(i64::MAX),
                current: true,
            })
            .collect(),
        daily,
        baseline,
        ..Default::default()
    };
    for duplicate in [false, true] {
        for daily in [false, true] {
            for denied in [false, true] {
                let mut current_order = Order::default();
                let mut old_order = ReferenceOrder::default();
                for baseline in [Some(7), Some(8), None] {
                    let current = build(duplicate, daily, baseline);
                    let old = build(duplicate, daily, baseline);
                    for shift in [0, 1, 17, 64, 127] {
                        for index in 0..COUNT + 1 {
                            let at = (index + shift) % (COUNT + 1);
                            for certificate in [None, Some((1, 0)), Some((1, 2)), Some((2, 0))] {
                                for fault in [false, true] {
                                    let seed = DivisionSeed {
                                        division: at as u64 + 1,
                                        pointer: at * 0x1000,
                                        handle: Some(at as u64 + 1),
                                        class: Some(0),
                                        primary_vtable: Some(carmy::VTABLE_RVA.0 as u64),
                                        secondary_vtable: Some(
                                            carmy::SECONDARY_VTABLE_RVA.0 as u64,
                                        ),
                                        ..Default::default()
                                    };
                                    let old_reads = Cell::new(0);
                                    let reads = Cell::new(0);
                                    let read = |counter: &Cell<usize>| {
                                        counter.set(counter.get() + 1);
                                        if fault {
                                            Err("synthetic scalar fault")
                                        } else {
                                            Ok(i64::MIN)
                                        }
                                    };
                                    let compare = || {
                                        let expected = old.reference_value(
                                            0,
                                            index,
                                            &seed,
                                            certificate,
                                            &mut old_order,
                                            || read(&old_reads),
                                        );
                                        let actual = current.prefetch_value(
                                            0,
                                            index,
                                            &seed,
                                            certificate,
                                            &mut current_order,
                                            || read(&reads),
                                        );
                                        assert_eq!(actual, expected);
                                        assert_eq!(reads.get(), old_reads.get());
                                    };
                                    if denied {
                                        collector_test_support::with_allocations_denied(compare);
                                    } else {
                                        let mut compare = compare;
                                        compare();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
