//! Icons drawn from their recipes as the request's selection shows them.
use super::*;

/// A served reply's icon recipes are drawn as its request shows them: the recording's own without
/// a selection, the selection's with one, each recipe once per source, a second reply hitting
/// only the memo; one neither can draw says why on its item and among the warnings.
#[test]
fn served_ledger_icons_follow_the_selection_once_per_recipe() {
    use dashboard_base::test_fixtures::visual_mods::{
        BLUE, Machine, RED, first_pixel, png, select,
    };
    let machine = Machine::new("ledger_art");
    machine.write(
        "common/defines/00_fixture.lua",
        b"NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, \
          NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_officers\" } } }",
    );
    machine.write(
        "interface/ledger.gfx",
        b"spriteTypes = { spriteType = { name = GFX_ability textureFile = gfx/ability.png } }",
    );
    machine.write("gfx/ability.png", &png(RED));
    machine.write_mod("art", "gfx/ability.png", &png(BLUE));
    let (source, mut options) = machine.open();
    let served = || {
        serde_json::json!({
            "spendItems": [
                { "id": "a", "iconArt": serde_json::to_value(LedgerArt::Sprite("GFX_ability".into())).unwrap() },
                { "id": "b", "iconArt": serde_json::to_value(LedgerArt::Sprite("GFX_ability".into())).unwrap() },
                { "id": "c", "iconArt": serde_json::to_value(LedgerArt::Sprite("GFX_missing".into())).unwrap() },
            ],
            "warnings": [],
        })
    };
    let renders = dashboard_base::catalogue::presented;
    let shown = |options: &dashboard_core::config::ContentOptions| {
        let mut data = served();
        present_icons(&source, Some(options), &mut data).unwrap();
        data
    };
    select(&source, &mut options, &[]);
    let before = renders();
    let data = shown(&options);
    assert_eq!(renders() - before, 2, "one render per distinct recipe");
    assert_eq!(
        first_pixel(data["spendItems"][0]["icon"].as_str().unwrap()),
        RED
    );
    assert!(data["spendItems"][0].get("iconArt").is_none());
    assert!(
        data["spendItems"][2]["unknown"]
            .as_str()
            .unwrap()
            .contains("GFX_missing")
    );
    assert!(
        data["warnings"][0]
            .as_str()
            .unwrap()
            .contains("GFX_missing")
    );
    let before = renders();
    shown(&options);
    assert_eq!(renders() - before, 0, "a second reply only hits the memo");
    select(&source, &mut options, &["art"]);
    assert_eq!(
        first_pixel(shown(&options)["spendItems"][1]["icon"].as_str().unwrap()),
        BLUE
    );
}

/// A holder's recorded portrait the selection cannot draw fails the selection's whole attempt:
/// the recording's own art, its portrait, is drawn and named, never the idea's generic art in
/// its place within the selection's attempt.
#[test]
fn a_holder_portrait_the_selection_cannot_draw_draws_the_recordings_own() {
    use dashboard_base::test_fixtures::visual_mods::{
        Machine, RED, fallback_line, first_pixel, png, select,
    };
    let machine = Machine::new("ledger_holder_portrait");
    machine.write(
        "common/defines/00_fixture.lua",
        b"NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 },           NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_officers\" } } }",
    );
    machine.write(
        "interface/ledger.gfx",
        b"spriteTypes = { spriteType = { name = GFX_holder textureFile = gfx/holder.png } }",
    );
    machine.write("gfx/holder.png", &png(RED));
    // The mod's declarations replace the recording's file: its portrait does not load.
    machine.write_mod(
        "broken",
        "interface/ledger.gfx",
        b"spriteTypes = { spriteType = { name = GFX_holder textureFile = gfx/broken.png } }",
    );
    machine.write_mod("broken", "gfx/broken.png", b"not a png");
    let (source, mut options) = machine.open();
    let recipe = LedgerArt::Idea {
        token: "fixture_idea".into(),
        character: None,
        tag: "GER".into(),
        naming: Default::default(),
        holder: Some((
            7,
            dashboard_base::products::character_portraits::RecordedPortrait::Entry {
                kind: 0,
                identity: b"GFX_holder".to_vec(),
            },
        )),
    };
    select(&source, &mut options, &["broken"]);
    for _ in 0..2 {
        let mut data = serde_json::json!({
            "spendItems": [{ "id": "a", "iconArt": serde_json::to_value(&recipe).unwrap() }],
            "warnings": [],
        });
        let ((), line) =
            fallback_line(|| present_icons(&source, Some(&options), &mut data).unwrap());
        assert_eq!(
            first_pixel(data["spendItems"][0]["icon"].as_str().unwrap()),
            RED
        );
        assert!(
            line.as_ref()
                .is_some_and(|line| line.contains("politics ledger icon (character 7 portrait")),
            "{line:?}"
        );
    }
}

/// A drawn item's note (a holder's portrait it could not use where the idea's art stands in)
/// is named on the item and counted in the reply's warnings, once per item showing it, as the
/// memo replays it.
#[test]
fn a_drawn_items_note_is_counted_in_the_warnings() {
    let recipe = serde_json::to_value(LedgerArt::Sprite("GFX_idea".into())).unwrap();
    let mut data = serde_json::json!({
        "spendItems": [
            { "id": "a", "iconArt": recipe.clone() },
            { "id": "b", "iconArt": recipe },
        ],
        "warnings": [],
    });
    let note = "character 7 portrait: not drawn";
    let icon = format!("artwork:{}.png", "a0".repeat(16));
    draw_icons(&mut data, |_| {
        Ok(Ok((Some(icon.clone()), Some(note.to_owned()))))
    })
    .unwrap();
    for item in data["spendItems"].as_array().unwrap() {
        assert_eq!(item["icon"], icon);
        assert_eq!(item["unknown"], note);
    }
    assert_eq!(
        data["warnings"],
        serde_json::json!([format!("{note} (count: 2)")])
    );
}
