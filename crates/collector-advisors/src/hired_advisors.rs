//! The per-country character object (CCountryCharacters at CCountry pointer): each country's
//! appointed advisors in native order, with the advisor's character, idea token and slot, hourly,
//! since `has_idea` triggers read them (a quiet hour publishes the latest complete walk,
//! [`appointed`]); each country's active-character records (CountryCharacters) and retired ones
//! (CountryRetiredCharacters), hourly, in one walk (a quiet hour publishes the latest complete
//! walk, [`records`]); the character manager's advisor idea-token set (AdvisorTokens), hourly; and
//! each country's available advisors (AvailableAdvisors), the sliced `available_advisors` family:
//! read once a game day per country, at its owner slice, and for a new country or one whose last
//! read was not published at the next hour (owner decision 2026-10-11: only panels read the pools).
use crate::{
    clock::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, table_iter, unavailable},
};
use collector_projection::projection::{Projection, project_table, scalar_table};
use engine_allocation::{copy_vec, owned_vec, reserve_exact};
use engine_characters::advisor_tokens;
use engine_native::{Memory, ccountry_characters as native, walk::Outcome};
use replay_data::{
    cadence::SKIPPED_REASON, cadence::should_walk, rows::AdvisorTokenRow,
    rows::CountryCharacterRow, rows::HiredAdvisorRow, rows::TableId, rows::tables,
};
use replay_frame::frame::{TableInput, TableView};

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
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (rows, bytes) = native::demand_appointed(memory, base, state)?;
        self.raise_for_appointed_advisors(rows, bytes);
        Ok(())
    }
    pub fn raise_for_appointed_advisors(&mut self, count: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows, rows(count, 1));
        raise(&mut self.bytes, rows(bytes, 1));
    }
    /// CountryCharacters and CountryRetiredCharacters from the live record
    /// totals (`country_scale::observe_politics`), with the three-quarters
    /// growth rule's headroom. Never lowers.
    pub fn raise_for_character_records(&mut self, active: usize, retired: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.active, rows(active, 1));
        raise(&mut self.retired, rows(retired, 1));
    }

    /// AdvisorTokens from the live key count and key bytes
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub fn raise_for_advisor_tokens(&mut self, tokens: usize, bytes: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.tokens, rows(tokens, 1));
        raise(&mut self.token_bytes, rows(bytes, 1));
    }

    pub fn raise_for_available_advisors(&mut self, count: usize, bytes: usize) {
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
    /// Which countries this hour's AvailableAdvisors rows cover.
    available_selection: SliceSelection,
}

impl Captured {
    /// The countries whose AvailableAdvisors rows this frame's read replaced.
    pub fn available_selection(&self) -> &SliceSelection {
        &self.available_selection
    }

    /// Start a frame's selection at its hour; the job resets it again.
    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.available_selection.reset(hour, force_full);
    }

    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.available_selection.reset(hour, force_full);
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
    pub fn vacant() -> Self {
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
            // Owner storage grows on the worker from the countries it scans.
            available_selection: SliceSelection::new(false),
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_carried(memory, base, game_state, hour, force_full, None, None)
    }

    /// `store` carries the AvailableAdvisors rows of untouched pools and an
    /// untouched AdvisorTokens table ([`AdvisorStore`]); without it both are
    /// read. `fingerprints` is the `available_advisors` family's store: with
    /// it the hour reads its country slice and the countries it selects
    /// early; without it every country is read.
    #[allow(clippy::too_many_arguments)]
    pub fn capture_carried(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        store: Option<&mut AdvisorStore>,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        let check =
            crate::sampled_verify::Check::burned_in(hour, crate::advisor_pool_changes::VERIFY);
        self.capture_with(
            memory,
            base,
            game_state,
            hour,
            force_full,
            store.map(|store| (store, &crate::advisor_pool_changes::Observer as _, check)),
            fingerprints,
        )
    }

    /// One capture, carrying through `reuse` when present.
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn capture_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        mut reuse: Option<(
            &mut AdvisorStore,
            &dyn crate::advisor_pool_changes::Changes,
            crate::sampled_verify::Check,
        )>,
        fingerprints: Option<&mut SliceFingerprints>,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        if should_walk(TABLES[0], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let started = qpc();
            let walk = match reuse.as_mut() {
                Some((store, _, _)) if appointed::CARRY => appointed::capture(
                    memory,
                    base,
                    game_state,
                    store.appointed(),
                    &appointed::Observer,
                    crate::sampled_verify::Check::burned_in(hour, appointed::VERIFY),
                    &mut self.rows.rows,
                    &mut self.bytes.first,
                ),
                _ => native::walk_growing(
                    memory,
                    base,
                    game_state,
                    &mut self.rows.rows,
                    &mut self.bytes.first,
                ),
            };
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
            let [active_outcome, retired_outcome] = match reuse.as_mut() {
                Some((store, _, _)) if records::CARRY && active && retired => records::capture(
                    memory,
                    base,
                    game_state,
                    store.records(),
                    &appointed::Observer,
                    crate::sampled_verify::Check::burned_in(hour, records::VERIFY),
                    &mut self.active.rows,
                    &mut self.retired.rows,
                ),
                _ => native::walk_records_buffer(
                    memory,
                    base,
                    game_state,
                    if active {
                        engine_native::walk::Buffer::Growing(&mut self.active.rows)
                    } else {
                        engine_native::walk::Buffer::Fixed(&mut [])
                    },
                    if retired {
                        engine_native::walk::Buffer::Growing(&mut self.retired.rows)
                    } else {
                        engine_native::walk::Buffer::Fixed(&mut [])
                    },
                ),
            };
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
            let walk = match reuse.as_mut() {
                Some((store, changes, check)) => {
                    let mut carry = TokenReuse::begin(store, *changes, *check);
                    let walk = advisor_tokens::walk_carried(
                        memory,
                        base,
                        game_state,
                        &mut self.tokens.rows,
                        &mut self.token_bytes.first,
                        &mut carry,
                    );
                    carry.finish(matches!(walk.outcome, Outcome::Rows { .. }));
                    walk
                }
                None => advisor_tokens::walk_growing(
                    memory,
                    base,
                    game_state,
                    &mut self.tokens.rows,
                    &mut self.token_bytes.first,
                ),
            };
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
            crate::slicing::scan_countries(
                memory,
                base,
                game_state,
                hour,
                force_full,
                &mut self.available_selection,
                fingerprints,
            );
            let selection = &self.available_selection;
            let select = |country: u32| selection.contains(country);
            let walk = match reuse {
                Some((store, changes, check)) => {
                    let mut carry = PoolReuse::begin(store, changes, check, hour);
                    let walk = native::walk_available_carried(
                        memory,
                        base,
                        game_state,
                        &mut self.available.rows,
                        &mut self.available_bytes.first,
                        &mut carry,
                        &select,
                    );
                    carry.finish(
                        matches!(walk.outcome, Outcome::Rows { .. }),
                        selection.scanned as usize,
                    );
                    walk
                }
                None => native::walk_available_carried(
                    memory,
                    base,
                    game_state,
                    &mut self.available.rows,
                    &mut self.available_bytes.first,
                    &mut native::Fresh,
                    &select,
                ),
            };
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
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }
    pub fn table_inputs(&self, mut projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let record = |row: &native::ActiveRow| CountryCharacterRow {
            country: row.country,
            ordinal: row.ordinal,
            character: row.character,
        };
        let key = |row: &native::ActiveRow| (u128::from(row.country) << 32) | u128::from(row.ordinal);
        [
            self.hired_input(projection.as_deref_mut()),
            scalar_table::<tables::CountryCharacters, _>(
                projection.as_deref_mut(),
                self.active.outcome,
                &self.active.rows,
                key,
                record,
            ),
            self.token_input(projection.as_deref_mut()),
            scalar_table::<tables::CountryRetiredCharacters, _>(
                projection,
                self.retired.outcome,
                &self.retired.rows,
                key,
                record,
            ),
            self.available_table().into(),
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
            replay_data::rows::AvailableAdvisorRow {
                country: row.country,
                ordinal: row.ordinal,
                character: row.character,
                idea: owned_vec(copy_vec(text(row).unwrap())),
            }
        })
    }
    /// The keys sorted by their bytes, numbered in that order: the set's
    /// bucket order follows hashes and insertion history, not a meaning.
    fn token_input(&self, projection: Option<&mut Projection<'_>>) -> TableInput {
        let Some(bytes) = self.token_bytes.first.get(..self.token_bytes.len) else {
            return unavailable("advisor token arena invalid").into();
        };
        let Some(rows) = self.tokens.rows.get(..self.tokens.outcome.len()) else {
            return unavailable("advisor token row count invalid").into();
        };
        let mut keys = Vec::new();
        if reserve_exact(&mut keys, rows.len()).is_err() {
            return unavailable("collector allocation failed").into();
        }
        for row in rows {
            match row
                .start
                .checked_add(row.len)
                .and_then(|end| bytes.get(row.start..end))
            {
                Some(key) => keys.push(key),
                None => return unavailable("advisor token outside captured arena").into(),
            }
        }
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return unavailable("advisor token set holds a key twice").into();
        }
        // A complete, equal fresh set can retain the encoder's allocation,
        // including on keyframes. Never carry a partial or refused set.
        if matches!(self.tokens.outcome, Outcome::Rows { .. })
            && let Some(previous) = projection
                .as_deref()
                .and_then(|projection| projection.observed_shared_baseline::<tables::AdvisorTokens>())
            && let Some(previous_rows) = previous.rows::<tables::AdvisorTokens>()
            && previous_rows.len() == keys.len()
            && keys
                .iter()
                .zip(previous_rows)
                .enumerate()
                .all(|(ordinal, (key, row))| row.ordinal as usize == ordinal && *key == row.token)
        {
            return TableInput::SharedComplete(std::sync::Arc::clone(previous));
        }
        let view = table_iter::<tables::AdvisorTokens, _>(
            self.tokens.outcome,
            keys.iter().enumerate(),
            |(ordinal, key)| AdvisorTokenRow {
                ordinal: ordinal as u32,
                token: owned_vec(copy_vec(key)),
            },
        );
        match view {
            TableView::Complete(rows) if projection.is_some() => {
                TableInput::SharedComplete(std::sync::Arc::new(rows))
            }
            view => view.into(),
        }
    }
    fn hired_input(&self, projection: Option<&mut Projection<'_>>) -> TableInput {
        let Some(bytes) = self.bytes.first.get(..self.bytes.len) else {
            return unavailable("hired advisor string arena invalid").into();
        };
        let text =
            |start: usize, len: usize| start.checked_add(len).and_then(|end| bytes.get(start..end));
        let Some(rows) = self.rows.rows.get(..self.rows.outcome.len()) else {
            return unavailable("hired advisor row count invalid").into();
        };
        if rows.iter().any(|row| {
            text(row.idea_start, row.idea_len).is_none()
                || text(row.slot_start, row.slot_len).is_none()
        }) {
            return unavailable("hired advisor strings outside captured arena").into();
        }
        project_table::<tables::HiredAdvisors, _>(
            projection,
            self.rows.outcome,
            rows,
            |row| (u128::from(row.country) << 32) | u128::from(row.ordinal),
            |row, previous| {
                row.character == previous.character
                    && text(row.idea_start, row.idea_len).unwrap() == previous.idea
                    && text(row.slot_start, row.slot_len).unwrap() == previous.slot
            },
            |row| HiredAdvisorRow {
                country: row.country,
                ordinal: row.ordinal,
                character: row.character,
                idea: owned_vec(copy_vec(text(row.idea_start, row.idea_len).unwrap())),
                slot: owned_vec(copy_vec(text(row.slot_start, row.slot_len).unwrap())),
            },
        )
    }
}

mod advisor_store;
mod appointed;
mod records;
pub use advisor_store::AdvisorStore;
use advisor_store::{PoolReuse, TokenReuse};
#[cfg(test)]
#[path = "hired_advisors/advisor_store_tests.rs"]
mod advisor_store_tests;

#[cfg(test)]
#[path = "hired_advisors/projection_tests.rs"]
mod projection_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    /// HiredAdvisors, which `has_idea` triggers read, is read every hour
    /// (here unreadable rather than skipped), as are the advisor roles it
    /// names.
    #[test]
    fn appointed_advisors_walk_every_hour() {
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            captured.capture(&memory, 0, 0x1000, hour, false);
            for index in [0, 4] {
                assert!(
                    matches!(
                        captured.outcomes()[index],
                        Outcome::Unavailable { reason } if reason != SKIPPED_REASON
                    ),
                    "{hour}"
                );
            }
            assert!(should_walk(TABLES[0], hour, false));
            assert!(should_walk(TableId::AdvisorRoles, hour, false));
        }
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
        let memory = engine_native::memory::SliceMemory {
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
        // A failed job hides every table, none skipped: all are hourly or
        // sliced.
        captured.fail_capture(5, false, "job failed");
        assert_eq!(
            captured.outcomes(),
            [
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
    fn available_advisors_are_lean_rows_sized_for_5000_countries() {
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
