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
                assert!(session.inputs.as_ref().unwrap().build(frame).is_err());
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
            .range_count(&fixture.source, &session.key.section("payloads"))
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
        let layers = if reverse {
            ResolvedLayers::from_folders(&[(&b, &[][..]), (&a, &[][..])])
        } else {
            ResolvedLayers::from_folders(&[(&a, &[][..]), (&b, &[][..])])
        }
        .unwrap();
        let inputs = Inputs::load(&layers).unwrap();
        Key::for_inputs(Ok((&inputs.cells, &inputs.winter)), 1)
            .unwrap()
            .bind_recording(&Replay::open_path(&fixture.path).unwrap(), || Ok(()))
            .unwrap()
    };
    let original = loaded(false);
    assert_eq!(original, loaded(true));
    fs::write(
        a.join("common/defines/snow.lua"),
        snow_content::DEFINES.replace("0.1", "0.4"),
    )
    .unwrap();
    fs::write(b.join("map/irrelevant.txt"), b"not a loaded snow operand").unwrap();
    assert_eq!(
        original,
        loaded(false),
        "overridden and irrelevant bytes cannot invalidate"
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
    version.version += 1;
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
        Key::for_inputs(Ok((&inputs.cells, &inputs.winter)), 1)
            .unwrap()
            .inputs
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
    stale.version += 1;
    assert_eq!(&*run(&fixture.reader(&stale, limits)), &snow);
    assert_eq!(calls.get(), 2);
    let mut packet = fixture.packet(&session.key, "endpoints/1", 0);
    packet[101] ^= 1;
    let mut writer = fixture.writer();
    writer
        .write_at(&session.key.records(1), 0, packet.as_slice())
        .unwrap();
    writer.commit().unwrap();
    assert_eq!(&*run(&reader), &snow);
    assert_eq!(calls.get(), 3);
    let body = codec::encode_reason("recorded weather unavailable");
    let packet = codec::pack(&session.key, 1, 0, 1, &body).unwrap();
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
    let n = trailing.len() - 133;
    trailing[61..69].copy_from_slice(&(n as u64).to_le_bytes());
    let checksum = codec::digest(&trailing[..101]);
    trailing[101..133].copy_from_slice(&checksum);
    writer.write("bad-compressed", trailing.as_slice()).unwrap();
    let mut oversized = codec::pack(&session.key, 1, 0, 3, &bytes).unwrap();
    oversized[53..61].copy_from_slice(&u64::MAX.to_le_bytes());
    let checksum = codec::digest(&oversized[..101]);
    oversized[101..133].copy_from_slice(&checksum);
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
        let body = codec::unpack(
            range,
            &session.key,
            Some((branch, hour)),
            kind,
            limits.maximum().unwrap(),
        )
        .unwrap();
        assert_eq!(codec::decode_reason(&body).unwrap(), expected);
        match fixture.reader(&session.key, limits).restore(branch, hour) {
            Restore::Miss(reason) => assert!(reason.contains(expected), "{reason}"),
            Restore::Hit(_) => panic!("status must not fabricate available snow"),
        }
    }
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.source, &session.key.section("payloads"))
            .unwrap(),
        None
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
    let mut dangling = u64::MAX.to_le_bytes().to_vec();
    dangling.extend_from_slice(&[0; 32]);
    let mut wrong_key = session.key.clone();
    wrong_key.inputs[0] ^= 1;
    let bytes = codec::encode_product(&snow, limits).unwrap();
    let mut reference = 0u64.to_le_bytes().to_vec();
    reference.extend_from_slice(&codec::digest(&bytes));
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
    let mut payload = fixture.packet(&session.key, "payloads", 0);
    let last = payload.len() - 1;
    payload[last] ^= 1;
    let mut writer = fixture.writer();
    writer
        .write_at(&session.key.records(1), 0, original.as_slice())
        .unwrap();
    writer
        .write_at(&session.key.section("payloads"), 0, payload.as_slice())
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
    let province_length = 16 + snow.texels.len();
    let factor_length = province_length
        + 8
        + snow
            .province_snow
            .iter()
            .map(|word| 1 + usize::from(word.is_some()) * 8)
            .sum::<usize>()
        + 1;
    for offset in [province_length, factor_length] {
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
    let original_payload = fixture.packet(&session.key, "payloads", 0);
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
        next.can_inherit(&fixture.store, &fixture.source, [1], || Ok(()))
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
    assert_eq!(fixture.packet(&next.key, "payloads", 0), original_payload);
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
            .can_inherit(&fixture.store, &fixture.source, [1], || Ok(()))
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
                .range_count(&fixture.source, &session.key.section("payloads"))
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
            next.can_inherit(&fixture.store, &fixture.source, [1], || Ok(()))
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
            .write(&session.key.section("payloads"), packet.as_slice())
            .unwrap();
        writer.commit().unwrap();
        let mut writer = fixture.writer();
        write_endpoint(&mut writer, &session.key, 1, 0, Ok(&snow), Some(limits)).unwrap();
        writer.commit().unwrap();
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.source, &session.key.section("payloads"))
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
    assert_eq!(full, codec::digest(replay.bytes()));
    assert_eq!(current, codec::digest(&replay.bytes()[..prefix as usize]));
    assert_eq!(old, Some(codec::digest(&replay.bytes()[..prior as usize])));
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
    stale.version += 1;
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
                .range_count(&fixture.source, &session.key.section("payloads"))
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
                current.can_inherit(&fixture.store, &fixture.source, [1], check)
            } else {
                current.reusable(&fixture.store, &fixture.source, [1], check)
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
        let mut checks = 0;
        let error = validate(&mut || {
            checks += 1;
            if checks == 8 {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(checks, 8);
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
    for suffix in ["payloads", "endpoints/1", "cursor/1"] {
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
