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
                source_record_id: "record-42".into(),
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
            source_record_id: "record-42".into(),
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

fn a29_source_record_sha256(slot: i64, branch: u32, quartet: [i64; 4]) -> (String, String, String) {
    let candidate_set_sha256 = "ab".repeat(32);
    let selected_present = format!(
        "{{\"candidate_count\":1,\"candidate_set_sha256\":\"{candidate_set_sha256}\",\"selected_candidate_id\":\"7\",\"selected_ordinal\":0,\"selected_game_hour_raw\":43800024,\"selected_qpc_ticks\":\"4\",\"selection_cutoff_qpc_ticks\":\"5\"}}"
    );
    let selected_present_sha256 = snapshot::sha256::Sha256::hex(selected_present.as_bytes());
    let mut source_record = Vec::new();
    source_record.extend_from_slice(b"the game:a729:a29-observed-producer:selected-present:v3\0");
    source_record.extend_from_slice(
        b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
    );
    source_record.extend_from_slice(
        b"82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2",
    );
    source_record.extend_from_slice(
        b"3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2",
    );
    source_record.extend_from_slice(
        b"8d802c9eb2fe5e78ab52d7512d8d01dd5793b9c29fac8627c17fb6cafcc621ba",
    );
    source_record.extend_from_slice(
        b"44169d40d8ccc979f2f78eb7be11e2e7995e28dc4f3c6feb9da48ae928257517",
    );
    source_record.extend_from_slice(&0x01BC_F384i64.to_le_bytes());
    source_record.extend_from_slice(&((slot + 1) as u64).to_le_bytes());
    source_record.extend_from_slice(&2u32.to_le_bytes());
    source_record.extend_from_slice(&3u64.to_le_bytes());
    source_record.extend_from_slice(&10_000_000u64.to_le_bytes());
    source_record.extend_from_slice(&43_800_024i32.to_le_bytes());
    source_record.extend_from_slice(&(slot as u32).to_le_bytes());
    source_record.extend_from_slice(&((200 + slot) as u32).to_le_bytes());
    source_record.extend_from_slice(&branch.to_le_bytes());
    for raw in quartet {
        source_record.extend_from_slice(&raw.to_le_bytes());
    }
    source_record.extend_from_slice(&1u32.to_le_bytes());
    source_record.extend_from_slice(&7u64.to_le_bytes());
    source_record.extend_from_slice(&0u32.to_le_bytes());
    source_record.extend_from_slice(&43_800_024i32.to_le_bytes());
    source_record.extend_from_slice(&4u64.to_le_bytes());
    source_record.extend_from_slice(&5u64.to_le_bytes());
    source_record.extend_from_slice(candidate_set_sha256.as_bytes());
    source_record.extend_from_slice(selected_present_sha256.as_bytes());
    (
        snapshot::sha256::Sha256::hex(&source_record),
        candidate_set_sha256,
        selected_present_sha256,
    )
}

fn live(slot: i64, name: &str, pct: f64, eta: i64) -> ResearchLiveProgressRow {
    let (source_record_sha256, candidate_set_sha256, selected_present_sha256) =
        a29_source_record_sha256(slot, 2, [150_000, 300_000, 50_000, 200_000]);
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
        ahead_of_time_fixed_scale: Leaf::Measured(100_000),
        ahead_of_time_years_raw: Leaf::Measured(150_000),
        ahead_of_time_penalty_raw: Leaf::Measured(300_000),
        ahead_of_time_reduction_years_raw: Leaf::Measured(50_000),
        ahead_of_time_reduced_penalty_raw: Leaf::Measured(200_000),
        ahead_of_time_provenance_authority: Leaf::Measured(
            "a729_observed_research_ahead_selected_present_v3".into(),
        ),
        ahead_of_time_provenance_executable_sha256: Leaf::Measured(
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".into(),
        ),
        ahead_of_time_provenance_exact_root_job_id: Leaf::Measured(
            "20260720T124311Z-w5-a29-effective-producer-roots-e50d79d0".into(),
        ),
        ahead_of_time_provenance_exact_root_manifest_sha256: Leaf::Measured(
            "82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2".into(),
        ),
        ahead_of_time_provenance_sentinel_job_id: Leaf::Measured(
            "20260720T130641Z-w5-a29-hook-sentinels-and-substitution-i-9016d012".into(),
        ),
        ahead_of_time_provenance_sentinel_dump_sha256: Leaf::Measured(
            "3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2".into(),
        ),
        ahead_of_time_provenance_action_gate_job_id: Leaf::Measured(
            "20260721T075619Z-wave6-a29-active-ahead-ui-producer-ced43f28".into(),
        ),
        ahead_of_time_provenance_action_gate_manifest_sha256: Leaf::Measured(
            "8d802c9eb2fe5e78ab52d7512d8d01dd5793b9c29fac8627c17fb6cafcc621ba".into(),
        ),
        ahead_of_time_provenance_action_gate_sentinel_job_id: Leaf::Measured(
            "20260721T081803Z-wave6-a29-year-action-gate-sentinels-442109ac".into(),
        ),
        ahead_of_time_provenance_action_gate_sentinel_dump_sha256: Leaf::Measured(
            "44169d40d8ccc979f2f78eb7be11e2e7995e28dc4f3c6feb9da48ae928257517".into(),
        ),
        ahead_of_time_provenance_ui_year_producer_rva: Leaf::Measured(0x01BC_EA60),
        ahead_of_time_provenance_ui_year_helper_rva: Leaf::Measured(0x00AB_EAA0),
        ahead_of_time_provenance_ui_year_action_identity_compare_rva: Leaf::Measured(0x01BC_F326),
        ahead_of_time_provenance_ui_year_helper_return_rva: Leaf::Measured(0x01BC_F384),
        ahead_of_time_provenance_effective_value_producer_rva: Leaf::Measured(0x00AB_E4C0),
        ahead_of_time_provenance_localization_consumer_rva: Leaf::Measured(0x0223_0F20),
        ahead_of_time_provenance_source_record_sha256: Leaf::Measured(source_record_sha256),
        ahead_of_time_provenance_localization_branch: Leaf::Measured(2),
        ahead_of_time_provenance_callback_sequence: Leaf::Measured(slot + 1),
        ahead_of_time_provenance_callback_thread_id: Leaf::Measured(2),
        ahead_of_time_provenance_producer_game_hour_raw: Leaf::Measured(43_800_024),
        ahead_of_time_provenance_callback_qpc_ticks: Leaf::Measured(3),
        ahead_of_time_provenance_qpc_frequency_hz: Leaf::Measured(10_000_000),
        ahead_of_time_provenance_selected_present_candidate_count: Leaf::Measured(1),
        ahead_of_time_provenance_selected_present_candidate_set_sha256: Leaf::Measured(
            candidate_set_sha256,
        ),
        ahead_of_time_provenance_selected_present_candidate_id: Leaf::Measured(7),
        ahead_of_time_provenance_selected_present_ordinal: Leaf::Measured(0),
        ahead_of_time_provenance_selected_present_game_hour_raw: Leaf::Measured(43_800_024),
        ahead_of_time_provenance_selected_present_qpc_ticks: Leaf::Measured(4),
        ahead_of_time_provenance_selected_present_cutoff_qpc_ticks: Leaf::Measured(5),
        ahead_of_time_provenance_selected_present_receipt_sha256: Leaf::Measured(
            selected_present_sha256,
        ),
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
fn save_rows_cannot_replace_observed_active_producer_receipts() {
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
    assert_eq!(
        admit(
            &research,
            &technology,
            &doctrine_canvas,
            &technology_order,
            &doctrine_order,
            &definitions,
        ),
        Err(ResearchStateRejection::ActiveSlotsUnavailable)
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
fn observed_no_penalty_branch_is_admitted_but_wrong_authority_rejects() {
    let (technology, doctrine_canvas) = canvases();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let mut no_penalty = research();
    let Leaf::Measured(rows) = &mut no_penalty.in_progress_live else {
        unreachable!()
    };
    rows[0].ahead_of_time_years = Leaf::Measured(0.0);
    rows[0].ahead_of_time_penalty_pct = Leaf::Measured(0.0);
    rows[0].ahead_of_time_reduced_penalty_pct = Leaf::Measured(0.0);
    rows[0].ahead_of_time_years_raw = Leaf::Measured(0);
    rows[0].ahead_of_time_penalty_raw = Leaf::Measured(0);
    rows[0].ahead_of_time_reduced_penalty_raw = Leaf::Measured(0);
    rows[0].ahead_of_time_provenance_localization_branch = Leaf::Measured(0);
    let (source_record_sha256, _, _) =
        a29_source_record_sha256(1, 0, [0, 0, 50_000, 0]);
    rows[0].ahead_of_time_provenance_source_record_sha256 =
        Leaf::Measured(source_record_sha256);
    let admitted = admit(
        &no_penalty,
        &technology,
        &doctrine_canvas,
        &technology_order,
        &doctrine_order,
        &definitions,
    )
    .expect("exact producer no-penalty branch");
    let slot = admitted
        .slots()
        .iter()
        .find(|slot| slot.tech_id == "tech_b")
        .expect("branch-zero slot");
    assert!(ahead_of_time_tooltip_inputs(slot).is_none());

    let Leaf::Measured(rows) = &mut no_penalty.in_progress_live else {
        unreachable!()
    };
    rows[0].ahead_of_time_provenance_authority =
        Leaf::Measured("base_factor_reconstruction".into());
    assert_eq!(
        admit(
            &no_penalty,
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
fn observed_ahead_receipt_cannot_be_transplanted_to_another_research_slot() {
    let (technology, doctrine_canvas) = canvases();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    let mut transplanted = research();
    let Leaf::Measured(rows) = &mut transplanted.in_progress_live else {
        unreachable!()
    };
    // Keep the authenticated producer hash unchanged while moving its row.
    // The v2 admission preimage binds slot 1, so slot 0 must fail closed.
    rows[0].slot_index = Leaf::Measured(0);
    assert_eq!(
        admit(
            &transplanted,
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
    rows[1] = rows[0].clone();
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

#[test]
fn canvas_and_state_source_records_must_match() {
    let (technology, doctrine_canvas) = canvases();
    let research = research();
    let definitions = definitions();
    let technology_order = vec!["tech_done".into(), "tech_a".into(), "tech_b".into()];
    let doctrine_order = vec!["doctrine_a".into(), "doctrine_b".into()];
    assert_eq!(
        admit_research_state(ResearchStateRequest {
            source_record_id: "record-43",
            research: &research,
            technology_canvas: &technology,
            doctrine_canvas: &doctrine_canvas,
            technology_node_order: &technology_order,
            doctrine_node_order: &doctrine_order,
            technology_definitions: &definitions,
        }),
        Err(ResearchStateRejection::SourceRecordMismatch)
    );
}
