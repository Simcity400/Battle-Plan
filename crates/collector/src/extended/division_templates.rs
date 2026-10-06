//! Template dictionary: the daily names and categories, and every hour what a
//! template edit changes at any hour, each with exact native text: the counter
//! inputs, the stored 3D-model inputs and the saved model choices. Both walks
//! read their sub-unit definitions through caller-owned slots that grow
//! off-thread.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec};
use engine::{Memory, creferenced_division_template as native, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{DivisionTemplateIconRow, DivisionTemplateRow, TableId, TemplateSubunit, tables},
};

pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::DivisionTemplates, TableId::DivisionTemplateIcons];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("division template dictionary and country census", 0, &[0]),
    ("division template counter and model inputs", 1, &[1]),
];

/// Distinct failure reasons one hour's summary keeps by name; the rest are
/// counted together.
pub const MODEL_REASONS: usize = 8;

/// This hour's DivisionTemplateIcons rows whose model inputs failed, by
/// reason, for status.json. Fixed size: nothing is allocated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelFailures {
    pub rows: u32,
    /// (reason, rows) in first-seen order.
    pub reasons: [(&'static str, u32); MODEL_REASONS],
    /// Rows whose reason did not fit `reasons`.
    pub other: u32,
}

impl ModelFailures {
    fn add(&mut self, reason: &'static str) {
        self.rows += 1;
        for slot in &mut self.reasons {
            if slot.1 == 0 {
                *slot = (reason, 1);
                return;
            }
            if slot.0 == reason {
                slot.1 += 1;
                return;
            }
        }
        self.other += 1;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
    counts: usize,
    variants: usize,
    icon_rows: usize,
    icon_bytes: usize,
    /// The daily walk's definition memo slots; a full memo only costs reads.
    memo: usize,
    /// The sub-unit database's length, for the hourly walk's two copies.
    database: usize,
    /// The hourly walk's per-country definition slots.
    locals: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 2048,
            bytes: 128 * 1024,
            counts: 16 * 1024,
            variants: 1024,
            icon_rows: 2048,
            icon_bytes: 32 * 1024,
            memo: 1024,
            database: 1024,
            locals: 256,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
    counts: Arena<native::SubunitCount>,
    variants: Arena<u64>,
    icon_rows: CaptureRows<native::IconRow>,
    icon_bytes: Arena<u8>,
    memo: Arena<native::MemoSlot>,
    model_failures: ModelFailures,
    database: Arena<usize>,
    globals: Arena<native::GlobalSlot>,
    locals: Arena<native::LocalSlot>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.icon_rows
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        // A failed capture has no rows of its own to count.
        self.model_failures = ModelFailures::default();
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            bytes: 0,
            counts: 0,
            variants: 0,
            icon_rows: 0,
            icon_bytes: 0,
            memo: 0,
            database: 0,
            locals: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
            counts: Arena::new(capacity.counts),
            variants: Arena::new(capacity.variants),
            icon_rows: CaptureRows::new(capacity.icon_rows),
            icon_bytes: Arena::new(capacity.icon_bytes),
            memo: Arena::new(capacity.memo),
            model_failures: ModelFailures::default(),
            database: Arena::new(capacity.database),
            globals: Arena::new(capacity.database),
            locals: Arena::new(capacity.locals),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
        self.counts.reset();
        self.variants.reset();
        self.icon_rows.reset();
        self.icon_bytes.reset();
        self.memo.reset();
        self.model_failures = ModelFailures::default();
        self.database.reset();
        self.globals.reset();
        self.locals.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.counts.allocated_bytes()
            + self.variants.allocated_bytes()
            + self.icon_rows.allocated_bytes()
            + self.icon_bytes.allocated_bytes()
            + self.memo.allocated_bytes()
            + self.database.allocated_bytes()
            + self.globals.allocated_bytes()
            + self.locals.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.bytes.grow(&mut capacity.bytes);
        self.counts.grow(&mut capacity.counts);
        self.variants.grow(&mut capacity.variants);
        self.icon_rows.grow(&mut capacity.icon_rows);
        self.icon_bytes.grow(&mut capacity.icon_bytes);
        self.memo.grow(&mut capacity.memo);
        let mut globals = capacity.database;
        self.database.grow(&mut capacity.database);
        self.globals.grow(&mut globals);
        self.locals.grow(&mut capacity.locals);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
    ) -> [u64; N] {
        self.reset();
        // The counter and model inputs every hour. The walk reads no names or
        // grids beyond the regiment grid of a template without a stored
        // priority unit.
        let icons = {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let start = qpc();
            let walk = native::walk_icons(
                memory,
                base,
                state,
                &mut self.icon_rows.rows,
                &mut self.icon_bytes.first,
                &mut self.counts.first,
                &mut self.variants.first,
                native::DefinitionSlots {
                    database: &mut self.database.first,
                    globals: &mut self.globals.first,
                    locals: &mut self.locals.first,
                },
            );
            self.icon_rows.outcome = walk.outcome;
            self.icon_bytes.len = walk.bytes_len;
            self.icon_bytes.required = walk.required_bytes;
            self.counts.len = walk.counts_len;
            self.counts.required = walk.required_counts;
            self.variants.len = walk.variants_len;
            self.variants.required = walk.required_variants;
            self.database.required = walk.required_database;
            self.globals.required = walk.required_database;
            self.locals.required = walk.required_locals;
            if let Some(rows) = self.icon_rows.rows.get(..walk.outcome.len()) {
                for reason in rows.iter().filter_map(|row| row.model_failure) {
                    self.model_failures.add(reason);
                }
            }
            qpc().saturating_sub(start).max(0) as u64
        };
        let _reads = crate::read_accounting::table(TABLES[0]);
        if !replay::cadence::should_walk(TABLES[0], hour, force) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return [0, icons];
        }
        let start = qpc();
        let first = native::walk(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.bytes.first,
            &mut self.memo.first,
        );
        self.rows.outcome = first.outcome;
        self.bytes.len = first.bytes_len;
        self.bytes.required = first.required_bytes;
        self.memo.required = first.required_memo;
        [qpc().saturating_sub(start).max(0) as u64, icons]
    }
    /// This hour's rows whose model inputs failed, by reason.
    pub(crate) fn model_failures(&self) -> ModelFailures {
        self.model_failures
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.icon_rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [self.templates(), self.icons()]
    }

    fn templates(&self) -> TableView {
        let outcome = self.rows.outcome;
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return unavailable("template byte arena invalid");
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return unavailable("template row length invalid");
        };
        if rows
            .iter()
            .any(|r| span(bytes, r.name).is_none() || span(bytes, r.name_key).is_none())
        {
            return unavailable("template row spans outside captured arenas");
        }
        table::<tables::DivisionTemplates, _>(outcome, &self.rows.rows, |r| DivisionTemplateRow {
            template: r.template,
            owner: r.owner,
            name: owned_vec(copy_vec(span(bytes, r.name).unwrap())),
            name_key: owned_vec(copy_vec(span(bytes, r.name_key).unwrap())),
            unit_type: r.unit_type,
        })
    }

    fn icons(&self) -> TableView {
        let outcome = self.icon_rows.outcome;
        let Some(bytes) = self.icon_bytes.first.get(..self.icon_bytes.len) else {
            return unavailable("template icon byte arena invalid");
        };
        let Some(counts) = self.counts.first.get(..self.counts.len) else {
            return unavailable("template grouped arena invalid");
        };
        let Some(variants) = self.variants.first.get(..self.variants.len) else {
            return unavailable("template variant arena invalid");
        };
        let Some(rows) = self.icon_rows.rows.get(..outcome.len()) else {
            return unavailable("template icon row length invalid");
        };
        if rows.iter().any(|r| {
            span(bytes, r.priority_unit_name).is_none()
                || span(bytes, r.model_entity).is_none()
                || span(counts, r.counts).is_none()
                || span(variants, r.excluded_variants).is_none()
        }) {
            return unavailable("template icon row spans outside captured arenas");
        }
        table::<tables::DivisionTemplateIcons, _>(outcome, &self.icon_rows.rows, |r| {
            DivisionTemplateIconRow {
                template: r.template,
                priority_unit: r.priority_unit,
                priority_unit_name: owned_vec(copy_vec(span(bytes, r.priority_unit_name).unwrap())),
                custom_icon: r.custom_icon,
                custom_icon_nato: r.custom_icon_nato,
                representative: r.representative.token,
                representative_local: u8::from(r.representative.local),
                subunits: owned_vec(engine::allocation::collect_vec(
                    span(counts, r.counts)
                        .unwrap()
                        .iter()
                        .map(|unit| TemplateSubunit {
                            token: unit.token,
                            local: u8::from(unit.local),
                            count: unit.count,
                        }),
                )),
                country: r.country,
                foreign_country: r.foreign_country,
                model_category: r.model_category,
                model_entity: owned_vec(copy_vec(span(bytes, r.model_entity).unwrap())),
                excluded_variants: owned_vec(copy_vec(
                    span(variants, r.excluded_variants).unwrap(),
                )),
                model_unreadable: u8::from(r.model_failure.is_some()),
            }
        })
    }
}

fn span<T>(arena: &[T], range: native::Span) -> Option<&[T]> {
    arena.get(range.start..range.start.checked_add(range.len)?)
}

#[cfg(test)]
mod tests;
