use super::*;
use super::tables;
use content_witness::{Entry, Outcome};
use installed_content_core::{
    content::ResolvedLayers,
    map::{ProvinceDefinition, ProvinceKind},
    test_support::Stack,
};
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
            let authored = String::from_utf8(values).unwrap().parse::<f32>().unwrap();
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
                        Some(installed_content_sound::combat_sounds::BattleDefines {
                            effect: "fixture_distant".into(),
                            falloff_default: 80.,
                            min_unit_count: 1,
                        }),
                        Default::default(),
                    ),
                )),
                triggers: Arc::new(trigger_content()),
                vision_memo: Arc::new(Keyed::new(perspective::VISION_TABLES, 8)),
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
    let result: io::Result<Vec<u8>> = reads.load(|layers| layers.read("map/missing.txt"));
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
                installed_content_core::content::LayerSource::Folder(root) => {
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

// Frozen pre-extraction viewer builders are the independent oracle.
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
    fn vision(&self, choice: &perspective::Choice) -> io::Result<Arc<perspective::Vision>> {
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
    fn intel_map_from_visibility(&self, fog: Option<&BTreeMap<u32, u8>>) -> io::Result<IntelMap> {
        match fog {
            None => textures::clear_intel_map(&self.map),
            Some(_) => Ok(textures::IntelCells::new(&self.map)?.intel_map(fog)),
        }
    }
    pub fn nuclear_impacts(&self) -> io::Result<viewer_world::map::strike_effects::Impacts> {
        viewer_world::map::strike_effects::Impacts::read(self.frame, |province| {
            self.impact_place(province)
        })
    }
    fn impact_place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        Some((
            viewer_foundation::map::counter_anchors::stack_anchor(
                &self.anchors,
                &self.default_anchors,
                province,
                viewer_foundation::map::unit_rules::STANDSTILL,
            )?,
            *self.bounds.get(&province)?,
        ))
    }
    pub fn map_text(
        &self,
        vision: &perspective::Vision,
    ) -> viewer_world::map::animated_map_text::events::Prepared {
        use viewer_world::map::animated_map_text::events::{Context, Prepared};
        let view = vision.ours();
        match viewer_foundation::table_access::rows::<replay_data::rows::tables::MapEvents>(
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
        let vision = self.vision(perspective)?;
        out.note_perspective(&tables::Perspective {
            tag: self.viewing_tag(&vision)?,
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
                "the rows this hour did not reach are read as no relation and no detection at \
                 all, so a stack among them carries the game's question mark where the game drew \
                 it in full",
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
        Ok(out)
    }
    pub fn viewing_tag(&self, vision: &perspective::Vision) -> io::Result<String> {
        let ours = vision.ours();
        Ok(tables::core::<row_tables::Countries>(self.frame)?
            .iter()
            .find(|country| country.canonical_slot == ours)
            .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
            .unwrap_or_else(|| format!("country slot {ours}")))
    }
    pub fn battle_sounds(&self) -> Option<viewer_audio::Battles> {
        use replay_data::rows::tables;
        use viewer_foundation::table_access::{Available, rows};
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
        for (&division, _) in members.filter(|(_, class)| **class == DIVISION_CLASS) {
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
        let vision = old.vision(&choice()).unwrap();
        assert_eq!(*hour.vision(&choice()).unwrap(), *vision);
        let fog = hour
            .fog(&choice(), &FogMemos::new(8), &mut BuilderSpans::new(false))
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
        drop(old);
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
            .fog(&choice(), &FogMemos::new(8), &mut BuilderSpans::new(false))
            .err()
            .unwrap();
        let expected = old.vision(&choice()).unwrap_err();
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
fn weather_delegates_the_existing_priority_field_including_unavailable_tables() {
    let stack = fixture("products_original_weather");
    let content = content(&stack);
    let mut frame = frame(2);
    put(
        &mut frame,
        TableRows::WeatherRegions(vec![WeatherRegionRow {
            id: 73,
            light_rain: 1,
            snow: 1,
            ..Default::default()
        }]),
    );
    assert!(
        ProductHour::new(&content, &frame, 0.)
            .weather_field()
            .unwrap()
            == weather_effects::field(&frame).unwrap()
    );
    frame.tables[TableId::WeatherRegions.index()] = TableView::Unavailable {
        reason: "fixture weather coverage".into(),
    }
    .into();
    let expected = weather_effects::field(&frame);
    let actual = ProductHour::new(&content, &frame, 0.).weather_field();
    assert_eq!(actual.is_err(), expected.is_err());
    if let (Err(actual), Err(expected)) = (actual, expected) {
        assert_eq!(actual.kind(), expected.kind());
        assert_eq!(actual.to_string(), expected.to_string());
    }
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
    let vision = old.vision(&choice()).unwrap();
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
