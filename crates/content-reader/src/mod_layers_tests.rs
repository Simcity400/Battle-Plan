
use crate::test_support::Stack;

#[test]
fn a_mod_adds_replaces_and_removes_idea_categories_and_ideas() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("ideas");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/idea_tags/00_idea.txt",
        "idea_categories = { political_advisor = { character_slot = political_advisor } \
         economy = { slot = economy_slot } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/00_idea.txt",
        "idea_categories = { political_advisor = { character_slot = political_advisor } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/mod_tags.txt",
        "idea_categories = { mod_council = { slot = mod_seat cost = @council_cost } }",
    );
    stack.write(
        &base,
        "common/ideas/b_base.txt",
        "ideas = { country = { base_spirit = { modifier = { stability_factor = 0.1 } } } }",
    );
    stack.write(
        &addon,
        "common/ideas/a_mod.txt",
        "@mod_cost = 150 \
         ideas = { mod_seat = { mod_law = { cost = @mod_cost removal_cost = [?scripted] \
         modifier = { stability_factor = [?mod_value]  } } } \
         country = { base_spirit = { modifier = { stability_factor = 0.5 } } } }",
    );
    stack.write(
        &addon,
        "common/ideas/nested/ignored.txt",
        "ideas = { country = { nested_spirit = { } } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/extra/ignored.txt",
        "idea_categories = { nested_category = { slot = nested_slot } }",
    );
    let layers = stack.layers(&[]);
    let (content, _, warnings) = crate::ideas::read(&layers, &[]).unwrap();
    let names: Vec<_> = content
        .tags
        .categories()
        .0
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["political_advisor", "mod_council"]);
    assert!(content.tags.resolve("economy").unwrap().is_none());
    assert!(content.tags.resolve("nested_category").unwrap().is_none());
    assert_eq!(
        content.tags.resolve("mod_council").unwrap().unwrap().cost,
        None
    );
    let law = content.idea("mod_law").unwrap().unwrap();
    assert_eq!((law.cost, law.removal_cost), (Some(150.0), None));
    assert!(content.idea("nested_spirit").unwrap().is_none());
    assert_eq!(
        content
            .idea("base_spirit")
            .unwrap()
            .unwrap()
            .modifiers
            .values[0]
            .value,
        10_000
    );
    let refused = warnings
        .iter()
        .find(|w| w.1.contains("failed native numeric conversion") && w.1.contains("mod_law"))
        .expect("refused forms are counted");
    for named in [
        "mod_law: removal_cost = [?scripted]",
        "mod_law: stability_factor = [?mod_value]",
    ] {
        assert!(refused.1.contains(named), "{named} in {refused:?}");
    }
    assert!(refused.1.contains("2 content values"), "{refused:?}");
    assert!(
        warnings
            .iter()
            .any(|w| w.1.contains("mod_council: cost = @council_cost")),
        "{warnings:?}"
    );
}

#[test]
fn a_mod_ideology_brings_its_colour_and_a_replaced_folder_drops_the_base_ones() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("ideologies");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/ideologies/00_ideologies.txt",
        "ideologies = { democratic = { types = { conservatism = { } } color = { 0 0 255 } } }",
    );
    stack.write(
        &addon,
        "common/ideologies/mod_ideologies.txt",
        "ideologies = { technocracy = { types = { futurism = { } } color = { 12 200 180 } } \
         mystic = { color = 16711680 } }",
    );
    let (added, warnings) = crate::ideologies::read(&stack.layers(&[])).unwrap();
    assert_eq!(
        added
            .groups()
            .0
            .iter()
            .map(|g| (g.token.as_str(), g.color.clone().ok()))
            .collect::<Vec<_>>(),
        [
            ("democratic", Some([0.0, 0.0, 1.0, 1.0])),
            (
                "technocracy",
                Some([12.0 / 255.0, 200.0 / 255.0, 180.0 / 255.0, 1.0])
            ),
            (
                "mystic",
                Some([113.0 / 255.0, 22.0 / 255.0, 128.0 / 255.0, 22.0 / 255.0])
            )
        ]
    );
    assert_eq!(
        added.group_of("futurism").unwrap().unwrap().token,
        "technocracy"
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let (replaced, _) = crate::ideologies::read(&stack.layers(&["common/ideologies"])).unwrap();
    assert!(replaced.get("democratic").unwrap().is_none());
    assert!(replaced.get("technocracy").unwrap().is_some());
}

#[test]
fn every_portrait_role_the_engine_accepts_is_read_and_others_are_refused_by_name() {
    let stack = Stack::new("characters");
    stack.write(
        &stack.addon(),
        "common/characters/mod_characters.txt",
        "characters = { \
           MOD_pilot = { portraits = { air = { small = GFX_pilot_small } } } \
           MOD_spy = { portraits = { operative = { small = GFX_spy_small } } } \
           MOD_scholar = { portraits = { scientist = { small = GFX_scholar_small } } } \
           MOD_wrong = { portraits = { airforce = { small = GFX_wrong } } } \
           MOD_adviser = { advisor = { slot = political_advisor } } }",
    );
    let (characters, warnings) =
        crate::characters::read(&stack.layers(&[]), Default::default()).unwrap();
    let small = |token: &str, role: &str| {
        characters
            .portraits(token)
            .unwrap()
            .unwrap()
            .get(&(role.to_owned(), "small".to_owned()))
            .cloned()
    };
    assert_eq!(
        small("MOD_pilot", "air"),
        Some(crate::characters::Authored::Asset("GFX_pilot_small".into()))
    );
    assert_eq!(
        small("MOD_spy", "operative"),
        Some(crate::characters::Authored::Asset("GFX_spy_small".into()))
    );
    assert_eq!(
        small("MOD_scholar", "scientist"),
        Some(crate::characters::Authored::Asset(
            "GFX_scholar_small".into()
        ))
    );
    let refused = warnings
        .iter()
        .map(|(_, message)| message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        refused.contains("MOD_wrong: portrait role `airforce` is not one the game reads"),
        "{refused}"
    );
    assert!(
        refused.contains("MOD_adviser: advisor idea_token or slot missing"),
        "{refused}"
    );
}

#[test]
fn one_unreadable_mod_sprite_costs_only_itself_and_keys_match_without_case() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("sprites");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "interface/base.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_x texturefile = gfx/interface/base_x.dds } }",
    );
    stack.write(
        &addon,
        "interface/zz_mod.gfx",
        "spriteTypes = { \
           SpriteType = { name = GFX_idea_x TextureFile = \"gfx//interface/mod_x.tga\" NoOfFrames = 2 } \
           spriteType = { name = GFX_broken texturefile = gfx/interface/b.dds noOfFrames = -2 } \
           spriteType = { name = GFX_fine texturefile = gfx/interface/f.dds } }",
    );
    let layers = stack.layers(&[]);
    let set = crate::gfx::read(&layers).unwrap();
    let idea = set.lookup("GFX_idea_x").unwrap();
    assert_eq!(
        (idea.texture.as_deref(), idea.frames),
        (Some("gfx/interface/mod_x.tga"), 2)
    );
    assert!(set.contains("GFX_fine"));
    assert!(set.find("GFX_broken").is_err());
    assert_eq!(set.refused.len(), 1);
    assert!(set.refused[0].contains("GFX_broken"), "{:?}", set.refused);
    assert!(
        set.refusal("GFX_broken")
            .unwrap()
            .contains("noOfFrames is -2")
    );
}

#[test]
fn a_refused_sprite_override_withdraws_the_earlier_declaration() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("sprite_override");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "interface/base.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_x texturefile = gfx/interface/base_x.dds } \
         spriteType = { name = GFX_idea_y texturefile = gfx/interface/base_y.dds } }",
    );
    stack.write(
        &addon,
        "interface/zz_mod.gfx",
        "spriteTypes = { \
           spriteType = { name = GFX_idea_x texturefile = gfx/interface/mod_x.dds noOfFrames = -2 } \
           spriteType = { name = GFX_idea_y texturefile = gfx/interface/mod_y.dds noOfFrames = -2 } \
           spriteType = { name = GFX_idea_y texturefile = gfx/interface/mod_y2.dds } }",
    );
    let set = crate::gfx::read(&stack.layers(&[])).unwrap();
    let reason = set.lookup("GFX_idea_x").unwrap_err();
    assert!(
        reason.contains("zz_mod.gfx") && reason.contains("GFX_idea_x"),
        "{reason}"
    );
    assert_eq!(
        set.lookup("GFX_idea_y").unwrap().texture.as_deref(),
        Some("gfx/interface/mod_y2.dds")
    );
    assert!(set.find("GFX_none").unwrap().is_none());
}

#[test]
fn a_tree_unreadable_sprite_file_keeps_its_native_prefix_and_later_replacements() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("sprite_unreadable");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "interface/base.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_x texturefile = gfx/interface/base_x.dds } \
         spriteType = { name = GFX_idea_y texturefile = gfx/interface/base_y.dds } \
         spriteType = { name = GFX_idea_z texturefile = gfx/interface/base_z.dds } }",
    );
    let broken = "spriteTypes = { spriteType = { name = GFX_idea_x texturefile = gfx/interface/mod_x.dds } \
                  spriteType = { name = \"GFX_idea_z\" texturefile = gfx/interface/mod_z.dds } } } \
                  spriteTypes = { spriteType = { name = GFX_other } }";
    assert!(crate::parser::parse(broken.as_bytes()).is_err());
    stack.write(&addon, "interface/mod.gfx", broken);
    stack.write(
        &addon,
        "interface/zz_mod.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_z texturefile = gfx/interface/zz_z.dds } }",
    );
    let set = crate::gfx::read(&stack.layers(&[])).unwrap();
    assert_eq!(
        set.lookup("GFX_idea_x").unwrap().texture.as_deref(),
        Some("gfx/interface/mod_x.dds")
    );
    assert_eq!(
        set.lookup("GFX_idea_y").unwrap().texture.as_deref(),
        Some("gfx/interface/base_y.dds")
    );
    assert_eq!(
        set.lookup("GFX_idea_z").unwrap().texture.as_deref(),
        Some("gfx/interface/zz_z.dds")
    );
    assert!(set.find("GFX_other").unwrap().is_none());
    assert!(set.refused.is_empty(), "{:?}", set.refused);
}

#[test]
fn a_mod_resource_takes_its_frame_through_a_file_variable() {
    let stack = Stack::new("resources");
    stack.write(
        &stack.base(),
        "common/resources/00_resources.txt",
        "resources = { oil = { icon_frame = 1 } }",
    );
    stack.write(
        &stack.addon(),
        "common/resources/mod_resources.txt",
        "@ore_frame = 7 resources = { mod_ore = { icon_frame = @ore_frame } mod_gas = { icon_frame = @missing } }",
    );
    let (resources, warnings) = crate::resources::read(&stack.layers(&[])).unwrap();
    assert_eq!(
        resources
            .declarations
            .iter()
            .map(|r| (r.token.as_str(), r.icon_frame))
            .collect::<Vec<_>>(),
        [("oil", Some(1)), ("mod_ore", Some(7)), ("mod_gas", None)]
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.1.contains("mod_gas: icon_frame = @missing")),
        "{warnings:?}"
    );
}

fn resource_icon(stack: &Stack) -> (std::io::Result<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let sprite = crate::gui::native::window_button_sprite(
        &crate::gui::native::read_layers(&stack.layers(&[])).unwrap(),
        "resources_info_entry",
        "resources_icon",
        &mut warnings,
    );
    (sprite, warnings)
}

#[test]
fn a_gui_window_is_found_in_whichever_file_declares_it_and_a_later_file_replaces_it() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("gui");
    stack.write(
        &stack.addon(),
        "interface/mod/mod_trade.gui",
        "guiTypes = { containerWindowType = { name = resources_info_entry \
         buttonType = { name = resources_icon spriteType = GFX_mod_resources_strip } } }",
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_mod_resources_strip");
    assert!(warnings.is_empty(), "{warnings:?}");
    stack.write(
        &stack.base(),
        "interface/countrytradeview.gui",
        "guiTypes = { containerWindowType = { name = resources_info_entry \
         buttonType = { name = resources_icon spriteType = GFX_resources_strip } } }",
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_mod_resources_strip");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("resources_info_entry is declared 2 times"),
        "{warnings:?}"
    );
    assert!(
        warnings[0].contains("interface/mod/mod_trade.gui"),
        "{warnings:?}"
    );
    stack.write(
        &stack.addon(),
        "interface/load_screen.gui",
        "guiTypes = { containerWindowType = { name = resources_info_entry \
         buttonType = { name = resources_icon spriteType = GFX_load_screen } } }",
    );
    assert_eq!(
        crate::gui::native::read_layers(&stack.layers(&[]))
            .unwrap()
            .files[0]
            .0,
        "interface/load_screen.gui"
    );
    assert_eq!(resource_icon(&stack).0.unwrap(), "GFX_mod_resources_strip");
}

#[test]
fn every_declaration_of_a_gui_window_in_one_file_is_weighed_as_the_registry_does() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("gui_one_file");
    let row = |sprite: &str| {
        format!(
            "containerWindowType = {{ name = resources_info_entry \
             buttonType = {{ name = resources_icon spriteType = {sprite} }} }}"
        )
    };
    stack.write(
        &stack.addon(),
        "interface/mod_trade.gui",
        &format!("guiTypes = {{ {} {} }}", row("GFX_A"), row("GFX_B")),
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_A");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("resources_info_entry is declared 2 times"),
        "{warnings:?}"
    );
    stack.write(
        &stack.addon(),
        "interface/mod_trade.gui",
        &format!(
            "guiTypes = {{ {} {} {} }}",
            row("GFX_A"),
            row("GFX_B"),
            row("GFX_C")
        ),
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_C");
    assert!(
        warnings[0].contains("resources_info_entry is declared 3 times"),
        "{warnings:?}"
    );
    stack.write(
        &stack.addon(),
        "interface/mod_trade.gui",
        &format!(
            "guiTypes = {{ {} {} }} guiTypes = {{ {} }}",
            row("GFX_A"),
            row("GFX_B"),
            row("GFX_D")
        ),
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_D");
    assert!(
        warnings[0].contains("resources_info_entry is declared 3 times"),
        "{warnings:?}"
    );
    stack.write(
        &stack.addon(),
        "interface/mod_trade.gui",
        &format!(
            "guiTypes = {{ {} {} iconType = {{ spriteType = GFX_unnamed }} }}",
            row("GFX_A"),
            row("GFX_B")
        ),
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_A");
    assert!(
        warnings[0].contains("resources_info_entry is declared 2 times"),
        "{warnings:?}"
    );
    stack.write(
        &stack.addon(),
        "interface/mod_trade.gui",
        &format!(
            "guiTypes = {{ {} {} iconType = {{ spriteType = GFX_unnamed }} }} guiTypes = {{ {} }}",
            row("GFX_A"),
            row("GFX_B"),
            row("GFX_C")
        ),
    );
    assert_eq!(resource_icon(&stack).0.unwrap(), "GFX_C");
    stack.write(
        &stack.addon(),
        "interface/mod_trade.gui",
        &format!(
            "guiTypes = {{ {} {} iconType = {{ spriteType = GFX_unnamed }} }}",
            row("GFX_A"),
            row("GFX_B")
        ),
    );
    stack.write(
        &stack.addon(),
        "interface/zz_trade.gui",
        &format!("guiTypes = {{ {} }}", row("GFX_D")),
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_D");
    assert!(
        warnings[0].starts_with("resources_info_entry is declared 3 times"),
        "{warnings:?}"
    );
}

#[test]
fn a_gui_override_is_closed_at_eof_and_a_stray_close_ends_the_file() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("gui_unclosed");
    stack.write(
        &stack.base(),
        "interface/topbar.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_shield_medium } } }",
    );
    let flag = |stack: &Stack| {
        let mut warnings = Vec::new();
        let sprite = crate::gui::native::window_button_sprite(
            &crate::gui::native::read_layers(&stack.layers(&[])).unwrap(),
            "topbar",
            "player_flag",
            &mut warnings,
        );
        (sprite, warnings)
    };
    stack.write(
        &stack.addon(),
        "interface/zz_mod.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_mod_shield } }",
    );
    assert_eq!(flag(&stack).0.unwrap(), "GFX_mod_shield");
    let broken = "guiTypes = { containerWindowType = { name = topbar } } } \
                  guiTypes = { containerWindowType = { name = other } }";
    assert!(crate::parser::parse(broken.as_bytes()).is_err());
    stack.write(&stack.addon(), "interface/zz_mod.gui", broken);
    let (sprite, warnings) = flag(&stack);
    let error = sprite.unwrap_err().to_string();
    assert!(error.contains("topbar has no player_flag"), "{error}");
    assert!(
        warnings[0].contains("topbar is declared 2 times"),
        "{warnings:?}"
    );
    stack.write(
        &stack.addon(),
        "interface/zzz_mod.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_later } } }",
    );
    assert_eq!(flag(&stack).0.unwrap(), "GFX_later");
    stack.write(
        &stack.addon(),
        "interface/zzz_mod.gui",
        "guiTypes = { containerWindowType = { name = topbar_extra } } }",
    );
    stack.write(
        &stack.addon(),
        "interface/zz_mod.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_mod_shield } } }",
    );
    let (sprite, warnings) = flag(&stack);
    assert_eq!(sprite.unwrap(), "GFX_mod_shield");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("topbar is declared 2 times")),
        "{warnings:?}"
    );
}

#[test]
fn a_gui_button_is_the_first_direct_button_of_its_name_in_its_own_window() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("gui_scope");
    stack.write(
        &stack.base(),
        "interface/topbar.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_shield_medium } } }",
    );
    stack.write(
        &stack.addon(),
        "interface/mod_diplomacy.gui",
        "guiTypes = { containerWindowType = { name = topbar_extra \
         buttonType = { name = player_flag quadTextureSprite = GFX_mod_flag } } \
         iconType = { name = player_flag spriteType = GFX_mod_icon } }",
    );
    let flag = |stack: &Stack| {
        crate::gui::native::window_button_sprite(
            &crate::gui::native::read_layers(&stack.layers(&[])).unwrap(),
            "topbar",
            "player_flag",
            &mut Vec::new(),
        )
    };
    assert_eq!(flag(&stack).unwrap(), "GFX_shield_medium");
    for (window, expected) in [
        (
            "containerWindowType = { name = TopBar \
             buttonType = { name = Player_Flag quadTextureSprite = GFX_cased } }",
            Ok("GFX_cased"),
        ),
        (
            "containerWindowType = { name = topbar containerWindowType = { name = inner \
             buttonType = { name = player_flag quadTextureSprite = GFX_nested } } }",
            Err("has no player_flag"),
        ),
        (
            "containerWindowType = { name = topbar \
             iconType = { name = player_flag spriteType = GFX_icon } \
             buttonType = { name = player_flag quadTextureSprite = GFX_later } }",
            Err("has no player_flag button"),
        ),
        (
            "containerWindowType = { name = topbar \
             buttonType = { name = player_flag quadTextureSprite = GFX_first } \
             buttonType = { name = player_flag quadTextureSprite = GFX_second } }",
            Ok("GFX_first"),
        ),
        (
            "windowType = { name = topbar \
             buttonType = { name = player_flag quadTextureSprite = GFX_window } }",
            Err("is windowType, not the containerWindowType"),
        ),
    ] {
        stack.write(
            &stack.addon(),
            "interface/zz_topbar.gui",
            &format!("guiTypes = {{ {window} }}"),
        );
        match (flag(&stack), expected) {
            (Ok(sprite), Ok(expected)) => assert_eq!(sprite, expected, "{window}"),
            (Err(error), Err(expected)) => {
                assert!(error.to_string().contains(expected), "{window}: {error}")
            }
            (result, _) => panic!("{window}: {result:?}"),
        }
    }
}

#[test]
fn a_gui_unread_suffix_refuses_only_until_a_later_declaration_settles_it() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("gui_poison");
    let row = "guiTypes = { containerWindowType = { name = resources_info_entry \
               buttonType = { name = resources_icon spriteType = GFX_kept } } }";
    stack.write(&stack.base(), "interface/00_row.gui", row);
    stack.write(
        &stack.addon(),
        "interface/zz_poison.gui",
        "guiTypes = { containerWindowType = { name = other \
         gridBoxType = { name = grid max_slots = { 1 2 3 4 q = 5 } } } }",
    );
    let (sprite, _) = resource_icon(&stack);
    let error = sprite.unwrap_err().to_string();
    assert!(
        error.contains("zz_poison.gui") && error.contains("unread suffix"),
        "{error}"
    );
    stack.write(&stack.addon(), "interface/zzz_row.gui", row);
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_kept");
    assert!(
        warnings.iter().all(|warning| !warning.contains("poison")),
        "{warnings:?}"
    );
}

#[test]
fn unrelated_gui_refusals_do_not_reach_a_window_lookup() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("gui_unrelated");
    stack.write(
        &stack.base(),
        "interface/row.gui",
        "guiTypes = { containerWindowType = { name = resources_info_entry \
         buttonType = { name = resources_icon spriteType = GFX_kept } } }",
    );
    stack.write(
        &stack.addon(),
        "interface/zz_other.gui",
        "guiTypes = { containerWindowType = { name = other \
         extendedScrollbarType = { name = scroll background = { parent = missing } } } }",
    );
    let guis = crate::gui::native::read_layers(&stack.layers(&[])).unwrap();
    let unrelated = guis.template("other").unwrap().subtree_refusals();
    assert!(!unrelated.is_empty());
    assert!(
        unrelated
            .iter()
            .all(|refusal| refusal.cause != crate::gui::native::Cause::Unaligned)
    );
    let (sprite, warnings) = resource_icon(&stack);
    assert_eq!(sprite.unwrap(), "GFX_kept");
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn resource_and_modifier_integer_notes_do_not_count_misread_blocks_as_scans() {
    let stack = Stack::new("integer_notes");
    for (path, key) in [
        ("common/resources/mod.txt", "icon_frame"),
        ("common/modifier_definitions/mod.txt", "precision"),
    ] {
        let body = format!("mod_value = {{ {key} = {{ }} }}");
        let text = if key == "icon_frame" {
            format!("resources = {{ {body} }}")
        } else {
            body
        };
        stack.write(&stack.addon(), path, &text);
    }
    let layers = stack.layers(&[]);
    let (_, resource_notes) = crate::resources::read(&layers).unwrap();
    let (_, definition_notes) = crate::modifier_definitions::read(&layers).unwrap();
    for notes in [resource_notes, definition_notes] {
        assert!(!notes.is_empty());
        assert!(
            notes
                .iter()
                .all(|note| !note.1.contains("numeric conversion")),
            "{notes:?}"
        );
    }
    std::fs::write(
        stack.addon().join("common/resources/mod.txt"),
        b"resources = { mod_value = { icon_frame = \"\xfe\" } }",
    )
    .unwrap();
    std::fs::write(
        stack.addon().join("common/modifier_definitions/mod.txt"),
        b"mod_value = { precision = \"\xfe\" }",
    )
    .unwrap();
    let layers = stack.layers(&[]);
    let (_, resource_notes) = crate::resources::read(&layers).unwrap();
    let (_, definition_notes) = crate::modifier_definitions::read(&layers).unwrap();
    for (notes, key) in [
        (resource_notes, "icon_frame"),
        (definition_notes, "precision"),
    ] {
        assert!(
            notes
                .iter()
                .any(|note| note.1.contains(&format!("{key} = \u{fffd}"))),
            "{notes:?}"
        );
    }
}

#[test]
fn a_mod_may_blank_a_base_building_file() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("buildings");
    stack.write(
        &stack.base(),
        "common/buildings/00_buildings.txt",
        "buildings = { infrastructure = { max_level = 5 } }",
    );
    stack.write(
        &stack.base(),
        "common/buildings/01_extra.txt",
        "buildings = { mod_removed = { max_level = 1 } }",
    );
    stack.write(&stack.addon(), "common/buildings/01_extra.txt", "");
    let buildings = crate::buildings::read(&stack.layers(&[])).unwrap();
    assert!(buildings.types.contains_key("infrastructure"));
    assert!(!buildings.types.contains_key("mod_removed"));
    assert!(
        crate::buildings::parse(b"", &crate::key_names::KeyNames::default())
            .unwrap()
            .types
            .is_empty()
    );
}

#[test]
fn malformed_redeclarations_follow_each_native_database_s_duplicate_rule() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("withdrawn");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/idea_tags/00_idea.txt",
        "idea_categories = { economy = { slot = economy } kept = { slot = kept } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/zz_mod.txt",
        "idea_categories = { economy = yes }",
    );
    stack.write(
        &base,
        "common/ideologies/00_ideologies.txt",
        "ideologies = { democratic = { color = { 0 0 255 } } kept = { color = { 1 2 3 } } }",
    );
    stack.write(
        &addon,
        "common/ideologies/zz_mod.txt",
        "ideologies = { democratic = yes }",
    );
    stack.write(
        &base,
        "common/country_leader/00_traits.txt",
        "leader_traits = { tough = { army_morale_factor = 0.1 } }",
    );
    stack.write(
        &addon,
        "common/country_leader/zz_mod.txt",
        "leader_traits = { tough = 1 }",
    );
    stack.write(
        &base,
        "common/modifier_definitions/00_defs.txt",
        "mod_speed = { value_type = percentage precision = 1 }",
    );
    stack.write(
        &addon,
        "common/modifier_definitions/zz_mod.txt",
        "@x = 1 mod_speed = fast",
    );
    stack.write(
        &base,
        "common/characters/00_chars.txt",
        "characters = { MOD_hero = { name = MOD_hero portraits = { civilian = { small = GFX_hero } } \
         advisor = { slot = political_advisor idea_token = MOD_hero_idea } } }",
    );
    stack.write(
        &addon,
        "common/characters/zz_mod.txt",
        "characters = { MOD_hero = yes }",
    );
    stack.write(
        &base,
        "common/resources/00_resources.txt",
        "resources = { oil = { icon_frame = 1 } }",
    );
    stack.write(
        &addon,
        "common/resources/zz_mod.txt",
        "resources = { oil = broken oil = { icon_frame = 5 } }",
    );
    let layers = stack.layers(&[]);

    let (ideas, _, warnings) = crate::ideas::read(&layers, &[]).unwrap();
    assert_eq!(
        ideas.tags.resolve("economy").unwrap().unwrap().slots,
        ["economy"]
    );
    assert!(ideas.tags.resolve("kept").unwrap().is_some());
    assert!(!warnings.is_empty(), "{warnings:?}");

    let (ideologies, warnings) = crate::ideologies::read(&layers).unwrap();
    assert_eq!(
        ideologies.get("democratic").unwrap().unwrap().color,
        Ok([0.0, 0.0, 1.0, 1.0])
    );
    assert!(ideologies.get("kept").unwrap().is_some());
    assert!(!warnings.is_empty(), "{warnings:?}");

    let (traits, warnings) = crate::leader_traits::read(&layers).unwrap();
    assert_eq!(
        traits.get("tough").unwrap().unwrap().modifiers.values[0].value,
        10_000
    );
    assert!(!warnings.is_empty(), "{warnings:?}");

    let (definitions, warnings) = crate::modifier_definitions::read(&layers).unwrap();
    assert_eq!(definitions.get("mod_speed").unwrap().precision, 1);
    assert!(!warnings.is_empty(), "{warnings:?}");

    let (characters, warnings) = crate::characters::read(&layers, Default::default()).unwrap();
    assert!(matches!(
        characters.portraits("MOD_hero").unwrap().unwrap()
            [&("civilian".to_owned(), "small".to_owned())],
        crate::characters::Authored::Ambiguous(_)
    ));
    assert!(!warnings.is_empty(), "{warnings:?}");

    let (resources, warnings) = crate::resources::read(&layers).unwrap();
    assert_eq!(
        resources
            .declarations
            .iter()
            .map(|r| (r.token.as_str(), r.icon_frame))
            .collect::<Vec<_>>(),
        [("oil", Some(1)), ("oil", None)]
    );
    assert!(!warnings.is_empty(), "{warnings:?}");
    stack.write(
        &addon,
        "common/resources/zzz_later.txt",
        "resources = { oil = { icon_frame = 5 } }",
    );
    let (resources, _) = crate::resources::read(&stack.layers(&[])).unwrap();
    assert_eq!(
        resources
            .declarations
            .iter()
            .map(|r| (r.token.as_str(), r.icon_frame))
            .collect::<Vec<_>>(),
        [("oil", Some(1)), ("oil", None), ("oil", Some(5))]
    );
}

#[test]
fn name_reader_kinds_follow_logged_fields() {
    content_reader_script::test_game::enter();
    let stack = Stack::new("name_kinds");
    stack.write(
        &stack.addon(),
        "common/units/names_divisions/mod.txt",
        "Mod = { type = unknown_type prefix = { lost } }",
    );
    let layers = stack.layers(&[]);
    let (_, notes) = crate::division_names::read(&layers).unwrap();
    assert!(
        notes
            .iter()
            .any(|(kind, _)| *kind == crate::DiagnosticKind::Logged)
    );
    assert!(
        notes
            .iter()
            .any(|(kind, _)| *kind == crate::DiagnosticKind::Misread)
    );
    let groups = crate::raids::names::Groups::read(&layers).unwrap();
    assert!(
        groups
            .warnings
            .iter()
            .any(|(kind, _)| *kind == crate::DiagnosticKind::Logged)
    );
}
