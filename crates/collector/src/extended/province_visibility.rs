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

pub(crate) fn source_outcome(outcome: Outcome, checked: bool, mismatches: usize) -> Outcome {
    if checked && mismatches != 0 {
        Outcome::Unavailable {
            reason: "province identity source changed during double read",
        }
    } else {
        outcome
    }
}

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
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            rows: CaptureRows::new(capacity.rows, successful[0]),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.rows.grow(&mut capacity.rows, successful[0]);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        provinces: &[cprovince::Row],
        source: Outcome,
        needed: bool,
    ) -> (bool, u64) {
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
        self.rows.checked = needed;
        // Only immutable province IDs are shared. Client roots, POV, headers
        // and raw visibility bytes are freshly acquired in each pass.
        let matched = if needed {
            let second = province_visibility_layout::walk_for_provinces(
                memory,
                base,
                state,
                provinces,
                source,
                &mut self.rows.second,
            );
            self.rows.compare(second)
        } else {
            false
        };
        (matched, qpc().saturating_sub(started).max(0) as u64)
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.rows.checked && self.rows.mismatches != 0 {
            Outcome::Unavailable {
                reason: "client province visibility changed during double read",
            }
        } else {
            self.rows.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.rows.mismatches]
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
    use crate::sampling::CONSISTENCY_TARGET;
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
        mutation: u8,
    }

    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == cgame_state::SINGLETON_RVA {
                self.contexts.set(self.contexts.get() + 1);
            }
            if (CURRENT..CURRENT + 4).contains(&address) {
                self.payload_reads.set(self.payload_reads.get() + 1);
            }
            // Each walk acquires its context initially and terminally. Change
            // only at the next walk, retaining a self-consistent second POV.
            let second = self.contexts.get() >= 3;
            for (index, destination) in destination.iter_mut().enumerate() {
                let at = address + index;
                let Some(&value) = self.bytes.get(&at) else {
                    return false;
                };
                *destination = if second && self.mutation == 1 && at == CURRENT + 1 {
                    value ^ 0x80
                } else if second
                    && self.mutation == 2
                    && [
                        STATE + cgame_state::OBSERVER_COUNTRY,
                        HOLDER + province_visibility_layout::VIEWING_COUNTRY,
                    ]
                    .contains(&at)
                {
                    2
                } else {
                    value
                };
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

        fn new(mutation: u8) -> Self {
            let mut fixture = Self {
                bytes: BTreeMap::new(),
                contexts: Cell::new(0),
                payload_reads: Cell::new(0),
                mutation,
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

        fn capture(&self, captured: &mut Captured, needed: bool) -> bool {
            captured
                .capture(
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
                    needed,
                )
                .0
        }
    }

    #[test]
    fn actual_two_pass_visibility_and_viewpoint_changes_are_rejected() {
        for mutation in 0..3 {
            let fixture = Fixture::new(mutation);
            let mut captured = Captured::with_capacities(&Capacities { rows: 3 }, &[0]);
            assert_eq!(fixture.capture(&mut captured, true), mutation == 0);
            assert_eq!(fixture.contexts.get(), 4);
            assert_eq!(fixture.payload_reads.get(), 2);
            assert_eq!(captured.rows.rows[0].visibility, 0);
            assert_eq!(captured.rows.rows[1].visibility, 255);
            assert_eq!(captured.rows.rows[1].province, 3);
            assert_eq!(
                captured.checked_mismatches(),
                [match mutation {
                    0 => 0,
                    1 => 1,
                    _ => 2,
                }]
            );
            assert_eq!(
                matches!(captured.tables()[0], TableView::Unavailable { .. }),
                mutation != 0
            );
        }
    }

    #[test]
    fn equal_truncation_never_earns_credit_and_growth_restores_complete_rows() {
        let mut capacity = Capacities { rows: 1 };
        let mut captured = Captured::with_capacities(&capacity, &[0]);
        let fixture = Fixture::new(0);
        assert!(!fixture.capture(&mut captured, true));
        assert_eq!(
            captured.outcomes(),
            [Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }]
        );
        assert_eq!(captured.checked_mismatches(), [0]);
        captured.grow(&mut capacity, &[0]);
        assert_eq!(capacity.rows, 2);
        assert_eq!(captured.rows.second.len(), 2);
        assert!(Fixture::new(0).capture(&mut captured, true));
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 2 }]);
    }

    #[test]
    fn successful_window_retires_comparison_and_branch_reset_reenables_it() {
        let mut capacity = Capacities { rows: 4 };
        let mut captured = Captured::with_capacities(&capacity, &[0]);
        assert!(Fixture::new(0).capture(&mut captured, true));
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(captured.rows.second.is_empty());
        let fixture = Fixture::new(1);
        assert!(!fixture.capture(&mut captured, false));
        assert_eq!(fixture.contexts.get(), 2);
        assert_eq!(fixture.payload_reads.get(), 1);
        assert!(!captured.rows.checked);
        assert_eq!(captured.outcomes(), [Outcome::Rows { len: 2 }]);
        captured.reset();
        captured.grow(&mut capacity, &[0]);
        assert_eq!(captured.rows.second.len(), capacity.rows);
        assert!(!Fixture::new(1).capture(&mut captured, true));
        assert_eq!(captured.checked_mismatches(), [1]);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn checked_core_failure_cannot_certify_visibility_coverage() {
        let complete = Outcome::Rows { len: 2 };
        assert_eq!(source_outcome(complete, true, 0), complete);
        assert_eq!(source_outcome(complete, false, 1), complete);
        assert!(matches!(
            source_outcome(complete, true, 1),
            Outcome::Unavailable { .. }
        ));
        let truncated = Outcome::Truncated {
            len: 1,
            unread_rows: 1,
        };
        assert_eq!(source_outcome(truncated, true, 0), truncated);
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 }, &[0]);
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let (matched, _) =
            captured.capture(&memory, 0, 0, &[], source_outcome(complete, true, 1), true);
        assert!(!matched);
        assert!(matches!(
            captured.tables()[0],
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn missing_sources_never_become_zero_visibility_or_successful_checks() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 }, &[0]);
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let (matched, _) = captured.capture(&memory, 0, 0, &[], super::super::MISSING, true);
        assert!(!matched);
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
    fn zero_is_recorded_but_changed_pov_invalidates_checked_rows() {
        let mut captured = Captured::with_capacities(&Capacities { rows: 2 }, &[0]);
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
        captured.rows.second[0] = province_visibility_layout::Row {
            viewing_country: 8,
            ..captured.rows.rows[0]
        };
        captured.rows.checked = true;
        assert!(!captured.rows.compare(Outcome::Rows { len: 1 }));
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
        captured.reset();
        assert_eq!(captured.checked_mismatches(), [0]);
        let mut capacity = Capacities { rows: 2 };
        captured.grow(&mut capacity, &[CONSISTENCY_TARGET]);
        assert!(captured.rows.second.is_empty());
    }
}
