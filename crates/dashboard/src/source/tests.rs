use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn policy_two_lazy_content_warnings_reach_metadata_catalogue_and_cached_products() {
    use crate::products::{ContentOptions, LoadContext};
    use replay::spool::Record;
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        ("hoi4.exe", "test"),
        (
            "user/settings.txt",
            "language=l_english force_pow2_textures=no",
        ),
        (
            "user/dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        ),
        (
            "common/defines/a.lua",
            "NDefines.NGame.MAX_SCRIPTED_LOC_RECURSION = 32",
        ),
    ]);
    std::fs::create_dir(folder.0.join("dlc")).unwrap();
    let options = ContentOptions {
        options: installed_content::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
    };
    // At recording time only policy-2 paths exist, so this is its exact hash.
    let gathered = installed_content::gather(&installed_content::GatherOptions {
        game_dir: options.options.game_dir.clone(),
        user_dir: options.options.user_dir.clone(),
        cache_dir: options.options.cache_dir.clone(),
        dlc_order: Some(Vec::new()),
    })
    .unwrap();
    let mut identity = gathered
        .identity(
            "test".into(),
            [
                0x9f, 0x86, 0xd0, 0x81, 0x88, 0x4c, 0x7d, 0x65, 0x9a, 0x2f, 0xea, 0xa0, 0xc5,
                0x5a, 0xd0, 0x15, 0xa3, 0xbf, 0x4f, 0x1b, 0x2b, 0x0b, 0x82, 0x2c, 0xd1, 0x5d,
                0x6c, 0x15, 0xb0, 0xf0, 0x0a, 0x08,
            ],
            4,
            "test".into(),
            [1; 32],
        )
        .unwrap();
    identity.coverage_policy = 2;
    for (path, text) in [
        (
            "events/a.txt",
            "add_namespace=s country_event={id=s.1 title=TITLE}",
        ),
        ("gfx/texticons/late.dds", "synthetic icon bytes"),
    ] {
        let path = folder.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let mut input = records(1, vec![source(1, 100, Some(1))]);
    let Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity;
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "policy-two".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[input]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let catalogue = crate::catalogue::catalogue(&source, &options).unwrap();
    assert!(
        source.metadata().warnings.iter().all(|note| !note.starts_with("Content: "))
    );
    let layers = &catalogue.content.layers;
    // The event reader loads headers/body lazily, after the catalogue was built.
    let events = installed_content::events::LazyEvents::new(Arc::new(layers.clone()));
    assert!(events.get(b"s.1").unwrap().refused.is_none());
    let expected = layers.read_warnings().warning().unwrap();
    assert!(expected.contains("Content: 1 file(s)"));
    assert!(expected.contains("coverage policy 2; these bytes are not hash-verified: events/a.txt"));
    assert!(catalogue.refusal_lines().contains(&expected));
    assert!(source.metadata().warnings.contains(&expected));
    let context = LoadContext {
        content: Some(options),
    };
    let reply = || {
        let json = (crate::products::find("country-fuel").unwrap().load)(
            &source,
            "lease",
            serde_json::json!({"branchId":"1", "tag":"GER"}),
            &context,
            &AtomicBool::new(false),
        )
        .unwrap();
        serde_json::from_str::<serde_json::Value>(&json).unwrap()
    };
    assert!(
        reply()["data"]["warnings"].as_array().unwrap().contains(&serde_json::json!(expected))
    );
    layers.read("gfx/texticons/late.dds").unwrap();
    let updated = layers.read_warnings().warning().unwrap();
    assert!(updated.contains("Content: 2 file(s)"));
    assert!(catalogue.refusal_lines().contains(&updated));
    assert!(source.metadata().warnings.contains(&updated));
    let cached = reply();
    let notes: Vec<_> = cached["data"]["warnings"]
        .as_array().unwrap().iter()
        .filter(|note| note.as_str().is_some_and(|note| note.starts_with("Content: ")))
        .collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].as_str(), Some(updated.as_str()));
}

#[test]
fn the_catalogue_names_countries_when_content_is_known_and_shows_tags_otherwise() {
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay::rows::CountryAppearanceRow {
                country: 9001,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![replay::rows::PoliticsRow {
            country: 9001,
            ..Default::default()
        }]));
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "source-1".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let localisation: installed_content::localisation::Localisation =
        [("GER".to_owned(), "German Reich".to_owned())]
            .into_iter()
            .collect();
    let named = source.named_metadata(Ok(Arc::new(localisation)));
    assert_eq!(named.countries[0].name, "German Reich");
    source.name_countries(None);
    let tagged = source.metadata();
    assert_eq!(tagged.countries[0].name, "GER");
    assert!(
        tagged
            .warnings
            .iter()
            .any(|w| w.contains("until the game folders"))
    );
}

impl crate::Source {
    pub(crate) fn set_test_content(
        &self,
        options: installed_content::ResolveOptions,
        content: Arc<installed_content::content::ResolvedContent>,
    ) {
        let reads = content.layers.report_uncovered_reads();
        *self.content.lock().unwrap() = Some((options, content, reads));
    }
}
