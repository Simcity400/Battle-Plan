//! Hourly stored client visibility over the shared complete province census.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{Memory, cprovince, province_visibility_layout, walk::Outcome};
use replay::{
    frame::TableView,
    rows::{ProvinceVisibilityRow, TableId, tables},
};

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::ProvinceVisibility];
// Shared province identity acquisition remains charged to Provinces.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self { rows: 20_000 }
    }
}
pub struct Captured {
    rows: CaptureRows<province_visibility_layout::Row>,
}
impl Captured {
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.rows.grow(&mut capacity.rows);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        provinces: &[cprovince::Row],
        source: Outcome,
    ) -> u64 {
        let _reads = crate::read_accounting::table(TABLES[0]);
        self.reset();
        let started = qpc();
        self.rows.outcome = province_visibility_layout::walk_for_provinces(
            memory,
            base,
            state,
            provinces,
            source,
            &mut self.rows.rows,
        );
        qpc().saturating_sub(started).max(0) as u64
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::ProvinceVisibility, _>(
            self.outcomes()[0],
            &self.rows.rows,
            |r| ProvinceVisibilityRow {
                province: r.province,
                visibility: r.visibility,
                viewing_country: r.viewing_country,
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    use engine::{ccountry, cgame_state, cgraphical_map, cin_game_idler};
    use std::{cell::Cell, collections::BTreeMap};

    const STATE: usize = 0x1000;
    const IDLER: usize = 0x2000;
    const DELEGATE: usize = 0x3000;
    const MAP: usize = 0x4000;
    const HOLDER: usize = 0x5000;
    const COUNTRIES: usize = 0x6000;
    const LINKS: usize = 0x6100;
    const COUNTRY: usize = 0x6200;
    const CURRENT: usize = 0x10000;

    struct Fixture {
        bytes: BTreeMap<usize, u8>,
        contexts: Cell<usize>,
        payload_reads: Cell<usize>,
    }

    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == cgame_state::SINGLETON_RVA {
                self.contexts.set(self.contexts.get() + 1);
            }
            if (CURRENT..CURRENT + 4).contains(&address) {
                self.payload_reads.set(self.payload_reads.get() + 1);
            }
            for (index, destination) in destination.iter_mut().enumerate() {
                let Some(&value) = self.bytes.get(&(address + index)) else {
                    return false;
                };
                *destination = value;
            }
            true
        }
    }

    impl Fixture {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.bytes
                .extend(bytes.iter().enumerate().map(|(i, &byte)| (at + i, byte)));
        }

        fn header(&mut self, at: usize, data: usize, count: u32) {
            self.put(at, &(data as u64).to_le_bytes());
            self.put(at + 8, &count.to_le_bytes());
            self.put(at + 12, &count.to_le_bytes());
        }

        fn new() -> Self {
            let mut fixture = Self {
                bytes: BTreeMap::new(),
                contexts: Cell::new(0),
                payload_reads: Cell::new(0),
            };
            for (at, value) in [
                (cgame_state::SINGLETON_RVA, STATE),
                (cin_game_idler::SINGLETON_A_RVA, IDLER),
                (IDLER, cin_game_idler::VTABLE_RVA),
                (IDLER + cin_game_idler::MAP_DELEGATE, DELEGATE),
                (DELEGATE + 0x370, MAP),
                (MAP + cgraphical_map::PROVINCE_VISIBILITY, HOLDER),
                (COUNTRIES + 8, COUNTRY),
                (COUNTRIES + 16, COUNTRY + 0x100),
                (COUNTRY, ccountry::VTABLE_RVA),
                (COUNTRY + 0x100, ccountry::VTABLE_RVA),
            ] {
                fixture.put(at, &(value as u64).to_le_bytes());
            }
            fixture.put(HOLDER, &[0; 0x1c]);
            fixture.put(
                HOLDER + province_visibility_layout::CURRENT,
                &(CURRENT as u64).to_le_bytes(),
            );
            fixture.put(
                HOLDER + province_visibility_layout::PREVIOUS,
                &0x11000_u64.to_le_bytes(),
            );
            fixture.put(
                HOLDER + province_visibility_layout::COUNT,
                &4_u32.to_le_bytes(),
            );
            fixture.put(
                HOLDER + province_visibility_layout::VIEWING_COUNTRY,
                &1_u32.to_le_bytes(),
            );
            fixture.put(HOLDER + province_visibility_layout::FOG_TOGGLE, &[1]);
            fixture.put(province_visibility_layout::FOG_TOGGLE_RVA, &[1]);
            fixture.put(STATE + cgame_state::OBSERVER_COUNTRY, &1_u32.to_le_bytes());
            fixture.put(
                STATE + cgame_state::OBSERVER_COUNTRY_FALLBACK,
                &2_u32.to_le_bytes(),
            );
            fixture.header(STATE + cgame_state::PROVINCES, 0x20000, 4);
            fixture.header(STATE + cgame_state::COUNTRIES, COUNTRIES, 3);
            fixture.header(STATE + cgame_state::COUNTRY_LINKS, LINKS, 3);
            fixture.put(COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
            fixture.put(COUNTRY + 0x100 + ccountry::ID, &2_u32.to_le_bytes());
            for slot in 0..3_u32 {
                fixture.put(LINKS + slot as usize * 4, &slot.to_le_bytes());
            }
            fixture.put(CURRENT, &[99, 0, 11, 255]);
            fixture
        }

        fn capture(&self, captured: &mut Captured) -> u64 {
            captured.capture(
                self,
                0,
                STATE,
                &[
                    cprovince::Row {
                        id: 1,
                        ..Default::default()
                    },
                    cprovince::Row {
                        id: 3,
                        ..Default::default()
                    },
                ],
                Outcome::Rows { len: 2 },
            )
        }
    }

    #[test]
    fn primary_visibility_is_acquired_once_between_context_checks() {
        let fixture = Fixture::new();
        let mut captured = Captured::with_capacities(&Capacities { rows: 3 });
        fixture.capture(&mut captured);
        assert_eq!(fixture.contexts.get(), 2);
        assert_eq!(fixture.payload_reads.get(), 1);
        assert_eq!(captured.rows.rows[0].visibility, 0);
        assert_eq!(captured.rows.rows[1].visibility, 255);
        assert_eq!(captured.rows.rows[1].province, 3);
        assert_eq!(captured.rows.rows[1].viewing_country, 1);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 2 }]);
        assert!(matches!(captured.tables()[0], TableView::Complete(_)));
    }

    #[test]
    fn capacity_growth_restores_complete_primary_rows() {
        let mut capacity = Capacities { rows: 1 };
        let mut captured = Captured::with_capacities(&capacity);
        Fixture::new().capture(&mut captured);
        assert_eq!(
            captured.outcomes(),
            [Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }]
        );
        captured.grow(&mut capacity);
        assert_eq!(capacity.rows, 2);
        assert_eq!(captured.rows.rows.len(), 2);
        Fixture::new().capture(&mut captured);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 2 }]);
    }

    #[test]
    fn missing_sources_never_become_zero_visibility() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 });
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        captured.capture(&memory, 0, 0, &[], super::super::MISSING);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn primary_zero_and_raw_viewpoint_are_preserved() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 });
        captured.rows.rows[0] = province_visibility_layout::Row {
            province: 1,
            visibility: 0,
            viewing_country: 7,
        };
        captured.rows.outcome = Outcome::Rows { len: 1 };
        let TableView::Complete(replay::rows::TableRows::ProvinceVisibility(rows)) =
            &captured.tables()[0]
        else {
            panic!("expected recorded row")
        };
        assert_eq!(rows[0].visibility, 0);
        assert_eq!(rows[0].viewing_country, 7);
    }
}
