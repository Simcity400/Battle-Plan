use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

#[test]
fn create_unit_officer_and_portrait_readers_skip_rejected_blocks() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_unit = { officer = { invalid = { @hidden = 9 } name = officer seed = 5 male = yes portraits = { army = { large = picture invalid = { @portrait = 7 } } invalid = { @role = 6 } } } @seen = 3 owner = ABC invalid = { @outer = 8 } } } final = @seen hidden = @hidden portrait = @portrait role = @role outer = @outer",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    for key in ["hidden", "portrait", "role", "outer"] {
        assert_eq!(scalar(&result.items, key), format!("@{key}").as_bytes());
    }
    let unit = block(block(&result.items, "body"), "create_unit");
    let officer = block(unit, "officer");
    assert_eq!(scalar(officer, "seed"), b"5");
    assert_eq!(scalar(officer, "male"), b"yes");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn create_unit_country_score_uses_mtth_and_trigger_modifier_readers() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_unit = { country_score = { modifier = { invalid = { @hidden = 9 } @seen = 3 always = yes } unknown = { @open = 4 factor = 2 } } } final = @seen open = @open hidden = @hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "open"), b"4");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn set_politics_parties_use_political_party_script_grammar() {
    crate::test_game::enter();
    let result = read(
        b"body = { set_politics = { parties = { arbitrary_ideology = { invalid = { @hidden = 9 } @seen = 3 popularity = 40 } } last_election = 1936.1.1 invalid = { @outer = 8 } } } final = @seen hidden = @hidden outer = @outer",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert_eq!(scalar(&result.items, "outer"), b"@outer");
    let politics = block(block(&result.items, "body"), "set_politics");
    assert_eq!(scalar(politics, "last_election"), b"1936.1.1");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn create_equipment_variant_delegates_to_spec_and_skips_rejected_blocks() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_equipment_variant = { type = equipment visible = yes design_team = team name = variant icon = GFX_picture invalid = { @hidden = 9 } @seen = 3 version = 1 } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn create_equipment_variant_upgrades_use_recorded_names() {
    crate::test_game::enter();
    let mut names = KeyNames::default();
    names
        .effects
        .equipment_upgrades
        .insert(b"mod_upgrade".to_vec());
    let result = read(
        b"body = { create_equipment_variant = { upgrades = { absent_upgrade = { @hidden = 9 } mod_upgrade = { @seen = 3 marker = 1 } } } final = @seen hidden = @hidden",
        &names,
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    let body = block(&result.items, "body");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    let variant = block(body, "create_equipment_variant");
    assert_eq!(scalar(block(variant, "upgrades"), "mod_upgrade"), b"{");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn create_equipment_variant_optional_assets_are_raw_object_entries() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_equipment_variant = { optional_assets = { { model = first invalid = { @hidden = 9 } } { icon = GFX_second @seen = 3 model = second } } name = variant } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn create_equipment_variant_cid_unknown_key_reads_one_value() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_equipment_variant = { industrial_organisation = { unknown = { @seen = 3 id = 7 } name = variant } } final = @seen",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    let body = block(&result.items, "body");
    assert_eq!(scalar(&result.items, "final"), b"3");
    let variant = block(body, "create_equipment_variant");
    assert_eq!(
        scalar(block(variant, "industrial_organisation"), "unknown"),
        b"{"
    );
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

#[test]
fn create_equipment_variant_modules_read_one_value_for_every_name() {
    crate::test_game::enter();
    let result = read(
        b"body = { create_equipment_variant = { modules = { slot = { @seen = 3 another_slot = none } name = variant } } final = @seen",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    let body = block(&result.items, "body");
    assert_eq!(scalar(&result.items, "final"), b"3");
    let variant = block(body, "create_equipment_variant");
    assert_eq!(scalar(block(variant, "modules"), "slot"), b"{");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}
