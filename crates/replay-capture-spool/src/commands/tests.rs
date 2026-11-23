use super::{test_support::*, *};
use replay_recording::command_packet::{Kind, test_support::{actor, batch, event}};

#[test]
fn coop_duplicate_names_late_join_reused_key_and_idle_player_stay_separate() {
    let report = analyze(&[session()], &[control(&batch(), 0)], 4).unwrap();
    assert!(report.recorded);
    assert_eq!(report.players.len(), 4);
    let players = report.players;
    assert_eq!(players[0].apm, Some(1.0));
    assert_eq!(players[1].observed_minutes, Some(1.5));
    assert_eq!(players[2].actor.player, players[0].actor.player);
    assert_ne!(players[2].actor.id, players[0].actor.id);
    assert_eq!(players[3].apm, Some(0.0));
    assert!(
        players
            .iter()
            .all(|player| player.confirmed_orders == player.commands)
    );
}

#[test]
fn loss_unresolved_and_unclassified_never_become_valid_zero_rates() {
    let mut source = batch();
    source.events[0].origin = Origin::Unresolved;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert_eq!(report.players[0].unresolved_commands, 1);
    assert_eq!(report.players[0].apm, None);
    source.kinds[0].category = Category::Other;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert_eq!(report.players[0].unclassified_commands, 1);
    assert_eq!(report.players[0].apm, None);
    let report = analyze(&[session()], &[control(&batch(), 1)], 4).unwrap();
    assert!(
        report
            .players
            .iter()
            .all(|player| !player.coverage_complete && player.apm.is_none())
    );
    assert!(!analyze(&[session()], &[], 4).unwrap().recorded);
}

#[test]
fn retries_are_idempotent_and_conflicting_packet_identity_is_rejected() {
    let first = control(&batch(), 0);
    let repeated = analyze(&[session()], &[first.clone(), first.clone()], 4).unwrap();
    assert_eq!(repeated.players[0].commands, 1);
    assert!(analyze(&[session()], &[first, control(&batch(), 1)], 4).is_err());
}

#[test]
fn reattachment_to_one_process_world_and_branch_has_independent_identities() {
    let first = control(&batch(), 0);
    let mut second = first.clone();
    let ControlRecord::PlayerCommands { packet, .. } = &mut second else {
        unreachable!()
    };
    packet.capture += 1;
    let report = analyze(&[session()], &[first, second], 4).unwrap();
    assert_eq!(report.players.len(), 8);
    assert_ne!(report.players[0].capture, report.players[4].capture);
    assert!(report.players.iter().all(|player| player.coverage_complete));
}

#[test]
fn metadata_after_departure_cannot_extend_or_reverse_observed_time() {
    let mut batch = batch();
    batch.actors.truncate(1);
    let mut after = batch.actors[0].clone();
    after.at_qpc = 180;
    batch.actors.push(after);
    batch.events.truncate(1);
    let index = index(&[session()], &[control(&batch, 0)], 4).unwrap();
    assert_eq!(index.analysis.players[0].observed_minutes, Some(1.0));
    assert!(index.timelines[0].measure(100, 100, 180, 160).is_none());
}

#[test]
fn event_outside_coverage_and_unknown_issuer_withhold_rates() {
    let mut source = batch();
    source.spans[0].from_qpc = 120;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert!(
        report
            .players
            .iter()
            .all(|player| !player.coverage_complete)
    );
    source.events[0].actor = 0;
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    assert_eq!(report.unattributed_commands, 1);
    assert!(report.players.iter().all(|player| player.apm.is_none()));
}

#[test]
fn automated_execution_with_a_human_sender_does_not_inflate_player_apm() {
    let mut source = batch();
    source.actors = vec![actor(1, 1, 100)];
    source.departures.clear();
    source.events = vec![event(1, 1, 110)];
    source.kinds.push(Kind {
        id: 2,
        token: 10,
        name: "synthetic automatic plan maintenance".into(),
        category: Category::Automatic,
    });
    for time in 111..=210 {
        let mut automatic = event(1, 1, time);
        automatic.origin = Origin::AutonomousCountry;
        automatic.command_country = Some(9);
        source.events.push(automatic);
        let mut maintenance = event(1, 1, time);
        maintenance.kind = 2;
        source.events.push(maintenance);
    }
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    let player = &report.players[0];
    assert_eq!(player.commands, 1);
    assert_eq!(player.confirmed_orders, 1);
    assert_eq!(player.apm, Some(0.5));
    assert_eq!(player.confirmed_orders_per_minute, Some(0.5));
    assert_eq!(player.categories, BTreeMap::from([(Category::Army, 1)]));
    assert!(player.unresolved_kinds.is_empty());

    let mut ambiguous = event(1, 1, 215);
    ambiguous.origin = Origin::Unresolved;
    source.events.push(ambiguous);
    let report = analyze(&[session()], &[control(&source, 0)], 4).unwrap();
    let player = &report.players[0];
    assert_eq!(player.confirmed_orders, 1);
    assert_eq!(player.unresolved_commands, 1);
    assert_eq!(player.apm, None);
    assert_eq!(player.confirmed_orders_per_minute, Some(0.5));
    assert_eq!(player.categories, BTreeMap::from([(Category::Army, 1)]));
    assert_eq!(player.unresolved_kinds["synthetic movement"], 1);
}
