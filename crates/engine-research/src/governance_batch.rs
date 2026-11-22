use crate::batch_fallback::{self, Batch};
use crate::memory::{Memory, RecordLayout, RecordSources};

pub struct Records<'a> {
    layout: &'a RecordLayout<'a>,
    index: Vec<(usize, usize)>,
    bytes: Vec<u8>,
}

impl<'a> Records<'a> {
    pub fn gather(
        memory: &impl Memory,
        batch: Batch,
        sources: RecordSources<'_>,
        layout: &'a RecordLayout<'a>,
    ) -> Self {
        let mut records = Self {
            layout,
            index: Vec::new(),
            bytes: Vec::new(),
        };
        if sources.is_empty() {
            return records;
        }
        let prepared = crate::allocation::advisory(|| {
            let len = sources.len().checked_mul(layout.stride())?;
            sources.len().checked_mul(layout.field_count())?;
            crate::allocation::reserve_exact(&mut records.index, sources.len()).ok()?;
            crate::allocation::reserve_exact(&mut records.bytes, len).ok()?;
            for row in 0..sources.len() {
                records.index.push((sources.pointer(row)?, row));
            }
            records.bytes.resize(len, 0);
            Some(())
        });
        if prepared.is_none() {
            return records;
        }
        let copied = memory.copy_records(sources, layout, &mut records.bytes);
        let done = if layout.field_count() == 0 {
            sources.len()
        } else {
            (copied / layout.field_count()).min(sources.len())
        };
        if done != sources.len() {
            batch_fallback::record(batch);
        }
        records.index.truncate(done);
        records.index.sort_unstable();
        records.index.dedup_by_key(|(pointer, _)| *pointer);
        records
    }

    pub(crate) fn get<const N: usize>(&self, address: usize) -> Option<[u8; N]> {
        self.field(address, N)?.try_into().ok()
    }

    pub fn targets(&self, offset: usize) -> Vec<usize> {
        if self.index.is_empty() {
            return Vec::new();
        }
        crate::allocation::advisory(|| {
            let mut targets = Vec::new();
            crate::allocation::reserve_exact(&mut targets, self.index.len()).ok()?;
            for &(pointer, _) in &self.index {
                if let Some(raw) = pointer.checked_add(offset).and_then(|at| self.get::<8>(at))
                    && let Ok(target) = usize::try_from(u64::from_le_bytes(raw))
                    && target != 0
                {
                    targets.push(target);
                }
            }
            Some(targets)
        })
        .unwrap_or_default()
    }

    fn field(&self, address: usize, len: usize) -> Option<&[u8]> {
        self.span(address)?.get(..len)
    }

    fn span(&self, address: usize) -> Option<&[u8]> {
        if self.index.is_empty() {
            return None;
        }
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let Some(pointer) = address.checked_sub(field.source_offset) else {
                continue;
            };
            let upper = self.index.partition_point(|&(at, _)| at <= pointer);
            let Some(&(at, row)) = upper.checked_sub(1).and_then(|i| self.index.get(i)) else {
                continue;
            };
            let Some(offset) = at
                .checked_add(field.source_offset)
                .and_then(|start| address.checked_sub(start))
            else {
                continue;
            };
            if offset < field.len {
                let start = row * self.layout.stride() + field.destination_offset + offset;
                return self.bytes.get(start..start + field.len - offset);
            }
        }
        None
    }

    fn next_cached(&self, address: usize, limit: usize) -> Option<usize> {
        if self.index.is_empty() || limit == 0 {
            return None;
        }
        let mut best: Option<usize> = None;
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let next = match address.checked_sub(field.source_offset) {
                Some(pointer) => self.index.partition_point(|&(at, _)| at <= pointer),
                None => 0,
            };
            let Some(&(at, _)) = self.index.get(next) else {
                continue;
            };
            let Some(start) = at.checked_add(field.source_offset) else {
                continue;
            };
            let distance = start - address;
            if distance < limit {
                best = Some(best.map_or(distance, |current| current.min(distance)));
            }
        }
        best
    }

    pub fn view<'b, M: Memory>(&'b self, source: &'b M) -> View<'b, 'a, M> {
        View {
            source,
            records: std::slice::from_ref(self),
        }
    }
}

pub struct View<'b, 'a, M> {
    source: &'b M,
    records: &'b [Records<'a>],
}

impl<M> View<'_, '_, M> {
    fn span(&self, address: usize) -> Option<&[u8]> {
        self.records.iter().find_map(|records| records.span(address))
    }

    fn next_cached(&self, address: usize, limit: usize) -> Option<usize> {
        self.records
            .iter()
            .filter_map(|records| records.next_cached(address, limit))
            .min()
    }

    fn intersects_cache(&self, address: usize, len: usize) -> bool {
        len != 0 && (self.span(address).is_some() || self.next_cached(address, len).is_some())
    }
}

impl<M: Memory> Memory for View<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return self.source.copy(address, destination);
        }
        let mut copied = 0;
        while copied < destination.len() {
            let Some(at) = address.checked_add(copied) else {
                return false;
            };
            if let Some(bytes) = self.span(at) {
                let count = bytes.len().min(destination.len() - copied);
                destination[copied..copied + count].copy_from_slice(&bytes[..count]);
                copied += count;
            } else {
                let remaining = destination.len() - copied;
                let count = self.next_cached(at, remaining).unwrap_or(remaining);
                if !self
                    .source
                    .copy(at, &mut destination[copied..copied + count])
                {
                    return false;
                }
                copied += count;
            }
        }
        true
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if !addresses
            .iter()
            .zip(destinations.iter())
            .any(|(&at, out)| self.intersects_cache(at, out.len()))
        {
            return self.source.copy_scattered(addresses, destinations);
        }
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if destination.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(destination.len()).is_none()
                || !self.copy(address, destination)
            {
                return completed;
            }
        }
        N
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        if count == 0 {
            return 0;
        }
        let cached = (0..sources.len()).any(|row| {
            sources.pointer(row).is_some_and(|pointer| {
                pointer != 0
                    && layout.fields().iter().any(|field| {
                        field.len != 0
                            && pointer
                                .checked_add(field.source_offset)
                                .is_some_and(|at| self.intersects_cache(at, field.len))
                    })
            })
        });
        if !cached {
            return self.source.copy_records(sources, layout, destination);
        }
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                        .filter(|&address| address.checked_add(field.len).is_some())
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.copy(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
}

const POINTER_FIELDS: [crate::memory::RecordField; 1] = crate::memory::packed_fields([(0, 8)]);
pub const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELDS, 8).expect("valid pointer layout");

const ASSIGNED_FIELDS: [crate::memory::RecordField; 7] = [
    crate::ctechnology::FIELDS[0],
    crate::ctechnology::FIELDS[1],
    crate::ctechnology::FIELDS[2],
    crate::ctechnology::FIELDS[3],
    crate::ctechnology::FIELDS[4],
    crate::ctechnology::FIELDS[5],
    crate::memory::RecordField {
        source_offset: crate::ctechnology::APPLIED_SPEED_BONUS,
        destination_offset: 56,
        len: 24,
    },
];
const ASSIGNED_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ASSIGNED_FIELDS, 80).unwrap();
const TECHNOLOGY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&crate::ctechnology::FIELDS, 56).unwrap();
const TEMPLATE_FIELDS: [crate::memory::RecordField; 3] = crate::memory::packed_fields([
    (crate::ctechnology_template::ID, 32),
    (crate::ctechnology_template::DATABASE_INDEX, 8),
    (crate::ctechnology_template::YEAR, 16),
]);
const TEMPLATE_LAYOUT: RecordLayout<'static> = RecordLayout::new(&TEMPLATE_FIELDS, 56).unwrap();
const IDENTITY_FIELDS: [crate::memory::RecordField; 1] =
    crate::memory::packed_fields([(crate::ctechnology_template::DATABASE_INDEX, 8)]);
const IDENTITY_LAYOUT: RecordLayout<'static> = RecordLayout::new(&IDENTITY_FIELDS, 8).unwrap();

pub(crate) struct Assignments([Records<'static>; 4]);
impl Assignments {
    pub(crate) fn gather(
        memory: &impl Memory,
        roster: crate::cpdx_array::Array,
        len: usize,
        inputs: bool,
    ) -> Self {
        let pointers = Records::gather(
            memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: roster.data,
                stride: 8,
                count: len,
            },
            &POINTER_LAYOUT,
        );
        let targets = pointers.targets(0);
        let slots = Records::gather(
            memory,
            Batch::GovernanceSlots,
            RecordSources::Pointers(&targets),
            &crate::cresearch_slot::LAYOUT,
        );
        let targets = slots.targets(crate::cresearch_slot::TECHNOLOGY);
        let technologies = Records::gather(
            memory,
            Batch::GovernanceAssignedTechnologies,
            RecordSources::Pointers(&targets),
            if inputs {
                &ASSIGNED_LAYOUT
            } else {
                &TECHNOLOGY_LAYOUT
            },
        );
        let targets = technologies.targets(crate::ctechnology::TEMPLATE);
        let templates = Records::gather(
            memory,
            Batch::GovernanceAssignedTemplates,
            RecordSources::Pointers(&targets),
            if inputs {
                &TEMPLATE_LAYOUT
            } else {
                &IDENTITY_LAYOUT
            },
        );
        Self([pointers, slots, technologies, templates])
    }
    pub(crate) fn view<'a, M: Memory>(&'a self, source: &'a M) -> View<'a, 'static, M> {
        View {
            source,
            records: &self.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{RecordField, SliceMemory};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    const BASE: usize = 0x1000;

    struct CountedFault<'a> {
        inner: SliceMemory<'a>,
        fault: Option<usize>,
        once: bool,
        spent: Cell<bool>,
        reads: RefCell<HashMap<usize, usize>>,
    }

    impl<'a> CountedFault<'a> {
        fn new(bytes: &'a [u8], fault: Option<usize>, once: bool) -> Self {
            Self {
                inner: SliceMemory { base: BASE, bytes },
                fault,
                once,
                spent: Cell::new(false),
                reads: RefCell::new(HashMap::new()),
            }
        }

        fn reads_at(&self, address: usize) -> usize {
            self.reads.borrow().get(&address).copied().unwrap_or(0)
        }
    }

    impl Memory for CountedFault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            {
                let mut reads = self.reads.borrow_mut();
                for offset in 0..destination.len() {
                    *reads.entry(address + offset).or_insert(0) += 1;
                }
            }
            if self
                .fault
                .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
                && (!self.once || !self.spent.get())
            {
                if self.once {
                    self.spent.set(true);
                }
                destination.fill(0xff);
                return false;
            }
            self.inner.copy(address, destination)
        }
    }

    fn put(bytes: &mut [u8], address: usize, value: u64) {
        let at = address - BASE;
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn partial_batch_keeps_completed_rows_retries_faulted_once_and_reads_pages_piecewise() {
        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x1000, 0x1200);
        put(&mut bytes, 0x1008, 0x1300);
        put(&mut bytes, 0x1010, 0x1400);

        let memory = CountedFault::new(&bytes, Some(0x1008), true);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: 0x1000,
                stride: 8,
                count: 3,
            },
            &POINTER_LAYOUT,
        );
        assert_eq!(pointers.targets(0), vec![0x1200]);
        let view = pointers.view(&memory);
        let mut page = [0_u8; 24];
        assert!(view.copy(0x1000, &mut page));
        assert_eq!(&page, &bytes[..24]);
        assert_eq!(memory.reads_at(0x1000), 1);
        assert_eq!(memory.reads_at(0x1008), 2);
        assert_eq!(memory.reads_at(0x1010), 1);

        let memory = CountedFault::new(&bytes, Some(0x1008), false);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: 0x1000,
                stride: 8,
                count: 3,
            },
            &POINTER_LAYOUT,
        );
        assert_eq!(pointers.targets(0), vec![0x1200]);
        let view = pointers.view(&memory);
        let mut page = [0_u8; 24];
        assert!(!view.copy(0x1000, &mut page));
        assert_eq!(memory.reads_at(0x1000), 1);
        assert_eq!(memory.reads_at(0x1008), 2);

        let memory = CountedFault::new(&bytes, Some(0x1000), true);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1008, 0x1000]),
            &POINTER_LAYOUT,
        );
        let view = pointers.view(&memory);
        let mut page = [0_u8; 16];
        assert!(view.copy(0x1000, &mut page));
        assert_eq!(&page, &bytes[..16]);
        assert_eq!(memory.reads_at(0x1008), 1);
        assert_eq!(memory.reads_at(0x1000), 2);
    }

    #[test]
    fn partial_batch_alias_lookup_uses_earliest_completed_duplicate_and_scattered_keeps_cached() {
        const PAIR_FIELDS: [RecordField; 2] = crate::memory::packed_fields([(0, 8), (0x20, 8)]);
        const PAIR_LAYOUT: RecordLayout<'static> = RecordLayout::new(&PAIR_FIELDS, 16).unwrap();

        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x1200, 11);
        put(&mut bytes, 0x1220, 22);
        put(&mut bytes, 0x1300, 33);
        put(&mut bytes, 0x1320, 44);

        for once in [true, false] {
            let memory = CountedFault::new(&bytes, Some(0x1320), once);
            let records = Records::gather(
                &memory,
                Batch::GovernanceAssignedTechnologies,
                RecordSources::Pointers(&[0x1200, 0x1300, 0x1200]),
                &PAIR_LAYOUT,
            );
            let view = records.view(&memory);
            let mut a0 = [0_u8; 8];
            let mut a1 = [0_u8; 8];
            assert_eq!(view.copy_scattered([0x1200, 0x1220], &mut [&mut a0, &mut a1]), 2);
            assert_eq!(u64::from_le_bytes(a0), 11);
            assert_eq!(u64::from_le_bytes(a1), 22);
            assert_eq!(memory.reads_at(0x1200), 1);
            assert_eq!(memory.reads_at(0x1220), 1);

            let mut mixed_a = [0_u8; 8];
            let mut mixed_b = [0_u8; 8];
            let copied = view.copy_scattered([0x1200, 0x1320], &mut [&mut mixed_a, &mut mixed_b]);
            assert_eq!(copied, if once { 2 } else { 1 });
            assert_eq!(u64::from_le_bytes(mixed_a), 11);
            if once {
                assert_eq!(u64::from_le_bytes(mixed_b), 44);
            }
            assert_eq!(memory.reads_at(0x1200), 1);
            assert_eq!(memory.reads_at(0x1320), 2);

            let mut out = [0_u8; 16];
            assert_eq!(
                view.copy_records(RecordSources::Pointers(&[0x1200]), &PAIR_LAYOUT, &mut out),
                2
            );
            assert_eq!(memory.reads_at(0x1200), 1);
            assert_eq!(memory.reads_at(0x1220), 1);
        }
    }
}
