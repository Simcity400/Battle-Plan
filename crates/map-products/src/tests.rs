use super::tables;
use super::*;
use content_witness::{Entry, Outcome};
use content_reader_layers::{content::ResolvedLayers, test_support::Stack};
use content_reader_map::map::{ProvinceDefinition, ProvinceKind};
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableView};
use sha2::Digest;

fn frame(count: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 7,
        hour: 40,
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
        metrics: Metrics::default(),
    };
    put(
        &mut frame,
        TableRows::Countries(
            (1..=count)
                .map(|slot| CountryRow {
                    id: slot + 100,
                    canonical_slot: slot,
                    tag: format!("Z{slot}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(
            (1..=count)
                .map(|slot| CountryLinkRow {
                    id: slot + 100,
                    canonical_slot: slot,
                })
                .collect(),
        ),
    );
    frame
}
fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}
fn choice() -> perspective::Choice {
    perspective::Choice {
        country: perspective::Country::Tag("Z1".into()),
        fog: false,
    }
}
fn trigger_content() -> TriggerContent {
    TriggerContent {
        triggers: Default::default(),
        ideologies: None,
        continents: Ok(Vec::new()),
        character_traits: None,
        tag_aliases: Default::default(),
        buildings: Default::default(),
        dlcs: Default::default(),
        state_provinces: Default::default(),
        impassable: Default::default(),
        neighbor_states: Ok(Default::default()),
        terrain: None,
        resource_bonuses: [0; 2],
        leader_roles: Err("fixture has no leader definitions".into()),
        surrender_limits: Err("fixture has no surrender defines".into()),
        invasion_prepare_days: Err("fixture has no invasion define".into()),
        script_constants: Err("fixture has no constants".into()),
    }
}
fn content(stack: &Stack) -> ProductContent {
    let layers = stack.layers(&[]);
    let reads = ContentReads::new(&layers);
    let inputs = reads
        .load(|layers| {
            let values = layers.read("map/fixture.txt")?;
            let authored = std::str::from_utf8(&values).unwrap().parse::<f32>().unwrap();
            Ok(ProductInputs {
                map: Arc::new(MapData {
                    width: 12,
                    height: 6,
                    province_ids: vec![1; 72],
                    definitions: [(
                        1,
                        ProvinceDefinition {
                            id: 1,
                            rgb: [1, 2, 3],
                            kind: ProvinceKind::Land,
                            continent: 1,
                        },
                    )]
                    .into(),
                    country_colors: Default::default(),
                }),
                fog: Arc::new(Err("fixture fog definitions unavailable".into())),
                anchors: Arc::new([((1, 0), [authored, 2., 3.])].into()),
                default_anchors: Arc::new([(2, [4., 5., 6.])].into()),
                bounds: Arc::new([(1, [0., 0., 12., 6.]), (2, [1., 2., 3., 4.])].into()),
                localisation: Arc::new(Default::default()),
                map_text: events::Content::new(&Default::default()),
                audio: Some(Arc::new(
                    viewer_audio::Content::new(Default::default(), None).with_battles(
                        Some(content_reader_sound::combat_sounds::BattleDefines {
                            effect: "fixture_distant".into(),
                            falloff_default: 80.,
                            min_unit_count: 1,
                        }),
                        Default::default(),
                    ),
                )),
                triggers: Arc::new(trigger_content()),
            })
        })
        .unwrap();
    ProductContent::from_inputs(reads, inputs)
}
fn fixture(name: &str) -> Stack {
    let stack = Stack::new(name);
    stack.write(&stack.base(), "map/fixture.txt", "7");
    stack
}

#[test]
fn display_content_pressure_is_retryable_without_changing_source_evidence() {
    let stack = fixture("display_content_pressure_evidence");
    let reads = ContentReads::new(&stack.layers(&[]));
    let records = || {
        let mut records = Vec::new();
        reads
            .witness()
            .unwrap()
            .visit(|_, bytes| {
                records.push(bytes.to_vec());
                Ok(())
            })
            .unwrap();
        records
    };
    let before = records();
    for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::OutOfMemory] {
        let error = reads
            .load_retryable(|_| Err::<(), _>(io::Error::new(kind, "display allocation pressure")))
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(before, records());
    }
    reads.load_retryable(|_| Ok(())).unwrap();
    assert_eq!(before, records());
    reads
        .load_retryable(|_| {
            Err::<(), _>(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid content",
            ))
        })
        .unwrap_err();
    assert_ne!(before, records());
}

#[test]
fn building_bank_survives_dependency_binding_changes_and_replacement() {
    use viewer_buildings::map_buildings::{CityTerrain, MapBuildingAssets};
    content_reader_script::test_game::enter();
    let stack = fixture("building_bank_immutable_binding");
    stack.write(
        &stack.base(),
        "common/buildings/synthetic.txt",
        "buildings = { synthetic_works = { military_production = 1 } }",
    );
    stack.write(
        &stack.base(),
        "common/defines/buildings.lua",
        r#"
        NDefines = {
            NBuildings = { MAX_BUILDING_LEVELS = 10 },
            NGraphics = { MAP_BUILDINGS_SHRINK_DISTANCE = 190,
                MESH_POPUP_SCALE_UP_SPEED = 6, MAP_BUILDINGS_DESTROYED_STATUS = 55,
                MAP_BUILDINGS_DESTROYED_DEAD_ZONE = 75, MAP_BUILDINGS_DAMAGED_THRESHOLD = 0.7 }
        }
    "#,
    );
    let content = content(&stack);
    let layers = stack.layers(&[]);
    let defines = content_reader_defines::defines::read(&layers).unwrap();
    let load_bank = || {
        MapBuildingAssets::load(
            &layers,
            content_reader_image::native::Decode::default(),
            &content.map,
            &defines,
            &mut Vec::new(),
        )
        .unwrap()
    };
    let assets = load_bank();
    let mut frame = frame(1);
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            owner: 101,
            controller: 101,
            ..Default::default()
        }]),
    );
    let city = Arc::new(vec![false; 72]);
    let dependencies = || {
        content
            .reads
            .dependent_binding(
                "buildings",
                &[
                    "map",
                    "defines",
                    "trigger",
                    "heightmap",
                    "display/buildings",
                    "session-error",
                ],
            )
            .unwrap()
    };
    let before = dependencies();
    let first = ProductHour::new(&content, &frame, 0.0)
        .buildings(
            &assets,
            &choice(),
            &city,
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap()
        .0;
    content.reads.session_error(&std::io::Error::other(
        "late unrelated optional content refusal",
    ));
    assert_ne!(before, dependencies());
    frame.hour += 1;
    let next = ProductHour::new(&content, &frame, 0.0)
        .buildings(
            &assets,
            &choice(),
            &city,
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap()
        .0;
    assert_eq!(first.bank, content.display_binding().unwrap());
    assert_eq!(first.bank, next.bank);
    let replacement = load_bank();
    assert_ne!(assets.projection_id(), replacement.projection_id());
    let replaced = ProductHour::new(&content, &frame, 0.0)
        .buildings(
            &replacement,
            &choice(),
            &city,
            &mut viewer_runtime::profiling::BuilderSpans::new(false),
        )
        .unwrap()
        .0;
    assert_eq!(replaced.bank, first.bank);
    assert_eq!(
        replacement.display_binding().unwrap(),
        content.display_binding().unwrap()
    );
    let hour = ProductHour::new(&content, &frame, 0.0);
    let vision = hour.vision(&choice()).unwrap();
    let (prepared, _) = assets
        .field(
            &frame,
            &vision,
            &CityTerrain {
                width: 12,
                height: 6,
                spawn_city: &city,
            },
        )
        .unwrap();
    let mut wrong = first.bank;
    wrong[0] ^= 1;
    let error = assets
        .display(
            &prepared,
            wrong,
            vision.ours(),
            vision.fog(),
            Vec::new(),
            None,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "building bank was replaced without rebinding its content"
    );
}

#[test]
fn content_loads_and_builds_the_clear_intel_image_without_a_device() {
    let stack = fixture("products_without_device");
    let content = content(&stack);
    let intel = content.intel_map_from_visibility(None).unwrap();
    assert_eq!(intel.texels, [0, 0]);
    assert!(!intel.fog);
    assert_eq!(
        content.impact_place(1),
        Some(([7., 2., 3.], [0., 0., 12., 6.]))
    );
    let entries = content.reads.entries();
    assert!(entries.iter().any(|entry| matches!(entry,
        Entry::Read { path, outcome: Outcome::Bytes { length: 1, .. }, .. }
            if path == b"map/fixture.txt")));
}

#[test]
fn witness_is_a_canonical_set_including_misses_errors_and_retained_reads() {
    let stack = fixture("products_witness_set");
    stack.write(&stack.base(), "map/later.txt", "later");
    let layers = stack.layers(&["map/hidden"]);
    let reads = ContentReads::new(&layers);
    let retained = reads
        .load(|layers| {
            assert!(!layers.has_file("map/missing.txt")?);
            assert_eq!(layers.read("map/fixture.txt")?, b"7");
            Ok(layers.clone())
        })
        .unwrap();
    let before = reads.witness().unwrap();
    assert_eq!(retained.read("map/fixture.txt").unwrap(), b"7");
    assert_eq!(before, reads.witness().unwrap());
    let other = ContentReads::new(&layers);
    other
        .load(|layers| {
            layers.read("map/fixture.txt")?;
            layers.has_file("map/missing.txt")
        })
        .unwrap();
    assert_eq!(before, other.witness().unwrap());
    let result: io::Result<content_reader_layers::content::Bytes> = reads.load(|layers| layers.read("map/missing.txt"));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    retained.read("map/later.txt").unwrap();
    assert_ne!(before, reads.witness().unwrap());
    let entries = reads.entries();
    assert_eq!(entries.len(), 6);
    assert!(matches!(&entries[0], Entry::Roster { layers } if layers.len() == 2));
    assert!(
        matches!(&entries[1], Entry::Result { purpose, status: 1, kind, message }
        if purpose == "session-error" && kind == "NotFound" && message == "map/missing.txt")
    );
    assert!(matches!(&entries[2], Entry::Read { path, purpose, .. }
        if path == b"map/fixture.txt" && purpose == "content"));
    assert!(matches!(&entries[3], Entry::Read { path, .. } if path == b"map/later.txt"));
    assert!(
        matches!(&entries[4], Entry::Read { path, purpose, outcome: Outcome::Probe { present: false }, .. }
        if path == b"map/missing.txt" && purpose == "content")
    );
    assert!(
        matches!(&entries[5], Entry::Read { path, purpose, outcome: Outcome::Error { kind, message, payload: None }, .. }
        if path == b"map/missing.txt" && purpose == "content" && kind == "NotFound" && message == "map/missing.txt")
    );
    let mut encoded = content_witness::WITNESS_CONTRACT.to_vec();
    reads
        .witness()
        .unwrap()
        .visit(|_, bytes| {
            encoded.extend((bytes.len() as u64).to_le_bytes());
            encoded.extend(bytes);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        reads.witness().unwrap().digest,
        <[u8; 32]>::from(sha2::Sha256::digest(&encoded))
    );
}

#[test]
fn witness_ignores_forced_internal_completion_order_and_duplicate_reads() {
    use std::sync::mpsc::channel;
    let stack = fixture("products_witness_gates");
    stack.write(&stack.base(), "map/z.txt", "last");
    let layers = stack.layers(&[]);
    let run = |order: [usize; 2]| {
        let loader = loader::Loader::new(&layers);
        let observed = loader.reader(&layers, "gated");
        let (first_tx, first_rx) = channel();
        let (second_tx, second_rx) = channel();
        let (done_tx, done_rx) = channel();
        std::thread::scope(|scope| {
            let observed = &observed;
            let first_done = done_tx.clone();
            scope.spawn(move || {
                first_rx.recv().unwrap();
                let read = observed.read("map/z.txt");
                first_done.send((0, read)).unwrap();
            });
            scope.spawn(move || {
                second_rx.recv().unwrap();
                let read = observed.read("map/fixture.txt");
                done_tx.send((1, read)).unwrap();
            });
            let gates = [first_tx, second_tx];
            for index in order {
                gates[index].send(()).unwrap();
                let (completed, read) = done_rx.recv().unwrap();
                assert_eq!(completed, index);
                assert_eq!(read.unwrap(), [b"last".as_slice(), b"7".as_slice()][index]);
            }
        });
        let before = loader.reads.witness().unwrap();
        observed.read("map/fixture.txt").unwrap();
        assert_eq!(before, loader.reads.witness().unwrap());
        let entries = loader.reads.entries();
        assert_eq!(entries.len(), 3);
        assert!(
            matches!(&entries[1], Entry::Read { path, purpose, .. } if path == b"map/fixture.txt" && purpose == "gated")
        );
        assert!(matches!(&entries[2], Entry::Read { path, .. } if path == b"map/z.txt"));
        (before, entries)
    };
    assert_eq!(run([0, 1]), run([1, 0]));
}

#[test]
fn renderer_and_gui_reads_do_not_enter_the_product_witness() {
    let stack = fixture("products_renderer_scope");
    stack.write(&stack.base(), "gfx/renderer.txt", "pixels");
    stack.write(&stack.base(), "interface/gui.txt", "widgets");
    let layers = stack.layers(&[]);
    let preparation = loader::Loader::new(&layers);
    let viewer = loader::Loader::new(&layers);
    preparation
        .reader(&layers, "anchors")
        .read("map/fixture.txt")
        .unwrap();
    viewer
        .reader(&layers, "anchors")
        .read("map/fixture.txt")
        .unwrap();
    let before = viewer.reads.witness().unwrap();
    assert_eq!(
        layers.read_uncovered("gfx/renderer.txt").unwrap(),
        b"pixels"
    );
    assert_eq!(layers.read("interface/gui.txt").unwrap(), b"widgets");
    assert_eq!(before, viewer.reads.witness().unwrap());
    assert_eq!(
        preparation.reads.witness().unwrap(),
        viewer.reads.witness().unwrap()
    );
    assert_eq!(viewer.reads.entries().len(), 2);
}

#[test]
fn retained_readers_keep_canonical_layer_indices_across_recorded_subsets() {
    let stack = fixture("products_reader_layer_indices");
    stack.write(&stack.addon(), "map/fixture.txt", "7");
    let layers = stack.layers(&[]);
    let loader = loader::Loader::new(&layers);
    let mut subset = ResolvedLayers::from_folders(&[(&stack.addon(), &[])]).unwrap();
    subset.layers[0].id = layers.layers[1].id.clone();
    loader
        .reader(&layers, "retained")
        .read("map/fixture.txt")
        .unwrap();
    let before = loader.reads.witness().unwrap();
    assert_eq!(
        loader
            .reader(&subset, "retained")
            .read("map/fixture.txt")
            .unwrap(),
        b"7"
    );
    assert_eq!(loader.reads.witness().unwrap(), before);
    assert_eq!(loader.reads.entries().len(), 2);
    assert!(loader.reads.entries().iter().any(|entry| matches!(entry,
        Entry::Read { layer: Some(layer), .. } if layer.index == 1 && layer.id == "test1")));
}

#[test]
fn witness_hashes_bytes_not_sizes_or_timestamps_and_logs_verification_errors() {
    use std::{
        fs::{File, FileTimes},
        time::{Duration, UNIX_EPOCH},
    };
    let stack = fixture("products_witness_bytes");
    let path = stack.base().join("map/fixture.txt");
    let stamp = UNIX_EPOCH + Duration::from_secs(123456789);
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(stamp))
        .unwrap();
    let old_layers = stack.layers(&[]);
    let before = loader::Loader::new(&old_layers);
    before
        .reader(&old_layers, "edit")
        .read("map/fixture.txt")
        .unwrap();
    let size = std::fs::metadata(&path).unwrap().len();
    stack.write(&stack.base(), "map/fixture.txt", "8");
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(stamp))
        .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    let fresh = stack.layers(&[]);
    let after = loader::Loader::new(&fresh);
    after
        .reader(&fresh, "edit")
        .read("map/fixture.txt")
        .unwrap();
    assert_ne!(
        before.reads.witness().unwrap().digest,
        after.reads.witness().unwrap().digest
    );
    let error = before
        .reader(&old_layers, "edit")
        .read("map/fixture.txt")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "content file changed since verification: map/fixture.txt"
    );
    assert!(before.reads.entries().iter().any(|entry| matches!(entry,
        Entry::Read { outcome: Outcome::Error { kind, message, payload: Some(payload) }, .. }
            if kind == "InvalidData" && message == &error.to_string()
                && payload.length == 1 && payload.digest == <[u8; 32]>::from(sha2::Sha256::digest(b"8")))));
}

#[test]
fn witness_changes_on_mod_order_replace_paths_and_a_new_higher_layer_file() {
    let stack = fixture("products_witness_layers");
    let low = stack.base();
    let high = stack.addon();
    stack.write(&low, "map/value.txt", "same");
    stack.write(&high, "map/value.txt", "same");
    let read = |folders: &[(&std::path::Path, &[&str])]| {
        let mut layers = ResolvedLayers::from_folders(folders).unwrap();
        for layer in &mut layers.layers {
            layer.id = match &layer.source {
                content_reader_layers::content::LayerSource::Folder(root) => {
                    root.file_name().unwrap().to_string_lossy().into_owned()
                }
                _ => unreachable!(),
            };
        }
        let loader = loader::Loader::new(&layers);
        assert_eq!(
            loader
                .reader(&layers, "layers")
                .read("map/value.txt")
                .unwrap(),
            b"same"
        );
        loader.reads
    };
    let resolved = |reads: &ContentReads| {
        reads
            .entries()
            .into_iter()
            .filter(|entry| matches!(entry, Entry::Read { .. }))
            .collect::<Vec<_>>()
    };
    let original = read(&[(&low, &[]), (&high, &[])]);
    let reordered = read(&[(&high, &[]), (&low, &[])]);
    assert_ne!(
        original.witness().unwrap().digest,
        reordered.witness().unwrap().digest
    );
    assert_ne!(resolved(&original), resolved(&reordered));
    assert!(
        matches!(&resolved(&original)[0], Entry::Read { layer: Some(layer), outcome: Outcome::Bytes { length: 4, digest }, .. }
        if layer.id == "mod" && digest == &<[u8; 32]>::from(sha2::Sha256::digest(b"same")))
    );
    assert!(
        matches!(&resolved(&reordered)[0], Entry::Read { layer: Some(layer), .. } if layer.id == "base")
    );
    let replaced = read(&[(&low, &[]), (&high, &["map/unused"])]);
    assert_ne!(
        original.witness().unwrap().digest,
        replaced.witness().unwrap().digest
    );
    assert_ne!(resolved(&original), resolved(&replaced));
    assert!(
        matches!(&resolved(&replaced)[0], Entry::Read { layer: Some(layer), .. }
        if layer.replace_paths == [b"map/unused".to_vec()])
    );
    stack.write(&low, "map/new.txt", "same");
    let layers = stack.layers(&[]);
    let missing_high = loader::Loader::new(&layers);
    missing_high
        .reader(&layers, "layers")
        .read("map/new.txt")
        .unwrap();
    stack.write(&high, "map/new.txt", "same");
    let layers = stack.layers(&[]);
    let higher = loader::Loader::new(&layers);
    higher
        .reader(&layers, "layers")
        .read("map/new.txt")
        .unwrap();
    assert_ne!(
        missing_high.reads.witness().unwrap().digest,
        higher.reads.witness().unwrap().digest
    );
    assert!(higher.reads.entries().iter().any(|entry| matches!(entry,
        Entry::Read { layer: Some(layer), .. } if layer.id == "test1")));
}

struct Original<'a> {
    content: &'a ProductContent,
    frame: &'a DecodedFrame,
    hour_fraction: f32,
}
impl std::ops::Deref for Original<'_> {
    type Target = ProductContent;
    fn deref(&self) -> &ProductContent {
        self.content
    }
}
impl Original<'_> {
    fn ancillary_vision(
        &self,
        choice: &perspective::Choice,
    ) -> io::Result<Arc<perspective::Vision>> {
        let links = tables::core::<row_tables::CountryLinks>(self.frame)?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let tags = tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .map(|row| {
                (
                    row.canonical_slot,
                    String::from_utf8_lossy(&row.tag).into_owned(),
                )
            })
            .collect();
        perspective::Vision::read(
            self.frame,
            &links,
            &tags,
            choice,
            FogContent::loaded(&self.fog),
        )
        .map(Arc::new)
    }
    fn fog_vision(&self, choice: &perspective::Choice) -> io::Result<fog_oracle::Vision> {
        let links = tables::core::<row_tables::CountryLinks>(self.frame)?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let tags = tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .map(|row| {
                (
                    row.canonical_slot,
                    String::from_utf8_lossy(&row.tag).into_owned(),
                )
            })
            .collect();
        fog_oracle::Vision::read(
            self.frame,
            &links,
            &tags,
            choice,
            FogContent::loaded(&self.fog),
        )
    }
    fn fog_display(&self, choice: &perspective::Choice) -> io::Result<map_display_data::Fog> {
        assert!(self.frame.complete::<row_tables::MapEvents>()?.is_empty());
        let vision = self.fog_vision(choice)?;
        let intel = self.intel_map_from_visibility(vision.province_fog())?;
        Ok(map_display_data::Fog {
            limitations: self.limitations_with(choice, Some(&intel))?.lines(),
            intel: Arc::new(intel),
            header: Header {
                viewing: Some(self.viewing_tag(vision.ours())?),
            },
            map_text: map_display_data::MapText {
                view: vision.ours(),
                interval: map_display_data::MapTextResult::Unavailable {
                    reason: "map text interval unavailable or not a singleton; coverage unknown"
                        .into(),
                },
            },
            music: None,
            battles: self.battle_sounds(),
        })
    }
    fn intel_map_from_visibility(&self, fog: Option<&BTreeMap<u32, u8>>) -> io::Result<IntelMap> {
        Ok(fog_oracle::intel(&self.map, fog))
    }
    pub fn nuclear_impacts(&self) -> io::Result<viewer_models::strike_effects::Impacts> {
        viewer_models::strike_effects::Impacts::read(self.frame, |province| {
            self.impact_place(province)
        })
    }
    fn impact_place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        Some((
            viewer_unit_reads::counter_anchors::stack_anchor(
                &self.anchors,
                &self.default_anchors,
                province,
                viewer_foundation::unit_rules::STANDSTILL,
            )?,
            *self.bounds.get(&province)?,
        ))
    }
    pub fn map_text(
        &self,
        vision: &perspective::Vision,
    ) -> viewer_map_text::animated_map_text::events::Prepared {
        use viewer_map_text::animated_map_text::events::{Context, Prepared};
        let view = vision.ours();
        match dashboard_text::table_access::rows::<replay_data::rows::tables::MapEvents>(
            self.frame,
        ) {
            Ok(intervals) => Prepared::new(
                &Context::new(
                    self.frame,
                    vision,
                    &self.content.map_text,
                    &self.content.localisation,
                ),
                intervals.rows(),
                intervals.is_complete(),
                |province| self.impact_place(province),
            ),
            Err(error) => Prepared::unavailable(view, format!("map text table malformed: {error}")),
        }
    }
    pub fn limitations_with(
        &self,
        perspective: &perspective::Choice,
        intel: Option<&IntelMap>,
    ) -> io::Result<Limitations> {
        let frame = self.frame;
        let mut out = tables::limitations(frame);
        out.note_trains(frame);
        out.note_aircraft(frame);
        out.note_point_lights();
        for refused in self.nuclear_impacts()?.refused {
            out.note_field("Nuclear impact effects", &refused);
        }
        let vision = self.fog_vision(perspective)?;
        out.note_perspective(&tables::Perspective {
            tag: self.viewing_tag(vision.ours())?,
            resolution: vision.resolution().clone(),
            fog: vision.fog(),
        });
        let links: BTreeMap<u32, u32> = tables::core::<row_tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        out.note_battle_plan(orders::own_order_count(frame, &links, vision.ours())?);
        if let Ok(raids) = frame.complete::<row_tables::Raids>() {
            let ours: Vec<_> = raids
                .iter()
                .filter(|raid| raid.parent_slot == vision.ours() && raid.phase != 5)
                .collect();
            let launched = ours.iter().filter(|raid| raid.phase == 4).count();
            out.note_raid_arrows(ours.len(), launched);
        }
        if vision.fog() {
            out.note_fog_on_sight(frame);
            if let Some(table) = vision.undecided() {
                out.note_undecided_sight(table);
            }
            if let Some(input) = vision.visibility_gap() {
                out.note_visibility_gap(input);
            }
        }
        if vision.province_fog().is_some() {
            let built;
            let intel = match intel {
                Some(intel) => intel,
                None => {
                    built = self.intel_map_from_visibility(vision.province_fog())?;
                    &built
                }
            };
            out.note_fog_of_war(intel.fogged_texels(), self.hour_fraction);
        }
        for table in vision.truncated() {
            out.note_field(
                &format!("{table} (perspective)"),
                if *table == "UnitIntel" {
                    "the rows this hour did not reach may hold any unit's detection, so a foreign \
                     unit without a recorded row is drawn in full, as with fog of war off, \
                     including where the game hid it or drew its question mark"
                } else {
                    "the rows this hour did not reach are not read as an absent value, so the \
                     intel they would decide is withheld"
                },
            );
        }
        let unreadable = labels::unreadable_overrides(frame)?;
        if !unreadable.is_empty() {
            out.note_field(
                "ProvinceNames.name",
                &format!(
                    "the recorded name of {} {} ({}) is not text this port can lay out, so {} \
                     keeps the static localised name of its province",
                    unreadable.len(),
                    if unreadable.len() == 1 {
                        "province"
                    } else {
                        "provinces"
                    },
                    unreadable
                        .iter()
                        .take(8)
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                    if unreadable.len() == 1 { "it" } else { "each" },
                ),
            );
        }
        labels::note_withheld_capitals(&mut out, frame, &links)?;
        Ok(out)
    }
    pub fn viewing_tag(&self, ours: u32) -> io::Result<String> {
        Ok(tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .find(|country| country.canonical_slot == ours)
            .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
            .unwrap_or_else(|| format!("country slot {ours}")))
    }
    pub fn battle_sounds(&self) -> Option<viewer_audio::Battles> {
        use replay_data::rows::tables;
        use dashboard_text::table_access::{Available, rows};
        let audio = self.content.audio.as_ref()?;
        audio.battle.as_ref()?;
        let mut refusals = Vec::new();
        let mut note = |table: &str, available: Result<bool, String>| match available {
            Ok(true) => {}
            Ok(false) => refusals.push(format!("the recorded {table} table is incomplete")),
            Err(why) => refusals.push(format!("the recorded {table} table is unavailable ({why})")),
        };
        fn state<T: replay_data::rows::TableSchema>(
            available: &std::io::Result<Available<'_, T>>,
        ) -> Result<bool, String> {
            match available {
                Ok(Available::Complete(_)) => Ok(true),
                Ok(Available::Truncated { .. }) => Ok(false),
                Ok(Available::Unavailable { reason }) => Err(reason.to_string()),
                Ok(Available::NotRecorded { mode }) => {
                    Err(format!("not recorded in a {mode} recording"))
                }
                Err(error) => Err(error.to_string()),
            }
        }
        let recorded = rows::<tables::BattleSounds>(self.frame);
        note("BattleSounds", state(&recorded));
        let combats = rows::<tables::LandCombats>(self.frame);
        note("LandCombats", state(&combats));
        let divisions = rows::<tables::Divisions>(self.frame);
        let templates = rows::<tables::DivisionTemplateIcons>(self.frame);
        let tokens = rows::<tables::LexicalTokens>(self.frame);
        let layers = !audio.layers.sounds.is_empty();
        if layers {
            note("Divisions", state(&divisions));
            note("DivisionTemplateIcons", state(&templates));
            note("LexicalTokens", state(&tokens));
        }
        let recorded = recorded.as_ref().map_or(&[][..], Available::rows);
        let combats = combats.as_ref().map_or(&[][..], Available::rows);
        let divisions: BTreeMap<u64, u64> = divisions
            .as_ref()
            .map_or(&[][..], Available::rows)
            .iter()
            .map(|division| (division.id, division.template))
            .collect();
        let templates: BTreeMap<u64, &replay_data::rows::TemplateGrids> = templates
            .as_ref()
            .map_or(&[][..], Available::rows)
            .iter()
            .filter(|template| template.model_unreadable == 0)
            .filter_map(|template| match &template.grids {
                replay_data::rows::Recorded::Value(grids) => Some((template.template, grids)),
                replay_data::rows::Recorded::NotRecorded => None,
            })
            .collect();
        let tokens = tokens.as_ref().map_or(&[][..], Available::rows);
        let name = |token: u32| -> Option<String> {
            let found = tokens.binary_search_by_key(&token, |row| row.token).ok()?;
            Some(String::from_utf8_lossy(&tokens[found].literal).into_owned())
        };
        let (battles, unknown) = original_recorded_battles(recorded, combats, layers, |division| {
            let template = *divisions.get(&division)?;
            if template == 0 {
                return Some(Vec::new());
            }
            let grids = templates.get(&template)?;
            grids
                .ambience_slots()
                .filter(|subunit| subunit.token != 0)
                .map(|subunit| {
                    Some((
                        name(subunit.token)?,
                        usize::try_from(subunit.count).unwrap_or(0),
                    ))
                })
                .collect()
        });
        if unknown > 0 {
            refusals.push(format!(
                "{unknown} combat divisions have no recorded template composition or sub-unit name, so their sub-units do not count toward battle layers"
            ));
        }
        Some(viewer_audio::Battles { battles, refusals })
    }
}
fn original_recorded_battles(
    recorded: &[replay_data::rows::BattleSoundRow],
    combats: &[replay_data::rows::LandCombatRow],
    layers: bool,
    composition: impl Fn(u64) -> Option<Vec<(String, usize)>>,
) -> (Vec<viewer_audio::battle::Battle>, usize) {
    #[derive(Default)]
    struct Front {
        divisions: usize,
        sub_units: BTreeMap<String, usize>,
        unknown: usize,
    }
    let mut fronts: BTreeMap<u32, Front> = BTreeMap::new();
    for combat in combats {
        let front = fronts.entry(combat.province).or_default();
        front.divisions += combat.attackers.len() + combat.defenders.len();
        if !layers {
            continue;
        }
        let members = combat
            .attackers
            .iter()
            .zip(&combat.attacker_classes)
            .chain(combat.defenders.iter().zip(&combat.defender_classes));
        for (&division, _) in members.filter(|(_, class)| **class == viewer_audio::battle::DIVISION_CLASS) {
            match composition(division) {
                Some(units) => {
                    for (unit, count) in units {
                        *front.sub_units.entry(unit).or_default() += count;
                    }
                }
                None => front.unknown += 1,
            }
        }
    }
    let mut unknown = 0;
    let battles = recorded
        .iter()
        .map(|row| {
            let mut battle = viewer_audio::battle::Battle {
                key: row.object,
                provinces: row.provinces.clone(),
                center: [f32::from_bits(row.center_x), f32::from_bits(row.center_z)],
                radius: f32::from_bits(row.radius),
                divisions: 0,
                sub_units: BTreeMap::new(),
            };
            for province in &row.provinces {
                let Some(front) = fronts.get(province) else {
                    continue;
                };
                battle.divisions += front.divisions;
                unknown += front.unknown;
                for (unit, count) in &front.sub_units {
                    *battle.sub_units.entry(unit.clone()).or_default() += count;
                }
            }
            battle
        })
        .collect();
    (battles, unknown)
}

#[test]
fn fog_vision_ancillary_and_error_bytes_match_the_pre_extraction_builder() {
    let stack = fixture("products_original_fog");
    let content = content(&stack);
    for countries in [2, 1500, 5000] {
        let mut frame = frame(countries);
        put(
            &mut frame,
            TableRows::BattleSounds(vec![BattleSoundRow {
                ordinal: 0,
                object: 93,
                provinces: vec![1, 1],
                center_x: 2.5_f32.to_bits(),
                center_z: (-3.0_f32).to_bits(),
                radius: 7.5_f32.to_bits(),
                sounding: true,
            }]),
        );
        put(
            &mut frame,
            TableRows::LandCombats(vec![LandCombatRow {
                province: 1,
                attackers: vec![11, 22],
                defenders: vec![33],
                ..Default::default()
            }]),
        );
        let old = Original {
            content: &content,
            frame: &frame,
            hour_fraction: 0.,
        };
        let hour = ProductHour::new(&content, &frame, 0.);
        let vision = old.ancillary_vision(&choice()).unwrap();
        assert_eq!(*hour.vision(&choice()).unwrap(), *vision);
        let fog = hour
            .fog(&choice(), &FogMemos::new(), &mut BuilderSpans::new(false))
            .unwrap();
        assert_eq!(fog.intel.texels, [0, 0]);
        assert!(!fog.intel.fog);
        assert_eq!(fog.header.viewing.as_deref(), Some("Z1"));
        assert_eq!(
            fog.limitations,
            old.limitations_with(&choice(), Some(&fog.intel)).unwrap()
        );
        assert_eq!(
            format!("{:?}", fog.map_text),
            format!("{:?}", old.map_text(&vision))
        );
        assert_eq!(fog.battles.as_deref(), old.battle_sounds().as_ref());
        assert!(fog.music.is_none());
        let battles = &fog.battles.as_ref().unwrap().battles;
        assert_eq!(battles.len(), 1);
        assert_eq!(battles[0].key, 93);
        assert_eq!(battles[0].divisions, 6);
        assert_eq!(
            battles[0].center.map(f32::to_bits),
            [2.5_f32.to_bits(), (-3.0_f32).to_bits()]
        );

        drop(hour);
        frame.tables[TableId::Countries.index()] = TableView::Unavailable {
            reason: "fixture country read failed".into(),
        }
        .into();
        let hour = ProductHour::new(&content, &frame, 0.);
        let old = Original {
            content: &content,
            frame: &frame,
            hour_fraction: 0.,
        };
        let actual = hour
            .fog(&choice(), &FogMemos::new(), &mut BuilderSpans::new(false))
            .err()
            .unwrap();
        let expected = old.fog_vision(&choice()).unwrap_err();
        assert_eq!(actual.kind(), expected.kind());
        assert_eq!(actual.to_string(), expected.to_string());
        assert_eq!(
            format!("fog: {actual}"),
            "fog: hour 40 Countries is unavailable: fixture country read failed; the core tables Provinces, CountryLinks and Countries carry the political map itself, so this hour cannot be drawn"
        );
        assert_eq!(
            format!("{}: {actual}", FOG_PART_NAME),
            format!("fog: {expected}")
        );
        assert_eq!(FOG_PART_RANK, 6);
    }
}

#[test]
fn impacts_keep_recorded_order_anchor_fallbacks_and_named_missing_data() {
    let stack = fixture("products_original_impacts");
    let content = content(&stack);
    let mut frame = frame(2);
    put(
        &mut frame,
        TableRows::NuclearStrikes(vec![
            NuclearStrikeRow {
                id: 9,
                province: 2,
                nuke_type: 0,
                ..Default::default()
            },
            NuclearStrikeRow {
                id: 3,
                province: 1,
                nuke_type: strike_effects::THERMONUCLEAR_TYPE,
                ..Default::default()
            },
            NuclearStrikeRow {
                id: 4,
                province: 99,
                ..Default::default()
            },
        ]),
    );
    let actual = ProductHour::new(&content, &frame, 0.)
        .nuclear_impacts()
        .unwrap();
    assert_eq!(
        actual.impacts,
        [
            strike_effects::Impact {
                id: 9,
                province: 2,
                effect: strike_effects::Effect::Nuclear,
                position: [4., 5., 6.],
                bounds: [1., 2., 3., 4.]
            },
            strike_effects::Impact {
                id: 3,
                province: 1,
                effect: strike_effects::Effect::Thermonuclear,
                position: [7., 2., 3.],
                bounds: [0., 0., 12., 6.]
            },
        ]
    );
    assert_eq!(
        actual.refused,
        [
            "1 nuclear impacts have no effect: their province has no standstill anchor or painted box"
        ]
    );
    assert_eq!(
        actual,
        Original {
            content: &content,
            frame: &frame,
            hour_fraction: 0.
        }
        .nuclear_impacts()
        .unwrap()
    );
    frame.tables[TableId::NuclearStrikes.index()] = TableView::Unavailable {
        reason: "fixture impact coverage".into(),
    }
    .into();
    let unavailable = ProductHour::new(&content, &frame, 0.)
        .nuclear_impacts()
        .unwrap();
    assert!(unavailable.impacts.is_empty());
    assert_eq!(
        unavailable.refused,
        ["NuclearStrikes unavailable (fixture impact coverage): impacts have no effect"]
    );
}

#[test]
fn intel_bytes_preserve_the_six_pixel_average_and_missing_province_visibility() {
    let stack = fixture("products_intel_bytes");
    let content = content(&stack);
    let fog = content
        .intel_map_from_visibility(Some(&[(1, 128)].into()))
        .unwrap();
    assert_eq!(fog.texels, [127, 127]);
    assert!(fog.fog);
    assert_eq!(fog.fogged_texels(), 2);
    let missing = content
        .intel_map_from_visibility(Some(&BTreeMap::new()))
        .unwrap();
    assert_eq!(missing.texels, [0, 0]);
    assert!(missing.fog);
}

#[test]
fn content_uses_the_selected_mods_anchor_bytes() {
    let stack = fixture("products_mod_anchor");
    stack.write(&stack.addon(), "map/fixture.txt", "9");
    let content = content(&stack);
    assert_eq!(
        content.impact_place(1),
        Some(([9., 2., 3.], [0., 0., 12., 6.]))
    );
    assert!(content.reads.entries().iter().any(
        |entry| matches!(entry, Entry::Read { layer: Some(layer), .. } if layer.id == "test1")
    ));
}

#[test]
fn ancillary_preserves_named_unavailable_not_recorded_and_malformed_inputs() {
    let stack = fixture("products_ancillary_status");
    let content = content(&stack);
    let mut frame = frame(2);
    frame.tables[TableId::BattleSounds.index()] = TableView::Unavailable {
        reason: "fixture battle coverage".into(),
    }
    .into();
    frame.tables[TableId::NuclearStrikes.index()] = TableView::NotRecorded {
        mode: replay_data::scope::CaptureMode::Map,
    }
    .into();
    frame.tables[TableId::MapEvents.index()] =
        TableView::Complete(TableRows::Countries(Vec::new())).into();
    let hour = ProductHour::new(&content, &frame, 0.);
    let old = Original {
        content: &content,
        frame: &frame,
        hour_fraction: 0.,
    };
    let battles = hour.battle_sounds().unwrap();
    assert_eq!(
        battles.refusals,
        ["the recorded BattleSounds table is unavailable (fixture battle coverage)"]
    );
    assert_eq!(Some(battles), old.battle_sounds());
    assert_eq!(
        hour.nuclear_impacts().unwrap().refused,
        ["NuclearStrikes not recorded in a map recording: impacts have no effect"]
    );
    let vision = old.ancillary_vision(&choice()).unwrap();
    let actual = format!("{:?}", hour.map_text(&vision));
    assert_eq!(actual, format!("{:?}", old.map_text(&vision)));
    assert!(actual.contains("map text table malformed:"));
}

#[test]
fn startup_notes_and_errors_are_canonical_deduplicated_and_change_the_witness() {
    let stack = fixture("products_startup_notes");
    let reads = ContentReads::new(&stack.layers(&[]));
    let before = reads.witness().unwrap();
    reads.session_note("fixture optional content is unavailable");
    reads.session_error(&io::Error::new(
        io::ErrorKind::InvalidData,
        "fixture parse failure",
    ));
    assert_ne!(before.digest, reads.witness().unwrap().digest);
    assert!(
        reads.entries().iter().any(|entry| matches!(entry, Entry::Result { purpose, status: 1, message, .. } if purpose == "session-error" && message == "fixture parse failure"))
    );
    assert!(
        reads.entries().iter().any(|entry| matches!(entry, Entry::Result { purpose, status: 0, message, .. } if purpose == "session-note" && message == "fixture optional content is unavailable"))
    );
    let other = ContentReads::new(&stack.layers(&[]));
    other.session_error(&io::Error::new(
        io::ErrorKind::InvalidData,
        "fixture parse failure",
    ));
    other.session_note("fixture optional content is unavailable");
    other.session_note("fixture optional content is unavailable");
    assert_eq!(reads.entries(), other.entries());
    assert_eq!(reads.witness().unwrap(), other.witness().unwrap());
    let changed = ContentReads::new(&stack.layers(&[]));
    changed.session_note("fixture optional content has a different refusal");
    changed.session_error(&io::Error::new(
        io::ErrorKind::InvalidData,
        "fixture parse failure",
    ));
    assert_ne!(reads.witness().unwrap(), changed.witness().unwrap());
}

fn original_weather_field(frame: &DecodedFrame) -> io::Result<map_display_data::WeatherField> {
    let mut out = map_display_data::WeatherField {
        branch: frame.branch,
        hour: frame.hour,
        ..Default::default()
    };
    for row in dashboard_text::table_access::rows::<row_tables::WeatherRegions>(frame)?.rows() {
        if let Some(kind) = [
            row.light_rain,
            row.heavy_rain,
            row.snow,
            row.blizzard,
            row.sandstorm,
        ]
        .iter()
        .position(|value| *value != 0)
        {
            out.kinds = out.kinds.inserted(row.id, kind).unwrap();
        }
    }
    Ok(out)
}

#[test]
fn closed_fog_matches_independent_builder_for_countries_switches_and_source_gaps() {
    let stack = fixture("products_closed_fog");
    let content = content(&stack);
    for count in [2, 1500, 5000] {
        for status in 0..4 {
            let mut frame = frame(count);
            frame.tables[TableId::UnitIntel.index()] = match status {
                0 => TableView::Complete(TableRows::UnitIntel(Vec::new())),
                1 => TableView::Truncated {
                    rows: TableRows::UnitIntel(Vec::new()),
                    missing: 9,
                },
                2 => TableView::Unavailable {
                    reason: "independent synthetic intel gap".into(),
                },
                _ => TableView::NotRecorded {
                    mode: replay_data::scope::CaptureMode::Map,
                },
            }
            .into();
            for slot in [1, count] {
                for fog in [false, true] {
                    let choice = perspective::Choice {
                        country: perspective::Country::Tag(format!("Z{slot}")),
                        fog,
                    };
                    let old = Original {
                        content: &content,
                        frame: &frame,
                        hour_fraction: 0.5,
                    };
                    let expected = old.fog_display(&choice).unwrap();
                    let memos = FogMemos::new();
                    let actual = ProductHour::new(&content, &frame, 0.5)
                        .fog(&choice, &memos, &mut BuilderSpans::new(false))
                        .unwrap();
                    let wire = product_codec::encode(&actual.display()).unwrap();
                    assert_eq!(wire, product_codec::encode(&expected).unwrap());
                    let restored =
                        FogProduct::from_display(product_codec::decode(&wire).unwrap()).unwrap();
                    assert_eq!(product_codec::encode(&restored.display()).unwrap(), wire);
                    assert_eq!(restored.limitations.lines(), actual.limitations.lines());
                    let (_, reads) = replay_frame::frame::view_reads::record(|| {
                        product_codec::encode(&restored.display()).unwrap()
                    });
                    assert!(reads.is_empty());
                }
            }
        }
    }
}

#[test]
fn exact_country_requests_do_not_alias_duplicate_tags_and_reject_absent_instances() {
    let stack = fixture("products_closed_country");
    let content = content(&stack);
    let mut frame = frame(1500);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.tag = b"SAME".to_vec();
        }
    }
    let choice = perspective::Choice {
        country: perspective::Country::Tag("SAME".into()),
        fog: false,
    };
    let hour = ProductHour::new(&content, &frame, 0.0);
    hour.seed_country(&choice, 1600, 1500).unwrap();
    assert_eq!(hour.vision(&choice).unwrap().ours(), 1500);
    assert!(hour.seed_country(&choice, 101, 1500).is_err());
    assert!(hour.seed_country(&choice, 1601, 1501).is_err());
}

#[test]
fn snow_product_is_owned_deduplicates_equal_outputs_and_keeps_refresh_identity() {
    let stack = fixture("products_closed_snow");
    let content = content(&stack);
    let cells = map_snow::SnowCells::new(&content.map, &[false; 72]).unwrap();
    let winter = map_snow::winter::Winter {
        defines: map_snow::winter::SnowDefines {
            per_province: [0.0, 1.0],
            total: [0.0, 1.0],
        },
        records: vec![vec![map_snow::winter::SnowRecord {
            province: 1,
            weight: 1.0,
        }]],
    };
    let memo = display::SnowMemo::new();
    let mut frame = frame(2);
    put(
        &mut frame,
        TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            id: 1,
            snow: 50_000,
            display_snow: 173,
            display_mud: 9,
            ..Default::default()
        }]),
    );
    let original = display_oracle::snow(&content.map, &[false; 72], &frame, &winter).unwrap();
    let first = ProductHour::new(&content, &frame, 0.0)
        .snow(&cells, &winter, &memo)
        .unwrap();
    assert_eq!(first.texels, original.texels);
    assert_eq!(first.province_snow, original.province_snow);
    assert_eq!(first.summary, original.summary);
    assert_eq!(
        product_codec::encode(&*first).unwrap(),
        product_codec::encode(&original).unwrap()
    );
    frame.hour += 1;
    frame.tables[TableId::WeatherProvinces.index()] = (*frame.tables
        [TableId::WeatherProvinces.index()])
    .clone()
    .into();
    let equal = ProductHour::new(&content, &frame, 0.0)
        .snow(&cells, &winter, &memo)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &equal));
    let bytes = product_codec::encode(&*first).unwrap();
    drop(frame);
    assert_eq!(
        product_codec::encode(&product_codec::decode::<map_display_data::Snow>(&bytes).unwrap())
            .unwrap(),
        bytes
    );
    assert!(first.province_snow[0].is_none());
}

#[test]
fn snow_wire_and_status_match_the_frozen_scalar_for_gaps_edges_and_refresh() {
    let stack = fixture("products_snow_oracle");
    let content = content(&stack);
    let mut perm = [false; 72];
    perm[0..4].fill(true);
    let cells = map_snow::SnowCells::new(&content.map, &perm).unwrap();
    let winter = map_snow::winter::Winter {
        defines: map_snow::winter::SnowDefines {
            per_province: [0.0, 1.0],
            total: [0.0, 1.0],
        },
        records: vec![
            vec![map_snow::winter::SnowRecord {
                province: 1,
                weight: 1.0,
            }],
            vec![map_snow::winter::SnowRecord {
                province: 0,
                weight: 1.0,
            }],
        ],
    };
    let mut refresh_bytes = None;
    for identity in [4, 5] {
        for status in 0..4 {
            let mut frame = frame(2);
            let rows = vec![
                WeatherProvinceRow {
                    id: 0,
                    display: Recorded::Value(Some(WeatherDisplay {
                        width: 3,
                        height: 1,
                        refresh_interval: 6,
                        refresh_identity: identity,
                        edges: vec![31, 32, 33, 91, 92, 93],
                    })),
                    ..Default::default()
                },
                WeatherProvinceRow {
                    id: 1,
                    snow: 50_000,
                    display_snow: 173,
                    display_mud: 9,
                    ..Default::default()
                },
            ];
            frame.tables[TableId::WeatherProvinces.index()] = match status {
                0 => TableView::Complete(TableRows::WeatherProvinces(rows)),
                1 => TableView::Truncated {
                    rows: TableRows::WeatherProvinces(rows),
                    missing: 2,
                },
                2 => TableView::Unavailable {
                    reason: "synthetic snow read failure".into(),
                },
                _ => TableView::NotRecorded {
                    mode: replay_data::scope::CaptureMode::Map,
                },
            }
            .into();
            let actual = ProductHour::new(&content, &frame, 0.0)
                .snow(&cells, &winter, &display::SnowMemo::new())
                .unwrap();
            let expected = display_oracle::snow(&content.map, &perm, &frame, &winter).unwrap();
            let wire = product_codec::encode(&*actual).unwrap();
            assert_eq!(wire, product_codec::encode(&expected).unwrap());
            assert_eq!(actual.summary.line(), expected.summary.line());
            assert_eq!(actual.refresh, expected.refresh);
            if status == 0 {
                if let Some((texels, old)) = &refresh_bytes {
                    assert_eq!(actual.texels, *texels);
                    assert_ne!(wire, *old);
                }
                refresh_bytes = Some((actual.texels.clone(), wire));
            }
        }
    }
}

#[test]
fn fog_products_and_memos_do_not_pin_their_source_tables() {
    let stack = fixture("products_no_source_pins");
    let content = content(&stack);
    let frame = frame(1500);
    let memos = FogMemos::new();
    let product = ProductHour::new(&content, &frame, 0.0)
        .fog(&choice(), &memos, &mut BuilderSpans::new(false))
        .unwrap();
    for table in frame.tables {
        assert!(
            table.try_unwrap().is_ok(),
            "product retained a source allocation"
        );
    }
    let bytes = product_codec::encode(&product.display()).unwrap();
    let restored = FogProduct::from_display(product_codec::decode(&bytes).unwrap()).unwrap();
    assert_eq!(product_codec::encode(&restored.display()).unwrap(), bytes);
    drop(memos);
}

#[test]
fn evidence_changes_without_inventing_a_value_or_a_new_country_lifetime() {
    let mut frame = frame(2);
    let table = TableId::WeatherProvinces;
    frame.tables[table.index()] =
        TableView::Complete(TableRows::WeatherProvinces(Vec::new())).into();
    let empty = display::evidence(&frame, &[table]);
    assert!(matches!(
        empty[0].status,
        map_display_data::Availability::KnownEmpty {}
    ));
    assert_eq!(empty[0].supplying_hour, None);
    frame.tables[table.index()] = frame.tables[table.index()]
        .clone()
        .with_sampled_at(Some(39));
    let read = display::evidence(&frame, &[table]);
    assert_ne!(empty, read);
    frame.tables[table.index()] = TableView::Unavailable {
        reason: "unique poison: not empty".into(),
    }
    .into();
    let failed = display::evidence(&frame, &[table]);
    assert!(
        matches!(&failed[0].status,map_display_data::Availability::Unavailable { reason,.. } if reason=="unique poison: not empty")
    );
    assert_ne!(failed, read);
}

#[test]
fn weather_projection_reads_only_the_recorded_map_scope() {
    use viewer_foundation::test_support::{
        assert_reads_recorded,
        complete_frame,
        map_scoped,
        put_sample_campaign,
    };
    let mut frame = complete_frame(11, |table| {
        replay_data::rows::sample_extended_table(table, 0)
    });
    put_sample_campaign(&mut frame);
    let frame = map_scoped(frame);
    assert_reads_recorded(&frame, &[], || {
        display::weather_field(&frame).unwrap();
    });
}

#[test]
fn direct_owned_vision_keeps_the_old_keyed_source_census_and_decisions() {
    let stack = fixture("products_vision_old_census");
    let content = content(&stack);
    let frame = frame(1500);
    let memo = viewer_runtime::memo::Keyed::new(perspective::VISION_TABLES);
    for fog in [false, true] {
        let choice = perspective::Choice {
            country: perspective::Country::Tag("Z1500".into()),
            fog,
        };
        let expected =
            perspective::cached_vision(&frame, &choice, FogContent::loaded(&content.fog), &memo)
                .unwrap();
        let actual = ProductHour::new(&content, &frame, 0.0)
            .vision(&choice)
            .unwrap();
        assert_eq!(*actual, *expected);
    }
}

#[test]
fn vision_core_refusals_keep_the_independent_country_maps_precedence() {
    let stack = fixture("vision_core_refusal_order");
    let content = content(&stack);
    for count in [3, 1500, 5000] {
        for truncated in [false, true] {
            let mut frame = frame(count);
            for table in [TableId::CountryLinks, TableId::Countries] {
                frame.tables[table.index()] = if truncated {
                    TableView::Truncated {
                        rows: TableRows::empty(table),
                        missing: 1,
                    }
                } else {
                    TableView::Unavailable {
                        reason: format!("missing {table:?}"),
                    }
                }
                .into();
            }
            let hour = ProductHour::new(&content, &frame, 0.0);
            let expected = hour.country_maps().unwrap_err();
            let actual = hour.vision(&choice()).unwrap_err();
            assert_eq!(actual.kind(), expected.kind());
            assert_eq!(actual.to_string(), expected.to_string());
            assert!(actual.to_string().contains("CountryLinks"));
        }
    }
}

fn fog_content(stack: &Stack) -> ProductContent {
    use viewer_foundation::visibility::{Defines, Topology};
    let ProductContent {
        reads, mut inputs, ..
    } = content(stack);
    inputs.map = Arc::new(MapData {
        width: 18,
        height: 6,
        province_ids: (0..108).map(|at| (at % 18 / 6 + 1) as u32).collect(),
        definitions: (1..=3)
            .map(|id| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind: ProvinceKind::Land,
                        continent: 1,
                    },
                )
            })
            .collect(),
        country_colors: Default::default(),
    });
    inputs.fog = Arc::new(Ok(FogContent {
        topology: Topology {
            land: [1, 2, 3].into(),
            sea: Default::default(),
            neighbors: [(1, vec![2]), (2, vec![1]), (3, vec![])].into(),
        },
        defines: Defines {
            intel_maximums: [100_000; 4],
            radar_effect: 25,
            map_cutoffs: vec![50_000, 0, 100_000, 2_500_000],
            air_icon_threshold: 30_000,
            air_detail_thresholds: [0, 30_000, 70_000, 100_000],
            air_plane_fuzzy_range: 100_000,
            naval_detail_thresholds: [0, 30_000, 50_000, 70_000, 100_000],
        },
    }));
    ProductContent::from_inputs(reads, inputs)
}

fn fog_frame(count: u32) -> DecodedFrame {
    let mut frame = frame(count);
    put(
        &mut frame,
        TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: 101,
            observer_country: 0,
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    controller: id + 100,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: 7,
            roster_owner: 103,
            logical_owner: 103,
            province: 3,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::UnitIntel(vec![
            UnitIntelRow {
                unit: 8,
                viewer: 1,
                ordinal: 0,
                level: 1,
            },
            UnitIntelRow {
                unit: 8,
                viewer: 1,
                ordinal: 1,
                level: 2,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::CountryIntel(vec![CountryIntelRow {
            owner_country: 101,
            target_index: 2,
            army: 75_000,
            ..Default::default()
        }]),
    );
    frame
}

fn assert_frozen_fog(
    content: &ProductContent,
    frame: &DecodedFrame,
    choice: &perspective::Choice,
) -> map_display_data::Fog {
    let old = Original {
        content,
        frame,
        hour_fraction: 0.5,
    };
    let expected = old.fog_display(choice).unwrap();
    let reference = old.fog_vision(choice).unwrap();
    let hour = ProductHour::new(content, frame, 0.5);
    let actual = hour
        .fog(choice, &FogMemos::new(), &mut BuilderSpans::new(false))
        .unwrap();
    assert_eq!(
        product_codec::encode(&actual.display()).unwrap(),
        product_codec::encode(&expected).unwrap()
    );
    let vision = hour.vision(choice).unwrap();
    assert_eq!(
        (vision.ours(), vision.resolution(), vision.fog()),
        (reference.ours(), reference.resolution(), reference.fog())
    );
    assert_eq!(vision.province_fog(), reference.province_fog());
    assert_eq!(vision.undecided(), reference.undecided());
    assert_eq!(vision.visibility_gap(), reference.visibility_gap());
    for province in 1..=3 {
        assert_eq!(
            vision.province_byte(province),
            reference.province_byte(province)
        );
    }
    for owner in 1..=3 {
        assert_eq!(
            vision.shares_vision_proven(owner),
            reference.shares_vision_proven(owner)
        );
        for unit in [7, 8] {
            assert_eq!(
                vision.sees_land(unit, owner, 3),
                reference.sees_land(unit, owner, 3)
            );
            assert_eq!(
                vision.land_byte_known(unit, owner, 3),
                reference.land_byte_known(unit, owner, 3)
            );
        }
    }
    expected
}

#[test]
fn frozen_fog_covers_nonclear_perspectives_intel_and_directed_sharing() {
    let stack = fixture("products_real_fog");
    let content = fog_content(&stack);
    for count in [3, 1500, 5000] {
        for sharing in 0..4 {
            let mut frame = fog_frame(count);
            if sharing == 1 || sharing == 3 {
                put(
                    &mut frame,
                    TableRows::FactionMembers(
                        [101, 103]
                            .into_iter()
                            .enumerate()
                            .map(|(ordinal, country)| FactionMemberRow {
                                faction: 9,
                                country,
                                ordinal: ordinal as u32,
                            })
                            .collect(),
                    ),
                );
            }
            if sharing == 2 {
                put(
                    &mut frame,
                    TableRows::RelationLinks(vec![RelationLinkRow {
                        country: 103,
                        target_index: 1,
                        links: RelationLinkRow::ATTACHE,
                        ..Default::default()
                    }]),
                );
            }
            if sharing == 3 {
                put(
                    &mut frame,
                    TableRows::WarRelations(vec![WarRelationRow {
                        first_country: 101,
                        second_country: 103,
                        ..Default::default()
                    }]),
                );
            }
            let recorded = perspective::Choice::default();
            let third = perspective::Choice {
                country: perspective::Country::Tag("Z3".into()),
                fog: true,
            };
            let old = Original {
                content: &content,
                frame: &frame,
                hour_fraction: 0.5,
            };
            let first = old.fog_display(&recorded).unwrap();
            let other = old.fog_display(&third).unwrap();
            assert!(first.intel.fog && other.intel.fog);
            assert!(first.intel.texels.iter().any(|byte| *byte > 0));
            assert!(other.intel.texels.iter().any(|byte| *byte > 0));
            assert_eq!(
                first.intel.texels,
                if sharing == 0 {
                    vec![0, 80, 255]
                } else {
                    vec![0, 80, 0]
                }
            );
            if sharing == 0 || sharing == 2 {
                assert_eq!(other.intel.texels, [255, 255, 0]);
                assert_ne!(first.intel.texels, other.intel.texels);
            }
            let first_vision = old.fog_vision(&recorded).unwrap();
            assert_eq!(first_vision.shares_vision_proven(3), Ok(sharing == 1));
            assert_eq!(
                first_vision.land_byte_known(7, 3, 3),
                Some(if sharing == 0 { 0 } else { 255 })
            );
            assert_eq!(
                first_vision.sees_land(7, 3, 3),
                if sharing == 0 {
                    perspective::Sight::None
                } else {
                    perspective::Sight::Full
                }
            );
            if sharing == 0 {
                assert_eq!(first_vision.land_byte_known(8, 3, 3), Some(150));
                assert_eq!(first_vision.sees_land(8, 3, 3), perspective::Sight::Partial);
            }
            for country in [
                perspective::Country::Recorded,
                perspective::Country::Tag("z2".into()),
                third.country.clone(),
                perspective::Country::Tag("ABSENT".into()),
            ] {
                for fog in [true, false] {
                    assert_frozen_fog(
                        &content,
                        &frame,
                        &perspective::Choice {
                            country: country.clone(),
                            fog,
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn frozen_fog_keeps_hidden_intel_distinct_from_missing_and_truncated_sources() {
    let stack = fixture("products_real_fog_gaps");
    let content = fog_content(&stack);
    let choice = perspective::Choice::default();
    let whole = fog_frame(3);
    let hidden = assert_frozen_fog(&content, &whole, &choice);
    let reference = Original {
        content: &content,
        frame: &whole,
        hour_fraction: 0.5,
    }
    .fog_vision(&choice)
    .unwrap();
    assert_eq!(reference.land_byte_known(7, 3, 3), Some(0));
    assert_eq!(reference.sees_land(7, 3, 3), perspective::Sight::None);
    for table in [
        TableId::UnitIntel,
        TableId::CountryVisibility,
        TableId::CountryIntel,
        TableId::StateVisibility,
        TableId::FactionMembers,
    ] {
        for status in 0..3 {
            let mut frame = fog_frame(3);
            let TableView::Complete(rows) = &*frame.tables[table.index()] else {
                unreachable!()
            };
            frame.tables[table.index()] = match status {
                0 => TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                },
                1 => TableView::Unavailable {
                    reason: format!("synthetic {table:?} gap"),
                },
                _ => TableView::NotRecorded {
                    mode: replay_data::scope::CaptureMode::Map,
                },
            }
            .into();
            let old = Original {
                content: &content,
                frame: &frame,
                hour_fraction: 0.5,
            };
            let reference = old.fog_vision(&choice).unwrap();
            let expected = assert_frozen_fog(&content, &frame, &choice);
            assert_ne!(
                product_codec::encode(&expected).unwrap(),
                product_codec::encode(&hidden).unwrap()
            );
            if !reference.fog() {
                assert_eq!(reference.land_byte_known(7, 3, 3), Some(255));
                assert_eq!(reference.sees_land(7, 3, 3), perspective::Sight::Full);
                assert!(!expected.intel.fog);
            } else if table == TableId::UnitIntel && status == 0 {
                assert_eq!(reference.land_byte_known(7, 3, 3), None);
                assert_eq!(reference.land_byte_known(8, 3, 3), Some(150));
                assert_eq!(reference.sees_land(7, 3, 3), perspective::Sight::Full);
                assert_eq!(expected.intel, hidden.intel);
            } else {
                assert_eq!(reference.land_byte_known(7, 3, 3), None);
                assert_eq!(reference.sees_land(7, 3, 3), perspective::Sight::Full);
                if table == TableId::CountryVisibility {
                    assert_eq!(expected.intel, hidden.intel);
                } else if table != TableId::UnitIntel {
                    assert!(reference.visibility_gap().is_some());
                    assert!(!expected.intel.fog);
                    assert!(expected.intel.texels.iter().all(|byte| *byte == 0));
                }
            }
            if table == TableId::FactionMembers {
                assert!(matches!(
                    reference.shares_vision_proven(3),
                    Err(perspective::SharingUnknown::Relations("FactionMembers"))
                ));
            }
        }
    }
}

#[test]
fn snow_interior_matches_frozen_scalar_with_mixed_provinces_and_permanent_snow() {
    let stack = fixture("products_snow_interior");
    let ProductContent {
        reads, mut inputs, ..
    } = content(&stack);
    inputs.map = Arc::new(MapData {
        width: 16,
        height: 16,
        province_ids: (0..256).map(|at| if at % 4 < 2 { 1 } else { 2 }).collect(),
        definitions: (1..=2)
            .map(|id| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind: ProvinceKind::Land,
                        continent: 1,
                    },
                )
            })
            .collect(),
        country_colors: Default::default(),
    });
    let content = ProductContent::from_inputs(reads, inputs);
    let winter = map_snow::winter::Winter {
        defines: map_snow::winter::SnowDefines {
            per_province: [0.0, 1.0],
            total: [0.0, 1.0],
        },
        records: vec![],
    };
    let mut frame = frame(2);
    put(
        &mut frame,
        TableRows::WeatherProvinces(vec![
            WeatherProvinceRow {
                id: 1,
                snow: 20_000,
                display_snow: 64,
                display_mud: 16,
                ..Default::default()
            },
            WeatherProvinceRow {
                id: 2,
                snow: 80_000,
                display_snow: 192,
                display_mud: 80,
                ..Default::default()
            },
        ]),
    );
    let interior = (4 + 1) * 4;
    let mut previous = None;
    for permanent in [false, true] {
        let mut mask = vec![false; 256];
        mask[(16 - 1 - 4) * 16 + 4] = permanent;
        let expected = display_oracle::snow(&content.map, &mask, &frame, &winter).unwrap();
        assert_eq!([expected.width, expected.height], [4, 4]);
        assert_eq!(
            &expected.texels[interior..interior + 4],
            if permanent {
                &[48, 139, 139, 48]
            } else {
                &[48, 128, 128, 48]
            }
        );
        if let Some(previous) = &previous {
            assert_ne!(previous, &expected.texels);
        }
        previous = Some(expected.texels.clone());
        let cells = map_snow::SnowCells::new(&content.map, &mask).unwrap();
        let actual = ProductHour::new(&content, &frame, 0.0)
            .snow(&cells, &winter, &display::SnowMemo::new())
            .unwrap();
        assert_eq!(
            product_codec::encode(&*actual).unwrap(),
            product_codec::encode(&expected).unwrap()
        );
    }
}

#[test]
fn territory_labels_and_shared_rail_graph_release_the_borrowed_host() {
    use viewer_rail::railways::RailNetwork;
    let stack = fixture("landscape_closed_products");
    let content = content(&stack);
    let mut frame = frame(1501);
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            state: 1,
            owner: 101,
            controller: 102,
            victory_points: 10,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::RailEdges(vec![
            RailEdgeRow {
                from: 1,
                to: 2,
                level: 3,
            },
            RailEdgeRow {
                from: 1,
                to: 2,
                level: 1,
            },
            RailEdgeRow {
                from: 2,
                to: 1,
                level: 3,
            },
        ]),
    );
    let (territory, labels, graph) = {
        let hour = ProductHour::new(&content, &frame, 0.0);
        let territory = hour.territory_projection().unwrap();
        let labels = hour.labels(1).unwrap();
        let network = hour.rail_graph().unwrap();
        let projection = hour.rail_projection().unwrap();
        assert!(Arc::ptr_eq(&network.shared_display(), &projection.graph));
        assert_eq!(territory.facts.countries.len(), 1501);
        assert_eq!(territory.facts.provinces[0].owner_slot, Some(1));
        assert_eq!(territory.facts.provinces[0].controller_slot, Some(2));
        assert_eq!(labels.candidates.len(), 1);
        assert_eq!(network.display().graph[&1][&2], 1);
        assert_eq!(network.display(), projection.graph.as_ref());
        let decoded = product_codec::decode::<map_display_data::landscape::RailGraph>(
            &product_codec::encode(projection.graph.as_ref()).unwrap(),
        )
        .unwrap();
        assert_eq!(RailNetwork::from_display(decoded), *network);
        (territory, labels, projection)
    };
    frame.hour += 1;
    {
        let hour = ProductHour::new(&content, &frame, 0.0);
        let next = hour.territory().unwrap();
        let mut observed = (*territory.facts).clone();
        observed.countries.observed_hour = frame.hour;
        assert_eq!(*next, observed);
        let (before, after) = (&territory.facts.paint, &next.paint);
        assert!(Arc::ptr_eq(&before.country_of.0, &after.country_of.0));
        assert!(Arc::ptr_eq(&before.owner_of.0, &after.owner_of.0));
        assert!(Arc::ptr_eq(&before.state_of.0, &after.state_of.0));
        assert!(Arc::ptr_eq(
            &labels.candidates,
            &hour.labels(2).unwrap().candidates
        ));
        assert!(Arc::ptr_eq(
            &graph.graph,
            &hour.rail_graph().unwrap().shared_display()
        ));
    }
    for table in frame.tables {
        assert!(
            table.try_unwrap().is_ok(),
            "a migrated product pinned source data"
        );
    }
    assert_eq!(territory.facts.provinces[0].owner_raw, 101);
    assert_eq!(labels.candidates[0].controller, Some(2));
    assert_eq!(graph.graph.graph[&1][&2], 1);
}

#[test]
fn a_rail_source_gap_is_not_certified_as_known_empty() {
    let stack = fixture("landscape_rail_gap");
    let content = content(&stack);
    let mut frame = frame(1);
    let complete = ProductHour::new(&content, &frame, 0.0)
        .rail_projection()
        .unwrap();
    assert!(complete.graph.graph.is_empty());
    frame.tables[TableId::RailEdges.index()] = TableView::Truncated {
        rows: TableRows::RailEdges(Vec::new()),
        missing: 2,
    }
    .into();
    let missing = ProductHour::new(&content, &frame, 0.0)
        .rail_projection()
        .unwrap();
    assert_eq!(complete.graph, missing.graph);
    assert_ne!(complete.evidence, missing.evidence);
    assert!(missing.evidence.iter().any(|evidence| {
        matches!(
            evidence.status,
            map_display_data::Availability::Unknown { .. }
        )
    }));
}

#[test]
fn rail_graph_ignores_unrelated_controllers_but_preserves_changed_source_evidence() {
    let stack = fixture("landscape_rail_projection_changes");
    let content = content(&stack);
    let mut frame = frame(1);
    put(
        &mut frame,
        TableRows::ProvinceSupply(vec![ProvinceSupplyRow {
            province: 7,
            ..Default::default()
        }]),
    );
    let first = ProductHour::new(&content, &frame, 0.0)
        .rail_graph()
        .unwrap();
    frame.hour += 1;
    put(
        &mut frame,
        TableRows::ProvinceSupply(vec![ProvinceSupplyRow {
            province: 7,
            controller_index: 3,
            ..Default::default()
        }]),
    );
    let hour = ProductHour::new(&content, &frame, 0.0);
    let same = hour.rail_graph().unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    drop(hour);
    frame.hour += 1;
    put(
        &mut frame,
        TableRows::ProvinceSupply(vec![ProvinceSupplyRow {
            province: 7,
            naval_base_level: 1,
            ..Default::default()
        }]),
    );
    let changed = ProductHour::new(&content, &frame, 0.0)
        .rail_graph()
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.sources, std::collections::BTreeSet::from([7]));
}

#[test]
fn display_dependent_witnesses_split_late_positive_negative_and_error_reads() {
    let stack = fixture("display_dependent_shards");
    let layers = stack.layers(&[]);
    let reads = ContentReads::new(&layers);
    let modes = reads.reader(&layers, "display/modes");
    let buildings = reads.reader(&layers, "display/buildings");
    let mode_before = reads
        .dependent_binding("modes", &["display/modes"])
        .unwrap();
    let building_before = reads
        .dependent_binding("buildings", &["display/buildings"])
        .unwrap();
    assert_ne!(
        reads.dependent_binding("territory", &[]).unwrap(),
        reads.dependent_binding("modes", &[]).unwrap()
    );
    modes.read("map/fixture.txt").unwrap();
    let positive = reads
        .dependent_binding("modes", &["display/modes"])
        .unwrap();
    assert_ne!(positive, mode_before);
    assert_eq!(
        building_before,
        reads
            .dependent_binding("buildings", &["display/buildings"])
            .unwrap()
    );
    assert!(!modes.has_file("map/missing-mode.txt").unwrap());
    let negative = reads
        .dependent_binding("modes", &["display/modes"])
        .unwrap();
    assert_ne!(negative, positive);
    assert!(modes.read("map/missing-mode.txt").is_err());
    assert_ne!(
        negative,
        reads
            .dependent_binding("modes", &["display/modes"])
            .unwrap()
    );
    buildings.read("map/fixture.txt").unwrap();
    assert_ne!(
        building_before,
        reads
            .dependent_binding("buildings", &["display/buildings"])
            .unwrap()
    );
    let mut reordered = layers;
    reordered.layers.reverse();
    assert_ne!(
        reads.dependent_binding("territory", &[]).unwrap(),
        ContentReads::new(&reordered)
            .dependent_binding("territory", &[])
            .unwrap()
    );
}

#[test]
fn separate_product_jobs_share_vision_without_retaining_source_tables() {
    let stack = fixture("products_shared_source_free_vision");
    let content = content(&stack);
    let mut frame = frame(3);
    let roots: Vec<_> = frame.tables.iter().map(|view| view.identity()).collect();
    let first = ProductHour::new(&content, &frame, 0.0)
        .vision(&choice())
        .unwrap();
    let second = ProductHour::new(&content, &frame, 0.5)
        .vision(&choice())
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(roots.iter().all(|identity| identity.owner_count() == 1));
    let next_choice = perspective::Choice {
        country: perspective::Country::Tag("Z3".into()),
        ..choice()
    };
    let other = ProductHour::new(&content, &frame, 0.0)
        .vision(&next_choice)
        .unwrap();
    assert_eq!(other.ours(), 3);
    assert!(!Arc::ptr_eq(&first, &other));
    let again = ProductHour::new(&content, &frame, 0.0)
        .vision(&choice())
        .unwrap();
    assert_eq!(*first, *again);
    frame.hour += 1;
    let next = ProductHour::new(&content, &frame, 0.0)
        .vision(&choice())
        .unwrap();
    assert!(Arc::ptr_eq(&again, &next));
    put(&mut frame, TableRows::Campaign(vec![]));
    let replaced = ProductHour::new(&content, &frame, 0.0)
        .vision(&choice())
        .unwrap();
    assert!(Arc::ptr_eq(&next, &replaced));
    drop(frame);
    assert!(roots.iter().all(|identity| !identity.is_alive()));
    assert_eq!(first.ours(), 1);
    assert_eq!(replaced.ours(), 1);
}

#[test]
fn shared_vision_keeps_the_enforced_census_and_rejects_changed_same_hour_inputs() {
    let stack = fixture("products_vision_shared_census");
    let content = fog_content(&stack);
    let mut frame = fog_frame(1500);
    let choice = perspective::Choice::default();
    let memo = viewer_runtime::memo::Keyed::new(perspective::VISION_TABLES);
    let expected =
        perspective::cached_vision(&frame, &choice, FogContent::loaded(&content.fog), &memo)
            .unwrap();
    let (actual, reads) = replay_frame::frame::view_reads::record(|| {
        ProductHour::new(&content, &frame, 0.0)
            .vision(&choice)
            .unwrap()
    });
    assert_eq!(*actual, *expected);
    assert!(reads.iter().all(|identity| {
        perspective::VISION_TABLES
            .iter()
            .any(|table| identity.is(&frame.tables[table.index()]))
    }));
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 1,
            controller: 103,
            ..Default::default()
        }]),
    );
    let replaced = ProductHour::new(&content, &frame, 0.0)
        .vision(&choice)
        .unwrap();
    let expected =
        perspective::cached_vision(&frame, &choice, FogContent::loaded(&content.fog), &memo)
            .unwrap();
    assert!(!Arc::ptr_eq(&actual, &replaced));
    assert_eq!(*replaced, *expected);
}

#[test]
fn streaming_owned_content_witness_matches_buffered_bytes_and_refuses_partial_writes() {
    let stack = fixture("streamed_owned_content_witness");
    let layers = stack.layers(&[]);
    let buffered = crate::witness::ContentReads::new(&layers);
    let streamed = crate::witness::ContentReads::new(&layers);
    let bytes: Vec<_> = (0..70_001).map(|n| (n % 251) as u8).collect();
    buffered.operand_bytes(b"synthetic-owned-content", &bytes);
    streamed
        .operand_stream(b"synthetic-owned-content", |out| {
            for piece in bytes.chunks(7) {
                out.write_all(piece)?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(buffered.entries(), streamed.entries());
    let before = streamed.entries();
    let error = streamed
        .operand_stream(b"partial", |out| {
            out.write_all(b"not a complete operand")?;
            Err(std::io::Error::from(std::io::ErrorKind::Interrupted))
        })
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(before, streamed.entries());
}

#[test]
fn display_weather_deltas_skip_unused_fields_and_preserve_priority_and_clears() {
    use replay_frame::frame::TableUpdate;
    let mut advance_table = display_decoder();
    let content = content(&fixture("weather_delta_fold"));
    let mut frame = frame(2);
    let table = TableId::WeatherRegions;
    let mut previous = WeatherRegionRow {
        id: 7,
        light_rain: 1,
        ..Default::default()
    };
    let mut view = advance_table(
        table,
        &TableUpdate::Snapshot(TableRows::WeatherRegions(vec![previous.clone()])),
        None,
        40,
        true,
    )
    .unwrap();
    let mut prior: Option<map_display_data::WeatherField> = None;
    for step in 0..9 {
        if step != 0 {
            let mut next = previous.clone();
            match step {
                1 => next.max_snow = 11,
                2 => next.heavy_rain = 1,
                3 => next.light_rain = 0,
                4 => next.heavy_rain = 0,
                5 => {
                    next.snow = 1;
                    next.blizzard = 1;
                }
                6 => next.snow = 0,
                7 => next.max_snow = 22,
                _ => next.blizzard = 0,
            }
            view = advance_table(
                table,
                &TableUpdate::Delta {
                    baseline: 39 + step,
                    changed: vec![next.delta(Some(&previous)).unwrap().unwrap()],
                    deleted: vec![],
                },
                Some((39 + step, view)),
                40 + step,
                true,
            )
            .unwrap();
            previous = next;
        }
        frame.hour = 40 + step;
        frame.tables[table.index()] = view.clone();
        let actual = ProductHour::new(&content, &frame, 0.0)
            .weather_field()
            .unwrap();
        assert_eq!(
            product_codec::encode(&actual).unwrap(),
            product_codec::encode(&original_weather_field(&frame).unwrap()).unwrap()
        );
        if matches!(step, 1 | 2 | 7) {
            assert!(actual.kinds.ptr_eq(&prior.as_ref().unwrap().kinds));
        }
        prior = Some(actual);
    }
    let value = ProductHour::new(&content, &frame, 0.0)
        .weather_field()
        .unwrap();
    assert!(value.kinds.is_empty());
    let old = frame.tables[table.index()].identity();
    frame.tables[table.index()] = TableView::Unavailable {
        reason: "synthetic weather gap".into(),
    }
    .into();
    let missing = ProductHour::new(&content, &frame, 0.0)
        .weather_field()
        .unwrap();
    assert_eq!(missing, original_weather_field(&frame).unwrap());
    assert!(!old.is(&frame.tables[table.index()]));
}

#[test]
fn snow_change_fold_matches_frozen_scalar_through_edges_missing_and_recovery() {
    use replay_frame::frame::TableUpdate;
    let mut advance_table = display_decoder();
    let map = MapData {
        width: 16,
        height: 16,
        province_ids: (0..256).map(|at| if at % 4 < 2 { 1 } else { 2 }).collect(),
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let mut permanent = vec![false; 256];
    permanent[117] = true;
    let cells = map_snow::SnowCells::new(&map, &permanent).unwrap();
    let winter = map_snow::winter::Winter {
        defines: map_snow::winter::SnowDefines {
            per_province: [0.0, 1.0],
            total: [0.0, 1.0],
        },
        records: vec![vec![
            map_snow::winter::SnowRecord {
                province: 1,
                weight: 0.3,
            },
            map_snow::winter::SnowRecord {
                province: 2,
                weight: 0.7,
            },
        ]],
    };
    let mut fold = map_snow::fold::Fold::default();
    let mut frame = frame(2);
    let mut rows = vec![
        WeatherProvinceRow {
            id: 1,
            snow: 20_000,
            display_snow: 64,
            display_mud: 16,
            ..Default::default()
        },
        WeatherProvinceRow {
            id: 2,
            snow: 80_000,
            display_snow: 192,
            display_mud: 80,
            ..Default::default()
        },
    ];
    let table = TableId::WeatherProvinces;
    let mut view = advance_table(
        table,
        &TableUpdate::Snapshot(TableRows::WeatherProvinces(rows.clone())),
        None,
        40,
        true,
    )
    .unwrap();
    for step in 0..8 {
        if step != 0 {
            let mut next = rows.clone();
            match step {
                1 => next[0].snow = -100,
                2 => next[0].display_snow = 90,
                3 => next[1].display_mud = 170,
                4 => {
                    next[0].display_snow = 0;
                    next[0].display_mud = 0;
                }
                5 => {
                    next.remove(1);
                }
                6 => next.clear(),
                _ => next.push(WeatherProvinceRow {
                    id: 2,
                    display_snow: 17,
                    ..Default::default()
                }),
            };
            let changed = next
                .iter()
                .filter_map(|row| row.delta(rows.iter().find(|old| old.id == row.id)).unwrap())
                .collect();
            let deleted = rows
                .iter()
                .filter(|row| !next.iter().any(|next| next.id == row.id))
                .map(|row| u128::from(row.id))
                .collect();
            view = advance_table(
                table,
                &TableUpdate::Delta {
                    baseline: 39 + step,
                    changed,
                    deleted,
                },
                Some((39 + step, view)),
                40 + step,
                true,
            )
            .unwrap();
            rows = next;
        }
        frame.hour = 40 + step;
        frame.tables[table.index()] = view.clone();
        let actual = fold.project(&cells, &frame, &winter).unwrap();
        let expected = display_oracle::snow(&map, &permanent, &frame, &winter).unwrap();
        assert_eq!(
            product_codec::encode(&*actual).unwrap(),
            product_codec::encode(&expected).unwrap(),
            "step {step}"
        );
    }
    for view in [
        TableView::Truncated {
            rows: TableRows::WeatherProvinces(rows.clone()),
            missing: 3,
        },
        TableView::Unavailable {
            reason: "synthetic snow gap".into(),
        },
        TableView::NotRecorded {
            mode: replay_data::scope::CaptureMode::Map,
        },
        TableView::Complete(TableRows::WeatherProvinces(rows)),
    ] {
        frame.hour += 1;
        frame.tables[table.index()] = view.into();
        let actual = fold.project(&cells, &frame, &winter).unwrap();
        assert_eq!(
            product_codec::encode(&*actual).unwrap(),
            product_codec::encode(
                &display_oracle::snow(&map, &permanent, &frame, &winter).unwrap()
            )
            .unwrap()
        );
    }
    frame.tables[table.index()] = TableView::Complete(TableRows::Provinces(vec![])).into();
    assert!(fold.project(&cells, &frame, &winter).is_err());
    frame.tables[table.index()] = TableView::Complete(TableRows::WeatherProvinces(vec![])).into();
    assert_eq!(
        *fold.project(&cells, &frame, &winter).unwrap(),
        display_oracle::snow(&map, &permanent, &frame, &winter).unwrap()
    );
}

#[test]
fn snow_boundary_deltas_keep_refresh_only_changes_and_recover_after_refusal() {
    use replay_frame::frame::TableUpdate;
    let mut advance_table = display_decoder();
    let map = MapData {
        width: 16,
        height: 16,
        province_ids: vec![1; 256],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let permanent = vec![false; 256];
    let cells = map_snow::SnowCells::new(&map, &permanent).unwrap();
    let winter = map_snow::winter::Winter {
        defines: map_snow::winter::SnowDefines {
            per_province: [0.0, 1.0],
            total: [0.0, 1.0],
        },
        records: vec![],
    };
    let mut fold = map_snow::fold::Fold::default();
    let mut frame = frame(2);
    let mut row = WeatherProvinceRow {
        id: 0,
        display: Recorded::Value(Some(WeatherDisplay {
            width: 4,
            height: 4,
            refresh_interval: 25_000,
            refresh_identity: 91,
            edges: (0..14).map(|value| value * 11).collect(),
        })),
        ..Default::default()
    };
    let table = TableId::WeatherProvinces;
    let mut view = advance_table(
        table,
        &TableUpdate::Snapshot(TableRows::WeatherProvinces(vec![row.clone()])),
        None,
        60,
        true,
    )
    .unwrap();
    let mut previous: Option<Arc<map_display_data::SnowMud>> = None;
    for step in 0..5 {
        if step != 0 {
            let mut next = row.clone();
            match step {
                1 => {
                    if let Recorded::Value(Some(display)) = &mut next.display {
                        display.refresh_identity += 1;
                    }
                }
                2 => {
                    if let Recorded::Value(Some(display)) = &mut next.display {
                        display.edges[3] = 0;
                    }
                }
                3 => next.display = Recorded::Value(None),
                _ => next.display = Recorded::NotRecorded,
            }
            let update = if matches!(next.display, Recorded::NotRecorded) {
                assert!(next.delta(Some(&row)).is_err());
                TableUpdate::Snapshot(TableRows::WeatherProvinces(vec![next.clone()]))
            } else {
                TableUpdate::Delta {
                    baseline: 59 + step,
                    changed: vec![next.delta(Some(&row)).unwrap().unwrap()],
                    deleted: vec![],
                }
            };
            view = advance_table(table, &update, Some((59 + step, view)), 60 + step, true).unwrap();
            row = next;
        }
        frame.hour = 60 + step;
        frame.tables[table.index()] = view.clone();
        let actual = fold.project(&cells, &frame, &winter).unwrap();
        let expected = display_oracle::snow(&map, &permanent, &frame, &winter).unwrap();
        assert_eq!(
            product_codec::encode(&*actual).unwrap(),
            product_codec::encode(&expected).unwrap()
        );
        if step == 1 {
            assert!(actual.texels.ptr_eq(&previous.as_ref().unwrap().texels));
            assert_ne!(actual.refresh, previous.as_ref().unwrap().refresh);
        }
        previous = Some(actual);
    }
    row.display = Recorded::Value(Some(WeatherDisplay {
        width: 9,
        ..Default::default()
    }));
    frame.tables[table.index()] =
        TableView::Complete(TableRows::WeatherProvinces(vec![row.clone()])).into();
    assert!(fold.project(&cells, &frame, &winter).is_err());
    row.display = Recorded::Value(None);
    frame.tables[table.index()] =
        TableView::Complete(TableRows::WeatherProvinces(vec![row])).into();
    assert_eq!(
        *fold.project(&cells, &frame, &winter).unwrap(),
        display_oracle::snow(&map, &permanent, &frame, &winter).unwrap()
    );
}

#[test]
fn the_intel_map_follows_perspective_and_visibility_changes_against_frozen_output() {
    let stack = fixture("products_fog_delta_install");
    let content = fog_content(&stack);
    let memos = FogMemos::new();
    let mut frame = fog_frame(3);
    let mut previous: Option<Arc<IntelMap>> = None;
    for step in 0..8 {
        if step == 5 {
            put(
                &mut frame,
                TableRows::CountryIntel(vec![CountryIntelRow {
                    owner_country: 101,
                    target_index: 2,
                    army: 0,
                    ..Default::default()
                }]),
            );
        }
        if step == 6 {
            put(
                &mut frame,
                TableRows::Provinces(
                    (1..=3)
                        .map(|id| ProvinceRow {
                            id,
                            controller: if id == 2 { 101 } else { id + 100 },
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
        }
        if step == 7 {
            frame.tables[TableId::StateVisibility.index()] = TableView::Unavailable {
                reason: "synthetic fog gap".into(),
            }
            .into();
        }
        let choice = perspective::Choice {
            country: perspective::Country::Tag(if step == 1 { "Z3" } else { "Z1" }.into()),
            fog: step != 2,
        };
        let expected = Original {
            content: &content,
            frame: &frame,
            hour_fraction: 0.5,
        }
        .fog_display(&choice)
        .unwrap();
        let actual = ProductHour::new(&content, &frame, 0.5)
            .fog(&choice, &memos, &mut BuilderSpans::new(false))
            .unwrap();
        assert_eq!(
            product_codec::encode(&actual.display()).unwrap(),
            product_codec::encode(&expected).unwrap(),
            "step {step}"
        );
        if step == 4 {
            assert!(Arc::ptr_eq(&actual.intel, previous.as_ref().unwrap()));
        }
        previous = Some(Arc::clone(&actual.intel));
        frame.hour += 1;
    }
}

fn display_decoder() -> impl FnMut(
    TableId,
    &replay_frame::frame::TableUpdate,
    Option<(i32, replay_frame::frame::SharedView)>,
    i32,
    bool,
) -> io::Result<replay_frame::frame::SharedView> {
    use replay_frame::frame::{Frame, FrameDecoder, TableUpdate};
    let mut decoder = FrameDecoder::new().with_change_facts();
    move |table, update, _previous, hour, read| {
        let mut input = Frame {
            branch: 7,
            hour,
            keyframe: decoder.current().is_none(),
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableUpdate::Unavailable {
                reason: "not a fixture operand".into(),
            }),
        };
        input.tables[table.index()] = update.clone();
        input.metrics.table_ns[table.index()] = u64::from(read);
        Ok(decoder.advance(&input)?.tables[table.index()].clone())
    }
}

struct CertifiedFrames {
    encoder: replay_frame::frame::FrameEncoder,
    decoder: replay_frame::frame::FrameDecoder,
}
impl Default for CertifiedFrames {
    fn default() -> Self {
        Self {
            encoder: replay_frame::frame::FrameEncoder::new(),
            decoder: replay_frame::frame::FrameDecoder::new().with_change_facts(),
        }
    }
}
impl CertifiedFrames {
    fn advance(&mut self, frame: &mut DecodedFrame) {
        use replay_frame::frame::{SharedView, TableUpdate};
        let mut surrogate = frame.clone();
        let mut complete = Vec::new();
        let mut stamped = Vec::new();
        for (index, view) in frame.tables.iter().enumerate() {
            if let TableView::Complete(rows) = &**view
                && (rows.table().key_bits() != 0 || rows.len() == 1)
            {
                complete.push(index);
                if !PACKET_HOUR_HOSTS.contains(&rows.table())
                    || packet_hours(rows).iter().all(|hour| hour.stamp().is_none())
                {
                    continue;
                }
                stamped.push(index);
            }
            surrogate.tables[index] = TableView::Unavailable {
                reason: "synthetic uncertified operand".into(),
            }
            .into();
        }
        let mut encoded = self.encoder.encode(surrogate, false).unwrap();
        for &index in &stamped {
            encoded.tables[index] = match self.decoder.current() {
                Some(current)
                    if !encoded.keyframe
                        && SharedView::ptr_eq(&current.tables[index], &frame.tables[index]) =>
                {
                    TableUpdate::Unchanged {
                        baseline: current.hour,
                    }
                }
                _ => {
                    let TableView::Complete(rows) = &*frame.tables[index] else {
                        unreachable!("stamped rows are complete")
                    };
                    TableUpdate::Snapshot(rows.clone())
                }
            };
        }
        if !encoded.keyframe
            && let Some(current) = self.decoder.current()
        {
            for &index in &complete {
                let table = TableId::ALL[index];
                if stamped.contains(&index) || !PACKET_HOUR_HOSTS.contains(&table) {
                    continue;
                }
                let TableView::Complete(previous) = &*current.tables[index] else {
                    continue;
                };
                if packet_hours(previous)
                    .iter()
                    .all(|hour| hour.stamp().is_none())
                {
                    continue;
                }
                let TableView::Complete(rows) = &*frame.tables[index] else {
                    unreachable!("certified rows are complete")
                };
                let mut bytes = vec![1];
                bytes.extend_from_slice(&current.hour.to_le_bytes());
                rows.encode_changes(previous, &mut bytes, &mut Vec::new())
                    .unwrap();
                let mut input = replay_data::codec::Decoder::new(&bytes);
                encoded.tables[index] = replay_frame::frame::decode_update(
                    table,
                    table.last_change(),
                    replay_data::format::FormatVersion::CURRENT,
                    false,
                    &mut input,
                )
                .unwrap();
                input.finish().unwrap();
            }
        }
        let decoded = self.decoder.apply(&encoded).unwrap();
        for index in complete {
            frame.tables[index] = decoded.tables[index].clone();
        }
    }
}
fn fog_delta<T: TableSchema>(frame: &mut DecodedFrame, next: Vec<T::Row>) {
    frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(next)).into();
}

#[test]
fn incremental_fog_matches_frozen_bytes_and_knownness_without_population_sized_updates() {
    let stack = fixture("fog_incremental_changes");
    for count in [3, 1500, 5000] {
        let content = fog_content(&stack);
        let memos = FogMemos::new();
        let choice = perspective::Choice::default();
        let mut certified = CertifiedFrames::default();
        let mut frame = fog_frame(count);
        for step in 0..13 {
            if step > 0 {
                frame.hour += 1;
            }
            match step {
                1 => {
                    let mut rows = frame
                        .complete::<row_tables::CountryIntel>()
                        .unwrap()
                        .to_vec();
                    rows[0].army = 90_000;
                    fog_delta::<row_tables::CountryIntel>(&mut frame, rows);
                }
                2 => {
                    let mut rows = frame.complete::<row_tables::Divisions>().unwrap().to_vec();
                    rows[0].province = 2;
                    fog_delta::<row_tables::Divisions>(&mut frame, rows);
                }
                3 => {
                    let mut rows = frame.complete::<row_tables::Provinces>().unwrap().to_vec();
                    rows[1].controller = 101;
                    fog_delta::<row_tables::Provinces>(&mut frame, rows);
                }
                4 => fog_delta::<row_tables::FactionMembers>(
                    &mut frame,
                    vec![
                        FactionMemberRow {
                            country: 101,
                            faction: 9,
                            ordinal: 0,
                        },
                        FactionMemberRow {
                            country: 103,
                            faction: 9,
                            ordinal: 1,
                        },
                    ],
                ),
                5 => fog_delta::<row_tables::WarRelations>(
                    &mut frame,
                    vec![WarRelationRow {
                        first_country: 101,
                        second_country: 103,
                        ..Default::default()
                    }],
                ),
                6 => fog_delta::<row_tables::UnitIntel>(&mut frame, vec![]),
                7 => fog_delta::<row_tables::FactionMembers>(&mut frame, vec![]),
                8 => fog_delta::<row_tables::RelationLinks>(
                    &mut frame,
                    vec![RelationLinkRow {
                        country: 103,
                        target_index: 1,
                        links: RelationLinkRow::ATTACHE,
                        ..Default::default()
                    }],
                ),
                9 => fog_delta::<row_tables::Divisions>(&mut frame, vec![]),
                10 => fog_delta::<row_tables::RelationLinks>(&mut frame, vec![]),
                11 => fog_delta::<row_tables::WarRelations>(&mut frame, vec![]),
                _ => {}
            }
            certified.advance(&mut frame);
            let old = Original {
                content: &content,
                frame: &frame,
                hour_fraction: 0.5,
            };
            let reference = old.fog_vision(&choice).unwrap();
            let hour = ProductHour::new(&content, &frame, 0.5);
            let actual = hour
                .fog(&choice, &memos, &mut BuilderSpans::new(false))
                .unwrap();
            let vision = hour.vision(&choice).unwrap();
            assert_eq!(
                product_codec::encode(&actual.display()).unwrap(),
                product_codec::encode(&old.fog_display(&choice).unwrap()).unwrap(),
                "count {count} step {step}"
            );
            assert_eq!(vision.province_fog(), reference.province_fog());
            for owner in 1..=3 {
                assert_eq!(
                    vision.shares_vision_proven(owner),
                    reference.shares_vision_proven(owner)
                );
                for unit in [7, 8] {
                    for province in 1..=3 {
                        assert_eq!(
                            vision.land_byte_known(unit, owner, province),
                            reference.land_byte_known(unit, owner, province)
                        );
                        assert_eq!(
                            vision.sees_land(unit, owner, province),
                            reference.sees_land(unit, owner, province)
                        );
                    }
                }
            }
            if step > 0 && step < 12 {
                let work = content.visions.stats().unwrap();
                assert!(!work.cold);
                assert!(work.provinces <= 3);
                assert!(work.jobs < 100, "{count} countries: {work:?}");
            }
        }
    }
}

fn assert_incremental_vision(
    content: &ProductContent,
    frame: &DecodedFrame,
    memos: &FogMemos,
    label: &str,
) {
    let choice = perspective::Choice::default();
    let old = Original {
        content,
        frame,
        hour_fraction: 0.5,
    };
    let expected = old.fog_vision(&choice).unwrap();
    let hour = ProductHour::new(content, frame, 0.5);
    let actual = hour.vision(&choice).unwrap();
    assert_eq!(actual.province_fog(), expected.province_fog(), "{label}");
    for owner in 0..=3 {
        assert_eq!(
            actual.shares_vision_proven(owner),
            expected.shares_vision_proven(owner),
            "{label}"
        );
        for unit in [7, 8, 20] {
            for province in 1..=4 {
                assert_eq!(
                    actual.land_byte_known(unit, owner, province),
                    expected.land_byte_known(unit, owner, province),
                    "{label}"
                );
                assert_eq!(
                    actual.sees_land(unit, owner, province),
                    expected.sees_land(unit, owner, province),
                    "{label}"
                );
            }
        }
    }
    assert_eq!(
        actual,
        old.ancillary_vision(&choice).unwrap(),
        "supplemental whole Vision: {label}"
    );
    let fog = hour
        .fog(&choice, memos, &mut BuilderSpans::new(false))
        .unwrap();
    assert_eq!(
        product_codec::encode(&fog.display()).unwrap(),
        product_codec::encode(&old.fog_display(&choice).unwrap()).unwrap(),
        "{label}"
    );
}

#[test]
fn incremental_vision_keeps_fresh_empty_and_missing_views_change_proportional() {
    let stack = fixture("fog_fresh_absence_views");
    for count in [3, 1500, 5000] {
        let content = fog_content(&stack);
        let memos = FogMemos::new();
        for kind in 0..4 {
            let mut certified = CertifiedFrames::default();
            let mut frame = fog_frame(count);
            let mut previous = None;
            for step in 0..4 {
                frame.hour += 1;
                frame.tables[TableId::StateVisibility.index()] = match kind {
                    0 => TableView::Complete(TableRows::StateVisibility(vec![])),
                    1 => TableView::Truncated {
                        rows: TableRows::StateVisibility(vec![]),
                        missing: 2,
                    },
                    2 => TableView::Unavailable {
                        reason: "visibility not acquired".into(),
                    },
                    _ => TableView::NotRecorded {
                        mode: replay_data::scope::CaptureMode::Map,
                    },
                }
                .into();
                if step == 2 {
                    let mut rows = frame
                        .complete::<row_tables::CountryIntel>()
                        .unwrap()
                        .to_vec();
                    rows[0].army = 90_000;
                    fog_delta::<row_tables::CountryIntel>(&mut frame, rows);
                }
                certified.advance(&mut frame);
                assert_incremental_vision(&content, &frame, &memos, "fresh absent view");
                let work = content.visions.stats().unwrap();
                if step > 0 {
                    assert!(!work.cold, "{count} countries, state {kind}: {work:?}");
                    assert!(work.jobs < 100);
                    assert!(work.provinces <= 3);
                }
                let value = ProductHour::new(&content, &frame, 0.5)
                    .vision(&perspective::Choice::default())
                    .unwrap();
                if matches!(step, 1 | 3) {
                    assert!(Arc::ptr_eq(previous.as_ref().unwrap(), &value));
                }
                previous = Some(value);
            }
        }
    }
}

#[test]
fn incremental_vision_keeps_visibility_roots_when_changed_intel_leaves_province_bytes_equal() {
    let stack = fixture("fog_equal_visibility_roots");
    let content = fog_content(&stack);
    let memos = FogMemos::new();
    let mut certified = CertifiedFrames::default();
    let mut frame = fog_frame(3);
    let choice = perspective::Choice::default();
    certified.advance(&mut frame);
    let before = ProductHour::new(&content, &frame, 0.5)
        .vision(&choice)
        .unwrap();
    frame.hour += 1;
    let mut rows = frame
        .complete::<row_tables::CountryIntel>()
        .unwrap()
        .to_vec();
    rows[0].air += 1;
    fog_delta::<row_tables::CountryIntel>(&mut frame, rows);
    certified.advance(&mut frame);
    assert_incremental_vision(&content, &frame, &memos, "air-only intel change");
    let after = ProductHour::new(&content, &frame, 0.5)
        .vision(&choice)
        .unwrap();
    assert!(Arc::ptr_eq(
        before.shared_province_fog().unwrap(),
        after.shared_province_fog().unwrap()
    ));
}

#[test]
fn incremental_vision_refreshes_empty_descriptors_after_certified_insertions() {
    let stack = fixture("fog_empty_descriptor_refresh");
    let content = fog_content(&stack);
    let memos = FogMemos::new();
    let mut certified = CertifiedFrames::default();
    let mut frame = fog_frame(3);
    let rows = frame
        .complete::<row_tables::CountryIntel>()
        .unwrap()
        .to_vec();
    frame.tables[TableId::CountryIntel.index()] =
        TableView::Complete(TableRows::CountryIntel(vec![])).into();
    certified.advance(&mut frame);
    assert_incremental_vision(&content, &frame, &memos, "initial empty");
    frame.hour += 1;
    fog_delta::<row_tables::CountryIntel>(&mut frame, rows);
    certified.advance(&mut frame);
    assert_incremental_vision(&content, &frame, &memos, "certified insertion");
    frame.hour += 1;
    frame.tables[TableId::CountryIntel.index()] =
        TableView::Complete(TableRows::CountryIntel(vec![])).into();
    certified.advance(&mut frame);
    assert_incremental_vision(&content, &frame, &memos, "fresh empty replacement");
}

#[test]
fn incremental_fog_preserves_ordered_radar_network_fleet_volunteer_and_invasion_changes() {
    let stack = fixture("fog_incremental_ordered_sources");
    for count in [3, 1500, 5000] {
        let ProductContent {
            reads, mut inputs, ..
        } = fog_content(&stack);
        let fog = Arc::make_mut(&mut inputs.fog).as_mut().unwrap();
        fog.topology.sea.insert(4);
        fog.topology.neighbors.insert(4, vec![2, 3]);
        fog.defines.radar_effect = -225;
        fog.defines.map_cutoffs = vec![50_000, -500_000, 100_001, 500_000];
        let content = ProductContent::from_inputs(reads, inputs);
        let memos = FogMemos::new();
        let mut certified = CertifiedFrames::default();
        let mut frame = fog_frame(count);
        put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    controller: 101,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    controller: 102,
                    state: 9,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 3,
                    controller: 103,
                    state: 9,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 4,
                    controller: 102,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::States(vec![StateRow {
                id: 9,
                owner: 103,
                controller: 102,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::StateVisibility(vec![StateVisibilityRow {
                state: 9,
                network_modifier: -200_000,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 101,
                    faction: 9,
                    ordinal: 0,
                },
                FactionMemberRow {
                    country: 103,
                    faction: 9,
                    ordinal: 1,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                roster_owner: 103,
                logical_owner: 101,
                province: 4,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 7,
                path: vec![2],
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::WarRelations(vec![WarRelationRow {
                first_country: 101,
                second_country: 102,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Fleets(vec![FleetRow {
                id: 19,
                owner: 103,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: 20,
                fleet: 19,
                logical_owner: 101,
                province: 4,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::LandTheatres(vec![
                LandTheatreRow {
                    id: 30,
                    owner: 101,
                    volunteers_theatre: 103,
                    ordinal: 0,
                },
                LandTheatreRow {
                    id: 31,
                    owner: 102,
                    volunteers_theatre: 101,
                    ordinal: 0,
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::TheatreAreas(vec![
                TheatreAreaRow {
                    theatre: 30,
                    ordinal: 0,
                    provinces: vec![3, 3],
                    ..Default::default()
                },
                TheatreAreaRow {
                    theatre: 30,
                    ordinal: 1,
                    provinces: vec![],
                    ..Default::default()
                },
                TheatreAreaRow {
                    theatre: 30,
                    ordinal: 2,
                    provinces: vec![2],
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::StationedUnits(vec![StationedUnitRow {
                theatre: 31,
                ordinal: 0,
                unit: 7,
            }]),
        );
        put(
            &mut frame,
            TableRows::VolunteerHosts(vec![VolunteerHostRow {
                country: 101,
                position: 0,
                host: 103,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::RadarCoverage(vec![
                RadarCoverageRow {
                    country: 101,
                    ordinal: 0,
                    provinces: vec![2, 2, 3],
                    ..Default::default()
                },
                RadarCoverageRow {
                    country: 103,
                    ordinal: 0,
                    provinces: vec![2],
                    ..Default::default()
                },
            ]),
        );
        for step in 0..12 {
            if step > 0 {
                frame.hour += 1;
            }
            match step {
                1 => fog_delta::<row_tables::StateVisibility>(
                    &mut frame,
                    vec![StateVisibilityRow {
                        state: 9,
                        network_modifier: 100_000,
                        ..Default::default()
                    }],
                ),
                2 => fog_delta::<row_tables::Fleets>(
                    &mut frame,
                    vec![FleetRow {
                        id: 19,
                        owner: 102,
                        ..Default::default()
                    }],
                ),
                3 => fog_delta::<row_tables::TaskForces>(
                    &mut frame,
                    vec![TaskForceRow {
                        id: 20,
                        fleet: 19,
                        logical_owner: 101,
                        province: 2,
                        ..Default::default()
                    }],
                ),
                4 => fog_delta::<row_tables::TheatreAreas>(
                    &mut frame,
                    vec![
                        TheatreAreaRow {
                            theatre: 30,
                            ordinal: 0,
                            provinces: vec![],
                            ..Default::default()
                        },
                        TheatreAreaRow {
                            theatre: 30,
                            ordinal: 2,
                            provinces: vec![2],
                            ..Default::default()
                        },
                    ],
                ),
                5 => fog_delta::<row_tables::VolunteerHosts>(
                    &mut frame,
                    vec![VolunteerHostRow {
                        country: 101,
                        host: 102,
                        ..Default::default()
                    }],
                ),
                6 => fog_delta::<row_tables::StationedUnits>(
                    &mut frame,
                    vec![StationedUnitRow {
                        theatre: 31,
                        ordinal: 0,
                        unit: 20,
                    }],
                ),
                7 => fog_delta::<row_tables::DivisionMovement>(
                    &mut frame,
                    vec![DivisionMovementRow {
                        division: 7,
                        path: vec![3, 2],
                        ..Default::default()
                    }],
                ),
                8 => {
                    let mut rows = frame.complete::<row_tables::Provinces>().unwrap().to_vec();
                    rows[2].controller = 102;
                    fog_delta::<row_tables::Provinces>(&mut frame, rows);
                }
                9 => fog_delta::<row_tables::RadarCoverage>(
                    &mut frame,
                    vec![RadarCoverageRow {
                        country: 103,
                        ordinal: 0,
                        provinces: vec![2, 3, 2],
                        ..Default::default()
                    }],
                ),
                10 => fog_delta::<row_tables::LandTheatres>(
                    &mut frame,
                    vec![LandTheatreRow {
                        id: 31,
                        owner: 102,
                        volunteers_theatre: 101,
                        ordinal: 0,
                    }],
                ),
                _ => {}
            }
            certified.advance(&mut frame);
            assert_incremental_vision(
                &content,
                &frame,
                &memos,
                &format!("{count} countries source step {step}"),
            );
            if step > 0 && step < 11 {
                let work = content.visions.stats().unwrap();
                assert!(
                    !work.cold && work.jobs < 150 && work.provinces <= 4,
                    "{work:?}"
                );
            }
        }
    }
}

#[test]
fn incremental_fog_country_zero_packets_clocks_refusals_and_recovery_match_frozen_bytes() {
    let stack = fixture("fog_incremental_zero_packets");
    for count in [3, 1500, 5000] {
        let ProductContent {
            reads, mut inputs, ..
        } = fog_content(&stack);
        let fog = Arc::make_mut(&mut inputs.fog).as_mut().unwrap();
        fog.topology.neighbors.insert(2, vec![1, 3]);
        fog.topology.neighbors.insert(3, vec![2]);
        fog.defines.map_cutoffs = vec![50_000, -500_000, 100_001, 500_000];
        let content = ProductContent::from_inputs(reads, inputs);
        let memos = FogMemos::new();
        let mut certified = CertifiedFrames::default();
        let mut frame = fog_frame(count);
        let mut links = frame
            .complete::<row_tables::CountryLinks>()
            .unwrap()
            .to_vec();
        links.extend([
            CountryLinkRow {
                id: 0,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 10_000,
                canonical_slot: 0,
            },
        ]);
        links.sort_by_key(RowCodec::key);
        put(&mut frame, TableRows::CountryLinks(links));
        put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    controller: 101,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    controller: 10_000,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 3,
                    controller: 0,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                roster_owner: 101,
                logical_owner: 101,
                province: 2,
                ..Default::default()
            }]),
        );
        let cell = |target| {
            Some(ReverseZeroVisibilityInputs {
                target: Some(target),
                war: Some([0, 0]),
                subject: Some(ZeroSubjectInputs::default()),
                military_access: None,
            })
        };
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 101,
                forward_visibility: Recorded::Value(cell(10_000)),
                ..Default::default()
            }]),
        );
        let sample_hour = frame.hour;
        put(
            &mut frame,
            TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
                faction: Recorded::Value(Some(0)),
                country_intel_sample: Recorded::Value(Some(ReservedCountryIntelSample {
                    sample_hour,
                    vector_count: 4,
                })),
                reverse_visibility: Recorded::Value(Some(ReservedReverseVisibilityInputs {
                    sample_hour: PacketHour::Stamped(sample_hour),
                    cells: (0..4).map(cell).collect(),
                })),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryIntel(
                (0..4)
                    .map(|target_index| CountryIntelRow {
                        owner_country: 0,
                        target_index,
                        army: 75_000,
                        ..Default::default()
                    })
                    .chain([CountryIntelRow {
                        owner_country: 101,
                        target_index: 0,
                        army: 25_000,
                        ..Default::default()
                    }])
                    .collect(),
            ),
        );
        for step in 0..10 {
            if step > 0 {
                frame.hour += 1;
            }
            match step {
                2 | 4 | 7 | 9 => {
                    let mut rows = frame
                        .complete::<row_tables::ReservedCountryNaming>()
                        .unwrap()
                        .to_vec();
                    rows[0].reverse_visibility =
                        Recorded::Value(Some(ReservedReverseVisibilityInputs {
                            sample_hour: PacketHour::Host,
                            cells: (0..4).map(cell).collect(),
                        }));
                    if step == 2 {
                        rows[0].country_intel_sample =
                            Recorded::Value(Some(ReservedCountryIntelSample {
                                sample_hour: frame.hour + 1,
                                vector_count: 4,
                            }));
                    }
                    fog_delta::<row_tables::ReservedCountryNaming>(&mut frame, rows);
                }
                5 => fog_delta::<row_tables::CountryCounterInputs>(
                    &mut frame,
                    vec![CountryCounterInputRow {
                        country: 101,
                        forward_visibility: Recorded::Value(None),
                        ..Default::default()
                    }],
                ),
                6 => fog_delta::<row_tables::CountryCounterInputs>(
                    &mut frame,
                    vec![CountryCounterInputRow {
                        country: 101,
                        forward_visibility: Recorded::Value(cell(10_000)),
                        ..Default::default()
                    }],
                ),
                8 => {
                    let mut rows = frame
                        .complete::<row_tables::CountryIntel>()
                        .unwrap()
                        .to_vec();
                    rows.retain(|r| !(r.owner_country == 0 && r.target_index == 3));
                    fog_delta::<row_tables::CountryIntel>(&mut frame, rows);
                }
                _ => {}
            }
            certified.advance(&mut frame);
            assert_incremental_vision(
                &content,
                &frame,
                &memos,
                &format!("{count} countries zero step {step}"),
            );
            if step > 0 {
                let work = content.visions.stats().unwrap();
                assert!(
                    !work.cold && work.jobs < 100 && work.provinces <= 3,
                    "{count} countries zero step {step}: {work:?}"
                );
            }
        }
    }
}

#[test]
fn streamed_content_operand_witness_matches_bytes_and_never_publishes_partial_data() {
    let stack = fixture("streamed_display_operand");
    let layers = stack.layers(&[]);
    let buffered = ContentReads::new(&layers);
    let streamed = ContentReads::new(&layers);
    let bytes: Vec<_> = (0..200_000).map(|n| (n % 251) as u8).collect();
    buffered.operand_bytes(b"fixture", &[bytes.as_slice(), bytes.as_slice()].concat());
    streamed
        .operand_stream(b"fixture", |out| {
            for chunk in bytes.chunks(3) {
                out.write_all(chunk)?;
            }
            out.flush()?;
            out.write_all(&bytes)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(buffered.entries(), streamed.entries());
    let previous = streamed.entries();
    let refused = streamed
        .operand_stream(b"refused", |out| {
            out.write_all(b"unfinished")?;
            Err(io::Error::other("fixture failure"))
        })
        .unwrap_err();
    assert_eq!(refused.to_string(), "fixture failure");
    assert_eq!(streamed.entries(), previous);
}

#[test]
fn incremental_vision_keeps_carried_values_across_decoder_certificate_epochs() {
    let stack = fixture("fog_carried_epoch");
    for count in [3, 1500, 5000] {
        let content = fog_content(&stack);
        let choice = perspective::Choice::default();
        let mut certified = CertifiedFrames::default();
        let mut frame = fog_frame(count);
        certified.advance(&mut frame);
        let first = ProductHour::new(&content, &frame, 0.0)
            .vision(&choice)
            .unwrap();
        let prior = frame.tables[TableId::Countries.index()].identity();
        let allocation = frame.tables[TableId::Countries.index()].as_ptr();
        frame.hour += 2;
        certified.advance(&mut frame);
        assert_eq!(
            allocation,
            frame.tables[TableId::Countries.index()].as_ptr()
        );
        assert!(!prior.is(&frame.tables[TableId::Countries.index()]));
        let next = ProductHour::new(&content, &frame, 0.0)
            .vision(&choice)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &next));
        let work = content.visions.stats().unwrap();
        assert!(!work.cold);
        assert_eq!((work.jobs, work.provinces), (0, 0));
        assert_frozen_fog(&content, &frame, &choice);
    }
}

