use super::*;
use replay_data::event_feed_rows::decisions::{DecisionChange, Operation};
fn access(
    candidate: bool,
    observe: impl FnOnce(
        &mut Store,
        &native::provenance::World<'_, engine_native_memory::memory::LiveMemory>,
        &engine_native_memory::memory::LiveMemory,
        usize,
        u64,
    ) -> Result<(), &'static str>,
) {
    let Some(owner) = (unsafe { SHARED.load(Ordering::SeqCst).as_ref() }) else {
        return;
    };
    let publish = |store: &mut Store| {
        let Some(generation) = observed(owner, store) else {
            return;
        };
        if !store.decisions.matches(generation) {
            return;
        }
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let result = native::provenance::World::new(&memory, owner.base)
            .and_then(|world| observe(store, &world, &memory, owner.base, generation));
        if let Err(reason) = result {
            store.decisions.invalidate();
            store.reject(reason);
        }
        script_demands(store);
    };
    let _ = if candidate {
        owner.observe_candidate(publish)
    } else {
        owner.observe(Role::DecisionWriter, publish)
    };
}
fn country(memory: &impl Memory, base: usize, status: usize) -> Result<usize, &'static str> {
    if engine_sites::at(base, engine_decisions::cdecision_status::VTABLE_RVA)
        .is_none_or(|vtable| engine_native_memory::read::<u64>(memory, status) != Some(vtable as u64))
    {
        return Err("decision writer owner status differs");
    }
    status
        .checked_add(engine_decisions::cdecision_status::COUNTRY)
        .and_then(|p| engine_native_memory::read::<u64>(memory, p))
        .map(|p| p as usize)
        .ok_or("decision writer owner unreadable")
}
pub fn instance(status: usize, pointer: usize, container: u8, operation: Operation, flags: u8) {
    access(false, |store, world, memory, base, generation| {
        store.decision_selected(
            world,
            memory,
            base,
            generation,
            country(memory, base, status)?,
            pointer,
            DecisionChange {
                operation,
                container,
                flags,
                ordinal: 0,
                political_power: None,
            },
            false,
            (None, None),
        )
    });
}
pub fn write(pointer: usize, state: Option<i32>, days: Option<i32>) {
    access(false, |store, world, memory, base, generation| {
        let Some(slot) = store.decisions.find(pointer) else {
            return Ok(());
        };
        store.decision_selected(
            world,
            memory,
            base,
            generation,
            slot.owner,
            pointer,
            DecisionChange::new(Operation::Update, slot.container),
            false,
            (state, days),
        )
    });
}
pub fn cut(pointer: usize) {
    access(false, |store, world, memory, base, generation| {
        let Some(slot) = store.decisions.find(pointer) else {
            return Ok(());
        };
        store.decision_selected(
            world,
            memory,
            base,
            generation,
            slot.owner,
            pointer,
            DecisionChange::new(Operation::PassCut, slot.container),
            false,
            (None, None),
        )
    });
}
pub fn retire(pointer: usize, swap: bool, visited: bool) {
    access(false, |store, world, memory, base, generation| {
        store.decision_retire(world, memory, base, generation, pointer, swap, visited)
    });
}
pub fn marker(status: usize, operation: Operation, container: u8, ordinal: u32, flags: u8) {
    if operation == Operation::Rebuild && container != 3 {
        return;
    }
    access(false, |store, world, memory, base, generation| {
        store.decision_marker(
            world,
            memory,
            base,
            generation,
            country(memory, base, status)?,
            DecisionChange {
                operation,
                container,
                ordinal,
                flags,
                political_power: None,
            },
        )
    });
}
pub fn random(status: usize, definition: usize, target: u32) {
    access(false, |store, world, memory, base, generation| {
        store.decision_random(
            world,
            memory,
            base,
            generation,
            country(memory, base, status)?,
            definition,
            target,
            None,
        )
    });
}

pub fn taken_erase(status: usize, definition: usize, ordinal: Option<u32>, swap: bool) {
    access(false, |store, world, memory, base, generation| {
        let mut flags = if swap {
            replay_data::event_feed_rows::decisions::SWAP
        } else {
            0
        };
        flags |= if ordinal.is_some() {
            replay_data::event_feed_rows::decisions::ORDINAL
        } else {
            replay_data::event_feed_rows::decisions::ALL
        };
        let change = DecisionChange {
            operation: Operation::Erase,
            container: 0,
            flags,
            ordinal: ordinal.unwrap_or(0),
            political_power: None,
        };
        let mut value = crate::event_feed_history::registry::marker_for(
            memory,
            replay_data::event_feed_rows::EventFeedScriptChange::Decision(change),
            Some(0),
        );
        value.actor = world.country_object(country(memory, base, status)?)?;
        value.token = engine_native_memory::msvc_string::Descriptor::read(
            memory,
            definition + engine_decisions::cdecision::TOKEN,
        )?;
        store.script_known(memory, base, generation, world.hour()?, value)
    });
}
pub fn candidate(status: usize, kind: u8, definition: usize, target: u32, root: u32) {
    access(true, |store, world, memory, base, generation| {
        store.decision_key(
            world,
            memory,
            base,
            generation,
            country(memory, base, status)?,
            DecisionChange {
                operation: Operation::Candidate,
                container: kind,
                flags: 0,
                ordinal: target,
                political_power: None,
            },
            engine_native_memory::msvc_string::Descriptor::read(
                memory,
                definition + engine_decisions::cdecision::TOKEN,
            )?,
            Some(root),
        )
    });
}
pub fn ignored(country: usize, token: usize, ignored: bool) {
    access(false, |store, world, memory, base, generation| {
        store.decision_key(
            world,
            memory,
            base,
            generation,
            country,
            DecisionChange {
                operation: Operation::Ignore,
                container: 0,
                flags: if ignored {
                    replay_data::event_feed_rows::decisions::IGNORED
                } else {
                    0
                },
                ordinal: 0,
                political_power: None,
            },
            engine_native_memory::msvc_string::Descriptor::read(memory, token)?,
            None,
        )
    });
}
pub(crate) fn committed_flags(
    ignored: Option<(bool, bool)>,
    removed: Option<bool>,
) -> Option<(Option<bool>, bool)> {
    if ignored.is_some_and(|(was, now)| was == now) || removed == Some(true) {
        return None;
    }
    Some((ignored.map(|(_, now)| now), removed.is_some()))
}
pub fn flags_committed(pointer: usize, ignored: Option<(bool, bool)>, removed: Option<bool>) {
    let Some((ignored, removed)) = committed_flags(ignored, removed) else {
        return;
    };
    access(false, |store, world, memory, base, generation| {
        let Some(slot) = store.decisions.find(pointer) else {
            return Ok(());
        };
        let flags = if ignored == Some(true) {
            replay_data::event_feed_rows::decisions::IGNORED
        } else {
            0
        } | if removed {
            replay_data::event_feed_rows::decisions::REMOVE_REQUESTED
        } else {
            0
        };
        store.decision_selected(
            world,
            memory,
            base,
            generation,
            slot.owner,
            pointer,
            DecisionChange {
                operation: if ignored.is_some() {
                    Operation::Ignore
                } else {
                    Operation::Update
                },
                container: slot.container,
                flags,
                ordinal: 0,
                political_power: None,
            },
            false,
            (None, None),
        )
    });
}

pub fn activated(status: usize, pointer: usize, container: u8, state: i32, days: i32) -> bool {
    let mut recorded = false;
    access(false, |store, world, memory, base, generation| {
        store.decision_selected_source(
            world,
            memory,
            base,
            generation,
            country(memory, base, status)?,
            pointer,
            DecisionChange::new(Operation::Store, container),
            false,
            (Some(state), Some(days)),
            Some(replay_data::event_feed_rows::EventFeedSource::DecisionActivated),
        )?;
        recorded = true;
        Ok(())
    });
    recorded
}

pub fn write_result(
    pointer: usize,
    state: Option<i32>,
    days: Option<i32>,
    source: replay_data::event_feed_rows::EventFeedSource,
) -> bool {
    let mut recorded = false;
    access(false, |store, world, memory, base, generation| {
        recorded = store.decision_result(
            world,
            memory,
            base,
            generation,
            pointer,
            (state, days),
            source,
            "decision result reached an uncertified owner",
        )?;
        Ok(())
    });
    recorded
}

pub fn political_balance(pointer: usize, reference: u32, instance: u32, before: i64, after: i64) {
    if SHARED.load(Ordering::SeqCst).is_null() {
        return;
    }
    let mut flight = crate::script_event_observer::Flight::enter();
    if let Some(owner) = unsafe { SHARED.load(Ordering::SeqCst).as_ref() } {
        let _ = owner.observe(Role::PoliticalBalance, |store| {
            let Some(generation) = observed(owner, store) else {
                return;
            };
            if store.decisions.matches(generation) {
                store.decisions.power_update(
                    pointer,
                    reference,
                    instance,
                    before,
                    after,
                    crate::event_feed_history::decisions::POWER_EPOCH.load(Ordering::Acquire),
                );
            }
        });
    }
    flight.returned();
}

pub fn political_balance_lost() {
    crate::event_feed_history::decisions::POWER_EPOCH.fetch_add(1, Ordering::AcqRel);
}
fn reactivation_days(
    container: u8,
    pointer: usize,
    paired: (usize, i32),
) -> Result<Option<i32>, &'static str> {
    match container {
        replay_data::event_feed_rows::decisions::TARGETED => Ok(None),
        replay_data::event_feed_rows::decisions::TARGETED_TIMED if paired.0 == pointer => {
            Ok(Some(paired.1))
        }
        _ => Err("timed targeted reactivation duration certificate differs"),
    }
}
pub fn reactivated(pointer: usize, state: i32, paired: (usize, i32)) {
    access(false, |store, world, memory, base, generation| {
        const UNATTRIBUTED: &str =
            "targeted reactivation has no owner baseline/pointer certificate";
        let slot = store
            .decisions
            .find(pointer)
            .or_else(|| store.decisions.uncertified(pointer))
            .ok_or(UNATTRIBUTED)?;
        let days = reactivation_days(slot.container, pointer, paired)?;
        store
            .decision_result(
                world,
                memory,
                base,
                generation,
                pointer,
                (Some(state), days),
                replay_data::event_feed_rows::EventFeedSource::DecisionActivated,
                "targeted reactivation reached an uncertified owner",
            )?
            .then_some(())
            .ok_or(UNATTRIBUTED)
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::event_feed_rows::decisions::{TARGETED, TARGETED_TIMED};
    #[test]
    fn ordinary_reactivation_has_no_duration_setter_but_timed_requires_matching_pair() {
        assert_eq!(reactivation_days(TARGETED, 17, (0, 0)).unwrap(), None);
        assert_eq!(reactivation_days(TARGETED, 17, (19, 99)).unwrap(), None);
        assert_eq!(
            reactivation_days(TARGETED_TIMED, 17, (17, 15)).unwrap(),
            Some(15)
        );
        assert!(reactivation_days(TARGETED_TIMED, 17, (0, 0)).is_err());
        assert!(reactivation_days(TARGETED_TIMED, 17, (19, 15)).is_err());
    }
}
