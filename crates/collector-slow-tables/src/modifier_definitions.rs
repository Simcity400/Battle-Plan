use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_native_memory::{Memory, walk::Outcome};
use engine_native_parts::modifier_definition as native;
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::ModifierDefinitionRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ModifierDefinitions];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    text: usize,
}

collector_primitives::capacity_max!(Capacities { rows, text });

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
    ) -> Result<(), &'static str> {
        let (rows, text) = native::storage_demand(memory, base)?;
        crate::country_scale::raise(&mut self.rows, rows);
        crate::country_scale::raise(&mut self.text, text);
        Ok(())
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    text: Arena<u8>,
    pub header_changed: bool,
}

impl Captured {
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let scheduled = force_full || self.header_changed;
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, scheduled, reason));
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, text: 0 })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            text: Arena::new(capacity.text),
            header_changed: false,
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.text.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.text.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        self.text.grow(&mut capacity.text);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        if !should_walk(TABLES[0], hour, force_full || self.header_changed) {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
            return [0];
        }
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        let walk = native::walk(memory, base, &mut self.rows.rows, &mut self.text.first);
        self.rows.outcome = walk.outcome;
        self.text.len = walk.text_len;
        self.text.required = walk.text_required;
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let Some(text) = self.text.first.get(..self.text.len) else {
            return [unavailable("modifier definition text arena invalid")];
        };
        let bytes = |span: native::Span| {
            span.start
                .checked_add(span.len)
                .and_then(|end| text.get(span.start..end))
        };
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return [unavailable("modifier definition row count invalid")];
        };
        if rows.iter().any(|row| {
            [row.name_key, row.postfix_key, row.argument]
                .into_iter()
                .any(|span| bytes(span).is_none())
        }) {
            return [unavailable(
                "modifier definition strings outside captured arena",
            )];
        }
        [table::<tables::ModifierDefinitions, _>(
            self.rows.outcome,
            rows,
            |row| ModifierDefinitionRow {
                index: row.index,
                token: row.token,
                name_key: owned_vec(copy_vec(bytes(row.name_key).unwrap())),
                postfix_key: owned_vec(copy_vec(bytes(row.postfix_key).unwrap())),
                argument: owned_vec(copy_vec(bytes(row.argument).unwrap())),
                flags: row.flags,
                category_mask: row.category_mask,
                feature_selector: row.feature_selector,
                precision: row.precision,
                eligibility: row.eligibility,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    #[test]
    fn static_table_walks_on_full_hours_and_when_the_header_moved() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        let skipped = Outcome::Unavailable {
            reason: SKIPPED_REASON,
        };
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            assert_eq!(captured.outcomes()[0], skipped, "{hour}");
        }
        for (force_full, header_changed) in [(true, false), (false, true)] {
            captured.header_changed = header_changed;
            captured.capture(&memory, 0, 0x1000, 5, force_full);
            assert!(matches!(
                captured.outcomes()[0],
                Outcome::Unavailable { reason } if reason != SKIPPED_REASON
            ));
        }
        captured.header_changed = true;
        captured.fail_capture(5, false, "worker stopped");
        assert_eq!(
            captured.outcomes()[0],
            Outcome::Unavailable {
                reason: "worker stopped"
            }
        );
        captured.header_changed = false;
        captured.fail_capture(5, false, "worker stopped");
        assert_eq!(captured.outcomes()[0], skipped);
    }

    #[test]
    fn conversion_keeps_exact_strings_and_refuses_escaped_spans() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 1, text: 8 });
        captured.text.first[..8].copy_from_slice(b"KEYMOD\0x");
        captured.text.len = 8;
        let span = |start, len| native::Span { start, len };
        captured.rows.rows[0] = native::Row {
            index: 0x8d,
            token: 0x3433,
            name_key: span(0, 3),
            postfix_key: span(3, 3),
            argument: span(6, 2),
            flags: 3,
            category_mask: 8,
            feature_selector: 0,
            precision: -1,
            eligibility: 1,
        };
        captured.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::ModifierDefinitions(rows))] = captured.tables() else {
            panic!("complete modifier definitions");
        };
        assert_eq!(
            rows[0],
            ModifierDefinitionRow {
                index: 0x8d,
                token: 0x3433,
                name_key: b"KEY".to_vec(),
                postfix_key: b"MOD".to_vec(),
                argument: b"\0x".to_vec(),
                flags: 3,
                category_mask: 8,
                feature_selector: 0,
                precision: -1,
                eligibility: 1,
            }
        );
        captured.rows.rows[0].argument = span(7, 2);
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }
}
