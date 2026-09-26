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
        self.rows.fail(super::failed_capture(TABLES[0], hour, force_full, reason));
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
        if rows.iter().any(|r| span(bytes, r.name).is_none()) {
            return unavailable("template row spans outside captured arenas");
        }
        table::<tables::DivisionTemplates, _>(outcome, &self.rows.rows, |r| DivisionTemplateRow {
            template: r.template,
            owner: r.owner,
            name: span(bytes, r.name).unwrap().to_vec(),
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
                priority_unit_name: span(bytes, r.priority_unit_name).unwrap().to_vec(),
                custom_icon: r.custom_icon,
                custom_icon_nato: r.custom_icon_nato,
                representative: r.representative.token,
                representative_local: u8::from(r.representative.local),
                subunits: span(counts, r.counts)
                    .unwrap()
                    .iter()
                    .map(|unit| TemplateSubunit {
                        token: unit.token,
                        local: u8::from(unit.local),
                        count: unit.count,
                    })
                    .collect(),
                country: r.country,
                foreign_country: r.foreign_country,
                model_category: r.model_category,
                model_entity: span(bytes, r.model_entity).unwrap().to_vec(),
                excluded_variants: span(variants, r.excluded_variants).unwrap().to_vec(),
                model_unreadable: u8::from(r.model_failure.is_some()),
            }
        })
    }
}

fn span<T>(arena: &[T], range: native::Span) -> Option<&[T]> {
    arena.get(range.start..range.start.checked_add(range.len)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{
        ccountry, cgame_state, csub_unit_definition as subunit,
        memory::{CountedMemory, SliceMemory},
    };

    /// Base zero; the subunit database pointer reads as 0x8800.
    struct Fixture {
        bytes: Vec<u8>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == subunit::DATABASE_RVA && output.len() == 8 {
                output.copy_from_slice(&0x8800_u64.to_le_bytes());
                return true;
            }
            SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }
    fn fixture() -> Fixture {
        fn word(bytes: &mut [u8], at: usize, value: u64) {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn array(bytes: &mut [u8], at: usize, data: u64, count: u32) {
            word(bytes, at, data);
            word(bytes, at + 8, u64::from(count) | (u64::from(count) << 32));
        }
        fn text(bytes: &mut [u8], at: usize, value: &[u8]) {
            bytes[at..at + value.len()].copy_from_slice(value);
            word(bytes, at + 16, value.len() as u64);
            word(bytes, at + 24, 15);
        }
        let mut bytes = vec![0; 0x9000];
        array(&mut bytes, 0x1000 + cgame_state::COUNTRIES, 0x2000, 2);
        array(&mut bytes, 0x1000 + cgame_state::COUNTRY_LINKS, 0x2100, 2);
        word(&mut bytes, 0x2008, 0x3000);
        bytes[0x2104..0x2108].copy_from_slice(&1_u32.to_le_bytes());
        word(&mut bytes, 0x3000, ccountry::VTABLE_RVA as u64);
        word(&mut bytes, 0x3008, 1);
        array(&mut bytes, 0x3000 + ccountry::DIVISION_TEMPLATES, 0x4000, 1);
        word(&mut bytes, 0x4000, 0x5000);
        word(&mut bytes, 0x5000, native::VTABLE_RVA as u64);
        word(&mut bytes, 0x5008, 0xffff_ffff_0000_0034);
        text(&mut bytes, 0x5000 + native::NAME, &[b'A', 0, 255]);
        text(&mut bytes, 0x5000 + native::MODEL_ENTITY, b"GER_x_entity");
        array(&mut bytes, 0x5000 + native::SUBUNIT_COUNTS, 0x6000, 1);
        word(&mut bytes, 0x6000, 0x7000);
        word(&mut bytes, 0x6008, 3);
        word(&mut bytes, 0x5000 + native::PRIORITY_UNIT, 0x7000);
        word(&mut bytes, 0x7000, subunit::VTABLE_RVA as u64);
        word(&mut bytes, 0x7000 + subunit::TOKEN, 777);
        bytes[0x7000 + subunit::IS_REAL] = 1;
        text(&mut bytes, 0x7000 + subunit::NAME, b"infantry");
        word(&mut bytes, 0x7000 + subunit::DATABASE_INDEX, 1);
        array(&mut bytes, 0x8800 + subunit::DATABASE_VECTOR, 0x8900, 2);
        word(&mut bytes, 0x8908, 0x7000);
        Fixture { bytes }
    }
    fn capacity() -> Capacities {
        Capacities {
            rows: 2,
            bytes: 32,
            counts: 4,
            variants: 4,
            icon_rows: 2,
            icon_bytes: 32,
            memo: 8,
            database: 8,
            locals: 8,
        }
    }

    #[test]
    fn daily_refresh_reads_templates_and_bad_memory_cannot_claim_empty() {
        let mut capture = Captured::with_capacities(&capacity());
        let present = CountedMemory::new(fixture());
        let missing = CountedMemory::new(SliceMemory {
            base: 0,
            bytes: &[],
        });
        for (hour, force) in (-24..24).map(|hour| (hour, false)).chain([(0, true)]) {
            let reads = crate::read_accounting::Capture::new();
            capture.capture(&present, 0, 0x1000, hour, force);
            let read = reads.finish();
            // The counter inputs are walked every hour.
            assert!(read[TABLES[1].index()].spans > 0);
            if !replay::cadence::should_walk(TABLES[0], hour, force) {
                assert_eq!(read[TABLES[0].index()], crate::read_accounting::ZERO[TABLES[0].index()]);
                assert_eq!(
                    capture.outcomes(),
                    [
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        },
                        Outcome::Rows { len: 1 }
                    ]
                );
                continue;
            }
            assert!(read[TABLES[0].index()].spans > 0);
            assert_eq!(
                capture.outcomes(),
                [Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 }]
            );

            let reads = crate::read_accounting::Capture::new();
            capture.capture(&missing, 0, 0x1000, hour, force);
            assert!(reads.finish()[TABLES[0].index()].spans > 0);
            let [Outcome::Unavailable { reason }, Outcome::Unavailable { .. }] =
                capture.outcomes()
            else {
                panic!("unreadable templates must fail at hour {hour}");
            };
            assert_ne!(reason, replay::cadence::SKIPPED_REASON);
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
            assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
        }
    }

    #[test]
    fn a_failed_capture_counts_no_model_failures() {
        let mut capture = Captured::with_capacities(&capacity());
        capture.model_failures.add("template grouped subunits unreadable");
        assert_eq!(capture.model_failures().rows, 1);
        capture.fail_capture(3, false, "slow acquisition failed");
        assert_eq!(capture.model_failures(), ModelFailures::default());
    }

    #[test]
    fn conversion_preserves_exact_bytes_and_rejects_every_invalid_span() {
        let mut capture = Captured::with_capacities(&capacity());
        capture.capture(&fixture(), 0, 0x1000, 3, true);
        let [
            TableView::Complete(replay::rows::TableRows::DivisionTemplates(rows)),
            TableView::Complete(replay::rows::TableRows::DivisionTemplateIcons(icons)),
        ] = capture.tables()
        else {
            panic!("complete templates");
        };
        assert_eq!(rows[0].template, 0xffff_ffff_0000_0034);
        assert_eq!(rows[0].name, &[b'A', 0, 255]);
        assert_eq!(icons[0].model_entity, b"GER_x_entity");
        assert_eq!((icons[0].representative, icons[0].representative_local), (777, 0));
        assert_eq!(
            icons[0].subunits,
            [TemplateSubunit {
                token: 777,
                local: 0,
                count: 3
            }]
        );
        assert_eq!(icons[0].template, 0xffff_ffff_0000_0034);
        assert_eq!(icons[0].priority_unit, 777);
        assert_eq!(icons[0].priority_unit_name, b"infantry");
        // A row whose model inputs failed publishes the flag, keeps its
        // counter fields and is counted by reason.
        let good = capture.icon_rows.rows[0];
        capture.icon_rows.rows[0].model_failure = Some("template grouped subunits unreadable");
        let [_, TableView::Complete(replay::rows::TableRows::DivisionTemplateIcons(flagged))] =
            capture.tables()
        else {
            panic!("complete icons");
        };
        assert_eq!(flagged[0].model_unreadable, 1);
        assert_eq!(flagged[0].priority_unit_name, b"infantry");
        capture.icon_rows.rows[0] = good;
        let mut failures = ModelFailures::default();
        for reason in ["a", "b", "a"] {
            failures.add(reason);
        }
        assert_eq!(
            (failures.rows, &failures.reasons[..3], failures.other),
            (3, &[("a", 2), ("b", 1), ("", 0)][..], 0)
        );
        capture.rows.rows[0].name = native::Span {
            start: usize::MAX,
            len: 1,
        };
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        let original = capture.icon_rows.rows[0];
        for which in 0..4 {
            capture.icon_rows.rows[0] = original;
            let row = &mut capture.icon_rows.rows[0];
            let span = match which {
                0 => &mut row.priority_unit_name,
                1 => &mut row.model_entity,
                2 => &mut row.counts,
                _ => &mut row.excluded_variants,
            };
            *span = native::Span {
                start: usize::MAX,
                len: 1,
            };
            assert!(matches!(capture.tables()[1], TableView::Unavailable { .. }));
        }
    }
}
