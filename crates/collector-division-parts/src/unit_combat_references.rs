
use engine_native::cunit::MapCombatReference;
use replay_data::{
    rows::PacketHour, rows::Recorded, rows::UnitCombatReference, rows::UnitCombatReferences,
};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

const NAMES: [&str; 6] = [
    "unit combat-reference prepared capacity refused",
    "unit combat-reference identity or registry refused",
    "unit combat-reference pointer or vector read refused",
    "unit combat-reference length or demand refused",
    "unit combat-reference projection or sample-hour refused",
    "unit combat-reference other guarded failure",
];
static REFUSALS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static DEMAND: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
pub fn observe_failure(reason: &'static str) {
    let index = if reason.contains("capacity") {
        0
    } else if reason.contains("identity") || reason.contains("registry") || reason.contains("class")
    {
        1
    } else if reason.contains("pointer") || reason.contains("vector") || reason.contains("header") {
        2
    } else if reason.contains("length") || reason.contains("demand") {
        3
    } else {
        5
    };
    REFUSALS[index].fetch_add(1, Relaxed);
}
pub fn demand(railway: bool, required: usize, capacity: usize) {
    let index = if railway { 2 } else { 0 };
    DEMAND[index].store(required, Relaxed);
    DEMAND[index + 1].store(capacity, Relaxed);
}
pub fn data() -> impl serde::Serialize {
    crate::status::status!({
        "refusals":crate::status::map(NAMES.into_iter().zip(REFUSALS.iter())
            .map(|(name,count)|(name,count.load(Relaxed))).filter(|(_,count)|*count!=0)),
        "division_required":DEMAND[0].load(Relaxed),"division_capacity":DEMAND[1].load(Relaxed),
        "railway_required":DEMAND[2].load(Relaxed),"railway_capacity":DEMAND[3].load(Relaxed),
        "scope":"complete ordered simulation combat identities; independent row suffix; writer/world/runtime acceptance held",
    })
}
fn valid(entries: &[MapCombatReference], count: u32) -> bool {
    entries.len() == count as usize
        && entries
            .iter()
            .all(|entry| entry.combat != 0 && entry.kind <= 2)
}
pub fn project(
    hour: Option<i32>,
    entries: Option<&[MapCombatReference]>,
    count: u32,
) -> Option<UnitCombatReferences> {
    let (Some(_), Some(entries)) = (hour, entries) else {
        return None;
    };
    if !valid(entries, count) {
        REFUSALS[4].fetch_add(1, Relaxed);
        return None;
    }
    match engine_allocation::collect_vec(entries.iter().map(|entry| UnitCombatReference {
        combat: entry.combat,
        kind: entry.kind,
    })) {
        Ok(entries) => Some(UnitCombatReferences {
            sample_hour: PacketHour::Host,
            entries,
        }),
        Err(error) => {
            crate::allocation::observe(&error);
            REFUSALS[4].fetch_add(1, Relaxed);
            None
        }
    }
}
pub fn matches(
    hour: Option<i32>,
    entries: Option<&[MapCombatReference]>,
    count: u32,
    previous: &Recorded<Option<UnitCombatReferences>>,
) -> bool {
    let (Some(_), Some(entries)) = (hour, entries) else {
        return *previous == Recorded::Value(None);
    };
    if !valid(entries, count) {
        return *previous == Recorded::Value(None);
    }
    let Recorded::Value(Some(previous)) = previous else {
        return false;
    };
    previous.sample_hour == PacketHour::Host
        && previous.entries.len() == entries.len()
        && entries
            .iter()
            .zip(&previous.entries)
            .all(|(a, b)| a.combat == b.combat && a.kind == b.kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_projection_preserves_order_repeats_full_bits_and_equal_next_hours_reuse() {
        let entries = [
            MapCombatReference {
                combat: u64::MAX,
                kind: 2,
            },
            MapCombatReference {
                combat: 0x8000_0001_0000_1b5a,
                kind: 0,
            },
            MapCombatReference {
                combat: u64::MAX,
                kind: 2,
            },
        ];
        let projected = project(Some(i32::MIN), Some(&entries), 3).unwrap();
        assert_eq!(projected.sample_hour, PacketHour::Host);
        assert_eq!(
            projected
                .entries
                .iter()
                .map(|entry| (entry.combat, entry.kind))
                .collect::<Vec<_>>(),
            entries.map(|entry| (entry.combat, entry.kind))
        );
        let previous = Recorded::Value(Some(projected));
        assert!(matches(Some(i32::MIN), Some(&entries), 3, &previous));
        assert!(matches(Some(i32::MIN + 1), Some(&entries), 3, &previous));
        assert!(!matches(None, Some(&entries), 3, &previous));
        assert!(!matches(
            Some(i32::MIN + 1),
            Some(&entries[..2]),
            2,
            &previous
        ));
    }

    #[test]
    fn authentic_empty_is_unchanged_next_hour_and_refusal_does_not_reuse_it() {
        let empty = project(Some(1), Some(&[]), 0).unwrap();
        assert!(empty.entries.is_empty());
        let previous = Recorded::Value(Some(empty));
        assert!(matches(Some(1), Some(&[]), 0, &previous));
        assert!(matches(Some(2), Some(&[]), 0, &previous));
        assert_eq!(project(Some(2), None, 0), None);
        assert_eq!(project(None, Some(&[]), 0), None);
        assert!(!matches(Some(2), None, 0, &previous));
        assert!(!matches(None, Some(&[]), 0, &Recorded::NotRecorded));
        assert!(matches(None, Some(&[]), 0, &Recorded::Value(None)));
    }

    #[test]
    fn malformed_count_null_handle_or_kind_refuses_only_the_suffix() {
        for entries in [
            [MapCombatReference { combat: 7, kind: 0 }],
            [MapCombatReference { combat: 0, kind: 0 }],
            [MapCombatReference { combat: 7, kind: 3 }],
        ] {
            assert_eq!(project(Some(5), Some(&entries), 2), None);
        }
        for entries in [
            [MapCombatReference { combat: 0, kind: 0 }],
            [MapCombatReference { combat: 7, kind: 3 }],
        ] {
            assert_eq!(project(Some(5), Some(&entries), 1), None);
            assert!(matches(Some(5), Some(&entries), 1, &Recorded::Value(None)));
            assert!(!matches(Some(5), Some(&entries), 1, &Recorded::NotRecorded));
        }
    }
}
