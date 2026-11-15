//! What the dev encoder `journal-encode` prints of the recorder's writer, and the host it runs it
//! with: every branch written, every revision saved. Dev only: deleted with it before the lane's
//! PR.

use crate::{
    Hour,
    campaign::{
        SESSION_FACTS_FILE, SESSION_FACTS_MAGIC,
        live::{self, Admit, Applied, Asked, Committed, Host, LiveWriter},
        session_facts_for,
    },
    columnar_journal::{Journal, Record, Source},
};
use replay_data::identity::{Identity, SessionStart};
use serde_json::{Map, Value, json};
use std::{
    cell::Cell,
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Instant,
};

/// The campaign and the session capture `capture`'s journal in `directory` begins with, read
/// from its first record and never letting go of a segment.
pub fn journal_start(directory: &Path, capture: u32) -> io::Result<(Vec<u8>, SessionStart)> {
    let mut journal = Journal::open_read_only(directory, capture);
    journal.start(0)?;
    match journal.record()? {
        Some(Record::Begin { identity, capture }) => Ok((
            replay_columnar::envelope::Identity::read_capture(identity)?.campaign,
            live::capture_session(capture)?,
        )),
        _ => Err(io::Error::other(format!("{}'s journal does not begin with its capture's start", directory.display()))),
    }
}

/// The facts file the recorder keeps for the journal in `directory` of campaign `campaign`, by
/// its layout: `<session>\<campaign>\`, or that folder's `kept-journal\`, holds the journal, and
/// `<session>\facts.bin` the session's facts. None when the journal is not where a recorder puts
/// it or the session has no facts file.
pub fn session_facts_file(directory: &Path, campaign: &[u8]) -> Option<PathBuf> {
    let folder = match directory.file_name()? == "kept-journal" {
        true => directory.parent()?,
        false => directory,
    };
    let named = folder.file_name()?.as_encoded_bytes().eq_ignore_ascii_case(campaign);
    let file = folder.parent()?.join(SESSION_FACTS_FILE);
    (named && file.is_file()).then_some(file)
}

/// The facts pack the file at `path`, holding `bytes`, gives a recording of content `content`,
/// as the recorder's publish gives it: a session's facts file gives its pack verbatim when they
/// are `content`'s facts and is refused by name otherwise; any other file is a bare pack, given
/// as it is unless `session_only`.
pub fn facts_pack(path: &Path, bytes: Vec<u8>, content: &Identity, session_only: bool) -> io::Result<Vec<u8>> {
    if !bytes.starts_with(SESSION_FACTS_MAGIC) {
        return match session_only {
            true => Err(io::Error::other(format!("{} is not a session's facts file", path.display()))),
            false => Ok(bytes),
        };
    }
    session_facts_for(&bytes, content)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| io::Error::other(format!("{} is another content's facts", path.display())))
}

/// Writes every branch and saves every revision, counting what it is told.
pub struct Everything {
    pub facts: Option<Vec<u8>>,
    pub revisions: u64,
    /// A revision was committed since this was last taken.
    pub committed: bool,
    /// From the `pace.1`th hour on, at most `pace.0` hours a second, as a game gives them.
    pub pace: Option<(f64, u64)>,
    pub hours: u64,
    pub paced: Option<Instant>,
    /// Hours applied so far, for a `stop` that cannot borrow the host.
    pub taken: Rc<Cell<u64>>,
    /// The hour applied last.
    pub last: Option<Hour>,
}

impl Everything {
    pub fn new(facts: Option<Vec<u8>>, pace: Option<(f64, u64)>) -> Self {
        Self { facts, revisions: 0, committed: false, pace, hours: 0, paced: None, taken: Rc::default(), last: None }
    }
}

impl Host for Everything {
    fn admit(&mut self, _: &Asked) -> Admit {
        Admit::Write
    }

    fn saves(&mut self) -> bool {
        true
    }

    fn facts(&mut self, _: &Identity) -> Option<Vec<u8>> {
        self.facts.take()
    }

    fn hour(&mut self, applied: &Applied<'_>) {
        engine_allocation::labels::at_hour(i64::from(applied.hour) + 1);
        self.hours += 1;
        if self.hours == 1 {
            engine_allocation::labels::phase("block");
        }
        self.taken.set(self.hours);
        self.last = Some(applied.hour);
        let Some((rate, from)) = self.pace else { return };
        if self.hours < from {
            return;
        }
        let started = *self.paced.get_or_insert_with(Instant::now);
        let due = std::time::Duration::from_secs_f64((self.hours - from) as f64 / rate);
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }

    fn committed(&mut self, _: &Committed) {
        self.revisions += 1;
        self.committed = true;
    }
}

/// Reads the process's working set and commit into the phase under way every 50 ms, on a thread
/// of its own, when the build counts labels.
pub fn sample_memory() -> io::Result<()> {
    if !engine_allocation::labels::ENABLED {
        return Ok(());
    }
    // The system's peak counters cannot be reset per phase: each phase keeps the most a
    // sampler reads every 50 ms.
    std::thread::Builder::new().name("memory sampler".into()).spawn(|| {
        loop {
            engine_allocation::labels::sample_os();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    })?;
    Ok(())
}

/// Tables holding less than this are left out of a revision line's `tables`.
const TABLE_LINE_BYTES: usize = 64 << 10;

/// A committed revision's line (`--memory`): what the writer holds by part, the key index by
/// table (its bytes and the table's live rows), what each table over 64 KiB holds by part, what
/// the summaries hold by summarized table (its ladder; its block builder now and, in a labelled
/// build, at its most in the block before, read once a day), each derived-table writer's resume
/// state, and the encoder's peaks.
pub fn revision_line(writer: &LiveWriter, revision: u64, hours: u64, started: Instant) -> Map<String, Value> {
    let parts: Map<String, Value> = writer.held_parts().into_iter().map(|(name, bytes)| (name, json!(bytes))).collect();
    let encoder = writer.store().encoder().map(|encoder| (encoder.held, encoder.written));
    let by_table = writer.store().bytes().1;
    let name = |table: usize| format!("{:?}", replay_data::rows::TableId::ALL[table]);
    let key_index: Map<String, Value> =
        by_table.iter().enumerate().filter(|(_, bytes)| bytes.index > 0).map(|(table, bytes)| (name(table), json!([bytes.index, bytes.live_rows]))).collect();
    let tables: Map<String, Value> = by_table
        .iter()
        .enumerate()
        .filter(|(_, bytes)| bytes.total() > TABLE_LINE_BYTES)
        .map(|(table, bytes)| {
            let held = json!({ "total": bytes.total(), "cells": bytes.columns.numbers, "lists": bytes.columns.list_spans + bytes.columns.list_items,
                "bits": bytes.columns.bits + bytes.live, "keys": bytes.keys, "index": bytes.index, "other": bytes.views + bytes.last_complete, "live_rows": bytes.live_rows });
            (name(table), held)
        })
        .collect();
    let summaries: Map<String, Value> = writer
        .summary_bytes()
        .into_iter()
        .filter(|&(_, ladder, builder, peak)| ladder + builder + peak > 0)
        .map(|(table, ladder, builder, peak)| (name(table), json!({ "ladder": ladder, "builder": builder, "builder_peak_in_block": peak })))
        .collect();
    let blobs: Map<String, Value> = writer.writer_blobs().unwrap_or_default().into_iter().map(|(writer, bytes)| (writer.to_owned(), json!(bytes))).collect();
    let Value::Object(line) = json!({ "summaries": summaries, "writer_blobs": blobs, "revision": revision, "hours": hours, "seconds": started.elapsed().as_secs_f64(), "held": parts,
        "key_index": key_index, "tables": tables,
        "encoder_peaks": encoder.map(|(held, _)| json!({ "pending": held.pending, "previous": held.previous, "pending_and_previous": held.pending_and_previous,
            "closing": held.closing, "queued": held.queued, "zstd_context": held.context, "restoring": held.restoring }))
    }) else {
        unreachable!("an object")
    };
    line
}

/// Whether the commit just made, `hours` into the run, ended block `block`: printed when it did,
/// for the caller to end as a kill would.
pub fn stopped_after(block: Option<u64>, hours: u64) -> bool {
    if !block.is_some_and(|block| hours / 720 == block + 1) {
        return false;
    }
    eprintln!("{}", json!({ "stopped_after_block": block, "hours": hours }));
    true
}

/// The cells of rows inserted that no hour gave, by table.
pub fn unwritten_line(writer: &LiveWriter) -> Value {
    let unwritten: Map<String, Value> = writer
        .store()
        .unwritten()
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(table, &count)| (format!("{:?}", replay_data::rows::TableId::ALL[table]), json!(count)))
        .collect();
    json!({ "inserted_cells_not_given": unwritten })
}

/// The run's line, but for what only its encoder knows: the file and its parts, the writer's
/// time per game hour by stage, the encoder's counts and peaks, the derived writers' time.
pub fn written(writer: &LiveWriter, out: &Path, revisions: u64, hours_written: u64, started: Instant, timed: bool) -> io::Result<Map<String, Value>> {
    let hours = hours_written.max(1) as f64;
    let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1e3 / hours;
    let spent = writer.spent();
    let encoder = writer.store().encoder().map(|encoder| (encoder.written, encoder.held, encoder.timings));
    let file_bytes = std::fs::metadata(out)?.len();
    let written = writer.written().unwrap_or_default();
    let by_part = encoder.map(|(unit, _, _)| {
        let units = unit.stored();
        let restores = unit.restores.0;
        let known = units + restores + written.summaries + written.strings + written.clock + written.pages + written.roots;
        json!({ "units": units, "restores": restores, "summaries": written.summaries, "strings": written.strings, "clock": written.clock,
            "pages": written.pages, "roots": written.roots, "records_facts_footers_state": file_bytes.saturating_sub(known),
            "units_by_kind": { "numeric": unit.numeric.0, "list": unit.list.0, "lifetimes": unit.lifetimes.0, "strings": unit.strings.0, "marks": unit.marks.0, "small": unit.small.0 } })
    });
    let split = encoder.map(|(_, _, timings)| {
        // Appending is timed a call at a time: the clock reads that costs are taken off.
        let calls = timings.appends.min(20_000_000);
        let clock = Instant::now();
        for _ in 0..calls {
            std::hint::black_box(Instant::now());
        }
        let read = if calls == 0 { 0.0 } else { clock.elapsed().as_secs_f64() / calls as f64 };
        let appending = timings.appending.as_secs_f64() - timings.appends as f64 * read;
        let appending_ms = appending * 1e3 / hours;
        json!({
            "reading_the_journal": ms(spent.journal),
            "applying_to_the_store": ms(spent.store) - appending_ms - 2.0 * timings.appends as f64 * read * 1e3 / hours,
            "appending_to_pending": appending_ms,
            "writers": ms(spent.writers),
            "summaries": ms(spent.summaries),
            "hour_ends": ms(spent.ending),
            "closing_parts": ms(timings.closing),
            "compressing_parts_on_its_thread": ms(timings.compressing_parts),
            "restores": { "making": ms(timings.restoring), "compressing_on_its_thread": ms(timings.compressing_restores), "reading_back": ms(timings.reading_back) },
            "frame_digests": ms(timings.digesting),
            "frames_to_the_file": ms(timings.writing),
            "waiting_for_the_compression_thread": ms(timings.waiting),
            "block_pages": ms(spent.blocks),
            "appends_timed": timings.appends, "clock_read_ns": read * 1e9,
        })
    });
    let Value::Object(line) = json!({
        "out": out.display().to_string(),
        "bytes": file_bytes,
        "bytes_by_part": by_part,
        "revisions": revisions,
        "hours": hours_written,
        "wall_s": started.elapsed().as_secs_f64(),
        "time_split_ms_per_game_hour": split,
        "timed": timed,
        "encoder": encoder.map(|(unit, held, _)| json!({ "parts": unit.parts, "frames": unit.frames, "events": unit.events,
            "closes": { "cap": unit.closes_cap, "budget": unit.closes_budget, "block": unit.closes_block },
            "restores": { "whole": unit.whole_restores, "based": unit.based_restores, "named_again": unit.repeated_restores, "chunks": unit.restore_chunks,
                "chunks_named_again": unit.named_chunks, "worst_replay_events": unit.worst_replay, "packed": unit.packed, "read_back": unit.read_back },
            "peaks": { "pending": held.pending, "previous": held.previous, "pending_and_previous": held.pending_and_previous, "closing": held.closing,
                "queued": held.queued, "zstd_context": held.context, "restoring": held.restoring } })),
        "summaries_us_per_hour": writer.derive_ns().1 as f64 / 1e3 / hours,
        "writers_us_per_hour": writer.derive_ns().0 as f64 / 1e3 / hours,
        "writer_ns": writer.writer_spent().into_iter().map(|(name, ns)| (name.to_owned(), json!(ns))).collect::<Map<_, _>>(),
    }) else {
        unreachable!("an object")
    };
    Ok(line)
}

/// Every phase, on stderr: the start with the run's first hour, then each block of the run (the
/// first without its first hour): its labelled peak, the most working set and commit sampled in
/// it, the hour at its peak, and every label holding a MiB or more at the peak or at its own.
pub fn report_phases() {
    for (at, phase) in engine_allocation::labels::phases().iter().enumerate() {
        let os = phase.os_at_peak.map(|os| json!({ "working": os.working, "commit": os.commit, "peak_working": os.peak_working }));
        let summary = json!({ "block": at.checked_sub(1), "first_hour": at == 0, "phase_peak": phase.tracked_peak, "hour_at_peak": phase.hour_at_peak,
            "os_peak_working": phase.os_peak_working, "os_peak_commit": phase.os_peak_commit, "os_at_peak": os });
        let mut labels: Vec<_> = phase.labels.iter().filter(|charge| charge.at_peak.abs() > 1 << 20 || charge.peak > 1 << 20).collect();
        labels.sort_by_key(|charge| std::cmp::Reverse(charge.at_peak));
        let held: Vec<Value> = labels
            .iter()
            .map(|charge| json!({ "owner": charge.owner, "kind": charge.kind.name(), "at_peak": charge.at_peak, "peak": charge.peak, "end": charge.end }))
            .collect();
        eprintln!("{}", json!({ "phase": summary, "phase_peak": phase.tracked_peak, "os_at_peak": os, "labels": held }));
    }
}
