use super::*;
use crate::combat::binding::NoIcons;

#[test]
fn combat_close_hovers_use_first_authored_shortcut_and_keep_tooltip_parts() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let loc = loc(&[
        ("SHORTCUT", "Shortcut: $KEY$"),
        ("MAIN", "Main"),
        ("DELAYED", "Body"),
    ]);
    let windows = windows(
        br#"containerWindowType={name=view
            buttonType={name=close shortcut=ESCAPE shortcut=F12
                pdx_tooltip=MAIN pdx_tooltip_delayed=DELAYED}}"#,
    );
    let gui = Gui {
        icons: &NoIcons,
        loc: &loc,
        windows: &windows,
    };
    let mut values = Values::new();
    values.entry("CLOSE".into()).or_default();
    let mut refused = BTreeMap::new();
    authored(
        &gui,
        windows.gui_type(b"view").unwrap(),
        "",
        &mut values,
        &mut refused,
    );
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(values.len(), 1);
    let close = &values["CLOSE"];
    assert_eq!(close.tooltip.as_deref(), Some("Main"));
    assert_eq!(close.tooltip_description.as_deref(), Some("Body"));
    assert_eq!(
        close.tooltip_shortcut.as_deref(),
        Some("Shortcut: \u{13}ESCAPE")
    );
    let missing = Gui {
        loc: &Localisation::default(),
        ..gui
    };
    assert_eq!(
        shortcut_tooltip(&missing, b"ESCAPE", "close"),
        Ok("SHORTCUT".into())
    );
    assert_eq!(shortcut_keycaps(" Ctrl + A "), "\u{13}Ctrl +\u{13}A ");
    assert_eq!(shortcut_keycaps("+AA"), "+\u{13}AA ");
    assert_eq!(shortcut_keycaps("+A"), "\u{13}+A ");
    assert_eq!(shortcut_keycaps(","), "\u{13}COMMA ");
    assert_eq!(shortcut_keycaps("."), "\u{13}PERIOD ");
    assert_eq!(shortcut_keycaps(" \r\n\t"), "");
}

#[test]
fn naval_progress_has_outlook_and_delayed_description_on_both_skins() {
    let loc: Localisation = [
        ("COMBAT_UNDECIDED", "Undecided"),
        ("COMBAT_LAST_LESS_THAN_DAY", "Less than $DAYS$ day"),
        ("COMBAT_PROGRESS_DESC", "Progress details"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut values = Values::new();
    let mut refused = BTreeMap::new();
    naval_progress_hovers(&mut values, &NoIcons, &loc, 1, &mut refused);
    assert!(refused.is_empty());
    for skin in ["attacker", "defender"] {
        let widget = &values[&format!("header_container/progress_{skin}")];
        assert_eq!(
            widget.tooltip.as_deref(),
            Some("Undecided\nLess than 0 day")
        );
        assert_eq!(
            widget.tooltip_description.as_deref(),
            Some("Progress details")
        );
    }
}

#[test]
fn land_balance_bar_and_frame_share_native_outlook_and_delayed_explanation() {
    let loc = loc(&[
        ("COMBAT_UNDECIDED", "Undecided"),
        ("COMBAT_WE_ARE_LOSING", "Losing"),
        ("COMBAT_LAST_LESS_THAN_DAY", "Less than a day"),
        ("COMBAT_LAST_MORE_THAN_DAY", "$DAYS$ days"),
        ("COMBAT_PROGRESS_DESC", "Power balance explanation"),
    ]);
    for (scores, expected) in [
        ([0x0017_4875_6160; 2], "Undecided\n41667 days"),
        ([100_000, 200_000], "Losing\nLess than a day"),
    ] {
        let mut values = Values::new();
        let mut refused = BTreeMap::new();
        let outlook =
            super::super::land_marker_tooltips::outlook(&NoIcons, &loc, scores, [true, false]);
        land_progress_hovers(&mut values, &NoIcons, &loc, outlook, &mut refused);
        assert!(refused.is_empty());
        for key in ["combat_progress", "land_battle_progress_frame"] {
            let widget = &values[key];
            assert_eq!(widget.tooltip.as_deref(), Some(expected));
            assert_eq!(
                widget.tooltip_description.as_deref(),
                Some("Power balance explanation")
            );
            assert!(widget.hover_owned);
        }
        // A missing recorded score clears an authored main tooltip, counted once.
        land_progress_hovers(
            &mut values,
            &NoIcons,
            &loc,
            Err("LandCombatSides lacks combat 9".into()),
            &mut refused,
        );
        assert_eq!(refused.get("LandCombatSides lacks combat 9"), Some(&1));
        for key in ["combat_progress", "land_battle_progress_frame"] {
            assert!(values[key].tooltip.is_none());
            assert!(values[key].tooltip_description.is_some());
        }
    }
}

#[test]
fn naval_ship_pride_hover_uses_executable_key_only_when_visible() {
    let loc: Localisation = [("NAVY_CONTAINS_PRIDE_OF_FLEET_WITHOUT_NAME", "Pride ship")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let mut values = Values::new();
    values.insert("pride".into(), super::super::Widget::default());
    let mut refused = BTreeMap::new();
    naval_pride_hover(&mut values, &NoIcons, &loc, 7, &mut refused);
    assert_eq!(values["pride"].tooltip.as_deref(), Some("Pride ship"));
    assert!(refused.is_empty());
    values.get_mut("pride").unwrap().hidden = true;
    naval_pride_hover(&mut values, &NoIcons, &loc, 7, &mut refused);
    assert_eq!(values["pride"].tooltip.as_deref(), Some("Pride ship"));
}

#[test]
fn naval_commander_hover_uses_recorded_name_and_description_tokens() {
    let loc: Localisation = [
        ("HEADER", "$KEY$\n"),
        ("ADMIRAL_NAME", "Example"),
        ("navy_leader", "Admiral"),
        ("ADMIRAL_DESC", "Career description"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let commander = replay::rows::NavalCommanderInputs {
        category: 2,
        name: b"ADMIRAL_NAME".to_vec(),
        description: b"ADMIRAL_DESC".to_vec(),
        ..Default::default()
    };
    let (main, delayed) =
        naval_commander_portrait(&NoIcons, &loc, &commander, Ok(false), Ok("AAA")).unwrap();
    assert_eq!(main, "Admiral Example\n");
    assert_eq!(delayed, "Career description");
}

#[test]
fn naval_flag_hover_titles_both_flags_from_first_side_and_refuses_unknown_alias() {
    let aliases = BTreeMap::from([(12, 120), (34, 340), (80, 800)]);
    let detail = |first: &[u32], second: &[u32]| {
        let mut detail = replay::rows::NavalCombatDetailsRow::default();
        detail.sides[0].summary.countries = first.to_vec();
        detail.sides[1].summary.countries = second.to_vec();
        detail
    };
    // The viewer in the first combatant is selected through its canonical
    // alias; viewing from the other combatant still titles BOTH flags with
    // the first country's name, not the enemy flag's country.
    let combat = detail(&[12, 34], &[80]);
    assert_eq!(naval_flag_country(&combat, &aliases, 340), Ok(340));
    assert_eq!(naval_flag_country(&combat, &aliases, 800), Ok(120));
    assert_eq!(
        naval_flag_country(&detail(&[800, 34], &[80]), &aliases, 800),
        Err("naval flag side country 800 has no CountryLinks row".to_owned())
    );
    assert!(naval_flag_country(&detail(&[12, 78], &[80]), &aliases, 800).is_err());
}

#[test]
fn naval_carrier_laurel_hover_uses_native_argument_order() {
    let loc: Localisation = [(
        "CARRIER_STACK_PENALTY_DESC",
        "Penalty $PERC$; enemy $ENEMY_FLEET$; minimum $MINIMUM$",
    )]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let text = carrier_penalty_text(&NoIcons, &loc, 50_000, 12, 3).unwrap();
    assert!(text.starts_with("Penalty "));
    assert!(text.contains("enemy 12; minimum 3"));
}

#[test]
fn naval_anti_air_header_owns_text_and_background_hovers() {
    let loc: Localisation = [("combat_stat_anti_air", "Anti-air")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let mut values = Values::new();
    let mut refused = BTreeMap::new();
    naval_anti_air_hovers(&mut values, &NoIcons, &loc, 4, &mut refused);
    assert!(refused.is_empty());
    for side in ["left", "right"] {
        for widget in ["stat_anti_air", "stat_anti_air_bg"] {
            let bound = &values[&format!("header_container/{side}_wnd/{widget}")];
            assert_eq!(bound.tooltip.as_deref(), Some("Anti-air"));
            assert!(bound.hover_owned);
        }
    }
}

fn windows(body: &[u8]) -> NativeWindows {
    let mut bytes = b"guiTypes = { containerWindowType = { name = unit_counter_navy } containerWindowType = { name = naval_combat_mapicon } ".to_vec();
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b" }");
    NativeWindows::read(&bytes, [1280, 720]).unwrap()
}

#[test]
fn localisation_inventory_includes_unbound_tooltip_keys() {
    let localisation = loc(&[
        ("MAIN", "£main"),
        ("DELAYED", "£delayed"),
        ("OTHER", "£unused"),
    ]);
    let assets = crate::map::native_gui::Assets::from_localisation(&localisation);
    assert!(assets.sprites.contains(b"GFX_main".as_slice()));
    assert!(assets.sprites.contains(b"GFX_delayed".as_slice()));
    assert!(assets.sprites.contains(b"GFX_unused".as_slice()));
}

#[test]
fn political_province_hover_keys_preload_native_inline_icons() {
    let localisation = loc(&[
        ("PROV_TOOLTIP_LOCATION_HEADER", "£pin"),
        ("PROV_TOOLTIP_WEATHER_MODIFIER", "£weather|$FRAME$ $TEXT$"),
        ("PROVINCE_VP_TOOLTIP", "£victory $VALUE$"),
        ("SUB_UNIT_RANGED_VALUE", "£estimate $MIN$-$MAX$"),
    ]);
    let assets = crate::map::native_gui::Assets::from_localisation(&localisation);
    for sprite in ["GFX_pin", "GFX_weather", "GFX_victory", "GFX_estimate"] {
        assert!(assets.sprites.contains(sprite.as_bytes()), "{sprite}");
    }
}

/// The tactic registry read through the real parse
/// (installed_content::combat_tactics), as the land hovers use it.
fn tactics(text: &[u8]) -> Tactics {
    Tactics::from_database(installed_content::combat_tactics::parse(
        text,
        &installed_content::key_names::KeyNames::default(),
    ))
}

fn loc(entries: &[(&str, &str)]) -> Localisation {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// The hovers find the recorded tactic at its recorded ID, which must carry
/// the recorded name;
/// a repeated name is its own tactic, and a refused effect or an
/// unproven entry names the tactic and why.
#[test]
fn recorded_tactics_find_their_registry_effects() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // Whether `x` names an ideology group is unknown while an ideology
    // file stops short.
    let mut names = installed_content::key_names::KeyNames::default();
    names.counts.ideologies_gap = Some("common/ideologies/a.txt is unknown from line 1".into());
    let registry = Tactics::from_database(installed_content::combat_tactics::parse(
        br#"@bonus = 0.25
            PHASES = { }
            tactic_a = { ATTACKER = @bonus Combat_Width = 0.1 attacker_movement_speed = 010 }
            tactic_b = { defender = @missing }
            tactic_a = { attacker = 0.5 }
            tactic_c = { defender_org_damage_modifier = 1.5e3 active = yes }
            tactic_d = { trigger = { x = { } } }"#,
        &names,
    ));
    use super::super::tactics::Effects;
    assert_eq!(
        registry.effects(2, b"tactic_a"),
        Ok(Effects {
            attacker: 25_000,
            width: 10_000,
            movement: 800_000,
            ..Effects::default()
        })
    );
    // The repeat is tactic 4, with its own values.
    assert_eq!(
        registry.effects(4, b"tactic_a").map(|t| t.attacker),
        Ok(50_000)
    );
    assert_eq!(registry.effects(1, b"PHASES"), Ok(Effects::default()));
    // the game reads any text: an unbound reference is its own text
    // (0), and 'e' is a digit to it.
    assert_eq!(registry.effects(3, b"tactic_b"), Ok(Effects::default()));
    assert_eq!(
        registry.effects(5, b"tactic_c"),
        Ok(Effects {
            defender_org: 203_300,
            ..Effects::default()
        })
    );
    // A trigger block the port cannot settle: that tactic is unproven
    // and no later one is known.
    let unsettled = registry.effects(6, b"tactic_d").unwrap_err();
    assert!(
        unsettled.starts_with("tactic tactic_d's ID or fields cannot be read as the game reads them: combat_tactics.txt line 7: field `x`"),
        "{unsettled}"
    );
    let past = registry.effects(10, b"tactic_a").unwrap_err();
    assert!(
        past.starts_with("recorded tactic tactic_a's id 10 is past the 6 combat tactics read before combat_tactics.txt line 7"),
        "{past}"
    );
    assert_eq!(
        registry.effects(0, b"x").unwrap_err(),
        "recorded tactic x's id 0 is not a content tactic id"
    );
    assert_eq!(
        registry.effects(1, b"tactic_b"),
        Err("recorded tactic id 1 is tactic_b, but the content's tactic 1 is PHASES".into())
    );
}

fn tactic_a() -> Described<'static> {
    Described::Content {
        id: 1,
        token: b"tactic_a",
    }
}

/// The null tactic (kind 1) takes the ordinary hover path: its constructor
/// name supplies the header, zero effects add no lines, and TACTIC_DESC supplies
/// the delayed text. A countered tactic adds the countered line.
#[test]
fn the_null_tactic_object_is_described_like_a_tactic() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let loc = loc(&[
        ("HEADER", "$KEY$\n"),
        ("nullCombatTactic", "No Tactic Chosen"),
        ("TACTIC_CURRENTLY_COUNTERED", "Countered"),
        ("TACTIC_DESC", "What tactics do"),
    ]);
    let content = tactics(b"");
    let null = LandCombatSideDetails {
        tactic_kind: 1,
        tactic_counter_id: 5,
        ..Default::default()
    };
    let other = LandCombatSideDetails {
        tactic_kind: 2,
        tactic_id: 5,
        ..Default::default()
    };
    assert_eq!(
        tactic_tooltip(
            &NoIcons,
            &loc,
            &content,
            &null,
            &LandCombatSideDetails::default(),
            true
        ),
        Ok((
            "No Tactic Chosen\n".into(),
            Some(Ok("What tactics do".into()))
        ))
    );
    assert_eq!(
        tactic_tooltip(&NoIcons, &loc, &content, &null, &other, true),
        Ok((
            "No Tactic Chosen\nCountered".into(),
            Some(Ok("What tactics do".into()))
        ))
    );
}

#[test]
fn tactic_description_goes_to_the_delayed_slot() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let loc = loc(&[
        ("tactic_a", "Tactic A"),
        ("HEADER", "$KEY$\n"),
        ("TACTICS_DAMAGE_ATTACKER", "Attack"),
        ("TACTICS_VALUE_POSITIVE", "$VALUE|G=%0$"),
        ("TACTIC_CURRENTLY_COUNTERED", "Countered"),
        ("TACTIC_DESC", "What tactics do"),
        ("NO_TACTIC", "None"),
    ]);
    let content = tactics(b"tactic_a = { attacker = 0.2 }");
    assert_eq!(
        tactic_description(&NoIcons, &loc, &content, tactic_a(), true, false),
        Ok((
            "Tactic A\nAttack \u{11}G+20%\u{11}!\n".into(),
            Ok("What tactics do".into())
        ))
    );
    // The countered overlay shares both slots.
    assert_eq!(
        tactic_description(&NoIcons, &loc, &content, tactic_a(), true, true),
        Ok(("Tactic A\nCountered".into(), Ok("What tactics do".into())))
    );
    let none = LandCombatSideDetails::default();
    assert_eq!(
        tactic_tooltip(&NoIcons, &loc, &content, &none, &none, true),
        Ok(("None".into(), None))
    );
}

/// Every hover these builders cannot make names why, and a missing or
/// undrawable delayed description leaves the immediate text standing.
#[test]
fn refused_hovers_name_their_reason() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let entries = |extra: &[(&str, &str)]| -> Localisation {
        [("tactic_a", "Tactic A"), ("HEADER", "$KEY$\n")]
            .iter()
            .chain(extra)
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    };
    let content = tactics(b"tactic_a = { }");
    let missing = tactic_description(&NoIcons, &entries(&[]), &content, tactic_a(), true, false);
    assert!(
        // A missing top-level key shows its own bytes.
        matches!(&missing, Ok((_, Ok(text))) if text == "TACTIC_DESC"),
        "{missing:?}"
    );
    let flagged = entries(&[("TACTIC_DESC", "@GER")]);
    let flagged = tactic_description(&NoIcons, &flagged, &content, tactic_a(), true, false);
    assert!(
        matches!(&flagged, Ok((_, Err(reason))) if reason == "localisation TACTIC_DESC holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw")
    );
    let unknown = tactic_description(
        &NoIcons,
        &entries(&[]),
        &content,
        Described::Content {
            id: 1,
            token: b"tactic_b",
        },
        true,
        false,
    );
    assert_eq!(
        unknown,
        Err("recorded tactic id 1 is tactic_b, but the content's tactic 1 is tactic_a".into())
    );
    // The null tactic's missing name key shows as itself.
    let side = LandCombatSideDetails {
        tactic_kind: 1,
        ..Default::default()
    };
    let described = entries(&[("TACTIC_DESC", "What tactics do")]);
    assert_eq!(
        tactic_tooltip(
            &NoIcons,
            &described,
            &content,
            &side,
            &LandCombatSideDetails::default(),
            true
        ),
        Ok((
            "nullCombatTactic
"
            .into(),
            Some(Ok("What tactics do".into()))
        ))
    );
    let odd = LandCombatSideDetails {
        tactic_kind: 3,
        ..Default::default()
    };
    assert!(
        tactic_tooltip(&NoIcons, &entries(&[]), &content, &odd, &odd, true)
            .unwrap_err()
            .starts_with("the recorded tactic kind 3")
    );
    let loc = entries(&[]);
    assert!(skill_tooltip(&NoIcons, &loc, 3, 0, None, true, Ok(())).is_err());
    assert!(
        skill_tooltip(&NoIcons, &loc, 3, 0, Some(Some(0)), true, Ok(()))
            .unwrap()
            .contains("LEADER_SKILL_WITH_PROGRESS")
    );
    // A missing top-level key shows its own bytes.
    assert!(
        skill_tooltip(&NoIcons, &loc, 3, 0, Some(None), false, Ok(()))
            .unwrap()
            .contains("LEADER_SKILL_DESC")
    );
    assert_eq!(
        skill_tooltip(
            &NoIcons,
            &loc,
            3,
            0,
            Some(None),
            true,
            Err("x is registered")
        ),
        Err("the content's leader skill levels are unknown: x is registered".into())
    );
}

/// The authored `pdx_tooltip` is the text a widget is created with: a
/// dynamic builder's text, or its refusal, replaces it whichever binds
/// first (the game sets its hovers on every update), and an unowned
/// widget keeps the seed.
#[test]
fn authored_seeds_never_replace_a_dynamic_hover_or_its_refusal() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let windows = windows(
        br#"containerWindowType = { name = view
            buttonType = { name = dynamic pdx_tooltip = AUTHORED }
            buttonType = { name = refused pdx_tooltip = AUTHORED pdx_tooltip_delayed = AUTHORED }
            buttonType = { name = plain pdx_tooltip = AUTHORED }
        }"#,
    );
    let window = windows.gui_type(b"view").unwrap();
    let loc = loc(&[("AUTHORED", "Authored"), ("DYNAMIC", "Dynamic")]);
    let gui = Gui {
        icons: &NoIcons,
        loc: &loc,
        windows: &windows,
    };
    let mut values = Values::new();
    set_hover(&mut values, "dynamic", hover_text("Dynamic"), None);
    let mut refused = BTreeMap::new();
    authored(&gui, window, "", &mut values, &mut refused);
    // A refusal after the seed clears it too.
    refuse_hovers(&mut values, ["refused"]);
    let hover = |key: &str| {
        let widget = &values[key];
        (
            widget.tooltip.as_deref().map(str::to_owned),
            widget.tooltip_description.as_deref().map(str::to_owned),
        )
    };
    assert_eq!(hover("dynamic"), (Some("Dynamic".into()), None));
    assert_eq!(hover("refused"), (None, None));
    assert_eq!(hover("plain"), (Some("Authored".into()), None));
    assert!(refused.is_empty());
}

#[test]
fn authored_seeds_resolve_case_without_aliasing_dynamic_hovers() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let windows = windows(
        br#"containerWindowType = { name = view
        buttonType = { name = Dynamic pdx_tooltip = AUTHORED }
        buttonType = { name = Refused pdx_tooltip = AUTHORED pdx_tooltip_delayed = AUTHORED }
        containerWindowType = { name = Rows }
    }
    containerWindowType = { name = entry
        buttonType = { name = Dynamic pdx_tooltip = AUTHORED }
    }"#,
    );
    let loc = loc(&[("AUTHORED", "Authored")]);
    let gui = Gui {
        icons: &NoIcons,
        loc: &loc,
        windows: &windows,
    };
    let mut values = Values::new();
    set_hover(&mut values, "dynamic", hover_text("Dynamic"), None);
    refuse_hovers(&mut values, ["refused"]);
    let mut entry = Values::new();
    set_hover(&mut entry, "DYNAMIC", hover_text("Entry"), None);
    let rows = values.entry("rows".into()).or_default();
    rows.entry_template = Some("entry".into());
    rows.entries.push(entry);
    let mut refused = BTreeMap::new();
    authored(
        &gui,
        windows.gui_type(b"view").unwrap(),
        "",
        &mut values,
        &mut refused,
    );
    assert_eq!(values.len(), 3);
    assert_eq!(values["dynamic"].tooltip.as_deref(), Some("Dynamic"));
    assert!(values["refused"].tooltip.is_none());
    assert!(values["refused"].tooltip_description.is_none());
    assert_eq!(values["rows"].entries[0].len(), 1);
    assert_eq!(
        values["rows"].entries[0]["DYNAMIC"].tooltip.as_deref(),
        Some("Entry")
    );
    assert!(refused.is_empty(), "{refused:?}");
}

#[test]
fn authored_tooltip_hashes_preserve_fallback_and_count_invalid_keys() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let windows = windows(
        br#"containerWindowType = { name = view
        buttonType = { name = fallback tooltipText = MISSING }
        buttonType = { name = delayed delayedTooltipText = DELAYED }
        buttonType = { name = empty pdx_tooltip = "" }
        buttonType = { name = corrupt pdx_tooltip = MAIN }
        buttonType = { name = undecodable pdx_tooltip = MAIN }
        buttonType = { name = zero_corrupt pdx_tooltip = MAIN }
    }"#,
    );
    let mut window = windows.gui_type(b"view").unwrap().clone();
    assert_eq!(window.children[0].common.tooltip_key, b"MISSING");
    assert_eq!(
        window.children[0].common.tooltip,
        native::tooltip_hash(b"MISSING")
    );
    window.children[3].common.tooltip ^= 1;
    window.children[4].common.tooltip_key = vec![0xFF];
    window.children[4].common.tooltip = native::tooltip_hash(&[0xFF]);
    window.children[5].common.tooltip = 0;
    let localisation = loc(&[("DELAYED", "Description")]);
    let gui = Gui {
        icons: &NoIcons,
        loc: &localisation,
        windows: &windows,
    };
    let mut values = Values::new();
    let mut refused = BTreeMap::new();
    authored(&gui, &window, "", &mut values, &mut refused);
    assert_eq!(values["fallback"].tooltip.as_deref(), Some("MISSING"));
    assert_eq!(
        values["delayed"].tooltip_description.as_deref(),
        Some("Description")
    );
    assert!(values["empty"].tooltip.is_none());
    assert!(values["corrupt"].tooltip.is_none());
    assert!(values["undecodable"].tooltip.is_none());
    assert!(values["zero_corrupt"].tooltip.is_none());
    assert_eq!(refused.values().copied().sum::<usize>(), 3);
    assert!(
        refused
            .keys()
            .any(|reason| reason.starts_with("authored_tooltip_hash_mismatch: corrupt"))
    );
    assert!(
        refused
            .keys()
            .any(|reason| reason.starts_with("authored_tooltip_undecodable_key: undecodable"))
    );
    assert!(
        refused
            .keys()
            .any(|reason| reason.starts_with("authored_tooltip_hash_mismatch: zero_corrupt"))
    );
}

#[test]
fn hover_text_trims_only_the_four_ascii_bytes() {
    assert_eq!(hover_text(" \r\n\tname\n ").as_deref(), Some("name"));
    assert_eq!(
        hover_text("\u{a0}name\u{a0}").as_deref(),
        Some("\u{a0}name\u{a0}")
    );
    assert_eq!(hover_text(" \r\n\t"), None);
}

#[test]
fn null_tactic_object_can_counter_but_null_pointer_cannot() {
    let side = LandCombatSideDetails {
        tactic_kind: 2,
        tactic_counter_id: 0,
        ..Default::default()
    };
    let mut other = LandCombatSideDetails {
        tactic_kind: 1,
        tactic_id: 0,
        ..Default::default()
    };
    assert!(countered(&side, &other));
    other.tactic_kind = 0;
    assert!(!countered(&side, &other));
}

#[test]
fn organisation_effects_follow_active_tactics_and_validate_damage_sums() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = tactics(
        b"tactic_a = { attacker = 0.2 attacker_org_damage_modifier = 0.15 \
          defender_org_damage_modifier = -0.05 }",
    );
    let mut detail = LandCombatDetailsRow {
        combat: 42,
        attacker_bonus: 20_000,
        ..Default::default()
    };
    let mut sides = LandCombatSideRow {
        combat: 42,
        attacker_primary: vec![1],
        defender_primary: vec![2],
        ..Default::default()
    };
    detail.sides[0].tactic_kind = 2;
    detail.sides[0].tactic_id = 1;
    detail.sides[0].tactic_token = b"tactic_a".to_vec();
    assert_eq!(
        organisation_effects(&content, &detail, Some(&sides)),
        Ok([15_000, -5_000])
    );
    detail.attacker_bonus = 0;
    assert!(
        organisation_effects(&content, &detail, Some(&sides))
            .unwrap_err()
            .contains("damage sums")
    );
    detail.sides[1].tactic_kind = 1;
    assert_eq!(
        organisation_effects(&content, &detail, Some(&sides)),
        Ok([0, 0])
    );
    assert_eq!(
        organisation_effects(&content, &detail, None),
        Err("the combat's LandCombatSides row is missing".into())
    );
    sides.defender_primary.clear();
    sides.defender_secondary.push(2);
    assert_eq!(
        organisation_effects(&content, &detail, Some(&sides)),
        Err("the defender's primary country set is empty".into())
    );
    sides.defender_primary.push(0);
    assert_eq!(
        organisation_effects(&content, &detail, Some(&sides)),
        Err("the defender's first primary country is null".into())
    );
    sides.defender_primary[0] = 2;
    sides.combat = 43;
    assert!(organisation_effects(&content, &detail, Some(&sides)).is_err());
}

#[test]
fn cas_reports_the_opponents_air_defence_against_our_planes() {
    let loc = loc(&[
        ("ATTACKING", "attacking"),
        (
            "LAND_COMBAT_CAS_INFO_TOOLTIP",
            "$NUM$ $SIDE$ $STR|1$/$STR_MAX|1$ $ORG|1$/$ORG_MAX|1$ $KILL$ $STR_PREVENT|1$ $ORG_PREVENT|1$ $AA_EFFECTIVENESS|%0$",
        ),
        ("LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL", "max $MAX_PLANES$"),
    ]);
    let own = LandCombatSideDetails {
        air_counts: vec![100, 50],
        ..Default::default()
    };
    let opponent = LandCombatSideDetails {
        committed: vec![7],
        damage: [120_000, 350_000, 0, 0, 10_000, 20_000],
        air_kills: 3,
        ..Default::default()
    };
    let conditions = Conditions::from([(
        7,
        super::super::land_details::Condition {
            strength: 900_000,
            organisation: 6_000_000,
            ..Default::default()
        },
    )]);
    let defines = super::super::defines::LandDefines {
        air_superiority: [62_500_000, 75_000],
        anti_air: [75_000, 70_000],
        planes_per_width: 300_000,
        ..Default::default()
    };
    let cas = || Cas {
        own: &own,
        opponent: &opponent,
        attacking: true,
    };
    let gap = |unit: u64| {
        format!(
            "division {unit}'s condition is unknown: the DivisionStatus table has no row for it"
        )
    };
    // No anti-air: zero effectiveness; the width of 40 fits 120 planes.
    assert_eq!(
        cas_tooltip(
            &NoIcons,
            &loc,
            cas(),
            &conditions,
            &gap,
            Ok(4_000_000),
            &defines
        ),
        Ok((
            "150 attacking 1.2/9.0 3.5/60.0 3 0.1 0.2 0%".into(),
            Ok("max 120".into())
        ))
    );
    // The refusal names the opposing division and the table that lacks it.
    assert_eq!(
        cas_tooltip(&NoIcons, &loc, cas(), &Conditions::new(), &gap, Ok(0), &defines),
        Err("opposing committed division 7's condition is unknown: the DivisionStatus table has no row for it".into())
    );
    // A missing executable key shows its own bytes.
    let bare: Localisation = [("ATTACKING".to_owned(), "attacking".to_owned())]
        .into_iter()
        .collect();
    assert_eq!(
        cas_tooltip(&NoIcons, &bare, cas(), &conditions, &gap, Ok(0), &defines),
        Ok((
            "LAND_COMBAT_CAS_INFO_TOOLTIP".into(),
            Ok("LAND_COMBAT_CAS_INFO_TOOLTIP_DETAIL".into())
        ))
    );
}
