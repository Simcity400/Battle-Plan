//! The compiled test gates must still describe the environment this run sees.
//!
//! Without this, a stale build-script probe could leave a gated test `ignored`
//! on a machine where its precondition is now available — quiet
//! non-execution of exactly the kind the gate mechanism exists to remove — or
//! leave it runnable after the resource disappeared. `rerun-if-env-changed`
//! cannot see a directory appearing at an unchanged path, so this check is the
//! safety net for that case.

include!("../test_gates.rs");

#[test]
fn compiled_test_gates_match_the_current_environment() {
    replay_test_gates::assert_gates_are_current(GATES, &|cfg| match cfg {
        "gate_hoi4_install" => cfg!(gate_hoi4_install),
        "gate_restore_gate_1_capture" => cfg!(gate_restore_gate_1_capture),
        "gate_xmap_native_evidence_replay" => cfg!(gate_xmap_native_evidence_replay),
        other => panic!("gate {other} has no cfg!() mapping in this test"),
    });
}
