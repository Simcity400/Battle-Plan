use super::*;
use crate::catalogue::tests::script_game;
use replay_data::rows::{CountryCharacterRow, ScientistRow, ScientistSkillRow};

fn scientist_frame() -> DecodedFrame {
    let mut frame = frame(&[(100, 1, "AAA", 1, "AAA", "")], &[]);
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::from_rows::<
        tables::LexicalTokens,
    >(vec![
        LexicalTokenRow {
            token: 50,
            literal: b"specialization_land".to_vec(),
        },
        LexicalTokenRow {
            token: 51,
            literal: b"specialization_air".to_vec(),
        },
    ]));
    *frame.tables[TableId::CountryCharacters.index()] =
        TableView::Complete(TableRows::from_rows::<tables::CountryCharacters>(
            [7, 8, 9]
                .into_iter()
                .enumerate()
                .map(|(ordinal, character)| CountryCharacterRow {
                    country: 100,
                    ordinal: ordinal as u32,
                    character: Some(character),
                })
                .collect(),
        ));
    *frame.tables[TableId::Scientists.index()] =
        TableView::Complete(TableRows::from_rows::<tables::Scientists>(vec![
            ScientistRow {
                character: 7,
                country: 100,
                ordinal: 1,
                skill_count: 2,
                active: Recorded::Value(Some(1)),
                ..Default::default()
            },
            ScientistRow {
                character: 8,
                country: 100,
                ordinal: 0,
                skill_count: 1,
                active: Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ScientistSkills.index()] =
        TableView::Complete(TableRows::from_rows::<tables::ScientistSkills>(
            [(7, 0, 50, 3), (7, 1, 51, 1), (8, 0, 51, 2)]
                .into_iter()
                .map(|(character, ordinal, token, level)| ScientistSkillRow {
                    character,
                    ordinal,
                    token,
                    level,
                    experience_raw: 0,
                })
                .collect(),
        ));
    frame
}

fn with_scientists(frame: &DecodedFrame, check: impl FnOnce(&World)) {
    script_game::enter();
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "common/special_projects/specialization/test.txt",
        "specialization_land={} specialization_air={}",
    )]);
    let catalog =
        content_reader_military::special_projects::Catalog::read(&catalogue.content.layers).unwrap();
    let dlcs = BTreeSet::new();
    let triggers = TriggerNames::default();
    let aliases = Arc::default();
    let world = World::read(frame, ContentFacts::content_only(&dlcs, &triggers, &aliases))
        .with_special_projects(&catalog);
    check(&world);
}

fn level(character: u32, body: &str) -> Eval {
    let frame = scientist_frame();
    let mut result = None;
    with_scientists(&frame, |world| {
        result = Some(world.evaluate(
            &items(&format!("has_scientist_level = {{ {body} }}")),
            Scopes::of(Scope::Character(character)),
        ));
    });
    result.unwrap()
}

#[test]
fn has_scientist_level_compares_the_scientists_integer_level_in_one_specialization() {
    for (character, body, expected) in [
        (7, "level > 2 specialization = specialization_land", true),
        (7, "level > 3 specialization = specialization_land", false),
        (7, "level < 4 specialization = specialization_land", true),
        (7, "level = 3 specialization = specialization_land", true),
        (7, "specialization = specialization_air level = 1", true),
        (7, "level = 3 level = 1 specialization = specialization_air", true),
        (8, "level = 0 specialization = specialization_land", true),
        (8, "level < 1 specialization = specialization_land", true),
        (8, "level > 1 specialization = specialization_air", true),
        (9, "level < 5 specialization = specialization_land", false),
        (7, "level < 5 specialization = specialization_magic", false),
        (0, "level < 5 specialization = specialization_land", false),
    ] {
        assert_eq!(level(character, body), Eval::from_bool(expected), "character {character}: {body}");
    }
}

#[test]
fn has_scientist_level_refuses_what_is_not_recorded_or_not_reproduced() {
    for (character, body) in [
        (12, "level < 5 specialization = specialization_land"),
        (7, "specialization = specialization_land"),
        (7, "level < 5"),
        (7, "level < 5 specialization = specialization_land tooltip = x"),
    ] {
        assert!(matches!(level(character, body), Eval::Unknown(_)), "character {character}: {body}");
    }
    let mut frame = scientist_frame();
    *frame.tables[TableId::ScientistSkills.index()] =
        TableView::Complete(TableRows::from_rows::<tables::ScientistSkills>(vec![ScientistSkillRow {
            character: 7,
            ordinal: 0,
            token: 50,
            level: 3,
            experience_raw: 0,
        }]));
    with_scientists(&frame, |world| {
        assert!(matches!(
            world.evaluate(
                &items("has_scientist_level = { level > 2 specialization = specialization_land }"),
                Scopes::of(Scope::Character(7)),
            ),
            Eval::Unknown(_)
        ));
    });
}

#[test]
fn is_active_scientist_matches_the_role_flag_and_is_false_without_a_role() {
    let frame = scientist_frame();
    with_scientists(&frame, |world| {
        for (character, operand, expected) in [
            (7, "yes", true),
            (7, "no", false),
            (8, "yes", false),
            (8, "no", true),
            (9, "yes", false),
            (9, "no", false),
        ] {
            assert_eq!(
                world.evaluate(
                    &items(&format!("is_active_scientist = {operand}")),
                    Scopes::of(Scope::Character(character)),
                ),
                Eval::from_bool(expected),
                "character {character}, {operand}"
            );
        }
    });
}

#[test]
fn any_scientist_counts_the_countrys_rostered_scientists() {
    let frame = scientist_frame();
    with_scientists(&frame, |world| {
        for (body, expected) in [
            ("has_scientist_level = { level > 2 specialization = specialization_land }", true),
            ("has_scientist_level = { level > 3 specialization = specialization_land }", false),
            ("count = 2 has_scientist_level = { level > 0 specialization = specialization_air }", true),
            ("count = 2 has_scientist_level = { level > 1 specialization = specialization_air }", false),
            ("is_active_scientist = no has_scientist_level = { level > 1 specialization = specialization_air }", true),
            ("is_active_scientist = no has_scientist_level = { level > 0 specialization = specialization_land }", false),
        ] {
            assert_eq!(
                world.evaluate(
                    &items(&format!("any_scientist = {{ {body} }}")),
                    Scopes::of(Scope::Country(1)),
                ),
                Eval::from_bool(expected),
                "{body}"
            );
        }
    });
    let mut frame = scientist_frame();
    let TableView::Complete(TableRows::Scientists(rows)) = &mut *frame.tables[TableId::Scientists.index()] else { unreachable!() };
    rows.push(ScientistRow { character: 9, country: 999, ordinal: 0, skill_count: 1, ..Default::default() });
    with_scientists(&frame, |world| {
        assert!(
            matches!(world.evaluate(&items("any_scientist = { has_scientist_level = { level > 3 specialization = specialization_land } }"), Scopes::of(Scope::Country(1))), Eval::Unknown(_)),
            "O-S3: a scientist whose owner has no recorded country link may be this country's, so the count is unknown, never false"
        );
    });
}
