//! Each country's appointed advisors in native order, with the advisor's
//! character, idea token and slot, daily in its own slot (the advisor roles
//! are read every hour, so also at that slot); each country's active-character
//! records (CountryCharacters) and retired ones (CountryRetiredCharacters),
//! hourly, in one walk; and the character manager's advisor idea-token set
//! (AdvisorTokens), hourly.
use crate::{
    sampling::qpc,
    spooling::{table, table_iter, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::allocation::{copy_vec, owned_vec, reserve_exact};
use engine::{Memory, advisor_tokens, ccountry_characters as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{AdvisorTokenRow, CountryCharacterRow, HiredAdvisorRow, TableId, tables},
};

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::HiredAdvisors,
    TableId::CountryCharacters,
    TableId::AdvisorTokens,
    TableId::CountryRetiredCharacters,
    TableId::AvailableAdvisors,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("active and retired character records", 1, &[1, 3])];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
    active: usize,
    tokens: usize,
    token_bytes: usize,
    retired: usize,
    available: usize,
    available_bytes: usize,
}

impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (rows, bytes) = native::demand_appointed(memory, base, state)?;
        self.raise_for_appointed_advisors(rows, bytes);
        Ok(())
    }
    pub(crate) fn raise_for_appointed_advisors(&mut self, count: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows, rows(count, 1));
        raise(&mut self.bytes, rows(bytes, 1));
    }
    /// CountryCharacters and CountryRetiredCharacters from the live record
    /// totals (`country_scale::observe_politics`), with the three-quarters
    /// growth rule's headroom. Never lowers.
    pub(crate) fn raise_for_character_records(&mut self, active: usize, retired: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.active, rows(active, 1));
        raise(&mut self.retired, rows(retired, 1));
    }

    /// AdvisorTokens from the live key count and key bytes
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_advisor_tokens(&mut self, tokens: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.tokens, rows(tokens, 1));
        raise(&mut self.token_bytes, rows(bytes, 1));
    }

    pub(crate) fn raise_for_available_advisors(&mut self, count: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.available, rows(count, 1));
        raise(&mut self.available_bytes, rows(bytes, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
    active: CaptureRows<native::ActiveRow>,
    tokens: CaptureRows<advisor_tokens::Row>,
    token_bytes: Arena<u8>,
    retired: CaptureRows<native::ActiveRow>,
    available: CaptureRows<native::Row>,
    available_bytes: Arena<u8>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.active
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.tokens
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
        self.retired
            .fail(super::failed_capture(TABLES[3], hour, force_full, reason));
        self.available
            .fail(super::failed_capture(TABLES[4], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            bytes: 0,
            active: 0,
            tokens: 0,
            token_bytes: 0,
            retired: 0,
            available: 0,
            available_bytes: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
            active: CaptureRows::new(capacity.active),
            tokens: CaptureRows::new(capacity.tokens),
            token_bytes: Arena::new(capacity.token_bytes),
            retired: CaptureRows::new(capacity.retired),
            available: CaptureRows::new(capacity.available),
            available_bytes: Arena::new(capacity.available_bytes),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
        self.active.reset();
        self.tokens.reset();
        self.token_bytes.reset();
        self.retired.reset();
        self.available.reset();
        self.available_bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.active.allocated_bytes()
            + self.tokens.allocated_bytes()
            + self.token_bytes.allocated_bytes()
            + self.retired.allocated_bytes()
            + self.available.allocated_bytes()
            + self.available_bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        // A truncated walk reports at least the bytes of its first unread row.
        self.bytes.grow(&mut capacity.bytes);
        self.active.grow(&mut capacity.active);
        self.tokens.grow(&mut capacity.tokens);
        // The token walk reports every key's bytes, read or not.
        self.token_bytes.grow(&mut capacity.token_bytes);
        self.retired.grow(&mut capacity.retired);
        self.available.grow(&mut capacity.available);
        self.available_bytes.grow(&mut capacity.available_bytes);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        if should_walk(TABLES[0], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let started = qpc();
            let walk = native::walk_growing(
                memory,
                base,
                game_state,
                &mut self.rows.rows,
                &mut self.bytes.first,
            );
            self.rows.outcome = walk.outcome;
            self.bytes.len = walk.bytes_len;
            self.bytes.required = walk.required_bytes;
            ticks[0] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.rows.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
        }
        // Both record arrays come from one walk of each country's object,
        // charged to CountryCharacters.
        let active = should_walk(TABLES[1], hour, force_full);
        let retired = should_walk(TABLES[3], hour, force_full);
        if active || retired {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            let [active_outcome, retired_outcome] = native::walk_records_buffer(
                memory,
                base,
                game_state,
                if active {
                    engine::walk::Buffer::Growing(&mut self.active.rows)
                } else {
                    engine::walk::Buffer::Fixed(&mut [])
                },
                if retired {
                    engine::walk::Buffer::Growing(&mut self.retired.rows)
                } else {
                    engine::walk::Buffer::Fixed(&mut [])
                },
            );
            self.active.outcome = active_outcome;
            self.retired.outcome = retired_outcome;
            ticks[1] = qpc().saturating_sub(started).max(0) as u64;
        }
        for (walked, rows) in [(active, &mut self.active), (retired, &mut self.retired)] {
            if !walked {
                rows.outcome = Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                };
            }
        }
        if should_walk(TABLES[2], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            let walk = advisor_tokens::walk_growing(
                memory,
                base,
                game_state,
                &mut self.tokens.rows,
                &mut self.token_bytes.first,
            );
            self.tokens.outcome = walk.outcome;
            self.token_bytes.len = walk.bytes_len;
            self.token_bytes.required = walk.required_bytes;
            ticks[2] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.tokens.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
        }
        if should_walk(TABLES[4], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[4]);
            let started = qpc();
            let walk = native::walk_available_growing(
                memory,
                base,
                game_state,
                &mut self.available.rows,
                &mut self.available_bytes.first,
            );
            self.available.outcome = walk.outcome;
            self.available_bytes.len = walk.bytes_len;
            self.available_bytes.required = walk.required_bytes;
            ticks[4] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.available.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
        }
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.rows.outcome,
            self.active.outcome,
            self.tokens.outcome,
            self.retired.outcome,
            self.available.outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        let record = |row: &native::ActiveRow| CountryCharacterRow {
            country: row.country,
            ordinal: row.ordinal,
            character: row.character,
        };
        [
            self.hired_table(),
            table::<tables::CountryCharacters, _>(self.active.outcome, &self.active.rows, record),
            self.token_table(),
            table::<tables::CountryRetiredCharacters, _>(
                self.retired.outcome,
                &self.retired.rows,
                record,
            ),
            self.available_table(),
        ]
    }
    fn available_table(&self) -> TableView {
        let Some(bytes) = self.available_bytes.first.get(..self.available_bytes.len) else {
            return unavailable("available advisor string arena invalid");
        };
        let Some(rows) = self.available.rows.get(..self.available.outcome.len()) else {
            return unavailable("available advisor row count invalid");
        };
        let text = |row: &native::Row| {
            row.idea_start
                .checked_add(row.idea_len)
                .and_then(|end| bytes.get(row.idea_start..end))
        };
        if rows.iter().any(|row| text(row).is_none()) {
            return unavailable("available advisor idea outside captured arena");
        }
        table::<tables::AvailableAdvisors, _>(self.available.outcome, rows, |row| {
            replay::rows::AvailableAdvisorRow {
                country: row.country,
                ordinal: row.ordinal,
                character: row.character,
                idea: owned_vec(copy_vec(text(row).unwrap())),
            }
        })
    }
    /// The keys sorted by their bytes, numbered in that order: the set's
    /// bucket order follows hashes and insertion history, not a meaning.
    fn token_table(&self) -> TableView {
        let Some(bytes) = self.token_bytes.first.get(..self.token_bytes.len) else {
            return unavailable("advisor token arena invalid");
        };
        let Some(rows) = self.tokens.rows.get(..self.tokens.outcome.len()) else {
            return unavailable("advisor token row count invalid");
        };
        let mut keys = Vec::new();
        if reserve_exact(&mut keys, rows.len()).is_err() {
            return unavailable("collector allocation failed");
        }
        for row in rows {
            match row
                .start
                .checked_add(row.len)
                .and_then(|end| bytes.get(row.start..end))
            {
                Some(key) => keys.push(key),
                None => return unavailable("advisor token outside captured arena"),
            }
        }
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return unavailable("advisor token set holds a key twice");
        }
        table_iter::<tables::AdvisorTokens, _>(
            self.tokens.outcome,
            keys.iter().enumerate(),
            |(ordinal, key)| AdvisorTokenRow {
                ordinal: ordinal as u32,
                token: owned_vec(copy_vec(key)),
            },
        )
    }
    fn hired_table(&self) -> TableView {
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return unavailable("hired advisor string arena invalid");
        };
        let text =
            |start: usize, len: usize| start.checked_add(len).and_then(|end| bytes.get(start..end));
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return unavailable("hired advisor row count invalid");
        };
        if rows.iter().any(|row| {
            text(row.idea_start, row.idea_len).is_none()
                || text(row.slot_start, row.slot_len).is_none()
        }) {
            return unavailable("hired advisor strings outside captured arena");
        }
        table::<tables::HiredAdvisors, _>(self.rows.outcome, rows, |row| HiredAdvisorRow {
            country: row.country,
            ordinal: row.ordinal,
            character: row.character,
            idea: owned_vec(copy_vec(text(row.idea_start, row.idea_len).unwrap())),
            slot: owned_vec(copy_vec(text(row.slot_start, row.slot_len).unwrap())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn walks_only_on_its_daily_slot_or_a_full_hour() {
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            assert!(matches!(
                captured.outcomes()[4],
                Outcome::Unavailable { reason } if reason != SKIPPED_REASON
            ));
            let skipped = captured.outcomes()[0]
                == Outcome::Unavailable {
                    reason: SKIPPED_REASON,
                };
            assert_eq!(skipped, !should_walk(TABLES[0], hour, false), "{hour}");
            // The advisor roles it names are hourly, so read at its slot too.
            assert!(should_walk(TableId::AdvisorRoles, hour, false));
        }
        captured.capture(&memory, 0, 0x1000, 3, true);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { reason } if reason != SKIPPED_REASON
        ));
    }

    #[test]
    fn capacities_follow_live_appointed_counts_and_token_bytes() {
        let mut capacity = Capacities::default();
        assert_eq!((capacity.rows, capacity.bytes), (0, 0));
        capacity.raise_for_appointed_advisors(5_000 * 137, 5_000 * 137 * 311);
        assert!(capacity.rows >= 5_000 * 137);
        assert!(capacity.bytes >= 5_000 * 137 * 311);
        let raised = capacity;
        capacity.raise_for_appointed_advisors(10, 1);
        assert!(capacity == raised);
    }

    #[test]
    fn conversion_keeps_order_and_exact_bytes_and_refuses_escaped_spans() {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: 2,
            bytes: 16,
            active: 0,
            tokens: 0,
            token_bytes: 0,
            retired: 0,
            available: 0,
            available_bytes: 0,
        });
        captured.bytes.first[..12].copy_from_slice(b"idea\0slotabc");
        captured.bytes.len = 12;
        captured.rows.rows[0] = native::Row {
            country: 7,
            ordinal: 0,
            character: 1482,
            idea_start: 0,
            idea_len: 5,
            slot_start: 5,
            slot_len: 4,
        };
        captured.rows.rows[1] = native::Row {
            country: 7,
            ordinal: 1,
            character: 1451,
            idea_start: 9,
            idea_len: 3,
            slot_start: 12,
            slot_len: 0,
        };
        captured.rows.outcome = Outcome::Rows { len: 2 };
        let [
            TableView::Complete(TableRows::HiredAdvisors(rows)),
            _,
            _,
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete hired advisors");
        };
        assert_eq!(rows[0].idea, b"idea\0");
        assert_eq!(rows[0].slot, b"slot");
        assert_eq!((rows[1].ordinal, rows[1].character), (1, 1451));
        assert!(rows[1].slot.is_empty());
        captured.rows.rows[1].slot_len = 1;
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn active_characters_walk_every_hour_and_keep_null_records() {
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities {
            active: 2,
            retired: 1,
            ..Default::default()
        });
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            // Hourly: never skipped, and here unreadable rather than stale.
            for index in [1, 3] {
                assert!(matches!(
                    captured.outcomes()[index],
                    Outcome::Unavailable { reason } if reason != SKIPPED_REASON
                ));
            }
        }
        captured.reset();
        captured.active.rows[0] = native::ActiveRow {
            country: 7,
            ordinal: 0,
            character: None,
        };
        captured.active.rows[1] = native::ActiveRow {
            country: 7,
            ordinal: 1,
            character: Some(1482),
        };
        captured.active.outcome = Outcome::Rows { len: 2 };
        captured.retired.rows[0] = native::ActiveRow {
            country: 9,
            ordinal: 0,
            character: Some(1451),
        };
        captured.retired.outcome = Outcome::Rows { len: 1 };
        let [
            _,
            TableView::Complete(TableRows::CountryCharacters(rows)),
            _,
            TableView::Complete(TableRows::CountryRetiredCharacters(retired)),
            _,
        ] = captured.tables()
        else {
            panic!("complete country characters");
        };
        assert_eq!(
            retired,
            [CountryCharacterRow {
                country: 9,
                ordinal: 0,
                character: Some(1451)
            }]
        );
        assert_eq!(
            rows,
            [
                CountryCharacterRow {
                    country: 7,
                    ordinal: 0,
                    character: None
                },
                CountryCharacterRow {
                    country: 7,
                    ordinal: 1,
                    character: Some(1482)
                },
            ]
        );
        // A failed job hides both tables and keeps the hourly one unskipped.
        let off_slot = (0..24)
            .find(|&hour| !should_walk(TABLES[0], hour, false))
            .unwrap();
        captured.fail_capture(off_slot, false, "job failed");
        assert_eq!(
            captured.outcomes(),
            [
                Outcome::Unavailable {
                    reason: SKIPPED_REASON
                },
                Outcome::Unavailable {
                    reason: "job failed"
                },
                Outcome::Unavailable {
                    reason: "job failed"
                },
                Outcome::Unavailable {
                    reason: "job failed"
                },
                Outcome::Unavailable {
                    reason: "job failed"
                }
            ]
        );
    }

    #[test]
    fn advisor_tokens_publish_sorted_and_refuse_bad_spans_or_repeats() {
        let mut captured = Captured::with_capacities(&Capacities {
            tokens: 3,
            token_bytes: 15,
            ..Default::default()
        });
        captured.token_bytes.first[..15].copy_from_slice(b"b_ideaa_idea\0zz");
        captured.token_bytes.len = 15;
        for (index, (start, len)) in [(0, 6), (6, 6), (12, 3)].into_iter().enumerate() {
            captured.tokens.rows[index] = advisor_tokens::Row { start, len };
        }
        captured.tokens.outcome = Outcome::Rows { len: 3 };
        let [
            _,
            _,
            TableView::Complete(TableRows::AdvisorTokens(rows)),
            _,
            _,
        ] = captured.tables()
        else {
            panic!("complete advisor tokens");
        };
        assert_eq!(
            rows.iter()
                .map(|row| (row.ordinal, row.token.as_slice()))
                .collect::<Vec<_>>(),
            [(0, b"\0zz".as_slice()), (1, b"a_idea"), (2, b"b_idea")]
        );
        captured.tokens.rows[2] = advisor_tokens::Row { start: 0, len: 6 };
        assert!(matches!(
            captured.tables()[2],
            TableView::Unavailable { .. }
        ));
        captured.tokens.rows[2] = advisor_tokens::Row { start: 12, len: 4 };
        assert!(matches!(
            captured.tables()[2],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn advisor_token_capacity_follows_the_live_set() {
        let mut capacity = Capacities::default();
        capacity.raise_for_advisor_tokens(3_000, 90_000);
        assert!(capacity.tokens >= 4_000 && capacity.token_bytes >= 120_000);
        let raised = capacity;
        capacity.raise_for_advisor_tokens(1, 1);
        assert!(capacity == raised);
    }

    #[test]
    fn record_capacities_follow_the_live_record_totals() {
        let mut capacity = Capacities::default();
        capacity.raise_for_character_records(90_000, 3_000);
        assert!(capacity.active >= 90_000 + 90_000 / 3);
        assert!(capacity.retired >= 3_000 + 3_000 / 3);
        let raised = capacity;
        capacity.raise_for_character_records(10, 1);
        assert!(capacity == raised);
    }

    #[test]
    fn available_advisors_are_lean_hourly_rows_sized_for_5000_countries() {
        let mut capacity = Capacities::default();
        capacity.raise_for_available_advisors(5_000, 20_000);
        assert!(capacity.available >= 5_000);
        assert!(capacity.available_bytes >= 20_000);
        let mut captured = Captured::with_capacities(&capacity);
        for index in 0..5_000 {
            captured.available.rows[index] = native::Row {
                country: index as u32 + 1,
                character: index as u32 + 10_000,
                idea_start: index * 4,
                idea_len: 4,
                // This inaccessible slot must not affect the lean projection.
                slot_start: usize::MAX,
                slot_len: 1,
                ..Default::default()
            };
            captured.available_bytes.first[index * 4..index * 4 + 4].copy_from_slice(b"idea");
        }
        captured.available.outcome = Outcome::Rows { len: 5_000 };
        captured.available_bytes.len = 20_000;
        let TableView::Complete(TableRows::AvailableAdvisors(rows)) = captured.available_table()
        else {
            panic!("complete available advisors");
        };
        assert_eq!(rows.len(), 5_000);
        assert_eq!(rows[4_999].character, 14_999);
        assert_eq!(rows[4_999].idea, b"idea");
        captured.available.rows[4_999].idea_len = usize::MAX;
        assert!(matches!(
            captured.available_table(),
            TableView::Unavailable { .. }
        ));
        captured.available.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            captured.available_table(),
            TableView::Complete(TableRows::AvailableAdvisors(rows)) if rows.is_empty()
        ));
    }
}
