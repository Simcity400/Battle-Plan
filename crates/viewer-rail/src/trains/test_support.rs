use super::*;

pub(super) const DEFINES: TrainDefines = TrainDefines {
    speed: 3.0,
    timetable: [9200, 12000],
    wagon_count: [3, 6],
};

pub(super) fn consist() -> Consist {
    Consist {
        wagon_0: Some(0),
        wagon_1: Some(0),
        wagon_2: None,
        wagon_last: Some(0),
        wagons: vec![0],
    }
}

pub fn assets() -> TrainAssets {
    TrainAssets {
        library: ModelLibrary::default(),
        cultures: Cultures::default(),
        database: train_gfx::Database {
            wagons: vec![Wagon {
                entity: "car".into(),
                scale: 1.0,
                rotation: 0.0,
                shift: [0.0; 3],
                length: 2.0,
            }],
            ..Default::default()
        },
        equipment: content_reader_key_names::equipment::Database::default(),
        defines: DEFINES,
        routes: RouteCache::default(),
    }
}

pub fn next_hour_plan() -> TrainPlan {
    let mut plan = plan();
    plan.hour += 1;
    plan.allocations[0].count = 3;
    plan.fleet_cap = 3;
    plan
}

pub(super) fn route(path: Vec<u32>, points: Vec<[f32; 3]>, distance: Vec<f32>, seed: u32) -> Route {
    Route {
        path,
        curve: Arc::new((crate::railways::TrainRoute { points, open_end: false }, distance)),
        seed,
    }
}

pub fn plan() -> TrainPlan {
    TrainPlan {
        routes: vec![route(vec![1, 2], vec![[0.0, 10.0, 0.0], [600.0, 10.0, 0.0]], vec![0.0, 600.0], SEED)],
        weights: vec![100000],
        departures: vec![vec![0]],
        allocations: vec![TrainAllocation {
            variant: 70,
            count: 100,
        }],
        consists: BTreeMap::from([(70, Some(consist()))]),
        fleet_cap: 100,
        branch: 1,
        hour: 10,
        country: 1,
        ..TrainPlan::default()
    }
}
