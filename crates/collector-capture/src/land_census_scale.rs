use crate::unit_scale_tests::fixture as unit_world;
use collector_division_parts::division_land::Captured;
use collector_divisions::division_census::Census;
use engine_native::{carmy, carmy_manpower_values, walk::Outcome};

fn run(countries: usize) {
    let count = countries * 10;
    let mut world = unit_world::build(unit_world::Shape {
        countries,
        divisions: count,
        wings: 1,
        planes: 1,
        ships: 0,
    });
    for (n, &pointer) in world.armies.iter().enumerate() {
        let at = pointer - world.base;
        let bytes = &mut world.bytes[at..at + 0x800];
        bytes[carmy::MANPOWER_VALUES..carmy::MANPOWER_VALUES + 8].copy_from_slice(
            &((world.base + carmy_manpower_values::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        bytes[carmy::MANPOWER_VALUES + 8..carmy::MANPOWER_VALUES + 16]
            .copy_from_slice(&((pointer + 0x780) as u64).to_le_bytes());
        bytes[carmy::MANPOWER_VALUES + 16..carmy::MANPOWER_VALUES + 20]
            .copy_from_slice(&2u32.to_le_bytes());
        bytes[carmy::MANPOWER_VALUES + 20..carmy::MANPOWER_VALUES + 24]
            .copy_from_slice(&2u32.to_le_bytes());
        bytes[carmy::MAXIMUM_SUPPLY_GRACE..carmy::MAXIMUM_SUPPLY_GRACE + 8]
            .copy_from_slice(&300_000i64.to_le_bytes());
        bytes[carmy::REMAINING_SUPPLY_GRACE..carmy::REMAINING_SUPPLY_GRACE + 8]
            .copy_from_slice(&200_000i64.to_le_bytes());
        bytes[0x780..0x784].copy_from_slice(&((n / 10 + 1) as u32).to_le_bytes());
        bytes[0x784..0x788].copy_from_slice(&10_000i32.to_le_bytes());
        bytes[0x788..0x78c].copy_from_slice(&0x8000_0001u32.to_le_bytes());
        bytes[0x78c..0x790].copy_from_slice(&(-7i32).to_le_bytes());
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let mut census = Census::new();
    for _ in 0..3 {
        census.capture_inputs(&memory, world.base, world.state);
        if !census.overflowed {
            break;
        }
        census.grow();
    }
    assert!(!census.overflowed);
    assert_eq!(
        census.unit_inputs().unwrap().outcome(),
        Outcome::Rows { len: count }
    );
    let mut land = Captured::new(count);
    census.capture_inputs(&memory, world.base, world.state);
    let inputs = census.unit_inputs().unwrap();
    land.capture(&memory, world.base, inputs.seeds(), inputs.outcome());
    assert_eq!(inputs.outcome(), Outcome::Rows { len: count });
    assert_eq!(inputs.seeds().len(), count);
    assert_eq!(land.outcome(), inputs.outcome());
    verify(&land, count);
}
fn verify(land: &Captured, count: usize) {
    let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::DivisionLandInputs(
        rows,
    )) = land.table()
    else {
        panic!("complete shared census must produce complete land input rows")
    };
    assert_eq!(rows.len(), count);
    assert_eq!(
        rows[count - 1].remaining_grace,
        replay_data::rows::RetiredI64::default()
    );
    assert_eq!(rows[count - 1].manpower.len(), 2);
    assert_eq!(rows[count - 1].manpower[1].amount, -7);
}
#[test]
fn shared_census_land_inputs_integrate_without_prebuilt_seeds() {
    for countries in [4, 40] {
        run(countries);
    }
}
