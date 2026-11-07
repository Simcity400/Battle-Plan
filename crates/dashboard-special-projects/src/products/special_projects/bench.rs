//! Offline scan timing; corpus files are read-only and timing excludes setup.
use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::ProjectProgramRow,
    rows::TableRows,
};
use std::time::Instant;

#[test]
#[ignore = "requires BATTLE_PLAN_SP_GAME read-only content"]
fn vanilla_scoped_name_audit() {
    let folder = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_SP_GAME").or_else(|| std::env::var_os("HOI4_SP_GAME")).expect("BATTLE_PLAN_SP_GAME"));
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(&folder, &[][..])]).unwrap();
    let definitions = Catalog::read(&layers).unwrap();
    let loc = installed_content_localisation::localisation::read(&layers, "english").unwrap();
    let names = layers.key_names().unwrap();
    let mut scoped = std::collections::BTreeSet::new();
    for (token, definition) in &definitions.projects {
        let key = definition
            .name
            .as_deref()
            .filter(|key| !key.is_empty())
            .unwrap_or(token);
        if needs_scope(&loc, key) {
            scoped.insert(key.to_owned());
        }
        for tag in &names.countries {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let prefixed = format!("{tag}_{key}");
            if loc.get(&prefixed).is_some() && needs_scope(&loc, &prefixed) {
                scoped.insert(prefixed);
            }
        }
    }
    println!(
        "vanilla: {} definitions, {} country tags, {} scoped name keys: {scoped:?}",
        definitions.projects.len(),
        names.countries.len(),
        scoped.len()
    );
}

#[test]
#[ignore = "requires BATTLE_PLAN_SP_PART_Q and constructs a ten-year offline recording"]
fn offline_scan_timings() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let cancelled = AtomicBool::new(false);
    let recording_path = std::env::var_os("BATTLE_PLAN_SP_PART_Q").or_else(|| std::env::var_os("HOI4_SP_PART_Q")).expect("BATTLE_PLAN_SP_PART_Q");
    let started = Instant::now();
    match Replay::open(std::fs::read(recording_path).unwrap()) {
        Ok(replay) => {
            let branch = replay.index[0].branch;
            let params = CountryParams {
                branch_id: branch.to_string(),
                tag: std::env::var("BATTLE_PLAN_SP_TAG").or_else(|_| std::env::var("HOI4_SP_TAG")).expect("BATTLE_PLAN_SP_TAG"),
            };
            let started = Instant::now();
            let scan = history::BranchScan::scan(&replay, params.branch(), &cancelled).unwrap();
            let reply = project(&scan, &params, None, None, &cancelled).unwrap().0;
            println!(
                "recording scan: {:?}; {} episodes",
                started.elapsed(),
                reply.projects.len()
            );
        }
        Err(error) => println!(
            "recording refused after {:?}: {error}; scan unavailable",
            started.elapsed()
        ),
    }

    let guard = tests::TempRoot::new("bench");
    let folder = &guard.0;
    for (path, body) in [
        (
            "common/special_projects/projects/mod.txt",
            "mod_project={specialization=mod_kind}",
        ),
        (
            "common/special_projects/specialization/mod.txt",
            "mod_kind={color={1 2 3}}",
        ),
        (
            "common/defines/mod.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} } }",
        ),
        (
            "localisation/english/project_l_english.yml",
            "\u{feff}l_english:\n mod_project:0 \"Mod project\"\n mod_kind:0 \"Mod science\"\n ONGOING_PROJECT_BREAKTHROUGHS_NUMBER:0 \"Trial count: $NUMBER$\"\n",
        ),
    ] {
        let file = folder.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    let layers = installed_content_core::ResolvedLayers::from_folders(&[(folder, &[][..])]).unwrap();
    let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
    let mut content = Content {
        definitions: Arc::new(Catalog::read(&catalogue.content.layers).unwrap()),
        politics: Arc::new(PoliticsContent::load(Arc::clone(&catalogue)).unwrap()),
        catalogue,
        icons: BTreeMap::new(),
        scoped: Vec::new(),
        refusals: Mutex::default(),
    };
    let countries = 1500_u32;
    let hours = 10 * 365 * 24;
    let mut sample = source(1, 100, None);
    for table in [
        TableId::ProjectHistory,
        TableId::Characters,
        TableId::CharacterPortraits,
        TableId::CountryAppearance,
    ] {
        *sample.tables[table.index()] =
            replay_frame::frame::TableView::Complete(TableRows::empty(table));
    }
    *sample.tables[TableId::Countries.index()] =
        replay_frame::frame::TableView::Complete(TableRows::Countries(
            (1..=countries)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id - 1,
                    tag: format!("opaque country {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
    *sample.tables[TableId::CountryLinks.index()] =
        replay_frame::frame::TableView::Complete(TableRows::CountryLinks(
            (1..=countries)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id - 1,
                })
                .collect(),
        ));
    *sample.tables[TableId::ProjectPrograms.index()] =
        replay_frame::frame::TableView::Complete(TableRows::ProjectPrograms(
            (1..=countries)
                .map(|id| ProjectProgramRow {
                    id,
                    owner: id,
                    scientist: 0,
                    ..Default::default()
                })
                .collect(),
        ));
    *sample.tables[TableId::LexicalTokens.index()] =
        replay_frame::frame::TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        }]));
    let mut frames = Vec::with_capacity(hours as usize);
    for offset in 0..hours {
        sample.hour = 100 + offset;
        sample.metrics.table_ns[TableId::ProjectPrograms.index()] = u64::from(offset % 24 == 0);
        if offset % 24 == 0 {
            *sample.tables[TableId::Projects.index()] =
                replay_frame::frame::TableView::Complete(TableRows::Projects(
                    (1..=countries)
                        .map(|owner| ProjectRow {
                            owner,
                            local_id: owner,
                            definition_id: 8,
                            program_reference: (u64::from(owner) << 32) | 83,
                            started: 1,
                            state: replay_data::rows::ProjectState {
                                current_offset: 0x18,
                                not_completed: 1,
                                prototype_progress: offset / 24 % 100,
                                ..Default::default()
                            },
                            ..Default::default()
                        })
                        .collect(),
                ));
        }
        frames.push(sample.clone());
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "opaque country 1500".into(),
    };
    for repetition in 1..=3 {
        let started = Instant::now();
        let scan = history::BranchScan::scan(&replay, params.branch(), &cancelled).unwrap();
        let reply = project(&scan, &params, None, Some(&mut content), &cancelled)
            .unwrap()
            .0;
        let elapsed = started.elapsed();
        assert_eq!(reply.projects.len(), 1);
        assert_eq!(reply.projects[0].runs.len(), 3650);
        assert_eq!(reply.presentations.len(), 1);
        assert_eq!(reply.people.len(), 1);
        assert_eq!(reply.iteration_texts.len(), 1);
        println!(
            "synthetic scan {repetition}: {elapsed:?}; {hours} hours, {countries} countries, one running project each; {} bytes; {} runs",
            replay.bytes().len(),
            reply.projects[0].runs.len()
        );
    }
}
