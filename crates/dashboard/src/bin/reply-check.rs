
use dashboard::{answer, reply_check};
use dashboard_core::config::LoadContext;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{self, BufReader},
    path::PathBuf,
    process::ExitCode,
    sync::atomic::AtomicBool,
    time::Instant,
};

#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

const USAGE: &str = "reply-check --recording R --transcripts DIR --report DIR [--families F,...] [--ignore PATH]... [--replies] [--cold | --no-index] [--game G --user U --cache C]";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> io::Result<bool> {
    let mut args = std::env::args_os().skip(1);
    let (mut recording, mut transcripts, mut report, mut families) = (None, None, None, None);
    let mut ignored: Vec<Vec<String>> = ["digest", "gate", "holes", "data.coverage", "data.pending", "data.approximate", "data.preparing"]
        .iter()
        .map(|path| path.split('.').map(str::to_owned).collect())
        .collect();
    let (mut replies, mut cold, mut no_index) = (false, false, false);
    let (mut game, mut user, mut cache) = (None, None, None);
    while let Some(flag) = args.next() {
        if flag == "--no-index" {
            no_index = true;
            continue;
        }
        if flag == "--replies" {
            replies = true;
            continue;
        }
        if flag == "--cold" {
            cold = true;
            continue;
        }
        let value = args.next().ok_or_else(|| invalid(USAGE))?;
        match flag.to_str() {
            Some("--recording") => recording = Some(PathBuf::from(value)),
            Some("--transcripts") => transcripts = Some(PathBuf::from(value)),
            Some("--report") => report = Some(PathBuf::from(value)),
            Some("--families") => families = Some(value.to_string_lossy().split(',').map(str::to_owned).collect::<Vec<_>>()),
            Some("--ignore") => ignored.push(value.to_string_lossy().split('.').map(str::to_owned).collect()),
            Some("--game") => game = Some(PathBuf::from(value)),
            Some("--user") => user = Some(PathBuf::from(value)),
            Some("--cache") => cache = Some(PathBuf::from(value)),
            _ => return Err(invalid(USAGE)),
        }
    }
    let (recording, transcripts, report) = (
        recording.ok_or_else(|| invalid(USAGE))?,
        transcripts.ok_or_else(|| invalid(USAGE))?,
        report.ok_or_else(|| invalid(USAGE))?,
    );
    if cold && no_index {
        return Err(invalid(USAGE));
    }
    let cold_cache = cold
        .then(|| {
            replay_tools::cold_cache::probe_file(
                &recording,
                replay_tools::cold_cache::ResetScope::FreshSourcePreIndexedPerExchange,
            )
        })
        .transpose()?;
    std::fs::create_dir_all(&report)?;
    let cancelled = AtomicBool::new(false);
    let opened = Instant::now();
    let open = || {
        dashboard_base::Source::open(
            dashboard_base::SourceKey { source_id: "reply-check".into(), revision: dashboard_core::protocol::replay_revision(&recording)? },
            recording.clone(),
            &cancelled,
        )
    };
    let source = std::cell::RefCell::new(open()?);
    let source_ref = source.borrow();
    engine_executable::executable_tables::enter(source_ref.tables());
    answer::open_all(&source_ref)?;
    drop(source_ref);
    let open_ms = opened.elapsed().as_secs_f64() * 1e3;
    let content = match (game, user, cache) {
        (Some(game_dir), Some(user_dir), Some(cache_dir)) => Some(dashboard_core::config::ContentOptions {
            options: content_reader_core::ResolveOptions { game_dir, user_dir, cache_dir },
            allow_unverified: false,
            presentation: Default::default(),
        }),
        (None, None, None) => None,
        _ => return Err(invalid(USAGE)),
    };
    let catalogue_ms = content
        .as_ref()
        .map(|content| {
            let source = source.borrow();
            reply_check::warm_catalogue(&source, content)
        })
        .transpose()?;
    let context = LoadContext { content };
    let mut names: Vec<String> = std::fs::read_dir(&transcripts)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().and_then(|name| name.strip_suffix(".zst")).map(str::to_owned))
        .collect();
    names.sort();
    if let Some(families) = &families {
        names.retain(|name| families.contains(name));
    }
    let mut same = true;
    let mut summary = Vec::new();
    for family in names {
        let file = File::open(transcripts.join(format!("{family}.zst")))?;
        let mut reader = BufReader::new(zstd::stream::read::Decoder::new(file)?);
        reply_check::read_header(&mut reader)?;
        let mut serve = |product: &str, params: &Value| {
            let source = source.borrow();
            let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                source.in_scope(|| answer::answer(&source, product, params, &context, &cancelled))
            }));
            match answered {
                Ok(Ok(data)) => match serde_json::to_string(&data) {
                    Ok(_) => reply_check::Reply::Answered(json!({ "data": data })),
                    Err(error) => reply_check::Reply::Refused(error.to_string()),
                },
                Ok(Err(error)) => reply_check::Reply::Refused(error.to_string()),
                Err(_) => reply_check::Reply::Refused("The query panicked".into()),
            }
        };
        let mut kept = replies
            .then(|| File::create(report.join(format!("{family}.replies.zst"))).and_then(|file| zstd::stream::write::Encoder::new(file, 3)))
            .transpose()?;
        let mut checked = reply_check::check(
            &mut reader,
            &|product, params| answer::answers(product, params),
            &mut serve,
            &mut |_: &Value| -> io::Result<f64> {
                if no_index {
                    answer::forget(&source.borrow());
                    return Ok(0.0);
                }
                if !cold {
                    return Ok(0.0);
                }
                replay_tools::cold_cache::probe_file(
                    &recording,
                    replay_tools::cold_cache::ResetScope::FreshSourcePreIndexedPerExchange,
                )?;
                let fresh = open()?;
                *source.borrow_mut() = fresh;
                let started = Instant::now();
                answer::open_all(&source.borrow())?;
                Ok(started.elapsed().as_secs_f64() * 1e3)
            },
            &ignored,
            kept.as_mut().map(|kept| kept as &mut dyn std::io::Write),
        )?;
        checked.cache = cold_cache.clone();
        if let Some(kept) = kept {
            kept.finish()?;
        }
        if checked.queries > 0 {
            same &= checked.differing == 0;
            summary.push(json!({ "family": family, "queries": checked.queries, "differing": checked.differing, "skipped": checked.skipped }));
            std::fs::write(report.join(format!("{family}.json")), serde_json::to_string_pretty(&checked)?)?;
        }
    }
    let mode = if cold {
        "fresh-source-pre-indexed"
    } else if no_index {
        "no-index"
    } else {
        "warm"
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "mode": mode,
            "cache": cold_cache,
            "openMilliseconds": open_ms,
            "catalogueMilliseconds": catalogue_ms,
            "families": summary,
        }))?
    );
    Ok(same)
}
