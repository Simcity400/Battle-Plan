use super::*;
use crate::catalogue::{Catalogue, View, built_catalogue, catalogue};
use crate::products::ContentOptions;
use content_reader_layers::content::diagnostics;
use std::sync::atomic::AtomicBool;

fn built() -> usize {
    BUILT.with(std::cell::Cell::get)
}

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kept-catalogue-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn localisation_build() -> (io::Result<Localisation>, Diagnostics) {
    diagnostics::capture(|| {
        Diagnostics::note("Text is shown in english, the language this recording was made in.".to_owned());
        Ok([("A", "first"), ("B", "§Ysecond§!"), ("EMPTY", "")]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect())
    })
}

#[test]
fn a_kept_part_is_rebuilt_when_missing_stale_cut_or_damaged() {
    let root = scratch("lifecycle");
    let place = |key: u8| Place {
        path: root.join("catalogue").join("facts-localisation-test.bin"),
        key: [key; KEY],
    };
    let (expected, noted) = localisation_build();
    let expected = expected.unwrap();
    assert!(!noted.is_empty());
    let read = |key: u8| {
        let before = built();
        let (part, diagnostics) = kept(Some(place(key)), localisation_build);
        assert_eq!(part.unwrap(), expected);
        assert_eq!(diagnostics, noted);
        built() - before
    };
    assert_eq!(read(1), 1, "built when missing");
    assert_eq!(read(1), 0, "read in place");
    assert_eq!(read(2), 1, "another key's file is built again");
    assert_eq!(read(2), 0);
    let path = place(2).path;
    let whole = std::fs::read(&path).unwrap();
    for cut in [0, 1, MAGIC.len() + KEY, whole.len() / 2, whole.len() - 1] {
        std::fs::write(&path, &whole[..cut]).unwrap();
        assert_eq!(read(2), 1, "cut at {cut}");
        assert_eq!(std::fs::read(&path).unwrap(), whole);
    }
    for at in [0, MAGIC.len(), MAGIC.len() + KEY + 3, whole.len() / 2, whole.len() - 1] {
        let mut damaged = whole.clone();
        damaged[at] ^= 0x40;
        std::fs::write(&path, &damaged).unwrap();
        assert_eq!(read(2), 1, "damaged at {at}");
        assert_eq!(read(2), 0);
    }
    let blocked = scratch("blocked");
    std::fs::write(blocked.join("catalogue"), b"not a folder").unwrap();
    let place = Place {
        path: blocked.join("catalogue").join("facts-localisation-test.bin"),
        key: [3; KEY],
    };
    let before = built();
    assert_eq!(kept(Some(place), localisation_build).0.unwrap(), expected);
    let place = Place {
        path: blocked.join("catalogue").join("facts-localisation-test.bin"),
        key: [3; KEY],
    };
    assert_eq!(kept(Some(place), localisation_build).0.unwrap(), expected);
    assert_eq!(built() - before, 2);
    let before = built();
    assert_eq!(kept(None, localisation_build).0.unwrap(), expected);
    assert_eq!(built(), before, "no place, nothing read or kept");
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(blocked).unwrap();
}

const GFX: &str = r#"spriteTypes = {
    spriteType = { name = "GFX_a" texturefile = "gfx//interface//a.dds" }
    SpriteType = { name = "GFX_b" textureFile = "gfx/interface/b.dds"
        NOOFFRAMES = 14 alwaysTransparent = yes }
    corneredTileSpriteType = { name = "GFX_tile" textureFile = "gfx/t.dds"
        noOfFrames = 2 size = { x = 8 y = 8 } borderSize = { x = 2 y = 2 } }
    progressbartype = { name = "GFX_bar" textureFile1 = "gfx/p.dds"
        color = { 1.0 0.0 0.0 } size = { x = 10 y = 4 } }
    frameAnimatedSpriteType = { name = "GFX_anim" texturefile = "gfx/f.dds"
        noOfFrames = 3 }
    spriteType = { name = "GFX_negative" texturefile = "gfx/n.dds" noOfFrames = -1 }
    spriteType = { name = "GFX_a" texturefile = "gfx/interface/a_later.dds" }
}"#;

#[test]
fn kept_sprites_read_back_as_every_kind_was_declared() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let read = SpriteSet::new(&[(
        "interface/test.gfx".to_owned(),
        content_reader_gui::gui::native::sprites::read_sprites(GFX.as_bytes()),
    )]);
    for name in ["GFX_a", "GFX_b", "GFX_tile", "GFX_bar", "GFX_anim"] {
        assert!(read.contains(name), "{name}: {read:?}");
    }
    assert!(read.refusal("GFX_negative").is_some(), "{:?}", read.refused);
    assert_eq!(read.refused.len(), 1, "{:?}", read.refused);
    assert_eq!(
        read.lookup("GFX_a").map(|sprite| sprite.texture.clone()),
        Ok(Some("gfx/interface/a_later.dds".to_owned()))
    );
    let mut out = Vec::new();
    Part::keep(&read, &mut out).unwrap();
    let bytes = Arc::new(out);
    let (kept, end) = <SpriteSet as Part>::read(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    for name in ["GFX_a", "GFX_b", "GFX_tile", "GFX_bar", "GFX_anim", "GFX_negative", "GFX_none"] {
        assert_eq!(kept.lookup(name), read.lookup(name), "{name}");
        assert_eq!(kept.find(name), read.find(name), "{name}");
        assert_eq!(kept.contains(name), read.contains(name), "{name}");
    }
    assert_eq!(kept, read);
    assert_eq!(kept.refused, read.refused);
    assert_eq!(
        kept.refused_names().collect::<Vec<_>>(),
        read.refused_names().collect::<Vec<_>>()
    );
}

fn recording_with_facts(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf, ContentOptions) {
    let (root, options, identity) = game(name, files);
    let path = root.join("campaign.battleplan");
    crate::test_fixtures::write_with_facts(&path, &options, &identity, session(&identity));
    std::fs::write(
        root.join("user/settings.txt"),
        "language=l_french force_pow2_textures=no",
    )
    .unwrap();
    (root, path, options)
}

fn game(name: &str, files: &[(&str, &str)]) -> (PathBuf, ContentOptions, replay_data::identity::Identity) {
    let root = scratch(name);
    std::fs::create_dir_all(root.join("user")).unwrap();
    for (path, text) in [
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
    ]
    .iter()
    .chain(files)
    {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let (options, identity) = crate::test_fixtures::content_identity(&root);
    (root, options, identity)
}

fn session(identity: &replay_data::identity::Identity) -> Vec<replay_capture_spool::spool::Record> {
    let mut input = crate::test_fixtures::records(1, vec![crate::test_fixtures::source(1, 100, Some(1))]);
    let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] else {
        unreachable!()
    };
    session.identity = identity.clone();
    input
}

fn open(path: &std::path::Path, options: &ContentOptions) -> crate::Source {
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "kept-catalogue".into(),
            revision: crate::protocol::replay_revision(path).unwrap(),
        },
        path.to_owned(),
        &AtomicBool::new(false),
    )
    .unwrap();
    source.recorded_presented(&options.options).unwrap();
    source
}

#[test]
fn a_cold_source_reads_the_kept_catalogue_as_it_was_built() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (root, path, options) = recording_with_facts(
        "cold",
        &[
            (
                "localisation/english/a_l_english.yml",
                "\u{feff}l_english:\n NAME:0 \"Recorded name\"\n SHARED:0 \"first\"\n BAD:0 \"bad \\q escape\"\n",
            ),
            (
                "localisation/english/b_l_english.yml",
                "\u{feff}l_english:\n SHARED:0 \"second\"\n EMPTY:0 \"\"\n",
            ),
            ("interface/test.gfx", GFX),
        ],
    );
    let folder = options.options.cache_dir.join("catalogue");
    let first = open(&path, &options);
    assert!(place::<Localisation>(&first, &options.options.cache_dir, &[]).is_some());
    let (made, made_noted) = diagnostics::capture(|| built_catalogue(&first, &options).unwrap());
    assert!(!folder.exists(), "the built catalogue keeps nothing");
    assert_eq!(made.localisation.language, "english");
    let mut noted = Vec::new();
    made_noted.append_to(&mut noted);
    assert!(noted.iter().all(|line| !line.contains("french")), "{noted:?}");
    let before = built();
    let (written, written_noted) = diagnostics::capture(|| catalogue(&first, &options).unwrap());
    assert_eq!(built() - before, 2, "both parts built and kept");
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 2);
    drop(first);
    let same = |catalogue: &Catalogue, noted: &Diagnostics| {
        assert_eq!(*catalogue.localisation, *made.localisation);
        assert_eq!(catalogue.sprites, made.sprites);
        assert_eq!(catalogue.warnings, made.warnings);
        assert_eq!(catalogue.verification, made.verification);
        assert_eq!(catalogue.recorded_language, made.recorded_language);
        assert_eq!(catalogue.gui_decode, made.gui_decode);
        assert_eq!(catalogue.content.notes, made.content.notes);
        assert_eq!(*noted, made_noted);
        for key in ["NAME", "SHARED", "BAD", "EMPTY", "MISSING"] {
            assert_eq!(
                catalogue.localised(key, View::Plain),
                made.localised(key, View::Plain),
                "{key}"
            );
        }
    };
    same(&*written, &written_noted);
    let before = built();
    let cold = open(&path, &options);
    let (loaded, loaded_noted) = diagnostics::capture(|| catalogue(&cold, &options).unwrap());
    assert_eq!(built(), before, "a cold source reads both parts from their files");
    same(&*loaded, &loaded_noted);
    assert_eq!(loaded.localised("SHARED", View::Plain).as_deref(), Some("first"));
    assert!(loaded.sprites.lookup("GFX_b").is_ok_and(|sprite| sprite.frames == 14));
    drop(cold);
    let sprites = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().contains("-sprites-"))
        .unwrap();
    let whole = std::fs::read(&sprites).unwrap();
    std::fs::write(&sprites, &whole[..whole.len() / 2]).unwrap();
    let before = built();
    let cold = open(&path, &options);
    let (rebuilt, rebuilt_noted) = diagnostics::capture(|| catalogue(&cold, &options).unwrap());
    assert_eq!(built() - before, 1, "the cut sprites are built again, the localisation read");
    same(&*rebuilt, &rebuilt_noted);
    assert_eq!(std::fs::read(&sprites).unwrap(), whole);
    drop(cold);
    std::fs::remove_dir_all(root).unwrap();

    let (bare, options, identity) = game("no-facts", &[]);
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "kept-catalogue-no-facts".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(
            replay::campaign::Replay::from_records(
                recorder_derive::writers(),
                crate::test_fixtures::CAMPAIGN.into(),
                &[session(&identity)],
            )
            .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(place::<Localisation>(&source, &options.options.cache_dir, &[]).is_none());
    let Err(why) = catalogue(&source, &options) else {
        panic!("a recording carrying no facts has no catalogue");
    };
    assert!(why.to_string().contains("carries no game facts"), "{why}");
    assert!(!options.options.cache_dir.join("catalogue").exists());
    drop(source);
    std::fs::remove_dir_all(bare).unwrap();
}
