use super::*;
use dashboard_base::{
    prepared::{coverage::Coverage, prefix::Prefix, series::tests::Fixture},
    test_fixtures::{self, world_frame},
};
use dashboard_keyed::{Observer, Scan, Values, query as keyed};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryRow, DoctrineCountryRow, DoctrineFolderRow, DoctrineTrackRow,
        LexicalTokenRow, TableId, TableRows,
    },
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = world_frame(hour, false);
    *frame.tables[TableId::DoctrineCountries.index()] =
        TableView::Complete(TableRows::DoctrineCountries(vec![DoctrineCountryRow {
            slot: 7,
            country: 11,
            expiry_events: 9,
        }]));
    *frame.tables[TableId::DoctrineFolders.index()] =
        TableView::Complete(TableRows::DoctrineFolders(vec![DoctrineFolderRow {
            slot: 7,
            ordinal: 2,
            folder: 10,
            grand_doctrine: Some(11),
            completed_subdoctrines: vec![12, 12],
        }]));
    *frame.tables[TableId::DoctrineTracks.index()] =
        TableView::Complete(TableRows::DoctrineTracks(vec![DoctrineTrackRow {
            slot: 7,
            folder: 2,
            ordinal: 0,
            sub_doctrine: Some(12),
            rewards: 2,
            mastery: i64::MAX,
            mastery_bank: -17,
            daily_mastery: 123,
            effective_daily_mastery: -456,
        }]));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (10, b"mod_folder".as_slice()),
            (11, b"mod_grand"),
            (12, b"mod_sub"),
        ]
        .into_iter()
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        })
        .collect(),
    ));
    frame
}

fn publish(fixture: &Fixture, frames: &[DecodedFrame]) -> Scan<DoctrineMasteryObserver> {
    let mut scan = Scan::default();
    let mut writer = fixture.writer();
    for frame in frames {
        scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
    }
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    scan
}

fn source(frames: Vec<DecodedFrame>) -> dashboard_base::Source {
    dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "doctrine-test".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(
            replay::container::Replay::from_records(
                test_fixtures::CAMPAIGN.into(),
                &[test_fixtures::records(1, frames)],
            )
            .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap()
}

fn prefix(first: i32, last: i32, complete: bool) -> Prefix {
    Prefix {
        coverage: Coverage {
            schema: 1,
            first_hour: first,
            last_hour: last,
            complete,
        },
        pending: Vec::new(),
    }
}

fn ask(
    source: &dashboard_base::Source,
    fixture: &Fixture,
    params: &Value,
    prefix: &Prefix,
    content: Option<&dashboard_core::config::ContentOptions>,
) -> Value {
    query(&Query {
        source,
        params,
        content,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix,
        cancelled: &AtomicBool::new(false),
    })
    .unwrap()
}

fn runs(window: &KeyedWindow, name: &str) -> Vec<(i32, i32, String, bool)> {
    window
        .series
        .iter()
        .find(|series| series.name == name)
        .map(|series| series.runs.clone())
        .unwrap_or_default()
}

#[test]
fn exact_values_source_rate_bank_and_selections_survive_flush_resume() {
    let fixture = Fixture::new();
    publish(&fixture, &[frame(100)]);
    let mut scan =
        dashboard_base::prepared::series::read_catalogue::<Scan<DoctrineMasteryObserver>>(
            &fixture.store,
            &fixture.key,
            "doctrine-mastery",
            1,
        )
        .unwrap()
        .unwrap()
        .1;
    let original_tokens = scan.observer.tokens.len();
    let mut next = frame(101);
    if let TableView::Complete(TableRows::DoctrineTracks(rows)) =
        &mut *next.tables[TableId::DoctrineTracks.index()]
    {
        rows[0].mastery = 0;
        rows[0].sub_doctrine = None;
    }
    let mut writer = fixture.writer();
    scan.prepare(FormatVersion::CURRENT, &next, false).unwrap();
    scan.advance_quiet(102, true).unwrap();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    assert_eq!(scan.observer.tokens.len(), original_tokens);
    let flag = AtomicBool::new(false);
    let mut opened = keyed::open::<DoctrineMasteryObserver>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let window = opened.window(100, 102, None, |_| true).unwrap();
    assert_eq!(
        runs(&window, "GER@7|2|0|mastery"),
        [
            (100, 100, i64::MAX.to_string(), true),
            (101, 102, "0".into(), false)
        ]
    );
    assert_eq!(
        runs(&window, "GER@7|2|0|bank"),
        [(100, 102, "-17".into(), true)]
    );
    assert_eq!(
        runs(&window, "GER@7|2|0|daily"),
        [(100, 102, "123".into(), true)]
    );
    assert_eq!(
        runs(&window, "GER@7|2|0|effective"),
        [(100, 102, "-456".into(), true)]
    );
    assert_eq!(
        runs(&window, "GER@7|2|0|rewards"),
        [(100, 102, "2".into(), true)]
    );
    assert_eq!(runs(&window, "GER@7|2|0|sub").last().unwrap().2, "0");
    assert_eq!(window.reads, [(100, 101, 100), (102, 102, 102)]);
    assert_eq!(
        runs(&window, "GER@7|2|completed|0"),
        runs(&window, "GER@7|2|completed|1")
    );
}

#[test]
fn absent_empty_unassigned_and_unavailable_doctrine_slots_are_distinct() {
    let fixture = Fixture::new();
    let mut empty = frame(101);
    *empty.tables[TableId::DoctrineFolders.index()] =
        TableView::Complete(TableRows::DoctrineFolders(Vec::new()));
    *empty.tables[TableId::DoctrineTracks.index()] =
        TableView::Complete(TableRows::DoctrineTracks(Vec::new()));
    let mut absent = empty.clone();
    absent.hour = 102;
    *absent.tables[TableId::DoctrineCountries.index()] =
        TableView::Complete(TableRows::DoctrineCountries(Vec::new()));
    let mut unassigned = empty.clone();
    unassigned.hour = 103;
    *unassigned.tables[TableId::DoctrineCountries.index()] =
        TableView::Complete(TableRows::DoctrineCountries(vec![DoctrineCountryRow {
            slot: 7,
            country: 0,
            ..Default::default()
        }]));
    let mut unknown = frame(104);
    *unknown.tables[TableId::DoctrineTracks.index()] = TableView::Unavailable {
        reason: "synthetic missing doctrine tracks".into(),
    };
    let frames = [frame(100), empty, absent, unassigned, unknown, frame(106)];
    publish(&fixture, &frames);
    let flag = AtomicBool::new(false);
    let mut opened = keyed::open::<DoctrineMasteryObserver>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let window = opened.window(100, 106, None, |_| true).unwrap();
    assert_eq!(
        runs(&window, "GER@7|present"),
        [(100, 101, "1".into(), true), (106, 106, "1".into(), true)]
    );
    assert_eq!(
        window
            .known
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 103), (106, 106)]
    );
    assert!(
        runs(&window, "GER@7|2|0|mastery")
            .iter()
            .all(|(first, last, _, _)| first == last)
    );
    assert!(
        opened.scan.observer.refusals["GER@7"]
            .keys()
            .any(|why| why.contains("identity differs"))
    );
}

#[test]
fn canonical_status_join_accepts_alias_or_direct_id_but_not_raw_slot_confusion() {
    let mut observer = DoctrineMasteryObserver::default();
    let mut values = Values::default();
    let mut input = frame(1);
    assert!(
        observer
            .observe(FormatVersion::CURRENT, &input, &mut values)
            .unwrap()
    );
    assert!(values.iter().any(|(name, _)| name == "GER@7|present"));
    *input.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(Vec::new()));
    *input.tables[TableId::DoctrineCountries.index()] =
        TableView::Complete(TableRows::DoctrineCountries(vec![
            DoctrineCountryRow {
                slot: 7,
                country: 9001,
                ..Default::default()
            },
            DoctrineCountryRow {
                slot: 9001,
                country: 9001,
                ..Default::default()
            },
        ]));
    values.clear();
    assert!(
        observer
            .observe(FormatVersion::CURRENT, &input, &mut values)
            .unwrap()
    );
    assert_eq!(
        values
            .iter()
            .filter(|(name, _)| name.ends_with("|present"))
            .count(),
        1
    );
}

#[test]
fn token_spelling_failure_does_not_erase_any_numeric_measure() {
    let fixture = Fixture::new();
    let mut input = frame(100);
    *input.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic token failure".into(),
    };
    publish(&fixture, &[input.clone()]);
    let source = source(vec![input]);
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1", "tag":"GER"}),
        &prefix(100, 100, true),
        None,
    );
    assert!(reply["tokens"].as_object().unwrap().is_empty());
    assert_eq!(reply["window"]["series"].as_array().unwrap().len(), 10);
    assert!(
        reply["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|why| why.as_str().unwrap().contains("spelling is not recorded"))
    );
    assert!(
        reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|series| series["name"] == "GER@7|2|0|mastery"
                && series["runs"][0][2].as_str() == Some(i64::MAX.to_string().as_str()))
    );
}

#[test]
fn content_failure_preserves_numbers_and_is_cached_per_binding() {
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: fixture.directory.join("missing-game"),
            user_dir: fixture.directory.join("missing-user"),
            cache_dir: fixture.directory.join("content-cache"),
        },
        allow_unverified: false,
    };
    let params = json!({"branchId":"1", "tag":"GER"});
    let prefix = prefix(100, 100, true);
    let first = ask(&source, &fixture, &params, &prefix, Some(&options));
    let second = ask(&source, &fixture, &params, &prefix, Some(&options));
    assert_eq!(first, second);
    assert_eq!(first["folders"][0]["id"], "2");
    assert!(first["folders"][0]["name"].is_null());
    assert_eq!(first["folders"][0]["tracks"][0]["id"], "0");
    assert_eq!(first["tokens"].as_object().unwrap().len(), 3);
    assert_eq!(first["window"]["series"].as_array().unwrap().len(), 10);
    assert!(
        first["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|why| why.as_str().unwrap().contains("database unavailable"))
    );
    assert_eq!(super::content::database_loads(&source, &options), 1);
}

#[test]
fn selected_windows_clip_pending_and_preserve_country_identity_on_tag_reuse() {
    let fixture = Fixture::new();
    let before = frame(100);
    let mut after = frame(101);
    *after.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 29,
            canonical_slot: 7000,
            tag: b"GER".to_vec(),
            ..Default::default()
        }]));
    *after.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 29,
            canonical_slot: 7000,
        }]));
    *after.tables[TableId::DoctrineCountries.index()] =
        TableView::Complete(TableRows::DoctrineCountries(vec![DoctrineCountryRow {
            slot: 7000,
            country: 29,
            ..Default::default()
        }]));
    *after.tables[TableId::DoctrineFolders.index()] =
        TableView::Complete(TableRows::DoctrineFolders(Vec::new()));
    *after.tables[TableId::DoctrineTracks.index()] =
        TableView::Complete(TableRows::DoctrineTracks(Vec::new()));
    publish(&fixture, &[before.clone(), after.clone()]);
    let source = source(vec![before, after]);
    let params = json!({"branchId":"1", "tag":"GER"});
    let early = ask(&source, &fixture, &params, &prefix(100, 100, false), None);
    assert!(
        early["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|series| !series["name"].as_str().unwrap().starts_with("GER@7000"))
    );
    let full = ask(&source, &fixture, &params, &prefix(100, 101, true), None);
    assert!(
        full["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|series| series["name"] == "GER@7000|present")
    );
    let none = ask(
        &source,
        &fixture,
        &json!({"branchId":"1", "tag":"ITA"}),
        &prefix(100, 101, true),
        None,
    );
    assert!(none["window"]["series"].as_array().unwrap().is_empty());
    assert!(none["folders"].as_array().unwrap().is_empty());
}

#[test]
fn cold_reply_has_exact_wire_shape_and_all_nested_types() {
    let fixture = Fixture::new();
    let source = source(vec![frame(100)]);
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1", "tag":"GER"}),
        &prefix(100, 99, false),
        None,
    );
    assert_eq!(
        reply
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["folders", "tokens", "warnings", "window"])
    );
    assert!(reply["window"]["known"].as_array().unwrap().is_empty());
    assert!(reply["warnings"].as_array().unwrap().is_empty());
    let types = DoctrineMastery::types().join("\n");
    for expected in [
        "DoctrineMasteryParams",
        "DoctrineMasteryReply",
        "DoctrineMasteryReward",
        "KeyedSeries",
        "HourSpan",
    ] {
        assert!(types.contains(expected), "missing {expected}");
    }
    assert!(
        DoctrineMasteryObserver::fields()
            .unwrap()
            .iter()
            .any(|field| field.table == TableId::LexicalTokens)
    );
}

#[test]
fn five_thousand_countries_stay_sparse_and_query_only_the_selected_holder() {
    let fixture = Fixture::new();
    let mut input = frame(100);
    *input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..5000)
            .map(|at| CountryRow {
                id: at + 10000,
                canonical_slot: at + 20000,
                tag: format!("M{at}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..5000)
            .map(|at| CountryLinkRow {
                id: at + 10000,
                canonical_slot: at + 20000,
            })
            .collect(),
    ));
    *input.tables[TableId::DoctrineCountries.index()] =
        TableView::Complete(TableRows::DoctrineCountries(
            (0..5000)
                .map(|at| DoctrineCountryRow {
                    slot: at + 20000,
                    country: at + 10000,
                    ..Default::default()
                })
                .collect(),
        ));
    *input.tables[TableId::DoctrineFolders.index()] =
        TableView::Complete(TableRows::DoctrineFolders(
            (0..5000)
                .map(|at| DoctrineFolderRow {
                    slot: at + 20000,
                    ordinal: 2,
                    folder: 10,
                    ..Default::default()
                })
                .collect(),
        ));
    *input.tables[TableId::DoctrineTracks.index()] =
        TableView::Complete(TableRows::DoctrineTracks(
            (0..5000)
                .map(|at| DoctrineTrackRow {
                    slot: at + 20000,
                    folder: 2,
                    ordinal: 0,
                    mastery: i64::from(at),
                    ..Default::default()
                })
                .collect(),
        ));
    let scan = publish(&fixture, &[input.clone()]);
    assert_eq!(scan.names().len(), 5000 * 8);
    assert_eq!(scan.observer.tokens.len(), 1);
    let source = source(vec![input]);
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1", "tag":"M4999"}),
        &prefix(100, 100, true),
        None,
    );
    let series = reply["window"]["series"].as_array().unwrap();
    assert_eq!(series.len(), 8);
    assert!(
        series
            .iter()
            .all(|row| row["name"].as_str().unwrap().starts_with("M4999@24999|"))
    );
}

#[test]
fn authored_mod_structure_unequal_rewards_and_generated_descriptions() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content readers");
    let (_, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NDoctrines={DEFAULT_REWARD_MASTERY=3.25},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/doctrines/folders/mod.txt",
            "mod_folder={name=MOD_FOLDER tab_gfx=GFX_Mod_Folder} extra_folder={name=EXTRA_FOLDER}",
        ),
        (
            "common/doctrines/tracks/mod.txt",
            "mod_track={name=MOD_TRACK} extra_track={name=EXTRA_TRACK}",
        ),
        (
            "common/doctrines/grand_doctrines/mod.txt",
            "mod_grand={name=MOD_GRAND folder=mod_folder icon=GFX_Mod_Grand tracks={mod_track}} Mod_Grand_Choice={name=CHOICE folder=mod_folder icon=GFX_Mod_Choice tracks={mod_track}} extra_grand={name=EXTRA_GRAND folder=extra_folder tracks={extra_track}}",
        ),
        (
            "common/doctrines/subdoctrines/mod/mod.txt",
            "mod_sub={name=MOD_SUB track=mod_track icon=GFX_Mod_Sub rewards={first={mastery=1.25} second={mastery=7} third={mastery=8.5} fourth={mastery=11} fifth={mastery=23} sixth={mastery=24} seventh={}}} Alternative_Sub={name=ALT_SUB track=mod_track rewards={Tier={mastery=2}}} extra_sub={name=EXTRA_SUB track=extra_track}",
        ),
        (
            "localisation/mod_l_english.yml",
            concat!(
                "\u{feff}l_english:\n MOD_FOLDER:0 \"Mod folder\"\n EXTRA_FOLDER:0 \"First folder\"\n MOD_TRACK:0 \"Mod track\"\n EXTRA_TRACK:0 \"First track\"\n",
                " CHOICE:0 \"Unwitnessed choice\"\n MOD_GRAND:0 \"Mod grand\"\n EXTRA_GRAND:0 \"First grand\"\n MOD_SUB:0 \"Mod sub\"\n ALT_SUB:0 \"Alternate sub\"\n EXTRA_SUB:0 \"First sub\"\n",
                " mod_sub_first:0 \"First reward\"\n mod_sub_first_desc:0 \"Authored generated prose\"\n mod_sub_second:0 \"Second reward\"\n",
                " mod_sub_third:0 \"Third reward\"\n mod_sub_fourth:0 \"Fourth reward\"\n mod_sub_fifth:0 \"Fifth reward\"\n mod_sub_sixth:0 \"Sixth reward\"\n mod_sub_seventh:0 \"Seventh reward\"\n Alternative_Sub_Tier:0 \"Alternative reward\"\n",
            ),
        ),
    ]);
    let (_, base) = dashboard_base::catalogue::tests::folder_catalogue(&[
        (
            "common/doctrines/grand_doctrines/hidden.txt",
            "hidden_grand={folder=mod_folder tracks={mod_track}}",
        ),
        (
            "common/doctrines/subdoctrines/hidden.txt",
            "hidden_sub={track=mod_track}",
        ),
        (
            "interface/hidden.gfx",
            "spriteTypes={spriteType={name=GFX_Hidden textureFile=gfx/hidden.png}}",
        ),
    ]);
    std::fs::create_dir_all(folder.0.join("interface")).unwrap();
    std::fs::write(
        folder.0.join("interface/mod.gfx"),
        concat!(
            "spriteTypes={spriteType={name=GFX_Mod_Folder textureFile=gfx/tab.png} ",
            "spriteType={name=GFX_Mod_Grand textureFile=gfx/grand.png} ",
            "spriteType={name=GFX_Mod_Choice textureFile=gfx/choice.png} ",
            "spriteType={name=GFX_Mod_Sub textureFile=gfx/sub.png}}",
        ),
    )
    .unwrap();
    let replacements = [
        "common/doctrines/grand_doctrines",
        "common/doctrines/subdoctrines",
        "interface",
    ];
    let layers =
        installed_content_core::ResolvedLayers::from_folders(&[(&base.0, &[]), (&folder.0, &replacements)])
            .unwrap();
    let catalogue = dashboard_base::catalogue::tests::layers_catalogue(layers);
    let database = installed_content_core::doctrines::read(&catalogue.content.layers).unwrap();
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let mut input = frame(100);
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *input.tables[TableId::LexicalTokens.index()]
    {
        for row in rows.iter_mut() {
            if row.token == 11 {
                row.literal = b"MOD_GRAND".to_vec();
            }
            if row.token == 12 {
                row.literal = b"MOD_SUB".to_vec();
            }
        }
        rows.push(LexicalTokenRow {
            token: 13,
            literal: b"extra_folder".to_vec(),
        });
        rows.push(LexicalTokenRow {
            token: 14,
            literal: b"extra_grand".to_vec(),
        });
    }
    if let TableView::Complete(TableRows::DoctrineFolders(rows)) =
        &mut *input.tables[TableId::DoctrineFolders.index()]
    {
        rows.insert(
            0,
            DoctrineFolderRow {
                slot: 7,
                ordinal: 0,
                folder: 13,
                grand_doctrine: Some(14),
                ..Default::default()
            },
        );
    }
    let fixture = Fixture::new();
    publish(&fixture, &[input.clone()]);
    let (source, context) = test_fixtures::source_with_content(vec![input], &folder.0);
    let options = context.content.as_ref().unwrap();
    super::content::install_fixture(&source, options, Arc::new(catalogue), database, 325_000);
    let params = json!({"branchId":"1", "tag":"GER"});
    let first = ask(
        &source,
        &fixture,
        &params,
        &prefix(100, 100, true),
        Some(options),
    );
    let second = ask(
        &source,
        &fixture,
        &params,
        &prefix(100, 100, true),
        Some(options),
    );
    assert_eq!(first, second);
    let folders = first["folders"].as_array().unwrap();
    assert_eq!(
        folders
            .iter()
            .map(|folder| folder["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["0", "2"]
    );
    assert_eq!(folders[0]["name"], "First folder");
    assert_eq!(folders[1]["name"], "Mod folder");
    assert_eq!(folders[1]["grand"][0]["id"], "MOD_GRAND");
    let track = &folders[1]["tracks"][0];
    assert_eq!(track["id"], "0");
    assert_eq!(track["name"], "Mod track");
    assert_eq!(track["subdoctrines"].as_array().unwrap().len(), 2);
    let sub = &track["subdoctrines"][0];
    assert_eq!(sub["name"], "Mod sub");
    assert_eq!(sub["id"], "MOD_SUB");
    assert_eq!(folders[1]["icon"], "GFX_Mod_Folder");
    assert_eq!(folders[1]["grand"][0]["icon"], "GFX_Mod_Grand");
    assert_eq!(folders[1]["grand"][1]["id"], "Mod_Grand_Choice");
    assert_eq!(folders[1]["grand"][1]["name"], "Unwitnessed choice");
    assert_eq!(folders[1]["grand"][1]["icon"], "GFX_Mod_Choice");
    assert_eq!(sub["icon"], "GFX_Mod_Sub");
    let alternative = &track["subdoctrines"][1];
    assert_eq!(alternative["id"], "Alternative_Sub");
    assert_eq!(alternative["rewards"][0]["name"], "Alternative reward");
    assert!(alternative["icon"].is_null());
    assert!(!first.to_string().contains("hidden_grand"));
    assert!(!first.to_string().contains("hidden_sub"));
    let rewards = sub["rewards"].as_array().unwrap();
    assert_eq!(
        rewards
            .iter()
            .map(|reward| reward["threshold"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "125000", "700000", "850000", "1100000", "2300000", "2400000", "325000"
        ]
    );
    assert_eq!(rewards[0]["name"], "First reward");
    assert!(
        rewards[0]["description"]
            .as_str()
            .unwrap()
            .contains("Authored generated prose")
    );
    assert_eq!(rewards[6]["id"], "seventh");
    assert!(first["warnings"].as_array().unwrap().iter().any(|why| {
        why.as_str()
            .unwrap()
            .contains("subdoctrine Alternative_Sub icon: authored icon key is absent")
    }));
    assert!(
        first["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "GER@7|2|0|rewards" && row["runs"][0][2] == "2")
    );
    let keys = folders[1]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        BTreeSet::from(["id", "name", "icon", "grand", "tracks"])
    );
    assert!(!first.to_string().contains("not retained"));
    assert!(!first.to_string().contains("unlockHour"));
    assert!(!first.to_string().contains("\"eta\""));
}

#[test]
fn cancellation_and_invalid_windows_are_refused_not_truncated() {
    let fixture = Fixture::new();
    let source = source(vec![frame(100)]);
    let params = json!({"branchId":"1", "tag":"GER"});
    let cancelled = AtomicBool::new(true);
    let error = query(&Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix(100, 100, true),
        cancelled: &cancelled,
    })
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(
        DoctrineMastery::validate(&DoctrineMasteryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            first_hour: Some(5),
            last_hour: Some(4),
            hour: None,
        })
        .is_err()
    );
}

#[test]
fn unresolved_tokens_are_not_relabelled_when_dictionary_appears() {
    let mut observer = DoctrineMasteryObserver::default();
    let mut input = frame(100);
    *input.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic dictionary missing".into(),
    };
    let mut values = Values::default();
    observer
        .observe(FormatVersion::CURRENT, &input, &mut values)
        .unwrap();
    let old = values
        .iter()
        .find(|(name, _)| name.ends_with("|sub"))
        .unwrap()
        .1;
    observer = serde_json::from_value(serde_json::to_value(observer).unwrap()).unwrap();
    values.clear();
    observer
        .observe(FormatVersion::CURRENT, &frame(101), &mut values)
        .unwrap();
    let new = values
        .iter()
        .find(|(name, _)| name.ends_with("|sub"))
        .unwrap()
        .1;
    assert_ne!(old, new);
    assert_eq!(observer.literal(old as u64), None);
    assert_eq!(observer.literal(new as u64), Some(b"mod_sub".as_slice()));
}

#[test]
fn reward_composite_localisation_retains_authored_token_case_without_a_witness() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content readers");
    let (catalogue, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        ("common/doctrines/tracks/mod.txt", "mod_track={name=TRACK}"),
        (
            "common/doctrines/subdoctrines/mod.txt",
            "Mixed_Sub={name=SUB track=mod_track rewards={Tier={mastery=1}}}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n SUB:0 \"Mixed sub\"\n Mixed_Sub_Tier:0 \"Correct case\"\n Mixed_Sub_Tier_desc:0 \"Correct prose\"\n mixed_sub_Tier:0 \"Wrong case\"\n",
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let database = installed_content_core::doctrines::read(&catalogue.content.layers).unwrap();
    let named = super::content::test_subdoctrine(&database, &catalogue, b"Mixed_Sub");
    assert_eq!(named["id"], "Mixed_Sub");
    assert_eq!(named["rewards"][0]["name"], "Correct case");
    assert!(
        named["rewards"][0]["description"]
            .as_str()
            .unwrap()
            .contains("Correct prose")
    );
    let unavailable = super::content::test_subdoctrine(&database, &catalogue, b"mixed_sub");
    assert_eq!(unavailable["id"], "mixed_sub");
    assert_eq!(unavailable["rewards"][0]["name"], "Correct case");
}
#[test]
fn unchanged_lexical_allocation_reuses_its_runtime_index() {
    let mut observer = DoctrineMasteryObserver::default();
    let first = frame(100);
    let mut second = first.clone();
    second.hour = 101;
    let mut values = Values::default();
    observer
        .observe(FormatVersion::CURRENT, &first, &mut values)
        .unwrap();
    values.clear();
    observer
        .observe(FormatVersion::CURRENT, &second, &mut values)
        .unwrap();
    assert_eq!(observer.lexical_builds(), 1);
    observer = serde_json::from_value(serde_json::to_value(observer).unwrap()).unwrap();
    values.clear();
    observer
        .observe(FormatVersion::CURRENT, &second, &mut values)
        .unwrap();
    assert_eq!(observer.lexical_builds(), 1);
}

#[test]
fn reached_descriptions_use_exact_country_hour_without_poisoning_history_cache() {
    use replay_data::rows::{ClockRow, ScriptFlagRow};
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (catalogue, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/mod.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
        ),
        (
            "common/doctrines/folders/mod.txt",
            "mod_folder={name=FOLDER}",
        ),
        ("common/doctrines/tracks/mod.txt", "mod_track={name=TRACK}"),
        (
            "common/doctrines/grand_doctrines/mod.txt",
            "mod_grand={name=GRAND folder=mod_folder tracks={mod_track}}",
        ),
        (
            "common/doctrines/subdoctrines/mod.txt",
            "mod_sub={name=SUB track=mod_track rewards={one={mastery=1 effect={if={limit={has_country_flag=mod_flag} add_political_power=3} else={add_political_power=8}}}}}",
        ),
        (
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n FOLDER:0 \"Folder\"\n TRACK:0 \"Track\"\n GRAND:0 \"Grand\"\n SUB:0 \"Sub\"\n mod_sub_one:0 \"Reward\"\n mod_sub_one_desc:0 \"Static prose\"\n POLITICS_ADD_POLITICAL_POWER:0 \"Power $VAL|0$\"\n",
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    let database = installed_content_core::doctrines::read(&catalogue.content.layers).unwrap();
    let mut frames = Vec::new();
    for hour in 100..=102 {
        let mut input = frame(hour);
        *input.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour,
                ..Default::default()
            }]));
        if let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut *input.tables[TableId::LexicalTokens.index()]
        {
            rows.push(LexicalTokenRow {
                token: 30,
                literal: b"mod_flag".to_vec(),
            });
        }
        *input.tables[TableId::ScriptFlags.index()] = if hour == 102 {
            TableView::Unavailable {
                reason: "synthetic missing flags".into(),
            }
        } else {
            // The other country's flag never decides GER's condition.
            let mut rows = vec![ScriptFlagRow {
                scope_kind: 1,
                scope_id: 21,
                token: 30,
                value: 1,
                ..Default::default()
            }];
            if hour == 100 {
                rows.push(ScriptFlagRow {
                    scope_kind: 1,
                    // Flags use Countries' current native id, not the doctrine
                    // status row's older alias of the same canonical holder.
                    scope_id: 9001,
                    token: 30,
                    value: 1,
                    ..Default::default()
                });
            }
            rows.sort_by_key(|row| (row.scope_kind, row.scope_id, row.token));
            TableView::Complete(TableRows::ScriptFlags(rows))
        };
        frames.push(input);
    }
    let fixture = Fixture::new();
    publish(&fixture, &frames);
    let (source, context) = test_fixtures::source_with_content(frames, &folder.0);
    let options = context.content.as_ref().unwrap();
    super::content::install_fixture(&source, options, Arc::new(catalogue), database, 100_000);
    let whole = prefix(100, 102, true);
    let params = json!({"branchId":"1", "tag":"GER"});
    let (history, decoded) =
        dashboard_base::prepared::query::progressive::without_block_decodes(|| {
            ask(&source, &fixture, &params, &whole, Some(options))
        });
    assert!(!decoded, "history content must use prepared pages only");
    let description = |reply: &Value| {
        reply["folders"][0]["tracks"][0]["subdoctrines"][0]["rewards"][0]["description"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(description(&history).contains("Static prose"));
    assert!(!description(&history).contains("Power"));
    for (hour, expected) in [(100, "Power 3"), (101, "Power 8"), (100, "Power 3")] {
        let reached = ask(
            &source,
            &fixture,
            &json!({"branchId":"1", "tag":"GER", "hour":hour}),
            &whole,
            Some(options),
        );
        assert!(
            description(&reached).contains(expected),
            "{}; {:?}",
            description(&reached),
            reached["warnings"]
        );
        assert!(
            !reached["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|why| why.as_str().unwrap().contains("reached-hour operands")),
            "{}",
            reached["warnings"]
        );
        assert_eq!(history["window"], reached["window"]);
        assert_eq!(history["tokens"], reached["tokens"]);
    }
    let missing = ask(
        &source,
        &fixture,
        &json!({"branchId":"1", "tag":"GER", "hour":102}),
        &whole,
        Some(options),
    );
    assert!(description(&missing).contains("Static prose"));
    assert!(!description(&missing).contains("Power"));
    assert_eq!(history["window"], missing["window"]);
    let pending = ask(
        &source,
        &fixture,
        &json!({"branchId":"1", "tag":"GER", "hour":101}),
        &prefix(100, 100, false),
        Some(options),
    );
    assert!(!description(&pending).contains("Power"));
    assert!(
        pending["warnings"]
            .to_string()
            .contains("outside the accepted window")
    );
    let history_again = ask(&source, &fixture, &params, &whole, Some(options));
    assert_eq!(history, history_again);
}

fn s2_options(fixture: &Fixture) -> dashboard_core::config::ContentOptions {
    dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: fixture.directory.join("missing-game"),
            user_dir: fixture.directory.join("missing-user"),
            cache_dir: fixture.directory.join("content-cache"),
        },
        allow_unverified: false,
    }
}

fn s2_assert_reachable(reply: &Value) {
    let folders = reply["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 1);
    let folder = &folders[0];
    assert_eq!(folder["id"], "2");
    assert!(folder["name"].is_null());
    assert!(folder["icon"].is_null());
    let track = &folder["tracks"][0];
    assert_eq!(track["id"], "0");
    assert!(track["name"].is_null());
    let key = format!(
        "GER@7|{}|{}|",
        folder["id"].as_str().unwrap(),
        track["id"].as_str().unwrap()
    );
    let series = reply["window"]["series"].as_array().unwrap();
    for (measure, value) in [
        ("mastery", i64::MAX.to_string()),
        ("bank", "-17".into()),
        ("daily", "123".into()),
        ("effective", "-456".into()),
        ("rewards", "2".into()),
    ] {
        let row = series
            .iter()
            .find(|row| row["name"] == format!("{key}{measure}"))
            .unwrap();
        assert_eq!(row["runs"][0][2], value);
    }
    let selected = series
        .iter()
        .find(|row| row["name"] == format!("{key}sub"))
        .unwrap();
    let sub = &track["subdoctrines"][0];
    assert_eq!(sub["tokenId"], selected["runs"][0][2]);
    assert!(sub["name"].is_null());
    let grand = &folder["grand"][0];
    let selected = series
        .iter()
        .find(|row| row["name"] == "GER@7|2|grand")
        .unwrap();
    assert_eq!(grand["tokenId"], selected["runs"][0][2]);
    assert!(grand["name"].is_null());
    assert_eq!(
        reply["window"]["known"],
        json!([{"firstHour":100,"lastHour":100}])
    );
    assert_eq!(reply["window"]["reads"], json!([[100, 100, 100]]));
}

#[test]
fn s2_recorded_tree_without_content_keeps_mastery_bank_and_gains_reachable() {
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1","tag":"GER"}),
        &prefix(100, 100, true),
        None,
    );
    s2_assert_reachable(&reply);
    assert_eq!(reply["folders"][0]["grand"][0]["id"], "mod_grand");
    assert_eq!(
        reply["folders"][0]["tracks"][0]["subdoctrines"][0]["id"],
        "mod_sub"
    );
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str().unwrap() == "1 doctrine mastery content refusals: game folders are not chosen; names, rewards and icons are unavailable"
    }));
}

#[test]
fn s2_recorded_tree_survives_database_failure_and_caches_the_refusal() {
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let options = s2_options(&fixture);
    super::content::install_reader_fixture(
        &source,
        &options,
        Err(io::Error::other("synthetic doctrine database failure")),
        Ok(100_000),
    );
    let params = json!({"branchId":"1","tag":"GER"});
    let prefix = prefix(100, 100, true);
    let reply = ask(&source, &fixture, &params, &prefix, Some(&options));
    s2_assert_reachable(&reply);
    assert_eq!(
        reply,
        ask(&source, &fixture, &params, &prefix, Some(&options))
    );
    assert_eq!(super::content::database_loads(&source, &options), 1);
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str().unwrap() == "1 doctrine mastery content refusals: doctrine database unavailable: synthetic doctrine database failure"
    }));
}

fn s2_database() -> installed_content_core::doctrines::Database {
    use installed_content_core::{
        doctrines::{Database, DefinitionSources, Presentation, RewardSource},
        tokens::Token,
    };
    let mut database = Database::default();
    let grand = Token::c_str(b"mod_grand");
    let sub = Token::c_str(b"mod_sub");
    let track = Token::c_str(b"mod_track");
    database.grand_presentations.insert(
        grand.clone(),
        Presentation {
            token: b"mod_grand".to_vec(),
            folder: Some(Token::c_str(b"mod_folder")),
            ..Default::default()
        },
    );
    database.sub_presentations.insert(
        sub.clone(),
        Presentation {
            token: b"mod_sub".to_vec(),
            icon: b"GFX_Mod_Sub".to_vec(),
            ..Default::default()
        },
    );
    database.tactic_sources.grand_order.push(grand.clone());
    database.tactic_sources.grand.insert(
        grand,
        DefinitionSources {
            tracks: vec![track],
            ..Default::default()
        },
    );
    database.tactic_sources.sub_order.push(sub.clone());
    database.tactic_sources.sub.insert(
        sub,
        DefinitionSources {
            rewards: [
                (b"first".as_slice(), 125_000),
                (b"second", 0),
                (b"third", -700_000),
            ]
            .into_iter()
            .map(|(token, mastery)| RewardSource {
                token: token.to_vec(),
                mastery,
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        },
    );
    database
}

#[test]
fn s2_recorded_tree_survives_catalogue_failure_with_independent_requirements() {
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let options = s2_options(&fixture);
    super::content::install_reader_fixture(&source, &options, Ok(s2_database()), Ok(900_000));
    let params = json!({"branchId":"1","tag":"GER"});
    let prefix = prefix(100, 100, true);
    let reply = ask(&source, &fixture, &params, &prefix, Some(&options));
    s2_assert_reachable(&reply);
    let rewards = &reply["folders"][0]["tracks"][0]["subdoctrines"][0]["rewards"];
    assert_eq!(
        rewards
            .as_array()
            .unwrap()
            .iter()
            .map(|reward| {
                (
                    reward["id"].as_str().unwrap(),
                    reward["threshold"].as_str().unwrap(),
                )
            })
            .collect::<Vec<_>>(),
        [
            ("first", "125000"),
            ("second", "900000"),
            ("third", "-700000")
        ]
    );
    assert!(
        rewards
            .as_array()
            .unwrap()
            .iter()
            .all(|reward| reward["name"].is_null())
    );
    assert_eq!(
        reply,
        ask(&source, &fixture, &params, &prefix, Some(&options))
    );
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str()
            .unwrap()
            .starts_with("1 doctrine mastery content refusals: localisation catalogue unavailable:")
    }));
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str()
            .unwrap()
            .contains("nonpositive mastery scale -700000")
    }));
}

#[test]
fn s2_recorded_tree_keeps_missing_content_choices_with_null_names() {
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let options = s2_options(&fixture);
    let (catalogue, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[]);
    super::content::install_fixture(
        &source,
        &options,
        Arc::new(catalogue),
        Default::default(),
        100_000,
    );
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1","tag":"GER"}),
        &prefix(100, 100, true),
        Some(&options),
    );
    s2_assert_reachable(&reply);
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str().unwrap() == "1 doctrine mastery content refusals: folder 2 has grand doctrine selections absent from content"
    }));
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str().unwrap() == "1 doctrine mastery content refusals: folder 2 track 0 selected subdoctrine is absent from content"
    }));
}

#[test]
fn s2_recorded_tree_keeps_numeric_token_identity_when_spelling_is_unknown() {
    let fixture = Fixture::new();
    let mut input = frame(100);
    *input.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic missing spelling".into(),
    };
    publish(&fixture, &[input.clone()]);
    let source = source(vec![input]);
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1","tag":"GER"}),
        &prefix(100, 100, true),
        None,
    );
    s2_assert_reachable(&reply);
    assert!(reply["tokens"].as_object().unwrap().is_empty());
    let sub = &reply["folders"][0]["tracks"][0]["subdoctrines"][0];
    assert_eq!(sub["id"], sub["tokenId"]);
    assert!(sub["rewards"].is_null());
}

#[test]
fn s2_recorded_tree_uses_window_tracks_not_the_branch_inventory() {
    let fixture = Fixture::new();
    let first = frame(100);
    let mut later = frame(101);
    if let TableView::Complete(TableRows::DoctrineTracks(rows)) =
        &mut *later.tables[TableId::DoctrineTracks.index()]
    {
        let mut track = rows[0].clone();
        track.ordinal = 9;
        rows.push(track);
    }
    publish(&fixture, &[first.clone(), later.clone()]);
    let source = source(vec![first, later]);
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1","tag":"GER"}),
        &prefix(100, 100, false),
        None,
    );
    s2_assert_reachable(&reply);
    assert_eq!(reply["folders"][0]["tracks"].as_array().unwrap().len(), 1);
    assert!(
        reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|series| !series["name"].as_str().unwrap().contains("|9|"))
    );
}

#[test]
fn s2_recorded_tree_keeps_reward_ordinals_when_the_default_requirement_fails() {
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let options = s2_options(&fixture);
    super::content::install_reader_fixture(
        &source,
        &options,
        Ok(s2_database()),
        Err(io::Error::other("synthetic default requirement failure")),
    );
    let reply = ask(
        &source,
        &fixture,
        &json!({"branchId":"1","tag":"GER"}),
        &prefix(100, 100, true),
        Some(&options),
    );
    s2_assert_reachable(&reply);
    let rewards = reply["folders"][0]["tracks"][0]["subdoctrines"][0]["rewards"]
        .as_array()
        .unwrap();
    assert_eq!(
        rewards
            .iter()
            .map(|reward| (&reward["id"], &reward["threshold"]))
            .collect::<Vec<_>>(),
        vec![
            (&json!("first"), &json!("125000")),
            (&json!("second"), &Value::Null),
            (&json!("third"), &json!("-700000"))
        ]
    );
    assert!(reply["warnings"].as_array().unwrap().iter().any(|note| {
        note.as_str().unwrap().contains("subdoctrine mod_sub reward second threshold unavailable: synthetic default requirement failure")
    }));
}

#[test]
fn s2_recorded_tree_distinguishes_unknown_rewards_from_authored_empty() {
    use installed_content_core::tokens::Token;
    let fixture = Fixture::new();
    let input = frame(100);
    publish(&fixture, std::slice::from_ref(&input));
    let source = source(vec![input]);
    let params = json!({"branchId":"1","tag":"GER"});
    let prefix = prefix(100, 100, true);
    let unknown = ask(&source, &fixture, &params, &prefix, None);
    assert!(unknown["folders"][0]["tracks"][0]["subdoctrines"][0]["rewards"].is_null());
    let options = s2_options(&fixture);
    let mut database = s2_database();
    database
        .tactic_sources
        .sub
        .get_mut(&Token::c_str(b"mod_sub"))
        .unwrap()
        .rewards
        .clear();
    super::content::install_reader_fixture(&source, &options, Ok(database), Ok(100_000));
    let empty = ask(&source, &fixture, &params, &prefix, Some(&options));
    s2_assert_reachable(&empty);
    assert_eq!(
        empty["folders"][0]["tracks"][0]["subdoctrines"][0]["rewards"],
        json!([])
    );
}
