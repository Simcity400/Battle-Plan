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

fn inventory_finding(
    kind: crate::content_check::Kind,
    message: &str,
    examples: &[&str],
) -> crate::content_check::Finding {
    let mut finding = crate::content_check::Finding::new(
        kind,
        if kind == crate::content_check::Kind::Offline {
            "offline coverage"
        } else {
            "product coverage"
        },
        message,
    );
    finding.count = Some(examples.len());
    finding.examples = examples
        .iter()
        .map(|example| (*example).to_owned())
        .collect();
    finding
}

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
        let bytes = bytes.as_ref();
        let bytes = if path.extension().is_some_and(|extension| extension == "yml")
            && !bytes.starts_with(b"\xEF\xBB\xBF")
        {
            [b"\xEF\xBB\xBF".as_slice(), bytes].concat()
        } else {
            bytes.to_vec()
        };
        std::fs::write(path, bytes).unwrap();
    }

    fn catalogue(&self, replace_paths: &[&str]) -> Arc<Catalogue> {
        Arc::new(Catalogue::of_layers(self.layers(replace_paths), "english").unwrap())
    }

    fn check_catalogue(&self, replace_paths: &[&str]) -> Arc<Catalogue> {
        Arc::new(Catalogue::of_layers_for_check(self.layers(replace_paths), "english").unwrap())
    }

    fn layers(&self, replace_paths: &[&str]) -> ResolvedLayers {
        let (base, addon) = (self.base(), self.addon());
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&addon).unwrap();
        ResolvedLayers::from_folders(&[(&base, &[]), (&addon, replace_paths)]).unwrap()
    }

    fn politics(&self, replace_paths: &[&str]) -> PoliticsContent {
        self.politics_defines();
        PoliticsContent::load(self.catalogue(replace_paths)).unwrap()
    }

    fn check_politics(&self, replace_paths: &[&str]) -> PoliticsContent {
        self.politics_defines();
        PoliticsContent::load(self.check_catalogue(replace_paths)).unwrap()
    }

    fn politics_defines(&self) {
        self.write(
            &self.base(),
            "common/defines/00_fixture.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, \
             NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_officers\" } } }",
        );
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

fn content_map(stack: &Stack, layer: &Path) {
    stack.write(
        layer,
        "map/default.map",
        "definitions = definition.csv provinces = provinces.bmp terrain = terrain.bmp rivers = rivers.bmp heightmap = heightmap.bmp tree_definition = trees.bmp continent = continents.txt",
    );
    stack.write(
        layer,
        "map/definition.csv",
        "0;0;0;0;land;false;fixture;1\n",
    );
    stack.write(
        layer,
        "map/continents.txt",
        "continents = { fixture_continent }",
    );
}

fn fixture_country() -> IdeaCountry<'static> {
    IdeaCountry {
        tag: "MOD",
        slot: 0,
        look: Some(CountryLook {
            name_tag: "",
            original_or_own_tag: Ok("MOD"),
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
        find_flag(
            layers,
            &sets,
            &[[1, 1]; 3],
            &candidates,
            &mut Default::default(),
        )
        .unwrap()
        .0
    };
    match search(&["variants/red", "AAA"]) {
        FlagSearch::Found { asset, image } => {
            assert_eq!(asset, "gfx/flags/variants/red.tga");
            assert_eq!(image.pixels, BLUE);
        }
        other => panic!("{other:?}"),
    }
    // A missing preferred file still falls through, as the engine does, and a
    // key past 256 bytes inside the flag folder is accepted like any other.
    let long = format!("variants/{}/red", vec!["a".repeat(60); 5].join("/"));
    assert!(long.len() > 256);
    for key in ["variants/blue", long.as_str()] {
        match search(&[key, "AAA"]) {
            FlagSearch::Found { asset, .. } => assert_eq!(asset, "gfx/flags/AAA.tga"),
            other => panic!("{key}: {other:?}"),
        }
    }
    for key in ["variants\\red", "../AAA", "variants/../AAA", "/AAA"] {
        match search(&[key, "AAA"]) {
            FlagSearch::Unresolvable => {}
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
/// included; only emptiness and NUL are refused.
#[test]
fn unusual_and_dynamic_tags_are_valid_country_keys() {
    for tag in [
        "D01",
        "ger",
        "my tag",
        "MOD_TAG",
        "Z99",
        "ÉTAT",
        "A-B.C",
        &"x".repeat(129),
    ] {
        assert_eq!(validate_tag(tag), Ok(()), "{tag}");
    }
    for tag in ["", "a\0b"] {
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
            "localisation/mod_l_english.yml",
            "l_english:\n PRODUCTION_MATERIALS_MOD_ORE:0 \"Mod ore\"\n",
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

/// The offline corpus uses each product's own preparation over the merged stack.
#[test]
fn content_check_exercises_every_product_and_names_exact_mod_fallbacks() {
    use crate::{
        content_check::{Finding, Kind, check},
        products::{self, Product},
    };
    let stack = Stack::new("product_corpus");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "common/terrain/fixture.txt",
        "categories = { fixture = {} }",
    );
    stack.write(
        &base,
        "common/resources/base.txt",
        "resources = { removed_resource = { icon_frame = 1 } }",
    );
    stack.write(
        &addon,
        "common/resources/mod.txt",
        "resources = { mod_crystal = { } }",
    );
    stack.write(
        &base,
        "common/national_focus/base.txt",
        "focus_tree = { focus = { id = removed_focus icon = GFX_removed } }",
    );
    stack.write(
        &addon,
        "common/national_focus/mod.txt",
        "focus_tree = { focus = { id = mod_focus icon = GFX_mod_focus } }",
    );
    stack.write(
        &base,
        "common/technologies/base.txt",
        "technologies = { removed_tech = {} }",
    );
    stack.write(
        &addon,
        "common/technologies/mod.txt",
        "technologies = { mod_tech = {} }",
    );
    stack.write(
        &base,
        "common/ideas/base.txt",
        "ideas = { spirits = { removed_idea = { } } }",
    );
    stack.write(
        &addon,
        "common/ideas/mod.txt",
        "ideas = { spirits = { mod_idea = { picture = undeclared } } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/mod.txt",
        "idea_categories = { spirits = { type = national_spirit } }",
    );
    stack.write(
        &addon,
        "common/country_tags/mod.txt",
        "\"mod_tag\" = \"countries/mod.txt\"",
    );
    stack.write(
        &addon,
        "common/ideologies/mod.txt",
        "ideologies = { mod_group = { color = { 0.2 0.4 0.6 } types = { mod_type = { } } } }",
    );
    stack.write(&addon, "common/defines/mod.lua", concat!(
        "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 }, NGame = { COMBAT_LOG_MAX_MONTHS = 6 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} }, ",
        "NMilitary = { SUPPLY_GRACE = 10, LOW_SUPPLY = .6 }, NOperatives = { CRYPTO_BASE_CRYPTO_LEVEL = 12.5, CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE = 3.25 }, ",
        "NNavy = { BATTLE_NAME_VP_FACTOR = 100, BATTLE_NAME_VP_CUTOFF = 1, NAVAL_COMBAT_RESULT_TIMEOUT_YEARS = 1, CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS = 1 }, ",
        "NFocus = { FOCUS_POINT_DAYS = 7 }, NGraphics = { SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD = .15, SUPPLY_UNIT_COUNTER_LOW_THRESHOLD = .4, ",
        "COUNTRY_FLAG_TEX_WIDTH = 1, COUNTRY_FLAG_TEX_HEIGHT = 1, COUNTRY_FLAG_TEX_MAX_SIZE = 8, ",
        "COUNTRY_FLAG_MEDIUM_TEX_WIDTH = 1, COUNTRY_FLAG_MEDIUM_TEX_HEIGHT = 1, COUNTRY_FLAG_MEDIUM_TEX_MAX_SIZE = 8, ",
        "COUNTRY_FLAG_SMALL_TEX_WIDTH = 1, COUNTRY_FLAG_SMALL_TEX_HEIGHT = 1, COUNTRY_FLAG_SMALL_TEX_MAX_SIZE = 8 } }")
    );
    content_map(&stack, &addon);
    let mut province_bitmap = vec![0_u8; 58];
    province_bitmap[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1), (34, 4)] {
        province_bitmap[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    province_bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    province_bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    stack.write(&addon, "map/provinces.bmp", province_bitmap);
    stack.write(&addon, "common/buildings/mod.txt", "buildings = { mod_works = { military_production = 1 naval_production = 1 general_production = 1 icon_frame = 1 } }");
    stack.write(
        &addon,
        "common/units/equipment/mod.txt",
        "equipments = { mod_archetype = { is_archetype = yes } }",
    );
    stack.write(
        &addon,
        "common/autonomous_states/mod.txt",
        "autonomy_state = { id = mod_level min_freedom_level = 0.25 }",
    );
    stack.write(
        &addon,
        "history/states/mod.txt",
        "state = { id = 900 name = MOD_STATE }",
    );
    stack.write(
        &addon,
        "map/strategicregions/mod.txt",
        "strategic_region = { id = 920 name = MOD_REGION provinces = {} }",
    );
    stack.write(
        &addon,
        "common/special_projects/specialization/mod.txt",
        "mod_specialization = {} ",
    );
    stack.write(
        &addon,
        "common/special_projects/projects/mod.txt",
        "mod_project = { specialization = mod_specialization icon = GFX_mod_project }",
    );
    stack.write(
        &addon,
        "common/raids/mod.txt",
        "types = { mod_sortie = { custom_map_icon = GFX_mod_sortie } }",
    );
    stack.write(
        &addon,
        "common/operations/mod.txt",
        "mod_covert = { name = MOD_COVERT icon = GFX_mod_covert }",
    );
    stack.write(&addon, "gfx/flags/mod_tag.tga", tga_row(&[RED]));
    stack.write(&addon, "gfx/interface/mod.tga", tga_row(&[GREEN]));
    stack.write(&addon, "interface/mod.gui", concat!(
        "guiTypes = { containerWindowType = { name = topbar buttonType = { name = player_flag quadTextureSprite = GFX_mod_shield } } ",
        "containerWindowType = { name = resources_info_entry buttonType = { name = resources_icon spriteType = GFX_mod_strip } } ",
        "containerWindowType = { name = production_building_line_entry iconType = { name = picture spriteType = GFX_mod_strip } iconType = { name = repair spriteType = GFX_mod_strip } } }")
    );
    stack.write(&addon, "interface/mod.gfx", concat!(
        "spriteTypes = { spriteType = { name = GFX_mod_focus texturefile = gfx/interface/mod.tga } ",
        "spriteType = { name = GFX_mod_tech_medium texturefile = gfx/interface/mod.tga } ",
        "spriteType = { name = GFX_mod_strip texturefile = gfx/interface/mod.tga } ",
        "spriteType = { name = GFX_mod_project texturefile = gfx/interface/mod.tga } ",
        "spriteType = { name = GFX_mod_sortie texturefile = gfx/interface/mod.tga } ",
        "spriteType = { name = GFX_mod_covert texturefile = gfx/interface/mod.tga } ",
        "spriteType = { name = GFX_mod_archetype_medium texturefile = gfx/interface/mod.tga } ",
        "maskedShieldType = { name = GFX_mod_shield textureFile2 = gfx/interface/mod.tga } }")
    );
    let raid_labels = std::iter::once("raid_type_mod_sortie")
        .chain(engine::craid_instance::PHASE_KEYS)
        .chain(engine::craid_instance::OUTCOME_KEYS)
        .map(|key| format!(" {key}:0 \"Modded {key}\"\n"))
        .collect::<String>();
    stack.write(
        &addon,
        "localisation/raids_l_english.yml",
        format!("l_english:\n{raid_labels}"),
    );
    stack.write(&addon, "localisation/mod_l_english.yml", concat!(
        "l_english:\n mod_focus_desc:0 \"Mod focus description\"\n mod_idea:0 \"Mod idea\"\n mod_idea_desc:0 \"Mod idea description\"\n",
        " mod_tech:0 \"Mod technology\"\n mod_tech_desc:0 \"Mod technology description\"\n",
        " spirits:0 \"Mod spirits\"\n mod_group:0 \"Mod government\"\n mod_type:0 \"Mod type\"\n",
        " PRODUCTION_MATERIALS_MOD_CRYSTAL:0 \"Mod crystal\"\n YES:0 \"Yes\"\n NO:0 \"No\"\n",
        " mod_tag:0 \"Mod country\"\n mod_works:0 \"Mod works\"\n mod_archetype:0 \"Mod equipment\"\n",
        " mod_level:0 \"Mod level\"\n MOD_STATE:0 \"Mod state\"\n MOD_REGION:0 \"\"\n mod_specialization:0 \"Mod specialization\"\n mod_project:0 \"Mod project\"\n")
    );
    stack.write(
        &addon,
        "localisation/naval_l_english.yml",
        "l_english:\n NAVAL_BATTLE_TITLE:0 \"Engagement $NAME$\"\n",
    );
    stack.write(&addon, "localisation/intelligence_l_english.yml",
        "l_english:\n MOD_COVERT:0 \"Mod covert operation\"\n AGENCY_NAME:0 \"Mod agency\"\n AGENCY_DEFENSE_LEVEL:0 \"$LEVEL|.1$ mod defense\"\n");
    let replace_paths = [
        "common/resources",
        "common/national_focus",
        "common/ideas",
        "common/technologies",
    ];
    let catalogue = stack.catalogue(&replace_paths);
    let checks = check(Arc::clone(&catalogue.content), "english");
    assert_eq!(
        checks
            .iter()
            .filter(|check| !crate::content_check::SHARED_CHECK_KEYS.contains(&check.key))
            .map(|check| check.key)
            .collect::<Vec<_>>(),
        products::PRODUCTS
            .iter()
            .map(|product| product.key)
            .collect::<Vec<_>>()
    );
    assert!(
        checks.iter().all(|check| check.error.is_none()),
        "{checks:#?}"
    );
    let nuclear_note = &checks
        .iter()
        .find(|check| check.key == products::country_nukes::CountryNukes::KEY)
        .unwrap()
        .note;
    for dependency in [
        "LexicalTokens",
        "Nukes",
        "Countries",
        "CountryLinks",
        "Campaign",
    ] {
        assert!(nuclear_note.contains(dependency), "{nuclear_note}");
    }
    let findings = |key| {
        checks
            .iter()
            .find(|check| check.key == key)
            .unwrap()
            .findings
            .clone()
    };
    assert_eq!(
        findings(products::place_names::PlaceNames::KEY),
        vec![Finding::new(
            Kind::Refused,
            "strategic region 920",
            "native region display name is empty",
        )]
    );
    for key in [
        products::land_battles::LandBattles::KEY,
        products::land_battles::LandBattlesManifest::KEY,
        products::land_battles::LandBattlesRanking::KEY,
    ] {
        let checked = checks.iter().find(|check| check.key == key).unwrap();
        assert!(checked.findings.is_empty(), "{checked:?}");
        assert!(checked.note.contains("No content dependency"));
    }
    assert_eq!(
        findings(products::national_focus::NationalFocus::KEY),
        vec![Finding::new(
            Kind::Content,
            "focus mod_focus",
            "localisation mod_focus is missing; the key is shown"
        )]
    );
    assert_eq!(
        findings(products::resource_balance::ResourceBalance::KEY),
        vec![Finding::new(
            Kind::Fallback,
            "resource mod_crystal declaration 1",
            "mod_crystal: the recorded content gives no usable icon frame."
        )]
    );
    assert_eq!(
        findings(products::raids::Raids::KEY),
        vec![inventory_finding(
            Kind::Offline,
            "raid target names and building-specific icon candidates need recorded target scope; unavailable offline",
            &["raid type mod_sortie"]
        )]
    );
    assert!(findings(products::espionage_log::EspionageLog::KEY).is_empty());
    assert_eq!(
        findings(products::agency_comparison::AgencyComparison::KEY),
        vec![inventory_finding(
            Kind::Offline,
            "agency defense numeric text needs the recorded IntelligenceAgencies value; unavailable offline",
            &["AGENCY_DEFENSE_LEVEL"]
        )]
    );
    assert_eq!(
        findings(products::idea_history::IdeaHistory::KEY),
        vec![
            inventory_finding(
                Kind::Offline,
                "idea icon needs the recorded holder tag after its static candidates; unavailable offline",
                &["idea mod_idea"]
            ),
            inventory_finding(
                Kind::Offline,
                "idea name can use recorded country localisation and civil-war appearance; unavailable offline",
                &["idea mod_idea"]
            ),
        ]
    );
    let mut medium = Finding::new(
        Kind::Fallback,
        "flag \"mod_tag\" group Some(\"mod_group\") Medium",
        "medium flag is missing or undecodable; normal flag gfx/flags/mod_tag.tga is shown",
    );
    medium.file = Some("gfx/flags/mod_tag.tga".into());
    assert_eq!(
        findings(products::country_flags::CountryFlags::KEY),
        vec![medium]
    );
    assert_eq!(
        findings(products::party_history::PartyHistory::KEY),
        vec![inventory_finding(
            Kind::Content,
            "party name keys are missing; the ideology group name is shown (14118F860 / 14118EAD0)",
            &[
                "party \"mod_tag\" / mod_group: party name keys for \"mod_tag\" / mod_group are missing; the ideology group name is shown"
            ]
        )]
    );
    for key in [
        products::research::Research::KEY,
        products::country_groups::CountryGroups::KEY,
        products::country_measure::CountryMeasure::KEY,
        products::country_fuel::CountryFuel::KEY,
        products::decryption::Decryption::KEY,
    ] {
        assert!(findings(key).is_empty(), "{key}: {:?}", findings(key));
    }
    for key in [
        products::country_stability::CountryStability::KEY,
        products::stability_parts::StabilityParts::KEY,
    ] {
        assert_eq!(
            findings(key),
            vec![inventory_finding(
                Kind::Offline,
                "idea name can use recorded country localisation and civil-war appearance; unavailable offline",
                &["idea mod_idea"]
            )]
        );
    }
    assert!(
        checks
            .iter()
            .flat_map(|check| &check.findings)
            .all(|finding| !finding.subject.contains("removed_"))
    );
    assert_eq!(
        products::decryption::content_defines(&catalogue.content.layers).unwrap(),
        crate::cryptology::Defines {
            base: 1_250_000,
            upgrade: 325_000,
        }
    );
    // The merged inventory has the mod's group and its authored colour.
    let politics = PoliticsContent::load(Arc::clone(&catalogue)).unwrap();
    let infos = ideology_infos(&politics, &mut Vec::new());
    assert_eq!(
        (infos[0].token.as_str(), infos[0].color),
        ("mod_group", Some([51, 102, 153]))
    );
    let intelligence =
        products::intelligence_content::IntelligenceContent::load(Arc::clone(&catalogue)).unwrap();
    assert_eq!(
        intelligence.default_agency_name().as_deref(),
        Some("Mod agency")
    );
    assert_eq!(intelligence.defense_label().as_deref(), Some("mod defense"));
    let (operation_art, reason) = intelligence.operation_artwork("mod_covert");
    assert_eq!(operation_art.name.as_deref(), Some("Mod covert operation"));
    assert!(operation_art.icon.is_some(), "{reason:?}");
    stack.write(
        &addon,
        "interface/raid_refusal.gfx",
        "spriteTypes = { spriteType = { name = GFX_broken_sortie texturefile = gfx/interface/missing_sortie.tga } }",
    );
    stack.write(
        &addon,
        "common/raids/mod.txt",
        "types = { mod_sortie = { custom_map_icon = GFX_broken_sortie } mod_unnamed_sortie = {} }",
    );
    stack.write(&addon, "common/operations/mod.txt",
        "mod_covert = { name = MOD_COVERT icon = GFX_broken_sortie } mod_unnamed_covert = { icon = GFX_mod_covert }");
    let changed_raids = stack.catalogue(&replace_paths);
    let changed_checks = check(Arc::clone(&changed_raids.content), "english");
    let raid_findings = &changed_checks
        .iter()
        .find(|check| check.key == products::raids::Raids::KEY)
        .unwrap()
        .findings;
    let operation_findings = &changed_checks
        .iter()
        .find(|check| check.key == products::espionage_log::EspionageLog::KEY)
        .unwrap()
        .findings;
    assert_eq!(
        operation_findings
            .iter()
            .filter(|finding| finding.kind == Kind::Refused
                && finding.subject == "GFX_broken_sortie"
                && finding.message.contains("gfx/interface/missing_sortie.tga"))
            .count(),
        1,
        "{operation_findings:#?}"
    );
    assert!(
        !operation_findings
            .iter()
            .any(|finding| finding.kind == Kind::Fallback
                && finding.subject == "operation mod_covert")
    );
    // LocalizeOperationName@140A6DF00 preserves the definition's empty name.
    let changed_intelligence =
        products::intelligence_content::IntelligenceContent::load(Arc::clone(&changed_raids))
            .unwrap();
    assert_eq!(
        changed_intelligence
            .operation_artwork("mod_unnamed_covert")
            .0
            .name
            .as_deref(),
        Some("")
    );
    assert!(
        !operation_findings
            .iter()
            .any(|finding| finding.kind == Kind::Fallback
                && finding.subject == "operation mod_unnamed_covert"),
        "{operation_findings:#?}"
    );
    assert!(
        raid_findings
            .iter()
            .any(|finding| finding.kind == Kind::Fallback
                && finding.subject == "GFX_broken_sortie"
                && finding.message.contains("gfx/interface/missing_sortie.tga")),
        "{raid_findings:#?}"
    );
    assert!(
        raid_findings
            .iter()
            .any(|finding| finding.kind == Kind::Fallback
                && finding.subject == "raid type mod_unnamed_sortie"
                && finding.message.contains("raid_type_mod_unnamed_sortie")),
        "{raid_findings:#?}"
    );
    stack.write(
        &addon,
        "common/defines/mod.lua",
        "NDefines = { NOperatives = { CRYPTO_BASE_CRYPTO_LEVEL = 12.5 } }",
    );
    let modified = stack.catalogue(&replace_paths);
    let refused = check(Arc::clone(&modified.content), "english");
    assert_eq!(
        refused
            .iter()
            .find(|check| check.key == products::decryption::Decryption::KEY)
            .unwrap()
            .error
            .as_deref(),
        Some("NDefines.NOperatives.CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE is not a number")
    );
}

#[test]
fn flag_search_reports_rejected_files_and_reuses_each_decode() {
    use crate::{
        content_check::{Finding, Kind},
        products::country_flags::FlagCache,
    };
    use installed_content::flags::FlagSet;
    let stack = Stack::new("flag_decode_cache");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(&base, "gfx/flags/plain.tga", tga_row(&[RED]));
    let mut rejected = tga_row(&[GREEN]);
    rejected[2] = 10;
    stack.write(&addon, "gfx/flags/medium/plain_mod.tga", rejected);
    let catalogue = stack.catalogue(&[]);
    let candidates = vec!["plain_mod".to_owned(), "plain".to_owned()];
    let sets = [FlagSet::Medium, FlagSet::Normal];
    let mut cache = FlagCache::default();
    let (first, findings) = find_flag(
        &catalogue.content.layers,
        &sets,
        &[[1, 1]; 3],
        &candidates,
        &mut cache,
    )
    .unwrap();
    let mut refused = Finding::new(
        Kind::Content,
        "plain_mod",
        "flag decoder refused gfx/flags/medium/plain_mod.tga at cell 1x1; this file was skipped",
    );
    refused.file = Some("gfx/flags/medium/plain_mod.tga".into());
    let mut fallback = Finding::new(
        Kind::Fallback,
        "plain",
        "medium flag is missing or undecodable; normal flag gfx/flags/plain.tga is shown",
    );
    fallback.file = Some("gfx/flags/plain.tga".into());
    assert_eq!(findings, vec![refused, fallback]);
    // A second ladder uses the prepared result, even if the uncovered file changes.
    stack.write(&addon, "gfx/flags/medium/plain_mod.tga", tga_row(&[GREEN]));
    let (found, repeated) = find_flag(
        &catalogue.content.layers,
        &sets,
        &[[1, 1]; 3],
        &candidates,
        &mut cache,
    )
    .unwrap();
    assert!(repeated.is_empty(), "{repeated:?}");
    assert!(matches!(&found, FlagSearch::Found { asset, .. } if asset == "gfx/flags/plain.tga"));
    match (&first, &found) {
        (
            FlagSearch::Found { image: first, .. },
            FlagSearch::Found {
                image: repeated, ..
            },
        ) => assert!(Arc::ptr_eq(first, repeated)),
        _ => panic!("the shared normal flag must be found"),
    }
    let (found, findings) = find_flag(
        &catalogue.content.layers,
        &[FlagSet::Normal],
        &[[1, 1]; 3],
        &["../plain".into(), "plain".into()],
        &mut cache,
    )
    .unwrap();
    assert!(matches!(found, FlagSearch::Unresolvable));
    assert_eq!(
        findings,
        vec![Finding::new(
            Kind::Refused,
            "../plain",
            "flag name \"../plain\" cannot be resolved inside the flag folder, so the country's flag is unavailable"
        )]
    );
}

#[test]
fn flag_cache_keeps_sets_separate_for_opaque_subfolder_keys() {
    use crate::{
        content_check::{Finding, Kind},
        products::country_flags::FlagCache,
    };
    use installed_content::flags::FlagSet;
    let stack = Stack::new("flag_set_cache");
    stack.write(
        &stack.addon(),
        "gfx/flags/medium/tag with spaces.tga",
        tga_row(&[BLUE]),
    );
    let catalogue = stack.catalogue(&[]);
    let cells = [[1, 1], [2, 1], [1, 1]];
    let mut cache = FlagCache::default();
    let (normal, findings) = find_flag(
        &catalogue.content.layers,
        &[FlagSet::Normal],
        &cells,
        &["medium/tag with spaces".into()],
        &mut cache,
    )
    .unwrap();
    assert!(matches!(normal, FlagSearch::Found { .. }));
    assert!(findings.is_empty());
    let (medium, findings) = find_flag(
        &catalogue.content.layers,
        &[FlagSet::Medium],
        &cells,
        &["tag with spaces".into()],
        &mut cache,
    )
    .unwrap();
    assert!(matches!(medium, FlagSearch::Absent));
    let mut refused = Finding::new(
        Kind::Content,
        "tag with spaces",
        "flag decoder refused gfx/flags/medium/tag with spaces.tga at cell 2x1; this file was skipped",
    );
    refused.file = Some("gfx/flags/medium/tag with spaces.tga".into());
    assert_eq!(
        findings,
        vec![
            refused,
            Finding::new(
                Kind::Fallback,
                "tag with spaces",
                "no decodable flag in the requested sets; flag artwork is unavailable"
            )
        ]
    );
}

#[test]
fn offline_ideas_report_traits_scripted_names_and_recorded_name_operands() {
    use crate::{
        content_check::{Finding, Kind, check},
        products::{self, Product},
    };
    let stack = Stack::new("offline_idea_context");
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/defines/mod.lua",
        "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/mod.txt",
        "idea_categories = { spirits = { type = national_spirit } generated = { } }",
    );
    stack.write(&addon, "common/ideas/mod.txt", "ideas = { spirits = { scoped = { traits = { missing_trait } } } generated = { generated_idea = { } } }");
    stack.write(&addon, "interface/mod.gfx", "spriteTypes = { spriteType = { name = GFX_idea_scoped texturefile = gfx/mod.tga } spriteType = { name = GFX_idea_generated_idea texturefile = gfx/mod.tga } }");
    stack.write(&addon, "gfx/mod.tga", tga_row(&[GREEN]));
    stack.write(&addon, "localisation/mod_l_english.yml", "l_english:\n spirits:0 \"Spirits\"\n generated:0 \"Advisors\"\n scoped:0 \"[ROOT.GetName]\"\n YES:0 \"Yes\"\n NO:0 \"No\"\n");
    let catalogue = stack.catalogue(&[]);
    let checks = check(Arc::clone(&catalogue.content), "english");
    let ideas = checks
        .iter()
        .find(|check| check.key == products::idea_history::IdeaHistory::KEY)
        .unwrap();
    assert_eq!(ideas.error, None);
    let generated = "generated advisor name needs the recorded country tag and canonical slot; unavailable offline";
    let scripted_example = "idea scoped: localisation scoped holds the scripted expression \"[ROOT.GetName]\", which the game evaluates for the country the text is shown for, and this product does not pass that country";
    assert_eq!(
        ideas.findings,
        vec![
            Finding::new(
                Kind::Fallback,
                "idea scoped trait missing_trait",
                "trait definition is missing; its name and effects are unavailable"
            ),
            inventory_finding(
                Kind::Unsupported,
                "scripted localisation is not evaluated because this product does not pass a country",
                &[scripted_example]
            ),
            inventory_finding(Kind::Offline, generated, &["idea generated_idea"]),
            inventory_finding(
                Kind::Offline,
                "idea name can use recorded country localisation and civil-war appearance; unavailable offline",
                &["idea generated_idea", "idea scoped"]
            ),
        ]
    );
    for key in [
        products::country_stability::CountryStability::KEY,
        products::stability_parts::StabilityParts::KEY,
    ] {
        let scripted_example = "idea scoped: localisation scoped holds the scripted expression \"[ROOT.GetName]\", which the game evaluates for the country the text is shown for, and recorded country context is unavailable offline";
        assert_eq!(
            checks
                .iter()
                .find(|check| check.key == key)
                .unwrap()
                .findings,
            vec![
                inventory_finding(Kind::Offline, generated, &["idea generated_idea"]),
                inventory_finding(
                    Kind::Offline,
                    "idea name can use recorded country localisation and civil-war appearance; unavailable offline",
                    &["idea generated_idea", "idea scoped"]
                ),
                inventory_finding(
                    Kind::Offline,
                    "scripted localisation needs recorded country context; unavailable offline",
                    &[scripted_example]
                ),
            ]
        );
    }
    for key in [
        products::country_groups::CountryGroups::KEY,
        products::country_measure::CountryMeasure::KEY,
        products::country_fuel::CountryFuel::KEY,
        products::occupation_trend::OccupationTrend::KEY,
    ] {
        let check = checks.iter().find(|check| check.key == key).unwrap();
        assert!(check.findings.is_empty() && check.error.is_none() && !check.note.is_empty());
    }
}

#[test]
fn country_name_checks_report_bare_opaque_tags() {
    use crate::{
        content_check::{Kind, check},
        products::{self, Product},
    };
    let stack = Stack::new("offline_country_name");
    stack.write(
        &stack.addon(),
        "common/country_tags/mod.txt",
        "\"tag with spaces\" = \"countries/mod.txt\"",
    );
    let catalogue = stack.catalogue(&[]);
    let checks = check(Arc::clone(&catalogue.content), "english");
    for key in [
        products::country_styles::CountryStyles::KEY,
        products::land_owners::LandOwnersProduct::KEY,
    ] {
        let check = checks.iter().find(|check| check.key == key).unwrap();
        assert_eq!(check.error, None);
        assert_eq!(
            check.findings,
            vec![inventory_finding(
                Kind::Content,
                "country-name ladder has no localisation; the bare tag key is copied (142230940)",
                &[
                    "country name \"tag with spaces\" group None: no localisation key names tag with spaces; the country name is unavailable and the tag is shown"
                ]
            )]
        );
    }
}

#[test]
fn offline_flags_use_groups_and_content_dynamic_boundary() {
    use crate::{
        content_check::{Kind, check},
        products::{self, Product},
    };
    let stack = Stack::new("flag_inventory");
    let addon = stack.addon();
    stack.write(&addon, "common/country_tags/mod.txt",
        "D01 = countries/static.txt missing = countries/missing.txt dynamic_tags = yes \"runtime slot\" = countries/runtime.txt");
    stack.write(
        &addon,
        "common/ideologies/mod.txt",
        "ideologies = { mod_group = { color = { 0.2 0.4 0.6 } types = { mod_type = {} } } }",
    );
    stack.write(&addon, "common/defines/mod.lua", concat!(
        "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} }, NGraphics = { ",
        "COUNTRY_FLAG_TEX_WIDTH = 1, COUNTRY_FLAG_TEX_HEIGHT = 1, COUNTRY_FLAG_TEX_MAX_SIZE = 8, ",
        "COUNTRY_FLAG_MEDIUM_TEX_WIDTH = 1, COUNTRY_FLAG_MEDIUM_TEX_HEIGHT = 1, COUNTRY_FLAG_MEDIUM_TEX_MAX_SIZE = 8, ",
        "COUNTRY_FLAG_SMALL_TEX_WIDTH = 1, COUNTRY_FLAG_SMALL_TEX_HEIGHT = 1, COUNTRY_FLAG_SMALL_TEX_MAX_SIZE = 8 } }")
    );
    // This static tag happens to look like a vanilla dynamic slot. Its
    // group-only flags never invite a no-group diagnostic.
    for folder in ["", "medium/", "small/"] {
        stack.write(
            &addon,
            &format!("gfx/flags/{folder}D01_mod_group.tga"),
            tga_row(&[BLUE]),
        );
    }
    let catalogue = stack.catalogue(&[]);
    let checks = check(Arc::clone(&catalogue.content), "english");
    let flags = checks
        .iter()
        .find(|check| check.key == products::country_flags::CountryFlags::KEY)
        .unwrap();
    assert!(flags.error.is_none());
    assert!(
        !flags
            .findings
            .iter()
            .any(|finding| finding.subject.contains("D01")),
        "{flags:?}"
    );
    let no_group: Vec<_> = flags
        .findings
        .iter()
        .filter(|finding| finding.subject.ends_with("group None"))
        .collect();
    assert_eq!(no_group.len(), 1);
    assert_eq!(no_group[0].kind, Kind::Fallback);
    assert!(no_group[0].subject.contains("missing"));
    for key in [
        products::country_flags::CountryFlags::KEY,
        products::country_styles::CountryStyles::KEY,
        products::land_owners::LandOwnersProduct::KEY,
    ] {
        let findings = &checks
            .iter()
            .find(|check| check.key == key)
            .unwrap()
            .findings;
        let offline: Vec<_> = findings
            .iter()
            .filter(|finding| finding.kind == Kind::Offline)
            .collect();
        assert_eq!(offline.len(), 1, "{key}: {findings:?}");
        assert_eq!(offline[0].count, Some(1));
        assert_eq!(offline[0].examples, ["country \"runtime slot\""]);
        assert!(
            !findings
                .iter()
                .filter(|finding| finding.kind != Kind::Offline)
                .any(|finding| finding.subject.contains("runtime slot"))
        );
    }
    let parties = checks
        .iter()
        .find(|check| check.key == products::party_history::PartyHistory::KEY)
        .unwrap();
    let dynamic: Vec<_> = parties
        .findings
        .iter()
        .filter(|finding| finding.kind == Kind::Offline)
        .collect();
    assert_eq!(dynamic.len(), 1, "{parties:?}");
    assert_eq!(dynamic[0].examples, ["country \"runtime slot\""]);
    assert!(
        parties
            .findings
            .iter()
            .filter(|finding| finding.kind != Kind::Offline)
            .all(|finding| !finding
                .examples
                .iter()
                .any(|example| example.contains("runtime slot"))),
        "{parties:?}"
    );
}

#[test]
fn missing_resource_keys_are_content_and_shared_icon_refusals_are_not_repeated() {
    use crate::content_check::Kind;
    let stack = Stack::new("resource_refusals");
    let catalogue = stack.catalogue(&[]);
    let mut cached = BTreeMap::new();
    let findings = resource_content(
        &ContentView::of(&catalogue),
        &[("mod_ore".into(), 1), ("mod_crystal".into(), 1)],
        &mut cached,
        &mut Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        cached[&("mod_ore".into(), 1)].name.as_deref(),
        Some("PRODUCTION_MATERIALS_MOD_ORE")
    );
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.kind == Kind::Content)
            .count(),
        2,
        "{findings:?}"
    );
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding.kind == Kind::Refused)
            .count(),
        1,
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding.kind != Kind::Fallback),
        "{findings:?}"
    );
}

#[test]
fn ideology_colour_refusals_emit_one_warning_without_text_refusals() {
    use crate::content_check::Kind;
    let stack = Stack::new("ideology_colour");
    stack.write(
        &stack.addon(),
        "common/ideologies/mod.txt",
        "ideologies = { mod_group = { color = 16711680 types = { mod_type = {} } } }",
    );
    let politics = stack.check_politics(&[]);
    let mut warnings = Vec::new();
    ideology_infos(&politics, &mut warnings);
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning.contains("colour is unavailable"))
            .count(),
        1,
        "{warnings:?}"
    );
    assert!(
        politics
            .catalogue
            .refusal_lines()
            .iter()
            .all(|warning| !warning.contains("colour is unavailable"))
    );
    assert_eq!(
        politics
            .catalogue
            .take_findings()
            .iter()
            .filter(|finding| finding.kind == Kind::Warning
                && finding.message.contains("colour is unavailable"))
            .count(),
        1
    );
}

#[test]
fn an_iconless_focus_draws_the_engine_s_unknown_goal_or_says_it_cannot() {
    use crate::{
        content_check::{Kind, check},
        products::{self, Product},
    };
    let stack = Stack::new("focus_omissions");
    stack.write(
        &stack.addon(),
        "common/national_focus/mod.txt",
        "focus_tree = { focus = { id = mod_focus } }",
    );
    stack.write(
        &stack.addon(),
        "localisation/mod_l_english.yml",
        "l_english:\n mod_focus:0 \"Mod focus\"\n",
    );
    let focus_check = |stack: &Stack| {
        let catalogue = stack.catalogue(&[]);
        check(Arc::clone(&catalogue.content), "english")
            .into_iter()
            .find(|check| check.key == products::national_focus::NationalFocus::KEY)
            .unwrap()
    };
    // The stack lacks GFX_goal_unknown: the fallback is named, not assumed.
    let focus = focus_check(&stack);
    assert!(focus.error.is_none(), "{focus:?}");
    assert_eq!(focus.findings.len(), 2);
    let missing = focus
        .findings
        .iter()
        .find(|finding| finding.kind == Kind::Fallback)
        .unwrap();
    assert_eq!(
        missing.message,
        "icon is unavailable; a placeholder is shown"
    );
    assert_eq!(missing.count, Some(1));
    assert_eq!(
        missing.examples,
        [
            "focus mod_focus: sprite GFX_goal_unknown is undeclared or has no texture; a placeholder is shown"
        ]
    );
    assert!(
        focus
            .note
            .contains("1 missing description localisation keys")
    );
    // A mod's own GFX_goal_unknown is drawn, as the game draws it.
    stack.write(
        &stack.addon(),
        "interface/goals.gfx",
        "spriteTypes = { spriteType = { name = GFX_goal_unknown texturefile = gfx/goal_unknown.tga } }",
    );
    stack.write(&stack.addon(), "gfx/goal_unknown.tga", tga_row(&[BLUE]));
    let focus = focus_check(&stack);
    assert_eq!(focus.findings.len(), 1);
    assert_eq!(focus.findings[0].kind, Kind::Content);
    assert!(
        focus.findings[0]
            .message
            .contains("mod_focus_desc is missing; the key is shown")
    );
    let layers = stack.catalogue(&[]).content.layers.clone();
    let live = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
    let mut context =
        products::artwork::Context::load(live, products::artwork::Kind::Focus).unwrap();
    let (reply, _) = context
        .catalogue(
            std::collections::BTreeSet::from(["mod_focus"]),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(reply[0].description.as_deref(), Some("mod_focus_desc"));
}

#[test]
fn truncated_flag_is_a_product_refusal_rather_than_engine_parity() {
    use crate::{content_check::Kind, products::country_flags::FlagCache};
    use installed_content::flags::FlagSet;
    let stack = Stack::new("truncated_flag");
    let mut bytes = tga_row(&[BLUE]);
    bytes.pop();
    stack.write(&stack.addon(), "gfx/flags/truncated.tga", bytes);
    let catalogue = stack.catalogue(&[]);
    let (found, findings) = find_flag(
        &catalogue.content.layers,
        &[FlagSet::Normal],
        &[[1, 1]; 3],
        &["truncated".into()],
        &mut FlagCache::default(),
    )
    .unwrap();
    assert!(matches!(found, FlagSearch::Absent));
    assert_eq!(findings[0].kind, Kind::Refused);
    assert!(
        findings[0]
            .message
            .contains("engine does not check read status")
    );
    assert!(!findings.iter().any(|finding| finding.kind == Kind::Content));
}

#[test]
fn native_reader_recovery_is_owned_by_the_corpus_reader() {
    let stack = Stack::new("native_recovery");
    stack.write(
        &stack.addon(),
        "common/ideas/mod.txt",
        "ideas = { group = {} group = { use_list_view = yes } }",
    );
    let politics = stack.politics(&[]);
    let (_, _, notes) = installed_content::ideas::read(&politics.catalogue.content.layers, &[]).unwrap();
    let (kind, message) = notes
        .iter()
        .find(|(_, note)| note.contains("where its reader reads a block"))
        .unwrap();
    assert_eq!(*kind, installed_content::DiagnosticKind::Misread);
    let checks = crate::content_check::check(Arc::clone(&politics.catalogue.content), "english");
    assert!(
        checks
            .iter()
            .flat_map(|check| &check.findings)
            .all(|finding| finding.message != *message
                && !finding
                    .examples
                    .iter()
                    .any(|example| example.contains(message)))
    );
}

#[test]
fn stability_advisor_names_and_roles_need_the_recorded_holder() {
    use crate::{
        content_check::{Kind, check},
        products::{self, Product},
    };
    let stack = Stack::new("stability_holder");
    stack.write(
        &stack.addon(),
        "localisation/mod_l_english.yml",
        "l_english:\n seat:0 \"Seat\"\n",
    );
    stack.write(&stack.addon(), "common/characters/mod.txt", "characters = { split = { instance = { allowed = { always = yes } name = NAME_ONE advisor = { idea_token = split_idea slot = seat } } instance = { allowed = { always = no } name = NAME_TWO advisor = { idea_token = split_idea slot = other_seat } } } named = { instance = { allowed = { always = yes } name = NAME_ONE advisor = { idea_token = named_idea slot = seat } } instance = { allowed = { always = no } name = NAME_TWO advisor = { idea_token = named_idea slot = seat } } } }");
    let politics = stack.politics(&[]);
    let checks = check(Arc::clone(&politics.catalogue.content), "english");
    for key in [
        products::country_stability::CountryStability::KEY,
        products::stability_parts::StabilityParts::KEY,
    ] {
        let check = checks.iter().find(|check| check.key == key).unwrap();
        assert!(check.error.is_none(), "{check:?}");
        assert!(
            check
                .findings
                .iter()
                .all(|finding| matches!(finding.kind, Kind::Offline | Kind::Ambiguous)),
            "{check:?}"
        );
        assert!(
            check.findings.iter().any(|finding| finding
                .message
                .starts_with("advisor role needs its recorded holder template;")),
            "{check:?}"
        );
        assert!(
            check
                .findings
                .iter()
                .any(|finding| finding.kind == Kind::Ambiguous
                    && finding.message.contains("alternatives name different keys")),
            "{check:?}"
        );
    }
}

#[test]
fn ideas_draw_the_mods_unknown_sprite_or_name_its_failure() {
    let stack = Stack::new("unknown_idea_art");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &addon,
        "common/ideas/mod.txt",
        "ideas = { spirits = { mod_artless = { picture = absent } } }",
    );
    stack.write(&base, "interface/unknown.gfx", "spriteTypes = { spriteType = { name = GFX_idea_unknown texturefile = gfx/interface/unknown.tga } }");
    stack.write(&base, "gfx/interface/unknown.tga", tga_row(&[RED]));
    // Same path overridden by a mod: the engine constant names a stack asset.
    stack.write(&addon, "gfx/interface/unknown.tga", tga_row(&[GREEN]));
    let politics = stack.politics(&[]);
    let (art, problem) = politics.idea_artwork("mod_artless", None, fixture_country());
    assert_eq!(art.icon, Some(pixel_png(GREEN)));
    assert_eq!(problem, None);
    let checks = crate::content_check::check(Arc::clone(&politics.catalogue.content), "english");
    let ideas = checks
        .iter()
        .find(|check| check.key == "idea-history")
        .unwrap();
    assert!(
        ideas
            .findings
            .iter()
            .all(|finding| !finding.message.contains("fallback sprite")
                && !finding.message.contains("placeholder")),
        "{ideas:?}"
    );
    // A replace_path removes the declaration as well as the mod's candidates.
    let politics = stack.politics(&["interface"]);
    let (art, problem) = politics.idea_artwork("mod_artless", None, fixture_country());
    assert!(art.icon.is_none());
    assert!(
        problem
            .unwrap()
            .contains("fallback sprite GFX_idea_unknown is undeclared")
    );
    stack.write(&addon, "interface/unknown.gfx", "spriteTypes = { spriteType = { name = GFX_idea_unknown texturefile = gfx/interface/broken.tga } }");
    stack.write(&addon, "gfx/interface/broken.tga", b"unreadable");
    let politics = stack.politics(&["interface"]);
    let (art, problem) = politics.idea_artwork("mod_artless", None, fixture_country());
    assert!(art.icon.is_none());
    assert!(
        problem
            .unwrap()
            .contains("fallback sprite GFX_idea_unknown is unreadable")
    );
}

#[test]
fn corpus_separates_recording_context_engine_names_and_uncited_technology_fallbacks() {
    use crate::content_check::{Kind, check};
    let stack = Stack::new("round3_classes");
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/terrain/fixture.txt",
        "categories = { fixture = {} }",
    );
    stack.write(&addon, "common/national_focus/mod.txt", "focus_tree = { focus = { id = scripted_focus icon = { GFX_trigger_chosen = { always = yes } } } focus = { id = absent_icon } }");
    stack.write(
        &addon,
        "common/ideas/mod.txt",
        "ideas = { spirits = { mod_idea = { } } }",
    );
    stack.write(
        &addon,
        "common/idea_tags/mod.txt",
        "idea_categories = { spirits = { type = national_spirit } }",
    );
    stack.write(
        &addon,
        "common/country_tags/mod.txt",
        "opaque_tag = countries/mod.txt",
    );
    stack.write(
        &addon,
        "common/technologies/mod.txt",
        "technologies = { mod_tech = { enable_equipments = { mod_equipment } } }",
    );
    stack.write(&addon, "interface/mod.gfx", "spriteTypes = { spriteType = { name = GFX_idea_unknown texturefile = gfx/interface/mod.tga } spriteType = { name = GFX_mod_tech_medium texturefile = gfx/interface/mod.tga } }");
    stack.write(&addon, "gfx/interface/mod.tga", tga_row(&[GREEN]));
    stack.write(&addon, "localisation/mod_l_english.yml", concat!(
        "l_english:\n scripted_focus_desc:0 \"[ROOT.GetName]\"\n absent_icon_desc:0 \"[THIS.GetName]\"\n",
        " mod_idea:0 \"Idea\"\n mod_idea_desc:0 \"[ROOT.GetName]\"\n spirits:0 \"Spirits\"\n",
        " mod_equipment:0 \"Equipment name\"\n mod_equipment_desc:0 \"Equipment description\"\n"));
    let politics = stack.politics(&[]);
    let checks = check(Arc::clone(&politics.catalogue.content), "english");
    let findings = |key| {
        &checks
            .iter()
            .find(|check| check.key == key)
            .unwrap()
            .findings
    };
    let focuses = findings("national-focus");
    assert!(
        focuses.iter().any(|finding| finding.kind == Kind::Offline
            && finding.count == Some(2)
            && finding.message
                == "scripted localisation needs recorded country context; unavailable offline"),
        "{focuses:?}"
    );
    assert!(
        focuses.iter().any(|finding| finding.kind == Kind::Offline
            && finding.message == "focus icon requires recorded context"
            && finding.count == Some(1)),
        "{focuses:?}"
    );
    assert!(
        focuses.iter().any(|finding| finding.kind == Kind::Fallback
            && finding.message == "icon is unavailable; a placeholder is shown"
            && finding.count == Some(1)
            && finding
                .examples
                .iter()
                .any(|example| example.starts_with("focus absent_icon: "))),
        "{focuses:?}"
    );
    assert!(findings("idea-history").iter().any(|finding| {
        finding.kind == Kind::Unsupported
            && finding.message == "scripted localisation is not evaluated because this product does not pass a country"
    }));
    assert!(
        focuses
            .iter()
            .flat_map(|finding| &finding.examples)
            .all(|example| !example.contains("this product does not pass"))
    );
    for key in ["land-owners", "country-styles"] {
        assert!(
            findings(key)
                .iter()
                .any(|finding| finding.kind == Kind::Content
                    && finding.message == "country-name ladder has no localisation; the bare tag key is copied (142230940)"
                    && finding
                        .examples
                        .iter()
                        .any(|example| example.contains("no localisation key names opaque_tag"))
                    && finding.count == Some(1)),
            "{:?}",
            findings(key)
        );
    }
    assert_eq!(
        findings("research"),
        &[
            crate::content_check::Finding::new(
                Kind::Content,
                "technology mod_tech",
                "localisation mod_tech is missing; the key is shown"
            ),
            crate::content_check::Finding::new(
                Kind::Content,
                "technology mod_tech",
                "localisation mod_tech_desc is missing; the key is shown"
            ),
        ]
    );
}

#[test]
fn character_reader_notes_are_not_repeated_by_product_preparation() {
    let stack = Stack::new("shared_politics_notes");
    stack.write(
        &stack.addon(),
        "common/characters/mod.txt",
        "characters = { mod_incomplete = { advisor = { idea_token = incomplete } } }",
    );
    let politics = stack.politics(&[]);
    let (_, diagnostics) =
        installed_content::characters::read(&politics.catalogue.content.layers, Default::default())
            .unwrap();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].0, installed_content::DiagnosticKind::Warning);
    let checks = crate::content_check::check(Arc::clone(&politics.catalogue.content), "english");
    for (_, message) in diagnostics {
        assert!(
            checks
                .iter()
                .flat_map(|check| &check.findings)
                .all(|finding| finding.message != message
                    && !finding
                        .examples
                        .iter()
                        .any(|example| example.contains(&message)))
        );
    }
}

#[test]
fn recorded_holder_names_settle_alternatives_but_authored_portraits_still_do_not() {
    use crate::content_check::{Kind, check};
    let stack = Stack::new("recorded_names");
    let addon = stack.addon();
    stack.write(&addon, "common/characters/mod.txt", concat!(
        "characters = { mod_character = { instance = { allowed = { always = yes } name = name_one ",
        "advisor = { idea_token = mod_advisor slot = seat } portraits = { civilian = { small = GFX_one } } } ",
        "instance = { allowed = { always = no } name = name_two advisor = { idea_token = mod_advisor slot = seat } ",
        "portraits = { civilian = { small = GFX_two } } } } }"));
    stack.write(
        &addon,
        "localisation/mod_l_english.yml",
        "l_english:\n seat:0 \"Seat\"\n saved_name:0 \"Recorded name\"\n",
    );
    let politics = stack.check_politics(&[]);
    let (art, problem) = politics.held_idea_artwork(
        "mod_advisor",
        Some("mod_character"),
        "saved_name",
        fixture_country(),
    );
    assert_eq!(art.name.as_deref(), Some("Recorded name"));
    assert!(
        problem
            .unwrap()
            .contains("first small portrait (civilian) is ambiguous")
    );
    assert!(
        !politics
            .catalogue
            .take_findings()
            .iter()
            .any(|finding| finding.message.contains("alternatives name different keys"))
    );
    let checks = check(Arc::clone(&politics.catalogue.content), "english");
    let ideas = checks
        .iter()
        .find(|check| check.key == "idea-history")
        .unwrap();
    assert!(
        ideas
            .findings
            .iter()
            .any(|finding| finding.kind == Kind::Offline
                && finding
                    .message
                    .starts_with("character name needs the recorded holder name;")),
        "{ideas:?}"
    );
    assert!(
        ideas
            .findings
            .iter()
            .any(|finding| finding.kind == Kind::Ambiguous
                && finding.message.contains("first small portrait")),
        "{ideas:?}"
    );
    assert_eq!(
        ideas
            .findings
            .iter()
            .filter(|finding| finding.kind == Kind::Ambiguous
                && finding.message.contains("first small portrait"))
            .count(),
        1,
        "{ideas:?}"
    );

    let (unnamed, _) =
        politics.held_idea_artwork("mod_advisor", Some("mod_character"), "", fixture_country());
    assert!(unnamed.name.is_none());
    let recorded = politics.catalogue.take_findings();
    assert!(
        recorded
            .iter()
            .any(|finding| finding.kind == Kind::Ambiguous
                && finding.message.contains("alternatives name different keys")),
        "{recorded:?}"
    );
}

#[test]
fn live_catalogue_does_not_accumulate_product_findings() {
    use crate::content_check::{Finding, Kind};
    let stack = Stack::new("live_findings");
    stack.write(&stack.addon(), "common/national_focus/mod.txt", "focus_tree = { focus = { id = mod_live_focus icon = { GFX_trigger_chosen = { always = yes } } } }");
    let prepared = stack.politics(&[]);
    let live = Arc::new(
        Catalogue::of_layers(prepared.catalogue.content.layers.clone(), "english").unwrap(),
    );
    let politics = PoliticsContent::load(Arc::clone(&live)).unwrap();
    let before = live.refusal_lines();
    for index in 0..12 {
        assert!(
            politics
                .localised(&format!("missing_mod_key_{index}"), catalogue::View::Plain)
                .is_some()
        );
        live.note(Finding::new(
            Kind::Fallback,
            format!("idea {index}"),
            "artwork unavailable",
        ));
        live.note(Finding::new(
            Kind::Ambiguous,
            format!("character {index}"),
            "name alternatives",
        ));
    }
    assert_eq!(live.refusal_lines(), before);
    assert!(live.take_findings().is_empty());
    let mut context = crate::products::artwork::Context::load(
        Arc::clone(&live),
        crate::products::artwork::Kind::Focus,
    )
    .unwrap();
    let tokens = std::collections::BTreeSet::from(["mod_live_focus"]);
    let (first, warnings) = context
        .catalogue(tokens.clone(), &AtomicBool::new(false))
        .unwrap();
    let mut fresh = crate::products::artwork::Context::load(
        Arc::clone(&live),
        crate::products::artwork::Kind::Focus,
    )
    .unwrap();
    let (second, second_warnings) = fresh.catalogue(tokens, &AtomicBool::new(false)).unwrap();
    assert_eq!(first[0].description.as_deref(), Some("mod_live_focus_desc"));
    assert_eq!(first[0].token, second[0].token);
    assert_eq!(first[0].description, second[0].description);
    assert_eq!(first[0].name, second[0].name);
    assert_eq!(first[0].icon, second[0].icon);
    assert!(
        warnings.iter().any(|warning| warning
            .contains("focus icon requires recorded country/hour trigger operands")),
        "{warnings:?}"
    );
    assert_eq!(second_warnings, warnings);
    assert!(live.take_findings().is_empty());
    live.refuse("same text refusal", "idea one");
    live.refuse("same text refusal", "idea two");
    assert_eq!(
        live.refusal_lines()
            .iter()
            .filter(|line| line.contains("same text refusal"))
            .count(),
        1
    );
}

#[test]
fn offline_idea_icons_skip_country_candidates_and_aggregate_the_dependency() {
    use crate::content_check::{Kind, check};
    let stack = Stack::new("offline_idea_icon_country");
    let addon = stack.addon();
    stack.write(&addon, "common/ideas/mod.txt", "ideas = { spirits = { mod_country_only = { name = country_name picture = country_picture } mod_other_country = { name = other_name picture = other_picture } mod_static = { name = static_name } } }");
    stack.write(
        &addon,
        "common/idea_tags/mod.txt",
        "idea_categories = { spirits = { type = national_spirit } }",
    );
    stack.write(&addon, "interface/mod.gfx", "spriteTypes = { spriteType = { name = GFX_idea_country_picture_MOD texturefile = gfx/mod.tga } spriteType = { name = GFX_idea_country_picture_ texturefile = gfx/invalid.tga } spriteType = { name = GFX_idea_mod_static texturefile = gfx/mod.tga } }");
    stack.write(&addon, "gfx/mod.tga", tga_row(&[GREEN]));
    stack.write(&addon, "gfx/invalid.tga", [0, 0, 1]);
    let politics = stack.politics(&[]);
    let (live, problem) = politics.idea_artwork("mod_country_only", None, fixture_country());
    assert_eq!(live.icon, Some(pixel_png(GREEN)));
    assert!(problem.is_none(), "{problem:?}");
    let checks = check(Arc::clone(&politics.catalogue.content), "english");
    let ideas = checks
        .iter()
        .find(|check| check.key == "idea-history")
        .unwrap();
    let dependency = ideas.findings.iter().find(|finding| finding.message == "idea icon needs the recorded holder tag after its static candidates; unavailable offline").unwrap();
    assert_eq!(dependency.kind, Kind::Offline);
    assert_eq!(dependency.count, Some(2));
    assert_eq!(
        dependency.examples,
        ["idea mod_country_only", "idea mod_other_country"]
    );
    assert!(
        !ideas
            .findings
            .iter()
            .any(|finding| matches!(finding.kind, Kind::Fallback | Kind::Refused)),
        "{ideas:?}"
    );
}

#[test]
fn trigger_chosen_focus_icons_are_one_counted_finding() {
    use crate::content_check::{Kind, check};
    let stack = Stack::new("focus_icon_aggregate");
    stack.write(&stack.addon(), "common/national_focus/mod.txt", "focus_tree = { focus = { id = mod_focus_one icon = { GFX_mod_one = { always = yes } } } focus = { id = mod_focus_two icon = { GFX_mod_two = { always = no } } } }");
    let catalogue = stack.catalogue(&[]);
    let checks = check(Arc::clone(&catalogue.content), "english");
    let focuses = checks
        .iter()
        .find(|check| check.key == "national-focus")
        .unwrap();
    let icons: Vec<_> = focuses
        .findings
        .iter()
        .filter(|finding| finding.message == "focus icon requires recorded context")
        .collect();
    assert_eq!(icons.len(), 1, "{focuses:?}");
    assert_eq!(icons[0].kind, Kind::Offline);
    assert_eq!(icons[0].count, Some(2));
    assert_eq!(
        icons[0].examples,
        [
            "focus mod_focus_one: focus icon requires recorded country/hour trigger operands",
            "focus mod_focus_two: focus icon requires recorded country/hour trigger operands"
        ]
    );
}

#[test]
fn corpus_checks_new_main_products_through_their_shared_preparation() {
    use crate::content_check::{Kind, check};
    let stack = Stack::new("main_product_preparation");
    let addon = stack.addon();
    content_map(&stack, &addon);
    stack.write(&addon, "common/defines/mod.lua", "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 }, NGame = { COMBAT_LOG_MAX_MONTHS = -1 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} } }");
    stack.write(
        &addon,
        "common/terrain/fixture.txt",
        "categories = { fixture = {} }",
    );
    stack.write(
        &addon,
        "common/units/equipment/mod.txt",
        "equipments = { mod_archetype = { is_archetype = yes } }",
    );
    stack.write(
        &addon,
        "common/buildings/mod.txt",
        "buildings = { mod_works = { military_production = 1 naval_production = 1 general_production = 1 icon_frame = 1 } unnamed_works = { military_production = 1 icon_frame = 2 } }",
    );
    stack.write(
        &addon,
        "common/autonomous_states/mod.txt",
        "autonomy_state = { id = mod_level min_freedom_level = 0.25 } autonomy_state = { id = unnamed_level min_freedom_level = 0.5 }",
    );
    stack.write(
        &addon,
        "common/occupation_laws/mod.txt",
        "mod_law = { icon = GFX_mod_law:1 }",
    );
    stack.write(
        &addon,
        "interface/mod.gfx",
        "spriteTypes = { spriteType = { name = GFX_mod_law texturefile = gfx/mod_law.tga } spriteType = { name = GFX_mod_buildings texturefile = gfx/mod_buildings.tga noOfFrames = 2 } spriteType = { name = GFX_mod_repair texturefile = gfx/mod_law.tga } spriteType = { name = GFX_mod_project texturefile = gfx/mod_law.tga } spriteType = { name = GFX_mod_archetype_medium texturefile = gfx/mod_equipment.tga } }",
    );
    stack.write(&addon, "interface/mod.gui", concat!(
        "guiTypes = { containerWindowType = { name = production_building_line_entry iconType = { name = picture spriteType = GFX_mod_buildings } iconType = { name = repair spriteType = GFX_mod_repair } } ",
        "containerWindowType = { name = combat_log_equipment_tab instantTextBoxType = { name = title_lost_in_combat text = MOD_LOST_IN_BATTLE } instantTextBoxType = { name = title_recovered text = MOD_SALVAGED } instantTextBoxType = { name = title_lost_by_attrition text = MOD_WORN_OUT } } ",
        "containerWindowType = { name = navallosseswindow instantTextBoxType = { name = total_ships pdx_tooltip = MOD_SHIPS_LOST } } }"));
    stack.write(&addon, "gfx/mod_law.tga", tga_row(&[BLUE]));
    stack.write(&addon, "gfx/mod_buildings.tga", tga_row(&[BLUE, GREEN]));
    stack.write(&addon, "gfx/mod_equipment.tga", [0, 0, 1]);
    stack.write(
        &addon,
        "history/states/mod.txt",
        "state = { id = 900 name = MOD_STATE }",
    );
    stack.write(
        &addon,
        "common/special_projects/specialization/mod.txt",
        "mod_specialization = {} ",
    );
    stack.write(&addon, "common/special_projects/projects/mod.txt", "mod_project = { specialization = mod_specialization icon = GFX_mod_project narrative = { name = missing_project_name } }");
    stack.write(
        &addon,
        "localisation/mod_l_english.yml",
        concat!("l_english:\n mod_level:0 \"Mod level\"\n mod_law:0 \"Mod law\"\n mod_works:0 \"Mod works\"\n MOD_STATE:0 \"Mod state\"\n mod_specialization:0 \"Mod specialization\"\n",
        " MOD_SALVAGED:0 \"Mod salvage\"\n MOD_WORN_OUT:0 \"Mod attrition\"\n MOD_SHIPS_LOST:0 \"Mod ships\"\n",
        " COMBAT_LOG_EQ_LOST_ATTR_OTHER:0 \"Other\"\n COMBAT_LOG_EQ_LOST_ATTR_COMBAT:0 \"Combat\"\n COMBAT_LOG_EQ_LOST_ATTR_TRAINING:0 \"Training\"\n",
        " LEDGER_CHART_DAMAGED_CONVOYS:0 \"Convoys\"\n LEND_LEASE_DETAILS_EQUIPMENT_SUNK:0 \"Sunk\"\n LOGISTICS_NEED_LEND_LEASE:0 \"Sent\"\n"),
    );
    let catalogue = stack.catalogue(&[]);
    let checks = check(Arc::clone(&catalogue.content), "english");
    type Expected = (Kind, Option<&'static str>, &'static str, Option<usize>);
    type Case<'a> = (
        &'a str,
        &'a [Expected],
        &'a [Kind],
        Option<usize>,
        Option<&'a str>,
    );
    let equipment = [(
        Kind::Fallback,
        Some("equipment type mod_archetype"),
        "",
        None,
    )];
    let autonomy = [(Kind::Content, None, "unnamed_level", None)];
    let construction = [
        (Kind::Content, None, "unnamed_works", None),
        (Kind::Offline, None, "", Some(2)),
    ];
    let production_lines = [(
        Kind::Fallback,
        Some("production equipment mod_archetype"),
        "picture unavailable",
        None,
    )];
    let cases: &[Case<'_>] = &[
        ("subjects", &autonomy, &[], None, None),
        ("occupied-states", &[], &[], Some(0), None),
        ("equipment-stockpile", &equipment, &[], None, None),
        ("equipment-produced", &equipment, &[], None, None),
        ("production-spending", &equipment, &[], None, None),
        (
            "equipment-losses",
            &[
                (
                    Kind::Fallback,
                    Some("loss cause CombatLog(0)"),
                    "MOD_LOST_IN_BATTLE",
                    None,
                ),
                (Kind::Refused, Some("Combat Log retention"), "is -1", None),
            ],
            &[],
            None,
            None,
        ),
        (
            "special-projects",
            &[(
                Kind::Content,
                Some("special project mod_project"),
                "missing_project_name",
                None,
            )],
            &[Kind::Fallback],
            None,
            None,
        ),
        ("autonomy", &autonomy, &[], None, None),
        (
            "construction-log",
            &construction,
            &[Kind::Fallback],
            None,
            None,
        ),
        (
            "construction-spending",
            &construction,
            &[Kind::Fallback],
            None,
            None,
        ),
        ("production-lines", &production_lines, &[], None, None),
        ("production-line-detail", &production_lines, &[], None, None),
        (
            "world-tension",
            &[],
            &[],
            Some(0),
            Some("WorldTension, ThreatCountryAggregates and ThreatSources"),
        ),
        ("trade-flows", &[], &[], Some(0), None),
        (
            "country-factories",
            &[(
                Kind::Content,
                Some("building unnamed_works"),
                "142230940",
                None,
            )],
            &[],
            Some(1),
            None,
        ),
    ];
    for (key, expected, forbidden, exact_count, note) in cases {
        let product = checks.iter().find(|product| product.key == *key).unwrap();
        assert!(product.error.is_none(), "{product:?}");
        if let Some(count) = exact_count {
            assert_eq!(product.findings.len(), *count, "{product:?}");
        }
        for (kind, subject, message, count) in *expected {
            let subject = *subject;
            assert!(
                product.findings.iter().any(|finding| {
                    finding.kind == *kind
                        && subject.is_none_or(|subject| {
                            finding.subject == subject
                                || finding.examples.iter().any(|example| {
                                    example == subject
                                        || example.starts_with(&format!("{subject}: "))
                                })
                        })
                        && (finding.message.contains(message)
                            || finding
                                .examples
                                .iter()
                                .any(|example| example.contains(message)))
                        && count.is_none_or(|count| finding.count == Some(count))
                }),
                "{key}: missing {kind:?}/{subject:?}/{message:?}/{count:?}: {product:?}"
            );
        }
        assert!(
            product
                .findings
                .iter()
                .all(|finding| !forbidden.contains(&finding.kind)),
            "{product:?}"
        );
        if let Some(note) = note {
            assert!(product.note.contains(note), "{product:?}");
        }
    }
}

#[test]
fn recorded_holder_names_survive_refused_content() {
    let stack = Stack::new("saved_name_refused_content");
    stack.write(
        &stack.addon(),
        "common/ideas/mod.txt",
        b"ideas = { spirits = { mod_idea = { name = \"\xfe\" } } }",
    );
    stack.write(
        &stack.addon(),
        "localisation/mod_l_english.yml",
        "l_english:\n saved_name:0 \"Recorded name\"\n",
    );
    let politics = stack.politics(&[]);
    assert!(politics.idea("mod_idea").is_err());
    let (art, problem) =
        politics.held_idea_artwork("mod_idea", None, "saved_name", fixture_country());
    assert_eq!(art.name.as_deref(), Some("Recorded name"));
    assert!(problem.is_some());
}

#[test]
fn factory_missing_names_are_content_and_refused_text_stays_refused() {
    use crate::{content_check::Kind, products::country_factories};
    let stack = Stack::new("factory_name_kind");
    let addon = stack.addon();
    stack.write(&addon, "common/buildings/mod.txt",
        "buildings = { unnamed_works = { military_production = 1 naval_production = 1 general_production = 1 } refused_works = { military_production = 1 naval_production = 1 general_production = 1 } }");
    stack.write(
        &addon,
        "localisation/mod_l_english.yml",
        "l_english:\n refused_works:0 \"¤\"\n",
    );
    let catalogue = stack.check_catalogue(&[]);
    let (groups, warnings) = country_factories::content_groups(&catalogue);
    assert!(
        groups
            .iter()
            .flat_map(|group| &group.buildings)
            .all(|building| building.name.is_none())
    );
    let findings = catalogue.take_findings();
    assert!(
        findings.iter().any(|finding| finding.kind == Kind::Content
            && finding.subject == "building unnamed_works"
            && finding.message.contains("142230940")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.kind == Kind::Unsupported
                && finding.subject == "refused_works"
                && finding.message.contains("control byte 0x14")),
        "{findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|finding| finding.kind == Kind::Content
                && finding.subject.contains("refused_works")),
        "{findings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("control byte 0x14")),
        "{warnings:?}"
    );
}

#[test]
fn shared_equipment_and_autonomy_reader_notes_are_not_repeated_by_products() {
    use crate::content_check::{Kind, check};
    let stack = Stack::new("shared_equipment_autonomy_notes");
    let addon = stack.addon();
    content_map(&stack, &addon);
    stack.write(&addon, "common/defines/mod.lua",
        "NDefines = { NGame = { COMBAT_LOG_MAX_MONTHS = 30 }, NBuildings = { MAX_BUILDING_LEVELS = 15 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} } }");
    let duplicates: String = (0..10)
        .map(|index| format!("ghost{index} = {{ archetype = nowhere }} "))
        .collect();
    stack.write(&addon, "common/units/equipment/mod.txt",
        format!("equipments = {{ mod_archetype = {{ is_archetype = yes }} }} duplicate_archetypes = {{ {duplicates} }}"));
    stack.write(
        &addon,
        "common/autonomous_states/mod.txt",
        "autonomy_state = { id = mod_subject min_freedom_level = nope }",
    );
    let catalogue = stack.catalogue(&[]);
    let database = installed_content::equipment::Database::load(&catalogue.content.layers).unwrap();
    assert_eq!(database.misreads.len(), 10, "{:?}", database.misreads);
    let (levels, warnings) = installed_content::autonomy::read(&catalogue.content.layers).unwrap();
    assert_eq!(levels.ordered().len(), 1);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let checks = check(Arc::clone(&catalogue.content), "english");
    for key in [
        "equipment-stockpile",
        "equipment-produced",
        "production-spending",
        "equipment-losses",
        "production-lines",
        "production-line-detail",
    ] {
        let product = checks.iter().find(|product| product.key == key).unwrap();
        assert!(product.error.is_none(), "{product:?}");
        assert!(
            product
                .findings
                .iter()
                .all(|finding| finding.kind != Kind::Misread && !finding.message.contains(" more")),
            "{product:?}"
        );
    }
    let emitted: Vec<_> = checks
        .iter()
        .flat_map(|product| {
            product
                .findings
                .iter()
                .filter(|finding| {
                    warnings
                        .iter()
                        .any(|(_, message)| message == &finding.message)
                })
                .map(move |finding| (product.key, finding))
        })
        .collect();
    assert_eq!(emitted.len(), 1, "{emitted:?}");
    assert_eq!(emitted[0].0, "autonomy_content");
    assert_eq!(emitted[0].1.kind, Kind::Refused);
}

#[test]
fn every_shared_preparation_note_has_one_owner_and_its_producer_kind() {
    use crate::content_check::{
        AUTONOMY_CHECK_KEY, CONSTRUCTION_CHECK_KEY, Finding, INTELLIGENCE_CHECK_KEY, Kind,
        PRODUCTION_LINES_CHECK_KEY, SHARED_CHECK_KEYS, check,
    };
    let stack = Stack::new("all_shared_note_owners");
    let addon = stack.addon();
    content_map(&stack, &addon);
    let mut bitmap = vec![0_u8; 58];
    bitmap[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1), (34, 4)] {
        bitmap[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    stack.write(&addon, "map/provinces.bmp", bitmap);
    stack.write(
        &addon,
        "common/terrain/fixture.txt",
        "categories = { fixture = {} }",
    );
    stack.write(&addon, "common/defines/mod.lua", "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 }, NGame = { COMBAT_LOG_MAX_MONTHS = 30 }, NNavy = { BATTLE_NAME_VP_FACTOR = 10, BATTLE_NAME_VP_CUTOFF = 1 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} }, NMilitary = { SUPPLY_GRACE = 10, LOW_SUPPLY = .6 }, NOperatives = { CRYPTO_BASE_CRYPTO_LEVEL = 12.5, CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE = 3.25 }, NGraphics = { SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD = .15, SUPPLY_UNIT_COUNTER_LOW_THRESHOLD = .4, COUNTRY_FLAG_TEX_WIDTH = 1, COUNTRY_FLAG_TEX_HEIGHT = 1, COUNTRY_FLAG_TEX_MAX_SIZE = 8, COUNTRY_FLAG_MEDIUM_TEX_WIDTH = 1, COUNTRY_FLAG_MEDIUM_TEX_HEIGHT = 1, COUNTRY_FLAG_MEDIUM_TEX_MAX_SIZE = 8, COUNTRY_FLAG_SMALL_TEX_WIDTH = 1, COUNTRY_FLAG_SMALL_TEX_HEIGHT = 1, COUNTRY_FLAG_SMALL_TEX_MAX_SIZE = 8 } }");
    stack.write(
        &addon,
        "common/buildings/mod.txt",
        "buildings = { mod_building = { icon_frame = 1 } }",
    );
    stack.write(
        &addon,
        "common/units/equipment/mod.txt",
        "equipments = { mod_equipment = { is_archetype = yes } }",
    );
    stack.write(
        &addon,
        "common/autonomous_states/mod.txt",
        "autonomy_state = { id = mod_subject min_freedom_level = nope }",
    );
    stack.write(
        &addon,
        "history/states/01-kept.txt",
        "@bad nope orphan=yes state={id=1 name=FIRST impassable=maybe provinces={0} resources={unknown=2} history={victory_points={1} buildings=4}}",
    );
    stack.write(
        &addon,
        "history/states/02-discarded.txt",
        "state = { id = 1 name = SECOND }",
    );
    stack.write(
        &addon,
        "common/country_tags/mod.txt",
        "MOD = countries/mod.txt",
    );
    stack.write(
        &addon,
        "common/resources/mod.txt",
        "resources = { mod_resource = {} }",
    );
    for (path, body) in [
        (
            "common/ideologies/mod.txt",
            "ideologies = { mod_group = { color = { 0.1 0.2 0.3 } } }",
        ),
        (
            "common/national_focus/mod.txt",
            "focus_tree = { focus = { id = mod_focus } }",
        ),
        (
            "common/technologies/mod.txt",
            "technologies = { mod_tech = { enable_equipments = { if = { limit = { always = yes } mod_equipment } } } }",
        ),
    ] {
        stack.write(&addon, path, format!("@bad nope orphan = yes {body}"));
    }
    stack.write(
        &addon,
        "common/operations/mod.txt",
        "mod_operation = { on_start = { free_operative = { operative = custom_name } } }",
    );
    stack.write(
        &addon,
        "common/operations/unsettled.txt",
        "unsettled_operation = { operation_target = { target_array = custom_array } }",
    );
    stack.write(&addon, "common/ideologies/mod.txt", "ideologies = { mod_group = { color = { 0.1 0.2 0.3 } modifiers = { mod_unknown_modifier = { value = 1 } } } }");
    let cat = stack.catalogue(&[]);
    let layers = &cat.content.layers;
    let checks = check(Arc::clone(&cat.content), "english");
    let errors: Vec<_> = checks
        .iter()
        .filter_map(|check| check.error.as_ref().map(|why| (check.key, why)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let emitted: Vec<_> = checks
        .iter()
        .flat_map(|check| check.findings.iter().map(move |note| (check.key, note)))
        .collect();
    let (buildings, states) = (
        crate::products::construction_log::named_all(
            &crate::products::construction_log::ContentView {
                layers,
                sprites: &cat.sprites,
                guis: &cat.guis,
                gui_decode: &cat.gui_decode,
                localise: &|key| cat.localised(key, crate::catalogue::View::Plain),
            },
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap(),
        crate::scripted_text::read_state_content(layers).unwrap(),
    );
    let equipment = Arc::new(installed_content::equipment::Database::load_complete(layers).unwrap());
    let icons = crate::products::equipment_icon::IconContent::with_equipment(
        Arc::clone(&cat),
        Some(equipment),
    )
    .unwrap();
    let (_, autonomy) = installed_content::autonomy::read(layers).unwrap();
    let intelligence =
        crate::products::intelligence_content::IntelligenceContent::load(Arc::clone(&cat)).unwrap();
    let mut shared = Vec::<(&str, Finding)>::new();
    shared.extend(
        autonomy
            .into_iter()
            .map(|note| (AUTONOMY_CHECK_KEY, Finding::reader("autonomy states", note))),
    );
    shared.extend(buildings.warnings.into_iter().map(|note| {
        (
            CONSTRUCTION_CHECK_KEY,
            Finding::reader("construction preparation", note),
        )
    }));
    shared.extend(icons.preparation_warnings().into_iter().map(|note| {
        (
            PRODUCTION_LINES_CHECK_KEY,
            Finding::reader("production-line preparation", note),
        )
    }));
    shared.extend(
        intelligence
            .check_reader_notes()
            .into_iter()
            .map(|note| (INTELLIGENCE_CHECK_KEY, note)),
    );
    for key in SHARED_CHECK_KEYS {
        assert!(
            shared.iter().any(|(owner, _)| owner == key),
            "{key}: {shared:?}"
        );
    }
    for (owner, note) in shared {
        let copies: Vec<_> = emitted
            .iter()
            .filter(|(_, found)| found.message == note.message)
            .collect();
        assert_eq!(copies.len(), 1, "{owner}: {note:?}: {copies:?}");
        assert_eq!(copies[0].0, owner);
        assert_eq!(copies[0].1.kind, note.kind);
    }
    let retention = crate::products::sinkings::retention(
        &crate::products::sinkings::Scanned::default(),
        Some(&cat),
    );
    assert_eq!(retention.warnings.len(), 2);
    for message in retention.warnings {
        let copies: Vec<_> = emitted
            .iter()
            .filter(|(_, note)| note.message == message)
            .collect();
        assert_eq!(copies.len(), 1, "{message}: {copies:?}");
        assert_eq!(copies[0].0, "naval-losses");
        assert_eq!(copies[0].1.kind, Kind::Warning);
    }
    for (kind, text) in [
        (Kind::Unsupported, "Deferred effect `free_operative`"),
        (Kind::Unsettled, "target_array"),
    ] {
        assert!(
            emitted.iter().any(|(owner, note)| {
                *owner == INTELLIGENCE_CHECK_KEY && note.kind == kind && note.message.contains(text)
            }),
            "{emitted:?}"
        );
        assert!(
            emitted.iter().all(|(owner, note)| {
                *owner != "agency-comparison" || !note.message.contains(text)
            }),
            "{emitted:?}"
        );
    }
    let proven: Vec<_> = states
        .warnings
        .iter()
        .filter(|note| note.cause.is_some())
        .collect();
    assert_eq!(proven.len(), 5, "{proven:?}");
    assert!(
        proven
            .iter()
            .all(|note| note.kind == installed_content::DiagnosticKind::Misread)
    );
    assert!(
        crate::scripted_text::live_state_warnings(&states.warnings)
            .any(|note| note.contains("definition `@bad` has no `=`"))
    );
    let map = installed_content::terrain::read_default_map(layers).unwrap();
    let provinces =
        installed_content::map::parse_definitions(&layers.read(&map.file("definitions").unwrap()).unwrap())
            .unwrap();
    let (_, offline) = installed_content::states::read(layers, &states.defines, &provinces).unwrap();
    let all_notes: Vec<_> = states
        .warnings
        .into_iter()
        .map(installed_content::states::Diagnostic::into_diagnostic)
        .collect();
    assert_eq!(offline, all_notes);
    assert_eq!(states.states.diagnostics(1).len(), 6);
    let mut owned = vec![all_notes];
    let (ideologies, ideology_notes) = installed_content::ideologies::read(layers).unwrap();
    assert!(ideologies.groups().1.is_some());
    assert!(
        ideology_notes
            .iter()
            .any(|(kind, _)| *kind == installed_content::DiagnosticKind::Unsettled),
        "{ideology_notes:?}"
    );
    assert!(
        emitted
            .iter()
            .all(|(owner, note)| *owner != "stability" || note.subject != "ideology inventory"),
        "{emitted:?}"
    );
    owned.push(ideology_notes);
    owned.push(installed_content::focus::read(layers).unwrap().1);
    owned.push(
        installed_content::technology::Database::read(layers)
            .unwrap()
            .inventory_warnings,
    );
    for notes in owned {
        assert!(
            !notes.is_empty(),
            "fixture must exercise each corpus-owned channel"
        );
        for (kind, message) in notes {
            let duplicates: Vec<_> = emitted
                .iter()
                .filter(|(_, note)| {
                    note.message.contains(&message)
                        || note
                            .examples
                            .iter()
                            .any(|example| example.contains(&message))
                })
                .collect();
            assert!(duplicates.is_empty(), "{kind:?}: {message}: {duplicates:?}");
        }
    }
}

#[test]
fn resource_reader_recovery_notes_do_not_enter_resource_product_checks() {
    use crate::content_check::check;
    let stack = Stack::new("resource_note_ownership");
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/resources/mod.txt",
        "@bad nope orphan = yes resources = { mod_resource = {} }",
    );
    let cat = stack.catalogue(&[]);
    let notes = installed_content::resources::read(&cat.content.layers).unwrap().1;
    assert!(!notes.is_empty());
    let checks = check(Arc::clone(&cat.content), "english");
    // A resource recovery refuses building-registry preparation; the resource
    // products read resources directly and retain their own names/icon checks.
    for key in ["resource-balance", "trade-flows"] {
        let product = checks.iter().find(|check| check.key == key).unwrap();
        assert!(product.error.is_none(), "{product:?}");
        for (_, message) in &notes {
            assert!(
                product.findings.iter().all(|finding| {
                    !finding.message.contains(message)
                        && finding
                            .examples
                            .iter()
                            .all(|example| !example.contains(message))
                }),
                "{product:?}"
            );
        }
    }
}
