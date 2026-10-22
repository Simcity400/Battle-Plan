//! The coded triggers and persistent store of an industrial organisation's scope.
use super::carriers::{MIO, OTHER, SIZES, in_organisation, sized};
use super::*;
use installed_content_core::{equipment::Groups, mio_traits::Catalog, parser::Item};
use replay_data::rows::{
    IndustrialOrganisationDetailsRow, IndustrialOrganisationFlag, IndustrialOrganisationRow,
    IndustrialOrganisationTask, IndustrialOrganisationVariable, TableRows,
};

/// An organisation of its own definition, whose `available` reaches itself.
const RECURSIVE: u64 = 10 << 32 | 79;
/// An organisation whose `available` would reach itself past a branch that decides first.
const GUARDED: u64 = 11 << 32 | 79;
/// The attached-policy value of an organisation with no policy attached.
const UNSELECTED: u32 = 0x4c17;
/// The frame's hour: three days and an hour past the day the fixture's flag was set.
const NOW: i32 = engine_native::cdate::HOUR_EPOCH + 1003 * 24 + 1;
/// The hour the fixture's flag was set: day 1000 (year 2, 28 September), hour 5.
const SET: i32 = engine_native::cdate::HOUR_EPOCH + 1000 * 24 + 5;

/// The lexer tokens the fixture's names take.
const TOKENS: [(u32, &str); 13] = [
    (10, "synthetic_project"),
    (11, "synthetic_mio"),
    (20, "first_trait"),
    (21, "second_trait"),
    (22, "rival_trait"),
    (23, "policy_a"),
    (24, "policy_b"),
    (25, "mio_flag"),
    (26, "zero_flag"),
    (27, "zero_parent"),
    (28, "unknown_only"),
    (29, "recursive_mio"),
    (30, "guarded_mio"),
];

fn token(name: &str) -> u32 {
    TOKENS
        .iter()
        .find(|(_, literal)| *literal == name)
        .expect("the fixture names the token")
        .0
}

/// The organisations' definitions and equipment groups, read through the content reader.
const DEFINITIONS: &str = "
    synthetic_mio = {
        equipment_type = { synthetic_group loose_type }
        research_categories = { cat_land }
        available = { has_mio_size > 5 }
        visible = { always = yes }
        initial_trait = { name = INITIAL }
        trait = { token = first_trait }
        trait = {
            token = second_trait
            parent = { traits = { first_trait } }
            mutually_exclusive = { rival_trait }
            visible = { has_mio_size > 4 }
            available = { has_mio_size > 5 }
        }
        trait = { token = rival_trait mutually_exclusive = { first_trait } }
        trait = { token = zero_parent parent = { traits = { first_trait } num_parents_needed = 0 } }
        trait = { token = loop_trait available = { is_mio_trait_available = loop_trait } }
    }
    recursive_mio = {
        available = { is_mio_available = yes }
        visible = { always = no }
    }
    guarded_mio = {
        available = { OR = { always = yes is_mio_available = yes } }
        visible = { always = no }
    }
";
const GROUPS: &str = "synthetic_group = { equipment_type = { grouped_type } }";

/// The definitions and groups as the recorded stack's readers leave them.
fn content() -> (Catalog, Groups) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_organisation_triggers_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in [
        ("common/military_industrial_organization/organizations/a.txt", DEFINITIONS),
        ("common/equipment_groups/g.txt", GROUPS),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let loaded = (
        installed_content_core::mio_traits::read(&layers).unwrap(),
        Groups::load(&layers).unwrap(),
    );
    std::fs::remove_dir_all(&root).unwrap();
    assert!(loaded.0.refusals.is_empty(), "{:?}", loaded.0.refusals);
    loaded
}

/// The details row of `organisation` in `frame`, to edit.
fn details(frame: &mut DecodedFrame, organisation: u64) -> &mut IndustrialOrganisationDetailsRow {
    let TableView::Complete(TableRows::IndustrialOrganisationDetails(rows)) =
        &mut *frame.tables[TableId::IndustrialOrganisationDetails.index()]
    else {
        unreachable!("the fixture holds its details");
    };
    rows.iter_mut()
        .find(|row| row.organisation == organisation)
        .expect("the fixture details the organisation")
}

/// The size fixture at [`NOW`], with the fixture's tokens and a [`RECURSIVE`] organisation.
/// [`MIO`] (size 6) has completed `first_trait` and a token no trait has, allows both
/// policies with `policy_b` attached, holds one empty task slot and two flags set at [`SET`]:
/// `mio_flag` of value 2 and `zero_flag` of value 0. [`OTHER`] (size 5) has none of these.
fn organisations() -> DecodedFrame {
    let mut f = sized(NOW, Some(SIZES));
    *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        TOKENS
            .iter()
            .map(|&(token, literal)| replay_data::rows::LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    ));
    let TableView::Complete(TableRows::IndustrialOrganisations(rows)) =
        &mut *f.tables[TableId::IndustrialOrganisations.index()]
    else {
        unreachable!("the fixture holds its organisations");
    };
    for (ordinal, definition, organisation) in
        [(2, "recursive_mio", RECURSIVE), (3, "guarded_mio", GUARDED)]
    {
        rows.push(IndustrialOrganisationRow {
            country: 101,
            owner: 103,
            ordinal,
            token: token(definition),
            organisation,
            ..Default::default()
        });
    }
    let TableView::Complete(TableRows::IndustrialOrganisationDetails(rows)) =
        &mut *f.tables[TableId::IndustrialOrganisationDetails.index()]
    else {
        unreachable!("the fixture holds its details");
    };
    for organisation in [RECURSIVE, GUARDED] {
        rows.push(IndustrialOrganisationDetailsRow {
            organisation,
            country: 101,
            ..Default::default()
        });
    }
    let mio = details(&mut f, MIO);
    mio.unlocked = vec![token("first_trait"), token("unknown_only")];
    mio.allowed_policies = vec![token("policy_a"), token("policy_b")];
    mio.state.policy = token("policy_b");
    mio.tasks = vec![IndustrialOrganisationTask::default()];
    mio.flags = [("mio_flag", 2), ("zero_flag", 0)]
        .into_iter()
        .map(|(name, value)| IndustrialOrganisationFlag {
            token: token(name),
            set_hour: SET,
            value,
            remaining_days: 0,
        })
        .collect();
    details(&mut f, OTHER).state.policy = UNSELECTED;
    f
}

/// Run `check` on a world of `frame` holding the fixture's definitions and groups.
fn with_organisations(frame: &DecodedFrame, check: impl FnOnce(&World)) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalog, groups) = content();
    let dlcs = BTreeSet::new();
    let triggers = TriggerNames::default();
    let aliases = Arc::default();
    let world = World::read(
        frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    )
    .with_organisation_content(&catalog, &groups);
    check(&world);
}

/// Assert each `(organisation, trigger, expected)`, `None` for undecided.
fn assert_cases(world: &World, cases: &[(u64, &str, Option<bool>)]) {
    for &(organisation, trigger, expected) in cases {
        let value = in_organisation(world, organisation, trigger);
        match expected {
            Some(expected) => assert_eq!(value, Eval::from_bool(expected), "{trigger}"),
            None => assert!(matches!(value, Eval::Unknown(_)), "{trigger}: {value:?}"),
        }
    }
}

/// An organisation's scope reads its own recorded persistent store by exact key, a key it does
/// not hold as absent, and a variable operand of its own. Another organisation's store is its
/// own, and an organisation without a details row leaves the read undecided. A key recorded
/// twice has no value the game's store could hold, and is refused.
#[test]
fn organisation_variables_read_its_own_store() {
    let mut f = organisations();
    details(&mut f, MIO).variables.entries = [
        ("dup", 100_000),
        ("dup", 200_000),
        ("mio_points", 300_000),
        ("size", 900_000),
    ]
    .into_iter()
    .map(|(name, value_raw)| IndustrialOrganisationVariable {
        name: name.as_bytes().to_vec(),
        value_raw,
    })
    .collect();
    with_organisations(&f, |world| {
        assert_cases(
            world,
            &[
                (MIO, "has_variable = mio_points", Some(true)),
                (MIO, "has_variable = MIO_POINTS", Some(false)),
                (MIO, "has_variable = missing", Some(false)),
                (MIO, "check_variable = { mio_points > 2 }", Some(true)),
                (MIO, "check_variable = { mio_points > 3 }", Some(false)),
                // Size 6 against the store's 3.
                (MIO, "has_mio_size > mio_points", Some(true)),
                (MIO, "has_mio_size = mio_points", Some(false)),
                (OTHER, "has_variable = mio_points", Some(false)),
                (MIO + (2 << 32), "has_variable = mio_points", None),
                (MIO, "check_variable = { dup > 0 }", None),
                // `size` is a built-in, computed before the stored key, which is not
                // reproduced; existence asks the stored keys alone.
                (MIO, "check_variable = { size > 1 }", None),
                (MIO, "has_variable = size", Some(true)),
            ],
        );
        // A named organisation entered from its holder reads the same store.
        assert_eq!(
            world.evaluate(
                &items("mio:synthetic_mio = { check_variable = { mio_points = 3 } }"),
                Scopes::of(Scope::Country(1)),
            ),
            Eval::True
        );
    });
}

/// Policy membership is the allowed list's and the attached policy's, by lexer token: a policy
/// allowed but not attached is not active, and the no-selection value matches no policy. A
/// quoted operand's token identity is not reproduced.
#[test]
fn policies_are_allowed_and_attached_tokens() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "has_mio_policy = policy_a", Some(true)),
                (MIO, "has_mio_policy = POLICY_B", Some(true)),
                (MIO, "has_mio_policy = policy_c", Some(false)),
                (OTHER, "has_mio_policy = policy_a", Some(false)),
                (MIO, "has_mio_policy_active = policy_b", Some(true)),
                (MIO, "has_mio_policy_active = policy_a", Some(false)),
                (OTHER, "has_mio_policy_active = policy_b", Some(false)),
                (MIO, "has_mio_policy = \"policy_a\"", None),
            ],
        );
    });
}

/// The definition's token decides identity, for every instance of it; task assignment counts
/// task slots, an empty one included, against yes or no.
#[test]
fn identity_and_task_slots() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "is_military_industrial_organization = synthetic_mio", Some(true)),
                (OTHER, "is_military_industrial_organization = SYNTHETIC_MIO", Some(true)),
                (MIO, "is_military_industrial_organization = recursive_mio", Some(false)),
                (MIO, "is_mio_assigned_to_task = yes", Some(true)),
                (MIO, "is_mio_assigned_to_task = no", Some(false)),
                (OTHER, "is_mio_assigned_to_task = yes", Some(false)),
                (OTHER, "is_mio_assigned_to_task = no", Some(true)),
                // The Boolean reader leaves a scope selector's expectation false.
                (OTHER, "is_mio_assigned_to_task = THIS", Some(true)),
                (MIO, "is_mio_assigned_to_task = ROOT", Some(false)),
            ],
        );
    });
}

/// Trait existence is the definition's first ordinary trait, completed or not, never the
/// initial trait; completion also needs that definition, so a completed token no trait has is
/// not completed.
#[test]
fn trait_existence_and_completion() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "has_mio_trait = second_trait", Some(true)),
                (OTHER, "has_mio_trait = { trait = first_trait show_modifiers = yes }", Some(true)),
                (MIO, "has_mio_trait = initial_trait", Some(false)),
                (MIO, "has_mio_trait = unknown_only", Some(false)),
                (MIO, "has_mio_trait = {}", Some(false)),
                (MIO, "is_mio_trait_completed = first_trait", Some(true)),
                (MIO, "is_mio_trait_completed = second_trait", Some(false)),
                (MIO, "is_mio_trait_completed = unknown_only", Some(false)),
                (OTHER, "is_mio_trait_completed = first_trait", Some(false)),
                (RECURSIVE, "has_mio_trait = first_trait", Some(false)),
            ],
        );
    });
}

/// The completed-trait count compares as an integer trigger: `<` and `>` against a scalar,
/// `=` through its `value` block (`=` inside is equality), and a scalar `=` keeps threshold
/// zero.
#[test]
fn completed_trait_count_is_an_integer_comparison() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "has_mio_number_of_completed_traits > 1", Some(true)),
                (MIO, "has_mio_number_of_completed_traits > 2", Some(false)),
                (MIO, "has_mio_number_of_completed_traits < 3", Some(true)),
                (MIO, "has_mio_number_of_completed_traits = { value = 2 }", Some(true)),
                (MIO, "has_mio_number_of_completed_traits = { value > 2 }", Some(false)),
                (MIO, "has_mio_number_of_completed_traits = { value < 3 }", Some(true)),
                (MIO, "has_mio_number_of_completed_traits = 2", Some(false)),
                (OTHER, "has_mio_number_of_completed_traits = 2", Some(true)),
                (OTHER, "has_mio_number_of_completed_traits = { }", Some(true)),
                (MIO, "has_mio_number_of_completed_traits = { value = 1 value = 2 }", None),
                // A scope selector leaves the threshold zero; it is no number. Size reads it
                // the same way, and its reader has no block form.
                (MIO, "has_mio_size > THIS", Some(true)),
                (MIO, "has_mio_size < event_target:anything", Some(false)),
                (MIO, "has_mio_size = { value > 1 }", None),
                (MIO, "has_mio_number_of_completed_traits > THIS", Some(true)),
                (MIO, "has_mio_number_of_completed_traits = { value = THIS }", Some(false)),
                (OTHER, "has_mio_number_of_completed_traits = { value = FROM }", Some(true)),
            ],
        );
    });
}

/// Trait availability runs its tests in order: the trait, its parents (a group needing none is
/// met by a first uncompleted parent and failed by a first completed one), its exclusions, its
/// `visible`, then its `available`. A completed trait can be available, each check can be
/// turned off, a misspelled check is refused, and a trait reaching itself is refused.
#[test]
fn trait_availability_follows_the_game_order() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "is_mio_trait_available = first_trait", Some(true)),
                (MIO, "is_mio_trait_available = second_trait", Some(true)),
                (OTHER, "is_mio_trait_available = second_trait", Some(false)),
                (
                    OTHER,
                    "is_mio_trait_available = { trait = second_trait check_mio_parent_completed = no }",
                    Some(false),
                ),
                (MIO, "is_mio_trait_available = rival_trait", Some(false)),
                (
                    MIO,
                    "is_mio_trait_available = { trait = rival_trait check_mio_mutually_exclusive = no }",
                    Some(true),
                ),
                (MIO, "is_mio_trait_available = zero_parent", Some(false)),
                (OTHER, "is_mio_trait_available = zero_parent", Some(true)),
                (MIO, "is_mio_trait_available = initial_trait", Some(false)),
                (MIO, "is_mio_trait_available = { trait = rival_trait check_parent = no }", None),
                (MIO, "is_mio_trait_available = loop_trait", None),
            ],
        );
    });
}

/// Equipment types match the definition's list expanded through the groups, group names kept,
/// without case; research categories match the definition's list without case.
#[test]
fn equipment_types_and_research_categories() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "has_mio_equipment_type = synthetic_group", Some(true)),
                (MIO, "has_mio_equipment_type = GROUPED_TYPE", Some(true)),
                (MIO, "has_mio_equipment_type = \"loose_type\"", Some(true)),
                (MIO, "has_mio_equipment_type = other_type", Some(false)),
                (MIO, "has_mio_research_category = CAT_LAND", Some(true)),
                (MIO, "has_mio_research_category = cat_air", Some(false)),
                (RECURSIVE, "has_mio_research_category = cat_land", Some(false)),
            ],
        );
    });
}

/// A flag is its first record: with no `value`, its value must be above zero; `value`
/// compares by its operator; `date` compares the setting hour strictly; `days` compares day
/// starts strictly; `date =` and `days =` add no test; a flag named through a country or
/// state is refused.
#[test]
fn flags_compare_value_date_and_days() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "has_mio_flag = mio_flag", Some(true)),
                (MIO, "has_mio_flag = \"MIO_FLAG\"", Some(true)),
                (MIO, "has_mio_flag = zero_flag", Some(false)),
                (MIO, "has_mio_flag = missing_flag", Some(false)),
                (OTHER, "has_mio_flag = mio_flag", Some(false)),
                (MIO, "has_mio_flag = { flag = zero_flag value = 0 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag value > 1 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag value > 2 }", Some(false)),
                (MIO, "has_mio_flag = { flag = mio_flag value < 2 }", Some(false)),
                (MIO, "has_mio_flag = { flag = mio_flag date > 2.9.28 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag date > 2.9.29 }", Some(false)),
                (MIO, "has_mio_flag = { flag = mio_flag date < 2.9.29 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag date = 1.1.1 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag days > 2 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag days > 3 }", Some(false)),
                (MIO, "has_mio_flag = { flag = mio_flag days < 3 }", Some(false)),
                (MIO, "has_mio_flag = { flag = mio_flag days < 4 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag days = 100 }", Some(true)),
                (MIO, "has_mio_flag = { flag = mio_flag date = 2.13.1 }", None),
                (MIO, "has_mio_flag = { flag = mio_flag@ROOT }", None),
            ],
        );
    });
}

/// `is_mio_available` is the definition's `available` and then `visible`, `is_mio_visible`
/// its `visible` alone, each against yes or no (a scope selector reads as `no`). A condition
/// reaching itself recurses without end in the game: it is refused by name, and no false
/// condition after it or beside it decides, while one whose self-reference is skipped decides.
#[test]
fn organisation_conditions_and_their_recursion() {
    with_organisations(&organisations(), |world| {
        assert_cases(
            world,
            &[
                (MIO, "is_mio_available = yes", Some(true)),
                (OTHER, "is_mio_available = yes", Some(false)),
                (OTHER, "is_mio_available = no", Some(true)),
                (OTHER, "is_mio_visible = yes", Some(true)),
                (OTHER, "is_mio_available = THIS", Some(true)),
                (MIO, "is_mio_available = FROM", Some(false)),
                (MIO, "is_mio_visible = ROOT", Some(false)),
                (RECURSIVE, "is_mio_visible = yes", Some(false)),
                (GUARDED, "is_mio_available = yes", Some(false)),
                (GUARDED, "is_mio_available = no", Some(true)),
            ],
        );
        let evaluator = Evaluator::new(world, None);
        for trigger in [
            "is_mio_available = yes",
            "is_mio_available = no",
            "is_mio_available = yes always = no",
            "OR = { is_mio_available = yes always = yes }",
        ] {
            let value = evaluator.evaluate(
                &items(trigger),
                Scopes::of(Scope::IndustrialOrganisation(RECURSIVE)),
            );
            assert!(
                matches!(&value, Eval::Unknown(_)) && format!("{value:?}").contains("inside itself"),
                "{trigger}: {value:?}"
            );
        }
        // The row gate's composition refuses it the same way, though `visible` is false.
        let tree = |name: &str| -> (Vec<Item>, Vec<Item>) {
            let (catalog, _) = content();
            let tree = catalog.tree(name).unwrap().unwrap();
            (tree.available.to_vec(), tree.visible.to_vec())
        };
        let (available, visible) = tree("recursive_mio");
        let gate = evaluator.organisation_conditions(
            RECURSIVE,
            &[
                (OrganisationCondition::Available, available.as_slice()),
                (OrganisationCondition::Visible, visible.as_slice()),
            ],
        );
        assert!(
            matches!(&gate, Err(Undecided::Diverged(why)) if why.contains("inside itself")),
            "{gate:?}"
        );
        // Empty conditions hold.
        assert_eq!(evaluator.organisation_conditions(MIO, &[]), Ok(true));
        // An evaluation after it starts afresh.
        assert_eq!(
            evaluator.evaluate(&items("always = yes"), Scopes::of(Scope::IndustrialOrganisation(MIO))),
            Eval::True
        );
        // Every organisation trigger accepts the organisation scope alone.
        assert_eq!(
            world.evaluate(&items("has_mio_trait = first_trait"), Scopes::of(Scope::Country(1))),
            Eval::False
        );
    });
}
