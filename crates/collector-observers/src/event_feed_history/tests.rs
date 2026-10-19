use super::*;
use engine_native::{
    ccountry, cdiplomacy_status, cgame_state, crelation, crelation_status, cwar_relation,
};
use std::collections::BTreeMap;

struct NoReads;
impl Memory for NoReads {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("owned annex settlement must not reread roles after authored dispatch")
    }
}

fn operands(hour: i32, actor: u32) -> native::Operands {
    native::Operands {
        source: native::Source::Annex,
        hour,
        actor: native::CountryIdentity {
            reference: actor + 1,
            id: actor,
            instance: 44,
        },
        target: native::CountryIdentity {
            reference: 0,
            id: 0,
            instance: 11,
        },
    }
}

fn boundary(generation: u64, hour: i32) -> OwnerBoundary {
    OwnerBoundary {
        generation,
        read_hour: hour,
        coverage: EventFeedCoverage::Complete,
        failure: None,
    }
}

fn pack(store: &mut Store, boundary: OwnerBoundary) -> Prepared {
    let mut snapshot = Snapshot::new(store.capacity());
    store.snapshot(&mut snapshot, boundary).unwrap();
    snapshot.prepare_pack().unwrap()
}

fn ready(capacity: usize) -> Store {
    let mut store = Store::new(None).unwrap();
    store.prepare_capacity(capacity).unwrap();
    let first = pack(&mut store, boundary(7, 0));
    assert_eq!(first.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(first.publication());
    store
}

fn script_fact(frame: usize, scope: usize, option: Option<i32>) -> ScriptFact {
    let mut header = [0; 32];
    header[..11].copy_from_slice(b"synthetic.1");
    header[16..24].copy_from_slice(&11_u64.to_le_bytes());
    header[24..32].copy_from_slice(&15_u64.to_le_bytes());
    ScriptFact {
        source: if option.is_some() {
            EventFeedSource::EventOption
        } else {
            EventFeedSource::ScriptEvent
        },
        actor: operands(0, 1).actor,
        target: operands(0, 2).actor,
        token: engine_native::msvc_string::Descriptor::from_header(&NoReads, 0, header).unwrap(),
        scope: Some(scope),
        dispatch_frame: frame,
        dispatch_definition: 123,
        cause: Default::default(),
        script: EventFeedScript {
            native_event: Some(replay_data::event_feed_rows::EventFeedEventIdentity {
                kind: 99,
                id: 123,
            }),
            option,
            ..Default::default()
        },
    }
}

fn script_image() -> WarImage {
    let mut image = WarImage::default();
    let scope = 0x80000;
    image.bytes(scope, &[0; engine_events::cevent_scope::SIZE]);
    image.q(scope, BASE + engine_events::cevent_scope::VTABLE_RVA);
    for offset in [
        engine_events::cevent_scope::ROOT,
        engine_events::cevent_scope::FROM,
        engine_events::cevent_scope::PREV,
    ] {
        image.q(scope + offset, scope);
    }
    image
}

#[test]
fn script_reentrant_instances_and_options_share_receipts_without_losing_repeats() {
    let mut store = ready(8);
    store.prepare_scripts(128, 8, 1).unwrap();
    let image = script_image();
    store
        .script_known(&image, BASE, 7, 1, script_fact(100, 0x80000, None))
        .unwrap();
    store
        .script_known(&image, BASE, 7, 1, script_fact(200, 0x80000, None))
        .unwrap();
    store.registered_instance(7, 0x80000, 123, 91);
    store.script_instance(7, 100, 90);
    store
        .script_known(&image, BASE, 7, 1, script_fact(0, 0x80000, Some(-1)))
        .unwrap();
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_growing(&mut snapshot, boundary(7, 2))
        .unwrap();
    let first = snapshot.prepare_pack().unwrap();
    let row = first.row();
    assert_eq!(row.events.len(), 3);
    assert_eq!(row.events[0].script.unwrap().instance, Some(90));
    assert_eq!(row.events[1].script.unwrap().instance, Some(91));
    assert_eq!(row.events[2].script.unwrap().option, Some(-1));
    assert_eq!(row.script_scopes.len(), 3);
    assert!(
        row.script_scopes
            .iter()
            .all(|scope| scope.bindings == [0; 3])
    );
    assert_eq!(row.counts.unwrap().settled, 3);
    store.acknowledged(first.publication());
    store
        .snapshot_growing(&mut snapshot, boundary(7, 3))
        .unwrap();
    assert!(snapshot.prepare_pack().unwrap().row().events.is_empty());
    assert_eq!(store.tokens_used, 0);
    assert_eq!(store.scopes_used, 0);
    // Later callbacks can overwrite staging; an immutable publication owns all
    // of its payload, including the original negative option and zero country.
    store
        .script_known(&image, BASE, 7, 3, script_fact(300, 0x80000, None))
        .unwrap();
    assert_eq!(first.row().events[2].script.unwrap().option, Some(-1));
    assert_eq!(first.row().script_scopes[0].country, 0);
}

#[test]
fn script_pressure_refuses_once_and_reports_exact_reached_demand() {
    let mut store = ready(1);
    store.prepare_scripts(0, 0, 0).unwrap();
    store
        .script_known(&NoReads, BASE, 7, 1, script_fact(100, 0x80000, None))
        .unwrap();
    assert_eq!(store.token_demand, 11);
    assert_eq!(store.scope_demand, 0);
    assert_eq!(store.counts.rejected, 2);
    assert_eq!(store.entries.len(), 1);
    let script = store.entries[0].event.script.unwrap();
    assert_eq!(
        script.native_event,
        Some(replay_data::event_feed_rows::EventFeedEventIdentity { kind: 99, id: 123 })
    );
    assert!(script.token_unrecorded);
    assert!(script.scope_unrecorded);
}

#[test]
fn script_continuous_occurrences_reclaim_acknowledged_arena_spans() {
    let mut store = ready(8);
    store.prepare_scripts(64, 4, 1).unwrap();
    let image = script_image();
    let mut snapshot = Snapshot::new(8);
    for hour in 1..50 {
        store
            .script_known(
                &image,
                BASE,
                7,
                hour,
                script_fact(hour as usize, 0x80000, None),
            )
            .unwrap();
        store
            .snapshot_growing(&mut snapshot, boundary(7, hour))
            .unwrap();
        let pack = snapshot.prepare_pack().unwrap();
        assert_eq!(pack.row().script_tokens, b"synthetic.1");
        assert_eq!(pack.row().script_scopes.len(), 1);
        assert_eq!((store.tokens_used, store.scopes_used), (11, 1));
        store.acknowledged(pack.publication());
    }
    assert_eq!(store.counts.rejected, 0);
}

#[test]
fn script_delayed_baseline_reads_only_first_seen_census_owners_and_carries_empty() {
    let mut store = ready(8);
    store.prepare_delayed(5001).unwrap();
    store.registry_state = Some(GAME);
    let country = engine_native::ccountry::Country {
        pointer: 0x80000,
        canonical_slot: 4999,
        id: 5000,
    };
    let mut image = WarImage::default();
    image.bytes(
        country.pointer + engine_native::ccountry::DELAYED_EVENTS,
        &[0; 16],
    );
    image.d(country.pointer + engine_native::ccountry::ID_INSTANCE, 88);
    store
        .delayed_baseline(&image, BASE, 7, GAME, 1, country)
        .unwrap();
    // A carried owner never rereads its identity, header or timer payload.
    store
        .delayed_baseline(&NoReads, BASE, 7, GAME, 1, country)
        .unwrap();
    let mut snapshot = Snapshot::new(8);
    store
        .snapshot_growing(&mut snapshot, boundary(7, 2))
        .unwrap();
    let pack = snapshot.prepare_pack().unwrap();
    assert_eq!(pack.row().events.len(), 1);
    assert_eq!(
        pack.row().events[0].script.unwrap().change,
        Some(replay_data::event_feed_rows::EventFeedScriptChange::DelayedEmpty)
    );
    assert_eq!(pack.row().events[0].target.instance, 88);
}

#[test]
fn script_registry_bootstrap_waits_for_the_failed_interval_receipt_without_polling() {
    let mut store = ready(8);
    store.reject("synthetic pre-observation refusal");
    let mut snapshot = Snapshot::new(8);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &NoReads, BASE)
        .unwrap();
    assert!(snapshot.events.is_empty());
    assert_eq!(store.registry_baseline, None);
    store.acknowledged(snapshot.publication());

    let mut image = WarImage::new();
    image.bytes(GAME + cgame_state::PENDING_EVENTS, &[0; 16]);
    image.bytes(GAME + cgame_state::FIRED_EVENTS, &[0; 16]);
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &image, BASE)
        .unwrap();
    let pack = snapshot.prepare_pack().unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(pack.row()).unwrap();
    assert!(state.pending().unwrap().is_empty());
    assert!(state.fired().unwrap().is_empty());
    store.acknowledged(pack.publication());

    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 3), &NoReads, BASE)
        .unwrap();
    assert!(snapshot.events.is_empty());
}

fn retained_registry_image(hour: i32) -> WarImage {
    let mut image = WarImage::new();
    image.0.extend(script_image().0);
    image.d(GAME + cgame_state::CURRENT_HOUR, hour as u32);
    let definition = 0x90000;
    image.q(definition, BASE + engine_decisions::cevent::VTABLE_RVA);
    image.d(definition + engine_decisions::cevent::REFERENCE, 99);
    image.d(definition + engine_decisions::cevent::REFERENCE + 4, 123);
    let mut token = [0; 32];
    token[..11].copy_from_slice(b"synthetic.1");
    token[16..24].copy_from_slice(&11_u64.to_le_bytes());
    token[24..32].copy_from_slice(&15_u64.to_le_bytes());
    image.bytes(definition + engine_decisions::cevent::TOKEN, &token);
    let pending = 0xa0000;
    image.array(GAME + cgame_state::PENDING_EVENTS, pending, 1);
    image.bytes(
        pending,
        &[0; engine_events::script_event_registry::PENDING_STRIDE],
    );
    image.d(pending, 11);
    image.q(
        pending + engine_events::script_event_registry::PENDING_DEFINITION,
        definition,
    );
    image.q(
        pending + engine_events::script_event_registry::PENDING_SCOPE,
        0x80000,
    );
    image.d(
        pending + engine_events::script_event_registry::PENDING_ORIGINATOR,
        1,
    );
    image.d(
        pending + engine_events::script_event_registry::PENDING_DUE,
        40,
    );
    image.d(GAME + cgame_state::FIRED_EVENTS, 1);
    image.d(GAME + cgame_state::FIRED_EVENTS + 4, 1);
    image.q(GAME + cgame_state::FIRED_EVENTS + 8, 0xb0000);
    image.q(0xb0000, 0xb1000);
    image.q(0xb1000, definition);
    image.q(0xb1008, 0);
    image
}

#[test]
fn retained_registry_writer_loss_requires_receipt_then_a_fresh_native_baseline() {
    let mut store = ready(8);
    let mut image = retained_registry_image(1);
    let mut snapshot = Snapshot::new(0);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    let first = snapshot.prepare_pack().unwrap();
    state.observe(first.row()).unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 11);
    assert_eq!(state.fired().unwrap().len(), 1);
    store.acknowledged(first.publication());

    // Exercise the actual writer's prepared-token refusal after a complete
    // baseline, rather than pretending that a sample proves the missed call.
    store
        .prepare_scripts(
            store.script_tokens.len(),
            store.scopes_used + 1,
            store.scope_nodes.len(),
        )
        .unwrap();
    store.script_tokens.truncate(store.tokens_used);
    store
        .script_known(&image, BASE, 7, 2, script_fact(0, 0x80000, None))
        .unwrap();
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let lost = snapshot.prepare_pack().unwrap();
    assert_eq!(lost.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(lost.row().counts.unwrap().rejected, 1);
    state.observe(lost.row()).unwrap();
    assert!(state.pending().is_err());
    assert!(state.fired().is_err());
    // An unsettled refusal cannot admit a baseline, even on another boundary.
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 3), &NoReads, BASE)
        .unwrap();
    let waiting = snapshot.prepare_pack().unwrap();
    state.observe(waiting.row()).unwrap();
    assert_eq!(waiting.row().coverage, EventFeedCoverage::Unavailable);
    store.acknowledged(lost.publication());
    store.acknowledged(waiting.publication());

    image.d(GAME + cgame_state::CURRENT_HOUR, 4);
    image.d(0xa0000, 22);
    image.d(
        0xa0000 + engine_events::script_event_registry::PENDING_DUE,
        80,
    );
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &image, BASE)
        .unwrap();
    let recovered = snapshot.prepare_pack().unwrap();
    state.observe(recovered.row()).unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 22);
    assert_eq!(state.pending().unwrap()[0].due_hour, 80);
    assert_eq!(state.fired().unwrap().len(), 1);
    assert_eq!(recovered.row().counts.unwrap().rejected, 1);
    store.acknowledged(recovered.publication());
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 5), &NoReads, BASE)
        .unwrap();
    let quiet = snapshot.prepare_pack().unwrap();
    assert_eq!(quiet.row().coverage, EventFeedCoverage::Complete);
    assert!(quiet.row().events.is_empty());
    state.observe(quiet.row()).unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 22);
}

#[test]
fn unsupported_registry_baseline_remains_refused_without_periodic_retries() {
    let mut store = ready(8);
    let mut image = retained_registry_image(1);
    image
        .0
        .remove(&(0x80000 + engine_events::cevent_scope::COUNTRY_TAG));
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    let failed = snapshot.prepare_pack().unwrap();
    assert_eq!(failed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        failed.row().failure.as_deref(),
        Some(b"pending event scope country unreadable".as_slice())
    );
    assert_eq!(failed.row().counts.unwrap().rejected, 1);
    store.acknowledged(failed.publication());
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let next = snapshot.prepare_pack().unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(failed.row()).unwrap();
    state.observe(next.row()).unwrap();
    assert!(next.row().events.is_empty());
    assert!(state.pending().is_err());
    assert_eq!(next.row().counts.unwrap().rejected, 1);
}

#[test]
fn unavailable_boundary_rebases_only_after_its_refusal_receipt() {
    let mut store = ready(8);
    let mut image = retained_registry_image(1);
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    store.acknowledged(snapshot.publication());
    let mut unavailable = boundary(7, 2);
    unavailable.failure = Some("synthetic boundary observation unavailable");
    store.snapshot_growing(&mut snapshot, unavailable).unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 3), &NoReads, BASE)
        .unwrap();
    let waiting = snapshot.prepare_pack().unwrap();
    assert_eq!(waiting.row().coverage, EventFeedCoverage::Unavailable);
    store.acknowledged(refused.publication());
    store.acknowledged(waiting.publication());
    image.d(GAME + cgame_state::CURRENT_HOUR, 4);
    image.d(0xa0000, 33);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &image, BASE)
        .unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state
        .observe(snapshot.prepare_pack().unwrap().row())
        .unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 33);
}

#[test]
fn partial_boundary_reseeds_after_receipt_and_preserves_its_fresh_first_baseline() {
    let mut store = Store::new(None).unwrap();
    let mut image = retained_registry_image(1);
    let mut snapshot = Snapshot::new(0);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    let first = snapshot.prepare_pack().unwrap();
    assert_eq!(first.row().coverage, EventFeedCoverage::Partial);
    state.observe(first.row()).unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 11);
    assert_eq!(state.fired().unwrap().len(), 1);
    store.acknowledged(first.publication());
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let quiet = snapshot.prepare_pack().unwrap();
    assert_eq!(quiet.row().coverage, EventFeedCoverage::Complete);
    assert!(quiet.row().events.is_empty());
    state.observe(quiet.row()).unwrap();
    store.acknowledged(quiet.publication());

    // This is the production boundary shape for a withdrawn forwarder or
    // partial input: no explicit failure and no newly acquired baseline.
    let mut partial = boundary(7, 3);
    partial.coverage = EventFeedCoverage::Partial;
    store
        .snapshot_with_registry(&mut snapshot, partial, &NoReads, BASE)
        .unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Partial);
    assert!(refused.row().failure.is_none());
    assert_eq!(refused.row().counts.unwrap().rejected, 0);
    state.observe(refused.row()).unwrap();
    assert!(state.pending().is_err());
    assert!(state.fired().is_err());
    // Healthy forwarders do not admit a read ahead of the loss's receipt.
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &NoReads, BASE)
        .unwrap();
    let waiting = snapshot.prepare_pack().unwrap();
    assert_eq!(waiting.row().coverage, EventFeedCoverage::Unavailable);
    state.observe(waiting.row()).unwrap();
    store.acknowledged(refused.publication());
    store.acknowledged(waiting.publication());

    // An unsupported interval must not become an hourly registry walk,
    // including after its output receipt has arrived.
    partial.read_hour = 5;
    store
        .snapshot_with_registry(&mut snapshot, partial, &NoReads, BASE)
        .unwrap();
    let still_partial = snapshot.prepare_pack().unwrap();
    state.observe(still_partial.row()).unwrap();
    assert_eq!(still_partial.row().coverage, EventFeedCoverage::Partial);
    assert!(still_partial.row().events.is_empty());
    store.acknowledged(still_partial.publication());

    image.d(GAME + cgame_state::CURRENT_HOUR, 6);
    image.d(0xa0000, 22);
    image.d(
        0xa0000 + engine_events::script_event_registry::PENDING_DUE,
        80,
    );
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 6), &image, BASE)
        .unwrap();
    let recovered = snapshot.prepare_pack().unwrap();
    state.observe(recovered.row()).unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 22);
    assert_eq!(state.pending().unwrap()[0].due_hour, 80);
    assert_eq!(state.fired().unwrap().len(), 1);
    assert_eq!(recovered.row().counts.unwrap().rejected, 0);
    store.acknowledged(recovered.publication());
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 7), &NoReads, BASE)
        .unwrap();
    let quiet = snapshot.prepare_pack().unwrap();
    state.observe(quiet.row()).unwrap();
    assert_eq!(quiet.row().coverage, EventFeedCoverage::Complete);
    assert!(quiet.row().events.is_empty());
    assert_eq!(state.pending().unwrap()[0].instance, 22);
}

#[test]
fn older_publication_receipt_cannot_unlock_a_later_boundary_refusal() {
    let mut store = ready(8);
    let mut image = retained_registry_image(1);
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    store.acknowledged(snapshot.publication());
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let older = snapshot.prepare_pack().unwrap();
    let mut unavailable = boundary(7, 3);
    unavailable.failure = Some("synthetic later boundary observation unavailable");
    store.snapshot_growing(&mut snapshot, unavailable).unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    // This old receipt used to clear failure_revision before the refused
    // boundary had any receipt, allowing the next boundary to read native state.
    store.acknowledged(older.publication());
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &NoReads, BASE)
        .unwrap();
    let waiting = snapshot.prepare_pack().unwrap();
    assert_eq!(waiting.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        waiting.row().failure.as_deref(),
        Some(b"synthetic later boundary observation unavailable".as_slice())
    );
    store.acknowledged(refused.publication());
    store.acknowledged(waiting.publication());
    image.d(GAME + cgame_state::CURRENT_HOUR, 5);
    image.d(0xa0000, 44);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 5), &image, BASE)
        .unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state
        .observe(snapshot.prepare_pack().unwrap().row())
        .unwrap();
    assert_eq!(state.pending().unwrap()[0].instance, 44);
}

#[test]
fn unsupported_delayed_owner_is_attempted_once_through_refusal_and_registry_recovery() {
    struct Reads {
        image: WarImage,
        failed_owner: std::cell::Cell<usize>,
        healthy_owner: std::cell::Cell<usize>,
    }
    impl Memory for Reads {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == TARGET + ccountry::DELAYED_EVENTS {
                self.failed_owner.set(self.failed_owner.get() + 1);
            } else if address == ACTOR + ccountry::DELAYED_EVENTS {
                self.healthy_owner.set(self.healthy_owner.get() + 1);
            }
            self.image.copy(address, output)
        }
    }
    let mut image = retained_registry_image(1);
    image.bytes(ACTOR + ccountry::DELAYED_EVENTS, &[0; 16]);
    image.array(TARGET + ccountry::DELAYED_EVENTS, 0xc0000, 1);
    image.q(0xc0000, 0xc1000);
    image.q(0xc1000, 0); // Populated, unsupported delayed object.
    let mut memory = Reads {
        image,
        failed_owner: Default::default(),
        healthy_owner: Default::default(),
    };
    let healthy = ccountry::Country {
        pointer: ACTOR,
        canonical_slot: 1,
        id: 1,
    };
    let unsupported = ccountry::Country {
        pointer: TARGET,
        canonical_slot: 2,
        id: 2,
    };
    let owner = EventFeedCountry {
        reference: 1,
        id: 1,
        instance: 44,
    };
    let unsupported_owner = EventFeedCountry {
        reference: 2,
        id: 2,
        instance: 55,
    };
    let mut store = ready(8);
    let mut snapshot = Snapshot::new(0);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &memory, BASE)
        .unwrap();
    state
        .observe(snapshot.prepare_pack().unwrap().row())
        .unwrap();
    store.acknowledged(snapshot.publication());
    store
        .delayed_baseline(&memory, BASE, 7, GAME, 1, healthy)
        .unwrap();
    let reason = store
        .delayed_baseline(&memory, BASE, 7, GAME, 1, unsupported)
        .unwrap_err();
    assert_eq!(reason, "delayed-event vtable differs");
    // The existing census forwarder records this actual baseline failure.
    store.reject(reason);
    assert_eq!(memory.failed_owner.get(), 1);
    assert_eq!(memory.healthy_owner.get(), 1);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(refused.row().failure.as_deref(), Some(reason.as_bytes()));
    assert_eq!(refused.row().counts.unwrap().rejected, 1);
    state.observe(refused.row()).unwrap();
    assert!(state.delayed(owner).is_err());
    store.acknowledged(refused.publication());

    memory.image.d(GAME + cgame_state::CURRENT_HOUR, 3);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 3), &memory, BASE)
        .unwrap();
    state
        .observe(snapshot.prepare_pack().unwrap().row())
        .unwrap();
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
    store.acknowledged(snapshot.publication());
    store
        .delayed_baseline(&memory, BASE, 7, GAME, 3, unsupported)
        .unwrap();
    assert_eq!(memory.failed_owner.get(), 1);
    // Previously ready owners lost their certificate and acquire it afresh.
    store
        .delayed_baseline(&memory, BASE, 7, GAME, 3, healthy)
        .unwrap();
    assert_eq!(memory.healthy_owner.get(), 2);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &NoReads, BASE)
        .unwrap();
    let recovered = snapshot.prepare_pack().unwrap();
    state.observe(recovered.row()).unwrap();
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Complete);
    assert!(state.delayed(owner).unwrap().is_empty());
    assert!(state.delayed(unsupported_owner).is_err());
    assert_eq!(recovered.row().counts.unwrap().rejected, 1);
    store.acknowledged(recovered.publication());
    for hour in 5..8 {
        store
            .delayed_baseline(&NoReads, BASE, 7, GAME, hour, unsupported)
            .unwrap();
        store
            .delayed_baseline(&NoReads, BASE, 7, GAME, hour, healthy)
            .unwrap();
        store
            .snapshot_with_registry(&mut snapshot, boundary(7, hour), &NoReads, BASE)
            .unwrap();
        state
            .observe(snapshot.prepare_pack().unwrap().row())
            .unwrap();
        store.acknowledged(snapshot.publication());
    }
    assert_eq!(memory.failed_owner.get(), 1);
    assert_eq!(memory.healthy_owner.get(), 2);
    store.invalidate_delayed_country(TARGET);
    let reason = store
        .delayed_baseline(&memory, BASE, 7, GAME, 8, unsupported)
        .unwrap_err();
    assert_eq!(reason, "delayed-event vtable differs");
    store.reject(reason);
    assert_eq!(memory.failed_owner.get(), 2);
    assert_eq!(store.counts.rejected, 2);
}

fn acquire(store: &mut Store, hour: i32, actor: u32) -> Ticket {
    store
        .acquire(
            7,
            |sequence| event(sequence, 7, operands(hour, actor), Default::default()),
            Some(Proof::Annex),
        )
        .unwrap()
}

fn settle(store: &mut Store, ticket: Ticket) {
    unsafe { store.normal_returned(ticket, 7, &NoReads, 0).unwrap() };
}

#[test]
fn callback_demand_follows_retained_entries_without_allocator_headroom() {
    for capacity in [8, 1500, 5000] {
        let mut store = ready(capacity);
        let ticket = acquire(&mut store, 1, 3);
        assert_eq!(store.capacity_demand(), 2);
        settle(&mut store, ticket);
        assert_eq!(store.capacity_demand(), 2);
    }
}

#[test]
fn repeated_same_hour_pair_has_distinct_acquisitions_and_later_visibility() {
    let mut store = ready(2);
    let first = acquire(&mut store, -17, 2);
    let second = acquire(&mut store, -17, 2);
    // Reentrant completion can reverse; history remains acquisition ordered.
    settle(&mut store, second);
    settle(&mut store, first);
    let packed = pack(&mut store, boundary(7, 8));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(packed.row().read_hour, 8);
    let events = &packed.row().events;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, 0);
    assert_eq!(events[1].sequence, 1);
    assert_eq!(events[0].hour, -17);
    assert_eq!(events[0].actor.reference, 3);
    assert_eq!(events[0].actor.id, 2);
    assert_eq!(events[0].actor.instance, 44);
    assert_eq!(events[0].target.id, 0);
    store.acknowledged(packed.publication());
    store.acknowledged(packed.publication());
    assert_eq!(store.unacknowledged(), 0);
    let empty = pack(&mut store, boundary(7, 9));
    assert!(empty.row().events.is_empty());
    assert_eq!(empty.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(empty.row().counts.unwrap().settled, 2);
}

#[test]
fn dropped_callback_ticket_never_releases_immutable_pending_storage() {
    let mut store = ready(1);
    {
        let _ticket = acquire(&mut store, 1, 5);
        // Deliberately no terminal call, as on the preserved RustDrop failure.
    }
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entries[0].event.actor.id, 5);
    assert!(store.entries[0].proof.is_some());
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().outstanding, 1);
    store.acknowledged(packed.publication());
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entries.len(), 1);
    let mut snapshot = Snapshot::new(1);
    assert!(store.snapshot(&mut snapshot, boundary(8, 10)).is_err());
}

#[test]
fn uncertain_normal_return_cannot_recertify_or_clear_a_claim() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 2, 4);
    store.uncertain(&ticket, "native dispatcher escaped");
    let retained = unsafe { store.normal_returned(ticket, 7, &NoReads, 0) }.unwrap_err();
    assert_eq!(
        retained.reason,
        "Event feed native completion remains uncertain"
    );
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.counts.settled, 0);
    assert_eq!(store.entry(&retained.ticket).unwrap().event.actor.id, 4);
    let packed = pack(&mut store, boundary(7, 3));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().events.len(), 1);
    assert_eq!(packed.row().events[0].source, EventFeedSource::Annex);
    assert_eq!(packed.row().counts.unwrap().settled, 0);
}

#[test]
fn same_hour_replacement_invalidates_generation_without_resetting_ownership() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 5, 6);
    store.invalidate("world replacement began");
    let mut snapshot = Snapshot::new(1);
    assert!(store.snapshot(&mut snapshot, boundary(8, 10)).is_err());
    let retained = unsafe { store.normal_returned(ticket, 8, &NoReads, 0) }.unwrap_err();
    assert_eq!(
        retained.reason,
        "Event feed world invalidated before settlement"
    );
    let packed = pack(&mut store, boundary(7, 5));
    assert_eq!(packed.row().generation, 7);
    assert_eq!(packed.row().read_hour, 5);
    assert_eq!(packed.row().outstanding, 1);
    assert_eq!(packed.row().counts.unwrap().acquired, 1);
    store.acknowledged(packed.publication());
    assert_eq!(store.outstanding(), 1);
    assert!(store.entries[0].proof.is_some());
}

#[test]
fn pressure_refuses_coverage_but_keeps_known_fact_and_requires_partial_recovery() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    let capacity = store.entries.capacity();
    while store.entries.len() < capacity {
        let other = acquire(&mut store, 1, 3);
        settle(&mut store, other);
    }
    assert_eq!(
        store
            .acquire(
                7,
                |sequence| event(sequence, 7, operands(1, 3), Default::default()),
                Some(Proof::Annex)
            )
            .err(),
        Some("Event feed callback storage needs growth")
    );
    assert_eq!(store.entries.capacity(), capacity);
    settle(&mut store, ticket);
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().events.len(), capacity);
    assert_eq!(packed.row().counts.unwrap().rejected, 1);
    store.acknowledged(packed.publication());
    let recovery = pack(&mut store, boundary(7, 3));
    assert_eq!(recovery.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(recovery.publication());
    let complete = pack(&mut store, boundary(7, 4));
    assert_eq!(complete.row().coverage, EventFeedCoverage::Complete);
    assert!(complete.row().events.is_empty());
}

#[test]
fn stale_prepared_pack_never_removes_a_later_acquisition() {
    let mut store = ready(2);
    let first = acquire(&mut store, 1, 3);
    settle(&mut store, first);
    let prepared = pack(&mut store, boundary(7, 2));
    let second = acquire(&mut store, 2, 4);
    store.acknowledged(prepared.publication());
    assert_eq!(prepared.row().events.len(), 1);
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entries[0].state, State::Published);
    assert_eq!(store.entry(&second).unwrap().event.actor.id, 4);
}

#[test]
fn missing_owner_observation_never_reads_native_identity_or_certifies_empty() {
    let mut store = ready(1);
    store.invalidate("country provider association unavailable");
    assert_eq!(
        store
            .annex_before(
                &NoReads,
                0,
                7,
                native::AnnexFrame {
                    rbp: 0,
                    r8: 0,
                    r9: 0
                },
                Default::default()
            )
            .err(),
        Some("Event feed world observation unavailable")
    );
    let packed = pack(&mut store, boundary(7, 1));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert!(packed.row().events.is_empty());
    assert_eq!(packed.row().counts.unwrap().rejected, 1);
}

#[test]
fn sequence_seed_and_cumulative_accounting_survive_a_genuine_generation_change() {
    let mut store = Store::new(Some(40)).unwrap();
    store.prepare_capacity(1).unwrap();
    let initial = pack(&mut store, boundary(7, 0));
    store.acknowledged(initial.publication());
    let first = acquire(&mut store, 6, 4);
    settle(&mut store, first);
    let packed = pack(&mut store, boundary(7, 6));
    assert_eq!(packed.row().events[0].sequence, 41);
    store.acknowledged(packed.publication());
    store.invalidate("world writer entered");
    let refused = pack(&mut store, boundary(7, 6));
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    store.acknowledged(refused.publication());
    let changed = pack(&mut store, boundary(8, 6));
    assert_eq!(changed.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(changed.publication());
    let second = store
        .acquire(
            8,
            |sequence| event(sequence, 8, operands(6, 4), Default::default()),
            Some(Proof::Annex),
        )
        .unwrap();
    unsafe { store.normal_returned(second, 8, &NoReads, 0).unwrap() };
    let packed = pack(&mut store, boundary(8, 6));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(packed.row().events[0].sequence, 42);
    assert_eq!(packed.row().counts.unwrap().acquired, 2);
}

#[test]
fn count_and_sequence_exhaustion_are_unavailable_without_wrap() {
    let mut store = ready(1);
    store.counts.acquired = u64::MAX;
    assert_eq!(
        store
            .acquire(
                7,
                |sequence| event(sequence, 7, operands(1, 3), Default::default()),
                Some(Proof::Annex)
            )
            .err(),
        Some("Event feed acquired count exhausted")
    );
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().counts, None);
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(store.counts.acquired, u64::MAX);
    assert!(store.entries.is_empty());

    let mut store = ready(1);
    store.next_sequence = u64::MAX;
    assert_eq!(
        store
            .acquire(
                7,
                |sequence| event(sequence, 7, operands(1, 3), Default::default()),
                Some(Proof::Annex)
            )
            .err(),
        Some("Event feed sequence exhausted")
    );
    assert_eq!(store.next_sequence, u64::MAX);
    assert!(store.entries.is_empty());
    assert!(Store::new(Some(u64::MAX)).is_err());
}

#[test]
fn accepted_unsettled_interval_requires_partial_after_healthy_later_return() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 2);
    let unavailable = pack(&mut store, boundary(7, 2));
    assert_eq!(unavailable.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(unavailable.row().outstanding, 1);
    assert_eq!(unavailable.row().events.len(), 1);
    store.acknowledged(unavailable.publication());
    settle(&mut store, ticket);
    let recovery = pack(&mut store, boundary(7, 3));
    assert_eq!(recovery.row().coverage, EventFeedCoverage::Partial);
    assert!(recovery.row().events.is_empty());
    store.acknowledged(recovery.publication());
    let complete = pack(&mut store, boundary(7, 4));
    assert_eq!(complete.row().coverage, EventFeedCoverage::Complete);
}

#[test]
fn escaping_annex_fact_is_owned_once_without_settling_or_losing_its_ticket() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, -17, 4);
    store.uncertain(&ticket, "native dispatcher escaped");
    let prepared = pack(&mut store, boundary(7, 9));
    assert_eq!(prepared.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(prepared.row().events[0].hour, -17);
    assert_eq!(prepared.row().counts.unwrap().acquired, 1);
    assert_eq!(prepared.row().counts.unwrap().settled, 0);
    assert_eq!(prepared.row().outstanding, 1);
    store.acknowledged(prepared.publication());
    assert_eq!(store.outstanding(), 1);
    assert_eq!(store.entry(&ticket).unwrap().state, State::PublishedPending);
    assert!(store.entry(&ticket).unwrap().proof.is_some());
    let next = pack(&mut store, boundary(7, 10));
    assert!(next.row().events.is_empty());
    assert_eq!(next.row().outstanding, 1);
    let mut snapshot = Snapshot::new(1);
    assert!(store.snapshot(&mut snapshot, boundary(8, 10)).is_err());
}

#[test]
fn pending_war_is_never_a_fact_before_same_invocation_normal_postproof() {
    let image = WarImage::new();
    let mut store = ready(1);
    let ticket = store
        .war_before(&image, BASE, 7, WarImage::frame(), Default::default())
        .unwrap();
    let prepared = pack(&mut store, boundary(7, 9));
    assert!(prepared.row().events.is_empty());
    assert_eq!(prepared.row().outstanding, 1);
    assert_eq!(prepared.row().counts.unwrap().settled, 0);
    store.acknowledged(prepared.publication());
    assert_eq!(store.entry(&ticket).unwrap().state, State::Pending);
    assert!(store.entry(&ticket).unwrap().proof.is_some());
}

#[test]
fn rejected_empty_interval_cannot_lose_old_refusal_on_generation_observation() {
    let mut store = ready(1);
    store.reject("observed identity read failed");
    store.invalidate("world writer entered");
    assert!(store.entries.is_empty());
    let diagnostic = pack(&mut store, boundary(8, 1));
    assert_eq!(diagnostic.row().generation, 8);
    assert_eq!(diagnostic.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        diagnostic.row().failure.as_deref(),
        Some(b"observed identity read failed".as_slice())
    );
    assert_eq!(diagnostic.row().counts.unwrap().rejected, 1);
    store.acknowledged(diagnostic.publication());
    let next = pack(&mut store, boundary(8, 1));
    assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
    assert_eq!(next.row().counts.unwrap().rejected, 1);
}

#[test]
fn hourly_snapshot_requires_exact_receipts_and_keeps_later_callbacks() {
    let mut store = ready(4);
    let first = acquire(&mut store, 1, 3);
    settle(&mut store, first);
    let mut first = Snapshot::new(4);
    store.snapshot(&mut first, boundary(7, 2)).unwrap();
    let first_pack = first.prepare_pack().unwrap();
    let later = acquire(&mut store, 2, 4);
    settle(&mut store, later);
    let mut later = Snapshot::new(4);
    store.snapshot(&mut later, boundary(7, 3)).unwrap();
    let later_pack = later.prepare_pack().unwrap();
    assert_eq!(first_pack.row().events[0].sequence, 0);
    assert_eq!(later_pack.row().events[0].sequence, 1);
    store.acknowledged(later_pack.publication());
    assert_eq!(store.unacknowledged(), 1);
    assert_eq!(
        store.snapshot(&mut later, boundary(8, 3)),
        Err("Event feed prior world awaits output acceptance")
    );
    store.acknowledged(first_pack.publication());
    store.snapshot(&mut later, boundary(8, 3)).unwrap();
    let next = later.prepare_pack().unwrap();
    assert_eq!(next.row().coverage, EventFeedCoverage::Partial);
    assert!(next.row().events.is_empty());
    assert_eq!(next.row().counts.unwrap().settled, 2);
}

#[test]
fn invalidated_normal_return_settles_without_reading_the_old_war_proof() {
    let image = WarImage::new();
    let mut store = ready(1);
    let ticket = store
        .war_before(&image, BASE, 7, WarImage::frame(), Default::default())
        .unwrap();
    store.invalidate("native reset reached");
    store.world_returned(ticket).unwrap();
    assert_eq!(store.outstanding(), 0);
    assert_eq!((store.counts.settled, store.counts.failed), (1, 1));
    assert!(store.entries[0].proof.is_none());
    assert!(store.retained.is_none());
    let mut snapshot = Snapshot::new(1);
    store.snapshot(&mut snapshot, boundary(8, 9)).unwrap();
    let packed = snapshot.prepare_pack().unwrap();
    assert!(packed.row().events.is_empty());
    assert_eq!(packed.row().generation, 8);
    assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(packed.row().counts.unwrap().failed, 1);
}

#[test]
fn reattachment_restores_counts_without_guessing_unsettled_history() {
    let counts = EventFeedCounts {
        acquired: 20,
        settled: 20,
        failed: 2,
        rejected: 3,
    };
    let mut store = Store::new(Some(40)).unwrap();
    store.restore_counts(counts);
    store.prepare_capacity(1).unwrap();
    let first = pack(&mut store, boundary(7, 0));
    assert_eq!(first.row().counts, Some(counts));
    assert_eq!(first.row().coverage, EventFeedCoverage::Partial);
    store.acknowledged(first.publication());
    let ticket = acquire(&mut store, 1, 2);
    settle(&mut store, ticket);
    let next = pack(&mut store, boundary(7, 2));
    assert_eq!(next.row().events[0].sequence, 41);
    assert_eq!(next.row().counts.unwrap().settled, 21);

    let mut store = Store::new(None).unwrap();
    store.restore_counts(EventFeedCounts {
        acquired: 1,
        ..Default::default()
    });
    let refused = pack(&mut store, boundary(7, 0));
    assert_eq!(refused.row().counts, None);
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    assert!(
        store
            .annex_before(
                &NoReads,
                0,
                7,
                native::AnnexFrame {
                    rbp: 0,
                    r8: 0,
                    r9: 0
                },
                Default::default()
            )
            .is_err()
    );
}

#[test]
fn old_snapshot_receipt_cannot_acknowledge_a_new_refusal() {
    let mut store = ready(1);
    let mut snapshot = Snapshot::new(1);
    store.snapshot(&mut snapshot, boundary(7, 1)).unwrap();
    let old = snapshot.prepare_pack().unwrap();
    store.reject("new callback refused");
    assert!(store.unaccepted_output());
    store.acknowledged(old.publication());
    assert!(store.unaccepted_output());
    store.snapshot(&mut snapshot, boundary(7, 2)).unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        refused.row().failure.as_deref(),
        Some(b"new callback refused".as_slice())
    );
    store.acknowledged(refused.publication());
    assert!(!store.unaccepted_output());
    store.snapshot(&mut snapshot, boundary(7, 3)).unwrap();
    assert_eq!(
        snapshot.prepare_pack().unwrap().row().coverage,
        EventFeedCoverage::Partial
    );
}

#[test]
fn final_facts_remain_unaccepted_until_a_successful_output_receipt() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    assert!(store.unaccepted_output());
    let prepared = pack(&mut store, boundary(7, 2));
    assert!(store.unaccepted_output());
    store.acknowledged(prepared.publication());
    assert!(!store.unaccepted_output());
}

#[test]
fn settlement_of_an_accepted_annex_fact_requires_its_own_output_receipt() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    let first = pack(&mut store, boundary(7, 2));
    store.acknowledged(first.publication());
    settle(&mut store, ticket);
    assert_eq!(store.outstanding(), 0);
    assert!(store.unaccepted_output());
    let settled = pack(&mut store, boundary(7, 3));
    assert!(settled.row().events.is_empty());
    assert_eq!(settled.row().counts.unwrap().settled, 1);
    store.acknowledged(settled.publication());
    assert!(!store.unaccepted_output());
}

#[test]
fn between_tick_event_detach_releases_with_a_named_tail_count() {
    let mut store = ready(1);
    let accepted = store.accepted_revision;
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let losses = store.detach_losses().unwrap();
    assert_eq!(losses.unrecorded_tail_events, 1);
    assert_eq!(losses.unrecorded_tail_revisions, 2);
    assert_eq!(losses.failed_batches, 0);
    assert_eq!(store.accepted_revision, accepted);
    assert_eq!(store.entries[0].publication, None);
}

#[test]
fn known_failed_final_batch_detach_releases_without_acknowledging_it() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let failed = pack(&mut store, boundary(7, 2));
    let accepted = store.accepted_revision;
    assert_eq!(
        store.detach_losses(),
        Err("Event feed output receipt remains uncertain")
    );
    store.discarded(failed.publication());
    let losses = store.detach_losses().unwrap();
    assert_eq!(losses.unrecorded_tail_events, 1);
    assert_eq!(losses.failed_batches, 1);
    assert_eq!(store.accepted_revision, accepted);
    assert_eq!(store.entries[0].state, State::Known);
    store.discarded(failed.publication());
    assert_eq!(store.detach_losses().unwrap(), losses);
}

#[test]
fn late_settlement_and_rejection_detach_count_unaccepted_revisions() {
    for settlement in [false, true] {
        let mut store = ready(1);
        if settlement {
            let ticket = acquire(&mut store, 1, 3);
            let accepted = pack(&mut store, boundary(7, 2));
            store.acknowledged(accepted.publication());
            assert!(store.detach_losses().is_err());
            settle(&mut store, ticket);
        } else {
            store.reject("final input refused");
        }
        let losses = store.detach_losses().unwrap();
        assert_eq!(losses.unrecorded_tail_events, 0);
        assert_eq!(losses.unrecorded_tail_revisions, 1);
        assert!(store.unaccepted_output());
    }
}

#[test]
fn accounting_only_batch_without_a_receipt_retains_detach() {
    let mut store = ready(1);
    let uncertain = pack(&mut store, boundary(7, 1));
    let later = pack(&mut store, boundary(7, 2));
    store.acknowledged(later.publication());
    assert_eq!(store.unacknowledged(), 0);
    assert_eq!(
        store.detach_losses(),
        Err("Event feed output receipt remains uncertain")
    );
    store.discarded(uncertain.publication());
    assert_eq!(store.detach_losses().unwrap().failed_batches, 1);
}

#[test]
fn receipt_storage_covers_the_frame_pool_and_early_recycled_spool_write() {
    let mut store = ready(0);
    let mut receipts = Vec::new();
    for hour in 1..=collector_extended_support::hourly::BUFFERS + 1 {
        receipts.push(pack(&mut store, boundary(7, hour as i32)).publication());
    }
    assert!(store.detach_losses().is_err());
    for receipt in receipts {
        store.acknowledged(receipt);
    }
    assert_eq!(store.detach_losses(), Ok(DetachLosses::default()));
}

#[test]
fn discarded_snapshot_can_be_retried_or_retired_at_a_world_change() {
    for world_changed in [false, true] {
        let mut store = ready(1);
        let ticket = acquire(&mut store, 1, 3);
        settle(&mut store, ticket);
        let failed = pack(&mut store, boundary(7, 2));
        store.discarded(failed.publication());
        let next = pack(&mut store, boundary(if world_changed { 8 } else { 7 }, 3));
        assert_eq!(next.row().coverage, EventFeedCoverage::Unavailable);
        assert_eq!(next.row().events.len(), usize::from(!world_changed));
        assert_eq!(
            next.row().counts.unwrap().rejected,
            u64::from(world_changed)
        );
        store.acknowledged(next.publication());
        assert_eq!(store.unacknowledged(), 0);
        let recovered = pack(&mut store, boundary(if world_changed { 8 } else { 7 }, 4));
        assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    }
}

#[test]
fn short_hourly_snapshot_preserves_facts_for_off_thread_growth() {
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let mut snapshot = Snapshot::new(0);
    assert_eq!(
        store.snapshot(&mut snapshot, boundary(7, 2)),
        Err("Event feed hourly storage needs growth")
    );
    assert_eq!(snapshot.required, 1);
    assert_eq!(store.entries[0].publication, None);
    snapshot.grow(snapshot.required).unwrap();
    store.snapshot(&mut snapshot, boundary(7, 3)).unwrap();
    assert_eq!(snapshot.prepare_pack().unwrap().row().events.len(), 1);
}

#[test]
fn hourly_snapshot_scales_at_1500_and_5000_countries_without_callback_growth() {
    for countries in [1500, 5000] {
        let mut store = ready(countries);
        let mut snapshot = Snapshot::new(countries);
        let capacity = store.capacity();
        for actor in 0..countries {
            let ticket = acquire(&mut store, 10, actor as u32);
            settle(&mut store, ticket);
        }
        store.snapshot(&mut snapshot, boundary(7, 11)).unwrap();
        assert_eq!(store.capacity(), capacity);
        let pack = snapshot.prepare_pack().unwrap();
        assert_eq!(pack.row().events.len(), countries);
        assert_eq!(
            pack.row().events[countries - 1].actor.id,
            countries as u32 - 1
        );
        assert_eq!(pack.row().counts.unwrap().settled, countries as u64);
        assert_eq!(pack.row().coverage, EventFeedCoverage::Complete);
        store.acknowledged(pack.publication());
        assert_eq!(store.unacknowledged(), 0);
    }
}

#[test]
fn worker_snapshot_keeps_first_and_later_mod_sized_events_at_five_thousand_countries() {
    let mut store = ready(17_000);
    let mut snapshot = Snapshot::new(0);
    let mut settled = 0;
    for (hour, events) in [(10, 5000), (12, 12_000)] {
        for index in 0..events {
            let ticket = acquire(&mut store, hour, (index % 5000) as u32);
            settle(&mut store, ticket);
        }
        settled += events;
        store
            .snapshot_growing(&mut snapshot, boundary(7, hour + 1))
            .unwrap();
        let packed = snapshot.prepare_pack().unwrap();
        assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
        assert_eq!(packed.row().events.len(), events);
        assert_eq!(packed.row().counts.unwrap().settled, settled as u64);
        assert_eq!(
            packed.row().events[events - 1].actor.id,
            ((events - 1) % 5000) as u32
        );
        store.acknowledged(packed.publication());
    }
}

const BASE: usize = 0x140000000;
const GAME: usize = 0x1000;
const ACTOR: usize = 0x20000;
const TARGET: usize = 0x30000;
const SELECTED: usize = 0x23000;
const RECIPROCAL: usize = 0x33000;
const CANDIDATE: usize = 0x50000;
const STACK: usize = 0x70000;

#[derive(Default)]
struct WarImage(BTreeMap<usize, u8>);
impl WarImage {
    fn bytes(&mut self, address: usize, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.0.insert(address + offset, *byte);
        }
    }
    fn q(&mut self, address: usize, value: usize) {
        self.bytes(address, &(value as u64).to_le_bytes());
    }
    fn d(&mut self, address: usize, value: u32) {
        self.bytes(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.q(address, data);
        self.d(address + 8, count);
        self.d(address + 12, count);
    }
    fn country(&mut self, pointer: usize, id: u32, instance: u32, diplomacy: usize, matrix: usize) {
        self.q(pointer, BASE + ccountry::VTABLE_RVA);
        self.d(pointer + ccountry::ID, id);
        self.d(pointer + ccountry::ID_INSTANCE, instance);
        self.q(pointer + ccountry::DIPLOMACY_STATUS, diplomacy);
        self.q(diplomacy, BASE + cdiplomacy_status::VTABLE_RVA);
        self.q(diplomacy + cdiplomacy_status::COUNTRY, pointer);
        self.array(diplomacy + cdiplomacy_status::RELATIONS, matrix, 4);
    }
    fn status(&mut self, pointer: usize, diplomacy: usize, target: u32) {
        self.q(pointer, BASE + crelation_status::VTABLE_RVA);
        self.q(pointer + crelation_status::DIPLOMACY_STATUS, diplomacy);
        self.d(pointer + crelation_status::TARGET_COUNTRY, target);
        self.q(pointer + crelation_status::WAR_RELATION, 0);
    }
    fn new() -> Self {
        let mut image = Self::default();
        image.q(BASE + cgame_state::SINGLETON_RVA, GAME);
        image.d(GAME + cgame_state::CURRENT_HOUR, (-17_i32) as u32);
        image.array(GAME + cgame_state::COUNTRIES, 0x4000, 4);
        image.array(GAME + cgame_state::COUNTRY_LINKS, 0x5000, 4);
        for (slot, country) in [0x10000, ACTOR, TARGET, TARGET].into_iter().enumerate() {
            image.q(0x4000 + slot * 8, country);
        }
        for (raw, canonical) in [0, 1, 2, 2].into_iter().enumerate() {
            image.d(0x5000 + raw * 4, canonical);
        }
        image.country(0x10000, 0, 11, 0x11000, 0x12000);
        image.country(ACTOR, 1, 44, 0x21000, 0x22000);
        image.country(TARGET, 2, 55, 0x31000, 0x32000);
        image.q(0x22000 + 2 * 8, SELECTED);
        image.q(0x32000 + 8, RECIPROCAL);
        image.status(SELECTED, 0x21000, 3);
        image.status(RECIPROCAL, 0x31000, 1);
        image.q(CANDIDATE, BASE + cwar_relation::VTABLE_RVA);
        image.d(CANDIDATE + crelation::TYPE_ID, cwar_relation::TYPE);
        image.d(CANDIDATE + crelation::FIRST_COUNTRY, 1);
        image.d(CANDIDATE + crelation::SECOND_COUNTRY, 2);
        image.bytes(CANDIDATE + crelation::CANCEL, &[0, 0]);
        image.q(STACK + 0x1d8, BASE + 0x00d0_baff);
        image.q(STACK + 0x448, BASE + 0x00d0_b57d);
        image.q(STACK + 0x438, SELECTED);
        image.q(STACK + 0x4a8, BASE + 0x00d2_080e);
        image.bytes(BASE + 0x00d0_baf9, &[0xff, 0x90, 0xa8, 0, 0, 0]);
        image.bytes(BASE + 0x00d0_b578, &[0xe8, 0x03, 0x05, 0, 0]);
        image.bytes(BASE + 0x00d2_0809, &[0xe8, 0xc2, 0xab, 0xfe, 0xff]);
        image
    }
    fn attach(&mut self) {
        self.q(SELECTED + crelation_status::WAR_RELATION, CANDIDATE);
        self.q(RECIPROCAL + crelation_status::WAR_RELATION, CANDIDATE);
    }
    fn frame() -> native::WarFrame {
        native::WarFrame {
            rsp_before_call: STACK,
            rcx: SELECTED,
            rdx: CANDIDATE,
            rdi: CANDIDATE,
        }
    }
}
impl Memory for WarImage {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        for (offset, destination) in output.iter_mut().enumerate() {
            let Some(byte) = address
                .checked_add(offset)
                .and_then(|address| self.0.get(&address))
            else {
                return false;
            };
            *destination = *byte;
        }
        true
    }
}

#[test]
fn guarded_war_acquisition_owns_opaque_proof_and_needs_fresh_reciprocal_return() {
    let mut image = WarImage::new();
    let mut store = ready(1);
    let ticket = store
        .war_before(&image, BASE, 7, WarImage::frame(), Default::default())
        .unwrap();
    assert!(matches!(
        store.entry(&ticket).unwrap().proof.as_ref(),
        Some(Proof::War(_))
    ));
    assert_eq!(store.outstanding(), 1);
    image.attach();
    image.d(GAME + cgame_state::CURRENT_HOUR, 9);
    unsafe { store.normal_returned(ticket, 7, &image, BASE).unwrap() };
    assert_eq!(store.outstanding(), 0);
    assert!(store.entries[0].proof.is_none());
    let packed = pack(&mut store, boundary(7, 10));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    let event = &packed.row().events[0];
    assert_eq!(event.source, EventFeedSource::Declaration);
    assert_eq!(event.hour, -17);
    assert_eq!(event.target.reference, 3);
    assert_eq!(event.target.id, 2);
    assert_eq!(event.target.instance, 55);
    assert_eq!(packed.row().counts.unwrap().settled, 1);
}

#[test]
fn guarded_war_failed_postproof_consumes_once_without_recording_a_fact() {
    for invalidate_identity in [false, true] {
        let mut image = WarImage::new();
        let mut store = ready(1);
        let ticket = store
            .war_before(&image, BASE, 7, WarImage::frame(), Default::default())
            .unwrap();
        if invalidate_identity {
            image.attach();
            image.d(TARGET + ccountry::ID_INSTANCE, 56);
        }
        unsafe { store.normal_returned(ticket, 7, &image, BASE).unwrap() };
        assert_eq!(store.outstanding(), 0);
        assert_eq!(store.entries[0].state, State::Failed);
        assert!(store.entries[0].proof.is_none());
        assert_eq!(store.counts.settled, 1);
        assert_eq!(store.counts.failed, 1);
        let packed = pack(&mut store, boundary(7, 2));
        assert_eq!(packed.row().coverage, EventFeedCoverage::Unavailable);
        assert!(packed.row().events.is_empty());
        let expected = if invalidate_identity {
            "event feed country identity changed during native operation"
        } else {
            "event feed candidate reciprocal attachment not proved"
        };
        assert_eq!(packed.row().failure.as_deref(), Some(expected.as_bytes()));
        store.acknowledged(packed.publication());
        let recovery = pack(&mut store, boundary(7, 3));
        assert_eq!(recovery.row().coverage, EventFeedCoverage::Partial);
        assert_eq!(recovery.row().counts.unwrap().settled, 1);
    }
}

fn fact(source: EventFeedSource, actor: u32) -> Fact {
    let operands = operands(5, actor);
    Fact {
        source,
        actor: operands.actor,
        target: operands.target,
        cause: EventFeedCause {
            route: replay_data::event_feed_rows::EventFeedRoute::StateAnnex,
            within: replay_data::event_feed_rows::EventFeedRoute::AnnexEffect,
            state: Some(64),
            ..Default::default()
        },
    }
}

#[test]
fn proved_provenance_events_are_settled_facts_with_their_cause() {
    let mut store = ready(4);
    let ticket = acquire(&mut store, 5, 3);
    store
        .known(7, 5, fact(EventFeedSource::StateTransfer, 4))
        .unwrap();
    store
        .known(7, 5, fact(EventFeedSource::WarEnded, 5))
        .unwrap();
    // The pending annex stays outstanding; the two known events settled.
    assert_eq!(store.outstanding(), 1);
    let packed = pack(&mut store, boundary(7, 5));
    let row = packed.row();
    row.validate().unwrap();
    assert_eq!(
        row.counts.map(|counts| (counts.acquired, counts.settled)),
        Some((3, 2))
    );
    let sources: Vec<_> = row.events.iter().map(|event| event.source).collect();
    assert_eq!(
        sources,
        [
            EventFeedSource::Annex,
            EventFeedSource::StateTransfer,
            EventFeedSource::WarEnded
        ]
    );
    let Recorded::Value(cause) = row.events[1].cause else {
        panic!("a provenance event records its cause");
    };
    assert_eq!(cause.state, Some(64));
    assert_eq!(row.events[1].actor.id, 4);
    store.acknowledged(packed.publication());
    settle(&mut store, ticket);
    // Storage is the callback's: a full store refuses by name.
    store
        .known(7, 5, fact(EventFeedSource::Release, 6))
        .unwrap();
    assert_eq!(
        store.known(7, 5, fact(EventFeedSource::Release, 7)).err(),
        Some("Event feed callback storage needs growth")
    );
    // An unobserved world refuses too.
    store.invalidate("world replaced");
    assert_eq!(
        store.known(7, 5, fact(EventFeedSource::Release, 8)).err(),
        Some("Event feed world observation unavailable")
    );
}

#[test]
fn certified_intervals_have_no_unconditional_decision_family_refusals() {
    let mut store = ready(8);
    for hour in [1, 2] {
        let output = pack(&mut store, boundary(7, hour));
        assert_eq!(output.row().script_refusals, 0);
        assert_eq!(output.row().refused_script_families().count(), 0);
        assert_eq!(output.row().coverage, EventFeedCoverage::Complete);
        assert!(output.row().events.is_empty());
        store.acknowledged(output.publication());
    }
}
