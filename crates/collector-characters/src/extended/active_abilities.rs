use collector_primitives::clock::qpc;
use engine_land_inputs::active_ability_record;
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{rows::TableId, rows::tables};
use replay_frame::frame::TableView;

use super::characters::{Buffer, ability_row, valid, view, walk_abilities};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ActiveAbilities];

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacities {
    pub rows: usize,
    pub bytes: usize,
}

collector_primitives::capacity_max!(Capacities { rows, bytes });
impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let (rows, bytes) = super::characters::ability_demand(memory, base, state)?;
        self.raise(rows, bytes);
        Ok(())
    }
    pub fn raise(&mut self, rows: usize, bytes: usize) {
        collector_primitives::country_slots::raise(
            &mut self.rows,
            collector_primitives::country_slots::rows(rows, 1),
        );
        collector_primitives::country_slots::raise(
            &mut self.bytes,
            collector_primitives::country_slots::rows(bytes, 1),
        );
    }
}

pub struct Captured {
    buffer: Buffer<active_ability_record::Row>,
}

impl Captured {
    pub fn vacant() -> Self {
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
        let _reads = collector_memory::read_accounting::table(TABLES[0]);
        let begin = qpc();
        let result = walk_abilities(
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
    use super::*;
    use replay_data::rows::TableRows;

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

    fn world(countries: u32, active_every: u32) -> Vec<u8> {
        use engine_land_inputs::cability;
        use engine_native::{ccountry, cgame_state};
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
                &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            &(cability::VTABLE_RVA.0 as u64).to_le_bytes(),
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
    const STRIDE: usize = 0x50;
    const RECORDS: usize = 0x100000;
    const DEFINITION: usize = 0x1c0000;
    const WORLD_BYTES: usize = 0x1d0000;

}
