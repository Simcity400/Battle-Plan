use super::*;

/// A part changes when what it holds changes, or a countdown rises, appears or goes;
/// countdowns that only ran down leave it as it was.
#[test]
fn only_countdowns_running_down_leave_a_part_unchanged() {
    let mut sections = Sections::default();
    let mut see = |slot, available, value: &str, countdowns: &[i32]| {
        sections
            .see(
                slot,
                available,
                |hasher| serde_json::to_writer(hasher, value),
                countdowns.iter().copied(),
            )
            .unwrap()
    };
    let timer = Slot::Delayed(5, 7);
    assert!(see(timer, true, "timer 17", &[500, 3]));
    assert!(!see(timer, true, "timer 17", &[500, 3]));
    assert!(!see(timer, true, "timer 17", &[499, 0]));
    assert!(see(timer, true, "timer 17", &[500, 0]));
    assert!(see(timer, true, "timer 17", &[500]));
    assert!(see(timer, true, "timer 17", &[500, 9]));
    assert!(see(timer, true, "timer 18", &[500, 9]));
    // A part first seen unavailable is no change; becoming available, or lost again, is.
    assert!(!see(Slot::Fired, false, "unavailable", &[]));
    assert!(see(Slot::Fired, true, "fired", &[]));
    assert!(see(Slot::Fired, false, "unavailable", &[]));
}
