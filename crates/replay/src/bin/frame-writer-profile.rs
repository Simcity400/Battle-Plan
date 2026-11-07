//! Profiles the columnar recording offline, both halves: every hour of a recording, as complete
//! views, through what the collector does in the game ([`replay::native_frame::FrameWriter`],
//! then appending the hour to a journal, [`replay::columnar_journal`]) and through what the
//! recorder does outside it ([`replay::columnar_blocks::BlockBuilder`] into
//! [`replay::columnar_file::FileWriter`] at its default settings, into a counting sink), and
//! through the row encoder on the same input. Reports per table the frame writer's wall and its
//! section bytes per recorded hour, against the row encoder's wall and the recording's size.
//! `frame-writer-profile <recording.battleplan> [--check]`. As the collector carries a table's
//! shared rows while they are unchanged, an unchanged complete table arrives as the hour
//! before's shared rows; a changed one arrives complete, where the collector hands most hourly
//! tables as their changed rows, so changed tables' costs here bound the live ones. As on the
//! spool thread, the row encoder prepares each hour first, and the writer diffs complete tables
//! with its baselines and refuses a truncated read at its keyframes; on one thread. `--check`
//! also writes every hour through a writer given no baselines, and stops at the first hour
//! whose bytes differ.

use replay::{
    columnar_blocks::BlockBuilder,
    columnar_file::{Capture, FileWriter, Settings},
    columnar_frames::schema,
    columnar_journal::JournalWriter,
    campaign::Replay,
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
    let mut content = Vec::new();
    replay_data::identity::IdentityCodec::encode(&replay.identity, &mut content)?;
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema()?),
        campaign: replay.campaign_id.clone(),
        content: content.clone(),
        capture_mode: replay.capture_mode.byte(),
    };
    let journal_dir = std::env::temp_dir().join(format!("frame-writer-profile-{}", std::process::id()));
    std::fs::create_dir_all(&journal_dir)?;
    let capture = Capture {
        campaign_id: replay.campaign_id.clone(),
        content,
        capture_mode: replay.capture_mode.byte(),
        sessions: Vec::new(),
        run_sessions: Vec::new(),
        stop: None,
        format: replay.format.version(),
        journaled: 0,
        controls: Vec::new(),
    };
    let mut journal = JournalWriter::create(&journal_dir, 0, &identity.chain_bytes()?, &capture)?;
    let mut file = FileWriter::create(Counting::default(), identity, TABLE_COUNT, Settings::HIGH)?;
    let mut blocks = BlockBuilder::new()?;
    let mut frames: Option<(FrameWriter, u64, i32, u32)> = None;
    let mut finished_ns = vec![0_u64; TABLE_COUNT];
    let mut section_raw = vec![0_u64; TABLE_COUNT];
    let mut hours = 0_u64;
    let (mut writer_ns, mut journal_ns, mut journal_max_ns, mut encoder_ns) = (0_u64, 0_u64, 0_u64, 0_u64);
    let (mut builder_ns, mut file_ns, mut file_max_ns) = (0_u64, 0_u64, 0_u64);
    let mut encoder = FrameEncoder::new();
    let mut carried: Vec<Option<Arc<replay_data::rows::TableRows>>> = vec![None; TABLE_COUNT];
    let started = Instant::now();
    let index: Vec<_> = replay.index.iter().collect();
    replay.visit_blocks(
        &index,
        &TableId::ALL,
        None,
        |_, recorded, decoded| {
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
            let continues = frames.as_ref().is_some_and(|(_, branch, next, _)| *branch == decoded.branch && *next == decoded.hour);
            if !continues {
                let epoch = if frames.is_none() { envelope::epoch::START } else { envelope::epoch::GAP };
                if let Some((ended, _, _, _)) = frames.take() {
                    for (total, ns) in finished_ns.iter_mut().zip(ended.table_ns()) {
                        *total += ns;
                    }
                }
                let begin = Instant::now();
                journal.run(decoded.branch, epoch, decoded.hour)?;
                journal_ns += begin.elapsed().as_nanos() as u64;
                let begin = Instant::now();
                if let Some(block) = blocks.end_run()? {
                    file.block(block.block())?;
                }
                file.start_run(decoded.branch, epoch, decoded.hour)?;
                blocks.start_run(decoded.branch, decoded.hour)?;
                file_ns += begin.elapsed().as_nanos() as u64;
                frames = Some((FrameWriter::new()?, decoded.branch, decoded.hour, 0));
                reference = check.then(FrameWriter::new).transpose()?;
            }
            let (writer, _, next, ordinal) = frames.as_mut().expect("a run under way");
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
            let sections: Vec<&[u8]> = prepared.sections().map(|(_, bytes)| bytes).collect();
            for (table, section) in prepared.sections() {
                section_raw[table] += section.len() as u64;
            }
            // The game's part: the hour appended to the journal, a block's last ending a segment.
            let begin = Instant::now();
            journal.hour(&prepared.manifest, &sections)?;
            if (*ordinal + 1) % envelope::UNIT_ORDINALS == 0 {
                journal.next_segment()?;
            }
            let elapsed = begin.elapsed().as_nanos() as u64;
            journal_ns += elapsed;
            journal_max_ns = journal_max_ns.max(elapsed);
            // The recorder's part: the hour applied and encoded, a full block written.
            let joined: Vec<u8> = sections.concat();
            let begin = Instant::now();
            let block = blocks.hour(&prepared.manifest, &joined)?;
            builder_ns += begin.elapsed().as_nanos() as u64;
            if let Some(block) = block {
                let begin = Instant::now();
                file.block(block.block())?;
                let elapsed = begin.elapsed().as_nanos() as u64;
                file_ns += elapsed;
                file_max_ns = file_max_ns.max(elapsed);
            }
            drop(sections);
            let begin = Instant::now();
            writer.commit(prepared)?;
            writer_ns += begin.elapsed().as_nanos() as u64;
            *next = decoded.hour + 1;
            *ordinal += 1;
            let begin = Instant::now();
            encoder.commit(row_frame)?;
            encoder_ns += begin.elapsed().as_nanos() as u64;
            hours += 1;
            Ok(true)
        },
        || Ok(()),
    )?;
    if let Some((ended, _, _, _)) = frames.take() {
        for (total, ns) in finished_ns.iter_mut().zip(ended.table_ns()) {
            *total += ns;
        }
    }
    if let Some(block) = blocks.end_run()? {
        file.block(block.block())?;
    }
    let begin = Instant::now();
    journal.finish(None)?;
    journal_ns += begin.elapsed().as_nanos() as u64;
    let finished = file.finish(|_| Ok(()))?;
    let _ = std::fs::remove_dir_all(&journal_dir);
    let per_hour = |value: u64| value as f64 / hours.max(1) as f64;
    let mut tables: Vec<usize> = (0..TABLE_COUNT).collect();
    tables.sort_unstable_by_key(|&table| std::cmp::Reverse(finished_ns[table]));
    let report = json!({
        "recording": input.display().to_string(),
        "hours": hours,
        "wall_seconds": started.elapsed().as_secs_f64(),
        "frame_writer_ms_per_hour": per_hour(writer_ns) / 1e6,
        "journal_ms_per_hour": per_hour(journal_ns) / 1e6,
        "journal_max_ms": journal_max_ns as f64 / 1e6,
        "builder_ms_per_hour": per_hour(builder_ns) / 1e6,
        "file_ms_per_hour": per_hour(file_ns) / 1e6,
        "file_max_ms": file_max_ns as f64 / 1e6,
        "row_encoder_ms_per_hour": per_hour(encoder_ns) / 1e6,
        "source_bytes_per_hour": per_hour(source_bytes),
        "columnar_bytes_per_hour": per_hour(finished.file.0),
        "records_stored_per_hour": per_hour(finished.written.records),
        "tables_stored_per_hour": per_hour(finished.written.tables),
        "restarts_stored_per_hour": per_hour(finished.written.restarts),
        "index_stored_per_hour": per_hour(finished.written.index),
        "tables": tables.iter().filter(|&&table| finished_ns[table] > 0 || section_raw[table] > 0).map(|&table| json!({
            "table": format!("{:?}", TableId::ALL[table]),
            "writer_us_per_hour": per_hour(finished_ns[table]) / 1e3,
            "section_raw_per_hour": per_hour(section_raw[table]),
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
