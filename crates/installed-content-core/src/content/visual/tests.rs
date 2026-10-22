use super::*;

/// A synthetic machine: a Steam library holding the game and its Workshop folder, and a user
/// folder. Removed when dropped.
struct Machine {
    root: PathBuf,
    options: ResolveOptions,
}

impl Machine {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_visual_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let game_dir = root.join("steamapps/common/Game");
        let user_dir = root.join("user");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(user_dir.join("mod")).unwrap();
        fs::create_dir_all(
            root.join("steamapps/workshop/content")
                .join(WORKSHOP_APP_ID),
        )
        .unwrap();
        Self {
            options: ResolveOptions {
                game_dir,
                user_dir,
                cache_dir: root.join("cache"),
            },
            root,
        }
    }

    fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn folder(&self, relative: &str) -> String {
        self.root
            .join(relative)
            .display()
            .to_string()
            .replace('\\', "/")
    }

    /// A user descriptor naming a folder below the machine root.
    fn local(&self, file: &str, body: &str, folder: &str) {
        fs::create_dir_all(self.root.join(folder)).unwrap();
        self.write(
            &format!("user/mod/{file}"),
            format!("{body}\npath=\"{}\"\n", self.folder(folder)).as_bytes(),
        );
    }

    fn item(&self, item: &str, descriptor: &str) -> String {
        let folder = format!("steamapps/workshop/content/{WORKSHOP_APP_ID}/{item}");
        self.write(&format!("{folder}/descriptor.mod"), descriptor.as_bytes());
        folder
    }

    fn recorded(&self, folders: &[&str]) -> ResolvedLayers {
        let paths: Vec<PathBuf> = folders
            .iter()
            .map(|folder| self.root.join(folder))
            .collect();
        for path in &paths {
            fs::create_dir_all(path).unwrap();
        }
        let folders: Vec<(&Path, &[&str])> =
            paths.iter().map(|path| (path.as_path(), &[][..])).collect();
        ResolvedLayers::from_folders(&folders).unwrap()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn pick(id: &str, name: &str) -> Choice {
    Choice {
        id: id.into(),
        name: name.into(),
    }
}

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

fn text(layers: &ResolvedLayers, path: &str) -> String {
    String::from_utf8(layers.read(path).unwrap()).unwrap()
}

/// A gameplay overhaul that also replaces terrain artwork shows its artwork; its map, scripts,
/// interface and shaders, which the eligibility gate refused, neither block it nor reach the
/// recorded stack that geometry and simulation read.
#[test]
fn a_mixed_overhaul_shows_its_artwork_over_the_recorded_geometry() {
    let machine = Machine::new("mixed");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.write("base/map/provinces.bmp", b"recorded provinces");
    machine.write("base/common/countries/a.txt", b"recorded country");
    machine.local(
        "overhaul.mod",
        "name=\"Overhaul\"\nreplace_path=\"common/countries\"",
        "overhaul",
    );
    machine.write("overhaul/map/terrain/atlas0.dds", b"mod atlas");
    machine.write("overhaul/map/provinces.bmp", b"mod provinces");
    machine.write("overhaul/common/countries/b.txt", b"mod country");
    machine.write("overhaul/interface/topbar.gui", b"mod gui");
    machine.write("overhaul/gfx/FX/pdxterrain.shader", b"mod shader");
    machine.write("overhaul/events/a.txt", b"mod events");
    machine.write("overhaul/gfx/models/supply/railroad.dds", b"mod rail");
    let mut layers = machine.recorded(&["base"]);
    let reads = layers.report_uncovered_reads();
    let plan = apply(
        &mut layers,
        &machine.options,
        &[pick("mod/overhaul.mod", "Overhaul")],
        &[],
        &never(),
    )
    .unwrap();
    assert_eq!(plan.order, [pick("mod/overhaul.mod", "Overhaul")]);
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas0.dds"),
        "mod atlas"
    );
    // The recorded stack is unchanged: geometry and recorded facts keep their recorded bytes.
    assert_eq!(text(&layers, "map/terrain/atlas0.dds"), "recorded atlas");
    assert_eq!(text(&layers, "map/provinces.bmp"), "recorded provinces");
    assert_eq!(
        layers.listed_paths("common/countries"),
        BTreeSet::from(["common/countries/a.txt".to_owned()])
    );
    assert_eq!(layers.layers.len(), 1);
    // A reader handed the presentation stack still reads recorded scripts and geometry: the
    // mod shows only under the presentation roots, its `replace_path` there too.
    let presentation = layers.presentation();
    assert_eq!(
        presentation.listed_paths("common/countries"),
        BTreeSet::from(["common/countries/a.txt".to_owned()])
    );
    assert_eq!(
        text(presentation, "common/countries/a.txt"),
        "recorded country"
    );
    assert_eq!(
        presentation
            .read("common/countries/b.txt")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        text(presentation, "map/provinces.bmp"),
        "recorded provinces"
    );
    assert_eq!(
        presentation
            .read_uncovered("events/a.txt")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        presentation.read_uncovered("interface/topbar.gui").unwrap(),
        b"mod gui"
    );
    // An uncovered file the mod supplies is its own too, never an uncovered recorded read.
    assert_eq!(
        layers
            .presentation()
            .read_uncovered("gfx/models/supply/railroad.dds")
            .unwrap(),
        b"mod rail"
    );
    // The mod's files are named and counted as its own, apart from recorded reads.
    let mut warnings = Vec::new();
    reads.append_to(&mut warnings);
    assert_eq!(
        warnings,
        [
            "Artwork from visual mods: 3 file(s) shown from mods that are not part of the recording, not its recorded artwork: Overhaul (3)."
        ]
    );
}

/// `replace_path` hides the recorded files below it from presentation lookups and listings
/// alike, including a lower file the mod has no replacement for.
#[test]
fn replace_path_hides_recorded_artwork_in_lookups_and_listings() {
    let machine = Machine::new("replace");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.write("base/map/terrain/strait.dds", b"recorded strait");
    machine.local(
        "art.mod",
        "name=\"Art\"\nreplace_path=\"map/terrain/\"",
        "art",
    );
    machine.write("art/map/terrain/atlas0.dds", b"mod atlas");
    let mut layers = machine.recorded(&["base"]);
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let presentation = layers.presentation();
    assert_eq!(
        presentation
            .read("map/terrain/strait.dds")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(!presentation.has_file("map/terrain/strait.dds").unwrap());
    assert_eq!(
        presentation.listed_paths("map/terrain"),
        BTreeSet::from(["map/terrain/atlas0.dds".to_owned()])
    );
    assert_eq!(
        presentation
            .descriptor_paths("map/terrain", &[".dds"])
            .unwrap(),
        ["map/terrain/atlas0.dds"]
    );
    assert_eq!(text(&layers, "map/terrain/strait.dds"), "recorded strait");
    assert_eq!(layers.listed_paths("map/terrain").len(), 2);
}

/// Each consumer sees the fallbacks it caused, nested observers each see them, a memo's replay
/// reaches its user, and a presentation mounted on a stack with its own diagnostics reports
/// apart from the stack it came from, in the line the map uses.
#[test]
fn consumers_observe_their_own_fallbacks_and_presentations_report_apart() {
    use super::super::{credit_overlay_refusals, observe_overlay_refusals, overlay_refusal_line};
    let machine = Machine::new("observe");
    machine.write("base/map/terrain/strait.dds", b"recorded strait");
    machine.local(
        "art.mod",
        "name=\"Art\"\nreplace_path=\"map/terrain/\"",
        "art",
    );
    let recorded = machine.recorded(&["base"]);
    let mut shown = recorded.with_own_overlay_diagnostics();
    apply(
        &mut shown,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let read = || {
        shown
            .artwork("strait", |stack| stack.read("map/terrain/strait.dds"))
            .unwrap()
    };
    let (inner, outer) = observe_overlay_refusals(|| observe_overlay_refusals(read));
    let (bytes, inner) = inner;
    assert_eq!(bytes, b"recorded strait");
    assert_eq!(inner.len(), 1, "{inner:?}");
    assert_eq!(
        outer, inner,
        "an enclosing observer sees what its inner one saw"
    );
    assert!(inner.iter().next().unwrap().starts_with("strait ("));
    let ((), replayed) = observe_overlay_refusals(|| credit_overlay_refusals(&inner));
    assert_eq!(replayed, inner);
    assert_eq!(
        read(),
        b"recorded strait",
        "nothing observes outside an observer"
    );
    assert_eq!(
        shown.read_warnings().overlay_refusals(),
        overlay_refusal_line(&inner)
    );
    assert_eq!(recorded.read_warnings().overlay_refusals(), None);
}

/// Artwork a visual mod cannot supply (here a lower file its `replace_path` hides, or a file the
/// loader refuses) is drawn from the recording and named; the recording still loads.
#[test]
fn artwork_a_mod_cannot_supply_falls_back_to_the_recording_by_name() {
    let machine = Machine::new("fallback");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.write("base/map/terrain/strait.dds", b"recorded strait");
    machine.local(
        "art.mod",
        "name=\"Art\"
replace_path=\"map/terrain\"",
        "art",
    );
    machine.write("art/map/terrain/atlas0.dds", b"unreadable");
    let mut layers = machine.recorded(&["base"]);
    let reads = layers.report_uncovered_reads();
    // Nothing is mounted: the recorded stack's own artwork, with no notice.
    let load = |stack: &ResolvedLayers, path: &str| -> io::Result<String> {
        let bytes = stack.read(path)?;
        if bytes == b"unreadable" {
            return Err(invalid("not a texture the map reads"));
        }
        Ok(String::from_utf8(bytes).unwrap())
    };
    assert_eq!(
        layers
            .artwork("strait", |stack| load(stack, "map/terrain/strait.dds"))
            .unwrap(),
        "recorded strait"
    );
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    for (path, name) in [
        ("map/terrain/strait.dds", "strait"),
        ("map/terrain/atlas0.dds", "atlas"),
    ] {
        assert_eq!(
            layers.artwork(name, |stack| load(stack, path)).unwrap(),
            format!("recorded {name}")
        );
    }
    let refusals = reads.overlay_refusals().unwrap();
    assert!(refusals.contains("2 file(s)"), "{refusals}");
    assert!(
        refusals.contains("atlas (not a texture the map reads)"),
        "{refusals}"
    );
    assert!(refusals.contains("strait ("), "{refusals}");
    // A failure of the recorded artwork itself is returned as it is without visual mods.
    assert_eq!(
        layers
            .artwork("missing", |stack| load(stack, "map/terrain/missing.dds"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
}

/// An item a loader reads apart from the rest falls back to the recording alone, named; one
/// neither stack gives is refused by its stable name for the map's fallback.
#[test]
fn an_artwork_item_a_mod_breaks_is_read_from_the_recording_by_name() {
    let layers = ResolvedLayers::default();
    let reads = layers.read_warnings();
    let mut asked = Vec::new();
    let item = layers.artwork_item("tree kind 3", true, |recorded| {
        asked.push(recorded);
        if recorded {
            Ok("recorded")
        } else {
            Err("not a mesh")
        }
    });
    assert_eq!(item, Ok("recorded"));
    assert_eq!(asked, [false, true]);
    let refusals = reads.overlay_refusals().unwrap();
    assert!(refusals.contains("tree kind 3 (not a mesh)"), "{refusals}");
    assert!(reads.refused().is_empty());
    // Without recorded layers below, or when they refuse it too: refused by name.
    let item = layers.artwork_item("tree kind 4", false, |_| Err::<&str, _>("not a mesh"));
    assert_eq!(item, Err("not a mesh"));
    let item = layers.artwork_item("tree kind 5", true, |_| Err::<&str, _>("not a mesh"));
    assert_eq!(item, Err("not a mesh"));
    assert_eq!(
        reads.refused(),
        BTreeSet::from(["tree kind 4".to_owned(), "tree kind 5".to_owned()])
    );
}

/// The world normal map is artwork: a mod's shows over the recording's in each form the game
/// reads, and updating it moves the selection's revision; the map's other bitmaps stay recorded.
#[test]
fn a_mods_world_normal_map_shows_and_moves_the_revision() {
    let machine = Machine::new("world-normal");
    machine.write("base/map/world_normal.bmp", b"recorded normal");
    machine.write("base/map/heightmap.bmp", b"recorded heights");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/world_normal.dds", b"mod normal");
    machine.write("art/map/heightmap.bmp", b"mod heights");
    let selected = [pick("mod/art.mod", "Art")];
    let mut layers = machine.recorded(&["base"]);
    apply(&mut layers, &machine.options, &selected, &[], &never()).unwrap();
    let presentation = layers.presentation();
    assert_eq!(
        presentation.read_uncovered("map/world_normal.dds").unwrap(),
        b"mod normal"
    );
    assert_eq!(text(presentation, "map/heightmap.bmp"), "recorded heights");
    let revision = || revision(&machine.options, &selected, &[], &never()).unwrap();
    let first = revision();
    machine.write("art/map/heightmap.bmp", b"mod heights, changed");
    assert_eq!(first, revision());
    for form in ["bmp", "dds", "tga"] {
        let before = revision();
        machine.write(
            &format!("art/map/world_normal.{form}"),
            format!("normal {form}, updated").as_bytes(),
        );
        assert_ne!(before, revision(), "{form}");
    }
}

/// A visual mod's descriptor file that does not parse is read as the recording's file of that
/// path, named; a name only the mod's file holds stays undecided.
#[test]
fn a_mods_descriptor_that_does_not_parse_reads_the_recordings_file() {
    let machine = Machine::new("descriptor-fallback");
    machine.write(
        "base/gfx/entities/a.asset",
        br#"entity = { name = "plane" pdxmesh = "plane_mesh" }"#,
    );
    machine.local("art.mod", "name=\"Art\"", "art");
    // Does not parse even when its blocks are closed.
    machine.write(
        "art/gfx/entities/a.asset",
        br#"entity = { name = "plane" zeppelin } }{ = ="#,
    );
    let mut layers = machine.recorded(&["base"]);
    let reads = layers.report_uncovered_reads();
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let roster = layers.presentation().descriptor_roster().unwrap();
    assert!(roster.descriptors.entities.contains_key("plane"));
    assert!(roster.unreadable.is_empty(), "{:?}", roster.unreadable);
    assert!(roster.unreadable_names.contains_key("zeppelin"));
    assert!(!roster.unreadable_names.contains_key("plane"));
    let refusals = reads.overlay_refusals().unwrap();
    assert!(
        refusals.contains("descriptor gfx/entities/a.asset ("),
        "{refusals}"
    );
}

/// A visual mod's interface file whose declarations are refused is read as the recording's file
/// of that path, named; the mod's other files are its own.
#[test]
fn a_mods_interface_file_with_refusals_reads_the_recordings_file() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let machine = Machine::new("interface-fallback");
    machine.write(
        "base/interface/a.gfx",
        br#"spriteTypes = { spriteType = { name = "GFX_a" texturefile = "gfx/a.dds" } }"#,
    );
    machine.local("art.mod", "name=\"Art\"", "art");
    // Past the texture id the anchor reaches the animation records: refused.
    let zeros = ["0"; 12].join(" ");
    machine.write(
        "art/interface/a.gfx",
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
    let mut layers = machine.recorded(&["base"]);
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
    let refusals = reads.overlay_refusals().unwrap();
    assert!(
        refusals.contains("interface file interface/a.gfx ("),
        "{refusals}"
    );
}

/// Workshop items are found in the game's own Steam library without a user descriptor; a user
/// descriptor of an item and an alias of one source are that one mod; a descriptor that cannot
/// be mounted is still listed, by name, with its problem.
#[test]
fn discovery_lists_local_and_workshop_mods_once_each() {
    let machine = Machine::new("discovery");
    let base = machine.item("100", "name=\"Base Art\"\nremote_file_id=\"100\"");
    machine.item(
        "200",
        "name=\"Dependent Art\"\ndependencies={\n\"Base Art\"\n}\nremote_file_id=\"200\"",
    );
    // A stale `path` in a Workshop descriptor is not where the launcher mounts the item from.
    machine.item("300", "name=\"Three\"\npath=\"mod/elsewhere\"");
    machine.write(
        "user/mod/ugc_100.mod",
        format!(
            "name=\"Base Art\"\nremote_file_id=\"100\"\npath=\"{}\"",
            machine.folder(&base)
        )
        .as_bytes(),
    );
    machine.write(
        "user/mod/zz_alias.mod",
        format!("name=\"Alias\"\npath=\"{}\"", machine.folder(&base)).as_bytes(),
    );
    machine.write("user/mod/broken.mod", b"name=\"Broken\"\npath=\"missing\"");
    machine.local("local.mod", "name=\"Local\"", "local");
    let installed = installed(&machine.options, &never()).unwrap();
    let ids: Vec<_> = installed.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "workshop/100",
            "mod/broken.mod",
            "workshop/200",
            "mod/local.mod",
            "workshop/300"
        ]
    );
    let broken = &installed[1];
    assert_eq!(broken.name, "Broken");
    assert!(broken.problem.as_deref().unwrap().contains("missing"));
    assert!(
        installed
            .iter()
            .filter(|item| item.id != broken.id)
            .all(|item| item.problem.is_none())
    );
    assert_eq!(installed[0].recorded_id, "100");
    assert_eq!(installed[4].recorded_id, "ugc_300.mod");
    assert!(
        installed[4]
            .source
            .replace('\\', "/")
            .ends_with("/394360/300")
    );
}

/// Overlays mount in the engine's order whatever order they were chosen in: dependencies
/// first, then descriptor path bytes. A selection that is not installed or cannot be mounted
/// is named, and the rest still mount.
#[test]
fn selections_mount_in_the_engines_order_and_name_what_cannot_mount() {
    let machine = Machine::new("order");
    for (item, name, extra) in [
        ("100", "Zulu", ""),
        ("200", "Alpha", "dependencies={\n\"Zulu\"\n}"),
        ("300", "Mike", ""),
    ] {
        let folder = machine.item(item, &format!("name=\"{name}\"\n{extra}"));
        machine.write(&format!("{folder}/map/terrain/atlas0.dds"), name.as_bytes());
    }
    machine.write("user/mod/broken.mod", b"name=\"Broken\"\npath=\"missing\"");
    let selected = [
        pick("workshop/300", "Mike"),
        pick("workshop/200", "Alpha"),
        pick("workshop/999", "Gone"),
        pick("mod/broken.mod", "Broken"),
        pick("workshop/100", "Zulu"),
    ];
    let planned = plan(&machine.options, &selected, &[], &never()).unwrap();
    // Alpha depends on Zulu, so Zulu mounts below it; Mike sorts after both by its path.
    assert_eq!(
        planned.order,
        [
            pick("workshop/100", "Zulu"),
            pick("workshop/200", "Alpha"),
            pick("workshop/300", "Mike")
        ]
    );
    assert_eq!(planned.warnings.len(), 2, "{:?}", planned.warnings);
    assert!(planned.warnings[0].contains("Gone is not installed"));
    assert!(planned.warnings[1].starts_with("Visual mod Broken is not shown"));
    let mut layers = machine.recorded(&["base"]);
    let applied = apply(&mut layers, &machine.options, &selected, &[], &never()).unwrap();
    assert_eq!(
        (&applied.order, &applied.warnings),
        (&planned.order, &planned.warnings)
    );
    assert!(planned.revision.is_empty() && !applied.revision.is_empty());
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas0.dds"),
        "Mike"
    );
    let ids: Vec<_> = layers
        .presentation()
        .layers
        .iter()
        .map(|layer| layer.id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "test0",
            "visual:workshop/100",
            "visual:workshop/200",
            "visual:workshop/300"
        ]
    );
    // A dependency cycle the engine cannot finish shows none of the cycle's selection.
    machine.item("400", "name=\"Ping\"\ndependencies={\n\"Pong\"\n}");
    machine.item("500", "name=\"Pong\"\ndependencies={\n\"Ping\"\n}");
    let cycle = apply(
        &mut layers,
        &machine.options,
        &[pick("workshop/400", "Ping"), pick("workshop/500", "Pong")],
        &[],
        &never(),
    )
    .unwrap();
    assert!(cycle.order.is_empty());
    assert!(
        cycle.warnings[0].contains("cannot order"),
        "{:?}",
        cycle.warnings
    );
    assert!(std::ptr::eq(layers.presentation(), &layers));
}

/// An empty selection mounts nothing: presentation reads are the recorded reads.
#[test]
fn an_empty_selection_shows_the_recorded_artwork() {
    let machine = Machine::new("empty");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/terrain/atlas0.dds", b"mod atlas");
    let mut layers = machine.recorded(&["base"]);
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas0.dds"),
        "mod atlas"
    );
    let plan = apply(&mut layers, &machine.options, &[], &[], &never()).unwrap();
    assert_eq!(plan, Plan::default());
    assert!(std::ptr::eq(layers.presentation(), &layers));
}

/// A recorded mod selected again mounts above the visual mods it sorts below, and its covered
/// bytes still verify against the recording.
#[test]
fn a_recorded_mod_selected_again_keeps_its_verification() {
    let machine = Machine::new("recorded");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.write("played/map/terrain/atlas0.dds", b"played atlas");
    machine.local("played.mod", "name=\"Played\"", "played");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/terrain/atlas0.dds", b"mod atlas");
    let mut layers = machine.recorded(&["base", "played"]);
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/played.mod", "Played"), pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    // `mod/played.mod` sorts after `mod/art.mod`, so the recorded mod is on top.
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas0.dds"),
        "played atlas"
    );
    machine.write("played/map/terrain/atlas0.dds", b"edited atlas");
    let error = layers
        .presentation()
        .read("map/terrain/atlas0.dds")
        .unwrap_err();
    assert!(
        error.to_string().contains("changed since verification"),
        "{error}"
    );
}

/// Visual mods mount below the built-in layers the engine mounts above every mod, as an
/// enabled mod would, and the recorded layers keep their order.
#[test]
fn visual_mods_mount_below_the_layers_the_engine_mounts_above_mods() {
    let machine = Machine::new("above");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.write("tweaker/map/terrain/atlas0.dds", b"tweaker atlas");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/terrain/atlas0.dds", b"mod atlas");
    machine.write("art/map/terrain/strait.dds", b"mod strait");
    let mut layers = machine.recorded(&["base", "tweaker"]);
    layers.above_mods = 1;
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let ids: Vec<_> = layers
        .presentation()
        .layers
        .iter()
        .map(|layer| layer.id.as_str())
        .collect();
    assert_eq!(ids, ["test0", "visual:mod/art.mod", "test1"]);
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas0.dds"),
        "tweaker atlas"
    );
    assert_eq!(
        text(layers.presentation(), "map/terrain/strait.dds"),
        "mod strait"
    );
}

/// An archive mod mounts through the one archive reader, like a recorded archive layer.
#[test]
fn an_archive_mod_mounts_its_records() {
    use super::super::tests::{Record, zip_records};
    let machine = Machine::new("archive");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    let archive = machine.write(
        "art.zip",
        &zip_records(
            &[],
            &[Record {
                name: b"map/terrain/atlas0.dds",
                host: 0,
                flags: 0,
                deflate: true,
                data: b"archived atlas",
            }],
        ),
    );
    machine.write(
        "user/mod/art.mod",
        format!(
            "name=\"Art\"\narchive=\"{}\"",
            archive.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    let mut layers = machine.recorded(&["base"]);
    let plan = apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas0.dds"),
        "archived atlas"
    );
}

/// No count of installed or selected mods is refused.
#[test]
fn catalogues_and_selections_are_not_capped() {
    let machine = Machine::new("uncapped");
    let mut selected = Vec::new();
    for index in 0..320 {
        let name = format!("Mod {index:03}");
        machine.local(
            &format!("m{index:03}.mod"),
            &format!("name=\"{name}\""),
            &format!("m{index:03}"),
        );
        selected.push(pick(&format!("mod/m{index:03}.mod"), &name));
    }
    assert_eq!(installed(&machine.options, &never()).unwrap().len(), 320);
    let planned = plan(&machine.options, &selected, &[], &never()).unwrap();
    assert_eq!(planned.order.len(), 320);
    assert!(planned.warnings.is_empty());
}

/// Opt-in and read-only: each Workshop item of the installed game's Steam library that ships
/// terrain artwork mounts alone over the installed game, the presentation stack decodes the
/// item's own texture as the map loads it, and the recorded stack keeps the game's.
#[test]
#[ignore = "reads the installed game and its Steam library's Workshop items; requires BATTLE_PLAN_GAME_DIR"]
fn installed_workshop_artwork_mounts_over_the_installed_game() {
    let game_dir = PathBuf::from(
        std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR must name the installed game"),
    );
    assert!(
        workshop_root(&game_dir).is_some(),
        "BATTLE_PLAN_GAME_DIR is not in a Steam library with a Workshop folder"
    );
    let machine = Machine::new("installed");
    machine.write(
        "user/dlc_load.json",
        br#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
    );
    machine.write("user/settings.txt", b"language=\"l_english\"");
    let options = ResolveOptions {
        game_dir: game_dir.clone(),
        ..machine.options.clone()
    };
    let gathered = gather(&GatherOptions {
        game_dir,
        user_dir: options.user_dir.clone(),
        cache_dir: options.cache_dir.clone(),
        dlc_order: None,
    })
    .unwrap();
    let artwork = [
        "map/terrain/colormap_rgb_cityemissivemask_a.dds",
        "map/terrain/atlas0.dds",
        "map/terrain/colormap_water_0.dds",
        "map/terrain/border_country_0.dds",
    ];
    let mut shown = 0;
    for item in installed(&options, &never()).unwrap() {
        let folder = PathBuf::from(&item.source);
        let Some(path) = artwork.iter().find(|path| folder.join(path).is_file()) else {
            continue;
        };
        let mut layers = gathered.layers.clone();
        let reads = layers.report_uncovered_reads();
        let plan = apply(
            &mut layers,
            &options,
            &[pick(&item.id, &item.name)],
            &[],
            &never(),
        )
        .unwrap();
        assert_eq!(plan.order.len(), 1, "{}: {:?}", item.name, plan.warnings);
        let presentation = layers.presentation();
        assert_eq!(
            presentation.read(path).unwrap(),
            fs::read(folder.join(path)).unwrap(),
            "{}",
            item.name
        );
        let texture = crate::gui::native::resources::texture(
            path.as_bytes(),
            &|request| presentation.read_path(request),
            Default::default(),
        )
        .unwrap()
        .unwrap_or_else(|reason| panic!("{}: {path}: {reason:?}", item.name));
        assert_eq!(
            layers.read(path).unwrap(),
            gathered.layers.read(path).unwrap()
        );
        assert!(
            reads
                .overlay_warning()
                .is_some_and(|line| line.contains(&item.name)),
            "{}",
            item.name
        );
        eprintln!(
            "{} ({}): {path} {}x{} shown over the installed game",
            item.name, item.id, texture.image.width, texture.image.height
        );
        shown += 1;
    }
    assert!(shown > 0, "no Workshop item ships terrain artwork");
}

/// A folder discovery cannot list is one named problem; the Workshop items still list.
#[test]
fn an_unreadable_mod_folder_is_named_and_the_rest_still_list() {
    let machine = Machine::new("unreadable");
    fs::remove_dir_all(machine.options.user_dir.join("mod")).unwrap();
    machine.write("user/mod", b"a file where the mod folder belongs");
    machine.item("100", "name=\"Art\"");
    let installed = installed(&machine.options, &never()).unwrap();
    assert_eq!(installed.len(), 2, "{installed:?}");
    let art = installed
        .iter()
        .find(|item| item.id == "workshop/100")
        .unwrap();
    assert!(art.problem.is_none());
    let unreadable = installed
        .iter()
        .find(|item| item.id.starts_with("unreadable/"))
        .unwrap();
    assert!(
        unreadable
            .problem
            .as_deref()
            .unwrap()
            .starts_with("this folder cannot be listed")
    );
}

/// A mod mounts as the game mounts it: an archive record whose name differs in case from a
/// recorded folder file stays its own file (the archive's lookup is exact), and an archive's
/// case variants are both kept. Neither drops the mod or the rest of the selection.
#[test]
fn case_variants_mount_as_the_game_mounts_them() {
    use super::super::tests::{Record, zip_records};
    let machine = Machine::new("case");
    machine.write("base/map/terrain/colormap_water_0.dds", b"recorded water");
    let record = |name, data| Record {
        name,
        host: 0,
        flags: 0,
        deflate: false,
        data,
    };
    let archive = machine.write(
        "case.zip",
        &zip_records(
            &[],
            &[
                record(b"map/terrain/Colormap_water_0.dds", b"mod water"),
                record(b"map/terrain/atlas0.dds", b"lower atlas"),
                record(b"map/terrain/ATLAS0.dds", b"upper atlas"),
            ],
        ),
    );
    machine.write(
        "user/mod/case.mod",
        format!(
            "name=\"Case\"\narchive=\"{}\"",
            archive.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    machine.local("other.mod", "name=\"Other\"", "other");
    machine.write("other/map/terrain/strait.dds", b"other strait");
    let mut layers = machine.recorded(&["base"]);
    let plan = apply(
        &mut layers,
        &machine.options,
        &[pick("mod/case.mod", "Case"), pick("mod/other.mod", "Other")],
        &[],
        &never(),
    )
    .unwrap();
    assert_eq!(plan.order.len(), 2, "{:?}", plan.warnings);
    let presentation = layers.presentation();
    assert_eq!(
        text(presentation, "map/terrain/colormap_water_0.dds"),
        "recorded water"
    );
    assert_eq!(
        text(presentation, "map/terrain/Colormap_water_0.dds"),
        "mod water"
    );
    assert_eq!(text(presentation, "map/terrain/atlas0.dds"), "lower atlas");
    assert_eq!(text(presentation, "map/terrain/ATLAS0.dds"), "upper atlas");
    assert_eq!(text(presentation, "map/terrain/strait.dds"), "other strait");
    // Byte-exact enumeration separates case variants; each opens through ordinary lookup.
    let listed = presentation
        .descriptor_paths("map/terrain", &[".dds"])
        .unwrap();
    assert!(
        [
            "map/terrain/atlas0.dds",
            "map/terrain/ATLAS0.dds",
            "map/terrain/colormap_water_0.dds",
            "map/terrain/Colormap_water_0.dds"
        ]
        .iter()
        .all(|path| listed.iter().any(|listed| listed == path)),
        "{listed:?}"
    );
    let read = |name: &str| {
        let path = listed.iter().find(|listed| **listed == name).unwrap();
        path.read_uncovered(presentation).unwrap()
    };
    assert_eq!(read("map/terrain/colormap_water_0.dds"), b"recorded water");
    assert_eq!(read("map/terrain/Colormap_water_0.dds"), b"mod water");
}

/// The game mounts one mod of a display name: a selected mod named as a recorded mod it is not
/// is not shown, and of two selected mods sharing a name only the first in mount order is.
#[test]
fn one_mod_of_a_display_name_is_shown() {
    let machine = Machine::new("names");
    for (file, name) in [("a.mod", "Twin"), ("b.mod", "Twin"), ("c.mod", "Played")] {
        let folder = file.trim_end_matches(".mod");
        machine.local(file, &format!("name=\"{name}\""), folder);
        machine.write(
            &format!("{folder}/map/terrain/atlas0.dds"),
            folder.as_bytes(),
        );
    }
    let recorded = [RecordedMod {
        id: "999".into(),
        name: "Played".into(),
    }];
    let planned = plan(
        &machine.options,
        &[
            pick("mod/b.mod", "Twin"),
            pick("mod/a.mod", "Twin"),
            pick("mod/c.mod", "Played"),
        ],
        &recorded,
        &never(),
    )
    .unwrap();
    assert_eq!(planned.order, [pick("mod/a.mod", "Twin")]);
    assert_eq!(planned.warnings.len(), 2, "{:?}", planned.warnings);
    assert!(planned.warnings[0].contains("recording's mod of that name"));
    assert!(planned.warnings[1].contains("(mod/b.mod)"));
}

/// Graphics consumers read defines merged natively with the mods' scripts; the recorded and the
/// presentation stacks themselves keep the recorded defines for every other consumer.
#[test]
fn presentation_defines_merge_the_mods_scripts_natively() {
    let machine = Machine::new("defines");
    machine.write(
        "base/common/defines/00_defines.lua",
        b"NDefines={NGraphics={TREES=1},NMilitary={WIDTH=2}}",
    );
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write(
        "art/common/defines/00_defines.lua",
        b"NDefines={NGraphics={TREES=5},NMilitary={WIDTH=9}}",
    );
    machine.write(
        "art/common/defines/01_art.lua",
        b"NDefines.NGraphics.TREES = NDefines.NGraphics.TREES * 2",
    );
    let mut layers = machine.recorded(&["base"]);
    assert!(layers.presentation_defines().is_none());
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let recorded = crate::defines::read(&layers).unwrap();
    let shown = crate::defines::read(layers.presentation()).unwrap();
    let art = crate::defines::read(layers.presentation_defines().unwrap()).unwrap();
    assert_eq!(recorded.number("NGraphics", "TREES").unwrap(), 1.0);
    assert_eq!(shown.number("NGraphics", "TREES").unwrap(), 1.0);
    assert_eq!(art.number("NGraphics", "TREES").unwrap(), 10.0);
    assert_eq!(art.number("NMilitary", "WIDTH").unwrap(), 9.0);
    assert_eq!(recorded.number("NMilitary", "WIDTH").unwrap(), 2.0);
}

/// The recorded stack below a presentation stack's mods is the recorded roster again.
#[test]
fn the_stack_without_overlays_is_the_recorded_roster() {
    let machine = Machine::new("without");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/terrain/atlas0.dds", b"mod atlas");
    let mut layers = machine.recorded(&["base"]);
    assert!(layers.without_overlays().is_none());
    apply(
        &mut layers,
        &machine.options,
        &[pick("mod/art.mod", "Art")],
        &[],
        &never(),
    )
    .unwrap();
    let recorded = layers.presentation().without_overlays().unwrap();
    assert_eq!(text(&recorded, "map/terrain/atlas0.dds"), "recorded atlas");
    assert_eq!(recorded.layers.len(), 1);
}

/// A selection's revision changes when a mod it shows is updated or a missing one is installed,
/// and not for a file the mod does not show.
#[test]
fn revisions_follow_what_the_mods_show() {
    let machine = Machine::new("revision");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/terrain/atlas0.dds", b"one");
    let selected = [pick("mod/art.mod", "Art"), pick("mod/later.mod", "Later")];
    let revision = || revision(&machine.options, &selected, &[], &never()).unwrap();
    let first = revision();
    assert_eq!(first, revision());
    assert_eq!(
        super::revision(&machine.options, &[], &[], &never()).unwrap(),
        ""
    );
    machine.write("art/common/countries/a.txt", b"not shown");
    assert_eq!(first, revision());
    machine.write("art/map/terrain/atlas0.dds", b"two, longer");
    let updated = revision();
    assert_ne!(first, updated);
    machine.local("later.mod", "name=\"Later\"", "later");
    assert_ne!(updated, revision());
    // Mounting records the revision it mounted.
    let mut layers = machine.recorded(&["base"]);
    let plan = apply(&mut layers, &machine.options, &selected, &[], &never()).unwrap();
    assert_eq!(plan.revision, revision());
}

/// A mod that changes while it mounts is mounted again, so the plan's revision and the mounted
/// files are the updated ones, never a revision taken before the update over older files.
#[test]
fn a_mod_updated_while_it_mounts_is_mounted_again() {
    let machine = Machine::new("revision-race");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/map/terrain/atlas0.dds", b"one");
    let selected = [pick("mod/art.mod", "Art")];
    let mut layers = machine.recorded(&["base"]);
    let mut taken = 0;
    let plan = apply_with(
        &mut layers,
        &machine.options,
        &selected,
        &[],
        &never(),
        || {
            taken += 1;
            // The update lands during the first mount, before its second revision is taken.
            if taken == 2 {
                machine.write("art/map/terrain/atlas1.dds", b"added by the update");
            }
            revision(&machine.options, &selected, &[], &never())
        },
    )
    .unwrap();
    assert_eq!(taken, 4, "the first mount is followed by a second");
    assert_eq!(
        plan.revision,
        revision(&machine.options, &selected, &[], &never()).unwrap()
    );
    assert_eq!(
        text(layers.presentation(), "map/terrain/atlas1.dds"),
        "added by the update"
    );
}

/// A selection none of whose mods can be shown records the same revision a later check takes,
/// so its map is not reloaded again and again.
#[test]
fn a_selection_that_shows_nothing_records_its_revision() {
    let machine = Machine::new("revision-missing");
    let selected = [pick("mod/gone.mod", "Gone")];
    let mut layers = machine.recorded(&["base"]);
    let plan = apply(&mut layers, &machine.options, &selected, &[], &never()).unwrap();
    assert!(layers.presentation_defines().is_none());
    assert!(plan.order.is_empty() && !plan.revision.is_empty());
    assert_eq!(
        plan.revision,
        revision(&machine.options, &selected, &[], &never()).unwrap()
    );
}
