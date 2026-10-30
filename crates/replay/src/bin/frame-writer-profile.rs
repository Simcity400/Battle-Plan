//! Profiles the live columnar writer offline: every hour of a recording, as complete views,
//! through [`replay::native_frame::FrameWriter`] and [`replay::columnar_file::FileWriter`]
//! (at the recorder's settings, into a counting sink), and through the row encoder on the
//! same input. Reports per table the writer's wall and its section and unit bytes per
//! recorded hour, against the row encoder's wall and the recording's size.
//! `frame-writer-profile <recording.battleplan>`. As the collector carries a table's shared
//! rows while they are unchanged, an unchanged complete table arrives as the hour before's
//! shared rows; a changed one arrives complete, where the collector hands most hourly tables
//! as their changed rows, so changed tables' costs here bound the live ones. As on the spool
//! thread, the row encoder prepares each hour first, and the writer diffs complete tables
//! with its baselines and refuses a truncated read at its keyframes; on one thread.
//! `--check` also writes every hour through a writer given no baselines, and stops at the first
//! hour or restart whose bytes differ.

use replay::{
    columnar_file::{Closed, FileWriter, Hour, Position, Settings},
    columnar_frames::schema,
    container::{Replay, ReplayStream},
    native_frame::{FrameWriter, OneThread, RowFrame},
};
use replay_columnar::envelope;
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_frame::frame::{FrameEncoder, FrameInput, TableInput, TableUpdate, TableView};
use serde_json::json;
use std::{
    io::{self, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

/// Counts the bytes written, as a file's position would.
#[derive(Default)]
struct Counting(u64);

impl Write for Counting {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for Counting {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        match position {
            SeekFrom::Current(0) => Ok(self.0),
            _ => Err(io::Error::other("a counting sink only reports its position")),
        }
    }
}

#[derive(Default, Clone, Copy)]
struct Totals {
    section_raw: u64,
    unit_stored: u64,
    restart_stored: u64,
}

fn main() -> io::Result<()> {
    let input = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| io::Error::other("frame-writer-profile <recording.battleplan> [--check]"))?,
    );
    let check = std::env::args().skip(2).any(|arg| arg == "--check");
    let mut reference: Option<FrameWriter> = None;
    let source_bytes = std::fs::metadata(&input)?.len();
    let replay = Replay::open_path(&input)?;
    let mut stream = ReplayStream::new(&TableId::ALL)?;
    let mut content = Vec::new();
    replay_data::identity::IdentityCodec::encode(&replay.identity, &mut content)?;
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema()?),
        campaign: replay.campaign_id.clone(),
        content,
        capture_mode: replay.capture_mode.byte(),
    };
    let columns: Vec<usize> = schema()?.iter().map(Vec::len).collect();
    let mut file = FileWriter::create(Counting::default(), identity, columns, Settings::RECORDING)?;
    let mut frames: Option<(FrameWriter, u64, i32)> = None;
    let mut finished_ns = vec![0_u64; TABLE_COUNT];
    let mut totals = vec![Totals::default(); TABLE_COUNT];
    let (mut manifests_stored, mut hours) = (0_u64, 0_u64);
    let (mut writer_ns, mut file_ns, mut close_ns, mut close_max_ns, mut encoder_ns) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    let mut encoder = FrameEncoder::new();
    let mut carried: Vec<Option<Arc<replay_data::rows::TableRows>>> = vec![None; TABLE_COUNT];
    let account = |closed: &Closed, totals: &mut [Totals], manifests: &mut u64| {
        for unit in &closed.units {
            if unit.stream == replay_columnar::framing::MANIFESTS {
                *manifests += unit.stored;
            } else {
                totals[unit.stream as usize - 1].unit_stored += unit.stored;
            }
        }
        if let Some(restart) = &closed.restart {
            for table in &restart.tables {
                totals[table.table as usize].restart_stored += table.extent.stored;
            }
        }
    };
    let started = Instant::now();
    while stream.next_block_recorded_with(
        &replay,
        |recorded, decoded| {
            let tables = std::array::from_fn(|index| match &*decoded.tables[index] {
                TableView::Complete(rows) => {
                    let unchanged = matches!(recorded.tables[index], TableUpdate::Unchanged { .. });
                    let shared = match &carried[index] {
                        Some(held) if unchanged => Arc::clone(held),
                        _ => Arc::new(rows.clone()),
                    };
                    carried[index] = Some(Arc::clone(&shared));
                    TableInput::SharedComplete(shared)
                }
                view => {
                    carried[index] = None;
                    TableInput::Sampled(view.clone())
                }
            });
            let input = FrameInput {
                branch: decoded.branch,
                hour: decoded.hour,
                tables,
                metrics: decoded.metrics.stored(),
            };
            let continues = frames.as_ref().is_some_and(|(_, branch, next)| *branch == decoded.branch && *next == decoded.hour);
            if !continues {
                let epoch = if frames.is_none() { envelope::epoch::START } else { envelope::epoch::GAP };
                if let Some((ended, _, _)) = frames.take() {
                    for (total, ns) in finished_ns.iter_mut().zip(ended.table_ns()) {
                        *total += ns;
                    }
                }
                account(&file.finish_run()?, &mut totals, &mut manifests_stored);
                file.start_run(decoded.branch, epoch, decoded.hour)?;
                frames = Some((FrameWriter::new()?, decoded.branch, decoded.hour));
                reference = check.then(FrameWriter::new).transpose()?;
            }
            let (writer, _, next) = frames.as_mut().expect("a run under way");
            let first = hours == 0;
            let begin = Instant::now();
            let row_frame = encoder.prepare_bytes(input.clone(), first)?;
            encoder_ns += begin.elapsed().as_nanos() as u64;
            let row = RowFrame {
                baselines: Some(&encoder),
                keyframe: row_frame.frame().keyframe(),
                changed: row_frame.changed_rows(),
            };
            let begin = Instant::now();
            let prepared = writer.hour_on(&input, &[], Some(row), &OneThread)?;
            if let Some(reference) = &mut reference {
                let plain = RowFrame {
                    baselines: None,
                    keyframe: row.keyframe,
                    changed: &[],
                };
                let expected = reference.hour_on(&input, &[], Some(plain), &OneThread)?;
                if let Some((table, _)) = expected
                    .sections()
                    .zip(prepared.sections())
                    .find(|(expected, actual)| expected != actual)
                {
                    return Err(io::Error::other(format!("hour {}: {:?}'s section differs", decoded.hour, TableId::ALL[table.0])));
                }
                if expected.manifest != prepared.manifest || expected.sections().count() != prepared.sections().count() {
                    return Err(io::Error::other(format!("hour {}: the manifest differs", decoded.hour)));
                }
                reference.commit(expected)?;
            }
            writer_ns += begin.elapsed().as_nanos() as u64;
            let sections: Vec<(usize, &[u8])> = prepared.sections().collect();
            for &(table, section) in &sections {
                totals[table].section_raw += section.len() as u64;
            }
            let begin = Instant::now();
            let position = file.hour(Hour {
                hour: prepared.hour(),
                manifest: &prepared.manifest,
                sections: &sections,
                statuses: prepared.statuses(),
                read: prepared.read(),
            })?;
            file_ns += begin.elapsed().as_nanos() as u64;
            drop(sections);
            let begin = Instant::now();
            writer.commit(prepared)?;
            writer_ns += begin.elapsed().as_nanos() as u64;
            if let Position::Full { restart } = position {
                let begin = Instant::now();
                let closed = if restart {
                    let (strings, tables) = writer.restart()?;
                    if let Some(reference) = &mut reference {
                        let (expected_strings, expected_tables) = reference.restart()?;
                        if expected_strings != strings {
                            return Err(io::Error::other(format!("hour {}: the restart's strings differ", decoded.hour)));
                        }
                        if let Some(table) = (0..TABLE_COUNT).find(|&table| {
                            (&expected_tables[table].bytes, &expected_tables[table].ids) != (&tables[table].bytes, &tables[table].ids)
                        }) {
                            return Err(io::Error::other(format!("hour {}: {:?}'s restart differs", decoded.hour, TableId::ALL[table])));
                        }
                    }
                    file.close_with_restart(&strings, &tables)?
                } else {
                    file.close_position()?
                };
                let elapsed = begin.elapsed().as_nanos() as u64;
                close_ns += elapsed;
                close_max_ns = close_max_ns.max(elapsed);
                account(&closed, &mut totals, &mut manifests_stored);
            }
            *next = decoded.hour + 1;
            let begin = Instant::now();
            encoder.commit(row_frame)?;
            encoder_ns += begin.elapsed().as_nanos() as u64;
            hours += 1;
            Ok(())
        },
        || Ok(()),
    )? {}
    if let Some((ended, _, _)) = frames.take() {
        for (total, ns) in finished_ns.iter_mut().zip(ended.table_ns()) {
            *total += ns;
        }
    }
    account(&file.finish_run()?, &mut totals, &mut manifests_stored);
    let finished = file.finish()?;
    let per_hour = |value: u64| value as f64 / hours.max(1) as f64;
    let mut tables: Vec<usize> = (0..TABLE_COUNT).collect();
    tables.sort_unstable_by_key(|&table| std::cmp::Reverse(finished_ns[table]));
    let report = json!({
        "recording": input.display().to_string(),
        "hours": hours,
        "wall_seconds": started.elapsed().as_secs_f64(),
        "frame_writer_ms_per_hour": per_hour(writer_ns) / 1e6,
        "file_hour_ms_per_hour": per_hour(file_ns) / 1e6,
        "close_ms_per_hour": per_hour(close_ns) / 1e6,
        "close_max_ms": close_max_ns as f64 / 1e6,
        "row_encoder_ms_per_hour": per_hour(encoder_ns) / 1e6,
        "source_bytes_per_hour": per_hour(source_bytes),
        "columnar_bytes_per_hour": per_hour(finished.file.0),
        "manifests_stored_per_hour": per_hour(manifests_stored),
        "index_raw_per_hour": per_hour(finished.index_bytes),
        "tables": tables.iter().filter(|&&table| finished_ns[table] > 0 || totals[table].section_raw > 0).map(|&table| json!({
            "table": format!("{:?}", TableId::ALL[table]),
            "writer_us_per_hour": per_hour(finished_ns[table]) / 1e3,
            "section_raw_per_hour": per_hour(totals[table].section_raw),
            "unit_stored_per_hour": per_hour(totals[table].unit_stored),
            "restart_stored_per_hour": per_hour(totals[table].restart_stored),
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
