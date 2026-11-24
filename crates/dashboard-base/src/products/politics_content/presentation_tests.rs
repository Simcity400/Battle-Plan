
use super::{PoliticsContent, politics_content};
use crate::test_fixtures::visual_mods::{
    BLUE, Machine, RED, fallback_line, first_pixel, png, select,
};
use std::sync::Arc;

const DEFINES: &[u8] = b"NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"mod_seat\" } } }";

#[test]
fn an_idea_the_mod_cannot_draw_makes_the_recordings_own_choice_again() {
    let machine = Machine::new("idea_selection");
    machine.write("common/defines/mod.lua", DEFINES);
    machine.write(
        "common/ideas/mod.txt",
        b"ideas = { country = { mod_spirit = { picture = mod_picture } } }",
    );
    machine.write(
        "interface/ideas.gfx",
        b"spriteTypes = { spriteType = { name = GFX_idea_mod_picture textureFile = gfx/a.png } }",
    );
    machine.write("gfx/a.png", &png(RED));
    machine.write_mod(
        "preferred",
        "interface/preferred.gfx",
        b"spriteTypes = { spriteType = { name = GFX_idea_mod_spirit textureFile = gfx/b.png } }",
    );
    machine.write_mod("preferred", "gfx/b.png", &png(BLUE));
    machine.write_mod("preferred", "gfx/a.png", b"not a png");
    let (source, mut options) = machine.open();
    select(&source, &mut options, &[]);
    let (art, line) = fallback_line(|| {
        politics_content(&source, &options)
            .unwrap()
            .idea_artwork_offline("mod_spirit")
    });
    assert_eq!(first_pixel(art.0.icon.as_deref().unwrap()), RED);
    assert_eq!(art.0.sprite.as_deref(), Some("GFX_idea_mod_picture"), "the sprite it was drawn from");
    assert_eq!(line, None, "without a selection nothing is named");
    let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
    let fresh = PoliticsContent::load(Arc::clone(&catalogue))
        .unwrap()
        .idea_artwork_offline("mod_spirit");
    assert_eq!(art, fresh, "without a selection, as a fresh load draws it");
    select(&source, &mut options, &["preferred"]);
    for _ in 0..2 {
        let ((art, problem), line) = fallback_line(|| {
            politics_content(&source, &options)
                .unwrap()
                .idea_artwork_offline("mod_spirit")
        });
        assert_eq!(problem, None);
        assert_eq!(first_pixel(art.icon.as_deref().unwrap()), RED, "the recording's own A");
        assert!(
            line.as_ref()
                .is_some_and(|line| line.contains("idea mod_spirit icon")),
            "{line:?}"
        );
    }
}

#[test]
fn a_portrait_the_mod_cannot_draw_is_the_recordings_named() {
    let machine = Machine::new("portrait_selection");
    machine.write("common/defines/mod.lua", DEFINES);
    machine.write(
        "interface/portraits.gfx",
        b"spriteTypes = { spriteType = { name = GFX_portrait_mod textureFile = gfx/portrait.png } }",
    );
    machine.write("gfx/portrait.png", &png(RED));
    machine.write_mod(
        "broken",
        "interface/portraits.gfx",
        b"spriteTypes = { spriteType = { name = GFX_portrait_mod textureFile = gfx/redeclared.png } }",
    );
    machine.write_mod("broken", "gfx/redeclared.png", &png(BLUE));
    machine.write_mod("broken", "gfx/portrait.png", b"not a png");
    let (source, mut options) = machine.open();
    select(&source, &mut options, &[]);
    let (portrait, line) = fallback_line(|| {
        politics_content(&source, &options)
            .unwrap()
            .recorded_portrait(0, b"GFX_portrait_mod")
    });
    assert_eq!(first_pixel(&portrait.unwrap()), RED);
    assert_eq!(line, None);
    select(&source, &mut options, &["broken"]);
    for _ in 0..2 {
        let (portrait, line) = fallback_line(|| {
            politics_content(&source, &options)
                .unwrap()
                .recorded_portrait(0, b"GFX_portrait_mod")
        });
        assert_eq!(first_pixel(&portrait.unwrap()), RED);
        assert!(
            line.as_ref()
                .is_some_and(|line| line.contains("portrait GFX_portrait_mod")),
            "{line:?}"
        );
    }
}
