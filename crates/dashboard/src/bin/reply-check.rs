//! Asks this build every request of reference transcripts that the query layer answers, and
//! compares the replies with the kept ones ([`dashboard::reply_check`]); every other request is
//! counted as skipped. Times each reply, the recording opened and indexed once. A chart this build
//! serves as tiles is answered from the tiles covering its view, assembled as the UI does, and
//! timed as the sum of those tile requests (`--cold`: the first tile cold, the rest after it, as a
//! chart's tiles are asked together).
//! `reply-check --recording R --transcripts DIR --report DIR [--families F,...] [--ignore PATH]... [--replies] [--cold]`
//! `--cold` opens the recording afresh before every request, builds its index (timed apart, as the
//! open's cost, `indexMilliseconds`), then drops the file's cached pages, so the request's reads
//! and decodes are cold and the index is present. `--no-index` forgets only the index before each
//! request, the file cache warm. With game content, the recording's content catalogue is kept in
//! the cache folder once, before the first request, as opening a recording in the app does (timed
//! apart, `catalogueMilliseconds`): a cold request loads it from there, never from memory.
//! `--replies` also keeps every exchange, both replies and the time, as `DIR/{family}.replies.zst`;
//! a tiled chart's exchange also names its tiles and each one's time (`tiles`).
//! A path is dotted from the reply's root, `*` for any member or element. Always passed over:
//! `digest` and the preparation states (`gate`, `holes`, `data.coverage`, `data.pending`,
//! `data.approximate`, `data.preparing`). Game content: none (the recording's own facts), unless
//! `--game G --user U --cache C` name the game's and the user's folders (read only) and a cache, for
//! products that read localisation.
//! Exits 1 on a difference, 2 on a setup error.

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
    std::fs::create_dir_all(&report)?;
    let cancelled = AtomicBool::new(false);
    let opened = Instant::now();
    let open = || {
        dashboard::Source::open(
            dashboard::SourceKey { source_id: "reply-check".into(), revision: dashboard::protocol::replay_revision(&recording)? },
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
            options: installed_content_core::ResolveOptions { game_dir, user_dir, cache_dir },
            allow_unverified: false,
            presentation: Default::default(),
        }),
        (None, None, None) => None,
        _ => return Err(invalid(USAGE)),
    };
    let catalogue_ms = content.as_ref().map(|content| {
        let started = Instant::now();
        let source = source.borrow();
        source.in_scope(|| dashboard_base::content::warm(&source, content));
        started.elapsed().as_secs_f64() * 1e3
    });
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
                // The reply as the service sends it: serialised.
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
        let checked = reply_check::check(
            &mut reader,
            &|product, params| answer::answers(product, params),
            &mut serve,
            &mut |_: &Value| -> f64 {
                if no_index {
                    answer::forget(&source.borrow());
                    return 0.0;
                }
                if !cold {
                    return 0.0;
                }
                let Ok(fresh) = open() else { return 0.0 };
                *source.borrow_mut() = fresh;
                let started = Instant::now();
                let _ = answer::open_all(&source.borrow());
                let index = started.elapsed().as_secs_f64() * 1e3;
                purge(&recording);
                index
            },
            &ignored,
            kept.as_mut().map(|kept| kept as &mut dyn std::io::Write),
        )?;
        if let Some(kept) = kept {
            kept.finish()?;
        }
        if checked.queries > 0 {
            same &= checked.differing == 0;
            summary.push(json!({ "family": family, "queries": checked.queries, "differing": checked.differing, "skipped": checked.skipped }));
            std::fs::write(report.join(format!("{family}.json")), serde_json::to_string_pretty(&checked)?)?;
        }
    }
    println!("{}", serde_json::to_string_pretty(&json!({ "openMilliseconds": open_ms, "catalogueMilliseconds": catalogue_ms, "families": summary }))?);
    Ok(same)
}

/// Drops the pages Windows caches of `path`: opening it unbuffered does.
fn purge(path: &std::path::Path) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
    let _ = std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_NO_BUFFERING).open(path);
}
