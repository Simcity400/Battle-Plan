use super::*;
use replay_data::event_feed_rows::EventFeedEventIdentity;

const OWNER: EventFeedCountry = EventFeedCountry {
    reference: 3,
    id: 3,
    instance: 0,
};

fn timer(instance: u32, token: &str, remaining: i32) -> DelayedEvent {
    DelayedEvent {
        instance,
        definition: ScriptIdentity {
            native_event: EventFeedEventIdentity { kind: 1, id: 9 },
            token: token.as_bytes().to_vec(),
        },
        originator: OWNER,
        remaining,
        scopes: vec![EventFeedScope::default()],
    }
}

/// `timers` seen as `slot` at `version`: whether it changed.
fn see(
    sections: &mut Sections,
    slot: Slot,
    version: Option<u64>,
    timers: Result<&[DelayedEvent], &'static str>,
) -> bool {
    let keep = || Arc::new(Kept::new(Part::Delayed(OWNER, timers.map(<[_]>::to_vec))));
    let countdowns = timers
        .unwrap_or_default()
        .iter()
        .map(|timer| timer.remaining);
    sections
        .see(
            slot,
            version,
            timers.is_ok(),
            View::Delayed(OWNER, timers),
            countdowns,
            keep,
        )
        .unwrap()
        .is_some()
}

/// A part changes when what it holds changes, or a countdown rises, appears or goes;
/// countdowns that only ran down leave it as it was.
#[test]
fn only_countdowns_running_down_leave_a_part_unchanged() {
    let mut sections = Sections::default();
    let slot = Slot::Delayed(5, 7);
    let mut seen = |timers: &[DelayedEvent]| see(&mut sections, slot, None, Ok(timers));
    assert!(seen(&[timer(1, "a", 500), timer(2, "a", 3)]));
    assert!(!seen(&[timer(1, "a", 500), timer(2, "a", 3)]));
    assert!(!seen(&[timer(1, "a", 499), timer(2, "a", 0)]));
    assert!(seen(&[timer(1, "a", 500), timer(2, "a", 0)]));
    assert!(seen(&[timer(1, "a", 500)]));
    assert!(seen(&[timer(1, "a", 500), timer(2, "a", 9)]));
    assert!(seen(&[timer(1, "b", 500), timer(2, "a", 9)]));
    // A part first seen unavailable is no change; becoming available, or lost again, is.
    let other = Slot::Delayed(6, 7);
    assert!(!see(&mut sections, other, None, Err("unavailable")));
    assert!(see(&mut sections, other, None, Ok(&[][..])));
    assert!(see(&mut sections, other, None, Err("unavailable")));
}

/// A part seen again at the version and availability it was compared at is not compared
/// again, and its countdowns still are; at a new version it is compared with the copy kept.
#[test]
fn a_part_at_its_compared_version_is_not_compared_again() {
    let mut sections = Sections::default();
    let slot = Slot::Delayed(5, 7);
    let held = [timer(1, "a", 9)];
    assert!(see(&mut sections, slot, Some(3), Ok(&held[..])));
    // What the version vouches for is not read.
    assert!(!see(
        &mut sections,
        slot,
        Some(3),
        Ok(&[timer(1, "b", 8)][..])
    ));
    assert!(see(
        &mut sections,
        slot,
        Some(3),
        Ok(&[timer(1, "b", 9)][..])
    ));
    assert!(!see(&mut sections, slot, Some(4), Ok(&held[..])));
    assert!(see(&mut sections, slot, Some(4), Err("unavailable")));
    assert!(see(&mut sections, slot, Some(5), Ok(&held[..])));
}

/// A part changing and changing back, at versions that moved, is a change and then another.
#[test]
fn a_part_that_changes_back_is_seen_to_change_back() {
    let mut sections = Sections::default();
    let slot = Slot::Delayed(5, 7);
    let held = [timer(1, "a", 9)];
    let moved = [timer(1, "a", 9), timer(2, "b", 9)];
    assert!(see(&mut sections, slot, Some(1), Ok(&held[..])));
    assert!(see(&mut sections, slot, Some(2), Ok(&moved[..])));
    assert!(see(&mut sections, slot, Some(3), Ok(&held[..])));
    assert!(!see(&mut sections, slot, Some(4), Ok(&held[..])));
}

/// A saved part keeps the digest a digest of all it held takes; restored, it is compared by
/// that digest once, then by the copy it keeps, and saved again as it was.
#[test]
fn a_saved_part_is_compared_by_its_digest_and_saved_as_it_was() {
    let mut sections = Sections::default();
    let slot = Slot::Delayed(5, 7);
    let held = [timer(1, "a", 9)];
    let digested = digests();
    assert!(see(&mut sections, slot, Some(1), Ok(&held[..])));
    assert_eq!(digests(), digested);
    let saved = serde_json::to_vec(&sections).unwrap();
    let expected = digest(|hasher| View::Delayed(OWNER, Ok(&held[..])).feed(hasher)).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(value[0][1]["digest"], serde_json::json!(expected));
    let mut restored: Sections = serde_json::from_slice(&saved).unwrap();
    let digested = digests();
    assert!(!see(&mut restored, slot, Some(1), Ok(&held[..])));
    assert_eq!(digests(), digested + 1);
    assert!(!see(&mut restored, slot, Some(2), Ok(&held[..])));
    assert_eq!(serde_json::to_vec(&restored).unwrap(), saved);
    assert_eq!(digests(), digested + 1);
    let mut restored: Sections = serde_json::from_slice(&saved).unwrap();
    assert!(see(
        &mut restored,
        slot,
        Some(1),
        Ok(&[timer(1, "b", 9)][..])
    ));
}
