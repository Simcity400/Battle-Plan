//! The compiled test gates must still describe the environment this run sees.
//!
//! Without this, a stale build-script probe could leave a gated test `ignored`
//! on a machine where its precondition is now available — quiet
//! non-execution of exactly the kind the gate mechanism exists to remove — or
//! leave it runnable after the resource disappeared.

include!("../test_gates.rs");

#[test]
fn compiled_test_gates_match_the_current_environment() {
    replay_test_gates::assert_gates_are_current(GATES, &|cfg| match cfg {
        "gate_hoi4_install" => cfg!(gate_hoi4_install),
        "gate_hoi4_user_dir" => cfg!(gate_hoi4_user_dir),
        other => panic!("gate {other} has no cfg!() mapping in this test"),
    });
}
