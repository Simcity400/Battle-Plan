use super::*;
use crate::template_changes::Changes;
use std::sync::Arc;

type Stamp = (u64, u64);
pub(super) type Certificate = [Stamp; 2];

#[derive(Default)]
struct Entry {
    template: u64,
    pointer: usize,
    owner: u32,
    ordinal: u32,
    certificate: Option<Certificate>,
    pending: Option<Certificate>,
    requested: bool,
    attempted: bool,
    acquired: bool,
    valid: bool,
    reordered: bool,
    layout: Option<Arc<TemplateLayout>>,
    layout_serial: u64,
    settings: Option<Arc<TemplateSettings>>,
    settings_serial: u64,
    raw_settings: Option<native::Settings>,
}

impl Entry {
    fn allocated_bytes(&self) -> usize {
        self.layout.as_ref().map_or(0, |layout| {
            size_of::<TemplateLayout>()
                + [
                    &layout.regiments,
                    &layout.support,
                    &layout.regimental_support,
                ]
                .iter()
                .map(|grid| grid.cells.capacity() * size_of::<TemplateCell>())
                .sum::<usize>()
        }) + self.settings.as_ref().map_or(0, |settings| {
            size_of::<TemplateSettings>()
                + settings.names_group.as_ref().map_or(0, Vec::capacity)
                + settings
                    .forbidden_equipment
                    .as_ref()
                    .map_or(0, |rows| rows.capacity() * 4)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template_changes::Watches;

    #[test]
    fn owned_payload_settlement_uses_the_parked_certificate() {
        let watches = Watches::enabled(16);
        let mut store = Store::default();
        let row = native::IconRow {
            template: 52,
            pointer: 0x5000,
            owner: 1,
            details_fresh: true,
            names_group: Some(native::Span { start: 0, len: 3 }),
            ..Default::default()
        };
        store.begin();
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
        store.certify(&watches, &[row]);
        drop(watches.notify(row.pointer, 0));
        store.settle(&[row], &[], &[], b"OLD");
        let settings = store
            .values(&row)
            .1
            .require("settings")
            .unwrap()
            .clone()
            .unwrap();
        assert_eq!(settings.names_group.as_deref(), Some(b"OLD".as_slice()));
        store.begin();
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
    }

    #[test]
    fn template_first_acquisition_refuses_a_writer_active_at_both_ends_and_retries() {
        let watches = Watches::enabled(16);
        let mut store = Store::default();
        let row = native::IconRow {
            template: 52,
            pointer: 0x5000,
            owner: 1,
            details_fresh: true,
            ..Default::default()
        };
        let running = watches.notify(row.pointer, 0);
        assert!(!store.request(&watches, row.owner, row.pointer, row.template, 0));
        store.finish(&watches, &[row], &[], &[], &[]);
        assert!(matches!(store.values(&row).1, Recorded::Value(None)));
        drop(running);
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
        store.finish(&watches, &[row], &[], &[], &[]);
        assert!(matches!(store.values(&row).1, Recorded::Value(_)));
    }

    #[test]
    fn naming_group_changes_in_unused_templates_replace_only_the_dirty_payload() {
        let watches = Watches::enabled(16);
        let mut store = Store::default();
        let row = native::IconRow {
            template: 52,
            pointer: 0x5000,
            owner: 1,
            details_fresh: true,
            names_group: Some(native::Span { start: 0, len: 3 }),
            ..Default::default()
        };
        assert!(store.request(&watches, 1, row.pointer, row.template, 0));
        store.finish(&watches, &[row], &[], &[], b"OLD");
        let first = store
            .values(&row)
            .1
            .require("settings")
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        drop(watches.notify(row.pointer, 11));
        assert!(store.request(&watches, 1, row.pointer, row.template, 0));
        store.finish(&watches, &[row], &[], &[], b"NEW");
        let second = store
            .values(&row)
            .1
            .require("settings")
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(first.names_group.as_deref(), Some(b"OLD".as_slice()));
        assert_eq!(second.names_group.as_deref(), Some(b"NEW".as_slice()));
        assert!(!Arc::ptr_eq(&first, &second));
        store.begin();
        assert!(!store.request(&watches, 1, row.pointer, row.template, 1));
        store.finish(&watches, &[row], &[], &[], &[]);
        let moved = store
            .values(&row)
            .1
            .require("settings")
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(moved.roster_ordinal, 1);
        assert_eq!(moved.names_group.as_deref(), Some(b"NEW".as_slice()));
        assert_eq!(
            store.payload_bytes,
            store
                .entries
                .iter()
                .map(Entry::allocated_bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn template_details_quiet_certificate_shares_payloads_without_allocation_and_writers_revoke_it()
    {
        let watches = Watches::enabled(16);
        let mut store = Store::default();
        let row = native::IconRow {
            template: (7_u64 << 32) | 52,
            pointer: 0x5000,
            owner: 1,
            details_fresh: true,
            grid_dimensions: [0x0002_0001, 0, 0],
            cells: [
                native::Span { start: 0, len: 2 },
                native::Span::default(),
                native::Span::default(),
            ],
            ..Default::default()
        };
        let cells = [
            native::SubunitCount {
                token: 77,
                count: 1,
                ..Default::default()
            },
            native::SubunitCount::default(),
        ];
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
        store.finish(&watches, &[row], &cells, &[], &[]);
        let (layout, settings) = store.values(&row);
        collector_test_support::with_allocations_denied(|| {
            store.begin();
            assert!(!store.request(&watches, row.owner, row.pointer, row.template, 0));
            struct QuietOnly((u64, u64));
            impl Changes for QuietOnly {
                fn quiet(&self) -> Option<(u64, u64)> {
                    Some(self.0)
                }
                fn sequence(&self, _: usize) -> Option<(u64, u64)> {
                    panic!("quiet finalization must not walk receiver certificates again")
                }
            }
            store.finish(&QuietOnly(watches.quiet().unwrap()), &[row], &[], &[], &[]);
            let (next_layout, next_settings) = store.values(&row);
            assert!(Arc::ptr_eq(
                layout.require("layout").unwrap().as_ref().unwrap(),
                next_layout.require("layout").unwrap().as_ref().unwrap()
            ));
            assert!(Arc::ptr_eq(
                settings.require("settings").unwrap().as_ref().unwrap(),
                next_settings.require("settings").unwrap().as_ref().unwrap()
            ));
        });
        drop(watches.notify(row.pointer, 0));
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
        collector_test_support::with_allocations_denied(|| {
            store.finish(&watches, &[row], &cells, &[], &[]);
            assert!(Arc::ptr_eq(
                settings.require("settings").unwrap().as_ref().unwrap(),
                store
                    .values(&row)
                    .1
                    .require("settings")
                    .unwrap()
                    .as_ref()
                    .unwrap()
            ));
        });
        drop(watches.notify(row.owner as usize, 0));
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
        let running = watches.notify(row.pointer, 0);
        store.finish(&watches, &[row], &cells, &[], &[]);
        assert!(matches!(store.values(&row).1, Recorded::Value(None)));
        drop(running);
        assert!(store.request(&watches, row.owner, row.pointer, row.template, 0));
        let failed = native::IconRow {
            details_fresh: false,
            ..row
        };
        store.finish(&watches, &[failed], &[], &[], &[]);
        assert!(matches!(store.values(&row).1, Recorded::Value(None)));
        assert!(!store.request(&watches, row.owner, row.pointer, row.template, 0));
        store.finish(&watches, &[failed], &[], &[], &[]);
        assert!(
            matches!(store.values(&row).1, Recorded::Value(None)),
            "failed acquisition must never revive old operands"
        );
        assert_eq!(
            store.payload_bytes,
            store
                .entries
                .iter()
                .map(Entry::allocated_bytes)
                .sum::<usize>()
        );
        store.prune(&[]);
        assert!(store.entries.is_empty());
        assert_eq!(store.payload_bytes, 0);
    }
}

#[derive(Default)]
pub(super) struct Store {
    entries: Vec<Entry>,
    index: super::core::TemplateIndex,
    live: Vec<u64>,
    payload_bytes: usize,
    requested: bool,
    reordered: bool,
    changed: bool,
    hints: Vec<u32>,
    visits: usize,
}

use super::core::hinted;

fn needs_read(
    entry: &Entry,
    owner: u32,
    pointer: usize,
    current: Option<Certificate>,
    fresh: bool,
) -> bool {
    let same = entry.pointer == pointer && entry.owner == owner;
    fresh || !same || !entry.attempted || current != entry.certificate
}

fn certificate(changes: &dyn Changes, pointer: usize, owner: u32) -> Option<Certificate> {
    Some([
        changes.sequence(pointer)?,
        changes.sequence(owner as usize)?,
    ])
}

#[derive(Clone, Copy)]
pub(super) struct Stamps {
    pub deployment: Option<usize>,
    dependencies: Option<Stamp>,
    owner: Option<Stamp>,
    batch: [(usize, Option<Stamp>); native::READ_BATCH],
}

impl Default for Stamps {
    fn default() -> Self {
        Self {
            deployment: None,
            dependencies: None,
            owner: None,
            batch: [(0, None); native::READ_BATCH],
        }
    }
}

impl Stamps {
    pub fn country(&mut self, changes: &dyn Changes, country: u32, deployment: Option<usize>) {
        self.owner = changes.sequence(country as usize);
        self.deployment = deployment;
        self.dependencies = deployment.and_then(|status| changes.deployment_reset_sequence(status));
    }

    pub fn records(&mut self, changes: &dyn Changes, templates: &[usize]) {
        for (slot, &pointer) in self.batch.iter_mut().zip(templates) {
            *slot = (pointer, changes.sequence(pointer));
        }
    }

    pub fn template(&self, pointer: usize, ordinal: u32) -> Option<Certificate> {
        let (expected, stamp) = self.batch[ordinal as usize % native::READ_BATCH];
        Some([stamp.filter(|_| expected == pointer)?, self.owner?])
    }

    pub fn core(
        &self,
        changes: &dyn Changes,
        pointer: usize,
        ordinal: u32,
    ) -> Option<super::core::Certificate> {
        if changes.fresh_only() {
            return None;
        }
        let [template, owner] = self.template(pointer, ordinal)?;
        Some([template, owner, self.dependencies?])
    }
}

pub(super) struct Demand<'a> {
    store: &'a mut Store,
    changes: &'a dyn Changes,
    core: &'a mut super::core::Store,
    stamps: Stamps,
    pub counts: [u64; 2],
}
impl<'a> Demand<'a> {
    pub fn new(
        store: &'a mut Store,
        core: &'a mut super::core::Store,
        changes: &'a dyn Changes,
    ) -> Self {
        Self {
            store,
            changes,
            core,
            stamps: Stamps::default(),
            counts: [0; 2],
        }
    }
}
impl native::DetailPolicy for Demand<'_> {
    fn before_country(&mut self, country: u32, deployment: Option<usize>) {
        self.stamps.country(self.changes, country, deployment);
    }
    fn before_records(&mut self, templates: &[usize]) {
        self.stamps.records(self.changes, templates);
    }
    fn demand(
        &mut self,
        country: u32,
        pointer: usize,
        template: u64,
        ordinal: u32,
    ) -> native::Acquire {
        let stamps = self.stamps.template(pointer, ordinal);
        let certificate = self.stamps.core(self.changes, pointer, ordinal);
        let details = self.store.request_certified(
            country,
            pointer,
            template,
            ordinal,
            stamps,
            self.changes.fresh_only(),
        );
        let carried = !details && self.core.carry(template, pointer, country, certificate);
        self.counts[usize::from(!carried)] += 1;
        if carried {
            native::Acquire::Held
        } else {
            native::Acquire::Read { details }
        }
    }
    fn held(&mut self, template: u64) -> Option<native::IconCore<'_>> {
        self.core.held(template)
    }
    fn remember(
        &mut self,
        row: &native::IconRow,
        bytes: &[u8],
        counts: &[native::SubunitCount],
        variants: &[u64],
        names: &[u8],
    ) {
        let certificate = self
            .stamps
            .core(self.changes, row.pointer, row.settings.roster_ordinal);
        self.core.remember(
            self.changes,
            certificate,
            self.stamps.deployment,
            row,
            bytes,
            counts,
            variants,
            names,
        );
    }
}

impl Store {
    fn lookup(&self, template: u64) -> Option<usize> {
        self.index.get(&template).map(|&at| at as usize)
    }

    pub fn forget(&mut self) {
        self.entries.clear();
        self.index.clear();
        self.payload_bytes = 0;
    }

    pub fn begin(&mut self) {
        self.requested = false;
        self.reordered = false;
        self.changed = false;
        self.visits = 0;
    }

    pub fn changed(&self) -> bool {
        self.changed
    }

    pub fn clear_changed(&mut self) {
        self.changed = false;
        self.reordered = false;
    }

    pub fn reordered(&self) -> bool {
        self.reordered
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.index.capacity() * (size_of::<u64>() + size_of::<u32>())
            + self.live.capacity() * size_of::<u64>()
            + self.payload_bytes
            + self.hints.capacity() * size_of::<u32>()
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
                self.payload_bytes -= entry.allocated_bytes();
            }
            keep
        });
        self.index.clear();
        for (at, entry) in self.entries.iter().enumerate() {
            self.index.insert(entry.template, at as u32);
        }
    }

    #[cfg(test)]
    pub fn request(
        &mut self,
        changes: &dyn Changes,
        owner: u32,
        pointer: usize,
        template: u64,
        ordinal: u32,
    ) -> bool {
        self.request_certified(
            owner,
            pointer,
            template,
            ordinal,
            certificate(changes, pointer, owner),
            changes.fresh_only(),
        )
    }
    pub(super) fn request_certified(
        &mut self,
        owner: u32,
        pointer: usize,
        template: u64,
        ordinal: u32,
        current: Option<Certificate>,
        fresh: bool,
    ) -> bool {
        let slot = self.visits;
        self.visits += 1;
        let found = hinted(&self.entries, &self.hints, slot, template, |entry| {
            entry.template
        })
        .or_else(|| self.lookup(template));
        let at = match found {
            Some(at) => at,
            None => {
                let Ok(index) = u32::try_from(self.entries.len()) else {
                    return false;
                };
                if engine_allocation::reserve(&mut self.entries, 1).is_err()
                    || engine_allocation::reserve_map(&mut self.index, 1).is_err()
                {
                    return false;
                }
                self.entries.push(Entry {
                    template,
                    ..Default::default()
                });
                self.index.insert(template, index);
                index as usize
            }
        };
        super::core::remember_hint(&mut self.hints, slot, at);
        let entry = &mut self.entries[at];
        if current.is_none() && !fresh {
            entry.valid = false;
            entry.attempted = false;
            entry.acquired = false;
            entry.requested = false;
            entry.reordered = false;
            return false;
        }
        let read = needs_read(entry, owner, pointer, current, fresh);
        if read {
            self.requested = true;
            entry.attempted = true;
            entry.certificate = current;
            entry.acquired = false;
        }
        entry.reordered = !read && entry.ordinal != ordinal;
        self.reordered |= entry.reordered;
        entry.pointer = pointer;
        entry.owner = owner;
        entry.ordinal = ordinal;
        entry.pending = current;
        entry.requested = read;
        entry.valid = !read && entry.acquired && current.is_some() && current == entry.certificate;
        read
    }

    pub(super) fn decide(
        &self,
        owner: u32,
        pointer: usize,
        template: u64,
        current: Option<Certificate>,
        fresh: bool,
    ) -> bool {
        if current.is_none() && !fresh {
            return false;
        }
        match self.lookup(template) {
            Some(at) => needs_read(&self.entries[at], owner, pointer, current, fresh),
            None => needs_read(&Entry::default(), owner, pointer, current, fresh),
        }
    }

    pub(super) fn reserve_new(&mut self, templates: &[u64]) -> bool {
        let new = templates
            .iter()
            .filter(|&&template| !self.index.contains_key(&template))
            .count();
        engine_allocation::reserve(&mut self.entries, new).is_ok()
            && engine_allocation::reserve_map(&mut self.index, new).is_ok()
    }

    pub fn certify(&mut self, changes: &dyn Changes, rows: &[native::IconRow]) {
        if !self.requested {
            return;
        }
        for row in rows {
            let Some(at) = self.lookup(row.template) else {
                continue;
            };
            let entry = &mut self.entries[at];
            if !entry.requested {
                continue;
            }
            if changes.fresh_only() {
                entry.valid = row.details_fresh;
                continue;
            }
            if entry.pending.is_none()
                || entry.pending != certificate(changes, row.pointer, row.owner)
            {
                entry.valid = false;
                entry.attempted = false;
                entry.acquired = false;
                continue;
            }
            entry.attempted = true;
            entry.certificate = entry.pending;
            entry.valid = row.details_fresh;
        }
    }

    #[cfg(test)]
    pub fn finish(
        &mut self,
        changes: &dyn Changes,
        rows: &[native::IconRow],
        counts: &[native::SubunitCount],
        variants: &[u64],
        bytes: &[u8],
    ) {
        self.certify(changes, rows);
        self.settle(rows, counts, variants, bytes);
    }

    pub fn settle(
        &mut self,
        rows: &[native::IconRow],
        counts: &[native::SubunitCount],
        variants: &[u64],
        bytes: &[u8],
    ) {
        if !self.requested && !self.reordered {
            return;
        }
        for row in rows {
            let Some(at) = self.lookup(row.template) else {
                continue;
            };
            let entry = &mut self.entries[at];
            if std::mem::take(&mut entry.reordered) {
                if entry.valid
                    && let Some(settings) = entry.settings.as_ref()
                {
                    let previous_bytes = entry.allocated_bytes();
                    let renumbered = Arc::new(TemplateSettings {
                        roster_ordinal: entry.ordinal,
                        names_group: settings
                            .names_group
                            .as_deref()
                            .map(|key| owned_vec(engine_allocation::copy_vec(key))),
                        forbidden_equipment: settings
                            .forbidden_equipment
                            .as_deref()
                            .map(|tokens| owned_vec(engine_allocation::copy_vec(tokens))),
                        ..**settings
                    });
                    entry.settings = Some(renumbered);
                    entry.settings_serial = collector_projection::projection::generation();
                    if let Some(raw) = &mut entry.raw_settings {
                        raw.roster_ordinal = entry.ordinal;
                    }
                    self.payload_bytes =
                        self.payload_bytes - previous_bytes + entry.allocated_bytes();
                }
                continue;
            }
            if !entry.requested || !entry.valid {
                continue;
            }
            settle_entry(
                entry,
                row,
                counts,
                variants,
                bytes,
                &mut self.changed,
                &mut self.payload_bytes,
            );
        }
        self.requested = false;
        self.reordered = false;
    }

    pub fn patch_dirty(
        &mut self,
        row: &native::IconRow,
        counts: &[native::SubunitCount],
        variants: &[u64],
        bytes: &[u8],
        certificate: Certificate,
    ) -> bool {
        if !row.details_fresh {
            return false;
        }
        let Some(at) = self.lookup(row.template) else {
            return false;
        };
        let entry = &mut self.entries[at];
        entry.pointer = row.pointer;
        entry.owner = row.owner;
        entry.ordinal = row.settings.roster_ordinal;
        entry.certificate = Some(certificate);
        entry.pending = Some(certificate);
        entry.attempted = true;
        entry.requested = true;
        entry.valid = true;
        entry.reordered = false;
        settle_entry(
            entry,
            row,
            counts,
            variants,
            bytes,
            &mut self.changed,
            &mut self.payload_bytes,
        );
        entry.valid && entry.settings.is_some()
    }

    pub fn settings_available(&self, row: &native::IconRow) -> bool {
        self.lookup(row.template).is_some_and(|at| {
            let entry = &self.entries[at];
            entry.valid
                && entry.pointer == row.pointer
                && entry.owner == row.owner
                && (entry.requested || entry.settings.is_some())
        })
    }

    #[expect(
        clippy::type_complexity,
        reason = "the held layout, settings and their serials are looked up together"
    )]
    pub(super) fn value_refs(
        &self,
        row: &native::IconRow,
    ) -> (
        Option<&Arc<TemplateLayout>>,
        u64,
        Option<&Arc<TemplateSettings>>,
        u64,
    ) {
        let entry = self
            .lookup(row.template)
            .map(|at| &self.entries[at])
            .filter(|entry| {
                entry.valid && entry.pointer == row.pointer && entry.owner == row.owner
            });
        match entry.and_then(|entry| entry.settings.as_ref().map(|settings| (entry, settings))) {
            Some((entry, settings)) => (
                entry.layout.as_ref(),
                if entry.layout.is_some() {
                    entry.layout_serial
                } else {
                    0
                },
                Some(settings),
                entry.settings_serial,
            ),
            None => (None, 0, None, 0),
        }
    }

    #[cfg(test)]
    #[expect(
        clippy::type_complexity,
        reason = "the layout and settings are returned together as recorded values"
    )]
    pub fn values(
        &self,
        row: &native::IconRow,
    ) -> (
        Recorded<Option<Arc<TemplateLayout>>>,
        Recorded<Option<Arc<TemplateSettings>>>,
    ) {
        let (layout, _, settings, _) = self.value_refs(row);
        (
            Recorded::Value(layout.cloned()),
            Recorded::Value(settings.cloned()),
        )
    }
}

fn settle_entry(
    entry: &mut Entry,
    row: &native::IconRow,
    counts: &[native::SubunitCount],
    variants: &[u64],
    bytes: &[u8],
    changed: &mut bool,
    payload_bytes: &mut usize,
) {
    let previous_bytes = entry.allocated_bytes();
    let grid = |index: usize| -> Option<TemplateGrid> {
        let cells = span(counts, row.cells[index])?;
        Some(TemplateGrid {
            columns: row.grid_dimensions[index] & 0xffff,
            rows: row.grid_dimensions[index] >> 16,
            cells: owned_vec(engine_allocation::collect_vec(cells.iter().map(|cell| {
                TemplateCell {
                    token: cell.token,
                    local: u8::from(cell.local),
                    occupied: cell.count != 0,
                }
            }))),
        })
    };
    let same_grid = |grid: &TemplateGrid, index: usize| {
        grid.columns == row.grid_dimensions[index] & 0xffff
            && grid.rows == row.grid_dimensions[index] >> 16
            && span(counts, row.cells[index]).is_some_and(|cells| {
                cells.len() == grid.cells.len()
                    && grid.cells.iter().zip(cells).all(|(a, b)| {
                        a.token == b.token
                            && a.local == u8::from(b.local)
                            && a.occupied == (b.count != 0)
                    })
            })
    };
    let layout_equal = entry.layout.as_ref().is_some_and(|layout| {
        row.model_failure.is_none()
            && same_grid(&layout.regiments, 0)
            && same_grid(&layout.support, 1)
            && same_grid(&layout.regimental_support, 2)
    });
    if !layout_equal {
        *changed = true;
        entry.layout = (|| {
            if row.model_failure.is_some() {
                return None;
            }
            Some(Arc::new(TemplateLayout {
                regiments: grid(0)?,
                support: grid(1)?,
                regimental_support: grid(2)?,
            }))
        })();
        entry.layout_serial = if entry.layout.is_some() {
            collector_projection::projection::generation()
        } else {
            0
        };
    }
    let r = &row.settings;
    let settings_equal = entry.raw_settings == Some(row.settings)
        && entry.settings.as_ref().is_some_and(|settings| {
            if settings.names_group.as_deref()
                != row.names_group.and_then(|range| span(bytes, range))
            {
                return false;
            }
            match (&settings.forbidden_equipment, row.equipment_failure) {
                (None, Some(_)) => true,
                (Some(old), None) => {
                    span(variants, row.forbidden_equipment).is_some_and(|tokens| {
                        old.len() == tokens.len()
                            && old.iter().zip(tokens).all(|(a, b)| u64::from(*a) == *b)
                    })
                }
                _ => false,
            }
        });
    if !settings_equal {
        *changed = true;
        entry.settings = Some(Arc::new(TemplateSettings {
            roster_ordinal: r.roster_ordinal,
            names_group: row
                .names_group
                .and_then(|range| span(bytes, range))
                .map(|key| owned_vec(engine_allocation::copy_vec(key))),
            priority: r.priority,
            is_army_hq: r.is_army_hq,
            locked: r.locked,
            obsolete: r.obsolete,
            obsolete_change_date: r.obsolete_change_date,
            force_allow_recruiting: r.force_allow_recruiting,
            allow_new_equipment: r.allow_new_equipment,
            allow_foreign_equipment: r.allow_foreign_equipment,
            parent: r.parent,
            creator_country: r.creator_country,
            origin_type: r.origin_type,
            fake_intel: r.fake_intel,
            division_cap: r.division_cap,
            equipment_niche: r.equipment_niche,
            role: r.role,
            ai_role: r.ai_role,
            ai_role_fallback: r.ai_role_fallback,
            total_manpower: r.total_manpower,
            command_power_cost: r.command_power_cost,
            training_time: r.training_time,
            forbidden_equipment: if row.equipment_failure.is_some() {
                None
            } else {
                span(variants, row.forbidden_equipment).map(|tokens| {
                    owned_vec(engine_allocation::collect_vec(
                        tokens.iter().map(|&token| token as u32),
                    ))
                })
            },
        }));
        entry.settings_serial = collector_projection::projection::generation();
    }
    entry.raw_settings = Some(row.settings);
    entry.acquired = true;
    entry.valid = true;
    entry.requested = false;
    *payload_bytes = *payload_bytes - previous_bytes + entry.allocated_bytes();
}
