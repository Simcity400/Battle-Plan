use super::*;
use crate::test_fixtures::{CAMPAIGN, records};
use replay::{
    frame::TableView,
    rows::{CountryLinkRow, CountryRow, LexicalTokenRow, TableRows},
};

fn completions(
    replay: &Replay,
    branch: u64,
    content: Option<&mut Content>,
    cancelled: &AtomicBool,
) -> io::Result<ProjectCompletions> {
    let scan = history::BranchScan::scan(replay, branch, cancelled)?;
    scan_completions(replay, &scan, branch, content, cancelled)
}
fn country(
    replay: &Replay,
    params: &CountryParams,
    content: Option<&mut Content>,
    cancelled: &AtomicBool,
) -> io::Result<SpecialProjectsReply> {
    let scan = history::BranchScan::scan(replay, params.branch(), cancelled)?;
    project(&scan, params, None, content, cancelled).map(|(reply, _)| reply)
}

fn frame(hour: i32, read: bool) -> DecodedFrame {
    let fixture = super::super::tests::frame(hour, read, 1);
    // Start with the shared encodable fixture. Keep only the project's known
    // observations; unrelated tables are unavailable, never invented empty.
    let mut frame = crate::test_fixtures::source(1, hour, None);
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Projects,
        TableId::ProjectPrograms,
        TableId::ProjectHistory,
        TableId::Characters,
        TableId::CharacterPortraits,
        TableId::CountryAppearance,
    ] {
        frame.tables[table.index()] = fixture.tables[table.index()].clone();
    }
    frame.metrics = fixture.metrics;
    *frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![replay::rows::CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        }]));
    frame
}

fn complete(frame: &mut DecodedFrame, exact: Option<i32>) {
    let mut rows = frame.complete::<tables::Projects>().unwrap().to_vec();
    for row in &mut rows {
        row.state.current_offset = 0xe8;
    }
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(rows));
    *frame.tables[TableId::ProjectHistory.index()] = match exact {
        Some(date) => TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
            owner: 5,
            project: 0,
            prototype_reward_handle: COMPLETION_MARKER,
            option_handle: COMPLETION_MARKER,
            date,
            ..Default::default()
        }])),
        None => TableView::Unavailable {
            reason: "not recorded".into(),
        },
    };
}

#[test]
fn completion_scan_agrees_with_country_history_exact_and_bounded_dates() {
    for exact in [None, Some(101)] {
        let first = frame(100, true);
        let middle = frame(101, false);
        let mut last = frame(102, true);
        complete(&mut last, exact);
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, middle, last])])
                .unwrap();
        let cancelled = AtomicBool::new(false);
        let params = CountryParams {
            branch_id: "1".into(),
            tag: "mod tag".into(),
        };
        let country = country(&replay, &params, None, &cancelled).unwrap();
        let feed = completions(&replay, 1, None, &cancelled).unwrap();
        assert_eq!(feed.events.len(), 1);
        let event = &feed.events[0];
        assert_eq!(
            Some(event.date.clone()),
            country.projects[0].runs.last().unwrap().completion
        );
        assert_eq!(event.observed_hour, 102);
        assert_eq!(event.item_token.as_deref(), Some("mod_project"));
        assert!(feed.unavailable.is_none());
    }
}

#[test]
fn completion_scan_country_rename_keeps_episode_and_names_native_hour_identity() {
    let first = frame(100, true);
    let middle = frame(101, false);
    let mut renamed = frame(102, true);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed tag".to_vec();
    }
    complete(&mut renamed, Some(101));
    let mut later = renamed.clone();
    later.hour = 103;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first, middle, renamed, later])],
    )
    .unwrap();
    let feed = completions(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(feed.events.len(), 1);
    assert_eq!(feed.events[0].tag, "mod tag");
    assert_eq!(
        feed.events[0].date,
        ProjectDate {
            hour: 101,
            exact: true
        }
    );
    assert_eq!(feed.events[0].observed_hour, 102);

    let first = frame(100, true);
    let mut after_gap = frame(103, true);
    complete(&mut after_gap, Some(101));
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, after_gap])]).unwrap();
    let feed = completions(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert!(feed.events.is_empty());
    assert!(feed.refusals.iter().any(|(reason, count)| reason
        == "Project completion hour was not recorded."
        && *count == 1));
}

#[test]
fn completion_presentation_keeps_each_native_hour_when_one_read_learns_several_dates() {
    let mut first = frame(100, true);
    let project = first.complete::<tables::Projects>().unwrap()[0].clone();
    *first.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
        (0..3)
            .map(|local_id| ProjectRow {
                local_id,
                ..project.clone()
            })
            .collect(),
    ));
    let mut frames = vec![first.clone()];
    for hour in 101..=104 {
        let mut next = first.clone();
        next.hour = hour;
        next.metrics.table_ns.fill(0);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].tag = format!("native hour {}", hour.min(103)).into_bytes();
        }
        if hour == 104 {
            next.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
            complete(&mut next, None);
            *next.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(
                    (0..3)
                        .map(|project| ProjectHistoryRow {
                            owner: 5,
                            project,
                            prototype_reward_handle: COMPLETION_MARKER,
                            option_handle: COMPLETION_MARKER,
                            date: 101 + project as i32,
                            ..Default::default()
                        })
                        .collect(),
                ));
        }
        frames.push(next);
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let feed = completions(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        feed.events
            .iter()
            .map(|event| (
                event.tag.as_str(),
                event.date.hour,
                event.date.exact,
                event.observed_hour,
                event.item_token.as_deref(),
            ))
            .collect::<Vec<_>>(),
        [
            ("native hour 101", 101, true, 104, Some("mod_project")),
            ("native hour 102", 102, true, 104, Some("mod_project")),
            ("native hour 103", 103, true, 104, Some("mod_project")),
        ],
    );
    assert!(feed.refusals.is_empty(), "{:?}", feed.refusals);
}

#[test]
fn completion_scripted_dependencies_follow_native_aliases_and_country_name_keys() {
    let guard = super::super::tests::TempRoot::new("feed-dependencies");
    let file = guard.0.join("localisation/project_l_english.yml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        concat!(
            "\u{feff}l_english:\n plain_name:0 \"Plain\"\n",
            " scoped_name:0 \"$scoped_alias$\"\n",
            " arbitrary_country_prefixed_name:0 \"$scoped_alias$\"\n",
            " scoped_alias:0 \"[ROOT.GetTag]\"\n",
            " unrelated_name:0 \"[ROOT.GetTag]\"\n",
        ),
    )
    .unwrap();
    let layers = installed_content::ResolvedLayers::from_folders(&[(&guard.0, &[][..])]).unwrap();
    let localisation = installed_content::localisation::read(&layers, "english").unwrap();
    let mut definitions = Catalog::default();
    for (name, expected) in [
        ("plain_name", false),
        ("scoped_name", true),
        ("prefixed_name", true),
    ] {
        let mut project = installed_content::special_projects::Project::default();
        project.name = Some(name.into());
        definitions.projects.insert("mod_project".into(), project);
        assert_eq!(
            scripted_names(&definitions, &localisation),
            expected,
            "{name}"
        );
    }
}

#[test]
fn completion_refusal_kinds_preserve_offsets_and_content_names() {
    assert_eq!(
        structural_reason("Project 73 has unsupported native state offset 0x123"),
        structural_reason("Project 91 has unsupported native state offset 0x123"),
    );
    assert_ne!(
        structural_reason("Project 73 has unsupported native state offset 0x123"),
        structural_reason("Project 73 has unsupported native state offset 0x456"),
    );
    for reason in [
        "Definition token 81 is not recorded",
        "Project name mod_name could not be resolved",
        "Project definition mod_project is absent from content",
        "Project mod_project: specialization mod_field is absent from content",
        "mod_project: unsupported icon object form",
    ] {
        assert_eq!(structural_reason(reason), reason);
    }
}

#[test]
fn completion_world_scan_skips_carried_views_and_reprojects_each_dependency() {
    let mut sample = frame(100, true);
    let mut world = WorldBuilder::default();
    world.observe(&sample);
    assert_eq!(world.projected_frames, 1);
    sample.metrics.table_ns.fill(0);
    for hour in 101..=103 {
        sample.hour = hour;
        world.observe(&sample);
    }
    assert_eq!(world.projected_frames, 1);
    assert_eq!(world.countries[&(0, 5)].last, Some(((0, 5), 103)));
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Projects,
        TableId::ProjectHistory,
    ] {
        sample.hour += 1;
        sample.tables[table.index()] = (*sample.tables[table.index()]).clone().into();
        world.observe(&sample);
    }
    assert_eq!(world.projected_frames, 5);
    sample.hour += 2;
    world.observe(&sample);
    assert_eq!(world.projected_frames, 6);
    assert!(!world.countries[&(0, 5)].admitted);
    sample.hour += 1;
    sample.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
    world.observe(&sample);
    assert_eq!(world.projected_frames, 7);
    assert!(world.countries[&(0, 5)].admitted);
}

#[test]
fn completion_world_scan_keeps_only_last_progress_run_per_country() {
    let mut sample = frame(100, true);
    *sample.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=3)
            .map(|id| CountryRow {
                id,
                canonical_slot: id - 1,
                tag: format!("opaque country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *sample.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=3)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id - 1,
            })
            .collect(),
    ));
    *sample.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
        (1..=3)
            .map(|owner| ProjectRow {
                owner,
                definition_id: 8,
                local_id: 1,
                started: 1,
                state: replay::rows::ProjectState {
                    current_offset: 0x18,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
    ));
    let mut world = WorldBuilder::default();
    world.observe(&sample);
    sample.hour = 101;
    let mut projects = sample.complete::<tables::Projects>().unwrap().to_vec();
    for project in &mut projects {
        project.state.prototype_progress = 73;
    }
    *sample.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(projects));
    world.observe(&sample);
    assert_eq!(world.countries.len(), 3);
    assert!(world.countries.values().all(|builder| {
        builder.episodes.len() == 1
            && builder.episodes[0].runs.len() == 1
            && builder.presentations.is_empty()
            && builder.people.is_empty()
    }));
    sample.hour = 102;
    complete(&mut sample, None);
    world.observe(&sample);
    assert!(world.countries.values().all(|builder| {
        builder.completions.len() == 1
            && builder.completions[&0].date
                == ProjectDate {
                    hour: 102,
                    exact: false,
                }
    }));
}

#[test]
fn completion_scan_gaps_identity_changes_and_history_reset_match_country_builder() {
    let mut world = WorldBuilder::default();
    let mut country = Builder::default();
    let mut sample = frame(100, true);
    for hour in [100, 101, 110, 111] {
        sample.hour = hour;
        if hour == 101 {
            *sample.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                    owner: 5,
                    project: 0,
                    ..Default::default()
                }]));
        }
        if hour == 110 {
            *sample.tables[TableId::ProjectHistory.index()] = TableView::Unavailable {
                reason: "not recorded".into(),
            };
        }
        if hour == 111 {
            *sample.tables[TableId::ProjectHistory.index()] =
                TableView::Complete(TableRows::ProjectHistory(Vec::new()));
        }
        world.observe(&sample);
        country.observe(&sample, "mod tag", None);
        assert_eq!(
            world.countries[&(0, 5)].episodes.len(),
            country.episodes.len()
        );
    }
    assert_eq!(country.episodes.len(), 2);
    sample.hour = 112;
    complete(&mut sample, None);
    world.observe(&sample);
    country.observe(&sample, "mod tag", None);
    let builder = &world.countries[&(0, 5)];
    assert_eq!(
        builder.completions[&1].date,
        country.episodes[1]
            .runs
            .last()
            .unwrap()
            .completion
            .clone()
            .unwrap()
    );
    assert_eq!(builder.completions.len(), 1);
    *sample.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 6,
            canonical_slot: 0,
            tag: b"mod tag".to_vec(),
            ..Default::default()
        }]));
    *sample.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 6,
            canonical_slot: 0,
        }]));
    let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
    project.owner = 6;
    *sample.tables[TableId::Projects.index()] =
        TableView::Complete(TableRows::Projects(vec![project]));
    sample.hour = 113;
    world.observe(&sample);
    country.observe(&sample, "mod tag", None);
    assert_eq!(
        world
            .countries
            .values()
            .map(|builder| builder.episodes.len())
            .sum::<usize>(),
        country.episodes.len()
    );
    assert_eq!(
        world
            .countries
            .values()
            .map(|builder| builder.completions.len())
            .sum::<usize>(),
        2
    );
}

#[test]
fn completion_scan_refuses_a_reassigned_country_at_its_completion_hour() {
    let first = frame(100, true);
    let mut reassigned = frame(101, false);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reassigned.tables[TableId::Countries.index()]
    {
        rows[0].id = 6;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *reassigned.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 6,
            canonical_slot: 0,
        });
    }
    let mut last = frame(102, true);
    complete(&mut last, Some(101));
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![first, reassigned, last])],
    )
    .unwrap();
    let feed = completions(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert!(feed.events.is_empty());
    assert!(feed.refusals.iter().any(|(reason, count)| reason
        == "Project country identity could not be resolved at its completion hour."
        && *count == 1));
}

#[test]
fn completion_projection_uses_mod_override_name_and_native_art_path() {
    let guard = super::super::tests::TempRoot::new("feed-mod");
    let base = guard.0.join("base");
    let over = guard.0.join("mod");
    for (layer, path, body) in [
        (
            &base,
            "common/defines/test.lua",
            "NDefines = { NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = {} } }",
        ),
        (
            &base,
            "common/special_projects/projects/test.txt",
            "mod_project={icon=GFX_base_project}",
        ),
        (
            &base,
            "localisation/english/project_l_english.yml",
            "\u{feff}l_english:\n mod_project:0 \"Original name\"\n",
        ),
        (
            &over,
            "common/special_projects/projects/test.txt",
            "mod_project={icon=GFX_mod_project narrative={name=mod_project_name}}",
        ),
        (
            &over,
            "localisation/english/project_l_english.yml",
            "\u{feff}l_english:\n mod_project_name:0 \"Mod completion name\"\n",
        ),
        (
            &over,
            "interface/mod.gfx",
            "spriteTypes={spriteType={name=GFX_mod_project texturefile=\"gfx/mod_project.png\"}}",
        ),
    ] {
        let file = layer.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    let art = over.join("gfx/mod_project.png");
    std::fs::create_dir_all(art.parent().unwrap()).unwrap();
    let file = std::fs::File::create(&art).unwrap();
    let mut encoder = png::Encoder::new(file, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[41, 73, 99, 255])
        .unwrap();
    let layers =
        installed_content::ResolvedLayers::from_folders(&[(&base, &[][..]), (&over, &[][..])]).unwrap();
    let catalogue = Arc::new(Catalogue::of_layers(layers, "english").unwrap());
    let mut content = Content {
        definitions: Catalog::read(&catalogue.content.layers).unwrap(),
        politics: Arc::new(PoliticsContent::load(Arc::clone(&catalogue)).unwrap()),
        catalogue,
        icons: BTreeMap::new(),
        scoped: Vec::new(),
        refusals: Mutex::default(),
    };
    let mut sample = frame(100, true);
    complete(&mut sample, None);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![sample])]).unwrap();
    let cancelled = AtomicBool::new(false);
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "mod tag".into(),
    };
    let country = country(&replay, &params, Some(&mut content), &cancelled).unwrap();
    let feed = completions(&replay, 1, Some(&mut content), &cancelled).unwrap();
    let event = &feed.events[0];
    assert_eq!(event.name.as_deref(), Some("Mod completion name"));
    assert_eq!(event.name.as_ref(), Some(&country.presentations[0].name));
    assert_eq!(event.icon, country.presentations[0].icon);
    // The mod replaces the same native relative file. A separately named
    // mod.txt would read before test.txt, whose later body restores base art
    // while retaining the narrative (140141670/140A85D70).
    assert!(event.icon_missing.is_none(), "{:?}", event.icon_missing);
    assert!(
        event
            .icon
            .as_deref()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    assert_eq!(
        crate::catalogue::texture_tests::from_url(event.icon.as_deref().unwrap()).pixels[0..4],
        [41, 73, 99, 255]
    );
}

#[test]
fn completion_compact_history_stays_constant_over_long_changing_progress() {
    let mut world = WorldBuilder::default();
    let mut sample = frame(100, true);
    for offset in 0..1000 {
        sample.hour = 100 + offset;
        let mut project = sample.complete::<tables::Projects>().unwrap()[0].clone();
        project.state.prototype_progress = offset % 100;
        *sample.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![project]));
        world.observe(&sample);
    }
    let builder = &world.countries[&(0, 5)];
    assert_eq!(builder.episodes.len(), 1);
    assert_eq!(builder.episodes[0].runs.len(), 1);
    assert_eq!(builder.available.len(), 1);
    assert!(builder.reads.is_empty());
    assert!(builder.completions.is_empty());
    assert!(builder.iteration_texts.is_empty());
}
