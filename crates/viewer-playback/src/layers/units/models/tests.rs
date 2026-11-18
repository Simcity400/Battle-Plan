use super::{Models, Select, tables};
use crate::playhead_host::{
    open,
    rows::{
        Rows,
        tests::{COMBATS, RECORDING, combat, hourly_steps, record_with, write},
    },
};
use replay_columnar::manifest::Status;
use replay_data::{rows::TableId, scope::CaptureMode};
use replay_frame::frame::DecodedFrame;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use viewer_foundation::map::perspective::Choice;
use viewer_world::map::{trade_routes::ConvoyLine, unit_models::PreparedModels};

/// Counts the jobs it is given and refuses each by name: a test makes no models without the content.
#[derive(Default)]
struct Counted {
    jobs: AtomicUsize,
}

impl Select for Counted {
    fn choose_meshes(&self, _frame: &DecodedFrame, _ours: u32, _kept: (&viewer_world::map::unit_models::KeptChoices, Option<&std::collections::BTreeSet<u64>>), _stacks: &std::collections::HashMap<u64, super::super::meshes::Asked>, _asked: &std::collections::HashSet<u64>, _told: &viewer_world::map::unit_models::Told, _parts: &mut dyn FnMut(&'static str, u64)) -> io::Result<super::super::meshes::Chosen> {
        Err(io::Error::other("no unit meshes in this test"))
    }

    fn prepare(&self, _frame: &DecodedFrame, _perspective: &Choice, _convoys: Option<&[ConvoyLine]>, _naval: bool, _kept: &mut super::Kept) -> io::Result<PreparedModels> {
        self.jobs.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::other("no unit models in this test"))
    }
}

#[test]
fn the_models_declare_the_tables_they_are_known_to_read_and_those_learned_not_the_map_selection() {
    let entry = open(record_with("models tables", 2, |_, _| {}), hourly_steps()).unwrap();
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let models = Models::new(Arc::new(Counted::default()), &rows, &[TableId::Operations]);
    let mut wanted = tables();
    wanted.push(TableId::Operations);
    wanted.sort_unstable_by_key(|table| table.index());
    assert_eq!(models.tables(), wanted);
    assert!(tables().len() < dashboard_base::map_tables::selected_tables().len() / 2, "{} tables", tables().len());
    let until = Instant::now() + Duration::from_secs(10);
    assert!(rows.declared(&wanted, until), "the reader declares every table named and learned");
    assert!(rows.released(TableId::AirRegionalMissions, until), "a table the models do not read is not declared");
}

#[test]
fn a_job_is_sent_only_for_rows_whose_declared_tables_changed() {
    // The Clock, which the ships' movement reads, from hour 0 and its speed moving at hour 2;
    // LandCombats, which the job no longer reads (the land figures are the layer's own), moving at
    // hour 1.
    let speed = crate::layers::recorded::column(TableId::Clock, "speed");
    let recorded = record_with("models jobs", 3, |store, at| {
        for table in [COMBATS, TableId::Clock] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        match at {
            0 => {
                write(store, COMBATS, &[combat(30, 0), combat(4, 0)]);
                let mut clock = store.table(TableId::Clock.index()).unwrap();
                clock.insert_at(0, 0).unwrap();
                clock.write(0, speed, replay_columnar::state::Cell::I32(1)).unwrap();
            }
            1 => write(store, COMBATS, &[combat(30, 1)]),
            2 => store.table(TableId::Clock.index()).unwrap().write(0, speed, replay_columnar::state::Cell::I32(2)).unwrap(),
            _ => {}
        }
    });
    let entry = open(recorded, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let counted = Arc::new(Counted::default());
    let mut models = Models::new(Arc::clone(&counted) as Arc<dyn Select>, &rows, &[]);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    let choice = Choice::default();
    for (hour, jobs) in [(0, 1), (1, 1), (2, 2)] {
        rows.drive(hour);
        models.exchange(&choice, None, true, Some(hour));
        assert_eq!(counted.jobs.load(Ordering::SeqCst), jobs, "hour {hour}: a job only when a table the job reads moved ({:?} read)", models.tables());
    }
    assert!(models.refused.as_deref().is_some_and(|why| why.contains("no unit models in this test")), "{:?}", models.refused);
}
