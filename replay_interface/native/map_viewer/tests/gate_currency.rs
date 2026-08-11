//! The compiled test gates must still describe the environment this run sees.
//!
//! Without this, a stale build-script probe could leave a gated test `ignored`
//! on a machine where its precondition is now available, or leave it runnable
//! after the resource disappeared.

include!("../test_gates.rs");

#[test]
fn compiled_test_gates_match_the_current_environment() {
    replay_test_gates::assert_gates_are_current(GATES, &|cfg| match cfg {
        "gate_hoi4_install" => cfg!(gate_hoi4_install),
        "gate_renderdoc_mined" => cfg!(gate_renderdoc_mined),
        "gate_retained_production_frame" => cfg!(gate_retained_production_frame),
        "gate_border_pool_with_hoi4" => cfg!(gate_border_pool_with_hoi4),
        "gate_wave1_cap4_no_fow" => cfg!(gate_wave1_cap4_no_fow),
        "gate_wave1_cap4_fow_pair" => cfg!(gate_wave1_cap4_fow_pair),
        "gate_restore_gate_1_capture" => cfg!(gate_restore_gate_1_capture),
        "gate_restore_gate_1_scene" => cfg!(gate_restore_gate_1_scene),
        "gate_map_bundle" => cfg!(gate_map_bundle),
        "gate_map_bundle_with_hoi4" => cfg!(gate_map_bundle_with_hoi4),
        "gate_replay_fixtures" => cfg!(gate_replay_fixtures),
        "gate_reflection_cube_proof" => cfg!(gate_reflection_cube_proof),
        "gate_river_captures_with_hoi4" => cfg!(gate_river_captures_with_hoi4),
        "gate_shader_capture_with_hoi4" => cfg!(gate_shader_capture_with_hoi4),
        "gate_terrain_id_capture_with_hoi4" => cfg!(gate_terrain_id_capture_with_hoi4),
        "gate_terrain_mesh_corpus" => cfg!(gate_terrain_mesh_corpus),
        "gate_trees2_shader_capture" => cfg!(gate_trees2_shader_capture),
        "gate_trees2_shader_capture_with_hoi4" => cfg!(gate_trees2_shader_capture_with_hoi4),
        "gate_trees2_template_capture_with_hoi4" => {
            cfg!(gate_trees2_template_capture_with_hoi4)
        }
        "gate_trees_measure_capture" => cfg!(gate_trees_measure_capture),
        "gate_trees_measure_capture_with_hoi4" => {
            cfg!(gate_trees_measure_capture_with_hoi4)
        }
        "gate_underwater_evidence_with_hoi4" => cfg!(gate_underwater_evidence_with_hoi4),
        other => panic!("gate {other} has no cfg!() mapping in this test"),
    });
}
