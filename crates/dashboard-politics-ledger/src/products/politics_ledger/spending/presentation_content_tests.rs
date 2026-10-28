use super::tests::{event, frame, put};
use super::*;
use installed_content_core::ResolvedLayers;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

// Reuse the installed corpus's descriptor/dependency planner, then gather
// exactly as a recording does (DLC archives and native mount order included).
#[path = "../../../../../viewer/src/corpus/stacks.rs"]
mod installed_stacks;

/// Synthetic assets only. The local worker supplies its capture scratch root.
struct Stack {
    root: PathBuf,
}
impl Stack {
    fn new() -> Self {
        let scratch = std::env::var_os("BATTLE_PLAN_POL_SPENDING_TEST_SCRATCH").or_else(|| std::env::var_os("HOI4_POL_SPENDING_TEST_SCRATCH"))
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = scratch.join(format!(
            "presentation_{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let stack = Self { root };
        for layer in [stack.base(), stack.addon()] {
            std::fs::create_dir_all(layer).unwrap();
        }
        dashboard_text::catalogue::texture_tests::write_sprite_program(&stack.base());
        stack
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
        // The file BOM is required before its language header.
        if path.extension().is_some_and(|extension| extension == "yml")
            && !bytes.starts_with(b"\xef\xbb\xbf")
        {
            std::fs::write(path, [b"\xef\xbb\xbf".as_slice(), bytes].concat()).unwrap();
        } else {
            std::fs::write(path, bytes).unwrap();
        }
    }
    fn layers(&self) -> ResolvedLayers {
        ResolvedLayers::from_folders(&[(&self.base(), &[]), (&self.addon(), &[])]).unwrap()
    }
    fn content(&self) -> PoliticsContent {
        self.write(&self.base(), "common/defines/00_fixture.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"mod_officers\" } } }");
        self.write(
            &self.base(),
            "common/terrain/00_fixture.txt",
            "categories = { fixture_terrain = { } }",
        );
        self.write(
            &self.base(),
            "common/buildings/00_fixture.txt",
            "buildings = { fixture_building = { } }",
        );
        let catalogue = Catalogue::of_layers(self.layers(), "english").unwrap();
        assert!(
            catalogue.localisation.get("C1").is_some(),
            "fixture localisation was not loaded: {:?}",
            catalogue.localisation.logged
        );
        assert!(
            catalogue.localisation.refused.is_empty(),
            "{:?}",
            catalogue.localisation.refused
        );
        let content = PoliticsContent::load(Arc::new(catalogue)).unwrap();
        assert!(
            content.power_content().is_ok(),
            "fixture power content was not loaded: {:?}",
            content.power_content().err()
        );
        content
    }
    fn png(&self, layer: &Path, path: &str, pixel: [u8; 4]) {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixel)
                .unwrap();
        }
        self.write(layer, path, bytes);
    }
}
impl Drop for Stack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn named_frame(hour: i32, target_cosmetic: &str) -> DecodedFrame {
    let mut frame = frame(hour, 2);
    put(
        &mut frame,
        TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 1,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 2,
                original_tag_reference: Recorded::Value(Some(0)),
                cosmetic_tag: target_cosmetic.as_bytes().to_vec(),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::Politics(
            (1..=2)
                .map(|country| PoliticsRow {
                    country,
                    ruling_party: None,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(&mut frame, TableRows::PoliticalParties(Vec::new()));
    put(&mut frame, TableRows::Autonomy(Vec::new()));
    frame
}

fn item_icon<'a>(reply: &'a PoliticsLedgerReply, item: &'a LedgerSpendItem) -> Option<&'a str> {
    item.tooltip_token
        .as_deref()
        .and_then(|token| {
            reply
                .idea_tooltips
                .as_ref()?
                .artwork
                .iter()
                .find(|art| art.token == token)?
                .icon
                .as_deref()
        })
        .or(item.icon.as_deref())
}

/// A served reply's icon recipes are drawn as its request shows them: the recording's own without
/// a selection, the selection's with one, each recipe once per source, a second reply hitting
/// only the memo; one neither can draw says why on its item and among the warnings.
#[test]
fn served_ledger_icons_follow_the_selection_once_per_recipe() {
    use dashboard_base::test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select};
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
    assert_eq!(first_pixel(data["spendItems"][0]["icon"].as_str().unwrap()), RED);
    assert!(data["spendItems"][0].get("iconArt").is_none());
    assert!(data["spendItems"][2]["unknown"].as_str().unwrap().contains("GFX_missing"));
    assert!(data["warnings"][0].as_str().unwrap().contains("GFX_missing"));
    let before = renders();
    shown(&options);
    assert_eq!(renders() - before, 0, "a second reply only hits the memo");
    select(&source, &mut options, &["art"]);
    assert_eq!(first_pixel(shown(&options)["spendItems"][1]["icon"].as_str().unwrap()), BLUE);
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
        assert_eq!(first_pixel(data["spendItems"][0]["icon"].as_str().unwrap()), RED);
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
    draw_icons(&mut data, |_| {
        Ok(Ok((Some("data:image/png;base64,AA==".to_owned()), Some(note.to_owned()))))
    })
    .unwrap();
    for item in data["spendItems"].as_array().unwrap() {
        assert_eq!(item["icon"], "data:image/png;base64,AA==");
        assert_eq!(item["unknown"], note);
    }
    assert_eq!(data["warnings"], serde_json::json!([format!("{note} (count: 2)")]));
}

fn expected_png(pixel: [u8; 4]) -> String {
    dashboard_text::catalogue::png_data_url(&installed_content_image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: pixel.to_vec(),
    })
    .unwrap()
}

#[test]
fn stacked_mod_law_advisor_decision_ability_and_trait_use_content_names_and_png_art() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let base = stack.base();
    let addon = stack.addon();
    stack.write(
        &base,
        "common/ideas/spending.txt",
        "ideas = { mod_laws = { law = yes mod_law = { } } }",
    );
    stack.write(&addon, "common/ideas/spending.txt", "idea_categories = { mod_law_category = { slot = mod_laws } } ideas = { mod_laws = { law = yes mod_law = { picture = mod_law_picture } } }");
    stack.write(&addon, "common/characters/spending.txt", "characters = { mod_advisor = { name = mod_advisor_name advisor = { slot = mod_seat idea_token = mod_advisor_idea } } }");
    stack.write(
        &addon,
        "common/decisions/categories/spending.txt",
        "mod_category = { }",
    );
    stack.write(&addon, "common/decisions/spending.txt", "mod_category = { mod_decision = { name = mod_decision_title icon = mod_decision_picture } }");
    stack.write(
        &addon,
        "common/abilities/spending.txt",
        "ability = { mod_ability = { name = mod_ability_title icon = GFX_mod_ability } }",
    );
    stack.write(
        &addon,
        "common/unit_leader/spending.txt",
        "leader_traits = { Mod_Trait = { } }",
    );
    stack.write(&addon, "common/national_focus/spending.txt", "focus_tree = { id = mod_tree focus = { id = mod_focus icon = GFX_mod_focus } focus = { id = mod_focus_random icon = GFX_mod_focus } focus = { id = mod_focus_conditional icon = { GFX_mod_wrong = { always = no } GFX_mod_focus = { always = yes } GFX_mod_wrong = yes } } focus = { id = mod_focus_false icon = { GFX_mod_wrong = { always = no } } } focus = { id = mod_focus_override icon = GFX_mod_wrong alternate_icon = GFX_mod_wrong } focus = { id = mod_focus_alternate icon = GFX_mod_focus alternate_icon = GFX_mod_wrong } }");
    stack.write(&addon, "common/scripted_localisation/spending.txt", "defined_text = { name = RandomFocus text = { random_list = { 1 = { localization_key = FIRST } 1 = { localization_key = SECOND } } } }");
    stack.write(&addon, "common/national_focus/static.txt", "focus_tree = { id = mod_static_tree focus = { id = mod_focus_static icon = GFX_mod_focus } }");
    stack.write(
        &addon,
        "localisation/english/focus_static_l_english.yml",
        "l_english:\n mod_focus_static:0 \"Static mod focus\"\n",
    );
    stack.write(
        &base,
        "localisation/english/spending_l_english.yml",
        "l_english:\n C1:0 \"Base country\"\n mod_law:0 \"Base law\"\n",
    );
    stack.write(&addon, "localisation/english/spending_l_english.yml", "l_english:\n C1:0 \"Mod country\"\n C2:0 \"First target\"\n mod_law:0 \"Mod law\"\n mod_law_desc:0 \"Law for [ROOT.GetName]\"\n mod_law_category:0 \"Mod law category\"\n mod_advisor_saved:0 \"Mod advisor\"\n mod_advisor_name:0 \"Authored advisor\"\n mod_advisor_idea_desc:0 \"Advisor for [ROOT.GetName]\"\n mod_decision_title:0 \"Mod decision\"\n mod_ability_title:0 \"Mod ability\"\n C1_Mod_Trait:0 \"Country trait\"\n mod_focus:0 \"[ROOT.GetName]/[FROM.GetName]\"\n mod_focus_random:0 \"[RandomFocus]\"\n mod_focus_conditional:0 \"[ROOT.RandomFocus]\"\n FIRST:0 \"First choice\"\n SECOND:0 \"Second choice\"\n");
    stack.write(&addon, "interface/spending.gfx", "spriteTypes = { spriteType = { name = GFX_idea_mod_law texturefile = gfx/law.png } spriteType = { name = GFX_mod_advisor texturefile = gfx/advisor.png } spriteType = { name = GFX_decision_mod_decision_picture texturefile = gfx/decision.png } spriteType = { name = GFX_mod_ability texturefile = gfx/ability.png } spriteType = { name = GFX_trait_Mod_Trait texturefile = gfx/trait.png } spriteType = { name = GFX_mod_focus texturefile = gfx/focus.png } spriteType = { name = GFX_mod_wrong texturefile = gfx/advisor.png } spriteType = { name = GFX_goal_unknown texturefile = gfx/fallback.png } spriteType = { name = GFX_C1_mod_focus_override texturefile = gfx/focus.png } }");
    let pixels = [
        [200, 10, 20, 255],
        [10, 200, 20, 255],
        [10, 20, 200, 255],
        [200, 200, 20, 255],
        [200, 20, 200, 255],
    ];
    for (path, pixel) in ["law", "advisor", "decision", "ability", "trait"]
        .into_iter()
        .zip(pixels)
    {
        stack.png(&addon, &format!("gfx/{path}.png"), pixel);
    }
    let focus_pixel = [90, 120, 200, 255];
    stack.png(&addon, "gfx/focus.png", focus_pixel);
    let fallback_pixel = [13, 14, 15, 255];
    stack.png(&addon, "gfx/fallback.png", fallback_pixel);
    let content = stack.content();
    {
        let (parsed, _) = installed_content_focus::focus::parse(
            b"focus_tree = { focus = { id = repeated icon = GFX_first icon = GFX_second } focus = { id = conditional icon = { GFX_first = { always = no } } icon = GFX_second } }",
            installed_content_focus::focus::Database::National,
            &installed_content_core::key_names::KeyNames::default(),
        ).unwrap();
        assert_eq!(parsed["repeated"].icon.as_deref(), Some("GFX_first"));
        assert_eq!(parsed["repeated"].icons.cases.len(), 2);
        assert_eq!(parsed["conditional"].icon, None);
        assert_eq!(parsed["conditional"].icons.cases.len(), 2);
    }
    let mut first = named_frame(24, "");
    put(
        &mut first,
        TableRows::Characters(vec![CharacterRow {
            id: 9,
            country: 1,
            token: b"mod_advisor_saved".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut first,
        TableRows::CharacterPortraits(vec![CharacterPortraitRow {
            character: 9,
            portraits: vec![CharacterPortrait {
                category: replay_data::rows::PORTRAIT_CIVILIAN,
                size: replay_data::rows::PORTRAIT_LARGE,
                is_set: 1,
                kind: 0,
                identity: b"GFX_mod_advisor".to_vec(),
            }],
        }]),
    );
    let mut rows = Vec::new();
    for (sequence, cause, token) in [
        (1, PowerCause::Idea, "mod_law"),
        (2, PowerCause::Advisor, "mod_advisor_idea"),
        (3, PowerCause::Decision, "mod_decision"),
        (4, PowerCause::AbilityActivation, "mod_ability"),
        (5, PowerCause::LeaderTrait, "mod_trait"),
        (6, PowerCause::Focus, "mod_focus"),
        (7, PowerCause::Focus, "mod_focus_random"),
        (8, PowerCause::Focus, "mod_focus_conditional"),
        (9, PowerCause::Focus, "mod_focus_false"),
        (10, PowerCause::Focus, "mod_focus_override"),
        (11, PowerCause::Focus, "mod_focus_alternate"),
    ] {
        let mut row = event(sequence, cause, PowerOperation::Subtract, Some(-100_000));
        row.identity.item_key = token.as_bytes().to_vec();
        if cause == PowerCause::Advisor {
            row.identity.character = Some(9);
        }
        if matches!(
            cause,
            PowerCause::AbilityActivation | PowerCause::LeaderTrait
        ) {
            row.currency = PowerCurrency::Command;
        }
        rows.push(row);
    }
    put(&mut first, TableRows::CountryPowerChanges(rows));
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    for (name, pixel) in [
        "Mod law",
        "Mod advisor",
        "Mod decision",
        "Mod ability",
        "Country trait",
    ]
    .into_iter()
    .zip(pixels)
    {
        let item = reply
            .spend_items
            .iter()
            .find(|item| item.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name}: {:?}", reply.spend_items));
        assert_eq!(
            item_icon(&reply, item),
            Some(expected_png(pixel).as_str()),
            "{name}"
        );
        if name == "Mod advisor" {
            assert_eq!(item.unknown, None);
        }
    }
    assert!(
        reply
            .spend_items
            .iter()
            .any(|item| item.action == "Hired Mod advisor")
    );
    assert_eq!(reply.spending.len(), 11);
    let focus = reply
        .spend_items
        .iter()
        .find(|item| item.name.as_deref() == Some("Mod country/Mod country"))
        .unwrap();
    assert_eq!(
        focus.icon.as_deref(),
        Some(expected_png(focus_pixel).as_str())
    );
    let random_row = reply
        .spending
        .iter()
        .find(|row| row.sequence == "7")
        .unwrap();
    let random = reply
        .spend_items
        .iter()
        .find(|item| item.id == random_row.item_id)
        .unwrap();
    assert_eq!(random.name, None);
    assert!(
        random
            .unknown
            .as_deref()
            .unwrap()
            .contains("seed is not recorded")
    );
    assert_eq!(
        random.icon.as_deref(),
        Some(expected_png(focus_pixel).as_str())
    );
    for (sequence, pixel) in [
        ("8", focus_pixel),
        ("9", fallback_pixel),
        ("10", focus_pixel),
    ] {
        let row = reply
            .spending
            .iter()
            .find(|row| row.sequence == sequence)
            .unwrap();
        let item = reply
            .spend_items
            .iter()
            .find(|item| item.id == row.item_id)
            .unwrap();
        if sequence == "8" {
            // Selecting country context binds a fresh scope with seed 424242.
            assert_eq!(item.name.as_deref(), Some("Second choice"));
        }
        assert_eq!(
            item_icon(&reply, item),
            Some(expected_png(pixel).as_str()),
            "focus sequence {sequence}"
        );
    }
    let row = reply
        .spending
        .iter()
        .find(|row| row.sequence == "11")
        .unwrap();
    let alternate = reply
        .spend_items
        .iter()
        .find(|item| item.id == row.item_id)
        .unwrap();
    assert_eq!(alternate.icon, None);
    assert!(alternate.unknown.as_deref().unwrap().contains("alternate"));
    assert!(
        reply
            .spending
            .iter()
            .all(|row| row.country_name.as_deref() == Some("Mod country"))
    );
    let tooltips = reply.idea_tooltips.as_ref().unwrap();
    assert_eq!(tooltips.ideas.len(), 2);
    assert_eq!(tooltips.artwork.len(), 2);
    for (name, law, description) in [
        ("Mod law", true, "Law for Mod country"),
        ("Mod advisor", false, "Advisor for Mod country"),
    ] {
        let item = reply
            .spend_items
            .iter()
            .find(|item| item.name.as_deref() == Some(name))
            .unwrap();
        let token = item.tooltip_token.as_deref().unwrap();
        let info = tooltips
            .ideas
            .iter()
            .find(|info| info.token == token)
            .unwrap();
        let artwork = tooltips
            .artwork
            .iter()
            .find(|art| art.token == token)
            .unwrap();
        assert_eq!(info.law, law);
        assert_eq!(artwork.name, item.name);
        assert_eq!(item.icon, None, "tooltip artwork is shipped once");
        assert!(artwork.icon.is_some());
        assert_eq!(artwork.description.as_deref(), Some(description));
        let row = reply
            .spending
            .iter()
            .find(|row| row.item_id == item.id)
            .unwrap();
        assert_eq!(row.hour, 24);
    }
    assert!(
        reply
            .spend_items
            .iter()
            .any(|item| item.action == "Mod law")
    );
    let mut repeated = first.clone();
    repeated.hour = 25;
    put(
        &mut repeated,
        TableRows::PoliticsCaptureCoverage(
            (1..=2)
                .map(|country| PoliticsCaptureCoverageRow {
                    country,
                    instance: 1,
                    epoch: 1,
                    since_hour: 25,
                    through_hour: 25,
                    streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
                })
                .collect(),
        ),
    );
    let mut repeats = Vec::new();
    for (sequence, cause, token) in [
        (101, PowerCause::Idea, "mod_law"),
        (102, PowerCause::Advisor, "mod_advisor_idea"),
    ] {
        let mut row = event(sequence, cause, PowerOperation::Subtract, Some(-100_000));
        row.hour = 25;
        row.identity.item_key = token.as_bytes().to_vec();
        row.identity.action = PowerAction::Replace;
        if cause == PowerCause::Advisor {
            row.identity.character = Some(9);
        }
        repeats.push(row);
    }
    let mut other_country = repeats[0].clone();
    other_country.country = 2;
    other_country.sequence = 103;
    repeats.push(other_country);
    put(&mut repeated, TableRows::CountryPowerChanges(repeats));
    observe_all(&repeated, &mut scan, Some(&content)).unwrap();
    let repeated_reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let tooltips = repeated_reply.idea_tooltips.as_ref().unwrap();
    assert_eq!(
        tooltips.ideas.len(),
        2,
        "dated identities dedupe across action variants"
    );
    assert_eq!(tooltips.artwork.len(), 2);
    let other_reply = drawn(&content, present(&scan, 1, "C2", Some(&content), &AtomicBool::new(false)).unwrap());
    let other_tooltips = other_reply.idea_tooltips.as_ref().unwrap();
    assert_eq!(other_tooltips.ideas.len(), 1);
    assert!(
        other_tooltips
            .ideas
            .iter()
            .all(|other| tooltips.ideas.iter().all(|info| info.token != other.token))
    );
    assert!(
        repeated_reply
            .spend_items
            .iter()
            .filter(|item| item.tooltip_token.is_some())
            .all(|item| item.icon.is_none())
    );
    let mut late = frame(26, 2);
    let mut trait_row = event(
        12,
        PowerCause::LeaderTrait,
        PowerOperation::Subtract,
        Some(-100_000),
    );
    trait_row.hour = 23;
    trait_row.currency = PowerCurrency::Command;
    trait_row.identity.item_key = b"mod_trait".to_vec();
    let mut focus_row = event(
        13,
        PowerCause::Focus,
        PowerOperation::Subtract,
        Some(-100_000),
    );
    focus_row.hour = 23;
    focus_row.identity.item_key = b"mod_focus".to_vec();
    let mut static_focus = focus_row.clone();
    static_focus.sequence = 14;
    static_focus.identity.item_key = b"mod_focus_static".to_vec();
    put(
        &mut late,
        TableRows::CountryPowerChanges(vec![trait_row, focus_row, static_focus]),
    );
    observe_all(&late, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let row = reply
        .spending
        .iter()
        .find(|row| row.sequence == "12")
        .unwrap();
    let item = reply
        .spend_items
        .iter()
        .find(|item| item.id == row.item_id)
        .unwrap();
    assert_eq!(item.name, None);
    assert!(
        item.unknown
            .as_deref()
            .unwrap()
            .contains("country identity")
    );
    assert_eq!(item.icon.as_deref(), Some(expected_png(pixels[4]).as_str()));
    let row = reply
        .spending
        .iter()
        .find(|row| row.sequence == "13")
        .unwrap();
    let item = reply
        .spend_items
        .iter()
        .find(|item| item.id == row.item_id)
        .unwrap();
    assert_eq!(item.name, None);
    assert_eq!(item.icon, None);
    assert!(item.unknown.as_deref().unwrap().contains("country"));
    let row = reply
        .spending
        .iter()
        .find(|row| row.sequence == "14")
        .unwrap();
    let item = reply
        .spend_items
        .iter()
        .find(|item| item.id == row.item_id)
        .unwrap();
    assert_eq!(item.name.as_deref(), Some("Static mod focus"));
    assert_eq!(item.icon, None);
    assert!(item.unknown.as_deref().unwrap().contains("country"));
}

#[test]
fn renamed_target_is_named_at_each_spend_hour_and_old_rows_are_not_relabelled() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    stack.write(&stack.addon(), "localisation/english/targets_l_english.yml", "l_english:\n C1:0 \"Owner\"\n C2:0 \"Old target\"\n RENAMED:0 \"New target\"\n DIPLOMACY_IMPROVERELATION:0 \"Improve relations\"\n");
    let content = stack.content();
    let mut scan = Scanned::default();
    for (hour, cosmetic, sequence) in [(24, "", 1), (25, "RENAMED", 2)] {
        let mut frame = named_frame(hour, cosmetic);
        let mut row = event(
            sequence,
            PowerCause::Diplomacy,
            PowerOperation::Subtract,
            Some(-100_000),
        );
        row.hour = hour;
        row.identity.item_token = Some(0x2CD7);
        row.identity.target_country = Some(2);
        put(&mut frame, TableRows::CountryPowerChanges(vec![row]));
        observe_all(&frame, &mut scan, Some(&content)).unwrap();
    }
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    for (hour, sentence) in [
        (24, "Improve relations: Old target"),
        (25, "Improve relations: New target"),
    ] {
        let row = reply.spending.iter().find(|row| row.hour == hour).unwrap();
        let item = reply
            .spend_items
            .iter()
            .find(|item| item.id == row.item_id)
            .unwrap();
        assert_eq!(item.action, sentence);
        assert_eq!(item.target_tag.as_deref(), Some("C2"));
    }
}

#[test]
fn daily_cost_modifier_and_mios_use_recorded_identity_and_stack_localisation() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    stack.write(
        &stack.base(),
        "localisation/english/daily_l_english.yml",
        "l_english:\n C1:0 \"Base country\"\n daily_modifier_name:0 \"Base modifier\"\n",
    );
    stack.write(&stack.addon(), "localisation/english/daily_l_english.yml", "l_english:\n C1:0 \"Recorded country\"\n C2:0 \"Target\"\n daily_modifier_name:0 \"Daily cost $ARG$\"\n C1_mod_mio_name:0 \"Recorded organisation\"\n");
    let content = stack.content();
    let mut scan = Scanned::default();
    for hour in [24, 48] {
        let mut current = named_frame(hour, "");
        put(
            &mut current,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 94,
                literal: b"mod_mio".to_vec(),
            }]),
        );
        put(
            &mut current,
            TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
                index: 0x26,
                token: 95,
                name_key: b"daily_modifier_name".to_vec(),
                argument: b"recorded argument".to_vec(),
                ..Default::default()
            }]),
        );
        put(
            &mut current,
            TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
                organisation: 123,
                country: 1,
                owner: 1,
                token: 94,
                name: b"mod_mio_name".to_vec(),
                ..Default::default()
            }]),
        );
        put(
            &mut current,
            TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_gross_gain_raw: Some(100_000),
                pp_cost_raw: Some(60_000),
                pp_net_gain_raw: Some(40_000),
                pp_realized_delta_raw: Some(40_000),
                pp_sequence: Some(hour as u64),
                ..Default::default()
            }]),
        );
        let costs = [
            PowerDailyCostKind::PoliticalPowerCostModifier,
            PowerDailyCostKind::MioResearch,
            PowerDailyCostKind::MioProduction,
        ]
        .into_iter()
        .enumerate()
        .map(|(ordinal, component)| {
            let identity = if ordinal == 0 {
                PowerIdentity::default()
            } else {
                PowerIdentity {
                    object_kind: PowerObjectKind::Organisation,
                    object_reference: Some(123),
                    item_token: Some(94),
                    ..Default::default()
                }
            };
            CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_sequence: hour as u64,
                ordinal: ordinal as u32,
                component,
                cost_raw: (ordinal as i64 + 1) * 10_000,
                cause: PowerCause::DailyGain,
                site: PowerSite::AddPoliticalPowerNoDeficit,
                identity,
            }
        })
        .collect();
        put(&mut current, TableRows::CountryPowerDailyCosts(costs));
        observe_all(&current, &mut scan, Some(&content)).unwrap();
    }
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    assert_eq!(reply.spending.len(), 6);
    assert_eq!(
        reply.spend_items.len(),
        2,
        "same resolved item identity dedupes across daily reads and MIO components"
    );
    assert!(
        reply.spend_items.iter().all(|item| item.unknown.is_none()),
        "{:?}",
        reply.spend_items
    );
    assert!(
        reply
            .spend_items
            .iter()
            .any(|item| item.name.as_deref() == Some("Daily cost recorded argument"))
    );
    assert!(
        reply
            .spend_items
            .iter()
            .any(|item| item.name.as_deref() == Some("Recorded organisation"))
    );
    assert!(
        !reply
            .warnings
            .iter()
            .any(|warning| warning.contains("DailyGain item"))
    );
}

#[test]
fn mio_scripted_names_are_frozen_at_the_read_and_missing_identity_is_counted() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    stack.write(&stack.addon(), "localisation/english/mio_l_english.yml", "l_english:\n C1:0 \"First owner\"\n C2:0 \"Second owner\"\n mod_mio:0 \"[FROM.ReadPrefix] organisation\"\n prefix:0 \"[THIS.GetTag]\"\n");
    stack.write(
        &stack.addon(),
        "common/scripted_localisation/mio.txt",
        "defined_text = { name = ReadPrefix text = { localization_key = prefix } }",
    );
    let content = stack.content();
    let mut scan = Scanned::default();
    for (hour, owner) in [(24, Some(1)), (48, Some(2)), (72, None)] {
        let mut current = named_frame(hour, "");
        put(
            &mut current,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 94,
                literal: b"mod_mio".to_vec(),
            }]),
        );
        if let Some(owner) = owner {
            put(
                &mut current,
                TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
                    organisation: 123,
                    country: 1,
                    owner,
                    token: 94,
                    ..Default::default()
                }]),
            );
        } else {
            *current.tables[TableId::IndustrialOrganisations.index()] =
                replay_frame::frame::TableView::Unavailable {
                    reason: "synthetic MIO gap".into(),
                };
        }
        put(
            &mut current,
            TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_gross_gain_raw: Some(100_000),
                pp_cost_raw: Some(10_000),
                pp_net_gain_raw: Some(90_000),
                pp_realized_delta_raw: Some(90_000),
                pp_sequence: Some(hour as u64),
                ..Default::default()
            }]),
        );
        put(
            &mut current,
            TableRows::CountryPowerDailyCosts(vec![CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_sequence: hour as u64,
                ordinal: 0,
                component: PowerDailyCostKind::MioResearch,
                cost_raw: 10_000,
                cause: PowerCause::DailyGain,
                site: PowerSite::AddPoliticalPowerNoDeficit,
                identity: PowerIdentity {
                    object_kind: PowerObjectKind::Organisation,
                    object_reference: Some(123),
                    item_token: Some(94),
                    ..Default::default()
                },
            }]),
        );
        observe_all(&current, &mut scan, Some(&content)).unwrap();
    }
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    assert_eq!(reply.spending.len(), 3);
    for (hour, expected) in [
        (24, Some("C1 organisation")),
        (48, Some("C2 organisation")),
        (72, None),
    ] {
        let row = reply.spending.iter().find(|row| row.hour == hour).unwrap();
        let item = reply
            .spend_items
            .iter()
            .find(|item| item.id == row.item_id)
            .unwrap();
        assert_eq!(item.name.as_deref(), expected, "hour {hour}: {item:?}");
    }
    let reasons = reply
        .warnings
        .iter()
        .filter(|reason| reason.contains("IndustrialOrganisations"))
        .collect::<Vec<_>>();
    assert_eq!(reasons.len(), 1, "{:?}", reply.warnings);
    assert!(reasons[0].contains("(count: 1)"));
}

#[test]
fn mio_explicit_name_and_icon_survive_missing_lexical_token() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(
        &addon,
        "localisation/english/mio_override_l_english.yml",
        "l_english:\n C1:0 \"Owner\"\n C2:0 \"Other\"\n known_mio_name:0 \"Known organisation\"\n",
    );
    stack.write(&addon, "interface/mio_override.gfx", "spriteTypes = { spriteType = { name = GFX_explicit_mio texturefile = gfx/explicit_mio.png } spriteType = { name = GFX_idea_unknown texturefile = gfx/explicit_mio.png } }");
    let pixel = [40, 80, 160, 255];
    stack.png(&addon, "gfx/explicit_mio.png", pixel);
    let content = stack.content();
    let mut scan = Scanned::default();
    for (hour, icon) in [(24, "GFX_explicit_mio"), (48, "")] {
        let mut current = named_frame(hour, "");
        put(&mut current, TableRows::LexicalTokens(Vec::new()));
        put(
            &mut current,
            TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
                organisation: 123,
                country: 1,
                owner: 1,
                token: 94,
                name: b"known_mio_name".to_vec(),
                icon: icon.as_bytes().to_vec(),
                ..Default::default()
            }]),
        );
        put(
            &mut current,
            TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_gross_gain_raw: Some(100_000),
                pp_cost_raw: Some(10_000),
                pp_net_gain_raw: Some(90_000),
                pp_realized_delta_raw: Some(90_000),
                pp_sequence: Some(hour as u64),
                ..Default::default()
            }]),
        );
        put(
            &mut current,
            TableRows::CountryPowerDailyCosts(vec![CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: hour,
                pp_sequence: hour as u64,
                ordinal: 0,
                component: PowerDailyCostKind::MioResearch,
                cost_raw: 10_000,
                cause: PowerCause::DailyGain,
                site: PowerSite::AddPoliticalPowerNoDeficit,
                identity: PowerIdentity {
                    object_kind: PowerObjectKind::Organisation,
                    object_reference: Some(123),
                    item_token: Some(94),
                    ..Default::default()
                },
            }]),
        );
        observe_all(&current, &mut scan, Some(&content)).unwrap();
    }
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let item_at = |hour| {
        let row = reply.spending.iter().find(|row| row.hour == hour).unwrap();
        reply
            .spend_items
            .iter()
            .find(|item| item.id == row.item_id)
            .unwrap()
    };
    let explicit = item_at(24);
    assert_eq!(explicit.name.as_deref(), Some("Known organisation"));
    assert_eq!(
        item_icon(&reply, explicit),
        Some(expected_png(pixel).as_str())
    );
    assert_eq!(explicit.unknown, None);
    let fallback = item_at(48);
    assert_eq!(fallback.name.as_deref(), Some("Known organisation"));
    assert_eq!(item_icon(&reply, fallback), None);
    assert!(
        fallback
            .unknown
            .as_deref()
            .unwrap()
            .contains("LexicalTokens")
    );
    let reasons = reply
        .warnings
        .iter()
        .filter(|reason| reason.contains("LexicalTokens"))
        .collect::<Vec<_>>();
    assert_eq!(reasons.len(), 1, "{:?}", reply.warnings);
    assert!(reasons[0].contains("(count: 1)"));
}

#[test]
fn military_and_scientist_actions_select_native_recorded_portrait_roles() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(&addon, "localisation/english/people_l_english.yml", "l_english:\n C1:0 \"Owner\"\n C2:0 \"Target\"\n mod_general:0 \"Mod general\"\n mod_admiral:0 \"Mod admiral\"\n mod_scientist:0 \"Mod scientist\"\n");
    let pixels = [[9, 80, 200, 255], [80, 200, 9, 255], [200, 9, 80, 255]];
    for (path, pixel) in ["army", "navy", "scientist"].into_iter().zip(pixels) {
        stack.png(&addon, &format!("gfx/{path}.png"), pixel);
    }
    stack.png(&addon, "gfx/civilian.png", [250, 250, 250, 255]);
    let content = stack.content();
    let mut frame = named_frame(24, "");
    put(
        &mut frame,
        TableRows::Characters(
            ["mod_general", "mod_admiral", "mod_scientist"]
                .into_iter()
                .enumerate()
                .map(|(index, token)| CharacterRow {
                    id: 10 + index as u32,
                    country: 1,
                    token: token.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CharacterPortraits(
            [(10, 1, "army"), (11, 2, "navy"), (12, 4, "scientist")]
                .into_iter()
                .map(|(character, category, path)| CharacterPortraitRow {
                    character,
                    portraits: vec![
                        CharacterPortrait {
                            category: 0,
                            size: 1,
                            is_set: 1,
                            kind: 1,
                            identity: b"gfx/civilian.png".to_vec(),
                        },
                        CharacterPortrait {
                            category,
                            size: 1,
                            is_set: 1,
                            kind: 1,
                            identity: format!("gfx/{path}.png").into_bytes(),
                        },
                    ],
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::UnitLeaders(vec![
            UnitLeaderRow {
                id: 10,
                reference_type: 37,
                character_id: 10,
                category: 1,
                ..Default::default()
            },
            UnitLeaderRow {
                id: 11,
                reference_type: 37,
                character_id: 11,
                category: 2,
                ..Default::default()
            },
        ]),
    );
    let mut rows = Vec::new();
    for (sequence, id, cause) in [
        (1, 10, PowerCause::LeaderPromotion),
        (2, 11, PowerCause::LeaderAssignment),
        (3, 12, PowerCause::ScientistRecruitment),
    ] {
        let mut row = event(sequence, cause, PowerOperation::Subtract, Some(-100_000));
        row.identity.character = Some(id);
        if cause != PowerCause::ScientistRecruitment {
            row.identity.object_kind = PowerObjectKind::Leader;
            row.identity.object_reference = Some((u64::from(id) << 32) | 37);
        }
        rows.push(row);
    }
    put(&mut frame, TableRows::CountryPowerChanges(rows));
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    for (name, pixel) in ["Mod general", "Mod admiral", "Mod scientist"]
        .into_iter()
        .zip(pixels)
    {
        let item = reply
            .spend_items
            .iter()
            .find(|item| item.name.as_deref() == Some(name))
            .unwrap();
        assert_eq!(
            item_icon(&reply, item),
            Some(expected_png(pixel).as_str()),
            "{name}"
        );
    }
}

/// Preparation names a holder from recorded facts and keeps its portrait as a recipe, drawing
/// nothing: a portrait the prepared content cannot draw leaves no note on the prepared item, so
/// content that supplies it draws it with nothing named, and content that does not names why.
#[test]
fn preparation_keeps_a_portrait_it_cannot_draw_as_a_recipe() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(&addon, "localisation/english/people_l_english.yml", "l_english:
 C1:0 \"Owner\"
 C2:0 \"Target\"
 mod_scientist:0 \"Mod scientist\"
");
    // The portrait's texture is not there when the ledger is prepared.
    let prepared_with = stack.content();
    let mut frame = named_frame(24, "");
    put(
        &mut frame,
        TableRows::Characters(vec![CharacterRow {
            id: 12,
            country: 1,
            token: b"mod_scientist".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CharacterPortraits(vec![CharacterPortraitRow {
            character: 12,
            portraits: vec![CharacterPortrait {
                category: 4,
                size: 1,
                is_set: 1,
                kind: 1,
                identity: b"gfx/scientist.png".to_vec(),
            }],
        }]),
    );
    let mut row = event(1, PowerCause::ScientistRecruitment, PowerOperation::Subtract, Some(-100_000));
    row.identity.character = Some(12);
    put(&mut frame, TableRows::CountryPowerChanges(vec![row]));
    let mut scan = Scanned::default();
    observe_all(&frame, &mut scan, Some(&prepared_with)).unwrap();
    let prepared =
        present(&scan, 1, "C1", Some(&prepared_with), &AtomicBool::new(false)).unwrap();
    let item = |reply: &PoliticsLedgerReply| {
        reply
            .spend_items
            .iter()
            .find(|item| item.name.as_deref() == Some("Mod scientist"))
            .unwrap()
            .clone()
    };
    assert_eq!(item(&prepared).unknown, None, "nothing is drawn, nor named, when prepared");
    // Content that cannot draw it names why when the reply is served, counted in its warnings.
    let missing = drawn(&prepared_with, prepared.clone());
    assert!(
        item(&missing).unknown.is_some_and(|why| why.contains("character 12 portrait")),
        "{:?}",
        item(&missing)
    );
    assert!(
        missing
            .warnings
            .iter()
            .any(|line| line.contains("character 12 portrait") && line.ends_with("(count: 1)")),
        "{:?}",
        missing.warnings
    );
    // Content that supplies it draws it, with nothing named.
    let pixel = [200, 9, 80, 255];
    stack.png(&addon, "gfx/scientist.png", pixel);
    let supplied = drawn(&stack.content(), prepared);
    assert_eq!(item(&supplied).unknown, None);
    assert_eq!(
        item_icon(&supplied, &item(&supplied)),
        Some(expected_png(pixel).as_str())
    );
}

#[test]
fn decision_icons_use_native_default_links_and_keep_known_art_when_name_is_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/decisions/categories/icons.txt",
        "mod_category = { }",
    );
    stack.write(&addon, "common/decisions/icons.txt", "mod_category = { mod_scope_icon = { icon = { key = chosen trigger = { FROM = { always = yes } PREV = { always = yes } } } icon = fallback } mod_unknown_name = { } mod_single_unknown = { icon = { key = chosen trigger = { always = no } } } }");
    stack.write(
        &addon,
        "localisation/english/icons_l_english.yml",
        "l_english:\n C1:0 \"Owner\"\n C2:0 \"Target\"\n mod_scope_icon:0 \"Scope icon\"\n",
    );
    stack.write(&addon, "interface/icons.gfx", "spriteTypes = { spriteType = { name = GFX_decision_chosen texturefile = gfx/chosen.png } spriteType = { name = GFX_decision_fallback texturefile = gfx/fallback.png } spriteType = { name = GFX_decision_mod_unknown_name texturefile = gfx/chosen.png } }");
    let chosen = [40, 180, 90, 255];
    stack.png(&addon, "gfx/chosen.png", chosen);
    stack.png(&addon, "gfx/fallback.png", [180, 40, 90, 255]);
    let content = stack.content();
    let mut scan = Scanned::default();
    let mut first = named_frame(24, "");
    let mut ordinary = event(
        1,
        PowerCause::Decision,
        PowerOperation::Subtract,
        Some(-100_000),
    );
    ordinary.identity.item_key = b"mod_scope_icon".to_vec();
    let mut targeted = ordinary.clone();
    targeted.sequence = 2;
    targeted.cause = PowerCause::TargetedDecision;
    targeted.identity.target_country = Some(2);
    put(
        &mut first,
        TableRows::CountryPowerChanges(vec![ordinary, targeted]),
    );
    observe_all(&first, &mut scan, Some(&content)).unwrap();
    let mut missing = frame(25, 2);
    let mut row = event(
        3,
        PowerCause::Decision,
        PowerOperation::Subtract,
        Some(-100_000),
    );
    row.hour = 25;
    row.identity.item_key = b"mod_unknown_name".to_vec();
    let mut no_owner = row.clone();
    no_owner.sequence = 4;
    no_owner.hour = 23;
    let mut single_no_owner = no_owner.clone();
    single_no_owner.sequence = 5;
    single_no_owner.identity.item_key = b"mod_single_unknown".to_vec();
    // The event predates every captured naming/frame sample. Own-token and
    // singleton art stay known; neither native path reads an owner scope.
    put(
        &mut missing,
        TableRows::CountryPowerChanges(vec![row, no_owner, single_no_owner]),
    );
    observe_all(&missing, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    assert_eq!(reply.spending.len(), 5);
    for item in &reply.spend_items {
        assert_eq!(
            item_icon(&reply, item),
            Some(expected_png(chosen).as_str()),
            "{item:?}"
        );
    }
    let unknown_row = reply
        .spending
        .iter()
        .find(|row| row.sequence == "3")
        .unwrap();
    let unknown = reply
        .spend_items
        .iter()
        .find(|item| item.id == unknown_row.item_id)
        .unwrap();
    assert!(
        unknown
            .unknown
            .as_deref()
            .unwrap()
            .contains("CountryAppearance")
    );
}

#[test]
fn daily_relation_title_and_missing_direct_charge_do_not_borrow_action_token_branches() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    stack.write(&stack.addon(), "localisation/english/relation_l_english.yml", "l_english:\n C1:0 \"Owner\"\n C2:0 \"Target\"\n DIPLOMACY_IMPROVERELATION:0 \"Improve relations action\"\n DIPLOMACY_IMPROVERELATION_TITLE:0 \"Improve relations upkeep\"\n");
    let content = stack.content();
    let mut first = named_frame(24, "");
    let mut missing = event(1, PowerCause::Diplomacy, PowerOperation::Subtract, None);
    missing.identity.item_token = Some(0x2CD7);
    missing.identity.target_country = Some(2);
    let daily_gain = event(2, PowerCause::DailyGain, PowerOperation::Add, Some(200_000));
    put(
        &mut first,
        TableRows::CountryPowerChanges(vec![missing, daily_gain]),
    );
    put(
        &mut first,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: Some(2),
            pp_cost_raw: Some(100_000),
            ..Default::default()
        }]),
    );
    let mut relation = CountryPowerDailyCostRow {
        country: 1,
        instance: 1,
        epoch: 1,
        day_hour: 24,
        pp_sequence: 2,
        cost_raw: 100_000,
        component: PowerDailyCostKind::Relation,
        cause: PowerCause::Diplomacy,
        ..Default::default()
    };
    // A recorded relation type ID is independent of the diplomatic action token.
    relation.identity.item_token = Some(7);
    relation.identity.target_country = Some(2);
    put(
        &mut first,
        TableRows::CountryPowerDailyCosts(vec![relation]),
    );
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let direct = reply
        .spending
        .iter()
        .find(|row| !row.id.starts_with("daily:"))
        .unwrap();
    let item = reply
        .spend_items
        .iter()
        .find(|item| item.id == direct.item_id)
        .unwrap();
    assert_eq!(item.name, None);
    assert_eq!(direct.charge_raw, None);
    let daily = reply
        .spending
        .iter()
        .find(|row| row.id.starts_with("daily:"))
        .unwrap();
    let item = reply
        .spend_items
        .iter()
        .find(|item| item.id == daily.item_id)
        .unwrap();
    assert_eq!(item.action, "Improve relations upkeep: Target");
}

#[test]
fn decision_scripted_names_use_known_scopes_and_native_random_fast_paths_without_guessing_seed() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/decisions/categories/names.txt",
        "mod_category = { }",
    );
    stack.write(
        &addon,
        "common/decisions/names.txt",
        "mod_category = { mod_root = { } mod_target = { } mod_single = { } mod_random = { } mod_promoted = { } }",
    );
    stack.write(&addon, "common/scripted_localisation/names.txt", "defined_text = { name = SingleName text = { random_list = { 0 = { localization_key = SINGLE } } } } defined_text = { name = RandomName text = { random_list = { 1 = { localization_key = FIRST } 1 = { localization_key = SECOND } } } }");
    stack.write(&addon, "localisation/english/names_l_english.yml", "l_english:\n C1:0 \"Owner\"\n C2:0 \"Target\"\n mod_root:0 \"[ROOT.GetName]/[FROM.GetName]\"\n mod_target:0 \"[ROOT.GetName]/[FROM.GetName]/[FROM.FROM.GetName]\"\n mod_single:0 \"[SingleName]\"\n mod_random:0 \"[RandomName]\"\n mod_promoted:0 \"[ROOT.RandomName]\"\n SINGLE:0 \"Only choice\"\n FIRST:0 \"First choice\"\n SECOND:0 \"Second choice\"\n");
    stack.write(&addon, "interface/names.gfx", "spriteTypes = { spriteType = { name = GFX_decision_mod_random texturefile = gfx/chosen.png } }");
    let chosen = [30, 60, 180, 255];
    stack.png(&addon, "gfx/chosen.png", chosen);
    let content = stack.content();
    let mut first = named_frame(24, "");
    let mut rows = Vec::new();
    for (sequence, token, cause) in [
        (1, "mod_root", PowerCause::Decision),
        (2, "mod_target", PowerCause::TargetedDecision),
        (3, "mod_single", PowerCause::Decision),
        (4, "mod_random", PowerCause::Decision),
        (5, "mod_promoted", PowerCause::Decision),
    ] {
        let mut row = event(sequence, cause, PowerOperation::Subtract, Some(-100_000));
        row.identity.item_key = token.as_bytes().to_vec();
        if cause == PowerCause::TargetedDecision {
            row.identity.target_country = Some(2);
        }
        rows.push(row);
    }
    put(&mut first, TableRows::CountryPowerChanges(rows));
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let item_at = |sequence: &str| {
        let row = reply
            .spending
            .iter()
            .find(|row| row.sequence == sequence)
            .unwrap();
        reply
            .spend_items
            .iter()
            .find(|item| item.id == row.item_id)
            .unwrap()
    };
    assert_eq!(item_at("1").name.as_deref(), Some("Owner/Owner"));
    assert_eq!(item_at("2").name.as_deref(), Some("Owner/Target/Target"));
    assert_eq!(item_at("3").name.as_deref(), Some("Only choice"));
    // Explicit country promotion gets a fresh seed, while the unpromoted
    // member below retains the caller's unrecorded seed.
    assert_eq!(item_at("5").name.as_deref(), Some("Second choice"));
    let unknown = item_at("4");
    assert_eq!(unknown.name, None);
    assert!(
        unknown
            .unknown
            .as_deref()
            .unwrap()
            .contains("seed is not recorded")
    );
    assert_eq!(unknown.icon.as_deref(), Some(expected_png(chosen).as_str()));
}

#[test]
fn decision_name_from_uses_native_added_trigger_children_and_targets_presence() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/decisions/categories/selector.txt",
        "mod_category = { }",
    );
    stack.write(&addon, "common/decisions/selector.txt", "mod_category = { mod_ignored = { highlight_states = { highlight_states_trigger = { fixture_unregistered = yes } } } mod_trigger = { highlight_states = { highlight_states_trigger = { always = yes } } } mod_targets = { highlight_states = { highlight_state_targets = { targets = { C2 } } } } }");
    stack.write(&addon, "localisation/english/selector_l_english.yml", "l_english:\n C1:0 \"Owner\"\n C2:0 \"Target\"\n mod_ignored:0 \"[FROM.GetName]\"\n mod_trigger:0 \"[FROM.GetName]\"\n mod_targets:0 \"[FROM.GetName]\"\n");
    let content = stack.content();
    let mut first = named_frame(24, "");
    put(&mut first, TableRows::LexicalTokens(Vec::new()));
    let power = content.power_content().unwrap();
    let world = event_world(&content, &first).unwrap();
    for (token, expected) in [
        ("mod_ignored", false),
        ("mod_trigger", true),
        ("mod_targets", true),
    ] {
        let decision = power.decisions.decision(token.as_bytes()).unwrap().unwrap();
        assert_eq!(
            decision.name_selects_state(|body| world.has_children(body)),
            Ok(expected),
            "{token}"
        );
    }
    let rows = ["mod_ignored", "mod_trigger", "mod_targets"]
        .into_iter()
        .enumerate()
        .map(|(index, token)| {
            let mut row = event(
                index as u64 + 1,
                PowerCause::Decision,
                PowerOperation::Subtract,
                Some(-100_000),
            );
            row.identity.item_key = token.as_bytes().to_vec();
            row
        })
        .collect();
    put(&mut first, TableRows::CountryPowerChanges(rows));
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let item_at = |sequence: &str| {
        let row = reply
            .spending
            .iter()
            .find(|row| row.sequence == sequence)
            .unwrap();
        reply
            .spend_items
            .iter()
            .find(|item| item.id == row.item_id)
            .unwrap()
    };
    assert_eq!(item_at("1").name.as_deref(), Some("Owner"));
    for sequence in ["2", "3"] {
        assert_eq!(item_at(sequence).name, None);
        assert!(
            item_at(sequence)
                .unknown
                .as_deref()
                .unwrap()
                .contains("FROM")
        );
    }
}

#[test]
fn targeted_decision_state_selector_uses_recorded_state_and_fresh_nested_seed() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(
        &addon,
        "common/decisions/categories/state.txt",
        "mod_category = { }",
    );
    stack.write(&addon, "common/decisions/state.txt", "mod_category = { mod_state = { highlight_states = { highlight_states_trigger = { always = yes } } } }");
    stack.write(&addon, "common/scripted_localisation/state.txt", "defined_text = { name = StateChoice text = { random_list = { 0 = { localization_key = FIRST } 1 = { localization_key = SECOND } } } }");
    stack.write(&addon, "localisation/english/state_l_english.yml", "l_english:\n C1:0 \"Owner\"\n C2:0 \"Target\"\n mod_state:0 \"[FROM.GetName]/[FROM.FROM.GetName]/[FROM.FROM.StateChoice]\"\n FIRST:0 \"First choice\"\n SECOND:0 \"Second choice\"\n");
    let content = stack.content();
    let mut first = named_frame(24, "");
    put(
        &mut first,
        TableRows::States(vec![StateRow {
            id: 17,
            owner: 1,
            controller: 1,
            custom_name: b"Recorded state".to_vec(),
            ..Default::default()
        }]),
    );
    let mut row = event(
        1,
        PowerCause::TargetedDecision,
        PowerOperation::Subtract,
        Some(-100_000),
    );
    row.identity.item_key = b"mod_state".to_vec();
    row.identity.target_state = Some(17);
    put(&mut first, TableRows::CountryPowerChanges(vec![row]));
    let mut scan = Scanned::default();
    observe_all(&first, &mut scan, Some(&content)).unwrap();
    let reply = drawn(&content, present(&scan, 1, "C1", Some(&content), &AtomicBool::new(false)).unwrap());
    let item = &reply.spend_items[0];
    assert_eq!(
        item.name.as_deref(),
        Some("Recorded state/Recorded state/Second choice")
    );
    assert_eq!(item.unknown, None);
}

/// Read-only installed-content audit. Explicit paths keep private inputs local.
#[test]
#[ignore = "requires installed game and Workshop corpus paths"]
fn spending_native_content_corpus_audit() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let game =
        PathBuf::from(std::env::var_os("BATTLE_PLAN_POL_SPENDING_GAME").or_else(|| std::env::var_os("HOI4_POL_SPENDING_GAME")).expect("BATTLE_PLAN_POL_SPENDING_GAME"));
    let workshop =
        PathBuf::from(std::env::var_os("BATTLE_PLAN_POL_SPENDING_MODS").or_else(|| std::env::var_os("HOI4_POL_SPENDING_MODS")).expect("BATTLE_PLAN_POL_SPENDING_MODS"));
    let scratch = Stack::new();
    let (candidates, unusable) =
        installed_stacks::candidates(&scratch.root, Some(&workshop)).unwrap();
    eprintln!("Installed Workshop descriptors: {}", candidates.len());
    for refused in unusable {
        eprintln!(
            "REFUSAL descriptor {}: {} (count: 1)",
            refused.path, refused.why
        );
    }
    let plans = installed_stacks::plans(&candidates);
    let only_root = std::env::var("BATTLE_PLAN_POL_SPENDING_ROOT").or_else(|_| std::env::var("HOI4_POL_SPENDING_ROOT")).ok();
    if let Some(root) = only_root.as_deref()
        && root != "vanilla"
        && !plans.iter().any(|plan| plan.id == root)
    {
        eprintln!("REFUSAL audit root {root}: no installed stack (count: 1)");
        panic!("requested audit root {root} has no installed stack");
    }
    let cache = std::env::var_os("BATTLE_PLAN_POL_SPENDING_CACHE").or_else(|| std::env::var_os("HOI4_POL_SPENDING_CACHE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| scratch.root.join("hashes"));
    for plan in std::iter::once(None).chain(plans.iter().map(Some)) {
        let root_name = plan.map_or("vanilla", |plan| plan.id.as_str());
        if only_root.as_deref().is_some_and(|root| root != root_name) {
            continue;
        }
        let selected = plan.map_or(&[][..], |plan| plan.mods.as_slice());
        if let Some(plan) = plan {
            eprintln!("PLAN {root_name}: {}", plan.name);
            for skipped in &plan.skipped {
                eprintln!("REFUSAL {root_name} dependency: {skipped} (count: 1)");
            }
        }
        let user = installed_stacks::user_folder(
            &scratch.root.join(format!("user_{root_name}")),
            "l_english",
            &candidates,
            selected,
        )
        .unwrap();
        eprintln!(
            "USER {root_name}: language={}",
            installed_stacks::language(&user).unwrap()
        );
        let gathered = installed_content_core::content::gather(&installed_content_core::GatherOptions {
            game_dir: game.clone(),
            user_dir: user,
            cache_dir: cache.clone(),
            dlc_order: None,
        })
        .unwrap();
        eprintln!(
            "MOUNTS {root_name}: dlcs={} mods={} order_note={:?}",
            gathered.dlc.len(),
            gathered.mods.len(),
            gathered.order_note
        );
        let layers = gathered.layers;
        let decisions = installed_content_core::decisions::read(&layers).unwrap();
        let icons: usize = decisions
            .entries
            .values()
            .map(|decision| decision.icons.len())
            .sum();
        let conditional: usize = decisions
            .entries
            .values()
            .flat_map(|decision| &decision.icons)
            .filter(|icon| !icon.trigger.is_empty())
            .count();
        eprintln!(
            "DECISIONS {root_name}: definitions={} icons={icons} conditional={conditional}",
            decisions.entries.len()
        );
        for reason in decisions.notes {
            eprintln!("REFUSAL {root_name}: {reason} (count: 1)");
        }
        match installed_content_abilities::abilities::read(&layers) {
            Ok(abilities) => {
                eprintln!(
                    "ABILITIES {root_name}: names={} static_icons={} animated_icons={} warnings={}",
                    abilities
                        .definitions
                        .keys()
                        .filter(|token| abilities.name_key(token).is_some())
                        .count(),
                    abilities
                        .by_token
                        .keys()
                        .filter(|token| abilities.static_sprite(token).is_some())
                        .count(),
                    abilities.by_token.len(),
                    abilities.warnings.len()
                );
                for (kind, warning) in abilities.warnings {
                    eprintln!("{kind:?} {root_name}: {warning}");
                }
            }
            Err(reason) => eprintln!("REFUSAL {root_name} abilities: {reason} (count: 1)"),
        }
        let traits = installed_content_core::unit_leader::Traits::registered(&layers).unwrap();
        eprintln!(
            "TRAITS {root_name}: registered={}",
            traits.spellings().count()
        );
        let (focuses, warnings) = installed_content_focus::focus::read(&layers).unwrap();
        let conditional = focuses
            .known()
            .filter(|focus| focus.icons.cases.iter().any(|case| case.trigger.is_some()))
            .count();
        let alternate = focuses
            .known()
            .filter(|focus| focus.alternate_icon_present)
            .count();
        eprintln!(
            "FOCUSES {root_name}: definitions={} conditional_icons={conditional} alternate_icons={alternate} warnings={}",
            focuses.ids().count(),
            warnings.len()
        );
        for (kind, warning) in warnings {
            eprintln!("{kind:?} {root_name}: {warning}");
        }
    }
}

#[test]
fn power_content_reader_notes_reach_counted_ledger_diagnostics() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let stack = Stack::new();
    let addon = stack.addon();
    stack.write(
        &addon,
        "localisation/english/notes_l_english.yml",
        "l_english:\n C1:0 \"Owner\"\n",
    );
    stack.write(
        &addon,
        "common/decisions/notes.txt",
        "absent_category = { omitted_decision = { } }",
    );
    stack.write(
        &addon,
        "common/national_focus/notes.txt",
        "focus_tree = { focus = { id = noted_focus cost = { x = 1 } } }",
    );
    stack.write(
        &addon,
        "common/abilities/notes.txt",
        "ability = { noted_ability = { duration = { x = 1 } } }",
    );
    let content = stack.content();
    let mut scan = Scanned::default();
    observe_all(&frame(24, 1), &mut scan, None).unwrap();
    let reply = present_view(
        &scan,
        1,
        "C1",
        Some(&content),
        &AtomicBool::new(false),
        LedgerView::Balance,
    )
    .unwrap();
    for (path, reason) in [
        (
            "common/decisions/notes.txt",
            "decision omitted_decision ignored: category absent_category is not registered",
        ),
        ("common/national_focus/notes.txt", "cost"),
        ("common/abilities/notes.txt", "duration"),
    ] {
        assert!(
            reply.warnings.iter().any(|warning| warning.contains(path)
                && warning.contains(reason)
                && warning.contains("count:")),
            "{path}: {:?}",
            reply.warnings
        );
    }
}

#[test]
fn frame_texts_share_same_hour_inputs_but_keep_each_country_root() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stack = Stack::new();
    stack.write(
        &stack.base(),
        "localisation/countries_l_english.yml",
        r#"l_english:
 C1:0 "First"
 C2:0 "Second"
"#,
    );
    let content = stack.content();
    let frame = named_frame(24, "");
    let texts = FrameTexts::default();
    let first = texts.scripted(&content, &frame, 1).unwrap();
    let second = texts.scripted(&content, &frame, 2).unwrap();
    assert!(!std::rc::Rc::ptr_eq(&first, &second));
    assert!(std::rc::Rc::ptr_eq(
        &first,
        &texts.scripted(&content, &frame, 1).unwrap()
    ));
    assert!(std::rc::Rc::ptr_eq(
        &second,
        &texts.scripted(&content, &frame, 2).unwrap()
    ));
    assert_eq!(
        first.expand("[ROOT.GetTag] / [ROOT.GetName]"),
        Ok("C1 / First".into())
    );
    assert_eq!(
        second.expand("[ROOT.GetTag] / [ROOT.GetName]"),
        Ok("C2 / Second".into())
    );
    let later = named_frame(25, "");
    let later_texts = FrameTexts::default();
    let later_second = later_texts.scripted(&content, &later, 2).unwrap();
    assert!(!std::rc::Rc::ptr_eq(&second, &later_second));
    assert_eq!(
        later_second.expand("[ROOT.GetTag] / [ROOT.GetName]"),
        Ok("C2 / Second".into())
    );
}
