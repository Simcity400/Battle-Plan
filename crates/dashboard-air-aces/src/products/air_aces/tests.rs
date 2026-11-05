use super::*;
use dashboard_core::test_fixtures::{CAMPAIGN, records, source};
use replay_data::{
    rows::AceDeath, rows::AirWingRow, rows::CountryLinkRow, rows::CountryRow,
    rows::LexicalTokenRow, rows::TableRows, rows::VariantDefinitionRow, rows::WingEquipmentRow,
};
use replay_frame::frame::TableView;

pub(super) fn ace(id: u64, alive: bool) -> AirAceRow {
    AirAceRow {
        id,
        country: 11,
        alive: u8::from(alive),
        name: b"Mod name".to_vec(),
        surname: b"Surname".to_vec(),
        callsign: b"Storm".to_vec(),
        ..Default::default()
    }
}
pub(super) fn frame(hour: i32, rows: Vec<AirAceRow>) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    frame.metrics.table_ns[TableId::AirAces.index()] = 1;
    frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    *frame.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(rows));
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 70,
            ..Default::default()
        }]));
    *frame.tables[TableId::WingEquipment.index()] =
        TableView::Complete(TableRows::WingEquipment(vec![WingEquipmentRow {
            wing: 70,
            variant: 80,
            amount_raw: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 80,
            display_name: b"First design".to_vec(),
            equipment_token: 50,
            ..Default::default()
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 50,
            literal: b"mod_aircraft".to_vec(),
        }]));
    frame
}
fn finish(scan: &mut Scan) {
    scan.close(scan.last_frame.unwrap());
    if scan.windows.is_empty() {
        scan.windows.push(HourSpan {
            first_hour: scan.first,
            last_hour: scan.last,
        });
    }
}
fn params(view: AirAcesView, window: Option<usize>) -> AirAcesParams {
    AirAcesParams {
        branch_id: "1".into(),
        view,
        window,
    }
}
fn history(scan: &Scan) -> AirAcesWindowReply {
    bounded(
        window_reply(
            scan,
            &params(AirAcesView::Window, Some(0)),
            None,
            &AtomicBool::new(false),
        )
        .unwrap(),
        "window",
    )
    .unwrap()
}
fn info(scan: &Scan) -> AirAcesManifestReply {
    bounded(
        manifest(
            scan,
            &params(AirAcesView::Manifest, None),
            None,
            &AtomicBool::new(false),
        )
        .unwrap(),
        "manifest",
    )
    .unwrap()
}
fn manifest_result(reply: AirAcesReply) -> AirAcesManifestReply {
    let AirAcesReply::Manifest(reply) = reply else {
        panic!("expected manifest")
    };
    reply
}

#[test]
fn air_aces_dates_are_bounds_and_first_seen_dead_stays_unknown() {
    let mut scan = Scan {
        first: 100,
        last: 149,
        ..Default::default()
    };
    scan.observe(&frame(100, vec![ace(1, true), ace(2, false)]));
    let mut carry = frame(101, vec![ace(1, true), ace(2, false)]);
    carry.metrics.table_ns[TableId::AirAces.index()] = 0;
    scan.observe(&carry);
    let mut dead = ace(1, false);
    dead.death = Some(AceDeath {
        killer_ace: 3,
        killer_country: 11,
    });
    scan.observe(&frame(124, vec![dead.clone(), ace(2, false), ace(3, true)]));
    scan.observe(&frame(148, vec![dead, ace(2, false), ace(3, true)]));
    finish(&mut scan);
    let reply = history(&scan);
    assert_eq!(reply.aces[0].first_seen, Some(100));
    assert_eq!(
        info(&scan).observed,
        vec![
            HourSpan {
                first_hour: 100,
                last_hour: 101
            },
            HourSpan {
                first_hour: 124,
                last_hour: 124
            },
            HourSpan {
                first_hour: 148,
                last_hour: 148
            }
        ]
    );
    let death = reply.aces[0].runs[1].death.as_ref().unwrap();
    assert_eq!((death.last_alive, death.by_hour), (Some(100), Some(124)));
    assert_eq!(death.killer_ace.as_deref(), Some("3"));
    assert_eq!(
        reply.aces[0].runs[2].death.as_ref().unwrap().by_hour,
        Some(124)
    );
    let death = reply.aces[1].runs[0].death.as_ref().unwrap();
    assert_eq!((death.last_alive, death.by_hour), (None, None));
    assert_eq!(scan.reads, [100, 124, 148]);
}

#[test]
fn air_aces_missing_reads_and_disappearance_end_runs_no_backfill() {
    let mut scan = Scan {
        first: 100,
        last: 105,
        ..Default::default()
    };
    scan.observe(&frame(100, vec![ace(1, true)]));
    let mut failed = frame(101, vec![]);
    *failed.tables[TableId::AirAces.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    scan.observe(&failed);
    let mut stale = frame(102, vec![ace(1, true)]);
    stale.metrics.table_ns[TableId::AirAces.index()] = 0;
    scan.observe(&stale);
    scan.observe(&frame(103, vec![ace(1, true)]));
    scan.observe(&frame(104, vec![]));
    finish(&mut scan);
    let reply = history(&scan);
    assert_eq!(
        reply.aces[0]
            .runs
            .iter()
            .map(|r| (r.first_hour, r.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103)]
    );
    assert!(scan.at(1, 101).is_none());
    assert!(scan.at(1, 104).is_none());
    assert_eq!(info(&scan).observed.last().unwrap().last_hour, 104);
}

#[test]
fn air_aces_aircraft_names_use_the_read_hour_and_unrecorded_wings_are_explicit() {
    let mut scan = Scan {
        first: 100,
        last: 102,
        ..Default::default()
    };
    let mut row = ace(1, true);
    row.assigned_wing = 70;
    scan.observe(&frame(100, vec![row.clone()]));
    let mut later = frame(101, vec![row.clone()]);
    *later.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 80,
            display_name: b"Renamed design".to_vec(),
            equipment_token: 50,
            ..Default::default()
        }]));
    scan.observe(&later);
    row.assigned_wing = 999;
    scan.observe(&frame(102, vec![row]));
    finish(&mut scan);
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        "l_english:\n",
    )]);
    let manifest = bounded(
        super::manifest(
            &scan,
            &params(AirAcesView::Manifest, None),
            Some(&catalogue),
            &AtomicBool::new(false),
        )
        .unwrap(),
        "manifest",
    )
    .unwrap();
    let reply = history(&scan);
    let runs = &reply.aces[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!((runs[0].first_hour, runs[0].last_hour), (100, 101));
    assert_eq!(runs[0].aircraft.as_ref().unwrap(), &["80"]);
    assert_eq!(
        manifest.variants["80"]
            .iter()
            .map(|n| (n.first_hour, n.name.as_deref()))
            .collect::<Vec<_>>(),
        [
            (100, Some("First design")),
            (101, Some("Renamed design")),
            (102, Some("First design"))
        ]
    );
    assert_eq!(runs[1].wing.as_deref(), Some("999"));
    assert!(runs[1].aircraft.is_none());
    assert!(runs[0].wing_name.is_none());
}

#[test]
fn air_aces_product_caches_branch_and_serves_all_campaign_reads() {
    let mut rows = vec![ace(1, true)];
    rows[0].assigned_wing = 70;
    let branches = (1..=6)
        .map(|branch| {
            let mut first = frame(100, rows.clone());
            let mut last = frame(820, rows.clone());
            first.branch = branch;
            last.branch = branch;
            records(branch, vec![first, last])
        })
        .collect::<Vec<_>>();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &branches).unwrap());
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "aces".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        replay,
        &flag,
    )
    .unwrap();
    let first = cached(&source, 1, &flag).unwrap();
    assert!(Arc::ptr_eq(&first, &cached(&source, 1, &flag).unwrap()));
    for branch in 2..=6 {
        cached(&source, branch, &flag).unwrap();
    }
    let second = cached(&source, 1, &flag).unwrap();
    assert!(
        !Arc::ptr_eq(&first, &second),
        "evicted branches rescan instead of becoming unavailable"
    );
    assert_eq!(source.state::<Cache>().lock().unwrap().branches.len(), 2);
    let result = manifest_result(
        AirAces::load(
            &source,
            &AirAcesParams {
                branch_id: "1".into(),
                view: AirAcesView::Manifest,
                window: None,
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap(),
    );
    assert_eq!((result.first_hour, result.last_hour), (100, 820));
    assert_eq!(result.aces[0].first_seen, 100);
    assert_eq!(result.read_hours, [100, 820]);
    assert_eq!(
        result.windows,
        source
            .replay()
            .index
            .iter()
            .filter(|block| block.branch == 1)
            .map(|block| HourSpan {
                first_hour: block.first_hour,
                last_hour: block.last_hour
            })
            .collect::<Vec<_>>()
    );
    let window = result
        .windows
        .iter()
        .position(|span| span.last_hour == 820)
        .unwrap();
    let AirAcesReply::Window(rows) = AirAces::load(
        &source,
        &params(AirAcesView::Window, Some(window)),
        &LoadContext { content: None },
        &flag,
    )
    .unwrap() else {
        panic!("expected window")
    };
    assert_eq!(rows.aces[0].runs.last().unwrap().first_hour, 820);
    assert_eq!(
        rows.aces[0].runs.last().unwrap().aircraft.as_ref().unwrap(),
        &["80"]
    );
    let portraits = AirAcePortraits::load(
        &source,
        &AirAcePortraitParams {
            branch_id: "1".into(),
            hour: 100,
            ids: (1..=5000).map(|id| id.to_string()).collect(),
        },
        &LoadContext { content: None },
        &flag,
    )
    .unwrap();
    assert_eq!(portraits.portraits.len(), 5000);
}

#[test]
fn air_aces_mod_content_portrait_name_and_opaque_tag_override_through_stack() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use dashboard_text::catalogue::tests::{folder_catalogue, layers_catalogue};
    use installed_content_core::ResolvedLayers;
    let (base_cat, base) = folder_catalogue(&[(
        "interface/aces.gfx",
        "spriteTypes = { spriteType = { name = \"GFX_mod tag:Ω_ace_Mod name_Surname\" texturefile = \"missing.tga\" } }",
    )]);
    let (mod_cat, layer) = folder_catalogue(&[
        (
            "localisation/english/aces_l_english.yml",
            "\u{feff}l_english:\n ACE_FULL_NAME:0 \"$SUR$ / $NAME$\"\n",
        ),
        (
            "interface/aces.gfx",
            "spriteTypes = { spriteType = { name = \"GFX_mod tag:Ω_ace_Mod name_Surname\" texturefile = \"gfx/mod_ace.tga\" } }",
        ),
    ]);
    drop(mod_cat);
    let mut tga = vec![0_u8; 18];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 1;
    tga[16] = 32;
    tga[17] = 0x28;
    tga.extend_from_slice(&[10, 20, 30, 255]);
    std::fs::create_dir_all(layer.0.join("gfx")).unwrap();
    std::fs::write(layer.0.join("gfx/mod_ace.tga"), tga).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&base.0, &[][..]), (&layer.0, &[][..])]).unwrap();
    let cat = layers_catalogue(layers);
    let mut sample = frame(100, vec![ace(1, true)]);
    *sample.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: "mod tag:Ω".as_bytes().to_vec(),
            ..Default::default()
        }]));
    let mut scan = Scan {
        first: 100,
        last: 100,
        ..Default::default()
    };
    scan.observe(&sample);
    finish(&mut scan);
    let result = window_reply(
        &scan,
        &params(AirAcesView::Window, Some(0)),
        Some(&cat),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.aces[0].runs[0].tag.as_deref(), Some("mod tag:Ω"));
    assert_eq!(
        result.aces[0].runs[0]
            .name
            .map(|index| result.texts[index].as_str()),
        Some("Surname / Mod name")
    );
    let mut warnings = Refusals::default();
    assert!(
        ace_name(
            &scan.aces[0].runs[0].facts.row,
            Some(&base_cat),
            &mut warnings
        )
        .is_none()
    );
    assert!(
        warnings
            .counts("ace fields")
            .iter()
            .any(|why| why.contains("ACE_FULL_NAME absent"))
    );
    assert!(
        dashboard_text::artwork::url_bytes(
            &portrait(&PortraitOperands::from(&scan.aces[0].runs[0].facts), &cat).unwrap()
        )
        .is_some()
    );
    let mut unknown = scan.aces[0].runs[0].facts.clone();
    unknown.row = AirAceRow {
        name: b"Other".to_vec(),
        ..unknown.row.clone()
    };
    assert!(
        portrait(&PortraitOperands::from(&unknown), &cat)
            .unwrap_err()
            .contains("Ace presentation capture failed")
    );
}

#[test]
fn air_aces_a_changed_ace_splits_only_its_own_run() {
    let mut sample = frame(100, vec![ace(1, true), ace(2, true)]);
    let mut scan = Scan {
        first: 100,
        last: 101,
        ..Default::default()
    };
    scan.observe(&sample);
    sample.hour = 101;
    let mut changed = ace(1, false);
    changed.name = b"Changed".to_vec();
    *sample.tables[TableId::AirAces.index()] =
        TableView::Complete(TableRows::AirAces(vec![changed, ace(2, true)]));
    scan.observe(&sample);
    finish(&mut scan);
    assert_eq!(scan.aces[0].runs.len(), 2);
    assert_eq!(scan.aces[1].runs.len(), 1);
    let reply = history(&scan);
    assert!(reply.aces[0].runs.iter().all(|run| run.name.is_none()));
    assert_eq!(scan.aces[0].runs[0].facts.row.name, b"Mod name");
    assert_eq!(scan.aces[0].runs[1].facts.row.name, b"Changed");
    assert!(reply.aces[0].runs[0].alive);
    assert!(!reply.aces[0].runs[1].alive);
}

fn scale_frame() -> DecodedFrame {
    let countries = 5000_u32;
    let mut sample = frame(100, vec![]);
    *sample.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=countries)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *sample.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=countries)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *sample.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(
        (1..=countries)
            .flat_map(|country| {
                (0..20).map(move |ordinal| AirAceRow {
                    id: u64::from(country) * 20 + u64::from(ordinal),
                    country,
                    ordinal,
                    alive: 1,
                    name: format!("Ace {ordinal}").into_bytes(),
                    surname: format!("Country {country}").into_bytes(),
                    ..Default::default()
                })
            })
            .collect(),
    ));
    sample
}

#[test]
fn air_aces_budget_refuses_and_invalid_portrait_requests_are_rejected() {
    let mut scan = Scan {
        first: 100,
        last: 100,
        ..Default::default()
    };
    let mut row = ace(1, true);
    row.callsign = vec![b'A'; MAX_PRODUCT_REPLY_BYTES];
    scan.observe(&frame(100, vec![row]));
    finish(&mut scan);
    let error = bounded(
        window_reply(
            &scan,
            &params(AirAcesView::Window, Some(0)),
            None,
            &AtomicBool::new(false),
        )
        .unwrap(),
        "window",
    )
    .unwrap_err();
    assert!(error.to_string().contains("MAX_PRODUCT_REPLY_BYTES"));
    assert!(
        AirAcePortraits::validate(&AirAcePortraitParams {
            branch_id: "1".into(),
            hour: 100,
            ids: (1..=5000).map(|id| id.to_string()).collect(),
        })
        .is_ok()
    );
    assert!(
        AirAcePortraits::validate(&AirAcePortraitParams {
            branch_id: "1".into(),
            hour: 100,
            ids: vec!["not a handle".into()]
        })
        .is_err()
    );
}

#[test]
fn air_aces_equal_alive_reads_still_bound_death_by_the_last_actual_read() {
    let mut scan = Scan {
        first: 100,
        last: 102,
        ..Default::default()
    };
    let mut sample = frame(100, vec![ace(1, true)]);
    scan.observe(&sample);
    sample.hour = 101;
    scan.observe(&sample);
    assert_eq!(scan.aces[0].runs.len(), 1);
    sample.hour = 102;
    *sample.tables[TableId::AirAces.index()] =
        TableView::Complete(TableRows::AirAces(vec![ace(1, false)]));
    scan.observe(&sample);
    finish(&mut scan);
    let result = history(&scan);
    let death = result.aces[0].runs.last().unwrap().death.as_ref().unwrap();
    assert_eq!((death.last_alive, death.by_hour), (Some(101), Some(102)));
}

#[test]
fn air_aces_killer_changes_do_not_split_the_victim_run() {
    let mut dead = ace(1, false);
    dead.death = Some(AceDeath {
        killer_ace: 2,
        killer_country: 11,
    });
    let mut killer = ace(2, true);
    let mut scan = Scan {
        first: 100,
        last: 101,
        ..Default::default()
    };
    scan.observe(&frame(100, vec![dead.clone(), killer.clone()]));
    killer.name = b"Later".to_vec();
    scan.observe(&frame(101, vec![dead, killer]));
    finish(&mut scan);
    let result = history(&scan);
    assert_eq!(result.aces[0].runs.len(), 1);
    assert_eq!(
        result.aces[0].runs[0]
            .death
            .as_ref()
            .unwrap()
            .killer_ace
            .as_deref(),
        Some("2")
    );
    assert_eq!(scan.aces[1].runs.len(), 2);
    assert_eq!(scan.aces[1].runs[0].facts.row.name, b"Mod name");
    assert_eq!(scan.aces[1].runs[1].facts.row.name, b"Later");
}

#[test]
fn air_aces_recorded_wing_names_preserve_cached_text_and_capture_refusals() {
    let mut sample = frame(
        100,
        vec![AirAceRow {
            assigned_wing: 70,
            ..ace(1, true)
        }],
    );
    let TableView::Complete(TableRows::AirWings(wings)) =
        &mut *sample.tables[TableId::AirWings.index()]
    else {
        unreachable!()
    };
    wings[0].name = replay_data::rows::Recorded::Value(Some(b"Ordinary cached wing".to_vec()));
    let mut warnings = Refusals::default();
    assert_eq!(
        dashboard_text::air_wing_names::recorded_names(
            &sample,
            &HashSet::from([70]),
            &mut warnings
        )[&70],
        b"Ordinary cached wing"
    );
    let TableView::Complete(TableRows::AirWings(wings)) =
        &mut *sample.tables[TableId::AirWings.index()]
    else {
        unreachable!()
    };
    wings[0].name = replay_data::rows::Recorded::Value(None);
    assert!(
        dashboard_text::air_wing_names::recorded_names(
            &sample,
            &HashSet::from([70]),
            &mut warnings
        )
        .is_empty()
    );
    assert!(
        warnings
            .counts("wing names")
            .iter()
            .any(|why| why.contains("capture failed"))
    );
}

#[test]
fn air_aces_stored_wing_names_show_their_plain_display_text() {
    let (cat, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/test_l_english.yml",
        "l_english:\n",
    )]);
    let mut row = ace(1, true);
    row.assigned_wing = 70;
    let mut sample = frame(100, vec![row]);
    let TableView::Complete(TableRows::AirWings(wings)) =
        &mut *sample.tables[TableId::AirWings.index()]
    else {
        unreachable!()
    };
    wings[0].name =
        replay_data::rows::Recorded::Value(Some("\u{11}YFirst\u{11}! wing".as_bytes().to_vec()));
    let mut scan = Scan {
        first: 100,
        last: 100,
        ..Default::default()
    };
    scan.observe(&sample);
    finish(&mut scan);
    let reply = window_reply(
        &scan,
        &params(AirAcesView::Window, Some(0)),
        Some(&cat),
        &AtomicBool::new(false),
    )
    .unwrap();
    let wing = reply.aces[0].runs[0].wing_name.unwrap();
    assert_eq!(reply.texts[wing], "First wing");
}

fn fixture_source(frames: Vec<DecodedFrame>) -> Source {
    Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "aces-r4".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn air_aces_picker_catalogue_skips_history_content_and_cache() {
    let flag = AtomicBool::new(false);
    let source = fixture_source(vec![
        frame(100, vec![ace(1, true)]),
        frame(820, vec![ace(2, true)]),
    ]);
    let result = manifest_result(
        AirAces::load(
            &source,
            &AirAcesParams {
                branch_id: "1".into(),
                view: AirAcesView::Catalogue,
                window: None,
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap(),
    );
    assert!(result.available);
    assert!(result.unavailable.is_none());
    assert_eq!((result.first_hour, result.last_hour), (100, 820));
    assert!(
        result.aces.is_empty()
            && result.variants.is_empty()
            && result.read_hours.is_empty()
            && result.observed.is_empty()
    );
    assert!(result.warnings.is_empty(), "picker needs no content");
    assert!(source.state::<Cache>().lock().unwrap().branches.is_empty());
    assert!(json_len(&result, MAX_PRODUCT_REPLY_BYTES).unwrap() < 300);

    let mut failed = frame(100, vec![]);
    *failed.tables[TableId::AirAces.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    let source = fixture_source(vec![failed]);
    let result = manifest_result(
        AirAces::load(
            &source,
            &AirAcesParams {
                branch_id: "1".into(),
                view: AirAcesView::Catalogue,
                window: None,
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap(),
    );
    assert!(!result.available);
    assert_eq!(
        result.unavailable.as_deref(),
        Some("AirAces was unavailable, incomplete or never read in this recording.")
    );
}

#[test]
fn air_aces_picker_unread_block_is_unknown_instead_of_absent() {
    let flag = AtomicBool::new(false);
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(100, vec![])])]).unwrap();
    assert_eq!(
        recording_available(&replay, 1, &flag).unwrap(),
        Some(true),
        "an empty complete roster is recorded"
    );
    replay.index[0].offset = u64::MAX;
    assert_eq!(recording_available(&replay, 1, &flag).unwrap(), None);
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "aces-unread".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let result = manifest_result(
        AirAces::load(
            &source,
            &AirAcesParams {
                branch_id: "1".into(),
                view: AirAcesView::Catalogue,
                window: None,
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap(),
    );
    assert!(!result.available);
    assert_eq!(
        result.unavailable.as_deref(),
        Some("AirAces availability could not be checked")
    );
    let result = manifest_result(
        AirAces::load(
            &source,
            &params(AirAcesView::Manifest, None),
            &LoadContext { content: None },
            &flag,
        )
        .unwrap(),
    );
    assert_eq!(
        result.unavailable.as_deref(),
        Some("AirAces availability could not be checked")
    );
    assert_eq!(
        result.windows.len(),
        1,
        "unread recording blocks remain in the manifest"
    );
}

#[test]
fn air_aces_duplicate_read_is_a_counted_gap_and_later_reads_recover() {
    let mut scan = Scan {
        first: 100,
        last: 103,
        ..Default::default()
    };
    scan.observe(&frame(100, vec![ace(1, true)]));
    let mut duplicate = frame(101, vec![ace(1, true), ace(1, false)]);
    scan.observe(&duplicate);
    duplicate.hour = 102;
    duplicate.metrics.table_ns[TableId::AirAces.index()] = 0;
    scan.observe(&duplicate);
    scan.observe(&frame(103, vec![ace(1, false)]));
    finish(&mut scan);
    let result = history(&scan);
    assert_eq!(info(&scan).read_hours, [100, 103]);
    assert_eq!(result.aces.len(), 1);
    assert_eq!(
        result.aces[0]
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103)]
    );
    assert!(scan.at(1, 101).is_none() && scan.at(1, 102).is_none());
    let death = result.aces[0].runs[1].death.as_ref().unwrap();
    assert_eq!((death.last_alive, death.by_hour), (Some(100), Some(103)));
    assert_eq!(
        info(&scan)
            .warnings
            .iter()
            .filter(|why| why.contains("Duplicate ace reference"))
            .collect::<Vec<_>>(),
        ["1 ace fields/reads not shown: Duplicate ace reference; read is a gap"]
    );
}

#[test]
fn air_aces_large_missing_country_links_and_names_have_counted_diagnostics() {
    let mut sample = scale_frame();
    *sample.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "links failed".into(),
    };
    let mut scan = Scan {
        first: 100,
        last: 100,
        ..Default::default()
    };
    scan.observe(&sample);
    finish(&mut scan);
    let result = history(&scan);
    assert_eq!(result.aces.len(), 100_000);
    assert!(
        result
            .aces
            .iter()
            .all(|ace| ace.runs[0].tag.is_none() && ace.runs[0].name.is_none())
    );
    assert_eq!(
        result.warnings,
        [
            "100000 ace fields/reads not shown: ACE_FULL_NAME content unavailable; name order unknown"
        ]
    );
    assert_eq!(
        info(&scan).warnings,
        ["5000 ace fields/reads not shown: Ace owner country link has no resolved recorded tag"]
    );
    assert!(json_len(&result, MAX_PRODUCT_REPLY_BYTES).is_some());
}

#[test]
fn air_aces_refusal_counts_only_assigned_wing_names() {
    let mut row = ace(1, true);
    row.assigned_wing = 70;
    let mut sample = frame(100, vec![row]);
    *sample.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(
        [70, 71]
            .into_iter()
            .map(|id| AirWingRow {
                id,
                name: replay_data::rows::Recorded::Value(None),
                ..Default::default()
            })
            .collect(),
    ));
    let mut scan = Scan {
        first: 100,
        last: 101,
        ..Default::default()
    };
    scan.observe(&sample);
    sample.hour = 101;
    let rows = sample.complete::<tables::AirAces>().unwrap().to_vec();
    *sample.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(rows));
    scan.observe(&sample);
    finish(&mut scan);
    let result = history(&scan);
    assert_eq!(
        result.warnings,
        ["1 ace fields/reads not shown: ACE_FULL_NAME content unavailable; name order unknown"]
    );
    let warnings = info(&scan).warnings;
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("1 ace wing names not shown"));
    assert!(warnings[0].contains("capture failed"));
    assert!(scan.refusals.counts("ace fields/reads").is_empty());
}

#[test]
fn air_aces_windows_carry_reads_clip_ends_and_date_changed_text() {
    use dashboard_text::catalogue::tests::folder_catalogue;
    let (cat, _folder) = folder_catalogue(&[(
        "localisation/english/aces_l_english.yml",
        "\u{feff}l_english:\n ACE_FULL_NAME:0 \"Pilot $SUR$, $NAME$\"\n",
    )]);
    let mut scan = Scan {
        first: 100,
        last: 105,
        windows: vec![
            HourSpan {
                first_hour: 100,
                last_hour: 102,
            },
            HourSpan {
                first_hour: 103,
                last_hour: 105,
            },
        ],
        ..Default::default()
    };
    let mut sample = frame(100, vec![ace(1, true), ace(2, true)]);
    scan.observe(&sample);
    for hour in 101..=103 {
        sample.hour = hour;
        sample.metrics.table_ns[TableId::AirAces.index()] = 0;
        scan.observe(&sample);
    }
    sample = frame(104, vec![ace(1, false), ace(2, true)]);
    let mut rows = sample.complete::<tables::AirAces>().unwrap().to_vec();
    rows[0].name = b"Changed".to_vec();
    *sample.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(rows));
    scan.observe(&sample);
    sample.hour = 105;
    sample.metrics.table_ns[TableId::AirAces.index()] = 0;
    scan.observe(&sample);
    finish(&mut scan);
    let before = window_reply(
        &scan,
        &params(AirAcesView::Window, Some(0)),
        Some(&cat),
        &AtomicBool::new(false),
    )
    .unwrap();
    let after = window_reply(
        &scan,
        &params(AirAcesView::Window, Some(1)),
        Some(&cat),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        (
            before.aces[0].runs[0].first_hour,
            before.aces[0].runs[0].last_hour
        ),
        (100, 102)
    );
    assert_eq!(
        before.aces[0].runs.len(),
        1,
        "future changes do not enter an earlier window"
    );
    let runs = &after.aces[0].runs;
    assert_eq!(
        runs.iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 103), (104, 105)]
    );
    assert_eq!(
        after.texts[runs[0].name.unwrap()],
        "Pilot Surname, Mod name"
    );
    assert_eq!(after.texts[runs[1].name.unwrap()], "Pilot Surname, Changed");
    assert_eq!(runs[0].callsign, runs[1].callsign);
    assert_eq!(
        runs[0].name, after.aces[1].runs[0].name,
        "equal text is interned across aces"
    );
    assert_eq!(after.texts.len(), 3);
    assert_eq!(runs[1].death.as_ref().unwrap().last_alive, Some(100));
    assert_eq!(info(&scan).read_hours, [100, 104]);
    assert!(
        window_reply(
            &scan,
            &params(AirAcesView::Window, Some(2)),
            None,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(AirAces::validate(&params(AirAcesView::Window, None)).is_err());
    assert!(AirAces::validate(&params(AirAcesView::Manifest, Some(0))).is_err());
}

#[test]
fn air_aces_generic_portrait_preserves_retained_pool_and_native_culture_holes() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use dashboard_text::catalogue::tests::folder_catalogue;
    use replay_data::{rows::AirAcePortraitInputs, rows::AirAcePresentation, rows::Recorded};
    let (cat, folder) = folder_catalogue(&[(
        "interface/aces.gfx",
        r#"
        spriteTypes = {
            spriteType = { name = "GFX_retained_ace" texturefile = "gfx/retained.tga" }
            spriteType = { name = "GFX_modculture_ace_f_0" texturefile = "gfx/culture.tga" }
            spriteType = { name = "GFX_modculture_ace_f_2" texturefile = "gfx/after_hole.tga" }
            spriteType = { name = "GFX_global_ace" texturefile = "gfx/global.tga" }
        }
    "#,
    )]);
    std::fs::create_dir_all(folder.0.join("gfx")).unwrap();
    for (name, color) in [("retained", 10), ("culture", 20), ("global", 30)] {
        let mut tga = vec![0_u8; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 32;
        tga[17] = 0x28;
        tga.extend_from_slice(&[color, 0, 0, 255]);
        std::fs::write(folder.0.join(format!("gfx/{name}.tga")), tga).unwrap();
    }
    let expected = |key: &str| {
        catalogue::sprite_frame_png(
            &cat.content.layers,
            cat.sprites.find(key).unwrap().unwrap(),
            cat.gui_decode.clone().unwrap(),
            0,
            96,
        )
        .unwrap()
    };
    let mut scan = Scan {
        first: 100,
        last: 100,
        ..Default::default()
    };
    scan.observe(&frame(100, vec![ace(1, true)]));
    let mut facts = scan.aces[0].runs[0].facts.clone();
    facts.tag = Some("MOD".into());
    let mut row = facts.row.clone();
    let mut fields = AirAcePresentation {
        gender: 7,
        pool_index: 1,
        portrait: Some(AirAcePortraitInputs {
            cached: Some(b"GFX_retained_ace".to_vec()),
            culture: Some(b"modculture".to_vec()),
            fallback: Some(b"GFX_global_ace".to_vec()),
        }),
        ..Default::default()
    };
    row.presentation = Recorded::Value(Some(fields.clone()));
    facts.row = row.clone();
    assert_eq!(
        portrait(&PortraitOperands::from(&facts), &cat).unwrap(),
        expected("GFX_retained_ace")
    );
    fields.portrait.as_mut().unwrap().cached = None;
    fields.portrait.as_mut().unwrap().fallback = None;
    row.presentation = Recorded::Value(Some(fields.clone()));
    facts.row = row.clone();
    // The missing f_1 ends the pool, so native 1 % 1 selects f_0, not f_2.
    assert_eq!(
        portrait(&PortraitOperands::from(&facts), &cat).unwrap(),
        expected("GFX_modculture_ace_f_0")
    );
    fields.gender = 0; // No male culture entries: use the recorded global pool.
    fields.portrait.as_mut().unwrap().fallback = Some(b"GFX_global_ace".to_vec());
    row.presentation = Recorded::Value(Some(fields.clone()));
    facts.row = row.clone();
    assert_eq!(
        portrait(&PortraitOperands::from(&facts), &cat).unwrap(),
        expected("GFX_global_ace")
    );
    fields.portrait.as_mut().unwrap().fallback = Some(b"GFX_undefined_ace".to_vec());
    row.presentation = Recorded::Value(Some(fields));
    facts.row = row;
    assert!(
        portrait(&PortraitOperands::from(&facts), &cat)
            .unwrap_err()
            .contains("absent from recorded content")
    );
}

/// The recording draws an ace from its retained pool sprite, A; a selected mod declares the
/// ace's own named sprite, B, which the ladder tries first and the mod cannot draw. The
/// recording's own choice, A, is drawn and named, on the first reply and again from the memo,
/// never left unavailable for want of the recording's B; without a selection nothing is named.
#[test]
fn an_ace_portrait_the_mod_cannot_draw_makes_the_recordings_own_choice_again() {
    use dashboard_base::test_fixtures::visual_mods::{
        Machine, RED, fallback_line, first_pixel, png, select,
    };
    let machine = Machine::new("ace_portrait_selection");
    machine.write(
        "interface/aces.gfx",
        b"spriteTypes = { spriteType = { name = GFX_retained_ace textureFile = gfx/retained.png } }",
    );
    machine.write("gfx/retained.png", &png(RED));
    machine.write_mod(
        "named",
        "interface/named.gfx",
        b"spriteTypes = { spriteType = { name = GFX_MOD_ace_Hans_Muster textureFile = gfx/named.png } }",
    );
    machine.write_mod("named", "gfx/named.png", b"not a png");
    let operands = PortraitOperands {
        tag: Some("MOD".into()),
        name: b"Hans".to_vec(),
        surname: b"Muster".to_vec(),
        fields: Ok(Some(PortraitSelection {
            gender: 0,
            pool_index: 0,
            portrait: Some(PortraitPool {
                cached: Some(b"GFX_retained_ace".to_vec()),
                culture: None,
                fallback: None,
            }),
        })),
    };
    let (source, mut options) = machine.open();
    select(&source, &mut options, &[]);
    let cat = catalogue::catalogue(&source, &options).unwrap();
    let (portrait, line) = fallback_line(|| cached_portrait(&source, &operands, &cat));
    assert_eq!(first_pixel(&portrait.unwrap()), RED);
    assert_eq!(line, None);
    select(&source, &mut options, &["named"]);
    for _ in 0..2 {
        let cat = catalogue::catalogue(&source, &options).unwrap();
        let (portrait, line) = fallback_line(|| cached_portrait(&source, &operands, &cat));
        assert_eq!(first_pixel(&portrait.unwrap()), RED, "the recording's own A");
        assert!(
            line.as_ref()
                .is_some_and(|line| line.contains("ace portrait MOD Hans Muster")),
            "{line:?}"
        );
    }
}
