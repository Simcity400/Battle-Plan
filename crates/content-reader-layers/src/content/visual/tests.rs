use super::fixtures::{Machine, pick, never};
use super::*;

fn text(layers: &ResolvedLayers, path: &str) -> String {
    std::str::from_utf8(&layers.read(path).unwrap()).unwrap().to_owned()
}

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
    assert_eq!(text(&layers, "map/terrain/atlas0.dds"), "recorded atlas");
    assert_eq!(text(&layers, "map/provinces.bmp"), "recorded provinces");
    assert_eq!(
        layers.listed_paths("common/countries"),
        BTreeSet::from(["common/countries/a.txt".to_owned()])
    );
    assert_eq!(layers.layers.len(), 1);
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
    assert_eq!(
        layers
            .presentation()
            .read_uncovered("gfx/models/supply/railroad.dds")
            .unwrap(),
        b"mod rail"
    );
    let mut warnings = Vec::new();
    reads.append_to(&mut warnings);
    assert_eq!(
        warnings,
        [
            "Artwork from visual mods: 3 file(s) shown from mods that are not part of the recording, not its recorded artwork: Overhaul (3)."
        ]
    );
}

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
    let load = |stack: &ResolvedLayers, path: &str| -> io::Result<String> {
        let bytes = stack.read(path)?;
        if bytes == b"unreadable" {
            return Err(invalid("not a texture the map reads"));
        }
        Ok(std::str::from_utf8(&bytes).unwrap().to_owned())
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
    assert_eq!(
        layers
            .artwork("missing", |stack| load(stack, "map/terrain/missing.dds"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
}

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
    let item = layers.artwork_item("tree kind 4", false, |_| Err::<&str, _>("not a mesh"));
    assert_eq!(item, Err("not a mesh"));
    let item = layers.artwork_item("tree kind 5", true, |_| Err::<&str, _>("not a mesh"));
    assert_eq!(item, Err("not a mesh"));
    assert_eq!(
        reads.refused(),
        BTreeSet::from(["tree kind 4".to_owned(), "tree kind 5".to_owned()])
    );
}

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



#[test]
fn discovery_lists_local_and_workshop_mods_once_each() {
    let machine = Machine::new("discovery");
    let base = machine.item("100", "name=\"Base Art\"\nremote_file_id=\"100\"");
    machine.item(
        "200",
        "name=\"Dependent Art\"\ndependencies={\n\"Base Art\"\n}\nremote_file_id=\"200\"",
    );
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
    assert!(std::ptr::eq(layers.presentation(), &raw const layers));
}

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
    assert!(std::ptr::eq(layers.presentation(), &raw const layers));
}

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

#[cfg(windows)]
#[test]
fn a_folder_mods_junctions_are_entered_and_read_through() {
    use super::super::tests::junction;
    let machine = Machine::new("junction");
    machine.write("base/gfx/interface/own.dds", b"recorded own");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/gfx/interface/own.dds", b"mod own");
    machine.write("elsewhere/inner.dds", b"linked inner");
    machine.write("elsewhere/deep/more.dds", b"linked more");
    junction(
        &machine.root.join("art/gfx/interface/linked"),
        &machine.root.join("elsewhere"),
    );
    fs::create_dir_all(machine.root.join("gone")).unwrap();
    junction(
        &machine.root.join("art/gfx/interface/gone"),
        &machine.root.join("gone"),
    );
    fs::remove_dir(machine.root.join("gone")).unwrap();
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
    let presentation = layers.presentation();
    assert_eq!(
        presentation
            .listed_paths("gfx/interface")
            .into_iter()
            .collect::<Vec<_>>(),
        [
            "gfx/interface/linked/deep/more.dds",
            "gfx/interface/linked/inner.dds",
            "gfx/interface/own.dds"
        ]
    );
    assert_eq!(
        text(presentation, "gfx/interface/linked/inner.dds"),
        "linked inner"
    );
    assert!(presentation.supplied_by_visual_mod("gfx/interface/linked/inner.dds"));
    assert_eq!(text(presentation, "gfx/interface/own.dds"), "mod own");
    let listed: Vec<_> = presentation
        .descriptor_paths("gfx/interface/linked", &[".dds"])
        .unwrap()
        .iter()
        .map(|path| path.to_string())
        .collect();
    assert_eq!(
        listed,
        [
            "gfx/interface/linked/deep/more.dds",
            "gfx/interface/linked/inner.dds"
        ]
    );
}

#[cfg(windows)]
#[test]
fn a_folder_mod_whose_junction_loops_is_refused_by_name() {
    use super::super::tests::junction;
    let machine = Machine::new("junction-loop");
    machine.write("base/gfx/interface/own.dds", b"recorded own");
    machine.local("art.mod", "name=\"Art\"", "art");
    machine.write("art/gfx/interface/own.dds", b"mod own");
    junction(
        &machine.root.join("art/gfx/interface/up"),
        &machine.root.join("art/gfx"),
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
    assert!(plan.order.is_empty(), "{:?}", plan.order);
    assert_eq!(plan.warnings.len(), 1, "{:?}", plan.warnings);
    assert!(
        plan.warnings[0].starts_with("Visual mod Art is not shown")
            && plan.warnings[0].contains("leads back to"),
        "{:?}",
        plan.warnings
    );
    assert_eq!(
        text(layers.presentation(), "gfx/interface/own.dds"),
        "recorded own"
    );
}

#[test]
fn an_archive_link_record_is_listed_and_its_read_refused_by_name() {
    use super::super::tests::{Record, zip_records};
    let machine = Machine::new("archive-link");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    let record = |name: &'static [u8], host: u8, data: &'static [u8]| Record {
        name,
        host,
        flags: 0,
        deflate: false,
        data,
    };
    let mut zip = zip_records(
        &[],
        &[
            record(b"map/terrain/target.dds", 0, b"linked atlas"),
            record(b"map/terrain/atlas0.dds", 3, b"map/terrain/target.dds"),
        ],
    );
    let central = (0..zip.len() - 46)
        .find(|&at| {
            zip[at..at + 4] == 0x0201_4B50_u32.to_le_bytes()
                && zip[at + 46..].starts_with(b"map/terrain/atlas0.dds")
        })
        .unwrap();
    zip[central + 38..central + 42].copy_from_slice(&0xA1FF_0000_u32.to_le_bytes());
    let archive = machine.write("link.zip", &zip);
    machine.write(
        "user/mod/link.mod",
        format!(
            "name=\"Link\"\narchive=\"{}\"",
            archive.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    let mut layers = machine.recorded(&["base"]);
    let plan = apply(
        &mut layers,
        &machine.options,
        &[pick("mod/link.mod", "Link")],
        &[],
        &never(),
    )
    .unwrap();
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    let presentation = layers.presentation();
    let error = presentation
        .read("map/terrain/atlas0.dds")
        .unwrap_err()
        .to_string();
    assert!(error.contains("is a symbolic link record"), "{error}");
    assert_eq!(text(presentation, "map/terrain/target.dds"), "linked atlas");
    let listed = presentation
        .descriptor_paths("map/terrain", &[".dds"])
        .unwrap();
    assert!(
        listed
            .iter()
            .any(|listed| *listed == "map/terrain/atlas0.dds"),
        "{listed:?}"
    );
}

#[test]
fn a_workshop_archive_keeps_its_id_beside_a_launcher_descriptor() {
    use super::super::tests::{Record, zip_records};
    let machine = Machine::new("workshop-archive");
    machine.write("base/map/terrain/atlas0.dds", b"recorded atlas");
    let folder = machine.item(
        "100",
        "name=\"Packed\"\nremote_file_id=\"100\"\narchive=\"packed.zip\"",
    );
    let archive = machine.write(
        &format!("{folder}/packed.zip"),
        &zip_records(
            &[],
            &[Record {
                name: b"map/terrain/atlas0.dds",
                host: 0,
                flags: 0,
                deflate: false,
                data: b"packed atlas",
            }],
        ),
    );
    let chosen = [pick("workshop/100", "Packed")];
    let shown = || {
        let mut layers = machine.recorded(&["base"]);
        let plan = apply(&mut layers, &machine.options, &chosen, &[], &never()).unwrap();
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        text(layers.presentation(), "map/terrain/atlas0.dds")
    };
    assert_eq!(shown(), "packed atlas");
    machine.write(
        "user/mod/ugc_100.mod",
        format!(
            "name=\"Packed\"\nremote_file_id=\"100\"\narchive=\"{}\"",
            archive.display().to_string().replace('\\', "/")
        )
        .as_bytes(),
    );
    let ids: Vec<_> = installed(&machine.options, &never())
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, ["workshop/100"]);
    assert_eq!(shown(), "packed atlas");
}

#[test]
fn only_an_exact_mod_suffix_lists_a_local_descriptor() {
    let machine = Machine::new("suffix");
    machine.local("lower.mod", "name=\"Lower\"", "lower");
    machine.local("upper.MOD", "name=\"Upper\"", "upper");
    let ids: Vec<_> = installed(&machine.options, &never())
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, ["mod/lower.mod"]);
}

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
    let mut layers = machine.recorded(&["base"]);
    let plan = apply(&mut layers, &machine.options, &selected, &[], &never()).unwrap();
    assert_eq!(plan.revision, revision());
}

#[test]
fn a_case_only_rename_moves_the_revision() {
    let machine = Machine::new("revision-case");
    machine.local("art.mod", "name=\"Art\"", "art");
    let lower = machine.write("art/common/defines/custom.lua", b"NDefines.NGraphics.A = 1");
    let selected = [pick("mod/art.mod", "Art")];
    let revision = || revision(&machine.options, &selected, &[], &never()).unwrap();
    let first = revision();
    let upper = lower.with_file_name("custom.LUA");
    let between = lower.with_file_name("custom.tmp");
    fs::rename(&lower, &between).unwrap();
    fs::rename(&between, &upper).unwrap();
    let renamed = revision();
    assert_ne!(first, renamed);
    fs::rename(&upper, &between).unwrap();
    fs::rename(&between, &lower).unwrap();
    assert_eq!(first, revision());
}

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
