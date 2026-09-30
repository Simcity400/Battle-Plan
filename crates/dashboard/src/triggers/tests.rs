//! The one evaluator: the children CTrigger::LoadKey (1405434F0) adds, the
//! scopes it enters, and tag aliases (AliasResolve 1407141F0).

use super::*;
use installed_content::{buildings::Buildings, tag_aliases::TagAliases, trigger_keys::TriggerNames};
use replay::{
    frame::{DecodedFrame, Metrics, TableView},
    rows::{
        CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow, FocusRow,
        LexicalTokenRow, ScriptFlagRow, TableId, TableRows, tables,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Countries by (raw link, slot, tag, owned states, original tag,
/// cosmetic tag), every other table complete and empty; flag `x` (token
/// 900) set on the countries `flagged`.
fn frame(countries: &[(u32, u32, &str, i32, &str, &str)], flagged: &[u32]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 1,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
        metrics: Metrics::default(),
    };
    let mut set = |id: TableId, rows: TableRows| {
        *frame.tables[id.index()] = TableView::Complete(rows);
    };
    set(
        TableId::Countries,
        TableRows::from_rows::<tables::Countries>(
            countries
                .iter()
                .map(|(id, slot, tag, ..)| CountryRow {
                    id: *id,
                    canonical_slot: *slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryLinks,
        TableRows::from_rows::<tables::CountryLinks>(
            countries
                .iter()
                .map(|(id, slot, ..)| CountryLinkRow {
                    id: *id,
                    canonical_slot: *slot,
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryCounterInputs,
        TableRows::from_rows::<tables::CountryCounterInputs>(
            countries
                .iter()
                .map(|(id, _, _, owned, ..)| CountryCounterInputRow {
                    country: *id,
                    owned_state_count: *owned,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryAppearance,
        TableRows::from_rows::<tables::CountryAppearance>(
            countries
                .iter()
                .map(|(id, _, _, _, original, cosmetic)| CountryAppearanceRow {
                    country: *id,
                    name_tag: original.as_bytes().to_vec(),
                    cosmetic_tag: cosmetic.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::Focus,
        TableRows::from_rows::<tables::Focus>(
            countries
                .iter()
                .map(|(id, _, tag, ..)| FocusRow {
                    country: *id,
                    tree: (*tag == "FRA").then(|| b"vichy_french_focus".to_vec()),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::LexicalTokens,
        TableRows::from_rows::<tables::LexicalTokens>(vec![LexicalTokenRow {
            token: 900,
            literal: b"x".to_vec(),
        }]),
    );
    set(
        TableId::ScriptFlags,
        TableRows::from_rows::<tables::ScriptFlags>(
            flagged
                .iter()
                .map(|country| ScriptFlagRow {
                    scope_kind: 1,
                    scope_id: *country,
                    token: 900,
                    value: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    frame
}

/// Evaluates `trigger` in the scope of country `slot` over `frame`, with
/// the aliases `aliases` declares.
fn check(frame: &DecodedFrame, aliases: &str, slot: u32, trigger: &str) -> Eval {
    with_world(frame, aliases, Err("none"), |world| {
        let items = installed_content::triggers::read(trigger.as_bytes()).items;
        world.evaluate(&items, Scopes::of(Scope::Country(slot)))
    })
}

/// Runs `run` on the world of `frame` with the aliases `aliases` declares
/// and the character templates `roles`.
fn with_world<T>(
    frame: &DecodedFrame,
    aliases: &str,
    roles: Result<&LeaderRoles, &str>,
    run: impl FnOnce(&World) -> T,
) -> T {
    let aliases: Arc<TagAliases> = Arc::new(installed_content::tag_aliases::parse(&[(
        "common/country_tag_aliases/a.txt",
        aliases.as_bytes(),
    )]));
    let empty = BTreeMap::new();
    let none = BTreeSet::new();
    let impassable = BTreeSet::new();
    let names = TriggerNames::default();
    let buildings = Buildings::default();
    // Lenient, so a test can hold an incomplete table.
    let world = World::read(
        frame,
        ContentFacts {
            state_provinces: &empty,
            impassable: &impassable,
            neighbor_states: Err("none"),
            terrain: None,
            dlcs: &none,
            triggers: &names,
            character_traits: Err("none"),
            tag_aliases: &aliases,
            buildings: &buildings,
            resource_bonuses: Err("none"),
            leader_roles: roles,
            invasion_prepare_days: Err("none"),
        },
    );
    run(&world)
}

fn items(text: &str) -> Vec<installed_content::parser::Item> {
    installed_content::triggers::read(text.as_bytes()).items
}

/// A key that names nothing adds no child (1405434F0): a body of only such
/// keys has none, which the adjacency rules and the raid state selector
/// test (141535130, 1415355E0, 14157A6A0), and an `else` attaches to the
/// last `if` added across it.
#[test]
fn only_the_children_load_key_adds_count() {
    let frame = frame(&countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        assert_eq!(world.has_children(&items("")), Ok(false));
        assert_eq!(
            world.has_children(&items("unregistered_key = yes")),
            Ok(false)
        );
        assert_eq!(
            world.has_children(&items("unregistered_key = yes always = no")),
            Ok(true)
        );
        let scopes = Scopes::of(Scope::Country(0));
        assert_eq!(
            world.evaluate(
                &items(
                    "if = { limit = { always = no } } unregistered_key = yes else = { always = no }"
                ),
                scopes
            ),
            Eval::False
        );
        // An `else` after no `if` is logged and adds nothing.
        assert_eq!(
            world.evaluate(
                &items("unregistered_key = yes else = { always = no }"),
                scopes
            ),
            Eval::True
        );
        // Whether a block key names a character template is unknown
        // without the templates, and so is whether it adds a child.
        assert!(
            world
                .has_children(&items("GER_rommel = { always = no }"))
                .is_err()
        );
    });
}

/// An alias checking every country scans the whole country array
/// (1407141F0, CGameState+0x310): without the complete roster a country it
/// would meet may be missing, so it is refused by name.
#[test]
fn a_check_all_alias_needs_the_whole_roster() {
    let mut frame = frame(&countries(), &[]);
    let aliases = "SEL = { always = yes }";
    assert_eq!(
        check(&frame, aliases, 0, "SEL = { always = yes }"),
        Eval::True
    );
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "not sampled".into(),
    };
    let result = check(&frame, aliases, 0, "SEL = { always = yes }");
    assert!(
        result
            .clone()
            .decided()
            .is_err_and(|why| why.contains("the country roster (CGameState+0x310) is not complete")),
        "{result:?}"
    );
}

/// A country target is set with clearing (14146BB30 -> set_country with
/// 1 -> 14052F3F0), so from a unit leader's
/// scope it carries no character and a character trigger fails its mask
/// (140542CA0); THIS and ROOT copy their scope, so a character they may
/// carry leaves such a trigger unknown.
#[test]
fn a_set_target_carries_nothing_it_was_entered_from() {
    let frame = frame(&countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        let leader = Scopes::of(Scope::Leader((7 << 32) | 1));
        let body = items("NOT = { GER = { has_trait = some_trait } }");
        assert_eq!(world.evaluate(&body, leader), Eval::True);
        let country = Scopes {
            root: Scope::Leader((7 << 32) | 1),
            ..Scopes::of(Scope::Country(0))
        };
        assert!(
            world
                .evaluate(&items("THIS = { has_trait = some_trait }"), country)
                .decided()
                .is_err()
        );
    });
}

/// A character target reads only THIS's country (14146BB30 -> 1406E03C0
/// -> 1410D6BF0): a unit leader as ROOT or PREV does not stop it.
#[test]
fn a_character_target_reads_only_this_country() {
    let frame = frame(&countries(), &[]);
    let roles = LeaderRoles::new(vec![installed_content::characters::LeaderTemplate {
        path: "common/characters/GER.txt".into(),
        tag: "GER_rommel".into(),
        line: None,
        role: Ok(None),
    }]);
    with_world(&frame, "", Ok(&roles), |world| {
        let leader = Scope::Leader((7 << 32) | 1);
        let scopes = Scopes {
            root: leader,
            prev: Some(leader),
            ..Scopes::of(Scope::Country(0))
        };
        // No live character of GER: the empty scope, where `always` holds.
        assert_eq!(
            world.evaluate(&items("GER_rommel = { always = yes }"), scopes),
            Eval::True
        );
    });
}

/// A fallback naming its own alias, or a pair naming each other, is a
/// cycle the game follows without a guard (1407141F0 -> 140536ED0): it is
/// refused by name, never followed until the stack overflows.
#[test]
fn a_fallback_cycle_is_refused_by_name() {
    let frame = frame(&countries(), &[]);
    for aliases in [
        "ALI = { original_tag = GER always = no fallback = ALI }",
        "ALI = { original_tag = GER always = no fallback = BLI }          BLI = { original_tag = SOV always = no fallback = ALI }",
    ] {
        let why = check(&frame, aliases, 0, "tag = ALI")
            .decided()
            .unwrap_err();
        assert!(why.contains("evaluated inside itself"), "{aliases}: {why}");
    }
}

/// AliasResolve (1407141F0) returns as soon as an unscored candidate
/// passes: the targets (140C1C820) after the original-tag candidates are
/// read only if it reaches them, so a target it never reaches cannot make
/// the answer unknown.
#[test]
fn later_candidates_are_read_only_when_reached() {
    let frame = frame(&countries(), &[]);
    let aliases = "ALI = { original_tag = GER country = SB1 } SB1 = { variable = global.foo }";
    assert_eq!(check(&frame, aliases, 0, "tag = ALI"), Eval::True);
    // A target member the port does not follow matters only there too.
    let aliases = "ALI = { original_tag = GER targets_dynamic = yes }";
    assert_eq!(check(&frame, aliases, 0, "tag = ALI"), Eval::True);
    let aliases = "ALI = { original_tag = GER targets_dynamic = yes always = no }";
    let why = check(&frame, aliases, 0, "tag = ALI")
        .decided()
        .unwrap_err();
    assert!(why.contains("targets_dynamic"), "{why}");
    // Repeated original_tag assignments are refused as a counted form.
    let aliases = "ALI = { original_tag = GER original_tag = SOV }";
    let why = check(&frame, aliases, 0, "tag = ALI")
        .decided()
        .unwrap_err();
    assert!(
        why.contains(REFUSED) && why.contains("original_tag 2 times"),
        "{why}"
    );
    // Reached when the original-tag candidates fail.
    let aliases =
        "ALI = { original_tag = GER country = SB1 always = no } SB1 = { variable = global.foo }";
    assert!(check(&frame, aliases, 0, "tag = ALI").decided().is_err());
}

/// Each candidate is a fresh scope (140528800): PREV is the caller, known
/// even where the caller's own PREV is not, and the candidate is set with
/// clearing (set_country 14052DD00), so it carries no character.
#[test]
fn a_candidate_scope_is_fresh() {
    let frame = frame(&countries(), &[]);
    with_world(
        &frame,
        "ALI = { original_tag = GER PREV = { tag = SOV } }",
        Err("none"),
        |world| {
            let caller = Scopes {
                unknown: UnknownLinks {
                    prev: true,
                    ..UnknownLinks::default()
                },
                ..Scopes::of(Scope::Country(3))
            };
            assert_eq!(
                world.evaluate(&items("ALI = { tag = GER }"), caller),
                Eval::True
            );
        },
    );
}

/// Conditionals as CIfTrigger::load_key (141395B90) loads them and
/// CIfTrigger::Evaluate (14138CC00) checks them: a limit loads only into an
/// empty limit; every conditional checks its own limit, an `else` included;
/// an alternative attaches to the last nested `if`, else to its enclosing
/// conditional; LoadKey refuses an alternative after an `else` it attached.
#[test]
fn conditionals_load_as_the_if_loader_does() {
    let frame = frame(&countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        for (body, expected) in [
            // The second limit is skipped: the first holds a child.
            (
                "if = { limit = { always = yes } limit = { always = no } always = no } else = { always = yes }",
                Eval::False,
            ),
            // A first limit that added nothing leaves the second loaded.
            (
                "if = { limit = { unregistered_key = yes } limit = { always = no } always = no } else = { always = yes }",
                Eval::True,
            ),
            // An `else` checks its own limit.
            (
                "if = { limit = { always = no } } else = { limit = { always = no } always = no }",
                Eval::True,
            ),
            // An inner `else` with no nested `if` joins the enclosing chain.
            (
                "if = { limit = { always = no } else = { always = no } }",
                Eval::False,
            ),
            // After a nested `if`, it joins the nested chain.
            (
                "if = { limit = { always = yes } if = { limit = { always = no } } else = { always = no } }",
                Eval::False,
            ),
            // Inside a conditional, alternatives after an `else` still join
            // (their key is 0x2968, never 0x36D3).
            (
                "if = { limit = { always = yes } if = { limit = { always = no } } else = { limit = { always = no } } else = { always = no } }",
                Eval::False,
            ),
            // In a conjunction, an alternative after an attached `else` is
            (
                "if = { limit = { always = no } } else = { limit = { always = no } } else = { always = no }",
                Eval::True,
            ),
        ] {
            assert_eq!(world.evaluate(&items(body), scopes), expected, "{body}");
        }
    });
}

/// A key names a character by the content's character templates (14146DB00
/// -> 1406B0030, by token, so without case), not by the live characters:
/// a template the scope country has no character of leaves the empty scope
/// (14146BB30), where only mask-0 triggers hold.
#[test]
fn a_character_scope_is_named_by_the_templates() {
    let frame = frame(&countries(), &[]);
    let roles = LeaderRoles::new(vec![installed_content::characters::LeaderTemplate {
        path: "common/characters/GER.txt".into(),
        tag: "GER_rommel".into(),
        line: None,
        role: Ok(None),
    }]);
    with_world(&frame, "", Ok(&roles), |world| {
        let scopes = Scopes::of(Scope::Country(0));
        for key in ["GER_rommel", "ger_ROMMEL"] {
            let body = items(&format!("{key} = {{ tag = GER }}"));
            assert_eq!(world.evaluate(&body, scopes), Eval::False, "{key}");
            let body = items(&format!("{key} = {{ always = yes }}"));
            assert_eq!(world.evaluate(&body, scopes), Eval::True, "{key}");
        }
        // No template: the key adds no child.
        assert_eq!(
            world.evaluate(&items("GER_other = { always = no }"), scopes),
            Eval::True
        );
    });
}

/// ROOT, FROM and PREV copy their scope's links (1405296C0), so FROM
/// through FROM is the FROM scope's own FROM, which the recording does not
/// hold; the combat scope's links are itself (1405286D0).
#[test]
fn a_link_through_a_link_is_its_scope_s_own() {
    let frame = frame(&countries(), &[]);
    with_world(&frame, "", Err("none"), |world| {
        let scopes = Scopes {
            from: Some(Scope::Country(1)),
            ..Scopes::of(Scope::Country(0))
        };
        assert_eq!(
            world.evaluate(&items("FROM = { tag = D01 }"), scopes),
            Eval::True
        );
        assert_eq!(
            world.evaluate(&items("GER = { FROM = { tag = D01 } }"), scopes),
            Eval::True
        );
        for body in [
            "FROM = { FROM = { tag = D01 } }",
            "FROM.FROM = { tag = D01 }",
            "ROOT = { FROM = { tag = D01 } }",
        ] {
            assert!(
                world.evaluate(&items(body), scopes).decided().is_err(),
                "{body}"
            );
        }
        assert_eq!(
            world.evaluate(
                &items("FROM = { FROM.FROM = { always = yes } }"),
                Scopes::combat()
            ),
            Eval::True
        );
    });
}

/// GER (slot 0) owns states; D01 and D02 are dynamic countries with GER
/// as their original tag, appended in one recorded frame; SOV and SOT.
fn countries() -> Vec<(u32, u32, &'static str, i32, &'static str, &'static str)> {
    vec![
        (100, 0, "GER", 2, "", ""),
        (101, 1, "D01", 1, "GER", ""),
        (102, 2, "D02", 1, "GER", ""),
        (103, 3, "SOV", 5, "", "SOV_left_opposition"),
        (104, 4, "FRA", 3, "", ""),
    ]
}

/// The first candidate whose triggers pass: the original-tag country, then
/// its dynamic countries (`_CountriesForOriginalTags`); the alias is used as
/// a scope and as a country operand, THIS the candidate and PREV the scope
/// that used it.
#[test]
fn an_alias_is_the_first_candidate_whose_triggers_pass() {
    let aliases = "ALI = { original_tag = GER has_country_flag = x } \
                   PRV = { original_tag = GER PREV = { tag = SOV } }";
    let flagged = frame(&countries(), &[101]);
    assert_eq!(
        check(&flagged, aliases, 3, "ALI = { tag = D01 }"),
        Eval::True
    );
    assert_eq!(check(&flagged, aliases, 1, "tag = ALI"), Eval::True);
    assert_eq!(check(&flagged, aliases, 0, "tag = ALI"), Eval::False);
    // The original-tag country itself comes first.
    let both = frame(&countries(), &[100, 101]);
    assert_eq!(check(&both, aliases, 0, "tag = ALI"), Eval::True);
    // PREV is the scope that used the alias.
    assert_eq!(check(&both, aliases, 3, "PRV = { tag = GER }"), Eval::True);
    assert_eq!(check(&both, aliases, 4, "PRV = { tag = GER }"), Eval::False);
    // No candidate and no fallback: no country, so the scope names nothing.
    let none = frame(&countries(), &[]);
    assert_eq!(
        check(&none, aliases, 0, "ALI = { always = yes }"),
        Eval::False
    );
}

/// Two passing candidates of one recorded interval: their order in
/// `_CountriesForOriginalTags` decides and is unknown, so it is refused by
/// name.
#[test]
fn an_unordered_group_is_refused_where_its_order_decides() {
    let aliases = "ALI = { original_tag = GER has_country_flag = x }";
    let both = frame(&countries(), &[101, 102]);
    match check(&both, aliases, 0, "tag = ALI") {
        Eval::Unknown(reasons) => assert!(
            reasons
                .iter()
                .any(|why| why.contains("tag alias ALI") && why.contains("CGameState+0x988")),
            "{reasons:?}"
        ),
        other => panic!("{other:?}"),
    }
}

/// `country_score` with a modifier: the highest score above 0 wins
/// (CMeanTimeToHappen::Evaluate 1405455A0), a tie keeping the earlier; the
/// fallback only when nothing wins; a check of every country without an
/// original tag or targets; `has_focus_tree` and `has_cosmetic_tag`.
#[test]
fn scores_fallbacks_and_every_country() {
    let aliases = "SOU = { original_tag = SOV country_score = { base = 1 \
                     modifier = { add = 2 has_cosmetic_tag = SOV_left_opposition exists = yes } } } \
                   VIC = { original_tag = FRA has_focus_tree = vichy_french_focus } \
                   NON = { original_tag = FRA has_focus_tree = other fallback = GER } \
                   ANY = { has_country_flag = x } \
                   TGT = { targets = { SOV D02 GER } has_country_flag = x }";
    let frame = frame(&countries(), &[102, 104]);
    assert_eq!(check(&frame, aliases, 3, "tag = SOU"), Eval::True);
    assert_eq!(check(&frame, aliases, 4, "tag = VIC"), Eval::True);
    assert_eq!(check(&frame, aliases, 0, "tag = NON"), Eval::True);
    // Every country in slot order: D02 (slot 2) before FRA (slot 4).
    assert_eq!(check(&frame, aliases, 2, "tag = ANY"), Eval::True);
    // Targets sorted by canonical slot: GER (0), D02 (2), SOV (3).
    assert_eq!(check(&frame, aliases, 2, "tag = TGT"), Eval::True);
}

/// An alias mapped to a variable or event target decides alone, from a
/// store the recording does not hold: refused by name.
#[test]
fn a_variable_alias_is_refused_by_name() {
    let aliases = "SB1 = { variable = SWI.SWI_country_to_appease_1 }";
    let frame = frame(&countries(), &[]);
    match check(&frame, aliases, 0, "SB1 = { always = yes }") {
        Eval::Unknown(reasons) => assert!(
            reasons.iter().any(|why| why.contains("tag alias SB1")),
            "{reasons:?}"
        ),
        other => panic!("{other:?}"),
    }
}
