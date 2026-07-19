use game_ui::compose::ComposeOptions;
use game_ui::research_canvas::{
    admit_research_canvas, DoctrineCanvasRequest, LocalizedTextWitness, PixelRect,
    ProducerDoctrineLayout, ProducerDoctrineTrack, ProducerTechnologyLayout, ResearchCanvasRequest,
    TechnologyCanvasRequest,
};
use game_ui::research_state::{
    admit_research_state, ahead_of_time_tooltip_inputs, apply_research_slots_to_compose_options,
    ResearchNodeState, ResearchStateRejection, ResearchStateRequest, TechnologyDefinition,
};
use replay_core::{
    DoctrineCompletedRow, DoctrineProgressRow, DoctrineView, Leaf, ResearchLiveProgressRow,
    ResearchProgressRow, ResearchTechnologyRow, ResearchView,
};
use std::collections::BTreeMap;

fn text() -> LocalizedTextWitness {
    LocalizedTextWitness {
        locale: Some("english".into()),
        required_keys: vec!["RESEARCH".into()],
        resolved_strings: BTreeMap::from([("RESEARCH".into(), "Research".into())]),
    }
}

fn canvases() -> (
    game_ui::research_canvas::AdmittedResearchCanvas,
    game_ui::research_canvas::AdmittedResearchCanvas,
) {
    let technology = admit_research_canvas(&ResearchCanvasRequest::Technology(
        TechnologyCanvasRequest {
            drawable_ids: vec!["tech_draw".into()],
            producer_layout: Some(ProducerTechnologyLayout {
                source_record_id: "ui09-tech".into(),
                drawable_rectangles: Some(BTreeMap::from([(
                    "tech_draw".into(),
                    PixelRect {
                        x: 10,
                        y: 20,
                        width: 100,
                        height: 40,
                    },
                )])),
                draw_order: Some(vec!["tech_draw".into()]),
                pan_pixels: Some([0.0, 0.0]),
                clip: Some(PixelRect {
                    x: 0,
                    y: 0,
                    width: 500,
                    height: 500,
                }),
            }),
            text: text(),
        },
    ))
    .expect("technology canvas");
    let doctrine = admit_research_canvas(&ResearchCanvasRequest::Doctrine(DoctrineCanvasRequest {
        track_ids: vec!["land".into()],
        producer_layout: Some(ProducerDoctrineLayout {
            source_record_id: "ui09-doctrine".into(),
            tracks: Some(BTreeMap::from([(
                "land".into(),
                ProducerDoctrineTrack {
                    extent: PixelRect {
                        x: 0,
                        y: 0,
                        width: 200,
                        height: 600,
                    },
                    selected_candidate_id: Some("doctrine_b".into()),
                    candidate_order: Some(vec!["doctrine_a".into(), "doctrine_b".into()]),
                },
            )])),
            clip: Some(PixelRect {
                x: 0,
                y: 0,
                width: 400,
                height: 700,
            }),
        }),
        text: text(),
    }))
    .expect("doctrine canvas");
    (technology, doctrine)
}

fn progress(name: &str, pct: f64, eta: i64) -> ResearchProgressRow {
    ResearchProgressRow {
        name: Leaf::Measured(name.into()),
        points_factor: Leaf::Measured(1.0),
        progress_points: Leaf::Measured(30.0),
        required_points_base: Leaf::Measured(100.0),
        remaining_points_base: Leaf::Measured(70.0),
        progress_pct: Leaf::Measured(pct),
        eta_days: Leaf::Measured(eta),
        progress_basis: Leaf::Measured("save_base_denominator".into()),
        progress_unavailable_reason: Leaf::Absent,
        eta_unavailable_reason: Leaf::Absent,
        ahead_of_time_years: Leaf::Measured(1.5),
        ahead_of_time_penalty_pct: Leaf::Measured(3.0),
        ahead_of_time_reduction_years: Leaf::Measured(0.5),
        ahead_of_time_reduced_penalty_pct: Leaf::Measured(2.0),
    }
}

fn live(slot: i64, name: &str, pct: f64, eta: i64) -> ResearchLiveProgressRow {
    ResearchLiveProgressRow {
        slot_index: Leaf::Measured(slot),
        tech_handle_raw: Leaf::Measured(1000 + slot),
        tech_def_index_raw: Leaf::Measured(200 + slot),
        tech_name: Leaf::Measured(name.into()),
        progress_raw: Leaf::Measured(30_000),
        progress_units: Leaf::Measured(30.0),
        live_progress_raw: Leaf::Measured(30_000),
        live_progress_units: Leaf::Measured(30.0),
        base_required_points_raw: Leaf::Measured(100_000),
        base_required_points_units: Leaf::Measured(100.0),
        points_factor: Leaf::Measured(1.0),
        points_factor_elapsed_hours: Leaf::Measured(24),
        required_points_base: Leaf::Measured(100.0),
        remaining_points_base: Leaf::Measured(70.0),
        progress_pct: Leaf::Measured(pct),
        eta_days: Leaf::Measured(eta),
        progress_basis: Leaf::Measured("live_base_denominator".into()),
        progress_unavailable_reason: Leaf::Absent,
        eta_unavailable_reason: Leaf::Absent,
        ahead_of_time_years: Leaf::Measured(1.5),
        ahead_of_time_penalty_pct: Leaf::Measured(3.0),
        ahead_of_time_reduction_years: Leaf::Measured(0.5),
        ahead_of_time_reduced_penalty_pct: Leaf::Measured(2.0),
    }
}

fn doctrine() -> DoctrineView {
    DoctrineView {
        completed: Leaf::Measured(vec![DoctrineCompletedRow {
            id: Leaf::Measured("doctrine_a".into()),
            level: Leaf::Measured(1),
            date: Leaf::Measured("1936.1.1.1".into()),
            research_points: Leaf::Measured(100.0),
        }]),
        completed_count: Leaf::Measured(1),
        in_progress: Leaf::Measured(vec![DoctrineProgressRow {
            id: Leaf::Measured("doctrine_b".into()),
            progress: Leaf::Measured(0.4),
        }]),
        source: Leaf::Measured("country_doctrine".into()),
    }
}

fn research() -> ResearchView {
    ResearchView {
        tech_count: Leaf::Measured(1),
        research_slots: Leaf::Measured(2),
        in_progress: Leaf::Measured(vec![progress("tech_a", 0.1, 30)]),
        in_progress_live: Leaf::Measured(vec![
            live(1, "tech_b", 0.7, 9),
            live(0, "tech_a", 0.3, 21),
        ]),
        technologies: Leaf::Measured(vec![ResearchTechnologyRow {
            name: Leaf::Measured("tech_done".into()),
            level: Leaf::Measured(1),
            date: Leaf::Measured("1936.1.1.1".into()),
            research_points: Leaf::Measured(100.0),
        }]),
        doctrine: Leaf::Measured(doctrine()),
    }
}

fn definitions() -> BTreeMap<String, TechnologyDefinition> {
    [("tech_a", 1937), ("tech_b", 1938), ("tech_done", 1936)]
        .into_iter()
        .map(|(id, year)| {
            (
                id.into(),
                TechnologyDefinition {
                    id: id.into(),
                    display_name: match id {
                        "tech_a" => "Technology A",
                        "tech_b" => "Technology B",
                        _ => "Completed Technology",
                    }
                    .into(),
                    start_year: year,
                },
            )
        })
        .collect()
}

fn admit<'a>(
    research: &'a ResearchView,
    technology: &'a game_ui::research_canvas::AdmittedResearchCanvas,
    doctrine_canvas: &'a game_ui::research_canvas::AdmittedResearchCanvas,
    technology_order: &'a [String],
    doctrine_order: &'a [String],
    definitions: &'a BTreeMap<String, TechnologyDefinition>,
) -> Result<game_ui::research_state::AdmittedResearchState, ResearchStateRejection> {
    admit_research_state(ResearchStateRequest {
        source_record_id: "record-42",
        research,
        technology_canvas: technology,
        doctrine_canvas,
        technology_node_order: technology_order,
        doctrine_node_order: doctrine_order,
        technology_definitions: definitions,
    })
}

#[test]
fn live_rows_win_and_authored_orders_receive_exact_states() {
    let (technology, doctrine_canvas) = canvases();
    let research = research();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec![
        "doctrine_a".into(),
        "doctrine_b".into(),
        "doctrine_c".into(),
    ];
    let admitted = admit(
        &research,
        &technology,
        &doctrine_canvas,
        &technology_order,
        &doctrine_order,
        &definitions,
    )
    .expect("complete state");

    assert_eq!(admitted.source_record_id(), "record-42");
    assert_eq!(admitted.completed_technologies(), ["tech_done"]);
    assert_eq!(
        admitted
            .slots()
            .iter()
            .map(|row| (row.slot_index, row.tech_id.as_str(), row.start_year))
            .collect::<Vec<_>>(),
        vec![(0, "tech_a", 1937), (1, "tech_b", 1938)]
    );
    assert_eq!(
        admitted
            .technology_nodes()
            .iter()
            .map(|row| row.state)
            .collect::<Vec<_>>(),
        vec![
            ResearchNodeState::Completed,
            ResearchNodeState::InProgress,
            ResearchNodeState::InProgress,
        ]
    );
    assert_eq!(
        admitted
            .doctrine_nodes()
            .iter()
            .map(|row| row.state)
            .collect::<Vec<_>>(),
        vec![
            ResearchNodeState::Completed,
            ResearchNodeState::InProgress,
            ResearchNodeState::Unresearched,
        ]
    );
}

#[test]
fn save_rows_are_ordered_by_array_ordinal_when_live_rows_are_unavailable() {
    let (technology, doctrine_canvas) = canvases();
    let mut research = research();
    research.in_progress_live = Leaf::Unobtained;
    research.in_progress = Leaf::Measured(vec![
        progress("tech_b", 0.2, 40),
        progress("tech_a", 0.6, 10),
    ]);
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let admitted = admit(
        &research,
        &technology,
        &doctrine_canvas,
        &technology_order,
        &doctrine_order,
        &definitions,
    )
    .expect("save fallback");
    assert_eq!(
        admitted
            .slots()
            .iter()
            .map(|row| (row.slot_index, row.tech_id.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, "tech_b"), (1, "tech_a")]
    );
}

#[test]
fn exact_slot_widget_names_are_bound_for_visible_compose() {
    let (technology, doctrine_canvas) = canvases();
    let research = research();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let admitted = admit(
        &research,
        &technology,
        &doctrine_canvas,
        &technology_order,
        &doctrine_order,
        &definitions,
    )
    .expect("complete state");
    let mut options = ComposeOptions::new(1920, 1080);
    apply_research_slots_to_compose_options(&admitted, &mut options);
    assert_eq!(
        options.text_overrides["research_slot_0_title"],
        "Technology A"
    );
    assert_eq!(options.text_overrides["research_slot_0_year"], "1937");
    assert_eq!(options.text_overrides["research_slot_0_eta"], "21d");
    assert_eq!(
        options.progress_overrides["research_slot_0_research_progressbar"],
        0.3
    );
    assert_eq!(
        options.text_overrides["research_slot_1_title"],
        "Technology B"
    );
    assert_eq!(
        options.progress_overrides["research_slot_1_research_progressbar"],
        0.7
    );
    let ahead = ahead_of_time_tooltip_inputs(&admitted.slots()[0]).expect("ahead tooltip");
    assert_eq!(ahead.localisation_key, "RESEARCH_YEAR_AHEAD_PENALTY_RED");
    assert_eq!(ahead.substitutions["YEARS"], 1.5);
    assert_eq!(ahead.substitutions["VALUE"], 3.0);
    assert_eq!(ahead.substitutions["REDUCED_YEARS"], 0.5);
    assert_eq!(ahead.substitutions["RED"], 2.0);
}

#[test]
fn missing_effective_ahead_inputs_fail_closed() {
    let (technology, doctrine_canvas) = canvases();
    let mut research = research();
    let Leaf::Measured(rows) = &mut research.in_progress_live else {
        unreachable!()
    };
    rows[0].ahead_of_time_penalty_pct = Leaf::Unobtained;
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    assert_eq!(
        admit(
            &research,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::MissingAheadOfTimeInputs)
    );
}

#[test]
fn duplicate_live_slot_and_invalid_progress_fail_closed() {
    let (technology, doctrine_canvas) = canvases();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let mut duplicate = research();
    let Leaf::Measured(rows) = &mut duplicate.in_progress_live else {
        unreachable!()
    };
    rows[0].slot_index = Leaf::Measured(0);
    assert_eq!(
        admit(
            &duplicate,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::InvalidSlotOrder)
    );

    let mut invalid = research();
    let Leaf::Measured(rows) = &mut invalid.in_progress_live else {
        unreachable!()
    };
    rows[0].progress_pct = Leaf::Measured(1.1);
    assert_eq!(
        admit(
            &invalid,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::InvalidProgressOrEta)
    );
}

#[test]
fn missing_definition_and_completed_active_overlap_fail_closed() {
    let (technology, doctrine_canvas) = canvases();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let mut missing = definitions();
    missing.remove("tech_b");
    assert_eq!(
        admit(
            &research(),
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &missing,
        ),
        Err(ResearchStateRejection::MissingTechnologyDefinition)
    );

    let mut overlap = research();
    overlap.technologies = Leaf::Measured(vec![ResearchTechnologyRow {
        name: Leaf::Measured("tech_a".into()),
        level: Leaf::Measured(1),
        date: Leaf::Measured("1936.1.1.1".into()),
        research_points: Leaf::Measured(100.0),
    }]);
    assert_eq!(
        admit(
            &overlap,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions(),
        ),
        Err(ResearchStateRejection::InvalidTechnologySubset)
    );
}

#[test]
fn doctrine_count_progress_and_authored_subset_fail_closed() {
    let (technology, doctrine_canvas) = canvases();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let mut bad_count = research();
    let Leaf::Measured(doctrine) = &mut bad_count.doctrine else {
        unreachable!()
    };
    doctrine.completed_count = Leaf::Measured(2);
    assert_eq!(
        admit(
            &bad_count,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::InvalidDoctrineSubset)
    );

    let mut bad_progress = research();
    let Leaf::Measured(doctrine) = &mut bad_progress.doctrine else {
        unreachable!()
    };
    let Leaf::Measured(rows) = &mut doctrine.in_progress else {
        unreachable!()
    };
    rows[0].progress = Leaf::Measured(-0.1);
    assert_eq!(
        admit(
            &bad_progress,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::InvalidDoctrineSubset)
    );

    let missing_doctrine_order = vec!["doctrine_a".into()];
    assert_eq!(
        admit(
            &research(),
            &technology,
            &doctrine_canvas,
            &technology_order,
            &missing_doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::InvalidDoctrineSubset)
    );
}

#[test]
fn wrong_canvas_kinds_and_empty_provenance_fail_closed() {
    let (technology, doctrine_canvas) = canvases();
    let research = research();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    assert_eq!(
        admit_research_state(ResearchStateRequest {
            source_record_id: "record-42",
            research: &research,
            technology_canvas: &doctrine_canvas,
            doctrine_canvas: &technology,
            technology_node_order: &technology_order,
            doctrine_node_order: &doctrine_order,
            technology_definitions: &definitions,
        }),
        Err(ResearchStateRejection::WrongCanvasKind)
    );
    assert_eq!(
        admit_research_state(ResearchStateRequest {
            source_record_id: "",
            research: &research,
            technology_canvas: &technology,
            doctrine_canvas: &doctrine_canvas,
            technology_node_order: &technology_order,
            doctrine_node_order: &doctrine_order,
            technology_definitions: &definitions,
        }),
        Err(ResearchStateRejection::MissingProvenance)
    );
}
