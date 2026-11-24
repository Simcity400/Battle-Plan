use replay::campaign::live::Progress;
use replay_settings::compression::Settings;
use replay::campaign::encode_report::{self, Everything};
use replay_journal::{columnar_journal::Source, multiplied::{Factors, Multiplied, Start}};
use serde_json::json;
use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

engine_allocation::counting_system_allocator!();

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || io::Error::new(io::ErrorKind::InvalidInput, "journal-encode <session directory> <capture> <out.battleplan> [--level=N] [--facts=<file>] [--memory] [--timed] [--pace=<hours a second>,<from hour>] [--times=<factors>] [--start=<block> --start-file=<file>] [--hours=N] [--stop-after=<block of the capture, from 0>] | journal-encode --preroll <session directory> <capture> <block> <file>");
    if let [flag, directory, capture, block, file] = args.as_slice()
        && flag == "--preroll"
    {
        let started = Instant::now();
        replay_journal::multiplied::preroll(Path::new(directory), capture.parse().map_err(|_| usage())?, block.parse().map_err(|_| usage())?, Path::new(file))?;
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
    if let (Some(start), Some(block)) = (start, stop_after)
        && block <= u64::from(start)
    {
        return Err(io::Error::other(format!("Block {block} of capture {capture} comes before block {}, the first this start writes", u64::from(start) + 1)));
    }
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
    let multiplied = times.as_ref().is_some_and(|times| times.factor.iter().any(|&factor| factor > 1));
    let encoded_as = encode_report::encoder_source(multiplied, start.as_ref().map(|start| start.block));
    let mut writer = encode_report::open_encoded(out, &state, capture, settings, &encoded_as, recorder_derive::writers())?;
    writer.time(timed);
    let multiplication_enabled = replay_journal::FEATURES.contains(&"multiplied");
    if (times.is_some() || start.is_some()) && !multiplication_enabled {
        return Err(io::Error::other("--times and --start need a build with the feature recorder-replay/multiplied"));
    }
    let tables = replay_data::rows::TableId::ALL.len();
    let factors = times.unwrap_or_else(|| Factors { factor: vec![1; tables], in_phase: vec![false; tables] });
    let started_after = start.as_ref().map(|start| u64::from(start.block) + 1);
    let mut source = Multiplied::open(Path::new(directory), capture, factors, start)?;
    let first_hours = source.hours_before(writer.segments());
    source.start(writer.segments())?;
    eprintln!("{}", json!({ "in_phase": source.in_phase(), "journal_schema_digest_is_a_prefix": source.exact_schema, "taken_up_at_segment": writer.segments(),
        "build": { "memory_labels": engine_allocation::labels::ENABLED, "multiplied": multiplication_enabled, "keep_journal": replay_journal::FEATURES.contains(&"keep-journal") } }));
    let facts = facts.filter(|_| !writer.carries_facts());
    let mut host = Everything::new(facts, pace);
    host.blocks = started_after.unwrap_or_else(|| source.blocks_before(writer.segments()));
    if let Some(block) = stop_after.filter(|&block| block < host.blocks) {
        return Err(io::Error::other(match started_after {
            Some(first) => format!("Block {block} of capture {capture} comes before block {first}, the first this start writes"),
            None => format!("Block {block} of capture {capture} is committed in {} already", out.display()),
        }));
    }
    let taken = host.taken.clone();
    loop {
        let stop = || memory || stop_after.is_some() || taken.get() >= limit;
        let progress = writer.advance(&mut source, &mut host, &stop)?;
        if std::mem::take(&mut host.committed) {
            if memory {
                eprintln!("{}", serde_json::Value::Object(encode_report::revision_line(&writer, host.revisions, host.hours + first_hours, started)));
            }
            if encode_report::stopped_after(stop_after, host.blocks, host.hours + first_hours) {
                std::process::exit(0);
            }
        }
        match progress {
            Progress::Finished => break,
            Progress::Waiting => {
                writer.finish_stopped(&mut source, &mut host, "the journal ends")?;
                if std::mem::take(&mut host.committed) {
                    encode_report::stopped_after(stop_after, host.blocks, host.hours + first_hours);
                }
                break;
            }
            Progress::Paused if host.hours >= limit => {
                writer.finish_stopped(&mut source, &mut host, "the hours asked are written")?;
                if std::mem::take(&mut host.committed) {
                    encode_report::stopped_after(stop_after, host.blocks, host.hours + first_hours);
                }
                break;
            }
            Progress::Paused => continue,
            Progress::Undecided(_) => return Err(io::Error::other("A branch waits to be admitted")),
            Progress::AwaitsFacts => return Err(io::Error::other("The session's game facts are still being packed")),
        }
    }
    if memory && engine_allocation::labels::ENABLED {
        encode_report::report_phases();
    }
    eprintln!("{}", encode_report::unwritten_line(&writer));
    let mut line = encode_report::written(&writer, out, host.revisions, host.hours, started, timed)?;
    line.insert("multiplier_s".into(), json!(source.spent.as_secs_f64()));
    line.insert("build".into(), json!({ "memory_labels": engine_allocation::labels::ENABLED, "multiplied": multiplication_enabled }));
    println!("{}", serde_json::Value::Object(line));
    Ok(())
}

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
