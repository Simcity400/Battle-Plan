use super::*;
use crate::template_changes::Changes;

pub(super) type Certificate = [(u64, u64); 3];

fn stands(
    changes: &dyn Changes,
    certificate: &Certificate,
    pointer: usize,
    owner: u32,
    deployment: usize,
) -> bool {
    changes.sequence(pointer) == Some(certificate[0])
        && changes.sequence(owner as usize) == Some(certificate[1])
        && changes.deployment_reset_sequence(deployment) == Some(certificate[2])
}

#[derive(Default, PartialEq)]
struct Core {
    row: native::IconRow,
    bytes: Vec<u8>,
    counts: Vec<native::SubunitCount>,
    variants: Vec<u64>,
    names: Vec<u8>,
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
        names: &[u8],
    ) -> Option<Self> {
        if row.model_failure.is_some() || row.unit_type_failure.is_some() {
            return None;
        }
        let display = row.display_name?;
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
        let mut literal = display.literal;
        let mut key = r.name_key?;
        append(&mut core.names, names, &mut literal)?;
        append(&mut core.names, names, &mut key)?;
        r.display_name = Some(native::DisplayName {
            literal,
            fallback_key: display.fallback_key.map(|_| key),
        });
        r.name_key = Some(key);
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
            + self.names.capacity()
    }
}

struct Entry {
    template: u64,
    certificate: Certificate,
    deployment: usize,
    core: Core,
    speed: Option<Vec<u32>>,
    carried: bool,
    renewed: bool,
}

#[derive(Default)]
pub(super) struct Store {
    entries: Vec<Entry>,
    live: Vec<u64>,
    payload_bytes: usize,
    changed: bool,
    hints: Vec<u32>,
    visits: usize,
    carried: Vec<u32>,
    next_held: usize,
}
pub(super) fn hinted<E>(
    entries: &[E],
    hints: &[u32],
    slot: usize,
    template: u64,
    key: impl Fn(&E) -> u64,
) -> Option<usize> {
    let at = *hints.get(slot)? as usize;
    entries
        .get(at)
        .filter(|entry| key(entry) == template)
        .map(|_| at)
}

pub(super) fn remember_hint(hints: &mut Vec<u32>, slot: usize, at: usize) {
    let Ok(at) = u32::try_from(at) else {
        return;
    };
    if let Some(hint) = hints.get_mut(slot) {
        *hint = at;
    } else if slot == hints.len()
        && engine_allocation::advisory(|| engine_allocation::reserve(hints, 1)).is_ok()
    {
        hints.push(at);
    }
}

fn carries(entry: &Entry, pointer: usize, owner: u32, certificate: &Certificate) -> bool {
    entry.core.row.pointer == pointer
        && entry.core.row.owner == owner
        && entry.certificate == *certificate
        && entry.speed.is_some()
}

impl Store {
    pub fn forget(&mut self) {
        self.entries.clear();
        self.payload_bytes = 0;
    }

    pub fn begin(&mut self) {
        self.changed = false;
        self.visits = 0;
        self.carried.clear();
        self.next_held = 0;
        for entry in &mut self.entries {
            entry.carried = false;
            entry.renewed = false;
        }
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.live.capacity() * size_of::<u64>()
            + (self.hints.capacity() + self.carried.capacity()) * size_of::<u32>()
            + self.payload_bytes
            + self
                .entries
                .iter()
                .filter_map(|entry| entry.speed.as_ref())
                .map(|keys| keys.capacity() * size_of::<u32>())
                .sum::<usize>()
    }

    pub fn carry(
        &mut self,
        template: u64,
        pointer: usize,
        owner: u32,
        certificate: Option<Certificate>,
    ) -> bool {
        let slot = self.visits;
        self.visits += 1;
        let Some(certificate) = certificate else {
            return false;
        };
        let Some(at) = hinted(&self.entries, &self.hints, slot, template, |entry| {
            entry.template
        })
        .or_else(|| {
            self.entries
                .binary_search_by_key(&template, |entry| entry.template)
                .ok()
        }) else {
            return false;
        };
        remember_hint(&mut self.hints, slot, at);
        let entry = &mut self.entries[at];
        let carried = carries(entry, pointer, owner, &certificate);
        entry.carried |= carried;
        if carried {
            let next = self.carried.len();
            remember_hint(&mut self.carried, next, at);
        }
        carried
    }

    pub fn would_carry(
        &self,
        template: u64,
        pointer: usize,
        owner: u32,
        certificate: Option<Certificate>,
    ) -> bool {
        certificate.is_some_and(|certificate| {
            self.entries
                .binary_search_by_key(&template, |entry| entry.template)
                .is_ok_and(|at| carries(&self.entries[at], pointer, owner, &certificate))
        })
    }

    pub fn renewed(&self, template: u64) -> bool {
        self.entries
            .binary_search_by_key(&template, |entry| entry.template)
            .is_ok_and(|at| self.entries[at].renewed)
    }

    pub fn core_of(&self, template: u64) -> Option<native::IconCore<'_>> {
        let entry = &self.entries[self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
            .ok()?];
        Some(native::IconCore {
            row: entry.core.row,
            bytes: &entry.core.bytes,
            counts: &entry.core.counts,
            variants: &entry.core.variants,
            names: &entry.core.names,
        })
    }

    pub fn held(&mut self, template: u64) -> Option<native::IconCore<'_>> {
        let slot = self.next_held;
        self.next_held += 1;
        let at = hinted(&self.entries, &self.carried, slot, template, |entry| {
            entry.template
        })
        .or_else(|| {
            self.entries
                .binary_search_by_key(&template, |entry| entry.template)
                .ok()
        })?;
        let entry = &self.entries[at];
        entry.carried.then_some(native::IconCore {
            row: entry.core.row,
            bytes: &entry.core.bytes,
            counts: &entry.core.counts,
            variants: &entry.core.variants,
            names: &entry.core.names,
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one remembered template keeps its certificate, deployment, row and each borrowed table"
    )]
    pub fn remember(
        &mut self,
        changes: &dyn Changes,
        certificate: Option<Certificate>,
        deployment: Option<usize>,
        row: &native::IconRow,
        bytes: &[u8],
        counts: &[native::SubunitCount],
        variants: &[u64],
        names: &[u8],
    ) {
        let (Some(certificate), Some(deployment)) = (certificate, deployment) else {
            self.changed = true;
            return;
        };
        if !stands(changes, &certificate, row.pointer, row.owner, deployment) {
            self.changed = true;
            return;
        }
        let found = self
            .entries
            .binary_search_by_key(&row.template, |entry| entry.template);
        if found.is_ok_and(|at| self.entries[at].carried) {
            return;
        }
        let Some(core) = Core::read(row, bytes, counts, variants, names) else {
            self.changed = true;
            if let Ok(at) = found {
                let entry = self.entries.remove(at);
                self.payload_bytes -= entry.core.allocated_bytes();
            }
            return;
        };
        let at = match found {
            Ok(at) => {
                let entry = &mut self.entries[at];
                self.changed |= entry.core != core;
                self.payload_bytes -= entry.core.allocated_bytes();
                entry.certificate = certificate;
                entry.deployment = deployment;
                entry.core = core;
                entry.renewed = true;
                at
            }
            Err(at) => {
                self.changed = true;
                if engine_allocation::reserve(&mut self.entries, 1).is_err() {
                    return;
                }
                self.entries.insert(
                    at,
                    Entry {
                        template: row.template,
                        certificate,
                        deployment,
                        core,
                        speed: None,
                        carried: false,
                        renewed: true,
                    },
                );
                at
            }
        };
        self.payload_bytes += self.entries[at].core.allocated_bytes();
    }

    pub fn speed(&self, template: u64) -> Option<&[u32]> {
        let entry = &self.entries[self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
            .ok()?];
        entry.carried.then_some(entry.speed.as_deref()).flatten()
    }

    pub fn remember_speed(&mut self, template: u64, keys: Option<&[u32]>, changes: &dyn Changes) {
        let Ok(at) = self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
        else {
            self.changed = true;
            return;
        };
        let entry = &mut self.entries[at];
        if !stands(
            changes,
            &entry.certificate,
            entry.core.row.pointer,
            entry.core.row.owner,
            entry.deployment,
        ) {
            self.changed = true;
            entry.speed = None;
            return;
        }
        if entry.speed.as_deref() == keys {
            return;
        }
        self.changed = true;
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
                self.payload_bytes -= entry.core.allocated_bytes();
            }
            keep
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template_changes::Watches;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    type Stamp = (u64, u64);

    const DEPLOYMENT: usize = 0x8400;
    const OTHER: usize = 0x9900;

    struct Stamped {
        watches: Watches,
        dependencies: [AtomicU64; 2],
    }

    impl Changes for Stamped {
        fn quiet(&self) -> Option<Stamp> {
            self.watches.quiet()
        }
        fn sequence(&self, pointer: usize) -> Option<Stamp> {
            self.watches.sequence(pointer)
        }
        fn deployment_reset_sequence(&self, status: usize) -> Option<Stamp> {
            let slot = [DEPLOYMENT, OTHER].iter().position(|&at| at == status)?;
            Some((1, self.dependencies[slot].load(Relaxed)))
        }
    }

    fn certificate(changes: &Stamped, row: &native::IconRow) -> Option<Certificate> {
        Some([
            changes.sequence(row.pointer)?,
            changes.sequence(row.owner as usize)?,
            changes.deployment_reset_sequence(DEPLOYMENT)?,
        ])
    }

    #[test]
    fn held_rows_carry_only_under_their_own_stamps_and_object() {
        let changes = Stamped {
            watches: Watches::enabled(8),
            dependencies: [AtomicU64::new(0), AtomicU64::new(0)],
        };
        let names = b"Alphakey";
        let row = native::IconRow {
            template: (7_u64 << 32) | u64::from(native::REFERENCE_TYPE),
            pointer: 0x5000,
            owner: 1,
            display_name: Some(native::DisplayName {
                literal: native::Span { start: 0, len: 5 },
                fallback_key: None,
            }),
            name_key: Some(native::Span { start: 5, len: 3 }),
            ..Default::default()
        };
        let mut store = Store::default();
        store.remember(
            &changes,
            certificate(&changes, &row),
            Some(DEPLOYMENT),
            &row,
            &[],
            &[],
            &[],
            names,
        );
        store.remember_speed(row.template, Some(&[155]), &changes);
        let carried = |store: &mut Store, changes: &Stamped, pointer: usize, owner: u32| {
            store.begin();
            let current = certificate(changes, &row);
            store.carry(row.template, pointer, owner, current)
        };
        assert!(carried(&mut store, &changes, row.pointer, row.owner));
        let core = store.held(row.template).unwrap();
        let display = core.row.display_name.unwrap();
        assert_eq!(
            &core.names[display.literal.start..][..display.literal.len],
            b"Alpha"
        );
        let key = core.row.name_key.unwrap();
        assert_eq!(&core.names[key.start..][..key.len], b"key");
        assert_eq!(store.speed(row.template), Some([155].as_slice()));
        changes.dependencies[1].fetch_add(1, Relaxed);
        assert!(carried(&mut store, &changes, row.pointer, row.owner));
        assert!(!carried(&mut store, &changes, 0x6000, row.owner));
        assert!(!carried(&mut store, &changes, row.pointer, 2));
        assert!(store.held(row.template).is_none() && store.speed(row.template).is_none());
        drop(changes.watches.notify(row.owner as usize, 29));
        assert!(!carried(&mut store, &changes, row.pointer, row.owner));
        store.begin();
        assert!(!store.carry(row.template, row.pointer, row.owner, None));
        changes.dependencies[0].fetch_add(1, Relaxed);
        assert!(!carried(&mut store, &changes, row.pointer, row.owner));
        let before = certificate(&changes, &row);
        drop(changes.watches.notify(row.pointer, 0));
        store.remember(
            &changes,
            before,
            Some(DEPLOYMENT),
            &row,
            &[],
            &[],
            &[],
            names,
        );
        assert!(!carried(&mut store, &changes, row.pointer, row.owner));
        store.remember(
            &changes,
            certificate(&changes, &row),
            Some(DEPLOYMENT),
            &row,
            &[],
            &[],
            &[],
            names,
        );
        assert!(carried(&mut store, &changes, row.pointer, row.owner));
        store.begin();
        let unnamed = native::IconRow {
            name_key: None,
            ..row
        };
        store.remember(
            &changes,
            certificate(&changes, &row),
            Some(DEPLOYMENT),
            &unnamed,
            &[],
            &[],
            &[],
            names,
        );
        assert!(store.entries.is_empty() && store.payload_bytes == 0);
    }
}
