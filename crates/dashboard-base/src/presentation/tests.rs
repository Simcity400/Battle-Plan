//! One presentation per source: catalogues and artwork follow the source's own visual mods, a
//! change rebinds them, recorded facts (declarations, text, scripts) never follow, and art the
//! mods cannot supply is drawn from the recording and named on the reply that drew it.

use super::*;
use crate::{
    Source,
    catalogue::{self, Catalogue},
    products::ContentOptions,
    test_fixtures::{self, source_with_content},
};
use installed_content_core::content::visual::Choice;
use installed_content_image::Rgba8Image;
use std::{path::Path, sync::Arc};

/// A one-pixel PNG of `rgba`.
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

/// A synthetic game (its user folder inside) and, beside it, the folder installed mods live in.
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

/// A recording over a synthetic game whose `GFX_icon` is red, whose `GFX_hidden` is drawn from
/// `gfx/hidden/hidden.png`, with recorded text and a recorded script, and a user folder that
/// installs the folder mod `Art` (a blue icon, its own text and a script of its own).
fn machine(name: &str) -> (Machine, Source, ContentOptions) {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // What politics content reads of the defines.
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

/// Installs a folder mod named `name` through the user descriptor `file`.
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

/// Preparation reads the recording's own catalogue and requests showing visual mods the shown
/// one; a cache built from a catalogue keeps an entry for each, so alternating between them
/// loads each once instead of displacing the other's. Without a visual mod both read the
/// recording's catalogue and share one entry.
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

/// Under visual mods the GUI registry is read ahead strictly, as a flag's shield reads it, so the
/// first shield reads no interface file again.
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
    let _ = installed_content_core::content::strict(|| catalogue.guis().is_ok());
    assert_eq!(catalogue.content_loads(), loads);
}

/// Shows `selected` on `source`, as the service does from its campaign's settings, and admits
/// the next request of content under it (`options`, as the service admits a request, its mount
/// resolved at its entry).
fn present(source: &Source, options: &mut ContentOptions, selected: &[Choice]) -> bool {
    let changed = bind(source, options, selected);
    *options = source
        .admit_resolved(crate::source::Work::Content(never()))
        .unwrap();
    changed
}

/// Shows `selected` on `source` with no request admitted yet, so nothing has mounted it; then
/// `options` is a request admitted under it whose entry has not resolved its mount.
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

/// What cancels work nobody cancels.
fn never() -> Arc<dyn crate::presentation::Cancel> {
    Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// The presentation the source's current binding has mounted, if any work has needed it.
fn mounted(source: &Source) -> Option<Presentation> {
    source
        .binding()?
        .mount
        .get()
        .map(|presented| presented.presentation.clone())
}

/// The colour of `key`'s first frame as a widget draws it, and the line its reply carries for
/// what it drew in the recording's style.
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

fn text(layers: &installed_content_core::ResolvedLayers, path: &str) -> String {
    std::str::from_utf8(&layers.read(path).unwrap()).unwrap().to_owned()
}

/// Two sources over the same recording: each draws its own campaign's visual mods, and neither
/// leaks into the other. Artwork follows the mods; text and scripts are recorded facts, so the
/// mod's own text and script are never read, not even by the presentation stack.
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
    // The plain source is bound to the folders with no visual mod.
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
    // Recorded facts read the recorded stack, with or without the presentation.
    assert_eq!(
        text(&shown.content.layers, "common/scripted/a.txt"),
        "recorded"
    );
    assert_eq!(
        text(shown.content.layers.presentation(), "common/scripted/a.txt"),
        "recorded"
    );
    // A selection that mounts in full adds no line to a reply: it reads as the recording's own.
    let mut warnings = Vec::new();
    styled.append_presentation_warnings(&mut warnings, Some(&options));
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut warnings = Vec::new();
    plain.append_presentation_warnings(&mut warnings, Some(&plain_options));
    assert!(warnings.is_empty(), "{warnings:?}");
}

/// A selection changed, reordered, updated or removed: the same mods in another order are one
/// identity, and every other change rebinds the catalogue, so its artwork is the mods' now.
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
    // Reordered: the engine decides mount order, so nothing changes.
    assert!(!present(
        &source,
        &mut options,
        &[both[1].clone(), both[0].clone()]
    ));
    assert!(Arc::ptr_eq(
        &first,
        &catalogue::catalogue(&source, &options).unwrap()
    ));
    // Changed: one mod fewer is another presentation.
    assert!(present(&source, &mut options, &both[..1]));
    let changed = catalogue::catalogue(&source, &options).unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    // Updated: the mod's icon is now green, which its revision sees.
    write(&machine.mods, "art/gfx/icon.png", &png(GREEN));
    write(&machine.mods, "art/gfx/icon2.png", &png(GREEN));
    assert!(present(&source, &mut options, &both[..1]));
    let updated = catalogue::catalogue(&source, &options).unwrap();
    assert_eq!(drawn(&updated, "GFX_icon").0, GREEN);
    // Removed: the mod is named as not shown and the recording's icon is drawn.
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

/// An archive mod's artwork shows, and a file its `replace_path` hides that it does not supply
/// is drawn from the recording, named on the reply that drew it and on each reply that reuses
/// the kept image, never on another widget's.
#[test]
fn archive_mods_show_and_what_they_hide_is_drawn_from_the_recording_by_name() {
    use installed_content_core::test_support::{Record, zip_records};
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
    // The kept image names it again; a widget drawing only what the mods supply names nothing.
    assert_eq!(
        drawn(&catalogue, "GFX_hidden").1.as_deref(),
        Some(line.as_str())
    );
    assert_eq!(drawn(&catalogue, "GFX_icon").1, None);
}

/// Work done ahead of a request never binds a presentation a request has replaced, and one a
/// late request asks for is mounted for it alone.
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
    // The selection is cleared while work admitted under the old one is still running.
    assert!(present(&source, &mut options, &[]));
    assert!(
        catalogue::bound_catalogue(&source, &shown).is_none(),
        "a read-ahead of the old presentation binds nothing"
    );
    // The new selection shows no visual mod, so read-ahead of it binds the recording's own
    // catalogue, in a slot of its own: the old presentation's, which a request bound, stays.
    let ahead = catalogue::bound_catalogue(&source, &options)
        .unwrap()
        .unwrap();
    assert_eq!(ahead.presentation, Presentation::default());
    // A late request of the old presentation draws it for itself alone.
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

/// Prepared families whose stored payloads hold artwork read the recording's while visual mods
/// are shown, from a catalogue of their own that never displaces the shown one. Reading it
/// credits nothing; the replies of those families name it themselves.
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

/// A sprite the visual mods redeclare is a recorded fact's and is never read: it is drawn as the
/// recording declares it, with nothing to name, though the mods' declaration would draw.
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

/// A sprite whose art the visual mods replace with art that does not load is drawn from the
/// recording's own art, and named.
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

/// Recorded content a shared cache loaded while visual mods were shown keeps no note of them:
/// the load credits nothing, so once the selection is cleared a reply reading what was kept
/// names nothing either.
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

/// What a selection shows depends on the recording's own mods too, so two recordings with the
/// same selection share a presentation only when they also share those mods.
#[test]
fn a_selections_identity_depends_on_the_recordings_own_mods() {
    let (_machine, source, options) = machine("recorded_mods");
    let selected = [choice("mod/art.mod", "Art")];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let alone =
        installed_content_core::content::visual::revision(&options.options, &selected, &[], &cancel)
            .unwrap();
    let recorded = [installed_content_core::content::visual::RecordedMod {
        id: "other".into(),
        name: "Art".into(),
    }];
    let beside =
        installed_content_core::content::visual::revision(&options.options, &selected, &recorded, &cancel)
            .unwrap();
    assert_ne!(alone, beside);
    assert_eq!(
        Presentation::current(&options.options, source.replay_ref(), &selected, &cancel)
            .unwrap()
            .revision,
        alone
    );
}

/// A portrait kept from an earlier request names what its draw fell back on again on every
/// later request, not only the first.
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

/// A request carries the presentation it was admitted under, by value, onto every thread it
/// starts: art a worker draws while the selection changes is still the admitted one's, never
/// the source's new one, and nothing of it is kept as the source's.
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
            // The worker starts drawing only once the selection has changed under it.
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
    // The source shows its new selection, and the admitted one is not its binding's.
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

/// A reply admitted under one presentation and finished after the selection changed many times
/// names what its own presentation could not show, from the mount it was admitted with, never the
/// source's current one: a refused selection says why however often the selection moves on, and
/// a healthy one never borrows a later selection's refusal. Nothing of it is looked up in the
/// source, so no number of switches loses it.
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
    // A healthy selection draws its art and, after five switches through refused ones, still
    // names nothing.
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
    // Admitted under Art and drawing only after the five switches, it still draws Art.
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
    // A selection that cannot be shown (its mod is no longer installed) draws the recording's
    // art, and its reply says why, however often the selection moves on.
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
    // Work that reads no content is admitted without a mount and names nothing.
    let idle = source.admit(crate::source::Work::Recorded).unwrap();
    assert!(notes(&idle).is_empty());
}

/// A folder and visual mod change is one swap of the source's binding: work admitted while the
/// configuration changes over and over always holds one binding's folders with that binding's
/// mods, never one's folders with another's.
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
                // Folders and mods change together.
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

/// A slow mount of one binding blocks no admission: neither recording-only work on it nor work
/// on a new binding waits for it, and work on the slow binding that is cancelled stops waiting.
/// A mount whose computation is cancelled leaves the cell empty, and the next work computes it.
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
                    // Cancelled while it computed: nothing is mounted.
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ))
                })
        });
        computing.wait();
        // While A's mount is computed: recording-only work is admitted at once, a new binding
        // is admitted at once, and a cancelled waiter on A stops waiting.
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
    // The next work on A's binding computes its mount at its entry.
    assert!(slow.presentation.held().is_none());
    source.resolve(&mut slow);
    let mount = source.presented_for(&slow).unwrap();
    assert_eq!(mount.presentation, *slow.presentation.identity());
    assert!(slow.presentation.held().is_some());
}

/// What a content reply names comes from its admission alone: work that reads no art, or reads
/// art kept from an earlier request (a flag context, a focus evaluator, a prepared answer), names
/// its own presentation all the same, and every art read draws the admitted mount, however the
/// selection has moved on.
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
    // An uninstalled selection names why it is not shown before any art is read.
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
    // Every art read of the admitted work draws the mount it was admitted with.
    let mounted = source.presented_for(&gone).unwrap();
    assert!(Arc::ptr_eq(&mounted, &gone.presentation.held().unwrap()));
    assert!(Arc::ptr_eq(&mounted, &source.presented_for(&gone).unwrap()));
    assert_eq!(
        drawn(&catalogue::catalogue(&source, &gone).unwrap(), "GFX_icon").0,
        RED
    );
}

/// Work done ahead of requests is cancelled by its binding's retirement: a read-ahead computing a
/// binding's mount sees its cancellation the moment the binding is swapped out, and stops.
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
                    // What `visual::apply` watches between mounts.
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
    // Requests admitted with their own cancellation are not cancelled by the swap.
    assert!(
        !options
            .presentation
            .cancel()
            .load(std::sync::atomic::Ordering::Acquire)
    );
}

/// A rebind whose configuration changed between deriving and installing derives again from the
/// newest configuration and installs that; it never drops the change or installs the stale one.
/// A configuration of no content unbinds the source the same way, and racing a configure it is
/// the newest of the two that stays installed.
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
    // The configuration: the folders, if any, and the selection saved for the campaign.
    type Settings = Option<(installed_content_core::ResolveOptions, Presentation)>;
    let run = |config: &Config<Settings>, change: Option<Box<dyn Fn(&Config<Settings>) + '_>>| {
        let derived = std::sync::atomic::AtomicUsize::new(0);
        rebind(
            config,
            &std::sync::atomic::AtomicBool::new(false),
            |settings: &Settings| {
                // The first derivation pauses while another writer changes the configuration.
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
    let shows = |folders: &installed_content_core::ResolveOptions, presentation: &Presentation| {
        source.binding().is_some_and(|binding| {
            binding.options.user_dir == folders.user_dir && binding.presentation == *presentation
        })
    };
    // A map settings save races the rebind: the newer selection wins.
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
    // A folder change races it: the newer folders win, with their own selection.
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
    // Unconfiguring races a configure: the newer, unconfigured, wins and the source is unbound.
    let retried = run(
        &config,
        Some(Box::new(|config: &Config<Settings>| {
            config.update(|settings| *settings = None)
        })),
    );
    assert_eq!(retried, 2);
    assert!(source.binding().is_none());
    // And a configure racing the unconfigure: the newer, configured, wins.
    let retried = run(
        &config,
        Some(Box::new(|config: &Config<Settings>| {
            config.update(|settings| *settings = Some((options.options.clone(), art.clone())))
        })),
    );
    assert_eq!(retried, 2);
    assert!(shows(&options.options, &art));
    // A cancelled rebind stops.
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

/// A rebind paused across shutdown installs nothing: closing the configuration is its last
/// change, the paused rebind stops instead of deriving again, and so does every later one, so
/// the source shutdown unbound stays unbound.
#[test]
fn a_rebind_paused_across_shutdown_installs_nothing() {
    use crate::presentation::{Config, rebind};
    let (_machine, source, options) = machine("rebind_shutdown");
    let config = Config::new(Some(options.options.clone()));
    let never = std::sync::atomic::AtomicBool::new(false);
    let install = |_: &Option<installed_content_core::ResolveOptions>,
                   folders: Option<installed_content_core::ResolveOptions>| {
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
            // Shutdown runs while this derivation is paused: it closes the configuration and
            // unbinds the source.
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
    // A rebind starting after shutdown stops before deriving anything.
    let error = rebind(
        &config,
        &never,
        |_| -> std::io::Result<Option<installed_content_core::ResolveOptions>> {
            unreachable!("a closed configuration is not derived")
        },
        install,
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert!(source.binding().is_none());
}

/// A catalogue request waiting for the binding's mount another request computes stops when it
/// is cancelled: it waits on the binding's cell, never on a catalogue cache lock.
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
        // The waiter waits at its entry, where its mount is resolved, never at a catalogue's
        // cache; cancelled, it reads nothing.
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

/// Preparation is recorded-only work: under a selected visual mod it reads the recording's own
/// catalogue and artwork, and never mounts the binding's visual mods.
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
