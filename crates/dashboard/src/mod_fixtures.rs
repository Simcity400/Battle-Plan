//! Modded content stacks for the Overview widgets' products: a base game layer
//! and a mod layer (`ResolvedLayers::from_folders`), read into a catalogue
//! exactly as a recording's content is, then joined by each product's own
//! code. Each test names the widget whose content it covers.

use crate::{
    catalogue::{self, Catalogue},
    products::{
        country_flags::{FlagSearch, ShieldStyle, find_flag},
        idea_history::{Joined, categories},
        party_history::ideology_infos,
        politics_content::{CountryLook, IdeaCountry, PoliticsContent},
        resources::{ContentView, resource_content},
        validate_tag,
    },
};
use installed_content::{ResolvedLayers, image::Rgba8Image};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

/// A base game folder and a mod folder under a fresh temporary root.
struct Stack {
    root: PathBuf,
}

impl Stack {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_dashboard_mod_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        Self { root }
    }

    fn base(&self) -> PathBuf {
        self.root.join("base")
    }

    fn addon(&self) -> PathBuf {
        self.root.join("mod")
    }

    fn write(&self, layer: &Path, path: &str, bytes: impl AsRef<[u8]>) {
        let path = layer.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn catalogue(&self, replace_paths: &[&str]) -> Arc<Catalogue> {
        let (base, addon) = (self.base(), self.addon());
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&addon).unwrap();
        let layers =
            ResolvedLayers::from_folders(&[(&base, &[]), (&addon, replace_paths)]).unwrap();
        Arc::new(Catalogue::of_layers(layers, "english").unwrap())
    }

    fn politics(&self, replace_paths: &[&str]) -> PoliticsContent {
        self.write(
            &self.base(),
            "common/defines/00_fixture.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, \
             NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_officers\" } } }",
        );
        PoliticsContent::load(self.catalogue(replace_paths)).unwrap()
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// An uncompressed top-left 32-bit Targa of one row of RGBA pixels.
fn tga_row(pixels: &[[u8; 4]]) -> Vec<u8> {
    let width = u16::try_from(pixels.len()).unwrap().to_le_bytes();
    let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&[width[0], width[1], 1, 0, 32, 0x28]);
    for [r, g, b, a] in pixels {
        bytes.extend_from_slice(&[*b, *g, *r, *a]);
    }
    bytes
}

/// The data URL an icon reply carries for one opaque pixel.
fn pixel_png(rgba: [u8; 4]) -> String {
    catalogue::png_data_url(&Rgba8Image {
        width: 1,
        height: 1,
        pixels: rgba.to_vec(),
    })
    .unwrap()
}

fn fixture_country() -> IdeaCountry<'static> {
    IdeaCountry {
        tag: "MOD",
        slot: 0,
        look: Some(CountryLook {
            name_tag: "",
            cosmetic_tag: "",
            civil_war: Ok(false),
        }),
    }
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GOLD: [u8; 4] = [255, 200, 0, 255];

/// Resource balance: a mod resource shows with its own name and icon frame, a
/// mod that moves the trade row into its own GUI file keeps every icon, and a
/// renamed base resource takes the mod's name.
#[test]
fn resource_balance_names_and_draws_a_mod_resource_from_the_mod_s_own_art() {
    let stack = Stack::new("resources");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/resources/00_resources.txt",
        "resources = { oil = { icon_frame = 1 } steel = { icon_frame = 2 } }",
    );
    stack.write(
        &base,
        "localisation/english/resources_l_english.yml",
        "\u{feff}l_english:\n PRODUCTION_MATERIALS_OIL:0 \"Oil\"\n PRODUCTION_MATERIALS_STEEL:0 \"Steel\"\n",
    );
    stack.write(
        &base,
        "interface/resources.gfx",
        "spriteTypes = { spriteType = { name = GFX_resources_strip \
         texturefile = gfx/interface/resources_strip.tga noOfFrames = 2 } }",
    );
    stack.write(
        &base,
        "gfx/interface/resources_strip.tga",
        tga_row(&[RED, GREEN]),
    );
    // The mod declares the trade row in a GUI file of its own, with its own strip.
    stack.write(
        &addon,
        "interface/mod/mod_trade.gui",
        "guiTypes = { containerWindowType = { name = resources_info_entry \
         buttonType = { name = resources_icon spriteType = GFX_mod_resources_strip } } }",
    );
    stack.write(
        &addon,
        "interface/mod_resources.gfx",
        "spriteTypes = { spriteType = { name = GFX_mod_resources_strip \
         texturefile = gfx/interface/mod_strip.tga noOfFrames = 3 } }",
    );
    stack.write(
        &addon,
        "gfx/interface/mod_strip.tga",
        tga_row(&[RED, GREEN, BLUE]),
    );
    stack.write(
        &addon,
        "common/resources/mod_resources.txt",
        "resources = { mod_ore = { icon_frame = 3 } }",
    );
    stack.write(
        &addon,
        "localisation/english/mod_l_english.yml",
        "\u{feff}l_english:\n PRODUCTION_MATERIALS_MOD_ORE:0 \"Mod Ore\"\n",
    );
    stack.write(
        &addon,
        "localisation/english/replace/mod_names_l_english.yml",
        "\u{feff}l_english:\n PRODUCTION_MATERIALS_OIL:0 \"Petroleum\"\n",
    );
    let catalogue = stack.catalogue(&[]);
    let wanted = [
        ("oil".to_owned(), 1),
        ("steel".to_owned(), 1),
        ("mod_ore".to_owned(), 1),
    ];
    let (mut declared, mut warnings) = (BTreeMap::new(), Vec::new());
    resource_content(
        &ContentView::of(&catalogue),
        &wanted,
        &mut declared,
        &mut warnings,
        &AtomicBool::new(false),
    )
    .unwrap();
    let shown = |token: &str| {
        let found = &declared[&(token.to_owned(), 1)];
        (found.name.clone(), found.icon.clone())
    };
    assert_eq!(
        shown("mod_ore"),
        (Some("Mod Ore".into()), Some(pixel_png(BLUE)))
    );
    assert_eq!(
        shown("oil"),
        (Some("Petroleum".into()), Some(pixel_png(RED)))
    );
    assert_eq!(shown("steel").1, Some(pixel_png(GREEN)));
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// Country flags: a cosmetic tag naming a subfolder of the flag folder draws
/// that file, and one this reader cannot resolve leaves the country's flag
/// unavailable instead of drawing a lower candidate.
#[test]
fn a_cosmetic_flag_in_a_subfolder_is_drawn_and_an_unresolvable_one_never_falls_through() {
    let stack = Stack::new("flag_variants");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(&base, "gfx/flags/AAA.tga", tga_row(&[RED]));
    stack.write(&addon, "gfx/flags/variants/red.tga", tga_row(&[BLUE]));
    let catalogue = stack.catalogue(&[]);
    let layers = &catalogue.content.layers;
    let sets = [installed_content::flags::FlagSet::Normal];
    let search = |candidates: &[&str]| {
        let candidates: Vec<String> = candidates.iter().map(|key| key.to_string()).collect();
        find_flag(layers, &sets, &[[1, 1]; 3], &candidates).unwrap()
    };
    match search(&["variants/red", "AAA"]) {
        FlagSearch::Found { asset, image } => {
            assert_eq!(asset, "gfx/flags/variants/red.tga");
            assert_eq!(image.pixels, BLUE);
        }
        other => panic!("{other:?}"),
    }
    // A missing preferred file still falls through, as the engine does.
    match search(&["variants/blue", "AAA"]) {
        FlagSearch::Found { asset, .. } => assert_eq!(asset, "gfx/flags/AAA.tga"),
        other => panic!("{other:?}"),
    }
    for key in ["variants\\red", "../AAA", "variants/../AAA", "/AAA"] {
        match search(&[key, "AAA"]) {
            FlagSearch::Unresolvable(found) => assert_eq!(found, key),
            other => panic!("{key}: {other:?}"),
        }
    }
    assert!(matches!(search(&["BBB"]), FlagSearch::Absent));
}

/// Country flags: the shield is the top bar's own `player_flag`. A mod window
/// elsewhere with a `player_flag` of its own, drawing other art, costs nothing,
/// and a mod that redeclares the top bar in a later file wins, with a warning.
#[test]
fn shield_flags_read_the_top_bar_s_own_flag_button_whatever_other_windows_declare() {
    let stack = Stack::new("shield");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "interface/topbar.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_shield_medium } } }",
    );
    stack.write(
        &base,
        "interface/general_stuff.gfx",
        "spriteTypes = { maskedShieldType = { name = GFX_shield_medium \
         textureFile1 = gfx/interface/shield_overlay.tga \
         textureFile2 = gfx/interface/shield_mask.tga } }",
    );
    stack.write(&base, "gfx/interface/shield_overlay.tga", tga_row(&[RED]));
    stack.write(&base, "gfx/interface/shield_mask.tga", tga_row(&[GREEN]));
    // A mod panel, in a file that also names the top bar, has its own
    // `player_flag` button with an undeclared sprite.
    stack.write(
        &addon,
        "interface/mod_panel.gui",
        "guiTypes = { containerWindowType = { name = mod_topbar_panel \
         buttonType = { name = player_flag quadTextureSprite = GFX_mod_undeclared } } }",
    );
    let style = ShieldStyle::load(&stack.catalogue(&[])).unwrap();
    assert_eq!(style.warnings, Vec::<String>::new());
    // The mod now replaces the top bar in a file loaded after the base one.
    stack.write(
        &addon,
        "interface/zz_mod_topbar.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_mod_undeclared } } }",
    );
    let error = ShieldStyle::load(&stack.catalogue(&[]))
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("GFX_mod_undeclared is not declared")
            && error.contains("GFX_default_fallback_texture"),
        "{error}"
    );
    stack.write(
        &addon,
        "interface/zz_mod_topbar.gui",
        "guiTypes = { containerWindowType = { name = topbar \
         buttonType = { name = player_flag quadTextureSprite = GFX_shield_medium } } }",
    );
    let style = ShieldStyle::load(&stack.catalogue(&[])).unwrap();
    assert_eq!(style.warnings.len(), 1);
    assert!(
        style.warnings[0].contains("topbar is declared 2 times"),
        "{:?}",
        style.warnings
    );
}

/// Idea timeline: the Category setting lists a mod's category with its own
/// name and slots, and a base category the mod removes is gone.
#[test]
fn idea_timeline_categories_come_from_the_mod_stack() {
    let stack = Stack::new("categories");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/idea_tags/00_idea.txt",
        "idea_categories = { economy = { slot = economy } trade = { slot = trade_laws } }",
    );
    // Same path: the mod's file replaces the base one and drops `trade`.
    stack.write(
        &addon,
        "common/idea_tags/00_idea.txt",
        "idea_categories = { economy = { slot = economy } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/mod_tags.txt",
        "idea_categories = { mod_council = { slot = mod_seat character_slot = mod_councillor } }",
    );
    stack.write(
        &addon,
        "common/ideas/mod_ideas.txt",
        "ideas = { mod_seat = { law = yes mod_open = { default = yes } mod_closed = { } } }",
    );
    stack.write(
        &addon,
        "localisation/english/mod_l_english.yml",
        "\u{feff}l_english:\n mod_council:0 \"Council\"\n mod_seat:0 \"Seat\"\n",
    );
    let politics = stack.politics(&[]);
    let localise = |key: &str, view| politics.localised(key, view);
    let listed = categories(&Joined::of(&politics, &localise));
    let tokens: Vec<_> = listed.iter().map(|c| c.token.as_str()).collect();
    assert_eq!(tokens, ["economy", "mod_council"]);
    let council = &listed[1];
    assert_eq!(council.name.as_deref(), Some("Council"));
    assert_eq!(
        council
            .slots
            .iter()
            .map(|slot| (slot.token.as_str(), slot.name.as_deref(), slot.character))
            .collect::<Vec<_>>(),
        [
            ("mod_seat", Some("Seat"), false),
            ("mod_councillor", Some("mod_councillor"), true)
        ]
    );
    assert_eq!(council.slots[0].laws, ["mod_open", "mod_closed"]);
}

/// Ruling party: a mod ideology charts with its own name and colour, a mod
/// may recolour a base ideology, and replacing the folder removes the base ones.
#[test]
fn ruling_party_ideologies_take_the_mod_s_names_and_colours() {
    let stack = Stack::new("ideologies");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/ideologies/00_ideologies.txt",
        "ideologies = { democratic = { color = { 0 0 255 } } fascism = { color = { 150 75 0 } } }",
    );
    stack.write(
        &base,
        "common/ideologies/01_neutrality.txt",
        "ideologies = { neutrality = { color = { 128 128 128 } } }",
    );
    // Same path: the mod's file replaces the base one, recolouring `democratic`
    // and removing `fascism`.
    stack.write(
        &addon,
        "common/ideologies/00_ideologies.txt",
        "ideologies = { democratic = { color = { 10 20 30 } } }",
    );
    stack.write(
        &addon,
        "common/ideologies/mod_ideologies.txt",
        "ideologies = { technocracy = { types = { futurism = { } } color = { 12 200 180 } } }",
    );
    stack.write(
        &addon,
        "localisation/english/mod_l_english.yml",
        "\u{feff}l_english:\n technocracy:0 \"Technocracy\"\n",
    );
    let shown = |politics: &PoliticsContent| {
        ideology_infos(politics, &mut Vec::new())
            .into_iter()
            .map(|info| (info.token, info.name, info.color))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        shown(&stack.politics(&[])),
        [
            (
                "democratic".to_owned(),
                Some("democratic".to_owned()),
                Some([10, 20, 30])
            ),
            (
                "neutrality".to_owned(),
                Some("neutrality".to_owned()),
                Some([128, 128, 128])
            ),
            (
                "technocracy".to_owned(),
                Some("Technocracy".to_owned()),
                Some([12, 200, 180])
            ),
        ]
    );
    // replace_path hides every base file, so only the mod's own ideologies remain.
    let replaced = shown(&stack.politics(&["common/ideologies"]));
    assert_eq!(
        replaced
            .iter()
            .map(|(t, _, _)| t.as_str())
            .collect::<Vec<_>>(),
        ["democratic", "technocracy"]
    );
}

/// National spirits and the idea timeline: a mod overrides a base idea's
/// sprite and name; the icon resolver's own-token sprite wins over `picture`,
/// as the engine's does; an advisor's portrait comes from a role beyond the
/// base game's usual three.
#[test]
fn idea_artwork_follows_the_mod_s_sprites_and_localisation() {
    let stack = Stack::new("artwork");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/ideas/spirits.txt",
        "ideas = { country = { base_spirit = { } pictured = { picture = shared_art } } }",
    );
    stack.write(
        &base,
        "interface/ideas.gfx",
        "spriteTypes = { \
           spriteType = { name = GFX_idea_base_spirit texturefile = gfx/interface/base.tga } \
           spriteType = { name = GFX_idea_shared_art texturefile = gfx/interface/shared.tga } }",
    );
    stack.write(&base, "gfx/interface/base.tga", tga_row(&[RED]));
    stack.write(&base, "gfx/interface/shared.tga", tga_row(&[GREEN]));
    stack.write(
        &base,
        "localisation/english/ideas_l_english.yml",
        "\u{feff}l_english:\n base_spirit:0 \"Base Spirit\"\n base_spirit_desc:0 \"Base text\"\n",
    );
    // The mod redeclares the sprite with its own texture, and gives `pictured`
    // an own-token sprite, which the resolver tries before the picture.
    stack.write(
        &addon,
        "interface/zz_mod_ideas.gfx",
        "spriteTypes = { \
           SpriteType = { name = GFX_idea_base_spirit TextureFile = gfx/interface/mod.tga } \
           spriteType = { name = GFX_idea_pictured texturefile = gfx/interface/own.tga } \
           spriteType = { name = GFX_mod_scholar texturefile = gfx/interface/scholar.tga } }",
    );
    stack.write(&addon, "gfx/interface/mod.tga", tga_row(&[BLUE]));
    stack.write(&addon, "gfx/interface/own.tga", tga_row(&[GOLD]));
    stack.write(&addon, "gfx/interface/scholar.tga", tga_row(&[GREEN]));
    stack.write(
        &addon,
        "localisation/english/replace/mod_l_english.yml",
        "\u{feff}l_english:\n base_spirit:0 \"Modded Spirit\"\n",
    );
    stack.write(
        &addon,
        "common/characters/mod_characters.txt",
        "characters = { MOD_scholar = { name = MOD_scholar \
           portraits = { scientist = { small = GFX_mod_scholar } } \
           advisor = { slot = mod_seat idea_token = MOD_scholar_idea } } }",
    );
    stack.write(
        &addon,
        "localisation/english/mod_l_english.yml",
        "\u{feff}l_english:\n MOD_scholar:0 \"Dr Scholar\"\n",
    );
    let politics = stack.politics(&[]);
    let (spirit, problem) = politics.idea_artwork("base_spirit", None, fixture_country());
    assert_eq!(problem, None);
    assert_eq!(spirit.name.as_deref(), Some("Modded Spirit"));
    assert_eq!(spirit.description.as_deref(), Some("Base text"));
    assert_eq!(spirit.icon, Some(pixel_png(BLUE)));
    let (pictured, _) = politics.idea_artwork("pictured", None, fixture_country());
    assert_eq!(pictured.icon, Some(pixel_png(GOLD)));
    let (advisor, _) = politics.idea_artwork("MOD_scholar_idea", None, fixture_country());
    assert_eq!(advisor.name.as_deref(), Some("Dr Scholar"));
    assert_eq!(advisor.icon, Some(pixel_png(GREEN)));
    // No artwork anywhere: no icon, and the token stays for the widget to show.
    let (unknown, problem) = politics.idea_artwork("MOD_unknown", None, fixture_country());
    assert!(problem.unwrap().contains("MOD_unknown: no artwork"));
    assert_eq!(
        (unknown.token.as_str(), unknown.icon),
        ("MOD_unknown", None)
    );
}

/// National spirits and the idea timeline: a mod redeclares a base idea's
/// sprite in a form this reader refuses. The idea shows no art (never the base
/// art the mod replaced), says why in its artwork problem; unrelated Politics
/// replies stay quiet.
#[test]
fn a_refused_sprite_override_shows_no_art_and_says_why() {
    let stack = Stack::new("refused_sprite");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/ideas/spirits.txt",
        "ideas = { country = { base_spirit = { picture = shared } } }",
    );
    stack.write(
        &base,
        "interface/ideas.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_base_spirit texturefile = gfx/interface/base.tga } \
         spriteType = { name = GFX_idea_shared texturefile = gfx/interface/shared.tga } }",
    );
    stack.write(&base, "gfx/interface/base.tga", tga_row(&[RED]));
    stack.write(&base, "gfx/interface/shared.tga", tga_row(&[GREEN]));
    stack.write(
        &addon,
        "interface/zz_mod.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_base_spirit \
         texturefile = gfx/interface/mod.tga noOfFrames = -2 } }",
    );
    let politics = stack.politics(&[]);
    let (art, problem) = politics.idea_artwork("base_spirit", None, fixture_country());
    // Neither the base art the mod overrode nor the picture's sprite after it.
    assert_eq!(art.icon, None);
    let problem = problem.expect("the missing art says why");
    assert!(
        problem.contains("GFX_idea_base_spirit") && problem.contains("zz_mod.gfx"),
        "{problem}"
    );
    assert!(politics.warnings.is_empty(), "{:?}", politics.warnings);
}

/// Country selection: tags are opaque lookup keys, dynamic and modded ones
/// included; only emptiness, NUL and the length bound are refused.
#[test]
fn unusual_and_dynamic_tags_are_valid_country_keys() {
    for tag in ["D01", "ger", "my tag", "MOD_TAG", "Z99", "ÉTAT", "A-B.C"] {
        assert_eq!(validate_tag(tag), Ok(()), "{tag}");
    }
    for tag in ["", "a\0b", &"x".repeat(129)] {
        assert!(validate_tag(tag).is_err(), "{tag:?}");
    }
}

/// An ambiguous first portrait cannot fall through to a lower role or token art.
#[test]
fn an_ambiguous_first_portrait_stops_the_native_role_search() {
    let stack = Stack::new("ambiguous_portrait");
    stack.write(
        &stack.addon(),
        "common/characters/mod.txt",
        "characters = { MOD_split = {
         instance = { allowed = { always = yes }
          portraits = { civilian = { small = GFX_a } scientist = { small = GFX_scholar } }
          advisor = { slot = mod_seat idea_token = MOD_split_idea } }
         instance = { allowed = { always = no }
          portraits = { civilian = { small = GFX_b } scientist = { small = GFX_scholar } }
          advisor = { slot = mod_seat idea_token = MOD_split_idea } } } }",
    );
    stack.write(
        &stack.addon(),
        "interface/mod.gfx",
        "spriteTypes = {
         spriteType = { name = GFX_scholar texturefile = gfx/scholar.tga }
         spriteType = { name = GFX_idea_MOD_split_idea texturefile = gfx/scholar.tga } }",
    );
    stack.write(&stack.addon(), "gfx/scholar.tga", tga_row(&[GREEN]));
    let politics = stack.politics(&[]);
    let (art, problem) = politics.idea_artwork("MOD_split_idea", None, fixture_country());
    assert_eq!(art.icon, None);
    assert!(problem.unwrap().contains("first small portrait (civilian)"));
    assert!(politics.warnings.is_empty(), "{:?}", politics.warnings);
    assert!(
        politics
            .catalogue
            .refusal_lines()
            .iter()
            .any(|w| w.contains("MOD_split")),
        "{:?}",
        politics.catalogue.refusal_lines()
    );
}

#[test]
fn a_tag_specific_idea_sprite_keeps_the_holder_tag_opaque() {
    let stack = Stack::new("tag_artwork");
    stack.write(
        &stack.addon(),
        "common/ideas/mod.txt",
        "ideas = { mod_group = { mod_idea = { picture = mod_picture } } }",
    );
    stack.write(
        &stack.addon(),
        "interface/mod.gfx",
        "spriteTypes = { spriteType = { name = \"GFX_idea_mod_picture_my tag_ÉTAT\" texturefile = gfx/mod.tga } }",
    );
    stack.write(&stack.addon(), "gfx/mod.tga", tga_row(&[BLUE]));
    let politics = stack.politics(&[]);
    let country = IdeaCountry {
        tag: "my tag_ÉTAT",
        ..fixture_country()
    };
    let (art, problem) = politics.idea_artwork("mod_idea", None, country);
    assert_eq!(art.icon, Some(pixel_png(BLUE)));
    assert_eq!(problem, None);
    assert!(
        politics
            .idea_artwork("mod_idea", None, fixture_country())
            .0
            .icon
            .is_none()
    );
}

#[test]
fn an_ideas_own_art_does_not_evaluate_an_ambiguous_advisor_portrait() {
    let stack = Stack::new("lazy_portrait");
    stack.write(
        &stack.addon(),
        "common/ideas/mod.txt",
        "ideas = { mod_group = { mod_idea = { } } }",
    );
    stack.write(
        &stack.addon(),
        "common/characters/mod.txt",
        "characters = { mod_character = { \
         instance = { allowed = { always = yes } portraits = { civilian = { small = GFX_a } } } \
         instance = { allowed = { always = no } portraits = { civilian = { small = GFX_b } } } } }",
    );
    stack.write(
        &stack.addon(),
        "interface/mod.gfx",
        "spriteTypes = { spriteType = { name = GFX_idea_mod_idea texturefile = gfx/mod.tga } }",
    );
    stack.write(&stack.addon(), "gfx/mod.tga", tga_row(&[BLUE]));
    let politics = stack.politics(&[]);
    let (art, problem) =
        politics.idea_artwork("mod_idea", Some("mod_character"), fixture_country());
    assert_eq!(art.icon, Some(pixel_png(BLUE)));
    assert_eq!(problem, None);
    let refusals = politics.catalogue.refusal_lines();
    assert!(
        refusals
            .iter()
            .all(|refusal| !refusal.contains("first small portrait")),
        "{refusals:?}"
    );
    // The same portrait is refused when an advisor's artwork actually needs it.
    let (_, problem) =
        politics.idea_artwork("mod_advisor_idea", Some("mod_character"), fixture_country());
    assert!(problem.unwrap().contains("first small portrait"));
}

#[test]
fn a_resource_icon_is_the_trade_row_s_button_whatever_file_declares_it() {
    for (kind, drawn) in [("buttonType", true), ("iconType", false)] {
        let stack = Stack::new(kind);
        stack.write(
            &stack.addon(),
            "common/resources/mod.txt",
            "resources = { mod_ore = { icon_frame = 1 } }",
        );
        stack.write(
            &stack.addon(),
            "interface/mod.gui",
            format!(
                "guiTypes = {{ containerWindowType = {{ name = resources_info_entry {kind} = {{ name = resources_icon spriteType = GFX_mod_strip }} }} }}"
            ),
        );
        stack.write(
            &stack.addon(),
            "interface/mod.gfx",
            "spriteTypes = { spriteType = { name = GFX_mod_strip texturefile = gfx/mod.tga } }",
        );
        stack.write(&stack.addon(), "gfx/mod.tga", tga_row(&[BLUE]));
        let catalogue = stack.catalogue(&[]);
        let (mut declared, mut warnings) = (BTreeMap::new(), Vec::new());
        resource_content(
            &ContentView::of(&catalogue),
            &[("mod_ore".to_owned(), 1)],
            &mut declared,
            &mut warnings,
            &AtomicBool::new(false),
        )
        .unwrap();
        let icon = &declared[&("mod_ore".to_owned(), 1)].icon;
        if drawn {
            assert_eq!(icon, &Some(pixel_png(BLUE)));
            assert!(warnings.is_empty(), "{warnings:?}");
        } else {
            // The trade view asks FindButton; an icon of the name is not its control.
            assert_eq!(icon, &None);
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.contains("has no resources_icon button")),
                "{warnings:?}"
            );
        }
    }
}
