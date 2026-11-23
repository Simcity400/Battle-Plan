use collector_divisions::extended::division_presentation::{
    CaptureContext, Captured, N, STATUS_TABLES, TABLES,
    tests::{PRESENTED, Populated, pass_failure_capacities, presentation_fixture},
};
use collector_production::production::Sources;
use engine_native_memory::walk::Outcome;
use replay_data::rows::TableId;
use replay_frame::frame::TableView;

#[test]
fn map_presentation_reads_only_the_movement_full_reads() {
    let populated = Populated::new();
    let inputs = engine_allocation::SharedBox::clone(populated.census.unit_inputs().unwrap());
    let sources = || Sources {
        rows: inputs.seeds(),
        outcome: inputs.outcome(),
    };
    let memory = engine_native_memory::read_counts::CountedMemory::new(populated.world.live());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut runs = Vec::new();
    for map in [false, true] {
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        captured.division_presentation = populated.capture();
        captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&inputs)));
        let context = populated.context();
        let mut request = captured.take_presentation_request(
            &countries,
            context.base,
            context.game_state,
            context.hour,
            None,
        );
        let mut movement = request.take_movement();
        let reads = collector_memory::read_accounting::Capture::new();
        let report = if map {
            movement.pass.omit_status();
            let ticks = movement.pass.capture(
                &memory,
                context.base,
                context.game_state,
                sources(),
                Some(context.hour),
            );
            let status = request
                .captured
                .omit_status(collector_extended_support::outcomes::NOT_RECORDED);
            request.restore_movement(movement, Ok(ticks), Ok(status))
        } else {
            let mut terms = request.take_speed_terms();
            let speed_ticks =
                terms
                    .pass
                    .capture(&memory, context.base, context.game_state, sources());
            let ticks = movement.pass.capture(
                &memory,
                context.base,
                context.game_state,
                sources(),
                Some(context.hour),
            );
            let status = request
                .captured
                .capture_status_holding(&memory, context, sources(), None);
            let report = request.restore_movement(movement, Ok(ticks), Ok(status));
            request.restore_speed_terms(terms, Ok(speed_ticks), report)
        };
        let counts = reads.finish();
        assert!(report.is_ok());
        captured.restore_presentation_request(request, report);
        captured.decode_division_presentation_inputs();
        runs.push((counts, captured.division_presentation.tables()));
    }
    let [(full_reads, full), (map_reads, map)] = <[_; 2]>::try_from(runs).ok().unwrap();
    assert!(matches!(full[0], TableView::Complete(_)));
    assert_eq!(map[0], full[0]);
    let movement = TableId::DivisionMovement.index();
    assert_eq!(map_reads[movement], full_reads[movement]);
    assert_ne!(
        full_reads[TableId::DivisionStatus.index()],
        engine_native_memory::read_counts::ReadCounts::ZERO
    );
    for table in STATUS_TABLES {
        assert_eq!(
            map_reads[table.index()],
            engine_native_memory::read_counts::ReadCounts::ZERO,
            "{table:?}"
        );
        let view = &map[TABLES.iter().position(|&id| id == table).unwrap()];
        assert!(
            matches!(view, TableView::Unavailable { .. }),
            "{table:?}: {view:?}"
        );
    }
}

#[test]
fn a_failed_presentation_job_publishes_no_presentation_table() {
    use collector_workers::acquisition::Failure;
    let populated = Populated::new();
    let inputs = engine_allocation::SharedBox::clone(populated.census.unit_inputs().unwrap());
    let sources = || Sources {
        rows: inputs.seeds(),
        outcome: inputs.outcome(),
    };
    let memory = populated.world.live();
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    for (status_failure, movement_failure, speed_failure) in [
        (Some(Failure::Stopped), None, None),
        (None, Some(Failure::Panicked), None),
        (None, Some(Failure::Stopped), None),
        (None, None, Some(Failure::Panicked)),
        (None, None, Some(Failure::Stopped)),
        (
            Some(Failure::Stopped),
            Some(Failure::Stopped),
            Some(Failure::Stopped),
        ),
    ] {
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        captured.division_presentation = populated.capture();
        captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&inputs)));
        let context = populated.context();
        let mut request = captured.take_presentation_request(
            &countries,
            context.base,
            context.game_state,
            context.hour,
            None,
        );
        let mut movement = request.take_movement();
        let mut terms = request.take_speed_terms();
        let speed_ticks = terms
            .pass
            .capture(&memory, context.base, context.game_state, sources());
        let speed_bytes = terms.allocated_bytes();
        let ticks = movement.pass.capture(
            &memory,
            context.base,
            context.game_state,
            sources(),
            Some(context.hour),
        );
        let status = request
            .captured
            .capture_status_holding(&memory, context, sources(), None);
        assert_eq!(
            movement.pass.movement.outcome,
            Outcome::Rows { len: PRESENTED }
        );
        assert_eq!(
            request.captured.status.outcome,
            Outcome::Rows { len: PRESENTED }
        );
        let report = request.restore_movement(
            movement,
            movement_failure.map_or(Ok(ticks), Err),
            status_failure.map_or(Ok(status), Err),
        );
        let report =
            request.restore_speed_terms(terms, speed_failure.map_or(Ok(speed_ticks), Err), report);
        assert_eq!(request.captured.speed_terms.allocated_bytes(), speed_bytes);
        assert!(report.is_err());
        captured.restore_presentation_request(request, report);
        assert_eq!(captured.acquire_presentation().ticks, [0; N]);
        captured.decode_division_presentation_inputs();
        for (id, view) in TABLES
            .into_iter()
            .zip(captured.division_presentation.tables())
        {
            assert!(
                matches!(view, TableView::Unavailable { .. }),
                "{id:?} published after {status_failure:?} {movement_failure:?} {speed_failure:?}"
            );
        }
    }
}

#[test]
fn failed_presentation_passes_keep_the_merged_failure_order() {
    use collector_workers::acquisition::Failure;
    let (seed, bytes) = presentation_fixture();
    let seeds = [seed];
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let context = CaptureContext {
        base: 0,
        game_state: 0x1000,
        hour: 0,
    };
    let sources = || Sources {
        rows: &seeds,
        outcome: Outcome::Rows { len: 1 },
    };
    for (status_failure, movement_failure, speed_failure) in [
        (None, None, None),
        (Some(Failure::Stopped), None, None),
        (Some(Failure::Panicked), None, None),
        (None, Some(Failure::Panicked), None),
        (None, Some(Failure::Stopped), None),
        (None, None, Some(Failure::Panicked)),
        (None, None, Some(Failure::Stopped)),
        (Some(Failure::Stopped), Some(Failure::Panicked), None),
        (
            Some(Failure::Panicked),
            Some(Failure::Stopped),
            Some(Failure::Panicked),
        ),
    ] {
        let mut captured = Captured::with_capacities(&pass_failure_capacities());
        captured.reserve_sources(1);
        let mut request = crate::acquisition_jobs::PresentationRequest {
            dispatch: Default::default(),
            countries: engine_allocation::shared(
                engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
            )
            .unwrap(),
            context,
            captured,
            inputs: None,
            army_stats: None,
        };
        let mut movement = request.take_movement();
        let mut terms = request.take_speed_terms();
        let speed_ticks = terms
            .pass
            .capture(&memory, context.base, context.game_state, sources());
        let speed_bytes = terms.allocated_bytes();
        let ticks = movement.pass.capture(
            &memory,
            context.base,
            context.game_state,
            sources(),
            Some(context.hour),
        );
        assert_eq!(movement.pass.movement.outcome, Outcome::Rows { len: 1 });
        let status = request
            .captured
            .capture_status_holding(&memory, context, sources(), None);
        let report = request.restore_movement(
            movement,
            movement_failure.map_or(Ok(ticks), Err),
            status_failure.map_or(Ok(status), Err),
        );
        let report =
            request.restore_speed_terms(terms, speed_failure.map_or(Ok(speed_ticks), Err), report);
        assert_eq!(request.captured.speed_terms.allocated_bytes(), speed_bytes);
        assert_eq!(request.captured.outcomes()[0], Outcome::Rows { len: 1 });
        match speed_failure.or(movement_failure).or(status_failure) {
            None => {
                let report = report.unwrap();
                assert_eq!(report.ticks[0], ticks.ticks[0]);
                assert_eq!(report.ticks[1], status.ticks[1] + ticks.ticks[1]);
                assert_eq!(report.ticks[2], status.ticks[2]);
                assert_eq!(report.ticks[3], ticks.ticks[3]);
                assert_eq!(report.ticks[7..], speed_ticks.ticks[7..]);
            }
            failure => assert_eq!(report.err(), failure),
        }
    }
}
