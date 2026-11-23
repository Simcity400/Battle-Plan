
use crate::synthetic;
use replay_recording::schema::{columns_at, schema, schema_at};
use replay_data::{rows::{TABLE_COUNT, TableId}, scope::CaptureMode};
use replay_frame::frame::{DecodedFrame, SharedView};
use crate::columnar_file::{FileWriter, HourWriter};
use replay_settings::compression::Settings;
use recorder_derive_framework::framework::Writers;
use replay_columnar::{
    container::{SectionKey, Tags},
    envelope::{self, Identity},
    pages::Block,
};
#[cfg(any(test, feature = "test-support"))]
use replay_columnar::{
    envelope::{Extent, Kind},
    manifest::{Manifest, Status},
};

#[cfg(any(test, feature = "test-support"))]
pub fn append(
    file: &mut Vec<u8>,
    kind: Kind,
    (run, stream, first, hours): (u32, u32, u32, u32),
    raw: &[u8],
) -> Extent {
    let stored = zstd::bulk::compress(raw, 1).unwrap();
    let extent = Extent {
        kind,
        run,
        stream,
        first,
        hours,
        offset: file.len() as u64,
        stored: stored.len() as u64,
        raw: raw.len() as u64,
        digest: envelope::digest(&stored),
    };
    file.extend_from_slice(&stored);
    extent
}

pub fn identity(campaign: &[u8], capture_mode: u8) -> Identity {
    Identity {
        schema: envelope::schema_digest(&schema().unwrap()),
        campaign: campaign.to_vec(),
        content: Vec::new(),
        capture_mode,
    }
}

const LEVEL: i32 = 1;

pub fn writer(writers: Writers, identity: Identity, tamper: impl Fn(&mut Block) + Send + 'static) -> HourWriter<std::io::Cursor<Vec<u8>>> {
    writer_with(writers, identity, replay_columnar::unit::Settings::at(LEVEL), tamper)
}

fn writer_with(writers: Writers, identity: Identity, unit: replay_columnar::unit::Settings, tamper: impl Fn(&mut Block) + Send + 'static) -> HourWriter<std::io::Cursor<Vec<u8>>> {
    let file = FileWriter::create(std::io::Cursor::new(Vec::new()), identity, TABLE_COUNT, Settings { level: unit.level }, None)
        .unwrap()
        .with_tamper(tamper);
    HourWriter::with_unit(file, unit.level, unit, writers).unwrap()
}

fn finished(writer: HourWriter<std::io::Cursor<Vec<u8>>>) -> Vec<u8> {
    writer.finish().unwrap().finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner()
}

#[cfg(any(test, feature = "test-support"))]
fn pages_of(file: &[u8]) -> Vec<Vec<Extent>> {
    use replay_columnar::{container, pages::RunDirectory};
    let (root, _) = container::locate(file, container::Open::Newest).unwrap().unwrap();
    let root = container::read_root(file, &root).unwrap();
    let Some(section) = root.open(SectionKey::WORLD_LOG, container::schema::WORLD_LOG).unwrap() else { return Vec::new() };
    (0..)
        .zip(&section.open)
        .map(|(run, extent)| {
            let stored = &file[extent.offset as usize..(extent.offset + extent.stored) as usize];
            let raw = container::decompress(stored, Some(extent.raw)).unwrap();
            RunDirectory::read(&raw, extent, run).unwrap().open
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
pub fn recording(
    writers: Writers,
    runs: &[(u64, i32, u32)],
    hour: impl Fn(u32, u32) -> i32,
    tamper: impl Fn(&mut Block) + Send + 'static,
) -> (Vec<u8>, Vec<Vec<Extent>>) {
    let columns: Vec<usize> = schema().unwrap().iter().map(Vec::len).collect();
    let mut file = writer(writers, identity(b"campaign", 0), tamper);
    for (run, &(branch, _, ordinals)) in (0..).zip(runs) {
        file.start_run(branch, envelope::epoch::START, hour(run, 0)).unwrap();
        for ordinal in 0..ordinals {
            let string = format!("run {run} ordinal {ordinal}").into_bytes();
            let manifest = Manifest {
                branch,
                hour: hour(run, ordinal),
                flags: 0,
                total_ns: 1,
                pool_bytes: 0,
                dropped_frames: 0,
                appends: vec![&string],
                statuses: vec![Status::NotRecorded; TABLE_COUNT],
                read: vec![0; TABLE_COUNT.div_ceil(64)],
                samples: Vec::new(),
                sections: Vec::new(),
            }
            .write(&columns)
            .unwrap();
            file.hour(&manifest, &[]).unwrap();
        }
    }
    let bytes = finished(file);
    let pages = pages_of(&bytes);
    (bytes, pages)
}

#[cfg(any(test, feature = "test-support"))]
pub fn every(runs: &[(u64, i32, u32)]) -> impl Fn(u32, u32) -> i32 + '_ {
    |run, ordinal| runs[run as usize].1 + ordinal as i32
}

#[cfg(any(test, feature = "test-support"))]
pub fn absent_recording(writers: Writers, units: u32, absent: std::ops::Range<i32>) -> Vec<u8> {
    use synthetic::{Input, Sample, Synthetic};
    use replay_data::rows::CountryRow;
    let mut file = writer(writers, identity(b"absent synthetic", 0), |_| {});
    file.start_run(7, envelope::epoch::START, 100).unwrap();
    let mut synthetic = Synthetic::new(7, 100).unwrap();
    for ordinal in 0..units * envelope::UNIT_ORDINALS {
        let hour = 100 + ordinal as i32;
        let mut rows = vec![CountryRow { id: 1, tag: b"held".to_vec(), political_power_raw: Some(500), ..Default::default() }];
        if !absent.contains(&hour) {
            rows.push(CountryRow { id: 3, tag: b"leaves".to_vec(), political_power_raw: Some(7), ..Default::default() });
        }
        synthetic
            .table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit })
            .unwrap();
        synthetic.end_encoded(|manifest, sections| file.hour(manifest, sections)).unwrap().unwrap();
    }
    finished(file)
}

#[cfg(any(test, feature = "test-support"))]
pub fn native_recording(writers: Writers, units: u32, tamper: impl Fn(&mut Block) + Send + 'static) -> Vec<u8> {
    use synthetic::{Input, Sample, Synthetic};
    use replay_data::rows::CountryRow;
    let mut file = writer(writers, identity(b"native synthetic", 0), tamper);
    file.start_run(7, envelope::epoch::START, 100).unwrap();
    let mut synthetic = Synthetic::new(7, 100).unwrap();
    for ordinal in 0..units * envelope::UNIT_ORDINALS {
        let rows = [
            CountryRow {
                id: 1,
                tag: b"old string".to_vec(),
                stability_raw: 123,
                political_power_raw: Some(500),
                ..Default::default()
            },
            CountryRow {
                id: 2,
                tag: format!("new string {ordinal}").into_bytes(),
                stability_raw: i64::from(ordinal),
                political_power_raw: Some(i64::from(ordinal / 24 % 17)),
                ..Default::default()
            },
        ];
        synthetic
            .table(
                TableId::Countries,
                Input::Snapshot {
                    rows: &rows,
                    partial: None,
                    read: ordinal % 2 == 0,
                    sample: if ordinal % 2 == 0 {
                        Sample::FromReadBit
                    } else {
                        Sample::Carried(100 + ordinal as i32 - 1)
                    },
                },
            )
            .unwrap();
        synthetic.end_encoded(|manifest, sections| file.hour(manifest, sections)).unwrap().unwrap();
    }
    finished(file)
}

pub type Seen = (DecodedFrame, Vec<Option<i32>>);

pub fn churned(writers: Writers, runs: &[(u64, i32, u32)]) -> (Vec<u8>, Vec<Seen>) {
    use synthetic::{Input, Sample};
    use replay_data::rows::{CountryRow, WorldTensionRow};
    let streams = [TableId::Countries, TableId::WorldTension];
    written(writers, b"churned synthetic", runs, &streams, |synthetic, _, ordinal| {
        let mut rows = vec![
            CountryRow {
                id: 1,
                tag: b"old string".to_vec(),
                stability_raw: 1,
                ..Default::default()
            },
            CountryRow {
                id: 2,
                tag: format!("new string {ordinal}").into_bytes(),
                stability_raw: i64::from(ordinal),
                ..Default::default()
            },
            CountryRow {
                id: 3,
                tag: format!("cycle {}", ordinal / 50 % 40).into_bytes(),
                ..Default::default()
            },
        ];
        if ordinal / 100 % 2 == 0 {
            rows.push(CountryRow {
                id: 4,
                tag: b"returning".to_vec(),
                ..Default::default()
            });
        }
        synthetic
            .table(
                TableId::Countries,
                Input::Snapshot {
                    rows: &rows,
                    partial: None,
                    read: true,
                    sample: Sample::FromReadBit,
                },
            )
            .unwrap();
        let block = ordinal / 300;
        let tension = if block % 2 == 0 {
            synthetic.table(
                TableId::WorldTension,
                Input::Snapshot {
                    rows: &[WorldTensionRow {
                        total_raw: i64::from(block),
                        source_count: 1,
                        country_aggregate_count: 2,
                    }],
                    partial: None,
                    read: true,
                    sample: Sample::FromReadBit,
                },
            )
        } else {
            synthetic.table::<WorldTensionRow>(
                TableId::WorldTension,
                Input::Unavailable {
                    reason: &format!("refused {}", block / 2 % 3),
                    read: false,
                    sample: Sample::FromReadBit,
                },
            )
        };
        tension.unwrap();
    })
}

pub fn edited(writers: Writers, units: u32) -> (Vec<u8>, Vec<Seen>) {
    let unit = replay_columnar::unit::Settings { restore_blocks: 1, whole_every: 1, pack_margin: 8, ..replay_columnar::unit::Settings::at(LEVEL) };
    edited_with(writers, unit, &[(1, 100, units)])
}

pub fn restored(writers: Writers, level: i32, runs: &[(u64, i32, u32)]) -> Vec<u8> {
    let unit = replay_columnar::unit::Settings { cap: 512, small: 64, restore_blocks: 1, whole_every: 3, pack_margin: 8, ..replay_columnar::unit::Settings::at(level) };
    edited_with(writers, unit, runs).0
}

pub fn one_large_unit(writers: Writers, level: i32) -> Vec<u8> {
    use synthetic::{Input, Sample, Synthetic};
    use replay_data::rows::CountryRow;
    let (branch, first_hour) = (1, 100);
    let mut file = writer_with(writers, identity(b"one large unit", 0), replay_columnar::unit::Settings::at(level), |_| {});
    let mut synthetic = Synthetic::new(branch, first_hour).unwrap();
    file.start_run(branch, envelope::epoch::START, first_hour).unwrap();
    for ordinal in 0..8 * envelope::UNIT_ORDINALS {
        let large = ordinal / envelope::UNIT_ORDINALS == 1;
        let rows: Vec<CountryRow> = (1..=10)
            .chain((1000..2000).filter(|_| large))
            .map(|id| CountryRow {
                id,
                tag: format!("tag {id}").into_bytes(),
                stability_raw: ((u64::from(id) << 32 | u64::from(ordinal)).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 1) as i64,
                ..Default::default()
            })
            .collect();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.end_encoded(|manifest, sections| file.hour(manifest, sections)).unwrap().unwrap();
    }
    finished(file)
}

fn edited_with(writers: Writers, unit: replay_columnar::unit::Settings, runs: &[(u64, i32, u32)]) -> (Vec<u8>, Vec<Seen>) {
    use synthetic::{Input, Sample};
    use replay_data::rows::CountryRow;
    let present = |id: u32, ordinal: u32| match id {
        1..=10 => true,
        11..=20 => (ordinal + id) % 4 != 0,
        _ => (10..20).contains(&(ordinal % envelope::UNIT_ORDINALS)),
    };
    let row = |id: u32, ordinal: u32| CountryRow {
        id,
        tag: format!("tag {id} {}", ordinal / (id % 5 + 1) % 3).into_bytes(),
        stability_raw: if id % 2 == 0 { i64::from(ordinal) } else { i64::from(id) },
        political_power_raw: (id % 3 == 0).then(|| i64::from(ordinal / 7)),
        ..Default::default()
    };
    written_captured_with(writers, b"edited synthetic", None, unit, runs, &[TableId::Countries], |synthetic, _, ordinal| {
        let rows: Vec<CountryRow> = (1..=20).chain(100..200).filter(|&id| present(id, ordinal)).map(|id| row(id, ordinal)).collect();
        let input = match ordinal % 61 {
            30 => Input::Snapshot { rows: &rows[..3], partial: Some(2), read: true, sample: Sample::FromReadBit },
            45 => Input::Unavailable { reason: "refused for an hour", read: false, sample: Sample::FromReadBit },
            _ => Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit },
        };
        synthetic.table(TableId::Countries, input).unwrap();
    })
}

pub fn written(
    writers: Writers,
    campaign: &[u8],
    runs: &[(u64, i32, u32)],
    streams: &[TableId],
    write: impl FnMut(&mut synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    written_captured(writers, campaign, None, runs, streams, write)
}

pub fn written_with_summaries(
    writers: Writers,
    campaign: &[u8],
    runs: &[(u64, i32, u32)],
    streams: &[TableId],
    registry: Vec<(usize, replay_columnar::summary::Shape)>,
    write: impl FnMut(&mut synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    let file = FileWriter::create(std::io::Cursor::new(Vec::new()), identity(campaign, 0), TABLE_COUNT, Settings { level: LEVEL }, None).unwrap();
    let summaries = crate::summaries::Summaries::with_registry(LEVEL, registry);
    let file = HourWriter::with_summaries(file, LEVEL, writers, summaries).unwrap();
    written_into(file, None, None, runs, streams, write)
}

pub fn recorded(writers: Writers, campaign: &[u8], ordinals: u32, mut frame: impl FnMut(u32) -> replay_frame::frame::FrameInput) -> Vec<u8> {
    use crate::native_frame::FrameWriter;
    let mut writer = FrameWriter::new().unwrap();
    let mut file = self::writer(writers, identity(campaign, CaptureMode::Full.byte()), |_| {});
    for ordinal in 0..ordinals {
        let input = frame(ordinal);
        if ordinal == 0 {
            file.start_run(input.branch, envelope::epoch::START, input.hour).unwrap();
        }
        let prepared = writer.hour(&input).unwrap();
        let sections: Vec<u8> = prepared.sections().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
        file.hour(&prepared.manifest, &sections).unwrap();
        writer.commit(prepared).unwrap();
    }
    finished(file)
}

pub fn written_captured(
    writers: Writers,
    campaign: &[u8],
    capture: Option<&replay_recording::capture::Capture>,
    runs: &[(u64, i32, u32)],
    streams: &[TableId],
    write: impl FnMut(&mut synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    written_captured_with(writers, campaign, capture, replay_columnar::unit::Settings::at(LEVEL), runs, streams, write)
}

pub fn written_at(
    writers: impl FnOnce(&[usize], &[usize]) -> Writers,
    format: u16,
    campaign: &[u8],
    runs: &[(u64, i32, u32)],
    streams: &[TableId],
    write: impl FnMut(&mut synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    use replay_data::identity::{CaptureMode as Mode, Identity as Game, IdentityCodec, SampleBoundary, SessionStart, Verification};
    let columns = columns_at(format).unwrap();
    let schema = schema_at(format).unwrap();
    let identity = Identity { schema: envelope::schema_digest_prefix(&schema[..columns.len()], &columns).unwrap(), ..identity(campaign, 0) };
    let game = Game {
        game_version: "synthetic".into(),
        exe_hash: [1; 32],
        exe_size: 1,
        vanilla_hash: [2; 32],
        dlc: Vec::new(),
        mods: Vec::new(),
        coverage_policy: 1,
        language: "synthetic".into(),
        recorder_version: "synthetic".into(),
        collector_hash: [3; 32],
        verification: Verification::Unchecked,
    };
    let mut session = Vec::new();
    SessionStart { id: 1, process_id: 1, process_start_time: 1, qpc_frequency: 1, sample_boundary: SampleBoundary::EndOfHour, capture_mode: Mode::Full, identity: game }.encode(&mut session).unwrap();
    let capture = replay_recording::capture::Capture {
        campaign_id: campaign.to_vec(),
        content: Vec::new(),
        capture_mode: 0,
        sessions: vec![session],
        run_sessions: Vec::new(),
        stop: None,
        format,
        journaled: 0,
        controls: Vec::new(),
    };
    let unit = replay_columnar::unit::Settings { restore_blocks: 1, ..replay_columnar::unit::Settings::at(LEVEL) };
    let file = FileWriter::create(std::io::Cursor::new(Vec::new()), identity, columns.len(), Settings { level: unit.level }, None).unwrap();
    let file = HourWriter::framed(file, unit.level, unit, &columns, writers).unwrap();
    written_into(file, Some(&capture), Some((format, &columns)), runs, streams, write)
}

fn written_captured_with(
    writers: Writers,
    campaign: &[u8],
    capture: Option<&replay_recording::capture::Capture>,
    unit: replay_columnar::unit::Settings,
    runs: &[(u64, i32, u32)],
    streams: &[TableId],
    write: impl FnMut(&mut synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    written_into(writer_with(writers, identity(campaign, 0), unit, |_| {}), capture, None, runs, streams, write)
}

fn written_into(
    mut file: HourWriter<std::io::Cursor<Vec<u8>>>,
    capture: Option<&replay_recording::capture::Capture>,
    framed: Option<(u16, &[usize])>,
    runs: &[(u64, i32, u32)],
    streams: &[TableId],
    mut write: impl FnMut(&mut synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    use synthetic::Synthetic;
    let mut seen = Vec::new();
    for (run, &(branch, first_hour, units)) in (0..).zip(runs) {
        let mut synthetic = match framed {
            Some((format, columns)) => Synthetic::framed_at(branch, first_hour, format, columns.to_vec()),
            None => Synthetic::new(branch, first_hour),
        }
        .unwrap();
        file.start_run(branch, envelope::epoch::START, first_hour).unwrap();
        for ordinal in 0..units * envelope::UNIT_ORDINALS {
            write(&mut synthetic, run, ordinal);
            synthetic.end_encoded(|manifest, sections| file.hour(manifest, sections)).unwrap().unwrap();
            let frame = file.frame(streams, (branch, first_hour + ordinal as i32), CaptureMode::Full).unwrap();
            let sampled = frame.tables.iter().map(SharedView::sampled_at).collect();
            seen.push((frame, sampled));
        }
    }
    let mut file = file.finish().unwrap();
    if let Some(capture) = capture {
        let capture = replay_recording::capture::Capture {
            run_sessions: vec![0; runs.len()],
            ..capture.clone()
        };
        let container = file.container();
        let extent = container.put_zstd(envelope::Kind::Controls, Tags::default(), &capture.write().unwrap(), LEVEL).unwrap();
        container.set_open(SectionKey::RECORDS, &[extent]).unwrap();
    }
    (file.finish_runs(&[], true, |_| Ok(())).unwrap().0.into_inner(), seen)
}
