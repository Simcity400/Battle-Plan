
use super::*;
use crate::{
    Source,
    catalogue::{self, Catalogue},
    products::ContentOptions,
    test_fixtures::{self, source_with_content},
};
use content_reader_layers::content::visual::Choice;
use content_reader_image::Rgba8Image;
use std::{path::Path, sync::Arc};

fn png(rgba: [u8; 4]) -> Vec<u8> {
    dashboard_text::catalogue::texture_tests::png_bytes(&Rgba8Image {
        width: 1,
        height: 1,
        pixels: rgba.to_vec(),
    })
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

struct Machine {
    root: std::path::PathBuf,
    mods: std::path::PathBuf,
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.mods);
    }
}

fn machine(name: &str) -> (Machine, Source, ContentOptions) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root =
        std::env::temp_dir().join(format!("hoi4_presentation_{name}_{}", std::process::id()));
    let mods = root.with_file_name(format!(
        "hoi4_presentation_{name}_{}_mods",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&mods);
    std::fs::create_dir_all(&root).unwrap();
    dashboard_text::catalogue::texture_tests::write_sprite_program(&root);
    write(
        &root,
        "interface/art.gfx",
        br#"spriteTypes = {
            spriteType = { name = GFX_icon textureFile = "gfx/icon.png" }
            spriteType = { name = GFX_hidden textureFile = "gfx/hidden/hidden.png" }
        }"#,
    );
    write(&root, "gfx/icon.png", &png(RED));
    write(&root, "gfx/hidden/hidden.png", &png(RED));
    write(
        &root,
        "localisation/a_l_english.yml",
        "\u{feff}l_english:\n NAME:0 \"Recorded\"\n".as_bytes(),
    );
    write(&root, "common/scripted/a.txt", b"recorded");
    write(
        &root,
        "common/defines/00_fixture.lua",
        b"NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, \
          NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"fixture_officers\" } } }",
    );
    let art = mods.join("art");
    write(&art, "gfx/icon.png", &png(BLUE));
    write(
        &art,
        "localisation/replace/b_l_english.yml",
        "\u{feff}l_english:\n NAME:0 \"Styled\"\n".as_bytes(),
    );
    write(&art, "common/scripted/a.txt", b"mod script");
    install(&root, "art.mod", "Art", &art);
    let (source, context) =
        source_with_content(vec![test_fixtures::source(1, 100, Some(1))], &root);
    (Machine { root, mods }, source, context.content.unwrap())
}

fn install(root: &Path, file: &str, name: &str, folder: &Path) {
    write(
        root,
        &format!("user/mod/{file}"),
        format!(
            "name=\"{name}\"\npath=\"{}\"\n",
            folder.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
}

fn choice(id: &str, name: &str) -> Choice {
    Choice {
        id: id.into(),
        name: name.into(),
    }
}

#[test]
fn preparation_and_requests_showing_visual_mods_keep_their_own_cache_entries() {
    use crate::products::politics_content::politics_content;
    let (_machine, source, mut options) = machine("cache_slots");
    present(&source, &mut options, &[]);
    let preparation = source.admit(crate::source::Work::Recorded).unwrap();
    let recorded = politics_content(&source, &preparation).unwrap();
    assert!(Arc::ptr_eq(
        &recorded,
        &politics_content(&source, &options).unwrap()
    ));
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let preparation = source.admit(crate::source::Work::Recorded).unwrap();
    let shown = politics_content(&source, &options).unwrap();
    assert!(!Arc::ptr_eq(&shown, &recorded), "the shown catalogue's own");
    for _ in 0..2 {
        assert!(Arc::ptr_eq(
            &politics_content(&source, &preparation).unwrap(),
            &recorded
        ));
        assert!(Arc::ptr_eq(
            &politics_content(&source, &options).unwrap(),
            &shown
        ));
    }
}

#[test]
fn the_gui_registry_is_read_ahead_in_the_mode_shields_read_it() {
    let (_machine, source, mut options) = machine("read_ahead_guis");
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let catalogue = crate::catalogue::bound_catalogue(&source, &options)
        .unwrap()
        .unwrap();
    assert!(catalogue.content.layers.presentation().shows_visual_mods());
    crate::products::read_ahead::warm_guis(&catalogue);
    let loads = catalogue.content_loads();
    let _ = content_reader_layers::content::strict(|| catalogue.guis().is_ok());
    assert_eq!(catalogue.content_loads(), loads);
}

fn present(source: &Source, options: &mut ContentOptions, selected: &[Choice]) -> bool {
    let changed = bind(source, options, selected);
    *options = source
        .admit_resolved(crate::source::Work::Content(never()))
        .unwrap();
    changed
}

fn bind(source: &Source, options: &mut ContentOptions, selected: &[Choice]) -> bool {
    let presentation = Presentation::current(
        &options.options,
        source.replay_ref(),
        selected,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let changed = source.bind(
        Some((&options.options, options.allow_unverified)),
        presentation,
    );
    *options = source.admit(crate::source::Work::Content(never())).unwrap();
    changed
}

fn never() -> Arc<dyn crate::presentation::Cancel> {
    Arc::new(std::sync::atomic::AtomicBool::new(false))
}

fn mounted(source: &Source) -> Option<Presentation> {
    source
        .binding()?
        .mount
        .get()
        .map(|presented| presented.presentation.clone())
}

fn drawn(catalogue: &Catalogue, key: &str) -> ([u8; 4], Option<String>) {
    let (image, own) = dashboard_text::catalogue::capture(|| {
        let sprite = catalogue.sprites.find(key).unwrap().unwrap();
        dashboard_text::catalogue::sprite_frame_png(
            &catalogue.content.layers,
            sprite,
            *catalogue.gui_decode.as_ref().unwrap(),
            0,
            32,
        )
        .unwrap()
    });
    let image = dashboard_text::catalogue::texture_tests::from_url(&image);
    (image.pixels[..4].try_into().unwrap(), own.fallback_line())
}

fn text(layers: &content_reader_layers::ResolvedLayers, path: &str) -> String {
    std::str::from_utf8(&layers.read(path).unwrap()).unwrap().to_owned()
}

#[test]
fn each_source_shows_its_own_visual_mods_and_recorded_facts_stay_recorded() {
    let (machine, styled, mut options) = machine("two_sources");
    let (plain, _) =
        source_with_content(vec![test_fixtures::source(1, 100, Some(1))], &machine.root);
    let mut plain_options = options.clone();
    assert!(present(
        &styled,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    present(&plain, &mut plain_options, &[]);
    assert!(plain_options.presentation.identity().is_empty());
    let shown = catalogue::catalogue(&styled, &options).unwrap();
    let recorded = catalogue::catalogue(&plain, &plain_options).unwrap();
    assert_eq!(drawn(&shown, "GFX_icon"), (BLUE, None));
    assert_eq!(drawn(&recorded, "GFX_icon"), (RED, None));
    assert_eq!(
        shown.presentation_localisation().unwrap().get("NAME"),
        Some("Recorded"),
        "the mod's text is not read"
    );
    assert_eq!(shown.localisation.get("NAME"), Some("Recorded"));
    assert_eq!(
        recorded.presentation_localisation().unwrap().get("NAME"),
        Some("Recorded")
    );
    assert_eq!(
        text(&shown.content.layers, "common/scripted/a.txt"),
        "recorded"
    );
    assert_eq!(
        text(shown.content.layers.presentation(), "common/scripted/a.txt"),
        "recorded"
    );
    let mut warnings = Vec::new();
    styled.append_presentation_warnings(&mut warnings, Some(&options));
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut warnings = Vec::new();
    plain.append_presentation_warnings(&mut warnings, Some(&plain_options));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn selection_changes_updates_and_removals_rebind_the_catalogue() {
    let (machine, source, mut options) = machine("changes");
    let other = machine.mods.join("other");
    write(&other, "gfx/unused.png", &png(GREEN));
    install(&machine.root, "other.mod", "Other", &other);
    let both = [
        choice("mod/art.mod", "Art"),
        choice("mod/other.mod", "Other"),
    ];
    assert!(present(&source, &mut options, &both));
    let first = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(drawn(&first, "GFX_icon").0, BLUE);
    assert!(!present(
        &source,
        &mut options,
        &[both[1].clone(), both[0].clone()]
    ));
    assert!(Arc::ptr_eq(
        &first,
        &catalogue::catalogue(&source, &options).unwrap()
    ));
    assert!(present(&source, &mut options, &both[..1]));
    let changed = catalogue::catalogue(&source, &options).unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    write(&machine.mods, "art/gfx/icon.png", &png(GREEN));
    write(&machine.mods, "art/gfx/icon2.png", &png(GREEN));
    assert!(present(&source, &mut options, &both[..1]));
    let updated = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(drawn(&updated, "GFX_icon").0, GREEN);
    std::fs::remove_file(machine.root.join("user/mod/art.mod")).unwrap();
    assert!(present(&source, &mut options, &both[..1]));
    let removed = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(drawn(&removed, "GFX_icon").0, RED);
    let mut warnings = Vec::new();
    source.append_presentation_warnings(&mut warnings, Some(&options));
    assert!(
        warnings
            .iter()
            .any(|line| line.contains("Art") && line.contains("not shown")),
        "{warnings:?}"
    );
}

#[test]
fn archive_mods_show_and_what_they_hide_is_drawn_from_the_recording_by_name() {
    use content_reader_layers::test_support::{Record, zip_records};
    let (machine, source, mut options) = machine("archive");
    let png = png(GREEN);
    write(
        &machine.mods,
        "art.zip",
        &zip_records(
            &[],
            &[Record {
                name: b"gfx/icon.png",
                host: 0,
                flags: 0,
                deflate: true,
                data: &png,
            }],
        ),
    );
    write(
        &machine.root,
        "user/mod/archive.mod",
        format!(
            "name=\"Archive\"\nreplace_path=\"gfx/hidden\"\narchive=\"{}\"\n",
            machine
                .mods
                .join("art.zip")
                .display()
                .to_string()
                .replace('\\', "/")
        )
        .as_bytes(),
    );
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/archive.mod", "Archive")]
    ));
    let catalogue = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(drawn(&catalogue, "GFX_icon"), (GREEN, None));
    let (colour, line) = drawn(&catalogue, "GFX_hidden");
    assert_eq!(colour, RED, "the recording's own style");
    let line = line.expect("the fallback is named on this reply");
    assert!(
        line.starts_with("Artwork from visual mods not shown: 1 file(s)"),
        "{line}"
    );
    assert!(line.contains("sprite GFX_hidden"), "{line}");
    assert_eq!(
        drawn(&catalogue, "GFX_hidden").1.as_deref(),
        Some(line.as_str())
    );
    assert_eq!(drawn(&catalogue, "GFX_icon").1, None);
}

#[test]
fn late_work_never_binds_a_replaced_presentation() {
    let (_machine, source, mut options) = machine("late");
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let shown = options.clone();
    let bound = catalogue::catalogue(&source, &options).unwrap();
    assert!(Arc::ptr_eq(
        &bound,
        &catalogue::bound_catalogue(&source, &options)
            .unwrap()
            .unwrap()
    ));
    assert!(present(&source, &mut options, &[]));
    assert!(
        catalogue::bound_catalogue(&source, &shown).is_none(),
        "a read-ahead of the old presentation binds nothing"
    );
    let ahead = catalogue::bound_catalogue(&source, &options)
        .unwrap()
        .unwrap();
    assert_eq!(ahead.presentation, Presentation::default());
    let late = catalogue::catalogue(&source, &shown).unwrap();
    assert_eq!(late.presentation, *shown.presentation.identity());
    assert_eq!(drawn(&late, "GFX_icon").0, BLUE);
    assert_eq!(
        mounted(&source),
        None,
        "a late request's mount is its own; the current binding shows no visual mod"
    );
    let current = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(drawn(&current, "GFX_icon").0, RED);
    assert!(Arc::ptr_eq(&current, &ahead));
    assert!(!Arc::ptr_eq(&current, &late));
    assert!(Arc::ptr_eq(
        &current,
        &catalogue::catalogue(&source, &options).unwrap()
    ));
    assert_eq!(mounted(&source), None);
}

#[test]
fn prepared_artwork_is_the_recordings_and_named() {
    let (_machine, source, mut options) = machine("prepared");
    assert!(
        Arc::ptr_eq(
            &catalogue::recorded_catalogue(&source, &options).unwrap(),
            &catalogue::catalogue(&source, &options).unwrap()
        ),
        "without visual mods both are the one catalogue"
    );
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let shown = catalogue::catalogue(&source, &options).unwrap();
    let (recorded, own) = dashboard_text::catalogue::capture(|| {
        catalogue::recorded_catalogue(&source, &options).unwrap()
    });
    assert_eq!(own.fallback_line(), None, "reading it credits nothing");
    assert_eq!(drawn(&recorded, "GFX_icon"), (RED, None));
    assert_eq!(drawn(&shown, "GFX_icon"), (BLUE, None));
    assert!(Arc::ptr_eq(
        &shown,
        &catalogue::catalogue(&source, &options).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &recorded,
        &catalogue::recorded_catalogue(&source, &options).unwrap()
    ));
}

#[test]
fn a_redeclared_sprite_is_not_read_and_the_recordings_own_declaration_is_drawn() {
    let (machine, source, mut options) = machine("redeclared");
    let redeclared = machine.mods.join("redeclared");
    write(
        &redeclared,
        "interface/art.gfx",
        br#"spriteTypes = {
            spriteType = { name = GFX_icon textureFile = "gfx/redeclared_only.png" }
        }"#,
    );
    write(&redeclared, "gfx/redeclared_only.png", &png(BLUE));
    install(&machine.root, "redeclared.mod", "Redeclared", &redeclared);
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/redeclared.mod", "Redeclared")]
    ));
    let catalogue = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(
        catalogue
            .sprites
            .find("GFX_icon")
            .unwrap()
            .unwrap()
            .texture
            .as_deref(),
        Some("gfx/icon.png")
    );
    assert_eq!(drawn(&catalogue, "GFX_icon"), (RED, None));
}

#[test]
fn a_sprite_the_mods_art_cannot_draw_falls_back_to_the_recordings_art() {
    let (machine, source, mut options) = machine("broken_art");
    let broken = machine.mods.join("broken");
    write(&broken, "gfx/icon.png", b"not a png");
    install(&machine.root, "broken.mod", "Broken", &broken);
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/broken.mod", "Broken")]
    ));
    let catalogue = catalogue::catalogue(&source, &options).unwrap();
    let (colour, line) = drawn(&catalogue, "GFX_icon");
    assert_eq!(colour, RED, "the recording's own GFX_icon");
    assert!(line.unwrap().contains("sprite GFX_icon ("));
}

#[test]
fn recorded_content_kept_under_visual_mods_names_nothing_once_they_are_cleared() {
    let (_machine, source, mut options) = machine("stored_cleared");
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let kept = catalogue::CachedResult::default();
    let read = |options: &ContentOptions| {
        dashboard_text::catalogue::capture(|| {
            kept.get_or_init(|| {
                crate::products::politics_content::recorded_politics_content(&source, options)
            })
            .unwrap()
        })
        .1
        .fallback_line()
    };
    assert_eq!(read(&options), None, "the load names nothing");
    assert!(present(&source, &mut options, &[]));
    assert_eq!(read(&options), None, "nor does a later reply reading it");
}

#[test]
fn a_selections_identity_depends_on_the_recordings_own_mods() {
    let (_machine, source, options) = machine("recorded_mods");
    let selected = [choice("mod/art.mod", "Art")];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let alone =
        content_reader_layers::content::visual::revision(&options.options, &selected, &[], &cancel)
            .unwrap();
    let recorded = [content_reader_layers::content::visual::RecordedMod {
        id: "other".into(),
        name: "Art".into(),
    }];
    let beside =
        content_reader_layers::content::visual::revision(&options.options, &selected, &recorded, &cancel)
            .unwrap();
    assert_ne!(alone, beside);
    assert_eq!(
        Presentation::current(&options.options, source.replay_ref(), &selected, &cancel)
            .unwrap()
            .revision,
        alone
    );
}

#[test]
fn a_memoised_portrait_names_its_fallback_on_every_request() {
    let (machine, source, mut options) = machine("portrait_memo");
    let broken = machine.mods.join("broken");
    write(&broken, "gfx/icon.png", b"not a png");
    install(&machine.root, "broken.mod", "Broken", &broken);
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/broken.mod", "Broken")]
    ));
    let politics = crate::products::politics_content::politics_content(&source, &options).unwrap();
    let request = || {
        let (portrait, own) = dashboard_text::catalogue::capture(|| {
            politics.recorded_portrait(0, b"GFX_icon").unwrap()
        });
        let image = dashboard_text::catalogue::texture_tests::from_url(&portrait);
        (image.pixels[..4].to_vec(), own.fallback_line())
    };
    let (colour, first) = request();
    assert_eq!(colour, RED.to_vec());
    assert!(
        first.as_deref().unwrap().contains("portrait GFX_icon ("),
        "{first:?}"
    );
    assert_eq!(
        request(),
        (RED.to_vec(), first),
        "the kept portrait names it again"
    );
}

#[test]
fn work_on_any_thread_draws_the_presentation_its_request_was_admitted_under() {
    let (_machine, source, mut options) = machine("worker");
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let admitted = options.clone();
    let worker_colour = std::thread::scope(|scope| {
        let (selected, selection_changed) = std::sync::mpsc::channel();
        let (worker_source, worker_admitted) = (&source, &admitted);
        let worker = scope.spawn(move || {
            selection_changed.recv().unwrap();
            let catalogue = catalogue::catalogue(worker_source, worker_admitted).unwrap();
            let colour = drawn(&catalogue, "GFX_icon").0;
            (catalogue.presentation.clone(), colour)
        });
        assert!(present(&source, &mut options, &[]));
        selected.send(()).unwrap();
        worker.join().unwrap()
    });
    assert_eq!(
        worker_colour,
        (admitted.presentation.identity().clone(), BLUE)
    );
    assert_eq!(
        catalogue::binding(&admitted).2,
        *admitted.presentation.identity()
    );
    assert_eq!(source.presentation(), Presentation::default());
    assert_ne!(
        mounted(&source),
        Some(admitted.presentation.identity().clone())
    );
    assert_eq!(
        drawn(
            &catalogue::catalogue(&source, &options).unwrap(),
            "GFX_icon"
        )
        .0,
        RED
    );
}

#[test]
fn a_reply_names_the_presentation_it_was_admitted_under() {
    let (machine, source, mut options) = machine("admitted_notes");
    let other = machine.mods.join("other");
    write(&other, "gfx/icon.png", &png(GREEN));
    install(&machine.root, "other.mod", "Other", &other);
    let notes = |options: &ContentOptions| {
        let mut warnings = Vec::new();
        source.append_presentation_warnings(&mut warnings, Some(options));
        warnings
    };
    let switch_five_times = |options: &mut ContentOptions| {
        for selection in [
            vec![choice("mod/other.mod", "Other")],
            vec![choice("mod/gone.mod", "Gone")],
            vec![
                choice("mod/art.mod", "Art"),
                choice("mod/other.mod", "Other"),
            ],
            vec![
                choice("mod/gone.mod", "Gone"),
                choice("mod/other.mod", "Other"),
            ],
            vec![],
        ] {
            present(&source, options, &selection);
            catalogue::catalogue(&source, options).unwrap();
        }
    };
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let healthy = options.clone();
    assert_eq!(
        drawn(
            &catalogue::catalogue(&source, &healthy).unwrap(),
            "GFX_icon"
        )
        .0,
        BLUE
    );
    switch_five_times(&mut options);
    assert!(notes(&healthy).is_empty(), "{:?}", notes(&healthy));
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let admitted = options.clone();
    switch_five_times(&mut options);
    assert_eq!(
        drawn(
            &catalogue::catalogue(&source, &admitted).unwrap(),
            "GFX_icon"
        )
        .0,
        BLUE
    );
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/gone.mod", "Gone")]
    ));
    let gone = options.clone();
    assert_eq!(
        drawn(&catalogue::catalogue(&source, &gone).unwrap(), "GFX_icon").0,
        RED
    );
    switch_five_times(&mut options);
    let refused = notes(&gone);
    assert!(
        refused
            .iter()
            .any(|line| line.contains("Gone") && line.contains("not shown")),
        "{refused:?}"
    );
    let idle = source.admit(crate::source::Work::Recorded).unwrap();
    assert!(notes(&idle).is_empty());
}

#[test]
fn admission_never_mixes_one_bindings_folders_with_anothers_mods() {
    let (_machine, source, options) = machine("mixed");
    let art = Presentation::current(
        &options.options,
        source.replay_ref(),
        &[choice("mod/art.mod", "Art")],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let mut other_folders = options.options.clone();
    other_folders.user_dir = other_folders.user_dir.join("elsewhere");
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for round in 0..2000 {
                if round % 2 == 0 {
                    source.bind(Some((&options.options, true)), art.clone());
                } else {
                    source.bind(Some((&other_folders, true)), Presentation::default());
                }
            }
            done.store(true, std::sync::atomic::Ordering::Release);
        });
        while !done.load(std::sync::atomic::Ordering::Acquire) {
            let Some(admitted) = source.admit(crate::source::Work::Ahead) else {
                continue;
            };
            let paired = if admitted.options.user_dir == options.options.user_dir {
                *admitted.presentation.identity() == art
            } else {
                admitted.presentation.identity().is_empty()
            };
            assert!(
                paired,
                "{:?} with {:?}",
                admitted.options.user_dir, admitted.presentation
            );
        }
    });
}

#[test]
fn a_slow_mount_blocks_no_admission_and_a_cancelled_one_is_retried() {
    use crate::presentation::Cancel;
    let (_machine, source, mut options) = machine("slow_mount");
    assert!(bind(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let mut slow = options.clone();
    let (computing, finish) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
    let computer = Arc::new(std::sync::atomic::AtomicBool::new(false));
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            slow.presentation
                .binding()
                .mount
                .get_or_mount(computer.flag(), || {
                    computing.wait();
                    finish.wait();
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ))
                })
        });
        computing.wait();
        assert!(source.admit(crate::source::Work::Recorded).is_some());
        assert!(present(&source, &mut options, &[]));
        assert!(
            source
                .admit(crate::source::Work::Content(never()))
                .is_some()
        );
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let waited = slow
            .presentation
            .binding()
            .mount
            .get_or_mount(&cancelled, || unreachable!("another computes it"));
        assert_eq!(
            waited.err().map(|error| error.kind()),
            Some(std::io::ErrorKind::Interrupted)
        );
        finish.wait();
        assert!(first.join().unwrap().is_err());
    });
    assert!(slow.presentation.held().is_none());
    source.resolve(&mut slow);
    let mount = source.presented_for(&slow).unwrap();
    assert_eq!(mount.presentation, *slow.presentation.identity());
    assert!(slow.presentation.held().is_some());
}

#[test]
fn every_content_reply_names_and_draws_its_admitted_mount() {
    let (machine, source, mut options) = machine("admitted_mount");
    let other = machine.mods.join("other");
    write(&other, "gfx/icon.png", &png(GREEN));
    install(&machine.root, "other.mod", "Other", &other);
    let notes = |options: &ContentOptions| {
        let mut warnings = Vec::new();
        source.append_presentation_warnings(&mut warnings, Some(options));
        warnings
    };
    assert!(present(
        &source,
        &mut options,
        &[choice("mod/gone.mod", "Gone")]
    ));
    let gone = options.clone();
    for _ in 0..5 {
        present(&source, &mut options, &[choice("mod/other.mod", "Other")]);
        present(&source, &mut options, &[]);
    }
    assert!(
        notes(&gone)
            .iter()
            .any(|line| line.contains("Gone") && line.contains("not shown")),
        "{:?}",
        notes(&gone)
    );
    let mounted = source.presented_for(&gone).unwrap();
    assert!(Arc::ptr_eq(&mounted, &gone.presentation.held().unwrap()));
    assert!(Arc::ptr_eq(&mounted, &source.presented_for(&gone).unwrap()));
    assert_eq!(
        drawn(&catalogue::catalogue(&source, &gone).unwrap(), "GFX_icon").0,
        RED
    );
}

#[test]
fn a_read_ahead_mid_mount_is_cancelled_by_a_binding_swap() {
    let (_machine, source, mut options) = machine("ahead_cancel");
    assert!(bind(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let ahead = source.admit(crate::source::Work::Ahead).unwrap();
    assert!(
        !ahead
            .presentation
            .cancel()
            .load(std::sync::atomic::Ordering::Acquire)
    );
    let (computing, swapped) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        let read_ahead = scope.spawn(|| {
            ahead
                .presentation
                .binding()
                .mount
                .get_or_mount(ahead.presentation.cancel(), || {
                    computing.wait();
                    swapped.wait();
                    assert!(
                        ahead
                            .presentation
                            .cancel()
                            .load(std::sync::atomic::Ordering::Acquire),
                        "the swap cancels the read-ahead"
                    );
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ))
                })
        });
        computing.wait();
        assert!(present(&source, &mut options, &[]));
        swapped.wait();
        assert!(read_ahead.join().unwrap().is_err());
    });
    assert!(ahead.presentation.held().is_none(), "nothing mounted");
    assert!(
        !options
            .presentation
            .cancel()
            .load(std::sync::atomic::Ordering::Acquire)
    );
}

#[test]
fn a_stale_rebind_retries_and_the_newest_configuration_wins() {
    use crate::presentation::{Config, rebind};
    let (_machine, source, options) = machine("rebind");
    let art = Presentation::current(
        &options.options,
        source.replay_ref(),
        &[choice("mod/art.mod", "Art")],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let mut elsewhere = options.options.clone();
    elsewhere.user_dir = elsewhere.user_dir.join("elsewhere");
    type Settings = Option<(content_reader_layers::ResolveOptions, Presentation)>;
    let run = |config: &Config<Settings>, change: Option<Box<dyn Fn(&Config<Settings>) + '_>>| {
        let derived = std::sync::atomic::AtomicUsize::new(0);
        rebind(
            config,
            &std::sync::atomic::AtomicBool::new(false),
            |settings: &Settings| {
                if derived.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
                    && let Some(change) = &change
                {
                    change(config);
                }
                Ok(settings.clone())
            },
            |_, settings: Settings| {
                let presentation = settings
                    .as_ref()
                    .map(|(_, presentation)| presentation.clone())
                    .unwrap_or_default();
                source.bind(
                    settings.as_ref().map(|(folders, _)| (folders, true)),
                    presentation,
                );
            },
        )
        .unwrap();
        derived.load(std::sync::atomic::Ordering::SeqCst)
    };
    let shows = |folders: &content_reader_layers::ResolveOptions, presentation: &Presentation| {
        source.binding().is_some_and(|binding| {
            binding.options.user_dir == folders.user_dir && binding.presentation == *presentation
        })
    };
    let config = Config::new(Some((options.options.clone(), Presentation::default())));
    let retried = run(
        &config,
        Some(Box::new(|config: &Config<Settings>| {
            config.update(|settings| {
                settings.as_mut().unwrap().1 = art.clone();
            })
        })),
    );
    assert_eq!(retried, 2, "the stale derivation is redone");
    assert!(shows(&options.options, &art));
    let retried = run(
        &config,
        Some(Box::new(|config: &Config<Settings>| {
            config.update(|settings| {
                *settings = Some((elsewhere.clone(), Presentation::default()));
            })
        })),
    );
    assert_eq!(retried, 2);
    assert!(shows(&elsewhere, &Presentation::default()));
    let retried = run(
        &config,
        Some(Box::new(|config: &Config<Settings>| {
            config.update(|settings| *settings = None)
        })),
    );
    assert_eq!(retried, 2);
    assert!(source.binding().is_none());
    let retried = run(
        &config,
        Some(Box::new(|config: &Config<Settings>| {
            config.update(|settings| *settings = Some((options.options.clone(), art.clone())))
        })),
    );
    assert_eq!(retried, 2);
    assert!(shows(&options.options, &art));
    assert_eq!(
        rebind(
            &config,
            &std::sync::atomic::AtomicBool::new(true),
            |_| Ok(()),
            |_, ()| unreachable!("a cancelled rebind installs nothing"),
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::Interrupted
    );
}

#[test]
fn a_rebind_paused_across_shutdown_installs_nothing() {
    use crate::presentation::{Config, rebind};
    let (_machine, source, options) = machine("rebind_shutdown");
    let config = Config::new(Some(options.options));
    let never = std::sync::atomic::AtomicBool::new(false);
    let install = |_: &Option<content_reader_layers::ResolveOptions>,
                   folders: Option<content_reader_layers::ResolveOptions>| {
        source.bind(
            folders.as_ref().map(|folders| (folders, true)),
            Presentation::default(),
        );
    };
    let derived = std::sync::atomic::AtomicUsize::new(0);
    let error = rebind(
        &config,
        &never,
        |folders| {
            derived.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            config.close();
            source.bind(None, Presentation::default());
            Ok(folders.clone())
        },
        install,
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(
        derived.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a closed configuration is not derived again"
    );
    assert!(source.binding().is_none(), "the stale derivation is not installed");
    let error = rebind(
        &config,
        &never,
        |_| -> std::io::Result<Option<content_reader_layers::ResolveOptions>> {
            unreachable!("a closed configuration is not derived")
        },
        install,
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(source.binding().is_none());
}

#[test]
fn a_catalogue_waiter_is_cancelled_while_another_request_mounts() {
    let (_machine, source, mut options) = machine("catalogue_waiter");
    assert!(bind(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let waiter = source
        .admit(crate::source::Work::Content(
            Arc::clone(&cancel) as Arc<dyn crate::presentation::Cancel>
        ))
        .unwrap();
    let (computing, finish) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        let mounting = scope.spawn(|| {
            options
                .presentation
                .binding()
                .mount
                .get_or_mount(options.presentation.cancel(), || {
                    computing.wait();
                    finish.wait();
                    source.presented_for(&source.admit(crate::source::Work::Recorded).unwrap())
                })
        });
        computing.wait();
        let waiting = scope.spawn(|| {
            let mut waiter = waiter.clone();
            source.resolve(&mut waiter);
            catalogue::catalogue(&source, &waiter)
        });
        cancel.store(true, std::sync::atomic::Ordering::Release);
        let waited = waiting.join().unwrap();
        assert_eq!(
            waited.err().map(|error| error.kind()),
            Some(std::io::ErrorKind::Interrupted)
        );
        finish.wait();
        mounting.join().unwrap().unwrap();
    });
}

#[test]
fn preparation_never_mounts_the_selected_visual_mods() {
    let (_machine, source, mut options) = machine("preparation");
    assert!(bind(
        &source,
        &mut options,
        &[choice("mod/art.mod", "Art")]
    ));
    let preparation = source.admit(crate::source::Work::Recorded).unwrap();
    let recorded = catalogue::catalogue(&source, &preparation).unwrap();
    assert_eq!(drawn(&recorded, "GFX_icon").0, RED);
    crate::products::politics_content::politics_content(&source, &preparation).unwrap();
    assert!(
        source.binding().unwrap().mount.get().is_none(),
        "nothing mounted the selected visual mods"
    );
    let mut warnings = Vec::new();
    source.append_presentation_warnings(&mut warnings, Some(&preparation));
    assert!(warnings.is_empty());
    assert!(source.binding().unwrap().mount.get().is_none());
}
