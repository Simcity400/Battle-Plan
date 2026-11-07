//! The check of a recording the recorder wrote from a capture's journal: every hour the
//! journal holds, applied in order, against the written file's walk, as [`super::check`]
//! compares a converted segment.

use replay::{
    columnar_frames::{Recording, keeps_last, schema},
    columnar_journal::{self, Record, Segment},
};
use replay_columnar::{manifest::Manifest, walk::State};
use replay_data::rows::TableId;
use serde_json::{Value, json};
use std::{io, path::Path};

/// Each run of capture `capture` the journal in `directory` holds: its hours' manifests and
/// sections, in order. A run holding no hour is no run of the file.
fn runs(directory: &Path, capture: u32) -> io::Result<Vec<Vec<(Vec<u8>, Vec<u8>)>>> {
    let mut runs: Vec<Vec<(Vec<u8>, Vec<u8>)>> = Vec::new();
    for (taken, _, path) in columnar_journal::segments(directory)? {
        if taken != capture {
            continue;
        }
        let segment = Segment::read(&path)?;
        for record in segment.records(0) {
            match record?.0 {
                Record::Run { .. } => runs.push(Vec::new()),
                Record::Hour { manifest, sections } => runs
                    .last_mut()
                    .ok_or_else(|| super::invalid("A journal's hour precedes its first run"))?
                    .push((manifest.to_vec(), sections.to_vec())),
                _ => {}
            }
        }
    }
    runs.retain(|run| !run.is_empty());
    Ok(runs)
}

fn fresh() -> io::Result<State> {
    let mut state = State::new(schema()?)?;
    state.keep_last(&keeps_last())?;
    Ok(state)
}

pub fn check(directory: &Path, capture: u32, written: &Path) -> io::Result<Value> {
    let runs = runs(directory, capture)?;
    let recording = Recording::open_path(written)?;
    let columns: Vec<usize> = schema()?.iter().map(Vec::len).collect();
    let mut failures: Vec<String> = Vec::new();
    let mut theirs: Option<State> = None;
    let (mut run, mut next, mut walked) = (0usize, 0usize, 0u64);
    let mut ours = fresh()?;
    recording.walk(&mut ours, |ours, manifest, first| {
        if first {
            // Each hour's changed tables are compared as it applies; whole tables, once the walk
            // ends, as the walker begins each run afresh.
            if theirs.take().is_some() {
                if next != runs[run].len() {
                    failures.push(format!("run {run}: the file holds {next} of the journal's {} hours", runs[run].len()));
                }
                run += 1;
            }
            theirs = Some(fresh()?);
            next = 0;
        }
        let Some((bytes, sections)) = runs.get(run).and_then(|hours| hours.get(next)) else {
            failures.push(format!("run {run} ordinal {next}: the file holds an hour the journal does not"));
            return Ok(false);
        };
        next += 1;
        walked += 1;
        let source = Manifest::read(bytes, &columns)?;
        let mut spans = Vec::with_capacity(source.sections.len());
        let mut at = 0usize;
        for section in &source.sections {
            spans.push(at..at + section.bytes as usize);
            at += section.bytes as usize;
        }
        let held = theirs.as_mut().expect("a run under way");
        held.apply(&source, |index| Ok(&sections[spans[index].clone()]))?;
        let ordinal = next - 1;
        if !super::same_record(manifest, &source) {
            failures.push(format!("run {run} ordinal {ordinal}: hour records differ"));
        }
        for table in 0..columns.len() {
            if ours.status(table) != held.status(table)
                || ours.sampled_at(table) != held.sampled_at(table)
                || ours.truncated_view(table) != held.truncated_view(table)
            {
                failures.push(format!("run {run} ordinal {ordinal}: {:?} status, sample or view differ", TableId::ALL[table]));
            }
        }
        for section in &source.sections {
            let table = section.table;
            let mine = super::digest(ours.table(table).expect("table"), ours.changes(table).expect("table"));
            let journal = super::digest(held.table(table).expect("table"), held.changes(table).expect("table"));
            if mine != journal {
                failures.push(format!("run {run} ordinal {ordinal}: {:?} differs", TableId::ALL[table]));
            }
        }
        Ok(failures.len() < 20)
    })?;
    if let Some(held) = &theirs {
        compare_whole(held, &ours, run, &mut failures);
        if next != runs[run].len() {
            failures.push(format!("run {run}: the file holds {next} of the journal's {} hours", runs[run].len()));
        }
        run += 1;
    }
    if run != runs.len() {
        failures.push(format!("the file holds {run} of the journal's {} runs", runs.len()));
    }
    let journaled: usize = runs.iter().map(Vec::len).sum();
    Ok(json!({
        "journal": directory.display().to_string(),
        "capture": capture,
        "written": written.display().to_string(),
        "runs": runs.len(),
        "journaled_hours": journaled,
        "walked_hours": walked,
        "failures": failures.len(),
        "first_failures": failures,
    }))
}

/// Every table whole, and the dictionary, at the walk's end.
fn compare_whole(theirs: &State, ours: &State, run: usize, failures: &mut Vec<String>) {
    for table in 0..TableId::ALL.len() {
        if ours.table(table) != theirs.table(table) {
            failures.push(format!("run {run}: {:?} differs at its end", TableId::ALL[table]));
        }
    }
    let shared = theirs.dictionary().len().min(ours.dictionary().len()) as u32;
    for id in 0..shared {
        let (mut left, mut right) = (None, None);
        let same = theirs.dictionary().resolve(id, &mut left).ok() == ours.dictionary().resolve(id, &mut right).ok();
        if !same {
            failures.push(format!("run {run}: dictionary id {id} differs"));
            break;
        }
    }
}
