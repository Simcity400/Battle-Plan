use content_reader_layers::content::visual::{
    apply,
    fixtures::{Machine, never, pick},
};
use content_reader_layers::{
    ResolvedLayers,
    content::fixtures::{archive_layer, folder_of, raw_zip},
};
use std::{collections::BTreeSet, fs};

#[test]
fn gui_and_gfx_files_are_enumerated_by_their_bytes() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_raw_gui_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let archive = root.join("mod.zip");
    fs::write(
        &archive,
        raw_zip(&[
            (
                b"interface/\xE9t\xE9.gui",
                0,
                b"guiTypes = { containerWindowType = { name = \"raw_window\" } }",
            ),
            (
                b"interface/sub/\x80.gfx",
                0,
                b"spriteTypes = { spriteType = { name = \"GFX_raw\" texturefile = \"gfx/x.dds\" } }",
            ),
            (b"interface/a.gfx", 0, b"spriteTypes = { }"),
            (b"common/synchronized_dynamic_tokens/\x80.txt", 0, b"raw_startup_token\n"),
            (b"common/synchronized_dynamic_tokens/nested/ignored.txt", 0, b"nested_token\n"),
            (b"common/synchronized_dynamic_tokens/ignored.TXT", 0, b"uppercase_extension_token\n"),
        ]),
    )
    .unwrap();
    let layers =
        ResolvedLayers::from_test_layers(vec![archive_layer("mod", &archive, &[])]).unwrap();
    let listed = layers.listed_layer_paths(b"interface/").unwrap();
    assert!(listed.contains(b"interface/\xE9t\xE9.gui".as_slice()));
    assert!(listed.contains(b"interface/sub/\x80.gfx".as_slice()));
    assert_eq!(
        layers.listed_paths("interface/"),
        BTreeSet::from(["interface/a.gfx".to_owned()])
    );
    assert_eq!(
        crate::gui::native::sprites::gfx_order(&layers).unwrap(),
        [
            b"interface/a.gfx".to_vec(),
            b"interface/sub/\x80.gfx".to_vec()
        ]
    );
    let guis = crate::gui::native::read_layers(&layers).unwrap();
    assert!(guis.template(b"raw_window").is_some());
    let dashboard = crate::gui::native::read_shared_layers(&layers, None).unwrap();
    let map = crate::gui::native::read_layers_at(&layers, [2048, 768]).unwrap();
    let game_window = crate::gui::native::read_layers_at(&layers, [2100, 960]).unwrap();
    assert!(std::sync::Arc::ptr_eq(&dashboard, &map));
    assert!(std::sync::Arc::ptr_eq(&dashboard, &game_window));
    assert_eq!(dashboard.files, guis.files);
    let builds: Vec<_> = layers.product_counts().into_iter()
        .filter(|entry| entry.parser == "inherited GUI layouts")
        .map(|entry| entry.builds).collect();
    assert_eq!(builds, [1]);
    let map_first = ResolvedLayers::from_test_layers(vec![archive_layer("mod", &archive, &[])]).unwrap();
    let map = crate::gui::native::read_layers_at(&map_first, [2048, 768]).unwrap();
    let dashboard = crate::gui::native::read_shared_layers(&map_first, None).unwrap();
    assert!(std::sync::Arc::ptr_eq(&map, &dashboard));
    let registry = crate::gui::native::resources::Registry::load(&layers).unwrap();
    assert!(registry.sprite(b"GFX_raw").is_some());
    let dynamic = crate::gui::native::keywords::DynamicTokens::read(&layers).unwrap();
    assert_eq!(
        dynamic,
        crate::gui::native::keywords::DynamicTokens::from_files(
            [b"raw_startup_token\n".as_slice()]
        )
    );
    fs::remove_dir_all(&root).unwrap();
}
#[test]
fn bootstrap_files_open_by_their_literal_paths() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_bootstrap_case_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let folder = root.join("folder");
    fs::create_dir_all(folder.join("interface")).unwrap();
    let window = b"guiTypes = { containerWindowType = { name = \"window\" } }";
    fs::write(folder.join("interface/Load_Screen.gui"), window).unwrap();
    fs::write(
        folder.join("interface/Load_Screen.gfx"),
        b"spriteTypes = { }",
    )
    .unwrap();
    let archive = root.join("mod.zip");
    fs::write(
        &archive,
        raw_zip(&[
            (b"interface/Load_Screen.gui", 0, window.as_slice()),
            (b"interface/Load_Screen.gfx", 0, b"spriteTypes = { }"),
        ]),
    )
    .unwrap();
    let archived =
        ResolvedLayers::from_test_layers(vec![archive_layer("mod", &archive, &[])]).unwrap();
    let foldered = ResolvedLayers::from_test_layers(vec![folder_of("mod", &folder, &[])]).unwrap();
    let labels = |layers: &ResolvedLayers| -> Vec<String> {
        crate::gui::native::read_layers(layers)
            .unwrap()
            .files
            .into_iter()
            .map(|(path, _)| path)
            .collect()
    };
    assert_eq!(labels(&archived), ["interface/Load_Screen.gui"]);
    assert_eq!(
        crate::gui::native::sprites::gfx_order(&archived).unwrap(),
        [b"interface/Load_Screen.gfx".to_vec()]
    );
    assert_eq!(
        labels(&foldered),
        ["interface/load_screen.gui", "interface/Load_Screen.gui"]
    );
    assert_eq!(
        crate::gui::native::sprites::gfx_order(&foldered).unwrap(),
        [
            b"interface/load_screen.gfx".to_vec(),
            b"interface/Load_Screen.gfx".to_vec()
        ]
    );
    fs::remove_dir_all(&root).unwrap();
}
#[test]
fn a_mods_interface_file_with_refusals_reads_the_recordings_file() {
    content_reader_script::test_game::enter();
    let _generation = content_reader_layers::content::Generation::enter();
    let machine = Machine::new("interface-fallback");
    machine.write(
        "base/interface/a.gfx",
        br#"spriteTypes = { spriteType = { name = "GFX_a" texturefile = "gfx/a.dds" } }"#,
    );
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write(
        "recorded/interface/z.gfx",
        br#"spriteTypes = { spriteType = { name = "GFX_a" texturefile = "gfx/recorded.dds" } }"#,
    );
    machine.write("base/gfx/map.gfx", b"objectTypes = { }");
    let zeros = ["0"; 12].join(" ");
    machine.write(
        "art/interface/z.gfx",
        format!(
            "spriteTypes = {{ spriteType = {{ name = \"GFX_a\" texturefile = \"gfx/b.dds\" \
             anchor = {{ {zeros} 5 }} }} }}"
        )
        .as_bytes(),
    );
    machine.write(
        "art/interface/b.gfx",
        br#"spriteTypes = { spriteType = { name = "GFX_b" texturefile = "gfx/b.dds" } }"#,
    );
    let mut layers = machine.recorded(&["base", "recorded"]);
    let reads = layers.report_uncovered_reads();
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let files = crate::gui::native::sprites::read_layers(layers.presentation()).unwrap();
    let file = |name: &str| {
        &files
            .iter()
            .find(|(path, _)| path == name)
            .unwrap_or_else(|| panic!("{name} is read"))
            .1
    };
    let a = file("interface/a.gfx").get(b"GFX_a").unwrap();
    assert!(a.refusals.is_empty(), "{:?}", a.refusals);
    assert!(file("interface/b.gfx").get(b"GFX_b").is_some());
    use crate::gui::native::{resources::Registry, sprites::read_object_layers};
    let presentation = layers.presentation();
    let window = Registry::load(presentation).unwrap();
    let map = Registry::from_files(read_object_layers(presentation).unwrap());
    let file_names = |registry: &Registry| registry.files.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
    assert_eq!(file_names(&window), ["interface/a.gfx", "interface/b.gfx", "interface/z.gfx"]);
    assert_eq!(file_names(&map), ["interface/a.gfx", "interface/b.gfx", "interface/z.gfx", "gfx/map.gfx"]);
    for registry in [&window, &map] {
        assert_eq!(registry.sprite_file(b"GFX_a"), Some("interface/z.gfx"));
        let sprite = registry.sprite(b"GFX_a").unwrap();
        assert_eq!(sprite.kind.plain().unwrap().texture, b"gfx/recorded.dds");
        assert!(sprite.refusals.is_empty());
    }
    let combat = Registry::with_sprites(std::sync::Arc::new(map), "generated".into(), Vec::new());
    assert!(std::ptr::eq(
        window.sprite(b"GFX_a").unwrap(),
        combat.sprite(b"GFX_a").unwrap()
    ));
    let refusals = reads.overlay_refusals().unwrap();
    assert!(
        refusals.contains("interface file interface/z.gfx ("),
        "{refusals}"
    );
}
