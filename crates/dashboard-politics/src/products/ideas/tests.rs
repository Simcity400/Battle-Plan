use super::{HoldingEnd, HoldingStart, IdeaCharacter, IdeaSpan, read};
use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source}};
use replay::campaign::Replay;
use replay_data::rows::{ActiveIdeaRow, CharacterRow, HiredAdvisorRow, LexicalTokenRow, TableId, TableRows};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::{Arc, atomic::AtomicBool};

const FIRST: i32 = 100;
const LAST: i32 = 130;
const GER: u32 = 9001;

#[test]
fn cached_idea_looks_keep_overlay_notes() {
    use dashboard_base::{catalogue, products::politics_content::politics_content, test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select}};

    let machine = Machine::new("idea_look_art_notes");
    machine.write("common/defines/fixture.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write("interface/fixture.gfx", b"spriteTypes={spriteType={name=GFX_idea_fixture_idea textureFile=gfx/fixture_idea.png}}");
    machine.write("gfx/fixture_idea.png", &png(RED));
    machine.write_mod("fixture_art", "gfx/fixture_idea.png", &png(BLUE));
    let (source, mut options) = machine.open();
    select(&source, &mut options, &["fixture_art"]);
    let content = politics_content(&source, &options).unwrap();
    let read = super::Read {
        ideas: super::Ideas { spans: vec![span("fixture_idea", 100, 100, None)], ..Default::default() },
        naming: Vec::new(),
        looks: std::sync::Mutex::new((std::sync::Weak::new(), Vec::new())),
        dates: std::sync::Arc::default(),
        variables: Arc::new(super::KeyedVariables::new(source.replay())),
        lexical: super::Lexical::for_source(&source),
        companions: super::CompanionTables::for_source(&source),
        presentation: Default::default(),
    };
    let reply = || {
        let mut own = catalogue::OwnRefusals::default();
        let shown = catalogue::own_refusals(&mut own, || read.looked(&content, "GER", 0));
        let mut warnings = Vec::new();
        own.diagnostics().append_to(&mut warnings);
        (first_pixel(shown.0.artwork.icon.as_deref().unwrap()), warnings)
    };
    let first = reply();
    assert_eq!(first.0, BLUE);
    assert!(first.1.iter().any(|line| line.starts_with("Artwork from visual mods") && line.contains("fixture_art")), "{first:?}");
    assert_eq!(reply(), first);
}

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    frame.tables[table.index()] = TableView::Complete(rows).into();
    frame.metrics.table_ns[table.index()] = 1;
}

fn frame(hour: i32, truncated: Option<i32>) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1_000));
    if hour == 115 {
        set(&mut frame, TableId::Countries, TableRows::Countries(Vec::new()));
    }
    let token = |text: &str| text.as_bytes().to_vec();
    let ideas = vec![
        ActiveIdeaRow { country: GER, ordinal: 0, token: token("civilian_economy") },
        ActiveIdeaRow { country: GER, ordinal: 1, token: token(if hour < 110 { "export_focus" } else if hour < 115 { "free_trade" } else { "harvest_pact" }) },
    ];
    if Some(hour) == truncated {
        *frame.tables[TableId::ActiveIdeas.index()] = TableView::Truncated { rows: TableRows::ActiveIdeas(ideas[..1].to_vec()), missing: 1 };
        frame.metrics.table_ns[TableId::ActiveIdeas.index()] = 1;
    } else {
        set(&mut frame, TableId::ActiveIdeas, TableRows::ActiveIdeas(ideas));
    }
    let mut advisors = if hour < 105 {
        Vec::new()
    } else {
        vec![HiredAdvisorRow { country: GER, ordinal: 0, character: if hour < 120 { 501 } else { 502 }, idea: token("adv_a"), slot: token("political_advisor") }]
    };
    if hour >= 125 {
        advisors.push(HiredAdvisorRow { country: GER, ordinal: 1, character: 503, idea: Vec::new(), slot: token("political_advisor") });
    }
    set(&mut frame, TableId::HiredAdvisors, TableRows::HiredAdvisors(advisors));
    let characters = vec![
        CharacterRow { id: 501, name_token_id: 7001, token: token("Saved A"), ..Default::default() },
        CharacterRow { id: 502, name_token_id: -1, token: token("Saved B"), ..Default::default() },
    ];
    set(&mut frame, TableId::Characters, TableRows::Characters(characters));
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 7001, literal: token("CHAR_A") }]));
    frame
}

fn recording() -> Source {
    recording_with(None)
}

fn recording_with(truncated: Option<i32>) -> Source {
    let frames = (FIRST..=LAST).map(|hour| frame(hour, truncated)).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "ideas".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn span(token: &str, first: i32, last: i32, advisor: Option<(u32, &str, &str)>) -> IdeaSpan {
    IdeaSpan {
        token: token.into(),
        first_hour: first,
        last_hour: last,
        slot: advisor.map(|_| "political_advisor".to_owned()),
        character: advisor.map(|(id, token, name)| IdeaCharacter { id, token: token.into(), name: name.into() }),
    }
}

#[test]
fn a_countrys_ideas_are_spans_of_held_tokens_and_an_advisor_span_keeps_its_seat_and_character() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let ideas = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    let a = Some((501, "CHAR_A", "Saved A"));
    let b = Some((502, "", "Saved B"));
    assert_eq!(
        ideas.spans,
        [
            span("civilian_economy", 100, 114, None),
            span("export_focus", 100, 109, None),
            span("adv_a", 105, 114, a),
            span("free_trade", 110, 114, None),
            span("adv_a", 116, 119, a),
            span("civilian_economy", 116, LAST, None),
            span("harvest_pact", 116, LAST, None),
            span("adv_a", 120, LAST, b),
        ],
        "spans never bridge an hour no country held the tag; another character in the seat is a new span"
    );
    assert_eq!(ideas.unobserved.iter().map(|span| (span.first_hour, span.last_hour)).collect::<Vec<_>>(), [(115, 115)]);
    assert_eq!(ideas.identity_starts, [100, 116]);
    let template = ideas.templates.get(&7).unwrap();
    assert!(template.as_ref().is_err_and(|reason| reason.contains("holds no template")), "{template:?}");
}

#[test]
fn identity_starts_are_named_through_one_seeker_as_each_hour_alone_names_them() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let read = super::history(&source, 1, &"GER".into()).unwrap();
    assert_eq!(read.ideas.identity_starts, [100, 116]);
    for (named, &hour) in read.naming.iter().zip(&read.ideas.identity_starts) {
        let mut alone = dashboard_base::products::at_hour::Frames::new(&query).unwrap();
        assert_eq!(*named, super::naming_at(&mut alone, hour, GER).unwrap(), "hour {hour}: a seeker moved on from the start before names it as a fresh read does");
    }
}

#[test]
fn the_recorded_definitions_are_read_once_for_every_reply_on_a_source() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let first = super::recorded_definitions(&source, &query).unwrap();
    let again = super::recorded_definitions(&source, &dashboard_base::queries::query(&source, 1).unwrap()).unwrap();
    assert!(Arc::ptr_eq(&first, &again), "the idea history and each power spending chunk share one read");
    let other = recording();
    let elsewhere = super::recorded_definitions(&other, &dashboard_base::queries::query(&other, 1).unwrap()).unwrap();
    assert!(!Arc::ptr_eq(&first, &elsewhere), "kept on its source, never shared with another recording's");
}

#[test]
fn a_truncated_inventory_is_unread_so_what_it_omits_is_never_ended() {
    const TRUNCATED: i32 = 128;
    let source = recording_with(Some(TRUNCATED));
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let ideas = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    assert!(ideas.unread_at(TRUNCATED));
    let held = ideas.at(LAST);
    let pact: Vec<_> = held.iter().filter(|holding| ideas.spans[holding.span].token == "harvest_pact").map(|holding| (holding.first_hour, holding.last_hour, holding.start, holding.end)).collect();
    assert_eq!(pact, [(116, LAST, HoldingStart::FirstObserved, HoldingEnd::Held)], "the spirit the truncated hour omitted is held across it, not ended and begun again");
}

#[test]
fn a_spirit_held_by_a_new_holder_from_the_next_hour_is_a_new_holding() {
    let ideas = super::Ideas {
        spans: vec![span("spirit", 100, 109, None), span("spirit", 110, 120, None)],
        identity_starts: vec![100, 110],
        starts: vec![(1, 7, 70), (1, 8, 80)],
        ..Default::default()
    };
    let held = ideas.at(115);
    assert_eq!(
        held.iter().map(|holding| (holding.span, holding.first_hour, holding.start)).collect::<Vec<_>>(),
        [(0, 100, HoldingStart::HeldBefore), (1, 110, HoldingStart::HeldBefore)],
        "the next holder's spirit never inherits the previous holder's acquisition"
    );
}

#[test]
fn at_an_hour_a_holding_is_named_from_its_own_first_run_and_bounded_by_what_the_hour_has_seen() {
    use HoldingEnd::{Ended, EndedBy, Held, LastSeen};
    use HoldingStart::{FirstObserved, HeldBefore, Seen};
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let ideas = read(&query, &"GER".into(), &AtomicBool::new(false)).unwrap();
    let at = |hour: i32| ideas.at(hour).into_iter().map(|held| (ideas.spans[held.span].token.as_str(), held.span, held.first_hour, held.last_hour, held.start, held.end)).collect::<Vec<_>>();
    assert_eq!(
        at(112),
        [
            ("civilian_economy", 0, 100, 112, HeldBefore, Held),
            ("export_focus", 1, 100, 109, HeldBefore, Ended),
            ("adv_a", 2, 105, 112, Seen, Held),
            ("free_trade", 3, 110, 112, Seen, Held),
        ]
    );
    assert!(ideas.unread_at(115));
    assert_eq!(
        at(115),
        [
            ("civilian_economy", 0, 100, 114, HeldBefore, LastSeen),
            ("export_focus", 1, 100, 109, HeldBefore, Ended),
            ("adv_a", 2, 105, 114, Seen, LastSeen),
            ("free_trade", 3, 110, 114, Seen, LastSeen),
        ],
        "inside the unread hour nothing read after it is known: no bridge, no end, no newcomer"
    );
    assert_eq!(
        at(125),
        [
            ("civilian_economy", 0, 100, 125, HeldBefore, Held),
            ("export_focus", 1, 100, 109, HeldBefore, Ended),
            ("adv_a", 2, 105, 119, Seen, Ended),
            ("free_trade", 3, 110, 114, Seen, EndedBy { hour: 116 }),
            ("harvest_pact", 6, 116, 125, FirstObserved, Held),
            ("adv_a", 7, 120, 125, Seen, Held),
        ],
        "a holding bridges the unread hour once it is seen again; each holder's holding keeps its own first run, so a later holder names nothing earlier"
    );
    assert_eq!(ideas.templates.keys().copied().collect::<Vec<_>>(), [2, 4, 7], "a holder's template is read for each run it begins, never taken from a later one");
    assert_eq!((ideas.next_change(112), ideas.next_change(115), ideas.next_change(120)), (Some(115), Some(116), Some(LAST + 1)));
    assert_eq!(ideas.warnings.get(super::TOKENLESS_SEATS), Some(&125), "a warning is dated to the hour it was first read");

    let mut kept = super::Presentation::default();
    let before_gap = kept.holdings(&ideas, 112);
    assert!(Arc::ptr_eq(&before_gap, &kept.holdings(&ideas, 113)));
    let gap = kept.holdings(&ideas, 115);
    assert!(!Arc::ptr_eq(&before_gap, &gap));
    assert!(!Arc::ptr_eq(&before_gap, &kept.holdings(&ideas, 112)));
    for hour in (FIRST..=LAST).chain((FIRST..=LAST).rev()) {
        let mut reconstructed = kept.holdings(&ideas, hour).to_vec();
        for holding in &mut reconstructed {
            holding.last_hour = holding.last_at(hour);
        }
        assert_eq!(reconstructed, ideas.at(hour), "holding structure at hour {hour}");
    }
}

#[test]
fn tokenless_holder_clears_sprite_and_sets_per_look_artwork_missing() {
    let (_, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("common/defines/mod.lua", "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"political_advisor\" } } }"),
        ("interface/ideas.gfx", "spriteTypes = { spriteType = { name = \"GFX_idea_adv_shared\" texturefile = \"gfx/interface/ideas/adv_shared.tga\" } }"),
    ]);
    let texture = folder.0.join("gfx/interface/ideas/adv_shared.tga");
    std::fs::create_dir_all(texture.parent().unwrap()).unwrap();
    let tga = [0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 0x28, 30, 20, 10, 255];
    std::fs::write(&texture, tga).unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&folder.0, &[][..])]).unwrap();
    let cat = dashboard_base::catalogue::tests::layers_catalogue(layers);
    let content = dashboard_base::products::politics_content::PoliticsContent::load(Arc::new(cat)).unwrap();
    let country = dashboard_text::idea_names::IdeaCountry { tag: "GER", slot: 7, look: None };
    let named = span("adv_shared", 100, 110, Some((2, "CHAR_B", "Saved Named")));
    let (control, control_problem) = super::look(&content, &named, None, country);
    assert!(control.artwork.icon.is_some(), "known holder decodes fallback idea icon: {control_problem:?}");
    assert_eq!(control.artwork.sprite.as_deref(), Some("GFX_idea_adv_shared"), "known holder retains fallback idea sprite");
    assert_eq!(control_problem, None);
    let unnamed = span("adv_shared", 100, 110, Some((1, "", "Saved Unnamed")));
    let (look, problem) = super::look(&content, &unnamed, None, country);
    assert_eq!(look.artwork.icon, None);
    assert_eq!(look.artwork.sprite, None, "withheld advisor clears stale sprite");
    assert!(look.artwork_missing.as_deref().is_some_and(|m| m.contains("no recorded script token")), "{:?}", look.artwork_missing);
    assert_eq!(look.artwork_missing, problem);
}

#[test]
fn scripted_idea_names_descriptions_and_tooltips_update_at_the_exact_variable_change_hour() {
    use replay_data::rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticsRow, Recorded,
        ScriptVariableNameRow, ScriptVariableValueRow, ScriptVariablesRow, StateRow,
    };

    let tables = dashboard_text::catalogue::tests::script_game::executable().build();
    engine_executable::executable_tables::scope(Some(tables), || {
    let frames: Vec<_> = (100..=120)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1_000));
            set(
                &mut frame,
                TableId::Countries,
                TableRows::Countries(vec![
                    CountryRow { id: GER, canonical_slot: 7, tag: b"AAA".to_vec(), ..Default::default() },
                    CountryRow { id: 9002, canonical_slot: 8, tag: b"BBB".to_vec(), ..Default::default() },
                ]),
            );
            set(
                &mut frame,
                TableId::CountryLinks,
                TableRows::CountryLinks(vec![
                    CountryLinkRow { id: GER, canonical_slot: 7 },
                    CountryLinkRow { id: 9002, canonical_slot: 8 },
                ]),
            );
            set(
                &mut frame,
                TableId::CountryAppearance,
                TableRows::CountryAppearance(vec![
                    CountryAppearanceRow { country: GER, ..Default::default() },
                    CountryAppearanceRow { country: 9002, ..Default::default() },
                ]),
            );
            set(
                &mut frame,
                TableId::Politics,
                TableRows::Politics(vec![
                    PoliticsRow { country: GER, ruling_party: None, ..Default::default() },
                    PoliticsRow { country: 9002, ruling_party: None, ..Default::default() },
                ]),
            );
            set(
                &mut frame,
                TableId::States,
                TableRows::States(vec![StateRow {
                    id: 1,
                    owner: if hour < 110 { GER } else { 9002 },
                    controller: if hour < 110 { GER } else { 9002 },
                    ..Default::default()
                }]),
            );
            set(
                &mut frame,
                TableId::ScriptVariables,
                TableRows::ScriptVariables(vec![ScriptVariablesRow {
                    scope_kind: 1,
                    scope_id: GER,
                    seed: Recorded::Value(None),
                    entries: Vec::new(),
                }]),
            );
            set(&mut frame, TableId::ScriptVariableNames, TableRows::ScriptVariableNames(vec![
                ScriptVariableNameRow { id: 1, name: b"synth_level".to_vec() },
            ]));
            set(&mut frame, TableId::ScriptVariableValues, TableRows::ScriptVariableValues(vec![
                ScriptVariableValueRow { scope_kind: 1, scope_id: GER, var_id: 1, value_raw: if hour < 110 { 100_000 } else { 200_000 } },
            ]));
            let mut active = vec![ActiveIdeaRow { country: GER, ordinal: 0, token: b"synth_dynamic".to_vec() }];
            if hour < 109 {
                active.push(ActiveIdeaRow { country: GER, ordinal: 1, token: b"synth_refused".to_vec() });
                active.push(ActiveIdeaRow { country: GER, ordinal: 2, token: b"synth_ending".to_vec() });
            }
            set(
                &mut frame,
                TableId::ActiveIdeas,
                TableRows::ActiveIdeas(active),
            );
            set(&mut frame, TableId::HiredAdvisors, TableRows::HiredAdvisors(Vec::new()));
            set(&mut frame, TableId::Characters, TableRows::Characters(Vec::new()));
            set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(Vec::new()));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey { source_id: "scripted-ideas".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let (cat, _dir) = dashboard_base::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/synth.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=7},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/scripted_localisation/synth.txt",
            concat!(
                "defined_text = {\n",
                "  name = synth_level_loc\n",
                "  text = { trigger = { check_variable = { synth_level > 1 } } localization_key = SYNTH_HIGH }\n",
                "  text = { localization_key = SYNTH_LOW }\n",
                "}\n",
            ),
        ),
        (
            "common/ideas/synth.txt",
            concat!(
                "idea_categories = { synth_category = {} }\n",
                "ideas = {\n",
                "  country = {\n",
                "    synth_dynamic = { modifier = { custom_modifier_tooltip = synth_tip } }\n",
                "    synth_refused = {}\n",
                "    synth_ending = {}\n",
                "  }\n",
                "}\n",
            ),
        ),
        (
            "localisation/english/synth_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n",
                " AAA_ADJ:0 \"Alphan\"\n",
                " BBB:0 \"Beta\"\n",
                " BBB_ADJ:0 \"Betan\"\n",
                " synth_category:0 \"\u{a4}Country category\"\n",
                " SYNTH_LOW:0 \"Level Low\"\n",
                " SYNTH_HIGH:0 \"Level High\"\n",
                " synth_dynamic:0 \"[synth_level_loc]\"\n",
                " synth_dynamic_desc:0 \"[ROOT.GetName] brief\"\n",
                " synth_tip:0 \"[1.owner.GetAdjective] bonus\"\n",
                " synth_refused:0 \"[(ROOT.GetName?a:b)]\"\n",
                " synth_ending:0 \"[synth_level_loc]\"\n",
            ),
        ),
    ]);
    let content = Arc::new(dashboard_base::products::politics_content::PoliticsContent::load(Arc::new(cat)).unwrap());
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let read = super::history(&source, 1, &"AAA".into()).unwrap();
    let reply_at = |hour: i32| {
        let params = super::IdeaHistoryParams { branch_id: "1".into(), country: "AAA".into(), hour };
        let asked = super::Asked { params: &params, tag: "AAA", hour, unrecorded: None };
        let mut noted = dashboard_base::catalogue::OwnRefusals::default();
        dashboard_base::catalogue::own_refusals(&mut noted, || {
            let reply = super::present(&query, &content, &asked, &read, None, &AtomicBool::new(false)).unwrap();
            assert!(reply.categories.iter().any(|category| category.token == "synth_category"));
            assert!(!reply.warnings.iter().any(|line| line.contains("0x14")));
            assert!(content.catalogue.refusal_lines().iter().any(|line| line.contains("0x14")));
            reply
        })
    };

    let r100 = reply_at(100);
    let dyn_100 = r100.looks.iter().find(|l| l.info.token == "synth_dynamic").unwrap();
    assert_eq!(dyn_100.artwork.name.as_deref(), Some("Level Low"));
    assert_eq!(dyn_100.artwork.description.as_deref(), Some("Alpha brief"));
    assert_eq!(dyn_100.info.name_missing, None);
    let tip_100 = r100.texts.iter().find(|t| t.key == "synth_tip").unwrap();
    assert_eq!(tip_100.text.as_deref(), Some("Alphan bonus"));
    assert_eq!(r100.next_change, Some(109));

    let refused_100 = r100.looks.iter().find(|l| l.info.token == "synth_refused").unwrap();
    assert_eq!(refused_100.artwork.name, None);
    assert!(
        refused_100
            .info
            .name_missing
            .as_deref()
            .is_some_and(|why| why.contains("synth_refused") && why.contains("conditional")),
        "{:?}",
        refused_100.info.name_missing
    );

    let (repeat, rows) = dashboard_base::products::at_hour::counted(|| reply_at(100));
    assert_eq!(serde_json::to_vec(&repeat).unwrap(), serde_json::to_vec(&r100).unwrap());
    assert_eq!(rows, 0, "another subscription at the same hour reuses the exact reply");

    let (r109, reads) = replay_recording::cells::count_reads(|| reply_at(109));
    assert_eq!(reads[&TableId::ScriptVariableValues].backward_seeks, 0);
    let ending = r109.holdings.iter().find(|h| h.token == "synth_ending").unwrap();
    assert_eq!(ending.last_hour, 108);
    assert_eq!(r109.next_change, Some(110), "validity holds until the exact variable change");
    assert!(r109.warnings.iter().any(|warning| warning.contains("synth_refused")));

    let r110 = reply_at(110);
    let ending = r110.holdings.iter().find(|h| h.token == "synth_ending").unwrap();
    assert_eq!(r110.looks[ending.look as usize].artwork.name.as_deref(), Some("Level Low"));
    let dyn_110 = r110.looks.iter().find(|l| l.info.token == "synth_dynamic").unwrap();
    assert_eq!(dyn_110.artwork.name.as_deref(), Some("Level High"));
    let tip_110 = r110.texts.iter().find(|t| t.key == "synth_tip").unwrap();
    assert_eq!(tip_110.text.as_deref(), Some("Betan bonus"));
    assert_eq!(r110.next_change, Some(121), "settled through end of branch after the change");
    });
}

#[test]
fn historical_idea_names_use_the_slots_tag_at_the_evaluation_hour() {
    use dashboard_base::products::LoadContext;
    use dashboard_query::identity::Country;
    use replay_data::rows::{CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticsRow};

    let frames: Vec<_> = (100..=110)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1_000));
            let tag: &[u8] = if hour < 106 { b"AAA" } else { b"BBB" };
            set(
                &mut frame,
                TableId::Countries,
                TableRows::Countries(vec![CountryRow {
                    id: GER,
                    canonical_slot: 1,
                    tag: tag.to_vec(),
                    ..Default::default()
                }]),
            );
            set(
                &mut frame,
                TableId::CountryLinks,
                TableRows::CountryLinks(vec![CountryLinkRow { id: GER, canonical_slot: 1 }]),
            );
            set(
                &mut frame,
                TableId::CountryAppearance,
                TableRows::CountryAppearance(vec![CountryAppearanceRow { country: GER, ..Default::default() }]),
            );
            set(
                &mut frame,
                TableId::Politics,
                TableRows::Politics(vec![PoliticsRow { country: GER, ruling_party: None, ..Default::default() }]),
            );
            let mut active = Vec::new();
            if (101..=104).contains(&hour) {
                active.push(ActiveIdeaRow { country: GER, ordinal: 0, token: b"synth_law".to_vec() });
            }
            if hour >= 101 {
                active.push(ActiveIdeaRow { country: GER, ordinal: 1, token: b"synth_span".to_vec() });
            }
            set(&mut frame, TableId::ActiveIdeas, TableRows::ActiveIdeas(active));
            set(&mut frame, TableId::HiredAdvisors, TableRows::HiredAdvisors(Vec::new()));
            set(&mut frame, TableId::Characters, TableRows::Characters(Vec::new()));
            set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(Vec::new()));
            frame
        })
        .collect();
    let (_, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
        ("localisation/languages.yml", "\u{feff}l_english:\n l_english: \"English\"\n"),
        (
            "common/defines/synth.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=7},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/ideas/synth.txt",
            "ideas = { economy = { law = yes synth_law = {} synth_span = {} } }",
        ),
        (
            "localisation/english/synth_l_english.yml",
            concat!(
                "\u{feff}l_english:\n",
                " AAA:0 \"Alpha\"\n",
                " BBB:0 \"Beta\"\n",
                " AAA_synth_law:0 \"[ROOT.GetName] Law\"\n",
                " BBB_synth_law:0 \"Wrong Beta Law\"\n",
                " AAA_synth_span:0 \"Alpha Span\"\n",
                " BBB_synth_span:0 \"Beta Span\"\n",
            ),
        ),
    ]);
    let tables = dashboard_text::catalogue::tests::script_game::executable()
        .executable(b"test")
        .build();
    let (options, identity) =
        dashboard_base::test_fixtures::content_identity_with_tables(&folder.0, tables);
    let mut input = records(1, frames);
    if let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] {
        session.identity = identity.clone();
    }
    let replay = dashboard_base::test_fixtures::with_facts_tables(&options, &identity, input, tables)
        .into_timelines(&Default::default())
        .unwrap();
    let source = Source::from_replay(
        dashboard_base::SourceKey { source_id: "slot-tag-change".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let content = dashboard_base::products::politics_content::politics_content(&source, &options).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country = Country::Slot(1);
    let read = super::history(&source, 1, &country).unwrap();
    let params = super::IdeaHistoryParams { branch_id: "1".into(), country: country.clone(), hour: 110 };
    let asked = super::Asked { params: &params, tag: "BBB", hour: 110, unrecorded: None };
    let name_of = |reply: &super::IdeaHistoryReply, token: &str| {
        let holding = reply.holdings.iter().find(|h| h.token == token).unwrap();
        reply.looks[holding.look as usize].artwork.name.clone()
    };

    let params_105 = super::IdeaHistoryParams { branch_id: "1".into(), country, hour: 105 };
    let asked_105 = super::Asked { params: &params_105, tag: "AAA", hour: 105, unrecorded: None };
    let present_105 = source
        .in_scope(|| super::present(&query, &content, &asked_105, &read, None, &AtomicBool::new(false)))
        .unwrap();
    assert_eq!(name_of(&present_105, "synth_span").as_deref(), Some("Alpha Span"));
    assert_eq!(present_105.next_change, Some(106));

    let present_before = source
        .in_scope(|| super::present(&query, &content, &asked, &read, None, &AtomicBool::new(false)))
        .unwrap();
    assert_eq!(name_of(&present_before, "synth_law").as_deref(), Some("Alpha Law"));
    assert_eq!(name_of(&present_before, "synth_span").as_deref(), Some("Beta Span"));

    let markers = source
        .in_scope(|| super::load_for_markers(&source, &params, &LoadContext { content: Some(options) }, &AtomicBool::new(false)))
        .unwrap();
    assert_eq!(name_of(&markers, "synth_law").as_deref(), Some("Alpha Law"));
    assert_eq!(name_of(&markers, "synth_span").as_deref(), Some("Alpha Span"));

    let present_after = source
        .in_scope(|| super::present(&query, &content, &asked, &read, None, &AtomicBool::new(false)))
        .unwrap();
    assert_eq!(name_of(&present_after, "synth_span").as_deref(), Some("Beta Span"));
}
