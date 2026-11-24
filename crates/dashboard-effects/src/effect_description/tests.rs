use content_reader_key_names::key_name_products::KeyNamesExt as _;
use super::*;
use crate::{
    catalogue::{Catalogue, tests::Folder},
    scripted_text::{EventObject, EventScope, Link},
    triggers::ContentFacts,
};
use replay_data::rows::*;
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::TableView};
use std::{collections::BTreeSet, sync::Arc};

#[test]
fn effect_key_names_share_the_complete_cached_registry_after_a_source_disappears() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/country_tags/test.txt",
            "SYN=\"countries/synthetic.txt\"",
        ),
        (
            "common/resources/test.txt",
            "resources={synthetic_resource={icon_frame=1}}",
        ),
        (
            "common/buildings/test.txt",
            "buildings={synthetic_building={}}",
        ),
        (
            "common/scripted_effects/test.txt",
            "synthetic_effect={army_experience=2}",
        ),
        (
            "common/units/equipment/test.txt",
            "equipments={synthetic_type={is_archetype=yes}} duplicate_archetypes={synthetic_copy={archetype=missing}}",
        ),
    ]);
    let layers = &catalogue.content.layers;
    let shared = layers.key_names().unwrap();
    let scripted = catalogue.scripted_content().unwrap();
    std::fs::remove_file(folder.0.join("common/country_tags/test.txt")).unwrap();
    assert!(content_reader_key_names::key_name_products::KeyNameProducts::read(layers).is_err());

    let cached = scripted.effects().unwrap();
    assert!(Arc::ptr_eq(&shared, cached.key_names.as_ref().unwrap()));
    let names = cached.view(None).key_names.unwrap();
    assert!(std::ptr::eq(&raw const shared.names, names));
    assert!(names.countries.contains(b"SYN".as_slice()));
    assert!(
        names
            .counts
            .resources
            .contains(b"synthetic_resource".as_slice())
    );
    assert!(
        names
            .counts
            .buildings
            .contains(b"synthetic_building".as_slice())
    );
    assert!(
        names
            .scripted_effects
            .contains(b"synthetic_effect".as_slice())
    );
    assert!(
        layers.key_name_products().unwrap()
            .equipment_database()
            .unwrap()
            .get("synthetic_type")
            .is_some()
    );
    assert!(!cached.gaps.contains_key("effect key names"));
    assert!(cached.scripted_effects.is_some());
    assert_eq!(
        cached
            .notes
            .iter()
            .filter(
                |note| note.contains("asked to duplicate missing, which is not a known archetype")
            )
            .count(),
        1
    );
}

#[test]
fn advisor_templates_and_idea_notes_share_one_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={\"synthetic_slot\"}},NSupply={MAX_RAILWAY_LEVEL=5}}",
        ),
        (
            "common/idea_tags/test.txt",
            "idea_categories={country={type=national_spirit} other={type=national_spirit}}",
        ),
        (
            "common/ideas/a.txt",
            "ideas={country={synthetic_idea={}} synthetic_slot={synthetic_advisor={name=synthetic_name}}}",
        ),
        (
            "common/ideas/b.txt",
            "ideas={other={synthetic_idea={}}}",
        ),
        (
            "common/ideologies/test.txt",
            "ideologies={synthetic_ideology={color=16711680}}",
        ),
        (
            "common/terrain/test.txt",
            "categories={synthetic_terrain={movement_cost=bad}}",
        ),
        (
            "common/country_leader/test.txt",
            "leader_traits={synthetic_trait={unregistered_modifier={}}}",
        ),
    ]);
    let scripted = catalogue.scripted_content().unwrap();
    let environment = scripted.environment().unwrap();
    let _ = environment.leader_traits();
    let _ = environment.terrain();
    let _ = environment.ideologies();
    let content = scripted.effects().unwrap();
    let warning = "1 ideas share a lexer token; the first database entry is used.";
    assert_eq!(
        content.notes.iter().filter(|note| *note == warning).count(),
        1
    );
    let mut previous = None;
    for catalogue_name in [
        "Ideology synthetic_ideology",
        "terrain category synthetic_terrain",
        "Leader traits unavailable",
    ] {
        assert!(
            environment
                .catalogue_notes()
                .iter()
                .any(|note| note.contains(catalogue_name)),
            "{catalogue_name}: {:?}",
            environment.catalogue_notes()
        );
        let position = environment
            .catalogue_notes()
            .iter()
            .position(|note| note.contains(catalogue_name))
            .unwrap();
        assert!(previous.is_none_or(|before| before < position));
        previous = Some(position);
    }
    for warning in &environment.catalogue_notes() {
        assert_eq!(
            content.notes.iter().filter(|note| *note == warning).count(),
            1,
            "{warning}"
        );
    }
    assert_eq!(
        scripted.recursion,
        environment
            .defines()
            .int32("NGame", "MAX_SCRIPTED_LOC_RECURSION")
            .unwrap()
    );
    assert_eq!(content.max_railway_level, Some(5));
    assert!(std::ptr::eq(environment, scripted.environment().unwrap()));
    assert!(std::ptr::eq(content, scripted.effects().unwrap()));
    let templates = content.advisor_templates.as_ref().unwrap();
    assert_eq!(
        templates
            .advisor("synthetic_advisor")
            .unwrap()
            .unwrap()
            .slot,
        "synthetic_slot"
    );
    assert_eq!(
        templates.name("synthetic_advisor").unwrap(),
        Some(&content_reader_key_names::characters::Authored::Asset(
            "synthetic_name".to_owned()
        ))
    );
}

#[test]
fn effect_catalogues_remain_available_without_map_inputs() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        ("common/ideologies/test.txt", "ideologies={synthetic={}}"),
        (
            "common/terrain/test.txt",
            "categories={synthetic={movement_cost=2}}",
        ),
    ]);
    let scripted = catalogue.scripted_content().unwrap();
    let environment = scripted.environment().unwrap();
    assert!(
        environment
            .ideologies()
            .unwrap()
            .get("synthetic")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        environment.terrain().unwrap().categories[0].movement_cost,
        200_000
    );
    assert!(scripted.effects().unwrap().ideas.is_some());
    let fixture = Fixture::new();
    let text = ScriptedText::new(&fixture.frame, &catalogue.localisation, scripted, 1);
    assert!(text.world().is_ok());
    let geography = text
        .trigger_world()
        .unwrap()
        .evaluate(
            &content_reader_script::triggers::read(
                b"impassable=no ",
                &content_reader_script::key_names::KeyNames::default(),
            )
            .items,
            crate::triggers::Scopes::of(crate::triggers::Scope::State(64)),
        )
        .decided()
        .unwrap_err();
    assert!(geography.contains("default.map"), "{geography}");
}

pub(super) struct Fixture {
    pub frame: DecodedFrame,
    pub catalogue: Catalogue,
    _folder: Folder,
    dlcs: BTreeSet<String>,
    pub(super) triggers: content_reader_script::trigger_keys::TriggerNames,
    pub(super) aliases: Arc<content_reader_key_names::tag_aliases::TagAliases>,
    pub neighbors: Option<BTreeMap<u32, Vec<u32>>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let (catalogue, folder) = crate::fixtures::folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NSupply={MIN_TRAIN_REQUIREMENT=2}}",
            ),
            (
                "localisation/test_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n CCC:0 \"Gamma\"\n",
                    " AAA_DEF:0 \"alpha\"\n BBB_DEF:0 \"beta\"\n CCC_DEF:0 \"gamma\"\n",
                    " POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
                    " EFFECT_ADD_STABILITY:0 \"S$VAL|0$\"\n EFFECT_ADD_WAR_SUPPORT:0 \"W$VAL|0$\"\n",
                    " EFFECT_TRANSFER_STATE:0 \"Transfer $STATE$\"\n EFFECT_ADD_STATE_CLAIM:0 \"Claim $STATE$\"\n",
                    " EFFECT_DECLARE_WAR:0 \"War $NAME$/$COUNTRY$\"\n EFFECT_ANNEX_COUNTRY:0 \"Annex $NAME$/$TARGET$\"\n",
                    " EFFECT_START_BORDER_WAR:0 \"Border $STATE1$/$STATE2$\"\n",
                    " EFFECT_CONTEXT_MIO:0 \"$ORGANISATION$ $FLAG$\"\n",
                    " EFFECT_CONTEXT_COUNTRY:0 \"@$TAG$$COUNTRY$\"\n EFFECT_CONTEXT_STATE:0 \"$COUNTRY$\"\n",
                    " EFFECT_LIST_COUNTRY:0 \"@$TAG$$COUNTRY$\"\n EFFECT_CONTEXT_LEADER:0 \"$FLAG$$LEADER$\"\n",
                    " RANDOM_EFFECT:0 \"$CHANCE|%0$: \"\n RANDOM_LIST_EFFECT:0 \"$SIZE$ options:\\n\"\n NO_EFFECT:0 \"No effect\"\n",
                    " EFFECT_SET_POLITICS:0 \"Politics\"\n EFFECT_SET_POLITICS_NO_ELECTIONS:0 \"No elections\"\n EFFECT_SET_POLITICS_ELECTIONS:0 \"Elections\"\n EFFECT_SET_POLITICS_ELECTION_FREQUENCY:0 \"Every $MONTHS$\"\n",
                    " EFFECT_CHARACTER_LIST_POLITICAL_ADVISORS:0 \"Advisors\"\n EFFECT_CHARACTER_LIST_MILITARY_ADVISORS:0 \"Military advisors\"\n EFFECT_CHARACTER_LIST_THEORISTS:0 \"Theorists\"\n EFFECT_CHARACTER_LIST_IN_SERVICE:0 \"in service\"\n",
                    " POLITICS_PARTY_NAME_CHANGE:0 \"$IDEOLOGY$=$NAME$\"\n EFFECT_SET_POLITICS_RULING_PARTY:0 \"Party $PARTY$\"\n",
                    " EFFECT_LIST_OTHERS:0 \"$LIST$ and $NUMBER|0$ others\"\n",
                    " COLLECTION_OP_FACTION_MEMBERS:0 \"Faction roster\"\n",
                    " EFFECT_ADD_TO_FACTION:0 \"Join $COUNTRY$\"\n EFFECT_ADD_TO_FACTION_NAME:0 \"Join $COUNTRY$/$FACTION$\"\n",
                    " EFFECT_RELEASE_NATION:0 \"Release $COUNTRY$\"\n",
                    " EFFECT_REMOVE_FROM_FACTION_NAME:0 \"Leave $COUNTRY$/$FACTION$\"\n EFFECT_REMOVE_FROM_FACTION_END_PUPPET:0 \"Free $PUPPET$/$MASTER$\"\n",
                    " ADD_MASTERY_EFFECT:0 \"Mastery $AMOUNT|0$\"\n TRACK_EFFECT_ONLY_FOLDER:0 \"$TRACK_EFFECT_DESC$ $NAME$\"\n",
                    " EFFECT_TECH_BONUS_USES:0 \"$USES|0$ uses\"\n EFFECT_ADD_DOCTRINE_COST_REDUCTION:0 \"Doctrine $LIST$/$REDUCTION|0$/$USES$\"\n",
                    " FOLDER:0 \"Land\"\n EFFECT_COUNTRY_EVENT:0 \"Event $EVENT$\"\n REFERENCE:0 \"[From.GetTag]\"\n",
                    " EFFECT_DELETE_UNIT_TEMPLATE_AND_UNITS:0 \"Delete $NAME$\"\n EFFECT_DISBAND_UNIT_TEMPLATE_AND_UNITS:0 \"Disband $NAME$\"\n",
                    " TIP:0 \"tip\"\n TINY:0 \"x\"\n EFFECT_ADD_TIMED_IDEA:0 \"Timed $IDEA$ for $TIME_DESC$ ($DESC$).\\n\"\n",
                    " VARIABLE_TIP:0 \"[This.GetTag]:$LEFT|0$/$RIGHT|0$\"\n",
                    " EFFECT_TIMED_IDEA_TIME_VALUE:0 \"$VALUE|0$\"\n FREQUENCY_YEAR:0 \"years\"\n FREQUENCY_MONTH:0 \"months\"\n FREQUENCY_DAY:0 \"days\"\n",
                    " SYN_IDEA:0 \"Spirit\"\n ATTACK:0 \" Attack\"\n DEFENSE:0 \" Defense\"\n",
                    " EFFECT_REMOVE_TARGETED_DECISION:0 \"Remove $NAME$\"\n SYN_DECISION:0 \"Conflict in [From.From.GetName]\"\n AAA_SYN_DECISION:0 \"Conflict in [From.From.GetName]\"\n"
                ),
            ),
        ]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        let countries = [(100, 1, "AAA"), (200, 2, "BBB"), (300, 3, "CCC")];
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                countries
                    .iter()
                    .map(|&(country, _, _)| CountryCounterInputRow {
                        country,
                        owned_state_count: 5,
                        owned_states: Recorded::Value(Some(if country == 100 {
                            (1..=5).collect()
                        } else {
                            Vec::new()
                        })),
                        controlled_states: Recorded::Value(Some(if country == 100 {
                            (1..=5).collect()
                        } else {
                            Vec::new()
                        })),
                        core_states: Recorded::Value(Some(Vec::new())),
                        army_leaders: Recorded::Value(Some(Vec::new())),
                        navy_leaders: Recorded::Value(Some(Vec::new())),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            ["One", "Two", "Three", "Four", "Five"]
                .iter()
                .enumerate()
                .map(|(index, name)| StateRow {
                    id: index as u32 + 1,
                    owner: 100,
                    controller: 100,
                    custom_name: name.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
            (1..=5)
                .map(|id| ProvinceRow {
                    id,
                    state: id,
                    ..Default::default()
                })
                .collect(),
        ));
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        let mut fixture = Self {
            frame,
            catalogue,
            _folder: folder,
            dlcs: Default::default(),
            triggers: Default::default(),
            aliases: Default::default(),
            neighbors: None,
        };
        fixture.set_countries(
            countries
                .iter()
                .map(|&(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        );
        fixture
    }

    pub(super) fn set_countries(&mut self, countries: Vec<CountryRow>) {
        *self.frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(crate::test_country_links::rows(
                countries
                    .iter()
                    .map(|row| CountryLinkRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                    }),
            ));
        *self.frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(
                countries
                    .iter()
                    .map(|row| CountryAppearanceRow {
                        country: row.id,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *self.frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
            countries
                .iter()
                .map(|row| PoliticsRow {
                    country: row.id,
                    ..Default::default()
                })
                .collect(),
        ));
        *self.frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
    }

    pub(super) fn render(
        &self,
        body: &str,
        scopes: &mut EventScopes,
        content: Content<'_>,
    ) -> Description {
        self.render_items(
            &content_reader_parser::parser::parse(body.as_bytes())
                .unwrap()
                .items,
            scopes,
            content,
        )
    }

    pub(super) fn render_native(
        &self,
        body: &str,
        scopes: &mut EventScopes,
        content: Content<'_>,
    ) -> Description {
        let items =
            content_reader_script::effect_keys::read_body(body.as_bytes(), content.key_names.unwrap())
                .unwrap();
        self.render_items(&items, scopes, content)
    }

    fn render_items(
        &self,
        items: &[Item],
        scopes: &mut EventScopes,
        content: Content<'_>,
    ) -> Description {
        self.with_context(content, |context| describe(items, context, scopes))
    }

    pub(super) fn with_renderer<T>(
        &self,
        scopes: &mut EventScopes,
        content: Content<'_>,
        run: impl FnOnce(&mut Renderer<'_, '_, '_>) -> T,
    ) -> T {
        self.with_context(content, |context| {
            let mut renderer = Renderer {
                context,
                graph: Rc::new(RefCell::new(scopes.clone())),
                evaluator: OnceCell::new(),
                out: Description::default(),
                scripted_stack: Vec::new(),
                concise: None,
            };
            let out = run(&mut renderer);
            *scopes = renderer.graph.borrow().clone();
            out
        })
    }

    pub(super) fn with_context<T>(
        &self,
        content: Content<'_>,
        run: impl FnOnce(&Context<'_, '_>) -> T,
    ) -> T {
        self.with_context_linking(content, false, run)
    }

    pub(super) fn with_context_linking<T>(
        &self,
        content: Content<'_>,
        links: bool,
        run: impl FnOnce(&Context<'_, '_>) -> T,
    ) -> T {
        let scripted = self.catalogue.scripted_content().unwrap();
        let text = ScriptedText::new(&self.frame, &self.catalogue.localisation, scripted, 1);
        let mut facts = ContentFacts::content_only(&self.dlcs, &self.triggers, &self.aliases);
        if let Some(neighbors) = &self.neighbors {
            facts.neighbor_states = Ok(neighbors);
        }
        let world = World::read(&self.frame, facts);
        text.set_synthetic_world(world);
        let context = Context::new(&text, &self.catalogue.localisation, 1, "synthetic", content);
        let context = if links { context.with_entity_links() } else { context };
        run(&context)
    }
}

pub(super) fn fresh() -> EventScopes {
    EventScopes::new(vec![EventScope::of(EventObject::Country(1))])
}

pub(super) fn synthetic_countries(count: u32) -> Vec<CountryRow> {
    (1..=count)
        .map(|slot| {
            let index = slot - 1;
            CountryRow {
                id: slot * 100,
                canonical_slot: slot,
                tag: if slot == count {
                    b"ZZZ".to_vec()
                } else {
                    vec![
                        b'A' + (index / 676) as u8,
                        b'A' + ((index / 26) % 26) as u8,
                        b'A' + (index % 26) as u8,
                    ]
                },
                ..Default::default()
            }
        })
        .collect()
}

#[test]
fn native_country_targets_read_custom_target_fields() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for body in [
        "puppet={target=BBB}",
        "remove_wargoal={target=BBB}",
        "release_autonomy={target=BBB}",
        "set_autonomy={target=BBB}",
        "send_equipment={target=BBB}",
        "send_equipment_fraction={target=BBB}",
        "transfer_units_fraction={target=BBB}",
    ] {
        let parsed = content_reader_parser::parser::parse(body.as_bytes()).unwrap();
        let target = fixture.with_renderer(&mut fresh(), Content::default(), |renderer| {
            renderer.country_target(&parsed.items[0], Scopes::of(Scope::Country(1)))
        });
        assert_eq!(target, Ok(Some(2)), "{body}");
    }
}

#[test]
fn equipment_and_autonomy_content_is_cached_with_reader_notes() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/units/equipment/test.txt",
            "equipments={synthetic_type={is_archetype=yes}} duplicate_archetypes={synthetic_copy={archetype=missing}}",
        ),
        (
            "common/autonomous_states/test.txt",
            "autonomy_state={id=synthetic_autonomy}",
        ),
    ]);
    let scripted = catalogue.scripted_content().unwrap();
    let cached = scripted.effects().unwrap();
    let content = cached.view(None);
    assert!(content.equipment.unwrap().get("synthetic_type").is_some());
    assert!(
        content
            .autonomy_states
            .unwrap()
            .get("synthetic_autonomy")
            .unwrap()
            .is_some()
    );
    for expected in [
        "asked to duplicate missing, which is not a known archetype",
        "autonomy state synthetic_autonomy has no min_freedom_level",
    ] {
        assert!(
            cached.notes.iter().any(|note| note.contains(expected)),
            "{:?}",
            cached.notes
        );
    }
    assert!(std::ptr::eq(cached, scripted.effects().unwrap()));
}

fn country_target_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *fixture.frame.tables[TableId::States.index()]
    {
        rows[1].controller = 200;
        rows[2].owner = 200;
    }
    *fixture.frame.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(vec![StateResistanceRow {
            state: 2,
            country: 300,
            ..Default::default()
        }]));
    fixture
}

#[test]
fn native_country_target_selected_states_return_their_owner() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = country_target_fixture();
    for (selector, name) in [
        ("THIS", "Alpha"),
        ("ROOT", "Beta"),
        ("PREV", "Beta"),
        ("FROM", "Beta"),
    ] {
        let mut graph = EventScopes::new(vec![
            EventScope {
                root: Link::Scope(1),
                prev: Link::Scope(1),
                from: Link::Scope(1),
                ..EventScope::of(EventObject::State(2))
            },
            EventScope::of(EventObject::State(3)),
        ]);
        let out = fixture.render(
            &format!("release={selector}"),
            &mut graph,
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!("Release {name}\n"),
            "{selector}: {:?}",
            out.refusals
        );
        assert!(out.refusals.is_empty(), "{selector}: {:?}", out.refusals);
    }
}

#[test]
fn native_country_target_state_selector_bits_read_recorded_countries() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = country_target_fixture();
    for (selector, name) in [
        ("owner", "Alpha"),
        ("controller", "Beta"),
        ("occupied", "Gamma"),
    ] {
        let mut graph = EventScopes::new(vec![EventScope::of(EventObject::State(2))]);
        let out = fixture.render(
            &format!("release={selector}"),
            &mut graph,
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!("Release {name}\n"),
            "{selector}: {:?}",
            out.refusals
        );
        assert!(out.refusals.is_empty(), "{selector}: {:?}", out.refusals);
    }
    for object in [EventObject::Country(1), EventObject::State(0)] {
        for selector in ["owner", "controller", "occupied"] {
            let mut graph = EventScopes::new(vec![EventScope::of(object)]);
            let out = fixture.render(
                &format!("release={selector}"),
                &mut graph,
                Content::default(),
            );
            assert_eq!(out, Description::default(), "{object:?}/{selector}");
        }
    }
}

#[test]
fn native_country_target_capital_and_random_keep_country_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = country_target_fixture();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: replay_data::rows::Recorded::Value(None),
            entries: vec![ScriptVariableEntry {
                name: b"capital".to_vec(),
                value_raw: 200 * 100_000,
            }],
        }]));
    for object in [EventObject::Country(1), EventObject::State(2)] {
        for selector in ["capital", "random"] {
            let mut graph = EventScopes::new(vec![EventScope::of(object)]);
            let out = fixture.render(
                &format!("release={selector}"),
                &mut graph,
                Content::default(),
            );
            assert_eq!(out, Description::default(), "{object:?}/{selector}");
        }
    }
}

#[test]
fn native_country_target_saved_probe_order_reads_separate_country_member() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = country_target_fixture();
    *fixture.frame.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(vec![
            GlobalSavedEventTargetRow {
                name: b"chosen".to_vec(),
                state: 2,
                country: 200,
                ..Default::default()
            },
        ]));
    for (stage, expected) in [
        (3, "Release Beta\n"),
        (2, "Release Gamma\n"),
        (1, "Release Beta\n"),
        (0, "Release Alpha\n"),
        (4, ""),
        (5, "Release Beta\n"),
    ] {
        let mut graph = EventScopes::new(vec![
            EventScope {
                root: Link::Scope(2),
                from: Link::Scope(1),
                ..EventScope::of(EventObject::Country(1))
            },
            EventScope::of(EventObject::Country(1)),
            EventScope::of(EventObject::Country(1)),
        ]);
        graph
            .tooltip_targets
            .insert(2, vec![(b"chosen".to_vec(), EventObject::Country(3))]);
        if stage <= 2 {
            graph
                .global_tooltip_targets
                .push((b"chosen".to_vec(), EventObject::Country(3)));
        }
        if stage <= 1 || stage == 5 {
            graph
                .saved_targets
                .insert(1, vec![(b"chosen".to_vec(), EventObject::Country(2))]);
        }
        if stage == 0 {
            graph
                .tooltip_targets
                .insert(1, vec![(b"chosen".to_vec(), EventObject::Country(1))]);
        } else if stage == 4 {
            graph
                .tooltip_targets
                .insert(1, vec![(b"chosen".to_vec(), EventObject::State(3))]);
        } else if stage == 5 {
            graph.tooltip_targets.insert(
                1,
                vec![
                    (b"chosen".to_vec(), EventObject::State(0)),
                    (b"chosen".to_vec(), EventObject::Country(1)),
                ],
            );
        }
        let out = fixture.render(
            "release=event_target:chosen",
            &mut graph,
            Content::default(),
        );
        assert_eq!(out.text, expected, "stage {stage}: {:?}", out.refusals);
        assert!(out.refusals.is_empty(), "stage {stage}: {:?}", out.refusals);
    }
    *fixture.frame.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(vec![
            GlobalSavedEventTargetRow {
                name: b"chosen".to_vec(),
                ..Default::default()
            },
            GlobalSavedEventTargetRow {
                name: b"chosen".to_vec(),
                country: 200,
                ..Default::default()
            },
        ]));
    assert_eq!(
        fixture.render(
            "release=event_target:chosen",
            &mut fresh(),
            Content::default()
        ),
        Description::default()
    );
}

#[test]
fn native_effect_saved_state_reads_member_without_country_or_context_projection() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    for (state, country, live, expected) in [
        (2, 9999, 0, "Claim Two\n"),
        (0, 9999, 0, "Claim \n"),
        (0, 0, GlobalSavedEventTargetRow::FACTION, "Claim \n"),
    ] {
        *fixture.frame.tables[TableId::GlobalSavedEventTargets.index()] =
            TableView::Complete(TableRows::GlobalSavedEventTargets(vec![
                GlobalSavedEventTargetRow {
                    name: b"chosen".to_vec(),
                    state,
                    country,
                    live,
                    ..Default::default()
                },
            ]));
        let mut graph = EventScopes::new(vec![EventScope::of(EventObject::State(3))]);
        let out = fixture.render(
            "add_state_claim=event_target:chosen",
            &mut graph,
            Content::default(),
        );
        assert_eq!(
            out.text, expected,
            "{state}/{country}/{live}: {:?}",
            out.refusals
        );
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn native_country_target_refuses_unported_projection_and_repeated_selector_forms() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for (body, object, reason) in [
        (
            "release=event_target:chosen",
            EventObject::Character(0),
            "lacks liveness",
        ),
        (
            "release=event_target:chosen",
            EventObject::Character(7),
            "lacks liveness",
        ),
        (
            "release=event_target:chosen",
            EventObject::UnitLeader(7),
            "lacks liveness",
        ),
        (
            "release=event_target:chosen",
            EventObject::Province(2),
            "lacks liveness",
        ),
        (
            "declare_war_on={target=ROOT target=FROM}",
            EventObject::Country(2),
            "retain selector flags",
        ),
    ] {
        let mut graph = fresh();
        graph
            .tooltip_targets
            .insert(0, vec![(b"chosen".to_vec(), object)]);
        let out = fixture.render(body, &mut graph, Content::default());
        assert_eq!(out.text, "", "{body}");
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            1,
            "{body}: {:?}",
            out.refusals
        );
        assert!(
            out.refusals.keys().next().unwrap().contains(reason),
            "{body}: {:?}",
            out.refusals
        );
    }
}

#[test]
fn state_leaves_refuse_unavailable_and_partial_rosters_before_invalid_state_logic() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    for (table, gap) in [
        (
            TableView::Unavailable {
                reason: "synthetic unavailable roster".into(),
            },
            [
                "states: not available: synthetic unavailable roster",
                "hour 1 States is unavailable: synthetic unavailable roster",
            ],
        ),
        (
            TableView::Truncated {
                rows: TableRows::States(vec![]),
                missing: 1,
            },
            [
                "states: read only in part (1 rows missing)",
                "hour 1 States is truncated (1 missing rows)",
            ],
        ),
    ] {
        *fixture.frame.tables[TableId::States.index()] = table;
        for leaf in [
            "add_state_claim=99",
            "transfer_state=99",
            "start_border_war={attacker={state=99} defender={state=2}}",
        ] {
            let out = fixture.render(leaf, &mut fresh(), Content::default());
            assert!(out.text.is_empty(), "{leaf}: {}", out.text);
            assert_eq!(
                out.refusals.values().sum::<usize>(),
                1,
                "{leaf}: {:?}",
                out.refusals
            );
            assert!(
                out.refusals
                    .keys()
                    .any(|why| gap.iter().any(|gap| why.contains(gap))),
                "{leaf}: {:?}",
                out.refusals
            );
        }
    }
    *fixture.frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![]));
    let out = fixture.render("add_state_claim=99 transfer_state=99 start_border_war={attacker={state=99} defender={state=2}}", &mut fresh(), Content::default());
    assert_eq!(out.text, "Claim \nTransfer \n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn unattached_else_and_final_else_continuations_are_named_once() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for body in [
        "else={army_experience=9} army_experience=2",
        "else_if={limit={always=yes} army_experience=9} army_experience=2",
        "if={limit={always=no} army_experience=9} else={army_experience=2} else={army_experience=8}",
        "if={limit={always=no} army_experience=9} else={army_experience=2} else_if={limit={always=yes} army_experience=8}",
    ] {
        let out = fixture.render(body, &mut fresh(), Content::default());
        assert_eq!(out.text, "XP2\n", "{body}");
        assert_eq!(
            out.refusals.values().sum::<usize>(),
            1,
            "{body}: {:?}",
            out.refusals
        );
        assert!(
            out.refusals
                .keys()
                .any(|why| why.contains("does not follow an open if chain"))
        );
    }
    let out = fixture.render(
        "BBB={else={army_experience=9} custom_effect_tooltip=TIP}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta tip");
    assert_eq!(
        out.refusals.values().sum::<usize>(),
        1,
        "{:?}",
        out.refusals
    );
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("does not follow an open if chain"))
    );
}

#[test]
fn failed_context_layout_probe_counts_one_description_occurrence() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let out = fixture.render(
        "BBB={unported_effect=1} army_experience=2",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP2\n");
    assert_eq!(
        out.refusals.values().sum::<usize>(),
        1,
        "{:?}",
        out.refusals
    );
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("unknown effect key unported_effect"))
    );
}

#[test]
fn empty_own_generic_builder_keeps_children_and_multiple_probe() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let out = fixture.render("BBB={create_dynamic_country={original_tag=AAA copy_tag=BBB army_experience=2 add_political_power=3}}", &mut fresh(), Content::default());
    assert_eq!(out.text, "\u{12}BBBBeta:\n   XP2\n   PP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "add_doctrine_cost_reduction={technology=synthetic_tech army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP2\n");
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(out.refusals.keys().any(|why| {
        why.contains("key add_doctrine_cost_reduction:")
            && why
                .contains("technology synthetic_tech doctrine reduction description is unavailable")
    }));
}

#[test]
fn doctrine_technology_is_named_instead_of_silently_dropped() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let mut doctrines = content_reader_military::doctrines::Database::default();
    doctrines.entries.insert(
        b"land".to_vec(),
        content_reader_military::doctrines::Folder {
            name: b"FOLDER".to_vec(),
            refused: None,
        },
    );
    let out = fixture.render("add_doctrine_cost_reduction={technology=synthetic_tech category=land_doctrine cost_reduction=1} army_experience=2", &mut fresh(), Content { doctrines: Some(&doctrines), ..Default::default() });
    assert_eq!(out.text, "XP2\n");
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|why| why.contains("technology synthetic_tech"))
    );
}

#[test]
fn referenced_event_sole_title_uses_c_string_and_last_text_field() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let reference = |_: &[u8]| {
        let mut titles =
            content_reader_parser::parser::parse(b"title={text=TIP text=REFERENCE trigger={always=no}}")
                .unwrap()
                .items;
        let content_reader_parser::parser::Value::Block(fields) = &mut titles[0].value else {
            unreachable!()
        };
        fields[1].value = content_reader_parser::parser::Value::Scalar(b"REFERENCE\0ignored".to_vec());
        Ok(Arc::new(content_reader_events::events::Event {
            titles,
            ..Default::default()
        }))
    };
    let out = fixture.render(
        "country_event={id=synthetic.1}",
        &mut fresh(),
        Content {
            events: Some(&reference),
            ..Default::default()
        },
    );
    assert_eq!(out.text, "Event AAA");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn evaluator_opened_after_custom_text_shares_then_cleans_its_session() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let text = ScriptedText::new(
        &fixture.frame,
        &fixture.catalogue.localisation,
        fixture.catalogue.scripted_content().unwrap(),
        1,
    );
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic lazy session",
        Content::default(),
    );
    let mut graph = fresh();
    let body = content_reader_parser::parser::parse(b"custom_effect_tooltip=TIP if={limit={set_temp_variable={x=7}} custom_effect_tooltip=TIP} army_experience=temp_var:x").unwrap().items;
    let out = describe(&body, &context, &mut graph);
    assert_eq!(out.text, "tip\ntip\nXP7\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = describe(
        &content_reader_parser::parser::parse(b"army_experience=temp_var:x")
            .unwrap()
            .items,
        &context,
        &mut graph,
    );
    assert_eq!(out.text, "XP0\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn custom_only_completion_cleans_from_targets_without_world() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "synthetic no World".into(),
    };
    let text = ScriptedText::new(
        &fixture.frame,
        &fixture.catalogue.localisation,
        fixture.catalogue.scripted_content().unwrap(),
        1,
    );
    let context = Context::new(
        &text,
        &fixture.catalogue.localisation,
        1,
        "synthetic custom cleanup",
        Content::default(),
    );
    let mut graph = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            from: Link::Scope(2),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::State(2)),
    ]);
    graph
        .tooltip_targets
        .insert(1, vec![(b"root".to_vec(), EventObject::Country(1))]);
    graph
        .tooltip_targets
        .insert(2, vec![(b"from".to_vec(), EventObject::State(2))]);
    graph
        .global_tooltip_targets
        .push((b"global".to_vec(), EventObject::Country(1)));
    let out = describe(
        &content_reader_parser::parser::parse(b"custom_effect_tooltip=TIP")
            .unwrap()
            .items,
        &context,
        &mut graph,
    );
    assert_eq!(out.text, "tip");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    assert!(graph.tooltip_targets.contains_key(&1));
    assert!(!graph.tooltip_targets.contains_key(&2));
    assert!(graph.global_tooltip_targets.is_empty());
}

#[test]
fn generic_join_and_both_indent_operations_are_native() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    assert_eq!(
        indent_nonempty_lines("first\n\n \nlast", 2),
        "  first\n\n   \n  last"
    );
    assert_eq!(
        trim_description("\r\n \tvalue\t\n\u{a0}"),
        "value\t\n\u{a0}"
    );
    let fixture = Fixture::new();
    let out=fixture.render("custom_effect_tooltip=TINY effect_tooltip={custom_effect_tooltip=TIP hidden_effect={add_political_power=9} army_experience=2}",&mut fresh(),Content::default());
    assert_eq!(out.text, "tip\nXP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "BBB={add_political_power=1 army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta:\n   PP1\n   XP2\n");
    let out = fixture.render(
        "BBB={effect_tooltip={add_political_power=1 army_experience=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "\u{12}BBBBeta:\n   PP1\n   XP2\n");
    let out = fixture.render("AAA={army_experience=2}", &mut fresh(), Content::default());
    assert_eq!(out.text, "XP2\n");
    let mut state = EventScopes::new(vec![EventScope::of(EventObject::State(2))]);
    assert_eq!(
        fixture
            .render("AAA={army_experience=2}", &mut state, Content::default())
            .text,
        "XP2\n"
    );
    let mut foreign_root = EventScopes::new(vec![
        EventScope::of(EventObject::State(2)),
        EventScope::of(EventObject::Country(2)),
    ]);
    foreign_root.scopes[0].root = Link::Scope(1);
    assert_eq!(
        fixture
            .render(
                "AAA={army_experience=2}",
                &mut foreign_root,
                Content::default()
            )
            .text,
        "\u{12}AAAAlpha XP2\n"
    );
    let out = fixture.render(
        "BBB={AAA={army_experience=2}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP2\n");
    let out=fixture.render("BBB={AAA={if={limit={tag=BBB} add_political_power=1 army_experience=2} else={add_political_power=3}} army_experience=4}",&mut fresh(),Content::default());
    assert_eq!(out.text, "PP3\n   XP4\n");
}

#[test]
fn branches_share_one_evaluator_and_description_does_not_execute_setters() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let out=fixture.render("set_country_flag=x if={limit={has_country_flag=x} army_experience=9} ignored_invalid=1 else_if={limit={check_variable={num_owned_states>4} any_owned_state={state=2}} add_political_power=2} else={army_experience=8} clear_variable=x clr_country_flag=x clr_state_flag=x",
        &mut fresh(),Content::default());
    assert_eq!(out.text, "PP2\n");
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
}

#[test]
fn numeric_operands_use_recorded_variables_and_state_values_remain_unvalidated() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 100,
            seed: replay_data::rows::Recorded::Value(None),
            entries: vec![ScriptVariableEntry {
                name: b"bonus".to_vec(),
                value_raw: -123_456,
            }],
        }]));
    let result=fixture.render("add_political_power=bonus army_experience=num_owned_states transfer_state=999 add_state_claim=-1 start_border_war={attacker={state=999} defender={state=2}}",&mut fresh(),Content::default());
    assert_eq!(result.text, "PP-1\nXP5\nTransfer \nClaim \n");
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    let mut state = EventScopes::new(vec![EventScope::of(EventObject::State(2))]);
    assert_eq!(
        fixture
            .render("transfer_state=0", &mut state, Content::default())
            .text,
        "Transfer \n"
    );
    assert_eq!(
        fixture
            .render("transfer_state=-1", &mut state, Content::default())
            .text,
        "Transfer \n"
    );
}

#[test]
fn native_description_probes_repeat_condition_temp_writes_in_one_session() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let result=fixture.render("BBB={if={limit={add_to_temp_variable={n=1} always=yes} add_political_power=n army_experience=n}}",&mut fresh(),Content::default());
    assert_eq!(result.text, "\u{12}BBBBeta:\n   PP2\n   XP2\n");
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    let result=fixture.render("if={limit={set_temp_variable={n=3} always=no} army_experience=9} else={add_political_power=n} army_experience=n",&mut fresh(),Content::default());
    assert_eq!(result.text, "PP3\nXP3\n");
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
}

#[test]
fn every_state_lists_targets_but_describes_body_once_in_first_state() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    for (limit, expected) in [
        ("state=9", ""),
        ("state=2", "\u{11}YTwo\u{11}!: \n   Transfer Two\n"),
        (
            "NOT={state=5}",
            "(\u{11}YOne\u{11}!, \u{11}YTwo\u{11}!, \u{11}YThree\u{11}!, \u{11}YFour\u{11}!):\n   Transfer One\n",
        ),
        (
            "always=yes",
            "(\u{11}YOne\u{11}!, \u{11}YTwo\u{11}!, \u{11}YThree\u{11}! and 2 others):\n   Transfer One\n",
        ),
    ] {
        let out = fixture.render(
            &format!("every_state={{limit={{{limit}}} transfer_state=THIS}}"),
            &mut fresh(),
            Content::default(),
        );
        assert_eq!(out.text, expected, "{limit}");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn temporary_targets_use_root_ownership_from_lookup_and_exact_cleanup() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let out = fixture.render(
        "save_event_target_as=our event_target:our={army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let mut graph = EventScopes::new(vec![
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::State(2)),
    ]);
    graph.scopes[0].root = Link::Scope(1);
    graph.scopes[0].from = Link::Scope(2);
    let out = fixture.render(
        "save_event_target_as=our event_target:our={army_experience=2}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "");
    assert!(out.refusals.is_empty());
    let out = fixture.render(
        "ROOT={event_target:our={army_experience=3}}",
        &mut graph,
        Content::default(),
    );
    assert_eq!(out.text, "XP3\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let mut one = fresh();
    fixture.render("save_event_target_as=our", &mut one, Content::default());
    let out = fixture.render(
        "event_target:our={army_experience=2}",
        &mut one,
        Content::default(),
    );
    assert!(out.text.is_empty());
    assert!(out.refusals.is_empty());
}

#[test]
fn compatibility_absent_subjects_refuses_only_faction_effect() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *fixture.frame.tables[TableId::CountryAppearance.index()]
    {
        rows[1].subjects = replay_data::rows::Recorded::NotRecorded;
    }
    let out = fixture.render(
        "add_to_faction=BBB army_experience=2",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "XP2\n");
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .any(|reason| reason.contains("subjects of country appearance not recorded"))
    );
}

#[test]
fn reached_event_compositions_keep_scope_and_hidden_effect_behavior() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(vec![
            GlobalSavedEventTargetRow {
                name: b"synthetic_partner".to_vec(),
                country: 200,
                ..Default::default()
            },
        ]));
    let mut doctrines = content_reader_military::doctrines::Database::default();
    doctrines.entries.insert(
        b"land".to_vec(),
        content_reader_military::doctrines::Folder {
            name: b"FOLDER".to_vec(),
            refused: None,
        },
    );
    let file = content_reader_politics::ideas::parse_ideas(
        b"ideas={country={SYN_IDEA={name=SYN_IDEA}}}",
        &[],
        &Default::default(),
    );
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), file.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let reference = |_: &[u8]| {
        Ok(Arc::new(content_reader_events::events::Event {
            titles: content_reader_parser::parser::parse(b"title=REFERENCE")
                .unwrap()
                .items,
            ..Default::default()
        }))
    };
    let content = Content {
        doctrines: Some(&doctrines),
        ideas: Some(&ideas),
        name_lists: Some(&names),
        events: Some(&reference),
        ..Default::default()
    };
    let cases = [
        (
            "Greece36",
            "add_timed_idea={idea=SYN_IDEA days=10} declare_war_on={target=BBB} BBB={add_to_faction=AAA}",
            "Timed Spirit for 10 days ().\nWar Alpha/Beta\n\u{12}BBBBeta Join Alpha\n",
        ),
        (
            "Greece37",
            "add_stability=1 add_war_support=2 army_experience=3 hidden_effect={country_event={id=synthetic.1}}",
            "S1\nW2\nXP3\n",
        ),
        (
            "Greece38",
            "start_border_war={attacker={state=1} defender={state=2}} start_border_war={attacker={state=3} defender={state=4}}",
            "Border One/Two\nBorder Three/Four\n",
        ),
        ("Greece39", "add_stability=1 add_war_support=2", "S1\nW2\n"),
        (
            "Greece40",
            "add_timed_idea={idea=SYN_IDEA days=3} declare_war_on={target=BBB} hidden_effect={BBB={add_to_faction=AAA}}",
            "Timed Spirit for 3 days ().\nWar Alpha/Beta\n",
        ),
        (
            "Greece41",
            "add_stability=1 add_war_support=2 declare_war_on={target=BBB}",
            "S1\nW2\nWar Alpha/Beta\n",
        ),
        (
            "Greece42",
            "add_stability=-1 add_war_support=-2",
            "S-1\nW-2\n",
        ),
        (
            "Greece44",
            "declare_war_on={target=BBB} hidden_effect={BBB={declare_war_on={target=CCC}}}",
            "War Alpha/Beta\n",
        ),
        (
            "Greece45",
            "add_stability=1 add_political_power=2",
            "S1\nPP2\n",
        ),
        (
            "Greece46",
            "add_stability=1 army_experience=2 hidden_effect={country_event={id=synthetic.1}}",
            "S1\nXP2\n",
        ),
        (
            "Africa1",
            "hidden_effect={delete_unit_template_and_units={division_template=guard}} every_state={limit={state=1} transfer_state=THIS}",
            "\u{11}YOne\u{11}!: \n   Transfer One\n",
        ),
        (
            "Africa2",
            "hidden_effect={delete_unit_template_and_units={division_template=guard}} army_experience=2",
            "XP2\n",
        ),
        (
            "Indochina1",
            "hidden_effect={delete_unit_template_and_units={division_template=guard}} event_target:synthetic_partner={transfer_state=1}",
            "\u{12}BBBBeta Transfer One\n",
        ),
        (
            "Mexico40",
            "transfer_state=1 add_war_support=2 army_experience=3",
            "Transfer One\nW2\nXP3\n",
        ),
        (
            "Mexico42",
            "add_war_support=2 army_experience=3",
            "W2\nXP3\n",
        ),
        (
            "Slovakia1",
            "add_political_power=1 army_experience=2 add_mastery={amount=5 folder=land} add_doctrine_cost_reduction={cost_reduction=1 uses=2 category=land_doctrine}",
            "PP1\nXP2\nMastery 5 Land\nDoctrine Land/1/2 uses\n",
        ),
        (
            "Slovakia2",
            "add_political_power=-1 army_experience=3 add_mastery={amount=4 folder=land} add_doctrine_cost_reduction={cost_reduction=2 uses=1 category=land_doctrine}",
            "PP-1\nXP3\nMastery 4 Land\nDoctrine Land/2/1 uses\n",
        ),
        (
            "LostAttacker",
            "save_event_target_as=our set_country_flag=show add_political_power=-1 army_experience=2 if={limit={has_country_flag=show} annex_country=BBB}",
            "PP-1\nXP2\n",
        ),
        (
            "LostDefender",
            "hidden_effect={save_event_target_as=our} if={limit={check_variable={num_owned_states>9}} annex_country=BBB} else={add_state_claim=2}",
            "Claim Two\n",
        ),
        (
            "WonAttacker",
            "save_event_target_as=our add_political_power=1 if={limit={check_variable={num_owned_states>9}} annex_country=BBB} else={transfer_state=2}",
            "PP1\nTransfer Two\n",
        ),
        (
            "WonDefender",
            "hidden_effect={save_event_target_as=our} add_political_power=1 army_experience=2 add_doctrine_cost_reduction={cost_reduction=1 uses=1 category=land_doctrine} if={limit={always=yes} add_state_claim=2}",
            "PP1\nXP2\nDoctrine Land/1/1 uses\nClaim Two\n",
        ),
    ];
    for (name, body, expected) in cases {
        let out = fixture.render(body, &mut fresh(), content);
        assert_eq!(out.text, expected, "{name}");
        assert!(out.refusals.is_empty(), "{name}: {:?}", out.refusals);
    }
}

#[test]
fn native_leaf_conditions_and_missing_content_refusals_remain_local() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let out=fixture.render("annex_country=BBB add_state_claim=1 delete_unit_template_and_units={division_template=guard disband=yes disband=invalid} hidden_effect={add_equipment_to_stockpile={type=synthetic_missing amount=1}} army_experience=2",&mut fresh(),Content::default());
    assert_eq!(
        out.text,
        "Annex Alpha/Beta\nClaim One\nDisband guard\nXP2\n"
    );
    assert_eq!(out.refusals.values().sum::<usize>(), 1);
    assert!(
        out.refusals
            .keys()
            .next()
            .unwrap()
            .contains("add_equipment_to_stockpile")
    );
}

#[test]
fn timed_idea_repeats_native_target_prefix_and_floors_duration_components() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    let tokens = ["attack_bonus_against", "defense_bonus_against"];
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(
            tokens
                .iter()
                .enumerate()
                .map(|(at, literal)| LexicalTokenRow {
                    token: at as u32 + 1,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
        ));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(
            ["ATTACK", "DEFENSE"]
                .iter()
                .enumerate()
                .map(|(at, key)| ModifierDefinitionRow {
                    index: at as u32,
                    token: at as u32 + 1,
                    name_key: key.as_bytes().to_vec(),
                    category_mask: 32,
                    precision: 1,
                    flags: 3,
                    ..Default::default()
                })
                .collect(),
        ));
    let parsed=content_reader_politics::ideas::parse_ideas(b"ideas={country={SYN_IDEA={name=SYN_IDEA targeted_modifier={tag=BBB attack_bonus_against=0.25 defense_bonus_against=0.15}}}}",&[],&Default::default());
    let ideas = content_reader_politics::ideas::IdeaContent::from_parsed(Vec::new(), parsed.ideas);
    let names = content_reader_politics::names::NameLists::default();
    let content = Content {
        ideas: Some(&ideas),
        name_lists: Some(&names),
        ..Default::default()
    };
    let result = fixture.render(
        "add_timed_idea={idea=SYN_IDEA years=1.9 months=2.99 days=3.01}",
        &mut fresh(),
        content,
    );
    assert_eq!(
        result.text,
        "Timed Spirit for 1 years 2 months 3 days (\u{12}BBBBeta Attack: \u{11}G+25.0%\u{11}!, \u{12}BBBBeta Defense: \u{11}G+15.0%\u{11}!).\n"
    );
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    let result = fixture.render(
        "add_timed_idea={idea=SYN_IDEA years=-0.01 months=0.99 days=-1}",
        &mut fresh(),
        content,
    );
    assert!(result.text.starts_with("Timed Spirit for  ("));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] = TableView::Unavailable {
        reason: "synthetic fault".into(),
    };
    let result = fixture.render(
        "add_timed_idea={idea=SYN_IDEA days=1} army_experience=2",
        &mut fresh(),
        content,
    );
    assert_eq!(result.text, "XP2\n");
    assert_eq!(result.refusals.values().sum::<usize>(), 1);
}

#[test]
fn country_event_random_block_is_described_as_a_native_child() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let names = content_reader_script::key_names::KeyNames::default();
    let reference = |_: &[u8]| {
        Ok(Arc::new(content_reader_events::events::Event {
            titles: content_reader_parser::parser::parse(b"title=TIP").unwrap().items,
            ..Default::default()
        }))
    };
    let content = Content {
        key_names: Some(&names),
        events: Some(&reference),
        ..Default::default()
    };
    let output = fixture.render_native(
        "country_event={id=synthetic.1 random=3 random={chance=100 army_experience=2}}",
        &mut fresh(),
        content,
    );
    assert_eq!(output.text, "Event tip\n100%: XP2\n");
    assert!(output.refusals.is_empty(), "{:?}", output.refusals);
}

#[test]
fn country_event_uses_title_only_fresh_from_root_and_advances_seed_lane() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let fixture = Fixture::new();
    let reference = |_: &[u8]| {
        Ok(Arc::new(content_reader_events::events::Event {
            titles: content_reader_parser::parser::parse(b"title=REFERENCE")
                .unwrap()
                .items,
            options: vec![
                content_reader_parser::parser::parse(b"add_political_power=999")
                    .unwrap()
                    .items,
            ],
            ..Default::default()
        }))
    };
    let mut graph = fresh();
    let result = fixture.render(
        "country_event={id=synthetic.1}",
        &mut graph,
        Content {
            events: Some(&reference),
            ..Default::default()
        },
    );
    assert_eq!(result.text, "Event AAA");
    assert_eq!(graph.scopes[0].random_seed, 2);
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    let result = fixture.render(
        "BBB={country_event={id=synthetic.1 desc=TIP}}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(result.text, "tip\n");
    let result = fixture.render(
        "BBB={country_event={id=synthetic.1 desc=\"\"}}",
        &mut fresh(),
        Content {
            events: Some(&reference),
            ..Default::default()
        },
    );
    assert_eq!(result.text, "\u{12}BBBBeta Event AAA");
    let result=fixture.render("save_event_target_as=our if={limit={has_event_target=our} army_experience=9} else={army_experience=2}",&mut fresh(),Content::default());
    assert_eq!(result.text, "XP2\n");
}

#[test]
fn wtt_shaped_immediate_and_option_keep_marker_links_and_recorded_flag_conditions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    *fixture.frame.tables[TableId::Clock.index()] =
        TableView::Complete(TableRows::Clock(vec![ClockRow::default()]));
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *fixture.frame.tables[TableId::Politics.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 100)
            .unwrap()
            .ruling_party = Some(1);
    }
    *fixture.frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 1,
            token: b"group".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 9,
            literal: b"show".to_vec(),
        }]));
    *fixture.frame.tables[TableId::ScriptFlags.index()] =
        TableView::Complete(TableRows::ScriptFlags(vec![ScriptFlagRow {
            scope_kind: 1,
            scope_id: 100,
            token: 9,
            value: 1,
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template: 1,
            owner: 100,
            name: b"Guard".to_vec(),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::Decisions.index()] =
        TableView::Complete(TableRows::Decisions(vec![DecisionRow {
            country: 100,
            container: 3,
            token: b"SYN_DECISION".to_vec(),
            country_target: 200,
            description_seed: replay_data::rows::Recorded::Value(Some(None)),
            ..Default::default()
        }]));
    let mut database = content_reader_politics::decisions::Database::default();
    database.entries.insert(
        b"syn_decision".to_vec(),
        content_reader_politics::decisions::Decision {
            id: b"SYN_DECISION".to_vec(),
            name: b"SYN_DECISION".to_vec(),
            seed_component: content_reader_politics::decisions::seed_component(b"SYN_DECISION"),
            has_state_targets: true,
            target_states: vec![b"2".to_vec()],
            ..Default::default()
        },
    );
    let content = Content {
        decisions: Some(&database),
        ..Default::default()
    };
    let mut graph = EventScopes::new(vec![
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::Country(1)),
        EventScope::of(EventObject::State(1)),
        EventScope::of(EventObject::Country(2)),
        EventScope::of(EventObject::Country(2)),
        EventScope::of(EventObject::State(2)),
    ]);
    graph.scopes[0].root = Link::Scope(1);
    graph.scopes[0].from = Link::Scope(2);
    graph.scopes[2].from = Link::Scope(3);
    graph.scopes[3].root = Link::Scope(4);
    graph.scopes[3].from = Link::Scope(5);
    let immediate = "save_event_target_as=our FROM={save_event_target_as=our_state} FROM.FROM={save_event_target_as=their} FROM.FROM.FROM={save_event_target_as=their_state} set_country_flag=show remove_targeted_decision={target=FROM.FROM decision=SYN_DECISION}";
    let result = fixture.render(immediate, &mut graph, content);
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    assert_eq!(result.text, "Remove Conflict in Two\n");
    let option = "if={limit={has_country_flag=show has_template=Guard any_owned_state={state=FROM.FROM.FROM}} effect_tooltip={transfer_state=FROM.FROM.FROM add_state_claim=FROM.FROM.FROM}} else={annex_country=FROM.FROM}";
    let result = fixture.render(option, &mut graph, content);
    assert_eq!(result.text, "Transfer Two\nClaim Two\n");
    assert!(result.refusals.is_empty(), "{:?}", result.refusals);
    *fixture.frame.tables[TableId::ScriptFlags.index()] =
        TableView::Complete(TableRows::ScriptFlags(Vec::new()));
    let result = fixture.render(option, &mut graph, content);
    assert_eq!(result.text, "Annex Alpha/Beta\n");
}

#[test]
fn faction_names_in_effect_descriptions_use_bound_template_names_and_literals() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut fixture = Fixture::new();
    let (catalogue, _folder) = crate::fixtures::folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/factions/templates/test.txt",
            "demo={name=DEMO_NAME}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n BBB:0 \"Beta\"\n",
                " EFFECT_ADD_TO_FACTION_NAME:0 \"Join $COUNTRY$/$FACTION$\"\n",
                " demo:0 \"Wrong token name\"\n DEMO_NAME:0 \"Template pact\"\n",
                " Literal:0 \"Wrong literal translation\"\n",
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            template_token: Some(17),
            ..Default::default()
        }]));
    *fixture.frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            country: 100,
            faction: 5,
            ordinal: 0,
        }]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 17,
            literal: b"demo".to_vec(),
        }]));
    let out = fixture.render("add_to_faction=BBB", &mut fresh(), Content::default());
    assert_eq!(out.text, "Join Beta/Template pact\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *fixture.frame.tables[TableId::Factions.index()]
    {
        rows[0].name = b"Literal".to_vec();
        rows[0].template_token = None;
    }
    *fixture.frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    let out = fixture.render("add_to_faction=BBB", &mut fresh(), Content::default());
    assert_eq!(out.text, "Join Beta/Literal\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    Arc::make_mut(&mut fixture.catalogue.localisation).language = "french".into();
    let out = fixture.render("add_to_faction=BBB", &mut fresh(), Content::default());
    assert!(out.refusals.keys().any(|line| line.contains(
        "faction name is text the game stored in english without its key, and the viewer's language is french"
    )), "{:?}", out.refusals);
}

#[path = "recorded_source_tests.rs"]
mod recorded_source;

#[test]
fn reads_alongside_reach_this_threads_consumers_once_joined() {
    let refused = BTreeSet::from(["artwork.dds: refused".to_owned()]);
    let (seen, noted) = crate::catalogue::capture(|| {
        std::thread::scope(|threads| {
            let read = alongside(threads, || {
                content_reader_layers::content::credit_overlay_refusals(&refused);
                7
            });
            joined(read)
        })
    });
    assert_eq!(seen, 7);
    assert!(
        noted
            .fallback_line()
            .is_some_and(|line| line.contains("artwork.dds: refused")),
        "{noted:?}"
    );
}
