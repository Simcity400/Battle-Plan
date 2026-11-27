use crate::counters::CounterAssets;
use crate::counters::{Carry, MissionMemo, Need, Shown, hour_tables_profiled};
use super::{
    Arc, BTreeMap, Countries, CountryIndex, DecodedFrame, core, countries, ideology_groups, io,
    tables, warm_initial_flags,
};
use viewer_counter_kit::{Child, Kinds, draw_flag, type_tint};
use viewer_foundation::perspective::Vision;
use viewer_foundation::tables::rows;
use viewer_native_gui::lazy_icons::{LazyFlags, LazyTextures};
use viewer_native_gui::sprites::Batch;
use replay_data::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, PoliticalPartyRow, PoliticsRow, Recorded,
    TableId, TableRows,
};
use replay_frame::frame::TableView;
use std::sync::Mutex;
use viewer_gpu::atlas_pages::CellPages;
use viewer_native_gui::lazy_icons::FlagReader;

fn frame(step: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1 + u64::from(step / 18),
        hour: 100
            + if step < 24 {
                step as i32
            } else {
                (48 - step) as i32
            },
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    let mut put = |rows: TableRows| {
        rows.validate_keys().unwrap();
        let id = rows.table();
        *frame.tables[id.index()] = TableView::Complete(rows);
    };
    put(TableRows::CountryLinks(
        [(10, 1), (11, 2), (12, 2), (13, 3)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .into(),
    ));
    put(TableRows::Countries(
        [
            (10, 1, "AAA"),
            (11, 2, "BBB"),
            (12, 2, "CCC"),
            (13, 3, "BROKEN"),
        ]
        .map(|(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .into(),
    ));
    let appearances = [10, 11, 12, 13]
        .into_iter()
        .filter(|id| step % 9 != 5 || *id != 12)
        .map(|country| CountryAppearanceRow {
            country,
            cosmetic_tag: match (country, step % 9) {
                (12, 0 | 1) => b"COS".to_vec(),
                (12, 2) => b"OTHER".to_vec(),
                (12, 3) => vec![0xff],
                (13, _) => b"BROKEN".to_vec(),
                _ => Vec::new(),
            },
            original_tag_reference: if country == 12 && step % 9 == 4 {
                Recorded::NotRecorded
            } else {
                Recorded::Value(Some(i32::from(country == 12 && step.is_multiple_of(2))))
            },
            original_tag: Recorded::Value(Some(b"ORI".to_vec())),
            ..Default::default()
        })
        .collect();
    put(TableRows::CountryAppearance(appearances));
    put(TableRows::Politics(
        [10, 11, 12, 13]
            .map(|country| PoliticsRow {
                country,
                ruling_party: (country != 12 || step % 9 != 6).then_some(7),
                ..Default::default()
            })
            .into(),
    ));
    put(TableRows::PoliticalParties(
        [10, 11, 12, 13]
            .into_iter()
            .filter(|id| step % 9 != 7 || *id != 12)
            .map(|country| PoliticalPartyRow {
                country,
                id: 7,
                token: if country == 12 && step % 9 == 8 {
                    vec![0xff]
                } else if step % 3 == 1 {
                    b"h".to_vec()
                } else {
                    b"g".to_vec()
                },
                ..Default::default()
            })
            .collect(),
    ));
    frame
}

fn reference(frame: &DecodedFrame) -> io::Result<Countries> {
    core::<tables::CountryLinks>(frame)?;
    let source = core::<tables::Countries>(frame)?;
    let country_rows: BTreeMap<_, _> = source.iter().map(|r| (r.canonical_slot, r)).collect();
    let appearances: BTreeMap<_, _> = rows::<tables::CountryAppearance>(frame)?
        .rows()
        .iter()
        .map(|r| (r.country, r))
        .collect();
    Ok(Countries {
        tags: source
            .iter()
            .map(|r| {
                (
                    r.canonical_slot,
                    String::from_utf8_lossy(&r.tag).into_owned(),
                )
            })
            .collect(),
        flag_tags: country_rows
            .iter()
            .filter_map(|(&slot, country)| {
                let row = appearances.get(&country.id)?;
                Some((
                    slot,
                    (
                        std::str::from_utf8(&row.cosmetic_tag)
                            .map(str::to_owned)
                            .map_err(|_| "country cosmetic tag is not UTF-8".into()),
                        content_reader_country_names::country_names::original_tag(country, row)
                            .map(str::to_owned),
                    ),
                ))
            })
            .collect(),
        groups: ideology_groups(frame, &country_rows)?,
        ..Default::default()
    })
}

type FlagReads = Vec<([u32; 2], String)>;

struct Stores {
    textures: Arc<LazyTextures>,
    flags: [Arc<LazyFlags>; 3],
    reads: Arc<Mutex<FlagReads>>,
}

impl Stores {
    fn new() -> Self {
        let textures = LazyTextures::new();
        textures.set_background(false);
        let reads = Arc::new(Mutex::new(Vec::new()));
        let flags = [[10, 7], [41, 26], [82, 52]].map(|cell| {
            let calls = Arc::clone(&reads);
            let read: FlagReader = Arc::new(move |key| {
                assert!(!key.starts_with("UNUSED"), "never scan all indexed flags");
                calls.lock().unwrap().push((cell, key.to_owned()));
                if key.starts_with("BROKEN") || key == "COS_g" {
                    return None;
                }
                let shade = key.bytes().fold(0u8, u8::wrapping_add);
                Some(content_reader_image::Rgba8Image {
                    width: cell[0],
                    height: cell[1],
                    pixels: vec![shade; (cell[0] * cell[1] * 4) as usize],
                })
            });
            let index = [
                "AAA", "BBB", "CCC", "COS", "OTHER", "ORI", "BROKEN", "COLD", "UNUSED",
            ]
            .into_iter()
            .flat_map(|tag| {
                ["", "_g", "_h"].map(move |group| {
                    let key = format!("{tag}{group}");
                    (key.clone(), key)
                })
            })
            .collect();
            LazyFlags::new(
                index,
                CellPages::plan(cell, 8192, "flags").unwrap(),
                read,
                &textures,
            )
        });
        Self {
            textures,
            flags,
            reads,
        }
    }

    fn install(&self, assets: &mut CounterAssets) {
        [assets.flags, assets.medium_flags, assets.large_flags] = self.flags.clone();
    }

    fn trace(&self) -> FlagReads {
        let mut trace = self.reads.lock().unwrap().clone();
        trace.sort();
        trace
    }
}

fn assert_stores(a: &Stores, b: &Stores) {
    assert_eq!(a.textures.version(), b.textures.version());
    assert_eq!(a.textures.has_pending(), b.textures.has_pending());
    assert_eq!(a.textures.resident_bytes(), b.textures.resident_bytes());
    let snapshots = [a.textures.snapshot(), b.textures.snapshot()];
    assert_eq!(snapshots[0].len(), snapshots[1].len());
    for ((aid, ai), (bid, bi)) in snapshots[0].iter().zip(&snapshots[1]) {
        assert_eq!(aid, bid);
        assert_eq!(ai.revision, bi.revision);
        assert_eq!(
            (ai.texture.width, ai.texture.height),
            (bi.texture.width, bi.texture.height)
        );
        assert_eq!(ai.texture.faces, bi.texture.faces);
        assert_eq!(ai.texture.format, bi.texture.format);
        assert_eq!(ai.texture.mips.len(), bi.texture.mips.len());
        for (am, bm) in ai.texture.mips.iter().zip(&bi.texture.mips) {
            assert_eq!(am.len(), bm.len());
            for (at, bt) in am.iter().zip(bm) {
                assert_eq!(at.data, bt.data);
            }
        }
    }
    for (a, b) in a.flags.iter().zip(&b.flags) {
        assert_eq!(a.loaded(), b.loaded());
        assert_eq!(a.rejected(), b.rejected());
    }
}

#[test]
fn initial_flag_warm_matches_cold_acquisition_across_changing_hours() {
    content_reader_script::test_game::enter();
    let cold = Stores::new();
    let warm = Stores::new();
    super::super::tests::with_counter_inputs_fixture(
        viewer_counter_kit::fixtures::action_gui(false),
        |a| cold.install(a),
        |_| {},
        |old| {
            super::super::tests::with_counter_inputs_fixture(
                viewer_counter_kit::fixtures::action_gui(false),
                |a| warm.install(a),
                |_| {},
                |new| {
                    warm_initial_flags(&frame(0), new.assets).unwrap();
                    assert!(warm.flags.iter().all(|f| f.loaded() == 0));
                    assert_eq!(warm.textures.version(), 0);
                    assert_eq!(warm.textures.resident_bytes(), 0);
                    assert!(!warm.textures.has_pending());
                    let initial_reads = warm.trace();
                    assert!(!initial_reads.is_empty());
                    let child = Child {
                        sprite: String::new(),
                        at: [3.5, 1.25],
                        scale: 0.8,
                        frame: 0,
                        rotation: 0.,
                        centered: false,
                        order: 2,
                    };
                    let mut emitted = 0;
                    for step in 0..36 {
                        let frame = frame(step);
                        let a = reference(&frame).unwrap();
                        let b = countries(&frame).unwrap();
                        assert_eq!(a.tags_map(), b.tags_map());
                        assert_eq!(a.flag_tags_map(), b.flag_tags_map());
                        assert_eq!(a.groups, b.groups);
                        for slot in [3, 1, 2, 99] {
                            assert_eq!(a.flag_keys(slot), b.flag_keys(slot));
                            assert_eq!(a.flag_refusal(slot), b.flag_refusal(slot));
                        }
                        a.warm_flags(old.assets);
                        b.warm_flags(new.assets);
                        if step == 0 {
                            assert_eq!(warm.trace(), initial_reads);
                        }
                        let mut batches = [Batch::default(), Batch::default()];
                        for size in 0..3 {
                            for slot in [3, 1, 2, 1, 99] {
                                draw_flag(
                                    &cold.flags[size],
                                    &mut batches[0],
                                    &child,
                                    [step as f32 * 0.125, 20.5],
                                    &a,
                                    slot,
                                );
                                draw_flag(
                                    &warm.flags[size],
                                    &mut batches[1],
                                    &child,
                                    [step as f32 * 0.125, 20.5],
                                    &b,
                                    slot,
                                );
                            }
                        }
                        assert_eq!(batches[0].quads, batches[1].quads);
                        emitted += batches[0].quads.len();
                        for (a, b) in batches[0].quads.iter().zip(&batches[1].quads) {
                            assert_eq!(a.rect.map(f32::to_bits), b.rect.map(f32::to_bits));
                            assert_eq!(a.uv.map(f32::to_bits), b.uv.map(f32::to_bits));
                            assert_eq!(a.color.map(f32::to_bits), b.color.map(f32::to_bits));
                            assert_eq!(
                                a.rotation.map(f32::to_bits),
                                b.rotation.map(f32::to_bits)
                            );
                            assert_eq!(a.shade.to_bits(), b.shade.to_bits());
                            assert_eq!(
                                cold.flags.iter().find_map(|f| f.key_at(a.texture, a.uv)),
                                warm.flags.iter().find_map(|f| f.key_at(b.texture, b.uv))
                            );
                        }
                        assert_stores(&cold, &warm);
                        assert_eq!(cold.trace(), warm.trace());
                    }
                    assert!(emitted > 0);
                    for size in 0..3 {
                        assert_eq!(
                            cold.flags[size].acquire(&["COLD", "not indexed"]),
                            warm.flags[size].acquire(&["COLD", "not indexed"])
                        );
                    }
                    assert_stores(&cold, &warm);
                    assert_eq!(cold.trace(), warm.trace());
                },
            );
        },
    );
}

#[test]
fn refused_initial_flag_inputs_leave_every_store_cold() {
    content_reader_script::test_game::enter();
    let stores = Stores::new();
    super::super::tests::with_counter_inputs_fixture(
        viewer_counter_kit::fixtures::action_gui(false),
        |a| stores.install(a),
        |_| {},
        |inputs| {
            for table in [
                TableId::CountryLinks,
                TableId::Countries,
                TableId::CountryAppearance,
                TableId::Politics,
                TableId::PoliticalParties,
            ] {
                let mut source = frame(0);
                *source.tables[table.index()] = TableView::Unavailable {
                    reason: "synthetic refused".into(),
                };
                match reference(&source) {
                    Err(error) => assert_eq!(
                        warm_initial_flags(&source, inputs.assets)
                            .unwrap_err()
                            .to_string(),
                        error.to_string()
                    ),
                    Ok(expected) => {
                        warm_initial_flags(&source, inputs.assets).unwrap();
                        let actual = countries(&source).unwrap();
                        for slot in [1, 2, 3] {
                            assert_eq!(actual.flag_keys(slot), expected.flag_keys(slot));
                            assert_eq!(actual.flag_refusal(slot), expected.flag_refusal(slot));
                            assert_eq!(actual.flag_keys(slot), None);
                        }
                    }
                }
                assert!(stores.trace().is_empty());
                assert_eq!(stores.textures.version(), 0);
                assert_eq!(stores.textures.resident_bytes(), 0);
            }
            for flags in &stores.flags {
                assert!(flags.acquire(&["COLD"]).is_some());
            }
            assert_eq!(stores.trace().len(), 3);
        },
    );
}

#[test]
fn truncated_initial_appearance_keeps_present_country_candidates() {
    content_reader_script::test_game::enter();
    let mut source = frame(0);
    let available = rows::<tables::CountryAppearance>(&source).unwrap();
    *source.tables[TableId::CountryAppearance.index()] = TableView::Truncated {
        rows: TableRows::CountryAppearance(available.rows()[..1].to_vec()),
        missing: 3,
    };
    let a = reference(&source).unwrap();
    let b = countries(&source).unwrap();
    assert_eq!(a.flag_tags_map(), b.flag_tags_map());
    for slot in [1, 2, 3] {
        assert_eq!(a.flag_keys(slot), b.flag_keys(slot));
        assert_eq!(a.flag_refusal(slot), b.flag_refusal(slot));
    }
    assert!(b.flag_keys(1).is_some());
    assert_eq!(b.flag_keys(2), None);
}

#[test]
fn overlay_hour_tables_reuse_settled_country_index_and_preserve_partial_appearance() {
    content_reader_script::test_game::enter();
    super::super::tests::with_counter_inputs_and_content(
        viewer_counter_kit::fixtures::action_gui(false),
        |_| {},
        |_| {},
        |inputs, content| {
            let moves = viewer_unit_reads::movement_reads::test_support::unnamed_assets();
            let carry = Carry::default();
            let need = Need {
                air_missions: false,
                by_key: true,
                shown: Shown::EVERY,
            };
            let build = |frame: &DecodedFrame| {
                hour_tables_profiled(
                    frame,
                    inputs.map,
                    Arc::new(Vision::sharing(1, [])),
                    content,
                    inputs.assets,
                    inputs.equipment,
                    inputs.localisation,
                    &Default::default(),
                    None,
                    None,
                    None,
                    &type_tint::Memo::new(type_tint::TABLES),
                    &MissionMemo::new(),
                    moves.movement_reads(),
                    Err("unreached synthetic naval topology".into()),
                    Kinds::default(),
                    need,
                    &carry,
                    None,
                    &mut viewer_runtime::profiling::BuilderSpans::new(false),
                )
            };

            let first_frame = frame(0);
            let first = build(&first_frame).unwrap();
            let expected = CountryIndex::read(&first_frame).unwrap();
            assert_eq!(&**first.country_index(), &expected);
            assert!(std::ptr::eq(first.country_index().links(), first.links()));
            assert!(std::ptr::eq(first.country_index().ids(), first.country_ids()));
            assert!(std::ptr::eq(first.country_index().tags(), first.countries.tags_map()));
            assert!(std::ptr::eq(
                first.country_index().flag_tags(),
                first.countries.flag_tags_map()
            ));
            assert_eq!(first.links(), expected.links());
            assert_eq!(first.country_ids(), expected.ids());
            assert_eq!(first.countries.tags_map(), expected.tags());
            assert_eq!(first.countries.flag_tags_map(), expected.flag_tags());

            let mut next_frame = first_frame.clone();
            next_frame.hour = first_frame.hour + 1;
            let TableView::Complete(TableRows::Countries(mut next_countries)) =
                (*next_frame.tables[TableId::Countries.index()]).clone()
            else {
                unreachable!()
            };
            next_countries[0].command_power_raw = 77;
            next_frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(next_countries)).into();
            let second = build(&next_frame).unwrap();
            assert!(
                Arc::ptr_eq(first.country_index(), second.country_index()),
                "hourly country values must reuse the settled CountryIndex through hour_tables_profiled"
            );
            assert!(std::ptr::eq(first.links(), second.links()));
            assert!(std::ptr::eq(first.country_ids(), second.country_ids()));
            assert!(std::ptr::eq(
                first.countries.tags_map(),
                second.countries.tags_map()
            ));
            assert!(std::ptr::eq(
                first.countries.flag_tags_map(),
                second.countries.flag_tags_map()
            ));

            let mut truncated_frame = first_frame.clone();
            truncated_frame.hour = first_frame.hour + 2;
            let available = rows::<tables::CountryAppearance>(&truncated_frame).unwrap();
            truncated_frame.tables[TableId::CountryAppearance.index()] = TableView::Truncated {
                rows: TableRows::CountryAppearance(available.rows()[..1].to_vec()),
                missing: 3,
            }
            .into();
            let partial = build(&truncated_frame).unwrap();
            assert!(!Arc::ptr_eq(first.country_index(), partial.country_index()));
            assert_eq!(partial.country_index().links(), expected.links());
            assert_eq!(partial.country_index().tags(), expected.tags());
            assert_eq!(partial.country_index().ids(), expected.ids());
            assert_eq!(partial.country_index().flag_tags().len(), 1);
            assert!(partial.countries.flag_tags_map().contains_key(&1));
            assert!(!partial.countries.flag_tags_map().contains_key(&2));

            let mut unavailable_frame = first_frame.clone();
            unavailable_frame.hour = first_frame.hour + 3;
            unavailable_frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
                reason: "not recorded".into(),
            }
            .into();
            let without = build(&unavailable_frame).unwrap();
            assert_eq!(without.country_index().links(), expected.links());
            assert_eq!(without.country_index().tags(), expected.tags());
            assert_eq!(without.country_index().ids(), expected.ids());
            assert!(without.country_index().flag_tags().is_empty());
            assert!(without.countries.flag_tags_map().is_empty());
        },
    );
}

#[test]
fn ideology_groups_preserve_unavailable_table_reasons() {
    let base = frame(0);
    let source = core::<tables::Countries>(&base).unwrap();
    let country_rows: BTreeMap<_, _> = source.iter().map(|r| (r.canonical_slot, r)).collect();

    let mut no_politics = base.clone();
    *no_politics.tables[TableId::Politics.index()] = TableView::Unavailable {
        reason: "politics damaged".into(),
    };
    let groups = ideology_groups(&no_politics, &country_rows).unwrap();
    assert!(groups[&1].as_ref().unwrap_err().contains("politics damaged"));

    let mut no_countries = base.clone();
    *no_countries.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "countries damaged".into(),
    };
    let groups = ideology_groups(&no_countries, &country_rows).unwrap();
    assert!(groups[&1].as_ref().unwrap_err().contains("countries damaged"));
}
