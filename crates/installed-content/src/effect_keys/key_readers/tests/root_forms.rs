use crate::{
    effect_keys::tests::{block, read, scalar},
    key_names::KeyNames,
};

// CCreateUnitEffect (1403B1AA0) -> SUnitOfficerData (1413D56B0) ->
// CCharacterPortraits (141389AC0): each object owns its rejecting fallback.
#[test]
fn create_unit_officer_and_portrait_readers_skip_rejected_blocks() {
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

// country_score stores a CMeanTimeToHappen (140544B60). Its slot3 is an
// unconditional CPersistent read; MTTH unknown fields log without a skip.
#[test]
fn create_unit_country_score_uses_mtth_and_trigger_modifier_readers() {
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

// CSetPoliticsEffect (1403B5350)'s parties loop constructs the same
// CSetPoliticalPartyEffect (14032CA40) that set_political_party3009 dispatches.
#[test]
fn set_politics_parties_use_political_party_script_grammar() {
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

// CCreateEquipmentVariantEffect (14049E440) delegates unknown own keys to
// CCreateEquipmentVariantSpec (1414514A0); the spec default reporter skips.
#[test]
fn create_equipment_variant_delegates_to_spec_and_skips_rejected_blocks() {
    let result = read(
        b"body = { create_equipment_variant = { type = equipment visible = yes design_team = team name = variant icon = GFX_picture invalid = { @hidden = 9 } @seen = 3 version = 1 } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

// Upgrade keys (140F79C80) resolve through the content database. Valid keys
// read one byte; invalid keys RecordError and skip their opening value.
#[test]
fn create_equipment_variant_upgrades_use_recorded_names() {
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

// Optional assets (14144E640) are a raw-token object list, not key/value
// entries; each asset (1414516B0) has icon/model and a rejecting fallback.
#[test]
fn create_equipment_variant_optional_assets_are_raw_object_entries() {
    let result = read(
        b"body = { create_equipment_variant = { optional_assets = { { model = first invalid = { @hidden = 9 } } { icon = GFX_second @seen = 3 model = second } } name = variant } } final = @seen hidden = @hidden",
        &KeyNames::default(),
    );
    assert!(result.unsettled.is_none(), "{result:?}");
    assert_eq!(scalar(&result.items, "final"), b"3");
    assert_eq!(scalar(&result.items, "hidden"), b"@hidden");
    assert!(result.misreads.iter().all(|note| note.count.is_none()));
}

// ReadCID (14220AA30) ignores an invalid key after reporting it. Its value
// is not skipped; the following fields remain inside that same CID loop.
#[test]
fn create_equipment_variant_cid_unknown_key_reads_one_value() {
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

// Modules (14191CCF0) reports an invalid module without RecordError's skip;
// a valid one also consumes only its already-read value.
#[test]
fn create_equipment_variant_modules_read_one_value_for_every_name() {
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
