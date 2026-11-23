use super::*;
use crate::event_feed_history::recovery::PreparedOwner;
use engine_decisions::{cdecision, cdecision_cooldown, cdecision_status, decision_runtime};

fn populated_cohort(countries: usize) -> (WarImage, Vec<engine_native_memory::country_identity::Country>) {
    let mut image = owners_image(1);
    let roster = 0x2000000;
    let links = 0x2100000;
    let definition = 0x2200000;
    let text = 0x2210000;
    image.array(GAME + cgame_state::COUNTRIES, roster, countries as u32 + 1);
    image.array(
        GAME + cgame_state::COUNTRY_LINKS,
        links,
        countries as u32 + 1,
    );
    image.q(roster, 0x10000);
    image.d(links, 0);
    image.q(definition, BASE + cdecision::VTABLE_RVA.0);
    image.bytes(definition + decision_runtime::PERMANENT, &[1]);
    let token = b"synthetic_definition_with_owned_worker_side_token_payload";
    image.bytes(text, token);
    image.bytes(text + token.len(), &[0]);
    let mut header = [0; 32];
    header[..8].copy_from_slice(&(text as u64).to_le_bytes());
    header[16..24].copy_from_slice(&(token.len() as u64).to_le_bytes());
    header[24..32].copy_from_slice(&(token.len() as u64).to_le_bytes());
    image.bytes(definition + cdecision::TOKEN, &header);
    let mut owners = Vec::with_capacity(countries);
    for slot in 0..countries {
        let id = slot as u32 + 1;
        let pointer = 0x3000000 + slot * 0x4000;
        let status = pointer + 0x2000;
        image.q(roster + id as usize * 8, pointer);
        image.d(links + id as usize * 4, id);
        image.q(pointer, BASE + ccountry::VTABLE_RVA.0);
        image.d(pointer + ccountry::ID, id);
        image.d(pointer + ccountry::SURRENDER_TARGET, id + 1000);
        image.decisions(pointer, status);
        image.array(pointer + ccountry::DELAYED_EVENTS, 0, 0);
        image.array(
            status + cdecision_status::DECISIONS_TAKEN,
            pointer + 0x2600,
            8,
        );
        for n in 0..8 {
            image.q(pointer + 0x2600 + n * 8, definition);
        }
        image.array(status + cdecision_status::RE_ENABLE, pointer + 0x2640, 2);
        for n in 0..2 {
            let member = pointer + 0x3000 + n * 0x40;
            image.q(pointer + 0x2640 + n * 8, member);
            image.q(member, BASE + cdecision_cooldown::VTABLE_RVA.0);
            image.q(member + cdecision_cooldown::DEFINITION, definition);
            image.d(
                member + cdecision_cooldown::DAYS,
                (-7_i32 - n as i32) as u32,
            );
        }
        if slot < 258 {
            let item = pointer + 0x2800;
            match slot % 3 {
                0 => {
                    image.array(status + cdecision_status::RANDOM_ITEMS, item, 1);
                    image.q(item + cdecision_status::RANDOM_ITEM_DEFINITION, definition);
                    image.d(item + cdecision_status::RANDOM_ITEM_TARGET, 1);
                    image.d(item + cdecision_status::RANDOM_ITEM_SEED, 17);
                }
                1 => {
                    image.array(status + decision_runtime::CANDIDATE_HEADERS[0], item, 1);
                    image.q(item + decision_runtime::CANDIDATE_DEFINITION, definition);
                    image.d(item, 1);
                    image.d(item + decision_runtime::CANDIDATE_ROOT, 1);
                }
                _ => {
                    image.array(pointer + decision_runtime::IGNORED_TOKENS, item, 1);
                    image.bytes(item, &header);
                }
            }
        }
        if slot < 3 {
            let timer = pointer + 0x2b00;
            image.array(pointer + ccountry::DELAYED_EVENTS, pointer + 0x2a00, 1);
            image.q(pointer + 0x2a00, timer);
            delayed_timer(&mut image, timer, -9);
        }
        owners.push(engine_native_memory::country_identity::Country {
            pointer,
            canonical_slot: id,
            id,
        });
    }
    (image, owners)
}

fn populated_recovery(countries: usize) {
    let _globals = crate::event_feed_observer::test_globals();
    collector_primitives::country_slots::note_slots(countries + 1);
    let (image, owners) = populated_cohort(countries);
    let mut store = census_store(4096);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    let mut all_labels = std::collections::BTreeSet::new();
    for (slot, country) in owners.iter().copied().enumerate() {
        let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
        assert_eq!(work, [true; 2]);
        let before = store.entries.len();
        let emissions = 13 + usize::from(slot < 258) + 2 * usize::from(slot < 3);
        for registry in [OwnerRegistry::Decisions, OwnerRegistry::Delayed] {
            let mut selected = [false; 2];
            selected[registry as usize] = true;
            let mut plan =
                PreparedOwner::read(&image, BASE, 7, GAME, 1, country, selected).unwrap();
            let preceding = store.entries.len();
            let demand = store.prepare_owner_demand(&plan);
            assert!(demand[0] > preceding);
            crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
            crate::test_support::with_allocations_denied(|| {
                store
                    .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
                    .unwrap();
            });
            assert!(store.entry_demand > store.entries.len());
        }
        assert_eq!(store.entries.len(), before + emissions);
        assert_eq!(store.counts.rejected, 0);
        assert!(store.failure.is_none());
        for entry in &store.entries[before..] {
            let script = entry.event.script.unwrap();
            if matches!(script.change, Some(EventFeedScriptChange::Decision(change)) if change.operation == replay_data::event_feed_rows::decisions::Operation::Store)
            {
                assert!(all_labels.insert(script.instance.unwrap()));
            }
        }
    }
    store.finish_recovery(7, epoch, true);
    assert!(!crate::event_feed_observer::recovery_pending());
    let row = sealed_row(&mut store, 1);
    let expected = 6 + 13 * countries + countries.min(258) + 2 * countries.min(3);
    assert_eq!(row.row().events.len(), expected);
    if countries == 439 {
        assert_eq!(expected, 5977);
    }
    assert!(expected > 4096);
    assert_eq!(row.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(certifying_markers(row.row()), (countries, countries));
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(row.row()).unwrap();
    for (slot, country) in owners.iter().enumerate() {
        let owner = EventFeedCountry {
            reference: country.id,
            id: country.id,
            instance: country.id + 1000,
        };
        let decisions = state.decisions(owner).unwrap();
        assert_eq!(decisions.members(0).unwrap().len(), 8);
        assert_eq!(decisions.members(1).unwrap().len(), 2);
        assert_eq!(state.delayed(owner).unwrap().len(), usize::from(slot < 3));
    }
    assert_eq!(all_labels.len(), 10 * countries);
}

#[test]
fn prepared_owner_recovery_covers_the_live_439_owner_5977_emission_cohort() {
    populated_recovery(439);
}

#[test]
fn prepared_owner_shortage_keeps_local_control_space_and_never_publishes_a_prefix() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    let timers = [0xd0000, 0xd1000, 0xd2000, 0xd3000, 0xd4000];
    for timer in timers {
        delayed_timer(&mut image, timer, 9);
    }
    delayed_queue(&mut image, &timers);
    let mut store = census_store(8);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let country = CENSUS[1];
    let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, work).unwrap();
    let before = store.entries.len();
    store.prepare_owner_demand(&plan);
    assert!(store.entry_demand > store.capacity());
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
            .unwrap();
    });
    assert_eq!(store.entries.len(), before + 2);
    let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
    let mut repeated = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, work).unwrap();
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut repeated)
            .unwrap();
    });
    assert_eq!(store.entries.len(), before + 2);
    assert_eq!(store.counts.rejected, 2);
    let short = sealed_row(&mut store, 1);
    assert!(short.row().failure.is_none());
    assert_eq!(certifying_markers(short.row()), (0, 0));
    assert_eq!(owner_refusals(short.row()).len(), 2);
    assert!(
        owner_refusals(short.row())
            .iter()
            .all(|(_, reason)| reason == OWNER_STORAGE.as_bytes())
    );
    assert!(store.decisions.slot(0xe8000).is_none());
    let retained = store.entries.len();
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    store
        .prepare_boundary(boundary(7, 2), Some((&image, BASE)))
        .unwrap();
    let work = store.owner_work(&image, BASE, 7, GAME, 2, country);
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 2, country, work).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    assert!(store.capacity() >= retained + 9 + 2);
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 2, &mut plan)
            .unwrap()
    });
    let recovered = sealed_row(&mut store, 2);
    assert_eq!(certifying_markers(recovered.row()), (1, 1));
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(short.row()).unwrap();
    state.observe(recovered.row()).unwrap();
    assert_eq!(state.delayed(TARGET_OWNER).unwrap().len(), 5);
    assert!(state.decisions(TARGET_OWNER).is_ok());
}

#[test]
fn prepared_owner_publication_rechecks_the_writer_cut_without_native_reads() {
    let _globals = crate::event_feed_observer::test_globals();
    let image = member_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let country = CENSUS[1];
    let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, work).unwrap();
    crate::event_feed_observer::writer_moved();
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
            .unwrap()
    });
    let row = sealed_row(&mut store, 1);
    assert_eq!(certifying_markers(row.row()), (0, 0));
    assert!(
        owner_refusals(row.row())
            .iter()
            .all(|(_, reason)| reason == OWNER_CROSSED.as_bytes())
    );
    assert!(
        store
            .decisions
            .slot(MEMBER_DEFINITION - cdecision_cooldown::DEFINITION)
            .is_none()
    );
}

#[test]
fn older_pressure_does_not_refuse_a_smaller_selected_owner_that_fits() {
    let _globals = crate::event_feed_observer::test_globals();
    let image = owners_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let country = CENSUS[1];
    let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, work).unwrap();
    store.entry_demand = store.capacity() + 1;
    store.token_demand = store.script_tokens.len() + 1;
    store.scope_demand = store.script_scopes.len() + 1;
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
            .unwrap();
    });
    assert!(store.capacity_demand() > store.capacity());
    let row = sealed_row(&mut store, 1);
    assert_eq!(certifying_markers(row.row()), (1, 1));
    assert_eq!(refusal_markers(row.row()), (0, 0));
    assert_eq!(row.row().counts.unwrap().rejected, 0);
}

#[test]
fn private_owner_allocation_failure_uses_the_prepared_local_control_space() {
    let _globals = crate::event_feed_observer::test_globals();
    let image = owners_image(1);
    let mut store = census_store(8);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let country = CENSUS[1];
    let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
    crate::test_support::with_allocations_denied(|| {
        assert!(PreparedOwner::read(&image, BASE, 7, GAME, 1, country, work).is_err());
        store.refuse_owner_preparation(&image, BASE, 7, 1, country, work);
    });
    let row = sealed_row(&mut store, 1);
    assert!(row.row().failure.is_none());
    assert_eq!(certifying_markers(row.row()), (0, 0));
    assert_eq!(refusal_markers(row.row()), (1, 1));
    assert_eq!(row.row().counts.unwrap().rejected, 2);
    assert!(
        owner_refusals(row.row())
            .iter()
            .all(|(_, reason)| reason == OWNER_STORAGE.as_bytes())
    );
}

fn consecutive_private_owner_failures(tokens_tight: bool) {
    const HOUR: i32 = 2;
    let _globals = crate::event_feed_observer::test_globals();
    let (mut image, owners) = populated_cohort(4);
    collector_primitives::country_slots::note_slots(owners.len() + 1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let mut established =
        PreparedOwner::read(&image, BASE, 7, GAME, 1, owners[0], [true; 2]).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &established);
    store
        .publish_owner(&NoReads, BASE, 7, 1, &mut established)
        .unwrap();
    let earlier = sealed_row(&mut store, 1);
    image.d(GAME + cgame_state::CURRENT_HOUR, HOUR as u32);
    store
        .prepare_boundary(boundary(7, HOUR), Some((&image, BASE)))
        .unwrap();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    let mut selected = [false; 2];
    selected[OwnerRegistry::Decisions as usize] = true;
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, HOUR, owners[1], selected).unwrap();
    let demand = store.prepare_owner_demand(&plan);
    let capacity = if tokens_tight {
        store.capacity()
    } else {
        demand[0]
    };
    let mut entries = Vec::with_capacity(capacity);
    entries.append(&mut store.entries);
    store.entries = entries;
    if tokens_tight {
        tight_tokens(&mut store, demand[1]);
    }
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, HOUR, &mut plan)
            .unwrap();
    });
    assert!(store.decisions.certified(owners[1].pointer));
    let capacity = store.capacity();
    let mut failures = 0;
    for (ordinal, country) in owners[1..].iter().copied().enumerate() {
        for registry in [OwnerRegistry::Decisions, OwnerRegistry::Delayed] {
            if ordinal == 0 && registry == OwnerRegistry::Decisions {
                continue;
            }
            let mut selected = [false; 2];
            selected[registry as usize] = true;
            let before = store.entries.len();
            store.entry_demand = capacity + 1;
            crate::test_support::with_allocations_denied(|| {
                assert!(
                    PreparedOwner::read(&image, BASE, 7, GAME, HOUR, country, selected,).is_err()
                );
            });
            crate::test_support::with_allocations_denied(|| {
                crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
            });
            crate::test_support::with_allocations_denied(|| {
                store.refuse_owner_preparation(&image, BASE, 7, HOUR, country, selected);
            });
            failures += 1;
            assert_eq!(store.capacity(), capacity);
            assert!(store.failure.is_none(), "failure {failures}");
            assert_eq!(store.entries.len(), before + 1, "failure {failures}");
            assert!(store.decisions.certified(owners[0].pointer));
            assert!(store.delayed.countries[owners[0].canonical_slot as usize].ready);
            assert!(store.decisions.certified(owners[1].pointer));
            assert!(crate::event_feed_observer::recovery_pending());
        }
    }
    store.finish_recovery(7, epoch, true);
    assert!(crate::event_feed_observer::recovery_pending());
    let refused = sealed_row(&mut store, HOUR);
    assert!(refused.row().failure.is_none());
    assert_eq!(owner_refusals(refused.row()).len(), failures);
    assert_eq!(refused.row().counts.unwrap().rejected, failures as u64);
    assert!(
        owner_refusals(refused.row())
            .iter()
            .all(|(_, reason)| { reason == OWNER_STORAGE.as_bytes() })
    );
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(earlier.row()).unwrap();
    state.observe(refused.row()).unwrap();
    let identity = |country: engine_native_memory::country_identity::Country| EventFeedCountry {
        reference: country.id,
        id: country.id,
        instance: country.id + 1000,
    };
    assert_eq!(
        state
            .decisions(identity(owners[0]))
            .unwrap()
            .members(0)
            .unwrap()
            .len(),
        8
    );
    assert_eq!(state.delayed(identity(owners[0])).unwrap().len(), 1);
    assert!(state.decisions(identity(owners[1])).is_ok());
    assert!(state.delayed(identity(owners[1])).is_err());
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
}

#[test]
fn consecutive_private_failures_keep_census_sized_entry_controls_after_a_near_full_baseline() {
    consecutive_private_owner_failures(false);
}

#[test]
fn consecutive_private_failures_keep_census_sized_token_controls_after_a_near_full_baseline() {
    consecutive_private_owner_failures(true);
}

#[test]
fn failure_to_grow_the_census_refusal_reserve_stays_local_and_retries_unwritten_controls() {
    let _globals = crate::event_feed_observer::test_globals();
    let (mut image, owners) = populated_cohort(8);
    collector_primitives::country_slots::note_slots(2);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let mut healthy = PreparedOwner::read(&image, BASE, 7, GAME, 1, owners[0], [true; 2]).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &healthy);
    let demand = store.prepare_owner_demand(&healthy);
    let mut entries = Vec::with_capacity(demand[0]);
    entries.append(&mut store.entries);
    store.entries = entries;
    tight_tokens(&mut store, demand[1]);
    store
        .publish_owner(&NoReads, BASE, 7, 1, &mut healthy)
        .unwrap();
    let earlier = sealed_row(&mut store, 1);
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    store
        .prepare_boundary(boundary(7, 2), Some((&image, BASE)))
        .unwrap();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    collector_primitives::country_slots::note_slots(owners.len() + 1);
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 2, owners[1], [true; 2]).unwrap();
    crate::test_support::with_allocations_denied(|| {
        crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    });
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 2, &mut plan)
            .unwrap();
    });
    for country in owners[2..].iter().copied() {
        crate::test_support::with_allocations_denied(|| {
            assert!(PreparedOwner::read(&image, BASE, 7, GAME, 2, country, [true; 2]).is_err());
        });
        crate::test_support::with_allocations_denied(|| {
            crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
        });
        crate::test_support::with_allocations_denied(|| {
            store.refuse_owner_preparation(&image, BASE, 7, 2, country, [true; 2]);
        });
        assert!(store.failure.is_none());
        assert!(store.decisions.certified(owners[0].pointer));
        assert!(store.delayed.countries[owners[0].canonical_slot as usize].ready);
        assert!(!store.decisions.certified(country.pointer));
    }
    assert_eq!(store.counts.rejected, 2 * (owners.len() as u64 - 1));
    let written = store.entries.len();
    let rejected = store.counts.rejected;
    crate::test_support::with_allocations_denied(|| {
        store.refuse_owner_preparation(&image, BASE, 7, 2, owners[1], [true; 2]);
    });
    assert_eq!(store.entries.len(), written);
    assert_eq!(store.counts.rejected, rejected);
    crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    let last = *owners.last().unwrap();
    crate::test_support::with_allocations_denied(|| {
        store.refuse_owner_preparation(&image, BASE, 7, 2, last, [true; 2]);
    });
    assert_eq!(store.entries.len(), written + 2);
    assert!(store.failure.is_none());
    store.finish_recovery(7, epoch, true);
    assert!(crate::event_feed_observer::recovery_pending());
    let refused = sealed_row(&mut store, 2);
    assert_eq!(certifying_markers(refused.row()), (0, 0));
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(earlier.row()).unwrap();
    state.observe(refused.row()).unwrap();
    let owner = EventFeedCountry {
        reference: 1,
        id: 1,
        instance: 1001,
    };
    assert!(state.decisions(owner).is_ok());
    assert_eq!(state.delayed(owner).unwrap().len(), 1);
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
}

#[test]
fn ordinary_occurrences_cannot_spend_reserved_control_entries_or_token_bytes() {
    let _globals = crate::event_feed_observer::test_globals();
    collector_primitives::country_slots::note_slots(CENSUS.len() + 1);
    for tokens_tight in [false, true] {
        let image = owners_image(1);
        let mut store = census_store(64);
        store
            .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
            .unwrap();
        store.begin_recovery(7, GAME).unwrap();
        let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 1, CENSUS[1], [true; 2]).unwrap();
        let demand = store.prepare_owner_demand(&plan);
        let mut entries = Vec::with_capacity(if tokens_tight { 64 } else { demand[0] });
        entries.append(&mut store.entries);
        store.entries = entries;
        if tokens_tight {
            tight_tokens(&mut store, demand[1]);
        }
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
            .unwrap();
        let controls = store.owner_controls;
        let used = store.tokens_used;
        let entries = store.entries.len();
        assert!(controls > 1);
        let mut occurrence = script_fact(0, 0, None);
        occurrence.scope = None;
        crate::test_support::with_allocations_denied(|| {
            let result = store.script_known(&NoReads, BASE, 7, 1, occurrence);
            if tokens_tight {
                assert!(result.is_ok());
                assert_eq!(store.entries.len(), entries + 1);
                assert!(
                    store
                        .entries
                        .last()
                        .unwrap()
                        .event
                        .script
                        .unwrap()
                        .token_unrecorded
                );
            } else {
                assert_eq!(
                    result,
                    Err(crate::event_feed_history::OCCURRENCE_ALLOCATION)
                );
                assert_eq!(store.entries.len(), entries);
            }
        });
        assert!(store.failure.is_some());
        assert_eq!(store.tokens_used, used);
        assert_eq!(store.owner_controls, controls);
    }
}

#[test]
fn prepared_read_failure_retries_and_recovers_while_keeping_the_other_family() {
    let _globals = crate::event_feed_observer::test_globals();
    let memory = FlightReads::stable(member_image(1), MEMBER_DEFINITION);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&memory, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let country = CENSUS[1];
    let work = store.owner_work(&memory, BASE, 7, GAME, 1, country);
    let mut plan = PreparedOwner::read(&memory, BASE, 7, GAME, 1, country, work).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
            .unwrap();
        assert_eq!(
            store.owner_work(&NoReads, BASE, 7, GAME, 1, country),
            [false, true]
        );
    });
    assert_eq!(memory.reads.get(), 1);
    let row = sealed_row(&mut store, 1);
    assert_eq!(certifying_markers(row.row()), (1, 0));
    assert_eq!(refusal_markers(row.row()), (0, 1));
    assert_eq!(row.row().counts.unwrap().rejected, 1);
    assert!(row.row().failure.is_none());
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(row.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).unwrap().is_empty());
    assert!(state.decisions(TARGET_OWNER).is_err());
    store.acknowledged(row.publication());
    let mut image = memory.image;
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    store
        .prepare_boundary(boundary(7, 2), Some((&image, BASE)))
        .unwrap();
    let work = store.owner_work(&image, BASE, 7, GAME, 2, country);
    assert_eq!(work, [false, true]);
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 2, country, work).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    store
        .publish_owner(&NoReads, BASE, 7, 2, &mut plan)
        .unwrap();
    let recovered = sealed_row(&mut store, 2);
    assert_eq!(certifying_markers(recovered.row()), (0, 1));
    state.observe(recovered.row()).unwrap();
    assert!(state.decisions(TARGET_OWNER).is_ok());
    assert!(state.delayed(TARGET_OWNER).unwrap().is_empty());
}

#[test]
fn every_private_owner_growth_failure_remains_retryable_and_owner_local() {
    let _globals = crate::event_feed_observer::test_globals();
    let image = member_image(1);
    let country = CENSUS[1];
    let track = collector_test_support::NullGuard::new(usize::MAX);
    let complete = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, [true; 2]);
    let growths = collector_test_support::ATTEMPTS.load(Ordering::Relaxed);
    drop(track);
    assert!(complete.is_ok());
    drop(complete);
    assert!(growths > 1);
    for ordinal in 1..=growths {
        let mut store = census_store(8);
        store
            .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
            .unwrap();
        store.begin_recovery(7, GAME).unwrap();
        let deny = collector_test_support::NullGuard::new(ordinal);
        let prepared = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, [true; 2]);
        drop(deny);
        assert_eq!(prepared.err(), Some(OWNER_STORAGE), "growth {ordinal}");
        crate::test_support::with_allocations_denied(|| {
            store.refuse_owner_preparation(&image, BASE, 7, 1, country, [true; 2]);
        });
        assert!(store.failure.is_none());
        assert_eq!(
            store.owner_work(&NoReads, BASE, 7, GAME, 1, country),
            [true; 2]
        );
        let row = sealed_row(&mut store, 1);
        assert_eq!(certifying_markers(row.row()), (0, 0));
        assert_eq!(refusal_markers(row.row()), (1, 1));
    }
}

#[test]
fn prepared_recovery_after_a_loss_never_reuses_a_member_label() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = member_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let country = CENSUS[1];
    let member = MEMBER_DEFINITION - cdecision_cooldown::DEFINITION;
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 1, country, [true; 2]).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 1, &mut plan)
            .unwrap()
    });
    let first_id = store.decisions.find(member).unwrap().id;
    let first = sealed_row(&mut store, 1);
    store.acknowledged(first.publication());
    store.invalidate("synthetic observation loss");
    let loss = pack(&mut store, boundary(7, 2));
    store.acknowledged(loss.publication());
    image.d(GAME + cgame_state::CURRENT_HOUR, 3);
    store
        .prepare_boundary(boundary(7, 3), Some((&image, BASE)))
        .unwrap();
    store.begin_recovery(7, GAME).unwrap();
    let mut plan = PreparedOwner::read(&image, BASE, 7, GAME, 3, country, [true; 2]).unwrap();
    crate::event_feed_observer::presize_owner_for_test(&mut store, &plan);
    crate::test_support::with_allocations_denied(|| {
        store
            .publish_owner(&NoReads, BASE, 7, 3, &mut plan)
            .unwrap()
    });
    assert!(store.decisions.find(member).unwrap().id > first_id);
    let recovered = sealed_row(&mut store, 3);
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    assert_eq!(certifying_markers(recovered.row()), (1, 1));
}
