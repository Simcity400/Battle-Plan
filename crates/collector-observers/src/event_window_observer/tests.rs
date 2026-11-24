use super::*;

fn trace(apply: impl FnOnce(&mut Trace)) -> Trace {
    let mut trace = Trace {
        frame: 1,
        ..Trace::default()
    };
    apply(&mut trace);
    trace
}

#[test]
fn each_gate_path_resolves_to_the_branch_the_routine_left_by() {
    let passed_to_open = |trace: &mut Trace| {
        trace.past_ai = true;
        trace.past_viewer = true;
        trace.new_instance = true;
        trace.open_test = true;
    };
    let cases = [
        (trace(|_| {}), false, Ok(WindowVerdict::AiControlled)),
        (trace(|_| {}), true, Ok(WindowVerdict::HiddenOrAiControlled)),
        (trace(|t| t.past_ai = true), false, Ok(WindowVerdict::NotViewingCountry)),
        (
            trace(|t| {
                passed_to_open(t);
                t.sender_excluded = true;
            }),
            false,
            Ok(WindowVerdict::SenderExcluded),
        ),
        (
            trace(|t| {
                passed_to_open(t);
                t.player_restricted = Some(WindowVerdict::PlayerRestrictedOptionTrigger);
            }),
            false,
            Ok(WindowVerdict::PlayerRestrictedOptionTrigger),
        ),
        (
            trace(|t| {
                t.past_ai = true;
                t.past_viewer = true;
            }),
            false,
            Ok(WindowVerdict::ExistingInstance),
        ),
        (trace(passed_to_open), false, Ok(WindowVerdict::AlreadyOpen)),
        (
            trace(|t| {
                passed_to_open(t);
                t.requested = true;
            }),
            false,
            Ok(WindowVerdict::Requested),
        ),
        (
            trace(|t| {
                t.past_ai = true;
                t.past_viewer = true;
                t.new_instance = true;
            }),
            false,
            Err(SECTION_INCOMPLETE),
        ),
        (
            trace(|t| {
                passed_to_open(t);
                t.requested = true;
                t.refused = Some("unreadable");
            }),
            false,
            Err("unreadable"),
        ),
    ];
    for (trace, hidden, expected) in cases {
        assert_eq!(trace.resolve(hidden), expected, "{trace:?}");
    }
}

#[test]
fn a_frame_mark_never_inherits_another_frames_trace() {
    TRACE.with(|cell| cell.set(Trace::default()));
    mark(10, |trace| trace.past_ai = true);
    mark(20, |trace| trace.past_viewer = true);
    let held = TRACE.with(Cell::get);
    assert_eq!(held.frame, 20);
    assert!(!held.past_ai, "a nested firing starts its own trace");
    assert!(held.past_viewer);
}

#[test]
fn a_restricted_player_sees_only_major_events_whose_options_have_no_trigger() {
    let mut bytes = vec![0_u8; 0x600];
    let event = 0x1000_usize;
    let options = event + 0x500;
    let option = event + 0x100;
    bytes[cevent::OPTIONS..cevent::OPTIONS + 8].copy_from_slice(&(options as u64).to_le_bytes());
    bytes[cevent::OPTION_COUNT..cevent::OPTION_COUNT + 4].copy_from_slice(&1_i32.to_le_bytes());
    bytes[0x500..0x508].copy_from_slice(&(option as u64).to_le_bytes());
    let read = |bytes: &[u8]| {
        player_restriction(
            &engine_native_memory::memory::SliceMemory { base: event, bytes },
            event,
        )
    };
    assert_eq!(read(&bytes), Ok(Some(WindowVerdict::PlayerRestrictedNotMajor)));
    bytes[cevent::MAJOR] = 1;
    assert_eq!(read(&bytes), Ok(None));
    bytes[0x100 + cevent::OPTION_TRIGGER_COUNT] = 2;
    assert_eq!(read(&bytes), Ok(Some(WindowVerdict::PlayerRestrictedOptionTrigger)));
    assert!(read(&bytes[..0x400]).is_err(), "an unreadable option refuses rather than guesses");
}
