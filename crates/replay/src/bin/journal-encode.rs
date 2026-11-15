//! Writes a capture's journal into a new recording through the recorder's own writer
//! ([`replay::campaign::live::LiveWriter`]), every branch admitted and every revision saved, so a
//! fixture is made without the game. Dev tool: `journal-encode <session directory> <capture>
//! <out.battleplan> [--level=N] [--facts=<file>] [--memory] [--timed] [--times=<factors>]
//! [--start=<block>] [--hours=N] [--stop-after=<block>]`. The journal is read only, through
//! [`replay::multiplied`]: no segment is removed or moved, and a journal written before this
//! schema's newest tables is read as this schema's. `--times` multiplies its world; `--start`
//! begins the run after the journal's block `N` from `--start-file`, which a child process
//! (`journal-encode --preroll <session directory> <capture> <block> <file>`) writes first when it
//! is missing, so the journal's hours before the start are not walked in the process measured;
//! `--hours` stops after that many hours;
//! `--stop-after` ends the process right after the commit at that block's end, as a kill would,
//! and a later run on the same file takes it up. `--memory` prints what each block held, and every
//! phase's labels at its peak when the build counts them (the run's first hour is a phase of its
//! own); `--timed` times the encoder's work a call at a time. The recording gets its facts as the
//! recorder's publish gives them: `--facts` names a session's facts file (`facts.bin`), whose pack
//! is taken when it is the journal's content's, or a bare pack; without it, the session's
//! `facts.bin` where the recorder's layout puts it beside the journal, when there is one. Facts of
//! another content are refused by name. Prints what the writing took and stored as one JSON line.

use replay::{
    campaign::live::{LiveWriter, Progress},
    columnar_file::Settings,
    columnar_journal::Source,
    encode_report::{self, Everything},
    multiplied::{Factors, Multiplied, Start},
};
use serde_json::json;
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

#[cfg(feature = "memory-labels")]
#[global_allocator]
static ALLOCATOR: engine_allocation::labels::Counting<std::alloc::System> =
    engine_allocation::labels::Counting(std::alloc::System);

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || io::Error::new(io::ErrorKind::InvalidInput, "journal-encode <session directory> <capture> <out.battleplan> [--level=N] [--facts=<file>] [--memory] [--timed] [--pace=<hours a second>,<from hour>] [--times=<factors>] [--start=<block> --start-file=<file>] [--hours=N] [--stop-after=<block>] | journal-encode --preroll <session directory> <capture> <block> <file>");
    if let [flag, directory, capture, block, file] = args.as_slice()
        && flag == "--preroll"
    {
        let started = Instant::now();
        replay::multiplied::preroll(Path::new(directory), capture.parse().map_err(|_| usage())?, block.parse().map_err(|_| usage())?, Path::new(file))?;
        eprintln!("{}", json!({ "preroll": file, "seconds": started.elapsed().as_secs_f64(), "bytes": std::fs::metadata(file)?.len() }));
        return Ok(());
    }
    let [directory, capture, out, rest @ ..] = args.as_slice() else {
        return Err(usage());
    };
    let capture: u32 = capture.parse().map_err(|_| usage())?;
    let mut settings = Settings::HIGH;
    let (mut facts, mut memory, mut timed, mut pace, mut times) = (None, false, false, None, None);
    let (mut limit, mut start, mut stop_after, mut start_file) = (u64::MAX, None, None, None);
    for option in rest {
        if let Some(level) = option.strip_prefix("--level=") {
            settings.level = level.parse().map_err(|_| usage())?;
        } else if let Some(given) = option.strip_prefix("--pace=") {
            let (rate, from) = given.split_once(',').ok_or_else(usage)?;
            pace = Some((rate.parse().map_err(|_| usage())?, from.parse().map_err(|_| usage())?));
        } else if let Some(given) = option.strip_prefix("--hours=") {
            limit = given.parse().map_err(|_| usage())?;
        } else if let Some(path) = option.strip_prefix("--times=") {
            times = Some(Factors::read(Path::new(path))?);
        } else if let Some(given) = option.strip_prefix("--start=") {
            // `<block>`; a reference recording named before it is no longer read.
            let block = given.rsplit_once(',').map_or(given, |(_, block)| block);
            start = Some(block.parse::<u32>().map_err(|_| usage())?);
        } else if let Some(path) = option.strip_prefix("--start-file=") {
            start_file = Some(std::path::PathBuf::from(path));
        } else if let Some(given) = option.strip_prefix("--stop-after=") {
            stop_after = Some(given.parse::<u64>().map_err(|_| usage())?);
        } else if option == "--memory" {
            memory = true;
        } else if option == "--timed" {
            timed = true;
        } else if let Some(path) = option.strip_prefix("--facts=") {
            facts = Some(PathBuf::from(path));
        } else {
            return Err(usage());
        }
    }
    let facts = journal_facts(Path::new(directory), capture, facts)?;
    eprintln!("{}", json!({ "facts": facts.as_ref().map(|(path, pack)| json!({ "file": path, "pack_bytes": pack.len() })) }));
    let facts = facts.map(|(_, pack)| pack);
    let out = Path::new(out);
    let state = out.with_extension("state");
    let start = match (start, start_file) {
        (None, None) => None,
        (Some(block), Some(file)) => {
            if !file.exists() {
                let status = std::process::Command::new(std::env::current_exe()?).args(["--preroll", directory, &capture.to_string(), &block.to_string()]).arg(&file).status()?;
                if !status.success() {
                    return Err(io::Error::other(format!("The start's pre-roll failed: {status}")));
                }
            }
            Some(Start { block, file })
        }
        _ => return Err(usage()),
    };
    let started = Instant::now();
    if memory {
        encode_report::sample_memory()?;
    }
    let mut writer = LiveWriter::open(out, &state, "journal-encode", capture, settings, Vec::new())?;
    writer.time(timed);
    if (times.is_some() || start.is_some()) && !cfg!(feature = "multiplied") {
        return Err(io::Error::other("--times and --start need a build with the feature replay/multiplied"));
    }
    let tables = replay_data::rows::TableId::ALL.len();
    let factors = times.unwrap_or_else(|| Factors { factor: vec![1; tables], in_phase: vec![false; tables] });
    // Through the generator even at factor 1: it reads a journal written before this schema's
    // newest tables.
    let mut source = Multiplied::open(Path::new(directory), capture, factors, start)?;
    let first_hours = u64::from(writer.segments()).saturating_sub(1) * 720;
    source.start(writer.segments())?;
    eprintln!("{}", json!({ "in_phase": source.in_phase(), "journal_schema_digest_is_a_prefix": source.exact_schema, "taken_up_at_segment": writer.segments(),
        "build": { "memory_labels": engine_allocation::labels::ENABLED, "multiplied": cfg!(feature = "multiplied"), "keep_journal": cfg!(feature = "keep-journal") } }));
    // A recording taken up that carries facts reads its own: the pack is not held at all.
    let facts = facts.filter(|_| !writer.carries_facts());
    let mut host = Everything::new(facts, pace);
    let taken = host.taken.clone();
    loop {
        let stop = || memory || stop_after.is_some() || taken.get() >= limit;
        let progress = writer.advance(&mut source, &mut host, &stop)?;
        if std::mem::take(&mut host.committed) {
            if memory {
                eprintln!("{}", serde_json::Value::Object(encode_report::revision_line(&writer, host.revisions, host.hours + first_hours, started)));
            }
            if encode_report::stopped_after(stop_after, host.hours + first_hours) {
                std::process::exit(0);
            }
        }
        match progress {
            Progress::Finished => break,
            Progress::Waiting => {
                writer.finish_stopped(&mut source, &mut host, "the journal ends")?;
                break;
            }
            Progress::Paused if host.hours >= limit => {
                writer.finish_stopped(&mut source, &mut host, "the hours asked are written")?;
                break;
            }
            Progress::Paused => continue,
            Progress::Undecided(_) => return Err(io::Error::other("A branch waits to be admitted")),
        }
    }
    if memory && engine_allocation::labels::ENABLED {
        encode_report::report_phases();
    }
    eprintln!("{}", encode_report::unwritten_line(&writer));
    let mut line = encode_report::written(&writer, out, host.revisions, host.hours, started, timed)?;
    line.insert("multiplier_s".into(), json!(source.spent.as_secs_f64()));
    line.insert("build".into(), json!({ "memory_labels": engine_allocation::labels::ENABLED, "multiplied": cfg!(feature = "multiplied") }));
    println!("{}", serde_json::Value::Object(line));
    Ok(())
}

/// The facts pack the recording of the journal in `directory` gets, and the file it came from:
/// `given`, or else the session's facts file where the recorder's layout puts it; checked
/// against the journal's own content before anything is written, as the recorder's publish
/// checks it.
fn journal_facts(directory: &Path, capture: u32, given: Option<PathBuf>) -> io::Result<Option<(PathBuf, Vec<u8>)>> {
    let (campaign, session) = encode_report::journal_start(directory, capture)?;
    let (path, session_only) = match given {
        Some(path) => (path, false),
        None => match encode_report::session_facts_file(&std::path::absolute(directory)?, &campaign) {
            Some(path) => (path, true),
            None => return Ok(None),
        },
    };
    let bytes = std::fs::read(&path)?;
    let pack = encode_report::facts_pack(&path, bytes, &session.identity, session_only)?;
    Ok(Some((path, pack)))
}
