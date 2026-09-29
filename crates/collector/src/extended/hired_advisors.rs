//! The per-country character object (CCountryCharacters at CCountry
//! +0xff0): each country's appointed advisors (+0x58) in native order, with
//! the advisor's character, idea token and slot, daily in its own slot (the
//! advisor roles are read every hour, so also at that slot); each country's
//! active-character records (+0x10, CountryCharacters), hourly; and the
//! character manager's advisor idea-token set (AdvisorTokens), hourly.
use crate::{
    sampling::qpc,
    spooling::{table, table_iter, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{Memory, advisor_tokens, ccountry_characters as native, walk::Outcome};
use replay::{
    cadence::{SKIPPED_REASON, should_walk},
    frame::TableView,
    rows::{AdvisorTokenRow, CountryCharacterRow, HiredAdvisorRow, TableId, tables},
};

pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::HiredAdvisors,
    TableId::CountryCharacters,
    TableId::AdvisorTokens,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

/// Appointed advisors per country slot the first capture is sized for; the
/// engine's own slot counts come from content, so this only presizes.
const PER_COUNTRY: usize = 4;
/// Idea token and slot bytes per advisor the first capture is sized for.
const BYTES_PER_ADVISOR: usize = 48;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    bytes: usize,
    active: usize,
    tokens: usize,
    token_bytes: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: 1024,
            bytes: 1024 * BYTES_PER_ADVISOR,
            active: 1024,
            tokens: 256,
            token_bytes: 256 * BYTES_PER_ADVISOR,
        }
    }
}

impl Capacities {
    /// A few rows per country slot. Never lowers.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        let rows = crate::country_scale::rows(slots, PER_COUNTRY);
        self.rows = self.rows.max(rows);
        self.bytes = self.bytes.max(rows.saturating_mul(BYTES_PER_ADVISOR));
    }

    /// CountryCharacters from the live record total
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_active_characters(&mut self, records: usize) {
        crate::country_scale::raise(&mut self.active, crate::country_scale::rows(records, 1));
    }

    /// AdvisorTokens from the live key count and key bytes
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_advisor_tokens(&mut self, tokens: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.tokens, rows(tokens, 1));
        raise(&mut self.token_bytes, rows(bytes, 1));
    }
}

pub struct Captured {
    rows: CaptureRows<native::Row>,
    bytes: Arena<u8>,
    active: CaptureRows<native::ActiveRow>,
    tokens: CaptureRows<advisor_tokens::Row>,
    token_bytes: Arena<u8>,
}

impl Captured {
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.rows
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.active
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.tokens
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
    }
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            bytes: 0,
            active: 0,
            tokens: 0,
            token_bytes: 0,
        })
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            bytes: Arena::new(capacity.bytes),
            active: CaptureRows::new(capacity.active),
            tokens: CaptureRows::new(capacity.tokens),
            token_bytes: Arena::new(capacity.token_bytes),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
        self.active.reset();
        self.tokens.reset();
        self.token_bytes.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.bytes.allocated_bytes()
            + self.active.allocated_bytes()
            + self.tokens.allocated_bytes()
            + self.token_bytes.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
        // A truncated walk reports at least the bytes of its first unread row.
        self.bytes.grow(&mut capacity.bytes);
        self.active.grow(&mut capacity.active);
        self.tokens.grow(&mut capacity.tokens);
        // The token walk reports every key's bytes, read or not.
        self.token_bytes.grow(&mut capacity.token_bytes);
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
            let walk = native::walk(
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
        if should_walk(TABLES[1], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[1]);
            let started = qpc();
            self.active.outcome =
                native::walk_active(memory, base, game_state, &mut self.active.rows);
            ticks[1] = qpc().saturating_sub(started).max(0) as u64;
        } else {
            self.active.outcome = Outcome::Unavailable {
                reason: SKIPPED_REASON,
            };
        }
        if should_walk(TABLES[2], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[2]);
            let started = qpc();
            let walk = advisor_tokens::walk(
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
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome, self.active.outcome, self.tokens.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [
            self.hired_table(),
            table::<tables::CountryCharacters, _>(self.active.outcome, &self.active.rows, |row| {
                CountryCharacterRow {
                    country: row.country,
                    ordinal: row.ordinal,
                    character: row.character,
                }
            }),
            self.token_table(),
        ]
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
        let mut keys = Vec::with_capacity(rows.len());
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
                token: key.to_vec(),
            },
        )
    }
    fn hired_table(&self) -> TableView {
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return unavailable("hired advisor string arena invalid");
        };
        let text = |start: usize, len: usize| {
            start
                .checked_add(len)
                .and_then(|end| bytes.get(start..end))
        };
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
            idea: text(row.idea_start, row.idea_len).unwrap().to_vec(),
            slot: text(row.slot_start, row.slot_len).unwrap().to_vec(),
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
    fn capacities_follow_the_live_country_slots() {
        let mut capacity = Capacities::default();
        capacity.raise_for_countries(5_000);
        assert!(capacity.rows >= 5_000 * PER_COUNTRY);
        assert!(capacity.bytes >= capacity.rows * BYTES_PER_ADVISOR);
        let raised = capacity;
        capacity.raise_for_countries(10);
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
        let [TableView::Complete(TableRows::HiredAdvisors(rows)), _, _] = captured.tables() else {
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
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            // Hourly: never skipped, and here unreadable rather than stale.
            assert!(matches!(
                captured.outcomes()[1],
                Outcome::Unavailable { reason } if reason != SKIPPED_REASON
            ));
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
        let [_, TableView::Complete(TableRows::CountryCharacters(rows)), _] = captured.tables() else {
            panic!("complete country characters");
        };
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
                }
            ]
        );
    }

    #[test]
    fn advisor_tokens_publish_sorted_and_refuse_bad_spans_or_repeats() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.token_bytes.first[..15].copy_from_slice(b"b_ideaa_idea\0zz");
        captured.token_bytes.len = 15;
        for (index, (start, len)) in [(0, 6), (6, 6), (12, 3)].into_iter().enumerate() {
            captured.tokens.rows[index] = advisor_tokens::Row { start, len };
        }
        captured.tokens.outcome = Outcome::Rows { len: 3 };
        let [_, _, TableView::Complete(TableRows::AdvisorTokens(rows))] = captured.tables() else {
            panic!("complete advisor tokens");
        };
        assert_eq!(
            rows.iter()
                .map(|row| (row.ordinal, row.token.as_slice()))
                .collect::<Vec<_>>(),
            [(0, b"\0zz".as_slice()), (1, b"a_idea"), (2, b"b_idea")]
        );
        captured.tokens.rows[2] = advisor_tokens::Row { start: 0, len: 6 };
        assert!(matches!(captured.tables()[2], TableView::Unavailable { .. }));
        captured.tokens.rows[2] = advisor_tokens::Row { start: 12, len: 4 };
        assert!(matches!(captured.tables()[2], TableView::Unavailable { .. }));
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
    fn active_capacity_follows_the_live_record_total() {
        let mut capacity = Capacities::default();
        capacity.raise_for_active_characters(90_000);
        assert!(capacity.active >= 90_000 + 90_000 / 3);
        let raised = capacity;
        capacity.raise_for_active_characters(10);
        assert!(capacity == raised);
    }
}
