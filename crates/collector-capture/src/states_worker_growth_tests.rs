use crate::sampling;
use collector_states::acquisition_states::{
    Inputs, Report,
    tests::{FIRST, STATE},
};
use engine_native::{cstate, walk::Outcome};

#[test]
fn worker_failure_before_supplement_keeps_visited_demand_for_core_growth() {
    let mut frame =
    // SAFETY: The caller owns the session or isolated fixture state required by this constructor.
    unsafe { sampling::Captured::new() };
    frame.states = vec![cstate::Row::default(); 1];
    frame.state_inputs = Inputs::with_capacity(1).unwrap();
    let mut request = frame.take_states_request(0, STATE);
    request.visitor.visit(cstate::Source {
        pointer: FIRST,
        id: 1,
    });
    request.visitor.finish(Outcome::Truncated {
        len: 1,
        unread_rows: 4096,
    });
    assert_eq!(request.inputs.required(), 0);
    frame.restore_states_request(request, Report::default(), Some("states worker panicked"));
    assert_eq!(
        frame.state_inputs.occupation.outcome(),
        Outcome::Unavailable {
            reason: "states worker panicked",
        }
    );
    assert_eq!(frame.state_inputs.required(), 4097);
    assert_eq!(frame.state_inputs.fraction(1), None);
    let visitor = frame.extended.take_state_visitor();
    assert!(visitor.completed_sources().0.is_empty());
    frame.extended.restore_state_visitor(visitor);
    let mut capacities = sampling::INITIAL_CAPACITIES;
    capacities[3] = 1;
    frame.grow(&mut capacities);
    assert!(capacities[3] >= 4097);
    assert_eq!(frame.states.len(), capacities[3]);
    assert_eq!(frame.state_inputs.rows.len(), capacities[3]);
}
