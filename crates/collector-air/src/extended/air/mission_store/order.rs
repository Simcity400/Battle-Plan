use super::{Certificate, Entry};
use engine_allocation as allocation;
use engine_native_memory::{Memory, cpdx_array};
use engine_native_air::{
    cair_regional_mission::{Registration, WingIdentity},
    cstrategic_air,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderLabel {
    pub wing: u64,
    pub label: u64,
}

struct OrderedRoot {
    certificate: Certificate,
    members: Vec<OrderLabel>,
    known: bool,
}

fn same_order_certificate(left: Certificate, right: Certificate) -> bool {
    let at = cstrategic_air::ALL_ACTIVE_MISSIONS - cstrategic_air::REGIONAL_MISSIONS;
    left.epoch == right.epoch
        && left.sequence == right.sequence
        && left.root.address == right.root.address
        && left.root.canonical_slot == right.root.canonical_slot
        && left.root.headers[at..at + 16] == right.root.headers[at..at + 16]
}

#[derive(Default)]
pub(super) struct Orders {
    roots: Vec<OrderedRoot>,
    scratch: Vec<OrderLabel>,
    expected: Vec<usize>,
    all: Vec<OrderLabel>,
    dirty: bool,
}

impl Orders {
    pub(super) fn clear(&mut self) {
        self.roots.clear();
        self.scratch.clear();
        self.expected.clear();
        self.all.clear();
        self.dirty = false;
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.roots.capacity() * size_of::<OrderedRoot>()
            + self.scratch.capacity() * size_of::<OrderLabel>()
            + self.expected.capacity() * size_of::<usize>()
            + self.all.capacity() * size_of::<OrderLabel>()
            + self
                .roots
                .iter()
                .map(|r| r.members.capacity() * size_of::<OrderLabel>())
                .sum::<usize>()
    }

    pub(super) fn capture(
        &mut self,
        memory: &impl Memory,
        wings: &[WingIdentity],
        entries: &[Entry],
        registrations: &[Registration],
        output: &mut Vec<OrderLabel>,
    ) {
        let previous = self.roots.len();
        self.roots.retain(|root| {
            entries
                .binary_search_by_key(&root.certificate.root.address, |e| {
                    e.certificate.root.address
                })
                .is_ok()
        });
        self.dirty |= previous != self.roots.len();
        for entry in entries {
            let certificate = entry.certificate;
            let address = certificate.root.address;
            let index = match self
                .roots
                .binary_search_by_key(&address, |r| r.certificate.root.address)
            {
                Ok(index) => index,
                Err(index) => {
                    if allocation::advisory(|| allocation::reserve(&mut self.roots, 1)).is_err() {
                        continue;
                    }
                    self.roots.insert(
                        index,
                        OrderedRoot {
                            certificate,
                            members: Vec::new(),
                            known: false,
                        },
                    );
                    self.dirty = true;
                    self.read(
                        index,
                        memory,
                        wings,
                        certificate,
                        &registrations[entry.registrations.clone()],
                    );
                    index
                }
            };
            if !same_order_certificate(self.roots[index].certificate, certificate) {
                self.dirty = true;
                self.read(
                    index,
                    memory,
                    wings,
                    certificate,
                    &registrations[entry.registrations.clone()],
                );
            }
        }
        if self.dirty {
            self.all.clear();
            let len = self
                .roots
                .iter()
                .filter(|root| root.known)
                .try_fold(0usize, |sum, root| sum.checked_add(root.members.len()));
            let Some(len) = len else {
                return;
            };
            if allocation::advisory(|| allocation::reserve(&mut self.all, len)).is_err() {
                return;
            }
            self.all.extend(
                self.roots
                    .iter()
                    .filter(|root| root.known)
                    .flat_map(|root| root.members.iter().copied()),
            );
            self.all.sort_unstable_by_key(|row| row.wing);
            self.dirty = false;
        }
        if allocation::advisory(|| allocation::reserve(output, self.all.len())).is_ok() {
            output.extend_from_slice(&self.all);
        }
    }

    fn read(
        &mut self,
        index: usize,
        memory: &impl Memory,
        wings: &[WingIdentity],
        certificate: Certificate,
        registrations: &[Registration],
    ) {
        let root = &mut self.roots[index];
        root.known = false;
        root.certificate = certificate;
        self.scratch.clear();
        self.expected.clear();
        let at = cstrategic_air::ALL_ACTIVE_MISSIONS - cstrategic_air::REGIONAL_MISSIONS;
        let Some(array) = cpdx_array::Array::from_header(
            certificate.root.headers[at..at + 16]
                .try_into()
                .expect("all-active header"),
            8,
        ) else {
            return;
        };
        if array.count as usize != registrations.len()
            || allocation::advisory(|| allocation::reserve(&mut self.expected, registrations.len()))
                .is_err()
        {
            return;
        }
        self.expected
            .extend(registrations.iter().map(|row| row.pointer));
        self.expected.sort_unstable();
        if allocation::advisory(|| allocation::reserve(&mut self.scratch, array.count as usize))
            .is_err()
        {
            return;
        }
        let read = cpdx_array::pointers(
            memory,
            array,
            "Air all-active order vector unreadable",
            |_, pointer| {
                if self.expected.binary_search(&pointer).is_err() {
                    return Err(
                        "Air all-active order mission is absent from regional registrations",
                    );
                }
                let wing = wings
                    .binary_search_by_key(&pointer, |wing| wing.mission_pointer)
                    .ok()
                    .map(|at| &wings[at])
                    .ok_or("Air all-active order mission is absent from wing census")?;
                if wing.owner_slot != certificate.root.canonical_slot {
                    return Err("Air all-active order mission belongs to another country");
                }
                let label = root
                    .members
                    .binary_search_by_key(&wing.wing, |row| row.wing)
                    .ok()
                    .map_or(0, |at| root.members[at].label);
                self.scratch.push(OrderLabel {
                    wing: wing.wing,
                    label,
                });
                Ok(())
            },
        );
        if read.is_err() {
            return;
        }
        assign_labels(&mut self.scratch);
        self.scratch.sort_unstable_by_key(|row| row.wing);
        if self
            .scratch
            .windows(2)
            .any(|pair| pair[0].wing == pair[1].wing)
        {
            return;
        }
        std::mem::swap(&mut root.members, &mut self.scratch);
        root.known = true;
    }
}

fn assign_labels(rows: &mut [OrderLabel]) {
    const SPACE: u64 = 1 << 16;
    let mut left = 0;
    let mut first = 0;
    while first < rows.len() {
        if rows[first].label > left {
            left = rows[first].label;
            first += 1;
            continue;
        }
        let mut end = first + 1;
        while end < rows.len() && rows[end].label.saturating_sub(left) <= (end - first) as u64 {
            end += 1;
        }
        let count = (end - first) as u64;
        let step = if end < rows.len() {
            (rows[end].label - left) / (count + 1)
        } else {
            SPACE.min((u64::MAX - left) / count)
        };
        if step == 0 {
            let step = u64::MAX / (rows.len() as u64 + 1);
            for (at, row) in rows.iter_mut().enumerate() {
                row.label = step * (at as u64 + 1);
            }
            return;
        }
        for row in &mut rows[first..end] {
            left += step;
            row.label = left;
        }
        first = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removals_preserve_labels_and_insertions_use_the_gap() {
        let mut rows: Vec<_> = [0, 0, 0, 0]
            .into_iter()
            .enumerate()
            .map(|(at, label)| OrderLabel {
                wing: at as u64 + 1,
                label,
            })
            .collect();
        assign_labels(&mut rows);
        let original = rows.clone();
        rows.remove(1);
        assign_labels(&mut rows);
        assert_eq!(rows, [original[0], original[2], original[3]]);
        rows.insert(1, OrderLabel { wing: 5, label: 0 });
        assign_labels(&mut rows);
        assert_eq!(rows[0], original[0]);
        assert_eq!(&rows[2..], &original[2..]);
        assert!(rows.windows(2).all(|p| p[0].label < p[1].label));
    }

    #[test]
    fn reordered_or_exhausted_labels_still_prove_the_exact_vector_order() {
        for labels in [vec![30, 20, 10], vec![0, 1, 2], vec![u64::MAX, 0]] {
            let mut rows: Vec<_> = labels
                .into_iter()
                .enumerate()
                .map(|(at, label)| OrderLabel {
                    wing: at as u64,
                    label,
                })
                .collect();
            assign_labels(&mut rows);
            assert!(rows[0].label > 0);
            assert!(rows.windows(2).all(|p| p[0].label < p[1].label));
            assert_eq!(
                rows.iter().map(|r| r.wing).collect::<Vec<_>>(),
                (0..rows.len() as u64).collect::<Vec<_>>()
            );
        }
    }
}
