use super::*;
use crate::script_test_game as game;
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
use std::{cell::RefCell, rc::Rc};

fn faction_frame() -> DecodedFrame {
    let mut frame = frame(
        &[(100, 1, "AAA", 1, "", ""), (200, 2, "GER", 1, "", "")],
        &[],
    );
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                faction: Recorded::Value(Some(5)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![
            FactionMemberRow {
                country: 100,
                faction: 5,
                ordinal: 1,
            },
            FactionMemberRow {
                country: 0,
                faction: 5,
                ordinal: 0,
            },
        ]));
    frame
}

#[test]
fn faction_leader_raw_zero_is_empty_at_the_context_mask_in_flat_and_graph_paths() {
    game::enter();
    let mut frame = faction_frame();
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "masked operand must not be reached".into(),
    };
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        for condition in [
            "faction_leader = { }",
            "faction_leader = { always = yes }",
            "faction_leader = { has_war = yes }",
        ] {
            assert_eq!(
                evaluator.evaluate(&items(condition), Scopes::of(Scope::Country(1))),
                Eval::False,
                "{condition}"
            );
        }
        assert_eq!(
            evaluator.evaluate(
                &items("faction_leader.PREV = { tag = AAA }"),
                Scopes::of(Scope::Country(1))
            ),
            Eval::True
        );

        let graph = Rc::new(RefCell::new(EventScopes::new(vec![
            EventScope {
                root: Link::Scope(1),
                from: Link::Scope(2),
                ..EventScope::of(EventObject::Country(1))
            },
            EventScope::of(EventObject::Country(2)),
            EventScope::of(EventObject::State(64)),
        ])));
        let evaluator = evaluator.with_event_scopes(graph);
        let caller = evaluator.event_scopes(0).unwrap();
        let parsed = content_reader_script::scope::parse("faction_leader", false, world.recorded());
        let entered = evaluator.enter_scope(&parsed, caller).unwrap().unwrap();
        assert_eq!(entered.this, Scope::State(NULL_STATE));
        assert_eq!(entered.root, Scope::Country(2));
        assert_eq!(entered.from, Some(Scope::State(64)));
        assert_eq!(entered.prev, Some(Scope::Country(1)));
        for condition in ["faction_leader = { }", "faction_leader = { always = yes }"] {
            assert_eq!(evaluator.evaluate(&items(condition), caller), Eval::False);
        }
        assert_eq!(
            evaluator.evaluate(&items("faction_leader.PREV = { tag = AAA }"), caller),
            Eval::True
        );
    });
}

#[test]
fn only_known_clearing_carriers_use_the_reserved_faction_fallback() {
    game::enter();
    let mut frame = faction_frame();
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    {
        rows[1].country = 200;
    }
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let parsed = content_reader_script::scope::parse("faction_leader", false, world.recorded());
        for this in [
            Scope::State(NULL_STATE),
            Scope::State(64),
            Scope::Character(7),
            Scope::Combat,
            Scope::Leader(9),
            Scope::IndustrialOrganisation(10),
            Scope::Project { owner: 1, id: 11 },
            Scope::Raid(12),
            Scope::Unit(13),
            Scope::CountryLeader {
                country: 1,
                leader: 9,
            },
        ] {
            assert_eq!(
                evaluator
                    .enter_scope(&parsed, Scopes::of(this))
                    .unwrap()
                    .unwrap()
                    .this,
                Scope::Country(2),
                "{this:?}"
            );
        }
        assert!(
            evaluator
                .enter_scope(&parsed, Scopes::of(Scope::Province(14)))
                .unwrap_err()
                .contains("carried country/faction")
        );
        let mut unknown = Scopes::of(Scope::Country(1));
        unknown.unknown.prev = true;
        let parsed = content_reader_script::scope::parse("PREV.faction_leader", false, world.recorded());
        assert!(evaluator.enter_scope(&parsed, unknown).is_err());
    });

    *frame.tables[TableId::ReservedCountryNaming.index()] = TableView::Unavailable {
        reason: "reserved getter fault".into(),
    };
    with_world(&frame, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let parsed = content_reader_script::scope::parse("faction_leader", false, world.recorded());
        assert_eq!(
            evaluator.enter(&parsed, Scopes::of(Scope::Country(1))),
            Ok(Some(Scope::Country(2)))
        );
        assert!(
            evaluator
                .enter(&parsed, Scopes::empty())
                .unwrap_err()
                .contains("reserved country naming")
        );
    });
}

#[test]
fn faction_leader_predicate_does_not_compare_raw_zero_by_canonical_identity() {
    game::enter();
    let mut frame = faction_frame();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 0;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 0;
    }
    assert_eq!(check(&frame, "", 0, "is_faction_leader = yes"), Eval::False);
    assert_eq!(check(&frame, "", 0, "is_faction_leader = no"), Eval::True);
    with_world(&frame, "", Err("none"), |world| {
        assert_eq!(
            world.evaluate(&items("is_faction_leader = no"), Scopes::empty()),
            Eval::False
        );
    });
    if let TableView::Complete(TableRows::FactionMembers(rows)) =
        &mut *frame.tables[TableId::FactionMembers.index()]
    {
        rows[1].country = 100;
    }
    assert_eq!(check(&frame, "", 0, "is_faction_leader = yes"), Eval::True);
}
