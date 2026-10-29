use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, source};
use installed_content_core::map::{MapData, ProvinceDefinition, ProvinceKind};
use map_snow::winter::{SnowDefines, SnowRecord};
use replay::container::{Replay, ReplayStream};
use replay_data::rows::{Recorded, TableId, TableRows, WeatherDisplay, WeatherProvinceRow};
use replay_frame::frame::TableView;
use std::{
    cell::Cell,
    fs::{self, File, FileTimes, OpenOptions},
    io::Read,
    path::PathBuf,
};

struct Fixture {
    path: PathBuf,
    root: PathBuf,
    store: Arc<Store>,
    source: SourceKey,
    id: uuid::Uuid,
}
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let id = uuid::Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("hoi4-snow-export-{id}"));
        fs::create_dir(&root).unwrap();
        let path = root.join("recording.battleplan");
        fs::write(&path, bytes).unwrap();
        let source = SourceKey::capture(&path, &[]).unwrap();
        Self {
            path,
            root,
            source,
            store: Arc::new(Store::local(id).unwrap()),
            id,
        }
    }
    fn writer(&self) -> Writer {
        if self.store.saved_key().unwrap().is_some() {
            self.store.extend(&self.source).unwrap()
        } else {
            self.store.replace(self.source.clone()).unwrap()
        }
    }
    fn reader(&self, key: &Key, limits: Limits) -> Reader {
        Reader::new(
            Arc::new(Store::local(self.id).unwrap()),
            key.clone(),
            limits,
        )
    }
    fn simple() -> Self {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(1, 0)])]).unwrap();
        Self::new(replay.bytes())
    }
    fn session(&self) -> Session {
        let replay = Replay::open_path(&self.path).unwrap();
        let prefix = replay
            .index
            .iter()
            .map(|b| b.offset + b.length)
            .max()
            .unwrap();
        Session::from_inputs(
            &replay,
            Ok(inputs()),
            prefix,
            &self.store,
            &self.source,
            || Ok(()),
        )
        .unwrap()
    }
    fn packet(&self, key: &Key, suffix: &str, index: u64) -> Vec<u8> {
        let mut out = Vec::new();
        self.store
            .read(&self.source, &key.section(suffix), index)
            .unwrap()
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        out
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.store.clear().unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn inputs() -> Inputs {
    let map = MapData {
        width: 12,
        height: 8,
        province_ids: (0..96)
            .map(|at| 1 + ((at / 12 + at % 12) % 3) as u32)
            .collect(),
        definitions: [1, 2, 3, 9]
            .into_iter()
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
    };
    let mask: Vec<_> = (0..96).map(|at| at % 7 == 0).collect();
    Inputs {
        cells: SnowCells::new(&map, &mask).unwrap(),
        witness: Witness::default(),
        winter: Winter {
            defines: SnowDefines {
                per_province: [0.1, 0.2],
                total: [0.0, 1.0],
            },
            records: vec![
                vec![
                    SnowRecord {
                        province: 3,
                        weight: 0.3,
                    },
                    SnowRecord {
                        province: 1,
                        weight: 0.7,
                    },
                ],
                vec![SnowRecord {
                    province: 9,
                    weight: 1.0,
                }],
                vec![],
            ],
        },
    }
}
fn frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(1));
    let display = WeatherDisplay {
        width: 3,
        height: 2,
        refresh_interval: 25_000,
        refresh_identity: hour as u64,
        edges: (0..8).map(|n| (hour + n).rem_euclid(256) as u8).collect(),
    };
    let mut rows = vec![WeatherProvinceRow {
        id: 0,
        display: Recorded::Value(Some(display)),
        ..Default::default()
    }];
    for id in 1..=3 {
        rows.push(WeatherProvinceRow {
            id,
            snow: 10_000 + i64::from(hour.rem_euclid(10_000)),
            display_snow: (hour + id as i32).rem_euclid(256) as u8,
            display_mud: id as u8,
            ..Default::default()
        });
    }
    frame.tables[TableId::WeatherProvinces.index()] = if hour == -40 {
        TableView::Truncated {
            rows: TableRows::WeatherProvinces(rows),
            missing: 1,
        }
    } else {
        TableView::Complete(TableRows::WeatherProvinces(rows))
    }
    .into();
    frame
}
fn hit(reader: &Reader, branch: u64, hour: i32) -> Arc<SnowMud> {
    match reader.restore(branch, hour) {
        Restore::Hit(snow) => snow,
        other => panic!("expected hit at {branch}/{hour}: {other:?}"),
    }
}

#[test]
fn all_grid_gap_and_branch_endpoints_restore_the_live_builder_across_committed_blocks() {
    let range = |branch, first, last| (first..=last).map(|hour| frame(branch, hour)).collect();
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, range(1, -85, -41))]).unwrap();
    let second = first
        .append(&[continuation_records(&first, 1, range(1, -40, 7))])
        .unwrap();
    let third = second
        .append(&[continuation_records(&second, 1, range(1, 21, 83))])
        .unwrap();
    let replay = third.append(&[records(2, range(2, 101, 142))]).unwrap();
    assert!(replay.index.len() >= 4);
    let fixture = Fixture::new(replay.bytes());
    let session = fixture.session();
    let limits = session.inputs.as_ref().unwrap().limits();
    let mut stream = ReplayStream::new(&[TableId::Campaign, TableId::WeatherProvinces]).unwrap();
    let mut observed = Vec::new();
    while stream.cursor().next_block < replay.index.len() {
        let mut writer = fixture.writer();
        session.initialise(&mut writer).unwrap();
        stream
            .next_block(&replay, |frame| {
                session.observe(&mut writer, frame, if frame.branch == 1 { 83 } else { 142 })?;
                observed.push(frame.clone());
                Ok(())
            })
            .unwrap();
        writer.commit().unwrap();
        let reader = fixture.reader(&session.key, limits);
        for frame in &observed {
            let exported = [
                (-85, 1),
                (-80, 1),
                (-40, 1),
                (0, 1),
                (21, 1),
                (40, 1),
                (80, 1),
                (83, 1),
                (101, 2),
                (120, 2),
                (142, 2),
            ]
            .contains(&(frame.hour, frame.branch));
            if (frame.branch, frame.hour) == (1, -40) {
                assert!(matches!(
                    reader.restore(1, -40),
                    Restore::Miss(reason)
                        if reason == "snow unavailable: truncated at keyframe: 1 rows not read"
                ));
                let input = session.inputs.as_ref().unwrap();
                let live = input.build(frame).unwrap();
                assert!(live.province_snow.iter().all(Option::is_none));
                assert!(!live.summary.complete);
                let shown = reader
                    .build_or_restore(1, -40, || input.build(frame).map(Arc::new))
                    .unwrap();
                assert_eq!(&*shown, &live);
            } else if exported {
                assert_eq!(
                    &*hit(&reader, frame.branch, frame.hour),
                    &session.inputs.as_ref().unwrap().build(frame).unwrap()
                );
            } else {
                assert!(matches!(
                    reader.restore(frame.branch, frame.hour),
                    Restore::Miss(_)
                ));
            }
        }
        assert!(matches!(reader.restore(1, 8), Restore::Miss(_)));
    }
    let reader = fixture.reader(&session.key, limits);
    assert_eq!(
        reader.coverage(1).unwrap(),
        Some(Coverage {
            endpoints: 8,
            first: -85,
            last: 83
        })
    );
    assert_eq!(
        reader.coverage(2).unwrap(),
        Some(Coverage {
            endpoints: 3,
            first: 101,
            last: 142
        })
    );
}

#[test]
fn equal_products_intern_by_digest_then_bytes_without_erasing_endpoint_provenance() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let limits = input.limits();
    let snow = input.build(&frame(1, 0)).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    for (branch, hour) in [(1, 0), (1, 40), (2, -40)] {
        write_endpoint(
            &mut writer,
            &session.key,
            branch,
            hour,
            Ok(&snow),
            Some(limits),
        )
        .unwrap();
    }
    writer.commit().unwrap();
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.source, &session.key.section("layer/1"))
            .unwrap(),
        Some(1)
    );
    let reader = fixture.reader(&session.key, limits);
    for (branch, hour) in [(1, 0), (1, 40), (2, -40)] {
        assert_eq!(&*hit(&reader, branch, hour), &snow);
    }
}

#[test]
fn loaded_operand_keys_ignore_irrelevant_and_overridden_edits_but_reload_effective_changes() {
    use crate::test_fixtures::snow_content;
    let fixture = Fixture::simple();
    let a = fixture.root.join("a");
    let b = fixture.root.join("b");
    for root in [&a, &b] {
        snow_content::write(root);
    }
    let loaded = |reverse: bool| {
        let mut layers = if reverse {
            ResolvedLayers::from_folders(&[(&b, &[][..]), (&a, &[][..])])
        } else {
            ResolvedLayers::from_folders(&[(&a, &[][..]), (&b, &[][..])])
        }
        .unwrap();
        for (layer, id) in
            layers
                .layers
                .iter_mut()
                .zip(if reverse { ["b", "a"] } else { ["a", "b"] })
        {
            layer.id = id.into();
        }
        let inputs = Inputs::load(&layers).unwrap();
        Key::for_loaded_inputs(&inputs, 1)
            .unwrap()
            .bind_recording(&Replay::open_path(&fixture.path).unwrap(), || Ok(()))
            .unwrap()
    };
    let original = loaded(false);
    let reordered = loaded(true);
    assert_eq!(original.inputs, reordered.inputs);
    assert_ne!(
        original.content, reordered.content,
        "the ordered resolving layer roster is witnessed"
    );
    fs::write(
        a.join("common/defines/snow.lua"),
        snow_content::DEFINES.replace("0.1", "0.4"),
    )
    .unwrap();
    fs::write(b.join("map/irrelevant.txt"), b"not a loaded snow operand").unwrap();
    assert_eq!(
        original,
        loaded(false),
        "unread overridden and irrelevant bytes cannot invalidate"
    );
    assert_ne!(
        original,
        loaded(true),
        "mod order changes the actual winning operands"
    );
    let file = b.join("common/defines/snow.lua");
    let metadata = fs::metadata(&file).unwrap();
    fs::write(&file, snow_content::DEFINES.replace("0.1", "0.2")).unwrap();
    File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    assert_eq!(fs::metadata(&file).unwrap().len(), metadata.len());
    assert_eq!(
        fs::metadata(&file).unwrap().modified().unwrap(),
        metadata.modified().unwrap()
    );
    assert_ne!(
        original,
        loaded(false),
        "freshly reloaded f32 operand bits differ"
    );
    let before_mask = loaded(false);
    fs::write(
        b.join("common/terrain/snow.txt"),
        snow_content::TERRAIN.replace("perm_snow=yes", "perm_snow=no"),
    )
    .unwrap();
    assert_ne!(
        before_mask.inputs,
        loaded(false).inputs,
        "the reloaded permanent-snow composition changes the key"
    );
    let mut version = original.clone();
    version.producers.get_mut(&1).unwrap().version += 1;
    assert_ne!(original.digest(), version.digest());
    let failed =
        |reason: &str| Key::for_inputs(Err(&Status::Unavailable(reason.into())), 1).unwrap();
    assert_ne!(failed("missing volume"), failed("invalid volume"));
    assert_ne!(
        failed("missing volume"),
        Key::for_inputs(Err(&Status::Unsupported("missing volume".into())), 1).unwrap()
    );
    let missing = a.join("gfx/posteffect_volumes.txt");
    fs::remove_file(&missing).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&a, &[][..])]).unwrap();
    let error = Inputs::load(&layers).err().unwrap();
    let unavailable = failed(&error.to_string());
    fs::write(&missing, snow_content::VOLUMES).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&a, &[][..])]).unwrap();
    let inputs = Inputs::load(&layers).unwrap();
    assert_ne!(
        unavailable.inputs,
        Key::for_loaded_inputs(&inputs, 1).unwrap().inputs
    );
}

#[test]
fn dropped_or_rejected_transactions_reopen_at_the_last_committed_endpoint_prefix() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let limits = input.limits();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    session.observe(&mut writer, &frame(1, 0), 80).unwrap();
    writer.commit().unwrap();
    {
        let mut writer = fixture.writer();
        session.observe(&mut writer, &frame(1, 40), 80).unwrap();
    }
    let reader = fixture.reader(&session.key, limits);
    hit(&reader, 1, 0);
    assert!(matches!(reader.restore(1, 40), Restore::Miss(_)));
    let mut writer = fixture.writer();
    session.observe(&mut writer, &frame(1, 40), 80).unwrap();
    let metadata = fs::metadata(&fixture.path).unwrap();
    OpenOptions::new()
        .write(true)
        .open(&fixture.path)
        .unwrap()
        .set_times(
            FileTimes::new()
                .set_modified(metadata.modified().unwrap() + std::time::Duration::from_secs(10)),
        )
        .unwrap();
    assert!(writer.commit().is_err());
    File::options()
        .write(true)
        .open(&fixture.path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    let reader = fixture.reader(&session.key, limits);
    hit(&reader, 1, 0);
    assert_eq!(reader.coverage(1).unwrap().unwrap().endpoints, 1);
    assert!(matches!(reader.restore(1, 40), Restore::Miss(_)));
    assert!(fixture.store.extend(&fixture.source).is_err());
    let retry = SourceKey::capture(&fixture.path, &[]).unwrap();
    let mut writer = fixture.store.extend(&retry).unwrap();
    session.observe(&mut writer, &frame(1, 40), 80).unwrap();
    writer.commit().unwrap();
    fixture
        .source
        .refresh_saved_binding(&fixture.store.saved_key().unwrap().unwrap());
    assert!(fixture.store.extend(&fixture.source).is_err());
    hit(&fixture.reader(&session.key, limits), 1, 40);
}

#[test]
fn missing_corrupt_stale_and_unavailable_endpoints_use_the_real_viewer_fallback_adapter() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let limits = input.limits();
    let snow = input.build(&frame(1, 0)).unwrap();
    let calls = Cell::new(0);
    let run = |reader: &Reader| {
        reader
            .build_or_restore(1, 0, || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(snow.clone()))
            })
            .unwrap()
    };
    let reader = fixture.reader(&session.key, limits);
    assert_eq!(&*run(&reader), &snow);
    assert_eq!(calls.get(), 1);
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(&mut writer, &session.key, 1, 0, Ok(&snow), Some(limits)).unwrap();
    writer.commit().unwrap();
    assert_eq!(&*run(&reader), &snow);
    assert_eq!(calls.get(), 1);
    let mut stale = session.key.clone();
    stale.producers.get_mut(&1).unwrap().version += 1;
    assert_eq!(&*run(&fixture.reader(&stale, limits)), &snow);
    assert_eq!(calls.get(), 2);
    let mut packet = fixture.packet(&session.key, "endpoints/1", 0);
    packet[103] ^= 1;
    let mut writer = fixture.writer();
    writer
        .write_at(&session.key.records(1), 0, packet.as_slice())
        .unwrap();
    writer.commit().unwrap();
    assert_eq!(&*run(&reader), &snow);
    assert_eq!(calls.get(), 3);
    let body = product_codec::encode(&Manifest {
        endpoint: 1,
        layers: [(
            SNOW,
            LayerStatus::Unavailable {
                reason: "recorded weather unavailable".into(),
            },
        )]
        .into_iter()
        .collect(),
        tables: Default::default(),
    })
    .unwrap();
    let packet = codec::pack(&session.key, 1, 0, 0, &body).unwrap();
    let mut writer = fixture.writer();
    writer
        .write_at(&session.key.records(1), 0, packet.as_slice())
        .unwrap();
    writer.commit().unwrap();
    assert_eq!(&*run(&reader), &snow);
    assert_eq!(calls.get(), 4);
    assert!(
        reader
            .build_or_restore(1, 40, || Err(invalid(
                "live failure is not a gap or empty layer"
            )))
            .is_err()
    );
}

#[test]
fn codec_rejects_unadmitted_lengths_dimensions_and_decoded_or_compressed_trailing_bytes() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let limits = input.limits();
    let snow = input.build(&frame(1, 0)).unwrap();
    let bytes = codec::encode_product(&snow, limits).unwrap();
    assert_eq!(codec::decode_product(&bytes, limits).unwrap(), snow);
    let mut wrong = bytes.clone();
    wrong[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(codec::decode_product(&wrong, limits).is_err());
    let mut wrong = bytes.clone();
    wrong[0..4].copy_from_slice(&99u32.to_le_bytes());
    assert!(codec::decode_product(&wrong, limits).is_err());
    let mut wrong = bytes.clone();
    wrong.push(0);
    assert!(codec::decode_product(&wrong, limits).is_err());
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    let mut trailing = codec::pack(&session.key, 1, 0, 3, &bytes).unwrap();
    trailing.push(0);
    let n = trailing.len() - 135;
    trailing[63..71].copy_from_slice(&(n as u64).to_le_bytes());
    let checksum = codec::digest(&trailing[..103]);
    trailing[103..135].copy_from_slice(&checksum);
    writer.write("bad-compressed", trailing.as_slice()).unwrap();
    let mut oversized = codec::pack(&session.key, 1, 0, 3, &bytes).unwrap();
    oversized[55..63].copy_from_slice(&u64::MAX.to_le_bytes());
    let checksum = codec::digest(&oversized[..103]);
    oversized[103..135].copy_from_slice(&checksum);
    writer.write("bad-length", oversized.as_slice()).unwrap();
    writer.commit().unwrap();
    for section in ["bad-compressed", "bad-length"] {
        let range = fixture
            .store
            .read(&fixture.source, section, 0)
            .unwrap()
            .unwrap();
        assert!(
            codec::unpack(
                range,
                &session.key,
                Some((1, 0)),
                3,
                limits.maximum().unwrap()
            )
            .is_err()
        );
    }
}

#[test]
fn statuses_keep_exact_names_and_weather_unavailability_is_not_an_available_zero_layer() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let limits = inputs().limits();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    let unavailable = "weather sample refused: missing province operand";
    let unsupported = "snow export: named synthetic unsupported volume";
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        -40,
        Err(Status::Unavailable(unavailable.into())),
        Some(limits),
    )
    .unwrap();
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        0,
        Err(Status::Unsupported(unsupported.into())),
        Some(limits),
    )
    .unwrap();
    let mut absent = frame(2, 40);
    absent.tables[TableId::WeatherProvinces.index()] = TableView::Unavailable {
        reason: "weather table unavailable".into(),
    }
    .into();
    session.observe(&mut writer, &absent, 40).unwrap();
    let mut unrecorded = frame(3, 80);
    unrecorded.tables[TableId::WeatherProvinces.index()] = TableView::NotRecorded {
        mode: replay_data::scope::CaptureMode::Full,
    }
    .into();
    session.observe(&mut writer, &unrecorded, 80).unwrap();
    writer.commit().unwrap();
    for (branch, index, hour, kind, expected) in [
        (1, 0, -40, 1, unavailable),
        (1, 1, 0, 2, unsupported),
        (2, 0, 40, 1, "weather table unavailable"),
        (3, 0, 80, 1, "WeatherProvinces not recorded in Full"),
    ] {
        let range = fixture
            .store
            .read(&fixture.source, &session.key.records(branch), index)
            .unwrap()
            .unwrap();
        let body = codec::unpack(range, &session.key, Some((branch, hour)), 0, usize::MAX).unwrap();
        let manifest: Manifest = product_codec::decode(&body).unwrap();
        let reason = match &manifest.layers[&SNOW] {
            LayerStatus::Unavailable { reason } if kind == 1 => reason,
            LayerStatus::Unsupported { reason } if kind == 2 => reason,
            status => panic!("wrong manifest status: {status:?}"),
        };
        assert_eq!(reason, expected);
        match fixture.reader(&session.key, limits).restore(branch, hour) {
            Restore::Miss(reason) => assert!(reason.contains(expected), "{reason}"),
            Restore::Hit(_) => panic!("status must not fabricate available snow"),
        }
    }
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.source, &session.key.section("layer/1"))
            .unwrap(),
        Some(0)
    );
}

#[test]
fn wrong_provenance_dangling_and_corrupted_payloads_fall_back_without_changing_source_availability()
{
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let limits = input.limits();
    let snow = input.build(&frame(1, 0)).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(&mut writer, &session.key, 1, 0, Ok(&snow), Some(limits)).unwrap();
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, limits);
    let original = fixture.packet(&session.key, "endpoints/1", 0);
    let mut wrong_key = session.key.clone();
    wrong_key.inputs[0] ^= 1;
    let bytes = codec::encode_product(&snow, limits).unwrap();
    let mut manifest = Manifest {
        endpoint: 1,
        layers: [(
            SNOW,
            LayerStatus::Available {
                payload: Payload {
                    section: 1,
                    ordinal: 0,
                    digest: codec::digest(&bytes),
                    raw: bytes.len() as u64,
                    branch: 1,
                    hour: 0,
                },
            },
        )]
        .into_iter()
        .collect(),
        tables: Default::default(),
    };
    let reference = product_codec::encode(&manifest).unwrap();
    if let LayerStatus::Available { payload } = manifest.layers.get_mut(&SNOW).unwrap() {
        payload.ordinal = u64::MAX;
    }
    let dangling = product_codec::encode(&manifest).unwrap();
    let packets = [
        codec::pack(&session.key, 2, 0, 0, &reference).unwrap(),
        codec::pack(&session.key, 1, 40, 0, &reference).unwrap(),
        codec::pack(&wrong_key, 1, 0, 0, &reference).unwrap(),
        codec::pack(&session.key, 1, 0, 0, &dangling).unwrap(),
    ];
    for packet in packets {
        let mut writer = fixture.writer();
        writer
            .write_at(&session.key.records(1), 0, packet.as_slice())
            .unwrap();
        writer.commit().unwrap();
        let calls = Cell::new(0);
        let result = reader
            .build_or_restore(1, 0, || {
                calls.set(calls.get() + 1);
                Ok(Arc::new(snow.clone()))
            })
            .unwrap();
        assert_eq!(&*result, &snow);
        assert_eq!(calls.get(), 1);
    }
    let mut payload = fixture.packet(&session.key, "layer/1", 0);
    let last = payload.len() - 1;
    payload[last] ^= 1;
    let mut writer = fixture.writer();
    writer
        .write_at(&session.key.records(1), 0, original.as_slice())
        .unwrap();
    writer
        .write_at(&session.key.section("layer/1"), 0, payload.as_slice())
        .unwrap();
    writer.commit().unwrap();
    assert!(matches!(reader.restore(1, 0), Restore::Miss(_)));
    let calls = Cell::new(0);
    assert_eq!(
        &*reader
            .build_or_restore(1, 0, || {
                calls.set(1);
                Ok(Arc::new(snow.clone()))
            })
            .unwrap(),
        &snow
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn product_codec_preserves_optional_float_bits_and_refuses_every_unadmitted_typed_length() {
    let input = inputs();
    let limits = input.limits();
    let mut snow = input.build(&frame(1, 0)).unwrap();
    for factors in [
        None,
        Some(vec![Some(f32::from_bits(0x7fc0_0123)), Some(-0.0), None].into()),
        Some(vec![Some(f32::INFINITY), Some(0.0), Some(f32::NEG_INFINITY)].into()),
    ] {
        snow.post_factors = factors;
        let bytes = codec::encode_product(&snow, limits).unwrap();
        assert_eq!(codec::decode_product(&bytes, limits).unwrap(), snow);
    }
    let bytes = codec::encode_product(&snow, limits).unwrap();
    let mask_length = 16 + snow.texels.len();
    let province_length = mask_length + 8 + snow.unknown.len();
    let factor_length = province_length
        + 8
        + snow
            .province_snow
            .iter()
            .map(|word| 1 + usize::from(word.is_some()) * 8)
            .sum::<usize>()
        + 1;
    for offset in [mask_length, province_length, factor_length] {
        let mut bad = bytes.clone();
        bad[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(codec::decode_product(&bad, limits).is_err());
        let mut bad = bytes.clone();
        bad[offset..offset + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(codec::decode_product(&bad, limits).is_err());
    }
    for end in 0..bytes.len() {
        assert!(codec::decode_product(&bytes[..end], limits).is_err());
    }
}

#[test]
fn append_inheritance_rebinds_only_a_freshly_verified_source_prefix_without_rewriting_records() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (0..=43).map(|hour| frame(1, hour)).collect())],
    )
    .unwrap();
    let mut fixture = Fixture::new(first.bytes());
    let session = fixture.session();
    let limits = inputs().limits();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    for hour in 0..=43 {
        session.observe(&mut writer, &frame(1, hour), 43).unwrap();
    }
    writer.commit().unwrap();
    let original = fixture.packet(&session.key, "endpoints/1", 0);
    let original_payload = fixture.packet(&session.key, "layer/1", 0);
    let appended = first
        .append(&[continuation_records(
            &first,
            1,
            (44..=83).map(|hour| frame(1, hour)).collect(),
        )])
        .unwrap();
    let previous = fixture.source.clone();
    fs::write(&fixture.path, appended.bytes()).unwrap();
    fixture.source = SourceKey::capture(&fixture.path, &[]).unwrap();
    let next = fixture.session();
    assert_ne!(session.key.recording, next.key.recording);
    assert_eq!(session.key.digest(), next.key.digest());
    assert!(
        next.can_inherit(
            &fixture.store,
            &fixture.source,
            [(
                1,
                Some(Cursor {
                    hour: 43,
                    block: None
                })
            )],
            || Ok(())
        )
        .unwrap()
    );
    {
        let mut writer = fixture
            .store
            .append(&previous, fixture.source.clone())
            .unwrap();
        next.inherit(&mut writer).unwrap();
        assert_eq!(writer.pending_range_ownership(), (0, 1));
    }
    let old = fixture
        .store
        .read_prefix(
            &previous,
            &fixture.source,
            &session.key.section("binding"),
            0,
        )
        .unwrap()
        .unwrap();
    assert_eq!(binding::Binding::read(old).unwrap().key, session.key);
    let mut writer = fixture
        .store
        .append(&previous, fixture.source.clone())
        .unwrap();
    next.inherit(&mut writer).unwrap();
    for hour in 44..=83 {
        next.observe(&mut writer, &frame(1, hour), 83).unwrap();
    }
    writer.commit().unwrap();
    assert_eq!(fixture.packet(&next.key, "endpoints/1", 0), original);
    assert_eq!(fixture.packet(&next.key, "layer/1", 0), original_payload);
    let reader = fixture.reader(&next.key, limits);
    for hour in [0, 40, 43, 80, 83] {
        assert_eq!(
            &*hit(&reader, 1, hour),
            &inputs().build(&frame(1, hour)).unwrap()
        );
    }
    assert!(matches!(
        fixture.reader(&session.key, limits).restore(1, 0),
        Restore::Miss(_)
    ));
    let mut changed = appended.bytes().to_vec();
    changed[appended.index[0].offset as usize] ^= 1;
    fs::write(&fixture.path, changed).unwrap();
    fixture.source = SourceKey::capture(&fixture.path, &[]).unwrap();
    let current = fixture.session();
    assert!(
        !current
            .can_inherit(
                &fixture.store,
                &fixture.source,
                [(
                    1,
                    Some(Cursor {
                        hour: 83,
                        block: None
                    })
                )],
                || Ok(())
            )
            .unwrap()
    );
}

#[test]
fn append_pending_index_ownership_is_constant_as_committed_endpoint_history_grows() {
    for endpoints in [1, 8, 64] {
        let first = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                (0..endpoints)
                    .map(|index| frame(1, index * STRIDE))
                    .collect(),
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(first.bytes());
        let session = fixture.session();
        let mut writer = fixture.writer();
        session.initialise(&mut writer).unwrap();
        for index in 0..endpoints {
            let hour = index * STRIDE;
            let snow = inputs().build(&frame(1, hour)).unwrap();
            write_endpoint(
                &mut writer,
                &session.key,
                1,
                hour,
                Ok(&snow),
                Some(inputs().limits()),
            )
            .unwrap();
        }
        writer.commit().unwrap();
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.records(1))
                .unwrap(),
            Some(endpoints as u64)
        );
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.section("layer/1"))
                .unwrap(),
            Some(endpoints as u64)
        );
        let appended = first
            .append(&[continuation_records(
                &first,
                1,
                vec![frame(1, (endpoints - 1) * STRIDE + 1)],
            )])
            .unwrap();
        let previous = fixture.source.clone();
        fs::write(&fixture.path, appended.bytes()).unwrap();
        fixture.source = SourceKey::capture(&fixture.path, &[]).unwrap();
        let next = fixture.session();
        assert!(
            next.can_inherit(
                &fixture.store,
                &fixture.source,
                [(
                    1,
                    Some(Cursor {
                        hour: (endpoints - 1) * STRIDE,
                        block: None
                    })
                )],
                || Ok(())
            )
            .unwrap()
        );
        let mut writer = fixture
            .store
            .append(&previous, fixture.source.clone())
            .unwrap();
        next.inherit(&mut writer).unwrap();
        assert_eq!(
            writer.pending_range_ownership(),
            (0, 1),
            "history={endpoints}"
        );
        assert_eq!(
            writer.bytes_written(),
            binding::Binding {
                key: next.key.clone(),
                prefix_length: next.prefix_length,
                prefix_digest: next.prefix_digest
            }
            .encode()
            .len() as u64
        );
        writer.commit().unwrap();
        hit(
            &fixture.reader(&next.key, inputs().limits()),
            1,
            (endpoints - 1) * STRIDE,
        );
    }
}

#[test]
fn malformed_unreferenced_interning_candidates_are_skipped_instead_of_failing_new_endpoints() {
    for body_damage in [false, true] {
        let fixture = Fixture::simple();
        let session = fixture.session();
        let snow = inputs().build(&frame(1, 0)).unwrap();
        let limits = inputs().limits();
        let mut packet = codec::pack(
            &session.key,
            1,
            0,
            3,
            &codec::encode_product(&snow, limits).unwrap(),
        )
        .unwrap();
        if body_damage {
            let last = packet.len() - 1;
            packet[last] ^= 1;
        } else {
            packet[0] ^= 1;
        }
        let mut writer = fixture.writer();
        session.initialise(&mut writer).unwrap();
        writer
            .write(&session.key.section("layer/1"), packet.as_slice())
            .unwrap();
        let bytes = codec::encode_product(&snow, limits).unwrap();
        let digest = codec::digest(&bytes);
        let entry = index::Entry {
            digest,
            section: 1,
            ordinal: 0,
            raw_hint: bytes.len() as u32,
        }
        .bytes()
        .unwrap();
        writer
            .write(
                &session.key.section(&format!("index/{:02x}", digest[0])),
                entry.as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let mut writer = fixture.writer();
        write_endpoint(&mut writer, &session.key, 1, 0, Ok(&snow), Some(limits)).unwrap();
        writer.commit().unwrap();
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.section("layer/1"))
                .unwrap(),
            Some(2)
        );
        assert_eq!(&*hit(&fixture.reader(&session.key, limits), 1, 0), &snow);
    }
}

#[test]
fn recording_witness_uses_one_held_pass_for_full_and_both_prefixes() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 0), frame(1, 1)])],
    )
    .unwrap();
    let fixture = Fixture::new(replay.bytes());
    let held = Replay::open_path(&fixture.path).unwrap();
    let prefix = held.index[0].offset + held.index[0].length;
    let prior = prefix - 1;
    let mut calls = 0;
    let (full, current, old) = binding::witness(&held, prefix, Some(prior), || {
        calls += 1;
        Ok(())
    })
    .unwrap();
    let mut identity = replay.bytes().to_vec();
    let campaign_len = u32::from_le_bytes(identity[10..14].try_into().unwrap()) as usize;
    let identity_at = 14 + campaign_len;
    let identity_len =
        u32::from_le_bytes(identity[identity_at..identity_at + 4].try_into().unwrap()) as usize;
    let slot = identity_at + 4 + identity_len;
    identity[slot..slot + 32].fill(0);
    assert_ne!(codec::digest(&identity), codec::digest(replay.bytes()));
    assert_eq!(full, codec::digest(&identity));
    assert_eq!(current, codec::digest(&identity[..prefix as usize]));
    assert_eq!(old, Some(codec::digest(&identity[..prior as usize])));
    assert_eq!(calls, replay.bytes().len().div_ceil(64 * 1024) + 2);
    let error = binding::witness(&held, prefix, None, || {
        Err(io::Error::from(io::ErrorKind::Interrupted))
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn cache_rejections_do_not_swallow_source_or_storage_failures() {
    for kind in [io::ErrorKind::InvalidData, io::ErrorKind::UnexpectedEof] {
        assert!(!reuse_result(Err(io::Error::from(kind))).unwrap());
    }
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::Other,
        io::ErrorKind::OutOfMemory,
        io::ErrorKind::Interrupted,
    ] {
        assert_eq!(
            reuse_result(Err(io::Error::from(kind))).unwrap_err().kind(),
            kind
        );
    }
}

#[test]
fn background_binding_keeps_live_available_and_discards_late_source_epoch_completions() {
    use std::sync::mpsc;
    let fixture = Fixture::simple();
    let session = fixture.session();
    let limits = inputs().limits();
    let snow = inputs().build(&frame(1, 0)).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(&mut writer, &session.key, 1, 0, Ok(&snow), Some(limits)).unwrap();
    writer.commit().unwrap();
    let held = Arc::new(Replay::open_path(&fixture.path).unwrap());
    let background = Background::default();
    let (started, wait_started) = mpsc::channel();
    let (release, wait_release) = mpsc::channel();
    let reader = fixture.reader(&session.key, limits);
    let old = background
        .start(Arc::clone(&held), move |_, _| {
            started.send(()).unwrap();
            wait_release.recv().unwrap();
            Ok(reader)
        })
        .unwrap();
    wait_started.recv().unwrap();
    assert!(background.reader(&held).err().unwrap().contains("pending"));
    let calls = Cell::new(0);
    let live = || {
        calls.set(calls.get() + 1);
        Ok::<_, io::Error>(Arc::new(snow.clone()))
    };
    let shown = match background.reader(&held) {
        Ok(reader) => reader.build_or_restore(1, 0, live),
        Err(_) => live(),
    }
    .unwrap();
    assert_eq!(&*shown, &snow);
    assert_eq!(
        calls.get(),
        1,
        "pending binding does not block the live producer"
    );
    let newer = Arc::new(Replay::open_path(&fixture.path).unwrap());
    let mut stale = session.key.clone();
    stale.producers.get_mut(&1).unwrap().version += 1;
    let reader = fixture.reader(&stale, limits);
    background
        .start(Arc::clone(&newer), move |_, _| Ok(reader))
        .unwrap()
        .join()
        .unwrap();
    assert!(background.reader(&held).is_err());
    assert!(matches!(
        background.reader(&newer).unwrap().restore(1, 0),
        Restore::Miss(_)
    ));
    release.send(()).unwrap();
    old.join().unwrap();
    assert!(
        matches!(
            background.reader(&newer).unwrap().restore(1, 0),
            Restore::Miss(_)
        ),
        "late old hit cannot replace the newer epoch's miss"
    );
}

#[test]
fn retiring_content_cancels_an_in_flight_background_bind() {
    use std::sync::mpsc;
    let fixture = Fixture::simple();
    let replay = Arc::new(Replay::open_path(&fixture.path).unwrap());
    let background = Background::default();
    let (started, wait_started) = mpsc::channel();
    let (release, wait_release) = mpsc::channel();
    let (cancelled, wait_cancelled) = mpsc::channel();
    let handle = background
        .start(replay, move |_, check| {
            started.send(()).unwrap();
            wait_release.recv().unwrap();
            let error = check().unwrap_err();
            cancelled.send(error.kind()).unwrap();
            Err(error)
        })
        .unwrap();
    wait_started.recv().unwrap();
    drop(background);
    release.send(()).unwrap();
    handle.join().unwrap();
    assert_eq!(wait_cancelled.recv().unwrap(), io::ErrorKind::Interrupted);
}

#[test]
fn reuse_and_append_validation_cancel_between_committed_available_endpoints() {
    for append in [false, true] {
        let first = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                (0..8).map(|index| frame(1, index * STRIDE)).collect(),
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(first.bytes());
        let session = fixture.session();
        let mut writer = fixture.writer();
        session.initialise(&mut writer).unwrap();
        for index in 0..8 {
            session
                .observe(&mut writer, &frame(1, index * STRIDE), 7 * STRIDE)
                .unwrap();
        }
        writer.commit().unwrap();
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.records(1))
                .unwrap(),
            Some(8)
        );
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.section("layer/1"))
                .unwrap(),
            Some(8)
        );
        let before = fixture.packet(&session.key, "endpoints/1", 0);
        if append {
            let replay = first
                .append(&[continuation_records(&first, 1, vec![frame(1, 8 * STRIDE)])])
                .unwrap();
            fs::write(&fixture.path, replay.bytes()).unwrap();
            fixture.source = SourceKey::capture(&fixture.path, &[]).unwrap();
        }
        let current = fixture.session();
        let validate = |check: &mut dyn FnMut() -> io::Result<()>| {
            if append {
                current.can_inherit(
                    &fixture.store,
                    &fixture.source,
                    [(
                        1,
                        Some(Cursor {
                            hour: 7 * STRIDE,
                            block: None,
                        }),
                    )],
                    check,
                )
            } else {
                current.reusable(
                    &fixture.store,
                    &fixture.source,
                    [(
                        1,
                        Some(Cursor {
                            hour: 7 * STRIDE,
                            block: None,
                        }),
                    )],
                    check,
                )
            }
        };
        let mut full_checks = 0;
        assert!(
            validate(&mut || {
                full_checks += 1;
                Ok(())
            })
            .unwrap()
        );
        assert!(
            full_checks > 8,
            "the control scans actual available products"
        );
        let cancel_at = full_checks / 2;
        let mut checks = 0;
        let error = validate(&mut || {
            checks += 1;
            if checks == cancel_at {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(checks, cancel_at);
        assert!(validate(&mut || Ok(())).unwrap());
        let saved = fixture.store.saved_key().unwrap().unwrap();
        let mut after = Vec::new();
        let range = if saved == fixture.source {
            fixture
                .store
                .read(&fixture.source, &session.key.records(1), 0)
        } else {
            fixture
                .store
                .read_prefix(&saved, &fixture.source, &session.key.records(1), 0)
        };
        range.unwrap().unwrap().read_to_end(&mut after).unwrap();
        assert_eq!(before, after);
    }
}

#[test]
fn zero_walk_snow_history_check_detects_payload_endpoint_and_cursor_without_a_binding() {
    for suffix in ["layer/1", "endpoints/1", "cursor/1"] {
        let fixture = Fixture::simple();
        let session = fixture.session();
        assert!(
            !session
                .has_history(&fixture.store, &fixture.source, [1], || Ok(()))
                .unwrap()
        );
        let mut writer = fixture.writer();
        writer
            .write_at(
                &session.key.section(suffix),
                0,
                b"damaged history".as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        assert!(!Session::ready(&fixture.store, &fixture.source, &session.key).unwrap());
        assert!(
            session
                .has_history(&fixture.store, &fixture.source, [1], || Ok(()))
                .unwrap(),
            "a zero cursor cannot bypass {suffix}"
        );
        assert_eq!(
            session
                .has_history(&fixture.store, &fixture.source, [1], || {
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                })
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }
}

#[test]
fn multi_layer_manifests_restore_live_snow_and_share_equal_bytes_globally() {
    let fixture = Fixture::simple();
    let mut session = fixture.session();
    let mut second = session.key.producers[&SNOW].clone();
    second.contract = b"synthetic/second-layer".to_vec();
    session.key.producers.insert(2, second);
    let input = inputs();
    let live = input.build(&frame(1, 0)).unwrap();
    let bytes = codec::encode_product(&live, input.limits()).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    let first = intern_layer(&mut writer, &session.key, SNOW, 1, 0, &bytes).unwrap();
    let second = intern_layer(&mut writer, &session.key, 2, 9, 80, &bytes).unwrap();
    let table = intern_table(&mut writer, &session.key, 17, 3, -40, &bytes).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, table);
    write_manifest(
        &mut writer,
        &session.key,
        9,
        80,
        &Manifest {
            endpoint: 3,
            layers: [
                (
                    SNOW,
                    LayerStatus::Available {
                        payload: first.clone(),
                    },
                ),
                (2, LayerStatus::Available { payload: second }),
            ]
            .into_iter()
            .collect(),
            tables: [(17, table)].into_iter().collect(),
        },
    )
    .unwrap();
    let operands = b"exact independent table bytes";
    let different = intern_table(&mut writer, &session.key, 19, 9, 80, operands).unwrap();
    assert_ne!(first, different);
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, input.limits());
    assert_eq!(&*hit(&reader, 9, 80), &live);
    let admitted = Headroom {
        available: Some((u64::MAX, u64::MAX)),
        low: false,
    };
    assert!(
        matches!(reader.layer(9, 80, 2, admitted).unwrap(), Some(LayerProduct::Available(restored)) if restored == bytes)
    );
    assert_eq!(reader.table(9, 80, 17, admitted).unwrap().unwrap(), bytes);
    assert!(reader.layer(9, 81, 2, admitted).unwrap().is_none());
    assert!(reader.table(9, 80, 18, admitted).unwrap().is_none());
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.source, &session.key.section("layer/1"))
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.source, &session.key.section("layer/2"))
            .unwrap(),
        Some(0)
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.source, &session.key.section("tables"))
            .unwrap(),
        Some(1)
    );
    assert!(
        session
            .reusable(
                &fixture.store,
                &fixture.source,
                [(
                    9,
                    Some(Cursor {
                        hour: 80,
                        block: None
                    })
                )],
                || Ok(())
            )
            .unwrap()
    );
}

#[test]
fn digest_collisions_compare_bytes_and_scan_only_one_fixed_record_at_a_time() {
    for count in [1_u32, 16, 257] {
        let fixture = Fixture::simple();
        let session = fixture.session();
        let mut writer = fixture.writer();
        session.initialise(&mut writer).unwrap();
        let forced = [0x61; 32];
        for value in 0..count {
            let bytes = value.to_le_bytes();
            let payload = index::intern_digest(
                &mut writer,
                &session.key,
                1,
                1,
                value as i32,
                &bytes,
                forced,
                &mut index::Scan::default(),
            )
            .unwrap();
            assert_eq!(payload.ordinal, u64::from(value));
        }
        let poison = b"not an index record";
        writer
            .write(&session.key.section("index/62"), poison.as_slice())
            .unwrap();
        let mut scan = index::Scan::default();
        let last = index::intern_digest(
            &mut writer,
            &session.key,
            1,
            2,
            40,
            &(count - 1).to_le_bytes(),
            forced,
            &mut scan,
        )
        .unwrap();
        assert_eq!(last.ordinal, u64::from(count - 1));
        assert_eq!(scan.bucket, Some(0x61));
        assert_eq!(scan.entries, u64::from(count));
        assert_eq!(scan.peak_index_bytes, 48);
        let unequal = index::intern_digest(
            &mut writer,
            &session.key,
            1,
            3,
            80,
            &count.to_le_bytes(),
            forced,
            &mut index::Scan::default(),
        )
        .unwrap();
        assert_eq!(unequal.ordinal, u64::from(count));
        writer.commit().unwrap();
        for value in 0..=count {
            let range = fixture
                .store
                .read(
                    &fixture.source,
                    &session.key.section("layer/1"),
                    u64::from(value),
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                codec::unpack(range, &session.key, None, 3, 4).unwrap(),
                value.to_le_bytes()
            );
        }
    }
}

#[test]
fn live_commit_physical_unknown_and_low_headroom_are_misses_not_missing_snow() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let expected = input.build(&frame(1, 0)).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        0,
        Ok(&expected),
        Some(input.limits()),
    )
    .unwrap();
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, input.limits());
    for headroom in [
        Headroom {
            available: Some((0, u64::MAX)),
            low: false,
        },
        Headroom {
            available: Some((u64::MAX, 0)),
            low: false,
        },
        Headroom {
            available: None,
            low: false,
        },
        Headroom {
            available: Some((u64::MAX, u64::MAX)),
            low: true,
        },
    ] {
        assert!(matches!(
            reader.restore_with_headroom(1, 0, headroom),
            Restore::Miss(_)
        ));
        let calls = Cell::new(0);
        let shown = reader
            .build_or_restore_admitted(1, 0, headroom, || {
                calls.set(calls.get() + 1);
                input.build(&frame(1, 0)).map(Arc::new)
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(&*shown, &expected);
    }
    let shown = reader
        .build_or_restore_admitted(
            1,
            0,
            Headroom {
                available: Some((u64::MAX, u64::MAX)),
                low: false,
            },
            || panic!("admitted verified endpoint must not build live"),
        )
        .unwrap();
    assert_eq!(&*shown, &expected);
}

#[test]
fn payload_envelope_has_135_bytes_and_rejects_wrong_key_branch_hour_kind_layer_and_trailing_frames()
{
    let fixture = Fixture::simple();
    let session = fixture.session();
    let short = [0x5a];
    let compressible = vec![0x3c; 2048];
    let mut writer = fixture.writer();
    let key = session.key.layer_digest(SNOW).unwrap();
    for (name, bytes, raw) in [
        ("raw", short.as_slice(), true),
        ("zstd", compressible.as_slice(), false),
    ] {
        let packet = codec::pack_for(key, 7, -40, 3, SNOW, bytes).unwrap();
        assert_eq!(&packet[..8], b"H4MAPE02");
        assert_eq!(&packet[8..40], &key);
        assert_eq!(&packet[40..48], &7_u64.to_le_bytes());
        assert_eq!(&packet[48..52], &(-40_i32).to_le_bytes());
        assert_eq!(packet[52], 3 | if raw { 0x80 } else { 0 });
        assert_eq!(&packet[53..55], &SNOW.to_le_bytes());
        assert_eq!(&packet[55..63], &(bytes.len() as u64).to_le_bytes());
        assert_eq!(
            u64::from_le_bytes(packet[63..71].try_into().unwrap()) as usize + 135,
            packet.len()
        );
        writer.write(name, packet.as_slice()).unwrap();
    }
    let mut trailing = codec::pack_for(key, 7, -40, 3, SNOW, &compressible).unwrap();
    trailing.extend_from_slice(&zstd::bulk::compress(b"extra frame", 3).unwrap());
    let compressed = trailing.len() as u64 - 135;
    trailing[63..71].copy_from_slice(&compressed.to_le_bytes());
    let checksum = codec::digest(&trailing[..103]);
    trailing[103..135].copy_from_slice(&checksum);
    writer.write("trailing-frame", trailing.as_slice()).unwrap();
    writer.commit().unwrap();
    for (name, expected) in [("raw", short.as_slice()), ("zstd", compressible.as_slice())] {
        let range = fixture
            .store
            .read(&fixture.source, name, 0)
            .unwrap()
            .unwrap();
        assert_eq!(
            codec::unpack_for(
                range.clone(),
                key,
                Some((7, -40)),
                3,
                SNOW,
                expected.len(),
                &mut 0
            )
            .unwrap(),
            expected
        );
        for (key, target, kind, layer) in [
            ([0; 32], (7, -40), 3, SNOW),
            (key, (8, -40), 3, SNOW),
            (key, (7, 0), 3, SNOW),
            (key, (7, -40), 4, SNOW),
            (key, (7, -40), 3, 2),
        ] {
            assert!(
                codec::unpack_for(
                    range.clone(),
                    key,
                    Some(target),
                    kind,
                    layer,
                    expected.len(),
                    &mut 0
                )
                .is_err()
            );
        }
    }
    let range = fixture
        .store
        .read(&fixture.source, "trailing-frame", 0)
        .unwrap()
        .unwrap();
    assert!(
        codec::unpack_for(
            range,
            key,
            Some((7, -40)),
            3,
            SNOW,
            compressible.len(),
            &mut 0
        )
        .is_err()
    );
}

fn witness_records(witness: &Witness) -> Vec<Vec<u8>> {
    let mut records = Vec::new();
    witness
        .visit(|_, bytes| {
            records.push(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    records
}

fn resolution_digest(records: &[Vec<u8>]) -> [u8; 32] {
    let mut bytes = witness::WITNESS_CONTRACT.to_vec();
    for record in records {
        bytes.extend_from_slice(&(record.len() as u64).to_le_bytes());
        bytes.extend_from_slice(record);
    }
    codec::digest(&bytes)
}

#[test]
fn content_witness_canonical_set_keeps_probes_bytes_exact_errors_and_layer_identity() {
    let fixture = Fixture::simple();
    let root = fixture.root.join("witness-content");
    fs::create_dir_all(root.join("map")).unwrap();
    fs::write(root.join("map/read.txt"), b"read bytes").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (result, witness) = Witness::capture(&layers, |layers| {
        assert!(!layers.has_file("map/absent.txt")?);
        assert_eq!(layers.read("map/read.txt")?, b"read bytes");
        assert!(layers.has_file("map/read.txt")?);
        layers.read("map/missing.txt")
    })
    .unwrap();
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "map/missing.txt");
    let records = witness_records(&witness);
    let entries: Vec<witness::Entry> = records
        .iter()
        .map(|bytes| product_codec::decode(bytes).unwrap())
        .collect();
    let layer = witness::Layer {
        index: 0,
        id: "test0".into(),
        kind: 0,
        replace_paths: vec![],
    };
    assert_eq!(
        entries,
        vec![
            witness::Entry::Roster {
                layers: vec![layer.clone()]
            },
            witness::Entry::Result {
                purpose: "".into(),
                status: 1,
                kind: "NotFound".into(),
                message: "map/missing.txt".into()
            },
            witness::Entry::Read {
                path: b"map/absent.txt".to_vec(),
                purpose: "".into(),
                layer: None,
                outcome: witness::Outcome::Probe { present: false }
            },
            witness::Entry::Read {
                path: b"map/missing.txt".to_vec(),
                purpose: "".into(),
                layer: None,
                outcome: witness::Outcome::Error {
                    kind: "NotFound".into(),
                    message: "map/missing.txt".into(),
                    payload: None
                }
            },
            witness::Entry::Read {
                path: b"map/read.txt".to_vec(),
                purpose: "".into(),
                layer: Some(layer.clone()),
                outcome: witness::Outcome::Bytes {
                    length: 10,
                    digest: codec::digest(b"read bytes")
                }
            },
            witness::Entry::Read {
                path: b"map/read.txt".to_vec(),
                purpose: "".into(),
                layer: Some(layer),
                outcome: witness::Outcome::Probe { present: true }
            },
        ]
    );
    assert_eq!(witness.entries, 6);
    assert_eq!(witness.digest, resolution_digest(&records));
    assert_eq!(
        Witness::default().digest,
        codec::digest(witness::WITNESS_CONTRACT)
    );
    let mut writer = fixture.writer();
    let mut key = fixture.session().key;
    key.content = witness.digest;
    key.content_entries = witness.entries;
    witness.publish(&mut writer, &key).unwrap();
    writer.commit().unwrap();
    let reopened = Store::local(fixture.id).unwrap();
    witness::verify(
        &key,
        |ordinal| reopened.read(&fixture.source, &key.section("witness"), ordinal),
        || Ok(()),
    )
    .unwrap();
    let mut writer = fixture.writer();
    writer
        .write_at(&key.section("witness"), 1, b"corrupt".as_slice())
        .unwrap();
    writer.commit().unwrap();
    assert!(
        witness::verify(
            &key,
            |ordinal| fixture
                .store
                .read(&fixture.source, &key.section("witness"), ordinal),
            || Ok(())
        )
        .is_err()
    );
}

#[test]
fn traced_parallel_load_completion_order_does_not_change_the_canonical_witness_or_first_error() {
    use std::sync::mpsc;
    let fixture = Fixture::simple();
    let root = fixture.root.join("parallel-witness");
    fs::create_dir_all(root.join("map")).unwrap();
    fs::write(root.join("map/shared.txt"), b"shared bytes").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let capture = |order: [usize; 2]| {
        let recorder = witness::Recorder::new(&layers);
        let log = recorder.clone();
        let traced = layers.with_read_trace(Arc::new(move |read| log.record_read(read)));
        let readers = [
            traced.with_read_purpose("long-reader"),
            traced.with_read_purpose("a"),
        ];
        let errors = std::thread::scope(|scope| {
            let (ready_tx, ready_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let mut releases = Vec::new();
            let mut handles = Vec::new();
            for (index, reader) in readers.iter().enumerate() {
                let (release, gate) = mpsc::channel();
                releases.push(release);
                let done = done_tx.clone();
                let ready = ready_tx.clone();
                let recorder = recorder.clone();
                handles.push(scope.spawn(move || {
                    assert!(reader.has_file("map/shared.txt").unwrap());
                    assert!(!reader.has_file("map/absent.txt").unwrap());
                    assert_eq!(reader.read("map/shared.txt").unwrap(), b"shared bytes");
                    ready.send(index).unwrap();
                    gate.recv().unwrap();
                    let missing = if index == 0 {
                        "map/a-long-first-missing.txt"
                    } else {
                        "map/z.txt"
                    };
                    let results =
                        reader.read_paths(&["map/shared.txt", missing, "map/second-missing.txt"]);
                    assert_eq!(results[0].as_ref().unwrap(), b"shared bytes");
                    let result: io::Result<()> = results
                        .into_iter()
                        .try_for_each(|result| result.map(|_| ()));
                    recorder.record_result(if index == 0 { "long-reader" } else { "a" }, &result);
                    done.send(index).unwrap();
                    result.unwrap_err()
                }));
            }
            let mut ready = [ready_rx.recv().unwrap(), ready_rx.recv().unwrap()];
            ready.sort_unstable();
            assert_eq!(ready, [0, 1]);
            for index in order {
                releases[index].send(()).unwrap();
                assert_eq!(done_rx.recv().unwrap(), index);
            }
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(errors[0].kind(), io::ErrorKind::NotFound);
        assert_eq!(errors[0].to_string(), "map/a-long-first-missing.txt");
        assert_eq!(errors[1].kind(), io::ErrorKind::NotFound);
        assert_eq!(errors[1].to_string(), "map/z.txt");
        recorder.snapshot().unwrap()
    };
    let first = capture([0, 1]);
    let second = capture([1, 0]);
    assert_eq!(first.digest, second.digest);
    assert_eq!(first.entries, second.entries);
    assert_eq!(witness_records(&first), witness_records(&second));
    let records = witness_records(&first);
    assert_eq!(first.digest, resolution_digest(&records));
    let reads: Vec<_> = records
        .iter()
        .filter_map(
            |bytes| match product_codec::decode::<witness::Entry>(bytes).unwrap() {
                witness::Entry::Read { path, purpose, .. } => Some((path, purpose)),
                _ => None,
            },
        )
        .collect();
    assert!(reads.windows(2).all(|pair| pair[0] <= pair[1]));
    let shared: Vec<_> = reads
        .iter()
        .filter(|(path, _)| path == b"map/shared.txt")
        .map(|(_, purpose)| purpose.as_str())
        .collect();
    assert_eq!(shared, ["a", "a", "long-reader", "long-reader"]);
}

#[test]
fn repeated_and_retained_reads_do_not_expand_or_change_the_resolution_set() {
    let fixture = Fixture::simple();
    let root = fixture.root.join("retained-witness");
    fs::create_dir_all(root.join("map")).unwrap();
    fs::create_dir_all(root.join("gfx/custom")).unwrap();
    fs::write(root.join("map/read.txt"), b"read bytes").unwrap();
    fs::write(root.join("gfx/custom/read.bin"), b"uncovered bytes").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let once = Witness::capture(&layers, |layers| layers.read("map/read.txt"))
        .unwrap()
        .1;
    let twice = Witness::capture(&layers, |layers| {
        layers.read("map/read.txt")?;
        layers.read("map/read.txt")
    })
    .unwrap()
    .1;
    assert_eq!(once.entries, 3);
    assert_eq!(once.entries, twice.entries);
    assert_eq!(once.digest, twice.digest);
    assert_eq!(witness_records(&once), witness_records(&twice));
    let recorder = witness::Recorder::new(&layers);
    let log = recorder.clone();
    let retained = layers
        .with_read_trace(Arc::new(move |read| log.record_read(read)))
        .with_read_purpose("retained");
    assert_eq!(
        retained.read_prefix("map/read.txt", 4).unwrap(),
        (b"read".to_vec(), 10)
    );
    assert_eq!(
        retained.read_uncovered("gfx/custom/read.bin").unwrap(),
        b"uncovered bytes"
    );
    let first = recorder.snapshot().unwrap();
    let retained_clone = retained.clone();
    assert_eq!(retained_clone.read("map/read.txt").unwrap(), b"read bytes");
    assert_eq!(
        retained_clone
            .read_uncovered("gfx/custom/read.bin")
            .unwrap(),
        b"uncovered bytes"
    );
    let second = recorder.snapshot().unwrap();
    assert_eq!(first.entries, 3);
    assert_eq!(first.digest, second.digest);
    assert_eq!(witness_records(&first), witness_records(&second));
    let unwitnessed = layers.read("map/read.txt").unwrap();
    assert_eq!(unwitnessed, b"read bytes");
    assert_eq!(second.digest, recorder.snapshot().unwrap().digest);
}

#[test]
fn rejected_payloads_with_identical_errors_have_distinct_witnesses_and_exact_byte_evidence() {
    let fixture = Fixture::simple();
    let root = fixture.root.join("rejected-witness");
    fs::create_dir_all(root.join("map")).unwrap();
    let path = root.join("map/read.txt");
    fs::write(&path, b"verified").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let load = |layers: &ResolvedLayers, mode| match mode {
        0 => layers.read("map/read.txt"),
        1 => layers
            .read_prefix("map/read.txt", 2)
            .map(|(bytes, _)| bytes),
        2 => layers.read_uncovered_path(b"map/read.txt"),
        _ => unreachable!(),
    };
    let mut witnesses = Vec::new();
    for payload in [b"invalid1", b"invalid2"] {
        fs::write(&path, payload).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
            .unwrap();
        assert_eq!(metadata.len(), fs::metadata(&path).unwrap().len());
        assert_eq!(
            metadata.modified().unwrap(),
            fs::metadata(&path).unwrap().modified().unwrap()
        );
        let mut modes = Vec::new();
        for mode in 0..3 {
            let original = load(&layers, mode).unwrap_err();
            let (result, witness) = Witness::capture(&layers, |layers| load(layers, mode)).unwrap();
            let error = result.unwrap_err();
            assert_eq!(error.kind(), original.kind());
            assert_eq!(error.to_string(), original.to_string());
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                error.to_string(),
                "content file changed since verification: map/read.txt"
            );
            let records = witness_records(&witness);
            assert_eq!(witness.entries, 3);
            assert!(
                matches!(product_codec::decode::<witness::Entry>(&records[2]).unwrap(),
                witness::Entry::Read { path, layer: Some(layer), outcome: witness::Outcome::Error { kind, message, payload: Some(bytes) }, .. }
                if path == b"map/read.txt" && layer.index == 0 && kind == "InvalidData" && message == error.to_string()
                && bytes.length == payload.len() as u64 && bytes.digest == codec::digest(payload))
            );
            modes.push(witness);
        }
        assert!(
            modes
                .iter()
                .all(|witness| witness.digest == modes[0].digest)
        );
        witnesses.push(modes.remove(0));
    }
    assert_ne!(witnesses[0].digest, witnesses[1].digest);
}

#[test]
fn witness_verification_rejects_partial_trailing_duplicate_and_unsorted_sets_after_reopen() {
    let fixture = Fixture::simple();
    let root = fixture.root.join("witness-verification");
    fs::create_dir_all(root.join("map")).unwrap();
    fs::write(root.join("map/a-long-file.txt"), b"a").unwrap();
    fs::write(root.join("map/z.txt"), b"z").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (result, witness) = Witness::capture(&layers, |layers| {
        layers.read("map/z.txt")?;
        layers.read("map/a-long-file.txt")
    })
    .unwrap();
    result.unwrap();
    let records = witness_records(&witness);
    let mut key = fixture.session().key;
    key.content = witness.digest;
    key.content_entries = witness.entries;
    let mut writer = fixture.writer();
    witness.publish(&mut writer, &key).unwrap();
    writer.commit().unwrap();
    let reopened = Store::local(fixture.id).unwrap();
    let read = |ordinal| reopened.read(&fixture.source, &key.section("witness"), ordinal);
    witness::verify(&key, read, || Ok(())).unwrap();
    let error = witness::verify(
        &key,
        |ordinal| {
            if ordinal == 2 {
                Ok(None)
            } else {
                read(ordinal)
            }
        },
        || Ok(()),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "content witness entry is dangling");
    let mut trailing = key.clone();
    trailing.content_entries -= 1;
    assert_eq!(
        witness::verify(&trailing, read, || Ok(()))
            .unwrap_err()
            .to_string(),
        "content witness has trailing entries"
    );
    for order in [vec![0, 1, 3, 2], vec![0, 1, 2, 2]] {
        let mut changed = key.clone();
        changed.content = resolution_digest(
            &order
                .iter()
                .map(|&at| records[at].clone())
                .collect::<Vec<_>>(),
        );
        let error = witness::verify(
            &changed,
            |ordinal| match order.get(ordinal as usize) {
                Some(&at) => read(at as u64),
                None => Ok(None),
            },
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "content witness entries are not a canonical set"
        );
    }
}

#[test]
fn common_and_layer_keys_change_on_content_variant_schema_contract_and_producer_version() {
    let fixture = Fixture::simple();
    let original = fixture.session().key;
    let common = original.digest();
    let layer = original.layer_digest(SNOW).unwrap();
    for change in 0..6 {
        let mut changed = original.clone();
        match change {
            0 => changed.content[0] ^= 1,
            1 => {
                changed
                    .variant
                    .fields
                    .insert("perspective".into(), b"different recorded view".to_vec());
            }
            2 => changed.schema[0] ^= 1,
            3 => changed.producers.get_mut(&SNOW).unwrap().version += 1,
            4 => changed.producers.get_mut(&SNOW).unwrap().contract.push(1),
            5 => changed.producers.get_mut(&SNOW).unwrap().variant.push(1),
            _ => unreachable!(),
        }
        assert_ne!(changed.digest(), common);
        assert_ne!(changed.layer_digest(SNOW).unwrap(), layer);
    }
    assert!(original.layer_digest(9).is_err());
}

#[test]
fn endpoint_kinds_and_cursor_keep_absolute_grid_gap_edges_and_observed_block() {
    assert_eq!(endpoint_kind(-80, 83, Some(-81)), 1);
    assert_eq!(endpoint_kind(-85, 83, None), 4);
    assert_eq!(endpoint_kind(21, 83, Some(7)), 8);
    assert_eq!(endpoint_kind(83, 83, Some(82)), 2);
    assert_eq!(endpoint_kind(17, 83, Some(16)), 0);
    let fixture = Fixture::simple();
    let session = fixture.session();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    session
        .observe_in_block_after(&mut writer, &frame(1, 0), 2, None, Some(7))
        .unwrap();
    session
        .observe_in_block_after(&mut writer, &frame(1, 1), 2, Some(0), Some(7))
        .unwrap();
    writer.commit().unwrap();
    let cursor = Cursor::read(
        fixture
            .store
            .read(&fixture.source, &session.key.section("cursor/1"), 0)
            .unwrap()
            .unwrap(),
        &session.key,
        1,
    )
    .unwrap();
    assert_eq!(
        cursor,
        Cursor {
            hour: 1,
            block: Some(7)
        }
    );
    assert_eq!(
        fixture
            .reader(&session.key, inputs().limits())
            .coverage(1)
            .unwrap()
            .unwrap()
            .endpoints,
        1
    );
}

#[test]
fn content_witness_catches_equal_sized_semantic_noops_higher_layer_arrivals_and_replace_paths() {
    use crate::test_fixtures::snow_content;
    let fixture = Fixture::simple();
    let a = fixture.root.join("base");
    let b = fixture.root.join("mod");
    snow_content::write(&a);
    fs::create_dir_all(b.join("common/defines")).unwrap();
    let load = |replace: bool| {
        let paths: &[&str] = if replace { &["map/unused"] } else { &[] };
        let layers = ResolvedLayers::from_folders(&[(&a, &[][..]), (&b, paths)]).unwrap();
        let input = Inputs::load(&layers).unwrap();
        Key::for_loaded_inputs(&input, PRODUCER_VERSION).unwrap()
    };
    let original = load(false);
    let file = a.join("common/defines/snow.lua");
    let metadata = fs::metadata(&file).unwrap();
    fs::write(&file, snow_content::DEFINES.replace("0.1", ".10")).unwrap();
    File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    let edited = load(false);
    assert_eq!(metadata.len(), fs::metadata(&file).unwrap().len());
    assert_eq!(
        metadata.modified().unwrap(),
        fs::metadata(&file).unwrap().modified().unwrap()
    );
    assert_eq!(
        original.inputs, edited.inputs,
        "parsed snow operands deliberately do not change"
    );
    assert_ne!(
        original.content, edited.content,
        "the consumed source bytes do change"
    );
    fs::write(
        b.join("common/defines/snow.lua"),
        snow_content::DEFINES.replace("0.1", ".10"),
    )
    .unwrap();
    let higher = load(false);
    assert_eq!(edited.inputs, higher.inputs);
    assert_ne!(
        edited.content, higher.content,
        "the identical bytes now resolve in a higher layer"
    );
    let replaced = load(true);
    assert_eq!(higher.inputs, replaced.inputs);
    assert_ne!(
        higher.content, replaced.content,
        "replace paths belong to the layer witness"
    );
}

#[test]
fn fixed_digest_index_length_hint_saturates_without_narrowing_payload_lengths() {
    let entry = index::Entry {
        digest: [7; 32],
        section: TABLE_SECTION | 9,
        ordinal: u64::MAX - 1,
        raw_hint: u32::try_from(u64::from(u32::MAX) + 1).unwrap_or(u32::MAX),
    };
    let bytes = entry.bytes().unwrap();
    assert_eq!(bytes.len(), 48);
    assert_eq!(&bytes[..32], &[7; 32]);
    assert_eq!(&bytes[32..36], &(TABLE_SECTION | 9).to_le_bytes());
    assert_eq!(&bytes[36..44], &(u64::MAX - 1).to_le_bytes());
    assert_eq!(&bytes[44..], &u32::MAX.to_le_bytes());
    let reference = Payload {
        section: TABLE_SECTION | 9,
        ordinal: 0,
        digest: [7; 32],
        raw: u64::from(u32::MAX) + 1,
        branch: 1,
        hour: 0,
    };
    assert_eq!(
        product_codec::decode::<Payload>(&product_codec::encode(&reference).unwrap())
            .unwrap()
            .raw,
        u64::from(u32::MAX) + 1
    );
}

#[test]
fn typed_manifest_statuses_preserve_exact_part_errors_and_refuse_unknown_contracts() {
    let fixture = Fixture::simple();
    let mut session = fixture.session();
    let producer = session.key.producers[&SNOW].clone();
    for layer in [2, 3] {
        session.key.producers.insert(layer, producer.clone());
    }
    let manifest = Manifest {
        endpoint: 4,
        layers: [
            (
                1,
                LayerStatus::Unavailable {
                    reason: "WeatherProvinces: native read refused".into(),
                },
            ),
            (
                2,
                LayerStatus::Unsupported {
                    reason: "synthetic visual transform".into(),
                },
            ),
            (
                3,
                LayerStatus::PartError {
                    kind: "fog".into(),
                    display: "field: missing exact operand".into(),
                },
            ),
        ]
        .into_iter()
        .collect(),
        tables: Default::default(),
    };
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_manifest(&mut writer, &session.key, 1, -3, &manifest).unwrap();
    let mut invalid = manifest.clone();
    invalid.endpoint = 16;
    assert!(write_manifest(&mut writer, &session.key, 1, 0, &invalid).is_err());
    invalid.endpoint = 1;
    invalid.layers.insert(
        9,
        LayerStatus::Unavailable {
            reason: "unbound".into(),
        },
    );
    assert!(write_manifest(&mut writer, &session.key, 1, 0, &invalid).is_err());
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, inputs().limits());
    let headroom = Headroom {
        available: Some((u64::MAX, u64::MAX)),
        low: false,
    };
    assert!(matches!(reader.layer(1, -3, 1, headroom).unwrap(),
        Some(LayerProduct::Unavailable(reason)) if reason == "WeatherProvinces: native read refused"));
    assert!(matches!(reader.layer(1, -3, 2, headroom).unwrap(),
        Some(LayerProduct::Unsupported(reason)) if reason == "synthetic visual transform"));
    assert!(matches!(reader.layer(1, -3, 3, headroom).unwrap(),
        Some(LayerProduct::PartError { kind, display }) if kind == "fog" && display == "field: missing exact operand"));
    assert!(
        session
            .reusable(
                &fixture.store,
                &fixture.source,
                [(
                    1,
                    Some(Cursor {
                        hour: -3,
                        block: None
                    })
                )],
                || Ok(())
            )
            .unwrap()
    );
    let mut malformed = product_codec::encode(&manifest).unwrap();
    malformed[11] = 255;
    assert!(product_codec::decode::<Manifest>(&malformed).is_err());
}

#[test]
fn initial_binding_publishes_empty_operand_and_all_digest_bucket_sections() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    writer.commit().unwrap();
    for section in ["tables".into(), "layer/1".into(), "witness".into()]
        .into_iter()
        .chain((0..=u8::MAX).map(|bucket| format!("index/{bucket:02x}")))
    {
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.section(&section))
                .unwrap(),
            Some(0)
        );
        assert!(
            fixture
                .store
                .read(&fixture.source, &session.key.section(&section), 0)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        !session
            .has_history(&fixture.store, &fixture.source, [1], || Ok(()))
            .unwrap()
    );
    assert!(
        session
            .reusable(&fixture.store, &fixture.source, [(1, None)], || Ok(()))
            .unwrap()
    );
}

#[test]
fn snow_geometry_rejects_plausible_container_counts_before_generated_allocation() {
    let input = inputs();
    let snow = input.build(&frame(1, 0)).unwrap();
    let limits = input.limits();
    let bytes = codec::encode_product(&snow, limits).unwrap();
    let cells = 16 + snow.texels.len();
    let provinces = cells + 8 + snow.unknown.len();
    let factors = provinces
        + 8
        + snow
            .province_snow
            .iter()
            .map(|value| 1 + usize::from(value.is_some()) * 8)
            .sum::<usize>()
        + 1;
    for (offset, expected) in [
        (8, snow.texels.len()),
        (cells, snow.unknown.len()),
        (provinces, limits.provinces),
        (factors, limits.factors),
    ] {
        for count in [expected - 1, expected + 1] {
            let mut wrong = bytes.clone();
            wrong[offset..offset + 8].copy_from_slice(&(count as u64).to_le_bytes());
            assert_eq!(
                codec::decode_product(&wrong, limits)
                    .unwrap_err()
                    .to_string(),
                "snow typed length differs from admitted geometry or remaining bytes"
            );
        }
    }
}

#[test]
fn finite_headroom_between_encoded_buffers_and_geometry_decode_peak_uses_the_unchanged_live_builder()
 {
    let fixture = Fixture::simple();
    let map = MapData {
        width: 12,
        height: 8,
        province_ids: vec![1; 96],
        definitions: (1..=32_768)
            .map(|id| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [0; 3],
                        kind: ProvinceKind::Land,
                        continent: 1,
                    },
                )
            })
            .collect(),
        country_colors: Default::default(),
    };
    let input = Inputs {
        cells: SnowCells::new(&map, &[false; 96]).unwrap(),
        ..inputs()
    };
    let replay = Replay::open_path(&fixture.path).unwrap();
    let prefix = replay
        .index
        .iter()
        .map(|block| block.offset + block.length)
        .max()
        .unwrap();
    let session = Session::from_inputs(
        &replay,
        Ok(input),
        prefix,
        &fixture.store,
        &fixture.source,
        || Ok(()),
    )
    .unwrap();
    let input = session.inputs.as_ref().unwrap();
    let limits = input.limits();
    let expected = input.build(&frame(1, 0)).unwrap();
    assert!(expected.province_snow.len() >= 32_768);
    assert!(
        expected
            .province_snow
            .iter()
            .filter(|snow| snow.is_none())
            .count()
            > 32_000
    );
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(&mut writer, &session.key, 1, 0, Ok(&expected), Some(limits)).unwrap();
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, limits);
    let manifest = fixture.packet(&session.key, "endpoints/1", 0);
    let payload = fixture.packet(&session.key, "layer/1", 0);
    let raw = codec::encode_product(&expected, limits).unwrap().len();
    let manifest_raw = u64::from_le_bytes(manifest[55..63].try_into().unwrap()) as usize;
    let workspace = zstd::zstd_safe::DCtx::try_create().unwrap().sizeof();
    assert_eq!(
        payload[52] & 0x80,
        0,
        "the control must exercise zstd workspace admission"
    );
    let encoded_peak = (raw + payload.len() - codec::HEADER + manifest_raw + workspace)
        .max(manifest_raw + manifest.len() - codec::HEADER + workspace);
    let typed_peak = raw
        + manifest_raw
        + expected.texels.len()
        + expected.unknown.len()
        + expected.province_snow.len() * size_of::<Option<i64>>()
        + 2 * limits.factors * size_of::<Option<f32>>()
        + size_of::<SnowMud>()
        + 4 * size_of::<usize>();
    assert!(typed_peak > encoded_peak);
    let finite = encoded_peak + (typed_peak - encoded_peak) / 2;
    assert!(finite > encoded_peak && finite < typed_peak);
    for available in [(finite as u64, u64::MAX), (u64::MAX, finite as u64)] {
        let headroom = Headroom {
            available: Some(available),
            low: false,
        };
        codec::PRODUCT_DECODES.set(0);
        let calls = Cell::new(0);
        let shown = reader
            .build_or_restore_admitted(1, 0, headroom, || {
                calls.set(calls.get() + 1);
                input.build(&frame(1, 0)).map(Arc::new)
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(codec::PRODUCT_DECODES.get(), 0);
        assert_eq!(&*shown, &expected);
    }
    let admitted = encoded_peak.max(limits.decode_peak(raw).unwrap() + manifest_raw);
    codec::PRODUCT_DECODES.set(0);
    let shown = reader
        .build_or_restore_admitted(
            1,
            0,
            Headroom {
                available: Some((admitted as u64, admitted as u64)),
                low: false,
            },
            || panic!("geometry-derived peak must admit this verified endpoint"),
        )
        .unwrap();
    assert_eq!(codec::PRODUCT_DECODES.get(), 1);
    assert_eq!(&*shown, &expected);
    assert!(
        Limits {
            provinces: usize::MAX,
            ..limits
        }
        .decode_peak(raw)
        .is_err()
    );
    assert!(
        Limits {
            factors: usize::MAX,
            ..limits
        }
        .decode_peak(raw)
        .is_err()
    );
}

#[test]
fn borrowed_manifest_view_matches_generated_statuses_and_refuses_corrupt_maps() {
    let session = Fixture::simple().session();
    let payload = Payload {
        section: 1,
        ordinal: 5,
        digest: [7; 32],
        raw: 42,
        branch: 3,
        hour: -4,
    };
    for status in [
        LayerStatus::Available {
            payload: payload.clone(),
        },
        LayerStatus::Unavailable {
            reason: "exact refusal".into(),
        },
        LayerStatus::Unsupported {
            reason: "exact unsupported".into(),
        },
        LayerStatus::PartError {
            kind: "field".into(),
            display: "exact error".into(),
        },
    ] {
        let manifest = Manifest {
            endpoint: 1,
            layers: [(SNOW, status.clone())].into_iter().collect(),
            tables: [(17, payload.clone())].into_iter().collect(),
        };
        let bytes = product_codec::encode(&manifest).unwrap();
        match (
            status,
            records::snow_status(&bytes, &session.key).unwrap().unwrap(),
        ) {
            (
                LayerStatus::Available { payload: expected },
                records::SnowStatus::Available(actual),
            ) => assert_eq!(actual, expected),
            (LayerStatus::Unavailable { reason }, records::SnowStatus::Unavailable(actual))
            | (LayerStatus::Unsupported { reason }, records::SnowStatus::Unsupported(actual)) => {
                assert_eq!(actual, reason)
            }
            (
                LayerStatus::PartError { kind, display },
                records::SnowStatus::PartError {
                    kind: actual_kind,
                    display: actual_display,
                },
            ) => {
                assert_eq!(actual_kind, kind);
                assert_eq!(actual_display, display);
            }
            _ => panic!("borrowed manifest status changed"),
        }
        for end in 0..bytes.len() {
            assert!(records::snow_status(&bytes[..end], &session.key).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(records::snow_status(&trailing, &session.key).is_err());
        for (offset, byte) in [(0, 16), (9, 0), (9, 9), (11, 255)] {
            let mut damaged = bytes.clone();
            damaged[offset] = byte;
            assert!(records::snow_status(&damaged, &session.key).is_err());
        }
    }
    let mut duplicate = vec![1];
    duplicate.extend_from_slice(&2u64.to_le_bytes());
    for _ in 0..2 {
        duplicate.extend_from_slice(&SNOW.to_le_bytes());
        duplicate.push(1);
        duplicate.extend_from_slice(&0u64.to_le_bytes());
    }
    duplicate.extend_from_slice(&0u64.to_le_bytes());
    assert!(
        records::snow_status(&duplicate, &session.key)
            .unwrap_err()
            .to_string()
            .contains("strictly ordered")
    );
}

#[test]
fn cursor_checksum_provenance_and_accepted_hour_and_block_are_all_required_before_reuse_or_append()
{
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (0..=39).map(|hour| frame(1, hour)).collect())],
    )
    .unwrap();
    for damage in 0..6 {
        let mut fixture = Fixture::new(first.bytes());
        let session = fixture.session();
        let mut writer = fixture.writer();
        session.initialise(&mut writer).unwrap();
        for hour in 0..=39 {
            session
                .observe_in_block_after(
                    &mut writer,
                    &frame(1, hour),
                    39,
                    hour.checked_sub(1).filter(|_| hour > 0),
                    Some(0),
                )
                .unwrap();
        }
        writer.commit().unwrap();
        let accepted = Cursor {
            hour: 39,
            block: Some(0),
        };
        assert!(
            session
                .reusable(
                    &fixture.store,
                    &fixture.source,
                    [(1, Some(accepted.clone()))],
                    || Ok(())
                )
                .unwrap()
        );
        let original = fixture.packet(&session.key, "cursor/1", 0);
        let mut damaged = match damage {
            0 => original.clone(),
            1 => Cursor {
                hour: 400,
                ..accepted.clone()
            }
            .encode(&session.key, 1)
            .unwrap(),
            2 => Cursor {
                block: Some(1),
                ..accepted.clone()
            }
            .encode(&session.key, 1)
            .unwrap(),
            3 => Cursor {
                block: None,
                ..accepted.clone()
            }
            .encode(&session.key, 1)
            .unwrap(),
            4 => accepted.encode(&session.key, 2).unwrap(),
            _ => {
                let mut key = session.key.clone();
                key.inputs[0] ^= 1;
                accepted.encode(&key, 1).unwrap()
            }
        };
        if damage == 0 {
            let body_length = product_codec::encode(&accepted).unwrap().len();
            let at = damaged.len() - 32 - body_length;
            damaged[at..at + 4].copy_from_slice(&400i32.to_le_bytes());
            assert_eq!(
                product_codec::decode::<Cursor>(&damaged[at..damaged.len() - 32])
                    .unwrap()
                    .hour,
                400
            );
        }
        let mut writer = fixture.writer();
        writer
            .write_at(&session.key.section("cursor/1"), 0, damaged.as_slice())
            .unwrap();
        writer.commit().unwrap();
        assert!(
            !session
                .reusable(
                    &fixture.store,
                    &fixture.source,
                    [(1, Some(accepted.clone()))],
                    || Ok(())
                )
                .unwrap()
        );
        let appended = first
            .append(&[continuation_records(&first, 1, vec![frame(1, 40)])])
            .unwrap();
        fs::write(&fixture.path, appended.bytes()).unwrap();
        fixture.source = SourceKey::capture(&fixture.path, &[]).unwrap();
        let next = fixture.session();
        assert!(
            !next
                .can_inherit(
                    &fixture.store,
                    &fixture.source,
                    [(1, Some(accepted))],
                    || Ok(())
                )
                .unwrap()
        );
        let mut writer = fixture.store.replace(fixture.source.clone()).unwrap();
        next.initialise(&mut writer).unwrap();
        for hour in 0..=40 {
            next.observe_in_block_after(
                &mut writer,
                &frame(1, hour),
                40,
                hour.checked_sub(1).filter(|_| hour > 0),
                Some(u64::from(hour == 40)),
            )
            .unwrap();
        }
        writer.commit().unwrap();
        assert_eq!(
            &*hit(&fixture.reader(&next.key, inputs().limits()), 1, 40),
            &inputs().build(&frame(1, 40)).unwrap()
        );
    }
}

#[test]
fn malformed_deflate_and_password_failures_bind_the_actual_boundary_input_into_the_witness() {
    use installed_content_core::test_support::{Record, zip_records};
    let fixture = Fixture::simple();
    for encrypted in [false, true] {
        let root = fixture.root.join(if encrypted {
            "password-witness"
        } else {
            "deflate-witness"
        });
        let game = root.join("game");
        let user = root.join("user");
        fs::create_dir_all(game.join("dlc")).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::write(game.join("hoi4.exe"), b"synthetic executable").unwrap();
        fs::write(user.join("settings.txt"), b"language=l_english").unwrap();
        fs::write(
            user.join("dlc_load.json"),
            br#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        )
        .unwrap();
        let archive = game.join("tweakergui_assets.zip");
        let name = b"common/one.txt";
        let mut valid = if encrypted {
            include_bytes!("../../../installed-content-core/src/content/testdata/zipcrypto_deflated.zip")
                .to_vec()
        } else {
            zip_records(
                &[],
                &[Record {
                    name,
                    host: 0,
                    flags: 0,
                    deflate: false,
                    data: &[1, 6, 0, 249, 255, b'v', b'a', b'l', b'i', b'd', b'!'],
                }],
            )
        };
        let central = valid.windows(4).position(|b| b == b"PK\x01\x02").unwrap();
        if encrypted {
            let size =
                u32::from_le_bytes(valid[central + 24..central + 28].try_into().unwrap()) + 1;
            valid[22..26].copy_from_slice(&size.to_le_bytes());
            valid[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
        } else {
            valid[8..10].copy_from_slice(&8u16.to_le_bytes());
            valid[22..26].copy_from_slice(&6u32.to_le_bytes());
            valid[central + 10..central + 12].copy_from_slice(&8u16.to_le_bytes());
            valid[central + 24..central + 28].copy_from_slice(&6u32.to_le_bytes());
        }
        fs::write(&archive, &valid).unwrap();
        let layers = installed_content_core::gather(&installed_content_core::GatherOptions {
            game_dir: game,
            user_dir: user,
            cache_dir: root.join("cache"),
            dlc_order: Some(Vec::new()),
        })
        .unwrap()
        .layers;
        let modified = fs::metadata(&archive).unwrap().modified().unwrap();
        let request = if encrypted {
            "gfx/fonts/x.fnt$secret"
        } else {
            "common/one.txt"
        };
        if !encrypted {
            assert_eq!(layers.read(request).unwrap(), b"valid!");
        }
        let mut witnesses = Vec::new();
        let mut errors = Vec::new();
        for byte in if encrypted {
            vec![b'A']
        } else {
            vec![b'A', b'B']
        } {
            let mut damaged = valid.clone();
            let data = 30
                + u16::from_le_bytes(valid[26..28].try_into().unwrap()) as usize
                + u16::from_le_bytes(valid[28..30].try_into().unwrap()) as usize;
            if !encrypted {
                damaged[data..data + 11].copy_from_slice(&[1, 1, 0, 254, 255, byte, 0, 0, 0, 0, 0]);
            }
            fs::write(&archive, &damaged).unwrap();
            File::options()
                .write(true)
                .open(&archive)
                .unwrap()
                .set_times(FileTimes::new().set_modified(modified))
                .unwrap();
            let original = layers.read(request).unwrap_err();
            assert!(
                original
                    .to_string()
                    .contains("it inflates to fewer bytes than its uncompressed size")
            );
            let compressed =
                u32::from_le_bytes(valid[central + 20..central + 24].try_into().unwrap()) as usize;
            let expected = [
                &damaged[..30],
                &damaged[data..data + compressed + if encrypted { 12 } else { 0 }],
            ]
            .concat();
            let mut modes = Vec::new();
            for mode in 0..3 {
                let (result, witness) = Witness::capture(&layers, |layers| match mode {
                    0 => layers.read(request),
                    1 => layers.read_prefix(request, 1).map(|(bytes, _)| bytes),
                    _ => layers.read_uncovered(request),
                })
                .unwrap();
                let error = result.unwrap_err();
                assert_eq!(error.kind(), original.kind());
                assert_eq!(error.to_string(), original.to_string());
                let reads: Vec<_> = witness_records(&witness)
                    .into_iter()
                    .map(|record| product_codec::decode::<witness::Entry>(&record).unwrap())
                    .filter_map(|entry| match entry {
                        witness::Entry::Read { path, outcome, .. }
                            if path == request.as_bytes() =>
                        {
                            Some(outcome)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    reads,
                    vec![witness::Outcome::Error {
                        kind: "InvalidData".into(),
                        message: error.to_string(),
                        payload: Some(witness::Payload {
                            length: expected.len() as u64,
                            digest: codec::digest(&expected)
                        }),
                    }]
                );
                modes.push(witness.digest);
            }
            assert!(modes.iter().all(|digest| *digest == modes[0]));
            witnesses.push(modes[0]);
            errors.push(original.to_string());
        }
        if !encrypted {
            assert_eq!(errors[0], errors[1]);
            assert_ne!(witnesses[0], witnesses[1]);
        }
    }
}

#[test]
fn decompressor_workspace_is_admitted_before_allocating_encoded_or_decoded_buffers() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let bytes = vec![7; 8192];
    let packet = codec::pack(&session.key, 1, 0, 3, &bytes).unwrap();
    assert_eq!(packet[52] & 0x80, 0);
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    writer
        .write_at(&session.key.section("layer/1"), 0, packet.as_slice())
        .unwrap();
    writer.commit().unwrap();
    let range = fixture
        .store
        .read(&fixture.source, &session.key.section("layer/1"), 0)
        .unwrap()
        .unwrap();
    let header = codec::header(&range).unwrap();
    let encoded = header.raw + header.compressed;
    let layer_key = session.key.layer_digest(SNOW).unwrap();
    let before = engine_allocation::growths();
    let error = codec::unpack_for_admitted(
        range.clone(),
        layer_key,
        Some((1, 0)),
        3,
        SNOW,
        bytes.len(),
        encoded,
        0,
        &mut 0,
    )
    .unwrap_err();
    assert!(error.to_string().contains("decode peak not admitted"));
    assert_eq!(engine_allocation::growths(), before);
    let workspace = zstd::zstd_safe::DCtx::try_create().unwrap().sizeof();
    let maximum = encoded + workspace;
    let mut peak = 0;
    assert_eq!(
        codec::unpack_for_admitted(
            range,
            layer_key,
            Some((1, 0)),
            3,
            SNOW,
            bytes.len(),
            maximum,
            0,
            &mut peak
        )
        .unwrap(),
        bytes
    );
    assert_eq!(peak, maximum);
}

#[test]
fn missing_endpoints_use_bounded_point_reads_and_recheck_source_and_publications() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let snow = input.build(&frame(1, 0)).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        0,
        Ok(&snow),
        Some(input.limits()),
    )
    .unwrap();
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, input.limits());
    assert!(matches!(reader.restore(1, 9), Restore::Miss(_)));
    let before = store::counters::now();
    for hour in [9, 10, 11, 17] {
        assert!(
            matches!(reader.restore(1, hour), Restore::Miss(ref reason) if reason == "missing or stale endpoint")
        );
    }
    let work = store::counters::now() - before;
    assert_eq!(work.manifest_reads, 8);
    assert!(work.log_opens <= 16, "{work:?}");
    let expected = product_codec::encode(&snow).unwrap();
    assert_eq!(
        product_codec::encode(&*hit(&reader, 1, 0)).unwrap(),
        expected
    );
    let mut writer = fixture.writer();
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        9,
        Ok(&snow),
        Some(input.limits()),
    )
    .unwrap();
    writer.commit().unwrap();
    assert_eq!(
        product_codec::encode(&*hit(&reader, 1, 9)).unwrap(),
        expected
    );
    assert!(matches!(reader.restore(1, 10), Restore::Miss(_)));
    let source = fs::read_dir(fixture.store.directory())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("source_")
        })
        .unwrap();
    let mut bytes = fs::read(&source).unwrap();
    bytes.push(b' ');
    fs::write(&source, bytes).unwrap();
    let before = store::counters::now();
    assert!(matches!(reader.restore(1, 10), Restore::Miss(_)));
    assert!(store::counters::now().manifest_reads > before.manifest_reads);
    let before = store::counters::now();
    assert!(matches!(reader.restore(1, 10), Restore::Miss(_)));
    assert_eq!(
        store::counters::now().manifest_reads - before.manifest_reads,
        2
    );
}

fn export_headroom() -> Headroom {
    Headroom {
        available: Some((u64::MAX, u64::MAX)),
        low: false,
    }
}

#[test]
fn appended_export_endpoints_keep_scoped_generations_and_refresh_after_scope() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let input = inputs();
    let snow = input.build(&frame(1, 0)).unwrap();
    let expected = product_codec::encode(&snow).unwrap();
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        0,
        Ok(&snow),
        Some(input.limits()),
    )
    .unwrap();
    writer.commit().unwrap();
    let reader = fixture.reader(&session.key, input.limits());
    let scope = fixture.store.scope(&fixture.source).unwrap();
    let mut writer = fixture.writer();
    write_endpoint(
        &mut writer,
        &session.key,
        1,
        9,
        Ok(&snow),
        Some(input.limits()),
    )
    .unwrap();
    write_endpoint(
        &mut writer,
        &session.key,
        2,
        9,
        Ok(&snow),
        Some(input.limits()),
    )
    .unwrap();
    writer.commit().unwrap();
    assert!(reader.manifest(1, 9, export_headroom()).unwrap().is_none());
    assert!(reader.manifest(2, 9, export_headroom()).unwrap().is_none());
    assert!(matches!(reader.restore(1, 9), Restore::Miss(_)));
    assert_eq!(
        product_codec::encode(&*hit(&reader, 1, 0)).unwrap(),
        expected
    );
    drop(scope);
    for branch in [1, 2] {
        assert!(
            reader
                .manifest(branch, 9, export_headroom())
                .unwrap()
                .is_some()
        );
        assert_eq!(
            product_codec::encode(&*hit(&reader, branch, 9)).unwrap(),
            expected
        );
    }
    assert!(reader.manifest(2, 0, export_headroom()).unwrap().is_none());

    fixture.store.clear().unwrap();
    assert!(matches!(reader.restore(1, 9), Restore::Miss(_)));
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    write_endpoint(
        &mut writer,
        &session.key,
        2,
        12,
        Ok(&snow),
        Some(input.limits()),
    )
    .unwrap();
    writer.commit().unwrap();
    assert!(reader.manifest(1, 9, export_headroom()).unwrap().is_none());
    assert!(reader.manifest(2, 9, export_headroom()).unwrap().is_none());
    assert_eq!(
        product_codec::encode(&*hit(&reader, 2, 12)).unwrap(),
        expected
    );
}

struct Counted {
    snapshot: store::Snapshot,
    opened: Vec<(String, u64)>,
}
impl crate::prepared::facts::ReadPages for Counted {
    fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
        self.opened.push((section.into(), range));
        self.snapshot.read(section, range)
    }
}
#[test]
fn publications_preserve_other_branches_and_misses_read_only_the_keyed_path() {
    let fixture = Fixture::simple();
    let session = fixture.session();
    let manifest = Manifest {
        endpoint: 1,
        layers: Default::default(),
        tables: Default::default(),
    };
    let mut writer = fixture.writer();
    session.initialise(&mut writer).unwrap();
    for branch in [1, 2] {
        for hour in 0..129 {
            write_manifest(&mut writer, &session.key, branch, hour * 2, &manifest).unwrap();
        }
    }
    writer.commit().unwrap();
    let root_id = |branch| {
        fixture
            .store
            .read(
                &fixture.source,
                &session.key.section(&format!("endpoint-index/{branch}")),
                0,
            )
            .unwrap()
            .unwrap()
            .id()
    };
    let roots = [root_id(1), root_id(2)];
    let reader = fixture.reader(&session.key, inputs().limits());
    for publication in 0..4 {
        let mut writer = fixture.writer();
        writer
            .write_at("unrelated/prepared-section", 0, [publication].as_slice())
            .unwrap();
        writer.commit().unwrap();
        assert_eq!([root_id(1), root_id(2)], roots);
        let before = store::counters::now();
        assert!(reader.manifest(1, 3, export_headroom()).unwrap().is_none());
        let work = store::counters::now() - before;
        assert_eq!(work.manifest_reads, 2);
        assert!(
            work.log_opens <= 5,
            "historical reads after publication: {work:?}"
        );
        for branch in [1, 2] {
            let mut source = Counted {
                snapshot: fixture
                    .store
                    .session(&fixture.source)
                    .unwrap()
                    .unwrap()
                    .snapshot(),
                opened: Vec::new(),
            };
            assert_eq!(
                endpoints::get_in(&mut source, &session.key, branch, 3, 129).unwrap(),
                None
            );
            assert!(source.opened.len() <= 4, "{:?}", source.opened);
            assert!(
                source
                    .opened
                    .iter()
                    .all(|(name, _)| name != &session.key.records(branch))
            );
            assert_eq!(
                endpoints::get_in(&mut source, &session.key, branch, 256, 129).unwrap(),
                Some(128)
            );
        }
    }
    let mut writer = fixture.writer();
    write_manifest(&mut writer, &session.key, 1, 258, &manifest).unwrap();
    writer.commit().unwrap();
    assert_ne!(root_id(1), roots[0]);
    assert_eq!(root_id(2), roots[1]);
    assert_eq!(
        reader.manifest(1, 258, export_headroom()).unwrap(),
        Some(manifest)
    );
    assert!(
        reader
            .manifest(2, 258, export_headroom())
            .unwrap()
            .is_none()
    );
}
