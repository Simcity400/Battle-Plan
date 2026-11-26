use super::*;
use content_witness::{Entry, Outcome};
use content_reader_layers::{content::ResolvedLayers, test_support::Stack};
use loader::Loader;
use sha2::Digest;
use std::path::Path;

fn fixture(name: &str) -> Stack {
    content_reader_script::test_game::enter();
    let stack = Stack::new(name);
    let base = stack.base();
    stack.write(
        &base,
        "map/default.map",
        r#"
        definitions="definition.csv" provinces="provinces.bmp"
        terrain="terrain.bmp" rivers="rivers.bmp" heightmap="heightmap.bmp"
        tree_definition="trees.bmp" continent="continents.txt" adjacencies="adjacencies.csv"
    "#,
    );
    stack.write(
        &base,
        "map/definition.csv",
        "0;0;0;0;land;false;synthetic;0\n1;1;2;3;land;false;synthetic;1\n",
    );
    stack.write(&base, "map/adjacencies.csv", "From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n-1;-1;;-1;-1;-1;-1;-1;;\n");
    stack.write(&base, "map/continents.txt", "continents = { synthetic }");
    stack.write(&base, "map/unitstacks.txt", "1;0;7;2;3;0;1\n");
    stack.write(
        &base,
        "common/terrain/synthetic.txt",
        "categories = { synthetic = { movement_cost = 1 } }",
    );
    stack.write(
        &base,
        "common/buildings/synthetic.txt",
        "buildings = { synthetic = { } }",
    );
    stack.write(
        &base,
        "common/combat_tactics.txt",
        "phases = { } tactics = { }",
    );
    stack.write(
        &base,
        "common/defines/synthetic.lua",
        r#"
        NDefines = {
            NBuildings = { MAX_BUILDING_LEVELS = 10,
                INFRASTRUCTURE_RESOURCE_BONUS = 0.25, SUPPLY_ROUTE_RESOURCE_BONUS = 0.5 },
            NNavy = { NAVAL_INVASION_PREPARE_DAYS = 7 },
            NGame = { MUSIC_PLAYER_RECENTLY_PLAYED_SIZE = 3 }
        }
    "#,
    );
    stack.write(
        &base,
        "localisation/synthetic_l_english.yml",
        "\u{feff}l_english:\n SYNTHETIC:0 \"Shared operand\"\n",
    );
    stack.write(
        &base,
        "playlists/settings.txt",
        "enabled = { synthetic_station }",
    );
    std::fs::write(base.join("map/provinces.bmp"), bitmap(24)).unwrap();
    std::fs::write(base.join("map/heightmap.bmp"), bitmap(8)).unwrap();
    stack
}

fn bitmap(bits: u16) -> Vec<u8> {
    let palette = if bits == 8 { 1024 } else { 0 };
    let offset = 54 + palette;
    let stride = if bits == 8 { 12 } else { 36 };
    let mut bytes = vec![0; offset + stride * 6];
    let length = bytes.len() as u32;
    bytes[0..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&length.to_le_bytes());
    bytes[10..14].copy_from_slice(&(offset as u32).to_le_bytes());
    bytes[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&12_i32.to_le_bytes());
    bytes[22..26].copy_from_slice(&6_i32.to_le_bytes());
    bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&bits.to_le_bytes());
    if bits == 8 {
        bytes[offset..].fill(32);
    } else {
        for pixel in bytes[offset..].chunks_exact_mut(3) {
            pixel.copy_from_slice(&[3, 2, 1]);
        }
    }
    bytes
}

fn previous_inputs(
    layers: &ResolvedLayers,
    user: &Path,
    language: &str,
) -> io::Result<ProductInputs> {
    let map = Arc::new(content_reader_map::map::load(layers)?);
    let (_, default_map) = content_reader_key_names::terrain::read(layers)?;
    let defines = content_reader_defines::defines::read(layers)?;
    let audio = previous_audio_source(layers, &defines, user, &mut Vec::new());
    let heightmap = content_reader_image::bmp::decode_indexed(
        &layers.read(&format!("map/{}", default_map.heightmap))?,
    )
    .map_err(|error| {
        viewer_runtime::errors::invalid(format!("map/{}: {error}", default_map.heightmap))
    })?;
    viewer_gpu::textures::expect_size(
        "map/heightmap.bmp",
        heightmap.width,
        heightmap.height,
        [map.width, map.height],
    )?;
    let traits = content_reader_key_names::unit_leader::Traits::registered(layers)?;
    let _sub_units =
        content_reader_key_names::unit_leader::sub_units(layers).map_err(|error| error.to_string());
    let country_traits = content_reader_military::leader_traits::read(layers)
        .map(|(traits, _)| Arc::new(traits))
        .map_err(|error| error.to_string());
    let tactics = content_reader_military::combat_tactics::read(layers)?;
    let mut scripted = content_reader_key_names::scripted_triggers::read(layers)?;
    for template in scripted.templates.values_mut() {
        if let Ok(body) = &mut template.body {
            content_reader_military::combat_tactics::resolve_phase_triggers(&tactics.phases, body);
        }
    }
    let buildings = content_reader_key_names::buildings::read(layers)?;
    let tag_aliases = Arc::new(content_reader_key_names::tag_aliases::read(layers)?);
    let leader_roles = content_reader_key_names::characters::leader_templates(layers)
        .map(|templates| dashboard_text::triggers::LeaderRoles::read_through(templates, layers.clone()))
        .map(Arc::new);
    let (states, _) = content_reader_states::states::read(layers, &defines, &map.definitions)?;
    let states = states.definitions();
    let graph = match content_reader_map::province_graph::ProvinceGraph::read(layers) {
        Ok((graph, _)) => Ok(graph),
        Err(error) => {
            content_reader_map::province_geometry::ProvinceGeometry::read(layers)?;
            Err(error.to_string())
        }
    };
    let state_provinces: BTreeMap<u32, Vec<u32>> = states
        .iter()
        .map(|(&id, state)| (id, state.provinces.clone()))
        .collect();
    let neighbor_states = match &graph {
        Ok(graph) => graph
            .neighbor_states(&state_provinces)
            .map_err(|error| error.to_string()),
        Err(error) => Err(format!("the province graph cannot be built ({error})")),
    };
    let dlcs = layers.dlc_names()?;
    let resource_bonuses = dashboard_text::state_resources::resource_bonuses(&defines)?;
    let continents = content_reader_key_names::terrain::read_default_map(layers)
        .and_then(|map| layers.read(&map.file("continent")?))
        .and_then(|bytes| {
            content_reader_equipment_graphics::equipment_graphics::context::continent_names(&bytes)
        })
        .map_err(|error| error.to_string());
    let invasion_prepare_days = defines
        .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
        .map_err(|error| error.to_string());
    let script_constants =
        content_reader_politics::script_constants::read(layers).map_err(|error| error.to_string());
    let localisation = layers.artwork("map names", |stack| {
        let rows = content_reader_map::adjacencies::read(stack)?;
        viewer_map_text::name_pass::ProvinceGraph::build(
            map.width,
            map.height,
            &map.province_ids,
            &rows,
        )?;
        content_reader_localisation::localisation::read(stack, language).map(Arc::new)
    })?;
    let mut anchors = BTreeMap::new();
    for stack in content_reader_map::positions::read_unit_stacks(layers)? {
        anchors.insert((stack.province, stack.kind), [stack.x, stack.y, stack.z]);
    }
    let fog = Arc::new(FogContent::load(layers, &map, &defines).map_err(|error| error.to_string()));
    let buildings = content_reader_key_names::buildings::read(layers).map(|_| buildings)?;
    let (ideologies, _) = content_reader_key_names::ideologies::read(layers)?;
    let (resources, _) = content_reader_key_names::resources::read(layers)?;
    let movement: io::Result<MovementContent> = (|| {
        Ok(MovementContent::new(
            content_reader_map::movement::MovementMap::read(layers, &defines)?,
            content_reader_map::movement::MovementDefines::read(&defines)?,
        ))
    })();
    let geometry = viewer_gpu::province_geometry::ProvinceGeometry::build(&map);
    let heights = viewer_gpu::province_map::HeightField {
        width: heightmap.width,
        height: heightmap.height,
        values: &heightmap.indices,
    };
    let mut default_anchors = BTreeMap::new();
    for province in map
        .definitions
        .keys()
        .copied()
        .filter(|&province| province != 0)
    {
        if let Some([x, z]) = geometry.centroid(province) {
            let [x, z] = [(x as i32) as f32, (z as i32) as f32];
            default_anchors.insert(province, [x, heights.native_height(x, z), z]);
        }
    }
    let triggers = TriggerContent {
        triggers: Arc::new(content_reader_script::trigger_keys::TriggerNames {
            scripted,
            counts: Some(content_reader_key_names::key_name_products::count_trigger_names(
                &buildings,
                &ideologies,
                &resources,
            )),
        }),
        ideologies: Some(ideologies),
        continents,
        character_traits: Some((traits, country_traits)),
        tag_aliases,
        buildings: buildings.clone(),
        dlcs,
        state_provinces,
        impassable: states
            .iter()
            .filter(|(_, state)| state.impassable)
            .map(|(&id, _)| id)
            .collect(),
        neighbor_states,
        terrain: movement.ok().map(Arc::new),
        resource_bonuses,
        leader_roles,
        surrender_limits: dashboard_text::triggers::SurrenderLimits::read(&defines),
        invasion_prepare_days,
        script_constants,
    };
    Ok(ProductInputs {
        map: Arc::clone(&map),
        fog,
        anchors: Arc::new(anchors),
        default_anchors: Arc::new(default_anchors),
        bounds: Arc::new(viewer_gpu::province_admission::province_boxes(
            &viewer_gpu::province_map::ProvinceMap {
                width: map.width,
                height: map.height,
                ids: &map.province_ids,
            },
        )),
        localisation,
        map_text: events::Content::new(&buildings),
        audio: audio.map(|source| Arc::clone(&source.content)),
        triggers: Arc::new(triggers),
    })
}

use viewer_foundation::movement_timing::MovementContent;

#[test]
fn standalone_content_loads_without_a_device_and_matches_previous_bootstrap_operands() {
    let stack = fixture("product_loader_complete");
    let layers = stack.layers(&[]);
    let old = previous_inputs(&layers, &stack.base(), "english").unwrap();
    let actual = ProductContent::load(&layers, &stack.base(), "english").unwrap();
    assert_eq!(actual.map.as_ref(), old.map.as_ref());
    assert_eq!([actual.map.width, actual.map.height], [12, 6]);
    assert_eq!(actual.anchors.as_ref(), old.anchors.as_ref());
    assert_eq!(actual.anchors[&(1, 0)], [7., 2., 3.]);
    assert_eq!(
        actual.default_anchors.as_ref(),
        old.default_anchors.as_ref()
    );
    assert_eq!(actual.bounds.as_ref(), old.bounds.as_ref());
    assert_eq!(actual.bounds[&1], [0., 0., 12., 6.]);
    assert_eq!(actual.fog.as_ref(), old.fog.as_ref());
    assert_eq!(actual.localisation.as_ref(), old.localisation.as_ref());
    assert_eq!(actual.localisation.get("SYNTHETIC"), Some("Shared operand"));
    let (audio, old_audio) = (actual.audio.as_ref().unwrap(), old.audio.as_ref().unwrap());
    assert_eq!(format!("{audio:?}"), format!("{old_audio:?}"));
    assert_eq!(
        audio.stations.as_deref(),
        Some(["synthetic_station".to_owned()].as_slice())
    );
    assert_eq!(audio.recently_played, 3);
    assert_eq!(actual.triggers.resource_bonuses, [25_000, 50_000]);
    assert_eq!(actual.triggers.continents, old.triggers.continents);
    assert_eq!(actual.triggers.continents.as_ref().unwrap(), &["synthetic"]);
    assert_eq!(actual.triggers.invasion_prepare_days, Ok(7));
    assert_eq!(actual.triggers.tag_aliases, old.triggers.tag_aliases);
    assert_eq!(actual.triggers.buildings, old.triggers.buildings);
    assert_eq!(
        actual.triggers.state_provinces,
        old.triggers.state_provinces
    );
    assert_eq!(actual.triggers.impassable, old.triggers.impassable);
    assert_eq!(
        actual.triggers.neighbor_states,
        old.triggers.neighbor_states
    );
    assert_eq!(
        format!("{:?}", actual.triggers.triggers),
        format!("{:?}", old.triggers.triggers)
    );
    assert_eq!(
        actual.triggers.resource_bonuses,
        old.triggers.resource_bonuses
    );
    assert_eq!(actual.triggers.dlcs, old.triggers.dlcs);
    assert_eq!(actual.triggers.ideologies, old.triggers.ideologies);
    assert_eq!(
        actual.triggers.script_constants,
        old.triggers.script_constants
    );
    assert_eq!(
        format!("{:?}", actual.triggers.character_traits),
        format!("{:?}", old.triggers.character_traits)
    );
    assert_eq!(
        format!("{:?}", actual.triggers.leader_roles),
        format!("{:?}", old.triggers.leader_roles)
    );
    assert_eq!(
        format!("{:?}", actual.triggers.terrain),
        format!("{:?}", old.triggers.terrain)
    );
    assert_eq!(
        actual.intel_map_from_visibility(None).unwrap().texels,
        [0, 0]
    );
}

#[test]
fn standalone_and_staged_loader_witnesses_match_in_both_completion_orders() {
    use std::sync::mpsc::channel;
    let stack = fixture("product_loader_staged_witness");
    let layers = stack.layers(&[]);
    let standalone = ProductContent::load(&layers, &stack.base(), "english").unwrap();
    let expected = standalone.reads.witness().unwrap();
    assert!(
        standalone
            .reads
            .entries()
            .iter()
            .any(|entry| matches!(entry,
        Entry::Read { path, purpose, outcome: Outcome::Bytes { length, .. }, .. }
            if path == b"map/heightmap.bmp" && purpose == "heightmap" && *length > 54))
    );
    let staged = |order: [usize; 6]| {
        let loader = Loader::new(&layers);
        let map = loader.map().unwrap();
        let (_, default_map) = loader.terrain().unwrap();
        let defines = loader.defines().unwrap();
        loader.audio(&defines, &stack.base(), &mut Vec::new());
        let (done_tx, done_rx) = channel();
        std::thread::scope(|scope| {
            let mut gates = Vec::new();
            for stage in 0..6 {
                let (gate_tx, gate_rx) = channel();
                gates.push(gate_tx);
                let done = done_tx.clone();
                let (loader, map, default_map, defines, layers) =
                    (&loader, &map, &default_map, &defines, &layers);
                scope.spawn(move || {
                    gate_rx.recv().unwrap();
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match stage {
                            0 => {
                                let heightmap = loader.heightmap(default_map).unwrap();
                                loader
                                    .validate_heightmap(&heightmap, [map.width, map.height])
                                    .unwrap();
                            }
                            1 => {
                                loader.combat(layers).unwrap();
                                loader::TriggerLoader::new(layers, Some(loader))
                                    .load(defines, map)
                                    .unwrap();
                            }
                            2 => {
                                layers
                                    .artwork("map names", |stack| {
                                        loader.localisation(stack, "english")
                                    })
                                    .unwrap();
                            }
                            3 => {
                                loader.anchors(layers).unwrap();
                                loader.fog(layers, map, defines);
                            }
                            4 => {
                                loader.buildings().unwrap();
                                loader.ideologies().unwrap();
                                loader.resources().unwrap();
                            }
                            5 => {
                                let _ = loader.movement(defines);
                            }
                            _ => unreachable!(),
                        }));
                    done.send((stage, result)).unwrap();
                });
            }
            for stage in order {
                gates[stage].send(()).unwrap();
                let (completed, result) = done_rx.recv().unwrap();
                assert_eq!(completed, stage);
                result.unwrap();
            }
        });
        assert_eq!(loader.reads.witness().unwrap(), expected);
        let retained = loader.reader(&layers, "retained-product");
        let before = loader.reads.witness().unwrap();
        assert_eq!(
            retained.read("map/unitstacks.txt").unwrap(),
            b"1;0;7;2;3;0;1\n"
        );
        assert_ne!(loader.reads.witness().unwrap(), before);
        loader.reads.witness().unwrap()
    };
    assert_eq!(staged([0, 1, 2, 3, 4, 5]), staged([5, 4, 3, 2, 1, 0]));
}

#[test]
fn shared_loader_preserves_missing_and_refused_file_errors_and_their_precedence() {
    let stack = fixture("product_loader_errors");
    let layers = stack.layers(&["map/unitstacks.txt"]);
    let missing = Loader::new(&layers).anchors(&layers).err().unwrap();
    let old_missing = content_reader_map::positions::read_unit_stacks(&layers).unwrap_err();
    assert_eq!(
        (missing.kind(), missing.to_string()),
        (old_missing.kind(), old_missing.to_string())
    );
    stack.write(&stack.addon(), "map/unitstacks.txt", "1;0;NaN;2;3;0;1\n");
    let layers = stack.layers(&[]);
    let refused = Loader::new(&layers).anchors(&layers).err().unwrap();
    let old_refused = content_reader_map::positions::read_unit_stacks(&layers).unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        (refused.kind(), refused.to_string()),
        (old_refused.kind(), old_refused.to_string())
    );
    let full = ProductContent::load(&layers, &stack.base(), "english")
        .err()
        .unwrap();
    let old = previous_inputs(&layers, &stack.base(), "english")
        .err()
        .unwrap();
    assert_eq!(
        (full.kind(), full.to_string()),
        (old.kind(), old.to_string())
    );
    stack.write(&stack.addon(), "map/heightmap.bmp", "refused bitmap");
    let layers = stack.layers(&[]);
    let full = ProductContent::load(&layers, &stack.base(), "english")
        .err()
        .unwrap();
    let old = previous_inputs(&layers, &stack.base(), "english")
        .err()
        .unwrap();
    assert_eq!(
        (full.kind(), full.to_string()),
        (old.kind(), old.to_string())
    );
    assert!(full.to_string().starts_with("map/heightmap.bmp:"));
}

#[test]
fn invalid_payload_witnesses_differ_while_original_errors_and_stamps_match() {
    use std::{
        fs::{File, FileTimes},
        time::{Duration, UNIX_EPOCH},
    };
    let stack = Stack::new("product_invalid_payloads");
    stack.write(&stack.base(), "map/invalid.txt", "original");
    let path = stack.base().join("map/invalid.txt");
    let stamp = UNIX_EPOCH + Duration::from_secs(123456789);
    let layers = stack.layers(&[]);
    let read = |bytes: &[u8], prefix: bool| {
        std::fs::write(&path, bytes).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(stamp))
            .unwrap();
        let reads = witness::ContentReads::new(&layers);
        let error = if prefix {
            reads
                .load(|layers| layers.read_prefix("map/invalid.txt", 2))
                .unwrap_err()
        } else {
            reads
                .load(|layers| layers.read("map/invalid.txt"))
                .unwrap_err()
        };
        let old = layers.read("map/invalid.txt").unwrap_err();
        assert_eq!(
            (error.kind(), error.to_string()),
            (old.kind(), old.to_string())
        );
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
        assert!(reads.entries().iter().any(|entry| matches!(entry, Entry::Read { outcome: Outcome::Error { payload: Some(payload), .. }, .. } if payload.length == bytes.len() as u64 && payload.digest == <[u8;32]>::from(sha2::Sha256::digest(bytes)))));
        ((error.kind(), error.to_string()), reads)
    };
    for prefix in [false, true] {
        let (first_error, first) = read(b"invalid1", prefix);
        let (second_error, second) = read(b"invalid2", prefix);
        assert_eq!(first_error, second_error);
        assert_ne!(first.witness().unwrap(), second.witness().unwrap());
        let strip_payload = |reads: witness::ContentReads| {
            reads
                .entries()
                .into_iter()
                .map(|mut entry| {
                    if let Entry::Read {
                        outcome: Outcome::Error { payload, .. },
                        ..
                    } = &mut entry
                    {
                        *payload = None;
                    }
                    entry
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strip_payload(first), strip_payload(second));
    }
}

fn previous_audio_source(
    layers: &content_reader_layers::ResolvedLayers,
    defines: &content_reader_defines::defines::Defines,
    user: &std::path::Path,
    warnings: &mut Vec<String>,
) -> Option<Arc<viewer_audio::Source>> {
    let catalog = match content_reader_sound::sound::Catalog::read(layers) {
        Ok(catalog) => catalog,
        Err(error) => {
            warnings.push(format!(
                "Map audio is unavailable: the sound definitions are unreadable ({error})."
            ));
            return None;
        }
    };
    if !catalog.notes.is_empty() {
        warnings.push(format!(
            "Sound definitions the game refuses or reads past: {}.",
            catalog.notes.lines().join("; ")
        ));
    }
    let recently_played = defines
        .int32("NGame", "MUSIC_PLAYER_RECENTLY_PLAYED_SIZE")
        .map_err(|error| warnings.push(format!("Map music repeats songs freely: {error}.")))
        .map_or(0, |size| usize::try_from(size).unwrap_or(0));
    let battle = content_reader_sound::combat_sounds::BattleDefines::load(defines)
        .map_err(|error| warnings.push(format!("Map battles are silent: {error}.")))
        .ok();
    let combat_sounds = match content_reader_sound::combat_sounds::Layers::read(layers) {
        Ok(combat_sounds) => {
            if !combat_sounds.notes.is_empty() {
                warnings.push(format!(
                    "Battle sound definitions the game refuses or reads past: {}.",
                    combat_sounds.notes.lines().join("; ")
                ));
            }
            combat_sounds
        }
        Err(error) => {
            warnings.push(format!("Map battles play no layers: {error}."));
            Default::default()
        }
    };
    let defines = content_reader_sound::sound::Defines::load(defines)
        .map_err(|error| {
            warnings.push(format!(
                "Map audio plays its height and voice categories without their volumes: {error}."
            ))
        })
        .ok();
    let music = match content_reader_sound::music::Music::read(layers) {
        Ok(music) => {
            if !music.notes.is_empty() {
                warnings.push(format!(
                    "Music definitions the game refuses or reads past: {}.",
                    music.notes.lines().join("; ")
                ));
            }
            music
        }
        Err(error) => {
            warnings.push(format!("Map music is unavailable: {error}."));
            Default::default()
        }
    };
    let stations = content_reader_sound::music::enabled_stations(user)
        .map_err(|error| {
            warnings.push(format!(
                "Map music plays every station: the playlist settings are unreadable ({error})."
            ))
        })
        .ok()
        .flatten();
    Some(Arc::new(viewer_audio::Source {
        content: Arc::new(
            viewer_audio::Content::new(catalog, defines)
                .with_music(music, stations, recently_played)
                .with_battles(battle, combat_sounds),
        ),
        layers: layers.clone(),
    }))
}

#[test]
fn artwork_localisation_keeps_the_previous_named_fallback_and_renderer_isolation() {
    let stack = fixture("product_loader_artwork");
    let root = stack.base().parent().unwrap().to_path_buf();
    let art = root.join("art");
    let relative = "localisation/synthetic_l_english.yml";
    stack.write(
        &art,
        relative,
        "\u{feff}l_english:\n SYNTHETIC:0 \"Visual operand\"\n",
    );
    stack.write(
        &stack.base(),
        "mod/art.mod",
        &format!(
            "name=\"Synthetic art\"\npath=\"{}\"\n",
            art.display().to_string().replace('\\', "/")
        ),
    );
    let game = root.join("steamapps/common/Game");
    std::fs::create_dir_all(&game).unwrap();
    let options = content_reader_layers::ResolveOptions {
        game_dir: game,
        user_dir: stack.base(),
        cache_dir: root.join("cache"),
    };
    let mut layers = stack.layers(&[]);
    content_reader_layers::content::visual::apply(
        &mut layers,
        &options,
        &[content_reader_layers::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Synthetic art".into(),
        }],
        &[],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let shown = ProductContent::load(&layers, &stack.base(), "english").unwrap();
    assert_eq!(shown.localisation.get("SYNTHETIC"), Some("Visual operand"));
    let visual_index = layers.presentation().layers.len() - 1;
    let visual_layer = &layers.presentation().layers[visual_index];
    assert!(shown.reads.entries().iter().any(|entry| matches!(entry,
        Entry::Read { path, layer: Some(layer), .. } if path == relative.as_bytes()
            && layer.id == visual_layer.id && layer.index == visual_index)));

    let path = art.join(relative);
    use std::os::windows::fs::OpenOptionsExt;
    let unreadable = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let old = previous_inputs(&layers, &stack.base(), "english").unwrap();
    let fallback = ProductContent::load(&layers, &stack.base(), "english").unwrap();
    assert_eq!(fallback.localisation, old.localisation);
    assert_eq!(
        fallback.localisation.get("SYNTHETIC"),
        Some("Shared operand")
    );
    assert!(
        layers
            .read_warnings()
            .overlay_refusals()
            .unwrap()
            .contains("map names (")
    );
    assert_ne!(
        shown.reads.witness().unwrap(),
        fallback.reads.witness().unwrap()
    );
    let before = fallback.reads.witness().unwrap();
    layers
        .presentation()
        .has_file("interface/not_loaded.gui")
        .unwrap();
    assert_eq!(before, fallback.reads.witness().unwrap());
    drop(unreadable);
}

#[test]
fn audio_fallback_matches_the_old_policy_and_sample_playback_is_unobserved() {
    let stack = fixture("product_loader_audio");
    stack.write(
        &stack.base(),
        "sound/synthetic.asset",
        "sound = { name = synthetic file = synthetic.wav }",
    );
    stack.write(&stack.base(), "sound/synthetic.wav", "sample bytes");
    let layers = stack.layers(&[]);
    let loader = Loader::new(&layers);
    let defines = loader.defines().unwrap();
    let (mut warnings, mut old_warnings) = (Vec::new(), Vec::new());
    let actual = loader
        .audio(&defines, &stack.base(), &mut warnings)
        .unwrap();
    let old = previous_audio_source(&layers, &defines, &stack.base(), &mut old_warnings).unwrap();
    assert_eq!(warnings, old_warnings);
    assert!(!warnings.is_empty());
    assert_eq!(
        format!("{:?}", actual.content),
        format!("{:?}", old.content)
    );
    let before = loader.reads.witness().unwrap();
    assert_eq!(
        actual.layers.read_uncovered("sound/synthetic.wav").unwrap(),
        b"sample bytes"
    );
    assert_eq!(before, loader.reads.witness().unwrap());
    use std::os::windows::fs::OpenOptionsExt;
    let path = stack.base().join("sound/synthetic.asset");
    let unreadable = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let loader = Loader::new(&layers);
    let (mut warnings, mut old_warnings) = (Vec::new(), Vec::new());
    assert!(
        loader
            .audio(&defines, &stack.base(), &mut warnings)
            .is_none()
    );
    assert!(previous_audio_source(&layers, &defines, &stack.base(), &mut old_warnings).is_none());
    assert_eq!(warnings, old_warnings);
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0].starts_with("Map audio is unavailable: the sound definitions are unreadable (")
    );
    assert!(
        loader.reads.entries().iter().any(
            |entry| matches!(entry, Entry::Result { purpose, status: 0, message, .. } if purpose == "session-note" && message == &warnings[0])
        )
    );
    drop(unreadable);
}
