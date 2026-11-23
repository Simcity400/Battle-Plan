use collector_naval_history::extended::naval_history::{
    Capacities,
    reserved_land_tests::{Observed, fixture, typed},
};
use engine_native::ctheatre;
use engine_native_memory::walk::Outcome;
use std::cell::RefCell;

#[test]
fn committed_suffix_growth_counts_rows_once_and_restored_naming_failure_removes_only_its_tail() {
    for (count, initial, expected) in [(0, 1, 2), (1, 2, 4), (3, 6, 6)] {
        let bytes = fixture(count);
        let memory = Observed {
            bytes: &bytes,
            fault: None,
            calls: RefCell::new(Vec::new()),
        };
        let mut captured = typed(&memory, initial);
        assert_eq!(captured.reserved_land_count(), Some(count));
        let positive = captured.land.rows[0];
        let mut capacity = Capacities::fixture();
        capacity.rows[4] = initial;
        captured.grow(&mut capacity);
        assert_eq!(capacity.rows[4], expected);
        let grown = capacity.rows[4];
        captured.grow(&mut capacity);
        assert_eq!(capacity.rows[4], grown);
        assert_eq!(
            captured.land.outcome,
            Outcome::Rows {
                len: 1 + count as usize
            }
        );
        let mut extended =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        extended.naval_history = captured;
        extended.country_appearance =
            collector_country_counters::country_appearance::Captured::vacant();
        extended.install_reserved_land_certificate();
        assert_eq!(extended.naval_history.reserved_land_count(), None);
        assert_eq!(
            extended.naval_history.land.outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(extended.naval_history.land.rows[0], positive);
        assert_eq!(
            extended.naval_history.reserved_land_required_rows,
            count as usize
        );
        assert!(
            extended.naval_history.land.rows[1..1 + count as usize]
                .iter()
                .all(|row| *row == ctheatre::Row::default())
        );
        extended.install_reserved_land_certificate();
        assert_eq!(
            extended.naval_history.land.outcome,
            Outcome::Rows { len: 1 }
        );
    }
}
