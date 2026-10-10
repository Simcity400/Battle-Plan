//! Hourly CCountry active-ability records for the land counter's ability grid.
//!
//! Abilities start, end and change phase at any hour, so the table is hourly. It runs in the
//! Ordinary pool job over the authenticated country roster view, never on the game thread;
//! rows and token bytes grow from the original headers before publication.
use crate::sampling::qpc;
use engine::{Memory, active_ability_record, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{TableId, tables},
};

use super::characters::{Buffer, ability_row, valid, view, walk_abilities_growing};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ActiveAbilities];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacities {
    pub rows: usize,
    pub bytes: usize,
}
impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (rows, bytes) = super::characters::ability_demand(memory, base, state)?;
        self.raise(rows, bytes);
        Ok(())
    }
    pub(crate) fn raise(&mut self, rows: usize, bytes: usize) {
        crate::country_scale::raise(&mut self.rows, crate::country_scale::rows(rows, 1));
        crate::country_scale::raise(&mut self.bytes, crate::country_scale::rows(bytes, 1));
    }
}

pub struct Captured {
    buffer: Buffer<active_ability_record::Row>,
}

impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities { rows: 0, bytes: 0 })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            buffer: Buffer::with(capacity.rows, capacity.bytes),
        }
    }

    pub fn reset(&mut self) {
        self.buffer.rows.reset();
        self.buffer.bytes.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.buffer.rows.allocated_bytes() + self.buffer.bytes.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.buffer.rows.grow(&mut capacity.rows);
        self.buffer.bytes.grow(&mut capacity.bytes);
    }

    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let result = walk_abilities_growing(
            memory,
            base,
            game_state,
            &mut self.buffer.rows.rows,
            &mut self.buffer.bytes.first,
        );
        self.buffer.rows.outcome = result.outcome;
        self.buffer.bytes.len = result.bytes_len;
        self.buffer.bytes.required = result.required_bytes;
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.buffer.rows.outcome]
    }

    pub fn tables(&self) -> [TableView; N] {
        [view::<tables::ActiveAbilities, _>(
            &self.buffer,
            |row, bytes| valid(bytes, row.token_start, row.token_len),
            ability_row,
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::super::characters::walk_abilities;
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn first_and_later_hours_grow_from_5000_country_headers() {
        use engine::{ccountry, cgame_state};
        let mut bytes = world(5_000, 1);
        let mut captured = Captured::with_capacities(&Capacities { rows: 0, bytes: 0 });
        captured.capture(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GAME_STATE,
        );
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 5_000 }]);
        assert_eq!(captured.buffer.bytes.len, 35_000);
        let record = bytes[RECORDS..RECORDS + active_ability_record::STRIDE].to_vec();
        let data = bytes.len();
        for _ in 0..4_096 {
            bytes.extend_from_slice(&record);
        }
        let header = COUNTRY0 + ccountry::ACTIVE_ABILITIES;
        bytes[header..header + 8].copy_from_slice(&(data as u64).to_le_bytes());
        for offset in [8, 12] {
            bytes[header + offset..header + offset + 4].copy_from_slice(&4_096_u32.to_le_bytes());
        }
        captured.capture(
            &engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GAME_STATE,
        );
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 9_095 }]);
        assert_eq!(captured.buffer.bytes.len, 9_095 * 7);
        assert_eq!(captured.buffer.rows.rows[4_095].ordinal, 4_095);
        assert_eq!(
            bytes[GAME_STATE + cgame_state::COUNTRIES + 12
                ..GAME_STATE + cgame_state::COUNTRIES + 16],
            5_001_u32.to_le_bytes()
        );
    }

    #[test]
    fn conversion_refuses_bad_ranges_and_duplicate_keys() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2, bytes: 16 });
        captured.buffer.rows.rows[0].token_len = 1;
        captured.buffer.rows.outcome = Outcome::Rows { len: 1 };
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        captured.buffer.bytes.len = 1;
        captured.buffer.bytes.required = 1;
        captured.buffer.bytes.first[0] = 0xff;
        captured.buffer.rows.rows[1] = captured.buffer.rows.rows[0];
        captured.buffer.rows.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
        captured.buffer.rows.outcome = Outcome::Rows { len: 1 };
        let [TableView::Complete(TableRows::ActiveAbilities(rows))] = captured.tables() else {
            panic!("complete ability table");
        };
        assert_eq!(rows[0].token, [0xff]);
        captured.reset();
        assert_eq!(captured.buffer.bytes.len, 0);
    }

    #[test]
    fn a_short_hour_grows_rows_and_token_bytes_off_thread() {
        let mut capacity = Capacities { rows: 1, bytes: 1 };
        let mut captured = Captured::with_capacities(&capacity);
        captured.buffer.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 7,
        };
        captured.buffer.bytes.required = 40;
        captured.grow(&mut capacity);
        assert!(capacity.rows >= 8 && capacity.bytes >= 40);
        assert!(captured.buffer.bytes.first.len() >= 40);
    }

    /// A synthetic world of `countries` live countries; every `active_every`th one holds one
    /// ability record without caster or region.
    fn world(countries: u32, active_every: u32) -> Vec<u8> {
        use engine::{cability, ccountry, cgame_state};
        let put = |bytes: &mut Vec<u8>, at: usize, value: &[u8]| {
            bytes[at..at + value.len()].copy_from_slice(value)
        };
        let header = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
            bytes[at..at + 8].copy_from_slice(&(data as u64).to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        };
        let mut bytes = vec![0; WORLD_BYTES];
        header(
            &mut bytes,
            GAME_STATE + cgame_state::COUNTRIES,
            CTAB,
            countries + 1,
        );
        header(
            &mut bytes,
            GAME_STATE + cgame_state::COUNTRY_LINKS,
            LTAB,
            countries + 1,
        );
        for slot in 1..=countries {
            let country = COUNTRY0 + (slot as usize - 1) * STRIDE;
            put(
                &mut bytes,
                CTAB + slot as usize * 8,
                &(country as u64).to_le_bytes(),
            );
            put(&mut bytes, LTAB + slot as usize * 4, &slot.to_le_bytes());
            put(
                &mut bytes,
                country,
                &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(&mut bytes, country + ccountry::ID, &slot.to_le_bytes());
            if slot % active_every == 0 {
                let record = RECORDS + (slot as usize - 1) * active_ability_record::STRIDE;
                header(&mut bytes, country + ccountry::ACTIVE_ABILITIES, record, 1);
                put(
                    &mut bytes,
                    record + active_ability_record::DEFINITION,
                    &(DEFINITION as u64).to_le_bytes(),
                );
            }
        }
        put(
            &mut bytes,
            DEFINITION,
            &(cability::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, DEFINITION + cability::TOKEN, b"ability");
        put(
            &mut bytes,
            DEFINITION + cability::TOKEN_LENGTH,
            &7_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + cability::TOKEN_CAPACITY,
            &15_u64.to_le_bytes(),
        );
        bytes
    }

    const GAME_STATE: usize = 0x1000;
    const CTAB: usize = 0x10000;
    const LTAB: usize = 0x20000;
    const COUNTRY0: usize = 0x30000;
    // Country objects overlap: only +0, +8 and the ability header are populated.
    const STRIDE: usize = 0x50;
    const RECORDS: usize = 0x100000;
    const DEFINITION: usize = 0x1c0000;
    const WORLD_BYTES: usize = 0x1d0000;

    /// The Ordinary job's cost for the hourly walk on the pool worker, over the job's cached
    /// roster view, at 1,500 and 5,000 live countries with one country in ten holding an
    /// active ability. The roster view serves the country authentication from the prepared
    /// census, so the per-country cost is the ability array header alone.
    #[test]
    fn the_hourly_walk_on_the_cached_roster_view_costs_one_call_per_country() {
        use engine::read_counts::{CountedMemory, ReadCounts};
        for countries in [1_500_u32, 5_000] {
            let bytes = world(countries, 10);
            let memory = engine::memory::SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            // Sized for every census read, as the collector sizes it from the live registry.
            let mut storage = std::sync::Arc::new(
                engine::country_memory::Cache::try_with_capacity((countries as usize + 1) * 16)
                    .unwrap(),
            );
            let cache =
                engine::country_memory::Cache::prepare_shared(&mut storage, &memory, 0, GAME_STATE)
                    .unwrap();
            assert!(
                cache
                    .roster_view(&memory)
                    .authenticated_countries(0, GAME_STATE)
                    .is_some()
            );
            let active = (countries / 10) as usize;
            let mut rows = vec![active_ability_record::Row::default(); active];
            let mut arena = vec![0; active * 7];
            // `Captured::capture` charges these reads to the table's accounting slot; the
            // walk itself is counted here.
            ReadCounts::take_current();
            let counted = CountedMemory::new(memory);
            let started = std::time::Instant::now();
            let walk = walk_abilities(
                &cache.roster_view(&counted),
                0,
                GAME_STATE,
                &mut rows,
                &mut arena,
            );
            let elapsed = started.elapsed();
            let reads = ReadCounts::take_current();
            assert_eq!(walk.outcome, Outcome::Rows { len: active });
            let calls = reads.copy_calls + reads.scattered_calls;
            // One guarded array-header read per country (the prepared census answers the
            // country authentication) and three per record without caster or region.
            assert_eq!(
                calls,
                u64::from(countries) + 3 * active as u64,
                "{countries} countries"
            );
            eprintln!(
                "active abilities, {countries} countries on the roster view: {calls} guarded calls, {elapsed:?} (debug build)"
            );
        }
    }
}
