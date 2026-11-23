use super::*;
use engine_native::{
    ccountry,
    cgame_state,
    crelation::{self, status as crelation_status},
    cwar_relation,
};
use std::collections::BTreeMap;

struct NoReads;
impl Memory for NoReads {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("owned event publication must not reread native operands")
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
        token: engine_native_memory::msvc_string::Descriptor::from_header(&NoReads, 0, header).unwrap(),
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
    image.q(scope, BASE + engine_events::cevent_scope::VTABLE_RVA.0);
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
    let _globals = crate::event_feed_observer::test_globals();
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
        .snapshot(&mut snapshot, boundary(7, 2))
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
    assert_eq!(store.tokens_used, 33);
    assert_eq!(row.script_tokens, b"synthetic.1");
    assert!(row.events.iter().all(|event| {
        let script = event.script.unwrap();
        (script.token_start, script.token_len) == (0, 11)
    }));
    store.acknowledged(first.publication());
    store
        .snapshot(&mut snapshot, boundary(7, 3))
        .unwrap();
    assert!(snapshot.prepare_pack().unwrap().row().events.is_empty());
    assert_eq!(store.tokens_used, 0);
    assert_eq!(store.scopes_used, 0);
    store
        .script_known(&image, BASE, 7, 3, script_fact(300, 0x80000, None))
        .unwrap();
    assert_eq!(first.row().events[2].script.unwrap().option, Some(-1));
    assert_eq!(first.row().script_scopes[0].country, 0);
}

#[test]
fn script_scopes_record_the_native_country_and_names_each_reference_resolved_to() {
    let _globals = crate::event_feed_observer::test_globals();
    use engine_events::cevent_scope::{COUNTRY_TAG, FROM, PREV, ROOT, SIZE, VTABLE_RVA};
    crate::country_input_changes::observe_names();
    let mut store = ready(4);
    store.prepare_scripts(64, 6, 2).unwrap();
    let mut image = WarImage::new();
    for (offset, text) in [
        (ccountry::NAME, "Reich"),
        (ccountry::DEFINITE_NAME, "the Reich"),
        (ccountry::ADJECTIVE, "German"),
    ] {
        image.bytes(ACTOR + offset, &[0; 32]);
        image.bytes(ACTOR + offset, text.as_bytes());
        image.q(ACTOR + offset + 16, text.len());
        image.q(ACTOR + offset + 24, 15);
    }
    let (this, from) = (0x80000, 0x81000);
    for (scope, country, links) in [(this, 3, [this, from, this]), (from, 1, [from; 3])] {
        image.bytes(scope, &[0; SIZE]);
        image.q(scope, BASE + VTABLE_RVA.0);
        image.d(scope + COUNTRY_TAG, country);
        for (offset, link) in [ROOT, FROM, PREV].into_iter().zip(links) {
            image.q(scope + offset, link);
        }
    }
    store
        .script_known(&image, BASE, 7, 1, script_fact(100, this, None))
        .unwrap();
    image.d(this + COUNTRY_TAG, 9);
    store
        .script_known(&image, BASE, 7, 1, script_fact(200, this, None))
        .unwrap();
    image.d(this + COUNTRY_TAG, 1);
    let rebuild = crate::country_input_changes::rebuilding_name();
    store
        .script_known(&image, BASE, 7, 1, script_fact(300, this, None))
        .unwrap();
    drop(rebuild);
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot(&mut snapshot, boundary(7, 2))
        .unwrap();
    let packed = snapshot.prepare_pack().unwrap();
    let names = |scope: &EventFeedScope| {
        scope
            .country_names
            .map(|texts| texts.map(|text| String::from_utf8(text.as_bytes().to_vec()).unwrap()))
    };
    let reich = Some(["Reich", "the Reich", "German"].map(String::from));
    assert_eq!(
        packed
            .row()
            .script_scopes
            .iter()
            .map(|scope| (scope.country, scope.country_id, names(scope)))
            .collect::<Vec<_>>(),
        [
            (3, Some(2), None),
            (1, Some(1), reich.clone()),
            (9, None, None),
            (1, Some(1), reich),
            (1, Some(1), None),
            (1, Some(1), None)
        ]
    );
    assert!(
        packed
            .row()
            .script_scopes
            .iter()
            .all(|scope| scope.country_attempted)
    );
}

#[test]
fn script_staging_grows_from_empty_and_only_a_denied_allocation_refuses_without_rereading() {
    let _globals = crate::event_feed_observer::test_globals();
    let identity =
        Some(replay_data::event_feed_rows::EventFeedEventIdentity { kind: 99, id: 123 });

    let mut store = ready(1);
    store.prepare_scripts(0, 0, 0).unwrap();
    store
        .script_known(&script_image(), BASE, 7, 1, script_fact(100, 0x80000, None))
        .unwrap();
    assert_eq!(
        (store.token_demand, store.scope_demand, store.graph_demand),
        (11, 1, 1)
    );
    assert_eq!(store.counts.rejected, 0);
    assert_eq!(store.entries.len(), 1);
    let script = store.entries[0].event.script.unwrap();
    assert_eq!(script.native_event, identity);
    assert!(!script.token_unrecorded && !script.scope_unrecorded);
    assert_eq!((script.token_start, script.token_len), (0, 11));
    assert_eq!((script.scope_start, script.scope_len), (0, 1));
    let mut snapshot = Snapshot::new(0);
    store.snapshot(&mut snapshot, boundary(7, 1)).unwrap();
    let grown = snapshot.prepare_pack().unwrap();
    assert_eq!(grown.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(grown.row().script_tokens, b"synthetic.1");
    assert_eq!(grown.row().script_scopes.len(), 1);
    assert_eq!(grown.row().counts.unwrap().rejected, 0);

    let mut store = ready(1);
    store.prepare_scripts(0, 0, 0).unwrap();
    assert_eq!(
        (store.script_tokens.capacity(), store.scope_nodes.capacity()),
        (0, 0)
    );
    assert!(store.entries.len() < store.capacity());
    let known = crate::test_support::with_allocations_denied(|| {
        store.script_known(&NoReads, BASE, 7, 1, script_fact(100, 0x80000, None))
    });
    known.unwrap();
    assert_eq!(store.counts.rejected, 2);
    assert_eq!(store.failure, Some("event feed script token allocation failed"));
    assert_eq!(
        (store.token_demand, store.scope_demand, store.graph_demand),
        (11, 0, 1)
    );
    assert_eq!(store.entries.len(), 1);
    let script = store.entries[0].event.script.unwrap();
    assert_eq!(script.native_event, identity);
    assert!(script.token_unrecorded && script.scope_unrecorded);
    assert_eq!((script.token_len, script.scope_len), (0, 0));
    let mut snapshot = Snapshot::new(0);
    store.snapshot(&mut snapshot, boundary(7, 1)).unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(refused.row().counts.unwrap().rejected, 2);
}

#[test]
fn script_continuous_occurrences_reclaim_acknowledged_arena_spans() {
    let _globals = crate::event_feed_observer::test_globals();
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
            .snapshot(&mut snapshot, boundary(7, hour))
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
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(8);
    store.prepare_delayed(5001).unwrap();
    store.registry_state = Some(GAME);
    let country = engine_native_memory::country_identity::Country {
        pointer: 0x80000,
        canonical_slot: 4999,
        id: 5000,
    };
    let mut image = WarImage::default();
    image.bytes(
        country.pointer + engine_native::ccountry::DELAYED_EVENTS,
        &[0; 16],
    );
    image.d(
        country.pointer + engine_native::ccountry::SURRENDER_TARGET,
        88,
    );
    store.recover_delayed(&image, BASE, 7, GAME, 1, country);
    store.recover_delayed(&NoReads, BASE, 7, GAME, 1, country);
    let mut snapshot = Snapshot::new(8);
    store
        .snapshot(&mut snapshot, boundary(7, 2))
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
    let _globals = crate::event_feed_observer::test_globals();
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
    image.q(definition, BASE + engine_decisions::cevent::VTABLE_RVA.0);
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
    let _globals = crate::event_feed_observer::test_globals();
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

    store
        .prepare_scripts(
            store.script_tokens.len(),
            store.scopes_used + 1,
            store.scope_nodes.len(),
        )
        .unwrap();
    let used = store.tokens_used;
    tight_tokens(&mut store, used);
    let missed = crate::test_support::with_allocations_denied(|| {
        store.script_known(&image, BASE, 7, 2, script_fact(0, 0x80000, None))
    });
    missed.unwrap();
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let lost = snapshot.prepare_pack().unwrap();
    assert_eq!(lost.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(lost.row().counts.unwrap().rejected, 1);
    state.observe(lost.row()).unwrap();
    assert!(state.pending().is_err());
    assert!(state.fired().is_err());
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(8);
    let mut image = retained_registry_image(1);
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    store.acknowledged(snapshot.publication());
    let mut unavailable = boundary(7, 2);
    unavailable.failure = Some("synthetic boundary observation unavailable");
    store.snapshot(&mut snapshot, unavailable).unwrap();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &NoReads, BASE)
        .unwrap();
    let waiting = snapshot.prepare_pack().unwrap();
    assert_eq!(waiting.row().coverage, EventFeedCoverage::Unavailable);
    state.observe(waiting.row()).unwrap();
    store.acknowledged(refused.publication());
    store.acknowledged(waiting.publication());

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
    let _globals = crate::event_feed_observer::test_globals();
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
    store.snapshot(&mut snapshot, unavailable).unwrap();
    let refused = snapshot.prepare_pack().unwrap();
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
fn a_failed_delayed_baseline_recovers_an_empty_queue_after_native_removal() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    image.array(TARGET + ccountry::DELAYED_EVENTS, 0xc0000, 1);
    image.q(0xc0000, 0);
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let refused = census_hour(&mut store, &image, 1, &CENSUS);
    assert_eq!(refusal_markers(refused.row()), (1, 0));
    state.observe(refused.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).is_err());
    store.acknowledged(refused.publication());
    assert!(crate::event_feed_observer::recovery_pending());
    let world = native::provenance::World::new(&image, BASE).unwrap();
    store
        .delayed_removed(&world, &image, BASE, 7, 0xc1000, Some((TARGET, 7)), false)
        .unwrap();
    image.array(TARGET + ccountry::DELAYED_EVENTS, 0, 0);
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let recovered = census_hour(&mut store, &image, 2, &CENSUS);
    assert_eq!(certifying_markers(recovered.row()), (1, 0));
    assert_eq!(refusal_markers(recovered.row()), (0, 0));
    state.observe(recovered.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).unwrap().is_empty());
    assert!(state.decisions(TARGET_OWNER).is_ok());
    assert!(!crate::event_feed_observer::recovery_pending());
}

fn delayed_timer(image: &mut WarImage, pointer: usize, remaining: i32) {
    use engine_events::{cevent_scope, script_event_registry as registry};
    image.q(pointer, BASE + registry::DELAYED_VTABLE_RVA.0);
    image.q(pointer + registry::DELAYED_DEFINITION, 0x90000);
    let scope = pointer + registry::DELAYED_SCOPE;
    image.bytes(scope, &[0; cevent_scope::SIZE]);
    image.q(scope, BASE + cevent_scope::VTABLE_RVA.0);
    for offset in [cevent_scope::ROOT, cevent_scope::FROM, cevent_scope::PREV] {
        image.q(scope + offset, scope);
    }
    image.d(pointer + registry::DELAYED_ORIGINATOR, 1);
    image.d(pointer + registry::DELAYED_REMAINING, remaining as u32);
}

fn delayed_queue(image: &mut WarImage, timers: &[usize]) {
    image.array(
        TARGET + ccountry::DELAYED_EVENTS,
        0xc0000,
        timers.len() as u32,
    );
    for (ordinal, timer) in timers.iter().enumerate() {
        image.q(0xc0000 + ordinal * 8, *timer);
    }
}

fn schedule_delayed(store: &mut Store, image: &WarImage, hour: i32) {
    use engine_events::script_event_registry as registry;
    let world = native::provenance::World::new(image, BASE).unwrap();
    let owner = world.country_object(TARGET).unwrap();
    let array = registry::delayed_array(image, TARGET).unwrap();
    let row = registry::delayed_at(image, BASE, array, array.count - 1).unwrap();
    store
        .delayed_stored(&world, image, BASE, 7, hour, owner, row)
        .unwrap();
}

fn delayed_labels(row: &EventFeedHistoryRow) -> Vec<(EventFeedScriptChange, Option<u32>)> {
    row.events
        .iter()
        .filter_map(|event| event.script)
        .filter_map(|script| Some((script.change?, script.instance)))
        .filter(|(change, _)| {
            matches!(
                change,
                EventFeedScriptChange::DelayedBaseline
                    | EventFeedScriptChange::DelayedStored { .. }
                    | EventFeedScriptChange::DelayedReady
            )
        })
        .collect()
}

#[test]
fn delayed_labels_never_alias_across_an_overlap_and_the_owner_waits_for_its_census() {
    let _globals = crate::event_feed_observer::test_globals();
    use EventFeedScriptChange::{DelayedBaseline, DelayedReady, DelayedStored};
    let owner = EventFeedCountry {
        reference: 2,
        id: 2,
        instance: 55,
    };
    let census = engine_native_memory::country_identity::Country {
        pointer: TARGET,
        canonical_slot: 2,
        id: 2,
    };
    let (first, second) = (0xd0000, 0xd1000);
    let mut image = retained_registry_image(1);
    delayed_timer(&mut image, first, 360);
    delayed_timer(&mut image, second, 1);
    let mut store = ready(8);
    store.prepare_scripts(256, 16, 4).unwrap();
    store.prepare_delayed(8).unwrap();
    let mut snapshot = Snapshot::new(0);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();

    delayed_queue(&mut image, &[first]);
    schedule_delayed(&mut store, &image, 1);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &image, BASE)
        .unwrap();
    let scheduled = snapshot.prepare_pack().unwrap();
    assert_eq!(
        delayed_labels(scheduled.row()),
        [(DelayedStored { remaining: 360 }, Some(1))]
    );
    state.observe(scheduled.row()).unwrap();
    assert!(state.pending().is_ok());
    assert!(state.delayed(owner).is_err());
    store.acknowledged(scheduled.publication());

    store.invalidate(crate::event_feed_observer::OBSERVATION_OVERLAP);
    store.reject_many(crate::event_feed_observer::OBSERVATION_OVERLAP, 1);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &NoReads, BASE)
        .unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    state.observe(refused.row()).unwrap();

    image.d(GAME + cgame_state::CURRENT_HOUR, 3);
    delayed_queue(&mut image, &[first, second]);
    schedule_delayed(&mut store, &image, 3);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 3), &NoReads, BASE)
        .unwrap();
    let waiting = snapshot.prepare_pack().unwrap();
    assert_eq!(waiting.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        delayed_labels(waiting.row()),
        [(DelayedStored { remaining: 1 }, Some(2))]
    );
    state.observe(waiting.row()).unwrap();
    store.recover_delayed(&NoReads, BASE, 7, GAME, 3, census);
    store.acknowledged(refused.publication());
    store.acknowledged(waiting.publication());

    image.d(GAME + cgame_state::CURRENT_HOUR, 4);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 4), &image, BASE)
        .unwrap();
    let reseeded = snapshot.prepare_pack().unwrap();
    assert_eq!(reseeded.row().coverage, EventFeedCoverage::Partial);
    state.observe(reseeded.row()).unwrap();
    assert!(state.pending().is_ok());
    store.acknowledged(reseeded.publication());

    image.d(GAME + cgame_state::CURRENT_HOUR, 5);
    delayed_timer(&mut image, first, 24);
    delayed_queue(&mut image, &[second, first]);
    schedule_delayed(&mut store, &image, 5);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 5), &NoReads, BASE)
        .unwrap();
    let rescheduled = snapshot.prepare_pack().unwrap();
    assert_eq!(rescheduled.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(
        delayed_labels(rescheduled.row()),
        [(DelayedStored { remaining: 24 }, Some(3))]
    );
    state.observe(rescheduled.row()).unwrap();
    assert!(state.delayed(owner).is_err());
    store.acknowledged(rescheduled.publication());

    store.recover_delayed(&image, BASE, 7, GAME, 5, census);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 6), &NoReads, BASE)
        .unwrap();
    let recovered = snapshot.prepare_pack().unwrap();
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(
        delayed_labels(recovered.row()),
        [
            (DelayedBaseline, None),
            (DelayedStored { remaining: 1 }, Some(4)),
            (DelayedStored { remaining: 24 }, Some(3)),
            (DelayedReady, None),
        ]
    );
    state.observe(recovered.row()).unwrap();
    assert_eq!(
        state
            .delayed(owner)
            .unwrap()
            .iter()
            .map(|timer| (timer.instance, timer.remaining))
            .collect::<Vec<_>>(),
        [(4, 1), (3, 24)]
    );
    assert_eq!(store.counts.rejected, 1);
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
    let _globals = crate::event_feed_observer::test_globals();
    for capacity in [8, 40] {
        let mut store = ready(capacity);
        let ticket = acquire(&mut store, 1, 3);
        assert_eq!(store.capacity_demand(), 2);
        settle(&mut store, ticket);
        assert_eq!(store.capacity_demand(), 2);
    }
}

#[test]
fn repeated_same_hour_pair_has_distinct_acquisitions_and_later_visibility() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(2);
    let first = acquire(&mut store, -17, 2);
    let second = acquire(&mut store, -17, 2);
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
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(1);
    {
        let _ticket = acquire(&mut store, 1, 5);
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
fn a_callback_burst_past_the_prepared_store_keeps_every_event() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(1);
    let prepared = store.entries.capacity();
    let burst = prepared * 4 + 3;
    let ticket = acquire(&mut store, 1, 3);
    for _ in 1..burst {
        let other = acquire(&mut store, 1, 3);
        settle(&mut store, other);
    }
    settle(&mut store, ticket);
    assert!(store.entries.capacity() >= burst);
    let packed = pack(&mut store, boundary(7, 2));
    assert_eq!(packed.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(packed.row().events.len(), burst);
    assert_eq!(packed.row().counts.unwrap().acquired, burst as u64);
    assert_eq!(packed.row().counts.unwrap().rejected, 0);
}

#[test]
fn a_failed_callback_allocation_refuses_coverage_but_keeps_known_fact_and_requires_partial_recovery()
 {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    let capacity = store.entries.capacity();
    while store.entries.len() < capacity {
        let other = acquire(&mut store, 1, 3);
        settle(&mut store, other);
    }
    let refused = crate::test_support::with_allocations_denied(|| {
        store
            .acquire(
                7,
                |sequence| event(sequence, 7, operands(1, 3), Default::default()),
                Some(Proof::Annex),
            )
            .err()
    });
    assert_eq!(refused, Some(OCCURRENCE_ALLOCATION));
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
fn a_refused_receipt_covers_its_loss_even_with_an_older_healthy_handoff() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(4);
    let healthy = pack(&mut store, boundary(7, 1));
    store.reject("synthetic storage refusal");
    let refused = pack(&mut store, boundary(7, 2));
    assert_eq!(refused.row().coverage, EventFeedCoverage::Unavailable);
    store.acknowledged(healthy.publication());
    assert_eq!(store.failure, Some("synthetic storage refusal"));
    store.acknowledged(refused.publication());
    assert_eq!(store.failure, None);
    let recovered = pack(&mut store, boundary(7, 3));
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    assert!(recovered.row().failure.is_none());
}

#[test]
fn a_later_loss_advances_the_required_refused_receipt_past_an_older_handoff() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(4);
    let healthy = pack(&mut store, boundary(7, 1));
    store.reject("synthetic first refusal");
    let first = pack(&mut store, boundary(7, 2));
    store.invalidate("synthetic newer overlap");
    store.reject("synthetic newer overlap");
    let newer = pack(&mut store, boundary(7, 3));
    store.acknowledged(healthy.publication());
    store.acknowledged(first.publication());
    assert!(store.failure.is_some());
    store.acknowledged(newer.publication());
    assert_eq!(store.failure, None);
    let recovered = pack(&mut store, boundary(7, 4));
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
}

#[path = "recovery_tests.rs"]
mod recovery_tests;

#[test]
fn the_first_refused_publication_covers_repeated_seals_without_a_new_loss() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(4);
    let healthy = pack(&mut store, boundary(7, 1));
    store.reject("synthetic refusal");
    let first = pack(&mut store, boundary(7, 2));
    let tail = pack(&mut store, boundary(7, 3));
    store.acknowledged(healthy.publication());
    assert!(store.failure.is_some());
    store.acknowledged(first.publication());
    assert_eq!(store.failure, None);
    store.acknowledged(tail.publication());
    assert_eq!(
        pack(&mut store, boundary(7, 4)).row().coverage,
        EventFeedCoverage::Partial
    );
}

#[test]
fn a_post_release_seal_decline_supersedes_an_older_healthy_handoff() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(4);
    let healthy = pack(&mut store, boundary(7, 1));
    let sealed = pack(&mut store, boundary(7, 2));
    store.seal_declined(sealed.publication(), 1);
    store.acknowledged(healthy.publication());
    assert!(store.failure.is_some());
    store.acknowledged(sealed.publication());
    assert_eq!(store.failure, None);
    let recovered = pack(&mut store, boundary(7, 3));
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    assert!(recovered.row().failure.is_none());
}

#[test]
fn final_facts_remain_unaccepted_until_a_successful_output_receipt() {
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
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
fn a_short_hourly_snapshot_grows_in_the_seal() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(1);
    let ticket = acquire(&mut store, 1, 3);
    settle(&mut store, ticket);
    let mut snapshot = Snapshot::new(0);
    store.snapshot(&mut snapshot, boundary(7, 2)).unwrap();
    assert_eq!(snapshot.required, 1);
    assert_eq!(snapshot.prepare_pack().unwrap().row().events.len(), 1);
}

#[test]
fn hourly_snapshot_needs_no_callback_growth() {
    let _globals = crate::event_feed_observer::test_globals();
    let countries = 6;
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
        self.q(pointer, BASE + ccountry::VTABLE_RVA.0);
        self.d(pointer + ccountry::ID, id);
        self.d(pointer + ccountry::SURRENDER_TARGET, instance);
        self.q(pointer + ccountry::DIPLOMACY_STATUS, diplomacy);
        self.q(diplomacy, BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0);
        self.q(diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY, pointer);
        self.array(diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS, matrix, 4);
    }
    fn status(&mut self, pointer: usize, diplomacy: usize, target: u32) {
        self.q(pointer, BASE + crelation_status::VTABLE_RVA.0);
        self.q(pointer + crelation_status::DIPLOMACY_STATUS, diplomacy);
        self.d(pointer + crelation_status::TARGET_COUNTRY, target);
        self.q(pointer + crelation_status::WAR_RELATION, 0);
    }
    fn new() -> Self {
        let mut image = Self::default();
        image.q(BASE + cgame_state::SINGLETON_RVA.0, GAME);
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
        image.q(CANDIDATE, BASE + cwar_relation::VTABLE_RVA.0);
        image.d(CANDIDATE + crelation::TYPE_ID, cwar_relation::TYPE);
        image.d(CANDIDATE + crelation::FIRST_COUNTRY, 1);
        image.d(CANDIDATE + crelation::SECOND_COUNTRY, 2);
        image.bytes(CANDIDATE + crelation::CANCEL, &[0, 0]);
        image.q(STACK + 0x1d8, BASE + 0x00d8_1bff);
        image.q(STACK + 0x448, BASE + 0x00d8_167d);
        image.q(STACK + 0x438, SELECTED);
        image.q(STACK + 0x4a8, BASE + 0x00d9_8a1e);
        image.bytes(BASE + 0x00d8_1bf9, &[0xff, 0x90, 0xa8, 0, 0, 0]);
        image.bytes(BASE + 0x00d8_1678, &[0xe8, 0x03, 0x05, 0, 0]);
        image.bytes(BASE + 0x00d9_8a19, &[0xe8, 0xb2, 0x8a, 0xfe, 0xff]);
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
    let _globals = crate::event_feed_observer::test_globals();
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
    let _globals = crate::event_feed_observer::test_globals();
    for invalidate_identity in [false, true] {
        let mut image = WarImage::new();
        let mut store = ready(1);
        let ticket = store
            .war_before(&image, BASE, 7, WarImage::frame(), Default::default())
            .unwrap();
        if invalidate_identity {
            image.attach();
            image.d(TARGET + ccountry::ID, 3);
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
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(4);
    let ticket = acquire(&mut store, 5, 3);
    store
        .known(7, 5, fact(EventFeedSource::StateTransfer, 4))
        .unwrap();
    store
        .known(7, 5, fact(EventFeedSource::WarEnded, 5))
        .unwrap();
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
    store
        .known(7, 5, fact(EventFeedSource::Release, 6))
        .unwrap();
    store
        .known(7, 5, fact(EventFeedSource::Release, 7))
        .unwrap();
    while store.entries.len()
        < store
            .entries
            .capacity()
            .saturating_sub(store.owner_controls)
    {
        store
            .known(7, 5, fact(EventFeedSource::Release, 7))
            .unwrap();
    }
    let refused = crate::test_support::with_allocations_denied(|| {
        store.known(7, 5, fact(EventFeedSource::Release, 7)).err()
    });
    assert_eq!(refused, Some(OCCURRENCE_ALLOCATION));
    store.invalidate("world replaced");
    assert_eq!(
        store.known(7, 5, fact(EventFeedSource::Release, 8)).err(),
        Some("Event feed world observation unavailable")
    );
}

#[test]
fn certified_intervals_have_no_unconditional_decision_family_refusals() {
    let _globals = crate::event_feed_observer::test_globals();
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

impl WarImage {
    fn decisions(&mut self, country: usize, status: usize) {
        use engine_decisions::{cdecision_status, decision_runtime};
        self.q(country + ccountry::POLITICAL_STATUS, 0);
        self.q(country + ccountry::DECISION_STATUS, status);
        self.array(country + decision_runtime::IGNORED_TOKENS, 0, 0);
        self.q(status, BASE + cdecision_status::VTABLE_RVA.0);
        self.q(status + cdecision_status::COUNTRY, country);
        for offset in decision_runtime::HEADERS
            .into_iter()
            .chain(decision_runtime::CANDIDATE_HEADERS)
            .chain([cdecision_status::RANDOM_ITEMS])
        {
            self.array(status + offset, 0, 0);
        }
    }
}

const ACTOR_STATUS: usize = 0x24000;
const TARGET_STATUS: usize = 0x34000;
const ACTOR_OWNER: EventFeedCountry = EventFeedCountry {
    reference: 1,
    id: 1,
    instance: 44,
};
const TARGET_OWNER: EventFeedCountry = EventFeedCountry {
    reference: 2,
    id: 2,
    instance: 55,
};
const CENSUS: [engine_native_memory::country_identity::Country; 2] = [
    engine_native_memory::country_identity::Country {
        pointer: ACTOR,
        canonical_slot: 1,
        id: 1,
    },
    engine_native_memory::country_identity::Country {
        pointer: TARGET,
        canonical_slot: 2,
        id: 2,
    },
];

fn owners_image(hour: i32) -> WarImage {
    let mut image = retained_registry_image(hour);
    for (country, status) in [(ACTOR, ACTOR_STATUS), (TARGET, TARGET_STATUS)] {
        image.decisions(country, status);
        image.bytes(country + ccountry::DELAYED_EVENTS, &[0; 16]);
    }
    image
}

struct QueueReads {
    image: WarImage,
    reads: std::cell::RefCell<BTreeMap<usize, usize>>,
}
impl QueueReads {
    fn new(image: WarImage) -> Self {
        Self {
            image,
            reads: Default::default(),
        }
    }
    fn count(&self, address: usize) -> usize {
        self.reads.borrow().get(&address).copied().unwrap_or(0)
    }
    fn hour(&mut self, hour: i32) {
        self.image.d(GAME + cgame_state::CURRENT_HOUR, hour as u32);
    }
}
impl Memory for QueueReads {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if [ACTOR, TARGET].into_iter().any(|country| {
            address == country + ccountry::DELAYED_EVENTS
                || address == country + ccountry::DECISION_STATUS
        }) {
            *self.reads.borrow_mut().entry(address).or_default() += 1;
        }
        self.image.copy(address, output)
    }
}

fn tight_tokens(store: &mut Store, len: usize) {
    let len = len.min(store.script_tokens.len());
    let mut tokens = Vec::with_capacity(len);
    tokens.extend_from_slice(&store.script_tokens[..len]);
    store.script_tokens = tokens;
}

fn census_store(capacity: usize) -> Store {
    let mut store = ready(capacity);
    store.prepare_scripts(4096, 256, 64).unwrap();
    store.prepare_delayed(16).unwrap();
    store.prepare_decisions(16).unwrap();
    store
}

fn census_hour(
    store: &mut Store,
    memory: &impl Memory,
    hour: i32,
    owners: &[engine_native_memory::country_identity::Country],
) -> Prepared {
    let mut snapshot = Snapshot::new(0);
    store
        .prepare_boundary(boundary(7, hour), Some((memory, BASE)))
        .unwrap();
    if let Some(epoch) = store.begin_recovery(7, GAME) {
        for country in owners {
            store.recover_owner(memory, BASE, 7, GAME, hour, *country);
        }
        store.finish_recovery(7, epoch, true);
    }
    store.seal(&mut snapshot, boundary(7, hour)).unwrap();
    snapshot.prepare_pack().unwrap()
}

fn owner_refusals(row: &EventFeedHistoryRow) -> Vec<(EventFeedCountry, Vec<u8>)> {
    row.events
        .iter()
        .filter_map(|event| {
            let script = event.script?;
            let owner = match script.change? {
                EventFeedScriptChange::DelayedRefused => event.target,
                EventFeedScriptChange::Decision(change)
                    if change.operation
                        == replay_data::event_feed_rows::decisions::Operation::Refused =>
                {
                    event.actor
                }
                _ => return None,
            };
            let start = script.token_start as usize;
            let token = &row.script_tokens[start..start + script.token_len as usize];
            Some((owner, token.to_vec()))
        })
        .collect()
}

fn assert_certified(state: &replay_data::event_feed_rows::ScriptEventState) {
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
    for owner in [ACTOR_OWNER, TARGET_OWNER] {
        assert!(state.decisions(owner).is_ok());
        assert!(state.delayed(owner).unwrap().is_empty());
    }
}

#[test]
fn census_owner_markers_are_published_in_the_hour_they_are_read() {
    let _globals = crate::event_feed_observer::test_globals();
    let image = owners_image(5);
    let mut store = census_store(64);
    let row = census_hour(&mut store, &image, 5, &CENSUS);
    assert_eq!(row.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(row.row().read_hour, 5);
    assert!(row.row().events.iter().all(|event| event.hour == 5));
    assert_eq!(row.row().counts.unwrap().rejected, 0);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(row.row()).unwrap();
    assert_certified(&state);
    assert!(!crate::event_feed_observer::recovery_pending());
}

#[test]
fn an_overlap_loss_recovers_registry_and_owners_in_the_first_partial_row_after_its_receipt() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut memory = QueueReads::new(owners_image(1));
    let actor = ACTOR + ccountry::DELAYED_EVENTS;
    let target = TARGET + ccountry::DELAYED_EVENTS;
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let certified = census_hour(&mut store, &memory, 1, &CENSUS);
    state.observe(certified.row()).unwrap();
    assert_certified(&state);
    store.acknowledged(certified.publication());
    assert_eq!((memory.count(actor), memory.count(target)), (1, 1));

    let overlap = |store: &mut Store| {
        store.invalidate(crate::event_feed_observer::OBSERVATION_OVERLAP);
        store.reject_many(crate::event_feed_observer::OBSERVATION_OVERLAP, 1);
    };
    overlap(&mut store);
    let mut receipts = Vec::new();
    for hour in [2, 3] {
        memory.hour(hour);
        let lost = census_hour(&mut store, &memory, hour, &CENSUS);
        assert_eq!(lost.row().coverage, EventFeedCoverage::Unavailable);
        state.observe(lost.row()).unwrap();
        assert!(state.pending().is_err());
        assert!(state.decisions(ACTOR_OWNER).is_err());
        receipts.push(lost.publication());
    }
    for receipt in receipts.drain(..) {
        store.acknowledged(receipt);
    }
    assert_eq!((memory.count(actor), memory.count(target)), (1, 1));
    memory.hour(4);
    let recovered = census_hour(&mut store, &memory, 4, &CENSUS);
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    state.observe(recovered.row()).unwrap();
    assert_certified(&state);
    store.acknowledged(recovered.publication());
    assert_eq!((memory.count(actor), memory.count(target)), (2, 2));
    memory.hour(5);
    let quiet = census_hour(&mut store, &memory, 5, &CENSUS);
    assert_eq!(quiet.row().coverage, EventFeedCoverage::Complete);
    assert!(quiet.row().events.is_empty());
    state.observe(quiet.row()).unwrap();
    assert_certified(&state);
    store.acknowledged(quiet.publication());
    assert_eq!((memory.count(actor), memory.count(target)), (2, 2));

    overlap(&mut store);
    memory.hour(6);
    let lost = census_hour(&mut store, &memory, 6, &CENSUS);
    state.observe(lost.row()).unwrap();
    store.acknowledged(lost.publication());
    memory.hour(7);
    store
        .prepare_boundary(boundary(7, 7), Some((&memory, BASE)))
        .unwrap();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    store.recover_owner(&memory, BASE, 7, GAME, 7, CENSUS[0]);
    overlap(&mut store);
    store.recover_owner(&memory, BASE, 7, GAME, 7, CENSUS[1]);
    store.finish_recovery(7, epoch, true);
    assert!(crate::event_feed_observer::recovery_pending());
    let mut snapshot = Snapshot::new(0);
    store.seal(&mut snapshot, boundary(7, 7)).unwrap();
    let interrupted = snapshot.prepare_pack().unwrap();
    assert_eq!(interrupted.row().coverage, EventFeedCoverage::Unavailable);
    state.observe(interrupted.row()).unwrap();
    assert!(state.decisions(ACTOR_OWNER).is_err());
    store.acknowledged(interrupted.publication());
    assert_eq!((memory.count(actor), memory.count(target)), (3, 2));
    memory.hour(8);
    let recovered = census_hour(&mut store, &memory, 8, &CENSUS);
    assert_eq!(recovered.row().coverage, EventFeedCoverage::Partial);
    state.observe(recovered.row()).unwrap();
    assert_certified(&state);
    assert_eq!((memory.count(actor), memory.count(target)), (4, 3));
    assert!(!crate::event_feed_observer::recovery_pending());
}

#[test]
fn an_unindexed_pass_retirement_refuses_only_its_uncertified_owner() {
    let _globals = crate::event_feed_observer::test_globals();
    use delayed::{CERTIFIED_RETIREMENT_MISSING, UNCERTIFIED_RETIREMENT};
    let empty = EventFeedCountry {
        reference: 0,
        id: 0,
        instance: 11,
    };
    let mut image = retained_registry_image(1);
    image.bytes(0x10000 + ccountry::DELAYED_EVENTS, &[0; 16]);
    delayed_timer(&mut image, 0xd0000, 30);
    delayed_queue(&mut image, &[0xd0000]);
    let mut store = ready(16);
    store.prepare_scripts(256, 16, 4).unwrap();
    store.prepare_delayed(8).unwrap();
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    store.recover_delayed(&image, BASE, 7, GAME, 1, CENSUS[1]);
    store.recover_delayed(
        &image,
        BASE,
        7,
        GAME,
        1,
        engine_native_memory::country_identity::Country {
            pointer: 0x10000,
            canonical_slot: 0,
            id: 0,
        },
    );
    let mut snapshot = Snapshot::new(0);
    store.seal(&mut snapshot, boundary(7, 1)).unwrap();
    let certified = snapshot.prepare_pack().unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(certified.row()).unwrap();
    assert_eq!(state.delayed(TARGET_OWNER).unwrap().len(), 1);
    assert!(state.delayed(empty).unwrap().is_empty());
    store.acknowledged(certified.publication());

    let world = native::provenance::World::new(&image, BASE).unwrap();
    crate::test_support::with_allocations_denied(|| {
        assert_eq!(
            store.delayed_removed(&world, &image, BASE, 7, 0xe0000, Some((ACTOR, 7)), true),
            Ok(())
        );
        assert_eq!(
            store.delayed_removed(&world, &image, BASE, 7, 0xe1000, None, false),
            Ok(())
        );
    });
    store
        .snapshot(&mut snapshot, boundary(7, 2))
        .unwrap();
    let refused = snapshot.prepare_pack().unwrap();
    assert_eq!(refused.row().coverage, EventFeedCoverage::Complete);
    assert!(refused.row().failure.is_none());
    assert_eq!(refused.row().counts.unwrap().rejected, 1);
    assert_eq!(refused.row().events.len(), 1);
    assert_eq!(
        owner_refusals(refused.row()),
        [(ACTOR_OWNER, UNCERTIFIED_RETIREMENT.as_bytes().to_vec())]
    );
    state.observe(refused.row()).unwrap();
    assert!(state.delayed(ACTOR_OWNER).is_err());
    assert_eq!(state.delayed(TARGET_OWNER).unwrap().len(), 1);
    assert!(state.delayed(empty).unwrap().is_empty());
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
    store.acknowledged(refused.publication());

    assert!(
        store
            .delayed_removed(&world, &image, BASE, 7, 0xe0000, Some((0x99000, 7)), true)
            .is_err()
    );
    assert_eq!(
        store.delayed_removed(&world, &image, BASE, 7, 0xe0000, Some((ACTOR, 6)), true),
        Err(UNCERTIFIED_RETIREMENT)
    );
    assert_eq!(
        store.delayed_removed(&world, &image, BASE, 7, 0xe0000, Some((TARGET, 7)), true),
        Err(CERTIFIED_RETIREMENT_MISSING)
    );
    store.reject(CERTIFIED_RETIREMENT_MISSING);
    store
        .snapshot(&mut snapshot, boundary(7, 3))
        .unwrap();
    let global = snapshot.prepare_pack().unwrap();
    assert_eq!(global.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        global.row().failure.as_deref(),
        Some(CERTIFIED_RETIREMENT_MISSING.as_bytes())
    );
    assert!(owner_refusals(global.row()).is_empty());
    state.observe(global.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).is_err());
}

#[test]
fn an_unsupported_decision_owner_retries_until_an_exact_baseline_recovers() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    image.q(TARGET_STATUS, BASE + 1);
    let mut memory = QueueReads::new(image);
    let status = TARGET + ccountry::DECISION_STATUS;
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    for hour in [1, 2] {
        memory.hour(hour);
        let row = census_hour(&mut store, &memory, hour, &CENSUS);
        assert_eq!(refusal_markers(row.row()), (0, usize::from(hour == 1)));
        state.observe(row.row()).unwrap();
        assert!(state.decisions(TARGET_OWNER).is_err());
        assert!(state.delayed(TARGET_OWNER).unwrap().is_empty());
        store.acknowledged(row.publication());
        assert!(crate::event_feed_observer::recovery_pending());
    }
    assert_eq!(memory.count(status), 2);
    memory.image.q(
        TARGET_STATUS,
        BASE + engine_decisions::cdecision_status::VTABLE_RVA.0,
    );
    memory.hour(3);
    let recovered = census_hour(&mut store, &memory, 3, &CENSUS);
    assert_eq!(certifying_markers(recovered.row()), (0, 1));
    assert_eq!(refusal_markers(recovered.row()), (0, 0));
    state.observe(recovered.row()).unwrap();
    assert!(state.decisions(TARGET_OWNER).is_ok());
    assert!(!crate::event_feed_observer::recovery_pending());
}

#[test]
fn certified_owners_recover_without_native_reads() {
    let _globals = crate::event_feed_observer::test_globals();
    let countries = 6;
    let mut store = ready(8);
    store.prepare_delayed(countries).unwrap();
    store.prepare_decisions(countries).unwrap();
    store.registry_state = Some(GAME);
    store.decisions.reset(7);
    let census: Vec<_> = (0..countries)
        .map(|slot| engine_native_memory::country_identity::Country {
            pointer: 0x100000 + slot * 0x2000,
            canonical_slot: slot as u32,
            id: slot as u32 + 1,
        })
        .collect();
    for country in &census {
        store.delayed.countries[country.canonical_slot as usize] = delayed::Country {
            pointer: country.pointer,
            id: country.id,
            ready: true,
            refused: false,
        };
        store.decisions.certify_quiet(
            country.pointer,
            native::CountryIdentity {
                reference: country.id,
                id: country.id,
                instance: 1,
            },
        );
    }
    crate::event_feed_history::recovery_needed();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    for country in &census[..countries / 2] {
        store.recover_owner(&NoReads, BASE, 7, GAME, 1, *country);
    }
    store.finish_recovery(7, epoch, false);
    assert!(crate::event_feed_observer::recovery_pending());
    let epoch = store.begin_recovery(7, GAME).unwrap();
    crate::test_support::with_allocations_denied(|| {
        for country in &census {
            store.recover_owner(&NoReads, BASE, 7, GAME, 1, *country);
        }
    });
    store.finish_recovery(7, epoch, true);
    assert!(!crate::event_feed_observer::recovery_pending());
    assert!(store.entries.is_empty());
    assert_eq!(store.counts.rejected, 0);
}

#[test]
fn dispatch_instances_scan_only_unpublished_entries_and_a_discard_republishes() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut store = ready(4096);
    store.prepare_scripts(11 * 4096, 4096, 1).unwrap();
    let image = script_image();
    for _ in 0..3000 {
        store
            .script_known(&image, BASE, 7, 1, script_fact(100, 0x80000, None))
            .unwrap();
    }
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot(&mut snapshot, boundary(7, 1))
        .unwrap();
    let published = snapshot.prepare_pack().unwrap();
    assert_eq!(store.entries.len(), 3000);
    assert_eq!(store.unpublished_from, 3000);
    store
        .script_known(&image, BASE, 7, 1, script_fact(100, 0x80000, None))
        .unwrap();
    crate::test_support::with_allocations_denied(|| store.script_instance(7, 100, 77));
    assert!(
        store.entries[..3000]
            .iter()
            .all(|entry| entry.event.script.unwrap().instance.is_none())
    );
    assert_eq!(store.entries[3000].event.script.unwrap().instance, Some(77));

    store.discarded(published.publication());
    assert_eq!(store.unpublished_from, 0);
    store
        .snapshot(&mut snapshot, boundary(7, 2))
        .unwrap();
    let retried = snapshot.prepare_pack().unwrap();
    assert_eq!(retried.row().events.len(), 3001);
    assert_eq!(
        retried.row().events[3000].script.unwrap().instance,
        Some(77)
    );
    assert_eq!(store.unpublished_from, 3001);
    store
        .script_known(&image, BASE, 7, 2, script_fact(200, 0x80000, None))
        .unwrap();
    store.script_instance(7, 200, 78);
    assert_eq!(store.entries[3001].event.script.unwrap().instance, Some(78));
}

fn decision_member(image: &mut WarImage, pointer: usize, definition: usize) {
    use engine_decisions::{cdecision, cdecision_cooldown as cooldown, decision_runtime};
    image.q(pointer, BASE + cooldown::VTABLE_RVA.0);
    image.q(pointer + cooldown::DEFINITION, definition);
    image.d(pointer + cooldown::DAYS, 5);
    image.q(definition, BASE + cdecision::VTABLE_RVA.0);
    image.bytes(definition + decision_runtime::PERMANENT, &[0]);
    let mut token = [0; 32];
    token[..10].copy_from_slice(b"cooldown_0");
    token[16..24].copy_from_slice(&10_u64.to_le_bytes());
    token[24..32].copy_from_slice(&15_u64.to_le_bytes());
    image.bytes(definition + cdecision::TOKEN, &token);
}

fn refusal_markers(row: &EventFeedHistoryRow) -> (usize, usize) {
    use replay_data::event_feed_rows::decisions::Operation;
    let mut markers = (0, 0);
    for event in &row.events {
        match event.script.and_then(|script| script.change) {
            Some(EventFeedScriptChange::DelayedRefused) => markers.0 += 1,
            Some(EventFeedScriptChange::Decision(change))
                if change.operation == Operation::Refused =>
            {
                markers.1 += 1
            }
            _ => {}
        }
    }
    markers
}

#[test]
fn repeated_owner_refusals_keep_recovery_pending_without_duplicate_markers() {
    let _globals = crate::event_feed_observer::test_globals();
    use replay_data::event_feed_rows::decisions::{DecisionChange, Operation, REENABLE};
    const MEMBER: usize = 0xf0000;
    let repeated = |registry: OwnerRegistry| {
        REPEATED_OWNER_REFUSALS[registry as usize].load(Ordering::Relaxed)
    };
    let mut image = owners_image(1);
    image.q(TARGET_STATUS, BASE + 1);
    delayed_timer(&mut image, 0xd0000, 30);
    image.q(0xd0000, BASE + 1);
    delayed_queue(&mut image, &[0xd0000]);
    decision_member(&mut image, MEMBER, 0xf1000);
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();

    let failed = census_hour(&mut store, &image, 1, &CENSUS);
    assert_eq!(failed.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(refusal_markers(failed.row()), (1, 1));
    assert_eq!(failed.row().counts.unwrap().rejected, 2);
    state.observe(failed.row()).unwrap();
    assert!(state.decisions(ACTOR_OWNER).is_ok());
    assert!(state.decisions(TARGET_OWNER).is_err());
    assert!(state.delayed(TARGET_OWNER).is_err());
    store.acknowledged(failed.publication());
    assert!(crate::event_feed_observer::recovery_pending());
    let recovery = RECOVERY.load(Ordering::Acquire);
    let delayed_repeats = repeated(OwnerRegistry::Delayed);
    let decision_repeats = repeated(OwnerRegistry::Decisions);

    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let world = native::provenance::World::new(&image, BASE).unwrap();
    store
        .decision_selected(
            &world,
            &image,
            BASE,
            7,
            TARGET,
            MEMBER,
            DecisionChange::new(Operation::Store, REENABLE),
            false,
            (None, None),
        )
        .unwrap();
    crate::test_support::with_allocations_denied(|| {
        for timer in [0xe0000, 0xe1000, 0xe2000] {
            assert_eq!(
                store.delayed_removed(&world, &image, BASE, 7, timer, Some((TARGET, 7)), true),
                Ok(())
            );
        }
        for days in [3, 2] {
            assert_eq!(
                store.decision_result(
                    &world,
                    &image,
                    BASE,
                    7,
                    MEMBER,
                    (None, Some(days)),
                    EventFeedSource::DecisionDurationChanged,
                    "decision result reached an uncertified owner",
                ),
                Ok(true)
            );
        }
    });
    assert!(RECOVERY.load(Ordering::Acquire) > recovery);
    assert!(crate::event_feed_observer::recovery_pending());
    assert_eq!(repeated(OwnerRegistry::Delayed), delayed_repeats + 3);
    assert_eq!(repeated(OwnerRegistry::Decisions), decision_repeats + 2);
    let quiet = census_hour(&mut store, &image, 2, &CENSUS);
    assert_eq!(quiet.row().coverage, EventFeedCoverage::Complete);
    assert_eq!(refusal_markers(quiet.row()), (0, 0));
    let results: Vec<_> = quiet
        .row()
        .events
        .iter()
        .filter(|event| event.source == EventFeedSource::DecisionDurationChanged)
        .map(|event| event.script.unwrap().days)
        .collect();
    assert_eq!(results, [Some(3), Some(2)]);
    assert_eq!(quiet.row().counts.unwrap().rejected, 2);
    state.observe(quiet.row()).unwrap();
    assert!(state.decisions(TARGET_OWNER).is_err());
    assert!(state.delayed(TARGET_OWNER).is_err());
    store.acknowledged(quiet.publication());
    assert!(RECOVERY.load(Ordering::Acquire) > recovery);
    assert!(crate::event_feed_observer::recovery_pending());
    assert_eq!(repeated(OwnerRegistry::Delayed), delayed_repeats + 4);
    assert_eq!(repeated(OwnerRegistry::Decisions), decision_repeats + 3);

    store.invalidate(crate::event_feed_observer::OBSERVATION_OVERLAP);
    store.reject_many(crate::event_feed_observer::OBSERVATION_OVERLAP, 1);
    image.d(GAME + cgame_state::CURRENT_HOUR, 3);
    let lost = census_hour(&mut store, &image, 3, &CENSUS);
    assert_eq!(lost.row().coverage, EventFeedCoverage::Unavailable);
    state.observe(lost.row()).unwrap();
    store.acknowledged(lost.publication());
    image.d(GAME + cgame_state::CURRENT_HOUR, 4);
    let rearmed = census_hour(&mut store, &image, 4, &CENSUS);
    assert_eq!(rearmed.row().coverage, EventFeedCoverage::Partial);
    assert_eq!(refusal_markers(rearmed.row()), (1, 1));
    state.observe(rearmed.row()).unwrap();
    assert!(state.decisions(ACTOR_OWNER).is_ok());
    store.acknowledged(rearmed.publication());
    let world = native::provenance::World::new(&image, BASE).unwrap();
    assert_eq!(
        store.delayed_removed(&world, &image, BASE, 7, 0xe3000, Some((TARGET, 7)), true),
        Ok(())
    );
    assert_eq!(repeated(OwnerRegistry::Delayed), delayed_repeats + 5);
    assert!(crate::event_feed_observer::recovery_pending());
}

fn crossed(sample: usize) -> u64 {
    CROSSED_SAMPLES[sample].load(Ordering::Relaxed)
}

fn certifying_markers(row: &EventFeedHistoryRow) -> (usize, usize) {
    use replay_data::event_feed_rows::decisions::Operation;
    let mut markers = (0, 0);
    for event in &row.events {
        match event.script.and_then(|script| script.change) {
            Some(EventFeedScriptChange::DelayedEmpty | EventFeedScriptChange::DelayedReady) => {
                markers.0 += 1
            }
            Some(EventFeedScriptChange::Decision(change))
                if change.operation == Operation::Ready =>
            {
                markers.1 += 1
            }
            _ => {}
        }
    }
    markers
}

struct CrossingReads {
    image: WarImage,
    at: usize,
    crossed: std::cell::Cell<bool>,
}
impl CrossingReads {
    fn new(image: WarImage, at: usize) -> Self {
        Self {
            image,
            at,
            crossed: Default::default(),
        }
    }
}
impl Memory for CrossingReads {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == self.at && !self.crossed.replace(true) {
            crate::event_feed_observer::writer_moved();
        }
        self.image.copy(address, output)
    }
}

#[test]
fn a_native_original_in_flight_crosses_owner_samples_which_are_retried_when_quiet() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let delayed = crossed(OwnerRegistry::Delayed as usize);
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    let mut snapshot = Snapshot::new(0);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    let mut flight = crate::script_event_observer::Flight::enter();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    store.recover_owner(&image, BASE, 7, GAME, 1, CENSUS[0]);
    store.finish_recovery(7, epoch, true);
    flight.returned();
    drop(flight);
    store.seal(&mut snapshot, boundary(7, 1)).unwrap();
    let row = snapshot.prepare_pack().unwrap();
    assert_eq!(certifying_markers(row.row()), (0, 0));
    assert_eq!(refusal_markers(row.row()), (1, 1));
    assert_eq!(
        owner_refusals(row.row()),
        [
            (ACTOR_OWNER, OWNER_CROSSED.as_bytes().to_vec()),
            (ACTOR_OWNER, OWNER_CROSSED.as_bytes().to_vec()),
        ]
    );
    assert!(row.row().failure.is_none());
    assert_eq!(crossed(OwnerRegistry::Delayed as usize), delayed + 1);
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions + 1);
    state.observe(row.row()).unwrap();
    assert!(state.pending().is_ok());
    assert!(state.decisions(ACTOR_OWNER).is_err());
    assert!(state.delayed(ACTOR_OWNER).is_err());
    store.acknowledged(row.publication());
    assert!(crate::event_feed_observer::recovery_pending());
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let quiet = census_hour(&mut store, &image, 2, &CENSUS[..1]);
    assert_eq!(certifying_markers(quiet.row()), (1, 1));
    state.observe(quiet.row()).unwrap();
    assert!(state.decisions(ACTOR_OWNER).is_ok());
    assert!(state.delayed(ACTOR_OWNER).unwrap().is_empty());
    assert!(!crate::event_feed_observer::recovery_pending());
}

#[test]
fn a_writer_entry_attempt_during_an_owner_sample_refuses_only_that_registry() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    delayed_timer(&mut image, 0xd0000, 30);
    delayed_queue(&mut image, &[0xd0000]);
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let delayed = crossed(OwnerRegistry::Delayed as usize);
    let memory = CrossingReads::new(image, TARGET + ccountry::DELAYED_EVENTS);
    let row = census_hour(&mut store, &memory, 1, &CENSUS[1..]);
    assert_eq!(certifying_markers(row.row()), (0, 1));
    assert_eq!(
        owner_refusals(row.row()),
        [(TARGET_OWNER, OWNER_CROSSED.as_bytes().to_vec())]
    );
    assert_eq!(crossed(OwnerRegistry::Delayed as usize), delayed + 1);
    state.observe(row.row()).unwrap();
    assert!(state.decisions(TARGET_OWNER).is_ok());
    assert!(state.delayed(TARGET_OWNER).is_err());
    store.acknowledged(row.publication());
    let mut image = memory.image;
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let quiet = census_hour(&mut store, &image, 2, &CENSUS[1..]);
    assert_eq!(certifying_markers(quiet.row()), (1, 0));
    state.observe(quiet.row()).unwrap();
    assert_eq!(state.delayed(TARGET_OWNER).unwrap().len(), 1);
}

#[test]
fn a_crossed_registry_sample_certifies_nothing_and_retries_at_the_next_boundary() {
    let _globals = crate::event_feed_observer::test_globals();
    let registry = crossed(REGISTRY_SAMPLE);
    let memory = CrossingReads::new(
        retained_registry_image(1),
        GAME + cgame_state::PENDING_EVENTS,
    );
    let mut store = ready(8);
    store.prepare_scripts(256, 16, 4).unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &memory, BASE)
        .unwrap();
    let crossed_row = snapshot.prepare_pack().unwrap();
    let changes = |row: &EventFeedHistoryRow| -> Vec<_> {
        row.events
            .iter()
            .filter_map(|event| event.script.and_then(|script| script.change))
            .collect()
    };
    assert!(!changes(crossed_row.row()).contains(&EventFeedScriptChange::PendingReady));
    assert!(!changes(crossed_row.row()).contains(&EventFeedScriptChange::FiredReady));
    assert!(crossed_row.row().failure.is_none());
    assert_eq!(crossed_row.row().counts.unwrap().rejected, 0);
    assert_eq!(crossed(REGISTRY_SAMPLE), registry + 1);
    state.observe(crossed_row.row()).unwrap();
    assert!(state.pending().is_err());
    store.acknowledged(crossed_row.publication());
    let mut image = memory.image;
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &image, BASE)
        .unwrap();
    let retried = snapshot.prepare_pack().unwrap();
    assert!(changes(retried.row()).contains(&EventFeedScriptChange::PendingReady));
    assert!(changes(retried.row()).contains(&EventFeedScriptChange::FiredReady));
    state.observe(retried.row()).unwrap();
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
}

#[test]
fn a_storage_short_owner_baseline_is_refused_raises_demand_and_is_retried_without_growth() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    let timers = [0xd0000, 0xd1000, 0xd2000, 0xd3000, 0xd4000];
    for timer in timers {
        delayed_timer(&mut image, timer, 30);
    }
    delayed_queue(&mut image, &timers);
    let mut store = ready(64);
    store.prepare_scripts(4096, 256, 64).unwrap();
    store.prepare_decisions(16).unwrap();
    store.prepare_delayed(4).unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let mut snapshot = Snapshot::new(0);
    store
        .prepare_boundary(boundary(7, 1), Some((&image, BASE)))
        .unwrap();
    let epoch = store.begin_recovery(7, GAME).unwrap();
    for country in CENSUS {
        let work = store.owner_work(&image, BASE, 7, GAME, 1, country);
        let mut prepared =
            recovery::PreparedOwner::read(&image, BASE, 7, GAME, 1, country, work).unwrap();
        crate::test_support::with_allocations_denied(|| {
            store
                .publish_owner(&NoReads, BASE, 7, 1, &mut prepared)
                .unwrap();
        });
    }
    store.finish_recovery(7, epoch, true);
    store.seal(&mut snapshot, boundary(7, 1)).unwrap();
    let short = snapshot.prepare_pack().unwrap();
    assert!(short.row().failure.is_none());
    assert_eq!(
        owner_refusals(short.row()),
        [(TARGET_OWNER, OWNER_STORAGE.as_bytes().to_vec())]
    );
    assert!(store.capacity_demand() >= timers.len());
    state.observe(short.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).is_err());
    assert!(state.decisions(TARGET_OWNER).is_ok());
    assert!(state.delayed(ACTOR_OWNER).unwrap().is_empty());
    store.acknowledged(short.publication());
    assert!(crate::event_feed_observer::recovery_pending());
    store.prepare_delayed(store.capacity_demand()).unwrap();
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let retried = census_hour(&mut store, &image, 2, &CENSUS);
    assert_eq!(certifying_markers(retried.row()), (1, 0));
    state.observe(retried.row()).unwrap();
    assert_eq!(state.delayed(TARGET_OWNER).unwrap().len(), timers.len());
    assert!(!crate::event_feed_observer::recovery_pending());
}

struct FlightReads {
    image: WarImage,
    at: usize,
    fail_always: bool,
    reads: std::cell::Cell<usize>,
    flight: std::cell::RefCell<Option<crate::script_event_observer::Flight>>,
}
impl FlightReads {
    fn new(image: WarImage, at: usize) -> Self {
        Self {
            image,
            at,
            fail_always: false,
            reads: Default::default(),
            flight: Default::default(),
        }
    }
    fn stable(image: WarImage, at: usize) -> Self {
        Self {
            fail_always: true,
            ..Self::new(image, at)
        }
    }
    fn end(&self) {
        if let Some(mut flight) = self.flight.borrow_mut().take() {
            flight.returned();
        }
    }
}
impl Memory for FlightReads {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == self.at {
            self.reads.set(self.reads.get() + 1);
            if self.fail_always {
                return false;
            }
            if self.reads.get() == 1 {
                *self.flight.borrow_mut() = Some(crate::script_event_observer::Flight::enter());
                return false;
            }
        }
        self.image.copy(address, output)
    }
}

fn member_image(hour: i32) -> WarImage {
    use engine_decisions::decision_runtime;
    let mut image = owners_image(hour);
    let header = TARGET_STATUS
        + decision_runtime::HEADERS[replay_data::event_feed_rows::decisions::REENABLE as usize];
    image.array(header, 0xe8000, 1);
    image.q(0xe8000, 0xf0000);
    decision_member(&mut image, 0xf0000, 0xf1000);
    image
}
const MEMBER_DEFINITION: usize = 0xf0000 + engine_decisions::cdecision_cooldown::DEFINITION;

#[test]
fn a_queue_header_changed_by_a_writer_during_acquisition_is_crossed_not_failed() {
    let _globals = crate::event_feed_observer::test_globals();
    struct Appending {
        image: std::cell::RefCell<WarImage>,
        reads: std::cell::Cell<usize>,
    }
    impl Memory for Appending {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == TARGET + ccountry::DELAYED_EVENTS {
                self.reads.set(self.reads.get() + 1);
                if self.reads.get() == 2 {
                    crate::event_feed_observer::writer_moved();
                    delayed_queue(&mut self.image.borrow_mut(), &[0xd0000, 0xd1000]);
                }
            }
            self.image.borrow().copy(address, output)
        }
    }
    let mut image = owners_image(1);
    delayed_timer(&mut image, 0xd0000, 30);
    delayed_timer(&mut image, 0xd1000, 40);
    delayed_queue(&mut image, &[0xd0000]);
    let memory = Appending {
        image: std::cell::RefCell::new(image),
        reads: Default::default(),
    };
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let delayed = crossed(OwnerRegistry::Delayed as usize);
    let row = census_hour(&mut store, &memory, 1, &CENSUS[1..]);
    assert_eq!(certifying_markers(row.row()), (0, 1));
    assert_eq!(
        owner_refusals(row.row()),
        [(TARGET_OWNER, OWNER_CROSSED.as_bytes().to_vec())]
    );
    assert_eq!(crossed(OwnerRegistry::Delayed as usize), delayed + 1);
    state.observe(row.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).is_err());
    store.acknowledged(row.publication());
    assert!(crate::event_feed_observer::recovery_pending());
    memory
        .image
        .borrow_mut()
        .d(GAME + cgame_state::CURRENT_HOUR, 2);
    let quiet = census_hour(&mut store, &memory, 2, &CENSUS[1..]);
    assert_eq!(certifying_markers(quiet.row()), (1, 0));
    state.observe(quiet.row()).unwrap();
    assert_eq!(state.delayed(TARGET_OWNER).unwrap().len(), 2);
}

#[test]
fn a_decision_record_read_failing_during_a_writer_flight_is_crossed_then_certified() {
    let _globals = crate::event_feed_observer::test_globals();
    let memory = FlightReads::new(member_image(1), MEMBER_DEFINITION);
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    let row = census_hour(&mut store, &memory, 1, &CENSUS[1..]);
    memory.end();
    assert_eq!(certifying_markers(row.row()), (0, 0));
    assert_eq!(
        owner_refusals(row.row()),
        [
            (TARGET_OWNER, OWNER_CROSSED.as_bytes().to_vec()),
            (TARGET_OWNER, OWNER_CROSSED.as_bytes().to_vec()),
        ]
    );
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions + 1);
    state.observe(row.row()).unwrap();
    store.acknowledged(row.publication());
    let mut image = memory.image;
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let quiet = census_hour(&mut store, &image, 2, &CENSUS[1..]);
    assert_eq!(certifying_markers(quiet.row()), (1, 1));
    state.observe(quiet.row()).unwrap();
    assert!(state.decisions(TARGET_OWNER).is_ok());
}

#[test]
fn a_registry_read_failing_during_a_writer_flight_retries_at_the_next_boundary() {
    let _globals = crate::event_feed_observer::test_globals();
    let registry = crossed(REGISTRY_SAMPLE);
    let memory = FlightReads::new(
        retained_registry_image(1),
        GAME + cgame_state::PENDING_EVENTS,
    );
    let mut store = ready(8);
    store.prepare_scripts(256, 16, 4).unwrap();
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let mut snapshot = Snapshot::new(0);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 1), &memory, BASE)
        .unwrap();
    memory.end();
    let crossed_row = snapshot.prepare_pack().unwrap();
    assert!(crossed_row.row().failure.is_none());
    assert_eq!(crossed_row.row().counts.unwrap().rejected, 0);
    assert_eq!(crossed(REGISTRY_SAMPLE), registry + 1);
    state.observe(crossed_row.row()).unwrap();
    assert!(state.pending().is_err());
    store.acknowledged(crossed_row.publication());
    let mut image = memory.image;
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    store
        .snapshot_with_registry(&mut snapshot, boundary(7, 2), &image, BASE)
        .unwrap();
    let retried = snapshot.prepare_pack().unwrap();
    state.observe(retried.row()).unwrap();
    assert!(state.pending().is_ok());
    assert!(state.fired().is_ok());
}

#[test]
fn a_read_error_under_an_unbroken_cut_is_retried_with_its_original_reason() {
    let _globals = crate::event_feed_observer::test_globals();
    let memory = FlightReads::stable(member_image(1), MEMBER_DEFINITION);
    let mut store = census_store(64);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    let row = census_hour(&mut store, &memory, 1, &CENSUS[1..]);
    let refusals = owner_refusals(row.row());
    assert_eq!(refusals.len(), 1);
    assert_eq!(refusals[0].0, TARGET_OWNER);
    assert_ne!(refusals[0].1, OWNER_CROSSED.as_bytes());
    assert_ne!(refusals[0].1, OWNER_STORAGE.as_bytes());
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions);
    state.observe(row.row()).unwrap();
    store.acknowledged(row.publication());
    assert_eq!(memory.reads.get(), 1);
    let mut image = memory.image;
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    let memory = FlightReads::stable(image, MEMBER_DEFINITION);
    let quiet = census_hour(&mut store, &memory, 2, &CENSUS[1..]);
    assert_eq!(memory.reads.get(), 1);
    assert!(owner_refusals(quiet.row()).is_empty());
}

fn visit(store: &mut Store, memory: &impl Memory, owners: &[engine_native_memory::country_identity::Country]) {
    let epoch = store.begin_recovery(7, GAME).unwrap();
    for country in owners {
        store.recover_owner(memory, BASE, 7, GAME, 1, *country);
    }
    store.finish_recovery(7, epoch, true);
}
fn sealed_row(store: &mut Store, hour: i32) -> Prepared {
    let mut snapshot = Snapshot::new(0);
    store.seal(&mut snapshot, boundary(7, hour)).unwrap();
    snapshot.prepare_pack().unwrap()
}

#[test]
fn an_early_world_read_failing_during_a_writer_flight_is_crossed_then_certified() {
    let _globals = crate::event_feed_observer::test_globals();
    let quiet = owners_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&quiet, BASE)))
        .unwrap();
    let memory = FlightReads::new(owners_image(1), GAME + cgame_state::CURRENT_HOUR);
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    visit(&mut store, &memory, &CENSUS[..1]);
    memory.end();
    assert_eq!(memory.reads.get(), 1);
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions + 1);
    visit(&mut store, &quiet, &CENSUS[..1]);
    let row = sealed_row(&mut store, 1);
    assert_eq!(
        owner_refusals(row.row()),
        [
            (ACTOR_OWNER, OWNER_CROSSED.as_bytes().to_vec()),
            (ACTOR_OWNER, OWNER_CROSSED.as_bytes().to_vec()),
        ]
    );
    assert_eq!(certifying_markers(row.row()), (1, 1));
}

#[test]
fn an_early_world_read_failing_under_an_unbroken_cut_is_retryable() {
    let _globals = crate::event_feed_observer::test_globals();
    let quiet = owners_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&quiet, BASE)))
        .unwrap();
    let memory = FlightReads::stable(owners_image(1), GAME + cgame_state::CURRENT_HOUR);
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    visit(&mut store, &memory, &CENSUS[..1]);
    visit(&mut store, &memory, &CENSUS[..1]);
    assert_eq!(memory.reads.get(), 2);
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions);
    let row = sealed_row(&mut store, 1);
    let refusals = owner_refusals(row.row());
    assert_eq!(refusals.len(), 1);
    assert_ne!(refusals[0].1, OWNER_CROSSED.as_bytes());
    assert_eq!(certifying_markers(row.row()), (1, 0));
}

#[test]
fn a_political_balance_read_failing_during_a_writer_flight_is_crossed_then_certified() {
    let _globals = crate::event_feed_observer::test_globals();
    let quiet = owners_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&quiet, BASE)))
        .unwrap();
    visit(&mut store, &quiet, &CENSUS[..1]);
    crate::event_feed_history::decisions::POWER_EPOCH.fetch_add(1, Ordering::AcqRel);
    let memory = FlightReads::new(owners_image(1), ACTOR + ccountry::POLITICAL_STATUS);
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    visit(&mut store, &memory, &CENSUS[..1]);
    memory.end();
    assert_eq!(memory.reads.get(), 1);
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions + 1);
    visit(&mut store, &quiet, &CENSUS[..1]);
    let row = sealed_row(&mut store, 1);
    assert_eq!(
        owner_refusals(row.row()),
        [(ACTOR_OWNER, OWNER_CROSSED.as_bytes().to_vec())]
    );
    assert_eq!(certifying_markers(row.row()), (1, 2));
}

#[test]
fn a_political_balance_read_failing_under_an_unbroken_cut_remains_retryable() {
    let _globals = crate::event_feed_observer::test_globals();
    let quiet = owners_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&quiet, BASE)))
        .unwrap();
    visit(&mut store, &quiet, &CENSUS[..1]);
    crate::event_feed_history::decisions::POWER_EPOCH.fetch_add(1, Ordering::AcqRel);
    let memory = FlightReads::stable(owners_image(1), ACTOR + ccountry::POLITICAL_STATUS);
    let decisions = crossed(OwnerRegistry::Decisions as usize);
    for _ in 0..3 {
        visit(&mut store, &memory, &CENSUS[..1]);
    }
    assert_eq!(memory.reads.get(), 3);
    assert_eq!(crossed(OwnerRegistry::Decisions as usize), decisions);
    visit(&mut store, &quiet, &CENSUS[..1]);
    let row = sealed_row(&mut store, 1);
    let refusals = owner_refusals(row.row());
    assert_eq!(refusals.len(), 1);
    assert_ne!(refusals[0].1, OWNER_CROSSED.as_bytes());
    assert_eq!(certifying_markers(row.row()), (1, 2));
}

#[test]
fn a_decline_recorded_during_an_owner_sample_prevents_its_certification() {
    let _globals = crate::event_feed_observer::test_globals();
    struct DeclinedReads(WarImage);
    impl Memory for DeclinedReads {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == TARGET + ccountry::DELAYED_EVENTS {
                crate::event_feed_observer::TEST_UNTAKEN_DECLINES.store(1, Ordering::SeqCst);
            }
            self.0.copy(address, output)
        }
    }
    let quiet = owners_image(1);
    let mut store = census_store(64);
    store
        .prepare_boundary(boundary(7, 1), Some((&quiet, BASE)))
        .unwrap();
    let delayed = crossed(OwnerRegistry::Delayed as usize);
    visit(&mut store, &DeclinedReads(owners_image(1)), &CENSUS[1..]);
    assert_eq!(crossed(OwnerRegistry::Delayed as usize), delayed + 1);
    crate::event_feed_observer::TEST_UNTAKEN_DECLINES.store(0, Ordering::SeqCst);
    visit(&mut store, &quiet, &CENSUS[1..]);
    let row = sealed_row(&mut store, 1);
    assert_eq!(
        owner_refusals(row.row()),
        [(TARGET_OWNER, OWNER_CROSSED.as_bytes().to_vec())]
    );
    assert_eq!(certifying_markers(row.row()), (1, 1));
}

fn token(thread: u32, seq: u32) -> u64 {
    (u64::from(thread) << 32) | u64::from(seq)
}

#[test]
fn detour_witness_arena_keeps_each_frames_order_and_separates_frames() {
    let _globals = crate::event_feed_observer::test_globals();
    use crate::script_event_observer::staging::Witness;
    let mut store = ready(8);
    store.prepare_decisions(4).unwrap();
    let a = token(0x11, 1);
    let b = token(0x22, 1);
    crate::test_support::with_allocations_denied(|| {
        store
            .stage_witness(a, Witness::Selected { instance: 10 })
            .unwrap();
        store
            .stage_witness(b, Witness::Selected { instance: 20 })
            .unwrap();
        store
            .stage_witness(a, Witness::Selected { instance: 11 })
            .unwrap();
    });
    assert_eq!(store.staged_live(), 3);
    assert_eq!(
        store.take_staged(a, None),
        Ok(Some(Witness::Selected { instance: 10 }))
    );
    assert_eq!(
        store.take_staged(a, None),
        Ok(Some(Witness::Selected { instance: 11 }))
    );
    assert_eq!(store.take_staged(a, None), Ok(None));
    assert_eq!(
        store.take_staged(b, None),
        Ok(Some(Witness::Selected { instance: 20 }))
    );
    assert_eq!(store.staged_live(), 0);
}

#[test]
fn a_detour_witness_burst_grows_the_arena_and_only_a_failed_allocation_refuses() {
    let _globals = crate::event_feed_observer::test_globals();
    use crate::script_event_observer::staging::Witness;
    let mut store = ready(8);
    store.prepare_decisions(1).unwrap();
    let a = token(1, 1);
    for instance in 1..=3 {
        store
            .stage_witness(a, Witness::Selected { instance })
            .unwrap();
    }
    assert_eq!(store.staged_live(), 3);
    assert!(store.capacity_demand() >= 3);
    for instance in 1..=3 {
        assert_eq!(
            store.take_staged(a, None),
            Ok(Some(Witness::Selected { instance }))
        );
    }
    let mut instance = 4;
    while store.staged_used < store.witness_capacity() {
        store
            .stage_witness(a, Witness::Selected { instance })
            .unwrap();
        instance += 1;
    }
    let refused = crate::test_support::with_allocations_denied(|| {
        store.stage_witness(a, Witness::Selected { instance })
    });
    assert_eq!(refused, Err(WITNESS_ALLOCATION));
}

#[test]
fn abandoned_detour_witnesses_are_dropped_and_refused_at_the_next_boundary() {
    let _globals = crate::event_feed_observer::test_globals();
    use crate::script_event_observer::staging::Witness;
    let mut store = ready(8);
    store.prepare_decisions(4).unwrap();
    store
        .stage_witness(token(1, 1), Witness::Selected { instance: 1 })
        .unwrap();
    assert_eq!(store.staged_live(), 1);
    let row = pack(&mut store, boundary(7, 1));
    assert_eq!(store.staged_live(), 0);
    assert_eq!(
        row.row().failure.as_deref(),
        Some(WITNESS_ABANDONED.as_bytes())
    );
    assert_eq!(row.row().counts.unwrap().rejected, 1);
    store.acknowledged(row.publication());
    store
        .stage_witness(token(2, 1), Witness::Selected { instance: 2 })
        .unwrap();
    assert_eq!(
        store.take_staged(token(2, 1), None),
        Ok(Some(Witness::Selected { instance: 2 }))
    );
    let clean = pack(&mut store, boundary(7, 2));
    assert!(clean.row().failure.is_none());
}

#[test]
fn a_detour_open_across_a_boundary_reset_refuses_its_interval_by_name() {
    let _globals = crate::event_feed_observer::test_globals();
    use crate::script_event_observer::staging::Witness;
    let mut store = ready(8);
    store.prepare_decisions(4).unwrap();
    let open = token(0x33, 1);
    let epoch = store.staged_epoch();
    store
        .stage_witness(open, Witness::Selected { instance: 1 })
        .unwrap();
    let reset = pack(&mut store, boundary(7, 1));
    assert_eq!(
        reset.row().failure.as_deref(),
        Some(WITNESS_ABANDONED.as_bytes())
    );
    store.acknowledged(reset.publication());
    assert_eq!(store.take_staged(open, Some(epoch)), Err(WITNESS_RESET));
    store.reject(WITNESS_RESET);
    let published = pack(&mut store, boundary(7, 2));
    assert_eq!(published.row().coverage, EventFeedCoverage::Unavailable);
    assert_eq!(
        published.row().failure.as_deref(),
        Some(WITNESS_RESET.as_bytes())
    );
    let fresh = token(0x33, 2);
    let epoch = store.staged_epoch();
    store
        .stage_witness(fresh, Witness::Selected { instance: 2 })
        .unwrap();
    assert_eq!(
        store.take_staged(fresh, Some(epoch)),
        Ok(Some(Witness::Selected { instance: 2 }))
    );
}

#[test]
fn published_removals_and_refusals_read_the_current_recipient_not_the_baseline_metadata() {
    let _globals = crate::event_feed_observer::test_globals();
    let mut image = owners_image(1);
    delayed_timer(&mut image, 0xd0000, 1);
    delayed_queue(&mut image, &[0xd0000]);
    let mut store = census_store(64);
    let baseline = census_hour(&mut store, &image, 1, &CENSUS);
    let timer = baseline
        .row()
        .events
        .iter()
        .find(|event| {
            matches!(
                event.script.and_then(|script| script.change),
                Some(EventFeedScriptChange::DelayedStored { .. })
            )
        })
        .unwrap();
    assert_eq!(timer.target.instance, TARGET_OWNER.instance);
    let label = timer.script.unwrap().instance;
    store.acknowledged(baseline.publication());
    image.d(GAME + cgame_state::CURRENT_HOUR, 2);
    image.d(TARGET + ccountry::SURRENDER_TARGET, 99);
    image.q(TARGET_STATUS, BASE + 1);
    let world = native::provenance::World::new(&image, BASE).unwrap();
    store
        .script_known(
            &image,
            BASE,
            7,
            2,
            delayed::step(&world, &image, TARGET, 1, false).unwrap(),
        )
        .unwrap();
    store
        .delayed_removed(&world, &image, BASE, 7, 0xd0000, Some((TARGET, 7)), true)
        .unwrap();
    store
        .script_known(
            &image,
            BASE,
            7,
            2,
            delayed::step(&world, &image, TARGET, 0, true).unwrap(),
        )
        .unwrap();
    delayed_queue(&mut image, &[]);
    store.decisions.revoke();
    let published = census_hour(&mut store, &image, 2, &CENSUS);
    let removal = published
        .row()
        .events
        .iter()
        .find(|event| {
            matches!(
                event.script.and_then(|script| script.change),
                Some(EventFeedScriptChange::DelayedRemoved { .. })
            )
        })
        .unwrap();
    assert_eq!(removal.target.instance, 99);
    assert_eq!(removal.script.unwrap().instance, label);
    assert_eq!(owner_refusals(published.row())[0].0.instance, 99);
    assert_eq!(timer.target.instance, TARGET_OWNER.instance);
    let mut state = replay_data::event_feed_rows::ScriptEventState::default();
    state.observe(baseline.row()).unwrap();
    state.observe(published.row()).unwrap();
    assert!(state.delayed(TARGET_OWNER).unwrap().is_empty());
    assert!(state.decisions(TARGET_OWNER).is_err());
}
