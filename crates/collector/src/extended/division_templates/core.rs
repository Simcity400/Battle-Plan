//! Certified icon-core segments. The existing census still authenticates every
//! roster occurrence and reads its complete fixed record and literal name.
use super::*;
use crate::template_changes::Changes;

type Stamp = (u64, u64);
type Certificate = [Stamp; 3];

#[derive(Default, PartialEq, Eq)]
struct Core {
    row: native::IconRow,
    bytes: Vec<u8>,
    counts: Vec<native::SubunitCount>,
    variants: Vec<u64>,
}

fn append<T: Copy>(output: &mut Vec<T>, source: &[T], span: &mut native::Span) -> Option<()> {
    let values = source.get(span.start..span.start.checked_add(span.len)?)?;
    engine_allocation::reserve(output, values.len()).ok()?;
    span.start = output.len();
    output.extend_from_slice(values);
    Some(())
}

impl Core {
    fn read(
        row: &native::IconRow,
        bytes: &[u8],
        counts: &[native::SubunitCount],
        variants: &[u64],
    ) -> Option<Self> {
        // Refused fields always retry on the original path, as before.
        if row.model_failure.is_some() || row.unit_type_failure.is_some() {
            return None;
        }
        let mut core = Self {
            row: *row,
            ..Self::default()
        };
        let r = &mut core.row;
        append(&mut core.bytes, bytes, &mut r.priority_unit_name)?;
        append(&mut core.bytes, bytes, &mut r.model_entity)?;
        append(&mut core.counts, counts, &mut r.counts)?;
        for grid in &mut r.grids {
            append(&mut core.counts, counts, grid)?;
        }
        append(&mut core.variants, variants, &mut r.excluded_variants)?;
        // These belong to the independently held extension and fresh-name paths.
        r.display_name = None;
        r.names_group = None;
        r.cells = Default::default();
        r.grid_dimensions = Default::default();
        r.details_fresh = false;
        r.details_failure = None;
        r.settings = Default::default();
        r.forbidden_equipment = Default::default();
        r.equipment_failure = None;
        if core
            .counts
            .iter()
            .chain(std::iter::once(&core.row.representative))
            .any(|unit| unit.token != 0 && unit.width.is_none())
        {
            return None;
        }
        Some(core)
    }

    fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
            + self.counts.capacity() * size_of::<native::SubunitCount>()
            + self.variants.capacity() * size_of::<u64>()
    }
}

struct Entry {
    template: u64,
    certificate: Certificate,
    record: Vec<u8>,
    core: Core,
    speed: Option<Vec<u32>>,
    carried: bool,
    verify_speed: bool,
}

#[derive(Default)]
pub(super) struct Store {
    entries: Vec<Entry>,
    live: Vec<u64>,
    pending: Option<(u64, usize, u32, Certificate)>,
    payload_bytes: usize,
}

impl Store {
    pub fn forget(&mut self) {
        self.entries.clear();
        self.pending = None;
        self.payload_bytes = 0;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.live.capacity() * size_of::<u64>()
            + self.payload_bytes
            + self
                .entries
                .iter()
                .filter_map(|entry| entry.speed.as_ref())
                .map(|keys| keys.capacity() * size_of::<u32>())
                .sum::<usize>()
    }

    pub fn prepare(
        &mut self,
        template: u64,
        pointer: usize,
        owner: u32,
        certificate: Option<Certificate>,
    ) {
        if let Ok(at) = self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
        {
            self.entries[at].carried = false;
        }
        self.pending = certificate.map(|certificate| (template, pointer, owner, certificate));
    }

    pub fn carry(&mut self, record: &[u8], changes: &dyn Changes) -> Option<native::IconCore<'_>> {
        let (template, pointer, owner, certificate) = self.pending?;
        if crate::sampled_verify::current_burned_in(crate::template_changes::VERIFY)
            .covers(template >> 32)
        {
            return None;
        }
        let at = self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
            .ok()?;
        let entry = &mut self.entries[at];
        if entry.core.row.pointer != pointer
            || entry.core.row.owner != owner
            || entry.certificate != certificate
            || entry.record != record
            || changes.sequence(entry.core.row.pointer) != Some(certificate[0])
            || changes.sequence(entry.core.row.owner as usize) != Some(certificate[1])
            || changes.core_dependencies() != Some(certificate[2])
        {
            return None;
        }
        entry.carried = true;
        Some(native::IconCore {
            row: entry.core.row,
            bytes: &entry.core.bytes,
            counts: &entry.core.counts,
            variants: &entry.core.variants,
        })
    }

    pub fn remember(
        &mut self,
        changes: &dyn Changes,
        record: &[u8],
        row: &native::IconRow,
        bytes: &[u8],
        counts: &[native::SubunitCount],
        variants: &[u64],
    ) {
        let Some((template, _, _, certificate)) = self.pending else {
            return;
        };
        if changes.sequence(row.pointer) != Some(certificate[0])
            || changes.sequence(row.owner as usize) != Some(certificate[1])
            || changes.core_dependencies() != Some(certificate[2])
        {
            return;
        }
        let Some(core) = Core::read(row, bytes, counts, variants) else {
            if let Ok(at) = self
                .entries
                .binary_search_by_key(&template, |entry| entry.template)
            {
                let entry = self.entries.remove(at);
                self.payload_bytes -= entry.record.capacity() + entry.core.allocated_bytes();
            }
            return;
        };
        let at = match self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
        {
            Ok(at) => {
                let entry = &mut self.entries[at];
                if entry.certificate == certificate
                    && entry.record == record
                    && crate::sampled_verify::current_burned_in(crate::template_changes::VERIFY)
                        .covers(template >> 32)
                {
                    let mismatch = entry.core != core;
                    crate::template_changes::record_core_comparison(mismatch);
                    if mismatch {
                        changes.mismatch();
                    }
                }
                let previous_bytes = entry.record.capacity() + entry.core.allocated_bytes();
                if engine_allocation::reserve(&mut entry.record, record.len()).is_err() {
                    return;
                }
                entry.verify_speed = entry.certificate == certificate && entry.record == record;
                entry.carried = false;
                self.payload_bytes -= previous_bytes;
                entry.certificate = certificate;
                entry.record.clear();
                entry.record.extend_from_slice(record);
                entry.core = core;
                at
            }
            Err(at) => {
                let Ok(record) = engine_allocation::copy_vec(record) else {
                    return;
                };
                if engine_allocation::reserve(&mut self.entries, 1).is_err() {
                    return;
                }
                self.entries.insert(
                    at,
                    Entry {
                        template,
                        certificate,
                        record,
                        core,
                        speed: None,
                        carried: false,
                        verify_speed: false,
                    },
                );
                at
            }
        };
        let entry = &self.entries[at];
        self.payload_bytes += entry.record.capacity() + entry.core.allocated_bytes();
    }

    pub fn speed(&self, template: u64, changes: &dyn Changes) -> Option<&[u32]> {
        let entry = &self.entries[self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
            .ok()?];
        (entry.carried
            && changes.sequence(entry.core.row.pointer) == Some(entry.certificate[0])
            && changes.sequence(entry.core.row.owner as usize) == Some(entry.certificate[1])
            && changes.core_dependencies() == Some(entry.certificate[2]))
        .then_some(entry.speed.as_deref())
        .flatten()
    }

    pub fn remember_speed(&mut self, template: u64, keys: Option<&[u32]>, changes: &dyn Changes) {
        let Ok(at) = self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
        else {
            return;
        };
        let entry = &mut self.entries[at];
        if changes.sequence(entry.core.row.pointer) != Some(entry.certificate[0])
            || changes.sequence(entry.core.row.owner as usize) != Some(entry.certificate[1])
            || changes.core_dependencies() != Some(entry.certificate[2])
        {
            entry.speed = None;
            return;
        }
        if entry.verify_speed
            && let Some((kept, fresh)) = entry.speed.as_deref().zip(keys)
            && crate::sampled_verify::current_burned_in(crate::template_changes::VERIFY)
                .covers(template >> 32)
        {
            let mismatch = kept != fresh;
            crate::template_changes::record_core_comparison(mismatch);
            if mismatch {
                changes.mismatch();
            }
        }
        entry.speed = keys.and_then(|keys| engine_allocation::copy_vec(keys).ok());
    }

    pub fn prune(&mut self, rows: &[native::IconRow]) {
        if self.entries.len() <= rows.len() {
            return;
        }
        self.live.clear();
        if engine_allocation::reserve(&mut self.live, rows.len()).is_err() {
            return;
        }
        self.live.extend(rows.iter().map(|row| row.template));
        self.live.sort_unstable();
        self.entries.retain(|entry| {
            let keep = self.live.binary_search(&entry.template).is_ok();
            if !keep {
                self.payload_bytes -= entry.record.capacity() + entry.core.allocated_bytes();
            }
            keep
        });
    }
}
