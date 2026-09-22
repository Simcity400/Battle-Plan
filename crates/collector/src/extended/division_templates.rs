//! Scheduled template dictionary, with exact native text and grid-slot arenas.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, creferenced_division_template as native, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{DivisionTemplateRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::DivisionTemplates];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("division template dictionary and country census", 0, &[0])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
    units: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 2048,
            bytes: 128 * 1024,
            units: 128 * 1024,
        }
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
    units: Arena<u32>,
}
impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows.fail(super::failed_capture(TABLES[0], hour, force_full, reason));
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            bytes: 0,
            units: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
            units: Arena::new(capacity.units),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
        self.units.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.bytes.allocated_bytes() + self.units.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.bytes.grow(&mut capacity.bytes);
        self.units.grow(&mut capacity.units);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force: bool,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        if !replay::cadence::should_walk(TABLES[0], hour, force) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            return [0];
        }
        let start = qpc();
        let first = native::walk(
            memory,
            base,
            state,
            &mut self.rows.rows,
            &mut self.bytes.first,
            &mut self.units.first,
        );
        self.rows.outcome = first.outcome;
        self.bytes.len = first.bytes_len;
        self.bytes.required = first.required_bytes;
        self.units.len = first.units_len;
        self.units.required = first.required_units;
        [qpc().saturating_sub(start).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        fn span<T>(arena: &[T], range: native::Span) -> Option<&[T]> {
            arena.get(range.start..range.start.checked_add(range.len)?)
        }
        let outcome = self.outcomes()[0];
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return [unavailable("template byte arena invalid")];
        };
        let Some(units) = self.units.first.get(..self.units.len) else {
            return [unavailable("template unit arena invalid")];
        };
        let Some(rows) = self.rows.rows.get(..outcome.len()) else {
            return [unavailable("template row length invalid")];
        };
        if rows.iter().any(|r| {
            span(bytes, r.name).is_none()
                || span(bytes, r.priority_unit_name).is_none()
                || r.composition.iter().any(|&r| span(units, r).is_none())
        }) {
            return [unavailable("template row spans outside captured arenas")];
        }
        [table::<tables::DivisionTemplates, _>(
            outcome,
            &self.rows.rows,
            |r| DivisionTemplateRow {
                template: r.template,
                owner: r.owner,
                name: span(bytes, r.name).unwrap().to_vec(),
                priority_unit: r.priority_unit,
                priority_unit_name: span(bytes, r.priority_unit_name).unwrap().to_vec(),
                unit_type: r.unit_type,
                custom_icon: r.custom_icon,
                custom_icon_nato: r.custom_icon_nato,
                dimensions: r.dimensions.to_vec(),
                regiments: span(units, r.composition[0]).unwrap().to_vec(),
                support: span(units, r.composition[1]).unwrap().to_vec(),
                regimental_support: span(units, r.composition[2]).unwrap().to_vec(),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{
        ccountry, cgame_state, csub_unit_definition as subunit,
        memory::{CountedMemory, SliceMemory},
    };

    struct Fixture {
        bytes: Vec<u8>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
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
        word(&mut bytes, 0x5000 + native::GRID_DIMENSIONS, 0x0001_0001);
        array(&mut bytes, 0x5000 + native::REGIMENT_DATA, 0x6000, 1);
        word(&mut bytes, 0x6000, 0x7000);
        word(&mut bytes, 0x5000 + native::PRIORITY_UNIT, 0x7000);
        word(&mut bytes, 0x7000, subunit::VTABLE_RVA as u64);
        word(&mut bytes, 0x7000 + subunit::TOKEN, 777);
        bytes[0x7000 + subunit::IS_REAL] = 1;
        text(&mut bytes, 0x7000 + subunit::NAME, b"infantry");
        Fixture { bytes }
    }
    fn capacity() -> Capacities {
        Capacities {
            rows: 2,
            bytes: 32,
            units: 4,
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
            if !replay::cadence::should_walk(TABLES[0], hour, force) {
                assert_eq!(reads.finish(), crate::read_accounting::ZERO);
                assert_eq!(
                    capture.outcomes(),
                    [Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON
                    }]
                );
                continue;
            }
            assert!(reads.finish()[TABLES[0].index()].spans > 0);
            assert_eq!(capture.outcomes(), [Outcome::Rows { len: 1 }]);

            let reads = crate::read_accounting::Capture::new();
            capture.capture(&missing, 0, 0x1000, hour, force);
            assert!(reads.finish()[TABLES[0].index()].spans > 0);
            let [Outcome::Unavailable { reason }] = capture.outcomes() else {
                panic!("unreadable templates must fail at hour {hour}");
            };
            assert_ne!(reason, replay::cadence::SKIPPED_REASON);
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        }
    }

    #[test]
    fn conversion_preserves_exact_bytes_and_rejects_every_invalid_span() {
        let mut capture = Captured::with_capacities(&capacity());
        capture.capture(&fixture(), 0, 0x1000, 3, true);
        let [TableView::Complete(replay::rows::TableRows::DivisionTemplates(rows))] =
            capture.tables()
        else {
            panic!("complete templates");
        };
        assert_eq!(rows[0].template, 0xffff_ffff_0000_0034);
        assert_eq!(rows[0].name, &[b'A', 0, 255]);
        assert_eq!(rows[0].priority_unit_name, b"infantry");
        assert_eq!(rows[0].regiments, [777]);
        let original = capture.rows.rows[0];
        for which in 0..5 {
            capture.rows.rows[0] = original;
            let row = &mut capture.rows.rows[0];
            let span = match which {
                0 => &mut row.name,
                1 => &mut row.priority_unit_name,
                _ => &mut row.composition[which - 2],
            };
            *span = native::Span {
                start: usize::MAX,
                len: 1,
            };
            assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        }
    }
}
