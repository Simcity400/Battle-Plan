use super::*;
use replay_data::rows::{
    CountryLinkRow, PoliticalPartyRow, PoliticsRow, Recorded, TableId, TableRows,
};
use replay_frame::frame::TableView;
use std::sync::Mutex;
use viewer_foundation::map::{atlas_pages::CellPages, lazy_icons::FlagReader};

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

/// Independent copy of the prior country's flag inputs, not the extracted helper.
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
                        installed_content_country_names::country_names::original_tag(country, row)
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
                Some(installed_content_image::Rgba8Image {
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let cold = Stores::new();
    let warm = Stores::new();
    super::super::tests::with_counter_inputs_fixture(
        super::super::tests::action_gui(false),
        |a| cold.install(a),
        |_| {},
        |old| {
            super::super::tests::with_counter_inputs_fixture(
                super::super::tests::action_gui(false),
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
                        assert_eq!(a.tags, b.tags);
                        assert_eq!(a.flag_tags, b.flag_tags);
                        assert_eq!(a.groups, b.groups);
                        for slot in [3, 1, 2, 99] {
                            assert_eq!(a.flag_keys(slot), b.flag_keys(slot));
                            assert_eq!(a.flag_refusal(slot), b.flag_refusal(slot));
                        }
                        // Both retain the normal hour warm, even after initial prewarm.
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
                            assert_eq!(a.rotation.to_bits(), b.rotation.to_bits());
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
                    // An unrelated GUI demand was not in any initial/hour candidates.
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let stores = Stores::new();
    super::super::tests::with_counter_inputs_fixture(
        super::super::tests::action_gui(false),
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
                        // Noncore omissions retain their field-local refusals, not
                        // an invented fatal table error or fallback flag key.
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
            // Skipped initial warming never removes the existing cold fallback.
            for flags in &stores.flags {
                assert!(flags.acquire(&["COLD"]).is_some());
            }
            assert_eq!(stores.trace().len(), 3);
        },
    );
}

#[test]
fn truncated_initial_appearance_keeps_present_country_candidates() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut source = frame(0);
    let available = rows::<tables::CountryAppearance>(&source).unwrap();
    *source.tables[TableId::CountryAppearance.index()] = TableView::Truncated {
        rows: TableRows::CountryAppearance(available.rows()[..1].to_vec()),
        missing: 3,
    };
    let a = reference(&source).unwrap();
    let b = countries(&source).unwrap();
    assert_eq!(a.flag_tags, b.flag_tags);
    for slot in [1, 2, 3] {
        assert_eq!(a.flag_keys(slot), b.flag_keys(slot));
        assert_eq!(a.flag_refusal(slot), b.flag_refusal(slot));
    }
    assert!(b.flag_keys(1).is_some());
    assert_eq!(b.flag_keys(2), None);
}
