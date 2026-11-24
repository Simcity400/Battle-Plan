use super::*;
use replay_data::event_feed_rows::decisions::{DecisionChange, Operation};
fn access_raw(
    candidate: bool,
    observe: impl FnOnce(
        &mut Store,
        &engine_native_memory::memory::LiveMemory,
        usize,
        u64,
    ) -> Result<bool, &'static str>,
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
        match observe(store, &memory, owner.base, generation) {
            Ok(false) => return,
            Ok(true) => {}
            Err(reason) => {
                store.decisions.invalidate();
                store.reject(reason);
            }
        }
        script_demands(store);
    };
    let _ = if candidate {
        owner.observe_candidate(publish)
    } else {
        owner.observe(Role::DecisionWriter, publish)
    };
}
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
    access_raw(candidate, |store, memory, base, generation| {
        let world = native::provenance::World::new(memory, base)?;
        observe(store, &world, memory, base, generation)?;
        Ok(true)
    });
}
fn country(memory: &impl Memory, base: usize, status: usize) -> Result<usize, &'static str> {
    let expected = engine_sites::at(base, engine_decisions::cdecision_status::VTABLE_RVA)
        .ok_or("decision writer owner status differs")? as u64;
    let Some(country_addr) = status.checked_add(engine_decisions::cdecision_status::COUNTRY) else {
        if engine_native_memory::read::<u64>(memory, status) != Some(expected) {
            return Err("decision writer owner status differs");
        }
        return Err("decision writer owner unreadable");
    };
    let mut vtable_bytes = [0u8; 8];
    let mut country_bytes = [0u8; 8];
    let copied = memory.copy_scattered(
        [status, country_addr],
        &mut [&mut vtable_bytes[..], &mut country_bytes[..]],
    );
    if copied == 0 || u64::from_le_bytes(vtable_bytes) != expected {
        return Err("decision writer owner status differs");
    }
    if copied < 2 {
        return Err("decision writer owner unreadable");
    }
    Ok(u64::from_le_bytes(country_bytes) as usize)
}
struct StatusWorld<'a, M: Memory> {
    world: native::provenance::World<'a, M>,
    country: usize,
    owner: native::CountryIdentity,
    hour: i32,
    rebuild: Option<(i32, i32)>,
}

fn prefetch_status_world<'a, M: Memory>(
    memory: &'a M,
    base: usize,
    status: usize,
    need_rebuild: bool,
) -> Result<StatusWorld<'a, M>, &'static str> {
    if let (
        Some(singleton_addr),
        Some(status_vtable_expected),
        Some(country_vtable_expected),
        Some(country_addr),
    ) = (
        engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA),
        engine_sites::at(base, engine_decisions::cdecision_status::VTABLE_RVA),
        engine_sites::at(base, engine_native::ccountry::VTABLE_RVA),
        status.checked_add(engine_decisions::cdecision_status::COUNTRY),
    ) {
        let override_addr = if need_rebuild {
            engine_sites::at(base, engine_decisions::decision_runtime::ADMISSION_OVERRIDE_RVA)
        } else {
            Some(singleton_addr)
        };
        if let Some(override_addr) = override_addr {
            let mut state_bytes = [0_u8; 8];
            let mut status_vtable = [0_u8; 8];
            let mut country_bytes = [0_u8; 8];
            let mut override_byte = [0_u8; 1];
            let copied = memory.copy_scattered(
                [singleton_addr, status, country_addr, override_addr],
                &mut [
                    &mut state_bytes[..],
                    &mut status_vtable[..],
                    &mut country_bytes[..],
                    &mut override_byte[..],
                ],
            );
            if copied == 4 {
                let state = usize::try_from(u64::from_le_bytes(state_bytes))
                    .map_err(|_| "event feed pointer overflow")?;
                if state == 0 {
                    return Err("event feed game state is null");
                }
                if u64::from_le_bytes(status_vtable) != status_vtable_expected as u64 {
                    return Err("decision writer owner status differs");
                }
                let country_ptr = u64::from_le_bytes(country_bytes) as usize;
                if country_ptr == 0 {
                    return Err("event feed object identity mismatch");
                }
                if let (
                    Some(hour_addr),
                    Some(countries_addr),
                    Some(links_addr),
                    Some(id_addr),
                    Some(target_addr),
                    Some(seed_addr),
                ) = (
                    state.checked_add(engine_native::cgame_state::CURRENT_HOUR),
                    state.checked_add(engine_native::cgame_state::COUNTRIES),
                    state.checked_add(engine_native::cgame_state::COUNTRY_LINKS),
                    country_ptr.checked_add(engine_native::ccountry::ID),
                    country_ptr.checked_add(engine_native::ccountry::SURRENDER_TARGET),
                    if need_rebuild {
                        state.checked_add(engine_native::cgame_state::CAMPAIGN_SEED)
                    } else {
                        state.checked_add(engine_native::cgame_state::CURRENT_HOUR)
                    },
                ) {
                    let mut hour_bytes = [0_u8; 4];
                    let mut countries_bytes = [0_u8; 16];
                    let mut links_bytes = [0_u8; 16];
                    let mut country_vtable = [0_u8; 8];
                    let mut id_bytes = [0_u8; 4];
                    let mut target_bytes = [0_u8; 4];
                    let mut seed_bytes = [0_u8; 4];
                    let copied2 = memory.copy_scattered(
                        [
                            hour_addr,
                            countries_addr,
                            links_addr,
                            country_ptr,
                            id_addr,
                            target_addr,
                            seed_addr,
                        ],
                        &mut [
                            &mut hour_bytes[..],
                            &mut countries_bytes[..],
                            &mut links_bytes[..],
                            &mut country_vtable[..],
                            &mut id_bytes[..],
                            &mut target_bytes[..],
                            &mut seed_bytes[..],
                        ],
                    );
                    if copied2 == 7 {
                        if u64::from_le_bytes(country_vtable) != country_vtable_expected as u64 {
                            return Err("event feed object identity mismatch");
                        }
                        if let (Some(countries_arr), Some(links_arr)) = (
                            engine_native_memory::cpdx_array::Array::from_header(
                                countries_bytes,
                                8,
                            ),
                            engine_native_memory::cpdx_array::Array::from_header(links_bytes, 4),
                        ) {
                            let world = native::provenance::World::from_prefetched(
                                memory,
                                base,
                                state,
                                Some([countries_arr, links_arr]),
                            )?;
                            let reference = u32::from_le_bytes(id_bytes);
                            let instance = u32::from_le_bytes(target_bytes);
                            let (_, owner) = world.verify_country_identity(
                                country_ptr,
                                reference,
                                instance,
                            )?;
                            let hour = i32::from_le_bytes(hour_bytes);
                            let rebuild = need_rebuild.then(|| {
                                (
                                    i32::from_le_bytes(seed_bytes),
                                    i32::from(override_byte[0] != 0),
                                )
                            });
                            return Ok(StatusWorld {
                                world,
                                country: country_ptr,
                                owner,
                                hour,
                                rebuild,
                            });
                        }
                    }
                }
            }
        }
    }
    let world = native::provenance::World::new(memory, base)?;
    let country_ptr = country(memory, base, status)?;
    let owner = world.country_object(country_ptr)?;
    let rebuild = if need_rebuild {
        let days = engine_native_memory::read::<i32>(
            memory,
            world.state_pointer() + engine_native::cgame_state::CAMPAIGN_SEED,
        )
        .ok_or("decision rebuild campaign seed unreadable")?;
        let lifecycle = i32::from(
            engine_sites::at(base, engine_decisions::decision_runtime::ADMISSION_OVERRIDE_RVA)
                .and_then(|at| engine_native_memory::read::<u8>(memory, at))
                .ok_or("decision rebuild admission override unreadable")?
                != 0,
        );
        Some((days, lifecycle))
    } else {
        None
    };
    let hour = world.hour()?;
    Ok(StatusWorld {
        world,
        country: country_ptr,
        owner,
        hour,
        rebuild,
    })
}

pub fn instance(status: usize, pointer: usize, container: u8, operation: Operation, flags: u8) {
    access_raw(false, |store, memory, base, generation| {
        let sw = prefetch_status_world(memory, base, status, false)?;
        store.decision_selected_source_resolved(
            &sw.world,
            memory,
            base,
            generation,
            sw.country,
            sw.owner,
            || Ok(sw.hour),
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
            None,
        )?;
        Ok(true)
    });
}
pub fn write(pointer: usize, state: Option<i32>, days: Option<i32>) {
    access_raw(false, |store, memory, base, generation| {
        let Some(slot) = store.decisions.find(pointer) else {
            return Ok(false);
        };
        let world = native::provenance::World::new(memory, base)?;
        store.decision_selected(
            &world,
            memory,
            base,
            generation,
            slot.owner,
            pointer,
            DecisionChange::new(Operation::Update, slot.container),
            false,
            (state, days),
        )?;
        Ok(true)
    });
}
pub fn cut(pointer: usize) {
    access_raw(false, |store, memory, base, generation| {
        let Some(slot) = store.decisions.find(pointer) else {
            return Ok(false);
        };
        let world = native::provenance::World::new(memory, base)?;
        store.decision_selected(
            &world,
            memory,
            base,
            generation,
            slot.owner,
            pointer,
            DecisionChange::new(Operation::PassCut, slot.container),
            false,
            (None, None),
        )?;
        Ok(true)
    });
}
pub fn retire(pointer: usize, swap: bool, visited: bool) {
    access_raw(false, |store, memory, base, generation| {
        if store.decisions.find(pointer).is_none() && store.decisions.uncertified(pointer).is_none()
        {
            return Ok(false);
        }
        let world = native::provenance::World::new(memory, base)?;
        store.decision_retire(&world, memory, base, generation, pointer, swap, visited)?;
        Ok(true)
    });
}
pub fn marker(status: usize, operation: Operation, container: u8, ordinal: u32, flags: u8) {
    if operation == Operation::Rebuild && container != 3 {
        return;
    }
    access_raw(false, |store, memory, base, generation| {
        let sw = prefetch_status_world(memory, base, status, operation == Operation::Rebuild)?;
        store.decision_marker_resolved(
            memory,
            base,
            generation,
            sw.country,
            sw.owner,
            || Ok(sw.hour),
            sw.rebuild,
            DecisionChange {
                operation,
                container,
                ordinal,
                flags,
                political_power: None,
            },
        )?;
        Ok(true)
    });
}
pub fn random(status: usize, definition: usize, target: u32) {
    access_raw(false, |store, memory, base, generation| {
        let sw = prefetch_status_world(memory, base, status, false)?;
        store.decision_random_resolved(
            memory,
            base,
            generation,
            sw.owner,
            || Ok(sw.hour),
            definition,
            target,
            None,
        )?;
        Ok(true)
    });
}

pub fn taken_erase(status: usize, definition: usize, ordinal: Option<u32>, swap: bool) {
    access_raw(false, |store, memory, base, generation| {
        let sw = prefetch_status_world(memory, base, status, false)?;
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
        value.actor = sw.owner;
        value.token = engine_native_memory::msvc_string::Descriptor::read(
            memory,
            definition + engine_decisions::cdecision::TOKEN,
        )?;
        store.script_known(memory, base, generation, sw.hour, value)?;
        Ok(true)
    });
}
pub fn candidates(
    status: usize,
    kind: u8,
    items: &[(usize, u32, u32)],
) {
    if items.is_empty() {
        return;
    }
    access_raw(true, |store, memory, base, generation| {
        let sw = prefetch_status_world(memory, base, status, false)?;
        for &(definition, target, root) in items {
            let token = engine_native_memory::msvc_string::Descriptor::read(
                memory,
                definition + engine_decisions::cdecision::TOKEN,
            )?;
            store.decision_key_resolved(
                &sw.world,
                memory,
                base,
                generation,
                sw.owner,
                sw.hour,
                DecisionChange {
                    operation: Operation::Candidate,
                    container: kind,
                    flags: 0,
                    ordinal: target,
                    political_power: None,
                },
                token,
                Some(root),
            )?;
        }
        Ok(true)
    });
}
pub fn candidate(status: usize, kind: u8, definition: usize, target: u32, root: u32) {
    candidates(status, kind, &[(definition, target, root)]);
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
    access_raw(false, |store, memory, base, generation| {
        let Some(slot) = store.decisions.find(pointer) else {
            return Ok(false);
        };
        let world = native::provenance::World::new(memory, base)?;
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
            &world,
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
        )?;
        Ok(true)
    });
}

pub fn activated(status: usize, pointer: usize, container: u8, state: i32, days: i32) -> bool {
    let mut recorded = false;
    access_raw(false, |store, memory, base, generation| {
        let sw = prefetch_status_world(memory, base, status, false)?;
        store.decision_selected_source_resolved(
            &sw.world,
            memory,
            base,
            generation,
            sw.country,
            sw.owner,
            || Ok(sw.hour),
            pointer,
            DecisionChange::new(Operation::Store, container),
            false,
            (Some(state), Some(days)),
            Some(replay_data::event_feed_rows::EventFeedSource::DecisionActivated),
        )?;
        recorded = true;
        Ok(true)
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
    access_raw(false, |store, memory, base, generation| {
        if store.decisions.find(pointer).is_none() && store.decisions.uncertified(pointer).is_none()
        {
            return Ok(false);
        }
        let world = native::provenance::World::new(memory, base)?;
        recorded = store.decision_result(
            &world,
            memory,
            base,
            generation,
            pointer,
            (state, days),
            source,
            "decision result reached an uncertified owner",
        )?;
        Ok(true)
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
