//! Pass-local exact-field governance batches. A failed optional gather is
//! discarded; the original reader then makes its one serial pass, retaining
//! its checks, independent table boundaries and persistent-fault reasons.
use crate::batch_fallback::{self, Batch};
use crate::memory::{Memory, RecordLayout, RecordSources};

/// Advisory storage follows the observed native count, without a population
/// cap. Allocation failure declines batching and leaves the serial reader.
/// Each layout describes fields of one object, never a span across objects.
pub(crate) struct Records<'a> {
    layout: &'a RecordLayout<'a>,
    // Sorted address -> original record ordinal. Duplicate observations keep
    // the same healthy bytes; healthy memory stands still within the acquisition lease.
    index: Vec<(usize, usize)>,
    bytes: Vec<u8>,
    complete: bool,
}

impl<'a> Records<'a> {
    pub(crate) fn gather(
        memory: &impl Memory,
        batch: Batch,
        sources: RecordSources<'_>,
        layout: &'a RecordLayout<'a>,
    ) -> Self {
        let mut records = Self {
            layout,
            index: Vec::new(),
            bytes: Vec::new(),
            complete: false,
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
                // Invalid geometry declines speculation; the original reader
                // reports its own overflow at its original validation point.
                records.index.push((sources.pointer(row)?, row));
            }
            records.bytes.resize(len, 0);
            Some(())
        });
        if prepared.is_none() {
            return records;
        }
        records.complete = memory.copy_records(sources, layout, &mut records.bytes)
            == sources.len() * layout.field_count();
        if !records.complete {
            // No batch retry, no partial bytes served, and no eager fallback.
            // Only the serial reader's original calls are made from here.
            batch_fallback::record(batch);
        }
        records.index.sort_unstable();
        records
    }

    pub(crate) fn complete(&self) -> bool {
        self.complete
    }

    pub(crate) fn get<const N: usize>(&self, address: usize) -> Option<[u8; N]> {
        self.field(address, N)?.try_into().ok()
    }

    /// Nonnull pointers obtained only from complete copied object fields.
    /// A storage miss declines child speculation, preserving the serial pass.
    pub(crate) fn targets(&self, offset: usize) -> Vec<usize> {
        if !self.complete() {
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
        if !self.complete {
            return None;
        }
        for field in self.layout.fields() {
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

    fn copy_cached(&self, address: usize, destination: &mut [u8]) -> bool {
        if !self.complete {
            return false;
        }
        let mut copied = 0;
        while copied < destination.len() {
            let Some(at) = address.checked_add(copied) else {
                return false;
            };
            let Some(bytes) = self.span(at) else {
                return false;
            };
            let count = bytes.len().min(destination.len() - copied);
            destination[copied..copied + count].copy_from_slice(&bytes[..count]);
            copied += count;
        }
        true
    }

    pub(crate) fn view<'b, M: Memory>(&'b self, source: &'b M) -> View<'b, 'a, M> {
        View {
            source,
            records: std::slice::from_ref(self),
        }
    }
}

pub(crate) struct View<'b, 'a, M> {
    source: &'b M,
    records: &'b [Records<'a>],
}

impl<M> View<'_, '_, M> {
    fn field(&self, address: usize, len: usize) -> Option<&[u8]> {
        self.records
            .iter()
            .find_map(|records| records.field(address, len))
    }
}

impl<M: Memory> Memory for View<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self
            .records
            .iter()
            .any(|records| records.copy_cached(address, destination))
        {
            true
        } else {
            self.source.copy(address, destination)
        }
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if addresses
            .iter()
            .zip(destinations.iter())
            .all(|(&at, out)| out.is_empty() || self.field(at, out.len()).is_some())
        {
            for (&at, out) in addresses.iter().zip(destinations.iter_mut()) {
                if !out.is_empty() {
                    out.copy_from_slice(self.field(at, out.len()).unwrap());
                }
            }
            N
        } else {
            self.source.copy_scattered(addresses, destinations)
        }
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
        let cached = (0..sources.len()).all(|row| {
            layout.fields().iter().all(|field| {
                field.len == 0
                    || sources
                        .pointer(row)
                        .and_then(|p| p.checked_add(field.source_offset))
                        .is_some_and(|at| self.field(at, field.len).is_some())
            })
        });
        if !cached {
            return self.source.copy_records(sources, layout, destination);
        }
        for row in 0..sources.len() {
            for field in layout.fields().iter().filter(|field| field.len != 0) {
                let at = sources.pointer(row).unwrap() + field.source_offset;
                let start = row * layout.stride() + field.destination_offset;
                destination[start..start + field.len]
                    .copy_from_slice(self.field(at, field.len).unwrap());
            }
        }
        count
    }
}

const POINTER_FIELDS: [crate::memory::RecordField; 1] = crate::memory::packed_fields([(0, 8)]);
pub(crate) const POINTER_LAYOUT: RecordLayout<'static> =
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
// Research update and serialization establish the selected template identity;
// cost calculation consumes its authored year, level and base cost.
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
#[path = "governance_batch/test_kit.rs"]
pub(crate) mod test_kit;
