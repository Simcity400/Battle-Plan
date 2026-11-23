use super::*;
use collector_division_parts::army_stat_changes::Watches;
use engine_native::{
    acclimatization_category as category, acclimatization_data as acclimatization,
    corders_group as group,
};
use replay_data::rows::Recorded;

fn put(world: &mut scale_fixture::World, address: usize, value: &[u8]) {
    let at = address - world.base;
    world.bytes[at..at + value.len()].copy_from_slice(value);
}

fn header(data: usize, count: u32) -> [u8; 16] {
    let mut header = [0; 16];
    header[..8].copy_from_slice(&(data as u64).to_le_bytes());
    header[8..12].copy_from_slice(&count.to_le_bytes());
    header[12..].copy_from_slice(&count.to_le_bytes());
    header
}

#[test]
fn map_model_witness_equals_full_division_status_columns() {
    let mut populated = Populated::new();
    let (base, state) = (populated.world.base, populated.world.state);
    let thresholds = Box::new([100_i64, 200, 300]);
    let categories: Vec<Box<[u8; 0x40]>> = [1_u32, 2]
        .into_iter()
        .map(|token| {
            let mut object = Box::new([0; 0x40]);
            object[category::TOKEN..category::TOKEN + 4].copy_from_slice(&token.to_le_bytes());
            object
        })
        .collect();
    let registered = Box::new([categories[0].as_ptr() as u64, categories[1].as_ptr() as u64]);
    let registry = Box::new(header(registered.as_ptr() as usize, 2));
    let world = &mut populated.world;
    put(
        world,
        base + carmy::EXPERIENCE_THRESHOLDS_RVA.0,
        &header(thresholds.as_ptr() as usize, 3),
    );
    put(
        world,
        base + carmy::EXERCISE_MAXIMUM_LEVEL_RVA.0,
        &2_i32.to_le_bytes(),
    );
    put(
        world,
        base + carmy::EXERCISE_MINIMUM_STRENGTH_RVA.0,
        &50_000_i64.to_le_bytes(),
    );
    put(
        world,
        base + acclimatization::CAMO_SHOW_AT_RVA.0,
        &50_000_i64.to_le_bytes(),
    );
    put(
        world,
        base + category::REGISTRY_RVA.0,
        &(registry.as_ptr() as u64).to_le_bytes(),
    );
    let groups: Vec<Box<[u8; 0x200]>> = [1_u8, 0]
        .into_iter()
        .enumerate()
        .map(|(index, training)| {
            let mut object = Box::new([0; 0x200]);
            object[..8].copy_from_slice(&((base + group::VTABLE_RVA.0) as u64).to_le_bytes());
            object[group::HANDLE..group::HANDLE + 8].copy_from_slice(
                &((index as u64 + 1) << 32 | u64::from(group::REFERENCE_TYPE)).to_le_bytes(),
            );
            object[group::TRAINING] = training;
            object
        })
        .collect();
    let entries: Vec<Box<[u8; 32]>> = [9_000_000_i64, 7_000_000]
        .into_iter()
        .map(|weight| {
            let mut list = Box::new([0; 32]);
            for (entry, weight) in [weight, weight / 2].into_iter().enumerate() {
                let at = entry * 16;
                list[at..at + 8]
                    .copy_from_slice(&(categories[entry].as_ptr() as u64).to_le_bytes());
                list[at + 8..at + 16].copy_from_slice(&weight.to_le_bytes());
            }
            list
        })
        .collect();
    let mut manpower = Box::new([0; 8]);
    manpower[4..].copy_from_slice(&100_i32.to_le_bytes());
    let armies = world.armies.clone();
    for (index, &army) in armies.iter().enumerate() {
        put(
            world,
            army + carmy::RETAINED_VISUAL_VARIANT,
            &(index as i32 * 37 - 400).to_le_bytes(),
        );
        let object = army + carmy::ACCLIMATIZATION;
        put(
            world,
            object + acclimatization::MAXIMUM,
            &15_000_000_i64.to_le_bytes(),
        );
        put(
            world,
            army + carmy::MANPOWER_DATA,
            &header(manpower.as_ptr() as usize, 1),
        );
        let experience = if index % 8 == 5 { 20_000_i64 } else { 15_000 };
        put(world, army + carmy::EXPERIENCE, &experience.to_le_bytes());
        let (training, list) = match index % 4 {
            0 => (None, None),
            1 => (Some(true), Some((entries[0].as_ptr() as usize, 2))),
            2 => {
                put(world, army + carmy::STRENGTH, &50_000_i64.to_le_bytes());
                (Some(true), Some((entries[1].as_ptr() as usize, 2)))
            }
            _ => (Some(false), Some((0x100, 2))),
        };
        if let Some(training) = training {
            let group = &groups[usize::from(!training)];
            put(
                world,
                army + carmy::ORDERS_GROUP,
                &(group.as_ptr() as u64).to_le_bytes(),
            );
        }
        if let Some((data, count)) = list {
            put(world, object + acclimatization::DATA, &header(data, count));
        }
    }
    populated.census.clear();
    populated
        .census
        .capture(&populated.world.live(), base, state);
    assert!(!populated.census.overflowed);
    let inputs = engine_allocation::SharedBox::clone(populated.census.unit_inputs().unwrap());
    assert_eq!(inputs.outcome(), Outcome::Rows { len: PRESENTED });

    let changes = Watches::enabled(2 * PRESENTED);
    let slot = held_store::Slot::new(Box::new(
        collector_division_parts::attack_stats::ArmyStatStore::new(),
    )).unwrap();
    let mut full = populated.capture();
    full.enable_owned_settlement();
    full.status_workers = 2;
    let mut witness =
        crate::extended::division_model_inputs::Captured::with_capacities(&Default::default());
    for hour in [1, 2] {
        slot.prepare_hour(hour, false);
        let mut store = slot.take().unwrap();
        let context = CaptureContext {
            hour,
            ..populated.context()
        };
        let live = populated.world.live();
        super::daily_status::acquire(
            &live,
            &mut full,
            &mut store,
            context,
            inputs.seeds(),
            inputs.outcome(),
            &changes,
        );
        assert!(
            full.retain_owned_settlement(
                store,
                Some(engine_allocation::SharedBox::clone(&inputs)),
                context
            )
            .is_none()
        );
        full.decode_inputs();
        let TableView::Complete(TableRows::DivisionStatus(status)) = full.tables()[1].clone()
        else {
            panic!("complete DivisionStatus at {hour}");
        };
        drop(full.take_settled_store().unwrap());

        witness.reset();
        witness.capture(&live, base, state, Some(&inputs));
        let TableView::Complete(TableRows::DivisionModelInputs(rows)) = witness.tables()[0].clone()
        else {
            panic!("complete DivisionModelInputs at {hour}");
        };
        assert_eq!(witness.training(), Some((PRESENTED / 2) as u32));
        assert_eq!(rows.len(), status.len());
        for (row, status) in rows.iter().zip(&status) {
            assert_eq!(
                (
                    row.division,
                    row.model_number,
                    Recorded::Value(row.camouflage),
                    row.exercising
                ),
                (
                    status.division,
                    Some(status.model_number),
                    status.camouflage,
                    Some(status.exercising)
                ),
                "hour {hour}"
            );
        }
        let count = |test: &dyn Fn(&replay_data::rows::DivisionModelInputRow) -> bool| {
            rows.iter().filter(|row| test(row)).count()
        };
        assert_eq!(count(&|row| row.exercising == Some(1)), PRESENTED / 8);
        assert_eq!(
            count(&|row| row
                .camouflage
                .is_some_and(|shown| shown.category == Some(1))),
            PRESENTED / 4
        );
        assert_eq!(count(&|row| row.camouflage.is_none()), PRESENTED / 4);
    }
}
