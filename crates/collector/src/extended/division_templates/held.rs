//! Immutable extension payloads retained under native writer certificates.
//! Ordinary hours reuse their allocations and acquire only dirty templates.
use super::*;
use crate::template_changes::Changes;
use std::sync::Arc;

type Stamp = (u64, u64);
type Certificate = [Stamp; 2];

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
    layout: Option<Arc<TemplateLayout>>,
    settings: Option<Arc<TemplateSettings>>,
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
        store.begin(&watches);
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
        store.begin(&watches);
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
        assert!(
            store.request(&watches, 1, row.pointer, row.template, 1),
            "roster reordering must refresh the first-name-match operand"
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
        crate::test_support::with_allocations_denied(|| {
            store.begin(&watches);
            assert!(!store.request(&watches, row.owner, row.pointer, row.template, 0));
            struct QuietOnly((u64, u64));
            impl Changes for QuietOnly {
                fn quiet(&self) -> Option<(u64, u64)> {
                    Some(self.0)
                }
                fn sequence(&self, _: usize) -> Option<(u64, u64)> {
                    panic!("quiet finalization must not walk receiver certificates again")
                }
                fn mismatch(&self) {
                    unreachable!()
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
        crate::test_support::with_allocations_denied(|| {
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
    live: Vec<u64>,
    payload_bytes: usize,
    before: Option<(u64, u64)>,
    requested: bool,
}

fn certificate(changes: &dyn Changes, pointer: usize, owner: u32) -> Option<Certificate> {
    Some([
        changes.sequence(pointer)?,
        changes.sequence(owner as usize)?,
    ])
}

pub(super) struct Demand<'a> {
    store: &'a mut Store,
    changes: &'a dyn Changes,
    core: &'a mut super::core::Store,
    dependencies: Option<Stamp>,
    owner: Option<(u64, u64)>,
    batch: [(usize, Option<Stamp>); native::READ_BATCH],
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
            dependencies: None,
            owner: None,
            batch: [(0, None); native::READ_BATCH],
        }
    }
}
impl native::DetailPolicy for Demand<'_> {
    fn before_country(&mut self, country: u32) {
        self.owner = self.changes.sequence(country as usize);
    }
    fn before_records(&mut self, templates: &[usize]) {
        self.dependencies = self.changes.core_dependencies();
        for (slot, &pointer) in self.batch.iter_mut().zip(templates) {
            *slot = (pointer, self.changes.sequence(pointer));
        }
    }
    fn demand(&mut self, country: u32, pointer: usize, template: u64, ordinal: u32) -> bool {
        let (expected, stamp) = self.batch[ordinal as usize % native::READ_BATCH];
        let before = stamp
            .zip(self.owner)
            .map(|(template, owner)| [template, owner]);
        self.core.prepare(
            template,
            pointer,
            country,
            before
                .filter(|_| expected == pointer)
                .zip(self.dependencies)
                .map(|([template, owner], dependencies)| [template, owner, dependencies]),
        );
        self.store.request_certified(
            country,
            pointer,
            template,
            ordinal,
            (expected == pointer).then_some(before).flatten(),
            self.changes.fresh_only(),
        )
    }
    fn core(&mut self, record: &[u8]) -> Option<native::IconCore<'_>> {
        self.core.carry(record, self.changes)
    }
    fn remember_core(
        &mut self,
        record: &[u8],
        row: &native::IconRow,
        bytes: &[u8],
        counts: &[native::SubunitCount],
        variants: &[u64],
    ) {
        self.core
            .remember(self.changes, record, row, bytes, counts, variants);
    }
}

impl Store {
    pub fn forget(&mut self) {
        self.entries.clear();
        self.payload_bytes = 0;
        self.before = None;
    }

    pub fn begin(&mut self, changes: &dyn Changes) {
        self.before = changes.quiet();
        self.requested = false;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.live.capacity() * size_of::<u64>()
            + self.payload_bytes
    }

    /// Only a complete existing census may retire payloads. Ordinary quiet
    /// hours do not walk the cache again.
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
    fn request_certified(
        &mut self,
        owner: u32,
        pointer: usize,
        template: u64,
        ordinal: u32,
        current: Option<Certificate>,
        fresh: bool,
    ) -> bool {
        let at = match self
            .entries
            .binary_search_by_key(&template, |entry| entry.template)
        {
            Ok(at) => at,
            Err(at) => {
                if engine_allocation::reserve(&mut self.entries, 1).is_err() {
                    return false;
                }
                self.entries.insert(
                    at,
                    Entry {
                        template,
                        ..Default::default()
                    },
                );
                at
            }
        };
        let entry = &mut self.entries[at];
        let same = entry.pointer == pointer && entry.owner == owner && entry.ordinal == ordinal;
        if current.is_none() && !fresh {
            entry.valid = false;
            entry.attempted = false;
            entry.acquired = false;
            entry.requested = false;
            return false;
        }
        let read = fresh || !same || !entry.attempted || current != entry.certificate;
        if read {
            self.requested = true;
            entry.attempted = true;
            entry.certificate = current;
            entry.acquired = false;
        }
        entry.pointer = pointer;
        entry.owner = owner;
        entry.ordinal = ordinal;
        entry.pending = current;
        entry.requested = read;
        entry.valid = !read && entry.acquired && current.is_some() && current == entry.certificate;
        read
    }

    pub fn certify(&mut self, changes: &dyn Changes, rows: &[native::IconRow]) {
        if !self.requested && self.before.is_some() && self.before == changes.quiet() {
            return;
        }
        for row in rows {
            let Ok(at) = self
                .entries
                .binary_search_by_key(&row.template, |entry| entry.template)
            else {
                continue;
            };
            let entry = &mut self.entries[at];
            if changes.fresh_only() {
                entry.valid = entry.requested && row.details_fresh;
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
            if entry.requested {
                entry.attempted = true;
                entry.certificate = entry.pending;
                entry.valid = row.details_fresh;
            }
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

    /// Certificates are frozen by `certify`; only owned payloads are reached.
    pub fn settle(
        &mut self,
        rows: &[native::IconRow],
        counts: &[native::SubunitCount],
        variants: &[u64],
        bytes: &[u8],
    ) {
        if !self.requested {
            return;
        }
        for row in rows {
            let Ok(at) = self
                .entries
                .binary_search_by_key(&row.template, |entry| entry.template)
            else {
                continue;
            };
            let entry = &mut self.entries[at];
            if !entry.requested || !entry.valid {
                continue;
            }
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
            }
            entry.raw_settings = Some(row.settings);
            entry.acquired = true;
            entry.valid = true;
            entry.requested = false;
            self.payload_bytes = self.payload_bytes - previous_bytes + entry.allocated_bytes();
        }
        self.requested = false;
    }

    pub fn settings_available(&self, row: &native::IconRow) -> bool {
        self.entries
            .binary_search_by_key(&row.template, |entry| entry.template)
            .ok()
            .is_some_and(|at| {
                let entry = &self.entries[at];
                entry.valid
                    && entry.pointer == row.pointer
                    && entry.owner == row.owner
                    && (entry.requested || entry.settings.is_some())
            })
    }

    fn value_refs(
        &self,
        row: &native::IconRow,
    ) -> (Option<&Arc<TemplateLayout>>, Option<&Arc<TemplateSettings>>) {
        let entry = self
            .entries
            .binary_search_by_key(&row.template, |entry| entry.template)
            .ok()
            .map(|at| &self.entries[at])
            .filter(|entry| {
                entry.valid && entry.pointer == row.pointer && entry.owner == row.owner
            });
        match entry.and_then(|entry| entry.settings.as_ref().map(|settings| (entry, settings))) {
            Some((entry, settings)) => (entry.layout.as_ref(), Some(settings)),
            None => (None, None),
        }
    }

    pub fn matches_values(
        &self,
        row: &native::IconRow,
        layout: &Recorded<Option<Arc<TemplateLayout>>>,
        settings: &Recorded<Option<Arc<TemplateSettings>>>,
    ) -> bool {
        let (Recorded::Value(layout), Recorded::Value(settings)) = (layout, settings) else {
            return false;
        };
        let (current_layout, current_settings) = self.value_refs(row);
        current_layout == layout.as_ref() && current_settings == settings.as_ref()
    }

    // Preserve the independent recorded layout/settings availability in the borrowed view.
    #[allow(clippy::type_complexity)]
    pub fn values(
        &self,
        row: &native::IconRow,
    ) -> (
        Recorded<Option<Arc<TemplateLayout>>>,
        Recorded<Option<Arc<TemplateSettings>>>,
    ) {
        let (layout, settings) = self.value_refs(row);
        (
            Recorded::Value(layout.cloned()),
            Recorded::Value(settings.cloned()),
        )
    }
}
