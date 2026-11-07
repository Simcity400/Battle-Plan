//! Synthetic version 4 recordings, written as the recorder writes them, for tests.

use super::*;
use crate::{
    columnar_blocks::BlockBuilder,
    columnar_file::{FileWriter, Settings},
};
use replay_columnar::envelope::Identity;

/// One extent of `raw` appended to `file`, compressed as the converter stores it.
#[cfg(test)]
pub(crate) fn append(
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

fn identity(campaign: &[u8], capture_mode: u8) -> Identity {
    Identity {
        schema: envelope::schema_digest(&schema().unwrap()),
        campaign: campaign.to_vec(),
        content: Vec::new(),
        capture_mode,
    }
}

/// A file writer into memory, each page changed by `tamper` before it is written.
fn writer(identity: Identity, tamper: impl Fn(&mut IndexPage) + Send + 'static) -> FileWriter<std::io::Cursor<Vec<u8>>> {
    let settings = Settings {
        level: 1,
        parallel: false,
    };
    FileWriter::create(std::io::Cursor::new(Vec::new()), identity, TABLE_COUNT, settings)
        .unwrap()
        .with_tamper(tamper)
}

/// Every page of `file`'s runs, as their directories list them.
#[cfg(test)]
fn pages_of(file: &[u8]) -> Vec<Vec<Extent>> {
    let recording = Recording::open(file.to_vec()).unwrap();
    (0..recording.run_count())
        .map(|run| recording.run(run).map(|run| run.page_extents().to_vec()).unwrap_or_default())
        .collect()
}

/// A recording of `runs` (branch, first hour, ordinals) as the recorder writes one, of hour
/// records alone: ordinal `n` of run `r` is the hour `hour(r, n)` gives, appending one
/// dictionary string, and every `restart_units`th full block ends in a restart. `tamper`
/// changes each page before it is written. The file, and each run's page extents.
#[cfg(test)]
pub(crate) fn recording(
    runs: &[(u64, i32, u32)],
    hour: impl Fn(u32, u32) -> i32,
    restart_units: u32,
    tamper: impl Fn(&mut IndexPage) + Send + 'static,
) -> (Vec<u8>, Vec<Vec<Extent>>) {
    let columns: Vec<usize> = schema().unwrap().iter().map(Vec::len).collect();
    let mut file = writer(identity(b"campaign", 0), tamper);
    for (run, &(branch, first_hour, ordinals)) in (0..).zip(runs) {
        file.start_run(branch, envelope::epoch::START, first_hour).unwrap();
        let mut builder = BlockBuilder::new().unwrap().with_restart_blocks(restart_units);
        builder.start_run(branch, hour(run, 0)).unwrap();
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
            if let Some(block) = builder.hour(&manifest, &[]).unwrap() {
                file.block(block.block()).unwrap();
            }
        }
        if let Some(block) = builder.end_run().unwrap() {
            file.block(block.block()).unwrap();
        }
        file.finish_run().unwrap();
    }
    let bytes = file.finish(|_| Ok(())).unwrap().file.into_inner();
    let pages = pages_of(&bytes);
    (bytes, pages)
}

/// Every ordinal of `runs` at its run's hour.
#[cfg(test)]
pub(crate) fn every(runs: &[(u64, i32, u32)]) -> impl Fn(u32, u32) -> i32 + '_ {
    |run, ordinal| runs[run as usize].1 + ordinal as i32
}

/// Two live countries, one retaining its first string through unique-string churn, a restart
/// ending every block. `tamper` changes each page before it is written.
#[cfg(test)]
pub(crate) fn native_recording(units: u32, tamper: impl Fn(&mut IndexPage) + Send + 'static) -> Vec<u8> {
    use super::synthetic::{Input, Sample, Synthetic};
    use crate::rows::CountryRow;
    let mut file = writer(identity(b"native synthetic", 0), tamper);
    file.start_run(7, envelope::epoch::START, 100).unwrap();
    let mut builder = BlockBuilder::new().unwrap().with_restart_blocks(1);
    builder.start_run(7, 100).unwrap();
    let mut synthetic = Synthetic::new(&[TableId::Countries], 7, 100).unwrap();
    let columns: Vec<_> = schema().unwrap().iter().map(Vec::len).collect();
    for ordinal in 0..units * envelope::UNIT_ORDINALS {
        let rows = [
            CountryRow {
                id: 1,
                tag: b"old string".to_vec(),
                stability_raw: 123,
                ..Default::default()
            },
            CountryRow {
                id: 2,
                tag: format!("new string {ordinal}").into_bytes(),
                stability_raw: i64::from(ordinal),
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
        let (manifest, sections) = synthetic
            .end_encoded(|_, manifest, _, _, sections| {
                let bytes = manifest.write(&columns).unwrap();
                let joined: Vec<u8> = sections.iter().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
                (bytes, joined)
            })
            .unwrap();
        if let Some(block) = builder.hour(&manifest, &sections).unwrap() {
            file.block(block.block()).unwrap();
        }
    }
    if let Some(block) = builder.end_run().unwrap() {
        file.block(block.block()).unwrap();
    }
    file.finish(|_| Ok(())).unwrap().file.into_inner()
}

/// What a synthetic writer saw of an hour: its frame, and each table's sample hour.
pub type Seen = (DecodedFrame, Vec<Option<i32>>);

/// Countries and world tension over `runs` (branch, first hour, units), written as the native
/// recorder writes them and stored as the converter stores them ([`written`]). Strings come
/// back pages after their last use, as text cells and as an unavailable table's reasons, and
/// a row leaves and comes back with the string it held.
pub fn churned(runs: &[(u64, i32, u32)], restart_units: u32) -> (Vec<u8>, Vec<Seen>) {
    use super::synthetic::{Input, Sample};
    use crate::rows::{CountryRow, WorldTensionRow};
    let streams = [TableId::Countries, TableId::WorldTension];
    written(
        b"churned synthetic",
        runs,
        restart_units,
        &streams,
        |synthetic, _, ordinal| {
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
                // The same 40 strings, each back 2,000 hours after it was last held.
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
        },
    )
}

/// `streams` over `runs` (branch, first hour, units), `write(synthetic, run, ordinal)` writing
/// each hour's tables as the native recorder writes them, stored as the converter stores them:
/// each unit's manifests and sections, and a restart ending every `restart_units`th unit. The
/// file, and what the writer saw of each hour, run after run.
pub fn written(
    campaign: &[u8],
    runs: &[(u64, i32, u32)],
    restart_units: u32,
    streams: &[TableId],
    write: impl FnMut(&mut super::synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    written_captured(campaign, None, runs, restart_units, streams, write)
}

/// One run of `ordinals` hours, `frame(ordinal)` each, written as the recorder writes them:
/// through the frame writer, the block builder and the columnar file writer, a restart ending
/// every `restart_units`th block. The run is the first frame's branch from its hour.
pub fn recorded(
    campaign: &[u8],
    ordinals: u32,
    restart_units: u32,
    mut frame: impl FnMut(u32) -> crate::frame::FrameInput,
) -> Vec<u8> {
    use crate::native_frame::FrameWriter;
    let mut writer = FrameWriter::new().unwrap();
    let mut file = writer_plain(identity(campaign, CaptureMode::Full.byte()));
    let mut builder = BlockBuilder::new().unwrap().with_restart_blocks(restart_units);
    for ordinal in 0..ordinals {
        let input = frame(ordinal);
        if ordinal == 0 {
            file.start_run(input.branch, envelope::epoch::START, input.hour).unwrap();
            builder.start_run(input.branch, input.hour).unwrap();
        }
        let prepared = writer.hour(&input).unwrap();
        let sections: Vec<u8> = prepared.sections().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
        let block = builder.hour(&prepared.manifest, &sections).unwrap();
        writer.commit(prepared).unwrap();
        if let Some(block) = block {
            file.block(block.block()).unwrap();
        }
    }
    if let Some(block) = builder.end_run().unwrap() {
        file.block(block.block()).unwrap();
    }
    file.finish(|_| Ok(())).unwrap().file.into_inner()
}

/// A file writer into memory.
fn writer_plain(identity: Identity) -> FileWriter<std::io::Cursor<Vec<u8>>> {
    writer(identity, |_| {})
}

/// [`written`], its root's control records holding `capture` when given.
pub fn written_captured(
    campaign: &[u8],
    capture: Option<&crate::columnar_file::Capture>,
    runs: &[(u64, i32, u32)],
    restart_units: u32,
    streams: &[TableId],
    mut write: impl FnMut(&mut super::synthetic::Synthetic, u32, u32),
) -> (Vec<u8>, Vec<Seen>) {
    use super::synthetic::Synthetic;
    let columns: Vec<_> = schema().unwrap().iter().map(Vec::len).collect();
    let mut file = writer_plain(identity(campaign, 0));
    let mut seen = Vec::new();
    for (run, &(branch, first_hour, units)) in (0..).zip(runs) {
        let mut synthetic = Synthetic::new(streams, branch, first_hour).unwrap();
        file.start_run(branch, envelope::epoch::START, first_hour).unwrap();
        let mut builder = BlockBuilder::new().unwrap().with_restart_blocks(restart_units);
        builder.start_run(branch, first_hour).unwrap();
        for ordinal in 0..units * envelope::UNIT_ORDINALS {
            write(&mut synthetic, run, ordinal);
            let (manifest, sections) = synthetic
                .end_encoded(|_, manifest, frame, _, encoded| {
                    seen.push((
                        frame.clone(),
                        frame.tables.iter().map(SharedView::sampled_at).collect(),
                    ));
                    let joined: Vec<u8> = encoded.iter().flat_map(|(_, bytes)| bytes.iter().copied()).collect();
                    (manifest.write(&columns).unwrap(), joined)
                })
                .unwrap();
            if let Some(block) = builder.hour(&manifest, &sections).unwrap() {
                file.block(block.block()).unwrap();
            }
        }
        if let Some(block) = builder.end_run().unwrap() {
            file.block(block.block()).unwrap();
        }
        file.finish_run().unwrap();
    }
    let finished = match capture {
        Some(capture) => {
            let capture = crate::columnar_file::Capture {
                run_sessions: vec![0; runs.len()],
                ..capture.clone()
            };
            file.finish_with_capture(&capture, |_| Ok(())).unwrap()
        }
        None => file.finish(|_| Ok(())).unwrap(),
    };
    (finished.file.into_inner(), seen)
}
